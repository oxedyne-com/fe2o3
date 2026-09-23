//! Delays and failures a test puts in the store's way, so that a slow or failing disk, or a slow
//! start, can be reproduced on a machine that has none of them.  Each is off until a test sets it,
//! and each is process-wide, so a test that sets one runs in a test binary of its own.

use std::{
    sync::atomic::{
        AtomicBool,
        AtomicU64,
        Ordering,
    },
    thread,
    time::Duration,
};

static BARRIER_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before each barrier
static PUBLISH_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before channels are handed over
static COLLECT_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before each garbage collection
static BARRIER_FAILS:    AtomicBool = AtomicBool::new(false); // every durability barrier fails
static BARRIERS_FAILED:  AtomicU64  = AtomicU64::new(0);      // failed by the switch above

/// Holds every durability barrier this long before it syncs, as an fsync queued behind the rest
/// of a busy disk's writes would be held.
pub fn set_barrier_delay(d: Duration) {
    BARRIER_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds the supervisor this long before it hands the database's channels to the handle that
/// started it, as a starved machine spawning a few dozen threads would.
pub fn set_publish_delay(d: Duration) {
    PUBLISH_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds every garbage collection this long before it reads the file, as a large file on a busy
/// disk would take, so that writes can be made to land while a file is being collected.
pub fn set_collect_delay(d: Duration) {
    COLLECT_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Makes every durability barrier fail without syncing, as a disk that has started returning
/// write-back errors would, and counts each barrier it fails.
pub fn set_barrier_failure(on: bool) {
    BARRIER_FAILS.store(on, Ordering::Relaxed);
}

/// How many durability barriers `set_barrier_failure` has failed so far.
pub fn barriers_failed() -> u64 {
    BARRIERS_FAILED.load(Ordering::Relaxed)
}

pub(crate) fn barrier_delay() {
    pause(&BARRIER_DELAY_MS);
}

pub(crate) fn publish_delay() {
    pause(&PUBLISH_DELAY_MS);
}

pub(crate) fn collect_delay() {
    pause(&COLLECT_DELAY_MS);
}

/// Is the disk to fail this sync?  Counted when it is.
pub(crate) fn sync_fails() -> bool {
    let fails = BARRIER_FAILS.load(Ordering::Relaxed);
    if fails {
        BARRIERS_FAILED.fetch_add(1, Ordering::Relaxed);
    }
    fails
}

fn millis(d: Duration) -> u64 {
    d.as_millis().min(u64::MAX as u128) as u64
}

fn pause(ms: &AtomicU64) {
    let ms = ms.load(Ordering::Relaxed);
    if ms > 0 {
        thread::sleep(Duration::from_millis(ms));
    }
}
