//! Live-set migration / compaction integration test.
//!
//! Proves the `migrate` module on synthetic, ENCRYPTED stores that reproduce
//! the exact conditions of the daimond gateway store:
//!
//! - **Orphaned chunks are dropped.** A chunked value overwritten many times
//!   under the pre-fix random-ticket scheme leaves its old chunk-data keys as
//!   live index entries with nothing referencing them. The source bloats; the
//!   target holds only the current value's chunks, so it is dramatically
//!   smaller. The current live value reads back byte-identical from the target.
//! - **Unreferenced `Complete` keys survive (the sbody trap).** A small
//!   `sync:`-like record holds only hashes; several large `sbody:`-like records
//!   are separate `Complete` keys that nothing in the value graph points at. The
//!   migration must copy every one of them -- a reachability copy would drop
//!   them. This is the data-loss case, tested explicitly.
//! - **Several keyspaces, several keys each, all carried.** The per-prefix
//!   report accounts for every keyspace.
//! - **Tombstones are dropped.** A deleted key is emitted by a scan but reads as
//!   absent; the target simply lacks it, and reads it as absent too.
//! - **The source is not mutated in any data or index file by a read-only,
//!   gc-off open.** The one file a plain open rewrites is `config.jdat`; every
//!   `.dat`/`.ind` file is byte-identical after the migration. This is the
//!   backup-safety property the rename-based runbook depends on.
//! - **An unchunked-only store migrates and compacts too.**
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
};
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    data::core::RestSchemesInput,
    file::{
        core::{
            FileAccess,
            FileType,
        },
        stored::{
            StoredIndex,
            StoredKey,
        },
        zdir::ZoneDir,
    },
    migrate,
    test::setup,
};

use std::{
    collections::BTreeMap,
    fs,
    io::BufReader,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};


type Enc = EncryptionScheme;
type Kh  = HashScheme;
type Cs  = ChecksumScheme;

/// A generous scan deadline: a scan walks every index file, so a real store
/// needs far more than the shared user-request deadline. Ample for the test.
const SCAN_WAIT: Wait = Wait {
    max_wait:       Duration::from_secs(120),
    check_interval: constant::CHECK_INTERVAL,
};


#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run_all();
    log_finish_wait!();
    outcome
}

fn run_all() -> Outcome<()> {
    res!(scenario_orphan_shrink_and_untouched());
    res!(scenario_unchunked_only());
    Ok(())
}

// A 32-byte at-rest key, as the gateway holds in a key file. Fixed here so the
// test is deterministic; NEVER hardcode a real key -- the tool reads it from a
// path at runtime.
fn test_key() -> [u8; 32] { [0x5au8; 32] }

fn schemes_input() -> Outcome<RestSchemesInput<Enc, Kh, Kh, Cs>> {
    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&test_key()[..]));
    let crc32 = ChecksumScheme::new_crc32();
    Ok(RestSchemesInput::new(
        Some(aes_gcm),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32),
    ))
}

fn fresh_root(name: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(name);
    res!(fs::create_dir_all(name));
    Ok(res!(Path::new(name).canonicalize()))
}

fn empty_root(name: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(name);
    res!(fs::create_dir_all(name));
    Ok(res!(Path::new(name).canonicalize()))
}

/// Every regular file under `root`, keyed by path, mapped to its exact bytes.
/// The synthetic stores are small, so exact bytes are kept rather than a hash.
fn snapshot(root: &Path) -> Outcome<BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in rd {
            let entry = res!(entry);
            let path = entry.path();
            let meta = res!(entry.metadata());
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.insert(path.clone(), res!(fs::read(&path)));
            }
        }
    }
    Ok(out)
}

/// The base configuration for the synthetic stores: encrypted, several zones,
/// small files and a low chunk threshold so values chunk and files roll over,
/// no per-zone overrides so everything lives under the store root (as the
/// gateway store does). Garbage collection is driven per-store at start.
fn base_cfg() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 3;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.data_file_max_bytes     = 4_000;
    cfg.rest_chunk_threshold    = 500;
    cfg.rest_chunk_bytes        = 128;
    cfg.cache_size_limit_bytes  = 40_000_000;
    cfg.zone_overrides          = BTreeMap::new();
    Ok(cfg)
}


/// The value length written in overwrite `round`. Strictly increasing, so every
/// round has a distinct length -> a distinct chunk geometry -> distinct chunk
/// keys, whether keys are random-ticket (today) or deterministic (post-fix). All
/// lengths are well over the chunk threshold, so every round chunks.
fn round_size(round: usize) -> usize { 2_000 + round * 200 }

