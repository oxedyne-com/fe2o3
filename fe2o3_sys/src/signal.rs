//! Signals to the caller's own process group, to one process, and to a process and all it started.
//!
//! A program that runs a child it must end when asked to stop, an editor under `ore edit`, has two
//! ways to reach it. The child shares the caller's process group where the caller was started as
//! a job by an interactive shell, and a signal to the group then reaches the child and whatever it
//! started in turn. Where the caller was not, as under a script or a test harness, the group is its
//! parent's, and signalling it would reach that parent's other processes; [`to_own_group`] declines
//! to, and [`to_tree`] reaches the child and everything it started, which is what a child run through
//! `sh -c` needs, since the shell is not the program.
//!
//! The shell that stands between the caller and the program ends at once when it is signalled, and
//! the caller waiting on it alone would think the program over while it ran on. [`tree_of`] reads the
//! whole tree before anything is signalled, [`to_members`] signals the processes farthest from the
//! child first, so that none is left without its parent's death to find out about, and
//! [`still_running`] answers which of them are yet to end.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use nix::sys::signal::{
    kill,
    killpg,
    Signal as Sig,
};
use nix::unistd::{
    getpgrp,
    getpid,
    Pid,
};

/// A signal that asks a process to end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    Hangup,
    Interrupt,
    Terminate,
}

impl Signal {
    fn nix(&self) -> Sig {
        match self {
            Self::Hangup    => Sig::SIGHUP,
            Self::Interrupt => Sig::SIGINT,
            Self::Terminate => Sig::SIGTERM,
        }
    }
}

/// Does the caller lead its own process group?
pub fn leads_its_group() -> bool {
    getpgrp() == getpid()
}

/// Sends `sig` to every process in the caller's own process group, the caller included, and
/// answers whether it did.
///
/// It declines, answering false and sending nothing, where the caller does not lead the group:
/// the group is then shared with the process that started the caller, and with its other children.
pub fn to_own_group(sig: Signal) -> Outcome<bool> {
    if !leads_its_group() {
        return Ok(false);
    }
    match killpg(getpgrp(), sig.nix()) {
        Ok(())  => Ok(true),
        Err(e)  => Err(err!(
            "Cannot send {:?} to the caller's own process group: {}.", sig, e;
            IO, System)),
    }
}

/// Sends `sig` to the one process `pid`.
pub fn to_process(pid: u32, sig: Signal) -> Outcome<()> {
    let pid = match i32::try_from(pid) {
        Ok(p)   => Pid::from_raw(p),
        Err(_)  => return Err(err!(
            "The process number {} is not one a signal can be sent to.", pid;
            Invalid, Input, Range)),
    };
    match kill(pid, sig.nix()) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!(
            "Cannot send {:?} to process {}: {}.", sig, pid, e;
            IO, System)),
    }
}

/// A process, told from a later one that is given its number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Proc {
    pub pid:    u32,
    start:      u64,    // the tick it began on, which a process that takes the number later does not share
}

impl Proc {
    /// The process now running as `pid`, where there is one that has not ended.
    pub fn of(pid: u32) -> Option<Self> {
        match Stat::of(pid) {
            Some(st) if st.state != 'Z' => Some(Self { pid, start: st.start }),
            _                           => None,
        }
    }

    /// Is it still running? One that has ended and awaits only its parent's wait is not.
    pub fn running(&self) -> bool {
        match Stat::of(self.pid) {
            Some(st)    => st.start == self.start && st.state != 'Z',
            None        => false,
        }
    }
}

// What /proc/<pid>/stat says of a process, the fields this module asks.
struct Stat {
    state:  char,
    parent: u32,
    start:  u64,
}

