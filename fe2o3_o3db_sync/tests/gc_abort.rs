//! A collection that fails before it has replaced anything must leave the file's pair as it was,
//! must not leave the file held, and must leave nothing the next collection of the file would be
//! hurt by (D-C, 2026-10-01).  Until then a failed collection returned an error that was only
//! logged: the file bot went on holding every read and write of the file for a collection that was
//! not coming, so the file could never be collected again, and a temporary file the collection had
//! begun was left to be appended to by the next one.  `test::hooks::set_collect_failure` fails
//! each collection once it has written its temporary files, and is process-wide, which is why this
//! is a test binary of its own with a single test.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
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
    time::Duration,
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_abort";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());

    let probe = res!(probe("./test_db_gc_abort_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let zone = zone_dir(&root, &cfg);
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let mut want = vec![1u8; NKEYS];
    res!(fill(&db));
    let (d_old, i_old) = (size(&d1), size(&i1));
    assert_eq!(d_old, 5 * len, "file 1 does not hold the five records it was sized for");

    // 1. The collection that fails.  Superseding two of file 1's records starts it, which then
    //    waits a second and a half before it reads the file, and a third record is superseded
    //    while it waits, so that the file bot holds that supersession in the collection's buffer.
    hooks::set_collect_delay(Duration::from_millis(1_500));
    hooks::set_collect_failure(true);
    for i in 0..3 {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
        want[i] = 2;
    }
    assert!(wait_until(Duration::from_secs(30), || hooks::collections_failed() >= 1),
        "the collection of file 1 never started");

    // 2. The file is not held.  When the collection is abandoned the file bot replays what it
    //    held, and the third supersession finds the file eligible again, so a second collection
    //    starts and fails.  A file bot still waiting on the first would never start another.
    assert!(wait_until(Duration::from_secs(30), || hooks::collections_failed() >= 2),
        "file 1 was never collected again after a collection of it failed");
    // The abandoned collections take their temporaries with them.
    assert!(wait_until(Duration::from_secs(10), || temporaries(&zone).is_empty()),
        "an abandoned collection left {:?} behind", temporaries(&zone));
    hooks::set_collect_delay(Duration::ZERO);
    assert_eq!((size(&d1), size(&i1)), (d_old, i_old),
        "file 1 is not the pair it was after collections that replaced nothing");
    let held = judge(&db, &want, "After two failed collections");

    // 3. A temporary left in the directory, as a crash or a failed removal would leave it, must
    //    not become the head of the next collected file.
    let mut stale = zone.clone();
    stale.push(ZoneDir::relative_gc_temp_path(&FileType::Data, 1));
    res!(std::fs::write(&stale, vec![0xabu8; 777]));
    hooks::set_collect_failure(false);
    res!(db.insert(key(3), value(3, 2), Uid::default(), None));
    want[3] = 2;
    // Keys 0 to 3 are superseded, so the collection carries key 4's record alone.
    assert!(wait_until(Duration::from_secs(30), || size(&d1) != d_old),
        "file 1 was not collected once collection could succeed");
    assert!(wait_until(Duration::from_secs(10), || size(&i1) == probe.ind[1]),
        "the index of file 1 was not rebuilt to one record");
    let collected = size(&d1);
    let after = judge(&db, &want, "After the collection that worked");
    res!(db.close());

    let index = res!(std::fs::read(&i1));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let reopened = judge(&db, &want, "After reopening");
    res!(db.close());
    log_finish_wait!();
    let unchanged = res!(std::fs::read(&i1)) == index;

    assert_eq!(held, 0, "{} keys read wrong after the failed collections", held);
    assert_eq!(collected, len,
        "file 1 is {} bytes after its collection, where its one current record is {}", collected, len);
    assert_eq!(after, 0, "{} keys read wrong after the collection that worked", after);
    assert_eq!(reopened, 0, "{} keys read wrong after reopening", reopened);
    assert!(unchanged, "the start rebuilt an index that agreed with its data file");
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
