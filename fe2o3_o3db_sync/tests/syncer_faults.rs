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
    base::cfg::OzoneConfig,
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
    thread,
    time::Duration,
};

type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

const PERIOD_MS:    u64 = 200;                          // the interval policy's period
const WATCH:        Duration = Duration::from_millis(1_500);  // how long a failing disk is watched

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let failing = failing_disk_is_retried_once_a_period();
    // Whatever failed, the next check and the next binary must not inherit a failing disk.
    hooks::set_barrier_failure(false);
    log_finish_wait!();
    let failed: Vec<Error<ErrTag>> = [failing].into_iter()
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