fn scenario_orphan_shrink_and_untouched() -> Outcome<()> {

    let src_root = res!(fresh_root("./test_db_migrate_src"));
    let tgt_root = res!(empty_root("./test_db_migrate_tgt"));
    let user = setup::Uid::default();

    // Distinct, self-checking payload for a key so a readback can be compared.
    let payload = |tag: &str, n: usize| -> Dat {
        let seed = fmt!("{}:", tag);
        let mut s = String::with_capacity(n);
        while s.len() < n { s.push_str(&seed); }
        s.truncate(n);
        Dat::Str(s)
    };

    // ── Phase 1: build the source, populate it, shut it down cleanly. ──
    {
        let db = res!(setup::start_db(
            src_root.clone(),
            Some(res!(base_cfg())),
            res!(schemes_input()),
            None,
            false, // gc OFF: nothing is collected, so orphans accumulate as they would pre-fix
            true,  // wipe
        ));
        thread::sleep(Duration::from_secs(1));

        // (a) Several keyspaces, several keys each -- small unchunked Complete keys.
        for (prefix, keys) in &[
            ("acct",  3usize),
            ("sess",  4),
            ("lease", 2),
            ("sub",   2),
            ("a",     3),
        ] {
            for i in 0..*keys {
                let k = dat!(fmt!("{}:{:04}", prefix, i));
                res!(store_wait(db.api(), k, payload(prefix, 40), user));
            }
        }

        // (b) The sbody trap: one small "sync"-like record holding only hashes,
        //     and several large "sbody" Complete keys that NOTHING references.
        //     They are independent live Complete keys; a reachability copy from
        //     `sync:` would drop them.
        let mut sync_hashes = String::new();
        for i in 0..5usize {
            let body = payload("sbody", 300 + i * 10); // under the chunk threshold => unchunked Complete
            let hk = fmt!("sbody:acctZ:{:08x}", i as u64);
            sync_hashes.push_str(&hk);
            sync_hashes.push(',');
            res!(store_wait(db.api(), dat!(hk.clone()), body, user));
        }
        // The sync record's value contains only the hashes; it does not carry the bodies.
        res!(store_wait(db.api(), dat!("sync:acctZ"), Dat::Str(sync_hashes), user));

        // (c) A chunked value overwritten many times -> orphaned chunk keys.
        //     Each overwrite CHANGES THE GEOMETRY (a different value length, so a
        //     different data_len and part count), which changes the chunk keys
        //     under any deterministic scheme as well as under today's random
        //     ticket. So the prior generation's chunk-data keys are genuinely
        //     orphaned either way, and this test cannot go vacuous once the
        //     deterministic-key leak fix lands on the same branch. A same-size
        //     overwrite would reuse the chunk keys under deterministic keys and
        //     mint no orphans at all.
        let chunk_key = dat!("chunked:big");
        for round in 0..30usize {
            let sz = round_size(round);
            res!(store_wait(db.api(), chunk_key.clone(), payload(&fmt!("v{:02}", round), sz), user));
        }

        // (d) A key that is written then deleted -> a live tombstone that a scan
        //     emits but get() reads as absent. The migration must drop it.
        res!(store_wait(db.api(), dat!("acct:deleteme"), payload("gone", 40), user));
        res!(delete_wait(db.api(), dat!("acct:deleteme"), user));

        thread::sleep(Duration::from_secs(2));
        res!(db.shutdown());
    }
    thread::sleep(Duration::from_secs(1));

    // Snapshot the source while it is closed -- this is what the migration must
    // not disturb in any data or index file.
    let before = res!(snapshot(&src_root));
    let src_bytes_before: u64 = before.values().map(|v| v.len() as u64).sum();

    // ── Phase 2: open source read-only (gc off), create target with the
    //    source's config, migrate, verify. ──
    let report = {
        let src_db = res!(setup::start_db(
            src_root.clone(),
            None,  // load the source's own config.jdat
            res!(schemes_input()),
            None,
            false, // gc OFF: a read-only, non-collecting open
            false, // do NOT wipe
        ));
        thread::sleep(Duration::from_secs(1));

        // Prove, structurally, that the source actually holds orphaned chunk
        // data BEFORE the migration -- otherwise the orphan-drop path this tool
        // exists for would not be exercised and the test would be vacuous. Only
        // one key ("chunked:big") ever chunked, so its current bunch key names
        // exactly `live_parts` live chunk keys; every other chunk-data record on
        // disk (index >= 1) is an orphan of a superseded generation.
        let live_parts = res!(live_bunch_num_parts(src_db.api(), &dat!("chunked:big")));
        let total_chunk_records = res!(count_chunk_data_records(src_db.api()));
        let orphans = total_chunk_records.saturating_sub(live_parts);
        warn!(sync_log::stream(),
            "Orphan check: {} chunk-data records on disk, {} referenced by the live \
            value, so {} orphaned chunk-data keys with no live bunch key.",
            total_chunk_records, live_parts, orphans);
        if orphans == 0 {
            return Err(err!(
                "No orphaned chunk-data keys were minted (total chunk-data records {} == \
                live parts {}); the orphan-drop path is not being exercised, so the test \
                would be vacuous. The overwrite geometry must change per round.",
                total_chunk_records, live_parts;
                Test, Data));
        }

        // Carry the source's config to the fresh target, unchanged.
        let src_cfg = src_db.cfg().clone();
        let tgt_db = res!(setup::start_db(
            tgt_root.clone(),
            Some(src_cfg),
            res!(schemes_input()),
            None,
            false, // gc off during the bulk load
            true,  // fresh
        ));
        thread::sleep(Duration::from_secs(1));

        let report = res!(migrate::migrate_live_set(
            src_db.api(),
            tgt_db.api(),
            user,
            None,       // use each store's own (encrypted) default schemes
            SCAN_WAIT,
        ));

        // Print the report so the run carries the evidence.
        warn!(sync_log::stream(), "{}", report.summary());

        res!(tgt_db.shutdown());
        res!(src_db.shutdown());
        report
    };
    thread::sleep(Duration::from_secs(1));

    // ── Assertions on the report. ──
    if !report.verified_ok {
        return Err(err!("Migration did not verify."; Test, Data));
    }
    // Present keys: acct(3)+sess(4)+lease(2)+sub(2)+a(3) = 14, + 5 sbody + 1 sync
    //   + 1 chunked = 21. The deleted key is a tombstone and must NOT be copied.
    let expect_copied = 3 + 4 + 2 + 2 + 3 + 5 + 1 + 1;
    if report.copied != expect_copied {
        return Err(err!(
            "Expected to copy {} present keys, copied {}.", expect_copied, report.copied;
            Test, Data, Mismatch));
    }
    if report.tombstones < 1 {
        return Err(err!(
            "Expected at least one tombstone (the deleted key) to be dropped, saw {}.",
            report.tombstones;
            Test, Data, Mismatch));
    }
    // The sbody trap: every unreferenced sbody Complete key survived.
    match report.per_prefix.get("sbody") {
        Some(5) => (),
        other => return Err(err!(
            "Expected 5 unreferenced sbody Complete keys copied, per-prefix says {:?}. \
            Unreferenced Complete keys were dropped -- the data-loss case.", other;
            Test, Data, Mismatch)),
    }
    // The deleted key must not appear under acct: acct should be exactly 3.
    match report.per_prefix.get("acct") {
        Some(3) => (),
        other => return Err(err!(
            "Expected acct prefix count 3 (the deleted key dropped), got {:?}.", other;
            Test, Data, Mismatch)),
    }
    // The orphan-shrink property: the target is dramatically smaller than the
    // source, because ~29 superseded generations of chunk data were dropped.
    if !(report.target_bytes * 4 < report.source_bytes) {
        return Err(err!(
            "Expected the target to be far smaller than the source (orphans dropped): \
            source {} bytes, target {} bytes.", report.source_bytes, report.target_bytes;
            Test, Data));
    }

    // ── The backup-safety property: no data or index file of the source changed. ──
    let after = res!(snapshot(&src_root));
    let mut changed_data_index: Vec<String> = Vec::new();
    let mut other_changes: Vec<String> = Vec::new();
    for (path, bytes_before) in &before {
        match after.get(path) {
            Some(bytes_after) if bytes_after == bytes_before => (),
            Some(_) => {
                let name = path.to_string_lossy().to_string();
                if name.ends_with(".dat") || name.ends_with(".ind") {
                    changed_data_index.push(name);
                } else {
                    other_changes.push(fmt!("{} (modified)", name));
                }
            },
            None => other_changes.push(fmt!("{} (removed)", path.to_string_lossy())),
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            let name = path.to_string_lossy().to_string();
            if name.ends_with(".dat") || name.ends_with(".ind") {
                // A newly created (empty) live file is additive and does not
                // touch existing data; note it but do not fail on it.
                other_changes.push(fmt!("{} (new)", name));
            } else {
                other_changes.push(fmt!("{} (new)", name));
            }
        }
    }
    warn!(sync_log::stream(),
        "Source after migration: {} data/index files changed in place; other changes: {:?}. \
        Source bytes before: {}.",
        changed_data_index.len(), other_changes, src_bytes_before);
    if !changed_data_index.is_empty() {
        return Err(err!(
            "A read-only migration modified {} of the source's data/index files in place: {:?}. \
            The rename-based runbook keeps the source as the only backup, so this would corrupt \
            the backup and is a blocker.",
            changed_data_index.len(), changed_data_index;
            Test, Data));
    }

    // ── Phase 3: reopen the target and confirm the live values read back. ──
    {
        let tgt_db = res!(setup::start_db(
            tgt_root.clone(),
            None,
            res!(schemes_input()),
            None,
            false,
            false,
        ));
        thread::sleep(Duration::from_secs(1));

        // The chunked value reads back byte-identical to what was last written.
        let expect = payload("v29", round_size(29));
        match res!(tgt_db.api().get_wait(&dat!("chunked:big"), None)) {
            Some((v, _)) => {
                if res!(v.as_bytes()) != res!(expect.as_bytes()) {
                    return Err(err!(
                        "Target chunked value does not match the last written value.";
                        Test, Data, Mismatch));
                }
            },
            None => return Err(err!(
                "Target is missing the chunked live value."; Test, Data, Missing)),
        }
        // Every sbody body survived and reads back.
        for i in 0..5usize {
            let expect = payload("sbody", 300 + i * 10);
            let hk = fmt!("sbody:acctZ:{:08x}", i as u64);
            match res!(tgt_db.api().get_wait(&dat!(hk.clone()), None)) {
                Some((v, _)) => {
                    if res!(v.as_bytes()) != res!(expect.as_bytes()) {
                        return Err(err!(
                            "Target sbody value {} does not match.", hk; Test, Data, Mismatch));
                    }
                },
                None => return Err(err!(
                    "Target dropped an unreferenced sbody key: {}.", hk; Test, Data, Missing)),
            }
        }
        // The deleted key reads as absent in the target, as in the source.
        if res!(tgt_db.api().get_wait(&dat!("acct:deleteme"), None)).is_some() {
            return Err(err!(
                "Target holds the deleted key; a tombstone should have been dropped.";
                Test, Data));
        }
        res!(tgt_db.shutdown());
    }
    thread::sleep(Duration::from_secs(1));

    let _ = fs::remove_dir_all(&src_root);
    let _ = fs::remove_dir_all(&tgt_root);
    Ok(())
}


