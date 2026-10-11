//! An index standing without its data file is judged at the open (2026-10-11).  A collection
//! deletes a whole file only when every record in it is old, by two unlinks.  Builds before this
//! day removed the data file first, so a crash between the two left the index alone, and the open
//! refused the store as if the data had been lost.  A store opened by this build completes such a
//! delete, rebuilds an index removed first, and refuses only when the index names a record nothing
//! else holds, saying which and what to do.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    file::core::FileType,
    test::setup::{
        self,
        Uid,
    },
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
};

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

// Collection is off throughout, so the files stay as the test leaves them.
fn open(root: &Path, cfg: &OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(cfg.clone()), schemes(), None, false, wipe)
}

// Five records to a file, written at version 1 and then all overwritten at version 2, so files 1
// and 2 hold only old records: the files a collection deletes whole.
fn superseded(name: &str) -> Outcome<(PathBuf, OzoneConfig)> {
    let root = res!(fresh(&fmt!("./test_db_lone_index_{}", name)));
    let probe = res!(probe(&fmt!("./test_db_lone_index_{}_probe", name)));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db = res!(open(&root, &cfg, true));
    res!(fill(&db));
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
    }
    assert_eq!(judge(&db, &[2u8; NKEYS], "before the cut"), 0);
    res!(db.close());
    for n in 1..3 {
        assert!(file(&root, &cfg, FileType::Data, n).is_file()
            && size(&file(&root, &cfg, FileType::Index, n)) > 0,
            "positive control: file {} stands whole with its index", n);
    }
    Ok((root, cfg))
}

