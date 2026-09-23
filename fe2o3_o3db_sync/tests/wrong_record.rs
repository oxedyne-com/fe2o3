//! Reads, writes and garbage collection all at once, with every record the same size, and every
//! value judged against something outside the store: the key it was asked for, and the versions
//! its writer issued and had acknowledged.  A read that raced a collection returned another key's
//! value, or an older version of its own, about once in three collections (QA of lane sto,
//! 2026-09-23, whose stress harness this is); with collection off there were none.  Each writer
//! thread owns its keys, so versions and stamps agree.  The long variant is ignored by default.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::cfg::OzoneConfig,
    comm::response::Wait,
    data::core::RestSchemesInput,
    test::setup::{
        self,
        Uid,
        UID_LEN,
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

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

fn env_u64(name: &str, dflt: u64) -> u64 {
    match std::env::var(name) {
        Ok(s) => match s.parse::<u64>() {
            Ok(n) => n,
            Err(_) => dflt,
        },
        Err(_) => dflt,
    }
}

#[derive(Default)]
struct Counts {
    writes:     AtomicU64,
    write_errs: AtomicU64,
    reads:      AtomicU64,
    wrong_key:  AtomicU64,
    stale:      AtomicU64,
    phantom:    AtomicU64,
    none:       AtomicU64,
    read_errs:  AtomicU64,
    bad_shape:  AtomicU64,
    shrinks:    AtomicU64,
    removed:    AtomicU64,
    clears:     AtomicU64,
}

impl Counts {
    fn line(&self) -> String {
        fmt!("writes {} write_errs {} reads {} wrong_key {} stale {} phantom {} none {} read_errs {} \
            bad_shape {} | file shrinks {} removed {} clears {}",
            self.writes.load(Ordering::Relaxed),
            self.write_errs.load(Ordering::Relaxed),
            self.reads.load(Ordering::Relaxed),
            self.wrong_key.load(Ordering::Relaxed),
            self.stale.load(Ordering::Relaxed),
            self.phantom.load(Ordering::Relaxed),
            self.none.load(Ordering::Relaxed),
            self.read_errs.load(Ordering::Relaxed),
            self.bad_shape.load(Ordering::Relaxed),
            self.shrinks.load(Ordering::Relaxed),
            self.removed.load(Ordering::Relaxed),
            self.clears.load(Ordering::Relaxed),
        )
    }
}

struct Plan {
    nkeys:      usize,
    vbytes:     usize,
    nw:         usize,
    nr:         usize,
    secs:       u64,
    clear_ms:   u64,
    phases:     u64,
    min_gc:     u64, // collections a run must see to have tested anything
}

fn key(i: usize) -> Dat { dat!(fmt!("wrong record key {:04}", i)) }

fn value(i: usize, ver: u64, len: usize) -> Dat {
    let mut v = vec![0u8; len];
    v[0] = (i >> 8) as u8;
    v[1] = i as u8;
    v[2..10].copy_from_slice(&ver.to_be_bytes());
    for j in 10..len {
        v[j] = (i as u8) ^ (ver as u8) ^ (j as u8);
    }
    Dat::BU32(v)
}

/// The key index and version a value says it holds, if it has the shape of one of ours.
fn parse(d: &Dat, len: usize) -> Option<(usize, u64)> {
    let v = d.bytes_ref()?;
    if v.len() != len { return None; }
    let i = ((v[0] as usize) << 8) | (v[1] as usize);
    let mut b = [0u8; 8];
    b.copy_from_slice(&v[2..10]);
    let ver = u64::from_be_bytes(b);
    for j in 10..len {
        if v[j] != (i as u8) ^ (ver as u8) ^ (j as u8) { return None; }
    }
    Some((i, ver))
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 2;
    cfg.num_fbots_per_zone      = 2;
    cfg.num_igbots_per_zone     = 2;
    cfg.num_rbots_per_zone      = 2;
    cfg.num_wbots_per_zone      = 2;
    let fbytes = 8_000;
    cfg.data_file_max_bytes     = fbytes;
    cfg.rest_chunk_threshold    = fbytes * 7 / 10; // the store refuses over 80%, values are far below
    cfg.zone_overrides          = BTreeMap::new();
    cfg.sync_on_write           = true;
    Ok(cfg)
}

fn schemes() -> RestSchemesInput<(), HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

fn open(root: &Path, wipe: bool) -> Outcome<TestDb> {
    // Collection off is the control: no fault of any kind in 3.1 M reads (2026-09-23).
    let gc_on = env_u64("WRONG_RECORD_GC", 1) == 1;
    setup::start_db(root.to_path_buf(), Some(res!(config())), schemes(), None, gc_on, wipe)
}

fn classify<M>(
    c:      &Counts,
    got:    Outcome<Option<(Dat, M)>>,
    i:      usize,
    lo:     u64,
    hi:     u64,
    len:    usize,
    who:    &str,
) {
    c.reads.fetch_add(1, Ordering::Relaxed);
    match got {
        Err(e) => {
            let n = c.read_errs.fetch_add(1, Ordering::Relaxed);
            if n < 5 { test!(sync_log::stream(), "{} read error on key {}: {}", who, i, e); }
        },
        Ok(None) => {
            if lo > 0 {
                let n = c.none.fetch_add(1, Ordering::Relaxed);
                if n < 5 { test!(sync_log::stream(), "{} key {} read None, acked {}", who, i, lo); }
            }
        },
        Ok(Some((d, _))) => match parse(&d, len) {
            None => {
                let n = c.bad_shape.fetch_add(1, Ordering::Relaxed);
                if n < 5 { test!(sync_log::stream(), "{} key {} read a value of no known shape: {:?}", who, i, d); }
            },
            Some((j, ver)) => {
                if j != i {
                    let n = c.wrong_key.fetch_add(1, Ordering::Relaxed);
                    if n < 10 { test!(sync_log::stream(), "{} WRONG KEY: asked {} got key {} version {}", who, i, j, ver); }
                } else if ver < lo {
                    let n = c.stale.fetch_add(1, Ordering::Relaxed);
                    if n < 10 { test!(sync_log::stream(), "{} STALE: key {} got version {} < acked {}", who, i, ver, lo); }
                } else if ver > hi {
                    let n = c.phantom.fetch_add(1, Ordering::Relaxed);
                    if n < 10 { test!(sync_log::stream(), "{} PHANTOM: key {} got version {} > issued {}", who, i, ver, hi); }
                }
            },
        },
    }
}

fn run_phase(
    db:     &TestDb,
    root:   &Path,
    plan:   &Plan,
    issued: &Arc<Vec<AtomicU64>>,
    acked:  &Arc<Vec<AtomicU64>>,
    c:      &Arc<Counts>,
) -> Outcome<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut hs = Vec::new();
    for w in 0..plan.nw {
        let db = db.clone();
        let (issued, acked, c, stop) = (issued.clone(), acked.clone(), c.clone(), stop.clone());
        let (nkeys, nw, vb) = (plan.nkeys, plan.nw, plan.vbytes);
        hs.push(res!(thread::Builder::new().name(fmt!("qa writer {}", w)).spawn(move || {
            let mine: Vec<usize> = (0..nkeys).filter(|k| k % nw == w).collect();
            let mut rng = Rng(0x9e3779b97f4a7c15 ^ (w as u64 + 1));
            while !stop.load(Ordering::Relaxed) {
                let k = mine[(rng.next() as usize) % mine.len()];
                let ver = issued[k].load(Ordering::Relaxed) + 1;
                issued[k].store(ver, Ordering::Relaxed);
                c.writes.fetch_add(1, Ordering::Relaxed);
                match db.insert(key(k), value(k, ver, vb), Uid::default(), None) {
                    Ok(_) => {
                        acked[k].store(ver, Ordering::Relaxed);
                        // Read one's own write back straight away.
                        let got = db.get(&key(k), None);
                        let hi = issued[k].load(Ordering::Relaxed);
                        classify(&c, got, k, ver, hi, vb, "rw");
                    },
                    Err(e) => {
                        let n = c.write_errs.fetch_add(1, Ordering::Relaxed);
                        if n < 5 { test!(sync_log::stream(), "write error on key {} v{}: {}", k, ver, e); }
                    },
                }
            }
        })));
    }
    for r in 0..plan.nr {
        let db = db.clone();
        let (issued, acked, c, stop) = (issued.clone(), acked.clone(), c.clone(), stop.clone());
        let (nkeys, vb) = (plan.nkeys, plan.vbytes);
        hs.push(res!(thread::Builder::new().name(fmt!("qa reader {}", r)).spawn(move || {
            let mut rng = Rng(0x2545f4914f6cdd1d ^ (r as u64 + 101));
            while !stop.load(Ordering::Relaxed) {
                let k = (rng.next() as usize) % nkeys;
                let lo = acked[k].load(Ordering::Relaxed);
                let got = db.get(&key(k), None);
                let hi = issued[k].load(Ordering::Relaxed);
                classify(&c, got, k, lo, hi, vb, "r");
            }
        })));
    }
    // Monitor: clears cached values so reads go to the files, and counts collections by
    // watching data files shrink or vanish.
    {
        let db = db.clone();
        let (c, stop) = (c.clone(), stop.clone());
        let zdir = res!(config()).zone_root(root).join("zone_001");
        let clear_ms = plan.clear_ms;
        hs.push(res!(thread::Builder::new().name(fmt!("qa monitor")).spawn(move || {
            let mut sizes: BTreeMap<PathBuf, u64> = BTreeMap::new();
            let mut last_clear = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                if clear_ms > 0 && last_clear.elapsed() >= Duration::from_millis(clear_ms) {
                    last_clear = Instant::now();
                    if db.api().clear_cache_values(Wait {
                        max_wait: Duration::from_secs(5),
                        check_interval: Duration::from_millis(10),
                    }).is_ok() {
                        c.clears.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let mut now: BTreeMap<PathBuf, u64> = BTreeMap::new();
                if let Ok(rd) = std::fs::read_dir(&zdir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        let is_dat = match p.extension() { Some(x) => x == "dat", None => false };
                        if is_dat {
                            if let Ok(m) = std::fs::metadata(&p) { now.insert(p, m.len()); }
                        }
                    }
                }
                for (p, s) in &sizes {
                    match now.get(p) {
                        Some(n) if n < s => { c.shrinks.fetch_add(1, Ordering::Relaxed); },
                        None => { c.removed.fetch_add(1, Ordering::Relaxed); },
                        _ => (),
                    }
                }
                sizes = now;
                thread::sleep(Duration::from_millis(5));
            }
        })));
    }
    thread::sleep(Duration::from_secs(plan.secs));
    stop.store(true, Ordering::Relaxed);
    for h in hs {
        if h.join().is_err() { test!(sync_log::stream(), "a thread panicked"); }
    }
    Ok(())
}

