//! Races of the durable mark (2026-10-10, from QA agent 2's probes P1, P5 and P6).  A chunked
//! put's answer waits for the barrier over its chunks, a power cut taken at random through a run
//! with automatic collection on never reads a torn, lost or stale value, and a close racing a
//! writer leaves every live file covered by its mark.  The hooks are process-wide, so the tests
//! run one at a time.

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
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
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
    sync::{
        Arc,
        Mutex,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
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
    Policy::EveryN(100_000),
    Policy::Interval(3_600_000),
    Policy::Never,
];

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
    hooks::set_barrier_delay(Duration::ZERO);
    g
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn fresh(name: &str) -> Outcome<PathBuf> {
    let dir = fmt!("./test_db_races_{}", name);
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(&dir));
    Ok(res!(Path::new(&dir).canonicalize()))
}

fn open(root: &Path, c: &OzoneConfig, gc: bool, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(c.clone()), schemes(), None, gc, wipe)
}

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

fn put(db: &TestDb, k: &Dat, v: Dat) -> Outcome<()> {
    db.insert(k.clone(), v, Uid::default(), None).map(|_| ())
}

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

fn zdir(root: &Path, c: &OzoneConfig, z: usize) -> PathBuf {
    c.zone_root(root).join(fmt!("zone_{:03}", z + 1))
}

/// The power cut of `durable_mark.rs`, at one instant (`gc_pair::instant_cut`).
fn power_cut(db: &TestDb, root: &Path, c: &OzoneConfig, to: &str) -> Outcome<PathBuf> {
    let dst = res!(fresh(to));
    res!(instant_cut(db, root, c, &dst));
    Ok(dst)
}

fn read_after_cut(db: &TestDb, root: &Path, c: &OzoneConfig, to: &str, keys: &[Dat]) -> Outcome<Vec<String>> {
    let cut = res!(power_cut(db, root, c, to));
    let db2 = res!(open(&cut, c, false, false));
    let got = keys.iter().map(|k| read(&db2, k)).collect();
    res!(db2.close());
    let _ = fs::remove_dir_all(&cut);
    Ok(got)
}

// ---------------------------------------------------------------------------------------------
// R1 (P1, M8).  A chunked put's caller waits on the barrier over its chunks (M8 closer), and a power
// cut while that barrier is in progress gives the old value, never the new head without chunks.
#[test]
fn a_chunk_answer_waits_for_its_barrier() -> Outcome<()> {
    let _g = lock();
    for p in [Policy::Interval(3_600_000), Policy::Never, Policy::EveryN(100_000)] {
        for (z, w) in [(1u16, 1u16), (2, 2)] {
            let label = fmt!("{:?} {}x{}", p, z, w);
            let c = res!(cfg(p, z, w));
            let root = res!(fresh("p1"));
            let db = Arc::new(res!(open(&root, &c, false, true)));
            let k = dat!("races p1 chunked");
            res!(put(&db, &k, val(3_000, 1)));
            res!(db.api().sync_now(wait()));
            let delay = Duration::from_millis(900);
            hooks::set_barrier_delay(delay);
            let db2 = db.clone();
            let k2 = k.clone();
            let t0 = Instant::now();
            let h = std::thread::spawn(move || {
                let r = put(&db2, &k2, val(3_000, 2));
                (r, Instant::now())
            });
            std::thread::sleep(Duration::from_millis(300));
            let mid = res!(read_after_cut(&db, &root, &c, "p1cut", &[k.clone()]));
            let (r, t1) = match h.join() {
                Ok(x) => x,
                Err(_) => return Err(err!("put thread panicked"; Test)),
            };
            hooks::set_barrier_delay(Duration::ZERO);
            res!(r);
            let took = t1.duration_since(t0);
            println!("{}: chunked put took {:?}; cut mid-barrier read {}", label, took, mid[0]);
            assert!(took >= delay - Duration::from_millis(50),
                "{}: the chunked put returned in {:?}, before the barrier over its chunks ({:?})", label, took, delay);
            assert_eq!(mid[0], "3000/1", "{}: a cut during the chunk barrier did not give the old value", label);
            let after = res!(read_after_cut(&db, &root, &c, "p1cut", &[k.clone()]));
            // The head is past its mark under these policies unless an owed barrier covered it.
            assert!(after[0] == "3000/1" || after[0] == "3000/2", "{}: after: {}", label, after[0]);
            assert_eq!(read(&db, &k), "3000/2", "{}", label);
            match Arc::try_unwrap(db) {
                Ok(db) => res!(db.close()),
                Err(_) => return Err(err!("db still shared"; Test)),
            }
            let _ = fs::remove_dir_all(&root);
        }
    }
    Ok(())
}

