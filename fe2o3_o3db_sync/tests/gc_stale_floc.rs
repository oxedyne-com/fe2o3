//! A key carried through a garbage collection must keep reading back, every time.
//!
//! # The fault
//!
//! Garbage collection transcribes a sealed data file to a temporary, then renames the temporary
//! over it (`InitGarbageBot::collect_file`, step 7).  The rename replaces the path, so every
//! surviving record now sits at a new offset in a new inode, and the old inode is unlinked.  Two
//! caches have to follow that.
//!
//! The location cache does follow it: `cache_data_file` sends each carried record's new location
//! to its cbot, which re-anchors the entry (`Cache::update_if_same_fnum`), and any location still
//! in flight is mapped through the file state's ephemeral old -> new move map.
//!
//! The reader's file cache does not.  `ReaderBot::get_file` keeps an open `File` per file number
//! for `constant::FILE_CACHE_EXPIRY_SECS` -- fifteen minutes -- and nothing invalidates it when a
//! collection renames that file away.  A reader that had read the file before the collection goes
//! on reading the unlinked old inode, at the NEW offsets.  Those offsets are not record boundaries
//! in the old file, so the bytes are not a record, and the checksum verify in `ReaderBot::read`
//! fails: `api.rs get_wait` -> `bot_reader.rs` -> `fe2o3_hash csum.rs` -> `[Checksum][Input
//! Mismatch]`.  That is the production symptom exactly, and it explains its shape -- key-specific
//! (only keys in a collected file), intermittent (only readers that touched the file first), on a
//! store that is byte-perfect on disk, cleared by a cold start, and re-made by the next collection.
//!
//! The `postgc` flag is the signal that would have caught it, and it is discarded: the fbot sets it
//! when a location came through the move map, the rbot passes it up through `Value` and
//! `Responder::recv_daticle`, and `api.rs get_wait` drops it on the floor.
//!
//! # What the tests do
//!
//! Each case writes one survivor key behind a few fillers -- behind, because a survivor at the head
//! of its file is transcribed to the head of the new file and never moves -- then fills the zone's
//! data files, supersedes enough fillers to push the survivor's file past the 30% old-data
//! collection trigger, and reads the survivor back.  Cached values are cleared first
//! (`clear_cache_values`), so every read must go to the file: this is the production shape, where
//! values are jettisoned under memory pressure or never loaded and only locations are held.
//!
//!  - `no_compaction`  -- the control: the same writes and churn with collection off.  Nothing
//!                        shrinks and every read is clean.
//!  - `some_survive`   -- half the fillers superseded, so the compacted file keeps several records.
//!  - `only_survivor`  -- every filler superseded, so the survivor is the one record carried over.
//!                        Neither of these two reads the survivor until the collection has
//!                        finished, so the reader opens the new inode and both pass: that is what
//!                        localises the fault to a handle opened beforehand rather than to the
//!                        location or to the file on disk.
//!  - `read_during_compaction` -- the repro.  A reader hammers the survivor while the supersession
//!                        burst drives collection, which is the shape a live gateway is in.  The
//!                        reads are clean until the rename, fail from the rename onwards, still
//!                        fail once everything has settled, and are clean again after a restart.
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
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::constant,
    comm::response::Wait,
    data::core::RestSchemesInput,
    prelude::{
        Checksummer,
        Encrypter,
        Hasher,
    },
    test::setup,
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
            AtomicUsize,
            Ordering,
        },
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

const VALUE_BYTES:  usize = 200;    // well under the chunking threshold: one record per value
const NPRE:         usize = 5;      // fillers written ahead of the survivor in its own data file
const NFILL:        usize = 60;     // enough fillers to seal several data files behind the survivor
const NREADS:       usize = 6;      // a first read plus five more, so a read-once fault is visible
const GC_TIMEOUT:   Duration = Duration::from_secs(30);

/// Which supersession pattern the case drives, and hence whether a collection runs at all.
#[derive(Clone, Copy, Debug)]
enum Case {
    /// Half the fillers superseded: the compacted file keeps several records beside the survivor.
    SomeSurvive,
    /// Every filler superseded: the survivor is the only record carried into the new file.
    OnlySurvivor,
    /// The control.  Collection is off, so nothing moves and every read must be clean.
    NoCompaction,
}

