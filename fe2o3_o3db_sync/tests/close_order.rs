//! A close finishes the bots in the order they depend on one another: the readers, scanners and
//! collectors that ask the cache and file bots for things, then the writers, whose syncers release
//! records to the cache bots, and only then the cache and file bots.  Each check holds one bot or
//! barrier up past the shutdown's three seconds and says what the callers waiting on it must be
//! told.  The hooks are process-wide, hence a binary of its own with a single test.  Found by QA
//! of lane o3i, 2026-09-24.

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
    comm::msg::OzoneMsg,
    data::core::RestSchemesInput,
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
    sync::{
        Arc,
        Mutex,
        atomic::{
            AtomicBool,
            Ordering,
        },
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

const HOLD:         Duration = Duration::from_millis(4_000);    // past the shutdown's 3 s
const READERS:      u64 = 8;                                    // threads, one read each queued
const KEYS:         u64 = 20;
const CLOSE_WITHIN: Duration = Duration::from_secs(8);          // one read timing out adds 5 s
const DEADLINE:     Duration = Duration::from_secs(15);         // stands in for the 120 s one
const TOLD_WITHIN:  Duration = Duration::from_secs(10);

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    // Whatever fails, the next check must not inherit the fault.
    let reads = close_under_read_load_answers_the_reads_queued();
    hooks::set_insert_delay(Duration::ZERO);
    let failed = close_tells_a_write_its_barrier_failed();
    hooks::set_barrier_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    let slow = close_confirms_a_write_behind_a_slow_barrier();
    hooks::set_barrier_delay(Duration::ZERO);
    log_finish_wait!();
    let failed: Vec<Error<ErrTag>> = [reads, failed, slow].into_iter()
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

/// The store is closed while eight reads wait at its one reader bot behind a cache bot held by an
/// insert.  Every read asked before the close is answered with its value, and the close takes
/// about as long as the hold.  The cache bots used to be finished while reads were still queued
/// at the reader bot, and once nothing was drained any more each of those reads waited out
/// `BOT_REQUEST_TIMEOUT` on a cache bot that had ended: 38.65 s for this close, where it took
/// 4.70 s when the queue was drained.
fn close_under_read_load_answers_the_reads_queued() -> Outcome<()> {
    let root = res!(fresh("./test_db_close_order_reads"));
    let db = res!(open(&root, res!(config())));
    for i in 0..KEYS {
        res!(db.insert(key(i), dat!(i), Uid::default(), None));
    }

    hooks::set_insert_delay(HOLD);
    let writing = db.clone();
    let writer = res!(thread::Builder::new().name(fmt!("close order writer")).spawn(move || {
        // Its answer is not the point: it holds the cache bot.
        let _ = writing.insert(key(KEYS), dat!(KEYS), Uid::default(), None);
    }));
    thread::sleep(Duration::from_millis(50));

    let stop = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(Mutex::new(Vec::new())); // (asked at, key, answer)
    let mut readers = Vec::new();
    for r in 0..READERS {
        let (db, stop, asked) = (db.clone(), stop.clone(), asked.clone());
        readers.push(res!(thread::Builder::new().name(fmt!("close order reader {}", r)).spawn(
            move || {
                let mut i = r;
                while !stop.load(Ordering::Relaxed) {
                    let at = Instant::now();
                    let answer = match db.get(&key(i % KEYS), None) {
                        Ok(Some((v, _))) if v == dat!(i % KEYS) => Ok(()),
                        Ok(Some((v, _))) => Err(fmt!("the value {:?}", v)),
                        Ok(None) => Err(fmt!("nothing")),
                        Err(e) => Err(fmt!("{}", e)),
                    };
                    if let Ok(mut asked) = asked.lock() {
                        asked.push((at, i % KEYS, answer));
                    }
                    i += 1;
                }
            })));
    }
    thread::sleep(Duration::from_millis(300));

    let begun = Instant::now();
    let closed = db.close();
    let took = begun.elapsed();
    stop.store(true, Ordering::Relaxed);
    hooks::set_insert_delay(Duration::ZERO);
    for reader in readers {
        let _ = reader.join();
    }
    let _ = writer.join();
    res!(closed);
    let asked = match asked.lock() {
        Ok(asked) => asked.clone(),
        Err(_) => return Err(err!("A reader thread panicked holding the answers."; Test, Thread)),
    };
    let queued: Vec<_> = asked.iter().filter(|(at, _, _)| *at < begun).collect();
    msg!("A close with {} reads queued behind a cache bot held for {:?} took {:?}.",
        queued.len(), HOLD, took);
    let wrong: Vec<_> = queued.iter()
        .filter_map(|(_, k, a)| a.as_ref().err().map(|e| fmt!("key {}: {}", k, e)))
        .collect();
    if !wrong.is_empty() {
        return Err(err!(
            "Of {} reads asked before the store was closed, which took {:?}, {} were not \
            answered with their value: {:?}", queued.len(), took, wrong.len(), wrong;
            Test, Missing));
    }
    if took > CLOSE_WITHIN {
        return Err(err!(
            "A close with {} reads queued behind a cache bot held for {:?} took {:?}, where it \
            should take about as long as the hold and at most {:?}.",
            queued.len(), HOLD, took, CLOSE_WITHIN;
            Test, Timeout));
    }
    Ok(())
}

/// The store is closed while a written record waits on a barrier that fails, after the
/// shutdown's three seconds.  Its caller is told the barrier failed, tagged `Unconfirmed`, as
/// soon as it does.  The failure is told by the cache bot the syncer releases the record to, and
/// the cache bots used to be finished once the three seconds ran out, so it was never told: its
/// caller waited out the durability deadline and heard a timeout.
fn close_tells_a_write_its_barrier_failed() -> Outcome<()> {
    let root = res!(fresh("./test_db_close_order_failed"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(open(&root, cfg));
    res!(db.insert(key(1), dat!(1u64), Uid::default(), None));

    hooks::set_barrier_delay(HOLD);
    hooks::set_barrier_failure(true);
    let writer = res!(write_during_close(&db, key(2), dat!(2u64)));
    thread::sleep(Duration::from_millis(100));
    let closed = db.close();
    let heard = match writer.join() {
        Ok(heard) => heard,
        Err(_) => return Err(err!("The writing thread panicked."; Test, Thread)),
    };
    hooks::set_barrier_failure(false);
    hooks::set_barrier_delay(Duration::ZERO);
    res!(closed);
    match heard {
        (_, Ok(())) => Err(err!(
            "A write whose barrier failed as the store closed was confirmed.";
            Test, Unexpected)),
        (took, Err(e)) => {
            if !e.tags().contains(&ErrTag::Unconfirmed) || e.tags().contains(&ErrTag::Timeout) {
                return Err(err!(
                    "A write whose barrier failed as the store closed was told after {:?}, tagged \
                    {:?}, where it should hear the barrier failed, tagged Unconfirmed and not \
                    Timeout: {}", took, e.tags(), e;
                    Test, Mismatch));
            }
            if took > TOLD_WITHIN {
                return Err(err!(
                    "A write whose barrier failed as the store closed was told after {:?}, where \
                    the barrier failed after {:?}.", took, HOLD;
                    Test, Timeout));
            }
            Ok(())
        },
    }
}

/// The store is closed while a written record waits on a barrier that succeeds, after the
/// shutdown's three seconds.  Its caller is told it is durable, and it is there after a restart.
/// The cache bots used to be finished once the three seconds ran out, so the record the syncer
/// released afterwards was never answered, and its caller waited out the durability deadline.
fn close_confirms_a_write_behind_a_slow_barrier() -> Outcome<()> {
    let root = res!(fresh("./test_db_close_order_slow"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(open(&root, cfg.clone()));
    res!(db.insert(key(1), dat!(1u64), Uid::default(), None));

    hooks::set_barrier_delay(HOLD);
    let writer = res!(write_during_close(&db, key(3), dat!(3u64)));
    thread::sleep(Duration::from_millis(100));
    let closed = db.close();
    let heard = match writer.join() {
        Ok(heard) => heard,
        Err(_) => return Err(err!("The writing thread panicked."; Test, Thread)),
    };
    hooks::set_barrier_delay(Duration::ZERO);
    res!(closed);
    match heard {
        (took, Err(e)) => return Err(err!(
            "A write whose slow barrier succeeded as the store closed was told after {:?}: {}",
            took, e;
            Test, Missing)),
        (took, Ok(())) if took > TOLD_WITHIN => return Err(err!(
            "A write whose slow barrier succeeded as the store closed was confirmed after {:?}, \
            where the barrier took {:?}.", took, HOLD;
            Test, Timeout)),
        _ => (),
    }
    let db = res!(open(&root, cfg));
    match res!(db.get(&key(3), None)) {
        Some((v, _)) => {
            res!(db.close());
            req!(v, dat!(3u64), "A write confirmed as the store closed.");
            Ok(())
        },
        None => {
            let _ = db.close(); // the check has failed already, and says why
            Err(err!(
                "A write confirmed as the store closed is missing after a restart.";
                Test, Missing))
        },
    }
}

/// Writes on a thread of its own, returning how long it waited for its final answer and what
/// that answer was.
fn write_during_close(
    db:     &TestDb,
    k:      Dat,
    v:      Dat,
)
    -> Outcome<thread::JoinHandle<(Duration, Outcome<()>)>>
{
    let writing = db.clone();
    Ok(res!(thread::Builder::new().name(fmt!("close order writer")).spawn(move || {
        let begun = Instant::now();
        let result = write(&writing, k, v);
        (begun.elapsed(), result)
    })))
}

/// Writes and waits `DEADLINE` for durability.  The final answer's error is passed on as it is,
/// since `Error::tags` reports only the outermost error's tags.
fn write(db: &TestDb, k: Dat, v: Dat) -> Outcome<()> {
    let resp = res!(db.api().store(k, v, Uid::default()));
    let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => n,
        msg => return Err(err!(
            "Expected the record count, received {:?}.", msg; Test, Unexpected)),
    };
    ok!(resp.recv_write_acks(n, constant::USER_REQUEST_TIMEOUT, DEADLINE));
    Ok(())
}

fn key(i: u64) -> Dat {
    dat!(fmt!("close order key {:03}", i))
}

/// One zone with one bot of each kind, so every read queues at the one reader bot and every
/// record reaches the one cache bot.
fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.num_zones           = 1;
    cfg.num_cbots_per_zone  = 1;
    cfg.num_fbots_per_zone  = 1;
    cfg.num_igbots_per_zone = 1;
    cfg.num_rbots_per_zone  = 1;
    cfg.num_wbots_per_zone  = 1;
    cfg.zone_overrides      = BTreeMap::new();
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
