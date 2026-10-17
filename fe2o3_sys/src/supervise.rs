//! Supervision of a child process: a memory cap it cannot pass, a death that follows its parent's,
//! and a budget on how often it may be restarted.
//!
//! **The cap.** Given a cgroup v2 root delegated to the caller with the `memory` controller (a
//! systemd unit or scope with `Delegate=yes`), [`Confined`] makes a child cgroup under it, sets
//! its `memory.max`, a `memory.swap.max` of 0 so that the cap is not met by swapping, and
//! `memory.oom.group`, so that an out-of-memory kill takes every process in it. The child enters
//! the cgroup before the program runs: it starts as `/bin/sh`, which writes itself into the
//! cgroup's `cgroup.procs`, sets any address-space limit, and then replaces itself with the
//! program, keeping its process number. When it ends, [`Supervised::wait`] reads the cgroup's
//! `memory.events` to report whether the kernel killed it for memory, kills anything it left
//! behind, and removes the cgroup. The root is a parameter: [`CgroupRoot::open`] takes it as
//! given, and [`CgroupRoot::from_own_cgroup`] makes one of the caller's own delegated cgroup.
//!
//! Where there is no delegated root, the cap falls back to `RLIMIT_AS`, the address-space limit,
//! which an allocation meets as a failure rather than a kill.
//!
//! **The parent's death.** A child that is itself a program of ours calls [`arm_parent_death`]
//! first thing, to have the kernel send it a signal when its parent ends. The kernel sends it when
//! the *thread* that started the child ends, so a supervisor starts its children from a thread
//! that lives as long as it does. The parent's number travels in [`SUPERVISOR_ENV`], so a child
//! whose parent ended before it was armed finds out and is told so.
//!
//! **The restart budget.** [`RestartBudget`] allows at most `n` restarts in a sliding window, on a
//! clock the caller passes in.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use crate::signal::Signal;

use oxedyne_fe2o3_core::prelude::*;

use nix::sys::prctl::set_pdeathsig;
use nix::unistd::getppid;

use std::{
    collections::VecDeque,
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Child,
        Command,
        ExitStatus,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

/// Where the cgroup v2 hierarchy is mounted.
pub const CGROUP_FS: &str = "/sys/fs/cgroup";

/// The variable through which a child learns its supervisor's process number.
pub const SUPERVISOR_ENV: &str = "FE2O3_SUPERVISOR_PID";

// The shell that enters the cgroup, sets the address-space limit and becomes the program. Its
// arguments are the cgroup directory or nothing, the limit in KiB or nothing, then the program and
// its arguments.
const TRAMPOLINE: &str = "cg=$1; kb=$2; shift 2
if [ -n \"$cg\" ]; then echo $$ > \"$cg/cgroup.procs\" || exit 126; fi
if [ -n \"$kb\" ]; then ulimit -v \"$kb\" || exit 126; fi
exec \"$@\"
";

// How long a finished child's cgroup is given to empty before it is removed.
const EMPTYING: Duration = Duration::from_secs(5);
const NAP:      Duration = Duration::from_millis(10);

fn io<T>(r: std::io::Result<T>, what: &str, path: &Path) -> Outcome<T> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("Cannot {} {}: {}.", what, path.display(), e; IO, File)),
    }
}

// A cgroup name is one path component the caller chose, never a way up or across.
fn check_name(name: &str) -> Outcome<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(err!(
            "The cgroup name {:?} is not one path component.", name;
            Invalid, Input, Name));
    }
    Ok(())
}

/// The caller's own cgroup v2 directory, read from `/proc/self/cgroup`.
pub fn own_cgroup() -> Outcome<PathBuf> {
    let src = Path::new("/proc/self/cgroup");
    let text = res!(io(fs::read_to_string(src), "read", src));
    match text.lines().find_map(|l| l.strip_prefix("0::")) {
        Some(rel) => Ok(Path::new(CGROUP_FS).join(rel.trim().trim_start_matches('/'))),
        None      => Err(err!(
            "The caller is not in a cgroup v2 hierarchy: {:?}.", text;
            Missing, System)),
    }
}

/// A cgroup v2 directory delegated to the caller, under which each child gets a cgroup of its own
/// with the `memory` controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupRoot {
    path: PathBuf,
}

