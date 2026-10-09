//! Chunk sets (B1, B2, M1, 2026-10-09): every chunked write has a set of its own, and the
//! store retires the set its key no longer names.  The hooks are process-wide, so every test
//! takes one lock.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    Meta,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    api::OzoneApi,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    api::CompactReport,
    comm::response::Wait,
    data::cache::{
        Cache,
        CacheEntry,
        is_chunk_key,
    },
    file::{
        floc::FileLocation,
        state::DataState,
        stored::RecordDigest,
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
    collections::{
        BTreeMap,
        BTreeSet,
    },
    path::Path,
    sync::{
        Mutex,
        MutexGuard,
    },
    thread,
    time::Duration,
};

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

// Every byte is `seed ^ j`, so a value assembled from two values' chunks is told by `whole`.
fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn whole(v: &Dat, lens: &[usize]) -> bool {
    match v {
        Dat::BU32(b) | Dat::BU64(b) | Dat::BU8(b) | Dat::BU16(b) => {
            if !lens.contains(&b.len()) || b.is_empty() { return false; }
            let seed = b[0];
            b.iter().enumerate().all(|(j, x)| *x == seed ^ (j as u8))
        },
        _ => false,
    }
}

fn start(dir: &str, c: OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(c), schemes(), None, false, wipe)
}

fn scan_wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(60),
        check_interval: constant::CHECK_INTERVAL,
    }
}

fn head(db: &TestDb, k: &Dat) -> Outcome<Option<[u64; 5]>> {
    match res!(db.api().get_head_wait(k, None)) {
        Some((Dat::Tup5u64(t), _)) => Ok(Some(t)),
        _ => Ok(None),
    }
}

fn pause(d: Duration) {
    thread::sleep(d);
}

// A value written by a build that derived its set from the key alone reads whole after an
// overwrite of the same length, which moves it to a set of its own and retires the old one.
#[test]
fn legacy_key_derived_value_overwritten_same_length_reads_whole_and_retires() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_legacy";
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("legacy key");
    let (kbuf, _, _) = res!(db.api().ozone_key_dat(&k, None));
    let legacy = OzoneApi::<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>::legacy_chunk_set_id(&kbuf);
    let resp = db.api().responder();
    res!(db.api().store_dat_using_responder_forcing_set_id(
        k.clone(), val(1_100, 1), Uid::default(), None, resp.clone(), legacy));
    res!(resp.recv_store_ack());
    let old = res!(res!(head(&db, &k)).ok_or_else(|| err!("No legacy head."; Test, Missing)));
    assert_eq!(old[0], legacy, "the forced value is not under the legacy set");

    res!(db.insert(k.clone(), val(1_100, 2), Uid::default(), None));
    let got = res!(db.get(&k, None));
    let new = res!(res!(head(&db, &k)).ok_or_else(|| err!("No new head."; Test, Missing)));
    let gone = db.api().fetch_chunks(&Dat::Tup5u64(old), None).is_err();
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(matches!(&got, Some((v, _)) if *v == val(1_100, 2)), "the overwrite did not read back whole");
    assert_ne!(new[0], legacy, "the overwrite reused the legacy set");
    assert!(gone, "the legacy chunks were not retired");
    Ok(())
}

// The orphan sweep, run while a chunked put is between its chunks and its bunch key, leaves the
// put's chunks alone: they are named by no key yet, and only the pending sets say whose they are.
#[test]
fn sweep_spares_chunks_of_a_put_in_flight() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_sweep";
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("in flight");
    let v = val(3_000, 7); // 8 chunks
    hooks::set_chunk_insert_delay(Duration::from_millis(300));
    let put = {
        let (db, k, v) = (db.clone(), k.clone(), v.clone());
        thread::spawn(move || db.insert(k, v, Uid::default(), None).map(|_| ()).map_err(|e| fmt!("{}", e)))
    };
    pause(Duration::from_millis(400));
    let report = sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO);
    let put = match put.join() { Ok(r) => r, Err(_) => Err(fmt!("panicked")) };
    hooks::set_chunk_insert_delay(Duration::ZERO);
    let got = db.get(&k, None);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let report = res!(report);
    msg!("sweep mid-put: {}", report.summary().replace('\n', " | "));
    assert!(put.is_ok(), "the put failed: {:?}", put);
    assert!(report.skipped_recent > 0, "the sweep saw none of the put's chunks, so it proved nothing");
    assert_eq!(report.orphans_found, 0, "the sweep took the chunks of a put in flight for orphans");
    assert!(matches!(&got, Ok(Some((g, _))) if *g == v), "the put does not read back whole: {:?}",
        got.map(|o| o.map(|(d, _)| fmt!("{:?}", d).chars().take(120).collect::<String>())));
    Ok(())
}

