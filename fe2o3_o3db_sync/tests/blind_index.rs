//! An index file that does not account for its data file must not read as an empty store.
//!
//! A zone is scanned by walking its index files. A data file that holds records beside an
//! index file of zero bytes therefore contributes nothing to a scan -- successfully, with no
//! error and no warning the caller can see -- while `get()` by key over those same records
//! goes on working perfectly. That combination is the signature: reads fine, scans empty.
//!
//! It is not a rare state. A store was left in it by an ordinary gate run: `zone_001` with a
//! `.dat` of 5,805 bytes holding two passcodes and three settings writes, and a `.ind` of
//! zero. One of those passcodes was successfully redeemed by key during the same run, while
//! the console answered `200` and `minted: 0` over the records that held the answer. The
//! gateway had said so at start-up, once, to the log:
//!
//! ```ignore
//! WARN bot_initgc.rs: InitGarbageBot:1:1: The index file 1 is empty, trying data file...
//! ```
//!
//! Two defects meet there, and this test holds each of them separately.
//!
//! 1. **A scan cannot report an under-count.** The walk had no way to tell the caller that a
//!    file it was asked to read accounted for none of what it held, so "nothing there" and "I
//!    could not look" arrived as the same answer.
//!
//! 2. **The repair detached the writer.** Start-up does notice the empty index and rebuild it
//!    from the data file -- but it used to rename a freshly written file over the old one, and
//!    `ZoneBot::survey_files` hands the wbot its live `(data, index)` pair before
//!    `init_caches` asks for the rebuild. So a wbot was already holding an append handle on
//!    that index file, the rename replaced the inode underneath it, and every index record
//!    written for the rest of the process went into an unlinked file. The data file is never
//!    renamed and kept everything, which is why keyed reads stayed correct. Every restart
//!    reproduced the state, and a restart is every deploy.
//!
//! The sharpest assertion here is the last: after the recovery, the index file on disk must
//! GROW when a record is written. That one does not go through the scan at all, so it cannot
//! be satisfied by the scan being fixed.
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
    base::index::ZoneInd,
    data::core::RestSchemesInput,
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
    test::setup,
};

use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::Path,
    thread,
    time::Duration,
};

// Records written before the index is emptied.
const KEYS: usize = 40;

// The key written after the recovery, whose index record is the one that used
// to go into an unlinked file.
const AFTER: &str = "blind:after-recovery";

fn key(i: usize) -> Dat {
    dat!(fmt!("blind:{:04}", i))
}

fn val(i: usize) -> Dat {
    dat!(fmt!("value of blind record {:04}", i))
}

