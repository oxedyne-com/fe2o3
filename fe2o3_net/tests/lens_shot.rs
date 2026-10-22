//! `fe2o3_net::lens::Shots`, the picture store behind Daimond's Lens screenshot (D-20261006-14).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::lens::{
    stamp,
    Answer,
    Ask,
    Filed,
    ShotGates,
    ShotRefusal,
    Shots,
};

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = SERIAL.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(fmt!("lens-shot-{}-{}-{}", std::process::id(), tag, n));
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }
    fn shots(&self) -> Shots { Shots::new(self.0.join("shots"), ShotGates::daimond()) }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest-of-a-picture";

// The wall clock, since the hand-out and ask files are aged by modification time.
fn now() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d)   => d.as_millis() as u64,
        Err(_)  => 0,
    }
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = match fs::read_dir(dir) {
        Ok(rd)  => rd.filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect(),
        Err(_)  => Vec::new(),
    };
    v.sort();
    v
}

#[test]
fn nothing_is_wanted_until_asked() -> Outcome<()> {
    let s = Scratch::new("none");
    let shots = s.shots();
    assert_eq!(res!(shots.take("acct", "dev1", now())), Ask::Nothing);
    Ok(())
}

#[test]
fn an_ask_is_handed_out_once_and_answered_with_a_picture() -> Outcome<()> {
    let s = Scratch::new("pic");
    let shots = s.shots();
    let t = now();
    res!(shots.ask("dev1", "r1"));
    assert_eq!(res!(shots.take("acct", "dev1", t)), Ask::Take("r1".to_string()));
    assert_eq!(res!(shots.take("acct", "dev1", t)), Ask::Nothing, "an ask is collected once");
    let filed = res!(shots.answer("acct", "dev1", "r1", Answer::Picture(PNG), t + 1_000));
    let want = shots.dir().join(fmt!("{}.acct.dev1.r1.png", stamp(t + 1_000)));
    assert_eq!(filed, Filed::Stored(want.clone()));
    assert_eq!(res!(fs::read(&want)), PNG.to_vec());
    assert_eq!(res!(shots.find("dev1", "r1")), Some(want));
    // Consumed: the same answer again is unasked.
    assert_eq!(
        res!(shots.answer("acct", "dev1", "r1", Answer::Picture(PNG), t + 2_000)),
        Filed::Refused(ShotRefusal::Unasked),
    );
    assert!(names(shots.dir()).iter().all(|n| !n.ends_with(".part")), "no part file is left");
    Ok(())
}

#[test]
fn an_answer_nobody_asked_for_is_refused() -> Outcome<()> {
    let s = Scratch::new("unasked");
    let shots = s.shots();
    let t = now();
    assert_eq!(
        res!(shots.answer("acct", "dev1", "r1", Answer::Picture(PNG), t)),
        Filed::Refused(ShotRefusal::Unasked),
    );
    res!(shots.ask("dev1", "r1"));
    res!(shots.take("acct", "dev1", t));
    // Another account, another id and a late answer are each unasked.
    for (acct, id, at) in [("other", "r1", t), ("acct", "r2", t), ("acct", "r1", t + 120_001)] {
        assert_eq!(
            res!(shots.answer(acct, "dev1", id, Answer::Picture(PNG), at)),
            Filed::Refused(ShotRefusal::Unasked),
        );
    }
    assert_eq!(ShotRefusal::Unasked.status(), 409);
    assert!(names(shots.dir()).is_empty(), "nothing was filed");
    Ok(())
}

#[test]
fn a_picture_must_be_a_png_within_the_cap() -> Outcome<()> {
    let s = Scratch::new("cap");
    let shots = Shots::new(s.0.join("shots"), ShotGates { max_bytes: 64, ..ShotGates::daimond() });
    let t = now();
    res!(shots.ask("dev1", "r1"));
    res!(shots.take("acct", "dev1", t));
    assert_eq!(
        res!(shots.answer("acct", "dev1", "r1", Answer::Picture(b"GIF89a....."), t)),
        Filed::Refused(ShotRefusal::NotPicture),
    );
    let mut big = PNG.to_vec();
    big.resize(65, 0);
    assert_eq!(
        res!(shots.answer("acct", "dev1", "r1", Answer::Picture(&big), t)),
        Filed::Refused(ShotRefusal::TooLarge),
    );
    assert_eq!(ShotRefusal::TooLarge.status(), 413);
    // A refused body does not consume the ask.
    assert!(matches!(
        res!(shots.answer("acct", "dev1", "r1", Answer::Picture(PNG), t)),
        Filed::Stored(_),
    ));
    Ok(())
}

