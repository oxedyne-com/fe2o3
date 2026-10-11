//! QA round 2, agent A (2026-10-11): sticky refusal, `FaultReport::recorded()`, and a missing
//! file in doubt.  Adversarial probes only; the hooks are process-wide, so one at a time.

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
    comm::channels::{
        Cut,
        Fault,
        FaultReport,
    },
    db::O3db,
    file::refusal::Refusal,
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
    sync::Mutex,
    time::Duration,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    let g = match SERIAL.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_barrier_failure(false);
    hooks::set_barrier_failure_for(None);
    hooks::set_fault_record_fails(false);
    hooks::set_fault_record_fails_at(0);
    hooks::set_replay_sync_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    g
}

fn cfg(zones: usize) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = zones as u16;
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

fn refused(r: Outcome<TestDb>, what: &str) -> Outcome<String> {
    match r {
        Ok(db)  => {
            let _ = db.close();
            Err(err!("{}: the store opened.", what; Test))
        },
        Err(e)  => Ok(fmt!("{}", e)),
    }
}

// A store with durable "kept 0..n" in every zone, stopped by a failed barrier over "lost".
// Returns the store still open when `keep_open`.
fn stopped(dir: &str, zones: usize, keep_open: bool)
    -> Outcome<(PathBuf, OzoneConfig, FaultReport, Option<TestDb>)>
{
    let root = res!(fresh(dir));
    let c = res!(cfg(zones));
    let db = res!(open(&root, &c, true));
    for i in 0..20u8 {
        res!(put(&db, &fmt!("kept {}", i), i + 1));
    }
    hooks::set_barrier_failure(true);
    let lost = put(&db, "lost", 99);
    hooks::set_barrier_failure(false);
    assert!(lost.is_err(), "a write whose barrier failed was confirmed");
    let report = res!(db.api().fault().ok_or_else(|| err!("No fault raised."; Test)));
    assert!(!report.cuts.is_empty(), "the fault names no file in doubt");
    if keep_open {
        return Ok((root, c, report, Some(db)));
    }
    let _ = db.close();
    Ok((root, c, report, None))
}

fn cut(fnum: u32) -> Cut {
    Cut {
        zone:       0,
        fnum,
        data:       PathBuf::from(fmt!("/nonexistent/{}.dat", fnum)),
        data_upto:  10,
        index:      PathBuf::from(fmt!("/nonexistent/{}.ind", fnum)),
        index_upto: 10,
    }
}

/// P1 (surface 2).  A second raise whose record rewrite fails keeps `record` pointing at the old
/// record, which lacks the new cut, so `recorded()` says yes while the new cut is durable nowhere.
#[test]
fn p1_stale_record_counts_as_recorded() -> Outcome<()> {
    let _g = lock();
    let root = res!(fresh("./test_db_qa_r2a_p1"));
    // The marker's temporary path is a directory, so the marker cannot be written; the record can.
    res!(fs::create_dir_all(root.join(fmt!("{}.tmp", constant::REFUSAL_FILENAME))));
    let f = Fault::default();
    let e = err!("injected barrier failure"; Test);
    let r1 = f.raise(&root, vec![cut(1)], &e);
    println!("P1 first raise: record {:?} refusal {:?}", r1.record, r1.refusal);
    assert!(r1.record.is_some() && r1.refusal.is_none(), "setup: record only");
    hooks::set_fault_record_fails(true);
    let r2 = f.raise(&root, vec![cut(2)], &e);
    hooks::set_fault_record_fails(false);
    let text = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    let marker = Refusal::path(&root).exists();
    println!("P1 second raise: record {:?} refusal {:?} recorded() {}", r2.record, r2.refusal, r2.recorded());
    println!("P1 record on disk:\n{}P1 marker exists: {}", text, marker);
    let cut2_durable = text.lines().any(|l| l.starts_with("cut 0 2 ")) || marker;
    let _ = fs::remove_dir_all(&root);
    assert!(!r2.recorded() || cut2_durable,
        "FINDING: recorded() is true but cut 2 is in neither the record nor a marker");
    Ok(())
}

