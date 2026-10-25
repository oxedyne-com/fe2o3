//! Two IMAP sessions on one folder, over the real server and a real Maildir.
//!
//! Thunderbird and a phone on one account are two sessions. A flag one sets must not be
//! undone by the other's stale copy, and a message the other expunges must not shift this
//! session's sequence numbers until it has been told (RFC 3501 §7.4.1).
//! From QA F2a's `qa_imap.rs` (D-20261026-01, M1 and M2).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::{
    imap::server::ImapServer,
    mail::{
        store::{
            FolderName,
            MailStore,
            MailUser,
            MessageFlags,
        },
        user::UserStore,
    },
};

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};


#[derive(Clone, Debug)]
struct OneUser;

impl UserStore for OneUser {
    fn authenticate(&self, _a: &str, _p: &str) -> Outcome<Option<MailUser>> { self.lookup("") }
    fn lookup(&self, _a: &str) -> Outcome<Option<MailUser>> {
        Ok(Some(MailUser {
            local:          fmt!("alice"),
            domain:         fmt!("test.local"),
            delivery_key:   fmt!("test.local/alice"),
            send_as:        Vec::new(),
        }))
    }
}

/// One logged-in session with INBOX selected.
struct Sess {
    r:  BufReader<TcpStream>,
    n:  u32,
}

impl Sess {
    async fn open(port: u16) -> Outcome<Self> {
        let t = res!(TcpStream::connect(("127.0.0.1", port)).await, IO, Network);
        let mut s = Sess { r: BufReader::new(t), n: 0 };
        let mut l = String::new();
        res!(s.r.read_line(&mut l).await, IO, Network);
        res!(s.cmd("LOGIN alice@test.local pw").await);
        res!(s.cmd("SELECT INBOX").await);
        Ok(s)
    }

    /// Every line of the reply, the tagged one last.
    async fn cmd(&mut self, c: &str) -> Outcome<String> {
        self.n += 1;
        let tag = fmt!("t{}", self.n);
        res!(self.r.get_mut().write_all(fmt!("{} {}\r\n", tag, c).as_bytes()).await, IO, Network);
        let mut out = String::new();
        loop {
            let mut l = String::new();
            if res!(self.r.read_line(&mut l).await, IO, Network) == 0 {
                return Err(err!("The server closed the connection."; IO, Network));
            }
            out.push_str(&l);
            if l.starts_with(&fmt!("{} ", tag)) { return Ok(out); }
        }
    }
}

/// A Maildir with `n` messages served on a local port.
async fn setup(name: &str, n: usize) -> Outcome<(u16, PathBuf)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_sessions_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = match res!(OneUser.lookup("")) {
        Some(u) => u,
        None    => return Err(err!("The test user did not resolve."; Test, Bug)),
    };
    res!(store.ensure_user(&u));
    for i in 1..=n {
        let bytes = fmt!("Subject: m{}\r\n\r\nid=m{}\r\n", i, i);
        res!(store.append(&u, &FolderName::new("INBOX"), bytes.as_bytes(), MessageFlags::default(), None));
    }
    let lis = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let port = res!(lis.local_addr(), IO, Network).port();
    let srv = ImapServer { store, users: OneUser, hostname: Arc::new(fmt!("test.local")) };
    tokio::spawn(async move {
        while let Ok((sock, peer)) = lis.accept().await {
            let s = srv.clone();
            tokio::spawn(async move { let _ = s.run(sock, peer).await; });
        }
    });
    Ok((port, root))
}

/// The FLAGS a `UID FETCH n (FLAGS)` reply gives UID `uid`, if it names it.
fn flags_of(reply: &str, uid: u32) -> Option<String> {
    for l in reply.lines() {
        if l.contains(&fmt!("UID {} ", uid)) || l.contains(&fmt!("UID {})", uid)) {
            if let Some(i) = l.find("FLAGS (") {
                let rest = &l[i + 7..];
                return rest.find(')').map(|j| rest[..j].to_string());
            }
        }
    }
    None
}

