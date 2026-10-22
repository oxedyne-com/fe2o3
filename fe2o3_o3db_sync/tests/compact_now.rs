//! `compact_now` erases (A3 R3, 2026-10-08): once it returns `Ok`, no data file holds a record that
//! a write acknowledged before the call superseded or deleted.  Every test counts the bytes of the
//! files from outside the store, with a needle that is a whole record's value, and finds it first
//! (the positive control) so that a needle that is never there cannot pass.  The hooks used are
//! process-wide, so every test takes one lock and puts each hook back.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
};
use oxedyne_fe2o3_o3db_sync::{
    api::CompactReport,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    file::core::FileType,
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
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{
            AtomicBool,
            AtomicU64,
            Ordering,
        },
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

const GC_ON:    bool = true;
const GC_OFF:   bool = false;

// A store of the plain kind: records of one length, five to a file.
struct Plain {
    dir:    String,
    root:   PathBuf,
    cfg:    OzoneConfig,
    probe:  Probe,
    db:     TestDb,
}

fn plain(name: &str, gc_on: bool, cbots: u16, fbots: u16) -> Outcome<Plain> {
    log_set_level!("error");
    let dir = fmt!("./test_db_compact_{}", name);
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    let root = res!(Path::new(&dir).canonicalize());
    let probe = res!(probe(&fmt!("./test_db_compact_{}_probe", name)));
    let mut cfg = res!(config(5 * probe.len + probe.len / 2));
    cfg.num_cbots_per_zone = cbots;
    cfg.num_fbots_per_zone = fbots;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, gc_on, true));
    Ok(Plain { dir, root, cfg, probe, db })
}

impl Plain {
    // How many data files hold the whole value of key `i` at version `ver`.
    fn holds(&self, i: usize, ver: u8) -> usize {
        let needle = match value(i, ver).bytes_ref() {
            Some(b) => b.clone(),
            None => Vec::new(),
        };
        assert!(!needle.is_empty(), "the needle of key {} version {} is empty", i, ver);
        files_holding(&self.root, &self.cfg, &needle).len()
    }

    fn end(self) -> Outcome<()> {
        res!(self.db.close());
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(())
    }
}

/// Reads every key and counts the answers that are not what was last written: a version, or `None`
/// for a key that was deleted.
fn reads(db: &TestDb, want: &[Option<u8>], label: &str) -> usize {
    let mut bad = 0;
    for (i, w) in want.iter().enumerate() {
        match (db.get(&key(i), None), w) {
            (Ok(Some((v, _))), Some(ver)) if v == value(i, *ver)    => (),
            (Ok(None), None)                                        => (),
            (got, _) => {
                bad += 1;
                let s: String = fmt!("{:?}", got.map(|o| o.map(|(d, _)| d))).chars().take(200).collect();
                msg!("{}: key {} wanted {:?} and read {}", label, i, w, s);
            },
        }
    }
    bad
}

fn small(label: &str, found: usize) -> String {
    fmt!("{}: {} data files still hold it", label, found)
}

#[test]
fn deleted_value_leaves_every_dat() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("delete", GC_ON, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    // Key 2 is in sealed file 1, key 11 in the live file.
    for i in [2usize, 11] {
        assert!(s.holds(i, 1) >= 1, "positive control: key {} version 1 is in a data file", i);
        assert!(res!(s.db.delete(&key(i), user, None)), "the delete of key {} found nothing", i);
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(500));
    // One old record is under the trigger, and the live file is never collected, so the delete
    // alone has erased nothing.
    for i in [2usize, 11] {
        assert!(s.holds(i, 1) >= 1, "positive control: the delete of key {} did not erase it by itself", i);
    }

    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    for i in [2usize, 11] {
        assert_eq!(s.holds(i, 1), 0, "{}", small(&fmt!("key {} version 1 after compact_now", i), s.holds(i, 1)));
    }
    assert!(report.bytes_after < report.bytes_before, "compact_now removed nothing: {:?}", report);
    let mut want: Vec<Option<u8>> = vec![Some(1); NKEYS];
    want[2] = None;
    want[11] = None;
    assert_eq!(reads(&s.db, &want, "After compact_now"), 0);

    // Reopened, so that what the files hold is what is read.
    let (root, cfg) = (s.root.clone(), s.cfg.clone());
    res!(s.db.close());
    let db: TestDb = res!(setup::start_db(root, Some(cfg), schemes(), None, GC_ON, false));
    assert_eq!(reads(&db, &want, "After reopening"), 0);
    res!(db.close());
    let _ = std::fs::remove_dir_all(&s.dir);
    Ok(())
}