impl CgroupRoot {
    /// Takes `path` as the root, enabling the `memory` controller for its children where it is not
    /// already. It refuses a root whose parent does not offer `memory`, and one that still holds
    /// processes of its own, which cgroup v2 forbids from enabling it.
    pub fn open<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let path = path.as_ref().to_path_buf();
        let controllers = path.join("cgroup.controllers");
        let offered = res!(io(fs::read_to_string(&controllers), "read", &controllers));
        if !offered.split_whitespace().any(|c| c == "memory") {
            return Err(err!(
                "The cgroup {} is not given the memory controller ({:?}); it must be delegated, \
                as by a systemd unit or scope with Delegate=yes.", path.display(), offered.trim();
                Missing, System));
        }
        let subtree = path.join("cgroup.subtree_control");
        let enabled = res!(io(fs::read_to_string(&subtree), "read", &subtree));
        if !enabled.split_whitespace().any(|c| c == "memory") {
            if let Err(e) = fs::write(&subtree, "+memory") {
                return Err(err!(
                    "Cannot enable the memory controller below {}: {}. A cgroup that holds \
                    processes of its own cannot; CgroupRoot::from_own_cgroup moves them to a leaf.",
                    path.display(), e;
                    IO, System));
            }
        }
        Ok(Self { path })
    }

    /// Makes a root of the caller's own cgroup, which must be delegated to it: every process in it,
    /// the caller among them, moves to the child cgroup `leaf`, and the cgroup is then opened as the
    /// root. A caller already in a cgroup named `leaf` opens that cgroup's parent, so a second call,
    /// or a restart in place, finds the same root.
    pub fn from_own_cgroup(leaf: &str) -> Outcome<Self> {
        res!(check_name(leaf));
        let own = res!(own_cgroup());
        if own.file_name() == Some(OsStr::new(leaf)) {
            return match own.parent() {
                Some(p) => Self::open(p),
                None    => Err(err!("The cgroup {} has no parent.", own.display(); Missing, Path)),
            };
        }
        let leaf_dir = own.join(leaf);
        if let Err(e) = fs::create_dir(&leaf_dir) {
            if e.kind() != ErrorKind::AlreadyExists {
                return Err(err!("Cannot make the cgroup {}: {}.", leaf_dir.display(), e; IO, Create));
            }
        }
        let procs = own.join("cgroup.procs");
        let into = leaf_dir.join("cgroup.procs");
        // A process forked while the others move lands in the old cgroup, so read until it is empty.
        for _ in 0..8 {
            let text = res!(io(fs::read_to_string(&procs), "read", &procs));
            let pids: Vec<&str> = text.split_whitespace().collect();
            if pids.is_empty() {
                return Self::open(own);
            }
            for pid in pids {
                if let Err(e) = fs::write(&into, pid) {
                    // A process that ended meanwhile is no longer there to move.
                    if e.raw_os_error() != Some(3) {
                        return Err(err!(
                            "Cannot move process {} into {}: {}.", pid, leaf_dir.display(), e;
                            IO, System));
                    }
                }
            }
        }
        Err(err!("The cgroup {} kept gaining processes as they were moved out.", own.display();
            Excessive, System))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The memory cap a child is held to.
#[derive(Clone, Debug)]
pub struct Limit<'a> {
    /// The cap in bytes: the cgroup's `memory.max`, or without a cgroup the address-space limit.
    pub memory_max: u64,
    /// The root and the name of the child's cgroup below it, if there is one.
    pub cgroup:     Option<(&'a CgroupRoot, String)>,
    /// An address-space limit in bytes to set as well as the cgroup's cap. Without a cgroup the
    /// limit is this, or failing it `memory_max`.
    pub rlimit_as:  Option<u64>,
}

impl<'a> Limit<'a> {
    /// A cap of `memory_max` bytes in the cgroup `name` below `root`.
    pub fn cgroup(root: &'a CgroupRoot, name: &str, memory_max: u64) -> Self {
        Self { memory_max, cgroup: Some((root, name.to_string())), rlimit_as: None }
    }

    /// A cap of `memory_max` bytes of address space, the fallback where no root is delegated.
    pub fn rlimit(memory_max: u64) -> Self {
        Self { memory_max, cgroup: None, rlimit_as: None }
    }

    /// Sets an address-space limit as well, in bytes.
    pub fn with_rlimit_as(mut self, bytes: u64) -> Self {
        self.rlimit_as = Some(bytes);
        self
    }
}

/// A child ready to start under its cap. Its [`Command`] takes further arguments, environment,
/// a working directory and its standard streams before [`Confined::spawn`] starts it. A child
/// never started has its cgroup removed when this is dropped.
#[derive(Debug)]
pub struct Confined {
    cmd:    Command,
    cgroup: Option<PathBuf>,
}

impl Confined {
    /// Makes the child's cgroup, if the limit names one, and the command that starts `program`
    /// inside it.
    pub fn new<P: AsRef<OsStr>>(program: P, limit: &Limit) -> Outcome<Self> {
        let cgroup = match &limit.cgroup {
            Some((root, name)) => Some(res!(Self::make_cgroup(root, name, limit.memory_max))),
            None               => None,
        };
        let rlimit = match (&cgroup, limit.rlimit_as) {
            (_, Some(b))    => Some(b),
            (None, None)    => Some(limit.memory_max),
            (Some(_), None) => None,
        };
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(TRAMPOLINE).arg("sh");
        match &cgroup {
            Some(p) => cmd.arg(p),
            None    => cmd.arg(""),
        };
        match rlimit {
            // ulimit counts in KiB; a cap below one is no cap the program could run in.
            Some(b) => cmd.arg(format!("{}", std::cmp::max(b / 1024, 1))),
            None    => cmd.arg(""),
        };
        cmd.arg(program).env(SUPERVISOR_ENV, format!("{}", std::process::id()));
        Ok(Self { cmd, cgroup })
    }

    fn make_cgroup(root: &CgroupRoot, name: &str, memory_max: u64) -> Outcome<PathBuf> {
        res!(check_name(name));
        let dir = root.path().join(name);
        if dir.exists() {
            // One left by a supervisor that died is removed if it is empty, and refused if not.
            if let Err(e) = fs::remove_dir(&dir) {
                return Err(err!(
                    "The cgroup {} already exists and cannot be removed: {}.", dir.display(), e;
                    Exists, System));
            }
        }
        res!(io(fs::create_dir(&dir), "make the cgroup", &dir));
        let set = |file: &str, value: String| -> Outcome<()> {
            let path = dir.join(file);
            io(fs::write(&path, value), "write", &path)
        };
        let made = (|| -> Outcome<()> {
            res!(set("memory.max", format!("{}", memory_max)));
            // Absent where the kernel was built without swap accounting, and then there is no swap
            // to meet the cap with.
            if dir.join("memory.swap.max").exists() {
                res!(set("memory.swap.max", "0".to_string()));
            }
            res!(set("memory.oom.group", "1".to_string()));
            Ok(())
        })();
        match made {
            Ok(())  => Ok(dir),
            Err(e)  => {
                let _ = fs::remove_dir(&dir);
                Err(e)
            }
        }
    }

    /// The command that starts the child, for its further arguments, environment and streams.
    pub fn command(&mut self) -> &mut Command {
        &mut self.cmd
    }

    /// The child's cgroup, if it has one.
    pub fn cgroup(&self) -> Option<&Path> {
        self.cgroup.as_deref()
    }

    /// Starts the child.
    pub fn spawn(mut self) -> Outcome<Supervised> {
        let cgroup = self.cgroup.take();
        match self.cmd.spawn() {
            Ok(child)   => Ok(Supervised { child, cgroup, ended: false }),
            Err(e)      => {
                if let Some(dir) = &cgroup {
                    let _ = fs::remove_dir(dir);
                }
                Err(err!("Cannot start the child: {}.", e; IO, System))
            }
        }
    }
}

impl Drop for Confined {
    fn drop(&mut self) {
        if let Some(dir) = self.cgroup.take() {
            let _ = fs::remove_dir(&dir);
        }
    }
}

/// How a supervised child ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ended {
    pub status:     ExitStatus,
    /// How many processes the kernel killed in the child's cgroup for want of memory.
    pub oom_kills:  u64,
}

