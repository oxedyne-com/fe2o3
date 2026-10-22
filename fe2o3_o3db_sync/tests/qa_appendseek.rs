//! The APPENDSEEK QA probes (rc-b, 2026-10-11, written against ac09ec89), adopted with the
//! fix-forward of findings F1 and F3: a stop after a failed cut back is named as such, never as a
//! failed sync, and a tear hook that an append outran is not counted as a tear.  Each test gathers
//! every deviation into `bad` and prints its observations, so one run shows the whole picture.
//! The hooks are process-wide, so the tests run one at a time.

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
    file::{
        core::FileType,
        refusal::Refusal,
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
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    time::Duration,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    let g = match SERIAL.lock() { Ok(g) => g, Err(p) => p.into_inner() };
    disarm();
    g
}

fn disarm() {
    hooks::set_write_tear(FileType::Data, None);
    hooks::set_write_tear(FileType::Index, None);
    hooks::set_cut_back_failure(false);
    hooks::set_barrier_failure(false);
    hooks::set_fault_record_fails_at(0);
    hooks::set_owed_drop(false);
}

// One zone, one writer.  `held` holds the sync back for an hour, else every write syncs.
fn cfg(held: bool, chunked: bool, max: u64) -> Outcome<OzoneConfig> {
    let mut c = res!(setup::default_cfg());
    c.num_zones             = 1;
    c.num_wbots_per_zone    = 1;
    c.num_cbots_per_zone    = 4;
    c.num_fbots_per_zone    = 1;
    c.zone_overrides        = BTreeMap::new();
    c.sync_on_write         = !held;
    c.sync_every_n_writes   = 0;
    c.sync_interval_ms      = if held { 3_600_000 } else { 0 };
    c.data_file_max_bytes   = max;
    if chunked {
        c.rest_chunk_threshold  = 1_000;
        c.rest_chunk_bytes      = 400;
    }
    Ok(c)
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path, c: &OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(c.clone()), schemes(), None, false, wipe)
}

fn wait() -> Wait {
    Wait { max_wait: Duration::from_secs(60), check_interval: constant::CHECK_INTERVAL }
}

fn val(len: usize, seed: u8) -> Dat {
    Dat::BU32((0..len).map(|j| seed ^ (j as u8)).collect())
}

fn put(db: &TestDb, k: &str, len: usize, seed: u8) -> Outcome<()> {
    db.insert(dat!(k), val(len, seed), Uid::default(), None).map(|_| ())
}

// "L/S" for a whole value, "none", "TORN" or "ERR:...".
fn read(db: &TestDb, k: &str) -> String {
    match db.get(&dat!(k), None) {
        Ok(None) => fmt!("none"),
        Ok(Some((Dat::BU32(b), _))) if !b.is_empty()
            && b.iter().enumerate().all(|(j, x)| *x == b[0] ^ (j as u8)) => fmt!("{}/{}", b.len(), b[0]),
        Ok(Some(_)) => fmt!("TORN"),
        Err(e) => fmt!("ERR:{}", plain(&fmt!("{}", e))),
    }
}

// Text without colour codes.
fn bare(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for ch in s.chars() {
        if ch == '\u{1b}' { esc = true; continue; }
        if esc { if ch == 'm' { esc = false; } continue; }
        out.push(ch);
    }
    out
}

// The innermost message of an error, without colour codes.
fn plain(s: &str) -> String {
    let out = bare(s);
    match out.rfind("LocalErr") {
        Some(i) => out[i..].chars().take(400).collect(),
        None    => out.chars().take(400).collect(),
    }
}

fn live(root: &Path, c: &OzoneConfig) -> Outcome<(u32, PathBuf, PathBuf)> {
    let n = res!((1..10_000u32).rev().find(|n| file(root, c, FileType::Data, *n).exists())
        .ok_or_else(|| err!("No data file is live."; Test)));
    Ok((n, file(root, c, FileType::Data, n), file(root, c, FileType::Index, n)))
}

fn marker(root: &Path) -> String {
    fs::read_to_string(Refusal::path(root)).unwrap_or_else(|_| fmt!("<no marker>"))
}

fn record(root: &Path) -> String {
    fs::read_to_string(root.join(constant::FAULT_FILENAME)).unwrap_or_else(|_| fmt!("<no record>"))
}

