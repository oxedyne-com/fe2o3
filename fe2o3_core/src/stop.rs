//! Hearing the operating system ask a program to stop.
//!
//! A long-running program is not asked to stop in words. It is sent a signal:
//! `SIGINT` when somebody presses Ctrl-C, `SIGTERM` when a service manager or a
//! reboot says to go. A program that hears neither is killed where it stands,
//! and anything holding a store open is killed in the middle of a write.
//!
//! [`on_stop_request`] is the whole of the module: give it a closure, and the
//! closure is called every time this process is asked to stop.
//!
//! # Why this needs no `unsafe`
//!
//! Three of the four routes from a signal to a program need an `unsafe` block.
//! The C library's `sigaction` and Windows's `SetConsoleCtrlHandler` are
//! `extern "C"`; `signal_hook_registry::register` is an `unsafe fn`, for the
//! good reason that whatever it registers runs in signal context, where almost
//! nothing is allowed to happen.
//!
//! The fourth route is tokio's `signal` module, and its whole public surface is
//! safe. The registration and the signal-context work sit inside tokio, which
//! owns and audits them; what reaches the caller is an ordinary asynchronous
//! stream read on an ordinary thread. So this crate keeps
//! `#![forbid(unsafe_code)]`, and so does everything downstream of it.
//!
//! That matters for more than a lint. Because `on_ask` is called from a plain
//! thread rather than from signal context, it is under none of the restrictions
//! a signal handler is under: it may allocate, lock, log and take as long as it
//! likes.
//!
//! # Not on `wasm32`
//!
//! There are no signals in a browser, and tokio's `signal` feature pulls in
//! `mio`, `libc` and `signal-hook-registry`, none of which belong in a wasm
//! build. The module is therefore compiled only for the other targets, and the
//! dependency that carries it is declared for those targets alone.

use crate::prelude::*;

use std::{
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
    thread,
};

/// The name the listening thread carries, so that it can be told apart in a
/// debugger or a stack dump.
const THREAD_NAME: &str = "fe2o3-stop";

/// Whether a listener has already been installed in this process.
static LISTENING: AtomicBool = AtomicBool::new(false);

/// Calls `on_ask` every time the operating system asks this process to stop.
///
/// What counts as an ask depends on the platform:
///
/// | Platform	| Heard                                                  |
/// |-----------|--------------------------------------------------------|
/// | Unix	| `SIGINT` and `SIGTERM`                                 |
/// | Windows	| Ctrl-C, the console window closing, the machine going  |
/// | Other	| Ctrl-C                                                 |
///
/// The call returns as soon as the listener is installed, and the listening is
/// done on a thread of its own. `on_ask` is called **once per ask**, not once
/// and then never again: a program that reads a second ask as a firmer one --
/// the first polite, the second immediate -- gets to see both.
///
/// One listener to a process, because a signal arrives at a process rather than
/// at an object. A second call is refused rather than quietly stacking a second
/// thread behind the first; a caller with two things to do should do both in the
/// one closure.
///
/// # Errors
///
/// Returns an error if a listener is already installed, or if the thread cannot
/// be spawned. A failure *after* the listener is running -- a runtime that will
/// not build, a signal that cannot be registered -- cannot be returned to
/// anybody, and is logged at error level instead.
///
/// # Example
///
/// ```no_run
/// use oxedyne_fe2o3_core::prelude::*;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// static ASKS: AtomicUsize = AtomicUsize::new(0);
///
/// fn main() -> Outcome<()> {
///     res!(oxedyne_fe2o3_core::stop::on_stop_request(|| {
///         // The first ask is polite; the second means now.
///         if ASKS.fetch_add(1, Ordering::Relaxed) > 0 {
///             std::process::exit(130);
///         }
///     }));
///     while ASKS.load(Ordering::Relaxed) == 0 {
///         std::thread::sleep(std::time::Duration::from_millis(100));
///     }
///     Ok(())
/// }
/// ```
pub fn on_stop_request<F>(on_ask: F) -> Outcome<()>
where
    F: Fn() + Send + 'static,
{
    if LISTENING.swap(true, Ordering::SeqCst) {
        return Err(err!(
            "A stop request listener is already installed in this process. A \
            signal arrives at a process rather than at an object, so there is \
            one listener and it should be given a closure that does everything \
            which has to happen.";
        Conflict, Exists));
    }
    let _listener = res!(thread::Builder::new()
        .name(THREAD_NAME.to_string())
        .spawn(move || match listen(&on_ask) {
            Ok(()) => (),
            // In statement position on purpose: `error!` expands to a call and
            // a semicolon, which a future compiler will refuse to read as an
            // expression.
            Err(e) => {
                error!(e,
                    "The stop request listener could not start, so a Ctrl-C or \
                    a service manager's stop will kill this process where it \
                    stands rather than ask it to come home.");
            },
        }), Thread, Init);
    Ok(())
}