// A put whose bunch key loses to a newer value already there (its stamp is older) retires its
// own chunks, so the sweep finds nothing, and the newer value survives a restart.
#[test]
fn losing_concurrent_put_retires_its_own_chunks() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_losing";
    let k = dat!("losing put");
    let v0 = val(1_100, 3);
    let (report, before, after) = {
        let db = res!(start(dir, res!(cfg()), true));
        res!(db.insert(k.clone(), v0.clone(), Uid::default(), None));
        let t0 = match res!(db.get(&k, None)) {
            Some((_, meta)) => *meta.time,
            None => return Err(err!("v0 is missing."; Test, Missing)),
        };
        hooks::set_stale_stamp(Some(t0.saturating_sub(Duration::from_secs(1))));
        let lost = db.insert(k.clone(), val(1_500, 4), Uid::default(), None);
        hooks::set_stale_stamp(None);
        res!(lost);
        let before = res!(db.get(&k, None)).map(|(v, _)| v);
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        res!(db.close());
        let db = res!(start(dir, res!(cfg()), false));
        let after = res!(db.get(&k, None)).map(|(v, _)| v);
        res!(db.close());
        (report, before, after)
    };
    let _ = std::fs::remove_dir_all(dir);
    msg!("losing put: {}", report.summary().replace('\n', " | "));
    assert!(before == Some(v0.clone()), "the older put displaced the newer value");
    assert_eq!(report.orphans_found, 0, "the losing put left its chunks for the sweep");
    assert!(after == Some(v0), "the newer value did not survive a restart");
    Ok(())
}

// Two puts of one key at once, of one length, under one clock reading, or one stamp outright.
// Returns, per key, whether it read one value whole live and after a restart, and the failures.
fn two_puts_per_key(dir: &str, stale: bool) -> Outcome<(Vec<bool>, Vec<bool>, Vec<String>)> {
    let lens = [1_100usize];
    let keys: Vec<Dat> = (0..6u8).map(|i| dat!(fmt!("tick {}", i))).collect();
    let db = res!(start(dir, res!(cfg()), true));
    let now = *res!(Timestamp::now());
    if stale { hooks::set_stale_stamp(Some(now)); } else { hooks::set_fixed_stamp(Some(now)); }
    hooks::set_chunk_insert_delay(Duration::from_millis(5));
    let ws: Vec<_> = keys.iter().enumerate().flat_map(|(i, k)| {
        (0..2u8).map(|j| {
            let (db, k) = (db.clone(), k.clone());
            let seed = (i as u8).wrapping_mul(16).wrapping_add(j);
            thread::spawn(move || db.insert(k, val(1_100, seed), Uid::default(), None)
                .map(|_| ()).map_err(|e| fmt!("{}", e)))
        }).collect::<Vec<_>>()
    }).collect();
    let mut failed = Vec::new();
    for w in ws {
        match w.join() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => failed.push(e.chars().take(200).collect::<String>()),
            Err(_) => failed.push(fmt!("panicked")),
        }
    }
    hooks::set_chunk_insert_delay(Duration::ZERO);
    hooks::set_fixed_stamp(None);
    hooks::set_stale_stamp(None);
    let mut live = Vec::new();
    for k in &keys {
        live.push(matches!(db.get(k, None), Ok(Some((v, _))) if whole(&v, &lens)));
    }
    res!(db.close());
    let db = res!(start(dir, res!(cfg()), false));
    let mut restarted = Vec::new();
    for k in &keys {
        restarted.push(matches!(db.get(k, None), Ok(Some((v, _))) if whole(&v, &lens)));
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    Ok((live, restarted, failed))
}

// Two puts of one key with one stamp get distinct chunk sets, so neither writes over the other's
// chunks and every key reads one value whole.  Which value a restart keeps is not asked: two
// records of a key with one stamp have no order, which is why a process never stamps two alike.
#[test]
fn two_puts_one_stamp_get_distinct_sets() -> Outcome<()> {
    let _lock = lock();
    let (live, _, failed) = res!(two_puts_per_key("./test_db_chunk_set_stamp", true));
    assert!(failed.is_empty(), "puts failed: {:?}", failed);
    assert!(live.iter().all(|b| *b), "a key read torn or failed: {:?}", live);
    Ok(())
}

// Two puts of one key in one clock tick are stamped apart, so the value a key reads is the one it
// reads after a restart, and its chunks were not retired by the put that lost.
#[test]
fn two_puts_one_clock_tick_settle_alike_across_a_restart() -> Outcome<()> {
    let _lock = lock();
    let (live, restarted, failed) = res!(two_puts_per_key("./test_db_chunk_set_tick", false));
    assert!(failed.is_empty(), "puts failed: {:?}", failed);
    assert!(live.iter().all(|b| *b), "a key read torn or failed: {:?}", live);
    assert!(restarted.iter().all(|b| *b), "after a restart a key read torn or failed: {:?}", restarted);
    Ok(())
}

