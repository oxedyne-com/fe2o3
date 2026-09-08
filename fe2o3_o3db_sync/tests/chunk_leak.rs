//! Chunked-value overwrite and delete must not leak chunk records.
//!
//! A value whose encoding exceeds the chunk threshold is split: the bytes live in chunk records
//! and the user's key holds a `Tup5u64` bunch key naming them.  Garbage collection is
//! supersession-based -- a record becomes reclaimable only when the same key is written again --
//! so when the chunk records were keyed by a random per-operation ticket, overwriting or deleting
//! such a value superseded the bunch key but left every chunk record under an unreachable key.
//! Nothing rewrote those keys, so nothing flagged them old, so the collector never saw them: every
//! overwrite leaked a whole value's worth of chunk bytes, permanently.  On the gateway this reached
//! ~19 GB.
//!
//! The fix derives the chunk set identifier from the key rather than from a random ticket, so an
//! overwrite writes the new chunks under the same keys as the old and the ordinary supersession
//! path reclaims them; and the delete path reads the current bunch key and tombstones each chunk.
//! Reads are unaffected: a value's chunk keys are reconstructed from the set_id stored in its bunch
//! key, so values written under the old random scheme still read after the switch.
//!
//! These tests force chunking with an explicit, prod-like threshold (not the drifting dev
//! default).  `overwrite_reclaims_chunks` hammers one key -- which is also the concurrent
//! same-key supersession that exposed the accounting race the fix closes -- and insists the
//! footprint stays near the one-value baseline.  `delete_reclaims_chunks` insists a deleted
//! chunked value's chunks are reclaimed, not only its bunch key tombstoned.  `shrink_is_bounded`
//! insists repeated overwrites at a smaller geometry reach a bounded steady state.
//! `old_random_key_value_still_reads` writes a value under a simulated pre-upgrade (random)
//! set_id and insists it still reads, before and after a normal overwrite.  `churn_survives_restart`
//! insists the on-disk accounting a heavy same-key churn leaves behind is self-consistent across a
//! restart.
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
    data::core::RestSchemesInput,
    test::setup,
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

const NOVERWRITES: usize = 20;      // overwrites of the one chunked key
const VALUE_BYTES:  usize = 6_000;  // comfortably over the threshold, several chunks
const SMALL_BYTES:  usize = 3_200;  // still over the threshold, but fewer chunks

/// A byte string of the given size, filled deterministically so each version is distinct on disk.
fn value_of(seed: u8, len: usize) -> Dat {
    let mut v = vec![0u8; len];
    for (i, b) in v.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    Dat::BU32(v)
}

fn chunky_value(seed: u8) -> Dat { value_of(seed, VALUE_BYTES) }

pub fn test_chunk_leak(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(canonical_dir("./test_db_chunk_leak"));

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

    // Force chunking deterministically: an explicit threshold the value exceeds, a small chunk
    // size so a value is several chunks, and a data file small enough that files seal and garbage
    // collection runs -- but large enough to satisfy the threshold-to-file-size ratio check.
    let mut cfg = res!(setup::default_cfg());
    cfg.data_file_max_bytes    = 16_000;
    cfg.rest_chunk_threshold   = 3_000;
    cfg.rest_chunk_bytes       = 1_000;
    cfg.sync_on_write          = true;
    cfg.zone_overrides         = DaticleMap::new();

    res!(overwrite_reclaims_chunks(&db_root, &cfg, &schms_input, schms2, user));
    res!(delete_reclaims_chunks(&db_root, &cfg, &schms_input, schms2, user));
    res!(shrink_is_bounded(&db_root, &cfg, &schms_input, schms2, user));
    res!(old_random_key_value_still_reads(&db_root, &cfg, &schms_input, schms2, user));
    res!(churn_survives_restart(&db_root, &cfg, &schms_input, schms2, user));

    Ok(())
}

