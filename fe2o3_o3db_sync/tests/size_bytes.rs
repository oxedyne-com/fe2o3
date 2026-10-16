//! `O3db::size_bytes` against the files themselves (A3 R1, 2026-10-08).  A cap on disk use needs
//! the length the zone directories hold now, not the file bots' accounted figure, which is pushed
//! periodically and lags any write still draining.  Every figure here is checked against a walk of
//! the directories made by the test with nothing of the store's own (`zone_files_len`).

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    file::core::FileType,
    test::setup::{
        self,
        Uid,
    },
};

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread,
};

fn fresh(dir: &str) -> Outcome<std::path::PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

#[test]
fn size_equals_zone_files_after_known_writes() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_size_bytes_known";
    let root = res!(fresh(dir));

    let probe = res!(probe("./test_db_size_bytes_known_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    res!(fill(&db));

    // Twelve records of one length, five to a data file: files 1 and 2 hold five, file 3 holds two.
    let (full, part) = (5 * len, 2 * len);
    let (ifull, ipart) = (probe.ind[5], probe.ind[2]);
    for (n, d, i) in [(1, full, ifull), (2, full, ifull), (3, part, ipart)] {
        assert_eq!(size(&file(&root, &cfg, FileType::Data, n)), d, "data file {}", n);
        assert_eq!(size(&file(&root, &cfg, FileType::Index, n)), i, "index file {}", n);
    }
    let want = 2 * full + part + 2 * ifull + ipart;

    let got = res!(db.size_bytes());
    let walked = zone_files_len(&root, &cfg);
    msg!("size_bytes {}, walked {}, probe says {}.", got, walked, want);
    assert_eq!(walked, want, "the files are not the probe's twelve records");
    assert_eq!(got, walked, "size_bytes is not the length of the files");

    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn size_counts_every_zone() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_size_bytes_zones";
    let root = res!(fresh(dir));

    let probe = res!(probe("./test_db_size_bytes_zones_probe"));
    let mut cfg = res!(config(20 * probe.len));
    cfg.num_zones = 3;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    for i in 0..30 {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }

    let zones = cfg.zone_root(&root);
    let per_zone: Vec<u64> = (1..=3).map(|z| dir_len(&zones.join(fmt!("zone_{:03}", z)))).collect();
    let got = res!(db.size_bytes());
    msg!("size_bytes {}, zone directories {:?}.", got, per_zone);
    assert_eq!(per_zone.iter().sum::<u64>(), zone_files_len(&root, &cfg));
    assert!(per_zone.iter().all(|n| *n > 0), "a zone holds nothing: {:?}", per_zone);
    assert_eq!(got, zone_files_len(&root, &cfg), "size_bytes is not the length of all three zones");
    assert!(got > per_zone[0], "size_bytes {} counts no more than zone 1's {}", got, per_zone[0]);

    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn size_tracks_delete_and_reopen() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_size_bytes_tracks";
    let root = res!(fresh(dir));

    let probe = res!(probe("./test_db_size_bytes_tracks_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let mut seen = Vec::new();
    let mut check = |db: &TestDb, label: &str| -> Outcome<()> {
        let got = res!(db.size_bytes());
        let walked = zone_files_len(&root, &cfg);
        msg!("{}: size_bytes {}, walked {}.", label, got, walked);
        assert_eq!(got, walked, "{}: size_bytes is not the length of the files", label);
        seen.push(got);
        Ok(())
    };

    // 1. Written, then one record deleted: both leave the files longer, with collection off.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, true));
    res!(fill(&db));
    res!(check(&db, "after the writes"));
    assert!(res!(db.delete(&key(0), Uid::default(), None)));
    res!(check(&db, "after the delete"));

    // 2. Closed and reopened: the reopened store starts a new live file.
    res!(db.close());
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    res!(check(&db, "after reopening"));
    res!(db.close());

    // 3. Reopened with collection on and overwritten three times: files are collected and
    //    deleted, so the length falls as well as grows.  Measured once the files have stopped
    //    changing.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    for ver in 2..5u8 {
        for i in 0..NKEYS {
            res!(db.insert(key(i), value(i, ver), Uid::default(), None));
        }
    }
    let settled = settled_len(&root, &cfg);
    res!(check(&db, "after the overwrites"));
    assert_eq!(*seen.last().unwrap_or(&0), settled, "the files moved while they were measured");

    // The figure moved with the files rather than being the first answer given.
    let first = seen[0];
    assert!(seen.iter().any(|n| *n != first), "size_bytes never changed: {:?}", seen);

    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn size_survives_files_vanishing() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_size_bytes_vanish";
    let root = res!(fresh(dir));

    let probe = res!(probe("./test_db_size_bytes_vanish_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    res!(fill(&db));

    // Overwrites keep collections renaming and deleting files while another thread asks for the
    // size, at least 200 times and until the overwrites are done.
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (db, done) = (db.clone(), done.clone());
        thread::spawn(move || -> Outcome<u64> {
            let mut calls = 0u64;
            loop {
                let n = res!(db.size_bytes());
                assert!(n > 0, "size_bytes gave 0 for a store that holds files");
                calls += 1;
                if calls >= 200 && done.load(Ordering::Relaxed) {
                    return Ok(calls);
                }
            }
        })
    };
    for round in 0..40u8 {
        for i in 0..NKEYS {
            res!(db.insert(key(i), value(i, 2 + round), Uid::default(), None));
        }
    }
    done.store(true, Ordering::Relaxed);
    let calls = match reader.join() {
        Ok(r) => res!(r),
        Err(_) => return Err(err!("The size_bytes reader thread panicked."; Test, Thread)),
    };
    msg!("{} size_bytes calls answered while 40 rounds of overwrites drove collections.", calls);
    assert!(calls >= 200);

    let settled = settled_len(&root, &cfg);
    let got = res!(db.size_bytes());
    msg!("size_bytes {} with the files settled at {}.", got, settled);
    assert_eq!(got, settled, "size_bytes is not the length of the settled files");
    assert_eq!(got, zone_files_len(&root, &cfg));

    res!(db.close());
    log_finish_wait!();
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