// Two records of a key with one stamp: the cache, replay and the scan pick the same one, by place
// (`supersedes`), so a key reads the same whole value live and after a restart, the live set
// verifies, and the put that lost left nothing for the sweep.  Each key's two puts are made in
// turn from a thread of its own, so the two writers interleave across the keys.
#[test]
fn equal_stamps_pick_one_record_live_and_after_restart() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_equal";
    let lens = [1_100usize];
    let keys: Vec<Dat> = (0..12u8).map(|i| dat!(fmt!("equal {}", i))).collect();
    let mut bad = Vec::new();
    for round in 0..4u8 {
        let db = res!(start(dir, res!(cfg()), true));
        hooks::set_stale_stamp(Some(*res!(Timestamp::now())));
        let ws: Vec<_> = keys.iter().enumerate().map(|(i, k)| {
            let (db, k) = (db.clone(), k.clone());
            let seed = round.wrapping_mul(64).wrapping_add((i as u8).wrapping_mul(2));
            thread::spawn(move || -> Vec<String> {
                let mut errs = Vec::new();
                for j in 0..2u8 {
                    if let Err(e) = db.insert(k.clone(), val(1_100, seed.wrapping_add(j)), Uid::default(), None) {
                        errs.push(fmt!("{}", e).chars().take(200).collect::<String>());
                    }
                }
                errs
            })
        }).collect();
        for w in ws {
            match w.join() {
                Ok(errs) => for e in errs { bad.push(fmt!("round {} put: {}", round, e)); },
                Err(_) => bad.push(fmt!("round {} put panicked", round)),
            }
        }
        hooks::set_stale_stamp(None);
        let mut live = Vec::new();
        for k in &keys {
            let got = res!(db.get(k, None)).map(|(v, _)| v);
            if !matches!(&got, Some(v) if whole(v, &lens)) {
                bad.push(fmt!("round {} live {:?}: not whole", round, k));
            }
            live.push(got);
        }
        let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
        if !v.clean() { bad.push(fmt!("round {} live verify: {}", round, v.summary(4))); }
        res!(db.close());
        let db = res!(start(dir, res!(cfg()), false));
        for (k, want) in keys.iter().zip(&live) {
            match db.get(k, None) {
                Ok(got) => if got.map(|(v, _)| v) != *want {
                    bad.push(fmt!("round {} {:?}: the restart reads another value than live", round, k));
                },
                Err(e) => bad.push(fmt!("round {} {:?}: after the restart: {}", round, k, e)),
            }
        }
        let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
        if !v.clean() { bad.push(fmt!("round {} restarted verify: {}", round, v.summary(4))); }
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        if report.orphans_found != 0 {
            bad.push(fmt!("round {} sweep: {}", round, report.summary().replace('\n', " | ")));
        }
        res!(db.close());
    }
    let _ = std::fs::remove_dir_all(dir);
    assert!(bad.is_empty(), "equal stamps did not settle on one record:\n{}", bad.join("\n"));
    Ok(())
}

// With one writer a zone's records lie in write order, so of two puts of a key with one stamp the
// later is the later place and must be the value read, live and after a restart.  The interleaved
// test above catches a split between the paths only when replay happens to read in another order;
// this one catches a tie broken by anything but place every time.
#[test]
fn equal_stamps_one_writer_later_put_wins() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_equal_one";
    let mut c = res!(cfg());
    c.num_wbots_per_zone = 1;
    let keys: Vec<Dat> = (0..6u8).map(|i| dat!(fmt!("equal one {}", i))).collect();
    let db = res!(start(dir, c.clone(), true));
    hooks::set_stale_stamp(Some(*res!(Timestamp::now())));
    for (i, k) in keys.iter().enumerate() {
        for j in 0..2u8 {
            res!(db.insert(k.clone(), val(1_100, (i as u8) * 2 + j), Uid::default(), None));
        }
    }
    hooks::set_stale_stamp(None);
    let mut bad = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        if res!(db.get(k, None)).map(|(v, _)| v) != Some(val(1_100, (i as u8) * 2 + 1)) {
            bad.push(fmt!("live {:?}: not the later put", k));
        }
    }
    res!(db.close());
    let db = res!(start(dir, c, false));
    for (i, k) in keys.iter().enumerate() {
        if res!(db.get(k, None)).map(|(v, _)| v) != Some(val(1_100, (i as u8) * 2 + 1)) {
            bad.push(fmt!("restarted {:?}: not the later put", k));
        }
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(bad.is_empty(), "a tie of stamps was not broken by place:\n{}", bad.join("\n"));
    Ok(())
}

// The riskiest surface: a store reads its key back once its bunch key is durable and retires its
// own set if the key names another.  Under many concurrent chunked overwrites of a few keys, with
// slow file reads, no store may retire the set its key ends up naming: every key's current head
// fetches whole, the live set verifies clean, before and after a restart and after a sweep.
#[test]
fn store_never_retires_its_own_live_chunks_under_load() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_load";
    let lens = vec![1_100usize, 1_500, 2_300];
    let keys: Vec<Dat> = (0..4u8).map(|i| dat!(fmt!("load {}", i))).collect();
    let mut c = res!(cfg());
    c.cache_size_limit_bytes = 1;
    let mut bad = Vec::new();
    let mut failed = Vec::new();
    {
        let db = res!(start(dir, c.clone(), true));
        hooks::set_read_delay(Duration::from_millis(3));
        hooks::set_chunk_insert_delay(Duration::from_millis(2));
        let ws: Vec<_> = (0..12usize).map(|t| {
            let (db, keys, lens) = (db.clone(), keys.clone(), lens.clone());
            thread::spawn(move || -> Vec<String> {
                let mut errs = Vec::new();
                for n in 0..10usize {
                    let k = keys[(t + n) % keys.len()].clone();
                    let seed = (t as u8).wrapping_mul(37).wrapping_add(n as u8);
                    if let Err(e) = db.insert(k, val(lens[(t * 7 + n) % 3], seed), Uid::default(), None) {
                        errs.push(fmt!("{}", e).chars().take(200).collect::<String>());
                    }
                }
                errs
            })
        }).collect();
        for w in ws {
            match w.join() {
                Ok(errs) => failed.extend(errs),
                Err(_) => failed.push(fmt!("panicked")),
            }
        }
        hooks::set_read_delay(Duration::ZERO);
        hooks::set_chunk_insert_delay(Duration::ZERO);
        res!(check(&db, &keys, &lens, "live", &mut bad));
        res!(db.close());
        let db = res!(start(dir, c.clone(), false));
        res!(check(&db, &keys, &lens, "restarted", &mut bad));
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        msg!("load: {}", report.summary().replace('\n', " | "));
        res!(check(&db, &keys, &lens, "swept", &mut bad));
        res!(db.close());
    }
    let _ = std::fs::remove_dir_all(dir);
    assert!(failed.is_empty(), "puts failed: {:?}", failed);
    assert!(bad.is_empty(), "a store retired chunks its key still names: {:?}", bad);
    Ok(())
}

