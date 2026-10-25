//! The Maildir store's UID bookkeeping under concurrency and damage.
//!
//! Steel's IMAP sessions and its SMTP delivery all read, change and write one
//! `dovecot-uidlist` per folder. A UID that is renumbered or swapped without a
//! change of UIDVALIDITY makes every UID-keyed client act on the wrong message,
//! so these tests hold the store to RFC 3501 §2.3.1.1: a UID names one message
//! for as long as the UIDVALIDITY lasts, and UIDNEXT never moves backwards.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::mail::store::{
    FolderName,
    MailStore,
    MailUser,
    MessageFlags,
    MessageUid,
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
    sync::{
        Arc,
        Mutex,
    },
    thread,
};


const CHILD_ENV:    &str    = "FE2O3_MAILDIR_CHILD_ROOT";  // set only in a spawned writer process
const CHILD_TAG:    &str    = "FE2O3_MAILDIR_CHILD_TAG";
const WRITERS:      usize   = 4;
const PER_WRITER:   usize   = 20;
const READERS:      usize   = 3;
const CHILD_PROCS:  usize   = 2;

fn user() -> MailUser {
    MailUser {
        local:          fmt!("alice"),
        domain:         fmt!("test.local"),
        delivery_key:   fmt!("test.local/alice"),
        send_as:        Vec::new(),
    }
}

fn fresh_root(name: &str) -> Outcome<PathBuf> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_maildir_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    Ok(root)
}