impl Ended {
    /// Did the kernel kill the child, or a process it started, for passing its cap?
    pub fn oom_killed(&self) -> bool {
        self.oom_kills > 0
    }
}

/// A child started under its cap. Dropping one that has not been waited on kills it and everything
/// in its cgroup, and removes the cgroup.
#[derive(Debug)]
pub struct Supervised {
    child:  Child,
    cgroup: Option<PathBuf>,
    ended:  bool,
}

impl Supervised {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The child's cgroup, if it has one.
    pub fn cgroup(&self) -> Option<&Path> {
        self.cgroup.as_deref()
    }

    /// The child, for its standard streams.
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Kills the child, and with a cgroup everything in it.
    pub fn kill(&mut self) -> Outcome<()> {
        if let Some(dir) = &self.cgroup {
            if fs::write(dir.join("cgroup.kill"), "1").is_ok() {
                return Ok(());
            }
        }
        match self.child.kill() {
            Ok(())  => Ok(()),
            Err(e)  => Err(err!("Cannot kill the child {}: {}.", self.child.id(), e; IO, System)),
        }
    }

    /// Waits for the child to end, then reports how, and removes its cgroup.
    pub fn wait(mut self) -> Outcome<Ended> {
        let status = match self.child.wait() {
            Ok(s)   => s,
            Err(e)  => return Err(err!("Cannot wait on the child {}: {}.", self.child.id(), e;
                IO, System)),
        };
        self.finish(status)
    }

