//! A store's chunk set stays held until its head is in the cache, or for the life of the process
//! once the head is in the files but could not be confirmed (run 2, 2026-10-10).  The hooks are
//! process-wide, so every test takes one lock.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::time::Timestamp;
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
        Cache,
        Prior,
        Superseded,
        ValueOrLocation,
    },
    file::floc::FileLocation,
    sweep,
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
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

fn start(dir: &str, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    if wipe {
        let _ = std::fs::remove_dir_all(dir);
    }
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    setup::start_db(root, Some(res!(cfg())), schemes(), None, false, wipe)
}

fn scan_wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

// "1100/<seed>" for a whole value, "none", "TORN" or "ERR:<text>".
fn describe(r: &Outcome<Option<(Dat, Meta<{ UID_LEN }, Uid>)>>) -> String {
    match r {
        Ok(None) => fmt!("none"),
        Ok(Some((Dat::BU32(b), _))) if !b.is_empty()
            && b.iter().enumerate().all(|(j, x)| *x == b[0] ^ (j as u8)) => fmt!("{}/{}", b.len(), b[0]),
        Ok(Some(_)) => fmt!("TORN"),
        Err(e) => fmt!("ERR:{}", fmt!("{}", e).chars().take(160).collect::<String>()),
    }
}

fn pending_empty(db: &TestDb) -> Outcome<bool> {
    let held = lock_mutex!(db.api().chans().pending_sets());
    Ok(held.is_empty())
}

// A put whose head failed after its record reached the files: the store reports failure, the
// sweep spares the put's chunks, and after a restart the key reads whole.
fn failed_head_keeps_hold(dir: &str, arm: fn(bool)) -> Outcome<Vec<String>> {
    let mut bad = Vec::new();
    let k = dat!("failed head");
    let db = res!(start(dir, true));
    res!(db.insert(k.clone(), val(1_100, 1), Uid::default(), None));
    hooks::set_durability_timeout(Some(Duration::from_secs(2)));
    arm(true);
    let put = db.insert(k.clone(), val(1_100, 2), Uid::default(), None);
    arm(false);
    hooks::set_durability_timeout(None);
    if put.is_ok() { bad.push(fmt!("the put whose head failed was confirmed")); }
    let live = describe(&db.get(&k, None));
    if live != "1100/1" { bad.push(fmt!("live read {}", live)); }
    let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
    if report.orphans_found > 0 { bad.push(fmt!("the sweep took {} chunks of the held set", report.orphans_found)); }
    if report.skipped_recent == 0 { bad.push(fmt!("the sweep saw none of the put's chunks, so it proved nothing")); }
    res!(db.close());
    let db = res!(start(dir, false));
    let after = describe(&db.get(&k, None));
    if !(after == "1100/1" || after == "1100/2") { bad.push(fmt!("restart read {}", after)); }
    res!(db.close());
    let _ = std::fs::remove_dir_all(dir);
    msg!("failed head: put {:?} live {} sweep {}/{} restart {}", put.map(|_| ()).map_err(|e| e.tags().to_vec()),
        live, report.orphans_found, report.skipped_recent, after);
    Ok(bad)
}