/// P2 (surfaces 1, 2).  The marker stands but the record does not (its write failed, or it was
/// lost).  Clearing then opening replays the unconfirmed tail: the marker names no file.
#[test]
fn p2_marker_without_record_replays_after_clear() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_p2", 1, false));
    let marker = res!(fs::read_to_string(Refusal::path(&root)));
    println!("P2 marker text: {:?}", marker);
    // Put back what the in-process cut removed, as a crash before that cut would leave it.
    let zdir = c.zone_root(&root).join("zone_001");
    let qdir = zdir.join(constant::QUARANTINE_DIR);
    let mut restored = 0;
    if let Ok(rd) = fs::read_dir(&qdir) {
        for ent in rd {
            let q = res!(ent).path();
            let name = q.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let base = match name.rsplit_once('.') { Some((b, _)) => b.to_string(), None => continue };
            let tail = res!(fs::read(&q));
            let mut f = res!(fs::OpenOptions::new().append(true).open(zdir.join(&base)));
            res!(f.write_all(&tail));
            restored += tail.len();
        }
    }
    println!("P2 tail bytes restored from quarantine: {}; cuts {:?}", restored,
        report.cuts.iter().map(|c| (c.fnum, c.data_upto, c.index_upto)).collect::<Vec<_>>());
    res!(fs::remove_file(root.join(constant::FAULT_FILENAME)));
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    let lost = res!(reads(&db, "lost"));
    let kept = res!(reads(&db, "kept 0"));
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    println!("P2 after clear with no record: lost = {:?}", lost.is_some());
    assert_eq!(kept, Some(val(1)));
    assert!(lost.is_none() || !marker.contains("cut"),
        "unreachable: the marker carried cuts");
    assert!(lost.is_none(), "FINDING: the unconfirmed value was served after clear, and the marker named no file to check: {:?}", marker);
    Ok(())
}

/// P3 (surface 1).  `Refusal::clear` while the faulted store is still open in a live process.
#[test]
fn p3_clear_while_open_elsewhere() -> Outcome<()> {
    let _g = lock();
    let (root, c, _, db) = res!(stopped("./test_db_qa_r2a_p3", 1, true));
    let db = res!(db.ok_or_else(|| err!("no db"; Test)));
    assert!(Refusal::path(&root).exists());
    let cleared = Refusal::clear(&root);
    let second = refused(open(&root, &c, false), "second open while held");
    println!("P3 clear while held: {:?}; a second open: {:?}", cleared.as_ref().map(|b| *b).map_err(|e| fmt!("{}", e)),
        second.as_ref().map(|s| s.chars().take(160).collect::<String>()).map_err(|e| fmt!("{}", e)));
    let _ = db.close();
    let _ = fs::remove_dir_all(&root);
    assert!(matches!(second, Ok(ref s) if s.contains("held by another open")), "setup: the lock was not held");
    assert!(!matches!(cleared, Ok(true)),
        "FINDING: the refusal was cleared while the faulted store was open (lock held)");
    Ok(())
}

/// P4 (surface 3).  The index file in doubt is missing: the open refuses, names the index file,
/// keeps the record, and refuses again on a second try.
#[test]
fn p4_missing_index_in_doubt() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_p4", 1, false));
    assert!(res!(Refusal::clear(&root)));
    let gone = report.cuts[0].index.clone();
    res!(fs::remove_file(&gone));
    let name = gone.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let e1 = res!(refused(open(&root, &c, false), "missing index"));
    let e2 = res!(refused(open(&root, &c, false), "missing index, second try"));
    let rec = root.join(constant::FAULT_FILENAME).exists();
    let mk = Refusal::path(&root).exists();
    println!("P4 error: {}", e1.chars().take(400).collect::<String>());
    println!("P4 record kept {} marker {}", rec, mk);
    let _ = fs::remove_dir_all(&root);
    assert!(e1.contains("is missing") && e1.contains(&name), "the refusal does not name {}: {}", name, e1);
    assert!(e2.contains("is missing"));
    assert!(rec, "the record was removed");
    Ok(())
}

/// P5 (surface 3).  The data file in doubt is missing: the message names that file.
#[test]
fn p5_missing_data_names_the_file() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_p5", 1, false));
    assert!(res!(Refusal::clear(&root)));
    let gone = report.cuts[0].data.clone();
    res!(fs::remove_file(&gone));
    let name = gone.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let e = res!(refused(open(&root, &c, false), "missing data"));
    let _ = fs::remove_dir_all(&root);
    assert!(e.contains("is missing") && e.contains(&name), "the refusal does not name {}: {}", name, e);
    Ok(())
}

