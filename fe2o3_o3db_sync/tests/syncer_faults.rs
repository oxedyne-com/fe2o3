//! A writer's durability barrier runs on a thread of its own, its syncer, and these are the ways
//! that can go wrong: the disk failing its syncs, the syncer stopping, a new live pair that cannot
//! be handed to it, and a shutdown arriving while it holds records.  Each check says what the
//! store must tell its callers, and what must be on disk afterwards.  `test::hooks` makes the
//! faults, and the hooks are process-wide, which is why this is a test binary of its own with a
//! single test.  Found by QA of lane sto, 2026-09-23.

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
    comm::{
        msg::OzoneMsg,
        response::Wait,
    },
    data::core::RestSchemesInput,
    file::floc::FileNum,
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

const PERIOD_MS:        u64 = 200;                                  // the interval policy's
const WATCH:            Duration = Duration::from_millis(1_500);    // a failing disk watched
const ANSWERED_WITHIN:  Duration = Duration::from_secs(10);         // a closing store's last write
const SLOW_WRITES:      u8 = 20;                                    // arriving at a failing disk
const SLOW_BARRIERS:    u64 = 6;                                    // enough for them, and more

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    // Whatever fails, the next check and the next binary must not inherit the fault.
    let failing = failing_disk_is_retried_once_a_period();
    hooks::set_barrier_failure(false);
    let stopped = stopped_syncer_refuses_writes_before_appending();
    hooks::set_syncer_stop(false);
    let handoff = failed_hand_off_keeps_the_writer_on_its_pair();
    hooks::set_pair_hand_failure(false);
    let closing = close_answers_what_the_syncer_holds();
    hooks::set_barrier_delay(Duration::ZERO);
    let slowly = slowly_failing_disk_answers_waiting_writes_together();
    hooks::set_barrier_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    let queued = close_answers_what_a_slow_cache_bot_holds();
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_insert_delay(Duration::ZERO);
    log_finish_wait!();
    let failed: Vec<Error<ErrTag>> = [failing, stopped, handoff, closing, slowly, queued].into_iter()
        .filter_map(|r| r.err())
        .collect();
    match failed.len() {
        0 => Ok(()),
        1 => match failed.into_iter().next() {
            Some(e) => Err(e),
            None    => Ok(()), // unreachable
        },
        n => Err(err!(
            "{} checks failed: {:?}", n, failed;
            Test)),
    }
}

