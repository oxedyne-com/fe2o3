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
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    api::OzoneApi,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
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