fn check(bad: &mut Vec<String>, db: &TestDb, want: &[(&str, &str)], when: &str) {
    for (k, v) in want {
        let got = read(db, k);
        if got != *v {
            bad.push(fmt!("{}: {} reads {} not {}", when, k, got, v));
        }
    }
}

fn finish(name: &str, bad: Vec<String>) -> Outcome<()> {
    eprintln!("QA[{}] {} deviation(s)", name, bad.len());
    for b in &bad {
        eprintln!("QA[{}] BAD {}", name, b);
    }
    if !bad.is_empty() {
        return Err(err!("{}: {:?}", name, bad; Test));
    }
    Ok(())
}

// F1: a refusal on a store stopped by a failed cut back names the cut back, not a sync.
fn names_cut_back(bad: &mut Vec<String>, what: &str, r: &Outcome<()>) {
    match r {
        Ok(()) => bad.push(fmt!("{} succeeded on a stopped store", what)),
        Err(e) => {
            let t = bare(&fmt!("{}", e));
            if !t.contains("stopped writing after a failed cut back of a torn append") {
                bad.push(fmt!("{} does not name the failed cut back: {}", what, plain(&t)));
            }
            if t.contains("failed sync") {
                bad.push(fmt!("{} names a failed sync: {}", what, plain(&t)));
            }
        },
    }
}

// F1: the marker and the record left by a failed cut back carry its cause.
fn marks_cut_back(bad: &mut Vec<String>, root: &Path) {
    let mk = marker(root);
    if !mk.lines().any(|l| l.starts_with("why a torn append could not be cut back at run time: ")) {
        bad.push(fmt!("the marker's why line does not name the cut back:\n{}", mk));
    }
    if !mk.lines().any(|l| l == "cause cut-back") {
        bad.push(fmt!("the marker names no cut-back cause:\n{}", mk));
    }
    let rc = record(root);
    if !rc.lines().any(|l| l == "cause cut-back") {
        bad.push(fmt!("the record names no cut-back cause:\n{}", rc));
    }
}