/// P6 (surface 1).  A failed replay flush whose marker cannot be written either: the error says
/// so; the next open is then not refused (expected; documented in the error).
#[test]
fn p6_flush_fails_and_marker_fails() -> Outcome<()> {
    let _g = lock();
    let root = res!(fresh("./test_db_qa_r2a_p6"));
    let c = res!(cfg(1));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "kept 0", 1));
    res!(db.close());
    hooks::set_replay_sync_failure(true);
    hooks::set_fault_record_fails(true);
    let r = refused(open(&root, &c, false), "flush failed");
    hooks::set_replay_sync_failure(false);
    hooks::set_fault_record_fails(false);
    let e = res!(r);
    let mk = Refusal::path(&root).exists();
    let next = open(&root, &c, false);
    let next_opened = next.is_ok();
    if let Ok(db) = next { let _ = db.close(); }
    println!("P6 error has CRITICAL: {}; marker {}; next open opened: {}", e.contains("CRITICAL"), mk, next_opened);
    let _ = fs::remove_dir_all(&root);
    assert!(e.contains("CRITICAL"), "the double failure is not said: {}", e);
    Ok(())
}

/// P7 (surface 3).  A data file missing from a zone the record does not name.
#[test]
fn p7_missing_file_in_unnamed_zone() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_p7", 2, false));
    let named: Vec<usize> = report.cuts.iter().map(|c| c.zone).collect();
    let other = if named.contains(&0) { 1 } else { 0 };
    let zdir = c.zone_root(&root).join(fmt!("zone_{:03}", other + 1));
    let mut victim = None;
    for ent in res!(fs::read_dir(&zdir)) {
        let p = res!(ent).path();
        if p.is_file() && p.extension().map(|e| e == "dat" || e == "data").unwrap_or(false) {
            victim = Some(p);
        }
    }
    let listing: Vec<String> = res!(fs::read_dir(&zdir)).filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string()).collect();
    println!("P7 cuts in zones {:?}; other zone {} holds {:?}", named, other + 1, listing);
    let victim = res!(victim.ok_or_else(|| err!("no data file in {:?}", zdir; Test)));
    res!(fs::remove_file(&victim));
    assert!(res!(Refusal::clear(&root)));
    let r = open(&root, &c, false);
    let name = victim.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    match r {
        Ok(db) => {
            let mut missing = Vec::new();
            for i in 0..20u8 {
                match reads(&db, &fmt!("kept {}", i)) {
                    Ok(Some(v)) if v == val(i + 1) => (),
                    other => missing.push((i, fmt!("{:?}", other.map(|o| o.is_some())))),
                }
            }
            let _ = db.close();
            let _ = fs::remove_dir_all(&root);
            Err(err!("FINDING: the store opened with {:?} removed; keys not read back: {:?}",
                victim, missing; Test))
        },
        Err(e) => {
            let e = fmt!("{}", e);
            println!("P7 refused: {}", e.chars().take(300).collect::<String>());
            let _ = fs::remove_dir_all(&root);
            assert!(e.contains("is missing") && e.contains(&name),
                "the refusal does not name {}: {}", name, e);
            Ok(())
        },
    }
}

/// P8 (surface 3).  A record naming a zone the store does not have refuses, and is kept.
#[test]
fn p8_record_names_absent_zone() -> Outcome<()> {
    let _g = lock();
    let (root, c, _, _) = res!(stopped("./test_db_qa_r2a_p8", 1, false));
    assert!(res!(Refusal::clear(&root)));
    let rp = root.join(constant::FAULT_FILENAME);
    let mut text = res!(fs::read_to_string(&rp));
    text.push_str("cut 4 1 0 0\n");
    res!(fs::write(&rp, text));
    let e = res!(refused(open(&root, &c, false), "absent zone"));
    let kept = rp.exists();
    let _ = fs::remove_dir_all(&root);
    println!("P8 error: {}", e.chars().take(300).collect::<String>());
    assert!(e.contains("zone 5") && kept);
    Ok(())
}

// Does `text` hold a cut line for every cut in `cuts`?
fn holds(text: &str, cuts: &[Cut]) -> bool {
    cuts.iter().all(|c| text.lines().any(|l|
        l == fmt!("cut {} {} {} {}", c.zone, c.fnum, c.data_upto, c.index_upto)))
}

