//! The online orphan sweep must reclaim orphaned chunk records, leave every live value untouched,
//! and stay safe against a concurrent writer.
//!
//! A chunked value's chunk records are keyed by its geometry, so an overwrite that changes the
//! geometry -- or, for a value written under a pre-fix random ticket, any overwrite -- leaves the old
//! chunk records under keys no live bunch key names.  Supersession-based collection never reaches
//! them: they are orphans.  `sweep_orphans` tombstones each orphaned chunk key so the running
//! collector reclaims its bytes, guarding against concurrent writers with an epoch on the record
//! timestamp.
//!
//! `sweep_reclaims_orphans` seeds a store with random-ticket orphans, geometry-change orphans, dead
//! tombstones, a live chunked value and live unchunked values, shows the orphan bytes do not
//! self-reclaim, sweeps, and insists the footprint drops sharply while every live value reads back
//! byte-identical, the tombstones are left untouched, and the store is self-consistent across a
//! restart.  `concurrency_is_safe` runs a writer creating fresh chunked values during the sweep and
//! insists the epoch guard engaged (`skipped_recent > 0`) and every concurrent value survived
//! byte-identical -- the proof that a value written during the sweep is never mistaken for an orphan.
//!
//! These tests force chunking with an explicit, prod-like threshold (not the drifting dev default),
//! exactly as `chunk_leak.rs` does.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::constant,
    comm::response::Wait,
    data::core::RestSchemesInput,
    sweep,
    test::setup,
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            AtomicUsize,
            Ordering,
        },
    },
    thread,
    time::Duration,
};

const VALUE_BYTES:  usize = 6_000;  // comfortably over the threshold, several chunks
const SMALL_BYTES:  usize = 3_200;  // still over the threshold, but fewer chunks (a geometry change)
const TINY_BYTES:   usize = 200;    // under the threshold: an unchunked value

/// A byte string of the given size, filled deterministically so each version is distinct on disk.
fn value_of(seed: u8, len: usize) -> Dat {
    let mut v = vec![0u8; len];
    for (i, b) in v.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    Dat::BU32(v)
}

fn chunky_value(seed: u8) -> Dat { value_of(seed, VALUE_BYTES) }

fn scan_wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(60),
        check_interval: constant::CHECK_INTERVAL,
    }
}

pub fn test_sweep(_filter: &'static str) -> Outcome<()> {

    let db_root   = res!(canonical_dir("./test_db_sweep"));
    let db_root_c = res!(canonical_dir("./test_db_sweep_concurrent"));

    let mut enckey = [0u8; 32];
    Rand::fill_u8(&mut enckey);
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
    let crc32 = ChecksumScheme::new_crc32();
    let schms2: RestSchemesOverride<EncryptionScheme, HashScheme> =
        RestSchemesOverride::default().set_encrypter(Override::Default(aes_gcm.clone()));
    let schms2 = Some(&schms2);
    let user = setup::Uid::default();
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );

    // Force chunking deterministically, as chunk_leak.rs does: an explicit threshold the value
    // exceeds, a small chunk size so a value is several chunks, and a data file small enough that
    // files seal and collection runs.
    let mut cfg = res!(setup::default_cfg());
    cfg.data_file_max_bytes    = 16_000;
    cfg.rest_chunk_threshold   = 3_000;
    cfg.rest_chunk_bytes       = 1_000;
    cfg.sync_on_write          = true;
    cfg.zone_overrides         = DaticleMap::new();

    res!(sweep_reclaims_orphans(&db_root, &cfg, &schms_input, schms2, &aes_gcm, user));
    res!(concurrency_is_safe(&db_root_c, &cfg, &schms_input, schms2, &aes_gcm, user));

    Ok(())
}