fn inbox_dir(root: &Path) -> PathBuf {
    root.join("test.local").join("alice")
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
fn uid_map(store: &MaildirStore, u: &MailUser, f: &FolderName) -> Outcome<BTreeMap<u32, String>> {
    let msgs = res!(store.list_messages(u, f, true));
    let mut out = BTreeMap::new();
    for m in &msgs {
        let bytes = res!(store.fetch_bytes(u, f, m.uid));
        out.insert(m.uid.0, tag_of(&bytes));
    }
    Ok(out)
}

fn is_root() -> bool {
    match fs::read_to_string("/proc/self/status") {
        Ok(s) => s.lines().any(|l| l.starts_with("Uid:")
            && l.split_whitespace().nth(2) == Some("0")),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) -> Outcome<()> {
    use std::os::unix::fs::PermissionsExt;
    res!(fs::set_permissions(p, fs::Permissions::from_mode(mode)), IO, File);
    Ok(())
}

/// Not a test of its own: the body of a writer process spawned by
/// `concurrent_appends_and_reads_agree_on_every_uid`, a no-op otherwise.
#[test]
fn child_appender() -> Outcome<()> {
    let root = match std::env::var(CHILD_ENV) {
        Ok(r)   => PathBuf::from(r),
        Err(_)  => return Ok(()),
    };
    let tag = res!(std::env::var(CHILD_TAG), Missing);
    let store = res!(MaildirStore::new(root, "test.local"));
    let u = user();
    let inbox = FolderName::new("INBOX");
    for i in 0..PER_WRITER {
        res!(store.append(&u, &inbox, &body(&fmt!("{}-{}", tag, i)), MessageFlags::default(), None));
    }
    Ok(())
}

#[test]
fn concurrent_appends_and_reads_agree_on_every_uid() -> Outcome<()> {
    let root = res!(fresh_root("concurrent"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");

    // Writer processes, so the lock is proved across processes too.
    let exe = res!(std::env::current_exe(), IO, File);
    let mut children = Vec::new();
    for p in 0..CHILD_PROCS {
        let child = res!(Command::new(&exe)
            .args(["--exact", "child_appender", "--test-threads=1", "--quiet"])
            .env(CHILD_ENV, &root)
            .env(CHILD_TAG, fmt!("p{}", p))
            .spawn(), IO);
        children.push(child);
    }

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen: Arc<Mutex<Vec<BTreeMap<u32, String>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for t in 0..WRITERS {
        let (store, u, inbox, errors) = (store.clone(), u.clone(), inbox.clone(), errors.clone());
        handles.push(thread::spawn(move || {
            for i in 0..PER_WRITER {
                if let Err(e) = store.append(&u, &inbox, &body(&fmt!("t{}-{}", t, i)),
                    MessageFlags::default(), None)
                {
                    if let Ok(mut v) = errors.lock() { v.push(fmt!("append: {}", e)); }
                }
            }
        }));
    }
    for _ in 0..READERS {
        let (store, u, inbox, errors, seen) =
            (store.clone(), u.clone(), inbox.clone(), errors.clone(), seen.clone());
        handles.push(thread::spawn(move || {
            let mut last_next = 0u32;
            let mut last_validity = None;
            for _ in 0..PER_WRITER {
                match store.folder_status(&u, &inbox) {
                    Ok(st) => {
                        if st.uid_next < last_next {
                            if let Ok(mut v) = errors.lock() {
                                v.push(fmt!("UIDNEXT fell from {} to {}", last_next, st.uid_next));
                            }
                        }
                        last_next = st.uid_next;
                        if let Some(old) = last_validity {
                            if old != st.uid_validity {
                                if let Ok(mut v) = errors.lock() {
                                    v.push(fmt!("UIDVALIDITY moved {} -> {}", old, st.uid_validity));
                                }
                            }
                        }
                        last_validity = Some(st.uid_validity);
                    },
                    Err(e) => if let Ok(mut v) = errors.lock() { v.push(fmt!("status: {}", e)); },
                }
                match uid_map(&store, &u, &inbox) {
                    Ok(m)  => if let Ok(mut v) = seen.lock() { v.push(m); },
                    Err(e) => if let Ok(mut v) = errors.lock() { v.push(fmt!("read: {}", e)); },
                }
            }
        }));
    }
    for h in handles {
        if h.join().is_err() {
            return Err(err!("A test thread panicked."; Test, Bug));
        }
    }
    for mut c in children {
        let st = res!(c.wait(), IO);
        assert!(st.success(), "a writer process failed: {:?}", st);
    }

    let errors = res!(errors.lock().map_err(|_| err!("poisoned"; Lock))).clone();
    assert!(errors.is_empty(), "errors under concurrency: {:?}", errors);

    let fin = res!(uid_map(&store, &u, &inbox));
    let total = (WRITERS + CHILD_PROCS) * PER_WRITER;
    assert_eq!(fin.len(), total, "the folder holds {} of {} messages", fin.len(), total);
    let mut by_tag: BTreeMap<String, u32> = BTreeMap::new();
    for (uid, tag) in &fin {
        if let Some(other) = by_tag.insert(tag.clone(), *uid) {
            panic!("message {} holds two UIDs, {} and {}", tag, other, uid);
        }
    }
    // Every reader's view of every UID is the final one.
    let seen = res!(seen.lock().map_err(|_| err!("poisoned"; Lock))).clone();
    for snap in &seen {
        for (uid, tag) in snap {
            assert_eq!(fin.get(uid), Some(tag),
                "a reader saw UID {} as {}, but it is now {:?}", uid, tag, fin.get(uid));
        }
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn append_returns_the_uid_of_its_own_message() -> Outcome<()> {
    let root = res!(fresh_root("append_uid"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    for i in 0..3 {
        let tag = fmt!("m{}", i);
        let uid = res!(store.append(&u, &inbox, &body(&tag), MessageFlags::default(), None));
        let got = res!(store.fetch_bytes(&u, &inbox, uid));
        assert_eq!(tag_of(&got), tag, "append returned UID {} which is not its own message", uid.0);
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

fn garbled_uidlist_changes_uidvalidity(damage: &[u8]) -> Outcome<()> {
    let root = res!(fresh_root(&fmt!("garbled_{}", damage.len())));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    for i in 0..3 {
        res!(store.append(&u, &inbox, &body(&fmt!("g{}", i)), MessageFlags::default(), None));
    }
    let before = res!(store.folder_status(&u, &inbox));
    let map_before = res!(uid_map(&store, &u, &inbox));

    res!(fs::write(inbox_dir(&root).join("dovecot-uidlist"), damage), IO, File);
    res!(store.append(&u, &inbox, &body("g3"), MessageFlags::default(), None));

    let after = res!(store.folder_status(&u, &inbox));
    let map_after = res!(uid_map(&store, &u, &inbox));
    if after.uid_validity == before.uid_validity {
        // Under the old validity, no UID may name another message.
        for (uid, tag) in &map_after {
            if let Some(old) = map_before.get(uid) {
                assert_eq!(old, tag,
                    "UID {} was {} and is now {} under the same UIDVALIDITY {}",
                    uid, old, tag, after.uid_validity);
            }
        }
        panic!("a damaged uidlist {:?} left UIDVALIDITY at {} (before {:?}, after {:?})",
            String::from_utf8_lossy(damage), after.uid_validity, map_before, map_after);
    }
    assert!(after.uid_validity > before.uid_validity,
        "UIDVALIDITY went from {} to {}, not upwards", before.uid_validity, after.uid_validity);
    assert_eq!(map_after.len(), 4);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn a_truncated_uidlist_changes_uidvalidity() -> Outcome<()> {
    garbled_uidlist_changes_uidvalidity(b"")
}

#[test]
fn a_garbled_uidlist_changes_uidvalidity() -> Outcome<()> {
    garbled_uidlist_changes_uidvalidity(b"3 V0 N\x00\x00\nnot a line\n")
}

#[test]
fn a_missing_uidlist_changes_uidvalidity() -> Outcome<()> {
    let root = res!(fresh_root("missing"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    for i in 0..2 {
        res!(store.append(&u, &inbox, &body(&fmt!("x{}", i)), MessageFlags::default(), None));
    }
    let before = res!(store.folder_status(&u, &inbox));
    res!(fs::remove_file(inbox_dir(&root).join("dovecot-uidlist")), IO, File);
    let after = res!(store.folder_status(&u, &inbox));
    assert!(after.uid_validity > before.uid_validity,
        "a lost uidlist kept UIDVALIDITY {} -> {}", before.uid_validity, after.uid_validity);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The `dovecot-uidvalidity.<hex>` markers in a folder.
fn markers(dir: &Path) -> Outcome<Vec<String>> {
    let mut out = Vec::new();
    for e in res!(fs::read_dir(dir), IO, File) {
        let name = res!(e, IO, File).file_name().to_string_lossy().into_owned();
        if name.starts_with("dovecot-uidvalidity.") { out.push(name); }
    }
    out.sort();
    Ok(out)
}

/// The Dovecot layout, one marker per folder: the pre-F2a binary reads the first marker
/// `read_dir` returns, so an older one left beside the new would take UIDVALIDITY backwards.
/// Removing the uidlist with Steel stopped is also the deploy's one-shot bump, so the
/// messages must come back numbered afresh in delivery order.
#[test]
fn a_uidvalidity_bump_leaves_one_marker_and_numbers_afresh() -> Outcome<()> {
    let root = res!(fresh_root("one_marker"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    for i in 0..3 {
        res!(store.append(&u, &inbox, &body(&fmt!("b{}", i)), MessageFlags::default(), None));
    }
    let dir = inbox_dir(&root);
    let mut last = res!(store.folder_status(&u, &inbox)).uid_validity;
    for round in 0..2 {
        res!(fs::remove_file(dir.join("dovecot-uidlist")), IO, File);
        let st = res!(store.folder_status(&u, &inbox));
        assert!(st.uid_validity > last, "round {}: UIDVALIDITY {} -> {}", round, last, st.uid_validity);
        last = st.uid_validity;
        let want = vec![fmt!("dovecot-uidvalidity.{:08x}", st.uid_validity)];
        assert_eq!(res!(markers(&dir)), want, "round {}: markers after a bump", round);
        let map = res!(uid_map(&store, &u, &inbox));
        let got: Vec<(u32, String)> = map.into_iter().collect();
        assert_eq!(got, vec![(1, fmt!("b0")), (2, fmt!("b1")), (3, fmt!("b2"))],
            "round {}: UIDs after a bump", round);
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_failed_uidlist_write_is_an_error() -> Outcome<()> {
    if is_root() { return Ok(()); }
    let root = res!(fresh_root("write_fail"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    res!(store.append(&u, &inbox, &body("w0"), MessageFlags::default(), None));
    res!(store.folder_status(&u, &inbox));

    // A message arrives that the uidlist does not yet name, and the folder
    // directory, where the uidlist is rewritten, refuses the write.
    let dir = inbox_dir(&root);
    res!(fs::write(dir.join("cur").join("1.M1P1.elsewhere:2,"), body("w1")), IO, File);
    res!(set_mode(&dir, 0o555));
    let outcome = store.list_messages(&u, &inbox, true);
    res!(set_mode(&dir, 0o755));
    assert!(outcome.is_err(), "a uidlist that could not be written was reported as fine: {:?}",
        outcome.map(|v| v.len()));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[cfg(unix)]
#[test]
fn expunge_reports_only_what_it_removed() -> Outcome<()> {
    if is_root() { return Ok(()); }
    let root = res!(fresh_root("expunge"));
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let inbox = FolderName::new("INBOX");
    let mut uids: Vec<MessageUid> = Vec::new();
    for i in 0..2 {
        uids.push(res!(store.append(&u, &inbox, &body(&fmt!("e{}", i)), MessageFlags::default(), None)));
    }
    let del = MessageFlags { deleted: true, ..MessageFlags::default() };
    for uid in &uids {
        res!(store.set_flags(&u, &inbox, *uid, del));
    }
    let cur = inbox_dir(&root).join("cur");
    res!(set_mode(&cur, 0o555));
    let outcome = store.expunge(&u, &inbox);
    res!(set_mode(&cur, 0o755));
    let removed = res!(outcome);
    assert!(removed.is_empty(), "expunge reported {:?}, but no file could be removed", removed);
    assert_eq!(res!(store.list_messages(&u, &inbox, true)).len(), 2);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