fn check(db: &TestDb, keys: &[Dat], lens: &[usize], phase: &str, bad: &mut Vec<String>) -> Outcome<()> {
    for k in keys {
        match res!(head(db, k)) {
            Some(t) => if let Err(e) = db.api().fetch_chunks(&Dat::Tup5u64(t), None) {
                bad.push(fmt!("{} {:?}: head {:?}: {}", phase, k, t, e).chars().take(300).collect());
            },
            None => bad.push(fmt!("{} {:?}: no head", phase, k)),
        }
        match db.get(k, None) {
            Ok(Some((v, _))) if whole(&v, lens) => (),
            other => bad.push(fmt!("{} {:?}: read {:?}", phase, k,
                other.map(|o| o.map(|(d, _)| fmt!("{:?}", d).chars().take(80).collect::<String>())))),
        }
    }
    let report = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    if !report.clean() {
        bad.push(fmt!("{} verify: {}", phase, report.summary(4)));
    }
    Ok(())
}

// A delete is a store with no chunks: its head tombstone is durable before the chunks of the value
// it deletes are retired, so a read made between the two finds the value whole or gone, never a
// current bunch key missing a chunk (fix A, 2026-10-09).  The delete is held between the two by
// `set_retire_delay`.  `marker` deletes by storing the deleted marker, as the distributed adapters
// erase.
fn read_during_delete(dir: &str, marker: bool) -> Outcome<()> {
    let _lock = lock();
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("deleted while read");
    let v = val(1_100, 5); // 3 chunks
    let tomb = Dat::Usr(oxedyne_fe2o3_o3db_sync::base::id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
    res!(db.insert(k.clone(), v.clone(), Uid::default(), None));
    let old = res!(res!(head(&db, &k)).ok_or_else(|| err!("No head for the value."; Test, Missing)));
    hooks::set_retire_delay(Duration::from_millis(300));
    let del = {
        let (db, k, tomb) = (db.clone(), k.clone(), tomb.clone());
        thread::spawn(move || {
            let r = if marker {
                db.api().store(k, tomb, Uid::default()).and_then(|resp| resp.recv_store_ack().map(|_| ()))
            } else {
                db.delete(&k, Uid::default(), None).map(|_| ())
            };
            r.map_err(|e| fmt!("{}", e))
        })
    };
    pause(Duration::from_millis(100));
    let mid = db.get(&k, None);
    let del = match del.join() { Ok(r) => r, Err(_) => Err(fmt!("panicked")) };
    hooks::set_retire_delay(Duration::ZERO);
    let after = db.get(&k, None);
    let gone = db.api().fetch_chunks(&Dat::Tup5u64(old), None).is_err();
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(del.is_ok(), "the delete failed: {:?}", del);
    match &mid {
        Ok(None) => (),
        Ok(Some((g, _))) if *g == v || *g == tomb => (),
        other => panic!("a read during the delete found neither the value nor its absence: {}",
            fmt!("{:?}", other).chars().take(300).collect::<String>()),
    }
    match &after {
        Ok(None) => (),
        Ok(Some((g, _))) if *g == tomb => (),
        other => panic!("the key does not read as deleted after the delete: {}",
            fmt!("{:?}", other).chars().take(300).collect::<String>()),
    }
    assert!(gone, "the deleted value's chunks were not retired");
    Ok(())
}

#[test]
fn read_during_delete_of_chunked_value_is_whole_or_none() -> Outcome<()> {
    read_during_delete("./test_db_chunk_set_read_delete", false)
}

#[test]
fn read_during_marker_store_delete_is_whole_or_none() -> Outcome<()> {
    read_during_delete("./test_db_chunk_set_read_marker", true)
}

// A delete stamped before the value at its key loses to it, and must leave that value whole: the
// delete retires the set it read only when the key no longer names it.
#[test]
fn stale_delete_keeps_the_newer_value() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_stale_delete";
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("stale delete");
    let v0 = val(1_100, 6);
    res!(db.insert(k.clone(), v0.clone(), Uid::default(), None));
    let t0 = match res!(db.get(&k, None)) {
        Some((_, meta)) => *meta.time,
        None => return Err(err!("v0 is missing."; Test, Missing)),
    };
    hooks::set_stale_stamp(Some(t0.saturating_sub(Duration::from_secs(1))));
    let del = db.delete(&k, Uid::default(), None);
    hooks::set_stale_stamp(None);
    let got = db.get(&k, None);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    res!(del);
    assert!(matches!(&got, Ok(Some((g, _))) if *g == v0), "a stale delete took the newer value: {}",
        fmt!("{:?}", got).chars().take(300).collect::<String>());
    Ok(())
}

// Fix B (2026-10-09): a chunk tombstone is forgotten once every older record of its key is durably
// gone, so a store whose chunked values are overwritten does not grow its caches for ever.

const VLEN: usize = 1_100; // three chunks of 400

fn cfg_b() -> Outcome<OzoneConfig> {
    let mut c = res!(cfg());
    c.data_file_max_bytes = 4_000;
    c.num_wbots_per_zone  = 1; // each writer has a live file of its own, so one puts a value in one file
    Ok(c)
}