// Surface 1: sync held back, k1 k2 acknowledged past the mark, k3 torn on `typ`, cut fails.
fn torn_under_held_sync(name: &str, typ: FileType, crash_to: Option<i64>) -> Outcome<()> {
    let mut bad = Vec::new();
    let root = res!(fresh(&fmt!("./test_db_qa_seek_{}", name)));
    let c = res!(cfg(true, false, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 100, 1));
    res!(db.api().sync_now(wait()));
    let (_, dat, ind) = res!(live(&root, &c));
    let mark = res!(db.api().chans().marks().files(0));
    let m0 = size(&dat);
    for (k, s) in [("k1", 2u8), ("k2", 3)] {
        if let Err(e) = put(&db, k, 100, s) {
            bad.push(fmt!("{} under a held sync was refused: {}", k, e));
        }
    }
    let (dlen, ilen) = (size(&dat), size(&ind));
    let rec = dlen - m0;
    hooks::set_cut_back_failure(true);
    match typ {
        FileType::Data  => hooks::set_write_tear(FileType::Data, Some(rec / 4)),
        FileType::Index => hooks::set_write_tear(FileType::Index, Some(3)),
    }
    let failed = put(&db, "k3", 100, 4);
    disarm();
    if failed.is_ok() { bad.push(fmt!("the torn k3 was confirmed")); }
    eprintln!("QA[{}] marks {:?} m0 {} dlen {} ilen {} now ({}, {})",
        name, mark, m0, dlen, ilen, size(&dat), size(&ind));
    let report = db.api().fault();
    match &report {
        None => bad.push(fmt!("no fault after a failed cut back")),
        Some(r) => {
            eprintln!("QA[{}] cuts {:?}", name, r.cuts);
            if !r.cuts.iter().any(|x| x.data_upto == dlen && x.index_upto == ilen) {
                bad.push(fmt!("fault cuts {:?} do not name ({}, {})", r.cuts, dlen, ilen));
            }
        },
    }
    // Observation only: the durable-mark design refuses, in process, reads past the mark once
    // the store has stopped.
    for k in ["k0", "k1", "k2", "k3"] {
        eprintln!("QA[{}] OBS in process after the fault {} = {}", name, k, read(&db, k));
    }
    names_cut_back(&mut bad, "k4's put", &put(&db, "k4", 100, 5));
    names_cut_back(&mut bad, "sync_now", &db.api().sync_now(wait()));
    let _ = db.close();
    eprintln!("QA[{}] after close ({}, {}) marker:\n{}record:\n{}", name, size(&dat), size(&ind),
        marker(&root), record(&root));
    marks_cut_back(&mut bad, &root);
    // A power cut leaves the files somewhere between the mark and their lengths.
    if let Some(to) = crash_to {
        let upto = (m0 as i64 + to) as u64;
        let f = res!(fs::OpenOptions::new().write(true).open(&dat));
        res!(f.set_len(upto));
        let ilen0 = mark.iter().map(|(_, m)| m.1).max().unwrap_or(0);
        let f = res!(fs::OpenOptions::new().write(true).open(&ind));
        res!(f.set_len(ilen0));
        eprintln!("QA[{}] crash: data to {}, index to {}", name, upto, ilen0);
    }
    // The marker refuses the open until cleared.
    match open(&root, &c, false) {
        Ok(db) => { bad.push(fmt!("the store opened over its refusal marker")); let _ = db.close(); },
        Err(e) => eprintln!("QA[{}] refused as wanted: {}", name, fmt!("{}", e).chars().take(120).collect::<String>()),
    }
    match Refusal::clear(&root) {
        Ok(true) => (),
        other => bad.push(fmt!("clear returned {:?}", other)),
    }
    let db = res!(open(&root, &c, false));
    eprintln!("QA[{}] after open ({}, {})", name, size(&dat), size(&ind));
    let want: Vec<(&str, &str)> = match crash_to {
        None => vec![("k0", "100/1"), ("k1", "100/2"), ("k2", "100/3"), ("k3", "none")],
        // k1, k2 were never synced: lost or whole, never torn; k0 was synced.
        Some(_) => vec![("k0", "100/1"), ("k3", "none")],
    };
    check(&mut bad, &db, &want, "after the cleared open");
    if crash_to.is_none() && size(&dat) != dlen {
        bad.push(fmt!("data file {} after open, not the cut {}", size(&dat), dlen));
    }
    if crash_to.is_some() {
        for k in ["k1", "k2"] {
            let r = read(&db, k);
            if r != "none" && !r.starts_with("100/") { bad.push(fmt!("after crash {} reads {}", k, r)); }
            eprintln!("QA[{}] after crash {} = {}", name, k, r);
        }
    }
    if let Err(e) = put(&db, "k5", 100, 6) { bad.push(fmt!("k5 after the open failed: {}", e)); }
    if Refusal::path(&root).exists() { bad.push(fmt!("a marker stands after a clean open")); }
    res!(db.close());
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("k0", "100/1"), ("k3", "none"), ("k5", "100/6")], "after a second open");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish(name, bad)
}

#[test]
fn s1_torn_data_failed_cut_under_held_sync() -> Outcome<()> {
    let _g = lock();
    torn_under_held_sync("s1d", FileType::Data, None)
}

#[test]
fn s1_torn_index_failed_cut_under_held_sync() -> Outcome<()> {
    let _g = lock();
    torn_under_held_sync("s1i", FileType::Index, None)
}

// The power is cut part-way through k1 before the restart.
#[test]
fn s1_torn_data_failed_cut_then_power_cut_mid_record() -> Outcome<()> {
    let _g = lock();
    torn_under_held_sync("s1c", FileType::Data, Some(37))
}

