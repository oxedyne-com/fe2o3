//! The durable mark (Ozone, 2026-10-10).  A discard waits for its winner's barrier, a failed
//! barrier stops the store for good, and the next open cuts every file in doubt back to its mark.
//! Each test that can runs under every sync policy, and under two zones of two writers.  The
//! power cut is modelled in process: the store is copied as its files stand, every live file of
//! the copy is cut to its published mark, and the copy is opened.  The hooks are process-wide,
//! so the tests run one at a time.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    Meta,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    data::cache::{
        CacheEntry,
        is_chunk_key,
    },
    file::{
        core::FileType,
        refusal::Refusal,
        zdir::ZoneDir,
    },
    file::state::DataState,
    sweep,
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
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
    sync::Mutex,
    time::{
        Duration,
        Instant,
    },
};

static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug)]
enum Policy {
    EveryWrite,
    EveryN(u32),
    Interval(u64),
    Never,
}

const POLICIES: [Policy; 4] = [
    Policy::EveryWrite,
    Policy::EveryN(1_000),
    Policy::Interval(3_600_000),
    Policy::Never,
];

const SHAPES: [(u16, u16); 2] = [(1, 1), (2, 2)]; // zones, writers per zone

fn cfg(p: Policy, zones: u16, wbots: u16) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = zones;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = wbots;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = false;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = 0;
    match p {
        Policy::EveryWrite  => c.sync_on_write = true,
        Policy::EveryN(n)   => c.sync_every_n_writes = n,
        Policy::Interval(t) => c.sync_interval_ms = t,
        Policy::Never       => (),
    }
    Ok(c)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    let g = match SERIAL.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_barrier_failure(false);
    hooks::set_barrier_failure_for(None);
    hooks::set_fault_record_fails(false);
    hooks::set_owed_drop(false);
    g
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
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

fn put(db: &TestDb, k: &Dat, v: Dat) -> Outcome<()> {
    db.insert(k.clone(), v, Uid::default(), None).map(|_| ())
}

// "L/S" for a whole value of length L and seed S as `val` writes it, else what was read.
fn describe(r: &Outcome<Option<(Dat, Meta<{ UID_LEN }, Uid>)>>) -> String {
    match r {
        Ok(None) => fmt!("none"),
        Ok(Some((Dat::BU32(b), _))) | Ok(Some((Dat::BU8(b), _))) | Ok(Some((Dat::BU16(b), _))) |
        Ok(Some((Dat::BU64(b), _))) if !b.is_empty()
            && b.iter().enumerate().all(|(j, x)| *x == b[0] ^ (j as u8)) => fmt!("{}/{}", b.len(), b[0]),
        Ok(Some((d, _))) => fmt!("TORN:{:?}", d).chars().take(100).collect(),
        Err(e) => fmt!("ERR:{}", fmt!("{}", e).chars().take(200).collect::<String>()),
    }
}

fn read(db: &TestDb, k: &Dat) -> String {
    describe(&db.get(k, None))
}

fn head(db: &TestDb, k: &Dat) -> Outcome<Option<[u64; 5]>> {
    match res!(db.api().get_head_wait(k, None)) {
        Some((Dat::Tup5u64(t), _)) => Ok(Some(t)),
        _ => Ok(None),
    }
}

// Are all the chunks of the set `t` still readable?
fn chunks_live(db: &TestDb, t: [u64; 5]) -> bool {
    db.api().fetch_chunks(&Dat::Tup5u64(t), None).is_ok()
}

fn zdir(root: &Path, c: &OzoneConfig, z: usize) -> PathBuf {
    c.zone_root(root).join(fmt!("zone_{:03}", z + 1))
}

/// A power cut now: the store is imaged at one instant (`gc_pair::instant_cut`), and each
/// live file of the image is cut to the mark its last barrier published.
fn power_cut(db: &TestDb, root: &Path, c: &OzoneConfig, to: &str) -> Outcome<PathBuf> {
    let dst = res!(fresh(to));
    res!(instant_cut(db, root, c, &dst));
    Ok(dst)
}

