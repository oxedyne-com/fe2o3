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
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::{
        Duration,
        Instant,
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

/// MOVE-heavy stress: two threads move everything from INBOX to Archive, two move it back,
/// and one delivers, all at once. No (folder, UID) may name two messages, no UIDVALIDITY may
/// move, nothing may deadlock, and at the end every message is in exactly one folder.
#[test]
fn opposite_moves_and_deliveries_keep_every_message_once() -> Outcome<()> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_maildir_move_stress_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(root.clone(), "argonaut"));
    let u = user();
    let (inbox, arch) = (FolderName::new("INBOX"), FolderName::new("Archive"));
    res!(store.ensure_user(&u));
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for i in 0..20 {
        let t = fmt!("seed{}", i);
        res!(store.append(&u, &inbox, &body(&t), MessageFlags::default(), None));
        tags.insert(t);
    }
    let v0 = (res!(store.folder_status(&u, &inbox)).uid_validity,
        res!(store.folder_status(&u, &arch)).uid_validity);
    let start = Instant::now();
    let mut hs = Vec::new();
    for w in 0..5usize {
        let r = root.clone();
        hs.push(thread::spawn(move || -> Outcome<(Vec<(u8, u32, String)>, Vec<String>)> {
            let store = res!(MaildirStore::new(r, "argonaut"));
            let u = user();
            let (inbox, arch) = (FolderName::new("INBOX"), FolderName::new("Archive"));
            let mut seen = Vec::new();
            let mut sent = Vec::new();
            for i in 0..30 {
                if w == 4 {
                    let t = fmt!("new{}", i);
                    res!(store.append(&u, &inbox, &body(&t), MessageFlags::default(), None));
                    sent.push(t);
                    continue;
                }
                let (from, to, fi) = if w < 2 { (&inbox, &arch, 0u8) } else { (&arch, &inbox, 1u8) };
                let msgs = res!(store.list_messages(&u, from, false));
                for m in &msgs {
                    // None: moved by another worker since the listing.
                    if let Some(b) = res!(store.fetch_bytes(&u, from, m.uid)) {
                        seen.push((fi, m.uid.0, tag_of(&b)));
                    }
                }
                let uids: Vec<MessageUid> = msgs.iter().map(|m| m.uid).collect();
                if !uids.is_empty() {
                    res!(store.move_to(&u, from, &uids, to));
                }
            }
            Ok((seen, sent))
        }));
    }
    let mut by_key: BTreeMap<(u8, u32), String> = BTreeMap::new();
    let mut bad = Vec::new();
    for h in hs {
        let (seen, sent) = match h.join() {
            Ok(r)   => res!(r),
            Err(_)  => return Err(err!("A worker panicked."; Test)),
        };
        tags.extend(sent);
        for (f, uid, t) in seen {
            match by_key.get(&(f, uid)) {
                Some(prev) if prev != &t => bad.push(fmt!("folder {} UID {} named {} and {}", f, uid, prev, t)),
                Some(_) => (),
                None    => { by_key.insert((f, uid), t); },
            }
        }
    }
    let took = start.elapsed();
    let a = res!(uid_map(&store, &inbox));
    let b = res!(uid_map(&store, &arch));
    let v1 = (res!(store.folder_status(&u, &inbox)).uid_validity,
        res!(store.folder_status(&u, &arch)).uid_validity);
    let mut held: Vec<String> = a.values().chain(b.values()).cloned().collect();
    held.sort();
    let want: Vec<String> = tags.into_iter().collect();
    let _ = fs::remove_dir_all(&root);
    assert!(bad.is_empty(), "UIDs reused: {:?}", &bad[..bad.len().min(5)]);
    assert_eq!(v0, v1, "UIDVALIDITY moved");
    assert_eq!(held, want, "every message in exactly one folder");
    assert!(took < Duration::from_secs(60), "took {:?}", took);
    Ok(())
}
