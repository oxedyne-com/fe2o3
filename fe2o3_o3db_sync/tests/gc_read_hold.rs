//! A read that waits behind a collection longer than it will wait for its location must give back
//! the reader pin the file bot takes for it when it finally grants the location (L-DC2,
//! 2026-10-01).  The reader had gone, so it never sent `ReadFinished`, the file kept a reader for
//! the life of the process, and a file with a reader is neither collected nor deleted: a fully
//! superseded file stayed on the disk.  `test::hooks::set_commit_delay` holds the collection of
//! file 1 for longer than the read deadline, and is process-wide, which is why this is a test
//! binary of its own with a single test.

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
    path::Path,
    thread,
    time::{
        Duration,
        Instant,
    },
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_read_hold";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());

    let probe = res!(probe("./test_db_gc_read_hold_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let mut want = vec![1u8; NKEYS];
    res!(fill(&db));
    // Reopened, so that the cache holds locations and no values, and every read of a record in
    // file 1 goes to its file bot.  In the process that wrote the records their values are
    // served from the cache and a read never reaches the file bot at all.
    res!(db.close());
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));

    // 1. The collection of file 1, held for seven seconds between its renames, which is longer
    //    than a read waits for its location.  Superseding two of its records starts it.
    hooks::set_commit_delay(Duration::from_secs(7));
    for i in 0..2 {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
        want[i] = 2;
    }
    assert!(wait_until(Duration::from_secs(30), || size(&d1) == 3 * len),
        "the collection never committed its data file");

    // 2. A read of a record the collection carried.  It reaches the file bot while the file is
    //    held, is buffered there, and its reader stops waiting five seconds on.  Whether that read
    //    is answered or refused is not what this test is about.
    let t = Instant::now();
    let held = db.get(&key(2), None);
    msg!("The read held behind the collection returned {} after {:?}.",
        if held.is_ok() { "an answer" } else { "an error" }, t.elapsed());
    assert!(wait_until(Duration::from_secs(30), || size(&i1) == probe.ind[3]),
        "the collection never finished");
    hooks::set_commit_delay(Duration::ZERO);
    // Let the file bot replay the buffered read: it grants a location to a reader that is gone.
    thread::sleep(Duration::from_millis(500));
    assert_eq!(judge(&db, &want, "After the collection"), 0);

    // 3. Every record left in file 1 superseded, so that the file is wholly old.  A file bot that
    //    still counts a reader for the file keeps it, and its three records stay on the disk.
    for i in 2..5 {
        res!(db.insert(key(i), value(i, 3), Uid::default(), None));
        want[i] = 3;
    }
    assert!(wait_until(Duration::from_secs(10), || size(&d1) == 0),
        "file 1 is wholly superseded and is still {} bytes: a reader pin was never returned",
        size(&d1));
    assert_eq!(judge(&db, &want, "Settled"), 0);

    res!(db.close());
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    assert_eq!(judge(&db, &want, "After reopening"), 0);
    res!(db.close());
    log_finish_wait!();

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
