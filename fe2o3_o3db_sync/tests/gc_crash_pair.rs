//! A process killed between the two renames of a collection must leave a store that reads every
//! key right.  A collection puts two new files in place of two old ones, and no pair of renames
//! is one step: a process that dies between them leaves a data file of one generation beside the
//! index of the other.  The start used to believe such an index, and the cache then named the
//! offsets of one file in the other (D-C, found by QA of lane o3i, 2026-09-24).
//!
//! The test runs itself twice.  The child is this test binary again, started with the name of
//! a directory in its environment: it writes a store, holds its next collection between the
//! renames with `test::hooks::set_commit_delay`, and starts the collection.  The parent watches
//! the files, and the moment exactly one of the pair has changed it kills the child with SIGKILL,
//! which no handler in the child can answer.  The parent then opens the store the child left.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    file::core::FileType,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    path::PathBuf,
    process::{
        Command,
        Stdio,
    },
    thread,
    time::Duration,
};

const ROOT:     &str = "O3DB_GC_CRASH_ROOT";
const LEN:      &str = "O3DB_GC_CRASH_LEN";
const READY:    &str = "O3DB_GC_CRASH_READY";

/// The child: fills the store, then starts a collection that stops between its renames, and waits
/// there to be killed.  It starts the collection only when the parent has taken the sizes of
/// the pair, since the collection takes a few milliseconds.
fn child(root: PathBuf, len: u64, ready: PathBuf) -> Outcome<()> {
    log_set_level!("error");
    let cfg = res!(config(5 * len + len / 2));
    let db: TestDb = res!(setup::start_db(root, Some(cfg), schemes(), None, true, true));
    res!(fill(&db));
    hooks::set_commit_delay(Duration::from_secs(120));
    // The parent reads the sizes of the pair when it sees this, and answers with `go`.
    res!(std::fs::write(&ready, b"ready"));
    let go = ready.with_extension("go");
    assert!(wait_until(Duration::from_secs(120), || go.exists()), "the parent never said go");
    // Superseding two of file 1's five records starts its collection.
    res!(db.insert(key(0), value(0, 2), Uid::default(), None));
    res!(db.insert(key(1), value(1, 2), Uid::default(), None));
    // A child that is not killed is a failure of the parent, and ends itself.
    thread::sleep(Duration::from_secs(150));
    std::process::exit(3);
}

fn parent() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_gc_crash_pair";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(std::path::Path::new(dir).canonicalize());
    let ready = res!(std::env::current_dir()).join("test_db_gc_crash_pair.ready");
    let _ = std::fs::remove_file(&ready);
    let _ = std::fs::remove_file(ready.with_extension("go"));
    let log = res!(std::env::current_dir()).join("test_db_gc_crash_pair.log");

    let probe = res!(probe("./test_db_gc_crash_pair_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));

    let exe = res!(std::env::current_exe());
    let out = res!(std::fs::File::create(&log));
    let err = res!(out.try_clone());
    let mut kid = res!(Command::new(exe)
        .args(["main", "--exact", "--nocapture", "--test-threads=1"])
        .env(ROOT, &root)
        .env(LEN, fmt!("{}", len))
        .env(READY, &ready)
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn());

    // The child is ready when file 1 holds its five records.
    assert!(wait_until(Duration::from_secs(120), || ready.exists()),
        "the child never wrote its store; see {:?}", log);
    let (d_old, i_old) = (size(&d1), size(&i1));
    assert_eq!((d_old, i_old), (5 * len, probe.ind[5]), "file 1 does not hold the five records it was sized for");
    res!(std::fs::write(ready.with_extension("go"), b"go"));

    // Exactly one of the pair has changed: the collection is between its renames.
    let caught = wait_until(Duration::from_secs(60), ||
        (size(&d1) != d_old) != (size(&i1) != i_old));
    res!(kid.kill());
    let _ = kid.wait();
    assert!(caught, "the collection never stopped between its renames; see {:?}", log);
    let (d_torn, i_torn) = (size(&d1), size(&i1));
    msg!("Killed with file 1 at data {} (was {}) and index {} (was {}).", d_torn, d_old, i_torn, i_old);
    assert!((d_torn != d_old) != (i_torn != i_old), "the pair was whole when the child was killed");

    // The store the child left.
    let mut want = vec![1u8; NKEYS];
    want[0] = 2;
    want[1] = 2;
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let reopened = judge(&db, &want, "After the kill");
    // The index has been made the data file's own, whichever file the kill left new.
    let want_ind = probe.ind[(size(&d1) / len) as usize];
    let rebuilt = size(&i1);

    // Churn so that file 1 is collected again with whatever state the start built.
    for round in 4..7u8 {
        for i in 2..6 {
            res!(db.insert(key(i), value(i, round), Uid::default(), None));
            want[i] = round;
        }
    }
    thread::sleep(Duration::from_secs(3));
    let churned = judge(&db, &want, "After more collection");
    res!(db.close());

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, true, false));
    let second = judge(&db, &want, "After a second reopening");
    res!(db.close());
    log_finish_wait!();

    assert_eq!(reopened, 0, "{} keys read wrong after the kill", reopened);
    assert_eq!(rebuilt, want_ind,
        "the index is {} bytes beside a data file of {}, where records of {} bytes need {}",
        rebuilt, d_torn, len, want_ind);
    assert_eq!(churned, 0, "{} keys read wrong after file 1 was collected again", churned);
    assert_eq!(second, 0, "{} keys read wrong after the second reopening", second);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_file(&ready);
    let _ = std::fs::remove_file(ready.with_extension("go"));
    let _ = std::fs::remove_file(&log);
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    match (std::env::var(ROOT), std::env::var(LEN), std::env::var(READY)) {
        (Ok(root), Ok(len), Ok(ready)) => {
            let len = res!(len.parse::<u64>());
            child(PathBuf::from(root), len, PathBuf::from(ready))
        },
        _ => parent(),
    }
}