// Change 3: the head is in the data file, but the writer cannot hand it on.
#[test]
fn writer_failure_after_append_keeps_hold() -> Outcome<()> {
    let _lock = lock();
    let bad = res!(failed_head_keeps_hold("./test_db_chunk_hold_writer", hooks::set_head_hand_failure));
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// Change 4: the head is durable, but the cache bot cannot enter it.
#[test]
fn cbot_insert_failure_keeps_hold() -> Outcome<()> {
    let _lock = lock();
    let bad = res!(failed_head_keeps_hold("./test_db_chunk_hold_cbot", hooks::set_head_insert_failure));
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// Q1-4: a head that times out at the store keeps its set held until it lands, so a sweep in the
// meantime takes none of its chunks, and the key then reads whole.  The barrier delay is set once
// the chunks are past theirs; a round that catches a chunk's instead is staged again.
#[test]
fn timed_out_head_keeps_its_set_until_it_lands() -> Outcome<()> {
    let _lock = lock();
    let dir = "./test_db_chunk_hold_timeout";
    let db = res!(start(dir, true));
    let mut bad = Vec::new();
    let mut staged = false;
    for attempt in 0..3u8 {
        let k = dat!(fmt!("timed out head {}", attempt));
        res!(db.insert(k.clone(), val(1_100, 1), Uid::default(), None));
        hooks::set_durability_timeout(Some(Duration::from_secs(2)));
        hooks::set_chunk_insert_delay(Duration::from_millis(400));
        let put = { let (db, k) = (db.clone(), k.clone());
            thread::spawn(move || db.insert(k, val(1_100, 2), Uid::default(), None).map(|_| ())) };
        thread::sleep(Duration::from_millis(200));
        hooks::set_barrier_delay(Duration::from_secs(5));
        let put = match put.join() { Ok(r) => r, Err(_) => Err(err!("The put panicked."; Test)) };
        hooks::set_chunk_insert_delay(Duration::ZERO);
        let e = match put {
            Ok(()) => { bad.push(fmt!("attempt {}: the timed-out put was confirmed", attempt)); break; },
            Err(e) => e,
        };
        if !e.tags().contains(&ErrTag::Unconfirmed) {
            // A chunk's barrier was caught, so the head was never written.
            msg!("attempt {}: not staged: {}", attempt, e);
            hooks::set_barrier_delay(Duration::ZERO);
            hooks::set_durability_timeout(None);
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        staged = true;
        let report = res!(sweep::sweep_orphans(db.api(), Uid::default(), None, scan_wait(), Duration::ZERO));
        hooks::set_barrier_delay(Duration::ZERO);
        hooks::set_durability_timeout(None);
        if report.orphans_found > 0 { bad.push(fmt!("the sweep took {} chunks of a head in flight", report.orphans_found)); }
        if report.skipped_recent == 0 { bad.push(fmt!("the sweep saw none of the put's chunks, so it proved nothing")); }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut landed = describe(&db.get(&k, None));
        while landed != "1100/2" && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
            landed = describe(&db.get(&k, None));
        }
        if landed != "1100/2" { bad.push(fmt!("the head did not land whole: {}", landed)); }
        thread::sleep(Duration::from_millis(200));
        if !res!(pending_empty(&db)) { bad.push(fmt!("a set is still held after the head landed")); }
        res!(db.close());
        let db = res!(start(dir, false));
        let after = describe(&db.get(&k, None));
        if after != "1100/2" { bad.push(fmt!("restart read {}", after)); }
        res!(db.close());
        break;
    }
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_durability_timeout(None);
    let _ = std::fs::remove_dir_all(dir);
    assert!(staged, "no attempt staged a head that timed out after its chunks");
    assert!(bad.is_empty(), "{:#?}", bad);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The cache, driven directly.

type C = Cache<{ UID_LEN }, Uid>;

fn meta(s: u64) -> Meta<{ UID_LEN }, Uid> {
    Meta { time: Timestamp::new(s, 0), user: Uid::default() }
}

fn floc(f: u32) -> FileLocation {
    FileLocation { fnum: f, start: 0, klen: 10, vlen: 10 }
}

// Change 8: the write tracker returns to where it began after a head, a small value, a
// tombstone and a head overwrite one key, and a head never enters it.
#[test]
fn write_tracker_follows_head_transitions() -> Outcome<()> {
    let mut cache = C::new(None);
    cache.set_lim(1 << 30);
    let k = b"tracked key".to_vec();
    let t0 = cache.tracker_size();
    let size0 = cache.get_size();
    let steps: [(Option<usize>, &[u8], bool); 6] = [
        (Some(0),   b"head one",    false), // a head enters no tracker entry
        (None,      b"small",       true),  // head to non-head with bytes
        (None,      b"tombstone",   true),  // non-head to non-head
        (Some(0),   b"head two",    false), // non-head to head
        (Some(0),   b"head three",  false), // head to head
        (None,      b"small again", true),
    ];
    for (i, (cind, v, tracked)) in steps.iter().enumerate() {
        let s = i as u64 + 1;
        let sup = res!(cache.insert(k.clone(), Some(v.to_vec()), *cind, floc(s as u32), meta(s)));
        let grown = cache.tracker_size() > t0;
        assert_eq!(grown, *tracked, "step {}: the tracker is {} from {}", i, cache.tracker_size(), t0);
        if i == 4 {
            match sup {
                Superseded::Cached { prior: Prior::Head(b), .. } =>
                    assert_eq!(b, b"head two".to_vec(), "the displaced head's bytes"),
                other => panic!("a head over a head reported {:?}", other),
            }
        }
    }
    assert!(res!(cache.remove(&k, &meta(6))), "the entry was not removed");
    assert_eq!(cache.tracker_size(), t0, "the tracker did not return to its start");
    assert_eq!(cache.get_size(), size0, "the cache size did not return to its start");
    Ok(())
}

// A newer record that arrives with no value, as a replayed one does, takes the old value out with
// the old location: a reader is sent to the new record, never given the old bytes under the new
// record's metadata.  An older record with no value leaves the cache as it was.
#[test]
fn record_without_value_drops_the_old_value() -> Outcome<()> {
    for cind in [None, Some(0)] {
        let mut cache = C::new(None);
        cache.set_lim(1 << 30);
        let k = b"replayed key".to_vec();
        let t0 = cache.tracker_size();
        res!(cache.insert(k.clone(), Some(b"old value".to_vec()), cind, floc(1), meta(2)));
        // An older record, offered late: it loses, and the cached value stands.
        match res!(cache.insert(k.clone(), None, cind, floc(0), meta(1))) {
            Superseded::Offered { .. } => (),
            other => panic!("{:?}: an older record reported {:?}", cind, other),
        }
        match res!(cache.get(&k)) {
            Some(ValueOrLocation::Value(v, m)) => {
                assert_eq!(v, b"old value".to_vec(), "{:?}: the cached value changed", cind);
                assert_eq!(m, meta(2), "{:?}: the cached metadata changed", cind);
            },
            other => panic!("{:?}: after an older record the cache gave {:?}", cind, other),
        }
        // A newer record with no value.
        let sup = res!(cache.insert(k.clone(), None, cind, floc(2), meta(3)));
        match (&sup, cind) {
            (Superseded::Cached { prior: Prior::Head(b), .. }, Some(0)) =>
                assert_eq!(b, &b"old value".to_vec(), "the displaced head's bytes"),
            (Superseded::Cached { prior: Prior::NotHead, .. }, None) => (),
            other => panic!("a newer record reported {:?}", other),
        }
        match res!(cache.get(&k)) {
            Some(ValueOrLocation::Location(mloc)) => {
                assert_eq!(*mloc.file_location(), floc(2), "{:?}: the reader is not sent to the new record", cind);
                assert_eq!(*mloc.meta(), meta(3), "{:?}: the new record's metadata", cind);
            },
            other => panic!("{:?}: a newer record without a value left {:?}", cind, other),
        }
        assert_eq!(cache.tracker_size(), t0, "{:?}: the old value is still tracked", cind);
    }
    Ok(())
}