// M8.  A writer finishes its barrier before it takes the next record, so a head sharing a writer
// with a chunk waits on the chunk's barrier whether or not the chunk's answer was held.  With one
// writer in each of sixteen zones a head usually lands in a zone that took none of its three
// chunks; only the held chunk answers then keep the put from returning, and the head from being
// written, before its chunks are durable.
#[test]
fn a_head_with_its_own_writer_waits_for_its_chunks() -> Outcome<()> {
    let _g = lock();
    let delay = Duration::from_millis(600);
    for p in [Policy::Never, Policy::Interval(3_600_000)] {
        let c = res!(cfg(p, 16, 1));
        let root = res!(fresh("m8"));
        let db = res!(open(&root, &c, false, true));
        for i in 0..8 {
            let label = fmt!("{:?} key {}", p, i);
            let k = dat!(fmt!("races m8 chunked {}", i));
            res!(put(&db, &k, val(1_200, 1)));
            res!(db.api().sync_now(wait()));
            hooks::set_barrier_delay(delay);
            let t0 = Instant::now();
            let r = put(&db, &k, val(1_200, 2));
            let took = t0.elapsed();
            hooks::set_barrier_delay(Duration::ZERO);
            res!(r);
            assert!(took >= delay - Duration::from_millis(50),
                "{}: the chunked put returned in {:?}, before the barrier over its chunks ({:?})",
                label, took, delay);
            assert_eq!(read(&db, &k), "1200/2", "{}", label);
        }
        res!(db.close());
        let _ = fs::remove_dir_all(&root);
    }
    Ok(())
}

