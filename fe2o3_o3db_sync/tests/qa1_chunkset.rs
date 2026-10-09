// 2026-10-09 ore-a: describe() now labels the known literals ("small", "small one", "small two") and every deleted-kind Usr value as tomb; TORN only for anything else.
//! QA agent 1 (2026-10-09), chunk sets and fix A: adversarial probes of the write, delete and read
//! paths.  The hooks are process-wide, so every test takes one lock.  Cross-process cases run the
//! phases in child processes of this binary (`qa1_child`, env QA1_MODE / QA1_DIR / QA1_T), so a
//! phase starts with a fresh process stamp, as a real restart does.

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
            UID_LEN,
        },
    },
    verify,
};

use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{
        Arc,
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

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn seed_of(v: &Dat) -> Option<(usize, u8)> {
    match v {
        Dat::BU32(b) | Dat::BU64(b) | Dat::BU8(b) | Dat::BU16(b) if !b.is_empty() => {
            let s = b[0];
            if b.iter().enumerate().all(|(j, x)| *x == s ^ (j as u8)) { Some((b.len(), s)) } else { None }
        },
        _ => None,
    }
}

fn tomb() -> Dat {
    Dat::Usr(oxedyne_fe2o3_o3db_sync::base::id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)))
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
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

fn head(db: &TestDb, k: &Dat) -> Outcome<Option<[u64; 5]>> {
    match res!(db.api().get_head_wait(k, None)) {
        Some((Dat::Tup5u64(t), _)) => Ok(Some(t)),
        _ => Ok(None),
    }
}

// What a read found: "none", "tomb", "L/S" for a whole value of length L and seed S, "TORN", or
// "ERR:<text>".
fn describe(r: &Outcome<Option<(Dat, oxedyne_fe2o3_iop_db::api::Meta<UID_LEN, Uid>)>>) -> String {
    match r {
        Ok(None) => fmt!("none"),
        Ok(Some((Dat::Usr(kind, _), _))) if *kind == oxedyne_fe2o3_o3db_sync::base::id::usr_kind_id_deleted() => fmt!("tomb"),
        Ok(Some((d, _))) if *d == dat!("small") => fmt!("small"),
        Ok(Some((d, _))) if *d == dat!("small one") => fmt!("small one"),
        Ok(Some((d, _))) if *d == dat!("small two") => fmt!("small two"),
        Ok(Some((d, _))) => match seed_of(d) {
            Some((l, s)) => fmt!("{}/{}", l, s),
            None => fmt!("TORN:{:?}", d).chars().take(120).collect(),
        },
        Err(e) => fmt!("ERR:{}", fmt!("{}", e).chars().take(160).collect::<String>()),
    }
}

fn stamp_of(db: &TestDb, k: &Dat) -> String {
    match db.api().get_head_wait(k, None) {
        Ok(Some((_, m))) => fmt!("{:?}", *m.time),
        other => fmt!("{:?}", other.map(|o| o.is_some())),
    }
}

// ---------------------------------------------------------------------------------------------
// Child-process phases.