/// Another session expunges UID 1; this one, not yet told, marks its message 2, which is UID 2.
#[tokio::test(flavor = "multi_thread")]
async fn sequence_numbers_hold_until_the_expunge_is_announced() -> Outcome<()> {
    let (port, root) = res!(setup("seq", 3).await);
    let mut a = res!(Sess::open(port).await);
    let mut b = res!(Sess::open(port).await);
    res!(b.cmd("UID STORE 1 +FLAGS (\\Deleted)").await);
    res!(b.cmd("EXPUNGE").await);

    let store = res!(a.cmd("STORE 2 +FLAGS (\\Deleted)").await);
    assert!(!store.contains("EXPUNGE"), "an EXPUNGE was sent during STORE:\n{}", store);
    assert!(store.contains("* 2 FETCH (FLAGS (\\Deleted))"), "STORE 2 answered:\n{}", store);

    let fetch = res!(a.cmd("UID FETCH 1:* (UID FLAGS)").await);
    assert!(fetch.starts_with("* 1 EXPUNGE\r\n"), "the expunge of UID 1 was not announced first:\n{}", fetch);
    assert_eq!(flags_of(&fetch, 2).as_deref(), Some("\\Deleted"), "UID 2 after STORE 2:\n{}", fetch);
    assert_eq!(flags_of(&fetch, 3).as_deref(), Some(""), "UID 3 after STORE 2:\n{}", fetch);
    assert!(fetch.contains("* 1 FETCH (UID 2 "), "UID 2 is not message 1 after the EXPUNGE:\n{}", fetch);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A non-UID SEARCH answers in the numbers the client holds, and NOOP then announces the
/// expunge, highest first, and the delivery.
#[tokio::test(flavor = "multi_thread")]
async fn search_keeps_the_view_and_noop_reconciles_it() -> Outcome<()> {
    let (port, root) = res!(setup("search", 4).await);
    let mut a = res!(Sess::open(port).await);
    let mut b = res!(Sess::open(port).await);
    res!(b.cmd("UID STORE 1,3 +FLAGS (\\Deleted)").await);
    res!(b.cmd("EXPUNGE").await);
    res!(a.cmd("STORE 4 +FLAGS (\\Flagged)").await);

    let search = res!(a.cmd("SEARCH FLAGGED").await);
    assert!(search.contains("* SEARCH 4\r\n"), "SEARCH FLAGGED in the client's numbers:\n{}", search);
    assert!(!search.contains("EXPUNGE"), "an EXPUNGE was sent during SEARCH:\n{}", search);

    let noop = res!(a.cmd("NOOP").await);
    let want = "* 3 EXPUNGE\r\n* 1 EXPUNGE\r\n";
    assert!(noop.starts_with(want), "NOOP did not announce 3 then 1:\n{}", noop);
    assert!(!noop.contains("EXISTS"), "NOOP sent EXISTS with nothing delivered:\n{}", noop);
    let search = res!(a.cmd("SEARCH FLAGGED").await);
    assert!(search.contains("* SEARCH 2\r\n"), "SEARCH FLAGGED after NOOP:\n{}", search);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// CHECK (RFC 3501 §6.4.1) is a checkpoint: it completes, and like NOOP it announces another
/// session's expunge and a delivery.
#[tokio::test(flavor = "multi_thread")]
async fn check_completes_and_reconciles_the_view() -> Outcome<()> {
    let (port, root) = res!(setup("check", 3).await);
    let mut a = res!(Sess::open(port).await);
    let mut b = res!(Sess::open(port).await);
    res!(b.cmd("UID STORE 2 +FLAGS (\\Deleted)").await);
    res!(b.cmd("EXPUNGE").await);
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = res!(res!(OneUser.lookup("")).ok_or_else(|| err!("No test user."; Test, Bug)));
    res!(store.append(&u, &FolderName::new("INBOX"), b"Subject: m4\r\n\r\nid=m4\r\n", MessageFlags::default(), None));

    let check = res!(a.cmd("CHECK").await);
    assert!(check.starts_with("* 2 EXPUNGE\r\n"), "CHECK did not announce the expunge:\n{}", check);
    assert!(check.contains("* 3 EXISTS\r\n"), "CHECK did not announce the delivery:\n{}", check);
    assert!(check.lines().last().map_or(false, |l| l.starts_with("t3 OK")), "CHECK answered:\n{}", check);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// An idling session is told of another's expunge, without being asked.
#[tokio::test(flavor = "multi_thread")]
async fn idle_announces_an_expunge() -> Outcome<()> {
    let (port, root) = res!(setup("idle", 2).await);
    let mut a = res!(Sess::open(port).await);
    let mut b = res!(Sess::open(port).await);
    res!(a.r.get_mut().write_all(b"i1 IDLE\r\n").await, IO, Network);
    let mut l = String::new();
    res!(a.r.read_line(&mut l).await, IO, Network);
    assert!(l.starts_with("+"), "IDLE answered {:?}", l);
    res!(b.cmd("UID STORE 1 +FLAGS (\\Deleted)").await);
    res!(b.cmd("EXPUNGE").await);
    l.clear();
    let got = tokio::time::timeout(Duration::from_secs(15), a.r.read_line(&mut l)).await;
    assert!(got.is_ok(), "no untagged response within 15 s of the expunge");
    assert_eq!(l, "* 1 EXPUNGE\r\n");
    res!(a.r.get_mut().write_all(b"DONE\r\n").await, IO, Network);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A toggles \Seen on UID 1 while B sets \Flagged once per trial; \Flagged must survive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_concurrent_flag_is_never_lost() -> Outcome<()> {
    let (port, root) = res!(setup("flags", 1).await);
    let mut lost = 0;
    let trials = 40;
    for _ in 0..trials {
        let mut a = res!(Sess::open(port).await);
        let mut b = res!(Sess::open(port).await);
        res!(b.cmd("UID STORE 1 -FLAGS (\\Flagged)").await);
        let ta = tokio::spawn(async move {
            for i in 0..30 {
                let op = if i % 2 == 0 { "+" } else { "-" };
                let _ = a.cmd(&fmt!("UID STORE 1 {}FLAGS.SILENT (\\Seen)", op)).await;
            }
            a
        });
        tokio::time::sleep(Duration::from_millis(3)).await;
        res!(b.cmd("UID STORE 1 +FLAGS.SILENT (\\Flagged)").await);
        if ta.await.is_err() { return Err(err!("Session A's task failed."; Test, Bug)); }
        let f = res!(b.cmd("UID FETCH 1 (FLAGS)").await);
        if !f.contains("\\Flagged") { lost += 1; }
    }
    assert_eq!(lost, 0, "\\Flagged was lost in {} of {} trials", lost, trials);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