/// The disk fails every sync, under the interval policy, the library's default.  The barrier the
/// policy owed used to be retried as fast as it could fail: a million times in three seconds, on
/// nearly three cores, each failure logged.  It is retried once a period.  A write made while the
/// disk is failing waits on a barrier of its own and is told it failed, rather than being answered
/// as if its period will make it durable, and the store recovers with the disk.
fn failing_disk_is_retried_once_a_period() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_failing"));
    let mut cfg = res!(config());
    cfg.sync_interval_ms = PERIOD_MS;
    let db = res!(open(&root, cfg));
    // The first write is owed a barrier at once, and the disk is still good.
    res!(db.insert(key(1), dat!(1u8), Uid::default(), None));

    hooks::set_barrier_failure(true);
    let counted = hooks::barriers_failed();
    // Made within the period of that barrier, so released without one, leaving one owed.  Were
    // the machine slow enough for the period to pass first, this write would wait on a barrier of
    // its own and be told it failed; a barrier is owed either way, so the outcome is not the point.
    let _ = db.insert(key(2), dat!(2u8), Uid::default(), None);
    thread::sleep(WATCH);
    let tries = hooks::barriers_failed() - counted;
    let most = 2 * (WATCH.as_millis() as u64 / PERIOD_MS) + 2;
    if tries > most {
        let _ = db.close(); // the check has failed already, and says why
        return Err(err!(
            "While the disk failed every sync for {:?}, the owed barrier was tried {} times, where \
            once a {} ms period is at most {}: the syncer retries as fast as the disk fails.",
            WATCH, tries, PERIOD_MS, most;
            Test, Mismatch));
    }
    if tries < 2 {
        let _ = db.close(); // the check has failed already, and says why
        return Err(err!(
            "While the disk failed every sync for {:?}, the owed barrier was tried {} times: a \
            store that stops retrying never makes its writes durable once the disk recovers.",
            WATCH, tries;
            Test, Missing));
    }

    match db.insert(key(3), dat!(3u8), Uid::default(), None) {
        Ok(_) => {
            let _ = db.close(); // the check has failed already, and says why
            return Err(err!(
                "A write made while the disk failed every sync was answered as if its period \
                would make it durable.";
                Test, Unexpected));
        },
        Err(e) => {
            let text = fmt!("{:?}", e);
            if !text.contains("not confirmed durable") {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "A write made while the disk failed must be told its barrier failed, and it \
                    was told: {}", text;
                    Test, Mismatch));
            }
            // Written, so a caller must be able to tell it from a write that never landed
            // without reading the words.
            if !e.tags().contains(&ErrTag::Unconfirmed) {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "A write whose barrier failed after it was written reached its caller tagged \
                    {:?}, which does not say Unconfirmed: a caller cannot tell it from a write that \
                    never landed.", e.tags();
                    Test, Mismatch));
            }
        },
    }
    // The record is in the files all the same, and readable.
    match res!(db.get(&key(3), None)) {
        Some((v, _)) => req!(v, dat!(3u8), "A write whose barrier failed."),
        None => return Err(err!(
            "A write whose barrier failed is not readable, although it is in the files.";
            Test, Missing)),
    }

    // The disk recovers, and the store with it.
    hooks::set_barrier_failure(false);
    thread::sleep(Duration::from_millis(2 * PERIOD_MS));
    res!(db.insert(key(4), dat!(4u8), Uid::default(), None));
    res!(db.close());
    Ok(())
}

/// A writer's syncer stops, as one that panicked would.  Every later write through that writer is
/// refused before anything reaches the files.  Appended first, it was reported failed and came
/// back at the next start.
fn stopped_syncer_refuses_writes_before_appending() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_stopped"));
    let cfg = res!(config());
    let db = res!(open(&root, cfg.clone()));
    res!(db.insert(key(11), dat!(11u8), Uid::default(), None));

    hooks::set_syncer_stop(true);
    let counted = hooks::syncers_stopped();
    // Released, and then its syncer stops.
    res!(db.insert(key(12), dat!(12u8), Uid::default(), None));
    let begun = Instant::now();
    while hooks::syncers_stopped() == counted {
        if begun.elapsed() > constant::USER_REQUEST_TIMEOUT {
            let _ = db.close(); // the check has failed already, and says why
            return Err(err!("The syncer did not stop when told to."; Test, Timeout));
        }
        thread::sleep(Duration::from_millis(5));
    }
    hooks::set_syncer_stop(false);
    // A moment for its thread to end.
    thread::sleep(Duration::from_millis(100));

    let refused = match db.insert(key(13), dat!(13u8), Uid::default(), None) {
        Ok(_) => {
            let _ = db.close(); // the check has failed already, and says why
            return Err(err!(
                "A write through a writer whose syncer had stopped was confirmed.";
                Test, Unexpected));
        },
        Err(e) => {
            // Refused before anything was written, so a retry is needed, and nothing may say
            // otherwise.
            if e.tags().contains(&ErrTag::Unconfirmed) {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "A write refused before anything was written is tagged Unconfirmed, as a \
                    written one would be.";
                    Test, Mismatch));
            }
            fmt!("{:?}", e)
        },
    };
    res!(db.close());

    // Only what was confirmed is there after a restart.
    let db = res!(open(&root, cfg));
    for (k, v) in [(11u8, true), (12, true), (13, false)] {
        match (res!(db.get(&key(k), None)), v) {
            (Some(_), true) | (None, false) => (),
            (None, true) => {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "Write {}, confirmed before the syncer stopped, is missing after a restart.", k;
                    Test, Missing));
            },
            (Some(_), false) => {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "Write {}, reported failed because the syncer had stopped, came back after a \
                    restart: it had been written before it was refused.  It was told: {}",
                    k, refused;
                    Test, Unexpected));
            },
        }
    }
    res!(db.close());
    if !refused.contains("refused before anything was written") {
        return Err(err!(
            "A write refused because the syncer had stopped must say it was refused before \
            anything was written, and it said: {}", refused;
            Test, Mismatch));
    }
    Ok(())
}

