//! Steel hearing the operating system ask it to stop.
//!
//! The listening itself belongs to [`oxedyne_fe2o3_core::stop::on_stop_request`],
//! which answers `SIGINT` and `SIGTERM` on unix and the three console events on
//! Windows. What is here is the other half: the state a caught signal leaves
//! behind, and the two ways the rest of Steel reads it.
//!
//! # Why a flag is not enough
//!
//! A server asked to stop is almost always sitting in `accept`, waiting for a
//! connection that may never come. Setting a flag it will look at *next time
//! round* stops nothing, because there is no next time round until somebody
//! opens a socket. So the ask is published twice over: as a counter anything may
//! read at any moment ([`asked`]), and as a bell an asynchronous task can wait
//! on ([`wait`]) inside a `tokio::select!` alongside the accept itself.
//!
//! The order in [`wait`] is load-bearing. Interest is registered with the bell
//! *before* the counter is read, so an ask that lands between the two is heard
//! by the wait rather than slept through. `notify_waiters` stores no permit for
//! a waiter that is not yet there, which is why the counter is written first and
//! the bell rung second.
//!
//! # What a stop means for a process with no server in it
//!
//! Steel is a shell with a server in it, not the other way round: `run` may be
//! executing `wallet --list` or sitting at a prompt, with no accept loop
//! anywhere. Catching a signal there and merely noting it would be worse than
//! not catching it at all -- the process would ignore a `SIGTERM` it used to
//! obey. So [`listen`] ends such a process itself, once the log has been
//! flushed. [`serving`] is what tells the two cases apart.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    sync::{
        atomic::{
            AtomicBool,
            AtomicUsize,
            Ordering,
        },
        Arc,
        OnceLock,
    },
    time::{
        Duration,
        Instant,
    },
};

use tokio::sync::Notify;

static ASKS: AtomicUsize = AtomicUsize::new(0);

static SERVING: AtomicBool = AtomicBool::new(false);

/// The bell an accept loop waits on, rung once per ask.
///
/// Built on first use rather than declared, because `Notify::new` is not a
/// constant. `OnceLock` rather than a lazy cell so the crate takes no dependency
/// for one static.
fn bell() -> &'static Notify {
    static BELL: OnceLock<Notify> = OnceLock::new();
    BELL.get_or_init(Notify::new)
}

pub fn asked() -> bool {
    ASKS.load(Ordering::SeqCst) > 0
}

/// More than one means somebody asked again while the first ask was still being
/// obeyed, which is worth saying out loud even though Steel answers both the
/// same way; see [`listen`].
pub fn asks() -> usize {
    ASKS.load(Ordering::SeqCst)
}

/// Records an ask and wakes whatever is waiting on one, returning the number of
/// asks so far -- `1` for the first.
///
/// Safe to call from any thread, and from more than one at once: the counter
/// moves first, so a waiter that misses the bell still sees the count.
pub fn ask() -> usize {
    let n = ASKS.fetch_add(1, Ordering::SeqCst) + 1;
    bell().notify_waiters();
    n
}

/// Resolves as soon as this process has been asked to stop, and at once if it
/// already has.
///
/// Cancel-safe, which is what allows it to sit in a `tokio::select!` opposite an
/// `accept`: dropped part way through, it has consumed nothing, and the next
/// call registers afresh and re-reads the counter.
pub async fn wait() {
    loop {
        // Registered before the counter is read. The other order drops an ask
        // that lands in the gap between the two.
        let ringing = bell().notified();
        if asked() {
            return;
        }
        ringing.await;
        if asked() {
            return;
        }
    }
}

/// Declares whether something is running that will wind itself up when asked.
///
/// Set around the server's whole life -- from before its databases start opening
/// to after they have been closed -- rather than around the accept loop alone,
/// so that a signal arriving while a store is being opened is still answered by
/// the orderly path.
pub fn serving(now: bool) {
    SERVING.store(now, Ordering::SeqCst);
}

pub fn is_serving() -> bool {
    SERVING.load(Ordering::SeqCst)
}

