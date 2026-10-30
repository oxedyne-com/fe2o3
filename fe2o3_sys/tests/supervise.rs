//! Tests for `fe2o3_sys::supervise` against real processes, a real cgroup and the kernel's own
//! account of them.
//!
//! The cgroup tests need a cgroup v2 subtree delegated to the caller with the `memory` controller,
//! which a test run gets from
//! `systemd-run --user --scope -p Delegate=yes <this test binary>`: the test moves the scope's own
//! processes into a leaf, as [`CgroupRoot::from_own_cgroup`] does, and confines its children under
//! the scope. Where the subtree is not delegated they say so and pass, unless
//! `FE2O3_SYS_SUPERVISE_REQUIRE` is set, when they fail.
//!
//! Every child is a copy of this test binary run as a helper: one that fills memory, one that arms
//! its parent's death and sleeps, and one that stands as the parent of that one and is then killed.

#![cfg(all(target_os = "linux", feature = "supervise"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::signal::{
    to_process,
    Proc,
    Signal,
};
use oxedyne_fe2o3_sys::supervise::{
    arm_parent_death,
    CgroupRoot,
    Confined,
    Limit,
    RestartBudget,
    Scoped,
    SliceKills,
    start_slice,
};

use std::{
    env,
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::Stdio,
    sync::OnceLock,
    thread,
    time::{
        Duration,
        Instant,
    },
};

const HELPER:   &str        = "FE2O3_SYS_SUPERVISE_HELPER";
const MODE:     &str        = "FE2O3_SYS_SUPERVISE_MODE";
const REQUIRE:  &str        = "FE2O3_SYS_SUPERVISE_REQUIRE";
const LEAF:     &str        = "supervisor";
const MIB:      u64         = 1024 * 1024;
const NAP:      Duration    = Duration::from_millis(20);
const PATIENCE: Duration    = Duration::from_secs(10);

fn io<T>(r: std::io::Result<T>, what: &str) -> Outcome<T> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("{}: {}.", what, e; IO, File)),
    }
}

// A directory of this test's own, under the target directory rather than the shared temporary one.
struct Dir(PathBuf);

impl Dir {
    fn new(what: &str) -> Outcome<Self> {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("fe2o3_sys_supervise_{}_{}", what, std::process::id()));
        let _ = fs::remove_dir_all(&path);
        res!(io(fs::create_dir_all(&path), "make the test directory"));
        Ok(Self(path))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn until<F: Fn() -> bool>(what: &str, ask: F) -> Outcome<()> {
    let begun = Instant::now();
    while !ask() {
        if begun.elapsed() > PATIENCE {
            return Err(err!("Waited {} s for {}.", PATIENCE.as_secs(), what; Test, Timeout));
        }
        thread::sleep(NAP);
    }
    Ok(())
}

// The delegated root, once for the whole binary, or why there is none.
fn root() -> Option<&'static CgroupRoot> {
    static ROOT: OnceLock<std::result::Result<CgroupRoot, String>> = OnceLock::new();
    let got = ROOT.get_or_init(|| match CgroupRoot::from_own_cgroup(LEAF) {
        Ok(r)   => Ok(r),
        Err(e)  => Err(format!("{}", e)),
    });
    match got {
        Ok(r)   => Some(r),
        Err(e)  => {
            assert!(env::var(REQUIRE).is_err(), "no delegated cgroup subtree: {}", e);
            eprintln!("SKIP: no delegated cgroup subtree with memory ({}); run under \
                systemd-run --user --scope -p Delegate=yes", e);
            None
        }
    }
}

// This test binary as a helper in the given mode, confined to `limit`.
fn confine(dir: &Path, mode: &str, limit: &Limit) -> Outcome<Confined> {
    let exe = res!(io(env::current_exe(), "find this test binary"));
    let mut conf = res!(Confined::new(&exe, limit));
    conf.command()
        .args(["--exact", "helper", "--nocapture", "--test-threads=1"])
        .env(HELPER, dir)
        .env(MODE, mode)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    Ok(conf)
}

