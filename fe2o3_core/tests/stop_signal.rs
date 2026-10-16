//! A real signal, sent to a real process, and what `stop::on_stop_request`
//! makes of it.
//!
//! Everything else that touches `stop.rs` asks the code a question and reads
//! the code's own answer; `test_a_listener_installs_once_00` in the module
//! itself is exactly that. This file asks the operating system instead, because
//! the claim under test -- that `SIGINT`, `SIGTERM` and `SIGHUP` each reach the
//! closure as the right [`Stop`](oxedyne_fe2o3_core::stop::Stop) variant -- can
//! only be shown by a real signal crossing a real process boundary. A test
//! cannot send itself a signal and go on being that test: the closure would run
//! in whichever test binary happened to share the process. So the process under
//! signal is `stop_signal_child`, a binary of a few lines (`src/bin/`), and the
//! signal is sent by `kill`, the operating system's own tool, exactly as
//! `fe2o3_steel/tests/stopping_signal.rs` does for the same reason.
//!
//! Unix only: Windows has no `kill` and no signals, and the console events the
//! same listener answers there cannot be sent from a test.

#![cfg(unix)]

use oxedyne_fe2o3_core::prelude::*;

use std::{
    io::Read,
    process::{
        Child,
        Command,
        Stdio,
    },
    time::{
        Duration,
        Instant,
    },
};

const WAIT_SECS: u64 = 10;

/// Starts the child and waits until it has had a moment to install its
/// listener.
///
/// The child prints nothing until it hears a signal, so there is no readiness
/// line to wait on. A fixed pause is used instead: the listener installs
/// before `main` does anything else observable, and giving it two hundred
/// milliseconds costs far less than a pipe protocol just to carry one "ready".
fn spawn_child() -> Outcome<Child> {
    let exe = env!("CARGO_BIN_EXE_stop_signal_child");
    let child = res!(Command::new(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn(), IO, File);
    std::thread::sleep(Duration::from_millis(200));
    Ok(child)
}

/// Sends one signal to one process, by pid, using the operating system's own
/// tool.
///
/// `kill` rather than anything in this program: sending a signal from Rust
/// means `libc::kill`, an `extern "C"` call that would need an `unsafe` block
/// in a crate that forbids them -- and the external tool is also the better
/// oracle, since it is what a person at a terminal or a service manager would
/// use, rather than this crate agreeing with itself.
fn send(signal: &str, child: &Child) -> Outcome<()> {
    let out = res!(Command::new("kill")
        .arg(fmt!("-{}", signal))
        .arg(fmt!("{}", child.id()))
        .output(), IO, File);
    if !out.status.success() {
        return Err(err!(
            "kill -{} {} answered {:?}: {}",
            signal, child.id(), out.status,
            String::from_utf8_lossy(&out.stderr).trim();
        Test, System));
    }
    Ok(())
}

/// Waits for the child to exit, then reads back what it printed and how.
fn wait_and_read(mut child: Child) -> Outcome<(Option<i32>, String)> {
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if began.elapsed() >= Duration::from_secs(WAIT_SECS) {
                    return Err(err!(
                        "stop_signal_child was still running {} seconds \
                        after being signalled.", WAIT_SECS; Test, Timeout));
                }
                std::thread::sleep(Duration::from_millis(50));
            },
            Err(e) => return Err(err!(e,
                "Could not poll the child."; Test, IO)),
        }
    };
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    Ok((status.code(), out))
}

/// One signal, and the exact word the listener told the child about it.
///
/// This is the whole claim in one function, so `SIGINT`, `SIGTERM` and
/// `SIGHUP` are held to exactly the same standard rather than three slightly
/// different ones: the process exits by its own choice (a code, not a
/// felling), and it names the ask it heard, correctly.
fn a_signal_is_heard_and_named(signal: &str, expect: &str) -> Outcome<()> {
    let child = res!(spawn_child());
    res!(send(signal, &child));
    let (code, out) = res!(wait_and_read(child));
    req!(code, Some(0),
        "stop_signal_child did not exit cleanly after {}: it said {:?}. An \
        exit with no code at all means the signal felled it rather than being \
        caught, which is the fault this listener exists to fix.",
        signal, out);
    req!(out.trim(), expect,
        "stop_signal_child was sent {} but named a different ask: {:?}",
        signal, out);
    Ok(())
}

/// Ctrl-C, told apart from the other two.
#[test]
fn test_an_interrupt_is_heard_as_interrupt_00() -> Outcome<()> {
    a_signal_is_heard_and_named("INT", "Interrupt")
}

/// What a service manager and every reboot send.
#[test]
fn test_a_terminate_is_heard_as_terminate_01() -> Outcome<()> {
    a_signal_is_heard_and_named("TERM", "Terminate")
}

/// The controlling terminal going away, or `kill -HUP` -- the ask this unit
/// added. Before it, `on_stop_request` could not hear this signal at all, and
/// a program relying on it alone (as `ore edit`'s final mark does) would be
/// killed outright rather than asked.
#[test]
fn test_a_hangup_is_heard_as_hangup_02() -> Outcome<()> {
    a_signal_is_heard_and_named("HUP", "Hangup")
}