impl Case {
    fn name(&self) -> &'static str {
        match self {
            Self::SomeSurvive   => "some_survive",
            Self::OnlySurvivor  => "only_survivor",
            Self::NoCompaction  => "no_compaction",
        }
    }

    fn gc_on(&self) -> bool {
        match self {
            Self::SomeSurvive   |
            Self::OnlySurvivor  => true,
            Self::NoCompaction  => false,
        }
    }

    /// Is the filler at this index superseded during the churn phase?  Every filler written ahead
    /// of the survivor is superseded whatever the case, because those are the records whose
    /// removal shifts the survivor to a new offset: a survivor at the head of its file is carried
    /// to the head of the transcribed file and never moves, which would prove nothing.
    fn supersedes(&self, i: usize) -> bool {
        if i < NPRE {
            return true;
        }
        match self {
            Self::SomeSurvive   => i % 2 == 0,
            Self::OnlySurvivor  => true,
            Self::NoCompaction  => i % 2 == 0,
        }
    }
}

fn wait() -> Wait {
    Wait {
        max_wait:       Duration::from_secs(30),
        check_interval: constant::CHECK_INTERVAL,
    }
}

/// A byte string of the given size, filled deterministically so each version is distinct on disk.
fn value_of(seed: u8, len: usize) -> Dat {
    let mut v = vec![0u8; len];
    for (i, b) in v.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    Dat::BU32(v)
}

fn survivor_key() -> Dat { dat!("gcrace:survivor") }
fn survivor_val() -> Dat { value_of(0x5a, VALUE_BYTES) }
fn filler_key(i: usize) -> Dat { dat!(fmt!("gcrace:fill:{:04}", i)) }

/// Every data file under `root`, with its current length.
fn data_file_sizes(root: &Path) -> Outcome<BTreeMap<PathBuf, u64>> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in res!(fs::read_dir(&dir)) {
            let path = res!(entry).path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == constant::DATA_FILE_EXT).unwrap_or(false) {
                let len = res!(fs::metadata(&path)).len();
                found.insert(path, len);
            }
        }
    }
    Ok(found)
}

