//! One process per store (Ozone R2-2, 2026-10-10).  A start takes an exclusive lock on a file in
//! the store's root, so a second open, in another process or this one, is refused by name rather
//! than surveying and appending to the same live files.  The lock is the kernel's: a holder killed
//! with SIGKILL leaves no stale lock behind.
//!
//! The test runs itself as a child, the pattern of `compact_crash_pair.rs`: with the variables
//! below in its environment it opens the store, says so, and waits to be killed.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    gateway,
    sweep,
    test::setup::{
        self,
        Uid,
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

const ROOT:     &str = "O3DB_STORE_LOCK_ROOT";
const READY:    &str = "O3DB_STORE_LOCK_READY";
const GATEWAY:  &str = "O3DB_STORE_LOCK_GATEWAY";

const KEY: [u8; gateway::DB_KEY_LEN] = [7u8; gateway::DB_KEY_LEN]; // a test store's, allowlist secret

// The child: opens the store, writes a key, says it is ready, and waits to be killed.
fn child(root: PathBuf, ready: PathBuf, as_gateway: bool) -> Outcome<()> {
    log_set_level!("error");
    if as_gateway {
        let (db, _) = res!(gateway::open_store(&root, Some(res!(config(16_000))), &KEY, true, "child"));
        res!(db.insert(dat!("held"), dat!("by the child"), Uid::default(), None));
        res!(std::fs::write(&ready, b"ready"));
        thread::sleep(Duration::from_secs(150));
    } else {
        let db: TestDb = res!(setup::start_db(root, Some(res!(config(16_000))), schemes(), None, false, true));
        res!(db.insert(dat!("held"), dat!("by the child"), Uid::default(), None));
        res!(std::fs::write(&ready, b"ready"));
        thread::sleep(Duration::from_secs(150));
    }
    // A child that is not killed is a failure of the parent, and ends itself.
    std::process::exit(3);
}

fn spawn_holder(name: &str, as_gateway: bool) -> Outcome<(std::process::Child, PathBuf, PathBuf)> {
    let dir = PathBuf::from(fmt!("./test_db_store_lock_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir));
    let root = res!(dir.canonicalize());
    let ready = res!(std::env::current_dir()).join(fmt!("test_db_store_lock_{}.ready", name));
    let _ = std::fs::remove_file(&ready);
    let log = res!(std::env::current_dir()).join(fmt!("test_db_store_lock_{}.log", name));
    let out = res!(std::fs::File::create(&log));
    let err = res!(out.try_clone());
    let kid = res!(Command::new(res!(std::env::current_exe()))
        .args(["main", "--exact", "--nocapture", "--test-threads=1"])
        .env(ROOT, &root)
        .env(READY, &ready)
        .env(GATEWAY, if as_gateway { "1" } else { "0" })
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn());
    assert!(wait_until(Duration::from_secs(120), || ready.exists()),
        "the child never opened its store; see {:?}", log);
    Ok((kid, root, ready))
}

fn refusal<T>(r: Outcome<T>, root: &Path, what: &str) -> Error<ErrTag> {
    match r {
        Ok(_)   => panic!("{}: a second open of a held store at {:?} was allowed", what, root),
        Err(e)  => {
            let tags = e.tags();
            assert!(tags.contains(&ErrTag::Init) && tags.contains(&ErrTag::Lock)
                && tags.contains(&ErrTag::Conflict), "{}: refused with tags {:?}: {}", what, tags, e);
            let text = fmt!("{}", e);
            assert!(text.contains(&fmt!("{}", root.display())), "{}: the refusal does not name the store: {}", what, text);
            e
        },
    }
}

fn parent() -> Outcome<()> {
    log_set_level!("error");

    // Held by another process, then that process killed with SIGKILL.
    let (mut kid, root, ready) = res!(spawn_holder("kill", false));
    let cfg = res!(config(16_000));
    refusal(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false),
        &root, "held by a child");
    res!(kid.kill());
    let _ = kid.wait();
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    let held = res!(db.get(&dat!("held"), None));
    assert!(matches!(&held, Some((v, _)) if *v == dat!("by the child")), "the killed child's write reads {:?}", held);

    // Held in this process: a second open is refused here too.
    refusal(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false),
        &root, "held by this process");

    // Closed, then opened again.
    res!(db.close());
    drop(db);
    let db: TestDb = res!(setup::start_db(root.clone(), Some(cfg.clone()), schemes(), None, false, false));
    res!(db.close());
    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&ready);

    // The sweep tool against a store the gateway holds: refused, saying what to do instead.
    let (mut kid, root, ready) = res!(spawn_holder("sweep", true));
    let e = refusal(sweep::open_store_alone(&root, &KEY, "sweep"), &root, "the sweep tool");
    let text = fmt!("{}", e);
    res!(kid.kill());
    let _ = kid.wait();
    assert!(text.contains("Stop the gateway") && text.contains("sweep::sweep_orphans"),
        "the sweep tool's refusal does not say what to do: {}", text);
    // With the gateway gone the tool opens it.
    let (db, n) = res!(sweep::open_store_alone(&root, &KEY, "sweep"));
    res!(gateway::require_answer(n, "sweep", &root));
    res!(db.shutdown());
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&ready);
    log_finish_wait!();
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    match (std::env::var(ROOT), std::env::var(READY), std::env::var(GATEWAY)) {
        (Ok(root), Ok(ready), Ok(g)) => child(PathBuf::from(root), PathBuf::from(ready), g == "1"),
        _ => parent(),
    }
}
