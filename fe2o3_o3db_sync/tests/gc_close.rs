//! A close that arrives while a collection is between its two renames must leave a store that
//! reads every key right (D-C, 2026-10-01).  The collection is held between the renames for longer
//! than a close waits for the collectors, so the close finishes the cache bots and the file bots
//! with the collection still to replace its index and to ask the cache it has no one left to
//! answer.  Before the data file was committed first, that left a new index beside the old data
//! file, which the next start believed.  `test::hooks::set_commit_delay` is process-wide, which is
//! why this is a test binary of its own with a single test.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    file::core::FileType,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    time::{
        Duration,
        Instant,
    },
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_close";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(std::path::Path::new(dir).canonicalize());
    let probe = res!(probe("./test_db_gc_close_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let zone = zone_dir(&root, &cfg);
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let mut want = vec![1u8; NKEYS];
    res!(fill(&db));

    // Held for longer than a close gives the collectors, and longer than the cache update waits.
    hooks::set_commit_delay(Duration::from_secs(8));
    for i in 0..2 {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
        want[i] = 2;
    }
    assert!(wait_until(Duration::from_secs(30), || size(&d1) == 3 * len),
        "the collection of file 1 never reached its renames");
    assert_eq!(size(&i1), probe.ind[5], "the index was replaced before the delay");
    let start = Instant::now();
    res!(db.close());
    msg!("The close took {:?}.", start.elapsed());
    hooks::set_commit_delay(Duration::ZERO);

    // The collection ran to its end after the close began.
    let finished = wait_until(Duration::from_secs(30), || size(&i1) == probe.ind[3]);
    let temps = temporaries(&zone);
    let index = res!(std::fs::read(&i1));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let reopened = judge(&db, &want, "After the close");
    res!(db.close());
    log_finish_wait!();
    let unchanged = res!(std::fs::read(&i1)) == index;

    assert!(finished, "the index of file 1 is {} bytes, not the {} of the collected file",
        size(&i1), probe.ind[3]);
    assert!(temps.is_empty(), "a collection left {:?} behind", temps);
    assert_eq!(reopened, 0, "{} keys read wrong after the close", reopened);
    assert!(unchanged, "the start rebuilt an index that agreed with its data file");
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
