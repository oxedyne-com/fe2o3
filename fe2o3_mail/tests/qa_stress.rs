//! QA F2a: processes and threads deliver, flag, expunge and SELECT one folder; every UID seen
//! must name one message, APPEND's UID must be its own, UIDVALIDITY must not move.
//! From QA F2a's `qa_stress.rs` (D-20261026-01), ported to update_flags and the Option of
//! fetch_bytes: 4 processes (this test binary re-run as `qa_child`) of 3 threads each.
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::mail::store::{FlagUpdate, FolderName, MailStore, MailUser, MessageFlags};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, thread};

fn user() -> MailUser {
    MailUser { local: fmt!("alice"), domain: fmt!("test.local"), delivery_key: fmt!("test.local/alice"), send_as: Vec::new() }
}
fn body(tag: &str) -> Vec<u8> { fmt!("Subject: {}\r\n\r\nid={}\r\n", tag, tag).into_bytes() }
fn tag_of(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    match s.find("id=") { Some(i) => s[i + 3..].trim().to_string(), None => String::new() }
}

/// One worker: returns lines "uid tag" it observed or was given by append, and "V v".
fn work(root: PathBuf, who: String, rounds: usize) -> Outcome<Vec<String>> {
    let store = res!(MaildirStore::new(root.clone(), "argonaut"));
    let u = user();
    let inbox = FolderName::new("INBOX");
    let mut seen = Vec::new();
    for i in 0..rounds {
        let tag = fmt!("{}-{}", who, i);
        let uid = res!(store.append(&u, &inbox, &body(&tag), MessageFlags::default(), None));
        seen.push(fmt!("A {} {}", uid.0, tag));
        // An external lock-free MDA delivery into new/ as well.
        let ib = root.join("test.local/alice");
        let n = fmt!("1800000000.M{}P{}X{}.ext", i, std::process::id(), who);
        res!(fs::write(ib.join("tmp").join(&n), body(&fmt!("{}-ext{}", who, i))), IO, File);
        res!(fs::rename(ib.join("tmp").join(&n), ib.join("new").join(&n)), IO, File);
        let msgs = res!(store.list_messages(&u, &inbox, i % 2 == 0)); // SELECT moves new/ to cur/
        for m in &msgs {
            match res!(store.fetch_bytes(&u, &inbox, m.uid)) {
                Some(b) => seen.push(fmt!("S {} {}", m.uid.0, tag_of(&b))),
                None    => (), // Expunged since the listing.
            }
            // Delete every third message whose tag is one of ours, then expunge.
            if m.uid.0 % 3 == 0 {
                let f = MessageFlags { deleted: true, ..MessageFlags::default() };
                let _ = res!(store.update_flags(&u, &inbox, m.uid, FlagUpdate::Add(f)));
            }
        }
        res!(store.expunge(&u, &inbox, None));
        let st = res!(store.folder_status(&u, &inbox));
        seen.push(fmt!("V {} {}", st.uid_validity, st.uid_next));
    }
    Ok(seen)
}

#[test]
fn qa_child() -> Outcome<()> {
    let root = match std::env::var("QA_STRESS_ROOT") { Ok(r) => PathBuf::from(r), Err(_) => return Ok(()) };
    let who = res!(std::env::var("QA_STRESS_WHO"), Missing);
    let out = res!(std::env::var("QA_STRESS_OUT"), Missing);
    let mut lines = Vec::new();
    let hs: Vec<_> = (0..3).map(|t| { let r = root.clone(); let w = fmt!("{}t{}", who, t); thread::spawn(move || work(r, w, 25)) }).collect();
    for h in hs { match h.join() { Ok(Ok(l)) => lines.extend(l), Ok(Err(e)) => { println!("QA_CHILD_ERR {}", e); return Err(e) }, Err(_) => return Err(err!("panic"; Bug)) } }
    res!(fs::write(out, lines.join("\n")), IO, File);
    Ok(())
}

/// About 4 min in a debug build, so ignored. Run it alone:
/// `cargo test -p oxedyne_fe2o3_mail --test qa_stress -- --ignored --nocapture qa_stress_processes`.
/// Before the fair ExclusiveLock wait (2ed9d149) 1 to 3 of the 12 workers failed with File Lock
/// Timeout after 30 s, passed over by polling; after it, three runs gave uids=600 bad=0.
#[test]
#[ignore]
fn qa_stress_processes() -> Outcome<()> {
    if std::env::var("QA_STRESS_ROOT").is_ok() { return Ok(()); }
    let root = std::env::temp_dir().join(fmt!("qa_f2a_stress_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(root.clone(), "argonaut"));
    res!(store.ensure_user(&user()));
    let exe = res!(std::env::current_exe(), IO);
    let mut kids = Vec::new();
    for p in 0..4 {
        let out = root.join(fmt!("out{}", p));
        let c = res!(Command::new(&exe).args(["qa_child", "--exact", "--nocapture"])
            .env("QA_STRESS_ROOT", &root).env("QA_STRESS_WHO", fmt!("p{}", p)).env("QA_STRESS_OUT", &out).spawn(), IO);
        kids.push((c, out));
    }
    let mut by_uid: BTreeMap<u32, String> = BTreeMap::new();
    let mut vals = std::collections::BTreeSet::new();
    let mut bad = Vec::new();
    for (mut c, out) in kids {
        let st = res!(c.wait(), IO);
        if !st.success() { bad.push(fmt!("child failed {:?}", st)); continue; }
        let s = res!(fs::read_to_string(&out), IO);
        for l in s.lines() {
            let t: Vec<&str> = l.split(' ').collect();
            if t[0] == "V" { vals.insert(t[1].to_string()); continue; }
            let uid: u32 = res!(t[1].parse::<u32>().map_err(|e| err!(e, "uid"; Invalid)));
            match by_uid.get(&uid) {
                Some(prev) if prev != t[2] => bad.push(fmt!("UID {} named {} and {}", uid, prev, t[2])),
                Some(_) => (),
                None => { by_uid.insert(uid, t[2].to_string()); },
            }
        }
    }
    println!("uids={} validities={:?} bad={}", by_uid.len(), vals, bad.len());
    for b in bad.iter().take(10) { println!("{}", b); }
    let _ = fs::remove_dir_all(&root);
    if !bad.is_empty() || vals.len() != 1 { return Err(err!("{} conflicts, {} validities", bad.len(), vals.len(); Test)); }
    Ok(())
}