#[test]
fn overwritten_value_leaves_every_dat() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("overwrite", GC_ON, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    // One record in each sealed file, each under the trigger.
    let mut want: Vec<Option<u8>> = vec![Some(1); NKEYS];
    for i in [3usize, 7] {
        assert!(s.holds(i, 1) >= 1, "positive control: key {} version 1 is in a data file", i);
        res!(s.db.insert(key(i), value(i, 2), user, None));
        want[i] = Some(2);
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(500));
    for i in [3usize, 7] {
        assert!(s.holds(i, 1) >= 1, "positive control: the overwrite of key {} did not erase it by itself", i);
    }

    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    for i in [3usize, 7] {
        assert_eq!(s.holds(i, 1), 0, "{}", small(&fmt!("key {} version 1 after compact_now", i), s.holds(i, 1)));
        assert!(s.holds(i, 2) >= 1, "the new value of key {} is in no data file", i);
    }
    assert_eq!(reads(&s.db, &want, "After compact_now"), 0);
    s.end()
}

#[test]
fn compact_collects_with_gc_switched_off() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("gc_off", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    assert!(s.holds(3, 1) >= 1, "positive control: key 3 version 1 is in a data file");
    res!(s.db.insert(key(3), value(3, 2), user, None));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    // The switch is off and the trigger is not met, so nothing but the order can collect.
    thread::sleep(Duration::from_millis(1_000));
    assert!(s.holds(3, 1) >= 1, "positive control: nothing collected with the collector off");

    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    assert_eq!(s.holds(3, 1), 0, "{}", small("key 3 version 1 after compact_now", s.holds(3, 1)));
    assert!(report.files_collected + report.files_deleted >= 1, "nothing was collected: {:?}", report);
    let mut want: Vec<Option<u8>> = vec![Some(1); NKEYS];
    want[3] = Some(2);
    assert_eq!(reads(&s.db, &want, "After compact_now"), 0);
    s.end()
}

// The chunked store: three chunks of 400 bytes in a value of 1,100.
const CHUNKS: u64 = 3;

fn chunk_cfg() -> Outcome<OzoneConfig> {
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

fn big(seed: u8) -> Vec<u8> {
    (0..1_100usize).map(|j| seed ^ (j as u8)).collect()
}

// A hundred bytes from the middle of each chunk, so that a chunk is found by its own bytes.
fn windows(seed: u8) -> Vec<Vec<u8>> {
    let v = big(seed);
    vec![v[100..200].to_vec(), v[500..600].to_vec(), v[900..1_000].to_vec()]
}

struct Chunked {
    dir:    String,
    root:   PathBuf,
    cfg:    OzoneConfig,
    db:     TestDb,
}

fn chunked(name: &str, gc_on: bool) -> Outcome<Chunked> {
    log_set_level!("error");
    let dir = fmt!("./test_db_compact_{}", name);
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    let root = res!(Path::new(&dir).canonicalize());
    let cfg = res!(chunk_cfg());
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, gc_on, true));
    Ok(Chunked { dir, root, cfg, db })
}

impl Chunked {
    fn holds(&self, seed: u8) -> Vec<usize> {
        windows(seed).iter().map(|w| files_holding(&self.root, &self.cfg, w).len()).collect()
    }

