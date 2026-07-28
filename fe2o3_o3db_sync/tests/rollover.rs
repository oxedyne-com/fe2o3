//! Ozone data file rollover and supersession accounting test.
//!
//! A zone seals its live data file when the next record would push it past
//! `data_file_max_bytes`, and the writer bot asks its zone bot for the next
//! file number.  Every record written into a file is recorded in that file's
//! `FileState.dmap`, and when the record is superseded by a later write of the
//! same key the entry is flipped from `Cur` to `Old`, which is what tells the
//! garbage collector how much of the file is reclaimable.
//!
//! This test drives many rollovers with a tiny file size limit and repeatedly
//! overwrites a small set of keys, so that supersessions land in files that
//! have already been sealed.  It then asserts the accounting invariant:
//!
//! * one `dmap` entry per record written,
//! * exactly one `Cur` entry per distinct key,
//! * every other entry `Old`.
//!
//! When the zone hands a writer a file number that is already in use, the
//! receiving file bot replaces that file's `FileState` wholesale and the
//! entries recorded so far are lost.  Every later supersession of one of those
//! records then fails in `FileState::register_old` with "a data entry starting
//! at position N in the FileState was not found", garbage is never registered,
//! and the store grows without bound.  The invariant below catches that.

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
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::{
        cfg::OzoneConfig,
        constant,
        index::WorkerInd,
    },
    data::core::RestSchemesInput,
    file::state::{
        DataState,
        FileStateMap,
    },
    test::setup,
    O3db,
};

use std::{
    collections::BTreeMap,
    fs,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::Duration,
};

/// Number of distinct keys churned through the rollover.
const NKEYS:    usize = 40;
/// Number of times each key is overwritten after its first write.
const NOVER:    usize = 12;

pub fn test_rollover(_filter: &'static str) -> Outcome<()> {

    let db_root = res!(canonical_dir("./test_db_rollover"));

    let enckey = [0x7bu8; 32];
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

    // One writer bot per zone is the simple case: a rollover can only ever
    // collide with the writer's own live file.
    res!(supersession_accounting(
        "one writer per zone",
        &db_root,
        &schms_input,
        schms2,
        user,
        1,
    ));
    // Two writer bots per zone is the shipped default, and is the case where a
    // reused file number can be handed to a writer other than the one that
    // already holds it.
    res!(supersession_accounting(
        "two writers per zone",
        &db_root,
        &schms_input,
        schms2,
        user,
        2,
    ));

    res!(gc_reclaims_sealed_files(
        &db_root,
        &schms_input,
        schms2,
        user,
    ));

    test!(sync_log::stream(), "Rollover test passed.");
    Ok(())
}

/// Build the shared test configuration: tiny data files so rollovers happen
/// every few records, and garbage collection under the caller's control.
fn rollover_cfg(nwbots: u16) -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 2;
    cfg.num_fbots_per_zone      = 2;
    cfg.num_igbots_per_zone     = 2;
    cfg.num_wbots_per_zone      = nwbots;
    cfg.data_file_max_bytes     = 4_000;
    cfg.rest_chunk_threshold    = 3_000;
    cfg.rest_chunk_bytes        = 1_000;
    cfg.zone_overrides = mapdat!{
        1u16 => mapdat!{ "dir" => "", "max_size" => 100_000_000u64 },
    }.get_map().unwrap();
    Ok(cfg)
}