/// The sweep must retire every orphaned chunk set, reclaim its bytes, leave every live value
/// byte-identical and every dead tombstone untouched, and leave the store self-consistent across a
/// restart.  The teeth: garbage collection runs the whole time, yet the orphan bytes do not
/// self-reclaim -- only the sweep reclaims them.
fn sweep_reclaims_orphans(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    _aes_gcm:    &EncryptionScheme,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- sweep: reclaims orphans, keeps live values ---");

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,   // gc on -- the sweep relies on the running collector
        true,   // wipe
    ));

    let n_live_small = 6usize;
    let n_geo        = 16usize;
    let n_rnd        = 16usize;
    let n_tomb       = 6usize;

    // --- Live unchunked values: must survive byte-identical. ---
    for k in 0..n_live_small {
        res!(db.insert(dat!(fmt!("live:small:{:03}", k)), value_of(k as u8, TINY_BYTES), user, schms2));
    }

    // --- A live chunked value under a stable key: its chunks must never be retired. ---
    let (_, live_chunks) = res!(db.insert(dat!("live:chunked"), chunky_value(42), user, schms2));
    if live_chunks < 2 {
        return Err(err!("The live chunked value was not chunked ({} chunk(s)).", live_chunks;
            Test, Invalid, Configuration));
    }

    // --- Dead tombstones: chunked values deleted and never reused. NOT the sweep's job. ---
    for k in 0..n_tomb {
        res!(db.insert(dat!(fmt!("tomb:{:03}", k)), chunky_value(k as u8), user, schms2));
    }
    thread::sleep(Duration::from_secs(2));
    for k in 0..n_tomb {
        if !res!(db.delete(&dat!(fmt!("tomb:{:03}", k)), user, schms2)) {
            return Err(err!("Delete reported tombstone key {} absent.", k; Test, Missing, Data));
        }
    }

    // --- Geometry-change orphans: a big chunked value overwritten by a smaller chunked one. The
    //     prior geometry's chunk set is orphaned; the current small value stays live. ---
    for k in 0..n_geo {
        let (_, big) = res!(db.insert(dat!(fmt!("geo:{:03}", k)), value_of(k as u8, VALUE_BYTES), user, schms2));
        let (_, small) = res!(db.insert(dat!(fmt!("geo:{:03}", k)), value_of(k as u8, SMALL_BYTES), user, schms2));
        if big < 2 || small < 2 || small >= big {
            return Err(err!(
                "Geometry-orphan case geometry wrong: big={} small={} chunks.", big, small;
                Test, Invalid, Configuration));
        }
    }

    // --- Random-ticket orphans: a chunked value written under a random (pre-fix) set_id, then
    //     overwritten by a tiny unchunked value. The whole random-keyed chunk set is orphaned and
    //     the current value is a tiny live record. ---
    for k in 0..n_rnd {
        let key = dat!(fmt!("rnd:{:03}", k));
        let mut set_id_bytes = [0u8; 8];
        Rand::fill_u8(&mut set_id_bytes);
        let random_set_id = u64::from_be_bytes(set_id_bytes);
        let resp = db.api().responder();
        let nchunks = res!(db.api().store_dat_using_responder_forcing_set_id(
            key.clone(),
            value_of(k as u8, VALUE_BYTES),
            user,
            schms2,
            resp.clone(),
            random_set_id,
        ));
        if nchunks < 2 {
            return Err(err!("The random-ticket value was not chunked ({} chunk(s)).", nchunks;
                Test, Invalid, Configuration));
        }
        let _ = resp.recv_number(nchunks, constant::USER_REQUEST_WAIT);
        // Overwrite with a tiny unchunked value: supersedes the bunch key, orphans every chunk.
        res!(db.insert(key.clone(), value_of((k as u8).wrapping_add(1), TINY_BYTES), user, schms2));
    }

    // Let the collector fully settle: it reclaims every legitimately superseded record (old bunch
    // keys, same-geometry overwrites), then has nothing left to do, because an orphaned chunk set is
    // exactly what supersession can never reach. The settled footprint therefore still holds the
    // orphan bytes -- the teeth below confirm the sweep then finds them. Settling also ages the
    // orphans well past the epoch skew.
    let before = res!(settle_footprint(db_root));
    test!(sync_log::stream(), "sweep: {} data bytes at the settled pre-sweep baseline.", before);

    // Run the sweep against the live store.
    let report = res!(sweep::sweep_orphans(
        db.api(),
        user,
        schms2,
        scan_wait(),
        Duration::from_secs(2),
    ));
    test!(sync_log::stream(), "sweep: {}", report.summary().replace('\n', " | "));

    if report.orphans_found == 0 {
        return Err(err!(
            "The sweep found no orphans, but random-ticket and geometry-change orphans were seeded: \
            the orphan detection is not working.";
            Test, Invalid, Data));
    }
    if report.orphans_retired != report.orphans_found {
        return Err(err!(
            "The sweep retired {} of {} orphans found: a tombstone write did not land.",
            report.orphans_retired, report.orphans_found;
            Test, Mismatch, Data));
    }

    // Let the collector settle again, then the footprint must have dropped sharply: the orphan
    // chunk bytes, which GC could not reach before the sweep, have been reclaimed.
    let after = res!(settle_footprint(db_root));
    test!(sync_log::stream(),
        "sweep: data bytes {} before, {} after the sweep (ratio {:.2}).",
        before, after, after as f64 / before as f64);
    if after.saturating_mul(3) >= before.saturating_mul(2) {
        return Err(err!(
            "Sweep did not reclaim: {} bytes before, {} after, but the orphan chunk bytes should \
            have fallen away.", before, after;
            Test, Mismatch, Data));
    }

    // Every live value reads back byte-identical -- chunked and unchunked.
    res!(check_live_values(&db, n_live_small, n_geo, n_rnd, schms2));

    // Dead tombstones are untouched: a deleted key still reads absent.
    for k in 0..n_tomb {
        if res!(db.get(&dat!(fmt!("tomb:{:03}", k)), schms2)).is_some() {
            return Err(err!("A dead tombstone key {} reads back a value after the sweep.", k;
                Test, Invalid, Data));
        }
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_secs(1));

    // RESTART over what the sweep left on disk: every live value must read back unchanged and the
    // footprint must not have regrown, proving the retirement left the on-disk accounting exact.
    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        false,  // do not wipe: read what the sweep left
    ));
    res!(check_live_values(&db, n_live_small, n_geo, n_rnd, schms2));
    for k in 0..n_tomb {
        if res!(db.get(&dat!(fmt!("tomb:{:03}", k)), schms2)).is_some() {
            return Err(err!("After restart, dead tombstone key {} reads back a value.", k;
                Test, Invalid, Data));
        }
    }
    let after_restart = res!(zone_data_bytes(db_root));
    test!(sync_log::stream(), "sweep: {} data bytes after restart.", after_restart);
    if after_restart.saturating_mul(3) >= before.saturating_mul(2) {
        return Err(err!(
            "After a restart the footprint is {} bytes against {} before the sweep: the reclaim \
            did not survive the restart.", after_restart, before;
            Test, Mismatch, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- sweep: reclaims orphans, keeps live values : passed ---");
    Ok(())
}

/// Reads back every live value and checks it is byte-identical to what was written.
fn check_live_values(
    db:          &oxedyne_fe2o3_o3db_sync::db::O3db<{ setup::UID_LEN }, setup::Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    n_live_small: usize,
    n_geo:        usize,
    n_rnd:        usize,
    schms2:       Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
)
    -> Outcome<()>
{
    for k in 0..n_live_small {
        let want = value_of(k as u8, TINY_BYTES);
        match res!(db.get(&dat!(fmt!("live:small:{:03}", k)), schms2)) {
            Some((got, _)) => if got != want {
                return Err(err!("Live unchunked value {} came back changed.", k; Test, Invalid, Data));
            },
            None => return Err(err!("Live unchunked value {} is gone.", k; Test, Missing, Data)),
        }
    }
    match res!(db.get(&dat!("live:chunked"), schms2)) {
        Some((got, _)) => if got != chunky_value(42) {
            return Err(err!("The live chunked value came back changed."; Test, Invalid, Data));
        },
        None => return Err(err!("The live chunked value is gone."; Test, Missing, Data)),
    }
    for k in 0..n_geo {
        let want = value_of(k as u8, SMALL_BYTES);
        match res!(db.get(&dat!(fmt!("geo:{:03}", k)), schms2)) {
            Some((got, _)) => if got != want {
                return Err(err!("Geometry-orphan current value {} came back changed.", k; Test, Invalid, Data));
            },
            None => return Err(err!("Geometry-orphan current value {} is gone.", k; Test, Missing, Data)),
        }
    }
    for k in 0..n_rnd {
        let want = value_of((k as u8).wrapping_add(1), TINY_BYTES);
        match res!(db.get(&dat!(fmt!("rnd:{:03}", k)), schms2)) {
            Some((got, _)) => if got != want {
                return Err(err!("Random-ticket current value {} came back changed.", k; Test, Invalid, Data));
            },
            None => return Err(err!("Random-ticket current value {} is gone.", k; Test, Missing, Data)),
        }
    }
    Ok(())
}

/// A writer creating fresh chunked values during the sweep proves the online claim: the epoch guard
/// must skip values written after the sweep started (`skipped_recent > 0`), and every concurrently
/// written value must survive byte-identical.  A version of the sweep that retired a fresh value's
/// chunks would be the catastrophic bug this rules out.
fn concurrency_is_safe(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    aes_gcm:     &EncryptionScheme,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- sweep: concurrency is safe (online) ---");

    let db = Arc::new(res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        true,
    )));

    const EPOCH_SKEW_SECS: u64 = 3;

    // Seed geometry-change orphans and age them well past the skew: these are the genuine, settled
    // orphans the sweep must retire.
    let n_geo = 12usize;
    for k in 0..n_geo {
        res!(db.insert(dat!(fmt!("cgeo:{:03}", k)), value_of(k as u8, VALUE_BYTES), user, schms2));
        res!(db.insert(dat!(fmt!("cgeo:{:03}", k)), value_of(k as u8, SMALL_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_secs(EPOCH_SKEW_SECS + 3)); // age past the skew

    // A writer thread hammering fresh chunked values under new keys, with its own schemes so it
    // borrows nothing from this frame.  These are values written concurrently with the sweep; none
    // of them may be retired.
    let stop     = Arc::new(AtomicBool::new(false));
    let written  = Arc::new(AtomicUsize::new(0));
    let db_w     = Arc::clone(&db);
    let stop_w   = Arc::clone(&stop);
    let written_w = Arc::clone(&written);
    let aes_w    = aes_gcm.clone();
    let writer = thread::spawn(move || -> Outcome<()> {
        let schms_owned: RestSchemesOverride<EncryptionScheme, HashScheme> =
            RestSchemesOverride::default().set_encrypter(Override::Default(aes_w));
        let schms_w = Some(&schms_owned);
        let mut i = 0u32;
        while !stop_w.load(Ordering::Relaxed) {
            let key = dat!(fmt!("fresh:{:06}", i));
            res!(db_w.insert(key, chunky_value((i % 200) as u8), user, schms_w));
            written_w.fetch_add(1, Ordering::Relaxed);
            i += 1;
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    });

    // Let the writer get going, then -- immediately before the sweep -- overwrite some chunked
    // values at a new geometry.  Their OLD chunk sets become genuine orphans, but they are younger
    // than the skew, so the epoch guard must DEFER them (count them skipped_recent, not retire them):
    // this is exactly the hazard of a value overwritten just as the sweep starts, and it makes the
    // guard fire deterministically rather than relying on a scan-timing window.
    let n_recent = 6usize;
    for k in 0..n_recent {
        res!(db.insert(dat!(fmt!("recent:{:03}", k)), value_of(k as u8, VALUE_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_millis(200));
    for k in 0..n_recent {
        res!(db.insert(dat!(fmt!("recent:{:03}", k)), value_of(k as u8, SMALL_BYTES), user, schms2));
    }

    let report = res!(sweep::sweep_orphans(
        db.api(),
        user,
        schms2,
        scan_wait(),
        Duration::from_secs(EPOCH_SKEW_SECS),
    ));

    // Keep writing a touch longer, then stop and join.
    thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    match writer.join() {
        Ok(r) => res!(r),
        Err(_) => return Err(err!("The concurrent writer thread panicked."; Test, Bug)),
    }

    test!(sync_log::stream(), "sweep/concurrent: {}", report.summary().replace('\n', " | "));

    // The online-safety proof: the epoch guard deferred the recently overwritten values' old chunks
    // rather than retiring them.  Without the guard these recent orphans would have been retired.
    if report.skipped_recent == 0 {
        return Err(err!(
            "The epoch guard never engaged (skipped_recent == 0): a value overwritten just before \
            the sweep had its old chunks treated as retirable. The online safety guard is not being \
            applied.";
            Test, Invalid, Data));
    }

    // The aged seeded orphans should have been retired.
    if report.orphans_found == 0 {
        return Err(err!("The concurrency sweep found no orphans, but aged orphans were seeded.";
            Test, Invalid, Data));
    }

    // The recently overwritten values read back as their current (small) value, byte-identical: the
    // guard deferred their old chunks without touching the live value.
    for k in 0..n_recent {
        let want = value_of(k as u8, SMALL_BYTES);
        match res!(db.get(&dat!(fmt!("recent:{:03}", k)), schms2)) {
            Some((got, _)) => if got != want {
                return Err(err!("A value overwritten just before the sweep (recent:{:03}) came back \
                    changed.", k; Test, Invalid, Data));
            },
            None => return Err(err!("A value overwritten just before the sweep (recent:{:03}) is gone.",
                k; Test, Missing, Data)),
        }
    }

    // Every concurrently written value survives byte-identical -- none of its chunks were retired.
    let total = written.load(Ordering::Relaxed);
    if total == 0 {
        return Err(err!("The concurrent writer wrote nothing."; Test, Invalid, Data));
    }
    for i in 0..(total as u32) {
        let want = chunky_value((i % 200) as u8);
        match res!(db.get(&dat!(fmt!("fresh:{:06}", i)), schms2)) {
            Some((got, _)) => if got != want {
                return Err(err!(
                    "A value written concurrently with the sweep (fresh:{:06}) came back changed: \
                    the sweep retired a live value's chunks.", i;
                    Test, Invalid, Data));
            },
            None => return Err(err!(
                "A value written concurrently with the sweep (fresh:{:06}) is gone: the sweep \
                retired a live value's chunks.", i;
                Test, Missing, Data)),
        }
    }
    test!(sync_log::stream(),
        "sweep/concurrent: {} fresh values all intact, {} skipped by the epoch guard.",
        total, report.skipped_recent);

    // Recover the store for a clean shutdown (the writer's Arc clone is dropped after the join).
    match Arc::try_unwrap(db) {
        Ok(db) => res!(db.shutdown()),
        Err(_) => return Err(err!(
            "Could not reclaim sole ownership of the store to shut it down.";
            Test, Bug)),
    }
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- sweep: concurrency is safe (online) : passed ---");
    Ok(())
}

/// The caller's own settle: waits for the asynchronous collector to engage and then finish, and
/// returns the quiesced data-file footprint.  `sweep_orphans` is sleep-free -- it returns the moment
/// its tombstones are acknowledged, before the collector has reclaimed anything -- so a footprint
/// read immediately after it would be the un-reclaimed figure.  An initial wait lets collection
/// start (otherwise the footprint would read high and stable and the loop would conclude "settled"
/// before a single byte was reclaimed), then the loop polls until two reads two seconds apart differ
/// by under 3%, i.e. collection has stopped shrinking the store.
fn settle_footprint(db_root: &Path) -> Outcome<u64> {
    thread::sleep(Duration::from_secs(4));
    let mut prev = res!(zone_data_bytes(db_root));
    for _ in 0..20 {
        thread::sleep(Duration::from_secs(2));
        let now = res!(zone_data_bytes(db_root));
        // The footprint only falls as GC runs; stable means it fell by less than 3% this step.
        if now >= prev.saturating_mul(97) / 100 {
            return Ok(now);
        }
        prev = now;
    }
    Ok(prev)
}

fn zone_data_bytes(db_root: &Path) -> Outcome<u64> {
    let mut total = 0u64;
    res!(walk_data_files(db_root, &mut total));
    Ok(total)
}

fn walk_data_files(dir: &Path, total: &mut u64) -> Outcome<()> {
    for entry in res!(fs::read_dir(dir)) {
        let entry = res!(entry);
        let path = entry.path();
        if path.is_dir() {
            res!(walk_data_files(&path, total));
        } else if path.extension().map(|e| e == constant::DATA_FILE_EXT).unwrap_or(false) {
            let meta = res!(entry.metadata());
            *total += meta.len();
        }
    }
    Ok(())
}

/// Creates the directory if it does not exist.
fn canonical_dir(p: &str) -> Outcome<PathBuf> {
    match Path::new(p).canonicalize() {
        Ok(path) => Ok(path),
        Err(_) => {
            res!(fs::create_dir_all(p));
            match Path::new(p).canonicalize() {
                Ok(path) => Ok(path),
                Err(e) => Err(err!(e, "Cannot canonicalise {:?}.", p; IO, Path)),
            }
        },
    }
}

#[test]
fn main() -> Outcome<()> {
    log_set_level!("trace");
    let outcome = test_sweep("all");
    log_finish_wait!();
    outcome
}
