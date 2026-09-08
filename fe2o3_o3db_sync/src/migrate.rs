//! Live-set migration and compaction for an Ozone store.
//!
//! [`migrate_live_set`] copies exactly the live key set of a source store into
//! a fresh target store and, in doing so, drops the orphaned chunk-data records
//! that a pre-fix build leaked (chunk keys left behind when a chunked value was
//! overwritten or deleted under a fresh random ticket).  Nothing about the copy
//! is chunk-aware: it rests on three properties of the store this crate builds,
//! each verified against the code rather than assumed.
//!
//! - **`scan` answers from the live index and emits only main user keys.**  A
//!   scan walks every index file in every zone and returns each live
//!   `Key::Complete` and each live bunch key `Key::Chunk(_, 0)`, eliding the
//!   chunk-data keys (index `>= 1`) that carry a chunked value's bytes
//!   (`bots/worker/bot_scan.rs`, the `c >= 1` continue).  An orphaned chunk key
//!   has no live bunch key naming it, so a scan never emits it and the copy
//!   never carries it: that, and only that, is what reclaims the leak.
//! - **A scan enumerates every live `Complete` key independent of references.**
//!   The walk is over the index files themselves, not a reachability walk from
//!   any root, so a value stored under its own `Complete` key that nothing else
//!   points at -- a chat body keyed `sbody:<account>:<hex>`, referenced only by
//!   the hashes inside a separate `sync:` record -- is enumerated and copied
//!   like any other key.  A reachability-based copy would drop it; this does
//!   not.
//! - **`get` on a bunch key returns the whole current value.**  `get_wait`
//!   hands a `Dat::Tup5u64` bunch key to `fetch_chunks`, which reconstructs the
//!   current chunk keys from the stored `PartKey` and rejoins, decrypts and
//!   decodes them, so a copied value is the complete, current value of whatever
//!   kind it was stored as.
//!
//! Both stores are opened under the same schemes, including the same at-rest
//! encryption key: `get_wait` on the source returns plaintext and `store` on
//! the target re-encrypts it, so the target holds the same values, freshly
//! encrypted.  The source is only ever read; every write goes to the target.
//!
//! A key whose newest record is a deletion tombstone reads back as absent
//! (`Responder::recv_daticle` maps the deleted-kind marker to `None`), so a
//! scan emits it but `get_wait` yields nothing.  The migration copies nothing
//! for such a key, which is the correct compaction: the target simply lacks the
//! key and reads it as absent, exactly as the source did.  Tombstones are dead
//! weight that the copy leaves behind.

use crate::{
    prelude::*,
    comm::{
        msg::OzoneMsg,
        response::Wait,
    },
};

use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_iop_db::api::{
    RestSchemesOverride,
    ScanOpts,
};

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
};


/// What a migration copied, and whether it verified.
///
/// The byte and file totals are measured over each store's root directory, so
/// they are meaningful only when the store keeps its data under that root (no
/// absolute zone override pointing elsewhere), which is the single-directory
/// arrangement this tool is built for.
#[derive(Clone, Debug)]
pub struct MigrationReport {
    pub scanned:        usize,                  // keys the source scan emitted
    pub copied:         usize,                  // keys with a value present, copied to the target
    pub tombstones:     usize,                  // scanned keys whose value was absent (deleted)
    pub verified:       usize,                  // copied keys whose target bytes matched the source
    pub per_prefix:     BTreeMap<String, usize>,// copied keys grouped by key prefix (before first ':')
    pub source_bytes:   u64,
    pub target_bytes:   u64,
    pub source_files:   usize,
    pub target_files:   usize,
    pub verified_ok:    bool,                   // every copied key read back byte-identical, counts agreed
}

impl MigrationReport {
    /// A human-readable multi-line summary for a log or a console.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str(&fmt!(
            "Migration {}:\n",
            if self.verified_ok { "VERIFIED" } else { "NOT verified" }));
        s.push_str(&fmt!("  scanned live keys : {}\n", self.scanned));
        s.push_str(&fmt!("  copied (present)  : {}\n", self.copied));
        s.push_str(&fmt!("  tombstones (drop) : {}\n", self.tombstones));
        s.push_str(&fmt!("  verified round-trip: {}\n", self.verified));
        s.push_str(&fmt!("  source bytes      : {} in {} files\n", self.source_bytes, self.source_files));
        s.push_str(&fmt!("  target bytes      : {} in {} files\n", self.target_bytes, self.target_files));
        if self.source_bytes > 0 {
            let pct = (self.target_bytes as f64 / self.source_bytes as f64) * 100.0;
            s.push_str(&fmt!("  target is {:.2}% of source by bytes\n", pct));
        }
        s.push_str("  copied keys by prefix:\n");
        for (prefix, n) in &self.per_prefix {
            s.push_str(&fmt!("    {:<24} {}\n", prefix, n));
        }
        s
    }
}


