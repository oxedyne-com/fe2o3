//! QA agent 2 (2026-10-09): fix B reclaim, GC and restart, adversarial.
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
    data::cache::{
        CacheEntry,
        is_chunk_key,
    },
    sweep,
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
        atomic::{
            AtomicBool,
            AtomicUsize,
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

fn cfg_b() -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.num_wbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.rest_chunk_threshold  = 1_000;
    c.rest_chunk_bytes      = 400;
    c.sync_on_write         = true;
    c.data_file_max_bytes   = 4_000;
    Ok(c)
}

const VLEN: usize = 1_100;

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn whole(v: &Dat) -> bool {
    match v {
        Dat::BU32(b) | Dat::BU64(b) | Dat::BU8(b) | Dat::BU16(b) => {
            if b.len() != VLEN { return false; }
            let seed = b[0];
            b.iter().enumerate().all(|(j, x)| *x == seed ^ (j as u8))
        },
        _ => false,
    }
}

fn start(dir: &str, c: OzoneConfig, gc_on: bool, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(c), schemes(), None, gc_on, wipe)
}

fn scan_wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

fn head(db: &TestDb, k: &Dat) -> Outcome<Option<[u64; 5]>> {
    match res!(db.api().get_head_wait(k, None)) {
        Some((Dat::Tup5u64(t), _)) => Ok(Some(t)),
        _ => Ok(None),
    }
}

#[derive(Debug, Default, Clone)]
struct Census {
    chunks: usize,
    tombs:  usize,
    live:   usize,
    anc:    usize,
    older:  usize,
    disk:   u64,
}

fn census(db: &TestDb, dir: &str) -> Outcome<Census> {
    let _ = db.api().settle_for_test(Duration::from_secs(20));
    let mut c = Census::default();
    for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
        c.anc += cache.get_ancillary_size();
        let tt = cache.tomb_tracker();
        c.older += tt.older().values().map(|s| s.len()).sum::<usize>();
        for (k, e) in cache.map() {
            if !is_chunk_key(k) { continue; }
            if let CacheEntry::LocatedValue(..) = e {
                c.chunks += 1;
                match tt.tombs().contains_key(k) {
                    true    => c.tombs += 1,
                    false   => c.live += 1,
                }
            }
        }
    }
    c.disk = dir_len(&res!(Path::new(dir).canonicalize()));
    Ok(c)
}

// Background GC on throughout, several keys overwritten from one thread while another reads.
// The footprint, the tombstones held and the ancillary size must plateau, not grow with writes.
#[test]
fn qa2_bg_gc_overwrite_bounded() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_bg_bounded";
    let db = res!(start(dir, res!(cfg_b()), true, true));
    let keys: Vec<Dat> = (0..3).map(|i| dat!(fmt!("qa2 bounded {}", i))).collect();
    for k in &keys {
        res!(db.insert(k.clone(), val(VLEN, 0), Uid::default(), None));
    }
    let stop = AtomicBool::new(false);
    let torn = AtomicUsize::new(0);
    let missing = AtomicUsize::new(0);
    let mut series = Vec::new();
    let rounds = 5;
    let per = 120;
    let result: Outcome<()> = thread::scope(|s| {
        s.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                for k in &keys {
                    match db.get(k, None) {
                        Ok(Some((v, _))) => if !whole(&v) { torn.fetch_add(1, Ordering::Relaxed); },
                        Ok(None) => { missing.fetch_add(1, Ordering::Relaxed); },
                        Err(_) => { torn.fetch_add(1, Ordering::Relaxed); },
                    }
                }
            }
        });
        let mut n = 1u32;
        for _ in 0..rounds {
            for _ in 0..per {
                for k in &keys {
                    if let Err(e) = db.insert(k.clone(), val(VLEN, (n % 251) as u8 + 1), Uid::default(), None) {
                        stop.store(true, Ordering::Relaxed);
                        return Err(e);
                    }
                }
                n += 1;
            }
            thread::sleep(Duration::from_secs(2));
            match census(&db, dir) {
                Ok(c) => series.push(c),
                Err(e) => { stop.store(true, Ordering::Relaxed); return Err(e); },
            }
        }
        stop.store(true, Ordering::Relaxed);
        Ok(())
    });
    let mut fin = Vec::new();
    for k in &keys {
        fin.push(res!(db.get(k, None)).map(|(v, _)| whole(&v)).unwrap_or(false));
    }
    res!(db.close());
    // Restart: replay drops the lone ones; the store must be the same three values.
    let db = res!(start(dir, res!(cfg_b()), true, false));
    let after = res!(census(&db, dir));
    let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait()));
    let mut fin2 = Vec::new();
    for k in &keys {
        fin2.push(res!(db.get(k, None)).map(|(v, _)| whole(&v)).unwrap_or(false));
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    res!(result);
    for (i, c) in series.iter().enumerate() {
        println!("QA2 bounded round {} ({} overwrites): {:?}", i + 1, (i + 1) * per * keys.len(), c);
    }
    println!("QA2 bounded after restart: {:?}, sweep {:?}", after, report.orphans_found);
    assert_eq!(torn.load(Ordering::Relaxed), 0, "torn or failed reads");
    assert_eq!(missing.load(Ordering::Relaxed), 0, "reads found no value");
    assert!(fin.iter().all(|b| *b) && fin2.iter().all(|b| *b), "final values {:?} {:?}", fin, fin2);
    assert_eq!(report.orphans_found, 0, "orphans after restart");
    let first = &series[1];
    let last = &series[rounds - 1];
    assert!(last.tombs <= first.tombs * 2 + 40, "tombstones grow: {} then {}", first.tombs, last.tombs);
    assert!(last.anc <= first.anc * 2 + 4_096, "ancillary grows: {} then {}", first.anc, last.anc);
    assert!(last.disk <= first.disk * 2 + 40_000, "disk grows: {} then {}", first.disk, last.disk);
    Ok(())
}

