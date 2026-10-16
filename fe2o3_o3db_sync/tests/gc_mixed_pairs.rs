//! The start must not believe an index that is not its data file's own, whichever file of the pair
//! is the newer, and must leave alone an index that is (D-C, 2026-10-01).  A collection replaces a
//! file's data and index with shorter ones, so an interruption between the two renames leaves one
//! generation's file beside the other's.  Stores written before the collection committed its data
//! file first hold the other mixture, the index of the new generation beside the old data.  Both
//! used to be believed: the cache was filled from the index, the data file's own entries were
//! refused as no newer, and records were read from the offsets of the file beside them.
//!
//! The two generations of file 1 are made by really collecting it, and each mixture is made by
//! putting one generation's file beside the other's in the store.  Nothing here needs a hook.

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
    time::Duration,
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_mixed_pairs";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let probe = res!(probe("./test_db_gc_mixed_pairs_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    // The generations are kept outside the store, where nothing will look for them.
    let sdir = "./test_db_gc_mixed_pairs_stash";
    let _ = std::fs::remove_dir_all(sdir);
    res!(std::fs::create_dir_all(sdir));
    let sdir = res!(Path::new(sdir).canonicalize());
    let stash = |name: &str| sdir.join(name);

    // The old generation of file 1: five records.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    res!(fill(&db));
    res!(db.close());
    assert_eq!((size(&d1), size(&i1)), (5 * len, probe.ind[5]), "file 1 is not five records");
    res!(std::fs::copy(&d1, stash("old.dat")));
    res!(std::fs::copy(&i1, stash("old.ind")));

    // The new generation: superseding keys 0 and 1 collects file 1 down to keys 2 to 4.
    let mut want = vec![1u8; NKEYS];
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    for i in 0..2 {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
        want[i] = 2;
    }
    assert!(wait_until(Duration::from_secs(60), || size(&d1) == 3 * len && size(&i1) == probe.ind[3]),
        "file 1 was not collected to three records");
    res!(db.close());
    res!(std::fs::copy(&d1, stash("new.dat")));
    res!(std::fs::copy(&i1, stash("new.ind")));

    let mut bad = Vec::new();

    // The new index beside the old data file: what a collection used to leave.
    res!(std::fs::copy(stash("old.dat"), &d1));
    res!(std::fs::copy(stash("new.ind"), &i1));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let n = judge(&db, &want, "New index beside old data");
    if n != 0 { bad.push(fmt!("{} keys read wrong with the new index beside the old data", n)); }
    res!(db.close());
    if size(&i1) != probe.ind[5] {
        bad.push(fmt!("the index beside the old data was {} bytes, not rebuilt to {}",
            size(&i1), probe.ind[5]));
    }

    // The old index beside the new data file: what an interruption leaves.
    res!(std::fs::copy(stash("new.dat"), &d1));
    res!(std::fs::copy(stash("old.ind"), &i1));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let n = judge(&db, &want, "Old index beside new data");
    if n != 0 { bad.push(fmt!("{} keys read wrong with the old index beside the new data", n)); }
    res!(db.close());
    if size(&i1) != probe.ind[3] {
        bad.push(fmt!("the index beside the new data was {} bytes, not rebuilt to {}",
            size(&i1), probe.ind[3]));
    }

    // A pair that is whole is read as it is: the start must not rewrite an index it can believe.
    res!(std::fs::copy(stash("new.dat"), &d1));
    res!(std::fs::copy(stash("new.ind"), &i1));
    let before = res!(res!(std::fs::metadata(&i1)).modified());
    let bytes = res!(std::fs::read(&i1));
    thread_pause();
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let n = judge(&db, &want, "Whole pair");
    if n != 0 { bad.push(fmt!("{} keys read wrong with a whole pair", n)); }
    res!(db.close());
    log_finish_wait!();
    if res!(std::fs::read(&i1)) != bytes {
        bad.push(fmt!("the index of a whole pair was changed by the start"));
    }
    if res!(res!(std::fs::metadata(&i1)).modified()) != before {
        bad.push(fmt!("the index of a whole pair was rewritten by the start"));
    }

    assert!(bad.is_empty(), "{}", bad.join("; "));
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(&sdir);
    Ok(())
}

/// Long enough for a file written now to have a modification time of its own.
fn thread_pause() {
    std::thread::sleep(Duration::from_millis(50));
}