// Surface 1 at rollover: the torn append starts a new live pair, and its cut fails.
#[test]
fn s1_torn_first_append_of_a_new_pair_failed_cut() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_s1r"));
    let c = res!(cfg(false, false, 2_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 100, 1));
    res!(put(&db, "k1", 100, 2));
    let (n0, d0, _) = res!(live(&root, &c));
    let rec = size(&d0) / 2 + 1;
    let mut k = 2u8;
    // Fill until the next put cannot fit, so that it rolls over.
    loop {
        let before = size(&d0);
        if before + rec > 2_000 { break; }
        res!(put(&db, &fmt!("k{}", k), 100, k + 1));
        k += 1;
    }
    hooks::set_cut_back_failure(true);
    hooks::set_write_tear(FileType::Data, Some(20));
    let failed = put(&db, "kr", 100, 99);
    disarm();
    if failed.is_ok() { bad.push(fmt!("the torn kr was confirmed")); }
    let (n1, d1, i1) = res!(live(&root, &c));
    eprintln!("QA[s1r] pair {} -> {}; new data {} index {}; cuts {:?}", n0, n1,
        size(&d1), size(&i1), db.api().fault().map(|r| r.cuts));
    if n1 == n0 { bad.push(fmt!("positive control: no rollover happened")); }
    let _ = db.close();
    match Refusal::clear(&root) {
        Ok(true) => (),
        other => bad.push(fmt!("clear returned {:?}", other)),
    }
    let db = res!(open(&root, &c, false));
    eprintln!("QA[s1r] after open new data {} index {}", size(&d1), size(&i1));
    let mut want = vec![("kr".to_string(), "none".to_string())];
    for j in 0..k { want.push((fmt!("k{}", j), fmt!("100/{}", j + 1))); }
    let want: Vec<(&str, &str)> = want.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    check(&mut bad, &db, &want, "after the cleared open");
    if let Err(e) = put(&db, "kz", 100, 50) { bad.push(fmt!("kz after the open failed: {}", e)); }
    res!(db.close());
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("kz", "100/50"), ("kr", "none")], "after a second open");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("s1r", bad)
}

// Surface 2: the writer's cut above the mark, then a barrier fails, so the syncer cuts to the
// mark as well.  The two must compose to the lower; the marker must carry both.
#[test]
fn s2_writer_cut_above_mark_with_syncer_cut_to_mark() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_s2"));
    let c = res!(cfg(true, false, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 100, 1));
    res!(db.api().sync_now(wait()));
    let (_, dat, ind) = res!(live(&root, &c));
    let mark = res!(db.api().chans().marks().files(0));
    res!(put(&db, "k1", 100, 2));
    let (dlen, ilen) = (size(&dat), size(&ind));
    hooks::set_cut_back_failure(true);
    hooks::set_write_tear(FileType::Data, Some(30));
    let _ = put(&db, "k2", 100, 3);
    disarm();
    // The syncer's barrier now fails.  sync_now refuses on a stopped store, so the barrier comes
    // from the close; report what each does.
    hooks::set_barrier_failure(true);
    let sn = db.api().sync_now(wait());
    let cl = db.close();
    hooks::set_barrier_failure(false);
    eprintln!("QA[s2] mark {:?} writer cut ({}, {}); sync_now {:?}; close ok {}",
        mark, dlen, ilen, sn.as_ref().map_err(|e| fmt!("{}", e).chars().take(80).collect::<String>()), cl.is_ok());
    let mk = marker(&root);
    let rc = record(&root);
    eprintln!("QA[s2] marker:\n{}record:\n{}", mk, rc);
    let cuts = |t: &str| t.lines().filter(|l| l.starts_with("cut ")).count();
    if cuts(&mk) != cuts(&rc) { bad.push(fmt!("marker and record disagree on cuts")); }
    if open(&root, &c, false).map(|db| { let _ = db.close(); }).is_ok() {
        bad.push(fmt!("the store opened over its refusal marker"));
    }
    let _ = Refusal::clear(&root);
    let db = res!(open(&root, &c, false));
    let low = mark.iter().map(|(_, m)| m.0).max().unwrap_or(0);
    eprintln!("QA[s2] after open data {} (mark {}, writer cut {})", size(&dat), low, dlen);
    if size(&dat) > dlen { bad.push(fmt!("data {} above the writer's cut {}", size(&dat), dlen)); }
    let k1 = read(&db, "k1");
    eprintln!("QA[s2] k1 = {}", k1);
    if k1 != "none" && k1 != "100/2" { bad.push(fmt!("k1 reads {}", k1)); }
    check(&mut bad, &db, &[("k0", "100/1"), ("k2", "none")], "after the cleared open");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("s2", bad)
}