fn start_gc(dir: &str, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let db = res!(setup::start_db(root, Some(res!(cfg_b())), schemes(), None, true, wipe));
    // Only `compact_now` collects, so a test places its tombstones and holds before any collection,
    // and the reports count every collection and deletion.
    res!(db.api().activate_gc(false));
    Ok(db)
}

#[derive(Debug, Default)]
struct Census {
    chunks:     usize,                              // chunk-key cache entries
    tombs:      BTreeMap<Vec<u8>, FileLocation>,    // chunk tombstones, where they lie
    files:      BTreeSet<u32>,                      // files of the live chunk entries
    entries:    usize,                              // all cache entries
    anc:        usize,                              // ancillary bytes of every cache
}

fn census(db: &TestDb) -> Outcome<Census> {
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let mut c = Census::default();
    for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
        c.entries += cache.map().len();
        c.anc += cache.get_ancillary_size();
        let tombs = cache.tomb_tracker().tombs();
        for (k, e) in cache.map() {
            if !is_chunk_key(k) { continue; }
            c.chunks += 1;
            if let CacheEntry::LocatedValue(mloc, _) = e {
                match tombs.contains_key(k) {
                    true    => { c.tombs.insert(k.clone(), *mloc.file_location()); },
                    false   => { c.files.insert(mloc.file_number()); },
                }
            }
        }
    }
    Ok(c)
}

// Every cache entry is one current record in the file states, and nothing waits on a record.
fn accounted(db: &TestDb, c: &Census, label: &str) -> Outcome<()> {
    let mut cur = 0;
    for (_, fmap) in res!(db.api().collect_file_states(scan_wait())) {
        for (fnum, fstat) in fmap.map() {
            assert!(fstat.pending_old_empty(), "{}: file {} has parked supersessions", label, fnum);
            cur += fstat.data_map().values().filter(|d| **d == DataState::Cur).count();
        }
    }
    assert_eq!(cur, c.entries, "{}: current records against cache entries", label);
    Ok(())
}

// Compacts until quiet: a dropped tombstone is flagged old only after its shadowed records go.
fn quiet(db: &TestDb) -> Outcome<CompactReport> {
    let mut sum = CompactReport::default();
    for _ in 0..3 {
        res!(db.api().settle_for_test(Duration::from_secs(20)));
        let r = res!(db.api().compact_now(Duration::from_secs(60)));
        sum.files_collected += r.files_collected;
        sum.files_deleted   += r.files_deleted;
        pause(Duration::from_millis(300));
    }
    Ok(sum)
}

// As `quiet`, with a file held, which keeps its old bytes and so cannot settle.
fn quiet_held(db: &TestDb) {
    for _ in 0..2 {
        let _ = db.api().settle_for_test(Duration::from_secs(20));
        let _ = db.api().compact_now(Duration::from_secs(3));
        pause(Duration::from_millis(300));
    }
}

fn filler(db: &TestDb, tag: &str, n: usize, seed: u8) -> Outcome<()> {
    for i in 0..n {
        res!(db.insert(dat!(fmt!("filler {} {:03}", tag, i)), val(100, seed), Uid::default(), None));
    }
    Ok(())
}

fn reads(db: &TestDb, k: &Dat, want: &Dat) -> Outcome<bool> {
    Ok(matches!(res!(db.get(k, None)), Some((v, _)) if v == *want))
}

fn accumulate(dir: &str, n: usize) -> Outcome<(usize, Census, CompactReport)> {
    let db = res!(start_gc(dir, true));
    let k = dat!("accumulate");
    res!(db.insert(k.clone(), val(VLEN, 0), Uid::default(), None));
    let base = res!(census(&db)).chunks;
    // Each compaction seals the value just written in a file of its own, which goes all-old once
    // the next overwrite lands, so its deletion, and not only a collection, reports the records
    // a tombstone shadows gone.
    let mut rep = CompactReport::default();
    for i in 1..=n {
        res!(db.insert(k.clone(), val(VLEN, i as u8), Uid::default(), None));
        let r = res!(db.api().compact_now(Duration::from_secs(60)));
        rep.files_collected += r.files_collected;
        rep.files_deleted   += r.files_deleted;
    }
    let r = res!(quiet(&db));
    rep.files_collected += r.files_collected;
    rep.files_deleted   += r.files_deleted;
    let c = res!(census(&db));
    let acc = accounted(&db, &c, "accumulate");
    let ok = res!(reads(&db, &k, &val(VLEN, n as u8)));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    res!(acc);
    assert!(ok, "the last value does not read back");
    Ok((base, c, rep))
}

#[test]
fn chunk_tombstones_do_not_accumulate() -> Outcome<()> {
    let _lock = lock();
    let (base1, c1, r1) = res!(accumulate("./test_db_chunk_set_accumulate_a", 140));
    let (base2, c2, r2) = res!(accumulate("./test_db_chunk_set_accumulate_b", 280));
    assert!(base1 >= 2, "the value was not chunked");
    assert_eq!(c1.chunks, base1, "140 overwrites leave {} tombstones", c1.tombs.len());
    assert_eq!(c2.chunks, base2, "280 overwrites leave {} tombstones", c2.tombs.len());
    assert!(c2.anc <= c1.anc + 256, "the ancillary size grows with overwrites: {} then {}", c1.anc, c2.anc);
    assert!(r1.files_collected + r2.files_collected > 0, "no file was collected");
    assert!(r1.files_deleted + r2.files_deleted > 0, "no all-old file was deleted");
    Ok(())
}