    /// Reports how the child ended, if it has, and then removes its cgroup.
    pub fn try_wait(&mut self) -> Outcome<Option<Ended>> {
        match self.child.try_wait() {
            Ok(Some(s)) => match self.finish(s) {
                Ok(e)   => Ok(Some(e)),
                Err(e)  => Err(e),
            },
            Ok(None)    => Ok(None),
            Err(e)      => Err(err!("Cannot ask whether the child {} has ended: {}.",
                self.child.id(), e; IO, System)),
        }
    }

    fn finish(&mut self, status: ExitStatus) -> Outcome<Ended> {
        self.ended = true;
        let dir = match self.cgroup.take() {
            Some(d) => d,
            None    => return Ok(Ended { status, oom_kills: 0 }),
        };
        let events = dir.join("memory.events");
        let text = res!(io(fs::read_to_string(&events), "read", &events));
        let oom_kills = text.lines()
            .find_map(|l| l.strip_prefix("oom_kill "))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        res!(remove_cgroup(&dir));
        Ok(Ended { status, oom_kills })
    }
}

impl Drop for Supervised {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let _ = self.kill();
        if let Ok(status) = self.child.wait() {
            let _ = self.finish(status);
        }
    }
}

// Kills whatever a finished child left in its cgroup, and removes the cgroup once it is empty.
fn remove_cgroup(dir: &Path) -> Outcome<()> {
    let _ = fs::write(dir.join("cgroup.kill"), "1");
    let begun = Instant::now();
    loop {
        match fs::remove_dir(dir) {
            Ok(())  => return Ok(()),
            Err(e)  => if begun.elapsed() > EMPTYING {
                return Err(err!(
                    "The cgroup {} did not empty in {} s: {}.", dir.display(), EMPTYING.as_secs(), e;
                    IO, Timeout));
            },
        }
        thread::sleep(NAP);
    }
}

/// Has the kernel send `sig` to the caller when its parent ends, for a child to call before
/// anything else. It answers an error, and the caller should then end, where the parent has already
/// ended: the supervisor named in [`SUPERVISOR_ENV`], or where that is not set, any parent but
/// init.
pub fn arm_parent_death(sig: Signal) -> Outcome<()> {
    if let Err(e) = set_pdeathsig(sig.nix()) {
        return Err(err!("Cannot arm the parent's death signal: {}.", e; IO, System));
    }
    let parent = getppid().as_raw();
    let expected: Option<i32> = match env::var(SUPERVISOR_ENV) {
        Ok(v)   => match v.trim().parse() {
            Ok(p)   => Some(p),
            Err(_)  => return Err(err!(
                "{} holds {:?}, which is not a process number.", SUPERVISOR_ENV, v;
                Invalid, Input)),
        },
        Err(_)  => None,
    };
    let orphaned = match expected {
        Some(p) => parent != p,
        None    => parent == 1,
    };
    if orphaned {
        return Err(err!(
            "The parent ended before its death signal was armed (parent now {}, expected {:?}).",
            parent, expected;
            Missing, System));
    }
    Ok(())
}

/// At most `max` restarts in any `window`, on a clock the caller passes in. A refused restart is
/// not counted, so the budget frees as the oldest restart ages out of the window.
#[derive(Clone, Debug)]
pub struct RestartBudget {
    max:    usize,
    window: Duration,
    starts: VecDeque<Instant>,
}

impl RestartBudget {
    pub fn new(max: usize, window: Duration) -> Self {
        Self { max, window, starts: VecDeque::new() }
    }

    fn age_out(&mut self, now: Instant) {
        while let Some(first) = self.starts.front() {
            match now.checked_duration_since(*first) {
                Some(age) if age >= self.window => { self.starts.pop_front(); }
                _                               => break,
            }
        }
    }

    /// Records a restart at `now` and answers true, or answers false, recording nothing, where
    /// `max` already fall in the window ending at `now`.
    pub fn try_restart(&mut self, now: Instant) -> bool {
        self.age_out(now);
        if self.starts.len() >= self.max {
            return false;
        }
        self.starts.push_back(now);
        true
    }

    /// How many restarts the window ending at `now` still allows.
    pub fn remaining(&self, now: Instant) -> usize {
        let used = self.starts.iter()
            .filter(|s| match now.checked_duration_since(**s) {
                Some(age)   => age < self.window,
                None        => true,
            })
            .count();
        self.max.saturating_sub(used)
    }
}
