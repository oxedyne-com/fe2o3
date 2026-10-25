//! The Maildir store's MOVE across a crash.
//!
//! MOVE is one rename(2) of the message file followed by both folders' uidlist
//! writes. A crash between the two must leave the message in exactly one folder,
//! and the folder it reached must number it above every UID it has issued, even
//! when the message once lived there: a reused UID would make a client that
//! remembers the old one act on the wrong message.

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
};


fn user() -> MailUser {
    MailUser {
        local:          fmt!("alice"),
        domain:         fmt!("test.local"),
        delivery_key:   fmt!("test.local/alice"),
        send_as:        Vec::new(),
    }
}

fn body(tag: &str) -> Vec<u8> {
    fmt!("From: bob@example.org\r\nSubject: {}\r\n\r\nid={}\r\n", tag, tag).into_bytes()
}

fn tag_of(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    match s.find("id=") {
        Some(i) => s[i + 3..].trim().to_string(),
        None    => String::new(),
    }
}

fn uid_map(store: &MaildirStore, f: &FolderName) -> Outcome<BTreeMap<u32, String>> {
    let u = user();
    let mut out = BTreeMap::new();
    for m in res!(store.list_messages(&u, f, true)) {
        let bytes = res!(res!(store.fetch_bytes(&u, f, m.uid))
            .ok_or_else(|| err!("UID {} listed but not fetched.", m.uid.0; Missing)));
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

/// Moves `uid` from `from` to `to` with a crash between the rename and the uidlist writes:
/// both folder directories, where the uidlists are rewritten, refuse the writes, while the
/// `cur/` directories the rename uses do not.
fn crashed_move(store: &MaildirStore, dirs: [&Path; 2], from: &FolderName, uid: u32, to: &FolderName)
    -> Outcome<()>
{
    for d in dirs { res!(set_mode(d, 0o555)); }
    let outcome = store.move_to(&user(), from, &[MessageUid(uid)], to);
    for d in dirs { res!(set_mode(d, 0o755)); }
    assert!(outcome.is_err(), "the injected fault did not reach the move: {:?}", outcome);
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_crash_mid_move_leaves_one_copy_under_a_fresh_uid() -> Outcome<()> {
    if is_root() { return Ok(()); }
    let root = std::env::temp_dir().join(fmt!("fe2o3_maildir_move_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let u = user();
    res!(store.ensure_user(&u));
    let (inbox, archive) = (FolderName::new("INBOX"), FolderName::new("Archive"));
    let inbox_dir = root.join("test.local").join("alice");
    let archive_dir = inbox_dir.join(".Archive");
    let mut uids = Vec::new();
    for i in 0..3 {
        let MessageUid(uid) = res!(store.append(&u, &inbox, &body(&fmt!("i{}", i)), MessageFlags::default(), None));
        uids.push(uid);
    }
    res!(store.append(&u, &archive, &body("a0"), MessageFlags::default(), None));
    let inbox_next = res!(store.folder_status(&u, &inbox)).uid_next;
    let archive_next = res!(store.folder_status(&u, &archive)).uid_next;

    // INBOX -> Archive, crashing before either uidlist is written. The INBOX uidlist is
    // left naming the message, and is not read again before it comes back.
    res!(crashed_move(&store, [&inbox_dir, &archive_dir], &inbox, uids[1], &archive));
    let after = res!(uid_map(&store, &archive));
    let moved: Vec<u32> = after.iter().filter(|(_, t)| *t == "i1").map(|(u, _)| *u).collect();
    assert_eq!(moved.len(), 1, "the message is not in Archive once: {:?}", after);
    assert!(moved[0] >= archive_next, "Archive reused UID {} (UIDNEXT was {})", moved[0], archive_next);

    // And back, crashing again: INBOX's stale uidlist must not hand it its old UID.
    res!(crashed_move(&store, [&archive_dir, &inbox_dir], &archive, moved[0], &inbox));
    let back = res!(uid_map(&store, &inbox));
    let returned: Vec<u32> = back.iter().filter(|(_, t)| *t == "i1").map(|(u, _)| *u).collect();
    assert_eq!(returned.len(), 1, "the message is not in INBOX once: {:?}", back);
    assert!(returned[0] >= inbox_next,
        "INBOX gave the returning message UID {}, not above every UID it had issued (UIDNEXT {})",
        returned[0], inbox_next);
    assert!(res!(uid_map(&store, &archive)).values().all(|t| t != "i1"), "left in Archive too");
    assert_eq!(back.len(), 3);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