/// Copies the live key set of `source` into `target`, verifies the copy, and
/// returns a report.
///
/// The function is fail-closed: it returns an error if any copied key does not
/// read back from the target byte-identical to the source, or if the target's
/// live-key count does not equal the number of present source keys.  A returned
/// `Ok` report always has `verified_ok == true`.  Deciding whether the target
/// is *small enough* to trust (the orphan-shrink gate) is left to the caller,
/// which knows how much shrink to expect; the report carries the byte totals it
/// needs.
///
/// `target` must be freshly created with the *same* configuration as `source`
/// (so chunk geometry and encryption match) and opened under the same schemes.
/// `source` must be opened with garbage collection off and must not be written
/// to by anyone else for the duration; this function issues no write to it.
///
/// # Arguments
/// * `scan_wait` - how long each zone's scan may take.  A scan walks every index
///   file in the store, so a large store needs far longer than the shared
///   user-request deadline; pass a generous wait.
pub fn migrate_live_set<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    source:     &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    target:     &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    user:       UID,
    schms2:     Option<&RestSchemesOverride<ENC, KH>>,
    scan_wait:  Wait,
)
    -> Outcome<MigrationReport>
{
    // 1. Enumerate every live main key in the source, across all zones.  No
    //    prefix, no limit, values excluded: scan v1 returns Dat::Empty for
    //    values by design, and the value is fetched per key below.
    let opts = ScanOpts::all();
    let entries = res!(source.scan_with_wait(&opts, schms2, dup_wait(&scan_wait)));
    let scanned = entries.len();

    let mut copied      = 0usize;
    let mut tombstones  = 0usize;
    let mut per_prefix: BTreeMap<String, usize> = BTreeMap::new();

    // 2. Copy each present value into the target.  A key whose value is absent
    //    is a live tombstone: it is dropped, not copied.
    for (kdat, _empty, _meta) in &entries {
        match res!(source.get_wait(kdat, schms2)) {
            None => {
                tombstones += 1;
            },
            Some((val, _meta)) => {
                res!(store_and_wait(target, kdat.clone(), val, user, schms2));
                copied += 1;
                let prefix = key_prefix(kdat);
                *per_prefix.entry(prefix).or_insert(0) += 1;
            },
        }
    }

    // 3. Verify before reporting success.  Every present source key must read
    //    back from the target byte-identical, and the target must hold exactly
    //    the present source keys and no more.
    let verified = res!(verify_migration(source, target, user, schms2, scan_wait, copied));

    let (source_bytes, source_files) = res!(dir_bytes_and_files(source.db_root()));
    let (target_bytes, target_files) = res!(dir_bytes_and_files(target.db_root()));

    Ok(MigrationReport {
        scanned,
        copied,
        tombstones,
        verified,
        per_prefix,
        source_bytes,
        target_bytes,
        source_files,
        target_files,
        verified_ok: true,
    })
}