/// The listening thread's whole life: a runtime of its own, and then waiting.
///
/// A current-thread runtime, because this thread has exactly one thing to wait
/// for and a worker pool for it would be an absurdity. It is built here rather
/// than asked of the caller so that a program with no asynchronous code
/// anywhere, which is most of them, can still be asked to stop.
fn listen<F>(on_ask: &F) -> Outcome<()>
where
    F: Fn(),
{
    let rt = res!(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build(), Init, System);
    rt.block_on(wait(on_ask))
}

/// Waits on `SIGINT` and `SIGTERM`, answering each until the process ends.
///
/// `SIGINT` is Ctrl-C at a terminal and `SIGTERM` is what a service manager, and
/// every reboot, sends first. Both are asks rather than orders: the kill that
/// cannot be caught is `SIGKILL`, and by then it is too late to do anything at
/// all.
#[cfg(unix)]
async fn wait<F>(on_ask: &F) -> Outcome<()>
where
    F: Fn(),
{
    use tokio::signal::unix::{
        signal,
        SignalKind,
    };

    let mut int		= res!(signal(SignalKind::interrupt()), Init, System);
    let mut term	= res!(signal(SignalKind::terminate()), Init, System);
    loop {
        // Both arms are cancel-safe, which is what makes this loop legitimate:
        // the arm not taken is dropped part way through its wait and loses
        // nothing by it.
        let heard = tokio::select! {
            got = int.recv()	=> got,
            got = term.recv()	=> got,
        };
        match heard {
            Some(()) => on_ask(),
            // Neither stream ends while the process lives. If one somehow did,
            // there would be nothing left to hear and going round again would
            // only spin.
            None => return Ok(()),
        }
    }
}

/// Waits on the three console events Windows sends, answering each.
///
/// Ctrl-C, the console window being closed, and the machine shutting down.
/// The last two are on a clock: Windows gives the process a few seconds after
/// them and then ends it regardless, so whatever `on_ask` sets in motion should
/// be brief.
#[cfg(windows)]
async fn wait<F>(on_ask: &F) -> Outcome<()>
where
    F: Fn(),
{
    use tokio::signal::windows::{
        ctrl_c,
        ctrl_close,
        ctrl_shutdown,
    };

    let mut int		= res!(ctrl_c(), Init, System);
    let mut closed	= res!(ctrl_close(), Init, System);
    let mut down	= res!(ctrl_shutdown(), Init, System);
    loop {
        let heard = tokio::select! {
            got = int.recv()	=> got,
            got = closed.recv()	=> got,
            got = down.recv()	=> got,
        };
        match heard {
            Some(()) => on_ask(),
            None => return Ok(()),
        }
    }
}

/// Waits on Ctrl-C, which is all any other platform promises.
#[cfg(not(any(unix, windows)))]
async fn wait<F>(on_ask: &F) -> Outcome<()>
where
    F: Fn(),
{
    loop {
        res!(tokio::signal::ctrl_c().await, IO, System);
        on_ask();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listener installs, and a second one is refused rather than stacked.
    ///
    /// What this can check in-process, and no more. A test cannot send itself a
    /// signal and go on being a test: the closure would run in whichever test
    /// binary happened to be sharing the process. The claim that a real signal
    /// reaches a real program is a process-level test, and belongs to whoever
    /// has a program to stop.
    #[test]
    fn test_a_listener_installs_once_00() -> Outcome<()> {
        res!(on_stop_request(|| {}));
        let again = on_stop_request(|| {});
        req!(again.is_err(), true,
            "A second listener was installed. Two threads would then answer \
            the same signal.");
        Ok(())
    }
}