/// The order of builds before 2026-10-11: the data file of an all-old pair was removed and the
/// crash came before its index went.  The open completes the delete.
#[test]
fn the_old_order_completes_the_delete() -> Outcome<()> {
    let (root, cfg) = res!(superseded("old_order"));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&d1));
    let db = match open(&root, &cfg, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "An interrupted delete of an all-old file was refused."; Test)),
    };
    assert!(!i1.exists(), "the index of the deleted data file {:?} still stands", i1);
    assert!(!d1.exists());
    assert_eq!(judge(&db, &[2u8; NKEYS], "after the completed delete"), 0);
    // The store goes on as usual, and opens again.
    res!(db.insert(key(0), value(0, 3), Uid::default(), None));
    res!(db.close());
    let db = res!(open(&root, &cfg, false));
    let mut want = [2u8; NKEYS];
    want[0] = 3;
    assert_eq!(judge(&db, &want, "after a second open"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The present order: the index went first and the crash came before the data file.  The index
/// is rebuilt from the data and the old records stay old.
#[test]
fn the_current_order_rebuilds_the_index() -> Outcome<()> {
    let (root, cfg) = res!(superseded("current_order"));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&i1));
    let db = res!(open(&root, &cfg, false));
    assert!(d1.is_file(), "the data file {:?} is gone", d1);
    assert!(size(&i1) > 0, "the index {:?} was not rebuilt", i1);
    assert_eq!(judge(&db, &[2u8; NKEYS], "after the rebuild"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A data file really lost: its index names records nothing else holds.  The open refuses and
/// says which index, which record and which data file, and what to do.  Taking the way out it
/// names opens the store without those records.
#[test]
fn a_lone_index_naming_a_current_record_refuses() -> Outcome<()> {
    let root = res!(fresh("./test_db_lone_index_loss"));
    let probe = res!(probe("./test_db_lone_index_loss_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db = res!(open(&root, &cfg, true));
    res!(fill(&db));
    res!(db.close());
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&d1));
    let e = match open(&root, &cfg, false) {
        Ok(db) => {
            let bad = judge(&db, &[1u8; NKEYS], "opened without a data file");
            let _ = db.close();
            return Err(err!("The store opened with {:?} lost; {} keys read wrong.", d1, bad; Test));
        },
        Err(e) => fmt!("{}", e),
    };
    println!("Refused: {}", e);
    assert!(e.contains(&fmt!("{:?}", i1)), "the refusal does not name the index {:?}: {}", i1, e);
    assert!(e.contains(&fmt!("{:?}", d1)), "the refusal does not name the data file {:?}: {}", d1, e);
    assert!(e.contains("is missing"), "{}", e);
    assert!(e.contains("gc pair key 00"), "the refusal does not name the key: {}", e);
    assert!(e.contains("(record "), "the refusal does not name the record: {}", e);
    assert!(e.contains("backup or a replica"), "the refusal does not say to restore: {}", e);
    assert!(e.contains("remove the index file"), "the refusal does not give the last resort: {}", e);
    assert!(size(&i1) > 0, "a refused open removed the index {:?}", i1);

    // The way out: losing those records is accepted, so the index goes.
    res!(fs::remove_file(&i1));
    let db = res!(open(&root, &cfg, false));
    for i in 0..NKEYS {
        let got = res!(db.get(&key(i), None));
        match i < 5 {
            true    => assert!(got.is_none(), "key {} of the lost file reads back", i),
            false   => assert!(matches!(got, Some((ref v, _)) if *v == value(i, 1)), "key {} is wrong", i),
        }
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The chunks of a value since overwritten, in a file deleted whole whose index outlived it.
/// Nothing names them, so the delete is completed.
#[test]
fn chunks_of_a_retired_set_complete_the_delete() -> Outcome<()> {
    let root = res!(fresh("./test_db_lone_index_chunks"));
    let mut cfg = res!(config(2_000));
    cfg.rest_chunk_threshold    = 1_000;
    cfg.rest_chunk_bytes        = 400;
    let db = res!(open(&root, &cfg, true));
    let big = dat!("lone index big");
    res!(db.insert(big.clone(), Dat::BU32(vec![0xa7; 3_000]), Uid::default(), None));
    res!(db.insert(big.clone(), Dat::BU32(vec![0x5c; 200]), Uid::default(), None));
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    res!(db.close());
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    let chunky = files_holding(&root, &cfg, &[0xa7u8; 64]);
    assert!(chunky.contains(&d1), "positive control: file 1 holds chunks of the first value: {:?}", chunky);
    assert!(!files_holding(&root, &cfg, &[0x5cu8; 64]).contains(&d1)
        && !files_holding(&root, &cfg, b"gc pair key").contains(&d1),
        "positive control: file 1 holds nothing current");
    res!(fs::remove_file(&d1));
    let db = match open(&root, &cfg, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "An interrupted delete of retired chunks was refused."; Test)),
    };
    assert!(!i1.exists(), "the index {:?} of the retired chunks still stands", i1);
    let got = res!(db.get(&big, None));
    assert!(matches!(got, Some((Dat::BU32(ref v), _)) if *v == vec![0x5c; 200]), "the value of the big key is wrong");
    assert_eq!(judge(&db, &[1u8; NKEYS], "after the completed delete"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The number of an index standing alone at the survey is not handed out again as a live file,
/// even once the judgement has removed it: until then the old index would have named the new
/// file's records.  Here the index is put above every data file, so a counter seeded from the data
/// files alone gives its number to the first live file the store rolls over to.
#[test]
fn a_lone_index_number_is_not_reused() -> Outcome<()> {
    let (root, cfg) = res!(superseded("reuse"));
    let mut top = 0;
    for n in 1..40 {
        if file(&root, &cfg, FileType::Data, n).is_file() {
            top = n;
        }
    }
    let lone = top + 1;
    res!(fs::remove_file(file(&root, &cfg, FileType::Data, 1)));
    res!(fs::rename(file(&root, &cfg, FileType::Index, 1), file(&root, &cfg, FileType::Index, lone)));
    let db = res!(open(&root, &cfg, false));
    assert!(!file(&root, &cfg, FileType::Index, lone).exists(), "the lone index {} still stands", lone);
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 3), Uid::default(), None));
    }
    assert_eq!(judge(&db, &[3u8; NKEYS], "after the rollovers"), 0);
    res!(db.close());
    assert!(file(&root, &cfg, FileType::Data, lone + 1).is_file(), "positive control: the store rolled over past {}", lone);
    assert!(!file(&root, &cfg, FileType::Data, lone).exists(),
        "a live file took the number {} of the index that stood alone", lone);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