fn scenario_unchunked_only() -> Outcome<()> {

    let src_root = res!(fresh_root("./test_db_migrate_u_src"));
    let tgt_root = res!(empty_root("./test_db_migrate_u_tgt"));
    let user = setup::Uid::default();

    let payload = |tag: &str| -> Dat { Dat::Str(fmt!("{}-value-under-threshold", tag)) };

    {
        let db = res!(setup::start_db(
            src_root.clone(),
            Some(res!(base_cfg())),
            res!(schemes_input()),
            None,
            false,
            true,
        ));
        thread::sleep(Duration::from_secs(1));

        // A handful of keys, each overwritten many times with gc off, so the
        // source carries many superseded records the migration should drop. All
        // values are well under the chunk threshold, so nothing chunks.
        for i in 0..6usize {
            let k = dat!(fmt!("rec:{:04}", i));
            for round in 0..20usize {
                res!(store_wait(db.api(), k.clone(), payload(&fmt!("r{:02}", round)), user));
            }
        }
        thread::sleep(Duration::from_secs(2));
        res!(db.shutdown());
    }
    thread::sleep(Duration::from_secs(1));

    let report = {
        let src_db = res!(setup::start_db(
            src_root.clone(), None, res!(schemes_input()), None, false, false));
        thread::sleep(Duration::from_secs(1));
        let src_cfg = src_db.cfg().clone();
        let tgt_db = res!(setup::start_db(
            tgt_root.clone(), Some(src_cfg), res!(schemes_input()), None, false, true));
        thread::sleep(Duration::from_secs(1));

        let report = res!(migrate::migrate_live_set(
            src_db.api(), tgt_db.api(), user, None, SCAN_WAIT));
        warn!(sync_log::stream(), "{}", report.summary());
        res!(tgt_db.shutdown());
        res!(src_db.shutdown());
        report
    };
    thread::sleep(Duration::from_secs(1));

    if !report.verified_ok {
        return Err(err!("Unchunked migration did not verify."; Test, Data));
    }
    if report.copied != 6 {
        return Err(err!(
            "Expected 6 live unchunked keys, copied {}.", report.copied; Test, Data, Mismatch));
    }
    // No chunk records at all: the target is one current record per key, so it
    // is smaller than a source carrying 20 generations of each.
    if !(report.target_bytes < report.source_bytes) {
        return Err(err!(
            "Expected the unchunked target ({} bytes) smaller than the source ({} bytes).",
            report.target_bytes, report.source_bytes; Test, Data));
    }

    let _ = fs::remove_dir_all(&src_root);
    let _ = fs::remove_dir_all(&tgt_root);
    Ok(())
}


