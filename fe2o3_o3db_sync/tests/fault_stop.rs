//! The stop after a failed barrier (2026-10-10, QA F1, F2 and F3).  The fault record names each
//! file in doubt by zone and number, so a store moved or restored elsewhere is cut where it now
//! stands and a copy left at the old path is not touched; a file in doubt that is missing refuses
//! the open; the file a writer rolled over to while the failing barrier was held is cut too; and
//! the stop is written down, as the record and the refusal marker, before a process may exit on
//! it.  The hooks are process-wide, so the tests run one at a time.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::{
        channels::FaultReport,
        response::Wait,
    },
    file::{
        core::FileType,
        refusal::Refusal,
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
    collections::BTreeMap,
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    let g = match SERIAL.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_barrier_failure(false);
    hooks::set_barrier_failure_for(None);
    hooks::set_fault_record_fails(false);
    hooks::set_barrier_delay(Duration::ZERO);
    g
}

// One zone of one writer, syncing on every write unless `never`.
fn cfg(never: bool, max: u64) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.sync_on_write         = !never;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = 0;
    c.data_file_max_bytes   = max;
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

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
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

fn len(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn copy_tree(from: &Path, to: &Path) -> Outcome<()> {
    res!(fs::create_dir_all(to));
    for e in res!(fs::read_dir(from)) {
        let p = res!(e).path();
        let name = res!(p.file_name().ok_or_else(|| err!("{:?} has no name.", p; Test)));
        match p.is_dir() {
            true    => res!(copy_tree(&p, &to.join(name))),
            false   => { res!(fs::copy(&p, to.join(name))); },
        }
    }
    Ok(())
}

// A store at `dir` holding a durable "kept", stopped by a failed barrier over "lost".
fn stopped(dir: &str) -> Outcome<(PathBuf, OzoneConfig, FaultReport)> {
    let root = res!(fresh(dir));
    let c = res!(cfg(false, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "kept", 1));
    hooks::set_barrier_failure(true);
    let lost = put(&db, "lost", 2);
    hooks::set_barrier_failure(false);
    assert!(lost.is_err(), "a write whose barrier failed was confirmed");
    let report = res!(db.api().fault().ok_or_else(|| err!("The failed barrier raised no fault."; Test)));
    let _ = db.close();
    assert!(!report.cuts.is_empty(), "the fault names no file in doubt");
    Ok((root, c, report))
}

/// The stop is written down as the record and the marker, so `recorded()` lets a gateway exit,
/// and the store refuses to open until the marker is cleared.
#[test]
fn a_stop_is_recorded_and_refuses_the_open() -> Outcome<()> {
    let _g = lock();
    let (root, c, report) = res!(stopped("./test_db_fault_stop_recorded"));
    assert!(report.recorded(), "a stop written down says it is not");
    assert_eq!(report.record, Some(root.join(constant::FAULT_FILENAME)));
    assert_eq!(report.refusal, Some(Refusal::path(&root)));
    assert!(Refusal::path(&root).exists(), "the stop left no refusal marker");
    match open(&root, &c, false) {
        Ok(db)  => {
            let _ = db.close();
            return Err(err!("The store opened over its refusal marker."; Test));
        },
        Err(e)  => assert!(fmt!("{}", e).contains(constant::REFUSAL_FILENAME),
            "the refusal does not name the marker: {}", e),
    }
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    assert_eq!(res!(reads(&db, "kept")), Some(val(1)), "the durable value was lost");
    assert_eq!(res!(reads(&db, "lost")), None, "the unconfirmed value was replayed");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A stop that could not be written down says so, and a gateway must then stay up.
#[test]
fn a_stop_not_written_down_is_not_recorded() -> Outcome<()> {
    let _g = lock();
    hooks::set_fault_record_fails(true);
    let stopped = stopped("./test_db_fault_stop_unrecorded");
    hooks::set_fault_record_fails(false);
    let (root, _, report) = res!(stopped);
    assert!(!report.recorded(), "a stop written nowhere says it is recorded");
    assert!(report.record.is_none() && report.refusal.is_none());
    assert!(!root.join(constant::FAULT_FILENAME).exists() && !Refusal::path(&root).exists());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A stopped store moved elsewhere is cut where it now stands, and the copy left at the old path
/// is not touched (F2).  Each file in doubt is given a tail past its mark, as a crash before the
/// in-process cut would leave it.
#[test]
fn a_moved_store_is_cut_where_it_stands() -> Outcome<()> {
    let _g = lock();
    let (old, c, report) = res!(stopped("./test_db_fault_stop_old"));
    let new = res!(fresh("./test_db_fault_stop_new"));
    res!(copy_tree(&old, &new));
    let mut tails = Vec::new();
    for cut in &report.cuts {
        for (p, upto) in [(&cut.data, cut.data_upto), (&cut.index, cut.index_upto)] {
            let rel = res!(p.strip_prefix(&old).map_err(|e| err!(e, "{:?} is not under {:?}.", p, old; Test)));
            for root in [&old, &new] {
                let mut f = res!(fs::OpenOptions::new().append(true).open(root.join(rel)));
                res!(f.write_all(&[0xa5u8; 64]));
            }
            tails.push((rel.to_path_buf(), upto, len(&old.join(rel))));
        }
    }
    assert!(res!(Refusal::clear(&new)));
    let db = res!(open(&new, &c, false));
    assert_eq!(res!(reads(&db, "kept")), Some(val(1)), "the durable value was lost in the move");
    assert_eq!(res!(reads(&db, "lost")), None, "the moved store replayed the unconfirmed value");
    res!(db.close());
    for (rel, upto, old_len) in &tails {
        assert!(len(&new.join(rel)) <= *upto, "{:?} of the moved store was not cut to {}", rel, upto);
        assert_eq!(len(&old.join(rel)), *old_len, "{:?} at the old path was touched", rel);
    }
    assert!(old.join(constant::FAULT_FILENAME).exists(), "the old path's record was used up");
    assert!(!new.join(constant::FAULT_FILENAME).exists(), "the moved store kept its record");
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(&new);
    Ok(())
}

/// A file in doubt that is missing refuses the open, and the record is kept for the next try.
#[test]
fn a_missing_file_in_doubt_refuses_the_open() -> Outcome<()> {
    let _g = lock();
    let (root, c, report) = res!(stopped("./test_db_fault_stop_missing"));
    assert!(res!(Refusal::clear(&root)));
    let gone = report.cuts[0].data.clone();
    res!(fs::remove_file(&gone));
    match open(&root, &c, false) {
        Ok(db)  => {
            let _ = db.close();
            return Err(err!("The store opened though its file in doubt {:?} is missing.", gone; Test));
        },
        Err(e)  => assert!(fmt!("{}", e).contains("is missing"),
            "the refusal does not say the file is missing: {}", e),
    }
    assert!(root.join(constant::FAULT_FILENAME).exists(), "the refused open removed the record");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The writer rolls over while the failing barrier is held, so the file it rolls over to was
/// not in doubt when the stop named its files.  It joins the record, and is cut (F3).
#[test]
fn the_file_rolled_over_to_is_cut() -> Outcome<()> {
    let _g = lock();
    let root = res!(fresh("./test_db_fault_stop_rollover"));
    let c = res!(cfg(true, 2_000));
    let db = Arc::new(res!(open(&root, &c, true)));
    for i in 0..5 {
        res!(put(&db, &fmt!("before {}", i), i as u8 + 1));
    }
    res!(db.api().sync_now(wait()));
    let zdir = c.zone_root(&root).join("zone_001");
    let dat = |f: u32| zdir.join(ZoneDir::relative_file_path(&FileType::Data, f));
    let first = res!((1..10_000u32).rev().find(|f| dat(*f).exists())
        .ok_or_else(|| err!("No data file is live."; Test)));
    res!(put(&db, "dirty", 9)); // so the next sync_now has a barrier to run
    hooks::set_barrier_delay(Duration::from_millis(800));
    hooks::set_barrier_failure(true);
    let db2 = db.clone();
    let sync = std::thread::spawn(move || db2.api().sync_now(wait()));
    std::thread::sleep(Duration::from_millis(100));
    // A put waits on its release, which the held barrier holds back, so the puts that must fill
    // the live file during it run side by side rather than one after another.
    let puts: Vec<_> = (0..40).map(|i| {
        let db3 = db.clone();
        std::thread::spawn(move || put(&db3, &fmt!("during {}", i), 7))
    }).collect();
    let synced = sync.join();
    for p in puts {
        let _ = p.join();
    }
    hooks::set_barrier_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    assert!(matches!(synced, Ok(Err(_))), "the barrier armed to fail did not fail");
    let last = res!((1..10_000u32).rev().find(|f| dat(*f).exists())
        .ok_or_else(|| err!("No data file is live."; Test)));
    assert!(last > first, "the writer did not roll over during the held barrier ({} to {})", first, last);
    assert!(wait_until(Duration::from_secs(10), ||
        db.api().fault().map(|r| r.cuts.iter().any(|cut| cut.fnum == last)).unwrap_or(false)),
        "the file {} rolled over to is not in the fault's cuts: {:?}", last,
        db.api().fault().map(|r| r.cuts.iter().map(|cut| cut.fnum).collect::<Vec<_>>()));
    let report = res!(db.api().fault().ok_or_else(|| err!("No fault."; Test)));
    match Arc::try_unwrap(db) {
        Ok(db)  => { let _ = db.close(); },
        Err(_)  => return Err(err!("The store is still shared."; Test)),
    }
    let record = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    assert!(record.lines().any(|l| l.starts_with(&fmt!("cut 0 {} ", last))),
        "the record does not cut file {}: {}", last, record);
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    for i in 0..5 {
        assert_eq!(res!(reads(&db, &fmt!("before {}", i))), Some(val(i as u8 + 1)), "a durable value was lost");
    }
    res!(db.close());
    for cut in &report.cuts {
        assert!(len(&cut.data) <= cut.data_upto && len(&cut.index) <= cut.index_upto,
            "file {} was not cut to its mark ({}, {}): ({}, {})", cut.fnum, cut.data_upto,
            cut.index_upto, len(&cut.data), len(&cut.index));
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