// The chunk_leak shrink geometry with background GC, sampled every ten overwrites over two
// hundred, to tell a linear leak from sampling jitter around a plateau.
#[test]
fn qa2_shrink_footprint_series() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_shrink";
    let mut c = res!(setup::default_cfg());
    c.data_file_max_bytes    = 16_000;
    c.rest_chunk_threshold   = 3_000;
    c.rest_chunk_bytes       = 1_000;
    c.sync_on_write          = true;
    c.zone_overrides         = BTreeMap::new();
    let db = res!(start(dir, c, true, true));
    let k = dat!("chunked:shrinking");
    let big = Dat::BU32(vec![7u8; 6_000]);
    res!(db.insert(k.clone(), big, Uid::default(), None));
    let mut samples = Vec::new();
    for i in 1..=200u32 {
        let v = Dat::BU32((0..3_200).map(|j| (i as u8).wrapping_add((j % 251) as u8)).collect());
        res!(db.insert(k.clone(), v, Uid::default(), None));
        if i % 10 == 0 {
            thread::sleep(Duration::from_secs(3));
            let root = res!(Path::new(dir).canonicalize());
            let mut tombs = 0;
            for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
                tombs += cache.tomb_tracker().tombs().len();
            }
            samples.push((i, dir_len(&root), tombs));
        }
    }
    let rep = res!(db.api().compact_now(Duration::from_secs(60)));
    let root = res!(Path::new(dir).canonicalize());
    let compacted = dir_len(&root);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    for s in &samples {
        println!("QA2 shrink after {:3}: dir {:6} bytes, tombstones held {}", s.0, s.1, s.2);
    }
    println!("QA2 shrink compacted: {} bytes, {:?}", compacted, rep);
    let w10 = samples[0].1;
    let mx = samples.iter().map(|s| s.1).max().unwrap_or(0);
    let late: Vec<u64> = samples[10..].iter().map(|s| s.1).collect();
    let late_avg = late.iter().sum::<u64>() / late.len() as u64;
    let early_avg = samples[1..10].iter().map(|s| s.1).sum::<u64>() / 9;
    println!("QA2 shrink: first {} max {} early avg {} late avg {}", w10, mx, early_avg, late_avg);
    assert!(late_avg <= early_avg * 3 / 2, "linear growth: early avg {} late avg {}", early_avg, late_avg);
    Ok(())
}

