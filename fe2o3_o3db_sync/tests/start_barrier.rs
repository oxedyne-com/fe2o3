//! The start barrier (2026-10-09).  `O3db::start` returns once every record on disk is applied
//! to its cache, so a get, a put or a ping made the moment it returns waits on no replay.  Each
//! cache insert is held 1 ms, so the replay takes about two seconds to apply, and each file's
//! replay is held too, so it is still being queued for a while after the zones begin.  The stamp
//! clock is process-wide, so every store is written by a child process of this binary with its
//! clock a minute ahead (`sb_child`, env SB_MODE / SB_DIR / SB_T), and the put after a reopen is
//! made by another child.

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
    db::O3db,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    path::Path,
    process::Command,
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
    thread,
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

const NSMALL:   usize = 3_000;  // small keys, each written twice
const NCHUNKED: usize = 10;     // chunked keys, each written twice
const NPROBE:   usize = 100;    // the last small keys written, put again after a reopen
const ZONES:    u16   = 2;
const CBOTS:    u16   = 2;      // per zone
const INSERT_MS: u64  = 1;      // each cache insert is held this long
const FILE_MS:  u64   = 25;     // each file's replay is held this long

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = ZONES;
    c.num_cbots_per_zone    = CBOTS;
    c.num_igbots_per_zone   = 2;
    c.num_wbots_per_zone    = 1;
    c.data_file_max_bytes   = 8_192;
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = false;
    c.zone_overrides        = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 100_000_000u64 },
        2u16 => mapdat!{ "dir" => "", "max_size" => 100_000_000u64 },
    }.get_map().unwrap_or_default();
    Ok(c)
}

fn small_key(i: usize) -> Dat { dat!(fmt!("sb {:04}", i)) }
fn small_val(i: usize, ver: u8) -> Dat { dat!(fmt!("v{} of {:04}", ver, i)) }
fn chunked_key(i: usize) -> Dat { dat!(fmt!("sb chunked {}", i)) }

fn chunked_val(i: usize, ver: u8) -> Dat {
    let s = (i as u8).wrapping_mul(16) ^ ver;
    Dat::BU32((0..2_500usize).map(|j| s ^ (j as u8)).collect())
}

fn hooks_on() {
    hooks::set_insert_delay(Duration::from_millis(INSERT_MS));
    hooks::set_replay_file_delay(Duration::from_millis(FILE_MS));
}

fn hooks_off() {
    hooks::set_insert_delay(Duration::ZERO);
    hooks::set_replay_file_delay(Duration::ZERO);
}

/// Opens the store as the gateway does: `start`, then garbage collection switched at once.  The
/// write phase leaves it off, so every replaced record is still on disk to be replayed.
fn open(dir: &str, wipe: bool, gc: bool) -> Outcome<(TestDb, Duration)> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    if wipe {
        let _ = std::fs::remove_file(OzoneConfig::config_path(&root));
    }
    let mut db = res!(O3db::new(root, Some(res!(cfg())), schemes(), Uid::default()));
    let t = Instant::now();
    res!(db.start("start_barrier"));
    let took = t.elapsed();
    res!(ok!(db.updated_api()).activate_gc(gc));
    Ok((db, took))
}

// The replayed records a cache bot holds on average: every write, with each chunked value's
// bunch key and chunks.
fn inserts_per_cbot() -> u64 {
    let per_chunked = 1 + 2_500 / 400 + 1;
    (2 * (NSMALL + NCHUNKED * per_chunked) / (ZONES * CBOTS) as usize) as u64
}

// ---------------------------------------------------------------------------------------------
// Child-process phases.