/// Confirms the target holds exactly the present source keys, each byte-identical.
///
/// Returns the number of keys checked (the present source keys).  Errors on the
/// first mismatch or count disagreement, so a caller that gets `Ok` has a
/// verified copy.  `expected_present` is the count of keys the copy pass wrote,
/// used to cross-check against an independent re-scan of both stores.
pub fn verify_migration<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    source:             &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    target:             &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    _user:              UID,
    schms2:             Option<&RestSchemesOverride<ENC, KH>>,
    scan_wait:          Wait,
    expected_present:   usize,
)
    -> Outcome<usize>
{
    let opts = ScanOpts::all();

    // A. Re-scan the source independently and compare each key's value against
    //    the target, byte for byte.  A tombstone (absent value) must be absent
    //    in the target too.
    let src_entries = res!(source.scan_with_wait(&opts, schms2, dup_wait(&scan_wait)));
    let mut present = 0usize;
    for (kdat, _empty, _meta) in &src_entries {
        let sv = res!(source.get_wait(kdat, schms2));
        let tv = res!(target.get_wait(kdat, schms2));
        match (sv, tv) {
            (None, None) => {
                // A tombstoned source key: correctly absent from the target.
            },
            (None, Some(_)) => return Err(err!(
                "Verify: source key {:?} is a tombstone (absent) but the target \
                holds a value for it; the copy invented a key.", kdat;
                Data, Mismatch)),
            (Some(_), None) => return Err(err!(
                "Verify: source key {:?} has a value but the target does not; the \
                copy dropped a live key.", kdat;
                Data, Missing)),
            (Some((svd, _)), Some((tvd, _))) => {
                let sb = res!(svd.as_bytes());
                let tb = res!(tvd.as_bytes());
                if sb != tb {
                    return Err(err!(
                        "Verify: source key {:?} reads back {} value bytes from the \
                        target but {} from the source; the copied value differs.",
                        kdat, tb.len(), sb.len();
                        Data, Mismatch));
                }
                present += 1;
            },
        }
    }

    if present != expected_present {
        return Err(err!(
            "Verify: the copy pass wrote {} present keys, but an independent \
            re-scan of the source finds {} present keys; the source changed \
            underneath the migration, or a key was miscounted.",
            expected_present, present;
            Data, Mismatch));
    }

    // B. Re-scan the target: it must hold exactly the present source keys, none
    //    of them a tombstone, and no extras.
    let tgt_entries = res!(target.scan_with_wait(&opts, schms2, dup_wait(&scan_wait)));
    let mut tgt_present = 0usize;
    for (kdat, _empty, _meta) in &tgt_entries {
        match res!(target.get_wait(kdat, schms2)) {
            None => return Err(err!(
                "Verify: the target scan emitted key {:?} but it reads back absent; \
                the target holds a tombstone, which a fresh copy should never write.",
                kdat;
                Data, Mismatch)),
            Some(_) => tgt_present += 1,
        }
    }
    if tgt_present != present {
        return Err(err!(
            "Verify: the target holds {} live keys but the source has {} present \
            keys; the live-key counts do not agree.",
            tgt_present, present;
            Data, Mismatch));
    }

    Ok(present)
}

/// Stores one key-value pair into `api` and waits for every write part to be
/// acknowledged, so the value is in the target index before anything reads it.
///
/// A store dispatches `n` write parts -- one `Complete` record, or one bunch key
/// plus one record per chunk -- and reports `n` first as an `OzoneMsg::Chunks`,
/// then one acknowledgement per part.  Waiting for all `n` acknowledgements,
/// rather than on ordering between writers, is what makes the copy safe to
/// verify immediately.
fn store_and_wait<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    k:      Dat,
    v:      Dat,
    user:   UID,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
)
    -> Outcome<()>
{
    let resp = res!(api.store_using_schemes(k, v, user, schms2));
    let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => n,
        other => return Err(err!(
            "Expected an OzoneMsg::Chunks acknowledging the write, got {:?}.", other;
            Channel, Unexpected)),
    };
    res!(resp.recv_number(n, constant::USER_REQUEST_WAIT));
    Ok(())
}

/// A fresh copy of a `Wait`, since `scan_with_wait` takes it by value and it is
/// not `Copy`; its fields are, so this is a plain field copy.
fn dup_wait(w: &Wait) -> Wait {
    Wait {
        max_wait:       w.max_wait,
        check_interval: w.check_interval,
    }
}

/// The part of a string key before its first ':', or the whole key if it has
/// none; a non-string key groups under `<non-str>`.  Used only to group the
/// report so an operator can eyeball that every expected keyspace is present.
fn key_prefix(k: &Dat) -> String {
    match k {
        Dat::Str(s) => match s.find(':') {
            Some(i) => s[..i].to_string(),
            None    => s.clone(),
        },
        _ => "<non-str>".to_string(),
    }
}

/// Total bytes and file count under a directory, walked recursively.  Symlinks
/// are not followed; a store keeps regular files only.
fn dir_bytes_and_files(root: &Path) -> Outcome<(u64, usize)> {
    let mut bytes = 0u64;
    let mut files = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue, // A directory that is not there contributes nothing.
        };
        for entry in rd {
            let entry = res!(entry);
            let path = entry.path();
            let meta = res!(entry.metadata());
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    Ok((bytes, files))
}
