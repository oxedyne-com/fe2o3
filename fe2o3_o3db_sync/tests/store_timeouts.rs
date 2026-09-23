//! On a busy machine Oregami's store reported writes as failed that went on to land, and came up
//! answering nothing (2026-09-23).  Each fault is reproduced here without the machine having to be
//! busy: `test::hooks` holds every durability barrier, or the supervisor, for longer than the
//! deadline that used to expire.  The hooks are process-wide, which is why this is a test binary
//! of its own with a single test.

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
    os::unix::fs::PermissionsExt,
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

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    // Whatever failed, the next binary must not inherit a slow store.
    hooks::set_barrier_delay(Duration::ZERO);
    hooks::set_publish_delay(Duration::ZERO);
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {
    res!(slow_start_hands_over_live_channels());
    res!(slow_barrier_still_acknowledges());
    res!(durability_deadline_reports_written());
    res!(writer_failure_reaches_the_caller());
    res!(failed_zone_fails_the_start());
    Ok(())
}

/// The supervisor takes longer to hand over its channels than the one second `start` used to
/// sleep.  The handle must hold the live channels when `start` returns, with no `updated_api`
/// and no sleep: before, every bot but the supervisor was unreachable, and a ping heard one of
/// them.
fn slow_start_hands_over_live_channels() -> Outcome<()> {
    let delay = Duration::from_millis(2_500);
    let root = res!(fresh("./test_db_store_timeouts_start"));
    hooks::set_publish_delay(delay);
    let begun = Instant::now();
    let opened = open(&root, res!(config()));
    hooks::set_publish_delay(Duration::ZERO);
    let db = res!(opened);
    let took = begun.elapsed();
    if took < delay {
        return Err(err!(
            "start returned after {:?}, before the supervisor had handed over its channels \
            {:?} in.", took, delay;
            Test, Timeout));
    }

    // Straight through the handle, as Oregami's wake does.  A ping goes to every bot the handle
    // holds a channel for and fails unless each of them answers.
    let (_, pongs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    if pongs.len() < 2 {
        return Err(err!(
            "Only {} bot answered a ping as soon as start returned.", pongs.len();
            Test, Missing));
    }

    let key = dat!("reachable after a slow start");
    res!(db.insert(key.clone(), dat!(7u8), Uid::default(), None));
    match res!(db.get(&key, None)) {
        Some((v, _)) => req!(v, dat!(7u8), "The value written after a slow start."),
        None => return Err(err!(
            "A value written after a slow start could not be read back."; Test, Missing)),
    }
    res!(db.close());
    Ok(())
}

/// Every durability barrier takes longer than the user request deadline.  A write used to fail
/// at that deadline and land a moment later; now it is answered written at once and durable
/// when the barrier completes.  Writes queued behind the barrier share it, and none of them is
/// held by it before being written.
fn slow_barrier_still_acknowledges() -> Outcome<()> {
    let delay = constant::USER_REQUEST_TIMEOUT + Duration::from_secs(1);
    let root = res!(fresh("./test_db_store_timeouts_barrier"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(open(&root, cfg));

    hooks::set_barrier_delay(delay);
    let begun = Instant::now();
    let mut writers = Vec::new();
    for i in 0..3u8 {
        let db = db.clone();
        writers.push(res!(thread::Builder::new()
            .name(fmt!("writer {}", i))
            .spawn(move || -> Outcome<Duration> {
                let begun = Instant::now();
                res!(db.insert(key(i), dat!(i), Uid::default(), None));
                Ok(begun.elapsed())
            })));
    }
    for (i, writer) in writers.into_iter().enumerate() {
        match writer.join() {
            Ok(Ok(took)) => if took < delay {
                return Err(err!(
                    "Write {} was acknowledged in {:?}, before the {:?} barrier it waits on \
                    could have completed.", i, took, delay;
                    Test, Unexpected));
            },
            Ok(Err(e)) => return Err(err!(e,
                "Write {} failed behind a {:?} barrier, which the user request deadline of \
                {:?} does not measure.", i, delay, constant::USER_REQUEST_TIMEOUT;
                Test, Write)),
            Err(_) => return Err(err!("Writer thread {} panicked.", i; Test, Thread)),
        }
    }
    hooks::set_barrier_delay(Duration::ZERO);
    test!(sync_log::stream(), "Three writes behind {:?} barriers took {:?}.", delay, begun.elapsed());
    for i in 0..3u8 {
        match res!(db.get(&key(i), None)) {
            Some((v, _)) => req!(v, dat!(i), "The value of a slowly synced write."),
            None => return Err(err!("Write {} is not readable.", i; Test, Missing)),
        }
    }
    res!(db.close());

    // Reopened, to show the writes are in the files and not just the cache.
    let db = res!(open(&root, res!(config())));
    for i in 0..3u8 {
        match res!(db.get(&key(i), None)) {
            Some((v, _)) => req!(v, dat!(i), "The value of a slowly synced write, reopened."),
            None => return Err(err!("Write {} did not survive a reopen.", i; Test, Missing)),
        }
    }
    res!(db.close());
    Ok(())
}

/// When the durability deadline itself expires, the caller is told the write is in the files
/// but not yet confirmed durable, and that is true: the value is readable once the barrier
/// completes.  The deadline is passed in short here, since the real one is two minutes.
fn durability_deadline_reports_written() -> Outcome<()> {
    let delay = Duration::from_secs(4);
    let durability = Duration::from_secs(2);
    let root = res!(fresh("./test_db_store_timeouts_deadline"));
    let mut cfg = res!(config());
    cfg.sync_on_write = true;
    let db = res!(open(&root, cfg));

    hooks::set_barrier_delay(delay);
    let begun = Instant::now();
    let resp = res!(db.api().store(key(9), dat!(9u8), Uid::default()));
    let n = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
        OzoneMsg::Chunks(n) => n,
        msg => return Err(err!("Expected the record count, received {:?}.", msg;
            Test, Unexpected)),
    };
    let outcome = resp.recv_write_acks(n, constant::USER_REQUEST_TIMEOUT, durability);
    let took = begun.elapsed();
    hooks::set_barrier_delay(Duration::ZERO);
    match outcome {
        Ok(acks) => return Err(err!(
            "A write behind a {:?} barrier was confirmed durable within {:?}: {:?}.",
            delay, durability, acks;
            Test, Unexpected)),
        Err(e) => {
            let text = fmt!("{:?}", e);
            if !text.contains("not confirmed durable") || !text.contains("has not failed") {
                return Err(err!(
                    "An expired durability deadline must say the write is in the files but not \
                    yet durable, and it said: {}", text;
                    Test, Mismatch));
            }
            if took >= delay {
                return Err(err!(
                    "The durability deadline of {:?} took {:?} to expire.", durability, took;
                    Test, Timeout));
            }
        },
    }
    // What the error said: the write lands when the disk completes it.
    thread::sleep(delay.saturating_sub(begun.elapsed()) + Duration::from_millis(500));
    match res!(db.get(&key(9), None)) {
        Some((v, _)) => req!(v, dat!(9u8), "A write reported as not yet durable."),
        None => return Err(err!(
            "A write reported as written but not yet durable never became readable.";
            Test, Missing)),
    }
    res!(db.close());
    Ok(())
}

/// A writer that fails says why, where it used to leave the caller to time out on nothing.  The
/// zone directory is made read-only, so the write that needs a new live file cannot have one.
fn writer_failure_reaches_the_caller() -> Outcome<()> {
    let root = res!(fresh("./test_db_store_timeouts_writer"));
    let mut cfg = res!(config());
    cfg.num_zones = 1;
    let db = res!(open(&root, cfg.clone()));
    let big = || dat!(vec![0x5au8; 1_000]); // two of these overflow a 2,000 byte live file

    res!(db.insert(key(1), big(), Uid::default(), None));
    let zdir = res!(zone_dir(&root, &cfg));
    res!(std::fs::set_permissions(&zdir, std::fs::Permissions::from_mode(0o555)));
    let begun = Instant::now();
    let outcome = db.insert(key(2), big(), Uid::default(), None);
    let took = begun.elapsed();
    res!(std::fs::set_permissions(&zdir, std::fs::Permissions::from_mode(0o755)));
    match outcome {
        Ok(_) => return Err(err!(
            "A write needing a live file in a read-only zone directory succeeded."; Test, Unexpected)),
        Err(e) => {
            let text = fmt!("{:?}", e);
            if text.contains("Failed to receive a message via responder") {
                return Err(err!(
                    "A writer's failure reached the caller as a timeout: {}", text; Test, Mismatch));
            }
            if !text.contains("new live file") || !text.contains("ermission denied") {
                return Err(err!(
                    "A writer's failure must name its cause, and it said: {}", text; Test, Mismatch));
            }
            if took >= constant::USER_REQUEST_TIMEOUT {
                return Err(err!(
                    "The writer's failure took {:?} to arrive.", took; Test, Timeout));
            }
        },
    }
    // The directory is writable again, and so is the store.
    res!(db.insert(key(3), big(), Uid::default(), None));
    match res!(db.get(&key(3), None)) {
        Some((v, _)) => req!(v, big(), "A write after the zone recovered."),
        None => return Err(err!("A write after the zone recovered is missing."; Test, Missing)),
    }
    res!(db.close());
    Ok(())
}

/// A zone that cannot be initialised fails the start.  It used to be logged, and the store came
/// up with that zone's writer holding no directory, so its writes landed in the working directory
/// of whatever process had opened it.  The failed start returns once the bots it brought up have
/// stopped: it returned with 32 threads running where there had been 4, and a caller that opens
/// the directory again at once, as Oregami's forge does on its next request, could have two sets
/// of bots over one directory.
fn failed_zone_fails_the_start() -> Outcome<()> {
    let root = res!(fresh("./test_db_store_timeouts_zone"));
    let mut cfg = res!(config());
    cfg.zone_overrides = res!(mapdat!{
        1u16 => mapdat!{
            "dir"       => "../test_db_store_timeouts_no_such_container",
            "max_size"  => 1_000_000u64,
        },
    }.get_map().ok_or_else(|| err!("The zone override is not a map."; Test, Bug)));
    let before = res!(threads());
    let begun = Instant::now();
    let mut db = res!(TestDb::new(root.clone(), Some(cfg), schemes(), Uid::default()));
    match db.start("test") {
        Ok(_) => return Err(err!(
            "A store whose zone directory does not exist started."; Test, Unexpected)),
        Err(e) => {
            let text = fmt!("{:?}", e);
            if !text.contains("does not exist and must be created") {
                return Err(err!(
                    "The failed start must name the missing directory, and it said: {}", text;
                    Test, Mismatch));
            }
        },
    }
    let took = begun.elapsed();
    if took >= constant::USER_REQUEST_TIMEOUT {
        return Err(err!("The failed start took {:?} to report.", took; Test, Timeout));
    }
    // A thread that has let go of its bot ends a moment later, so the count gets that moment.
    let at_return = res!(threads());
    let settled = Instant::now() + Duration::from_millis(100);
    let mut after = at_return;
    while after > before && Instant::now() < settled {
        thread::sleep(Duration::from_millis(5));
        after = res!(threads());
    }
    if after > before {
        return Err(err!(
            "A failed start returned with {} threads running, {} a moment later, where there were \
            {} before it: the bots it brought up were still running.", at_return, after, before;
            Test, Unexpected));
    }
    // Nothing is left running for a close to wait on.
    let begun = Instant::now();
    res!(db.close());
    if begun.elapsed() >= Duration::from_secs(1) {
        return Err(err!(
            "Closing a store whose start failed took {:?}.", begun.elapsed(); Test, Timeout));
    }
    Ok(())
}

fn key(i: u8) -> Dat {
    dat!(fmt!("store timeouts key {}", i))
}

/// The threads this process is running.
fn threads() -> Outcome<usize> {
    Ok(res!(std::fs::read_dir("/proc/self/task")).count())
}

/// The crate's test configuration, with every zone inside the test's own directory.
fn config() -> Outcome<OzoneConfig> {
    let mut cfg = res!(setup::default_cfg());
    cfg.zone_overrides = BTreeMap::new();
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

fn open(root: &Path, cfg: OzoneConfig) -> Outcome<TestDb> {
    let mut db = res!(TestDb::new(root.to_path_buf(), Some(cfg), schemes(), Uid::default()));
    res!(db.start("test"));
    Ok(db)
}

/// The directory of the first zone, which is the only one when there is one.
fn zone_dir(root: &Path, cfg: &OzoneConfig) -> Outcome<PathBuf> {
    let zdir = cfg.zone_root(root).join("zone_001");
    if !zdir.is_dir() {
        return Err(err!("Expected a zone directory at {:?}.", zdir; Test, Missing));
    }
    Ok(zdir)
}
