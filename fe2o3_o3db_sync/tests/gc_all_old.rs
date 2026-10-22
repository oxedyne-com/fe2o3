//! A file whose records are all old is deleted without a collection (A3 R3, 2026-10-08).  Until
//! that day its state outlived it, with the old bytes still counted and its sizes still in the
//! shard's total, so a count of what the store still had to collect included files that were gone.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    file::core::FileType,
    test::setup,
};

use std::{
    path::Path,
    time::Duration,
};

#[test]
fn deleted_all_old_file_leaves_no_state() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_all_old";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let probe = res!(probe("./test_db_gc_all_old_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, true));
    let user = setup::Uid::default();
    res!(fill(&db));
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    let (d1, d2) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Data, 2));
    assert!(d1.is_file() && d2.is_file(), "positive control: files 1 and 2 hold the first records");
    let states = res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT));
    let mut counted = 0;
    for (_wind, fmap) in &states {
        if let Some(fstat) = fmap.map().get(&1) {
            counted = fstat.get_data_file_size();
        }
    }
    assert_eq!(counted as u64, 5 * probe.len, "positive control: file 1's state holds its five records");

    // Overwriting every key makes files 1 and 2 all old, and they are sealed.
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 2), user, None));
    }
    res!(db.api().settle_for_test(Duration::from_secs(60)));
    assert!(
        wait_until(Duration::from_secs(30), || !d1.is_file() && !d2.is_file()),
        "files 1 and 2 hold only old records and were not deleted",
    );
    res!(db.api().settle_for_test(Duration::from_secs(60)));
    std::thread::sleep(Duration::from_millis(1500));

    let states = res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT));
    let (mut accounted, mut seen) = (0usize, 0usize);
    for (_wind, fmap) in &states {
        accounted += fmap.get_size();
        for (fnum, fstat) in fmap.map() {
            seen += 1;
            if !file(&root, &cfg, FileType::Data, *fnum).is_file() {
                assert_eq!(fstat.get_old_sum(), 0, "file {} is gone but its state counts old bytes", fnum);
                assert_eq!(fstat.get_old_count(), 0, "file {} is gone but its state counts old records", fnum);
                assert!(fstat.data_map_empty(), "file {} is gone but its state maps records", fnum);
                assert_eq!(fstat.get_data_file_size() + fstat.get_index_file_size(), 0,
                    "file {} is gone but its state has a size", fnum);
            }
        }
    }
    assert!(seen >= 3, "positive control: the shards hold the states of the files still there");
    let mut physical = 0u64;
    for n in 1..40 {
        physical += size(&file(&root, &cfg, FileType::Data, n));
        physical += size(&file(&root, &cfg, FileType::Index, n));
    }
    msg!("Shard sizes total {}, files hold {}.", accounted, physical);
    assert_eq!(accounted as u64, physical, "the shards' total includes files that are gone");
    assert_eq!(judge(&db, &vec![2u8; NKEYS], "after the deletions"), 0);
    res!(db.close());
    Ok(())
}
