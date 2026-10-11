//! Barriers on a chunked put's path (Ozone chunk barriers, 2026-10-11).  A store's chunks are
//! never answered before a barrier covers them, and the store asks each writer it used for one
//! behind them, so a chunk must not trip the timed or every-N barrier as well: that barrier would
//! cover only the chunks taken so far, and the rest would wait on a second one, in series (M1).
//! Barriers are counted with `hooks::barriers_by_writer`, not timed.  The chunk phase is read at
//! the head hold, once every chunk is answered and before the head is sent.  The hooks are
//! process-wide, so the tests run one at a time.

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
    path::Path,
    sync::{
        Mutex,
        MutexGuard,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

static HOOKS: Mutex<()> = Mutex::new(());

const CHUNK:    u64 = 400;
const LENS:     [usize; 4] = [350, 1_100, 3_000, 6_400]; // 1, 3, 8 and 16 chunks

#[derive(Clone, Copy, Debug)]
enum Policy { EveryWrite, Interval(u64), Never }

fn lock() -> MutexGuard<'static, ()> {
    let g = match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_head_hold(false);
    g
}

fn cfg(p: Policy, zones: u16, wbots: u16, file_max: u64) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = zones;
    c.num_cbots_per_zone    = 2;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = wbots;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 300;
    c.rest_chunk_bytes      = CHUNK;
    c.data_file_max_bytes   = file_max;
    c.sync_on_write         = false;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = 0;
    match p {
        Policy::EveryWrite  => c.sync_on_write = true,
        Policy::Interval(t) => c.sync_interval_ms = t,
        Policy::Never       => (),
    }
    Ok(c)
}

fn start(dir: &str, c: &OzoneConfig) -> Outcome<TestDb> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(c.clone()), schemes(), None, false, true)
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

fn wait_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

// The barriers each writer made between two readings of the gauge.
fn since(before: &BTreeMap<(usize, usize), u64>) -> BTreeMap<(usize, usize), u64> {
    hooks::barriers_by_writer().into_iter()
        .map(|(w, n)| (w, n - before.get(&w).copied().unwrap_or(0)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

// Puts a fresh key and returns the barriers made, by writer, before its head was sent.
fn chunk_phase(db: &TestDb, k: Dat, v: Dat) -> Outcome<BTreeMap<(usize, usize), u64>> {
    let held = hooks::heads_held();
    let before = hooks::barriers_by_writer();
    hooks::set_head_hold(true);
    let put = { let db = db.clone(); thread::spawn(move || db.insert(k, v, Uid::default(), None).map(|_| ())) };
    let reached = wait_until(Duration::from_secs(30), || hooks::heads_held() > held);
    let phase = since(&before);
    hooks::set_head_hold(false);
    let put = match put.join() { Ok(r) => r, Err(_) => Err(err!("The put panicked."; Test)) };
    res!(put);
    if !reached {
        return Err(err!("The put never reached the head hold."; Test));
    }
    Ok(phase)
}

// Each layout puts values of 1 to 16 chunks to fresh keys, each after the writers have been
// idle past the period, so that a chunk would find the timed barrier due.
fn run_layouts(p: Policy, settle: Duration, tag: &str) -> Outcome<Vec<String>> {
    let mut bad = Vec::new();
    for (z, w) in [(1u16, 1u16), (1, 4), (2, 2)] {
        let c = res!(cfg(p, z, w, 16 * 1_048_576));
        let dir = fmt!("./test_db_chunk_barriers_{}_{}x{}", tag, z, w);
        let db = res!(start(&dir, &c));
        res!(db.insert(dat!("warm"), dat!("up"), Uid::default(), None));
        let mut counts = Vec::new();
        for (i, len) in LENS.iter().enumerate() {
            res!(db.api().sync_now(wait()));
            thread::sleep(settle);
            let phase = res!(chunk_phase(&db, dat!(fmt!("chunked {}", i)), val(*len, i as u8)));
            let total: u64 = phase.values().sum();
            let worst = phase.values().copied().max().unwrap_or(0);
            counts.push(fmt!("{}B:{}/{}w", len, total, phase.len()));
            // R1: the chunks are durable before the head goes out.
            if total == 0 {
                bad.push(fmt!("{:?} {}x{} {} bytes: no barrier before the head", p, z, w, len));
            }
            if let Policy::EveryWrite = p {
                continue;
            }
            // M1: one barrier per writer touched.
            if worst > 1 {
                bad.push(fmt!("{:?} {}x{} {} bytes: a writer made {} barriers in the chunk phase {:?}",
                    p, z, w, len, worst, phase));
            }
        }
        println!("BARRIERS {:?} {}x{}: {}", p, z, w, counts.join(" "));
        res!(db.close());
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(bad)
}

#[test]
fn chunk_phase_interval() -> Outcome<()> {
    let _lock = lock();
    let bad = res!(run_layouts(Policy::Interval(200), Duration::from_millis(400), "int"));
    if !bad.is_empty() {
        return Err(err!("{:?}", bad; Test));
    }
    Ok(())
}

#[test]
fn chunk_phase_never() -> Outcome<()> {
    let _lock = lock();
    let bad = res!(run_layouts(Policy::Never, Duration::ZERO, "never"));
    if !bad.is_empty() {
        return Err(err!("{:?}", bad; Test));
    }
    Ok(())
}

// Every write waits on a barrier of its own, as before: counted, and held to R1 only.
#[test]
fn chunk_phase_every_write() -> Outcome<()> {
    let _lock = lock();
    let bad = res!(run_layouts(Policy::EveryWrite, Duration::ZERO, "every"));
    if !bad.is_empty() {
        return Err(err!("{:?}", bad; Test));
    }
    Ok(())
}

// The interval still bounds the loss of acknowledged writes: a stream of small inserts over a
// second, one every 50 ms, barriers about once per 200 ms period.
#[test]
fn interval_floor_holds_for_plain_writes() -> Outcome<()> {
    let _lock = lock();
    let c = res!(cfg(Policy::Interval(200), 1, 1, 16 * 1_048_576));
    let dir = "./test_db_chunk_barriers_floor";
    let db = res!(start(dir, &c));
    res!(db.api().sync_now(wait()));
    let before = hooks::barriers_by_writer();
    for i in 0..20u32 {
        res!(db.insert(dat!(fmt!("small {}", i)), dat!(i), Uid::default(), None));
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(300));
    let made: u64 = since(&before).values().sum();
    println!("FLOOR Interval(200) 20 inserts over 1 s: {} barriers", made);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    if !(3..=10).contains(&made) {
        return Err(err!("{} barriers for a second of writes under a 200 ms interval.", made; Test));
    }
    Ok(())
}

// A chunk that does not fit rolls the live file, and the dirty old file is sealed with a barrier
// of its own (M2).  Documented, not bounded: the count is one plus the rollovers.
#[test]
fn rollover_seals_counted() -> Outcome<()> {
    let _lock = lock();
    let c = res!(cfg(Policy::Never, 1, 1, 2_500));
    let dir = "./test_db_chunk_barriers_roll";
    let db = res!(start(dir, &c));
    let mut counts = Vec::new();
    for i in 0..6u8 {
        let phase = res!(chunk_phase(&db, dat!(fmt!("roll {}", i)), val(3_000, i)));
        counts.push(phase.values().sum::<u64>());
    }
    println!("ROLLOVER Never 1x1 2500-byte files, 8-chunk puts: {:?}", counts);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    if counts.iter().any(|n| *n == 0) {
        return Err(err!("A put's head went out with no barrier over its chunks: {:?}", counts; Test));
    }
    Ok(())
}