fn child(mode: &str, dir: &str) -> Outcome<String> {
    let exe = res!(std::env::current_exe());
    let ahead = res!(SystemTime::now().duration_since(UNIX_EPOCH)) + Duration::from_secs(60);
    let out = res!(Command::new(exe)
        .args(["sb_child", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("SB_MODE", mode)
        .env("SB_DIR", dir)
        .env("SB_T", fmt!("{}", ahead.as_nanos()))
        .output());
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    // The harness prints `test sb_child ... ` with no newline, so a child's first line follows it.
    let lines: Vec<&str> = so.lines().filter_map(|l| l.find("SB ").map(|i| &l[i..])).collect();
    Ok(fmt!("status={:?}\n{}", out.status.code(), lines.join("\n")))
}

// Each probe key read back, "ok" when it holds the value put after the reopen.
fn probes(db: &TestDb) -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut bad = Vec::new();
    for i in NSMALL - NPROBE..NSMALL {
        match db.get(&small_key(i), None) {
            Ok(Some((v, _))) if v == small_val(i, 3) => ok += 1,
            other => bad.push(fmt!("{}:{:?}", i, other.map(|o| o.map(|(v, _)| v)))),
        }
    }
    match db.get(&chunked_key(0), None) {
        Ok(Some((v, _))) if v == chunked_val(0, 3) => ok += 1,
        _ => bad.push(fmt!("chunked 0")),
    }
    bad.truncate(5);
    (ok, bad)
}

#[test]
#[ignore]
fn sb_child() -> Outcome<()> {
    let mode = match std::env::var("SB_MODE") { Ok(m) => m, Err(_) => return Ok(()) };
    let dir = res!(std::env::var("SB_DIR"));
    match mode.as_str() {
        // The store, written while the clock read a minute ahead.  The chunked keys go first, so
        // the last records written, the highest stamps, are the probe keys' second versions.
        "write" => {
            let t: u128 = res!(res!(std::env::var("SB_T")).parse::<u128>());
            let (db, _) = res!(open(&dir, true, false));
            hooks::set_fixed_stamp(Some(Duration::new(
                (t / 1_000_000_000) as u64, (t % 1_000_000_000) as u32)));
            for ver in 1..3u8 {
                for i in 0..NCHUNKED {
                    res!(db.insert(chunked_key(i), chunked_val(i, ver), Uid::default(), None));
                }
            }
            for ver in 1..3u8 {
                let mut ts = Vec::new();
                for w in 0..4usize {
                    let db = db.clone();
                    ts.push(thread::spawn(move || -> Outcome<()> {
                        for i in (w..NSMALL - NPROBE).step_by(4) {
                            res!(db.insert(small_key(i), small_val(i, ver), Uid::default(), None));
                        }
                        Ok(())
                    }));
                }
                for t in ts {
                    match t.join() {
                        Ok(r) => res!(r),
                        Err(_) => return Err(err!("A writer thread panicked."; Bug)),
                    }
                }
            }
            for ver in 1..3u8 {
                for i in NSMALL - NPROBE..NSMALL {
                    res!(db.insert(small_key(i), small_val(i, ver), Uid::default(), None));
                }
            }
            hooks::set_fixed_stamp(None);
            res!(db.close());
            println!("SB written");
        },
        // A put of every probe key the moment `start` returns, with the replay still to apply
        // were `start` not to wait for it, then the same reads after a restart.
        "put" => {
            hooks_on();
            let (db, took) = res!(open(&dir, false, true));
            for i in NSMALL - NPROBE..NSMALL {
                res!(db.insert(small_key(i), small_val(i, 3), Uid::default(), None));
            }
            res!(db.insert(chunked_key(0), chunked_val(0, 3), Uid::default(), None));
            let (ok, bad) = probes(&db);
            println!("SB put start_ms={} ok={} bad={:?}", took.as_millis(), ok, bad);
            res!(db.close());
            hooks_off();
            let (db, _) = res!(open(&dir, false, true));
            let (ok, bad) = probes(&db);
            println!("SB restart ok={} bad={:?}", ok, bad);
            res!(db.close());
        },
        m => return Err(err!("Unknown child mode {}.", m; Invalid, Input)),
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------

/// Every key, read by 8 threads the moment `start` returns, holds its second version, no read
/// errs, and a ping then is answered at once.  `start` itself must have taken at least half the
/// replay's apply time, or the delay that makes the window did not act.
#[test]
fn a_get_at_once_after_start_finds_every_key() -> Outcome<()> {
    let _g = lock();
    let dir = "./test_db_start_barrier_get";
    let out = res!(child("write", dir));
    assert!(out.contains("SB written"), "The write phase failed: {}", out);

    hooks_on();
    let (db, took) = res!(open(dir, false, true));
    let t = Instant::now();
    let ping = db.api().ping_bots(constant::USER_REQUEST_WAIT);
    let ping_ms = t.elapsed().as_millis();

    let absent  = Arc::new(AtomicUsize::new(0));
    let stale   = Arc::new(AtomicUsize::new(0));
    let errs    = Arc::new(AtomicUsize::new(0));
    let first   = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut ts = Vec::new();
    for w in 0..8usize {
        let (db, absent, stale, errs, first) =
            (db.clone(), absent.clone(), stale.clone(), errs.clone(), first.clone());
        ts.push(thread::spawn(move || {
            let note = |what: String| {
                if let Ok(mut f) = first.lock() { if f.len() < 5 { f.push(what); } }
            };
            // Newest first: those keys are in the files replayed last.
            for i in (w..NSMALL + NCHUNKED).step_by(8).rev() {
                let (k, want) = match i < NSMALL {
                    true    => (small_key(i), small_val(i, 2)),
                    false   => (chunked_key(i - NSMALL), chunked_val(i - NSMALL, 2)),
                };
                match db.get(&k, None) {
                    Ok(Some((v, _))) if v == want => (),
                    Ok(Some(_)) => { stale.fetch_add(1, Ordering::SeqCst); note(fmt!("stale {}", i)); },
                    Ok(None)    => { absent.fetch_add(1, Ordering::SeqCst); note(fmt!("absent {}", i)); },
                    Err(e)      => { errs.fetch_add(1, Ordering::SeqCst); note(fmt!("error {}: {}", i, e)); },
                }
            }
        }));
    }
    for t in ts {
        assert!(t.join().is_ok(), "A reader thread panicked.");
    }
    let gets_ms = t.elapsed().as_millis();
    hooks_off();
    res!(db.close());

    let apply_ms = inserts_per_cbot() * INSERT_MS;
    println!("start_barrier: start {} ms (replay apply about {} ms per cache bot), ping {} ms, \
        every get done {} ms after start", took.as_millis(), apply_ms, ping_ms, gets_ms);
    let first = match first.lock() { Ok(f) => f.clone(), Err(p) => p.into_inner().clone() };
    assert_eq!((absent.load(Ordering::SeqCst), stale.load(Ordering::SeqCst), errs.load(Ordering::SeqCst)),
        (0, 0, 0), "Reads the moment start returned (absent, stale, errors): {:?}", first);
    assert!(ping.is_ok(), "The ping right after start failed: {:?}", ping.err());
    assert!(ping_ms < 250,
        "The ping right after start took {} ms: start returned with replayed inserts still queued \
        at the cache bots.", ping_ms);
    assert!(took.as_millis() as u64 >= apply_ms / 2,
        "start took {} ms, under half the {} ms the held inserts take to apply, so the hook did \
        not hold them.", took.as_millis(), apply_ms);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// A put the moment `start` returns, of keys whose last records on disk carry the highest stamps
/// and were written a minute ahead of this clock, outranks them, live and after a restart.
#[test]
fn a_put_at_once_after_start_outranks_its_replayed_record() -> Outcome<()> {
    let _g = lock();
    let dir = "./test_db_start_barrier_put";
    let out = res!(child("write", dir));
    assert!(out.contains("SB written"), "The write phase failed: {}", out);
    let out = res!(child("put", dir));
    println!("start_barrier put: {}", out.replace('\n', " | "));
    let want = fmt!("ok={} bad=[]", NPROBE + 1);
    assert!(out.lines().any(|l| l.starts_with("SB put ") && l.ends_with(&want)),
        "A put the moment start returned lost to its replayed record: {}", out);
    assert!(out.lines().any(|l| l.starts_with("SB restart ") && l.ends_with(&want)),
        "A put the moment start returned lost to its replayed record after a restart: {}", out);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