/// Handing a writer's new live pair to its syncer fails, as running out of file descriptors would.
/// The writer stays on the pair its syncer holds.  It used to switch to the new pair first, so its
/// next records went to a file no barrier covered and its file bot never flagged live, while the
/// file it had left stayed flagged live for good.
fn failed_hand_off_keeps_the_writer_on_its_pair() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_handoff"));
    let cfg = res!(config());
    let db = res!(open(&root, cfg.clone()));
    let big = |i: u8| dat!(vec![i; 1_000]); // two of these overflow a 2,000 byte live file

    res!(db.insert(key(21), big(21), Uid::default(), None));
    hooks::set_pair_hand_failure(true);
    // This one needs a new live file, whose pair cannot be handed over.
    if db.insert(key(22), big(22), Uid::default(), None).is_ok() {
        let _ = db.close(); // the check has failed already, and says why
        return Err(err!(
            "A write needing a live pair that could not be handed to the syncer was confirmed.";
            Test, Unexpected));
    }
    // Small enough for the file the writer is on.
    res!(db.insert(key(23), dat!(23u8), Uid::default(), None));
    hooks::set_pair_hand_failure(false);
    // Two rollovers that work.
    res!(db.insert(key(24), big(24), Uid::default(), None));
    res!(db.insert(key(25), big(25), Uid::default(), None));

    // One file is flagged live, the one the writer is on.
    let newest = res!(newest_data_file(&root, &cfg));
    let states = res!(db.api().collect_file_states(Wait {
        max_wait:       constant::USER_REQUEST_TIMEOUT,
        check_interval: constant::CHECK_INTERVAL,
    }));
    let mut live: Vec<FileNum> = Vec::new();
    for (_, shard) in &states {
        for (fnum, fstat) in shard.map() {
            if fstat.is_live() {
                live.push(*fnum);
            }
        }
    }
    live.sort();
    if live != vec![newest] {
        let _ = db.close(); // the check has failed already, and says why
        return Err(err!(
            "After a live pair could not be handed to the syncer, files {:?} are flagged live, \
            where the writer is on file {} alone.", live, newest;
            Test, Mismatch));
    }
    res!(db.close());

    // After a restart, every confirmed write is there and the refused one is not.
    let db = res!(open(&root, cfg));
    for (k, v) in [(21u8, Some(big(21))), (22, None), (23, Some(dat!(23u8))), (24, Some(big(24))),
        (25, Some(big(25)))]
    {
        match (res!(db.get(&key(k), None)), v) {
            (Some((got, _)), Some(want)) => req!(got, want, "A write around a failed hand-off."),
            (None, None) => (),
            (got, want) => {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "Write {} around a failed hand-off: after a restart it is {:?}, where it \
                    should be {:?}.", k, got.map(|(v, _)| v), want;
                    Test, Mismatch));
            },
        }
    }
    res!(db.close());
    Ok(())
}