// Surface 2, "nothing more": a torn append cut back cleanly leaves no marker, no record, and the
// store opens without a clear.
#[test]
fn s2_clean_cut_back_leaves_no_refusal() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_s2n"));
    let c = res!(cfg(true, false, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 100, 1));
    hooks::set_write_tear(FileType::Data, Some(30));
    let _ = put(&db, "k1", 100, 2);
    hooks::set_write_tear(FileType::Index, Some(3));
    let _ = put(&db, "k2", 100, 3);
    disarm();
    res!(put(&db, "k3", 100, 4));
    if db.api().fault().is_some() { bad.push(fmt!("a clean cut back stopped the store")); }
    if let Err(e) = db.api().sync_now(wait()) { bad.push(fmt!("sync_now after clean cuts: {}", e)); }
    res!(db.close());
    if Refusal::path(&root).exists() { bad.push(fmt!("a clean cut back wrote a marker")); }
    if root.join(constant::FAULT_FILENAME).exists() { bad.push(fmt!("a clean cut back wrote a record")); }
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("k0", "100/1"), ("k1", "none"), ("k2", "none"), ("k3", "100/4")], "after reopen");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("s2n", bad)
}

// Surface 3: a chunked put whose middle chunk is torn and whose cut fails.  The set stays held in
// the stopped process; after the clear and the open, the old value reads whole and the sweep
// retires what the failed put left.  The tear is armed by a thread watching the file grow, which
// under load can arm it only after the put has finished; such a try proves nothing and is run
// again on a fresh store.
fn chunk_case(name: &str, fail_cut: bool, arm_after: u64, tear: u64) -> Outcome<()> {
    for attempt in 1..=5 {
        match res!(chunk_try(name, fail_cut, arm_after, tear)) {
            Some(bad)   => return finish(name, bad),
            None        => eprintln!("QA[{}] try {}: the tear missed the put, trying again",
                name, attempt),
        }
    }
    Err(err!("{}: the tear missed the put in every try.", name; Test))
}

// One try of `chunk_case`: `None` when the tear did not fire within the put.
fn chunk_try(name: &str, fail_cut: bool, arm_after: u64, tear: u64)
    -> Outcome<Option<Vec<String>>>
{
    let mut bad = Vec::new();
    let root = res!(fresh(&fmt!("./test_db_qa_seek_{}", name)));
    let c = res!(cfg(false, true, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "K", 3_000, 1));
    res!(put(&db, "S", 50, 9));
    let (_, dat, _) = res!(live(&root, &c));
    let base = size(&dat);
    // Arm the tear once the put's first chunk is in, so a middle chunk tears.
    let stop = Arc::new(AtomicBool::new(false));
    let armer = {
        let stop = stop.clone();
        let dat = dat.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if size(&dat) > base + arm_after {
                    hooks::set_write_tear(FileType::Data, Some(tear));
                    return true;
                }
                std::hint::spin_loop();
            }
            false
        })
    };
    if fail_cut { hooks::set_cut_back_failure(true); }
    let torn0 = hooks::writes_torn();
    let failed = put(&db, "K", 3_000, 2);
    eprintln!("QA[{}] data grew {} bytes", name, size(&dat) - base);
    stop.store(true, Ordering::SeqCst);
    let armed = armer.join().unwrap_or(false);
    let torn = hooks::writes_torn() - torn0;
    disarm();
    eprintln!("QA[{}] armed {} torn {} put {:?}", name, armed, torn,
        failed.as_ref().map_err(|e| fmt!("{}", e).chars().take(100).collect::<String>()));
    if torn == 0 {
        let _ = db.close();
        let _ = fs::remove_dir_all(&root);
        return Ok(None);
    }
    if failed.is_ok() { bad.push(fmt!("a chunked put with a torn chunk was confirmed")); }
    let held = |db: &TestDb| -> bool {
        match db.api().chans().pending_sets().lock() { Ok(g) => !g.is_empty(), Err(p) => !p.into_inner().is_empty() }
    };
    std::thread::sleep(Duration::from_millis(500));
    let h1 = held(&db);
    eprintln!("QA[{}] fault {} held {}", name, db.api().fault().is_some(), h1);
    let head = arm_after > 3_000;
    if fail_cut && head && !h1 { bad.push(fmt!("the head's set was released after a failed cut")); }
    if !fail_cut && h1 { bad.push(fmt!("the set stays held after a clean cut")); }
    check(&mut bad, &db, &[("K", "3000/1"), ("S", "50/9")], "in process after the failed put");
    if fail_cut {
        match sweep::sweep_orphans(db.api(), Uid::default(), None, wait()) {
            Ok(r) => bad.push(fmt!("the sweep ran on a stopped store: {:?}", r.orphans_found)),
            Err(_) => (),
        }
        if head && !held(&db) { bad.push(fmt!("the hold went before the clear")); }
        let _ = db.close();
        if open(&root, &c, false).map(|db| { let _ = db.close(); }).is_ok() {
            bad.push(fmt!("the store opened over its refusal marker"));
        }
        match Refusal::clear(&root) {
            Ok(true) => (),
            other => bad.push(fmt!("clear returned {:?}", other)),
        }
    } else {
        res!(db.close());
    }
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("K", "3000/1"), ("S", "50/9")], "after the open");
    match sweep::sweep_orphans(db.api(), Uid::default(), None, wait()) {
        Ok(r) => eprintln!("QA[{}] sweep after open: orphans {} skipped_pending {}", name,
            r.orphans_found, r.skipped_pending),
        Err(e) => bad.push(fmt!("sweep after open failed: {}", e)),
    }
    if held(&db) { bad.push(fmt!("a set is held in the new process")); }
    check(&mut bad, &db, &[("K", "3000/1"), ("S", "50/9")], "after the sweep");
    if let Err(e) = put(&db, "K", 3_000, 3) { bad.push(fmt!("a new chunked put failed: {}", e)); }
    res!(db.close());
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("K", "3000/3"), ("S", "50/9")], "after the second open");
    match sweep::sweep_orphans(db.api(), Uid::default(), None, wait()) {
        Ok(r) => if r.orphans_found > 0 { bad.push(fmt!("{} orphans after the second open", r.orphans_found)); },
        Err(e) => bad.push(fmt!("second sweep failed: {}", e)),
    }
    check(&mut bad, &db, &[("K", "3000/3"), ("S", "50/9")], "after the second sweep");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(Some(bad))
}

