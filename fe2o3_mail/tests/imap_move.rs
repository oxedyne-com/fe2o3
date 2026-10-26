//! Steel's IMAP COPY, MOVE and UIDPLUS against a real Maildir over a real socket.
//!
//! A client files mail with `UID MOVE` where the server offers it, and with
//! `UID COPY`, `UID STORE +FLAGS (\Deleted)` and `UID EXPUNGE` where it offers
//! UIDPLUS, and learns the new UIDs from `COPYUID` and `APPENDUID`. Plain
//! EXPUNGE is no substitute for UID EXPUNGE: it removes every `\Deleted`
//! message, not just the ones being filed.

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
            MessageUid,
        },
        user::UserStore,
    },
};

use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::PathBuf,
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


const USER: &str = "alice@test.local";
const PASS: &str = "correct-horse";

#[derive(Clone, Debug)]
struct OneUser;

impl UserStore for OneUser {
    fn authenticate(&self, address: &str, password: &str) -> Outcome<Option<MailUser>> {
        if address.eq_ignore_ascii_case(USER) && password == PASS {
            return self.lookup(address);
        }
        Ok(None)
    }

    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>> {
        if !address.eq_ignore_ascii_case(USER) { return Ok(None); }
        Ok(Some(user()))
    }
}

fn user() -> MailUser {
    MailUser {
        local:          fmt!("alice"),
        domain:         fmt!("test.local"),
        delivery_key:   fmt!("test.local/alice"),
        send_as:        Vec::new(),
    }
}

fn body(tag: &str) -> Vec<u8> {
    fmt!("From: bob@example.org\r\nTo: alice@test.local\r\nSubject: {}\r\n\r\nid={}\r\n",
        tag, tag).into_bytes()
}

/// The `id=` tag a message carries, which names it independently of its UID.
fn tag_of(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    match s.find("id=") {
        Some(i) => s[i + 3..].trim().to_string(),
        None    => String::new(),
    }
}

/// UID → tag for every message in the folder now.
fn uid_map(store: &MaildirStore, f: &str) -> Outcome<BTreeMap<u32, String>> {
    let (u, f) = (user(), FolderName::new(f));
    let msgs = res!(store.list_messages(&u, &f, true));
    let mut out = BTreeMap::new();
    for m in &msgs {
        let bytes = res!(res!(store.fetch_bytes(&u, &f, m.uid))
            .ok_or_else(|| err!("UID {} listed but not fetched.", m.uid.0; Missing)));
        out.insert(m.uid.0, tag_of(&bytes));
    }
    Ok(out)
}

struct Conn {
    r:      BufReader<tokio::net::tcp::OwnedReadHalf>,
    w:      tokio::net::tcp::OwnedWriteHalf,
    n:      usize,
}

impl Conn {
    async fn open(addr: SocketAddr, folder: &str) -> Outcome<Self> {
        let sock = res!(TcpStream::connect(addr).await, IO, Network);
        let (r, w) = sock.into_split();
        let mut c = Conn { r: BufReader::new(r), w, n: 0 };
        let mut greeting = String::new();
        res!(c.r.read_line(&mut greeting).await, IO, Network);
        res!(c.cmd(&fmt!("LOGIN {} {}", USER, PASS)).await);
        res!(c.cmd(&fmt!("SELECT {}", folder)).await);
        Ok(c)
    }

    async fn send(&mut self, bytes: &[u8]) -> Outcome<()> {
        res!(self.w.write_all(bytes).await, IO, Network);
        Ok(())
    }

    async fn cmd(&mut self, text: &str) -> Outcome<Vec<String>> {
        self.n += 1;
        let tag = fmt!("t{}", self.n);
        res!(self.send(fmt!("{} {}\r\n", tag, text).as_bytes()).await);
        self.until(&tag).await
    }