// Reads `keys` from a store cut at a power cut, and closes it.
fn read_after_cut(db: &TestDb, root: &Path, c: &OzoneConfig, to: &str, keys: &[Dat]) -> Outcome<Vec<String>> {
    let cut = res!(power_cut(db, root, c, to));
    let db2 = res!(open(&cut, c, false));
    let got = keys.iter().map(|k| read(&db2, k)).collect();
    res!(db2.close());
    let _ = fs::remove_dir_all(&cut);
    Ok(got)
}

/// Live chunk-key cache entries, those that are not tombstones.
// The cache bot answers before it enters the hold, so a positive control waits for it.
fn held_soon(db: &TestDb) -> bool {
    wait_until(Duration::from_secs(1), || db.api().held_effects() > 0)
}

fn live_chunks(db: &TestDb) -> Outcome<usize> {
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let mut n = 0;
    for (_, cache) in res!(db.api().collect_caches(wait())) {
        let tombs = cache.tomb_tracker().tombs();
        for (k, e) in cache.map() {
            let CacheEntry::LocatedValue(_, _) = e;
            if is_chunk_key(k) && !tombs.contains_key(k) {
                n += 1;
            }
        }
    }
    Ok(n)
}

// Fails every barrier from now, makes one over a new record, and lets the switch go.  The store
// is stopped after.
fn fail_a_barrier(db: &TestDb) -> Outcome<()> {
    res!(put(db, &dat!("durable mark uncovered"), val(50, 7)));
    hooks::set_barrier_failure(true);
    let r = db.api().sync_now(wait());
    hooks::set_barrier_failure(false);
    match r {
        Err(_)  => Ok(()),
        Ok(())  => Err(err!("A barrier armed to fail succeeded."; Test, Unexpected)),
    }
}

