//! Opus QA agent 2 of A3 round 2a (2026-10-09): chunked values.  Each test is a finding's proof
//! and fails on bebb931d.  The hooks are process-wide, so every test takes one lock.

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
        Arc,
        Mutex,
        MutexGuard,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread,
    time::Duration,
};

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn cfg(cbots: u16) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = cbots;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 2;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    Ok(c)
}

// A value of `len` bytes whose every byte is `seed ^ j`, so a value assembled from the chunks of
// two values is told by `whole`.
fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

// Is this one value, whole, of one of the lengths stored?
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

struct Seen { none: usize, ok: usize, torn: usize, err: usize, sample: Option<String> }

fn reader(db: TestDb, k: Dat, lens: Vec<usize>, done: Arc<AtomicBool>) -> thread::JoinHandle<Seen> {
    thread::spawn(move || {
        let mut s = Seen { none: 0, ok: 0, torn: 0, err: 0, sample: None };
        while !done.load(Ordering::Relaxed) {
            match db.get(&k, None) {
                Ok(None) => s.none += 1,
                Ok(Some((v, _))) if whole(&v, &lens) => s.ok += 1,
                Ok(Some((v, _))) => {
                    s.torn += 1;
                    if s.sample.is_none() {
                        s.sample = Some(fmt!("{:?}", v).chars().take(160).collect());
                    }
                },
                Err(e) => {
                    s.err += 1;
                    if s.sample.is_none() {
                        s.sample = Some(fmt!("{}", e).chars().take(300).collect());
                    }
                },
            }
            thread::sleep(Duration::from_millis(1));
        }
        s
    })
}

fn join(h: thread::JoinHandle<Seen>) -> Outcome<Seen> {
    match h.join() {
        Ok(s) => Ok(s),
        Err(_) => Err(err!("A reader thread panicked."; Test, Invalid)),
    }
}

// F1.  A value overwritten by one of the same length is written over its own chunk records in
// place: the chunk set identifier comes from the key, and a chunk key carries only the set, the
// index, the length, the count and the size.  The new bunch key is the old one byte for byte, so
// publishing it last protects nothing, and a reader finds the old bunch key over a mixture of old
// and new chunks for as long as the chunks take to land.
#[test]
fn f1_same_length_overwrite_is_never_read_torn() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_f1";
    // No hook: the chunks go to two writers at once, and each writer's barrier releases its own.
    let db = res!(start(dir, res!(cfg(8)), true));
    let k = dat!("same length overwrite");
    let len = 40_000usize; // 100 chunks
    let lens = vec![len];
    res!(db.insert(k.clone(), val(len, 0), Uid::default(), None));

    let done = Arc::new(AtomicBool::new(false));
    let rs: Vec<_> = (0..3).map(|_| reader(db.clone(), k.clone(), lens.clone(), done.clone())).collect();
    let mut put = Ok(());
    for seed in 1..60u8 {
        if let Err(e) = db.insert(k.clone(), val(len, seed), Uid::default(), None) {
            put = Err(e);
        }
    }
    done.store(true, Ordering::Relaxed);
    let (mut torn, mut errs, mut oks, mut sample) = (0, 0, 0, None);
    for r in rs {
        let s = res!(join(r));
        torn += s.torn; errs += s.err; oks += s.ok;
        if sample.is_none() { sample = s.sample; }
    }
    res!(put);
    msg!("F1: whole {} torn {} err {}; first bad: {:?}", oks, torn, errs, sample);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(torn + errs, 0, "a reader found a value assembled from two values' chunks: {:?}", sample);
    Ok(())
}

// F2.  The same in-place overwrite interrupted after one chunk has landed -- the process stopped,
// which the new store makes more likely by sending every chunk at once and the bunch key only
// later -- leaves the old bunch key over one new chunk and the rest old, for good.  The control
// does the same with a value of another length, whose chunks have keys of their own: the old value
// survives whole.
#[test]
fn f2_overwrite_interrupted_after_one_chunk_keeps_a_whole_value() -> Outcome<()> {
    let _lock = lock();
    let mut results = Vec::new();
    for (label, newlen) in [("control, other length", 1_500usize), ("same length", 1_100usize)] {
        let dir = "./test_db_qa2_f2";
        let k = dat!("interrupted overwrite");
        {
            let db = res!(start(dir, res!(cfg(4)), true));
            res!(db.insert(k.clone(), val(1_100, 1), Uid::default(), None));
            // The new value, prepared as a store prepares it; only its first chunk reaches a
            // writer before the "crash".
            let resp = db.api().responder();
            let mut msgs = res!(db.api().prepare_write_dat(
                k.clone(), val(newlen, 2), Uid::default(), None, resp.clone(), None));
            assert!(msgs.len() >= 3, "the value was not chunked");
            let first = msgs.remove(1);
            res!(db.api().store_bytes(vec![first]));
            res!(resp.recv_write_acks(1, constant::USER_REQUEST_TIMEOUT, constant::DURABILITY_TIMEOUT));
            let live = res!(db.get(&k, None));
            let live_whole = match &live { Some((v, _)) => whole(v, &[1_100, newlen]), None => false };
            msg!("F2 {}: before the restart a reader gets a whole value: {}", label, live_whole);
            res!(db.close());
        }
        let db = res!(start(dir, res!(cfg(4)), false));
        let got = db.get(&k, None);
        let ok = match &got {
            Ok(Some((v, _))) => *v == val(1_100, 1),
            _ => false,
        };
        msg!("F2 {}: after the restart the old value reads back whole: {}", label, ok);
        results.push((label, ok));
        res!(db.close());
        let _ = std::fs::remove_dir_all(dir);
    }
    assert!(results[0].1, "the control failed: an interrupted overwrite of another length lost the old value");
    assert!(results[1].1, "an overwrite interrupted after one chunk left a value of mixed chunks, read as if whole");
    Ok(())
}