/// Total bytes held by the data files and by the index files of the given zones.
fn zone_bytes(zdirs: &BTreeMap<ZoneInd, ZoneDir>) -> Outcome<(u64, u64)> {
    let mut dat_bytes = 0u64;
    let mut ind_bytes = 0u64;
    for (_zind, zdir) in zdirs {
        for entry in res!(std::fs::read_dir(&zdir.dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if !path.is_file() || ZoneDir::is_gc_temp_file(&path) {
                continue;
            }
            let (_fnum, ftyp) = match ZoneDir::ozone_file_number_and_type(&path) {
                Ok(pair) => pair,
                Err(_) => continue,
            };
            let len = res!(entry.metadata()).len();
            match ftyp {
                FileType::Data  => dat_bytes += len,
                FileType::Index => ind_bytes += len,
            }
        }
    }
    Ok((dat_bytes, ind_bytes))
}

/// Truncate every index file in the given zones to zero bytes, leaving the data files alone.
///
/// This is the on-disk state the gate run left, produced deliberately rather than waited for.
/// The files are truncated rather than deleted, because a missing index file and an empty one
/// are surveyed differently: a missing one is `Present::Solo(Data)` and an empty one is
/// `Present::Pair`, and it is the second that occurred in production.
fn empty_all_index_files(zdirs: &BTreeMap<ZoneInd, ZoneDir>) -> Outcome<usize> {
    let mut n = 0;
    for (_zind, zdir) in zdirs {
        for entry in res!(std::fs::read_dir(&zdir.dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if !path.is_file() || ZoneDir::is_gc_temp_file(&path) {
                continue;
            }
            let (_fnum, ftyp) = match ZoneDir::ozone_file_number_and_type(&path) {
                Ok(pair) => pair,
                Err(_) => continue,
            };
            if ftyp == FileType::Index {
                let file = res!(OpenOptions::new().write(true).open(&path));
                res!(file.set_len(0));
                test!(sync_log::stream(), "Emptied index file {:?}.", path);
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Runs as its own integration binary, because it restarts the database and wants its own
/// directory while it does:
///
/// ```ignore
/// cargo test -p oxedyne_fe2o3_o3db_sync --test blind_index -- --nocapture
/// ```
#[test]
fn main() -> Outcome<()> {

    log_set_level!("test");

    let outcome = test_blind_index();

    log_finish_wait!();

    outcome
}

pub fn test_blind_index() -> Outcome<()> {

    let dir = "./test_db_blind_index";
    let db_root = res!(Path::new(dir).canonicalize().or_else(|_| {
        ok!(std::fs::create_dir_all(dir));
        Path::new(dir).canonicalize()
    }));

    // Fixed key so the test is deterministic.
    let enckey = [0x5au8; 32];
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
    // One zone and one bot of each kind: there is then exactly one live file, and the file
    // whose index is emptied is the same one the writer holds open.  That is the production
    // case, and with more files the detached-handle defect merely becomes intermittent.
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_scbots_per_zone     = 1;
    // Far larger than this test writes, so every record lands in file 1 and file 1 is still
    // "incomplete" on the restart -- which is what makes the writer take it as its live file
    // rather than starting a fresh one.
    cfg.data_file_max_bytes     = 1_000_000;
    // Values are tiny; keep chunking out of the way entirely.
    cfg.rest_chunk_threshold    = 100_000;
    cfg.rest_chunk_bytes        = 10_000;
    // Keep the zone inside this test's own root.  The default configuration sends zone 1 to a
    // container shared with the other tests.
    cfg.zone_overrides          = DaticleMap::new();

    test!(sync_log::stream(), "+---------------------------------------------+");
    test!(sync_log::stream(), "| BLIND INDEX TEST                            |");
    test!(sync_log::stream(), "+---------------------------------------------+");

    let zdirs: BTreeMap<ZoneInd, ZoneDir>;

    // ── Session 1: an emptied index must not read as an empty store ──
    {
        let mut db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            false,      // gc off: nothing here is about collection
            true,       // wipe: start from nothing
        ));

        for i in 0..KEYS {
            res!(db.insert(key(i), val(i), user, schms2));
        }
        thread::sleep(Duration::from_millis(500));

        zdirs = res!(db.api().get_zone_dirs());

        // The instrument works before the fault is introduced.  Without this, a scan that
        // failed for any other reason would satisfy the assertion below.
        let entries = res!(db.scan(&ScanOpts::all(), schms2));
        if entries.len() != KEYS {
            return Err(err!(
                "Before the index was touched, a scan returned {} entries, expected {}. \
                The rest of this test cannot mean anything.", entries.len(), KEYS;
                Test, Mismatch));
        }

        let (dat_bytes, ind_bytes) = res!(zone_bytes(&zdirs));
        test!(sync_log::stream(), "Zone holds {} bytes of data and {} bytes of index.",
            dat_bytes, ind_bytes);
        if dat_bytes == 0 {
            return Err(err!(
                "The zone holds no data at all, so emptying its index proves nothing.";
                Test, Missing, Data));
        }

        // 1. The fault: index files at zero, data files untouched.
        let emptied = res!(empty_all_index_files(&zdirs));
        if emptied == 0 {
            return Err(err!(
                "No index files were found to empty, so the fault was never introduced.";
                Test, Missing, File));
        }
        let (dat_after, ind_after) = res!(zone_bytes(&zdirs));
        if ind_after != 0 || dat_after != dat_bytes {
            return Err(err!(
                "After emptying, the zone should hold {} bytes of data and no index; it \
                holds {} and {}.", dat_bytes, dat_after, ind_after;
                Test, Mismatch, Data));
        }

        // A scan over a store whose records are all still there must not answer as though
        // they are not.  This is the assertion the old walk could not make: it returned an
        // empty list and `Ok`.
        match db.scan(&ScanOpts::all(), schms2) {
            Ok(entries) => return Err(err!(
                "A scan over {} bytes of records whose index files are empty answered \
                successfully with {} entries. A caller cannot tell that from an empty \
                store, and this is exactly how an operator's new limit was accepted and \
                then never enforced.", dat_bytes, entries.len();
                Test, Invalid, Data)),
            Err(e) => test!(sync_log::stream(),
                "Scan refused to answer over a blind zone, as it must: {}", e),
        }

        // And the signature that makes it dangerous: every record is still readable by key
        // throughout.  If this half fails, the store really was damaged and the test is
        // about something else.
        for i in 0..KEYS {
            match res!(db.get(&key(i), schms2)) {
                Some((v, _)) => if v != val(i) {
                    return Err(err!(
                        "Key {:?} read back as {:?}, expected {:?}.", key(i), v, val(i);
                        Test, Mismatch, Data));
                },
                None => return Err(err!(
                    "Key {:?} is unreadable, so the store is damaged rather than merely \
                    unscannable, and this test is measuring the wrong thing.", key(i);
                    Test, Missing, Data)),
            }
        }

        res!(db.shutdown());
    }

    thread::sleep(Duration::from_secs(1));

    // ── Session 2: the recovery repairs the index, and writes reach it ──
    {
        let mut db = res!(setup::start_db(
            db_root.clone(),
            Some(cfg.clone()),
            schms_input.clone(),
            None,
            false,      // gc off
            false,      // do not wipe: read what session 1 left
        ));

        // 2. Start-up rebuilt the index from the data file, so the zone scans again.
        let entries = res!(db.scan(&ScanOpts::all(), schms2));
        if entries.len() != KEYS {
            return Err(err!(
                "After the empty-index recovery, a scan returned {} entries, expected {}. \
                The index was not rebuilt from the data file.", entries.len(), KEYS;
                Test, Mismatch, Data));
        }

        let (_dat_rebuilt, ind_rebuilt) = res!(zone_bytes(&zdirs));
        if ind_rebuilt == 0 {
            return Err(err!(
                "After the empty-index recovery, the index files still hold no bytes.";
                Test, Missing, Data));
        }
        test!(sync_log::stream(), "Index rebuilt to {} bytes.", ind_rebuilt);

        // 3. THE SHARP ONE. A record written after the recovery must reach the index file
        //    on disk. When the rebuild renamed a fresh file over the index, the writer's
        //    append handle kept pointing at the replaced inode and this file never grew
        //    again for the life of the process -- while the data file took every record, so
        //    nothing else looked wrong.
        res!(db.insert(
            dat!(AFTER),
            dat!("written after the index was rebuilt"),
            user,
            schms2,
        ));
        thread::sleep(Duration::from_millis(500));

        let (_dat_after, ind_after) = res!(zone_bytes(&zdirs));
        if ind_after <= ind_rebuilt {
            return Err(err!(
                "A record was written after the empty-index recovery and the index files \
                went from {} bytes to {}. The write did not reach the index: it went to a \
                file handle the rebuild had detached, so it is on no index at all and no \
                scan will ever see it.", ind_rebuilt, ind_after;
                Test, Missing, Data));
        }

        // And the same thing said through the scan, which is where a caller would notice.
        let entries = res!(db.scan(&ScanOpts::all(), schms2));
        if entries.len() != KEYS + 1 {
            return Err(err!(
                "After the recovery and one further write, a scan returned {} entries, \
                expected {}.", entries.len(), KEYS + 1;
                Test, Mismatch, Data));
        }
        let after = dat!(AFTER);
        if !entries.iter().any(|(k, _v, _m)| *k == after) {
            return Err(err!(
                "The record written after the recovery is missing from the scan, though \
                the count came out right.";
                Test, Missing, Data));
        }

        res!(db.shutdown());
    }

    Ok(())
}