/// Every key must read back a version its writer issued no earlier than the last it had
/// acknowledged.  After a reopen this is the durability check.
fn verify(
    db:     &TestDb,
    plan:   &Plan,
    issued: &Arc<Vec<AtomicU64>>,
    acked:  &Arc<Vec<AtomicU64>>,
    label:  &str,
) -> Outcome<(u64, u64, String)> {
    let c = Counts::default();
    for k in 0..plan.nkeys {
        let lo = acked[k].load(Ordering::Relaxed);
        let hi = issued[k].load(Ordering::Relaxed);
        let got = db.get(&key(k), None);
        classify(&c, got, k, lo, hi, plan.vbytes, label);
    }
    let bad = c.wrong_key.load(Ordering::Relaxed) + c.stale.load(Ordering::Relaxed)
        + c.phantom.load(Ordering::Relaxed) + c.none.load(Ordering::Relaxed)
        + c.read_errs.load(Ordering::Relaxed) + c.bad_shape.load(Ordering::Relaxed);
    Ok((c.reads.load(Ordering::Relaxed), bad, fmt!("{}: {}", label, c.line())))
}

#[test]
fn main() -> Outcome<()> {
    run("./test_db_wrong_record", Plan {
        nkeys:      40,
        vbytes:     100,
        nw:         4,
        nr:         4,
        secs:       env_u64("WRONG_RECORD_SECS", 12),
        clear_ms:   20,
        phases:     2,
        min_gc:     20,
    })
}

