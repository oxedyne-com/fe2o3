//! Signals to the caller's own process group, and to one process.
//!
//! A program that runs a child it must end when asked to stop, an editor under `ore edit`, has two
//! ways to reach it. The child shares the caller's process group where the caller was started as
//! a job by an interactive shell, and a signal to the group then reaches the child and whatever it
//! started in turn. Where the caller was not, as under a script or a test harness, the group is its
//! parent's, and signalling it would reach that parent's other processes; [`to_own_group`] declines
//! to, and [`to_tree`] reaches the child and everything it started, which is what a child run through
//! `sh -c` needs, since the shell is not the program.
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

/// Sends `sig` to `pid` and to every process it started, however deep, and answers how many it
/// reached.
///
/// The tree is read from the parent of each process in `/proc`, and a process that ends between
/// being read and being signalled is passed over.
pub fn to_tree(pid: u32, sig: Signal) -> Outcome<usize> {
    let dir = match std::fs::read_dir("/proc") {
        Ok(d)   => d,
        Err(e)  => return Err(err!(e,
            "Cannot read /proc to find the processes process {} started.", pid;
            IO, File, Read)),
    };
    let mut parents: Vec<(u32, u32)> = Vec::new();
    for entry in dir.flatten() {
        let me: u32 = match entry.file_name().to_str().and_then(|n| n.parse().ok()) {
            Some(n) => n,
            None    => continue,
        };
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(s)   => s,
            Err(_)  => continue,
        };
        // The command name is in parentheses and may hold spaces and brackets, so the fields are
        // counted from the last one.
        let after = match stat.rfind(')') {
            Some(i) => &stat[i + 1..],
            None    => continue,
        };
        let mut fields = after.split_whitespace();
        let _state = fields.next();
        match fields.next().and_then(|p| p.parse().ok()) {
            Some(parent)    => parents.push((me, parent)),
            None            => continue,
        }
    }
    let mut found = vec![pid];
    let mut at = 0;
    while at < found.len() {
        let of = found[at];
        at += 1;
        for (me, parent) in &parents {
            if *parent == of && !found.contains(me) {
                found.push(*me);
            }
        }
    }
    let mut sent = 0;
    for one in &found {
        if to_process(*one, sig).is_ok() {
            sent += 1;
        }
    }
    Ok(sent)
}
