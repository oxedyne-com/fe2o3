//! An append to a live file that fails part-way (2026-10-11, QA round 1 F4).  The live files are
//! opened for appending, so a seek back has no effect on where the next write lands: the torn
//! bytes stayed in the file, the next record went in after them, and the writer's idea of the
//! file's length, from which the next record's location is taken, fell behind the file.  The torn
//! append is now cut back off, both files of the pair return to their last good lengths, and a
//! cut that fails stops the store through its fault.  The hooks are process-wide, so the tests
//! run one at a time.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    file::{
        core::FileType,
        refusal::Refusal,
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
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::Mutex,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    let g = match SERIAL.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_write_tear(FileType::Data, None);
    hooks::set_write_tear(FileType::Index, None);
    hooks::set_cut_back_failure(false);
    g
}

// One zone of one writer, syncing on every write.
fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.sync_on_write         = true;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = 0;
    c.data_file_max_bytes   = 1_000_000;
    Ok(c)
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path, c: &OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(c.clone()), schemes(), None, false, wipe)
}

fn val(seed: u8) -> Dat {
    Dat::BU32((0..100).map(|j| seed ^ (j as u8)).collect())
}

fn put(db: &TestDb, k: &str, seed: u8) -> Outcome<()> {
    db.insert(dat!(k), val(seed), Uid::default(), None).map(|_| ())
}

fn reads(db: &TestDb, k: &str) -> Outcome<Option<Dat>> {
    Ok(res!(db.get(&dat!(k), None)).map(|(v, _)| v))
}

// The live pair, the highest numbered data file and its index.
fn live(root: &Path, c: &OzoneConfig) -> Outcome<(PathBuf, PathBuf)> {
    let n = res!((1..10_000u32).rev().find(|n| file(root, c, FileType::Data, *n).exists())
        .ok_or_else(|| err!("No data file is live."; Test)));
    Ok((file(root, c, FileType::Data, n), file(root, c, FileType::Index, n)))
}

// Two good records, "k0" and "k1", and the pair's lengths before and after the second.
struct Start {
    root:   PathBuf,
    c:      OzoneConfig,
    db:     TestDb,
    dat:    PathBuf,
    ind:    PathBuf,
    rec:    u64,    // data bytes in one record
    dlen:   u64,    // data length after "k1"
    ilen:   u64,    // index length after "k1"
}

fn start(dir: &str) -> Outcome<Start> {
    let root = res!(fresh(dir));
    let c = res!(cfg());
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 1));
    let (dat, ind) = res!(live(&root, &c));
    let before = size(&dat);
    res!(put(&db, "k1", 2));
    let dlen = size(&dat);
    let ilen = size(&ind);
    Ok(Start { root, c, db, dat, ind, rec: dlen - before, dlen, ilen })
}

// After the next record, and again after a reopen, every key reads its value and the failed one
// is absent.
fn check_after(s: Start, failed: &str) -> Outcome<()> {
    let Start { root, c, db, dat, ind, rec, dlen, ilen } = s;
    res!(put(&db, "k3", 4));
    assert_eq!(size(&dat), dlen + rec,
        "the record after the failed one did not land at the last good length {}", dlen);
    assert!(size(&ind) > ilen, "the record after the failed one is not in the index");
    let want = [("k0", Some(val(1))), ("k1", Some(val(2))), (failed, None), ("k3", Some(val(4)))];
    for (k, v) in &want {
        assert_eq!(&res!(reads(&db, k)), v, "{} reads wrong before the reopen", k);
    }
    res!(db.close());
    let db = res!(open(&root, &c, false));
    for (k, v) in &want {
        assert_eq!(&res!(reads(&db, k)), v, "{} reads wrong after the reopen", k);
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A data append torn part-way is cut back off, and the next record lands where it began.
#[test]
fn a_torn_data_append_is_cut_back() -> Outcome<()> {
    let _g = lock();
    let s = res!(start("./test_db_append_rewind_data"));
    let torn = hooks::writes_torn();
    hooks::set_write_tear(FileType::Data, Some(s.rec / 2));
    let failed = put(&s.db, "k2", 3);
    assert_eq!(hooks::writes_torn(), torn + 1, "the tear was not reached");
    assert!(failed.is_err(), "a torn write was confirmed");
    assert_eq!(size(&s.dat), s.dlen, "the torn bytes were left in the data file");
    assert_eq!(size(&s.ind), s.ilen, "the index changed for a record that failed");
    assert!(s.db.api().fault().is_none(), "a torn write cut back cleanly stopped the store");
    check_after(s, "k2")
}

/// An index append torn part-way withdraws the whole record, its data as well, so a write
/// reported failed does not come back at the next open.
#[test]
fn a_torn_index_append_withdraws_the_record() -> Outcome<()> {
    let _g = lock();
    let s = res!(start("./test_db_append_rewind_index"));
    let torn = hooks::writes_torn();
    hooks::set_write_tear(FileType::Index, Some(3));
    let failed = put(&s.db, "k2", 3);
    assert_eq!(hooks::writes_torn(), torn + 1, "the tear was not reached");
    assert!(failed.is_err(), "a torn write was confirmed");
    assert_eq!(size(&s.ind), s.ilen, "the torn bytes were left in the index file");
    assert_eq!(size(&s.dat), s.dlen, "the data of a record whose index failed was left");
    assert!(s.db.api().fault().is_none(), "a torn write cut back cleanly stopped the store");
    check_after(s, "k2")
}

/// A torn append that cannot be cut back off stops the store through its fault, which names the
/// last good lengths, and the next open cuts the pair back to them.
#[test]
fn a_failed_cut_back_stops_the_store() -> Outcome<()> {
    let _g = lock();
    let Start { root, c, db, dat, ind, rec, dlen, ilen } =
        res!(start("./test_db_append_rewind_stop"));
    hooks::set_cut_back_failure(true);
    hooks::set_write_tear(FileType::Data, Some(rec / 2));
    let failed = put(&db, "k2", 3);
    hooks::set_cut_back_failure(false);
    assert!(failed.is_err(), "a torn write was confirmed");
    let report = res!(db.api().fault().ok_or_else(|| err!(
        "A torn append that could not be cut back did not stop the store."; Test)));
    assert!(report.recorded(), "the stop was not written down");
    assert!(report.cuts.iter().any(|cut| cut.data == dat && cut.data_upto == dlen
        && cut.index == ind && cut.index_upto == ilen),
        "the fault does not cut the pair to ({}, {}): {:?}", dlen, ilen, report.cuts);
    assert!(put(&db, "k3", 4).is_err(), "a stopped store took a write");
    let _ = db.close();
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    assert_eq!(size(&dat), dlen, "the open did not cut the torn bytes off the data file");
    assert_eq!(res!(reads(&db, "k0")), Some(val(1)));
    assert_eq!(res!(reads(&db, "k1")), Some(val(2)));
    assert_eq!(res!(reads(&db, "k2")), None, "the torn record was served");
    assert_eq!(res!(reads(&db, "k3")), None, "the refused record was served");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
