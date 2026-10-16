//! What the outbound spool does with a message the far end refused.
//!
//! On karri two stale messages were retried every thirty seconds for twenty-five days: one answered
//! `550 5.4.1 Recipient address rejected`, the other addressed a domain with a null MX. A 5xx is
//! final (RFC 5321 §4.2.1), and the client already tagged it `Permanent`. The worker never read the
//! tag, and left every failure on disk for the next sweep. The transport here is a closure, so what
//! is under test is the spool's disposition of each outcome, with no network.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::outbound::{
    GIVE_UP_SECS,
    OutboundSpool,
    RetrySchedule,
    Sweep,
    retry_after,
};

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


/// A spool in a directory of its own, holding one message.
fn spool_with_one(tag: &str) -> Outcome<(OutboundSpool, PathBuf)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_spool_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&root);
    let spool = res!(OutboundSpool::new(root.clone()));
    res!(spool.enqueue("alice@sender.test", &[fmt!("bob@far.test")], b"Subject: hi\r\n\r\nhi\r\n"));
    Ok((spool, root))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

fn queued(spool: &OutboundSpool) -> Outcome<usize> {
    Ok(res!(spool.list()).len())
}

fn failed_files(root: &PathBuf) -> usize {
    fs::read_dir(root.join("failed")).map(|r| r.flatten().count()).unwrap_or(0)
}


/// A sweep in which the far end answers every message with a 451, counting how often it is asked.
async fn greylisted(
    spool:  &OutboundSpool,
    sched:  &mut RetrySchedule,
    at:     u64,
    calls:  &Arc<AtomicUsize>,
)
    -> Outcome<Sweep>
{
    let calls = calls.clone();
    spool.sweep(sched, at, move |_m| {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let r: Outcome<String> = Err(err!(
                "RCPT TO:<bob@far.test> deferred: 451 4.7.1 Greylisted";
                IO, Network, Wire));
            r
        }
    }).await
}


/// The 550 case. Before the fix the message stayed queued and the far end was asked again on every
/// sweep, which is what the journal showed.
#[tokio::test]
async fn a_permanent_refusal_ends_the_message_at_once() -> Outcome<()> {
    let (spool, root) = res!(spool_with_one("permanent"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut sched = RetrySchedule::default();
    let t = now();

    let c = calls.clone();
    let first = res!(spool.sweep(&mut sched, t, move |_m| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            let r: Outcome<String> = Err(err!(
                "RCPT TO:<bob@far.test> rejected: 550 5.4.1 Recipient address rejected";
                IO, Network, Wire, Permanent));
            r
        }
    }).await);
    req!(Sweep { delivered: 0, failed: 1, deferred: 0, waiting: 0 }, first,
        "a 550 did not end the message");
    req!(0, res!(queued(&spool)), "the refused message is still queued");
    req!(1, failed_files(&root), "the refused message was not kept in failed/");

    // Another sweep, a day on: nothing is left to try.
    let c = calls.clone();
    let later = res!(spool.sweep(&mut sched, t + 86_400, move |_m| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            let r: Outcome<String> = Ok(fmt!("unexpected"));
            r
        }
    }).await);
    req!(Sweep::default(), later, "a later sweep found something to do");
    req!(1, calls.load(Ordering::SeqCst), "the far end was asked again after a 550");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The retry that remains is a retry, not a loop: it waits, and the wait grows.
#[tokio::test]
async fn a_transient_failure_waits_before_the_next_try() -> Outcome<()> {
    let (spool, root) = res!(spool_with_one("transient"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut sched = RetrySchedule::default();
    let t = now();

    let first = res!(greylisted(&spool, &mut sched, t, &calls).await);
    req!(Sweep { delivered: 0, failed: 0, deferred: 1, waiting: 0 }, first,
        "a 451 was not deferred");

    // The worker sweeps every thirty seconds; the message must not be tried on each one.
    let too_soon = res!(greylisted(&spool, &mut sched, t + 30, &calls).await);
    req!(Sweep { delivered: 0, failed: 0, deferred: 0, waiting: 1 }, too_soon,
        "a deferred message was tried again thirty seconds later");
    req!(1, calls.load(Ordering::SeqCst), "the far end was asked again before the wait was up");

    let due = res!(greylisted(&spool, &mut sched, t + retry_after(1), &calls).await);
    req!(Sweep { delivered: 0, failed: 0, deferred: 1, waiting: 0 }, due,
        "a deferred message was not tried once its wait was up");
    req!(2, calls.load(Ordering::SeqCst), "the second attempt did not happen");
    req!(1, res!(queued(&spool)), "a transient failure dropped the message");
    req!(0, failed_files(&root), "a transient failure was set aside as final");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A greylist that never lifts, or a host that is gone for good, must not hold a message for ever.
#[tokio::test]
async fn a_message_that_never_gets_through_is_given_up_on() -> Outcome<()> {
    let (spool, root) = res!(spool_with_one("stale"));
    let mut sched = RetrySchedule::default();

    let out = res!(spool.sweep(&mut sched, now() + GIVE_UP_SECS + 60, |_m| async move {
        let r: Outcome<String> = Err(err!("Connection timed out."; IO, Network, Timeout));
        r
    }).await);
    req!(Sweep { delivered: 0, failed: 1, deferred: 0, waiting: 0 }, out,
        "a message older than the give-up time stayed queued");
    req!(0, res!(queued(&spool)), "the stale message is still queued");
    req!(1, failed_files(&root), "the stale message was not kept in failed/");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The ordinary case, so the fix cannot have broken it.
#[tokio::test]
async fn a_delivered_message_leaves_the_queue_and_is_not_set_aside() -> Outcome<()> {
    let (spool, root) = res!(spool_with_one("delivered"));
    let mut sched = RetrySchedule::default();

    let out = res!(spool.sweep(&mut sched, now(), |_m| async move {
        let r: Outcome<String> = Ok(fmt!("250 queued as 1234"));
        r
    }).await);
    req!(Sweep { delivered: 1, failed: 0, deferred: 0, waiting: 0 }, out,
        "an accepted message was not counted as delivered");
    req!(0, res!(queued(&spool)), "an accepted message is still queued");
    req!(0, failed_files(&root), "an accepted message was set aside as failed");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn the_wait_doubles_from_a_minute_to_a_ceiling() -> Outcome<()> {
    req!(60,        retry_after(1),         "first wait");
    req!(120,       retry_after(2),         "second wait");
    req!(240,       retry_after(3),         "third wait");
    req!(4 * 3_600, retry_after(30),        "the ceiling");
    req!(4 * 3_600, retry_after(u32::MAX),  "the ceiling without overflow");
    Ok(())
}
