//! A process killed in the middle of `compact_now` must leave a store that reads every key right
//! and that `compact_now` can finish erasing (A3 R4, 2026-10-08).  A collection puts two new files
//! in place of two old ones, and no pair of renames is one step: killed between them, the data file
//! is of one generation beside the index of the other (see `gc_crash_pair.rs`, which kills an
//! ordinary collection).  `compact_now` runs the same collection by order, on a store whose
//! collection is switched off, and it is the call that an erasure relies on.
//!
//! The test runs itself twice.  The child is this test binary again, started with the name of
//! a directory in its environment: it writes a store, deletes one key in each of the first two
//! data files, holds the next commit between the renames with `test::hooks::set_commit_delay`, and
//! calls `compact_now`.  The parent watches the files, and the moment exactly one pair has changed
//! it kills the child with SIGKILL, which no handler in the child can answer.  The collection runs
//! one file at a time, so the other file still holds its deleted key's bytes: the control that
//! says the last step, an erase on the reopened store, had something to erase.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
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
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Stdio,
    },
    thread,
    time::Duration,
};

const ROOT:     &str = "O3DB_COMPACT_CRASH_ROOT";
const LEN:      &str = "O3DB_COMPACT_CRASH_LEN";
const READY:    &str = "O3DB_COMPACT_CRASH_READY";

const DELETED:  [usize; 2] = [0, 5];    // the first key of file 1, and of file 2

/// The child: fills the store, deletes a key from each of the first two files, then calls
/// `compact_now` with the commit held between its renames, and waits there to be killed.  It
/// starts only when the parent has taken the sizes of the files, since a collection takes a few
/// milliseconds.
fn child(root: PathBuf, len: u64, ready: PathBuf) -> Outcome<()> {
    log_set_level!("error");
    let cfg = res!(config(5 * len + len / 2));
    // Collection off: only the order to compact collects.
    let db: TestDb = res!(setup::start_db(root, Some(cfg), schemes(), None, false, true));
    res!(fill(&db));
    for i in DELETED {
        res!(db.delete(&key(i), Uid::default(), None));
    }
    res!(db.api().settle_for_test(Duration::from_secs(10)));
    hooks::set_commit_delay(Duration::from_secs(120));
    res!(std::fs::write(&ready, b"ready"));
    let go = ready.with_extension("go");
    assert!(wait_until(Duration::from_secs(120), || go.exists()), "the parent never said go");
    let _ = db.compact_now(Duration::from_secs(150));
    // A child that is not killed is a failure of the parent, and ends itself.
    thread::sleep(Duration::from_secs(150));
    std::process::exit(3);
}

// Every key reads as its version 1, and the deleted ones do not read at all.
fn judge_after(db: &TestDb, label: &str) -> usize {
    let mut bad = 0;
    for i in 0..NKEYS {
        let got = db.get(&key(i), None);
        if DELETED.contains(&i) {
            match got {
                Ok(None) => (),
                other => {
                    bad += 1;
                    let s: String = fmt!("{:?}", other).chars().take(200).collect();
                    msg!("{}: deleted key {} read back {}.", label, i, s);
                },
            }
        } else {
            match got {
                Ok(Some((v, _))) if v == value(i, 1) => (),
                other => {
                    bad += 1;
                    let s: String = fmt!("{:?}", other).chars().take(200).collect();
                    msg!("{}: key {} did not read back its value: {}.", label, i, s);
                },
            }
        }
    }
    msg!("{}: {} of {} keys wrong.", label, bad, NKEYS);
    bad
}

// How many data files hold each deleted key's value.
fn needles(root: &Path, cfg: &OzoneConfig) -> Vec<usize> {
    DELETED.iter().map(|i| {
        let bytes = match value(*i, 1).bytes_ref() { Some(b) => b.to_vec(), None => Vec::new() };
        dat_files_holding(root, cfg, &bytes).len()
    }).collect()
}

