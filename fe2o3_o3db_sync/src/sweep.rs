//! In-place, online orphan sweep for an Ozone store.
//!
//! Garbage collection here is supersession-based: a record's bytes become reclaimable only when a
//! newer record is written at the same stored key.  A chunked value's chunk-data records are keyed
//! by the value's geometry, so an overwrite that changes the geometry -- or, on a pre-fix build, any
//! overwrite at all, because the chunk set was keyed by a fresh random ticket -- leaves the old
//! chunk records under keys no live bunch key names.  Nothing rewrites those keys, so nothing
//! flags them old, so the collector never reaches them: they are orphans, and on the gateway they
//! reached ~19 GB.  [`sweep_orphans`] reclaims them in place, against a live store, without downtime
//! or a second store.
//!
//! The sweep is the in-place counterpart of [`crate::migrate`]: the migration copies the live set
//! into a fresh store and achieves zero residue (no orphans, no tombstones, no empty file states)
//! but needs the store stopped and twice the disk transiently; the sweep runs online and reclaims
//! the large chunk orphans, at the cost of leaving a small dead tombstone per orphan behind (the
//! residue §"What it does not do" names).  Operators run the sweep routinely and a migration rarely.
//!
//! # How it stays correct against concurrent writers
//!
//! A record's `Meta.time` is stamped once per write and cloned identically onto the bunch key and
//! every chunk of a chunked value (`api::OzoneApi::prepare_write`), so a chunked value's bunch key
//! and all its chunks share one timestamp.  The sweep stamps `T0` before it scans and retires a
//! chunk-data key only when it is both (a) absent from the live set reconstructed from every live
//! bunch key and (b) stamped strictly before `T0 - epoch_skew`.  A value written after `T0` has
//! chunk timestamps at or after `T0`, so it is never retired even if its bunch key was written too
//! late for the live scan to see it; a value present and still live before `T0` has its chunks in
//! the live set and is kept.  A conservative `epoch_skew` (a few seconds) only defers reclaiming a
//! genuine orphan to a later sweep; it never retires a live chunk.  `epoch_skew` of zero degenerates
//! to the quiesced/offline case, correct only when nothing is writing.
//!
//! # What it does not do
//!
//! Retiring a large chunk orphan converts it into a small dead tombstone (a deleted-kind marker
//! record at the chunk key).  The sweep therefore turns gigabytes of chunk orphans into a few
//! mebibytes of tombstones -- a >99.9% reclaim -- but does not remove the key.  Clearing the
//! tombstones is left to the offline migration, which drops them by never copying them.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    prelude::*,
    comm::response::Wait,
};

use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_iop_db::api::{
    RestSchemesOverride,
    ScanOpts,
};

use std::{
    collections::HashSet,
    fs,
    path::Path,
    time::Duration,
};


/// What a sweep found and retired.
///
/// `bytes_before` is the store's data-file footprint at the instant the sweep started, measured over
/// the store's root directory, so it is meaningful only when the store keeps its data under that root
/// (no absolute zone override pointing elsewhere), which is the single-directory arrangement this
/// tool is built for.  There is deliberately no `bytes_after`: collection is asynchronous and the
/// sweep returns as soon as every tombstone write is acknowledged, so the reclaimed footprint is only
/// visible once the collector has settled -- a caller that wants it measures the root itself after its
/// own settle (the `o3db_sweep` example does exactly this).
#[derive(Clone, Debug)]
pub struct SweepReport {
    pub scanned_main_keys:      usize,  // main keys the live scan emitted (Complete and bunch keys)
    pub live_chunk_keys:        usize,  // chunk keys referenced by a live bunch key
    pub chunk_data_keys:        usize,  // all chunk-data keys (cind >= 1) the inverse scan emitted
    pub orphans_found:          usize,  // not in the live set and older than the epoch threshold
    pub orphans_retired:        usize,  // tombstones written (equals orphans_found on success)
    pub skipped_recent:         usize,  // not in the live set but at/after the epoch threshold: kept
    pub bytes_before:           u64,    // zone data-file bytes when the sweep started
}