// A value, sealed in a file of its own, overwritten with its tombstones in later files.
fn overwritten(db: &TestDb, k: &Dat) -> Outcome<(usize, BTreeSet<u32>)> {
    res!(db.insert(k.clone(), val(VLEN, 1), Uid::default(), None));
    let c0 = res!(census(db));
    res!(filler(db, "a", 40, 1));
    res!(db.insert(k.clone(), val(VLEN, 2), Uid::default(), None));
    res!(filler(db, "b", 40, 1));
    Ok((c0.chunks, c0.files))
}

#[test]
fn failed_dir_sync_keeps_tombstone() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_dir_sync";
    let db = res!(start_gc(dir, true));
    let k = dat!("dir sync");
    let (base, _) = res!(overwritten(&db, &k));
    hooks::set_dir_sync_failure(true);
    quiet_held(&db);
    let c = census(&db);
    let acc = match &c { Ok(c) => accounted(&db, c, "dir sync"), Err(_) => Ok(()) };
    hooks::set_dir_sync_failure(false);
    let ok = reads(&db, &k, &val(VLEN, 2));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let c = res!(c);
    res!(acc);
    assert!(res!(ok), "the value does not read back");
    assert!(c.tombs.len() >= base, "{} tombstones kept after a failed directory sync, {} wanted",
        c.tombs.len(), base);
    Ok(())
}

#[test]
fn lone_chunk_tombstones_drop_at_restart() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_lone";
    let db = res!(start_gc(dir, true));
    let k = dat!("lone");
    let (base, _) = res!(overwritten(&db, &k));
    hooks::set_dir_sync_failure(true);
    quiet_held(&db);
    let before = census(&db);
    hooks::set_dir_sync_failure(false);
    res!(db.close());
    let before = res!(before);
    let db = res!(start_gc(dir, false));
    let after = census(&db);
    let rep = quiet(&db);
    let fin = census(&db);
    let acc = match &fin { Ok(c) => accounted(&db, c, "lone"), Err(_) => Ok(()) };
    let ok = reads(&db, &k, &val(VLEN, 2));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let (after, fin) = (res!(after), res!(fin));
    res!(rep);
    res!(acc);
    assert!(res!(ok), "the value does not read back");
    assert!(before.tombs.len() >= base, "the setup kept no tombstones");
    assert_eq!(after.chunks, base, "lone tombstones survived the restart: {}", after.tombs.len());
    assert_eq!(fin.chunks, base, "tombstones after compaction: {}", fin.tombs.len());
    Ok(())
}

#[test]
fn tombstone_outlives_uncollected_record() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_outlives";
    let db = res!(start_gc(dir, true));
    let k = dat!("outlives");
    let (base, files) = res!(overwritten(&db, &k));
    assert_eq!(files.len(), 1, "the first value is not in one file: {:?}", files);
    let f = files.iter().next().copied();
    hooks::set_collect_fails_for(f);
    quiet_held(&db);
    let held = census(&db);
    res!(db.close());
    let db = res!(start_gc(dir, false));
    let restarted = census(&db);
    hooks::set_collect_fails_for(None);
    let rep = quiet(&db);
    let fin = census(&db);
    let acc = match &fin { Ok(c) => accounted(&db, c, "outlives"), Err(_) => Ok(()) };
    let ok = reads(&db, &k, &val(VLEN, 2));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let (held, restarted, fin) = (res!(held), res!(restarted), res!(fin));
    res!(rep);
    res!(acc);
    assert!(res!(ok), "the value does not read back");
    assert!(held.tombs.len() >= base, "a tombstone dropped while its record is on disk: {}", held.tombs.len());
    assert!(restarted.tombs.len() >= base, "a tombstone dropped at a restart while its record is on disk: {}",
        restarted.tombs.len());
    assert_eq!(fin.chunks, base, "tombstones after the hold: {}", fin.tombs.len());
    Ok(())
}

#[test]
fn reanchored_tombstone_still_drops() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_reanchored";
    let db = res!(start_gc(dir, true));
    let k = dat!("reanchored");
    res!(db.insert(k.clone(), val(VLEN, 1), Uid::default(), None));
    let c0 = res!(census(&db));
    assert_eq!(c0.files.len(), 1, "the first value is not in one file: {:?}", c0.files);
    res!(filler(&db, "a", 40, 1));
    // Records ahead of the tombstones in their file, then made old, so that collecting that file
    // carries the tombstones to new offsets.
    res!(filler(&db, "b", 8, 1));
    res!(db.insert(k.clone(), val(VLEN, 2), Uid::default(), None));
    res!(filler(&db, "b", 8, 2));
    res!(filler(&db, "c", 40, 1));
    let placed = res!(census(&db));
    hooks::set_collect_fails_for(c0.files.iter().next().copied());
    quiet_held(&db);
    let held = census(&db);
    hooks::set_collect_fails_for(None);
    let rep = quiet(&db);
    let fin = census(&db);
    let acc = match &fin { Ok(c) => accounted(&db, c, "reanchored"), Err(_) => Ok(()) };
    let ok = reads(&db, &k, &val(VLEN, 2));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let (held, fin) = (res!(held), res!(fin));
    res!(rep);
    res!(acc);
    assert!(res!(ok), "the value does not read back");
    let moved = placed.tombs.iter().filter(|(t, floc)| match held.tombs.get(*t) {
        Some(now) => now.start != floc.start,
        None => false,
    }).count();
    assert!(moved > 0, "no tombstone was carried by a collection: {:?} then {:?}", placed.tombs, held.tombs);
    assert_eq!(fin.chunks, c0.chunks, "re-anchored tombstones did not drop: {}", fin.tombs.len());
    Ok(())
}