#[test]
fn a_device_may_refuse_and_say_why() -> Outcome<()> {
    let s = Scratch::new("refuse");
    let shots = s.shots();
    let t = now();
    res!(shots.ask("dev1", "r1"));
    res!(shots.take("acct", "dev1", t));
    let filed = res!(shots.answer("acct", "dev1", "r1", Answer::Refused("off\non this device"), t));
    let path = match filed {
        Filed::Stored(p)    => p,
        other               => return Err(err!("Expected a stored refusal, got {:?}.", other; Test)),
    };
    assert!(path.to_string_lossy().ends_with(".acct.dev1.r1.refused"));
    assert_eq!(res!(fs::read_to_string(&path)), "off on this device", "a line break cannot forge a line");
    Ok(())
}

#[test]
fn a_second_ask_inside_a_minute_is_refused_for_the_device() -> Outcome<()> {
    let s = Scratch::new("rate");
    let shots = s.shots();
    let t = now();
    res!(shots.ask("dev1", "r1"));
    res!(shots.take("acct", "dev1", t));
    res!(shots.answer("acct", "dev1", "r1", Answer::Picture(PNG), t + 1_000));
    res!(shots.ask("dev1", "r2"));
    match res!(shots.take("acct", "dev1", t + 30_000)) {
        Ask::TooSoon(id, wait)  => {
            assert_eq!(id, "r2");
            assert_eq!(wait, 30_000);
        }
        other                   => return Err(err!("Expected TooSoon, got {:?}.", other; Test)),
    }
    let refused = match res!(shots.find("dev1", "r2")) {
        Some(p) => p,
        None    => return Err(err!("The rate refusal was not filed."; Test)),
    };
    assert!(res!(fs::read_to_string(&refused)).starts_with("rate:"));
    // The refused ask was collected, so the device is not asked again; after the floor it is.
    assert_eq!(res!(shots.take("acct", "dev1", t + 30_001)), Ask::Nothing);
    res!(shots.ask("dev1", "r3"));
    assert_eq!(res!(shots.take("acct", "dev1", t + 60_000)), Ask::Take("r3".to_string()));
    Ok(())
}

#[test]
fn pictures_older_than_seven_days_are_pruned() -> Outcome<()> {
    let s = Scratch::new("prune");
    let shots = s.shots();
    let t = now();
    let day = 86_400_000;
    res!(fs::create_dir_all(shots.dir()));
    let old = shots.dir().join(fmt!("{}.acct.dev1.r0.png", stamp(t - 8 * day)));
    let young = shots.dir().join(fmt!("{}.acct.dev1.r1.png", stamp(t - 6 * day)));
    let stranger = shots.dir().join("README.txt");
    for p in [&old, &young, &stranger] {
        res!(fs::write(p, PNG));
    }
    assert_eq!(res!(shots.prune(t)), 1);
    assert!(!old.exists(), "eight days old goes");
    assert!(young.exists(), "six days old stays");
    assert!(stranger.exists(), "a file not named by the store is left alone");
    Ok(())
}

#[test]
fn the_oldest_pictures_go_when_the_total_is_exceeded() -> Outcome<()> {
    let s = Scratch::new("total");
    let gates = ShotGates { max_total: 2 * PNG.len() as u64, ..ShotGates::daimond() };
    let shots = Shots::new(s.0.join("shots"), gates);
    let t = now();
    res!(fs::create_dir_all(shots.dir()));
    for i in 0..3u64 {
        res!(fs::write(shots.dir().join(fmt!("{}.a.d.r{}.png", stamp(t - (3 - i) * 1_000), i)), PNG));
    }
    assert_eq!(res!(shots.prune(t)), 1);
    let left = names(shots.dir());
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|n| !n.ends_with(".r0.png")), "the oldest went: {:?}", left);
    Ok(())
}

#[test]
fn segments_cannot_walk_out_of_the_directory() -> Outcome<()> {
    let s = Scratch::new("walk");
    let shots = s.shots();
    let t = now();
    res!(shots.ask("../../etc", "../r1"));
    assert_eq!(res!(shots.take("a/../b", "../../etc", t)), Ask::Take("___r1".to_string()));
    let filed = res!(shots.answer("a/../b", "../../etc", "../r1", Answer::Picture(PNG), t));
    match filed {
        Filed::Stored(p)    => assert_eq!(p.parent(), Some(shots.dir())),
        other               => return Err(err!("Expected stored, got {:?}.", other; Test)),
    }
    Ok(())
}
