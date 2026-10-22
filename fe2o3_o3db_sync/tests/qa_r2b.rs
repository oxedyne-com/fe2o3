//! QA round 2, agent B (2026-10-11): zone+fnum cut on a restored store and across zones whose
//! file numbers collide; the F3 rollover cut twice in one window, under a collection, and at the
//! seal.  Every test compares what was acknowledged against what reads back after the reopen.
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
    comm::response::Wait,
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
    verify,
};

use std::{
    collections::BTreeMap,
    fs,
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
    reset();
    g
}

fn reset() {
    hooks::set_barrier_failure(false);
    hooks::set_barrier_failure_for(None);
    hooks::set_fault_record_fails(false);
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_collect_delay(Duration::ZERO);
}

#[derive(Clone, Copy)]
enum Pol { Every, N(u32), Never }

fn cfg(zones: u16, p: Pol, max: u64) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = zones;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.sync_on_write         = matches!(p, Pol::Every);
    c.sync_every_n_writes   = match p { Pol::N(n) => n, _ => 0 };
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

fn zdir(root: &Path, c: &OzoneConfig, z: usize) -> PathBuf {
    c.zone_root(root).join(fmt!("zone_{:03}", z))
}

fn dat(zd: &Path, f: u32) -> PathBuf { zd.join(ZoneDir::relative_file_path(&FileType::Data, f)) }

// Every ozone file of a zone directory with its length.
fn lens(zd: &Path) -> Outcome<BTreeMap<String, u64>> {
    let mut m = BTreeMap::new();
    for e in res!(fs::read_dir(zd)) {
        let p = res!(e).path();
        if p.is_file() {
            m.insert(p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), len(&p));
        }
    }
    Ok(m)
}

fn data_total(zd: &Path) -> u64 {
    match lens(zd) {
        Ok(m) => m.iter().filter(|(n, _)| n.ends_with(".dat")).map(|(_, l)| *l).sum(),
        Err(_) => 0,
    }
}