/// A compaction rewrites a sealed data file in place, so it shows up as a file present before the
/// churn whose length has fallen.  Polls until one is seen or the timeout expires.
fn wait_for_compaction(
    root:       &Path,
    before:     &BTreeMap<PathBuf, u64>,
    timeout:    Duration,
)
    -> Outcome<Option<(PathBuf, u64, u64)>>
{
    let start = Instant::now();
    loop {
        let now = res!(data_file_sizes(root));
        for (path, was) in before {
            if let Some(is) = now.get(path) {
                if is < was {
                    return Ok(Some((path.clone(), *was, *is)));
                }
            }
        }
        if start.elapsed() > timeout {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Reads the survivor once, insisting on the exact bytes written.  The read index is carried so
/// that a failure says which read of the sequence broke: the reads before a collection are clean
/// and the reads after it are not, which is what names the rename as the moment of damage.
fn read_survivor<
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    db:     &O3db<{ setup::UID_LEN }, setup::Uid, ENC, KH, PR, CS>,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
    label:  &str,
    n:      usize,
)
    -> Outcome<()>
{
    match db.get(&survivor_key(), schms2) {
        Err(e) => Err(err!(e,
            "[{}] Read {} of {} of the survivor key failed.  A store whose records are all \
            intact on disk can only fail a read this way by landing somewhere that is not a \
            record boundary -- a location that was not re-anchored after the collection, or a \
            file handle still open on the inode the collection renamed away.",
            label, n, NREADS;
            Test, Data)),
        Ok(None) => Err(err!(
            "[{}] Read {} of {} of the survivor key found nothing, but it was written and \
            never deleted.", label, n, NREADS;
            Test, Missing, Data)),
        Ok(Some((got, _))) => if got == survivor_val() {
            Ok(())
        } else {
            Err(err!(
                "[{}] Read {} of {} of the survivor key returned different bytes from those \
                written.", label, n, NREADS;
                Test, Invalid, Data))
        },
    }
}

fn run_case(case: Case) -> Outcome<()> {

    let label = case.name();
    let dirname = fmt!("./test_db_gc_stale_floc_{}", label);
    let db_root = res!(canonical_dir(&dirname));

    let enckey = [0x5cu8; 32];
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

    // One zone and one bot of each kind, so the survivor's routing is fixed and a compaction is
    // not spread over zones the test would have to reason about.  The data file is small enough
    // that the fillers seal several files behind the survivor, and the values stay well under the
    // chunking threshold so every write takes the single-record path.  Writes are synced so that
    // the collector's "has this file drained?" check passes promptly rather than on a timer.
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_rbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.data_file_max_bytes     = 4_000;
    cfg.rest_chunk_threshold    = 1_500;
    cfg.rest_chunk_bytes        = 64;
    cfg.init_load_caches        = true;
    cfg.sync_on_write           = true;
    cfg.zone_overrides          = DaticleMap::new();

    test!(sync_log::stream(), "+--- gc stale floc: {} ---", label);

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        case.gc_on(),
        true, // wipe
    ));
    thread::sleep(Duration::from_millis(500));

    // 1. A few fillers go in ahead of the survivor, so the survivor sits part way into the zone's
    //    first data file rather than at its head.  Superseding those puts the survivor at a
    //    different offset in the transcribed file, which is the whole subject of the test.
    for i in 0..NPRE {
        res!(db.insert(filler_key(i), value_of(i as u8, VALUE_BYTES), user, schms2));
    }
    res!(db.insert(survivor_key(), survivor_val(), user, schms2));

    // 2. The rest of the fillers seal that file and several after it.
    for i in NPRE..NFILL {
        res!(db.insert(filler_key(i), value_of(i as u8, VALUE_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_millis(500));

    // 3. Clear cached values, so from here a read of any key must go to its file.  This is the
    //    production shape: only locations are held, and a stale one cannot hide behind a cached
    //    copy of the value.
    res!(db.api().clear_cache_values(wait()));

    let before = res!(data_file_sizes(&db_root));
    if before.len() < 3 {
        let _ = db.shutdown();
        return Err(err!(
            "[{}] Only {} data files were written; the configuration should have sealed \
            several behind the survivor, so this case would prove nothing.", label, before.len();
            Test, Size));
    }

    // 4. Supersede the fillers.  Each overwrite flags the previous record old in its sealed file,
    //    and once a file passes the 30% old-data trigger the collector transcribes it, carrying
    //    the survivor to a new offset.
    for i in 0..NFILL {
        if case.supersedes(i) {
            res!(db.insert(filler_key(i), value_of((i + 128) as u8, VALUE_BYTES), user, schms2));
        }
    }

    // 5. Wait for a compaction to actually land before reading anything.
    let compacted = res!(wait_for_compaction(
        &db_root,
        &before,
        if case.gc_on() { GC_TIMEOUT } else { Duration::from_secs(3) },
    ));
    match (case.gc_on(), &compacted) {
        (true, None) => {
            let _ = db.shutdown();
            return Err(err!(
                "[{}] No data file shrank within {:?}, so no compaction ran and this case \
                would prove nothing about a location that survived one.", label, GC_TIMEOUT;
                Test, Missing));
        },
        (true, Some((path, was, is))) => test!(sync_log::stream(),
            "[{}] Compacted {:?}: {} -> {} bytes.", label, path, was, is),
        (false, Some((path, was, is))) => {
            let _ = db.shutdown();
            return Err(err!(
                "[{}] The control ran with collection off, but {:?} still shrank from {} to \
                {} bytes.", label, path, was, is;
                Test, Invalid));
        },
        (false, None) => test!(sync_log::stream(),
            "[{}] Control: no data file shrank, as expected with collection off.", label),
    }
    thread::sleep(Duration::from_millis(500));

    // Clear again, so the reads below cannot be answered from a value cached by the churn.
    res!(db.api().clear_cache_values(wait()));

    // 6. Read the survivor repeatedly.  No read has touched this file before now, so the reader
    //    opens it fresh and these reads are clean -- which is the half of the evidence that says
    //    the location re-anchor works and the transcribed file is right.
    let mut first_failure: Option<(usize, Error<ErrTag>)> = None;
    for n in 1..=NREADS {
        match read_survivor(&db, schms2, label, n) {
            Ok(()) => test!(sync_log::stream(), "[{}] Read {} of {}: clean.", label, n, NREADS),
            Err(e) => {
                test!(sync_log::stream(), "[{}] Read {} of {}: FAILED.", label, n, NREADS);
                if first_failure.is_none() {
                    first_failure = Some((n, e));
                }
            },
        }
    }

    let _ = db.shutdown();
    thread::sleep(Duration::from_millis(300));

    match first_failure {
        None => {
            test!(sync_log::stream(),
                "[{}] All {} reads of the survivor returned the bytes written.", label, NREADS);
            Ok(())
        },
        Some((n, e)) => Err(err!(e,
            "[{}] The survivor key stopped reading back at read {} of {}.", label, n, NREADS;
            Test, Data)),
    }
}

/// A reader hammering one key while a supersession burst drives collection.  This is the shape a
/// live gateway is in, and it is the one that matters: the reader opens the survivor's data file
/// before the collection renames a transcribed copy over it, and from the rename onwards it is
/// reading an unlinked inode at offsets that belong to the file that replaced it.  The reads after
/// everything settles say whether the damage lasts, and the reads after a restart say whether the
/// store on disk was ever at fault.
fn read_during_compaction() -> Outcome<()> {

    let label = "read_during_compaction";
    let db_root = res!(canonical_dir("./test_db_gc_stale_floc_concurrent"));

    let enckey = [0x5cu8; 32];
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
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_rbots_per_zone      = 2;
    cfg.num_wbots_per_zone      = 1;
    cfg.data_file_max_bytes     = 4_000;
    cfg.rest_chunk_threshold    = 1_500;
    cfg.rest_chunk_bytes        = 64;
    cfg.init_load_caches        = true;
    cfg.sync_on_write           = true;
    cfg.zone_overrides          = DaticleMap::new();

    test!(sync_log::stream(), "+--- gc stale floc: {} ---", label);

    let db = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        true, // gc on
        true, // wipe
    ));
    thread::sleep(Duration::from_millis(500));

    for i in 0..NPRE {
        res!(db.insert(filler_key(i), value_of(i as u8, VALUE_BYTES), user, schms2));
    }
    res!(db.insert(survivor_key(), survivor_val(), user, schms2));
    for i in NPRE..NFILL {
        res!(db.insert(filler_key(i), value_of(i as u8, VALUE_BYTES), user, schms2));
    }
    thread::sleep(Duration::from_millis(500));
    res!(db.api().clear_cache_values(wait()));

    let before = res!(data_file_sizes(&db_root));

    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    // Every fault is recorded and the reader carries on, because whether the damage is one blip
    // per collection or a key that stays unreadable is the difference between an in-flight
    // location and a cached one, and only a reader that keeps going can tell them apart.
    let faults: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let outcome = thread::scope(|scope| {
        let stop_r = Arc::clone(&stop);
        let reads_r = Arc::clone(&reads);
        let faults_r = Arc::clone(&faults);
        let db_r = &db;
        scope.spawn(move || {
            while !stop_r.load(Ordering::Relaxed) {
                let n = reads_r.fetch_add(1, Ordering::Relaxed) + 1;
                let fault = match db_r.get(&survivor_key(), schms2) {
                    Ok(Some((got, _))) => if got == survivor_val() {
                        None
                    } else {
                        Some(fmt!("Read {} returned different bytes from those written.", n))
                    },
                    Ok(None) => Some(fmt!(
                        "Read {} found nothing under a key that was never deleted.", n)),
                    Err(e) => Some(fmt!("Read {} failed: {}", n, e)),
                };
                if let Some(msg) = fault {
                    let mut slot = lock_mutex_thread!(faults_r, "concurrent reader");
                    slot.push(msg);
                }
                thread::sleep(Duration::from_millis(2));
            }
        });

        // Drive the collection underneath the reader.
        let mut result = Ok(());
        for i in 0..NFILL {
            if let Err(e) = db.insert(
                filler_key(i),
                value_of((i + 128) as u8, VALUE_BYTES),
                user,
                schms2,
            ) {
                result = Err(e);
                break;
            }
        }
        thread::sleep(Duration::from_secs(3));
        stop.store(true, Ordering::Relaxed);
        result
    });
    res!(outcome);

    let compacted = res!(wait_for_compaction(&db_root, &before, Duration::from_secs(5)));
    let nreads = reads.load(Ordering::Relaxed);

    let fault_list = {
        let slot = lock_mutex!(faults);
        slot.clone()
    };

    // Once everything has settled, read the survivor again.  A one-off race on a location that was
    // in flight during the transcription leaves nothing behind, so these reads would be clean; a
    // cached handle on the renamed-away inode keeps failing until it expires or the process ends.
    let mut after_settled = Vec::new();
    for n in 1..=NREADS {
        if let Err(e) = read_survivor(&db, schms2, label, n) {
            after_settled.push(fmt!("{}", e));
        }
    }
    // The file states carry the move map, so a dump here says whether an unconsumed old -> new
    // entry is still sitting on the file the survivor lives in.
    res!(db.api().dump_file_states(wait()));

    // Every read must hand its file back.  The fbot will not collect a file whose reader count is
    // above zero, so a read that returned early without sending its completion leaves a count that
    // never comes down and a file that can never be collected again -- a burst of failures here
    // was seen to strand a count in the thousands.  Nothing is reading by now, so every count must
    // be zero.
    let mut stranded = Vec::new();
    for (wind, fstates) in res!(db.api().collect_file_states(wait())) {
        for (fnum, fstat) in fstates.map() {
            if fstat.readers() != 0 {
                stranded.push(fmt!("{} file {} holds {} reader(s)", wind, fnum, fstat.readers()));
            }
        }
    }

    let _ = db.shutdown();
    thread::sleep(Duration::from_millis(300));

    // The same store reopened: the caches are rebuilt from the index files, so a fault that
    // clears here was in memory and not on disk.  This is the cold start that clears the
    // production symptom, and it is what says the store itself is intact.
    let db2 = res!(setup::start_db(
        db_root.clone(),
        Some(cfg.clone()),
        schms_input.clone(),
        None,
        false, // gc off: nothing should move during the check
        false, // keep what is there
    ));
    thread::sleep(Duration::from_millis(500));
    res!(db2.api().clear_cache_values(wait()));
    let mut after_restart = Vec::new();
    for n in 1..=NREADS {
        if let Err(e) = read_survivor(&db2, schms2, label, n) {
            after_restart.push(fmt!("{}", e));
        }
    }
    let _ = db2.shutdown();
    thread::sleep(Duration::from_millis(300));
    test!(sync_log::stream(),
        "[{}] After a restart, {} of {} reads of the survivor failed.",
        label, after_restart.len(), NREADS);

    match compacted {
        None => return Err(err!(
            "[{}] No data file shrank, so no compaction ran under the reader and this case \
            would prove nothing.", label;
            Test, Missing)),
        Some((path, was, is)) => test!(sync_log::stream(),
            "[{}] Compacted {:?}: {} -> {} bytes, under {} concurrent reads, {} of which \
            failed.  {} of {} reads after everything settled failed.",
            label, path, was, is, nreads, fault_list.len(), after_settled.len(), NREADS),
    }

    for msg in fault_list.iter().take(3) {
        test!(sync_log::stream(), "[{}] During collection: {}", label, msg);
    }
    for msg in after_settled.iter().take(3) {
        test!(sync_log::stream(), "[{}] After settling: {}", label, msg);
    }

    if !fault_list.is_empty() || !after_settled.is_empty() {
        return Err(err!(
            "[{}] {} of {} reads of the survivor broke while collection was running, and {} of \
            {} after it settled.  First: {}",
            label, fault_list.len(), nreads, after_settled.len(), NREADS,
            match fault_list.first() {
                Some(msg) => msg.clone(),
                None => match after_settled.first() {
                    Some(msg) => msg.clone(),
                    None => fmt!("none"),
                },
            };
            Test, Data));
    }

    if !stranded.is_empty() {
        return Err(err!(
            "[{}] Every read has finished, but {} file state(s) still hold a reader count: {}.  \
            A read that returns early without telling its fbot strands the count, and the fbot \
            will never collect that file again.", label, stranded.len(), stranded.join("; ");
            Test, Data, Mismatch));
    }

    test!(sync_log::stream(),
        "[{}] {} reads of the survivor during collection all returned the bytes written, and \
        every file state is back to zero readers.", label, nreads);
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

pub fn test_gc_stale_floc(_filter: &'static str) -> Outcome<()> {
    res!(run_case(Case::NoCompaction));
    res!(run_case(Case::SomeSurvive));
    res!(run_case(Case::OnlySurvivor));
    res!(read_during_compaction());
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    log_set_level!("debug");
    let outcome = test_gc_stale_floc("all");
    log_finish_wait!();
    outcome
}