// Cycles of overwrite, delete, held files and failed directory syncs, each closed and reopened
// with GC on.  A set once retired must never read again, the sweep must find no orphan after a
// clean close, and every live value must read whole.
#[test]
fn qa2_restart_cycles_no_resurrection() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_cycles";
    let keys: Vec<Dat> = (0..4).map(|i| dat!(fmt!("qa2 cycle {}", i))).collect();
    let mut retired: Vec<[u64; 5]> = Vec::new();
    let mut want: BTreeMap<usize, Option<u8>> = BTreeMap::new();
    let mut bad: Vec<String> = Vec::new();
    let mut seed = 1u8;
    for cycle in 0..10usize {
        let db = res!(start(dir, res!(cfg_b()), true, cycle == 0));
        // After the replay: retired sets stay gone, no orphan, live values whole.
        for h in &retired {
            if db.api().fetch_chunks(&Dat::Tup5u64(*h), None).is_ok() {
                bad.push(fmt!("cycle {}: retired set {:?} reads again", cycle, h));
            }
        }
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait()));
        if report.orphans_found != 0 {
            bad.push(fmt!("cycle {}: {} orphans after restart", cycle, report.orphans_found));
        }
        for (i, w) in &want {
            let got = res!(db.get(&keys[*i], None));
            match (w, got) {
                (Some(s), Some((v, _))) => if v != val(VLEN, *s) {
                    bad.push(fmt!("cycle {}: key {} wrong value", cycle, i));
                },
                (None, None) => (),
                (w, g) => bad.push(fmt!("cycle {}: key {} want {:?} got some={}", cycle, i, w, g.is_some())),
            }
        }
        let c = res!(census(&db, dir));
        println!("QA2 cycle {} start: {:?}", cycle, c);
        // A hold on some existing file, or failed directory syncs, for part of the cycle.
        match cycle % 3 {
            0 => hooks::set_collect_fails_for(Some((cycle as u32 % 5) + 1)),
            1 => hooks::set_dir_sync_failure(true),
            _ => (),
        }
        for round in 0..6 {
            for (i, k) in keys.iter().enumerate() {
                if let Some(h) = res!(head(&db, k)) {
                    if (round + i + cycle) % 7 == 3 {
                        res!(db.delete(k, Uid::default(), None));
                        retired.push(h);
                        want.insert(i, None);
                        continue;
                    }
                    retired.push(h);
                }
                seed = seed.wrapping_add(1).max(1);
                res!(db.insert(k.clone(), val(VLEN, seed), Uid::default(), None));
                want.insert(i, Some(seed));
            }
            if round == 3 {
                hooks::set_collect_fails_for(None);
                hooks::set_dir_sync_failure(false);
            }
        }
        if cycle % 2 == 0 {
            let _ = db.api().compact_now(Duration::from_secs(30));
        }
        thread::sleep(Duration::from_millis(500));
        res!(db.close());
    }
    let db = res!(start(dir, res!(cfg_b()), true, false));
    for h in &retired {
        if db.api().fetch_chunks(&Dat::Tup5u64(*h), None).is_ok() {
            bad.push(fmt!("final: retired set {:?} reads again", h));
        }
    }
    let fin = res!(census(&db, dir));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    println!("QA2 cycles final: {:?}", fin);
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// The chunk-form key refusal: a small user value under a Tup5u64 key is refused, a delete of
// one is not, and an ordinary key that merely starts with the Tup5u64 code byte is not refused.
#[test]
fn qa2_chunk_form_key_refusal() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_refusal";
    let db = res!(start(dir, res!(cfg_b()), false, true));
    let tk = Dat::Tup5u64([1, 2, 3, 4, 5]);
    let small = db.insert(tk.clone(), dat!("small"), Uid::default(), None);
    let del = db.delete(&tk, Uid::default(), None);
    // Byte strings whose raw form starts with 0x93 are framed by their own Dat code, so never
    // decode as a Tup5u64.
    let mut raw = vec![0x93u8];
    raw.extend_from_slice(&[0u8; 40]);
    let bk = Dat::BU8(raw);
    let bytes_key = db.insert(bk.clone(), dat!("v"), Uid::default(), None);
    let read_back = db.get(&bk, None);
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    assert!(small.is_err(), "a small value under a Tup5u64 key was accepted");
    assert!(del.is_ok(), "a delete under a Tup5u64 key was refused: {:?}", del.err());
    assert!(bytes_key.is_ok(), "a byte key starting 0x93 was refused");
    assert!(matches!(res!(read_back), Some(_)), "the byte key does not read back");
    Ok(())
}

// A store's config.jdat keeps `init_load_caches: true`, because a rolled-back binary needs the
// field to read it, and an unchanged open and close leaves the file byte for byte as it was.  A
// stored `false` asks for an open without replay, which no longer exists, and is refused by name.
#[test]
fn qa2_stored_init_load_caches_kept_and_false_refused() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa2_ilc";
    let db = res!(start(dir, res!(cfg_b()), false, true));
    res!(db.close());
    let path = OzoneConfig::config_path(&res!(Path::new(dir).canonicalize()));
    let before = res!(std::fs::read_to_string(&path));
    if !before.lines().any(|l| l.contains("\"init_load_caches\"") && l.contains("true")) {
        return Err(err!("A new store's config.jdat lacks init_load_caches: true:\n{}", before;
            Test, Missing));
    }
    for _ in 0..2 {
        let db = res!(start(dir, res!(cfg_b()), false, false));
        res!(db.close());
        let after = res!(std::fs::read_to_string(&path));
        if after != before {
            return Err(err!("An unchanged open and close rewrote config.jdat:\nwas\n{}\nnow\n{}",
                before, after; Test, Mismatch));
        }
    }
    let falsed = before.lines()
        .map(|l| if l.contains("\"init_load_caches\"") { l.replace("true", "false") } else { l.to_string() })
        .collect::<Vec<_>>().join("\n") + "\n";
    if falsed == before {
        return Err(err!("The test could not set init_load_caches to false."; Test, Bug));
    }
    res!(std::fs::write(&path, &falsed));
    match start(dir, res!(cfg_b()), false, false) {
        Ok(db) => {
            let _ = db.close();
            return Err(err!("A store whose config.jdat sets init_load_caches to false opened.";
                Test, Mismatch));
        },
        Err(e) => if !fmt!("{}", e).contains("init_load_caches") {
            return Err(err!("The refusal does not name init_load_caches: {}", e; Test, Mismatch));
        },
    }
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
