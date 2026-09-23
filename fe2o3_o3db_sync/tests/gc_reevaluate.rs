//! A file whose garbage the collector passed over once kept it for good (2026-09-23).  Two
//! faults, found because the online orphan sweep reclaimed a different fraction of what it retired
//! on every run, and on some runs too little:
//!
//! - A file bot applied a supersession it handled itself, rather than one sent to it by another
//!   file bot, straight to its file even while the file was being collected.  The collection's
//!   result then replaced the file's state, so the record was carried into the rewritten file as
//!   current, with a move entry nothing would clear, and a file holding a move entry is never
//!   collected again.
//! - Collection was considered only when a supersession arrived.  A file that passed the trigger
//!   while it was live, when it could not be collected, was not looked at again when it was
//!   sealed.
//!
//! Each is reproduced on one zone with one file bot, so that every supersession is one the bot
//! handles itself, and judged by the size of the data file on disk.  The second is checked twice,
//! once with the file drained when it is sealed and once with its last records still on their way,
//! which only their landing can follow up.  `test::hooks` holds collections open, so that
//! supersessions land during one or a file can be measured before one, and holds durability
//! barriers; the hooks are process-wide, which is why this is a test binary of its own.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    data::core::RestSchemesInput,
    file::{
        core::FileType,
        zdir::ZoneDir,
    },
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
        },
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

const FILE_BYTES:   u64 = 16_000;   // the collection trigger is 30% of this, 4,800 bytes
const VALUE_BYTES:  usize = 1_000;  // five superseded records pass the trigger, four do not
const NKEYS:        usize = 10;     // most of the first file
const NFILL:        usize = 8;      // seals the first file part way through
const HOLD:         Duration = Duration::from_millis(1_500);
const SETTLE:       Duration = Duration::from_secs(120); // a collection's syncs on a busy disk

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    // Whatever failed, the next check and the next binary must not inherit a slow collector.
    let during = supersession_during_collection_is_kept();
    hooks::set_collect_delay(Duration::ZERO);
    let sealed = garbage_made_while_live_is_collected_once_sealed();
    hooks::set_collect_delay(Duration::ZERO);
    let landed = garbage_is_collected_when_the_last_write_lands();
    hooks::set_collect_delay(Duration::ZERO);
    hooks::set_barrier_delay(Duration::ZERO);
    log_finish_wait!();
    let failed: Vec<Error<ErrTag>> = [during, sealed, landed].into_iter()
        .filter_map(|r| r.err())
        .collect();
    match failed.len() {
        0 => Ok(()),
        1 => match failed.into_iter().next() {
            Some(e) => Err(e),
            None    => Ok(()), // unreachable
        },
        n => Err(err!(
            "{} of the 3 checks failed: {:?}", n, failed;
            Test)),
    }
}

/// Half of ten records in a sealed file are superseded, which starts its collection, and the other
/// half while that collection is held open.  All ten must be reclaimed: the second half by a second
/// collection, once the first has finished and the supersessions it held back are applied.  Before,
/// the second half was lost and the file kept those five records for good.
fn supersession_during_collection_is_kept() -> Outcome<()> {
    let root = res!(fresh("./test_db_gc_reevaluate_during"));
    let db = res!(open(&root));
    let f1 = res!(data_file(&root, 1));

    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, VALUE_BYTES), Uid::default(), None));
    }
    let kbytes = size(&f1);
    for i in 0..NFILL {
        res!(db.insert(filler(i), value(i, VALUE_BYTES), Uid::default(), None));
    }
    let sealed = res!(sealed_size(&root, &f1));

    hooks::set_collect_delay(HOLD);
    for i in 0..(NKEYS / 2) {
        res!(db.insert(key(i), value(i + 100, VALUE_BYTES), Uid::default(), None));
    }
    res!(wait_for_collection(&db, 1));
    for i in (NKEYS / 2)..NKEYS {
        res!(db.insert(key(i), value(i + 100, VALUE_BYTES), Uid::default(), None));
    }
    hooks::set_collect_delay(Duration::ZERO);

    let want = res!(garbage_removed(sealed, kbytes));
    let got = settle_to(&f1, want);
    if got != want {
        let _ = db.close();
        return Err(err!(
            "Data file 1 held {} bytes of ten superseded records and {} of fillers when sealed. \
            With every record superseded, half of them while it was being collected, it should \
            have settled at {} bytes, but after {:?} it is {}: {} bytes of superseded records \
            were never reclaimed.",
            kbytes, want, want, SETTLE, got, got.saturating_sub(want);
            Test, Mismatch, Size));
    }
    for i in 0..NKEYS {
        match res!(db.get(&key(i), None)) {
            Some((v, _)) => req!(v, value(i + 100, VALUE_BYTES), "A value carried through two collections."),
            None => return Err(err!("Key {} is missing after two collections.", i; Test, Missing)),
        }
    }
    for i in 0..NFILL {
        match res!(db.get(&filler(i), None)) {
            Some((v, _)) => req!(v, value(i, VALUE_BYTES), "A filler carried through two collections."),
            None => return Err(err!("Filler {} is missing after two collections.", i; Test, Missing)),
        }
    }
    res!(db.close());
    test!(sync_log::stream(), "Supersessions made during a collection were kept: {} -> {} bytes.", sealed, got);
    Ok(())
}