fn parent() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_compact_crash_pair";
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let ready = res!(std::env::current_dir()).join("test_db_compact_crash_pair.ready");
    let _ = std::fs::remove_file(&ready);
    let _ = std::fs::remove_file(ready.with_extension("go"));
    let log = res!(std::env::current_dir()).join("test_db_compact_crash_pair.log");

    let probe = res!(probe("./test_db_compact_crash_pair_probe"));
    let len = probe.len;
    let cfg = res!(config(5 * len + len / 2));
    let pair = |n: u32| (file(&root, &cfg, FileType::Data, n), file(&root, &cfg, FileType::Index, n));
    let (d1, i1) = pair(1);
    let (d2, i2) = pair(2);

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

    // The child is ready when files 1 and 2 hold their five records each.
    assert!(wait_until(Duration::from_secs(120), || ready.exists()),
        "the child never wrote its store; see {:?}", log);
    let (sd1, si1, sd2, si2) = (size(&d1), size(&i1), size(&d2), size(&i2));
    assert_eq!((sd1, si1), (5 * len, probe.ind[5]), "file 1 does not hold the five records it was sized for");
    assert_eq!((sd2, si2), (5 * len, probe.ind[5]), "file 2 does not hold the five records it was sized for");
    res!(std::fs::write(ready.with_extension("go"), b"go"));

    // Exactly one file has had one of its pair replaced: a collection is between its renames.
    let torn = |d: &PathBuf, i: &PathBuf, sd: u64, si: u64| (size(d) != sd) != (size(i) != si);
    let caught = wait_until(Duration::from_secs(60), ||
        torn(&d1, &i1, sd1, si1) || torn(&d2, &i2, sd2, si2));
    res!(kid.kill());
    let _ = kid.wait();
    assert!(caught, "compact_now never stopped between its renames; see {:?}", log);
    let which = if torn(&d1, &i1, sd1, si1) { 1 } else { 2 };
    msg!("Killed with file {} torn: file 1 data {} index {}, file 2 data {} index {}.",
        which, size(&d1), size(&i1), size(&d2), size(&i2));

    // The store the child left, reopened with collection still off.
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let reopened = judge_after(&db, "After the kill");
    let temps = temporaries(&zone_dir(&root, &cfg));
    let size_reopened = res!(db.size_bytes());
    let walked_reopened = zone_files_len(&root, &cfg);
    let before = needles(&root, &cfg);
    msg!("Reopened: size_bytes {}, walked {}, deleted keys in {:?} data files.",
        size_reopened, walked_reopened, before);

    // The order that finishes the erasure.
    let report = res!(db.compact_now(Duration::from_secs(60)));
    msg!("{:?}", report);
    let after = needles(&root, &cfg);
    let finished = judge_after(&db, "After compact_now");
    let temps_after = temporaries(&zone_dir(&root, &cfg));
    let size_after = res!(db.size_bytes());
    let walked_after = zone_files_len(&root, &cfg);
    res!(db.close());

    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let second = judge_after(&db, "After a second reopening");
    res!(db.close());
    log_finish_wait!();

    assert_eq!(reopened, 0, "{} keys read wrong after the kill", reopened);
    assert!(temps.is_empty(), "the start left {:?} of the killed collection behind", temps);
    assert_eq!(size_reopened, walked_reopened, "size_bytes is not the length of the files after the kill");
    assert!(before.iter().sum::<usize>() >= 1,
        "positive control: the file that was not collected holds its deleted key ({:?}), file {} was torn", before, which);
    assert_eq!(after, vec![0, 0], "a deleted key is still in a data file after compact_now: {:?}", after);
    assert_eq!(finished, 0, "{} keys read wrong after compact_now", finished);
    assert!(temps_after.is_empty(), "compact_now left {:?} behind", temps_after);
    assert_eq!(size_after, walked_after, "size_bytes is not the length of the files after compact_now");
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