// The number of chunks the CURRENT value at `key` references, read from its
// bunch key. Errors if the key is not a chunked (bunch-key) value.
fn live_bunch_num_parts(
    api:    &oxedyne_fe2o3_o3db_sync::api::OzoneApi<{ setup::UID_LEN }, setup::Uid, Enc, Kh, Kh, Cs>,
    key:    &Dat,
)
    -> Outcome<usize>
{
    let resp = res!(api.fetch_using_schemes(key, None));
    let enc = api.schemes().encrypter();
    match res!(resp.recv_daticle(enc, None)) {
        (Some((Dat::Tup5u64(tup), _)), _) => Ok(PartKey(tup).num_parts() as usize),
        (other, _) => Err(err!(
            "Expected a chunked value (a bunch key Dat::Tup5u64) at {:?}, got {:?}.",
            key, other;
            Test, Data, Mismatch)),
    }
}

// Counts every chunk-data record (a stored key whose chunk index is >= 1)
// physically present across the store's index files. Mirrors the scan bot's
// index walk, but counts the chunk-data keys the scan elides. Used to prove
// orphaned chunks exist before a migration.
fn count_chunk_data_records(
    api: &oxedyne_fe2o3_o3db_sync::api::OzoneApi<{ setup::UID_LEN }, setup::Uid, Enc, Kh, Kh, Cs>,
)
    -> Outcome<usize>
{
    let csummer = api.schemes().checksummer().clone();
    let zdirs = res!(api.get_zone_dirs());
    let mut count = 0usize;
    for (_zind, zdir) in &zdirs {
        let mut fnums = Vec::new();
        for entry in res!(fs::read_dir(&zdir.dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if !path.is_file() || ZoneDir::is_gc_temp_file(&path) {
                continue;
            }
            if let Ok((fnum, FileType::Index)) = ZoneDir::ozone_file_number_and_type(&path) {
                fnums.push(fnum);
            }
        }
        for fnum in fnums {
            let (_, file) = match zdir.open_ozone_file(fnum, &FileType::Index, &FileAccess::Reading) {
                Ok(pair) => pair,
                Err(_) => continue, // A file collected away between listing and opening.
            };
            let mut reader = BufReader::new(file);
            loop {
                let key = match res!(StoredKey::<{ setup::UID_LEN }, setup::Uid>::load(
                    &mut reader, csummer.clone(),
                )) {
                    None => break,
                    Some((skey, _, _)) => skey.into_key(),
                };
                // Consume the index entry that follows the key, to reach the next.
                match res!(StoredIndex::read(&mut reader, fnum, csummer.clone())) {
                    (Some(_), _) => (),
                    (None, _) => break,
                }
                if let Some(c) = key.index() {
                    if c >= 1 {
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(count)
}

// Stores one pair and waits for every write part to be acknowledged.
fn store_wait(
    api:    &oxedyne_fe2o3_o3db_sync::api::OzoneApi<{ setup::UID_LEN }, setup::Uid, Enc, Kh, Kh, Cs>,
    k:      Dat,
    v:      Dat,
    user:   setup::Uid,
)
    -> Outcome<()>
{
    use oxedyne_fe2o3_o3db_sync::comm::msg::OzoneMsg;
    let resp = res!(api.store(k, v, user));
    let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => n,
        other => return Err(err!(
            "Expected OzoneMsg::Chunks, got {:?}.", other; Test, Channel, Unexpected)),
    };
    res!(resp.recv_number(n, constant::USER_REQUEST_WAIT));
    Ok(())
}

// Deletes one key and waits for the tombstone write to be acknowledged.
fn delete_wait(
    api:    &oxedyne_fe2o3_o3db_sync::api::OzoneApi<{ setup::UID_LEN }, setup::Uid, Enc, Kh, Kh, Cs>,
    k:      Dat,
    user:   setup::Uid,
)
    -> Outcome<()>
{
    let resp = api.responder();
    res!(api.delete_using_responder(&k, user, None, resp.clone()));
    // The delete writes a single tombstone record; wait for its acknowledgement.
    res!(resp.recv_number(1, constant::USER_REQUEST_WAIT));
    Ok(())
}