    fn num_chunks(&self, k: &Dat) -> Outcome<u64> {
        let api = self.db.api();
        let resp = res!(api.fetch_using_schemes(k, None));
        match res!(resp.recv_daticle(api.schemes().encrypter(), None)) {
            (Some((Dat::Tup5u64(tup), _)), _) => Ok(PartKey(tup).num_parts()),
            other => Err(err!("Key {:?} holds {:?}, not a bunch key.", k, other; Test, Invalid)),
        }
    }
}

#[test]
fn chunked_delete_leaves_every_dat() -> Outcome<()> {
    let _lock = lock();
    let s = res!(chunked("chunked", GC_OFF));
    let user = Uid::default();
    // A value that stays, in the same file, so that the file is collected and not merely removed.
    res!(s.db.insert(key(0), value(0, 9), user, None));
    let k = dat!("chunked erase");
    res!(s.db.insert(k.clone(), Dat::BU32(big(5)), user, None));
    assert_eq!(res!(s.num_chunks(&k)), CHUNKS, "the value did not split into {} chunks", CHUNKS);
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    assert!(s.holds(5).iter().all(|n| *n >= 1), "positive control: every chunk is in a data file: {:?}", s.holds(5));

    hooks::set_chunk_tombstone_delay(Duration::from_millis(300));
    let existed = s.db.delete(&k, user, None);
    hooks::set_chunk_tombstone_delay(Duration::ZERO);
    assert!(res!(existed), "the delete found nothing");
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    assert!(s.holds(5).iter().all(|n| *n >= 1), "positive control: the delete did not erase the chunks by itself");

    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    assert_eq!(s.holds(5), vec![0, 0, 0], "chunks still in data files after compact_now");
    match res!(s.db.get(&key(0), None)) {
        Some((v, _)) => assert_eq!(v, value(0, 9), "the value beside it changed"),
        None => return Err(err!("The value that was not deleted is gone."; Test, Missing)),
    }
    assert!(res!(s.db.get(&k, None)).is_none(), "the deleted value reads back");
    res!(s.db.close());
    let _ = std::fs::remove_dir_all(&s.dir);
    Ok(())
}

