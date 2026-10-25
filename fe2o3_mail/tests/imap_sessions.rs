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
