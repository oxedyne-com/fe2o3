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
    base::{
        cfg::OzoneConfig,
        constant,
    },
    db::O3db,
    file::refusal::Refusal,
    test::{
        hooks::{
            self,
            ReplayFlush,
        },
        setup::Uid,
    },
};

use std::{
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        MutexGuard,
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
    written_n(dir, NVAL)
}

fn written_n(dir: &str, nval: usize) -> Outcome<PathBuf> {
    log_set_level!("error");
    let _ = std::fs::remove_dir_all(dir);
    res!(std::fs::create_dir_all(dir));
    let root = res!(Path::new(dir).canonicalize());
    let _ = std::fs::remove_file(OzoneConfig::config_path(&root));
    let db = res!(open(&root));
    for i in 0..nval {
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
    all_read_n(db, NVAL)
}

fn all_read_n(db: &TestDb, nval: usize) -> bool {
    (0..nval).all(|i| matches!(db.get(&key(i), None), Ok(Some((v, _))) if v == value(i, 1)))
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

/// A failed flush refuses the open, made either way, and the refusal stands: the next open is
/// refused too, naming the marker, though the disk reports nothing now, as a kernel that has
/// reported a write-back error once would.  Once the marker is cleared the store opens whole.
/// The hook replaces the answer of the real `syncfs` or `sync_data`, so the error path is the
/// real one.
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
        match opened {
            Ok(db) => {
                let _ = db.close();
                hooks::set_replay_flush(ReplayFlush::Kernel);
                return Err(err!("The store opened though its flush by {:?} failed.", way; Test));
            },
            Err(e) => {
                let s = fmt!("{}", e);
                assert!(s.contains(says), "the refusal by {:?} does not say '{}': {}", way, says, s);
                assert!(s.contains(constant::REFUSAL_FILENAME),
                    "the refusal by {:?} does not name the marker it kept: {}", way, s);
            },
        }
        assert!(Refusal::path(&root).exists(), "the refused open by {:?} left no marker", way);
        match open(&root) {
            Ok(db) => {
                let _ = db.close();
                hooks::set_replay_flush(ReplayFlush::Kernel);
                return Err(err!("The store opened over the refusal by {:?}, as a retry under \
                    systemd would.", way; Test));
            },
            Err(e) => {
                let s = fmt!("{}", e);
                assert!(s.contains(constant::REFUSAL_FILENAME) && s.contains("Refusal::clear"),
                    "the second open after {:?} was refused without naming the marker and its \
                    verb: {}", way, s);
            },
        }
        hooks::set_replay_flush(ReplayFlush::Kernel);
        assert!(res!(Refusal::clear(&root)), "there was no marker to clear after {:?}", way);
        assert!(!res!(Refusal::clear(&root)), "a second clear after {:?} found a marker", way);
        let db = res!(open(&root));
        let ok = all_read(&db);
        res!(db.close());
        assert!(ok, "a key did not read back after the refused open by {:?}", way);
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// Before Linux 5.8 `syncfs` reports no write-back error, so the open syncs each file; a release
/// that cannot be read is taken as one of those.  From 5.8 one `syncfs` serves the filesystem.
#[test]
fn the_kernel_release_picks_the_flush() -> Outcome<()> {
    let _g = lock();
    let root = res!(written("./test_db_open_flush_release"));
    let files = res!(nfiles(&root));
    let mut counts = Vec::new();
    for rel in ["5.4.0-150-generic", "", "6.1.0"] {
        hooks::set_kernel_release(Some(rel));
        let n = flushes_on_open(&root, ReplayFlush::Kernel);
        hooks::set_kernel_release(None);
        counts.push((rel, res!(n)));
    }
    for (rel, n) in &counts[..2] {
        assert!(*n as usize >= files,
            "kernel release {:?} made {} flushes for {} files, not one per file", rel, n, files);
    }
    if cfg!(target_os = "linux") {
        assert_eq!(counts[2].1, 1, "kernel release 6.1.0 made {} flushes, not one syncfs", counts[2].1);
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

// Writes without syncing beside the store until stopped, as `dd` would, so the filesystem holds
// dirty pages that a `syncfs` must write back.  Returns the bytes written.
fn dirtier(dir: &Path, stop: Arc<AtomicBool>) -> thread::JoinHandle<u64> {
    let path = dir.with_extension("dirt");
    thread::spawn(move || {
        let block = vec![0x5au8; 1 << 20];
        let mut n = 0u64;
        let mut f = match fs::File::create(&path) {
            Ok(f)   => f,
            Err(_)  => return 0,
        };
        while !stop.load(Ordering::SeqCst) {
            if n >= 256 << 20 {
                // Start again, so the dirt stays bounded but keeps coming.
                if f.set_len(0).is_err() {
                    break;
                }
                n = 0;
            }
            if f.write_all(&block).is_err() {
                break;
            }
            n += block.len() as u64;
        }
        drop(f);
        let _ = fs::remove_file(&path);
        n
    })
}

// Opens the store at `root` while a writer dirties its filesystem, and reads every key.
fn loaded_open(root: &Path, nval: usize) -> Outcome<Duration> {
    let stop = Arc::new(AtomicBool::new(false));
    let h = dirtier(root, stop.clone());
    thread::sleep(Duration::from_millis(500));
    let t0 = Instant::now();
    let opened = open(root);
    let took = t0.elapsed();
    stop.store(true, Ordering::SeqCst);
    let dirt = h.join().unwrap_or(0);
    let db = res!(opened);
    let ok = all_read_n(&db, nval);
    res!(db.close());
    println!("loaded open of {} files: {:?}, with {} MiB written beside it",
        res!(nfiles(root)), took, dirt >> 20);
    if !ok {
        return Err(err!("A key did not read back after a loaded open."; Test));
    }
    Ok(took)
}

/// An open while another writer dirties the same filesystem: `syncfs` writes back the other
/// writer's pages too, so the open costs with the dirt, not the file count.  Always run, small.
#[test]
fn a_loaded_open_is_bounded() -> Outcome<()> {
    let _g = lock();
    let root = res!(written("./test_db_open_flush_loaded"));
    let took = res!(loaded_open(&root, NVAL));
    assert!(took < Duration::from_secs(30), "a loaded open of a small store took {:?}", took);
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// The same at 9.6k files, the size whose per-file flush once cost 10-25 s; QA measured 466 ms
/// warm and 880 ms cold unloaded.  Run alone, under the slot's I/O cap:
/// `cargo test -p oxedyne_fe2o3_o3db_sync --test open_flush -- --ignored --nocapture
/// a_loaded_open_of_9600_files`.
#[test]
#[ignore]
fn a_loaded_open_of_9600_files() -> Outcome<()> {
    let _g = lock();
    let nval = 48_000; // ten values a pair, so about 9.6k data and index files
    let root = res!(written_n("./test_db_open_flush_loaded_9600", nval));
    let files = res!(nfiles(&root));
    assert!(files >= 9_000, "the store holds only {} files", files);
    let took = res!(loaded_open(&root, nval));
    assert!(took < Duration::from_secs(60), "a loaded open of {} files took {:?}", files, took);
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// M11: the open really calls `syncfs`, once per filesystem.  `hooks::replay_flushes` counts the
/// flush before it is made, so only the kernel can tell whether it was: this test runs itself
/// again under `strace -f -e trace=syncfs` and counts the calls the child's open makes.  It
/// passes with a note when strace is absent.  Runner:
/// `cargo test -p oxedyne_fe2o3_o3db_sync --test open_flush -- --ignored --nocapture
/// an_open_calls_syncfs_once_per_filesystem`.
#[test]
#[ignore]
fn an_open_calls_syncfs_once_per_filesystem() -> Outcome<()> {
    const CHILD: &str = "OPEN_FLUSH_STRACE_ROOT";
    const NAME: &str = "an_open_calls_syncfs_once_per_filesystem";
    let _g = lock();
    if let Ok(root) = std::env::var(CHILD) {
        // The child: one open and close of the store its parent wrote, flushed by syncfs.
        hooks::set_replay_flush(ReplayFlush::Syncfs);
        let db = res!(open(Path::new(&root)));
        res!(db.close());
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let have = std::process::Command::new("strace").arg("-V").output();
    if !matches!(have, Ok(ref o) if o.status.success()) {
        println!("{}: strace is not installed, so the kernel's count is not checked.", NAME);
        return Ok(());
    }
    let root = res!(written("./test_db_open_flush_strace"));
    let trace = root.with_extension("strace");
    let exe = res!(std::env::current_exe());
    let out = res!(std::process::Command::new("strace")
        .args(["-f", "-qq", "-e", "trace=syncfs", "-o"])
        .arg(&trace)
        .arg(&exe)
        .args(["--ignored", "--exact", NAME, "--test-threads=1"])
        .env(CHILD, &root)
        .output());
    let text = fs::read_to_string(&trace).unwrap_or_default();
    let _ = fs::remove_file(&trace);
    let _ = fs::remove_dir_all(&root);
    assert!(out.status.success(), "the traced open failed: {}",
        String::from_utf8_lossy(&out.stdout));
    let calls = text.lines().filter(|l| l.contains("syncfs(")).count();
    assert_eq!(calls, 1, "two zones on one filesystem made {} syncfs calls:\n{}", calls, text);
    Ok(())
}