/// Writes `NKEYS` keys `NOVER + 1` times each through many rollovers, then
/// checks that every record written is still accounted for in some file state,
/// and that exactly one record per key is current.
fn supersession_accounting(
    label:       &str,
    db_root:     &PathBuf,
    schms_input: &RestSchemesInput<
                     EncryptionScheme,
                     HashScheme,
                     HashScheme,
                     ChecksumScheme,
                 >,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
    nwbots:      u16,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- rollover: {} ---", label);

    let cfg = res!(rollover_cfg(nwbots));
    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off: nothing is retired, so the accounting is exact.
        true,  // wipe: start from an empty zone directory.
    ));

    thread::sleep(Duration::from_millis(200));

    let mut nwrites = 0usize;
    for round in 0..(NOVER + 1) {
        for k in 0..NKEYS {
            res!(db.insert(
                dat!(fmt!("rk{:03}", k)),
                dat!(fmt!("rv{:03}r{:02}", k, round)),
                user,
                schms2,
            ));
            nwrites += 1;
        }
    }

    // Let the trailing UpdateData and ScheduleOld messages drain.
    thread::sleep(Duration::from_millis(500));

    // Read every key back, to confirm the rollover left the data readable.
    for k in 0..NKEYS {
        let key = dat!(fmt!("rk{:03}", k));
        match res!(db.get(&key, schms2)) {
            Some((val, _meta)) => {
                let expected = dat!(fmt!("rv{:03}r{:02}", k, NOVER));
                if val != expected {
                    return Err(err!(
                        "{}: key {} round-tripped as {:?}, expected {:?}.",
                        label, k, val, expected;
                        Test, Mismatch));
                }
            },
            None => return Err(err!(
                "{}: key {} not found after {} writes.", label, k, nwrites;
                Test, Missing)),
        }
    }

    let states = res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT));
    let (nfiles, ncur, nold, noldcnt) = count_data_states(&states);

    test!(sync_log::stream(),
        "{}: {} writes over {} tracked files: {} current, {} old, {} tracked \
        total, {} counted old.",
        label, nwrites, nfiles, ncur, nold, ncur + nold, noldcnt);

    if ncur + nold != nwrites {
        return Err(err!(
            "{}: {} records were written but the file states track {} \
            ({} current + {} old). Entries lost from a FileState can never be \
            flagged old, so their bytes can never be reclaimed.",
            label, nwrites, ncur + nold, ncur, nold;
            Test, Mismatch, Data));
    }
    if ncur != NKEYS {
        return Err(err!(
            "{}: {} distinct keys were written but {} records are flagged \
            current.", label, NKEYS, ncur;
            Test, Mismatch, Data));
    }
    if noldcnt != nold {
        return Err(err!(
            "{}: {} records are flagged old in the record maps but the \
            old-record counters total {}. The two disagree, so the \
            reclaimable byte total the collector trusts is wrong.",
            label, nold, noldcnt;
            Test, Mismatch, Data));
    }
    res!(assert_no_bot_errors(&db, label));

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));

    test!(sync_log::stream(), "+--- rollover: {} : passed ---", label);
    Ok(())
}

/// With garbage collection on, heavy overwrite churn over many sealed files
/// must actually shrink the zone directory rather than let it grow with every
/// superseded copy.
fn gc_reclaims_sealed_files(
    db_root:     &PathBuf,
    schms_input: &RestSchemesInput<
                     EncryptionScheme,
                     HashScheme,
                     HashScheme,
                     ChecksumScheme,
                 >,
    schms2:      Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>,
    user:        setup::Uid,
)
    -> Outcome<()>
{
    test!(sync_log::stream(), "+--- rollover: garbage collection ---");

    let cfg = res!(rollover_cfg(1));
    let mut db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true, // gc on.
        true, // wipe.
    ));

    thread::sleep(Duration::from_millis(200));

    // Enough churn that the superseded copies vastly outweigh the live set.
    let rounds = 40;
    let mut nwrites = 0usize;
    for round in 0..rounds {
        for k in 0..NKEYS {
            res!(db.insert(
                dat!(fmt!("gk{:03}", k)),
                dat!(fmt!("gv{:03}r{:02}", k, round)),
                user,
                schms2,
            ));
            nwrites += 1;
        }
    }

    // Give the gbots time to finish transcribing.
    thread::sleep(Duration::from_secs(2));

    let live_bytes = res!(zone_data_bytes(&db_root));
    let states = res!(db.api().collect_file_states(constant::USER_REQUEST_WAIT));
    let (_, ncur, nold, _) = count_data_states(&states);

    test!(sync_log::stream(),
        "gc: {} writes, {} bytes of data files remain, {} current and {} old \
        records tracked.", nwrites, live_bytes, ncur, nold);

    // Every key was written `rounds` times; without reclamation the data files
    // hold every copy. Allow generous slack for records not yet collected, but
    // require that the great majority of the dead bytes are gone.
    let all_copies = res!(zone_data_bytes_estimate(nwrites, NKEYS, live_bytes, ncur + nold));
    if live_bytes >= all_copies {
        return Err(err!(
            "Garbage collection reclaimed nothing: {} bytes of data files \
            remain after {} writes of {} keys, which is no better than \
            retaining every superseded copy.",
            live_bytes, nwrites, NKEYS;
            Test, Mismatch, Data));
    }
    if nold > nwrites - NKEYS {
        return Err(err!(
            "More records are flagged old ({}) than were superseded ({}).",
            nold, nwrites - NKEYS;
            Test, Mismatch, Data));
    }
    res!(assert_no_bot_errors(&db, "garbage collection"));

    res!(db.shutdown());
    thread::sleep(Duration::from_millis(200));

    test!(sync_log::stream(), "+--- rollover: garbage collection : passed ---");
    Ok(())
}