/// P, a durable chunked value, is displaced by U, whose barrier fails.  Nothing P owns is
/// retired or collected, the stopped store refuses compaction and the sweep, and the restart
/// cuts U, so P is the key's value again.  `small` makes P unchunked.
fn displaced_by_a_failed_write(p: Policy, zones: u16, wbots: u16, small: bool, collect: bool) -> Outcome<()> {
    let label = fmt!("{:?} {}x{} small={} collect={}", p, zones, wbots, small, collect);
    let c = res!(cfg(p, zones, wbots));
    let root = res!(fresh("./test_db_durable_mark_p"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark p");
    let pv = if small { val(200, 2) } else { val(1_500, 2) };
    let want = if small { "200/2" } else { "1500/2" };
    res!(put(&db, &k, pv));
    res!(db.api().sync_now(wait()));
    let set = res!(head(&db, &k));
    // U's own barrier fails under `EveryWrite`; otherwise the forced one does.
    hooks::set_barrier_failure(true);
    let u = put(&db, &k, val(50, 3));
    let _ = db.api().sync_now(wait());
    hooks::set_barrier_failure(false);
    assert!(db.api().fault().is_some(), "{}: a failed barrier did not stop the store", label);
    if let Policy::EveryWrite = p {
        assert!(u.is_err(), "{}: U was acknowledged although its barrier failed", label);
    }
    if collect {
        assert!(db.compact_now(Duration::from_secs(60)).is_err(), "{}: a stopped store compacted", label);
        assert!(sweep::sweep_orphans(db.api(), Uid::default(), None, wait()).is_err(),
            "{}: a stopped store swept", label);
    }
    if let Some(t) = set {
        assert!(chunks_live(&db, t), "{}: P's chunks were retired behind a failed barrier", label);
    }
    let _ = db.close();
    res!(Refusal::clear(&root)); // as the operator would
    let db = res!(open(&root, &c, false));
    assert_eq!(read(&db, &k), want, "{}: the restart did not give P back", label);
    assert!(db.api().fault().is_none(), "{}: the restart left the fault", label);
    let v = res!(verify::verify_live_set(db.api(), None, wait()));
    assert!(v.clean(), "{}: {}", label, v.summary(4));
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn a_head_displaced_by_a_failed_write_keeps_its_value() -> Outcome<()> {
    let _g = lock();
    for p in POLICIES {
        for (z, w) in SHAPES {
            res!(displaced_by_a_failed_write(p, z, w, false, false));
            res!(displaced_by_a_failed_write(p, z, w, false, true));
        }
    }
    res!(displaced_by_a_failed_write(Policy::Interval(3_600_000), 1, 1, true, true));
    res!(displaced_by_a_failed_write(Policy::Interval(3_600_000), 2, 2, true, true));
    Ok(())
}

/// With no fault, P's discard waits for Q's barrier: compaction does not drop P while Q may yet
/// be lost, so a power cut leaves P.  Once Q is durable the late discard is applied through the
/// collection that carried P, and marks nothing else old.
#[test]
fn collection_waits_for_the_winners_barrier() -> Outcome<()> {
    let _g = lock();
    hooks::set_owed_drop(true); // the hold's window stays open until the barrier under test
    for (z, w) in SHAPES {
        let label = fmt!("{}x{}", z, w);
        let mut c = res!(cfg(Policy::Interval(3_600_000), z, w));
        c.data_file_max_bytes = 4_000;
        let root = res!(fresh("./test_db_durable_mark_collect"));
        let db = res!(open(&root, &c, true));
        let k = dat!("durable mark collect");
        let others: Vec<Dat> = (0..8).map(|i| dat!(fmt!("durable mark other {}", i))).collect();
        res!(put(&db, &k, val(300, 2)));
        for (i, o) in others.iter().enumerate() {
            res!(put(&db, o, val(300, 10 + i as u8)));
        }
        res!(db.api().sync_now(wait()));
        res!(db.api().new_live_files());
        res!(put(&db, &k, val(300, 4)));
        assert!(held_soon(&db), "{}: P's discard was not held behind Q's barrier", label);
        let got = res!(read_after_cut(&db, &root, &c, "./test_db_durable_mark_collect_cut", &[k.clone()]));
        assert_eq!(got[0], "300/2", "{}: a power cut before Q's barrier did not give P back", label);
        // Compaction seals the live file first, which makes Q durable; P may go only after that.
        let _ = db.compact_now(Duration::from_secs(60));
        let got = res!(read_after_cut(&db, &root, &c, "./test_db_durable_mark_collect_cut", &[k.clone()]));
        assert!(got[0] == "300/2" || got[0] == "300/4",
            "{}: a power cut after compaction lost P under Q: {}", label, got[0]);
        res!(db.api().sync_now(wait()));
        assert!(wait_until(Duration::from_secs(10), || db.api().held_effects() == 0),
            "{}: the hold outlived Q's barrier", label);
        res!(db.compact_now(Duration::from_secs(60)));
        assert_eq!(read(&db, &k), "300/4", "{}", label);
        for (i, o) in others.iter().enumerate() {
            assert_eq!(read(&db, o), fmt!("300/{}", 10 + i), "{}: the late discard hit another record", label);
        }
        let v = res!(verify::verify_live_set(db.api(), None, wait()));
        assert!(v.clean(), "{}: {}", label, v.summary(4));
        res!(db.close());
        let db = res!(open(&root, &c, false));
        for (i, o) in others.iter().enumerate() {
            assert_eq!(read(&db, o), fmt!("300/{}", 10 + i), "{}: after the restart", label);
        }
        res!(db.close());
        let _ = fs::remove_dir_all(&root);
    }
    Ok(())
}

/// A hold lasts until the barrier the policy makes, and no longer.
#[test]
fn holds_release_at_each_policys_barrier() -> Outcome<()> {
    let _g = lock();
    let k = dat!("durable mark hold");
    let quiet = |db: &TestDb| wait_until(Duration::from_secs(10), || db.api().held_effects() == 0);

    let c = res!(cfg(Policy::EveryWrite, 1, 1));
    let root = res!(fresh("./test_db_durable_mark_hold"));
    let db = res!(open(&root, &c, true));
    res!(put(&db, &k, val(50, 1)));
    res!(put(&db, &k, val(50, 2)));
    assert!(quiet(&db), "EveryWrite: a hold outlived its write's own barrier");
    res!(db.close());

    let c = res!(cfg(Policy::EveryN(5), 1, 1));
    let db = res!(open(&root, &c, true));
    res!(put(&db, &k, val(50, 1)));
    res!(db.api().sync_now(wait()));
    res!(put(&db, &k, val(50, 2)));
    assert!(held_soon(&db), "EveryN(5): released before the fifth write");
    for i in 0..4 {
        res!(put(&db, &dat!(fmt!("durable mark filler {}", i)), val(50, 9)));
    }
    assert!(quiet(&db), "EveryN(5): neither the fifth write's barrier nor the owed one released the hold");
    res!(db.close());

    let c = res!(cfg(Policy::Never, 1, 1));
    let db = res!(open(&root, &c, true));
    res!(put(&db, &k, val(50, 1)));
    res!(db.api().sync_now(wait()));
    res!(put(&db, &k, val(50, 2)));
    assert!(held_soon(&db), "Never: released with no barrier");
    res!(db.api().new_live_files());
    assert!(quiet(&db), "Never: sealing the live file did not release the hold");
    res!(put(&db, &k, val(50, 3)));
    assert!(held_soon(&db), "Never: released with no barrier");
    res!(db.api().sync_now(wait()));
    assert!(quiet(&db), "Never: sync_now did not release the hold");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A failed barrier is never retried into success (fsyncgate): the marks stay where they were,
/// the holds stay held, and nothing more is written, whatever the disk does after.
#[test]
fn a_failed_barrier_gets_no_second_chance() -> Outcome<()> {
    let _g = lock();
    hooks::set_owed_drop(true); // the hold's window stays open until the barrier under test
    let c = res!(cfg(Policy::Interval(3_600_000), 1, 1));
    let root = res!(fresh("./test_db_durable_mark_fsyncgate"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark fsyncgate");
    res!(put(&db, &k, val(1_500, 2)));
    res!(db.api().sync_now(wait()));
    let _ = put(&db, &k, val(1_500, 4));
    let _ = held_soon(&db);
    let held = db.api().held_effects();
    assert!(held > 0, "positive control: the overwrite's discard is held");
    res!(fail_a_barrier(&db));
    let marks = res!(db.api().chans().marks().files(0));
    assert!(put(&db, &dat!("durable mark after"), val(50, 1)).is_err(), "a write after the fault was taken");
    assert!(db.api().sync_now(wait()).is_err(), "a barrier after the fault succeeded");
    let _ = db.api().new_live_files();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(res!(db.api().chans().marks().files(0)), marks, "a mark moved after the fault");
    assert_eq!(db.api().held_effects(), held, "a hold was released after the fault");
    let _ = db.close();
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// After a fault, writes, compaction and the sweep are refused with the advice to restart, a
/// read of what a barrier covered is served, and the fault is recorded for the next open.
#[test]
fn a_fault_stops_writes() -> Outcome<()> {
    let _g = lock();
    let c = res!(cfg(Policy::Interval(3_600_000), 1, 1));
    let root = res!(fresh("./test_db_durable_mark_stops"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark stops");
    res!(put(&db, &k, val(1_500, 2)));
    res!(db.api().sync_now(wait()));
    res!(fail_a_barrier(&db));
    let advice = |r: Outcome<()>, what: &str| match r {
        Ok(())  => panic!("{} was taken by a stopped store", what),
        Err(e)  => assert!(fmt!("{}", e).contains("Restart the store"), "{}: {}", what, e),
    };
    advice(put(&db, &dat!("durable mark new"), val(50, 1)), "a put");
    advice(db.delete(&k, Uid::default(), None).map(|_| ()), "a delete");
    advice(sweep::sweep_orphans(db.api(), Uid::default(), None, wait()).map(|_| ()), "the sweep");
    assert!(db.compact_now(Duration::from_secs(60)).is_err(), "compaction ran on a stopped store");
    assert_eq!(read(&db, &k), "1500/2", "a covered read was refused");
    assert!(db.api().fault().is_some());
    assert!(root.join(constant::FAULT_FILENAME).exists(), "no fault record was written");
    let _ = db.close();
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The next open cuts every file in doubt to its mark, keeps the tail in quarantine and clears
/// the fault record.  A second open over the same record changes no length.
#[test]
fn the_open_cuts_to_the_marks() -> Outcome<()> {
    let _g = lock();
    hooks::set_owed_drop(true); // the hold's window stays open until the barrier under test
    let c = res!(cfg(Policy::Interval(3_600_000), 1, 1));
    let root = res!(fresh("./test_db_durable_mark_cut"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark cut");
    res!(put(&db, &k, val(300, 2)));
    res!(db.api().sync_now(wait()));
    let _ = put(&db, &k, val(300, 4));
    let marks = res!(db.api().chans().marks().files(0));
    res!(fail_a_barrier(&db));
    let _ = db.close();
    let fault = res!(fs::read(root.join(constant::FAULT_FILENAME)));
    let lens = |root: &Path| -> Vec<(u64, u64)> {
        marks.iter().map(|(f, _)| (
            size(&zdir(root, &c, 0).join(ZoneDir::relative_file_path(&FileType::Data, *f))),
            size(&zdir(root, &c, 0).join(ZoneDir::relative_file_path(&FileType::Index, *f))),
        )).collect()
    };
    // The fault cut the tails in process; a crash before that cut would leave them, so one is
    // put back for the open to find.
    for (f, _) in &marks {
        let p = zdir(&root, &c, 0).join(ZoneDir::relative_file_path(&FileType::Data, *f));
        let mut b = res!(fs::read(&p));
        b.extend_from_slice(&[0xab; 64]);
        res!(fs::write(&p, &b));
    }
    let before = lens(&root);
    assert!(marks.iter().zip(&before).any(|((_, m), l)| l.0 > m.0), "positive control: nothing past a mark");
    res!(Refusal::clear(&root)); // as the operator would
    let db = res!(open(&root, &c, false));
    assert_eq!(read(&db, &k), "300/2");
    res!(db.close());
    let after = lens(&root);
    for ((f, m), l) in marks.iter().zip(&after) {
        assert_eq!(*l, *m, "file {} was not cut to its mark", f);
    }
    assert!(!root.join(constant::FAULT_FILENAME).exists(), "the fault record outlived the cut");
    let q = zdir(&root, &c, 0).join(constant::QUARANTINE_DIR);
    assert!(res!(fs::read_dir(&q)).count() > 0, "the cut tail was not quarantined");
    let held = |q: &Path| -> Outcome<Vec<(PathBuf, u64)>> {
        let mut v = Vec::new();
        for e in res!(fs::read_dir(q)) {
            let p = res!(e).path();
            v.push((p.clone(), size(&p)));
        }
        v.sort();
        Ok(v)
    };
    let evidence = res!(held(&q));
    res!(fs::write(root.join(constant::FAULT_FILENAME), &fault));
    let db = res!(open(&root, &c, false));
    assert_eq!(read(&db, &k), "300/2");
    res!(db.close());
    assert_eq!(lens(&root), after, "a second cut over a replanted fault record changed a length");
    // A file already at its mark has nothing to cut, so the first cut's evidence stands.
    assert_eq!(res!(held(&q)), evidence, "a second cut rewrote the quarantined tails");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A fault record that cannot be written still stops the store in process.
#[test]
fn a_fault_stops_the_store_without_its_record() -> Outcome<()> {
    let _g = lock();
    let c = res!(cfg(Policy::Interval(3_600_000), 1, 1));
    let root = res!(fresh("./test_db_durable_mark_norecord"));
    let db = res!(open(&root, &c, true));
    res!(put(&db, &dat!("durable mark norecord"), val(50, 1)));
    hooks::set_fault_record_fails(true);
    let r = fail_a_barrier(&db);
    hooks::set_fault_record_fails(false);
    res!(r);
    assert!(db.api().fault().is_some(), "the store did not stop");
    assert!(put(&db, &dat!("durable mark norecord 2"), val(50, 1)).is_err(), "a write was taken");
    let _ = db.close();
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A chunked put returns only once a barrier covers its chunks, so a power cut that keeps its
/// head keeps the whole value (R1).  The chunks and the head may go to different writers.
#[test]
fn a_head_never_outlives_its_chunks() -> Outcome<()> {
    let _g = lock();
    for p in [Policy::EveryN(1_000), Policy::Interval(3_600_000), Policy::Never] {
        let c = res!(cfg(p, 2, 2));
        let root = res!(fresh("./test_db_durable_mark_r1"));
        let db = res!(open(&root, &c, true));
        let keys: Vec<Dat> = (0..6).map(|i| dat!(fmt!("durable mark r1 {}", i))).collect();
        for k in &keys {
            res!(put(&db, k, val(1_500, 2)));
        }
        res!(db.api().sync_now(wait()));
        for k in &keys {
            res!(put(&db, k, val(1_300, 4)));
        }
        let got = res!(read_after_cut(&db, &root, &c, "./test_db_durable_mark_r1_cut", &keys));
        for g in got {
            assert!(g == "1500/2" || g == "1300/4", "{:?}: a power cut left {}", p, g);
        }
        res!(db.close());
        let _ = fs::remove_dir_all(&root);
    }
    Ok(())
}

/// The sweep makes its scan durable before it retires, which releases what the scan saw; a
/// barrier that fails there stops the sweep with nothing retired.
#[test]
fn the_sweep_syncs_after_its_scan() -> Outcome<()> {
    let _g = lock();
    hooks::set_owed_drop(true); // the hold's window stays open until the barrier under test
    let c = res!(cfg(Policy::Interval(3_600_000), 1, 1));
    let root = res!(fresh("./test_db_durable_mark_sweep"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark sweep");
    res!(put(&db, &k, val(1_500, 2)));
    res!(db.api().sync_now(wait()));
    let first = res!(res!(head(&db, &k)).ok_or_else(|| err!("No head."; Test, Missing)));
    res!(put(&db, &k, val(1_500, 4)));
    assert!(held_soon(&db), "positive control: the discard is held");
    assert!(chunks_live(&db, first), "positive control: P's chunks are live before the sweep");
    res!(sweep::sweep_orphans(db.api(), Uid::default(), None, wait()));
    // The retires the sweep's barrier releases are themselves written after it, so they are
    // held in turn: what is asserted is that P's displaced chunk set was retired.
    assert!(wait_until(Duration::from_secs(10), || !chunks_live(&db, first)),
        "the sweep's barrier did not release the retire of P's chunks");
    res!(db.close());

    let db = res!(open(&root, &c, true));
    res!(put(&db, &k, val(1_500, 2)));
    res!(db.api().sync_now(wait()));
    let old = res!(res!(head(&db, &k)).ok_or_else(|| err!("No head."; Test, Missing)));
    res!(put(&db, &k, val(1_500, 4)));
    hooks::set_barrier_failure(true);
    let swept = sweep::sweep_orphans(db.api(), Uid::default(), None, wait());
    hooks::set_barrier_failure(false);
    assert!(swept.is_err(), "the sweep ran past a failed barrier: {:?}", swept);
    assert!(db.api().fault().is_some(), "the failed barrier did not stop the store");
    assert!(chunks_live(&db, old), "P's chunks were retired");
    let _ = db.close();
    res!(Refusal::clear(&root)); // as the operator would
    let db = res!(open(&root, &c, false));
    assert_eq!(read(&db, &k), "1500/2", "the restart did not give P back");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// Under `Never`, a clean close makes everything durable, so every displaced chunk set is retired
/// and the reopened store holds the chunks of the last value only.
#[test]
fn a_clean_close_under_never_retires() -> Outcome<()> {
    let _g = lock();
    let c = res!(cfg(Policy::Never, 1, 1));
    let root = res!(fresh("./test_db_durable_mark_never"));
    let db = res!(open(&root, &c, true));
    let k = dat!("durable mark never");
    for s in 1..6u8 {
        res!(put(&db, &k, val(1_500, s)));
    }
    res!(db.close());
    let db = res!(open(&root, &c, false));
    assert_eq!(read(&db, &k), "1500/5");
    assert_eq!(res!(live_chunks(&db)), 4, "displaced chunk sets outlived a clean close");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

// The records the file states flag current, and those they flag old.
fn flagged(db: &TestDb) -> Outcome<(usize, usize)> {
    let mut n = (0, 0);
    for (_, fstates) in res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT)) {
        for (_, fstat) in fstates.map() {
            for (_, d) in fstat.data_map() {
                match d {
                    DataState::Cur      => n.0 += 1,
                    DataState::Old(_)   => n.1 += 1,
                }
            }
        }
    }
    Ok(n)
}

/// `Never` and `EveryN` owe a held discard a barrier within `HELD_BARRIER`: after overwrites
/// into a live file that then stops growing, with no seal, `sync_now` or close, the losers are
/// flagged old and the displaced chunk set retired.  Two writers per zone check that the
/// request reaches the writer that owns the winner's file.
#[test]
fn an_idle_store_is_owed_its_barrier() -> Outcome<()> {
    let _g = lock();
    const N: usize = 12;
    let bound = Duration::from_millis(1_500);
    for p in [Policy::Never, Policy::EveryN(1_000)] {
        for (zones, wbots) in SHAPES {
            let label = fmt!("{:?} {}x{}", p, zones, wbots);
            let mut c = res!(cfg(p, zones, wbots));
            c.data_file_max_bytes = 1_000_000;
            let root = res!(fresh("./test_db_durable_mark_owed"));
            let db = res!(open(&root, &c, true));
            let big = dat!("durable mark owed chunked");
            for i in 0..N {
                res!(put(&db, &dat!(fmt!("durable mark owed {}", i)), val(50, 1)));
            }
            res!(put(&db, &big, val(1_500, 1)));
            res!(db.api().sync_now(wait()));
            res!(put(&db, &big, val(1_500, 2)));
            assert!(held_soon(&db), "{}: an overwrite was released with no barrier", label);
            for i in 0..N {
                res!(put(&db, &dat!(fmt!("durable mark owed {}", i)), val(50, 2)));
            }
            let begun = Instant::now();
            assert!(wait_until(bound, || db.api().held_effects() == 0),
                "{}: {} discards still held {:?} after the writes stopped",
                label, db.api().held_effects(), bound);
            let took = begun.elapsed();
            // The losers: the small values, and the chunked value's head and four chunks.
            assert_eq!(res!(flagged(&db)).1, N + 5, "{}: losers not flagged old", label);
            assert_eq!(res!(live_chunks(&db)), 4, "{}: the displaced chunk set was not retired", label);
            assert_eq!(read(&db, &big), "1500/2");
            println!("{}: holds released {:?} after the last write.", label, took);
            res!(db.close());
            let _ = fs::remove_dir_all(&root);
        }
    }
    Ok(())
}

/// A burst of inserts that displace nothing holds nothing, so under `Never` it costs no barrier
/// at all: one would fail, and stop the store.
#[test]
fn an_insert_only_burst_costs_no_barrier() -> Outcome<()> {
    let _g = lock();
    let mut c = res!(cfg(Policy::Never, 1, 1));
    c.data_file_max_bytes = 1_000_000;
    let root = res!(fresh("./test_db_durable_mark_inserts"));
    let db = res!(open(&root, &c, true));
    let failed = hooks::barriers_failed();
    hooks::set_barrier_failure(true);
    for i in 0..200 {
        res!(put(&db, &dat!(fmt!("durable mark insert {}", i)), val(50, 1)));
    }
    // Absence is what is tested: three periods in which an owed barrier would have run.
    std::thread::sleep(constant::HELD_BARRIER * 3);
    hooks::set_barrier_failure(false);
    assert_eq!(hooks::barriers_failed(), failed, "an insert-only burst under Never ran a barrier");
    assert!(db.api().fault().is_none(), "an insert-only burst under Never faulted the store");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The two gates the owed barrier closed, under `Never` with no write after the overwrites:
/// the superseded records of a sealed file start its collection, and the rollover's file
/// states settle to one current record per key, each within a bounded time.
#[test]
fn an_idle_store_under_never_collects_and_settles() -> Outcome<()> {
    let _g = lock();
    const NK: usize = 10;   // most of the first file
    const NF: usize = 8;    // seals it part way through
    let bound = Duration::from_secs(2);
    let mut c = res!(cfg(Policy::Never, 1, 1));
    c.num_cbots_per_zone    = 1;
    c.data_file_max_bytes   = 16_000;   // collection triggers at 4,800 superseded bytes
    c.rest_chunk_threshold  = 3_000;
    let root = res!(fresh("./test_db_durable_mark_collect"));
    log_set_level!("error");
    let db = res!(setup::start_db(root.clone(), Some(c.clone()), schemes(), None, true, true));
    for i in 0..NK {
        res!(put(&db, &dat!(fmt!("durable mark key {}", i)), val(1_000, 1)));
    }
    for i in 0..NF {
        res!(put(&db, &dat!(fmt!("durable mark filler {}", i)), val(1_000, 1)));
    }
    assert!(zdir(&root, &c, 0).join(ZoneDir::relative_file_path(&FileType::Data, 2)).is_file(),
        "the fillers did not seal data file 1");
    // Held in collection long enough to be seen there.
    hooks::set_collect_delay(Duration::from_millis(1_500));
    for i in 0..(NK / 2) {
        res!(put(&db, &dat!(fmt!("durable mark key {}", i)), val(1_000, 2)));
    }
    let begun = Instant::now();
    let settled = wait_until(bound, || matches!(flagged(&db), Ok((n, _)) if n == NK + NF));
    let collecting = wait_until(bound, || match db.api().collect_file_states(wait()) {
        Ok(states) => states.values().any(|s| s.map().get(&1).map_or(false, |f| f.gc_active())),
        Err(_) => false,
    });
    let took = begun.elapsed();
    hooks::set_collect_delay(Duration::ZERO);
    assert!(settled, "rollover under Never: {:?} current and old, not {} current, {:?} after the \
        last write", flagged(&db), NK + NF, took);
    assert!(collecting, "data file 1 did not start its collection within {:?} of the last write", took);
    println!("Never, idle: settled and collecting {:?} after the last write.", took);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

// A small deterministic generator, so that a failing seed can be run again.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
}

/// The power-cut oracle (R5).  Random chunked and small puts, overwrites, deletes, barriers,
/// compactions and sweeps, then a cut at the marks: no read is torn, and no key comes back
/// missing or older than its newest write a barrier covered.
fn model(seed: u64, p: Policy, zones: u16, wbots: u16) -> Outcome<()> {
    let label = fmt!("seed {} {:?} {}x{}", seed, p, zones, wbots);
    let mut c = res!(cfg(p, zones, wbots));
    c.data_file_max_bytes = 6_000;
    let root = res!(fresh("./test_db_durable_mark_model"));
    let db = res!(open(&root, &c, true));
    let keys: Vec<Dat> = (0..6).map(|i| dat!(fmt!("durable mark model {}", i))).collect();
    // Per key, what it held after each acknowledged write, and how many of them are covered.
    let mut hist: Vec<Vec<String>> = vec![vec![fmt!("none")]; keys.len()];
    let mut covered: Vec<usize> = vec![1; keys.len()];
    let mut rng = Rng(seed | 1);
    let mut wseed = 0u8;
    for _ in 0..60 {
        let i = rng.below(keys.len() as u64) as usize;
        match rng.below(20) {
            0..=6 => {
                wseed = wseed.wrapping_add(1).max(1);
                let len = 1_100 + rng.below(500) as usize;
                res!(put(&db, &keys[i], val(len, wseed)));
                hist[i].push(fmt!("{}/{}", len, wseed));
            },
            7..=12 => {
                wseed = wseed.wrapping_add(1).max(1);
                let len = 50 + rng.below(250) as usize;
                res!(put(&db, &keys[i], val(len, wseed)));
                hist[i].push(fmt!("{}/{}", len, wseed));
            },
            13..=14 => {
                res!(db.delete(&keys[i], Uid::default(), None));
                hist[i].push(fmt!("none"));
            },
            15..=16 => {
                res!(db.api().sync_now(wait()));
                for (h, n) in hist.iter().zip(covered.iter_mut()) {
                    *n = h.len();
                }
            },
            17..=18 => { res!(db.compact_now(Duration::from_secs(60))); },
            _ => { res!(sweep::sweep_orphans(db.api(), Uid::default(), None, wait())); },
        }
        if let Policy::EveryWrite = p {
            for (h, n) in hist.iter().zip(covered.iter_mut()) {
                *n = h.len();
            }
        }
    }
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let got = res!(read_after_cut(&db, &root, &c, "./test_db_durable_mark_model_cut", &keys));
    for (i, g) in got.iter().enumerate() {
        let ok = &hist[i][covered[i] - 1..];
        assert!(ok.contains(g), "{}: key {} reads {} after the cut; allowed {:?}", label, i, g, ok);
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn power_cut_oracle() -> Outcome<()> {
    let _g = lock();
    let base = match std::env::var("DURABLE_MARK_SEED") {
        Ok(s) => res!(s.parse::<u64>().map_err(|e| err!(e, "DURABLE_MARK_SEED"; Test, Invalid))),
        Err(_) => 0x5eed_0000,
    };
    let t0 = Instant::now();
    for n in 0..3u64 {
        for p in POLICIES {
            for (z, w) in SHAPES {
                res!(model(base + n * 7_919, p, z, w));
            }
        }
    }
    msg!("The oracle ran 24 stores in {:?}.", t0.elapsed());
    Ok(())
}