impl SweepReport {
    /// A human-readable multi-line summary for a log or a console.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str("Orphan sweep:\n");
        s.push_str(&fmt!("  scanned main keys   : {}\n", self.scanned_main_keys));
        s.push_str(&fmt!("  live chunk keys     : {}\n", self.live_chunk_keys));
        s.push_str(&fmt!("  chunk-data keys seen: {}\n", self.chunk_data_keys));
        s.push_str(&fmt!("  orphans retired     : {} of {} found\n", self.orphans_retired, self.orphans_found));
        s.push_str(&fmt!("  skipped (too recent): {}\n", self.skipped_recent));
        s.push_str(&fmt!("  data bytes at start : {}\n", self.bytes_before));
        s
    }
}


/// Reclaims orphaned chunk-data records in place and returns a report.
///
/// The store must be running with garbage collection ON (unlike the migration, which needs it off):
/// the sweep retires an orphan by writing a deleted-kind tombstone at the chunk key, and it is the
/// running collector that then reclaims the superseded chunk bytes.  The sweep issues no write to
/// any value; it only tombstones chunk keys it has proven orphaned.
///
/// Safety rests on three things, each grounded in the store's own mechanics:
/// - **Exact membership.** The live set is reconstructed from every live bunch key exactly as the
///   reader reconstructs chunk keys, and membership is exact `[u64; 5]` tuple equality, so a
///   geometry-change orphan (same set id, different geometry) and a random-ticket orphan (unrelated
///   set id) are both absent from it while a live value's chunks are all present.
/// - **Conservative on uncertainty.** A failure to read any scanned main key aborts the sweep
///   rather than narrowing the live set, so a transient read error can never widen the orphan set.
/// - **Epoch guard.** Only chunk records stamped before `T0 - epoch_skew` are retired, so a value
///   written during the sweep is excluded even if its bunch key was enumerated late.  `epoch_skew`
///   of zero is correct only for a quiesced store.
///
/// # Arguments
/// * `scan_wait` - how long each of the two store-wide scans may take.  A scan walks every index
///   file in every zone, so a large store needs far longer than the shared user-request deadline;
///   pass a generous wait.
/// * `epoch_skew` - the clock margin below `T0`; a few seconds guards against a writer whose clock
///   is marginally behind the sweeper's.  Pass `Duration::ZERO` only for a store nothing is writing.
pub fn sweep_orphans<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    user:       UID,
    schms2:     Option<&RestSchemesOverride<ENC, KH>>,
    scan_wait:  Wait,
    epoch_skew: Duration,
)
    -> Outcome<SweepReport>
{
    let enc = api.schemes().encrypter();
    let or_enc = schms2.map(|s| s.encrypter());

    // 1. Stamp the epoch before touching the store, and derive the retirement threshold.  A chunk
    //    record is old enough to retire only if its timestamp is strictly below this.
    let t0 = res!(Timestamp::now());
    let t0_dur: Duration = *t0;
    let threshold_dur = t0_dur.saturating_sub(epoch_skew);
    let threshold = Timestamp::new(threshold_dur.as_secs(), threshold_dur.subsec_nanos());

    let bytes_before = res!(zone_data_bytes(api.db_root()));

    // 2. Build the LIVE chunk-key set from every live bunch key.  Scan for main keys, fetch each to
    //    its stored value, and for a chunked value (a Tup5u64 bunch value) reconstruct its chunk
    //    keys exactly as the reader does.  A read error aborts: never narrow the live set on
    //    uncertainty.  An absent value (a tombstone, or a value mid-write) contributes no live
    //    chunks, which is safe because the epoch guard keeps any recent value's chunks regardless.
    let main_opts = ScanOpts::all();
    let main_entries = res!(api.scan_with_wait(&main_opts, schms2, dup_wait(&scan_wait)));
    let scanned_main_keys = main_entries.len();

    let mut live: HashSet<[u64; 5]> = HashSet::new();
    for (kdat, _empty, _meta) in &main_entries {
        let resp = res!(api.fetch_using_schemes(kdat, schms2));
        match res!(resp.recv_daticle(enc, or_enc)) {
            (Some((Dat::Tup5u64(tup), _)), _) => {
                // A chunked value: its bunch value is the part key naming its chunks.
                let set_id    = tup[0];
                let data_len  = tup[2];
                let num_parts = tup[3];
                let part_size = tup[4];
                for i in 1..(num_parts + 1) {
                    live.insert([set_id, i, data_len, num_parts, part_size]);
                }
            },
            // An unchunked value, or an absent one (tombstone / mid-write): no live chunks.
            _ => (),
        }
    }
    let live_chunk_keys = live.len();

    // 3. Enumerate every chunk-data key (the inverse scan), classify against the live set and the
    //    epoch, and collect the orphans to retire.  Each candidate decodes to its Tup5u64 chunk
    //    key; one that does not is left untouched rather than guessed at.
    let chunk_opts = ScanOpts::all().chunk_data_only(true);
    let chunk_entries = res!(api.scan_with_wait(&chunk_opts, schms2, dup_wait(&scan_wait)));
    let chunk_data_keys = chunk_entries.len();

    let mut orphans: Vec<Dat> = Vec::new();
    let mut skipped_recent = 0usize;
    for (kdat, _empty, meta) in &chunk_entries {
        let tup = match kdat {
            Dat::Tup5u64(arr) => *arr,
            // A chunk-data key that is not a Tup5u64 cannot be classified; leave it alone.
            _ => continue,
        };
        if live.contains(&tup) {
            continue; // Referenced by a live bunch key: keep.
        }
        // Not referenced: an orphan by membership.  Retire only if old enough; a recent one is a
        // value that may still be settling, so it is kept and reclaimed by a later sweep.
        if meta.time < threshold {
            orphans.push(kdat.clone());
        } else {
            skipped_recent += 1;
        }
    }
    let orphans_found = orphans.len();

    // 4. Retire each orphan through the ordinary supersession path: an unencrypted deleted-kind
    //    tombstone at the chunk key, which supersedes the chunk record at the same stored key so the
    //    running collector flags its bytes old and reclaims them.  One shared responder collects one
    //    acknowledgement per tombstone, so the function returns only once every write has landed.
    let mut orphans_retired = 0usize;
    if orphans_found > 0 {
        let resp = api.responder();
        for ck in &orphans {
            res!(api.tombstone_chunk_key(ck, user, schms2, resp.clone()));
            orphans_retired += 1;
        }
        let ack_wait = dup_wait(&scan_wait);
        res!(resp.recv_number(orphans_retired, ack_wait));
    }

    // The function returns here, as soon as every tombstone is acknowledged.  It does NOT wait for
    // the collector: reclamation is asynchronous, so a caller that wants the reclaimed footprint
    // settles and re-measures the root itself (see the `o3db_sweep` example).
    Ok(SweepReport {
        scanned_main_keys,
        live_chunk_keys,
        chunk_data_keys,
        orphans_found,
        orphans_retired,
        skipped_recent,
        bytes_before,
    })
}

/// A fresh copy of a `Wait`, since `scan_with_wait` and `recv_number` take it by value and it is not
/// `Copy`; its fields are, so this is a plain field copy.
fn dup_wait(w: &Wait) -> Wait {
    Wait {
        max_wait:       w.max_wait,
        check_interval: w.check_interval,
    }
}

/// Total bytes of the store's data files (`.dat`) under a root, walked recursively.  Index files and
/// anything else are not counted; the reclaim the sweep drives shows up as data-file bytes.
fn zone_data_bytes(root: &Path) -> Outcome<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue, // A directory that is not there contributes nothing.
        };
        for entry in rd {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == constant::DATA_FILE_EXT).unwrap_or(false) {
                let meta = res!(entry.metadata());
                total += meta.len();
            }
        }
    }
    Ok(total)
}