/// B's F-B1.  `record` is set only while the record on disk holds every cut in the report, and
/// `refusal` only while the marker does, so `recorded()` never vouches for a cut written nowhere.
#[test]
fn b1_recorded_matches_the_record_on_disk() -> Outcome<()> {
    let _g = lock();
    let root = res!(fresh("./test_db_qa_r2a_b1"));
    let f = Fault::default();
    let e = err!("injected barrier failure"; Test);
    let r1 = f.raise(&root, vec![cut(1)], &e);
    assert!(r1.record.is_some() && r1.refusal.is_some(), "setup: both written");
    hooks::set_fault_record_fails_at(1);
    let r2 = f.raise(&root, vec![cut(2)], &e);
    hooks::set_fault_record_fails_at(0);
    let rec = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    let mk = fs::read_to_string(Refusal::path(&root)).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(r2.record.is_some(), holds(&rec, &r2.cuts),
        "record {:?} but the record on disk is {:?}", r2.record, rec);
    assert!(r2.refusal.is_none() || holds(&mk, &r2.cuts),
        "refusal {:?} but the marker on disk is {:?}", r2.refusal, mk);
    assert!(r2.recorded(), "the marker holds every cut, yet recorded() is false");
    Ok(())
}

/// B's I-B2.  A record left beside a store restored whole says what it is, where it is, and when
/// removing it is right.
#[test]
fn b2_a_left_over_record_names_itself() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_b2", 1, false));
    assert!(res!(Refusal::clear(&root)));
    res!(fs::remove_file(&report.cuts[0].data));
    let e = res!(refused(open(&root, &c, false), "left-over record"));
    let rp = root.join(constant::FAULT_FILENAME);
    let _ = fs::remove_dir_all(&root);
    assert!(e.contains(&fmt!("{:?}", rp)), "the error does not name {:?}: {}", rp, e);
    assert!(e.contains("left over from a stop"), "{}", e);
    assert!(e.contains("only if the store was restored whole from a copy taken before the stop"), "{}", e);
    Ok(())
}

/// F2.  With the record gone, the marker's cuts still cut the files in doubt.
#[test]
fn f2_cut_after_reads_the_marker() -> Outcome<()> {
    let _g = lock();
    let (root, c, report, _) = res!(stopped("./test_db_qa_r2a_f2", 1, false));
    let cut0 = report.cuts[0].clone();
    {
        let mut f = res!(fs::OpenOptions::new().append(true).open(&cut0.data));
        res!(f.write_all(&[7u8; 64]));
    }
    res!(fs::remove_file(root.join(constant::FAULT_FILENAME)));
    let zdirs = vec![c.zone_root(&root).join("zone_001")];
    res!(Fault::cut_after(&root, &zdirs));
    let n = res!(fs::metadata(&cut0.data)).len();
    let marker = Refusal::path(&root).exists();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(n, cut0.data_upto, "the marker's cut was not applied");
    assert!(marker, "cut_after removed the operator's marker");
    Ok(())
}

/// F5.  The marker and the record are plain text, without terminal colour escapes.
#[test]
fn f5_the_marker_and_record_are_plain() -> Outcome<()> {
    let _g = lock();
    let (root, _, _, _) = res!(stopped("./test_db_qa_r2a_f5", 1, false));
    let mk = res!(fs::read_to_string(Refusal::path(&root)));
    let rec = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    let _ = fs::remove_dir_all(&root);
    assert!(!mk.contains('\x1b') && !rec.contains('\x1b'), "escapes in {:?} or {:?}", mk, rec);
    assert!(mk.lines().any(|l| l.starts_with("why ") && l.len() > 10), "the marker does not say why: {:?}", mk);
    Ok(())
}

/// F3.  A start refused by the marker lets go of the store's lock, so the operator can clear it
/// while the refused process lives on.
#[test]
fn f3_a_refused_start_lets_go_of_the_lock() -> Outcome<()> {
    let _g = lock();
    let (root, c, _, _) = res!(stopped("./test_db_qa_r2a_f3", 1, false));
    let mut db = res!(O3db::new(root.clone(), Some(c.clone()), schemes(), Uid::default()));
    let started = db.start("qa_r2a");
    let cleared = Refusal::clear(&root);
    drop(db);
    let _ = fs::remove_dir_all(&root);
    assert!(started.is_err(), "the start was not refused");
    assert!(matches!(cleared, Ok(true)), "the clear was refused: {:?}", cleared.map_err(|e| fmt!("{}", e)));
    Ok(())
}