/// Installs this process's stop-request listener.
///
/// One to a process, because a signal arrives at a process rather than at an
/// object; the underlying listener refuses a second. Called from
/// [`crate::app::tui::run_with_extension`], which is the real `main` of every
/// Steel application, stock or extended.
///
/// A listener already installed, or a thread that cannot be spawned, is not
/// fatal to a server -- it will simply be killed rather than asked when the
/// machine goes -- so the caller logs and carries on.
pub fn listen() -> Outcome<()> {
    res!(oxedyne_fe2o3_core::stop::on_stop_request(|| {
        let n = ask();
        if n > 1 {
            // Said, and not acted on. A program whose store is a cache can
            // read a second Ctrl-C as "now" and leave where it stands, because
            // the worst it costs is a rebuild. A Steel store is the site's own
            // data, so the firmer ask is refused rather than obeyed: what it
            // would interrupt is a database part way through closing, and the
            // wait it is impatient with is bounded already -- see
            // `srv::server::DRAIN_SECS`.
            warn!("Asked to stop {} times. The first ask is being obeyed and \
                the wind-up is bounded; a store is not abandoned part way \
                through closing.", n);
            return;
        }
        info!("Asked to stop.");
        if !is_serving() {
            info!("Nothing is serving, so there is nothing to wind up.");
            if let Err(e) = flush_log() {
                error!(e, "Flushing the log on the way out.");
            }
            // Nought, not 130. This process was not felled: it was asked, and
            // it did as it was asked. A service manager reads anything else as
            // a unit that failed.
            std::process::exit(0);
        }
    }));
    Ok(())
}

/// Waits for the logger to finish, so nothing written on the way out is lost.
///
/// The macro can return an error, and one that can has to sit in a function that
/// can, which the listener's closure is not.
fn flush_log() -> Outcome<()> {
    log_finish_wait!();
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WORK IN FLIGHT                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// One piece of work, counted for exactly as long as it lasts.
///
/// Held by each connection task in the accept loop so that a wind-up can tell
/// the difference between a server with nothing left to do and one part way
/// through a reply. The count falls in `Drop`, so a task that panics is not
/// counted for ever after.
#[derive(Debug)]
pub struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    /// Counts one more piece of work, until the returned value is dropped.
    pub fn begin(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Waits for work in flight to finish, for no longer than `bound`, and reports
/// how much of it was still going when the wait ended.
///
/// A bound rather than a wait, because some of what is counted here does not
/// end on its own: a WebSocket held open by a browser tab is in flight until the
/// tab closes, which may be tomorrow. Everything that does end -- a response
/// being written, a file being read off disk -- ends in milliseconds, so the
/// bound is generous for the case it is for and short enough that a service
/// manager's own patience is never the thing that runs out first.
pub async fn drain(count: &Arc<AtomicUsize>, bound: Duration) -> usize {
    let began = Instant::now();
    loop {
        let left = count.load(Ordering::SeqCst);
        if left == 0 || began.elapsed() >= bound {
            return left;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The count and the bell agree, and a wait that arrives late still returns.
    ///
    /// What can be checked in-process, and no more. A test cannot send itself a
    /// signal and go on being a test; that claim is `tests/stopping_signal.rs`,
    /// which starts a real server and sends it a real `SIGTERM`.
    #[test]
    fn test_an_ask_is_heard_late_00() -> Outcome<()> {
        req!(asked(), false, "Nothing has asked yet.");
        req!(ask(), 1, "The first ask is the first.");
        req!(asked(), true, "An ask was made and not heard.");
        req!(ask(), 2, "A second ask counts separately.");

        // Registered after the fact, and returns anyway.
        let rt = res!(tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build(), Init, System);
        let returned = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), wait()).await
        });
        req!(returned.is_ok(), true,
            "A wait begun after the ask never returned.");
        Ok(())
    }

    /// Work in flight is counted while it lasts and not afterwards.
    #[test]
    fn test_work_in_flight_is_counted_01() -> Outcome<()> {
        let count = Arc::new(AtomicUsize::new(0));
        {
            let _one = InFlight::begin(&count);
            let _two = InFlight::begin(&count);
            req!(count.load(Ordering::SeqCst), 2, "Two pieces of work.");
        }
        req!(count.load(Ordering::SeqCst), 0, "Both have finished.");
        Ok(())
    }
}
