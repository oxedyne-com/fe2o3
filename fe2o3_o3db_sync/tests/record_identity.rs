//! A collection moves the records it keeps, and with records of one size a new offset can equal an
//! old one that something still refers to.  These checks read and supersede records around a
//! collection and judge every answer by the key and version the test wrote.  Found by QA of lane
//! sto, 2026-09-23 (F1, F2).  `test::hooks` holds collections open and delays a file bot, and the
//! hooks are process-wide, which is why this is a test binary of its own with a single test.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    Meta,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::response::Wait,
    data::{
        cache::Cache,
        core::RestSchemesInput,
    },
    file::{
        core::FileType,
        floc::FileLocation,
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

const VALUE_BYTES:  usize = 100;
const HOLD:         Duration = Duration::from_millis(1_500);    // a collection held open
const FORWARD:      Duration = Duration::from_secs(4);          // a supersession held back
const SETTLE:       Duration = Duration::from_secs(10);

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    // Whatever failed, the next check and the next binary must not inherit a slow bot.
    let queued = read_queued_behind_a_collection_returns_its_own_record();
    hooks::set_collect_delay(Duration::ZERO);
    let pending = new_offset_equal_to_a_pending_old_one_is_not_remapped();
    hooks::set_collect_delay(Duration::ZERO);
    hooks::set_forward_delay(Duration::ZERO);
    let carried = read_of_a_carried_record_leaves_its_move_for_its_supersession();
    hooks::set_collect_delay(Duration::ZERO);
    hooks::set_forward_delay(Duration::ZERO);
    log_finish_wait!();
    let failed: Vec<Error<ErrTag>> = [queued, pending, carried].into_iter()
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

/// Data file 1 holds a to e, a and b are superseded, and a read of c arrives while the collection
/// that follows is held open.  The read waits for the collection and then used the offset c had
/// before it, which in the rewritten file is where e now starts, and e's record passes its own
/// checksum: the read returned e's value for c.
fn read_queued_behind_a_collection_returns_its_own_record() -> Outcome<()> {
    let len = res!(record_len());
    let cfg = res!(config(1, 5 * len + len / 2));
    let root = res!(fresh("./test_db_record_identity_queued"));
    let db = res!(open(&root, cfg));
    res!(fill_file_1(&db, &root, len));

    res!(db.insert(key(0), value(0, 2), Uid::default(), None));
    hooks::set_collect_delay(HOLD);
    // Two of the five records superseded, past the collection trigger.
    res!(db.insert(key(1), value(1, 2), Uid::default(), None));
    res!(wait_for_collection(&db, 1));
    // The read goes to the file, and reaches its file bot while the collection is held.
    res!(clear_values(&db));
    let got = db.get(&key(2), None);
    hooks::set_collect_delay(Duration::ZERO);

    let result = judge(got, 2, 1, "A read that waited for a collection of its file");
    res!(db.close());
    result
}

/// With a file bot for odd files and one for even, file 1's collection is held open while c is
/// superseded by a write to file 2, whose file bot is slow to pass the supersession on.  So the
/// collection carries c, and leaves a move entry at c's old offset for the supersession to find
/// when it arrives.  In the rewritten file e starts at that same offset.  A read of e was matched
/// to c's move entry by offset alone and returned c's old value; the supersession, arriving to
/// find its entry gone, then flagged e's record as old, and the next collection dropped it.
fn new_offset_equal_to_a_pending_old_one_is_not_remapped() -> Outcome<()> {
    let len = res!(record_len());
    let cfg = res!(config(2, 5 * len + len / 2));
    let root = res!(fresh("./test_db_record_identity_pending"));
    let db = res!(open(&root, cfg.clone()));
    res!(fill_file_1(&db, &root, len));
    let f1 = data_file(&root, 1);

    res!(db.insert(key(0), value(0, 2), Uid::default(), None));
    hooks::set_collect_delay(HOLD);
    res!(db.insert(key(1), value(1, 2), Uid::default(), None));
    res!(wait_for_collection(&db, 1));
    // Superseded before the collection updates the caches, and passed on after it has finished.
    hooks::set_forward_delay(FORWARD);
    let forwarded = Instant::now();
    res!(db.insert(key(2), value(2, 2), Uid::default(), None));
    hooks::set_collect_delay(Duration::ZERO);
    // c, d and e are carried: c to where a was, e to where c was.
    if !settle_to(&f1, 3 * len) {
        let _ = db.close();
        return Err(err!(
            "Data file 1 did not settle at the three records its collection carries, {} bytes, \
            within {:?}: it is {} bytes.", 3 * len, SETTLE, size(&f1);
            Test, Timeout));
    }
    res!(clear_values(&db));
    let early = judge(db.get(&key(4), None), 4, 1,
        "A read of e at the offset c had before the collection");
    if forwarded.elapsed() >= FORWARD {
        let _ = db.close();
        return Err(err!(
            "The read of e came {:?} after c's supersession was held back for {:?}, so the check \
            did not test what it says.", forwarded.elapsed(), FORWARD;
            Test, Timeout));
    }
    // The supersession of c arrives, and is applied to its own record.
    thread::sleep(FORWARD.saturating_sub(forwarded.elapsed()) + Duration::from_millis(500));
    hooks::set_forward_delay(Duration::ZERO);

    // d superseded too: a second collection of file 1, which carries e alone.
    res!(db.insert(key(3), value(3, 2), Uid::default(), None));
    let _ = settle_to(&f1, len); // judged by the reads below
    res!(clear_values(&db));
    let late = judge(db.get(&key(4), None), 4, 1, "A read of e after a second collection");
    res!(db.close());

    let db = res!(open_again(&root, cfg));
    let mut after = Vec::new();
    for (i, ver) in [(0, 2), (1, 2), (2, 2), (3, 2), (4, 1), (5, 1)] {
        after.push(judge(db.get(&key(i), None), i, ver, "A read after a restart"));
    }
    res!(db.close());
    res!(early);
    res!(late);
    for result in after {
        res!(result);
    }
    Ok(())
}

/// As in the check before, file 1's collection is held open while c is superseded by a write to
/// file 2 whose supersession is held back, so the collection carries c and leaves its move entry
/// for that supersession.  This time a read of c, given c's old offset before the write, waits for
/// the collection and is remapped through c's entry.  A read that spent the entry left the
/// supersession to find it gone, and the supersession then flagged the record at c's old offset,
/// which is e's now, as old: the next collection dropped e, and kept the c it had been sent for.
fn read_of_a_carried_record_leaves_its_move_for_its_supersession() -> Outcome<()> {
    let len = res!(record_len());
    let cfg = res!(config(2, 5 * len + len / 2));
    let root = res!(fresh("./test_db_record_identity_carried"));
    let db = res!(open(&root, cfg.clone()));
    res!(fill_file_1(&db, &root, len));
    let f1 = data_file(&root, 1);

    res!(db.insert(key(0), value(0, 2), Uid::default(), None));
    hooks::set_collect_delay(HOLD);
    res!(db.insert(key(1), value(1, 2), Uid::default(), None));
    res!(wait_for_collection(&db, 1));
    // The read takes c's old offset from the cache bot and waits at the file bot.
    res!(clear_values(&db));
    let reading = db.clone();
    let reader = res!(thread::Builder::new().name(fmt!("record identity reader")).spawn(
        move || reading.get(&key(2), None)));
    thread::sleep(Duration::from_millis(100));
    // Superseded before the collection updates the caches, and passed on after it has finished.
    hooks::set_forward_delay(FORWARD);
    let forwarded = Instant::now();
    res!(db.insert(key(2), value(2, 2), Uid::default(), None));
    hooks::set_collect_delay(Duration::ZERO);
    let early = match reader.join() {
        // Asked before the write, so the version it was given, whichever that was.
        Ok(got) => match judge(got.clone(), 2, 1, "A read of c queued behind the collection") {
            Ok(()) => Ok(()),
            Err(_) => judge(got, 2, 2, "A read of c queued behind the collection"),
        },
        Err(_) => Err(err!("The reading thread panicked."; Test, Thread)),
    };
    if !settle_to(&f1, 3 * len) {
        let _ = db.close();
        return Err(err!(
            "Data file 1 did not settle at the three records its collection carries, {} bytes, \
            within {:?}: it is {} bytes.", 3 * len, SETTLE, size(&f1);
            Test, Timeout));
    }
    if forwarded.elapsed() >= FORWARD {
        let _ = db.close();
        return Err(err!(
            "The collection ended {:?} after c's supersession was held back for {:?}, so the \
            check did not test what it says.", forwarded.elapsed(), FORWARD;
            Test, Timeout));
    }
    // The supersession of c arrives, and is applied to its own record.
    thread::sleep(FORWARD.saturating_sub(forwarded.elapsed()) + Duration::from_millis(500));
    hooks::set_forward_delay(Duration::ZERO);

    // d superseded too: a second collection of file 1, which carries e alone.
    res!(db.insert(key(3), value(3, 2), Uid::default(), None));
    let _ = settle_to(&f1, len); // judged by the reads below
    res!(clear_values(&db));
    let late = judge(db.get(&key(4), None), 4, 1, "A read of e after a second collection");
    res!(db.close());

    let db = res!(open_again(&root, cfg));
    let mut after = Vec::new();
    for (i, ver) in [(0, 2), (1, 2), (2, 2), (3, 2), (4, 1), (5, 1)] {
        after.push(judge(db.get(&key(i), None), i, ver, "A read after a restart"));
    }
    res!(db.close());
    res!(early);
    res!(late);
    for result in after {
        res!(result);
    }
    Ok(())
}

/// A key with two records in one collected file, the older carried because its supersession has
/// yet to arrive, and the cache naming the newer.  The collection's update for the older record
/// leaves the cached location alone, and the newer record's update moves it and gives back where
/// it was, for its move entry to be spent.  Matched by file alone, the older record's update
/// moved the location to the older record's new offset, and the entries were spent against the
/// wrong offsets.
#[test]
fn reanchor_moves_only_the_record_the_cache_names() -> Outcome<()> {
    let mut cache = Cache::<{ UID_LEN }, Uid>::new(None);
    let k = res!(key(9).as_bytes());
    let older = Meta { time: res!(Timestamp::now()), user: Uid::default() };
    thread::sleep(Duration::from_millis(2));
    let newer = Meta { time: res!(Timestamp::now()), user: Uid::default() };
    let at = |start: u64| FileLocation { fnum: 1, start, klen: 30, vlen: 100 };
    res!(cache.insert(k.clone(), None, at(260), older.clone()));
    res!(cache.insert(k.clone(), None, at(390), newer.clone()));
    // The collection carries both, the older to 0 and the newer to 130.
    if let Some(was) = cache.reanchor(&k, &at(0), &older) {
        return Err(err!(
            "The update for the older of a key's two records in a collected file moved the \
            location the cache holds for the newer, from {:?} to offset 0.", was;
            Test, Mismatch));
    }
    match cache.reanchor(&k, &at(130), &newer) {
        Some(was) if was.start == 390 => Ok(()),
        other => Err(err!(
            "The update for the newer of a key's two records in a collected file gave back {:?} \
            as where the cache had it, where that was offset 390.", other;
            Test, Mismatch)),
    }
}

/// Writes a to e, which fill data file 1, and f, which seals it.
fn fill_file_1(db: &TestDb, root: &Path, len: u64) -> Outcome<()> {
    for i in 0..6 {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    let (f1, f2) = (data_file(root, 1), data_file(root, 2));
    if !f2.is_file() || size(&f1) != 5 * len {
        return Err(err!(
            "Data file 1 should hold five records of {} bytes and be sealed by the sixth, but it \
            is {} bytes and file 2 {}.", len, size(&f1),
            if f2.is_file() { "exists" } else { "does not exist" };
            Test, Invalid, Size));
    }
    Ok(())
}

/// The value read must be the one written for this key at this version.
fn judge(
    got:    Outcome<Option<(Dat, Meta<{ UID_LEN }, Uid>)>>,
    i:      usize,
    ver:    u8,
    what:   &str,
)
    -> Outcome<()>
{
    match got {
        Ok(Some((v, _))) => match parse(&v) {
            Some((j, w)) if j == i && w == ver => Ok(()),
            Some((j, w)) => Err(err!(
                "{}: key {} returned the value written for key {} at version {}, where it should \
                be version {} of its own.", what, i, j, w, ver;
                Test, Mismatch)),
            None => Err(err!(
                "{}: key {} returned a value the test never wrote: {:?}.", what, i, v;
                Test, Mismatch)),
        },
        Ok(None) => Err(err!(
            "{}: key {} is missing, where it should be version {}.", what, i, ver;
            Test, Missing)),
        Err(e) => Err(err!(e,
            "{}: key {} could not be read.", what, i;
            Test, Read)),
    }
}

/// The length of one record, every record here being that length.
fn record_len() -> Outcome<u64> {
    let root = res!(fresh("./test_db_record_identity_probe"));
    let db = res!(open(&root, res!(config(1, 2_000))));
    res!(db.insert(key(0), value(0, 1), Uid::default(), None));
    res!(db.close());
    let len = size(&data_file(&root, 1));
    if len == 0 {
        return Err(err!("The probe record left data file 1 empty."; Test, Missing));
    }
    Ok(len)
}

/// One zone, one writer, one cache bot and one reader, so that every step lands where the check
/// says, and each record synced as it is written.
fn config(nf: u16, max: u64) -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones               = 1;
    cfg.num_cbots_per_zone      = 1;
    cfg.num_fbots_per_zone      = nf;
    cfg.num_igbots_per_zone     = 1;
    cfg.num_rbots_per_zone      = 1;
    cfg.num_wbots_per_zone      = 1;
    cfg.data_file_max_bytes     = max;
    cfg.rest_chunk_threshold    = max * 7 / 10;
    cfg.zone_overrides          = BTreeMap::new();
    cfg.sync_on_write           = true;
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

fn open(root: &Path, cfg: OzoneConfig) -> Outcome<TestDb> {
    setup::start_db(root.to_path_buf(), Some(cfg), schemes(), None, true, true)
}

fn open_again(root: &Path, cfg: OzoneConfig) -> Outcome<TestDb> {
    setup::start_db(root.to_path_buf(), Some(cfg), schemes(), None, true, false)
}

/// Every key has the same length, and so every record.
fn key(i: usize) -> Dat {
    dat!(fmt!("record identity key {}", i))
}

/// A value that says which key and version it was written for.
fn value(i: usize, ver: u8) -> Dat {
    let mut v = vec![0u8; VALUE_BYTES];
    v[0] = i as u8;
    v[1] = ver;
    for j in 2..VALUE_BYTES {
        v[j] = (i as u8) ^ ver ^ (j as u8);
    }
    Dat::BU32(v)
}

fn parse(d: &Dat) -> Option<(usize, u8)> {
    let v = d.bytes_ref()?;
    if v.len() != VALUE_BYTES {
        return None;
    }
    let (i, ver) = (v[0], v[1]);
    for j in 2..VALUE_BYTES {
        if v[j] != i ^ ver ^ (j as u8) {
            return None;
        }
    }
    Some((i as usize, ver))
}

fn clear_values(db: &TestDb) -> Outcome<()> {
    db.api().clear_cache_values(Wait {
        max_wait:       constant::USER_REQUEST_TIMEOUT,
        check_interval: constant::CHECK_INTERVAL,
    })
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
        thread::sleep(Duration::from_millis(10));
    }
    Err(err!(
        "Superseding two of the five records in data file {} did not start its collection \
        within {:?}.", fnum, SETTLE;
        Test, Timeout))
}

/// Does the file reach the given size before the wait runs out?
fn settle_to(path: &Path, want: u64) -> bool {
    let begun = Instant::now();
    loop {
        if size(path) == want {
            return true;
        }
        if begun.elapsed() >= SETTLE {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn data_file(root: &Path, fnum: u32) -> PathBuf {
    let zones = match config(1, 2_000) {
        Ok(cfg) => cfg.zone_root(root),
        Err(_)  => root.to_path_buf(), // not reached: the fixture was built from the same config
    };
    zones.join("zone_001").join(ZoneDir::relative_file_path(&FileType::Data, fnum))
}

fn size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m)   => m.len(),
        Err(_)  => 0,
    }
}

/// An empty directory of this test's own.
fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = std::fs::remove_dir_all(dir); // absent the first time
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}