// F4.  Many puts and deletes at once on one key, of three lengths (one not chunked).  Afterwards
// the key holds nothing or one value whole, and every read during the race gets the same.
#[test]
fn f4_concurrent_puts_and_deletes_on_one_key_settle_whole() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_f4";
    let db = res!(start(dir, res!(cfg(8)), true));
    let k = dat!("contended");
    let lens = vec![300usize, 1_100, 1_500];
    let mut bad_final = Vec::new();
    let mut failed = Vec::new();
    let (mut torn, mut errs, mut sample) = (0, 0, None);
    hooks::set_chunk_insert_delay(Duration::from_millis(20));
    for round in 0..6u8 {
        let done = Arc::new(AtomicBool::new(false));
        let r = reader(db.clone(), k.clone(), lens.clone(), done.clone());
        let ws: Vec<_> = (0..16usize).map(|i| {
            let (db, k, lens) = (db.clone(), k.clone(), lens.clone());
            thread::spawn(move || -> Result<(), String> {
                if i % 5 == 4 {
                    db.delete(&k, Uid::default(), None).map(|_| ()).map_err(|e| fmt!("{}", e))
                } else {
                    let seed = round.wrapping_mul(31).wrapping_add(i as u8);
                    db.insert(k, val(lens[i % 3], seed), Uid::default(), None)
                        .map(|_| ()).map_err(|e| fmt!("{}", e))
                }
            })
        }).collect();
        for w in ws {
            match w.join() {
                Ok(Ok(())) => (),
                Ok(Err(e)) => failed.push(e.chars().take(200).collect::<String>()),
                Err(_) => failed.push(fmt!("panicked")),
            }
        }
        done.store(true, Ordering::Relaxed);
        let s = res!(join(r));
        torn += s.torn; errs += s.err;
        if sample.is_none() { sample = s.sample; }
        match db.get(&k, None) {
            Ok(None) => (),
            Ok(Some((v, _))) if whole(&v, &lens) => (),
            other => bad_final.push(fmt!("{:?}", other).chars().take(200).collect::<String>()),
        }
    }
    hooks::set_chunk_insert_delay(Duration::ZERO);
    msg!("F4: torn {} err {} during; finals bad {:?}; failed calls {:?}; first bad read {:?}",
        torn, errs, bad_final, failed, sample);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(bad_final.is_empty(), "the key settled on a value that is not whole: {:?}", bad_final);
    assert!(failed.is_empty(), "calls failed: {:?}", failed);
    assert_eq!(torn + errs, 0, "reads during the race were torn or failed: {:?}", sample);
    Ok(())
}

// F3b.  F3 with reads that go to the files and take their time: the cache holds no values, and
// each file read is held 15 ms, as a loaded disk would hold it.
#[test]
fn f3b_slow_reader_never_errs_while_a_key_is_overwritten_with_a_new_length() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_f3b";
    let mut c = res!(cfg(8));
    c.cache_size_limit_bytes = 1;
    let db = res!(start(dir, c, true));
    let k = dat!("changing length, slow reader");
    let lens = vec![1_100usize, 1_500];
    res!(db.insert(k.clone(), val(lens[0], 0), Uid::default(), None));

    let done = Arc::new(AtomicBool::new(false));
    hooks::set_read_delay(Duration::from_millis(15));
    let rs: Vec<_> = (0..3).map(|_| reader(db.clone(), k.clone(), lens.clone(), done.clone())).collect();
    let mut put = Ok(());
    for i in 1..40usize {
        if let Err(e) = db.insert(k.clone(), val(lens[i % 2], i as u8), Uid::default(), None) {
            put = Err(e);
        }
        thread::sleep(Duration::from_millis(20));
    }
    done.store(true, Ordering::Relaxed);
    let (mut torn, mut errs, mut oks, mut nones, mut sample) = (0, 0, 0, 0, None);
    for r in rs {
        let s = res!(join(r));
        torn += s.torn; errs += s.err; oks += s.ok; nones += s.none;
        if sample.is_none() { sample = s.sample; }
    }
    hooks::set_read_delay(Duration::ZERO);
    res!(put);
    msg!("F3b: whole {} none {} torn {} err {}; first bad: {:?}", oks, nones, torn, errs, sample);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(torn + errs + nones, 0, "a slow reader of a live key failed while it was overwritten: {:?}", sample);
    Ok(())
}