/// Ten records are superseded while their file is still live, which pushes it past the collection
/// trigger when it cannot be collected, and the file is then sealed with nothing in it superseded
/// again.  The ten must still be reclaimed.  Before, nothing looked at the file after the seal.
fn garbage_made_while_live_is_collected_once_sealed() -> Outcome<()> {
    let root = res!(fresh("./test_db_gc_reevaluate_sealed"));
    let db = res!(open(&root));
    let f1 = res!(data_file(&root, 1));

    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, VALUE_BYTES), Uid::default(), None));
    }
    let kbytes = size(&f1);
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 8), Uid::default(), None));
    }
    // The seal may start the collection at once, so it is held until the file has been measured.
    hooks::set_collect_delay(HOLD);
    for i in 0..NFILL {
        res!(db.insert(filler(i), value(i, VALUE_BYTES), Uid::default(), None));
    }
    let sealed = res!(sealed_size(&root, &f1));
    hooks::set_collect_delay(Duration::ZERO);

    let want = res!(garbage_removed(sealed, kbytes));
    let got = settle_to(&f1, want);
    if got != want {
        let _ = db.close();
        return Err(err!(
            "Data file 1 was sealed at {} bytes, {} of them ten records superseded while it was \
            live.  It should have settled at {} bytes, but after {:?} it is {}.",
            sealed, kbytes, want, SETTLE, got;
            Test, Mismatch, Size));
    }
    for i in 0..NKEYS {
        match res!(db.get(&key(i), None)) {
            Some((v, _)) => req!(v, value(i, 8), "A value written while its file was live."),
            None => return Err(err!("Key {} is missing after the collection.", i; Test, Missing)),
        }
    }
    res!(db.close());
    test!(sync_log::stream(), "Garbage made while live was collected once sealed: {} -> {} bytes.", sealed, got);
    Ok(())
}

/// As the previous check, but the file's last records are still on their way to their file bot
/// when it is sealed: every durability barrier is held, so the seal finds the file not yet drained
/// and cannot collect it.  The last of those records landing must.  Before, nothing looked at the
/// file again.
fn garbage_is_collected_when_the_last_write_lands() -> Outcome<()> {
    let root = res!(fresh("./test_db_gc_reevaluate_landed"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(setup::start_db(root.clone(), Some(cfg), schemes(), None, true, true));
    let f1 = res!(data_file(&root, 1));

    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, VALUE_BYTES), Uid::default(), None));
    }
    let kbytes = size(&f1);
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 8), Uid::default(), None));
    }
    // The fillers are sent together, so the one that seals the file is written while the records
    // before it wait on their barrier.  The collection is held until the file has been measured.
    hooks::set_barrier_delay(HOLD);
    hooks::set_collect_delay(HOLD);
    let mut resps = Vec::new();
    for i in 0..NFILL {
        resps.push(res!(db.api().store(filler(i), value(i, VALUE_BYTES), Uid::default())));
    }
    // Measured once sealed, while its last records still wait on their barrier.
    let sealed = res!(wait_until_sealed(&root, &f1));
    for resp in resps {
        res!(resp.recv_store_ack());
    }
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_collect_delay(Duration::ZERO);

    let want = res!(garbage_removed(sealed, kbytes));
    let got = settle_to(&f1, want);
    if got != want {
        let _ = db.close();
        return Err(err!(
            "Data file 1 was sealed at {} bytes before its last records had landed, {} of them \
            ten records superseded while it was live.  It should have settled at {} bytes once \
            they landed, but after {:?} it is {}.",
            sealed, kbytes, want, SETTLE, got;
            Test, Mismatch, Size));
    }
    for i in 0..NFILL {
        match res!(db.get(&filler(i), None)) {
            Some((v, _)) => req!(v, value(i, VALUE_BYTES), "A filler that sealed a collected file."),
            None => return Err(err!("Filler {} is missing after the collection.", i; Test, Missing)),
        }
    }
    res!(db.close());
    test!(sync_log::stream(), "Garbage was collected when the last write landed: {} -> {} bytes.", sealed, got);
    Ok(())
}