#[test]
fn helper() -> Outcome<()> {
    let dir = match env::var(HELPER) {
        Ok(d)   => PathBuf::from(d),
        Err(_)  => return Ok(()),
    };
    match env::var(MODE).as_deref() {
        // Touches every page of more than the cap allows, a mebibyte at a time.
        Ok("fill") => {
            let mut held = Vec::new();
            for _ in 0..512 {
                held.push(vec![7u8; MIB as usize]);
            }
            res!(io(fs::write(dir.join("filled"), format!("{}", held.len())), "note the fill"));
        }
        Ok("quiet") => {
            res!(io(fs::write(dir.join("quiet"), "ok"), "note the quiet run"));
        }
        // Arms its parent's death, says so, and sleeps long past the test.
        Ok("child") => {
            res!(arm_parent_death(Signal::Kill));
            res!(io(fs::write(dir.join("armed"), "ok"), "note the arming"));
            thread::sleep(Duration::from_secs(60));
        }
        // Starts the child, notes its number, and waits to be killed.
        Ok("parent") => {
            let conf = res!(confine(&dir, "child", &Limit::rlimit(1024 * MIB)));
            let sup = res!(conf.spawn());
            res!(io(fs::write(dir.join("child_pid"), format!("{}", sup.pid())), "note the child"));
            thread::sleep(Duration::from_secs(60));
        }
        _ => return Err(err!("No helper mode."; Test, Invalid, Input)),
    }
    Ok(())
}

#[test]
fn the_fourth_restart_in_the_window_is_refused_and_one_after_it_is_allowed() {
    let t0 = Instant::now();
    let mut budget = RestartBudget::new(3, Duration::from_secs(600));
    assert!(budget.try_restart(t0));
    assert!(budget.try_restart(t0 + Duration::from_secs(60)));
    assert!(budget.try_restart(t0 + Duration::from_secs(120)));
    assert_eq!(budget.remaining(t0 + Duration::from_secs(130)), 0);
    assert!(!budget.try_restart(t0 + Duration::from_secs(180)), "a fourth restart in ten minutes");
    // A refusal is not counted, so the window frees as the first restart ages out of it.
    assert!(!budget.try_restart(t0 + Duration::from_secs(599)));
    assert!(budget.try_restart(t0 + Duration::from_secs(601)));
    assert!(!budget.try_restart(t0 + Duration::from_secs(602)));
}

#[test]
fn a_child_over_its_memory_max_is_oom_killed_and_reported_so() -> Outcome<()> {
    let root = match root() {
        Some(r) => r,
        None    => return Ok(()),
    };
    let dir = res!(Dir::new("oom"));
    let limit = Limit::cgroup(root, "fill", 32 * MIB);
    let sup = res!(res!(confine(&dir.0, "fill", &limit)).spawn());
    let cg = match sup.cgroup() {
        Some(p) => p.to_path_buf(),
        None    => return Err(err!("A confined child has no cgroup."; Test, Missing)),
    };
    // The kernel's own account: the child sits in its own cgroup, under the cap.
    let procs = res!(io(fs::read_to_string(cg.join("cgroup.procs")), "read the child's cgroup"));
    let max = res!(io(fs::read_to_string(cg.join("memory.max")), "read memory.max"));
    assert_eq!(max.trim(), format!("{}", 32 * MIB));
    let ended = res!(sup.wait());
    assert!(ended.oom_killed(), "{:?} (cgroup held {:?})", ended, procs);
    assert!(!ended.status.success());
    assert!(!dir.file("filled").exists(), "the child filled past its cap");
    assert!(!cg.exists(), "the child's cgroup outlived it");
    Ok(())
}

#[test]
fn a_child_under_its_memory_max_ends_and_is_not_reported_oom_killed() -> Outcome<()> {
    let root = match root() {
        Some(r) => r,
        None    => return Ok(()),
    };
    let dir = res!(Dir::new("quiet"));
    let limit = Limit::cgroup(root, "quiet", 256 * MIB);
    let ended = res!(res!(res!(confine(&dir.0, "quiet", &limit)).spawn()).wait());
    assert!(ended.status.success(), "{:?}", ended);
    assert!(!ended.oom_killed());
    assert!(dir.file("quiet").exists());
    Ok(())
}