// P5: the power-cut oracle with automatic collection on, small files and more ops, every policy.
fn model(seed: u64, p: Policy, zones: u16, wbots: u16) -> Outcome<()> {
    let label = fmt!("seed {} {:?} {}x{}", seed, p, zones, wbots);
    let mut c = res!(cfg(p, zones, wbots));
    c.data_file_max_bytes = 5_000;
    let root = res!(fresh("p5"));
    let db = res!(open(&root, &c, true, true));
    let keys: Vec<Dat> = (0..8).map(|i| dat!(fmt!("races model {}", i))).collect();
    let mut hist: Vec<Vec<String>> = vec![vec![fmt!("none")]; keys.len()];
    let mut covered: Vec<usize> = vec![1; keys.len()];
    let mut rng = seed | 1;
    let mut next = |n: u64| { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng % n };
    let mut wseed = 0u8;
    let mut cut_n = 0;
    for step in 0..120 {
        let i = next(keys.len() as u64) as usize;
        match next(24) {
            0..=6 => {
                wseed = wseed.wrapping_add(1).max(1);
                let len = 1_100 + next(500) as usize;
                res!(put(&db, &keys[i], val(len, wseed)));
                hist[i].push(fmt!("{}/{}", len, wseed));
            },
            7..=15 => {
                wseed = wseed.wrapping_add(1).max(1);
                let len = 50 + next(250) as usize;
                res!(put(&db, &keys[i], val(len, wseed)));
                hist[i].push(fmt!("{}/{}", len, wseed));
            },
            16..=17 => {
                res!(db.delete(&keys[i], Uid::default(), None));
                hist[i].push(fmt!("none"));
            },
            18 => {
                res!(db.api().sync_now(wait()));
                for (h, n) in hist.iter().zip(covered.iter_mut()) { *n = h.len(); }
            },
            19 => { res!(db.api().new_live_files()); },
            20 => { res!(sweep::sweep_orphans(db.api(), Uid::default(), None, wait())); },
            21 => {
                // Let owed barriers and collections run, then cut mid-run.
                std::thread::sleep(Duration::from_millis(next(400)));
                let got = res!(read_after_cut(&db, &root, &c, "p5cut", &keys));
                for (j, g) in got.iter().enumerate() {
                    let ok = &hist[j][covered[j] - 1..];
                    assert!(ok.contains(g), "{} step {}: key {} reads {} after a mid-run cut; allowed {:?}", label, step, j, g, ok);
                }
                cut_n += 1;
            },
            _ => {
                // A cut right now, with no pause.
                let got = res!(read_after_cut(&db, &root, &c, "p5cut", &keys));
                for (j, g) in got.iter().enumerate() {
                    let ok = &hist[j][covered[j] - 1..];
                    assert!(ok.contains(g), "{} step {}: key {} reads {} after an immediate cut; allowed {:?}", label, step, j, g, ok);
                }
                cut_n += 1;
            },
        }
        if let Policy::EveryWrite = p {
            for (h, n) in hist.iter().zip(covered.iter_mut()) { *n = h.len(); }
        }
    }
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let got = res!(read_after_cut(&db, &root, &c, "p5cut", &keys));
    for (i, g) in got.iter().enumerate() {
        let ok = &hist[i][covered[i] - 1..];
        assert!(ok.contains(g), "{}: key {} reads {} after the final cut; allowed {:?}", label, i, g, ok);
    }
    for (i, k) in keys.iter().enumerate() {
        let want = match hist[i].last() { Some(w) => w.clone(), None => fmt!("none") };
        assert_eq!(read(&db, k), want, "{}: live read of key {}", label, i);
    }
    let v = res!(verify::verify_live_set(db.api(), None, wait()));
    assert!(v.clean(), "{}: {}", label, v.summary(4));
    res!(db.close());
    let db = res!(open(&root, &c, true, false));
    for (i, k) in keys.iter().enumerate() {
        let want = match hist[i].last() { Some(w) => w.clone(), None => fmt!("none") };
        assert_eq!(read(&db, k), want, "{}: read of key {} after a clean restart", label, i);
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    println!("{}: {} mid-run cuts passed", label, cut_n);
    Ok(())
}

#[test]
fn the_power_cut_oracle_with_collection_on() -> Outcome<()> {
    let _g = lock();
    let base: u64 = std::env::var("RACES_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0xa2a2_0001);
    let reps: u64 = std::env::var("RACES_REPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    for n in 0..reps {
        for p in POLICIES {
            for (z, w) in [(1u16, 1u16), (2, 2)] {
                res!(model(base + n * 7_919, p, z, w));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// M5 (P6): close with writes racing it under Never (M5).  After close, every live file's mark must
// cover the file's length on disk: the announced close barriered what was written after the
// close's own sync_now.
#[test]
fn a_close_covers_racing_writes() -> Outcome<()> {
    let _g = lock();
    for (z, w) in [(1u16, 1u16), (2, 2)] {
        for round in 0..5 {
            let label = fmt!("Never {}x{} round {}", z, w, round);
            let mut c = res!(cfg(Policy::Never, z, w));
            c.data_file_max_bytes = 10_000_000;
            let root = res!(fresh("p6"));
            let db = Arc::new(res!(open(&root, &c, false, true)));
            let stop = Arc::new(AtomicBool::new(false));
            let (db2, stop2) = (db.clone(), stop.clone());
            let h = std::thread::spawn(move || {
                let mut i = 0usize;
                let mut ok = 0usize;
                while !stop2.load(Ordering::SeqCst) && i < 200_000 {
                    if put(&db2, &dat!(fmt!("races p6 {}", i % 50)), val(60, (i % 200) as u8 + 1)).is_ok() {
                        ok += 1;
                    } else {
                        break;
                    }
                    i += 1;
                }
                ok
            });
            std::thread::sleep(Duration::from_millis(300));
            let marks = db.api().chans().marks().clone();
            let r = db.close();
            stop.store(true, Ordering::SeqCst);
            let ok = h.join().unwrap_or(0);
            let mut bad = Vec::new();
            for zi in 0..z as usize {
                for (f, (d, i)) in res!(marks.files(zi)) {
                    let dl = fs::metadata(zdir(&root, &c, zi).join(ZoneDir::relative_file_path(&FileType::Data, f))).map(|m| m.len()).unwrap_or(0);
                    let il = fs::metadata(zdir(&root, &c, zi).join(ZoneDir::relative_file_path(&FileType::Index, f))).map(|m| m.len()).unwrap_or(0);
                    if d < dl || i < il {
                        bad.push(fmt!("zone {} file {}: mark ({}, {}) disk ({}, {})", zi, f, d, i, dl, il));
                    }
                }
            }
            println!("{}: close {:?}, {} writes ok, uncovered: {:?}", label, r.is_ok(), ok, bad);
            assert!(bad.is_empty(), "{}: close left written bytes past the mark: {:?}", label, bad);
            drop(db);
            let _ = fs::remove_dir_all(&root);
        }
    }
    Ok(())
}