/// The stress the fault was found with, three minutes of it.
#[test]
#[ignore]
fn long() -> Outcome<()> {
    run("./test_db_wrong_record_long", Plan {
        nkeys:      40,
        vbytes:     100,
        nw:         4,
        nr:         4,
        secs:       env_u64("WRONG_RECORD_SECS", 60),
        clear_ms:   20,
        phases:     3,
        min_gc:     100,
    })
}

fn run(dir: &str, plan: Plan) -> Outcome<()> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir); // absent the first time
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let issued: Arc<Vec<AtomicU64>> = Arc::new((0..plan.nkeys).map(|_| AtomicU64::new(0)).collect());
    let acked: Arc<Vec<AtomicU64>> = Arc::new((0..plan.nkeys).map(|_| AtomicU64::new(0)).collect());
    let mut silent = 0u64;
    let mut collections = 0u64;
    let mut lines = Vec::new();
    let mut db = res!(open(&root, true));
    for p in 0..plan.phases {
        let c = Arc::new(Counts::default());
        let begun = Instant::now();
        res!(run_phase(&db, &root, &plan, &issued, &acked, &c));
        lines.push(fmt!("phase {} ({:?}): {}", p, begun.elapsed(), c.line()));
        silent += c.wrong_key.load(Ordering::Relaxed) + c.stale.load(Ordering::Relaxed)
            + c.phantom.load(Ordering::Relaxed) + c.none.load(Ordering::Relaxed)
            + c.bad_shape.load(Ordering::Relaxed);
        collections += c.shrinks.load(Ordering::Relaxed) + c.removed.load(Ordering::Relaxed);
        thread::sleep(Duration::from_secs(2));
        let (_, bad, line) = res!(verify(&db, &plan, &issued, &acked, &fmt!("phase {} settled", p)));
        silent += bad;
        lines.push(line);
        res!(db.close());
        db = res!(open(&root, false));
        let (_, bad, line) = res!(verify(&db, &plan, &issued, &acked, &fmt!("phase {} reopened", p)));
        silent += bad;
        lines.push(line);
    }
    res!(db.close());
    // The counts are the finding, so they are shown whichever way the run goes.
    for line in &lines {
        msg!("{}", line);
    }
    log_finish_wait!();
    if silent > 0 {
        return Err(err!(
            "{} reads returned a value other than the one asked for (another key's, an older \
            version, one never written, or none), over {} collections: {:?}",
            silent, collections, lines;
            Test, Mismatch));
    }
    if collections < plan.min_gc {
        return Err(err!(
            "Only {} collections ran, fewer than the {} this run needs to have tested reads \
            racing them: {:?}", collections, plan.min_gc, lines;
            Test, Missing));
    }
    Ok(())
}