#[test]
fn tombstone_store_leaves_every_dat() -> Outcome<()> {
    let _lock = lock();
    let s = res!(chunked("tombstone", GC_OFF));
    let user = Uid::default();
    res!(s.db.insert(key(0), value(0, 9), user, None));
    let k = dat!("tombstone erase");
    res!(s.db.insert(k.clone(), Dat::BU32(big(6)), user, None));
    assert_eq!(res!(s.num_chunks(&k)), CHUNKS, "the value did not split into {} chunks", CHUNKS);
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    assert!(s.holds(6).iter().all(|n| *n >= 1), "positive control: every chunk is in a data file: {:?}", s.holds(6));

    // The erase of the distributed adapters: a store of the deleted marker, not a delete.
    let tomb = Dat::Usr(oxedyne_fe2o3_o3db_sync::base::id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
    let resp = res!(s.db.api().store(k.clone(), tomb.clone(), user));
    res!(resp.recv_store_ack());
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    assert!(s.holds(6).iter().any(|n| *n >= 1), "positive control: the store alone left no chunk to erase");

    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    assert_eq!(s.holds(6), vec![0, 0, 0], "chunks still in data files after compact_now");
    match res!(s.db.api().get_wait(&k, None)) {
        Some((d, _)) => assert_eq!(d, tomb, "the key does not read back as the deleted marker"),
        None => (),
    }
    match res!(s.db.get(&key(0), None)) {
        Some((v, _)) => assert_eq!(v, value(0, 9), "the value beside it changed"),
        None => return Err(err!("The value that was not deleted is gone."; Test, Missing)),
    }
    res!(s.db.close());
    let _ = std::fs::remove_dir_all(&s.dir);
    Ok(())
}

#[test]
fn compact_waits_out_the_barrier() -> Outcome<()> {
    let _lock = lock();
    const N: usize = 20;
    let s = res!(plain("barrier", GC_OFF, 2, 2));
    let user = Uid::default();
    for i in 0..(N + 2) {
        res!(s.db.insert(key(i), value(i, 1), user, None));
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    for i in 0..N {
        assert!(s.holds(i, 1) >= 1, "positive control: key {} version 1 is in a data file", i);
    }

    // Each supersession a file bot passes to another is held, so that the old records are not yet
    // counted old when the deletes have all been acknowledged.
    hooks::set_forward_delay(Duration::from_millis(200));
    for i in 0..N {
        assert!(res!(s.db.delete(&key(i), user, None)), "the delete of key {} found nothing", i);
    }
    let report = s.db.compact_now(Duration::from_secs(60));
    hooks::set_forward_delay(Duration::ZERO);
    let report = res!(report);
    msg!("{:?}", report);
    let mut left = Vec::new();
    for i in 0..N {
        if s.holds(i, 1) > 0 {
            left.push(i);
        }
    }
    assert!(left.is_empty(), "keys {:?} are still in data files after compact_now", left);
    let mut want: Vec<Option<u8>> = vec![None; N];
    want.push(Some(1));
    want.push(Some(1));
    assert_eq!(reads(&s.db, &want, "After compact_now"), 0);
    s.end()
}

#[test]
fn compact_under_reads() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("reads", GC_ON, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    let issued: Arc<Vec<AtomicU64>> = Arc::new((0..NKEYS).map(|_| AtomicU64::new(1)).collect());
    let acked:  Arc<Vec<AtomicU64>> = Arc::new((0..NKEYS).map(|_| AtomicU64::new(1)).collect());
    let stop    = Arc::new(AtomicBool::new(false));
    let counts: Arc<[AtomicU64; 5]> = Arc::new([
        AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    ]);
    const READS: usize = 0; const WRONG: usize = 1; const STALE: usize = 2; const PHANTOM: usize = 3; const ERRS: usize = 4;
    let mut hs = Vec::new();
    for r in 0..4usize {
        let (db, issued, acked, stop, counts) =
            (s.db.clone(), issued.clone(), acked.clone(), stop.clone(), counts.clone());
        hs.push(res!(thread::Builder::new().name(fmt!("compact reader {}", r)).spawn(move || {
            let mut n = r;
            while !stop.load(Ordering::Relaxed) {
                n = (n * 7 + 3 + r) % NKEYS;
                let lo = acked[n].load(Ordering::Relaxed);
                let got = db.get(&key(n), None);
                let hi = issued[n].load(Ordering::Relaxed);
                counts[READS].fetch_add(1, Ordering::Relaxed);
                match got {
                    Err(e) => {
                        if counts[ERRS].fetch_add(1, Ordering::Relaxed) < 3 {
                            msg!("Read error on key {}: {}", n, e);
                        }
                    },
                    Ok(None) => { counts[WRONG].fetch_add(1, Ordering::Relaxed); },
                    Ok(Some((d, _))) => match d.bytes_ref() {
                        Some(b) if b.len() >= 2 && b[0] as usize == n => {
                            let ver = b[1] as u64;
                            if d != value(n, b[1]) || ver > hi { counts[PHANTOM].fetch_add(1, Ordering::Relaxed); }
                            else if ver < lo { counts[STALE].fetch_add(1, Ordering::Relaxed); }
                        },
                        _ => { counts[WRONG].fetch_add(1, Ordering::Relaxed); },
                    },
                }
            }
        })));
    }
    // A thread to keep the cached values away, so that reads go to the files and pin them.
    {
        let (db, stop) = (s.db.clone(), stop.clone());
        hs.push(res!(thread::Builder::new().name(fmt!("compact clearer")).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = db.api().clear_cache_values(Wait {
                    max_wait:       Duration::from_secs(5),
                    check_interval: Duration::from_millis(10),
                });
                thread::sleep(Duration::from_millis(15));
            }
        })));
    }

    let mut failure = None;
    for round in 2..12u8 {
        for i in 0..NKEYS {
            issued[i].store(round as u64, Ordering::Relaxed);
            if let Err(e) = s.db.insert(key(i), value(i, round), user, None) {
                failure = Some(e);
                break;
            }
            acked[i].store(round as u64, Ordering::Relaxed);
        }
        if failure.is_some() { break; }
        if let Err(e) = s.db.compact_now(Duration::from_secs(60)) {
            failure = Some(e);
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    for h in hs {
        let _ = h.join();
    }
    if let Some(e) = failure {
        return Err(e);
    }
    let c = |i: usize| counts[i].load(Ordering::Relaxed);
    msg!("{} reads: {} wrong, {} stale, {} phantom, {} errors.", c(READS), c(WRONG), c(STALE), c(PHANTOM), c(ERRS));
    assert!(c(READS) > 100, "only {} reads ran against the compactions", c(READS));
    assert_eq!(c(WRONG), 0, "reads returned another key's record, or none");
    assert_eq!(c(STALE), 0, "reads returned a version older than one acknowledged before the read");
    assert_eq!(c(PHANTOM), 0, "reads returned a version never written");
    assert_eq!(c(ERRS), 0, "reads failed while compact_now ran");
    let want: Vec<Option<u8>> = vec![Some(11); NKEYS];
    assert_eq!(reads(&s.db, &want, "After the last compact_now"), 0);
    for i in 0..NKEYS {
        for ver in 1..11u8 {
            assert_eq!(s.holds(i, ver), 0, "key {} version {} is in a data file after the last compact_now", i, ver);
        }
    }
    s.end()
}

#[test]
fn compact_names_what_it_waits_on() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("names", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    // Reopened, so that the cache holds locations and no values, and a read of a record in file 1
    // goes to its file bot, which pins the file for the reader.
    let (root, cfg, dir) = (s.root.clone(), s.cfg.clone(), s.dir.clone());
    res!(s.db.close());
    let db: TestDb = res!(setup::start_db(root, Some(cfg), schemes(), None, GC_OFF, false));
    res!(db.insert(key(0), value(0, 2), user, None));
    res!(db.api().settle_for_test(Duration::from_secs(10)));

    hooks::set_read_delay(Duration::from_secs(4));
    let reader = {
        let db = db.clone();
        thread::spawn(move || db.get(&key(2), None))
    };
    thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    let result = db.compact_now(Duration::from_secs(1));
    let took = t.elapsed();
    hooks::set_read_delay(Duration::ZERO);
    let e = match result {
        Err(e) => e,
        Ok(r) => return Err(err!("compact_now returned Ok({:?}) with a reader pinning file 1.", r; Test, Invalid)),
    };
    let text = fmt!("{}", e);
    msg!("compact_now failed after {:?} with: {}", took, text.chars().take(400).collect::<String>());
    assert!(e.tags().contains(&ErrTag::Timeout), "the error is not a timeout: {:?}", e.tags());
    assert!(text.contains("file 1 of zone 1 (readers)"), "the error does not name file 1 and its readers: {}",
        text.chars().take(400).collect::<String>());
    assert!(took >= Duration::from_secs(1) && took < Duration::from_secs(3), "the deadline was kept to {:?}", took);

    // Once the reader is done the same call finishes.
    match reader.join() {
        Ok(got) => match res!(got) {
            Some((v, _)) => assert_eq!(v, value(2, 1), "the pinned reader read the wrong value"),
            None => return Err(err!("The pinned reader found nothing."; Test, Missing)),
        },
        Err(_) => return Err(err!("The reader thread panicked."; Test, Bug)),
    }
    let report = res!(db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    res!(db.close());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn compact_is_idempotent_and_bounded() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("idem", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    for i in 0..NKEYS {
        res!(s.db.insert(key(i), value(i, 2), user, None));
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    let files = |s: &Plain| -> usize {
        match std::fs::read_dir(zone_dir(&s.root, &s.cfg)) {
            Ok(list) => list.flatten().filter(|e| {
                e.path().extension().map_or(false, |x| x == constant::DATA_FILE_EXT)
            }).count(),
            Err(_) => 0,
        }
    };
    let mut reports: Vec<CompactReport> = Vec::new();
    let mut counts = Vec::new();
    for _ in 0..5 {
        reports.push(res!(s.db.compact_now(Duration::from_secs(30))));
        counts.push(files(&s));
    }
    msg!("Reports {:?}, data files {:?}.", reports, counts);
    assert!(counts[4] <= counts[0] + 1, "data files grew from {} to {} over five calls", counts[0], counts[4]);
    for w in reports.windows(2) {
        assert!(w[1].bytes_after <= w[0].bytes_after, "size_bytes rose between calls: {:?}", reports);
    }
    assert_eq!(reports[4].bytes_before, reports[4].bytes_after, "a fifth call on a quiet store still removed bytes");
    for i in 0..NKEYS {
        assert_eq!(s.holds(i, 1), 0, "key {} version 1 is in a data file", i);
    }
    let want: Vec<Option<u8>> = vec![Some(2); NKEYS];
    assert_eq!(reads(&s.db, &want, "After five calls"), 0);
    s.end()
}

#[test]
fn size_falls_by_what_compact_removed() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("size", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    for i in 0..NKEYS {
        res!(s.db.insert(key(i), value(i, 2), user, None));
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    let before = settled_len(&s.root, &s.cfg);
    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    let after = settled_len(&s.root, &s.cfg);
    msg!("{:?}; the files held {} bytes before and {} after.", report, before, after);
    assert!(before > after, "positive control: compact_now removed nothing from {} bytes", before);
    assert_eq!(report.bytes_before, before, "bytes_before is not the length of the files before the call");
    assert_eq!(report.bytes_after, after, "bytes_after is not the length of the files after the call");
    assert_eq!(report.bytes_before - report.bytes_after, before - after, "the report does not give the drop");
    s.end()
}

#[test]
fn zone_bot_error_ends_compact_now_at_once() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("error", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.insert(key(0), value(0, 2), user, None));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    hooks::set_compact_failure(true);
    let t = Instant::now();
    let result = s.db.compact_now(Duration::from_secs(30));
    let took = t.elapsed();
    hooks::set_compact_failure(false);
    let e = match result {
        Err(e) => e,
        Ok(r) => return Err(err!("compact_now returned Ok({:?}) though no zone bot passed the order on.", r; Test, Invalid)),
    };
    let text = fmt!("{}", e);
    msg!("compact_now failed after {:?} with: {}", took, text.chars().take(400).collect::<String>());
    assert!(took < Duration::from_secs(10), "the error took {:?}, so the call waited towards its deadline", took);
    assert!(!e.tags().contains(&ErrTag::Timeout), "the error is the deadline's: {:?}", e.tags());
    assert!(text.contains("cannot be passed to the file bots"), "the error lost the zone bot's words: {}",
        text.chars().take(400).collect::<String>());

    // And the store still compacts once the zone bots can pass the order on.
    let report = res!(s.db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    s.end()
}

/// The files this process holds open under the root, as the kernel names them: `(deleted)` follows
/// the path of one whose name has been unlinked.
fn open_files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(list) = std::fs::read_dir("/proc/self/fd") {
        for entry in list.flatten() {
            if let Ok(target) = std::fs::read_link(entry.path()) {
                let t = target.to_string_lossy().to_string();
                if t.starts_with(&root.to_string_lossy().to_string()) {
                    found.push(t);
                }
            }
        }
    }
    found.sort();
    found
}

// QA A1-1 (2026-10-08): a file deleted for holding only old records stayed open in the readers'
// file cache, so its bytes stayed allocated on disk after `compact_now` returned.
#[test]
fn deleted_file_is_not_held_open() -> Outcome<()> {
    let _lock = lock();
    let s = res!(plain("held_open", GC_OFF, 1, 1));
    let user = Uid::default();
    res!(fill(&s.db));
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));
    // Reopened, so that the cache holds locations and no values and a read of key 2 goes to a
    // reader, which opens file 1 and keeps the handle.
    let (root, cfg, dir) = (s.root.clone(), s.cfg.clone(), s.dir.clone());
    res!(s.db.close());
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, GC_OFF, false));
    let mut want: Vec<Option<u8>> = vec![Some(1); NKEYS];
    assert_eq!(reads(&db, &want, "Before the overwrite"), 0);
    let dat1 = file(&root, &cfg, FileType::Data, 1).to_string_lossy().to_string();
    let held = open_files_under(&root);
    assert!(held.contains(&dat1), "positive control: no reader holds {} open, only {:?}", dat1, held);

    // Every key of file 1 overwritten leaves it holding old records alone.
    for i in 0..5usize {
        res!(db.insert(key(i), value(i, 2), user, None));
        want[i] = Some(2);
    }
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    let needle = match value(2, 1).bytes_ref() { Some(b) => b.clone(), None => Vec::new() };
    assert!(!files_holding(&root, &cfg, &needle).is_empty(), "positive control: key 2 version 1 is in no file");
    let report = res!(db.compact_now(Duration::from_secs(30)));
    msg!("{:?}", report);
    assert!(report.files_deleted >= 1, "no file was deleted, so the test proves nothing: {:?}", report);
    assert!(!file(&root, &cfg, FileType::Data, 1).is_file(), "file 1 is still on disk");
    assert!(files_holding(&root, &cfg, &needle).is_empty(), "key 2 version 1 is still in a data file");

    let gone: Vec<String> = open_files_under(&root).into_iter().filter(|t| t.ends_with(" (deleted)")).collect();
    assert!(gone.is_empty(), "compact_now returned with deleted files held open: {:?}", gone);
    assert_eq!(reads(&db, &want, "After compact_now"), 0);
    res!(db.close());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

// QA A1-4 (2026-10-08): the roll to new live files waited out the control deadline of five
// minutes instead of the caller's, so a writer behind a long queue held `compact_now` past its
// deadline before the first check of it.
#[test]
fn deadline_bounds_the_roll_to_new_live_files() -> Outcome<()> {
    let _lock = lock();
    const N: usize = 8;
    let s = res!(plain("roll", GC_OFF, 2, 2));
    let user = Uid::default();
    for i in 0..(N + 2) {
        res!(s.db.insert(key(i), value(i, 1), user, None));
    }
    res!(s.db.api().settle_for_test(Duration::from_secs(10)));

    // Each supersession a file bot receives from another is held for two seconds, so the writer's
    // request to close its old live file waits behind them.
    hooks::set_schedule_delay(Duration::from_secs(2));
    for i in 0..N {
        assert!(res!(s.db.delete(&key(i), user, None)), "the delete of key {} found nothing", i);
    }
    let t = Instant::now();
    let result = s.db.compact_now(Duration::from_secs(1));
    let took = t.elapsed();
    hooks::set_schedule_delay(Duration::ZERO);
    let e = match result {
        Err(e) => e,
        Ok(r) => return Err(err!("compact_now returned Ok({:?}) with the file bots held.", r; Test, Invalid)),
    };
    let text = fmt!("{}", e);
    msg!("compact_now failed after {:?} with: {}", took, text.chars().take(400).collect::<String>());
    assert!(e.tags().contains(&ErrTag::Timeout), "the error is not a timeout: {:?}", e.tags());
    assert!(took < Duration::from_secs(4), "the deadline of one second was kept to {:?}", took);

    // Released, the same call finishes.
    let report = res!(s.db.compact_now(Duration::from_secs(60)));
    msg!("{:?}", report);
    s.end()
}