fn last_fnum(zd: &Path) -> Option<u32> {
    (1..10_000u32).rev().find(|f| dat(zd, *f).exists())
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

// What each key was told: durable (proven by a barrier), acknowledged but not proven (under a
// lazy policy), and refused.  After the reopen a key reads its durable value or an acknowledged
// one (under EveryWrite, an acknowledged one if it has any), never only a refused one.
#[derive(Default)]
struct Book {
    every:      bool,
    durable:    BTreeMap<String, u8>,
    acked:      BTreeMap<String, Vec<u8>>,
    failed:     BTreeMap<String, Vec<u8>>,
}

impl Book {
    fn durable(&mut self, k: &str, s: u8) { self.durable.insert(k.to_string(), s); self.acked.remove(k); }
    fn acked(&mut self, k: &str, s: u8) {
        if self.every { self.durable(k, s); } else { self.acked.entry(k.to_string()).or_default().push(s); }
    }
    fn failed(&mut self, k: &str, s: u8) { self.failed.entry(k.to_string()).or_default().push(s); }
    // Everything acknowledged so far is now durable (a sync_now that succeeded).
    fn synced(&mut self) {
        let acked = std::mem::take(&mut self.acked);
        for (k, v) in acked { if let Some(s) = v.last() { self.durable.insert(k, *s); } }
    }
}

// (lost, resurrected, torn)
fn check(db: &TestDb, label: &str, b: &Book) -> Outcome<(usize, usize, usize)> {
    let mut keys: Vec<&String> = b.durable.keys().chain(b.acked.keys()).chain(b.failed.keys()).collect();
    keys.sort();
    keys.dedup();
    let (mut lost, mut risen, mut torn) = (0, 0, 0);
    for k in keys {
        let got = res!(reads(db, k));
        let acked = b.acked.get(k).cloned().unwrap_or_default();
        let allowed: Vec<u8> = b.durable.get(k).cloned().into_iter().chain(acked.iter().cloned()).collect();
        match got {
            None => if b.durable.contains_key(k) {
                msg!("{}: LOST {} (durable seed {:?})", label, k, b.durable.get(k));
                lost += 1;
            },
            Some(g) => if allowed.iter().any(|s| val(*s) == g) {
                // fine
            } else if b.failed.get(k).map(|f| f.iter().any(|s| val(*s) == g)).unwrap_or(false) {
                msg!("{}: RESURRECTED {} (refused value served; allowed {:?})", label, k, allowed);
                risen += 1;
            } else {
                msg!("{}: TORN {} (allowed {:?})", label, k, allowed);
                torn += 1;
            },
        }
    }
    Ok((lost, risen, torn))
}

// --- Surface 1 ---------------------------------------------------------------------------------

/// Two zones each hold a file 1.  The fault is in zone 2 only; zone 1's file 1, longer than zone
/// 2's mark, must come through the open untouched.
#[test]
fn colliding_file_numbers_cut_only_the_named_zone() -> Outcome<()> {
    let _g = lock();
    let c = res!(cfg(2, Pol::Every, 1_000_000));
    // Which zone each key lands in, learned from a throwaway store.
    let probe = res!(fresh("./test_db_qa_r2b_probe"));
    let mut z1 = Vec::new();
    let mut z2 = Vec::new();
    {
        let db = res!(open(&probe, &c, true));
        for i in 0..80 {
            let k = fmt!("collide {}", i);
            let (a, b) = (data_total(&zdir(&probe, &c, 1)), data_total(&zdir(&probe, &c, 2)));
            res!(put(&db, &k, 1));
            let (a2, b2) = (data_total(&zdir(&probe, &c, 1)), data_total(&zdir(&probe, &c, 2)));
            if a2 > a { z1.push(k); } else if b2 > b { z2.push(k); }
        }
        let _ = db.close();
    }
    let _ = fs::remove_dir_all(&probe);
    msg!("collide: {} keys in zone 1, {} in zone 2", z1.len(), z2.len());
    assert!(z1.len() > 10 && z2.len() > 3, "the probe did not split the keys");

    let root = res!(fresh("./test_db_qa_r2b_collide"));
    let db = res!(open(&root, &c, true));
    let mut bk = Book { every: true, ..Default::default() };
    for (i, k) in z1.iter().enumerate() {
        res!(put(&db, k, 10 + i as u8));
        bk.durable(k, 10 + i as u8);
    }
    for k in &z2[0..2] {
        res!(put(&db, k, 3));
        bk.durable(k, 3);
    }
    hooks::set_barrier_failure(true);
    let lost = put(&db, &z2[2], 4);
    hooks::set_barrier_failure(false);
    assert!(lost.is_err(), "the failed write was confirmed");
    bk.failed(&z2[2], 4);
    let report = res!(db.api().fault().ok_or_else(|| err!("No fault."; Test)));
    let _ = db.close();
    msg!("collide cuts: {:?}", report.cuts.iter().map(|c| (c.zone, c.fnum, c.data_upto, c.index_upto)).collect::<Vec<_>>());
    assert!(report.cuts.iter().all(|c| c.zone == 1), "a cut names zone 1: {:?}", report.cuts);
    let z1dir = zdir(&root, &c, 1);
    for cut in &report.cuts {
        assert!(len(&dat(&z1dir, cut.fnum)) > cut.data_upto,
            "precondition: zone 1's file {} is not longer than zone 2's mark", cut.fnum);
    }
    let before = res!(lens(&z1dir));
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    let t = res!(check(&db, "collide", &bk));
    res!(db.close());
    assert_eq!(t, (0, 0, 0), "collide: (lost, resurrected, torn) {:?}", t);
    let after = res!(lens(&z1dir));
    for (n, b) in &before {
        if n.ends_with(".dat") || n.ends_with(".ind") {
            assert_eq!(after.get(n), Some(b), "zone 1's {} changed in the open: {} to {:?}", n, b, after.get(n));
        }
    }
    let z2dir = zdir(&root, &c, 2);
    for cut in &report.cuts {
        assert!(len(&dat(&z2dir, cut.fnum)) <= cut.data_upto, "zone 2's file {} was not cut", cut.fnum);
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A store restored from an older backup under its newer record: (a) the file in doubt is in
/// the backup, shorter than its mark, and the open is a no-op cut; (b) the backup predates the
/// file in doubt, and the open refuses.  Then the operator restores the whole backup over the
/// root: does the store open?
#[test]
fn a_restore_from_an_older_backup() -> Outcome<()> {
    let _g = lock();
    let c = res!(cfg(1, Pol::Every, 1_000_000));
    let root = res!(fresh("./test_db_qa_r2b_restore"));
    let early = res!(fresh("./test_db_qa_r2b_restore_early"));
    let late = res!(fresh("./test_db_qa_r2b_restore_late"));
    let zd = zdir(&root, &c, 1);
    let mut db = res!(open(&root, &c, true));
    let mut bk = Book { every: true, ..Default::default() };
    for i in 0..5 { res!(put(&db, &fmt!("a {}", i), i + 1)); bk.durable(&fmt!("a {}", i), i + 1); }
    let _ = db.close();
    res!(copy_tree(&root, &early)); // before the file in doubt existed
    db = res!(open(&root, &c, false));
    res!(db.api().new_live_files());
    for i in 0..5 { res!(put(&db, &fmt!("b {}", i), i + 20)); bk.durable(&fmt!("b {}", i), i + 20); }
    let _ = db.close();
    res!(copy_tree(&root, &late)); // the file in doubt there, shorter than its mark
    db = res!(open(&root, &c, false));
    for i in 0..5 { res!(put(&db, &fmt!("c {}", i), i + 40)); }
    hooks::set_barrier_failure(true);
    assert!(put(&db, "lost", 99).is_err());
    hooks::set_barrier_failure(false);
    let report = res!(db.api().fault().ok_or_else(|| err!("No fault."; Test)));
    let _ = db.close();
    msg!("restore cuts {:?}, files now {:?}", report.cuts.iter().map(|c| (c.fnum, c.data_upto)).collect::<Vec<_>>(), res!(lens(&zd)));
    let record = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));

    // (a) late backup under the record.
    let a = res!(fresh("./test_db_qa_r2b_restore_a"));
    res!(copy_tree(&late, &a));
    res!(fs::write(a.join(constant::FAULT_FILENAME), &record));
    let alens = res!(lens(&zdir(&a, &c, 1)));
    let db = res!(open(&a, &c, false));
    bk.failed("lost", 99);
    let t = res!(check(&db, "restore a", &bk));
    for i in 0..5 { assert_eq!(res!(reads(&db, &fmt!("c {}", i))), None, "a key newer than the backup appeared"); }
    res!(db.close());
    assert_eq!(t, (0, 0, 0), "restore a: (lost, resurrected, torn) {:?}", t);
    let alens2 = res!(lens(&zdir(&a, &c, 1)));
    for (n, b) in &alens {
        if n.ends_with(".dat") || n.ends_with(".ind") {
            assert_eq!(alens2.get(n), Some(b), "restore a: {} changed", n);
        }
    }

    // (b) early backup under the record and the marker.
    let b = res!(fresh("./test_db_qa_r2b_restore_b"));
    res!(copy_tree(&early, &b));
    res!(fs::write(b.join(constant::FAULT_FILENAME), &record));
    match open(&b, &c, false) {
        Ok(db)  => { let _ = db.close(); msg!("restore b: OPENED over a record naming a file it lacks"); },
        Err(e)  => msg!("restore b: refused: {}", fmt!("{}", e).chars().take(600).collect::<String>()),
    }
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&early);
    let _ = fs::remove_dir_all(&late);
    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
    Ok(())
}