fn key(i: usize) -> Dat { dat!(fmt!("gc reevaluate key {:02}", i)) }
fn filler(i: usize) -> Dat { dat!(fmt!("gc reevaluate filler {:02}", i)) }

/// A byte string of the given length, distinct for each seed.
fn value(seed: usize, len: usize) -> Dat {
    let mut v = vec![0u8; len];
    for (i, b) in v.iter_mut().enumerate() {
        *b = (seed as u8).wrapping_add((i % 251) as u8);
    }
    Dat::BU32(v)
}

/// One zone and one bot of each kind, with values well under the chunking threshold, so that every
/// write is one record in the first zone and every supersession is handled by the one file bot.
fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = 1;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_rbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.data_file_max_bytes     = FILE_BYTES;
    cfg.rest_chunk_threshold    = 3_000;  // well over VALUE_BYTES
    cfg.zone_overrides          = BTreeMap::new();
    Ok(cfg)
}

fn schemes() -> RestSchemesInput<(), HashScheme, HashScheme, ChecksumScheme> {
    RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    )
}

/// An empty directory of this test's own.
fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path) -> Outcome<TestDb> {
    setup::start_db(
        root.to_path_buf(),
        Some(res!(config())),
        schemes(),
        None,
        true,   // collection on
        true,   // wipe
    )
}

fn data_file(root: &Path, fnum: u32) -> Outcome<PathBuf> {
    let cfg = res!(config());
    Ok(cfg.zone_root(root).join("zone_001").join(ZoneDir::relative_file_path(&FileType::Data, fnum)))
}

/// A file that has been collected away entirely is no longer there.
fn size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// The size of data file 1 once the writer has moved on to file 2, after which it cannot grow.
fn sealed_size(root: &Path, f1: &Path) -> Outcome<u64> {
    let f2 = res!(data_file(root, 2));
    if !f2.is_file() {
        return Err(err!(
            "The fillers did not seal data file 1: there is no data file 2 at {:?}.", f2;
            Test, Missing));
    }
    Ok(size(f1))
}

/// Waits for the writer to move on to data file 2, then gives the size of data file 1.
fn wait_until_sealed(root: &Path, f1: &Path) -> Outcome<u64> {
    let f2 = res!(data_file(root, 2));
    let begun = Instant::now();
    while !f2.is_file() {
        if begun.elapsed() >= SETTLE {
            return Err(err!(
                "The fillers did not seal data file 1 within {:?}: there is no data file 2 at {:?}.",
                SETTLE, f2;
                Test, Timeout));
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(size(f1))
}

/// The size a sealed file should settle at once its superseded records are gone.
fn garbage_removed(sealed: u64, garbage: u64) -> Outcome<u64> {
    match sealed.checked_sub(garbage) {
        Some(want) => Ok(want),
        None => Err(err!(
            "Data file 1 was sealed at {} bytes, less than the {} its first records took.",
            sealed, garbage;
            Test, Invalid, Size)),
    }
}

/// Waits until the file bot has handed file `fnum` to a collector.
fn wait_for_collection(db: &TestDb, fnum: u32) -> Outcome<()> {
    let begun = Instant::now();
    while begun.elapsed() < SETTLE {
        let states = res!(db.api().collect_file_states(Wait {
            max_wait:       constant::USER_REQUEST_TIMEOUT,
            check_interval: constant::CHECK_INTERVAL,
        }));
        for (_, shard) in &states {
            if let Some(fstat) = shard.map().get(&fnum) {
                if fstat.gc_active() {
                    return Ok(());
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(err!(
        "Superseding half the records in data file {} did not start its collection within {:?}.",
        fnum, SETTLE;
        Test, Timeout))
}

/// Waits for the file to reach the given size, returning the size it has when it does or when
/// the wait runs out.
fn settle_to(path: &Path, want: u64) -> u64 {
    let begun = Instant::now();
    loop {
        let got = size(path);
        if got == want || begun.elapsed() >= SETTLE {
            return got;
        }
        thread::sleep(Duration::from_millis(100));
    }
}
