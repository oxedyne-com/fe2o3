//! Ozone scan-under-garbage-collection integration test.
//!
//! A scan is a foreground request with a person waiting on it; garbage
//! collection is background work with no deadline at all. This test
//! holds the two apart.
//!
//! It builds a store whose files are all past the collection trigger
//! while collection is switched off, so no work has been done yet and
//! none is queued. A quiet scan then measures what the walk costs with
//! nothing in its way. Collection is switched on and a short burst of
//! writes dispatches the whole backlog at once, and the same scan is
//! issued again. Two properties are checked:
//!
//! 1. **A concurrent scan sees every key, and does not fail.**
//!    Collection rewrites a file's index. A scanner walking that index
//!    at the wrong moment can see a file with no index at all and drop
//!    every key whose current value lives in it, or read a half-written
//!    record and fail outright. Entry count and outcome are checked on
//!    every scan. This is the sharp half of the test: before the index
//!    rebuild was corrected it failed here every run.
//!
//! 2. **Latency does not track the collector's backlog.** The scan
//!    issued while the backlog is being worked through costs about what
//!    the quiet one did. This half is a tripwire rather than a proof.
//!    A one-shot backlog can only ever cost about as much as one full
//!    scan -- the collector's work over every file and the scan's walk
//!    over every file are both a fixed cost per record, so the whole
//!    backlog is roughly one scan's worth -- and the allowance below is
//!    set well clear of ordinary noise. What it catches is a change
//!    that puts the scan back on a queue behind unbounded background
//!    work; what it cannot show, at any size a test can afford, is the
//!    multi-second wait a store under sustained churn produces.
//!
//! A burst is used rather than a sustained write storm because it makes
//! the backlog a known quantity. Under a storm the collector is idle
//! most of the time -- work arrives for it in proportion to the write
//! rate, and it serves that work about as fast as several writers can
//! produce it -- so a queue forms only by luck, and the test would then
//! measure the machine rather than the database.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    path::Path,
    thread,
    time::{
        Duration,
        Instant,
    },
};

/// The dimensions of one run.
struct Shape {
    dir:        &'static str,   // directory name for this run's store
    keys:       usize,          // distinct keys held in the store
    // Value payload size in bytes, before encoding. Large, so that a
    // collection moves far more than a scan reads.
    val_bytes:  usize,
    file_bytes: u64,            // and so the size of one collection's work
}

// The store the crate's own test suite can afford.
const QUICK: Shape = Shape {
    dir:        "./test_db_scan_under_gc",
    keys:       6_000,
    val_bytes:  20_000,
    file_bytes: 8_000_000,
};

// A store several times larger, where the backlog is deep enough to cost more
// than the user request deadline outright rather than only running late. Writes
// some hundreds of megabytes.
const DEEP: Shape = Shape {
    dir:        "./test_db_scan_stall",
    keys:       30_000,
    val_bytes:  20_000,
    file_bytes: 8_000_000,
};

// Fraction of the keys superseded before collection is switched on. It has to
// clear OLD_DATA_PERCENT_GC_TRIGGER, which is measured against the maximum data
// file size rather than the file's own size.
const OLD_FRAC:     f64 = 0.5;

const NUM_BASELINE: usize = 3;  // quiet scans establishing the latency baseline
const NUM_LOADED:   usize = 5;  // scans issued while the backlog is worked off

/// The spacing of the keys superseded to set the backlog going: about two per
/// file, which reaches every file holding original
/// records while keeping the burst far shorter than the collection work
/// it dispatches. A finer stride lets the collector work the queue down
/// as fast as it is filled, and the test then measures nothing.
fn trigger_stride(shape: &Shape) -> usize {
    let per_file = (shape.file_bytes as usize) / shape.val_bytes;
    std::cmp::max(1, per_file / 2)
}