/// Overwriting a chunked value many times must leave the data footprint near the one-value
/// baseline, not growing by a value's worth of chunks each time.  Hammering one key is also the
/// concurrent same-key supersession that exposed the accounting race the fix closes: a bounded
/// footprint here means every superseded chunk was flagged old and collected without a
/// mis-accounting abort.  A genuine accounting inconsistency would abort collection in a bot
/// thread and the footprint would grow, tripping the bound below.
fn overwrite_reclaims_chunks(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- chunk leak: overwrite reclaims chunks ---");

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,   // gc on
        true,   // wipe
    ));

    let key = dat!("chunked:churned");

    // First write, then settle, and measure the single-value baseline.
    let (exists, nchunks) = res!(db.insert(key.clone(), chunky_value(0), user, schms2));
    if nchunks < 2 {
        return Err(err!(
            "The test value was not chunked ({} chunk(s)); the threshold or value size is wrong, \
            so the test would not exercise the leak.", nchunks;
            Test, Invalid, Configuration));
    }
    if exists {
        return Err(err!("The churned key unexpectedly already existed on a fresh store.";
            Test, Invalid, Data));
    }
    thread::sleep(Duration::from_secs(2));
    let baseline = res!(zone_data_bytes(db_root));
    test!(sync_log::stream(),
        "chunk leak: one {}-byte value is {} chunks, {} bytes of data files at baseline.",
        VALUE_BYTES, nchunks, baseline);

    // Overwrite the same key many times.  Each overwrite writes new chunk bytes under the same
    // key-derived chunk keys; the prior value's chunks must be reclaimed rather than left behind.
    for i in 1..=NOVERWRITES {
        res!(db.insert(key.clone(), chunky_value(i as u8), user, schms2));
    }
    thread::sleep(Duration::from_secs(3));
    let after = res!(zone_data_bytes(db_root));

    // The value must still read back as the last one written.
    match res!(db.get(&key, schms2)) {
        Some((got, _)) => if got != chunky_value(NOVERWRITES as u8) {
            return Err(err!(
                "After {} overwrites the chunked value came back changed.", NOVERWRITES;
                Test, Invalid, Data));
        },
        None => return Err(err!(
            "After {} overwrites the chunked value cannot be read back.", NOVERWRITES;
            Test, Missing, Data)),
    }

    test!(sync_log::stream(),
        "chunk leak: after {} overwrites, {} bytes of data files (baseline {}, ratio {:.1}x).",
        NOVERWRITES, after, baseline, after as f64 / baseline as f64);

    // Without reclamation the footprint grows by a value's worth of chunks per overwrite -- about
    // NOVERWRITES times the baseline.  With it, the collector reclaims the superseded chunks and
    // only the current value survives, a small multiple of the baseline.  The bound sits well
    // below the leak and above the reclaimed steady state.
    let bound = baseline.saturating_mul(6);
    if after >= bound {
        return Err(err!(
            "Chunk leak on overwrite: {} bytes of data files after {} overwrites of a {}-chunk \
            value, against a one-value baseline of {} bytes. The superseded chunk records are not \
            being reclaimed -- the footprint is growing with the overwrite count instead of \
            staying bounded.", after, NOVERWRITES, nchunks, baseline;
            Test, Mismatch, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- chunk leak: overwrite reclaims chunks : passed ---");
    Ok(())
}

/// Deleting a chunked value must reclaim its chunk records, not only tombstone the bunch key.
fn delete_reclaims_chunks(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- chunk leak: delete reclaims chunks ---");

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,   // gc on
        true,   // wipe
    ));

    // Enough chunked values under distinct keys that the reclaimed chunk bytes dominate the fixed
    // per-key tombstone residual, so the drop on delete is unambiguous.
    let nkeys = 16usize;
    for k in 0..nkeys {
        let (_, nchunks) = res!(db.insert(dat!(fmt!("del:{:03}", k)), chunky_value(k as u8), user, schms2));
        if nchunks < 2 {
            return Err(err!(
                "The delete-case value was not chunked ({} chunk(s)).", nchunks;
                Test, Invalid, Configuration));
        }
    }
    thread::sleep(Duration::from_secs(3));
    let with_values = res!(zone_data_bytes(db_root));

    // Delete them all.
    for k in 0..nkeys {
        let existed = res!(db.delete(&dat!(fmt!("del:{:03}", k)), user, schms2));
        if !existed {
            return Err(err!("Delete reported the chunked key {} absent.", k; Test, Missing, Data));
        }
    }
    thread::sleep(Duration::from_secs(4));
    let after_delete = res!(zone_data_bytes(db_root));

    // Every deleted key reads back as absent.
    for k in 0..nkeys {
        if res!(db.get(&dat!(fmt!("del:{:03}", k)), schms2)).is_some() {
            return Err(err!("Deleted chunked key {} still reads back a value.", k;
                Test, Invalid, Data));
        }
    }

    test!(sync_log::stream(),
        "chunk leak: {} bytes of data files with {} chunked values present, {} after deleting all.",
        with_values, nkeys, after_delete);

    // Deletion that reclaims chunks leaves only small tombstones plus whatever sits in still-live
    // files, so the footprint after deleting every value must fall well below the footprint with
    // them present.  Without reclamation the chunk bytes remain and the footprint barely moves.
    if after_delete * 2 >= with_values {
        return Err(err!(
            "Chunk leak on delete: deleting {} chunked values took the data footprint from {} \
            bytes only to {} bytes. The chunk records of a deleted value are not being reclaimed.",
            nkeys, with_values, after_delete;
            Test, Mismatch, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- chunk leak: delete reclaims chunks : passed ---");
    Ok(())
}

/// Repeatedly overwriting a chunked value with a smaller (still chunked) one must reach a bounded
/// steady state, not grow with the overwrite count.
///
/// A chunk key encodes the value's geometry (its length and part count) as well as the key-derived
/// set_id, so an overwrite that *changes* the geometry -- a grow or a shrink across a chunk
/// boundary -- writes its chunks under different keys and does not supersede the prior geometry's
/// chunks: that one prior value leaks and is left for the offline orphan sweep.  But overwrites at
/// the *same* geometry share chunk keys and reclaim normally, so a run of same-size overwrites is
/// bounded regardless of length.  This test shrinks once and then holds the smaller size, and
/// insists the steady state after many small overwrites is no worse than after a few.
fn shrink_is_bounded(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- chunk leak: shrink is bounded ---");

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        true,
    ));

    let key = dat!("chunked:shrinking");

    // A large chunked value, then shrink to a smaller (still chunked) one.
    let (_, big_chunks) = res!(db.insert(key.clone(), value_of(0, VALUE_BYTES), user, schms2));
    let (_, small_chunks) = res!(db.insert(key.clone(), value_of(1, SMALL_BYTES), user, schms2));
    if big_chunks < 2 || small_chunks < 2 || small_chunks >= big_chunks {
        return Err(err!(
            "Shrink case geometry is wrong: big={} chunks, small={} chunks; the small value must \
            still be chunked and have fewer chunks than the big one.", big_chunks, small_chunks;
            Test, Invalid, Configuration));
    }

    // Warm up to a steady state at the smaller geometry, measure, then run three times as many
    // more overwrites and measure again.  Same-size overwrites share chunk keys, so the collector
    // reclaims each prior small value: past the warm-up the footprint must not grow with the
    // count.  Comparing two post-warm-up points (rather than an early point against a late one)
    // isolates a genuine linear leak from the one-off ramp to steady state and the single big
    // value leaked at the shrink transition.
    for i in 2..=11 {
        res!(db.insert(key.clone(), value_of(i as u8, SMALL_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_secs(3));
    let after_warm = res!(zone_data_bytes(db_root));

    for i in 12..=41 {
        res!(db.insert(key.clone(), value_of(i as u8, SMALL_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_secs(3));
    let after_more = res!(zone_data_bytes(db_root));

    match res!(db.get(&key, schms2)) {
        Some((got, _)) => if got != value_of(41, SMALL_BYTES) {
            return Err(err!("After shrinking overwrites the value came back changed."; Test, Invalid, Data));
        },
        None => return Err(err!("After shrinking overwrites the value cannot be read back."; Test, Missing, Data)),
    }

    test!(sync_log::stream(),
        "chunk leak: shrink steady state {} bytes after 10 small overwrites, {} bytes after 40.",
        after_warm, after_more);

    // Thirty more overwrites must not have grown the footprint materially: reclaim at the smaller
    // geometry is keeping pace.  A generous factor absorbs sealing and collection timing jitter
    // while still catching a genuine linear leak (thirty un-reclaimed small values would be many
    // times larger).
    if after_more > after_warm * 3 / 2 {
        return Err(err!(
            "Chunk leak on shrink: the footprint grew from {} bytes after warm-up to {} bytes \
            after thirty more overwrites, so same-size overwrites are not reclaiming their \
            predecessors.", after_warm, after_more;
            Test, Mismatch, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- chunk leak: shrink is bounded : passed ---");
    Ok(())
}

/// A chunked value written under the old scheme -- a random per-operation set_id -- must still read
/// after the switch to key-derived identifiers, because the reader reconstructs the chunk keys from
/// the set_id stored in the bunch key, never from a recomputed one.  This forces a random set_id to
/// stand in for a pre-upgrade value, reads it, then overwrites it the ordinary way and reads again:
/// the value is correct throughout.  (The first ordinary overwrite does not reclaim the old
/// random-keyed chunks -- a different key scheme -- which is the documented migration cost left for
/// the offline sweep; correctness of reads, not reclamation, is what this asserts.)
fn old_random_key_value_still_reads(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- chunk leak: old random-key value still reads ---");

    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        true,
    ));

    let key = dat!("chunked:legacy");
    let legacy = value_of(7, VALUE_BYTES);

    // Write it as an earlier build did: chunks addressed by a random set_id, unrelated to the key.
    // A key-derived write would use `chunk_set_id(key)`, so a random one guarantees the stored
    // set_id differs from what the current scheme would compute -- exactly the pre-upgrade case.
    let mut set_id_bytes = [0u8; 8];
    Rand::fill_u8(&mut set_id_bytes);
    let random_set_id = u64::from_be_bytes(set_id_bytes);

    let resp = db.api().responder();
    let nchunks = res!(db.api().store_dat_using_responder_forcing_set_id(
        key.clone(),
        legacy.clone(),
        user,
        schms2,
        resp.clone(),
        random_set_id,
    ));
    if nchunks < 2 {
        return Err(err!(
            "The legacy value was not chunked ({} chunk(s)).", nchunks; Test, Invalid, Configuration));
    }
    // Drain the write acknowledgements so the value is settled before it is read.
    let _ = resp.recv_number(nchunks, constant::USER_REQUEST_WAIT);
    thread::sleep(Duration::from_secs(1));

    // It must read back correctly even though its set_id is not the key-derived one.
    match res!(db.get(&key, schms2)) {
        Some((got, _)) => if got != legacy {
            return Err(err!(
                "A chunked value written under a random (pre-upgrade) set_id did not read back \
                unchanged, so the reader is not reconstructing chunk keys from the stored set_id.";
                Test, Invalid, Data));
        },
        None => return Err(err!(
            "A chunked value written under a random (pre-upgrade) set_id cannot be read at all.";
            Test, Missing, Data)),
    }

    // Now overwrite it the ordinary (key-derived) way and read again: the new value is correct, and
    // the old value remained readable right up to the overwrite.
    let fresh = value_of(8, VALUE_BYTES);
    res!(db.insert(key.clone(), fresh.clone(), user, schms2));
    thread::sleep(Duration::from_secs(1));
    match res!(db.get(&key, schms2)) {
        Some((got, _)) => if got != fresh {
            return Err(err!(
                "After overwriting a legacy random-keyed value the new value did not read back.";
                Test, Invalid, Data));
        },
        None => return Err(err!(
            "After overwriting a legacy random-keyed value the key cannot be read.";
            Test, Missing, Data)),
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- chunk leak: old random-key value still reads : passed ---");
    Ok(())
}

/// A heavy same-key churn -- the concurrent supersession that exposed the accounting race -- must
/// leave the store self-consistent across a restart.  A restart rebuilds every file state from what
/// is physically on disk, so if the churn had left the accounting inconsistent (records flagged old
/// twice, records superseded but never inserted, a data file out of step with its state) the
/// rebuilt store would drop or corrupt the value or fail to start.  Reading the value back
/// unchanged, with a bounded footprint, after a restart is the black-box proof that the
/// reconciliation kept the on-disk accounting exact.
fn churn_survives_restart(
    db_root:     &PathBuf,
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- chunk leak: churn survives restart ---");

    let key = dat!("chunked:restart");
    let last = 40u8;

    {
        let db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            true,
            true,
        ));
        // Rapid, un-spaced overwrites of one key: the burst that races record insertion against
        // sibling-chunk supersession and drives the reconciliation and the drain-gated collector.
        for i in 0..=last {
            res!(db.insert(key.clone(), chunky_value(i), user, schms2));
        }
        thread::sleep(Duration::from_secs(3));
        res!(db.shutdown());
    }
    thread::sleep(Duration::from_secs(1));

    // Restart over what the churn left on disk.
    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        false,      // do not wipe: read what the churn left
    ));

    match res!(db.get(&key, schms2)) {
        Some((got, _)) => if got != chunky_value(last) {
            return Err(err!(
                "After a heavy same-key churn and a restart, the value came back changed: the \
                on-disk accounting the churn left was not self-consistent.";
                Test, Invalid, Data));
        },
        None => return Err(err!(
            "After a heavy same-key churn and a restart, the value is gone.";
            Test, Missing, Data)),
    }

    let after = res!(zone_data_bytes(db_root));
    let one_value = res!(single_value_baseline(cfg, schms_input, schms2, user));
    test!(sync_log::stream(),
        "chunk leak: after churn + restart, {} bytes of data files (one value ~{} bytes).",
        after, one_value);
    if after >= one_value.saturating_mul(8) {
        return Err(err!(
            "After a heavy same-key churn and a restart, the footprint is {} bytes against a \
            one-value baseline of ~{} bytes: the churn leaked rather than reclaiming.",
            after, one_value;
            Test, Mismatch, Data));
    }

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    test!(sync_log::stream(), "+--- chunk leak: churn survives restart : passed ---");
    Ok(())
}

/// The settled on-disk size of a single chunked value, measured in a throwaway store, for use as a
/// baseline the churn footprint is judged against.
fn single_value_baseline(
    cfg:         &oxedyne_fe2o3_o3db_sync::base::cfg::OzoneConfig,
    schms_input: &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<u64>
{
    let base_root = res!(canonical_dir("./test_db_chunk_leak_baseline"));
    let db = res!(setup::start_db(
        base_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true,
        true,
    ));
    res!(db.insert(dat!("chunked:one"), chunky_value(0), user, schms2));
    thread::sleep(Duration::from_secs(2));
    let bytes = res!(zone_data_bytes(&base_root));
    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));
    Ok(bytes)
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
