//! `MailStore::deliver` files a sender's retry once, and the `hematite-msgids` index behind it
//! never blocks a delivery nor loses one (F2a part 5, QA F2a4 M1 to M4).
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::mail::store::{
    FolderName,
    MailStore,
    MailUser,
    MessageFlags,
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
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

fn fresh(name: &str) -> Outcome<(PathBuf, MaildirStore)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_dedupe_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    res!(store.ensure_user(&user()));
    Ok((root, store))
}

fn inbox_dir(root: &Path) -> PathBuf { root.join("test.local").join("alice") }

fn flags() -> MessageFlags { MessageFlags { recent: true, ..MessageFlags::default() } }

fn with_id(id: &str, tag: &str) -> Vec<u8> {
    fmt!("From: bob@example.org\r\nTo: alice@test.local\r\nMessage-ID: {}\r\nSubject: {}\r\n\r\nid={}\r\n",
        id, tag, tag).into_bytes()
}

/// `bytes` as a later hop delivers it: the fields relays and verifiers add, prepended.
fn in_transit(bytes: &[u8], hop: &str) -> Vec<u8> {
    let mut out = fmt!(
        "Return-Path: <bob@example.org>\r\nDelivered-To: alice@test.local\r\n\
        X-Original-To: alice@test.local\r\nReceived: from {} by mx; Mon, 26 Oct 2026 10:00:00 +0000\r\n\
        Received-SPF: pass\r\nAuthentication-Results: mx; dkim=pass\r\nARC-Seal: i=1; cv=none; b={}\r\n\
        DKIM-Signature: v=1; t={}; b=abc\r\n\tdef\r\n",
        hop, hop, hop).into_bytes();
    out.extend_from_slice(bytes);
    out
}

/// UID → `id=` tag for every message in the INBOX.
fn tags(store: &MaildirStore) -> Outcome<BTreeMap<u32, String>> {
    let (u, f) = (user(), FolderName::new("INBOX"));
    let mut out = BTreeMap::new();
    for m in res!(store.list_messages(&u, &f, true)) {
        if let Some(b) = res!(store.fetch_bytes(&u, &f, m.uid)) {
            let s = String::from_utf8_lossy(&b).to_string();
            let tag = match s.find("id=") { Some(i) => s[i + 3..].trim().to_string(), None => String::new() };
            out.insert(m.uid.0, tag);
        }
    }
    Ok(out)
}

fn is_root() -> bool {
    match fs::read_to_string("/proc/self/status") {
        Ok(s) => s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(2) == Some("0")),
        Err(_) => false,
    }
}