#[test]
fn s3_chunk_torn_failed_cut_holds_then_clears() -> Outcome<()> {
    let _g = lock();
    chunk_case("s3f", true, 400, 100)
}

#[test]
fn s3_chunk_torn_clean_cut_releases() -> Outcome<()> {
    let _g = lock();
    chunk_case("s3c", false, 400, 100)
}

// The head, the last append of the put, torn: the writer keeps its set's hold when its cut fails.
#[test]
fn s3_head_torn_failed_cut_holds_then_clears() -> Outcome<()> {
    let _g = lock();
    chunk_case("s3hf", true, 3_951, 10)
}

#[test]
fn s3_head_torn_clean_cut_releases() -> Outcome<()> {
    let _g = lock();
    chunk_case("s3hc", false, 3_951, 10)
}

// Surface 1, pipelined: under sync_on_write, k1's barrier is held back (delayed) when k2 tears and
// its cut fails.  k1's caller's answer must agree with what the restart serves: confirmed and
// present, or failed and absent.
#[test]
fn s1_pipelined_barrier_held_when_cut_fails() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_s1p"));
    let c = res!(cfg(false, false, 1_000_000));
    let db = Arc::new(res!(open(&root, &c, true)));
    res!(put(&db, "k0", 100, 1));
    let (_, dat, ind) = res!(live(&root, &c));
    let d0 = size(&dat);
    hooks::set_barrier_delay(Duration::from_millis(1_500));
    let t = {
        let db = db.clone();
        std::thread::spawn(move || put(&db, "k1", 100, 2).map_err(|e| plain(&fmt!("{}", e))))
    };
    // Wait for k1 to reach the files, its barrier still to come.
    let landed = wait_until(Duration::from_secs(5), || size(&dat) > d0);
    let (dlen, ilen) = (size(&dat), size(&ind));
    hooks::set_cut_back_failure(true);
    hooks::set_write_tear(FileType::Data, Some(30));
    let t0 = std::time::Instant::now();
    let failed = put(&db, "k2", 100, 3);
    let k2_ms = t0.elapsed().as_millis();
    let k1_done = t.is_finished();
    hooks::set_cut_back_failure(false);
    hooks::set_write_tear(FileType::Data, None);
    eprintln!("QA[s1p] k2 answered in {} ms; k1 answered before k2: {}", k2_ms, k1_done);
    if k1_done { bad.push(fmt!("positive control: k1's barrier was not pending when k2 tore")); }
    let k1 = t.join().unwrap_or_else(|_| Err(fmt!("panicked")));
    hooks::set_barrier_delay(Duration::from_millis(0));
    eprintln!("QA[s1p] landed {} k1 answer {:?} k2 failed {} cuts {:?}", landed, k1, failed.is_err(),
        db.api().fault().map(|r| r.cuts.iter().map(|x| x.line()).collect::<Vec<_>>()));
    if !landed { bad.push(fmt!("positive control: k1 never reached the file")); }
    if failed.is_ok() { bad.push(fmt!("the torn k2 was confirmed")); }
    eprintln!("QA[s1p] OBS in process k1 = {}", read(&db, "k1"));
    let db = match Arc::try_unwrap(db) { Ok(db) => db, Err(_) => return Err(err!("db still shared"; Test)) };
    let _ = db.close();
    let _ = Refusal::clear(&root);
    let db = res!(open(&root, &c, false));
    let k1r = read(&db, "k1");
    eprintln!("QA[s1p] after open k1 = {} data {} (cut {}) index {} (cut {})", k1r, size(&dat), dlen, size(&ind), ilen);
    match (&k1, k1r.as_str()) {
        (Ok(()), "100/2") | (Err(_), "none") => (),
        (Ok(()), r) => bad.push(fmt!("k1 confirmed but reads {} after the restart", r)),
        (Err(e), r) => bad.push(fmt!("k1 reported failed ({}) but reads {} after the restart", e, r)),
    }
    check(&mut bad, &db, &[("k0", "100/1"), ("k2", "none")], "after the cleared open");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("s1p", bad)
}