    async fn until(&mut self, tag: &str) -> Outcome<Vec<String>> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let n = res!(self.r.read_line(&mut line).await, IO, Network);
            if n == 0 { return Err(err!("The server closed the connection."; IO, Network)); }
            let done = line.starts_with(&fmt!("{} ", tag));
            lines.push(line.trim_end().to_string());
            if done { return Ok(lines); }
        }
    }

    async fn status(&mut self, folder: &str, item: &str) -> Outcome<u32> {
        let lines = res!(self.cmd(&fmt!("STATUS {} ({})", folder, item)).await);
        for l in &lines {
            if let Some(i) = l.find(&fmt!("{} ", item)) {
                let rest = &l[i + item.len() + 1..];
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                return Ok(res!(digits.parse::<u32>(), Decode));
            }
        }
        Err(err!("No {} in the STATUS answer {:?}.", item, lines; Test, Missing))
    }
}

/// The text of the response code `[NAME ...]` on any of the lines.
fn code(lines: &[String], name: &str) -> Option<String> {
    let open = fmt!("[{} ", name);
    for l in lines {
        if let Some(i) = l.find(&open) {
            let rest = &l[i + open.len()..];
            if let Some(j) = rest.find(']') {
                return Some(rest[..j].to_string());
            }
        }
    }
    None
}

fn last(lines: &[String]) -> String {
    lines.last().cloned().unwrap_or_default()
}

/// A uid-set expanded: `3:5,9` is 3, 4, 5, 9.
fn expand(set: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for piece in set.split(',') {
        let (lo, hi) = match piece.split_once(':') {
            Some((a, b)) => (a.parse::<u32>().unwrap_or(0), b.parse::<u32>().unwrap_or(0)),
            None => {
                let n = piece.parse::<u32>().unwrap_or(0);
                (n, n)
            },
        };
        for n in lo.min(hi)..=lo.max(hi) { out.push(n); }
    }
    out
}

/// A Maildir with `n` messages tagged `<prefix>0..` in INBOX, served over a socket.
async fn serve(name: &str, prefix: &str, n: usize) -> Outcome<(SocketAddr, MaildirStore, Vec<u32>, PathBuf)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_move_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    let mut uids = Vec::new();
    for i in 0..n {
        let MessageUid(uid) = res!(store.append(&u, &inbox, &body(&fmt!("{}{}", prefix, i)),
            MessageFlags::default(), None));
        uids.push(uid);
    }
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let addr = res!(listener.local_addr(), IO, Network);
    let server = ImapServer {
        store:      store.clone(),
        users:      OneUser,
        hostname:   std::sync::Arc::new(fmt!("test.local")),
    };
    tokio::spawn(async move {
        while let Ok((sock, peer)) = listener.accept().await {
            let srv = server.clone();
            tokio::spawn(async move { let _ = srv.run(sock, peer).await; });
        }
    });
    Ok((addr, store, uids, root))
}