/// M1: a file the store cannot read is a Message-ID unknown, not a failed delivery.
#[cfg(unix)]
#[test]
fn an_unreadable_file_does_not_block_delivery() -> Outcome<()> {
    use std::os::unix::fs::PermissionsExt;
    if is_root() { return Ok(()); } // root reads a 000 file
    let (root, store) = res!(fresh("m1"));
    let (u, ib) = (user(), FolderName::new("INBOX"));
    let p = inbox_dir(&root).join("new").join("1.M1P1.other");
    res!(fs::write(&p, with_id("<o@x>", "o")), IO, File);
    res!(fs::set_permissions(&p, fs::Permissions::from_mode(0o000)), IO, File);
    let r1 = store.deliver(&u, &ib, &with_id("<n@x>", "n"), flags(), None);
    let r2 = store.deliver(&u, &ib, &with_id("<o@x>", "o"), flags(), None);
    res!(fs::set_permissions(&p, fs::Permissions::from_mode(0o600)), IO, File);
    assert!(r1.is_ok(), "deliver failed on an unreadable neighbour: {:?}", r1.err());
    assert!(r2.is_ok(), "deliver failed on an unreadable neighbour: {:?}", r2.err());
    let t = res!(tags(&store));
    assert_eq!(t.values().filter(|v| *v == "o").count(), 2,
        "an unreadable copy is unknown, so the delivery is filed: {:?}", t);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// M2: a row of the index naming the wrong Message-ID never drops a delivery.
#[test]
fn a_damaged_index_row_never_loses_a_message() -> Outcome<()> {
    let (root, store) = res!(fresh("m2"));
    let (u, ib) = (user(), FolderName::new("INBOX"));
    res!(store.deliver(&u, &ib, &with_id("<a@x>", "a"), flags(), None));
    res!(store.deliver(&u, &ib, &with_id("<b@x>", "b"), flags(), None));
    let idx = inbox_dir(&root).join("hematite-msgids");
    let text = res!(fs::read_to_string(&idx), IO, File);
    assert!(text.contains("<a@x>"), "a is indexed: {:?}", text);
    res!(fs::write(&idx, text.replace("<a@x>", "<z@x>")), IO, File);
    res!(store.deliver(&u, &ib, &with_id("<z@x>", "z"), flags(), None));
    let t = res!(tags(&store));
    assert!(t.values().any(|v| v == "z"), "z was dropped on a damaged row: {:?}", t);
    assert_eq!(t.len(), 3);
    // The row is corrected, so a's retry is still filed once.
    res!(store.deliver(&u, &ib, &with_id("<a@x>", "a"), flags(), None));
    assert_eq!(res!(tags(&store)).len(), 3);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// M3: only an identical resend is dropped. The fields added in transit do not count; a change
/// to any field the sender set, or to the body, does.
#[test]
fn only_an_identical_resend_is_dropped() -> Outcome<()> {
    let (root, store) = res!(fresh("m3"));
    let (u, ib) = (user(), FolderName::new("INBOX"));
    let sent = with_id("<a@x>", "a");
    let first = res!(store.deliver(&u, &ib, &in_transit(&sent, "hop1"), flags(), None));
    let retry = res!(store.deliver(&u, &ib, &in_transit(&sent, "hop2"), flags(), None));
    let bare = res!(store.deliver(&u, &ib, &sent, flags(), None));
    assert_eq!((first, first), (retry, bare), "a resend differing only in transit fields is one");
    assert_eq!(res!(tags(&store)).len(), 1);

    let body = res!(store.deliver(&u, &ib, &with_id("<a@x>", "a-other-body"), flags(), None));
    let mut subj = sent.clone();
    let at = sent.windows(9).position(|w| w == b"Subject: ").unwrap_or(0) + 9;
    subj.insert(at, b'X');
    let field = res!(store.deliver(&u, &ib, &subj, flags(), None));
    let resent = [b"Resent-From: carol@example.org\r\n".as_slice(), &sent].concat();
    let redirect = res!(store.deliver(&u, &ib, &resent, flags(), None));
    assert!(body != first && field != first && redirect != first && body != field && field != redirect);
    assert_eq!(res!(tags(&store)).len(), 4, "a reused Message-ID with other content is filed");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// M4: a delivery is linear in the folder's size. Run with
/// `cargo test -p oxedyne_fe2o3_mail --test delivery_dedupe -- --ignored --nocapture`.
#[test]
#[ignore]
fn delivery_is_linear_in_the_folder() -> Outcome<()> {
    let mut times: Vec<(usize, Duration, Duration)> = Vec::new();
    for n in [5_000usize, 10_000, 20_000] {
        let (root, store) = res!(fresh(&fmt!("m4_{}", n)));
        let (u, ib) = (user(), FolderName::new("INBOX"));
        let pad = fmt!("X-Pad: {}\r\n", "p".repeat(70));
        let big = |i: usize| fmt!("From: b@x\r\nMessage-ID: <big{}@x>\r\n{}{}Subject: s\r\n\r\nid=big{}\r\n{}",
            i, pad, pad, i, "body ".repeat(400)).into_bytes();
        for i in 0..n {
            res!(fs::write(inbox_dir(&root).join("cur").join(fmt!("{}.M{}P1.big:2,S", 1_000_000 + i, i)), big(i)), IO, File);
        }
        res!(store.list_messages(&u, &ib, false));
        let t = Instant::now();
        res!(store.deliver(&u, &ib, &with_id("<new0@x>", "new0"), flags(), None));
        let build = t.elapsed();
        let mut best = Duration::MAX;
        for k in 1..4 {
            let t = Instant::now();
            res!(store.deliver(&u, &ib, &with_id(&fmt!("<new{}@x>", k), "new"), flags(), None));
            best = best.min(t.elapsed());
        }
        let t = Instant::now();
        res!(store.deliver(&u, &ib, &in_transit(&big(7), "hop"), flags(), None));
        let hit = t.elapsed();
        assert_eq!(res!(store.list_messages(&u, &ib, false)).len(), n + 4, "big7 is a retry");
        println!("M4 n={} index build {:?}, deliver {:?}, retry hit {:?}", n, build, best, hit);
        times.push((n, build, best));
        let _ = fs::remove_dir_all(&root);
    }
    let ratio = times[2].2.as_secs_f64() / times[0].2.as_secs_f64();
    println!("M4 deliver 20k / 5k = {:.1}", ratio);
    assert!(ratio < 8.0, "x4 messages made deliver x{:.1}, more than linear", ratio);
    Ok(())
}