#[test]
fn legacy_tombstone_waits_for_every_older_record() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_legacy_wait";
    let db = res!(start_gc(dir, true));
    let k = dat!("legacy wait");
    let (kbuf, _, _) = res!(db.api().ozone_key_dat(&k, None));
    let legacy = OzoneApi::<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>::legacy_chunk_set_id(&kbuf);
    let mut first = BTreeSet::new();
    for (seed, tag) in [(1u8, "a"), (2u8, "b")] {
        let resp = db.api().responder();
        res!(db.api().store_dat_using_responder_forcing_set_id(
            k.clone(), val(VLEN, seed), Uid::default(), None, resp.clone(), legacy));
        res!(resp.recv_store_ack());
        if seed == 1 {
            first = res!(census(&db)).files;
        }
        res!(filler(&db, tag, 40, seed));
    }
    let old = res!(res!(head(&db, &k)).ok_or_else(|| err!("No legacy head."; Test, Missing)));
    let c0 = res!(census(&db));
    assert_eq!(first.len(), 1, "the first legacy value is not in one file: {:?}", first);
    assert!(c0.files.is_disjoint(&first), "the two legacy values share a file");
    let f1 = first.iter().next().copied();
    res!(db.insert(k.clone(), val(VLEN, 3), Uid::default(), None));
    res!(filler(&db, "c", 40, 3));
    hooks::set_collect_fails_for(f1);
    quiet_held(&db);
    let held = census(&db);
    res!(db.close());
    let db = res!(start_gc(dir, false));
    let restarted = census(&db);
    let report = sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO);
    let gone = db.api().fetch_chunks(&Dat::Tup5u64(old), None).is_err();
    hooks::set_collect_fails_for(None);
    let rep = quiet(&db);
    let fin = census(&db);
    let ok = reads(&db, &k, &val(VLEN, 3));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    let (held, restarted, fin, report) = (res!(held), res!(restarted), res!(fin), res!(report));
    res!(rep);
    assert!(res!(ok), "the value does not read back");
    assert!(held.tombs.len() >= c0.chunks, "a legacy tombstone dropped with an older record on disk: {}",
        held.tombs.len());
    assert!(restarted.tombs.len() >= c0.chunks, "a legacy tombstone dropped at the restart: {}",
        restarted.tombs.len());
    assert_eq!(report.orphans_found, 0, "a retired legacy chunk came back");
    assert!(gone, "the legacy value reads back after its retirement");
    assert_eq!(fin.chunks, c0.chunks, "tombstones after the hold: {}", fin.tombs.len());
    Ok(())
}

// A record reported gone before the replay ends drops no tombstone, since an older record of its
// key may lie in a file not yet replayed.  Driven on a cache directly: the start sequence lets no
// collection begin before the zone bot sends `ReplayDone`, so no store can reach this order.
#[test]
fn no_tombstone_drops_before_replay_done() -> Outcome<()> {
    type C = Cache<{ UID_LEN }, Uid>;
    let mut cache = C::new(None);
    let k = res!(Dat::Tup5u64([1, 2, 3, 4, 5]).to_bytes(Vec::new()));
    assert!(is_chunk_key(&k), "the key is not a chunk key");
    let meta = |s: u64| Meta::<{ UID_LEN }, Uid> { time: Timestamp::new(s, 0), user: Uid::default() };
    let floc = |f: u32| FileLocation { fnum: f, start: 0, klen: k.len() as u64, vlen: 10 };
    let put = |cache: &mut C, cind: Option<usize>, f: u32, s: u64| -> Outcome<()> {
        let sup = res!(cache.insert(k.clone(), None, cind, floc(f), meta(s)));
        cache.note(&k, cind, &floc(f), &meta(s), &sup);
        Ok(())
    };
    let current = |cache: &C| match cache.map().get(&k) {
        Some(CacheEntry::LocatedValue(mloc, _)) => Some(mloc.meta().time.secs()),
        _ => None,
    };
    // The replay meets a chunk record in file 1, then the tombstone in file 3; file 2 waits.
    res!(put(&mut cache, Some(0), 1, 1));
    res!(put(&mut cache, None, 3, 3));
    let early = res!(cache.records_gone(1, &[res!(RecordDigest::new(&k, &meta(1)))]));
    assert!(early.is_empty(), "a tombstone dropped before the replay ended: {:?}", early);
    // File 2's record of the key arrives late, and must not become current.
    res!(put(&mut cache, Some(0), 2, 2));
    assert_eq!(current(&cache), Some(3), "a record older than the tombstone became current");
    let at_end = res!(cache.replay_done());
    assert!(at_end.is_empty(), "the tombstone dropped with file 2's record unaccounted for");
    let last = res!(cache.records_gone(2, &[res!(RecordDigest::new(&k, &meta(2)))]));
    assert_eq!(last.len(), 1, "the tombstone did not drop once its last older record went");
    assert_eq!(current(&cache), None, "the dropped tombstone is still in the cache");
    Ok(())
}

// Run 2 (2026-10-10): a store retires the set of the head it displaced, whatever order
// overlapping puts and deletes land in, so live chunk records are bounded by the current heads
// with no sweep.

// Live chunk records, tombstones aside.
fn live_chunks(db: &TestDb) -> Outcome<usize> {
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let mut n = 0;
    for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
        let tombs = cache.tomb_tracker().tombs();
        for (k, e) in cache.map() {
            if is_chunk_key(k) && matches!(e, CacheEntry::LocatedValue(..)) && !tombs.contains_key(k) {
                n += 1;
            }
        }
    }
    Ok(n)
}