fn child(mode: &str, dir: &str, t: Option<u128>) -> Outcome<String> {
    let exe = res!(std::env::current_exe());
    let mut cmd = Command::new(exe);
    cmd.args(["qa1_child", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("QA1_MODE", mode)
        .env("QA1_DIR", dir);
    if let Some(t) = t { cmd.env("QA1_T", fmt!("{}", t)); }
    let out = res!(cmd.output());
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    // The harness prints `test qa1_child ... ` with no newline, so the child's first line
    // follows it on the same line.
    let lines: Vec<&str> = so.lines().filter_map(|l| l.find("QA1 ").map(|i| &l[i..])).collect();
    Ok(fmt!("status={:?}\n{}", out.status.code(), lines.join("\n")))
}

#[test]
#[ignore]
fn qa1_child() -> Outcome<()> {
    let mode = match std::env::var("QA1_MODE") { Ok(m) => m, Err(_) => return Ok(()) };
    let dir = res!(std::env::var("QA1_DIR"));
    let t: Option<u128> = std::env::var("QA1_T").ok().and_then(|s| s.parse().ok());
    let dur = |n: u128| Duration::new((n / 1_000_000_000) as u64, (n % 1_000_000_000) as u32);
    match mode.as_str() {
        // F3: a store written while the clock read a minute ahead.
        "f3_ahead" => {
            let db = res!(start(&dir, res!(cfg()), true));
            if let Some(t) = t { hooks::set_fixed_stamp(Some(dur(t))); }
            res!(db.insert(dat!("k1"), val(1_100, 1), Uid::default(), None));
            res!(db.insert(dat!("k2"), dat!("small one"), Uid::default(), None));
            res!(db.insert(dat!("k3"), val(1_100, 3), Uid::default(), None));
            hooks::set_fixed_stamp(None);
            println!("QA1 ahead k1={} stamp={}", describe(&db.get(&dat!("k1"), None)), stamp_of(&db, &dat!("k1")));
            res!(db.close());
        },
        // F3: the next process, with the clock back where it should be.
        "f3_behind" => {
            let db = res!(start(&dir, res!(cfg()), false));
            let p1 = db.insert(dat!("k1"), val(1_500, 2), Uid::default(), None).map(|_| ());
            let p2 = db.insert(dat!("k2"), dat!("small two"), Uid::default(), None).map(|_| ());
            let d3 = db.delete(&dat!("k3"), Uid::default(), None).map(|_| ());
            let p4 = db.insert(dat!("k4"), val(1_100, 4), Uid::default(), None).map(|_| ());
            println!("QA1 calls put_k1={:?} put_k2={:?} del_k3={:?} put_k4={:?}",
                p1.is_ok(), p2.is_ok(), d3.is_ok(), p4.is_ok());
            println!("QA1 live k1={} k2={:?} k3={} k4={} k1stamp={}",
                describe(&db.get(&dat!("k1"), None)),
                db.get(&dat!("k2"), None).map(|o| o.map(|(d, _)| d)).ok(),
                describe(&db.get(&dat!("k3"), None)),
                describe(&db.get(&dat!("k4"), None)),
                stamp_of(&db, &dat!("k1")));
            let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
            println!("QA1 sweep orphans_found={} skipped_recent={}", report.orphans_found, report.skipped_recent);
            res!(db.close());
            let db = res!(start(&dir, res!(cfg()), false));
            println!("QA1 restarted k1={} k2={:?} k3={}",
                describe(&db.get(&dat!("k1"), None)),
                db.get(&dat!("k2"), None).map(|o| o.map(|(d, _)| d)).ok(),
                describe(&db.get(&dat!("k3"), None)));
            res!(db.close());
        },
        // Equal stamps across processes: phase a and b write the same keys in the same order
        // with the clock fixed at T, so each key's two records carry one stamp.
        "tie_a" | "tie_b" => {
            let wipe = mode == "tie_a";
            let seed = if wipe { 1u8 } else { 2u8 };
            let db = res!(start(&dir, res!(cfg()), wipe));
            if let Some(t) = t { hooks::set_fixed_stamp(Some(dur(t))); }
            let mut stamps = Vec::new();
            for i in 0..12u8 {
                let k = dat!(fmt!("tie {}", i));
                let r = db.insert(k.clone(), val(1_100, seed.wrapping_add(i * 16)), Uid::default(), None);
                stamps.push(fmt!("{}:{}", i, if r.is_ok() { stamp_of(&db, &k) } else { fmt!("ERR") }));
            }
            hooks::set_fixed_stamp(None);
            let reads: Vec<String> = (0..12u8).map(|i| describe(&db.get(&dat!(fmt!("tie {}", i)), None))).collect();
            println!("QA1 {} reads {:?}", mode, reads);
            println!("QA1 {} stamps {:?}", mode, stamps);
            res!(db.close());
        },
        "tie_c" => {
            let db = res!(start(&dir, res!(cfg()), false));
            let reads: Vec<String> = (0..12u8).map(|i| describe(&db.get(&dat!(fmt!("tie {}", i)), None))).collect();
            let stamps: Vec<String> = (0..12u8).map(|i| stamp_of(&db, &dat!(fmt!("tie {}", i)))).collect();
            let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
            println!("QA1 tie_c reads {:?}", reads);
            println!("QA1 tie_c stamps {:?}", stamps);
            println!("QA1 tie_c verify clean={} {}", v.clean(), v.summary(4).replace('\n', " | "));
            res!(db.close());
        },
        // A legacy value, then an overwrite of one length killed between its durable head and its
        // retire, or before its head.
        "legacy_after_head" | "legacy_before_head" => {
            let db = res!(start(&dir, res!(cfg()), true));
            let k = dat!("legacy crash");
            let (kbuf, _, _) = res!(db.api().ozone_key_dat(&k, None));
            let legacy = OzoneApi::<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>::legacy_chunk_set_id(&kbuf);
            let resp = db.api().responder();
            res!(db.api().store_dat_using_responder_forcing_set_id(
                k.clone(), val(1_100, 1), Uid::default(), None, resp.clone(), legacy));
            res!(resp.recv_store_ack());
            println!("QA1 legacy head={:?}", res!(head(&db, &k)));
            if mode == "legacy_after_head" {
                hooks::set_retire_delay(Duration::from_secs(20));
            } else {
                hooks::set_chunk_insert_delay(Duration::from_secs(20));
            }
            let (db2, k2) = (db.clone(), k.clone());
            let _put = thread::spawn(move || db2.insert(k2, val(1_100, 2), Uid::default(), None).map(|_| ()));
            thread::sleep(Duration::from_millis(1_500));
            println!("QA1 aborting");
            std::process::abort();
        },
        // Overwritten chunked values, then a collection killed between its new data file and
        // its new index file.
        "collect_crash" => {
            let db = res!(start_collecting(&dir, true));
            for round in 0..6u8 {
                for i in 0..4u8 {
                    res!(db.insert(dat!(fmt!("crash {}", i)), val(1_100, round * 16 + i), Uid::default(), None));
                }
            }
            res!(db.delete(&dat!("crash 3"), Uid::default(), None));
            res!(db.api().settle_for_test(Duration::from_secs(20)));
            hooks::set_commit_delay(Duration::from_secs(30));
            let db2 = db.clone();
            let _gc = thread::spawn(move || db2.api().compact_now(Duration::from_secs(60)).map(|_| ()));
            thread::sleep(Duration::from_millis(2_500));
            println!("QA1 aborting");
            std::process::abort();
        },
        _ => return Err(err!("Unknown QA1_MODE {}.", mode; Test, Invalid)),
    }
    Ok(())
}

fn start_collecting(dir: &str, wipe: bool) -> Outcome<TestDb> {
    let mut c = res!(cfg());
    c.data_file_max_bytes = 4_000;
    c.num_wbots_per_zone  = 1;
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let db = res!(setup::start_db(root, Some(c), schemes(), None, true, wipe));
    res!(db.api().activate_gc(false));
    Ok(db)
}

// Live chunk records, tombstones aside, and the chunks the current heads name.
fn chunk_bound(db: &TestDb, keys: &[Dat]) -> Outcome<(usize, usize)> {
    res!(db.api().settle_for_test(Duration::from_secs(20)));
    let mut live = 0;
    for (_, cache) in res!(db.api().collect_caches(scan_wait())) {
        let tombs = cache.tomb_tracker().tombs();
        for (k, e) in cache.map() {
            if is_chunk_key(k) && matches!(e, CacheEntry::LocatedValue(..)) && !tombs.contains_key(k) {
                live += 1;
            }
        }
    }
    let mut named = 0;
    for k in keys {
        if let Some(t) = res!(head(db, k)) {
            named += t[3] as usize;
        }
    }
    Ok((live, named))
}

// A process killed mid-collection, between its new data file and its new index file: every key
// reads its last value whole after a restart, before and after the sweep, verify is clean, and
// the next overwrites leave no more live chunks than the heads name.
#[test]
fn qa1_kill_mid_collection() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa1_collect_crash";
    let out = res!(child("collect_crash", dir, None));
    let mut bad = Vec::new();
    if !out.contains("QA1 aborting") { bad.push(fmt!("child did not reach the crash: {}", out)); }
    let keys: Vec<Dat> = (0..4).map(|i| dat!(fmt!("crash {}", i))).collect();
    let want = |i: usize| if i == 3 { fmt!("none") } else { fmt!("1100/{}", 5 * 16 + i) };
    let reads = |db: &TestDb| -> Vec<String> { keys.iter()
        .map(|k| match describe(&db.get(k, None)) { d if d == "tomb" => fmt!("none"), d => d }).collect() };
    let wanted: Vec<String> = (0..4).map(want).collect();
    let db = res!(start_collecting(dir, false));
    let got = reads(&db);
    if got != wanted { bad.push(fmt!("restart read {:?} want {:?}", got, wanted)); }
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    if !v.clean() { bad.push(fmt!("verify {}", v.summary(4))); }
    let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
    let swept = reads(&db);
    if swept != wanted { bad.push(fmt!("after sweep read {:?}", swept)); }
    for (i, k) in keys.iter().enumerate() {
        res!(db.insert(k.clone(), val(1_100, 0xC0 + i as u8), Uid::default(), None));
    }
    for _ in 0..3 {
        res!(db.api().settle_for_test(Duration::from_secs(20)));
        res!(db.api().compact_now(Duration::from_secs(60)));
    }
    let (live, named) = res!(chunk_bound(&db, &keys));
    if live != named { bad.push(fmt!("live chunks {} against {} named", live, named)); }
    res!(db.close());
    let db = res!(start_collecting(dir, false));
    let last = reads(&db);
    let lasts: Vec<String> = (0..4).map(|i| fmt!("1100/{}", 0xC0 + i)).collect();
    if last != lasts { bad.push(fmt!("final restart read {:?}", last)); }
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    if !v.clean() { bad.push(fmt!("verify end {}", v.summary(4))); }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    println!("QA1 collect crash sweep_orphans={} {}", report.orphans_found, out.replace('\n', " | "));
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Q: two overlapping puts where the earlier-stamped publishes and reads back first and the later
// one wins: is the middle set left for the sweep?  Counts rounds with orphans after both settle.

#[test]
fn qa1_overlap_orphans() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa1_overlap";
    let db = res!(start(dir, res!(cfg()), true));
    let mut rounds_with_orphans = 0usize;
    let mut orphan_total = 0usize;
    let mut bad = Vec::new();
    let rounds = 10usize;
    for r in 0..rounds {
        let k = dat!(fmt!("overlap {}", r));
        res!(db.insert(k.clone(), val(1_100, 0), Uid::default(), None));
        hooks::set_chunk_insert_delay(Duration::from_millis(40));
        let b = { let (db, k) = (db.clone(), k.clone());
            thread::spawn(move || db.insert(k, val(1_100, 0xB0), Uid::default(), None).map(|_| ()).map_err(|e| fmt!("{}", e))) };
        thread::sleep(Duration::from_millis(15));
        let a = { let (db, k) = (db.clone(), k.clone());
            thread::spawn(move || db.insert(k, val(1_100, 0xA0), Uid::default(), None).map(|_| ()).map_err(|e| fmt!("{}", e))) };
        let rb = match b.join() { Ok(x) => x, Err(_) => Err(fmt!("panicked")) };
        let ra = match a.join() { Ok(x) => x, Err(_) => Err(fmt!("panicked")) };
        hooks::set_chunk_insert_delay(Duration::ZERO);
        if rb.is_err() || ra.is_err() { bad.push(fmt!("round {}: puts {:?} {:?}", r, rb, ra)); }
        let got = describe(&db.get(&k, None));
        if !(got == "1100/160" || got == "1100/176") { bad.push(fmt!("round {}: read {}", r, got)); }
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        if report.orphans_found > 0 { rounds_with_orphans += 1; orphan_total += report.orphans_found; }
        let after = describe(&db.get(&k, None));
        if after != got { bad.push(fmt!("round {}: after sweep read {} (was {})", r, after, got)); }
        println!("QA1 overlap round {} final={} orphans_found={}", r, got, report.orphans_found);
    }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    println!("QA1 overlap rounds_with_orphans={}/{} orphan_chunks={}", rounds_with_orphans, rounds, orphan_total);
    assert!(bad.is_empty(), "{:?}", bad);
    assert_eq!(rounds_with_orphans, 0, "overlapping puts left chunk sets for the sweep: {} chunks over {} rounds",
        orphan_total, rounds_with_orphans);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Q: concurrent puts and deletes (both roads) on one key, with readers: every read whole-or-none,
// the final value whole, the same across a restart, verify clean, and the sweep takes nothing live.

fn puts_deletes(dir: &str, cache_lim: Option<usize>, iters: usize) -> Outcome<(Vec<String>, usize)> {
    let mut c = res!(cfg());
    if let Some(l) = cache_lim { c.cache_size_limit_bytes = l as u64; }
    let mut bad = Vec::new();
    let mut orphans = 0usize;
    let lens = [1_100usize, 1_500, 2_300];
    for it in 0..iters {
        let db = res!(start(dir, c.clone(), true));
        let k = dat!("contended");
        hooks::set_read_delay(Duration::from_millis(2));
        hooks::set_chunk_insert_delay(Duration::from_millis(2));
        let stop = Arc::new(AtomicBool::new(false));
        let torn = Arc::new(Mutex::new(Vec::<String>::new()));
        let nreads = Arc::new(AtomicUsize::new(0));
        let readers: Vec<_> = (0..3).map(|_| {
            let (db, k, stop, torn, nreads) = (db.clone(), k.clone(), stop.clone(), torn.clone(), nreads.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let d = describe(&db.get(&k, None));
                    nreads.fetch_add(1, Ordering::Relaxed);
                    if d.starts_with("TORN") || d.starts_with("ERR") {
                        if let Ok(mut t) = torn.lock() { t.push(d); }
                    }
                }
            })
        }).collect();
        let writers: Vec<_> = (0..6usize).map(|t| {
            let (db, k) = (db.clone(), k.clone());
            thread::spawn(move || -> Vec<String> {
                let mut errs = Vec::new();
                for n in 0..15usize {
                    let r = match (t + n) % 5 {
                        0 => db.delete(&k, Uid::default(), None).map(|_| ()),
                        1 => db.api().store(k.clone(), tomb(), Uid::default())
                                .and_then(|resp| resp.recv_store_ack().map(|_| ())),
                        2 => db.insert(k.clone(), dat!("small"), Uid::default(), None).map(|_| ()),
                        _ => db.insert(k.clone(), val(lens[(t + n) % 3], (t as u8) * 16 + n as u8), Uid::default(), None).map(|_| ()),
                    };
                    if let Err(e) = r { errs.push(fmt!("{}", e).chars().take(200).collect()); }
                }
                errs
            })
        }).collect();
        for w in writers {
            match w.join() { Ok(e) => for x in e { bad.push(fmt!("it {} write: {}", it, x)); }, Err(_) => bad.push(fmt!("panic")) }
        }
        stop.store(true, Ordering::Relaxed);
        for r in readers { let _ = r.join(); }
        hooks::set_read_delay(Duration::ZERO);
        hooks::set_chunk_insert_delay(Duration::ZERO);
        if let Ok(t) = torn.lock() { for x in t.iter().take(5) { bad.push(fmt!("it {} mid-read: {}", it, x)); } }
        let live = describe(&db.get(&k, None));
        if live.starts_with("TORN") || live.starts_with("ERR") { bad.push(fmt!("it {} live final {}", it, live)); }
        if let Some(t) = res!(head(&db, &k)) {
            if let Err(e) = db.api().fetch_chunks(&Dat::Tup5u64(t), None) {
                bad.push(fmt!("it {} live head {:?} missing chunks: {}", it, t, fmt!("{}", e).chars().take(160).collect::<String>()));
            }
        }
        let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
        if !v.clean() { bad.push(fmt!("it {} verify live: {}", it, v.summary(4))); }
        res!(db.close());
        let db = res!(start(dir, c.clone(), false));
        let rest = describe(&db.get(&k, None));
        if rest != live { bad.push(fmt!("it {} restart read {} but live read {}", it, rest, live)); }
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        orphans += report.orphans_found;
        let swept = describe(&db.get(&k, None));
        if swept != live { bad.push(fmt!("it {} after sweep read {} but live read {}", it, swept, live)); }
        let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
        if !v.clean() { bad.push(fmt!("it {} verify swept: {}", it, v.summary(4))); }
        res!(db.close());
        println!("QA1 contended it {} final={} reads={} orphans={}", it, live, nreads.load(Ordering::Relaxed), report.orphans_found);
    }
    let _ = std::fs::remove_dir_all(dir);
    Ok((bad, orphans))
}

