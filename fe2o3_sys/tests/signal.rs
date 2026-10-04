//! Tests for `fe2o3_sys::signal` against real processes and the kernel's own account of them.
//!
//! Every tree is made of `sh` and `sleep`, whose parentage the kernel reports in
//! `/proc/<pid>/status`, a file the module under test does not read. A signal's effect is seen in
//! what the processes do with it: a shell that traps `TERM` writes a file, and one that does not
//! dies. The group tests run a copy of this test binary as the program that signals, because the
//! caller of `to_own_group` is itself in the group it signals and must outlive it.

#![cfg(all(target_os = "linux", feature = "signal"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::stop::on_stop_request;
use oxedyne_fe2o3_sys::signal::{
    still_running,
    to_members,
    to_own_group,
    tree_of,
    Proc,
    Signal,
};

use nix::sys::signal::{
    killpg,
    Signal as Sig,
};
use nix::unistd::Pid;

use std::{
    collections::BTreeMap,
    env,
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{
        Child,
        Command,
        Stdio,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

const HELPER:   &str        = "FE2O3_SYS_SIGNAL_HELPER";
const MODE:     &str        = "FE2O3_SYS_SIGNAL_MODE";
const NAP:      Duration    = Duration::from_millis(20);
const PATIENCE: Duration    = Duration::from_secs(8);

// A shell that notes the signal it is given and ends, after a pause if it is told one.
const TRAPPER: &str = "trap 'sleep \"$4\"; echo got > \"$1\"; exit 0' TERM
echo up > \"$2\"
n=0
while [ $n -lt $3 ]; do sleep 0.1; n=$((n + 1)); done
";

// A shell that starts the trapper below it, and notes the signal itself.
const PARENT: &str = "trap 'echo got > \"$1\"; exit 0' TERM
sh \"$2\" \"$3\" \"$4\" 50 0 &
n=0
while [ $n -lt 50 ]; do sleep 0.1; n=$((n + 1)); done
";

fn io<T>(r: std::io::Result<T>, what: &str) -> Outcome<T> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("{}: {}.", what, e; IO, File)),
    }
}

// A directory of this test's own, under the caller's temporary directory.
struct Dir(PathBuf);