/// Runs the scan-under-collection test as its own integration binary.
///
/// It is kept out of `tests/main.rs` deliberately: it is the slowest
/// test in the crate and the only one that wants the disk to itself, so
/// `cargo test -p oxedyne_fe2o3_o3db_sync --test scan_under_gc` can run
/// it alone.
#[test]
fn main() -> Outcome<()> {

    // Logging every stored key costs several times what the writes do,
    // and this test writes a great many of them. `test` is the lowest
    // level that still carries the test's own reporting.
    log_set_level!("test");

    let outcome = test_scan_under_gc(&QUICK);

    log_finish_wait!();

    outcome
}

/// The same test against a store deep enough to reproduce the
/// production symptom rather than only the mechanism behind it. It
/// writes for some minutes, so it is ignored by default:
///
/// ```ignore
/// cargo test -p oxedyne_fe2o3_o3db_sync --test scan_under_gc -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn deep() -> Outcome<()> {

    log_set_level!("test");

    let outcome = test_scan_under_gc(&DEEP);

    log_finish_wait!();

    outcome
}

pub fn test_scan_under_gc(shape: &Shape) -> Outcome<()> {

    let db_root = res!(Path::new(shape.dir).canonicalize().or_else(|_| {
        ok!(std::fs::create_dir_all(shape.dir));
        Path::new(shape.dir).canonicalize()
    }));

    // Fixed key so the test is deterministic.
    let enckey = [0x37u8; 32];
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default()
            .set_encrypter(Override::Default(aes_gcm.clone()));
    let schms2 = Some(&schms2);
    let user = setup::Uid::default();

    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    let mut cfg = res!(setup::default_cfg());
    // One zone, and one bot of each kind in it, so there is exactly one
    // queue per role and no ambiguity about what a scan is waiting for.
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    // One collector is the sharpest form of the defect: a scan sent to
    // the collector's pool is then guaranteed to sit behind whatever is
    // already queued there. It is a legal production setting, and with
    // more collectors the defect merely becomes intermittent.
    cfg.num_igbots_per_zone     = 1;
    cfg.data_file_max_bytes     = shape.file_bytes;
    // Values are well under the file size, so chunking stays out of the
    // way and each file holds whole records only.
    cfg.rest_chunk_threshold    = shape.file_bytes / 8;
    cfg.rest_chunk_bytes        = shape.file_bytes / 40;
    // Small enough that the cache jettisons values, so a scan cannot be
    // served from a fully resident cache by accident.
    cfg.cache_size_limit_bytes  = 40_000_000;
    cfg.zone_overrides          = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 8_000_000_000u64 },
    }.get_map().unwrap();

    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "| SCAN UNDER GARBAGE COLLECTION TEST          |");
    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "{} keys of {} bytes in {} byte files.",
        shape.keys, shape.val_bytes, shape.file_bytes);

    // Collection stays off while the store is built, so that nothing is
    // collected until the test chooses the moment.
    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off
        true,  // wipe
    ));
    thread::sleep(Duration::from_secs(1));

    let payload = |i: usize, round: usize| -> Dat {
        let seed = fmt!("k{:06}r{:02}:", i, round);
        let mut s = String::with_capacity(shape.val_bytes);
        while s.len() < shape.val_bytes {
            s.push_str(&seed);
        }
        s.truncate(shape.val_bytes);
        Dat::Str(s)
    };

    // 1. Populate.
    test!(sync_log::stream(), "Populating {} keys.", shape.keys);
    let t = Instant::now();
    for i in 0..shape.keys {
        res!(db.insert(dat!(fmt!("rec:{:06}", i)), payload(i, 0), user, schms2));
    }
    test!(sync_log::stream(), "Populated in {:?}.", t.elapsed());

    // 2. Supersede every second key, with collection still off. Half of
    //    every file is now garbage -- well past the trigger -- and the
    //    other half is untouched, so a later write to any of those keys
    //    still lands on the file it was written to and can set its
    //    collection going.
    let num_old = ((shape.keys as f64) * OLD_FRAC) as usize;
    test!(sync_log::stream(), "Superseding {} keys with collection off.", num_old);
    let t = Instant::now();
    let mut i = 0;
    while i < shape.keys {
        res!(db.insert(dat!(fmt!("rec:{:06}", i)), payload(i, 1), user, schms2));
        i += 2;
    }
    test!(sync_log::stream(), "Superseded in {:?}.", t.elapsed());
    thread::sleep(Duration::from_secs(2));

    // 3. Quiet baseline. The collector has done nothing and has nothing
    //    queued, so this is the cost of the walk alone.
    let mut worst_quiet = Duration::ZERO;
    for _ in 0..NUM_BASELINE {
        let t = Instant::now();
        let entries = res!(db.scan(&ScanOpts::all(), schms2));
        let dt = t.elapsed();
        if dt > worst_quiet {
            worst_quiet = dt;
        }
        if entries.len() != shape.keys {
            return Err(err!(
                "Quiet scan returned {} entries, expected {}.",
                entries.len(), shape.keys;
                Test, Mismatch));
        }
    }
    test!(sync_log::stream(), "Worst quiet scan latency {:?}.", worst_quiet);

    // 4. Switch collection on and dispatch the backlog. Each of these
    //    writes supersedes a record in a different file, and the file
    //    bot answers each by handing one collection to the zone's
    //    collector. They return as soon as the message is sent, so by
    //    the end of the burst the collector's queue holds the lot.
    res!(ok!(db.updated_api()).activate_gc(true));
    test!(sync_log::stream(), "Collection enabled; dispatching the backlog.");
    let stride = trigger_stride(shape);
    let t = Instant::now();
    let mut triggers = 0;
    let mut i = 1;
    while i < shape.keys {
        res!(db.insert(dat!(fmt!("rec:{:06}", i)), payload(i, 2), user, schms2));
        triggers += 1;
        i += stride;
    }
    test!(sync_log::stream(), "{} triggers dispatched in {:?}.", triggers, t.elapsed());

    // 5. Scan while the collector works through it. This is an operator
    //    refreshing a view.
    let mut worst_loaded = Duration::ZERO;
    let mut failures: Vec<String> = Vec::new();
    for _ in 0..NUM_LOADED {
        let t = Instant::now();
        match db.scan(&ScanOpts::all(), schms2) {
            Err(e) => {
                let dt = t.elapsed();
                if dt > worst_loaded {
                    worst_loaded = dt;
                }
                failures.push(fmt!("scan failed after {:?}: {}", dt, e));
            },
            Ok(entries) => {
                let dt = t.elapsed();
                if dt > worst_loaded {
                    worst_loaded = dt;
                }
                if entries.len() != shape.keys {
                    failures.push(fmt!(
                        "scan during collection returned {} entries, expected {}",
                        entries.len(), shape.keys));
                }
            },
        }
    }
    test!(sync_log::stream(), "Worst scan latency during collection {:?}, {} misbehaved.",
        worst_loaded, failures.len());
    for f in failures.iter().take(5) {
        test!(sync_log::stream(), "  {}", f);
    }

    thread::sleep(Duration::from_secs(5));
    res!(db.shutdown());

    // 6. Judgement.
    if !failures.is_empty() {
        return Err(err!(
            "{} scan(s) issued during garbage collection did not behave: first was '{}'.",
            failures.len(), failures[0];
            Test, Mismatch));
    }
    // A scan served independently of the collector costs what the walk
    // costs. The allowance is loose enough for the disk contention a
    // running collection genuinely causes, and far tighter than a
    // backlog of collections.
    let allowance = worst_quiet * 3 + Duration::from_millis(200);
    if worst_loaded > allowance {
        return Err(err!(
            "Worst scan latency during collection was {:?}, more than the allowance \
            of {:?} derived from the worst quiet latency of {:?}. The scan is \
            waiting on the collector, and on a larger store that wait passes the \
            user request deadline.",
            worst_loaded, allowance, worst_quiet;
            Test, Excessive));
    }

    test!(sync_log::stream(),
        "Scan under garbage collection test passed: worst during collection {:?}, \
        worst quiet {:?}.", worst_loaded, worst_quiet);
    Ok(())
}
