//! The files `test::file::save_multiple_files` writes must be a store a start believes (Q4,
//! 2026-10-01).  Its index entries named where each record ended, not where it started, so a
//! start that checks an index against its data file refused every one and rebuilt it, which
//! hid that the helper's idea of the format was wrong.  The oracle is the store itself: the
//! helper's files are put where a store keeps them and opened, and none of the index files may be
//! touched by the start nor any value read back wrong.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    file::core::FileType,
    test::{
        file::save_multiple_files,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    fs,
    path::Path,
    time::SystemTime,
};

#[test]
fn main() -> Outcome<()> {
    log_set_level!("error");
    let (dir_a, dir_b) = ("./test_db_helper_index_a", "./test_db_helper_index_b");
    for dir in [dir_a, dir_b] {
        let _ = fs::remove_dir_all(dir);
        res!(fs::create_dir_all(dir));
    }
    let (root_a, root_b) = (res!(Path::new(dir_a).canonicalize()), res!(Path::new(dir_b).canonicalize()));

    // What the writer makes of these records: five to a file, so twelve keys fill three files.
    let probe = res!(probe("./test_db_helper_index_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));

    // The helper encodes with a database's keys and schemes, then writes the files itself, into
    // the zone directory of a store that is not yet there.
    let mut db_a: TestDb = res!(setup::start_db(root_a, Some(cfg.clone()), schemes(), None, false, true));
    let zone = zone_dir(&root_b, &cfg);
    res!(fs::create_dir_all(&zone));
    let ks: Vec<_> = (0..NKEYS).map(key).collect();
    let vs: Vec<_> = (0..NKEYS).map(|i| value(i, 1)).collect();
    res!(save_multiple_files(
        zone.clone(),
        res!(cfg.data_file_max_bytes.try_into()),
        &mut db_a,
        Uid::default(),
        None,
        ks,
        vs,
        NKEYS * len as usize,
    ));
    res!(db_a.close());

    // Records of the writer's length, five to a file.
    let data = |n| file(&root_b, &cfg, FileType::Data, n);
    let index = |n| file(&root_b, &cfg, FileType::Index, n);
    assert_eq!(size(&data(1)), 5 * len, "the helper's first file does not hold five of the writer's records");
    assert_eq!(size(&data(2)), 5 * len, "the helper's second file does not hold five of the writer's records");
    assert_eq!(size(&data(3)), 2 * len, "the helper's third file does not hold the last two records");
    // And index files as long as the writer's, whose entries are as long as the offsets they name.
    assert_eq!(size(&index(1)), probe.ind[5], "the helper's first index is not the writer's length");
    assert_eq!(size(&index(3)), probe.ind[2], "the helper's last index is not the writer's length");
    let before: Vec<(Vec<u8>, SystemTime)> = (1..=3)
        .map(|n| {
            let p = index(n);
            (
                fs::read(&p).unwrap_or_default(),
                fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH),
            )
        })
        .collect();

    // A start over the helper's files.
    let db_b: TestDb = res!(setup::start_db(root_b.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let want = vec![1u8; NKEYS];
    assert_eq!(judge(&db_b, &want, "Over the helper's files"), 0);
    res!(db_b.close());
    log_finish_wait!();

    for (i, (bytes, mtime)) in before.iter().enumerate() {
        let p = index(i as u32 + 1);
        assert!(fs::read(&p).unwrap_or_default() == *bytes,
            "the start rewrote index file {}: it refused the helper's entries and rebuilt them", i + 1);
        assert!(fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH) == *mtime,
            "the start touched index file {}", i + 1);
    }

    for dir in [dir_a, dir_b] {
        let _ = fs::remove_dir_all(dir);
    }
    Ok(())
}