#[test]
fn qa1_puts_deletes_one_key_default_cache() -> Outcome<()> {
    let _lock = lock();
    let (bad, orphans) = res!(puts_deletes("./test_db_qa1_pd_def", None, 4));
    println!("QA1 contended default orphans_total={}", orphans);
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

#[test]
fn qa1_puts_deletes_one_key_tiny_cache() -> Outcome<()> {
    let _lock = lock();
    let (bad, orphans) = res!(puts_deletes("./test_db_qa1_pd_tiny", Some(1), 4));
    println!("QA1 contended tiny orphans_total={}", orphans);
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// F3 characterised: a process stamps a minute ahead, the next process stamps by the true clock.

#[test]
fn qa1_f3_clock_back_across_restart() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa1_f3";
    let _ = std::fs::remove_dir_all(dir);
    let ahead = (*res!(Timestamp::now()) + Duration::from_secs(60)).as_nanos();
    let a = res!(child("f3_ahead", dir, Some(ahead)));
    let b = res!(child("f3_behind", dir, None));
    let _ = std::fs::remove_dir_all(dir);
    println!("{}\n{}", a, b);
    assert!(b.contains("live k1=1500/2"), "a put after a clock step back is lost although it returned Ok:\n{}", b);
    assert!(b.contains("k3=none") || b.contains("k3=tomb"), "a delete after a clock step back is lost:\n{}", b);
    Ok(())
}

// Equal stamps across processes: does a restart keep the value the cache kept, whole?
#[test]
fn qa1_equal_stamp_across_processes() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa1_tie";
    let mut outs = Vec::new();
    let mut bad = Vec::new();
    for round in 0..4 {
        let _ = std::fs::remove_dir_all(dir);
        let t = (*res!(Timestamp::now())).as_nanos();
        let a = res!(child("tie_a", dir, Some(t)));
        let b = res!(child("tie_b", dir, Some(t)));
        let c = res!(child("tie_c", dir, None));
        let reads = |s: &str, tag: &str| s.lines().find(|l| l.starts_with(&fmt!("QA1 {} reads", tag))).unwrap_or("").to_string();
        let (rb, rc) = (reads(&b, "tie_b"), reads(&c, "tie_c"));
        let rb = rb.replace("tie_b", "");
        let rc = rc.replace("tie_c", "");
        if rb != rc || c.contains("TORN") || c.contains("ERR") || c.contains("clean=false") {
            bad.push(fmt!("round {}: live {} vs restart {}\n{}\n{}", round, rb, rc, b, c));
        }
        outs.push(fmt!("round {}\n{}\n{}\n{}", round, a, b, c));
    }
    let _ = std::fs::remove_dir_all(dir);
    println!("{}", outs.join("\n"));
    assert!(bad.is_empty(), "equal stamps across processes diverge at restart:\n{}", bad.join("\n"));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Legacy (key-derived) set: an overwrite of one length killed after its durable head and before
// its retire, or before its head.  After restart: whole, verify clean, sweep spares the live set,
// and overwrite, delete and restart behave.

fn legacy_crash(mode: &str, dir: &str) -> Outcome<Vec<String>> {
    let out = res!(child(mode, dir, None));
    let mut bad = Vec::new();
    if !out.contains("QA1 aborting") { bad.push(fmt!("child did not reach the crash: {}", out)); }
    let k = dat!("legacy crash");
    let db = res!(start(dir, res!(cfg()), false));
    let got = describe(&db.get(&k, None));
    let want = if mode == "legacy_after_head" { "1100/2" } else { "1100/1" };
    if got != want { bad.push(fmt!("{}: restart read {} want {}", mode, got, want)); }
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    if !v.clean() { bad.push(fmt!("{}: verify {}", mode, v.summary(4))); }
    let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
    let swept = describe(&db.get(&k, None));
    if swept != want { bad.push(fmt!("{}: after sweep read {}", mode, swept)); }
    res!(db.insert(k.clone(), val(1_100, 3), Uid::default(), None));
    let over = describe(&db.get(&k, None));
    if over != "1100/3" { bad.push(fmt!("{}: overwrite read {}", mode, over)); }
    res!(db.close());
    let db = res!(start(dir, res!(cfg()), false));
    let over2 = describe(&db.get(&k, None));
    if over2 != "1100/3" { bad.push(fmt!("{}: overwrite after restart read {}", mode, over2)); }
    res!(db.delete(&k, Uid::default(), None));
    let del = describe(&db.get(&k, None));
    if !(del == "none" || del == "tomb") { bad.push(fmt!("{}: delete read {}", mode, del)); }
    res!(db.close());
    let db = res!(start(dir, res!(cfg()), false));
    let del2 = describe(&db.get(&k, None));
    if !(del2 == "none" || del2 == "tomb") { bad.push(fmt!("{}: delete after restart read {}", mode, del2)); }
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    if !v.clean() { bad.push(fmt!("{}: verify end {}", mode, v.summary(4))); }
    res!(db.close());
    println!("QA1 {} restart={} sweep_orphans={} {}", mode, got, report.orphans_found, out.replace('\n', " | "));
    let _ = std::fs::remove_dir_all(dir);
    Ok(bad)
}

#[test]
fn qa1_legacy_crash_mid_overwrite() -> Outcome<()> {
    let _lock = lock();
    let mut bad = res!(legacy_crash("legacy_after_head", "./test_db_qa1_legacy_a"));
    bad.extend(res!(legacy_crash("legacy_before_head", "./test_db_qa1_legacy_b")));
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// An Unconfirmed bunch key that lands: the head's barrier fails after its chunks were confirmed.
// The store reports failure; the value is current and must read whole, survive the sweep and a
// restart.

#[test]
fn qa1_unconfirmed_head_that_lands() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_qa1_unconf";
    let db = res!(start(dir, res!(cfg()), true));
    let k = dat!("unconfirmed head");
    res!(db.insert(k.clone(), val(1_100, 1), Uid::default(), None));
    hooks::set_chunk_insert_delay(Duration::from_millis(400));
    let put = { let (db, k) = (db.clone(), k.clone());
        thread::spawn(move || db.insert(k, val(1_100, 2), Uid::default(), None).map(|_| ()).map_err(|e| fmt!("{}", e))) };
    thread::sleep(Duration::from_millis(200));
    hooks::set_barrier_failure(true);
    let put = match put.join() { Ok(x) => x, Err(_) => Err(fmt!("panicked")) };
    hooks::set_barrier_failure(false);
    hooks::set_chunk_insert_delay(Duration::ZERO);
    let live = describe(&db.get(&k, None));
    let report = sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO);
    let swept = describe(&db.get(&k, None));
    let _ = db.close();
    let db = res!(start(dir, res!(cfg()), false));
    let rest = describe(&db.get(&k, None));
    let v = res!(verify::verify_live_set(db.api(), None, scan_wait()));
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    println!("QA1 unconfirmed put={:?} live={} sweep={:?} swept={} restart={} verify={}",
        put.as_ref().map_err(|e| e.chars().take(200).collect::<String>()), live,
        report.as_ref().map(|r| r.orphans_found), swept, rest, v.clean());
    assert!(live == "1100/1" || live == "1100/2", "live read {}", live);
    assert_eq!(swept, live, "the sweep changed the read");
    assert!(rest == "1100/1" || rest == "1100/2", "restart read {}", rest);
    assert!(v.clean(), "verify after restart: {}", v.summary(4));
    Ok(())
}