impl Dir {
    fn new(what: &str) -> Outcome<Self> {
        let path = env::temp_dir().join(format!("fe2o3_sys_signal_{}_{}", what, std::process::id()));
        let _ = fs::remove_dir_all(&path);
        res!(io(fs::create_dir_all(&path), "make the test directory"));
        Ok(Self(path))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn script(&self, name: &str, body: &str) -> Outcome<PathBuf> {
        let path = self.file(name);
        res!(io(fs::write(&path, body), "write a script"));
        Ok(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// A process group that is ended however the test does.
struct Group(i32);

impl Drop for Group {
    fn drop(&mut self) {
        let _ = killpg(Pid::from_raw(self.0), Sig::SIGKILL);
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

// What the kernel's status file says a process's parent is.
fn parent_of(pid: u32) -> Option<u32> {
    let text = fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("PPid:"))
        .and_then(|v| v.trim().parse().ok())
}

// A root shell, in a group of its own, over a shell, over a `sleep`, and the tree read once all three stand.
fn three_deep() -> Outcome<(Child, Group, Vec<Proc>)> {
    let child = res!(io(Command::new("sh")
        .args(["-c", "sh -c 'sleep 60; :'; :"])
        .process_group(0)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn(), "start the tree"));
    let group = Group(child.id() as i32);
    let root = child.id();
    until("the tree to stand three deep", || {
        match tree_of(root) {
            Ok(t)   => t.len() >= 3,
            Err(_)  => false,
        }
    })?;
    let tree = res!(tree_of(root));
    Ok((child, group, tree))
}

#[test]
fn tree_of_reads_a_real_tree_with_each_process_after_the_one_that_started_it() -> Outcome<()> {
    let (mut child, _group, tree) = res!(three_deep());
    assert_eq!(tree[0].pid, child.id(), "the tree starts at the process asked about");
    assert_eq!(tree.len(), 3, "a shell, a shell and a sleep: {:?}", tree);
    for (at, one) in tree.iter().enumerate().skip(1) {
        let parent = match parent_of(one.pid) {
            Some(p) => p,
            None    => return Err(err!("The kernel has no status for process {}.", one.pid; Test, Missing)),
        };
        let before = tree[..at].iter().any(|p| p.pid == parent);
        assert!(before, "process {} was started by {}, which is not earlier in {:?}", one.pid, parent, tree);
    }
    assert_eq!(still_running(&tree).len(), 3, "all three run");
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

#[test]
fn a_signal_to_a_tree_goes_to_the_farthest_first_and_ends_every_process() -> Outcome<()> {
    let (mut child, _group, tree) = res!(three_deep());
    let mut parents: BTreeMap<u32, u32> = BTreeMap::new();
    for one in &tree {
        if let Some(p) = parent_of(one.pid) {
            parents.insert(one.pid, p);
        }
    }
    let sent = to_members(&tree, Signal::Terminate);
    assert_eq!(sent.len(), 3, "every process was reached: {:?}", sent);
    // The kernel's own parentage, read before anything was signalled: no process is reached before
    // each process it started.
    for (at, pid) in sent.iter().enumerate() {
        for (kid, parent) in &parents {
            if parent == pid {
                let kid_at = sent.iter().position(|p| p == kid);
                assert!(
                    matches!(kid_at, Some(k) if k < at),
                    "process {} was reached before {}, which it started: {:?}", pid, kid, sent);
            }
        }
    }
    let _ = child.wait();
    res!(until("every process to end", || still_running(&tree).is_empty()));
    Ok(())
}

#[test]
fn a_shell_that_ends_at_once_does_not_say_the_program_beneath_it_has() -> Outcome<()> {
    let dir = res!(Dir::new("trapper"));
    let trapper = res!(dir.script("trapper.sh", TRAPPER));
    let (got, up) = (dir.file("got"), dir.file("up"));
    // The program is slow to end: it traps the signal, saves for a second and goes. The shell above
    // it has no trap and ends where it stands, which is what `sh -c` does for an editor.
    let mut child = res!(io(Command::new("sh")
        .arg("-c")
        .arg("sh \"$0\" \"$1\" \"$2\" 100 1; :")
        .arg(&trapper).arg(&got).arg(&up)
        .process_group(0)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn(), "start the shell"));
    let _group = Group(child.id() as i32);
    res!(until("the program to start", || up.is_file()));
    let tree = res!(tree_of(child.id()));
    let program = tree[1];
    assert!(tree.len() >= 2, "the shell and the program: {:?}", tree);
    to_members(&tree, Signal::Terminate);
    let _ = child.wait();
    assert!(!tree[0].running(), "the shell ended with the signal");
    assert!(program.running(), "the program, saving for a second, is still running: {:?}", still_running(&tree));
    assert!(!got.exists(), "it has not saved yet");
    res!(until("the program to end", || still_running(&tree).is_empty()));
    assert!(got.is_file(), "the program was reached and saved before it ended");
    Ok(())
}

// The program that signals, run by the tests below as a copy of this binary. It is told its
// directory, and whether it was started to lead a group; it starts a shell that starts another,
// each noting `TERM` in a file, and signals its own group.
#[test]
fn helper_that_signals_its_own_group() -> Outcome<()> {
    let dir = match env::var(HELPER) {
        Ok(d)   => PathBuf::from(d),
        Err(_)  => return Ok(()),
    };
    let leads = env::var(MODE).map(|m| m == "lead").unwrap_or(false);
    // The caller is in the group it signals, so it hears the signal too and must not end by it.
    res!(on_stop_request(|_| {}));
    until("the stop listener to be installed", || {
        match fs::read_to_string("/proc/self/status") {
            // The mask of caught signals, the fifteenth bit of which is SIGTERM.
            Ok(s)   => s.lines()
                .find_map(|l| l.strip_prefix("SigCgt:"))
                .and_then(|m| u64::from_str_radix(m.trim(), 16).ok())
                .map(|m| m & (1 << 14) != 0)
                .unwrap_or(false),
            Err(_)  => false,
        }
    })?;
    let (mid, leaf, up) = (dir.join("mid"), dir.join("leaf"), dir.join("up"));
    let child = res!(io(Command::new("sh")
        .arg(dir.join("parent.sh")).arg(&mid).arg(dir.join("trapper.sh")).arg(&leaf).arg(&up)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn(), "start the shells"));
    res!(until("the shells to start", || up.is_file()));
    let sent = res!(to_own_group(Signal::Terminate));
    println!("HELPER sent={} leads={}", sent, leads);
    match sent {
        true    => {
            res!(until("both shells to note the signal", || mid.is_file() && leaf.is_file()));
            println!("HELPER noted=2");
        },
        false   => {
            thread::sleep(Duration::from_millis(1_000));
            println!("HELPER noted={}", [&mid, &leaf].iter().filter(|p| p.is_file()).count());
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
        },
    }
    Ok(())
}

// Runs the helper in this directory, and what it printed.
fn run_helper(dir: &Dir, lead: bool) -> Outcome<(Vec<String>, Group)> {
    let exe = res!(io(env::current_exe(), "find this test binary"));
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "helper_that_signals_its_own_group", "--nocapture", "--test-threads=1"])
        .env(HELPER, &dir.0)
        .env(MODE, if lead { "lead" } else { "follow" })
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if lead {
        cmd.process_group(0);
    }
    let child = res!(io(cmd.spawn(), "start the helper"));
    let group = Group(child.id() as i32);
    let out = res!(io(child.wait_with_output(), "wait on the helper"));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "the helper failed ({}): {}", out.status, text);
    // The harness leaves its own "test ... " on the line the first print lands on.
    Ok((text.lines().filter_map(|l| l.find("HELPER ").map(|i| l[i..].to_string())).collect(), group))
}

fn stand_the_shells(dir: &Dir) -> Outcome<()> {
    res!(dir.script("trapper.sh", TRAPPER));
    res!(dir.script("parent.sh", PARENT));
    Ok(())
}

#[test]
fn a_group_signal_reaches_a_child_and_a_grandchild_of_a_caller_that_leads_the_group() -> Outcome<()> {
    let dir = res!(Dir::new("lead"));
    res!(stand_the_shells(&dir));
    let (said, _group) = res!(run_helper(&dir, true));
    assert!(said.iter().any(|l| l == "HELPER sent=true leads=true"), "{:?}", said);
    assert!(said.iter().any(|l| l == "HELPER noted=2"), "{:?}", said);
    assert!(dir.file("mid").is_file(), "the child noted the signal");
    assert!(dir.file("leaf").is_file(), "and so did the grandchild");
    Ok(())
}

#[test]
fn a_caller_that_does_not_lead_its_group_declines_and_signals_nobody() -> Outcome<()> {
    let dir = res!(Dir::new("follow"));
    res!(stand_the_shells(&dir));
    let (said, _group) = res!(run_helper(&dir, false));
    assert!(said.iter().any(|l| l == "HELPER sent=false leads=false"), "{:?}", said);
    assert!(said.iter().any(|l| l == "HELPER noted=0"), "{:?}", said);
    assert!(!dir.file("mid").exists() && !dir.file("leaf").exists(), "a signal went out");
    Ok(())
}

#[test]
fn a_process_is_known_by_its_start_and_not_by_its_number_alone() -> Outcome<()> {
    let mut child = res!(io(Command::new("sleep").arg("30").spawn(), "start a sleep"));
    let seen = Proc::of(child.id());
    assert!(matches!(seen, Some(p) if p.pid == child.id() && p.running()), "{:?}", seen);
    let _ = child.kill();
    let _ = child.wait();
    let gone = match seen {
        Some(p) => p,
        None    => return Err(err!("The sleep was not seen."; Test, Missing)),
    };
    assert!(!gone.running(), "a process that has been waited on is not running");
    assert_eq!(Proc::of(child.id()), None, "and its number is not one");
    Ok(())
}

#[test]
fn a_process_that_has_ended_and_awaits_its_parents_wait_is_not_running() -> Outcome<()> {
    let mut child = res!(io(Command::new("sleep").arg("0.4").spawn(), "start a sleep"));
    let seen = match Proc::of(child.id()) {
        Some(p) => p,
        None    => return Err(err!("The sleep was not seen."; Test, Missing)),
    };
    assert!(seen.running());
    // The kernel's own word for it: state Z in its status, with this process not having waited.
    let state = |pid: u32| fs::read_to_string(format!("/proc/{}/status", pid)).ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("State:").map(|v| v.trim().to_string())));
    res!(until("the sleep to end and wait to be waited on", || {
        state(child.id()).map(|s| s.starts_with('Z')).unwrap_or(false)
    }));
    assert!(!seen.running(), "an ended process, still in the table, is not running");
    assert_eq!(Proc::of(child.id()), None, "and is not one that can be signalled");
    assert!(still_running(&[seen]).is_empty());
    let _ = child.wait();
    Ok(())
}