#[tokio::test]
async fn capability_names_move_and_uidplus() -> Outcome<()> {
    let (addr, _store, _uids, root) = res!(serve("cap", "c", 0).await);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    let lines = res!(c.cmd("CAPABILITY").await);
    let caps = lines.iter().find(|l| l.starts_with("* CAPABILITY")).cloned().unwrap_or_default();
    let words: Vec<&str> = caps.split_whitespace().collect();
    assert!(words.contains(&"MOVE") && words.contains(&"UIDPLUS"),
        "CAPABILITY lacks MOVE or UIDPLUS: {}", caps);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn copy_files_a_copy_and_answers_copyuid() -> Outcome<()> {
    let (addr, store, uids, root) = res!(serve("copy", "k", 3).await);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    let validity = res!(c.status("Archive", "UIDVALIDITY").await);
    let next = res!(c.status("Archive", "UIDNEXT").await);

    let lines = res!(c.cmd(&fmt!("UID COPY {},{} Archive", uids[0], uids[2])).await);
    assert!(last(&lines).contains(" OK "), "UID COPY: {:?}", lines);
    let cu = match code(&lines, "COPYUID") {
        Some(cu) => cu,
        None => return Err(err!("UID COPY gave no COPYUID: {:?}", lines; Test, Missing)),
    };
    let parts: Vec<&str> = cu.split_whitespace().collect();
    assert_eq!(parts.len(), 3, "COPYUID {:?}", cu);
    assert_eq!(parts[0], fmt!("{}", validity), "COPYUID's UIDVALIDITY is not the target's");
    assert_eq!(expand(parts[1]), vec![uids[0], uids[2]]);
    let new = expand(parts[2]);
    assert_eq!(new, vec![next, next + 1], "COPYUID's new UIDs");
    let archive = res!(uid_map(&store, "Archive"));
    assert_eq!(archive.get(&next).map(|s| s.as_str()), Some("k0"));
    assert_eq!(archive.get(&(next + 1)).map(|s| s.as_str()), Some("k2"));
    assert_eq!(res!(uid_map(&store, "INBOX")).len(), 3, "COPY took messages from the source");

    // By sequence number, as plain COPY names them.
    let lines = res!(c.cmd("COPY 2 Archive").await);
    assert!(last(&lines).contains(" OK ") && code(&lines, "COPYUID").is_some(), "COPY: {:?}", lines);
    assert_eq!(res!(uid_map(&store, "Archive")).values().filter(|t| *t == "k1").count(), 1);

    let lines = res!(c.cmd("UID COPY 1:* Nowhere").await);
    assert!(last(&lines).contains(" NO [TRYCREATE]"), "COPY to a missing folder: {:?}", lines);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn move_answers_copyuid_and_expunge() -> Outcome<()> {
    let (addr, store, uids, root) = res!(serve("move", "m", 3).await);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    let next = res!(c.status("Archive", "UIDNEXT").await);

    let lines = res!(c.cmd(&fmt!("UID MOVE {} Archive", uids[1])).await);
    assert!(last(&lines).contains(" OK "), "UID MOVE: {:?}", lines);
    let cu = code(&lines, "COPYUID").unwrap_or_default();
    assert_eq!(cu.split_whitespace().nth(2), Some(fmt!("{}", next).as_str()), "COPYUID {:?}", lines);
    assert!(lines.iter().any(|l| l == "* 2 EXPUNGE"), "UID MOVE sent no EXPUNGE: {:?}", lines);
    assert!(lines.iter().all(|l| !l.starts_with("* OK [COPYUID") || !l.contains("EXPUNGE")));
    let inbox = res!(uid_map(&store, "INBOX"));
    let archive = res!(uid_map(&store, "Archive"));
    assert_eq!(inbox.values().cloned().collect::<Vec<_>>(), vec![fmt!("m0"), fmt!("m2")]);
    assert_eq!(archive.get(&next).map(|s| s.as_str()), Some("m1"));

    // By sequence number: message 2 is now m2.
    let lines = res!(c.cmd("MOVE 2 Archive").await);
    assert!(last(&lines).contains(" OK ") && lines.iter().any(|l| l == "* 2 EXPUNGE"),
        "MOVE: {:?}", lines);
    assert_eq!(res!(uid_map(&store, "INBOX")).len(), 1);
    assert_eq!(res!(uid_map(&store, "Archive")).len(), 2);

    // EXAMINE is read-only: nothing may leave the folder.
    res!(c.cmd("EXAMINE INBOX").await);
    let lines = res!(c.cmd("UID MOVE 1:* Archive").await);
    assert!(last(&lines).contains(" NO "), "MOVE out of a read-only selection: {:?}", lines);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn uid_expunge_removes_only_the_messages_it_names() -> Outcome<()> {
    let (addr, store, uids, root) = res!(serve("uidexp", "e", 3).await);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    res!(c.cmd(&fmt!("UID STORE {},{} +FLAGS.SILENT (\\Deleted)", uids[0], uids[2])).await);
    let lines = res!(c.cmd(&fmt!("UID EXPUNGE {}", uids[2])).await);
    assert!(last(&lines).contains(" OK "), "UID EXPUNGE: {:?}", lines);
    assert!(lines.iter().any(|l| l == "* 3 EXPUNGE"), "UID EXPUNGE: {:?}", lines);
    let inbox = res!(uid_map(&store, "INBOX"));
    assert_eq!(inbox.keys().cloned().collect::<Vec<_>>(), vec![uids[0], uids[1]],
        "UID EXPUNGE removed a message it did not name");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn append_answers_appenduid() -> Outcome<()> {
    let (addr, store, _uids, root) = res!(serve("appenduid", "a", 2).await);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    let validity = res!(c.status("Archive", "UIDVALIDITY").await);
    let next = res!(c.status("Archive", "UIDNEXT").await);
    let msg = body("appended");
    c.n += 1;
    let tag = fmt!("t{}", c.n);
    res!(c.send(fmt!("{} APPEND Archive (\\Seen) {{{}+}}\r\n", tag, msg.len()).as_bytes()).await);
    res!(c.send(&msg).await);
    res!(c.send(b"\r\n").await);
    let lines = res!(c.until(&tag).await);
    assert!(last(&lines).contains(" OK "), "APPEND: {:?}", lines);
    assert_eq!(code(&lines, "APPENDUID"), Some(fmt!("{} {}", validity, next)), "APPEND: {:?}", lines);
    assert_eq!(res!(uid_map(&store, "Archive")).get(&next).map(|s| s.as_str()), Some("appended"));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// Two sessions move all they hold the other way at once, over and over: every
/// message is in exactly one folder at the end, and neither waits for ever on the
/// other's folder lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opposite_moves_neither_deadlock_nor_lose_mail() -> Outcome<()> {
    const N: usize = 30;
    let (addr, store, _uids, root) = res!(serve("opposite", "o", N).await);
    let u = user();
    let archive = FolderName::new("Archive");
    for i in 0..N {
        res!(store.append(&u, &archive, &body(&fmt!("p{}", i)), MessageFlags::default(), None));
    }
    let mut tasks = Vec::new();
    for (from, to) in [("INBOX", "Archive"), ("Archive", "INBOX")] {
        tasks.push(tokio::spawn(async move {
            let mut c = res!(Conn::open(addr, from).await);
            for _ in 0..8 {
                res!(c.cmd("NOOP").await);
                let lines = res!(c.cmd(&fmt!("UID MOVE 1:* {}", to)).await);
                if !last(&lines).contains(" OK ") {
                    return Err(err!("UID MOVE {} -> {}: {:?}", from, to, lines; Test, Unexpected));
                }
            }
            Ok::<(), Error<ErrTag>>(())
        }));
    }
    for t in tasks {
        match tokio::time::timeout(Duration::from_secs(60), t).await {
            Ok(Ok(outcome)) => res!(outcome),
            Ok(Err(e))      => return Err(err!(e, "A mover panicked."; Test, Thread)),
            Err(_)          => return Err(err!("Opposite moves did not finish in 60 s."; Test, Timeout)),
        }
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for f in ["INBOX", "Archive"] {
        for t in res!(uid_map(&store, f)).values() {
            *seen.entry(t.clone()).or_insert(0) += 1;
        }
    }
    assert_eq!(seen.len(), 2 * N, "messages lost: {:?}", seen);
    assert!(seen.values().all(|n| *n == 1), "a message in both folders: {:?}", seen);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The file under `dir` whose message carries the tag, at any depth.
fn file_of(dir: &std::path::Path, tag: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = file_of(&p, tag) { return Some(f); }
        } else if fs::read(&p).map(|b| tag_of(&b) == tag).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

#[tokio::test]
async fn a_stale_copy_or_move_sends_no_empty_copyuid() -> Outcome<()> {
    // QA F2b-A defect 1: B moves a message A's view still holds; A's COPY and MOVE of it
    // must not answer [COPYUID v  ], and A learns in that response that it left.
    let (addr, _store, uids, root) = res!(serve("stale", "s", 3).await);
    let mut a = res!(Conn::open(addr, "INBOX").await);
    let mut b = res!(Conn::open(addr, "INBOX").await);
    let lines = res!(b.cmd(&fmt!("UID MOVE {} Archive", uids[1])).await);
    assert!(last(&lines).contains(" OK "), "B's MOVE: {:?}", lines);

    let lines = res!(a.cmd(&fmt!("UID COPY {} Archive", uids[1])).await);
    assert!(code(&lines, "COPYUID").is_none(), "a stale COPY answered COPYUID: {:?}", lines);
    assert!(last(&lines).contains(" NO [EXPUNGEISSUED]"), "a stale COPY: {:?}", lines);
    assert!(lines.iter().any(|l| l == "* 2 EXPUNGE"), "a stale COPY announced nothing: {:?}", lines);

    // Again, with the message gone from a fresh view and one more stale: B moves s2 too.
    let lines = res!(b.cmd(&fmt!("UID MOVE {} Archive", uids[2])).await);
    assert!(last(&lines).contains(" OK "), "B's second MOVE: {:?}", lines);
    let lines = res!(a.cmd(&fmt!("UID MOVE {},{} Archive", uids[0], uids[2])).await);
    let cu = code(&lines, "COPYUID").unwrap_or_default();
    let parts: Vec<&str> = cu.split_whitespace().collect();
    assert_eq!(parts.len(), 3, "a partly stale MOVE's COPYUID: {:?}", lines);
    assert_eq!(expand(parts[1]), vec![uids[0]]);
    assert!(last(&lines).contains(" OK "), "a partly stale MOVE: {:?}", lines);
    assert!(lines.iter().filter(|l| l.ends_with(" EXPUNGE")).count() == 2,
        "the moved and the vanished message both announced: {:?}", lines);

    // Announced, s2 has left A's view, so a UID MOVE of it now names no message at all.
    let lines = res!(a.cmd(&fmt!("UID MOVE {} Archive", uids[2])).await);
    assert!(code(&lines, "COPYUID").is_none() && last(&lines).contains(" OK "),
        "a MOVE of a UID already announced gone: {:?}", lines);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn a_failed_copy_leaves_the_target_as_it_was() -> Outcome<()> {
    // QA F2b-A defect 2, RFC 3501 §6.4.7: the second message cannot be read, so the COPY
    // fails, and the copy of the first must not stay to be filed again on the retry.
    let (addr, store, uids, root) = res!(serve("copyfail", "f", 3).await);
    let path = res!(file_of(&root, "f1").ok_or_else(|| err!("No file for f1."; Test, Missing)));
    res!(fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o000)), IO, File);
    if fs::read(&path).is_ok() {
        // Run as root: the file stays readable, and the failure cannot be made.
        let _ = fs::remove_dir_all(&root);
        return Ok(());
    }
    let mut c = res!(Conn::open(addr, "INBOX").await);
    let lines = res!(c.cmd(&fmt!("UID COPY {}:{} Archive", uids[0], uids[2])).await);
    assert!(last(&lines).contains(" NO "), "a failing COPY: {:?}", lines);
    assert!(res!(uid_map(&store, "Archive")).is_empty(), "a failed COPY left copies: {:?}",
        res!(uid_map(&store, "Archive")));

    res!(fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)), IO, File);
    let lines = res!(c.cmd(&fmt!("UID COPY {}:{} Archive", uids[0], uids[2])).await);
    assert!(last(&lines).contains(" OK "), "the retried COPY: {:?}", lines);
    let tags: Vec<String> = res!(uid_map(&store, "Archive")).into_values().collect();
    assert_eq!(tags, vec![fmt!("f0"), fmt!("f1"), fmt!("f2")], "the retry filed one of each");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn a_folder_read_keeps_a_tmp_file_with_an_old_mtime() -> Outcome<()> {
    // The tmp/ sweep goes by creation time: an APPEND writing outside the lock sets its file's
    // mtime to the INTERNALDATE, which may be years old, and must not lose it to a reader.
    let (addr, _store, _uids, root) = res!(serve("tmpkeep", "t", 1).await);
    let inbox = res!(file_of(&root, "t0").ok_or_else(|| err!("No file for t0."; Test, Missing)));
    let tmp = res!(inbox.parent().and_then(|d| d.parent()).map(|d| d.join("tmp"))
        .ok_or_else(|| err!("No folder above {:?}.", inbox; Test, Missing)));
    let held = tmp.join("in-progress");
    res!(fs::write(&held, b"partial"), IO, File);
    let f = res!(fs::File::options().write(true).open(&held), IO, File);
    let old = std::time::SystemTime::now() - Duration::from_secs(3 * 365 * 86400);
    res!(f.set_modified(old), IO, File);
    let mut c = res!(Conn::open(addr, "INBOX").await);
    res!(c.cmd("NOOP").await);
    assert!(held.exists(), "a folder read swept a tmp/ file made a moment ago");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