// F1, on its own: the stop after a failed cut back is reported as one by every refusal, by the
// marker and by the record, under a sync on every write.
#[test]
fn f1_failed_cut_back_is_named_as_a_cut_back() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_f1"));
    let c = res!(cfg(false, false, 1_000_000));
    let db = res!(open(&root, &c, true));
    res!(put(&db, "k0", 100, 1));
    hooks::set_cut_back_failure(true);
    hooks::set_write_tear(FileType::Data, Some(30));
    let failed = put(&db, "k1", 100, 2);
    disarm();
    if failed.is_ok() { bad.push(fmt!("the torn k1 was confirmed")); }
    names_cut_back(&mut bad, "k2's put", &put(&db, "k2", 100, 3));
    names_cut_back(&mut bad, "sync_now", &db.api().sync_now(wait()));
    names_cut_back(&mut bad, "the sweep",
        &sweep::sweep_orphans(db.api(), Uid::default(), None, wait()).map(|_| ()));
    let _ = db.close();
    marks_cut_back(&mut bad, &root);
    let _ = Refusal::clear(&root);
    let db = res!(open(&root, &c, false));
    check(&mut bad, &db, &[("k0", "100/1"), ("k1", "none"), ("k2", "none")], "after the cleared open");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("f1", bad)
}

// F3: a tear armed past the end of the append writes it whole, so it is not counted as torn.
#[test]
fn f3_an_append_shorter_than_the_tear_is_not_counted_torn() -> Outcome<()> {
    let _g = lock();
    let mut bad = Vec::new();
    let root = res!(fresh("./test_db_qa_seek_f3"));
    let c = res!(cfg(false, false, 1_000_000));
    let db = res!(open(&root, &c, true));
    let before = hooks::writes_torn();
    hooks::set_write_tear(FileType::Data, Some(100_000));
    if let Err(e) = put(&db, "k0", 100, 1) { bad.push(fmt!("k0 under an outrun tear failed: {}", e)); }
    disarm();
    if hooks::writes_torn() != before {
        bad.push(fmt!("an append the tear never reached was counted torn ({} -> {})",
            before, hooks::writes_torn()));
    }
    // Positive control: a tear the append reaches is counted.
    hooks::set_write_tear(FileType::Data, Some(10));
    if put(&db, "k1", 100, 2).is_ok() { bad.push(fmt!("the torn k1 was confirmed")); }
    disarm();
    if hooks::writes_torn() != before + 1 {
        bad.push(fmt!("a real tear was not counted once ({} -> {})", before, hooks::writes_torn()));
    }
    check(&mut bad, &db, &[("k0", "100/1"), ("k1", "none")], "in process");
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    finish("f3", bad)
}
