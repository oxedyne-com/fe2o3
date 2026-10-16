//! A tiny process whose only job is to be sent a real signal and say which one.
//!
//! `tests/stop_signal.rs` is the only caller: it spawns this binary, sends it
//! `SIGINT`, `SIGTERM` or `SIGHUP` with the operating system's own `kill`, and
//! reads back which [`Stop`](oxedyne_fe2o3_core::stop::Stop) variant the
//! listener told it about. A signal cannot be sent to a running test binary and
//! still leave that binary able to keep testing, so the process under signal has
//! to be a child, and this is that child.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![forbid(unsafe_code)]

#[cfg(unix)]
fn main() -> oxedyne_fe2o3_core::prelude::Outcome<()> {
    use oxedyne_fe2o3_core::prelude::*;
    use oxedyne_fe2o3_core::stop::{
        on_stop_request,
        Stop,
    };
    use std::io::Write;

    res!(on_stop_request(|which: Stop| {
        let word = match which {
            Stop::Interrupt	=> "Interrupt",
            Stop::Terminate	=> "Terminate",
            Stop::Hangup	=> "Hangup",
        };
        // One line, flushed before exit: the parent reads this from a pipe and
        // a buffered line still sitting in libc's stdio at `exit` would be
        // lost.
        println!("{}", word);
        let _ = std::io::stdout().flush();
        std::process::exit(0);
    }));

    // Nothing to do until asked. The parent decides how long to wait before
    // giving up on it.
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Signals are a unix concept; nothing here would be exercised elsewhere.
#[cfg(not(unix))]
fn main() {}