/// The store is closed while a syncer holds a written record, its barrier still running.  The
/// record is answered before the store's bots stop.  The cache bots used to be stopped first, and
/// nothing counted what a syncer held, so the record it released afterwards was never answered:
/// its caller waited out the durability deadline and was told that a durable write was not
/// confirmed.
fn close_answers_what_the_syncer_holds() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_close"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(open(&root, cfg.clone()));

    hooks::set_barrier_delay(Duration::from_millis(1_500));
    let writing = db.clone();
    let writer = res!(thread::Builder::new()
        .name(fmt!("syncer faults writer"))
        .spawn(move || -> Outcome<()> {
            let resp = res!(writing.api().store(key(31), dat!(31u8), Uid::default()));
            let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
                OzoneMsg::Chunks(n) => n,
                msg => return Err(err!(
                    "Expected the record count, received {:?}.", msg; Test, Unexpected)),
            };
            // A durability deadline short enough to report an unanswered record in seconds.
            res!(resp.recv_write_acks(n, constant::USER_REQUEST_TIMEOUT, ANSWERED_WITHIN));
            Ok(())
        }));
    // Written by now, and waiting on its barrier.
    thread::sleep(Duration::from_millis(300));
    let closed = db.close();
    hooks::set_barrier_delay(Duration::ZERO);
    res!(closed);
    match writer.join() {
        Ok(Ok(())) => (),
        Ok(Err(e)) => return Err(err!(e,
            "A write its syncer held when the store was closed was not answered."; Test, Missing)),
        Err(_) => return Err(err!("The writing thread panicked."; Test, Thread)),
    }

    // And it is on disk.
    let db = res!(open(&root, cfg));
    match res!(db.get(&key(31), None)) {
        Some((v, _)) => req!(v, dat!(31u8), "A write answered as the store closed."),
        None => {
            let _ = db.close(); // the check has failed already, and says why
            return Err(err!(
                "A write answered as the store closed is missing after a restart."; Test, Missing));
        },
    }
    res!(db.close());
    Ok(())
}

/// The disk fails every sync, slowly, under the interval policy, and twenty writes arrive at once.
/// While the last barrier has failed each write waits on a barrier, and those waiting together
/// share one, as they do under `sync_on_write`.  Each had a barrier of its own, one after
/// another, so a disk taking its time to fail answered twenty writes ten times slower, and with
/// writes arriving faster than it failed the queue grew without bound.
fn slowly_failing_disk_answers_waiting_writes_together() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_slowly"));
    let mut cfg = res!(config());
    cfg.sync_interval_ms = PERIOD_MS;
    let db = res!(open(&root, cfg));
    res!(db.insert(key(41), dat!(41u8), Uid::default(), None));

    hooks::set_barrier_delay(Duration::from_millis(PERIOD_MS));
    hooks::set_barrier_failure(true);
    // Released within the period of the barrier before it, and owed one, which fails.
    let _ = db.insert(key(42), dat!(42u8), Uid::default(), None);
    let begun = Instant::now();
    let counted = hooks::barriers_failed();
    while hooks::barriers_failed() == counted {
        if begun.elapsed() > constant::USER_REQUEST_TIMEOUT {
            let _ = db.close(); // the check has failed already, and says why
            return Err(err!("The barrier owed on a failing disk was never tried."; Test, Timeout));
        }
        thread::sleep(Duration::from_millis(5));
    }
    let counted = hooks::barriers_failed();
    let mut resps = Vec::new();
    for i in 0..SLOW_WRITES {
        resps.push(res!(db.api().store(key(50 + i), dat!(50 + i), Uid::default())));
    }
    let mut told = 0;
    for resp in resps {
        let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::Chunks(n) => n,
            msg => {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "Expected the record count, received {:?}.", msg; Test, Unexpected));
            },
        };
        if resp.recv_write_acks(n, constant::USER_REQUEST_TIMEOUT, ANSWERED_WITHIN).is_err() {
            told += 1;
        }
    }
    let tries = hooks::barriers_failed() - counted;
    hooks::set_barrier_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    res!(db.close());
    if told != SLOW_WRITES {
        return Err(err!(
            "{} of {} writes made while the disk failed were told their barrier failed.",
            told, SLOW_WRITES;
            Test, Mismatch));
    }
    if tries > SLOW_BARRIERS {
        return Err(err!(
            "{} writes arriving together while the disk failed each sync slowly took {} \
            barriers, where waiting together they share one, and a few more is the most: they \
            were answered one barrier at a time.", SLOW_WRITES, tries;
            Test, Mismatch));
    }
    Ok(())
}