impl Stat {
    fn of(pid: u32) -> Option<Self> {
        let text = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Option<Self> {
        // The command name is in parentheses and may hold spaces and brackets, so the fields are
        // counted from the last one: the state is the first after it, the parent the second, and
        // the start time the twentieth.
        let after = &text[text.rfind(')')? + 1..];
        let mut fields = after.split_whitespace();
        let state = fields.next()?.chars().next()?;
        let parent = fields.next()?.parse().ok()?;
        let start = fields.nth(17)?.parse().ok()?;
        Some(Self { state, parent, start })
    }
}

/// `pid` and every process it started, however deep, as they are now: those nearest `pid` come
/// first, so that a process is always after the one that started it.
///
/// A process that has ended and awaits only its parent's wait is not in it, and nor is anything it
/// started that was passed to another parent when it ended.
pub fn tree_of(pid: u32) -> Outcome<Vec<Proc>> {
    let dir = match std::fs::read_dir("/proc") {
        Ok(d)   => d,
        Err(e)  => return Err(err!(e,
            "Cannot read /proc to find the processes process {} started.", pid;
            IO, File, Read)),
    };
    let mut all: Vec<(u32, Stat)> = Vec::new();
    for entry in dir.flatten() {
        let me: u32 = match entry.file_name().to_str().and_then(|n| n.parse().ok()) {
            Some(n) => n,
            None    => continue,
        };
        match Stat::of(me) {
            Some(st) if st.state != 'Z' => all.push((me, st)),
            _                           => continue,
        }
    }
    let mut found: Vec<Proc> = Vec::new();
    for (me, st) in &all {
        if *me == pid {
            found.push(Proc { pid: *me, start: st.start });
        }
    }
    let mut at = 0;
    while at < found.len() {
        let of = found[at].pid;
        at += 1;
        for (me, st) in &all {
            if st.parent == of && !found.iter().any(|p| p.pid == *me) {
                found.push(Proc { pid: *me, start: st.start });
            }
        }
    }
    Ok(found)
}

/// Sends `sig` to each of `tree`, the farthest from the first first, and answers the processes it
/// reached in the order it reached them.
///
/// A process that has ended since the tree was read, or whose number another process now holds, is
/// passed over.
pub fn to_members(tree: &[Proc], sig: Signal) -> Vec<u32> {
    let mut sent = Vec::new();
    for one in tree.iter().rev() {
        if Proc::of(one.pid) != Some(*one) {
            continue;
        }
        if to_process(one.pid, sig).is_ok() {
            sent.push(one.pid);
        }
    }
    sent
}

/// Sends `sig` to `pid` and to every process it started, however deep, the farthest first, and
/// answers the processes it reached in the order it reached them.
pub fn to_tree(pid: u32, sig: Signal) -> Outcome<Vec<u32>> {
    let tree = res!(tree_of(pid));
    Ok(to_members(&tree, sig))
}

/// Which of `tree` are yet to end.
pub fn still_running(tree: &[Proc]) -> Vec<Proc> {
    tree.iter().filter(|p| p.running()).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command;

    #[test]
    fn a_name_with_brackets_and_spaces_does_not_move_the_fields() {
        let line = "4242 (a b) (c) d) S 17 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 20 \
            18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let st = Stat::parse(line);
        assert!(matches!(st, Some(Stat { state: 'S', parent: 17, start: 987_654 })));
        assert!(Stat::parse("no brackets here").is_none());
        assert!(Stat::parse("4242 (cut) S 17").is_none());
    }

    // The kernel's own order of events: init began before the process this test has just started, and
    // the new one began a moment ago by the clock `/proc/uptime` keeps.
    #[test]
    fn the_start_it_reads_agrees_with_the_order_the_kernel_started_the_processes_in() {
        let mut child = match Command::new("sleep").arg("5").spawn() {
            Ok(c)   => c,
            Err(e)  => panic!("cannot start a sleep: {}", e),
        };
        let kid = Stat::of(child.id());
        let init = Stat::of(1);
        let up: f64 = std::fs::read_to_string("/proc/uptime").ok()
            .and_then(|t| t.split_whitespace().next().and_then(|f| f.parse().ok()))
            .unwrap_or(0.0);
        let ticks: f64 = Command::new("getconf").arg("CLK_TCK").output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(100.0);
        let _ = child.kill();
        let _ = child.wait();
        match (kid, init) {
            (Some(kid), Some(init)) => {
                assert!(init.start <= kid.start, "init {} began after the sleep {}", init.start, kid.start);
                let age = up - kid.start as f64 / ticks;
                assert!((0.0..20.0).contains(&age), "the sleep is {} s old by the kernel's uptime", age);
            },
            _ => panic!("no stat for the sleep or for init"),
        }
    }
}