// --- Surface 2 ---------------------------------------------------------------------------------

// Fills the live file during a held, failing barrier, so the writer rolls over, possibly more
// than once.  Returns the store's root, its config, the first and last live files, the ack map,
// the failed map and the fault.
fn rollover_window(dir: &str, p: Pol, max: u64, nputs: usize, overwrite: bool, collect: bool)
    -> Outcome<()>
{
    let label = dir.to_string();
    let root = res!(fresh(dir));
    let c = res!(cfg(1, p, max));
    let zd = zdir(&root, &c, 1);
    let db = Arc::new(res!(open(&root, &c, true)));
    let mut bk = Book { every: matches!(p, Pol::Every), ..Default::default() };
    // Several files, with old versions for a collection to compact.
    for round in 0..3u8 {
        for i in 0..10 {
            res!(put(&db, &fmt!("k {}", i), round * 10 + i as u8 + 1));
            bk.acked(&fmt!("k {}", i), round * 10 + i as u8 + 1);
        }
        res!(db.api().sync_now(wait()));
        bk.synced();
    }
    let first = res!(last_fnum(&zd).ok_or_else(|| err!("No file."; Test)));
    res!(put(&db, "dirty", 9));
    bk.acked("dirty", 9);
    let compactor = if collect {
        hooks::set_collect_delay(Duration::from_millis(600));
        let dbc = db.clone();
        let h = std::thread::spawn(move || dbc.compact_now(Duration::from_secs(30)).map(|r| fmt!("{:?}", r)));
        std::thread::sleep(Duration::from_millis(50));
        Some(h)
    } else { None };
    hooks::set_barrier_delay(Duration::from_millis(800));
    hooks::set_barrier_failure(true);
    let db2 = db.clone();
    let sync = std::thread::spawn(move || db2.api().sync_now(wait()));
    std::thread::sleep(Duration::from_millis(100));
    let puts: Vec<_> = (0..nputs).map(|i| {
        let db3 = db.clone();
        let k = if overwrite && i % 2 == 0 { fmt!("k {}", (i / 2) % 10) } else { fmt!("during {}", i) };
        let s = 100 + i as u8;
        std::thread::spawn(move || (k.clone(), s, put(&db3, &k, s)))
    }).collect();
    let synced = sync.join();
    let mut acked_during = 0;
    let mut nfailed = 0;
    for p in puts {
        if let Ok((k, s, r)) = p.join() {
            match r {
                Ok(())  => { bk.acked(&k, s); acked_during += 1; },
                Err(_)  => { bk.failed(&k, s); nfailed += 1; },
            }
        }
    }
    reset();
    let gc = compactor.map(|h| h.join());
    msg!("{}: sync {:?}, {} during acked, {} failed, compaction {:?}", label,
        synced.as_ref().map(|r| r.is_ok()), acked_during, nfailed, gc.map(|g| g.map(|r| r.is_ok())));
    let last = res!(last_fnum(&zd).ok_or_else(|| err!("No file."; Test)));
    let report = res!(db.api().fault().ok_or_else(|| err!("No fault."; Test)));
    // Let any late raise land.
    let _ = wait_until(Duration::from_secs(5), || db.api().fault()
        .map(|r| (first + 1..=last).all(|f| r.cuts.iter().any(|c| c.fnum == f))).unwrap_or(false));
    let report2 = res!(db.api().fault().ok_or_else(|| err!("No fault."; Test)));
    msg!("{}: files {}..{}, cuts {:?} (first look {})", label, first, last,
        report2.cuts.iter().map(|c| (c.fnum, c.data_upto, c.index_upto)).collect::<Vec<_>>(), report.cuts.len());
    match Arc::try_unwrap(db) {
        Ok(db)  => { let _ = db.close(); },
        Err(_)  => return Err(err!("{}: the store is still shared.", label; Test)),
    }
    let record = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    let pre = res!(lens(&zd));
    msg!("{}: record:\n{}files before reopen {:?}", label, record, pre);
    let mut missing = Vec::new();
    for f in (first + 1)..=last {
        if dat(&zd, f).exists() && !record.lines().any(|l| l.starts_with(&fmt!("cut 0 {} ", f))) {
            missing.push(f);
        }
    }
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    let t1 = res!(check(&db, &label, &bk));
    let v = res!(verify::verify_live_set(db.api(), None, wait()));
    res!(db.close());
    for cut in &report2.cuts {
        assert!(len(&cut.data) <= cut.data_upto && len(&cut.index) <= cut.index_upto,
            "{}: file {} past its mark ({}, {}): ({}, {})", label, cut.fnum, cut.data_upto,
            cut.index_upto, len(&cut.data), len(&cut.index));
    }
    // Round trip: a write after the cut, a second reopen.
    let db = res!(open(&root, &c, false));
    res!(put(&db, "after", 77));
    res!(db.close());
    let db = res!(open(&root, &c, false));
    let t2 = res!(check(&db, &label, &bk));
    let after = res!(reads(&db, "after"));
    res!(db.close());
    assert!(missing.is_empty(), "{}: files rolled over to but not in the record: {:?}", label, missing);
    assert!(v.clean(), "{}: {}", label, v.summary(4));
    assert_eq!((t1, t2), ((0, 0, 0), (0, 0, 0)), "{}: (lost, resurrected, torn) first reopen {:?}, second {:?}", label, t1, t2);
    assert_eq!(after, Some(val(77)), "{}: the write after the cut was lost", label);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn rollover_twice_never() -> Outcome<()> {
    let _g = lock();
    let r = rollover_window("./test_db_qa_r2b_roll2_never", Pol::Never, 2_000, 60, false, false);
    reset();
    r
}

#[test]
fn rollover_twice_every_write_overwrites() -> Outcome<()> {
    let _g = lock();
    let r = rollover_window("./test_db_qa_r2b_roll2_every", Pol::Every, 2_000, 60, true, false);
    reset();
    r
}

#[test]
fn rollover_under_collection_every_write() -> Outcome<()> {
    let _g = lock();
    let r = rollover_window("./test_db_qa_r2b_rollgc_every", Pol::Every, 2_000, 60, true, true);
    reset();
    r
}

#[test]
fn rollover_under_collection_every_n() -> Outcome<()> {
    let _g = lock();
    let r = rollover_window("./test_db_qa_r2b_rollgc_n", Pol::N(1000), 2_000, 60, true, true);
    reset();
    r
}

/// The first barrier to fail is the seal's: EveryN with a large N runs none per record, so the
/// rollover's own barrier over the old file is the one that fails.
#[test]
fn a_fault_exactly_at_the_seal() -> Outcome<()> {
    let _g = lock();
    let root = res!(fresh("./test_db_qa_r2b_seal"));
    let c = res!(cfg(1, Pol::N(100_000), 2_000));
    let zd = zdir(&root, &c, 1);
    let db = res!(open(&root, &c, true));
    let mut bk = Book::default();
    for i in 0..5 { res!(put(&db, &fmt!("s {}", i), i + 1)); bk.acked(&fmt!("s {}", i), i + 1); }
    res!(db.api().sync_now(wait()));
    bk.synced();
    let n = res!(last_fnum(&zd).ok_or_else(|| err!("No file."; Test)));
    let mark = len(&dat(&zd, n));
    hooks::set_barrier_failure_for(Some(n));
    let mut acked_unsynced = BTreeMap::new();
    let mut nfailed = 0;
    for i in 0..60u8 {
        let k = fmt!("t {}", i);
        match put(&db, &k, 50 + i) {
            Ok(())  => { acked_unsynced.insert(k.clone(), 50 + i); bk.acked(&k, 50 + i); },
            Err(_)  => { bk.failed(&k, 50 + i); nfailed += 1; },
        }
    }
    hooks::set_barrier_failure_for(None);
    let report = res!(db.api().fault().ok_or_else(|| err!("The seal's failed barrier raised no fault."; Test)));
    let last = res!(last_fnum(&zd).ok_or_else(|| err!("No file."; Test)));
    let _ = db.close();
    let record = res!(fs::read_to_string(root.join(constant::FAULT_FILENAME)));
    msg!("seal: file {} mark {}, last {}, {} acked unsynced, {} failed, cuts {:?}\nrecord:\n{}", n, mark, last,
        acked_unsynced.len(), nfailed,
        report.cuts.iter().map(|c| (c.fnum, c.data_upto, c.index_upto)).collect::<Vec<_>>(), record);
    assert!(last > n, "no rollover happened");
    assert!(record.lines().any(|l| l.starts_with(&fmt!("cut 0 {} ", n))), "the sealed file is not in the record");
    for f in (n + 1)..=last {
        assert!(record.lines().any(|l| l.starts_with(&fmt!("cut 0 {} ", f))), "file {} is not in the record", f);
    }
    assert!(res!(Refusal::clear(&root)));
    let db = res!(open(&root, &c, false));
    let t = res!(check(&db, "seal", &bk));
    let mut survived = 0;
    for (k, s) in &acked_unsynced {
        if res!(reads(&db, k)) == Some(val(*s)) { survived += 1; }
    }
    res!(db.close());
    msg!("seal: {} of {} acked-but-unsynced (EveryN) survived the cut", survived, acked_unsynced.len());
    assert_eq!(t, (0, 0, 0), "seal: (lost, resurrected, torn) {:?}", t);
    assert!(len(&dat(&zd, n)) <= mark.max(report.cuts.iter().find(|c| c.fnum == n).map(|c| c.data_upto).unwrap_or(0)),
        "file {} is past its mark", n);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