/// The store is closed while its one cache bot works through a queue of written records, and the
/// shutdown's time runs out with one still queued behind the barrier that ends the writers.  That
/// record is answered.  Its queue used to be drained into the log when the time ran out, which
/// destroyed it, and its caller waited out the durability deadline for a write that had landed.
fn close_answers_what_a_slow_cache_bot_holds() -> Outcome<()> {
    let root = res!(fresh("./test_db_syncer_faults_queued"));
    let mut cfg = res!(config());
    cfg.num_cbots_per_zone = 1;
    cfg.sync_interval_ms = PERIOD_MS;
    let db = res!(open(&root, cfg.clone()));

    // The first write waits on the barrier the policy owes it at once, the second behind it, and
    // both reach the cache bot as the barrier ends, after the close has begun.
    hooks::set_barrier_delay(Duration::from_millis(2_000));
    hooks::set_insert_delay(Duration::from_millis(1_500));
    let mut writers = Vec::new();
    for i in [61u8, 62] {
        let writing = db.clone();
        writers.push(res!(thread::Builder::new()
            .name(fmt!("syncer faults writer {}", i))
            .spawn(move || -> Outcome<()> {
                let resp = res!(writing.api().store(key(i), dat!(i), Uid::default()));
                let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
                    OzoneMsg::Chunks(n) => n,
                    msg => return Err(err!(
                        "Expected the record count, received {:?}.", msg; Test, Unexpected)),
                };
                res!(resp.recv_write_acks(n, constant::USER_REQUEST_TIMEOUT, ANSWERED_WITHIN));
                Ok(())
            })));
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(100));
    let closed = db.close();
    let mut unanswered = Vec::new();
    for (i, writer) in writers.into_iter().enumerate() {
        match writer.join() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => unanswered.push(fmt!("write {}: {}", i + 1, e)),
            Err(_) => unanswered.push(fmt!("write {}: the writing thread panicked", i + 1)),
        }
    }
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_insert_delay(Duration::ZERO);
    res!(closed);
    if !unanswered.is_empty() {
        return Err(err!(
            "Writes queued at the cache bot when the store closed were not answered: {:?}",
            unanswered;
            Test, Missing));
    }
    let db = res!(open(&root, cfg));
    for i in [61u8, 62] {
        match res!(db.get(&key(i), None)) {
            Some((v, _)) => req!(v, dat!(i), "A write answered as the store closed."),
            None => {
                let _ = db.close(); // the check has failed already, and says why
                return Err(err!(
                    "Write {} answered as the store closed is missing after a restart.", i;
                    Test, Missing));
            },
        }
    }
    res!(db.close());
    Ok(())
}

/// The highest-numbered data file in the one zone.
fn newest_data_file(root: &Path, cfg: &OzoneConfig) -> Outcome<FileNum> {
    let zdir = cfg.zone_root(root).join("zone_001");
    let mut newest = None;
    for entry in res!(std::fs::read_dir(&zdir)) {
        let path = res!(entry).path();
        if path.extension().map(|e| e == constant::DATA_FILE_EXT).unwrap_or(false) {
            let digits: String = path.file_stem()
                .map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_digit()).collect())
                .unwrap_or_default();
            let fnum: FileNum = res!(digits.parse::<FileNum>());
            newest = Some(newest.map_or(fnum, |n: FileNum| n.max(fnum)));
        }
    }
    newest.ok_or_else(|| err!("There is no data file in {:?}.", zdir; Test, Missing))
}

fn key(i: u8) -> Dat {
    dat!(fmt!("syncer faults key {}", i))
}

/// One zone, so one writer and one syncer, with every zone inside the test's own directory and
/// no durability policy until a check sets one.
fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones       = 1;
    cfg.zone_overrides  = BTreeMap::new();
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
    let _ = std::fs::remove_dir_all(dir); // absent the first time
    res!(std::fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

fn open(root: &Path, cfg: OzoneConfig) -> Outcome<TestDb> {
    let mut db = res!(TestDb::new(root.to_path_buf(), Some(cfg), schemes(), Uid::default()));
    res!(db.start("test"));
    Ok(db)
}