#[test]
fn without_a_cgroup_the_rlimit_stops_a_child_over_its_cap() -> Outcome<()> {
    let dir = res!(Dir::new("rlimit"));
    // The cap covers the address space, so it must leave room for the binary and its threads.
    let cap = Limit::rlimit(300 * MIB);
    let quiet = res!(res!(res!(confine(&dir.0, "quiet", &cap)).spawn()).wait());
    assert!(quiet.status.success(), "a child within its cap failed: {:?}", quiet);
    let ended = res!(res!(res!(confine(&dir.0, "fill", &cap)).spawn()).wait());
    assert!(!ended.status.success(), "{:?}", ended);
    assert!(!ended.oom_killed());
    assert!(!dir.file("filled").exists(), "the child filled past its cap");
    Ok(())
}

#[test]
fn a_child_that_arms_its_parents_death_dies_with_its_parent() -> Outcome<()> {
    let dir = res!(Dir::new("pdeath"));
    let parent = res!(res!(confine(&dir.0, "parent", &Limit::rlimit(1024 * MIB))).spawn());
    res!(until("the child to arm", || dir.file("armed").exists()));
    let text = res!(io(fs::read_to_string(dir.file("child_pid")), "read the child's number"));
    let pid: u32 = match text.trim().parse() {
        Ok(p)   => p,
        Err(e)  => return Err(err!("Child pid {:?}: {}.", text, e; Test, Invalid, Input)),
    };
    let child = match Proc::of(pid) {
        Some(p) => p,
        None    => return Err(err!("The child {} is not running.", pid; Test, Missing)),
    };
    res!(to_process(parent.pid(), Signal::Kill));
    let ended = res!(parent.wait());
    assert!(!ended.status.success());
    res!(until("the child to die with its parent", || !child.running()));
    Ok(())
}

// ── Scoped ──────────────────────────────────────────────────────────────────────────────────────

const TEST_SLICE: &str = "fe2o3_scope_test.slice";  // underscores: a dash would nest it in another slice

/// The test slice's counter, or `None`, said, where there is no user manager with memory.
fn test_slice() -> Option<PathBuf> {
    match oxedyne_fe2o3_sys::supervise::manager_has_memory() {
        Ok(true)    => (),
        other       => {
            eprintln!("skipped: no user manager with the memory controller ({:?})", other);
            return None;
        },
    }
    match start_slice(TEST_SLICE) {
        Ok(p)   => Some(p),
        Err(e)  => {
            eprintln!("skipped: the test slice could not be started: {}", e);
            None
        },
    }
}

/// **A scoped child over its cap is reported as memory, from the slice's counter.** A shell says
/// hello, then holds about 100 MB in a variable under a 32 MiB cap.
#[test]
fn a_scoped_child_over_its_cap_is_reported_as_memory() -> Outcome<()> {
    let events = match test_slice() {
        Some(p) => p,
        None    => return Ok(()),
    };
    let mut scoped = res!(Scoped::new("sh", "fe2o3-scope-test", TEST_SLICE, &events, 32 << 20));
    scoped.command()
        .args(["-c", "printf '\\006'; x=$(head -c 100000000 /dev/zero | tr '\\0' a); echo ${#x}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    let mut child = res!(scoped.spawn());
    assert!(res!(child.hello()), "the shell's hello was not read");
    let ended = res!(child.wait());
    assert!(ended.oom_killed(), "not reported as memory: {:?}", ended);
    Ok(())
}

/// **A scope that never started is told by its missing hello.** The slice named is a service, which
/// `systemd-run` refuses; the program never runs, and no kill is counted.
#[test]
fn a_scope_that_never_started_says_no_hello() -> Outcome<()> {
    let events = match test_slice() {
        Some(p) => p,
        None    => return Ok(()),
    };
    let mut scoped = res!(Scoped::new("sh", "fe2o3-scope-test", "not-a-slice.service", &events, 32 << 20));
    scoped.command().args(["-c", "printf '\\006'"]).stdin(Stdio::null()).stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = res!(scoped.spawn());
    assert!(!res!(child.hello()), "a hello from a scope that was refused");
    let ended = res!(child.wait());
    assert!(!ended.status.success());
    assert_eq!(ended.kills, SliceKills::None);
    Ok(())
}
