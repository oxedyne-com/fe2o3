//! The flush before replay (R2, 2026-10-10).  An open makes every data and index file durable
//! before replay calls their records durable.  Where the kernel reports write-back errors through
//! `syncfs` that is one flush per filesystem, not one per file, which on a store of thousands of
//! files took seconds.  Either way a failed flush refuses the open.  The hooks are process-wide,
//! so the tests hold one lock.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    db::O3db,
    test::{
        hooks::{
            self,
            ReplayFlush,
        },
        setup::Uid,
    },
};

use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Mutex,
        MutexGuard,
    },
};

const MAX:  u64   = 1_000;  // data file length, so the store holds many files
const NVAL: usize = 120;    // 100-byte values

static HOOKS: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

// Two zones on one filesystem, so a flush per directory would show.
fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(config(MAX));
    c.num_zones = 2;
    Ok(c)
}

fn open(root: &Path) -> Outcome<TestDb> {
    let mut db = res!(O3db::new(root.to_path_buf(), Some(res!(cfg())), schemes(), Uid::default()));
    res!(db.start("open_flush"));
    Ok(db)
}

// A store of many small files, closed.
fn written(dir: &str) -> Outcome<PathBuf> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let _ = std::fs::remove_file(OzoneConfig::config_path(&root));
    let db = res!(open(&root));
    for i in 0..NVAL {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    res!(db.close());
    Ok(root)
}

// The data and index files on disk, in both zones.
fn nfiles(root: &Path) -> Outcome<usize> {
    let zroot = res!(cfg()).zone_root(root);
    let mut n = 0;
    for z in ["zone_001", "zone_002"] {
        for item in res!(std::fs::read_dir(zroot.join(z))) {
            let path = res!(item).path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("dat") | Some("ind") => n += 1,
                _ => (),
            }
        }
    }
    Ok(n)
}

fn all_read(db: &TestDb) -> bool {
    (0..NVAL).all(|i| matches!(db.get(&key(i), None), Ok(Some((v, _))) if v == value(i, 1)))
}

// Opens and closes the store with the flush made one way, returning how many flushes it made.
fn flushes_on_open(root: &Path, way: ReplayFlush) -> Outcome<u64> {
    hooks::set_replay_flush(way);
    let n0 = hooks::replay_flushes();
    let db = res!(open(root));
    let n = hooks::replay_flushes() - n0;
    let ok = all_read(&db);
    res!(db.close());
    hooks::set_replay_flush(ReplayFlush::Kernel);
    if !ok {
        return Err(err!("A key did not read back after an open flushed by {:?}.", way; Test));
    }
    Ok(n)
}

/// One flush per filesystem with `syncfs`, one per file without, and the kernel's choice is one
/// of the two.
#[test]
fn an_open_flushes_each_filesystem_once() -> Outcome<()> {
    let _g = lock();
    let root = res!(written("./test_db_open_flush_count"));
    let files = res!(nfiles(&root));
    assert!(files >= 20, "the store holds only {} files, too few to tell the ways apart", files);

    let by_fs = res!(flushes_on_open(&root, ReplayFlush::Syncfs));
    if cfg!(target_os = "linux") {
        assert_eq!(by_fs, 1, "two zones on one filesystem were flushed {} times by syncfs", by_fs);
    }
    let by_file = res!(flushes_on_open(&root, ReplayFlush::PerFile));
    assert!(by_file as usize >= files,
        "the per-file flush made {} syncs for {} files", by_file, files);
    let chosen = res!(flushes_on_open(&root, ReplayFlush::Kernel));
    assert!(chosen == by_fs || chosen as usize >= files,
        "the kernel's choice made {} flushes, neither way's count", chosen);

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// A failed flush refuses the open, made either way, and the store opens whole once the disk
/// is well again.
#[test]
fn a_failed_flush_refuses_the_open() -> Outcome<()> {
    let _g = lock();
    let root = res!(written("./test_db_open_flush_fail"));
    for (way, says) in [
        (ReplayFlush::Syncfs,  if cfg!(target_os = "linux") { "write-back error" } else { "durable" }),
        (ReplayFlush::PerFile, "durable before replaying"),
    ] {
        hooks::set_replay_flush(way);
        hooks::set_replay_sync_failure(true);
        let opened = open(&root);
        hooks::set_replay_sync_failure(false);
        hooks::set_replay_flush(ReplayFlush::Kernel);
        match opened {
            Ok(db) => {
                let _ = db.close();
                return Err(err!("The store opened though its flush by {:?} failed.", way; Test));
            },
            Err(e) => {
                let s = fmt!("{}", e);
                assert!(s.contains(says), "the refusal by {:?} does not say '{}': {}", way, says, s);
            },
        }
        let db = res!(open(&root));
        let ok = all_read(&db);
        res!(db.close());
        assert!(ok, "a key did not read back after the refused open by {:?}", way);
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