// Chunks the current heads name.
fn named(db: &TestDb, keys: &[Dat]) -> Outcome<usize> {
    let mut n = 0;
    for k in keys {
        if let Some(t) = res!(head(db, k)) {
            n += t[3] as usize;
        }
    }
    Ok(n)
}

fn whole_or_none(r: &Outcome<Option<(Dat, Meta<{ UID_LEN }, Uid>)>>) -> bool {
    match r {
        Ok(None)                        => true,
        Ok(Some((Dat::Usr(..), _)))     => true, // a tombstone
        Ok(Some((v, _)))                => whole(v, &[VLEN]),
        Err(_)                          => false,
    }
}

// Puts B, A and C and a delete of each key, staggered and slowed so that they overlap.
fn overlap_round(db: &TestDb, keys: &[Dat], r: usize, bad: &mut Vec<String>) -> Outcome<()> {
    hooks::set_chunk_insert_delay(Duration::from_millis(40));
    let mut ops = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        // The delete lands third in even rounds and last in odd ones.
        let del_at = if r % 2 == 0 { 30 } else { 45 };
        for (j, (at, put)) in [(0u64, true), (15, true), (del_at, false), (75 - del_at, true)].into_iter().enumerate() {
            let (db, k) = (db.clone(), k.clone());
            let seed = ((r * 16 + i * 4 + j) % 251) as u8;
            ops.push(thread::spawn(move || {
                thread::sleep(Duration::from_millis(at));
                let res = match put {
                    true    => db.insert(k, val(VLEN, seed), Uid::default(), None).map(|_| ()),
                    false   => db.delete(&k, Uid::default(), None).map(|_| ()),
                };
                res.map_err(|e| fmt!("{}", e))
            }));
        }
    }
    for _ in 0..4 {
        for k in keys {
            let got = db.get(k, None);
            if !whole_or_none(&got) {
                bad.push(fmt!("round {}: a read mid-round was not whole: {:?}", r,
                    got.map(|o| o.map(|(d, _)| fmt!("{:?}", d).chars().take(80).collect::<String>()))));
            }
        }
        pause(Duration::from_millis(20));
    }
    for op in ops {
        match op.join() {
            Ok(Ok(()))  => (),
            Ok(Err(e))  => bad.push(fmt!("round {}: {}", r, e.chars().take(200).collect::<String>())),
            Err(_)      => bad.push(fmt!("round {}: an op panicked", r)),
        }
    }
    hooks::set_chunk_insert_delay(Duration::ZERO);
    for k in keys {
        let got = db.get(k, None);
        if !whole_or_none(&got) {
            bad.push(fmt!("round {}: {:?} does not read whole after the round", r, k));
        }
    }
    Ok(())
}

#[test]
fn overlapping_writes_leave_no_live_orphan() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_overlap_bound";
    let keys: Vec<Dat> = (0..3).map(|i| dat!(fmt!("overlap bound {}", i))).collect();
    let mut bad = Vec::new();
    let db = res!(start_gc(dir, true));
    for r in 0..20 {
        res!(overlap_round(&db, &keys, r, &mut bad));
    }
    // A round can end in a delete, so every key is given a value before the restart: each then
    // replays a head whose bytes are not resident, which the first write over it displaces.
    for (i, k) in keys.iter().enumerate() {
        res!(db.insert(k.clone(), val(VLEN, 0xA0 + i as u8), Uid::default(), None));
    }
    res!(quiet(&db));
    let (live1, named1) = (res!(live_chunks(&db)), res!(named(&db, &keys)));
    res!(db.close());
    let db = res!(start_gc(dir, false));
    for r in 20..22 {
        res!(overlap_round(&db, &keys, r, &mut bad));
    }
    res!(quiet(&db));
    let (live2, named2) = (res!(live_chunks(&db)), res!(named(&db, &keys)));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    msg!("overlap bound: live {} named {}, after a restart live {} named {}", live1, named1, live2, named2);
    assert!(bad.is_empty(), "{:#?}", bad);
    assert_eq!(live1, named1, "live chunk records against those the current heads name");
    assert_eq!(live2, named2, "after a restart, live chunk records against those the current heads name");
    Ok(())
}

// Two forced values under one legacy set, of one length: the second overwrites the first's
// chunks in place, and its head must retire nothing, or it retires its own chunks.
#[test]
fn forced_legacy_overwrite_retires_nothing() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_set_forced_legacy";
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("forced legacy");
    let (kbuf, _, _) = res!(db.api().ozone_key_dat(&k, None));
    let legacy = OzoneApi::<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>::legacy_chunk_set_id(&kbuf);
    for seed in [1u8, 2] {
        let resp = db.api().responder();
        res!(db.api().store_dat_using_responder_forcing_set_id(
            k.clone(), val(1_100, seed), Uid::default(), None, resp.clone(), legacy));
        res!(resp.recv_store_ack());
    }
    let live = res!(db.get(&k, None)).map(|(v, _)| v);
    res!(db.close());
    let db = res!(start(dir, res!(cfg()), false));
    let after = res!(db.get(&k, None)).map(|(v, _)| v);
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(live == Some(val(1_100, 2)), "the second forced value does not read whole live");
    assert!(after == Some(val(1_100, 2)), "the second forced value does not read whole after a restart");
    assert!(v.clean(), "verify after restart: {}", v.summary(4));
    Ok(())
}