/// Fails if any bot in the database has logged an error. The flag-as-old
/// failure this test exists for is logged by a file bot rather than returned
/// to the caller, so this is how the log storm itself is asserted away.
fn assert_no_bot_errors<
    ENC:    oxedyne_fe2o3_iop_crypto::enc::Encrypter + 'static,
    KH:     oxedyne_fe2o3_iop_hash::api::Hasher + 'static,
    PR:     oxedyne_fe2o3_iop_hash::api::Hasher + 'static,
    CS:     oxedyne_fe2o3_iop_hash::csum::Checksummer + 'static,
>(
    db:     &O3db<{ setup::UID_LEN }, setup::Uid, ENC, KH, PR, CS>,
    label:  &str,
)
    -> Outcome<()>
{
    let (errs, nbots) = res!(db.api().bot_error_count(constant::USER_REQUEST_WAIT));
    test!(sync_log::stream(), "{}: {} bots reported {} errors.", label, nbots, errs);
    if errs > 0 {
        return Err(err!(
            "{}: the {} bots of the database logged {} errors during the run. \
            A supersession that cannot be registered is logged, not returned, \
            so any error here means the rollover bookkeeping is still wrong.",
            label, nbots, errs;
            Test, Mismatch, Data));
    }
    Ok(())
}

/// Sums the tracked record counts across every file bot in every zone,
/// returning the number of tracked files and the current and old record counts.
fn count_data_states(
    states: &BTreeMap<WorkerInd, FileStateMap>,
)
    -> (usize, usize, usize, usize)
{
    let mut nfiles  = 0;
    let mut ncur    = 0;
    let mut nold    = 0;
    let mut noldcnt = 0;
    for (_wind, fstates) in states {
        for (_fnum, fstat) in fstates.map() {
            nfiles += 1;
            noldcnt += fstat.get_old_count();
            for (_start, dstat) in fstat.data_map() {
                match dstat {
                    DataState::Cur => ncur += 1,
                    DataState::Old => nold += 1,
                }
            }
        }
    }
    (nfiles, ncur, nold, noldcnt)
}

/// Total size in bytes of the data files under the database root.
fn zone_data_bytes(db_root: &Path) -> Outcome<u64> {
    let mut total = 0u64;
    res!(walk_data_files(db_root, &mut total));
    Ok(total)
}

/// Recursively sums the size of every `.dat` file beneath `dir`.
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

/// Estimates the bytes an uncollected store would hold, from the mean record
/// size implied by what is on disk now.
fn zone_data_bytes_estimate(
    nwrites:    usize,
    _nkeys:     usize,
    live_bytes: u64,
    ntracked:   usize,
)
    -> Outcome<u64>
{
    if ntracked == 0 {
        return Err(err!(
            "No records are tracked in any file state, so no size estimate \
            can be made.";
            Test, Missing, Data));
    }
    let mean = live_bytes / (ntracked as u64);
    Ok(mean * (nwrites as u64))
}

/// Canonicalise a directory path, creating it if it does not exist.
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
