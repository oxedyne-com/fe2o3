//! Delays and failures a test puts in the store's way, so that a slow or failing disk, or a slow
//! start, can be reproduced on a machine that has none of them.  Each is off until a test sets it,
//! and each is process-wide, so a test that sets one runs in a test binary of its own.

use oxedyne_fe2o3_jdat::Dat;

use std::{
    sync::{
        Mutex,
        atomic::{
            AtomicBool,
            AtomicU64,
            Ordering,
        },
    },
    thread,
    time::Duration,
};

// What a delete does with its tombstones, in order, for a test that checks a chunked value's
// chunks are retired only after its head tombstone is durable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Sent,       // a tombstone is dispatched
    Durable,    // a write's head is durable, and what it replaced may be retired
}

static TRACING:  AtomicBool    = AtomicBool::new(false);
static STEPS:    Mutex<Vec<Step>> = Mutex::new(Vec::new());

static BARRIER_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before each barrier
static PUBLISH_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before channels are handed over
static COLLECT_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before each garbage collection
static COMMIT_DELAY_MS:  AtomicU64  = AtomicU64::new(0);      // between a collection's two renames
static FORWARD_DELAY_MS: AtomicU64  = AtomicU64::new(0);      // before a supersession is forwarded
static SCHEDULE_DELAY_MS: AtomicU64 = AtomicU64::new(0);      // before a received supersession is handled
static SCHEDULES_HELD:   AtomicU64  = AtomicU64::new(0);      // handled after the hold above
static INSERT_DELAY_MS:  AtomicU64  = AtomicU64::new(0);      // before each cache bot insert
static READ_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // between a reader's pin and its read
static TOMB_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // before a cache bot enters a chunk tombstone
static CHUNK_DELAY_MS:   AtomicU64  = AtomicU64::new(0);      // before a cache bot enters a chunk
static LIST_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // between a directory's listing and its opens
static RETIRE_DELAY_MS:  AtomicU64  = AtomicU64::new(0);      // before a store retires what its head displaced
static SUP_PANICS:       AtomicBool = AtomicBool::new(false); // the supervisor panics starting up
static BARRIER_FAILS:    AtomicBool = AtomicBool::new(false); // every durability barrier fails
static BARRIERS_FAILED:  AtomicU64  = AtomicU64::new(0);      // failed by the switch above
static SYNCER_STOPS:     AtomicBool = AtomicBool::new(false); // syncers stop after their next batch
static SYNCERS_STOPPED:  AtomicU64  = AtomicU64::new(0);      // stopped by the switch above
static PAIR_HAND_FAILS:  AtomicBool = AtomicBool::new(false); // new live pairs cannot be handed over
static COLLECT_FAILS:    AtomicBool = AtomicBool::new(false); // every collection fails before it commits
static COLLECTS_FAILED:  AtomicU64  = AtomicU64::new(0);      // failed by the switch above
static COLLECT_FAILS_FOR: AtomicU64 = AtomicU64::new(u64::MAX); // collections of this one file fail
static DIR_SYNC_FAILS:   AtomicBool = AtomicBool::new(false); // the directory sync after a collection's data rename fails
static COMPACT_FAILS:    AtomicBool = AtomicBool::new(false); // the zone bots cannot pass on a compaction order
static HEAD_INSERT_FAILS: AtomicBool = AtomicBool::new(false); // a cache bot cannot enter a store's head
static HEAD_HAND_FAILS:  AtomicBool = AtomicBool::new(false); // a writer cannot hand a written head to its syncer
static DURABILITY_MS:    AtomicU64  = AtomicU64::new(0);      // a store's durability timeout, when set

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

/// Holds every garbage collection this long between putting its new data file in place and putting
/// its new index file in place, as a process stopped there would be, so that a test can kill it
/// with the data file of one generation beside the index file of the one before.
pub fn set_commit_delay(d: Duration) {
    COMMIT_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a file bot this long before it forwards a supersession to the file bot of the superseded
/// record's file, as a file bot behind a long queue would, so that the supersession can reach a
/// file after a collection of it has finished.
pub fn set_forward_delay(d: Duration) {
    FORWARD_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a file bot this long before it handles a supersession it has received from another file
/// bot, as one behind a long queue would, so that a barrier that stops at the sender can be told
/// from one that waits for the receiver.  A supersession a bot makes of its own file is not held.
pub fn set_schedule_delay(d: Duration) {
    SCHEDULE_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// How many supersessions `set_schedule_delay` has held so far, each counted when its hold ends,
/// just before the file bot registers it.
pub fn schedules_held() -> u64 {
    SCHEDULES_HELD.load(Ordering::Relaxed)
}

/// Holds a reader this long after its file bot has pinned the file for it and before it reads, as
/// a reader on a slow disk would be, so that a file can be kept pinned while a test looks at it.
pub fn set_read_delay(d: Duration) {
    READ_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds every cache bot insert this long, as a cache bot behind a long queue would, so that a
/// shutdown's time can run out with written records still queued at it.
pub fn set_insert_delay(d: Duration) {
    INSERT_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a cache bot this long before it enters a chunk of a value, as a chunk's writer behind a
/// slower disk than the bunch key's would, so that a bunch key made readable before its chunks can
/// be told from one made readable after them.  The bunch key and other records are not held.
pub fn set_chunk_insert_delay(d: Duration) {
    CHUNK_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a cache bot this long before it enters a chunk tombstone, the record that retires one
/// chunk of a deleted value, as a cache bot behind a queue of such records would, so that a delete
/// that does not wait for its chunks can be told from one that does.  Other records are not held.
pub fn set_chunk_tombstone_delay(d: Duration) {
    TOMB_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a zone bot this long between reading a directory's entries and opening the files they
/// name, as a directory on a busy disk would, so that a file can be taken away after it has been
/// listed.
pub fn set_list_delay(d: Duration) {
    LIST_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Holds a store this long once its head is durable and before it retires the chunk set its head
/// displaced, so that a read can be made while a delete or an overwrite is between the two.
pub fn set_retire_delay(d: Duration) {
    RETIRE_DELAY_MS.store(millis(d), Ordering::Relaxed);
}

/// Makes the supervisor panic once it has brought the bots up, before the database is ready, as a
/// fault in its start-up would.
pub fn set_supervisor_panic(on: bool) {
    SUP_PANICS.store(on, Ordering::Relaxed);
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

/// Makes each writer's syncer stop once it has released a batch of records begun after this call,
/// as one that had panicked would, and counts the syncers it stops.
pub fn set_syncer_stop(on: bool) {
    SYNCER_STOPS.store(on, Ordering::Relaxed);
}

/// How many syncers `set_syncer_stop` has stopped so far.
pub fn syncers_stopped() -> u64 {
    SYNCERS_STOPPED.load(Ordering::Relaxed)
}

/// Makes handing a writer's new live pair to its syncer fail, as running out of file descriptors
/// to duplicate the pair's with would.
pub fn set_pair_hand_failure(on: bool) {
    PAIR_HAND_FAILS.store(on, Ordering::Relaxed);
}

/// Makes every garbage collection fail once it has written its temporary files, before it has
/// replaced anything, as a disk that filled during the transcription would, and counts each
/// collection it fails.
pub fn set_collect_failure(on: bool) {
    COLLECT_FAILS.store(on, Ordering::Relaxed);
}

/// As `set_collect_failure`, for the collections of one file only, so that its old records stay
/// on disk while other files are collected.
pub fn set_collect_fails_for(fnum: Option<u32>) {
    COLLECT_FAILS_FOR.store(fnum.map_or(u64::MAX, |n| n as u64), Ordering::Relaxed);
}

/// Makes the directory sync after a collection's data rename fail, so the records the collection
/// left out are not known to be gone.
pub fn set_dir_sync_failure(on: bool) {
    DIR_SYNC_FAILS.store(on, Ordering::Relaxed);
}

/// Makes every zone bot fail to pass a compaction order on to its file bots, as a zone bot whose
/// file pool has gone would, so that the call that gave the order can be seen to stop at the
/// zone bot's error and not wait out its deadline.
pub fn set_compact_failure(on: bool) {
    COMPACT_FAILS.store(on, Ordering::Relaxed);
}

/// How many collections `set_collect_failure` has failed so far.
pub fn collections_failed() -> u64 {
    COLLECTS_FAILED.load(Ordering::Relaxed)
}

/// Makes every cache bot fail to enter the head of a store, after its record is written.
pub fn set_head_insert_failure(on: bool) {
    HEAD_INSERT_FAILS.store(on, Ordering::Relaxed);
}

pub(crate) fn head_insert_fails() -> bool {
    HEAD_INSERT_FAILS.load(Ordering::Relaxed)
}

/// Makes every writer fail to hand a store's head to its syncer once the record is in the files,
/// as a syncer that stopped between the writer's check and the hand-off would.
pub fn set_head_hand_failure(on: bool) {
    HEAD_HAND_FAILS.store(on, Ordering::Relaxed);
}

pub(crate) fn head_hand_fails() -> bool {
    HEAD_HAND_FAILS.load(Ordering::Relaxed)
}

/// Shortens how long a store waits on the disk for each of its steps.  `None` restores the
/// default.
pub fn set_durability_timeout(d: Option<Duration>) {
    DURABILITY_MS.store(d.map(millis).unwrap_or(0), Ordering::Relaxed);
}

pub(crate) fn durability_timeout() -> Option<Duration> {
    match DURABILITY_MS.load(Ordering::Relaxed) {
        0   => None,
        ms  => Some(Duration::from_millis(ms)),
    }
}

pub(crate) fn compact_fails() -> bool {
    COMPACT_FAILS.load(Ordering::Relaxed)
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

pub(crate) fn commit_delay() {
    pause(&COMMIT_DELAY_MS);
}

pub(crate) fn forward_delay() {
    pause(&FORWARD_DELAY_MS);
}

pub(crate) fn schedule_delay() {
    if SCHEDULE_DELAY_MS.load(Ordering::Relaxed) > 0 {
        pause(&SCHEDULE_DELAY_MS);
        SCHEDULES_HELD.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn read_delay() {
    pause(&READ_DELAY_MS);
}

pub(crate) fn insert_delay() {
    pause(&INSERT_DELAY_MS);
}

/// A chunk tombstone is written under the chunk's part key as a whole record, so its key is a
/// five-number tuple and it carries no chunk index of its own.
pub(crate) fn chunk_tombstone_delay(key: &[u8], cind: Option<usize>) {
    if cind.is_none() && key.first() == Some(&Dat::TUP5_U64_CODE) {
        pause(&TOMB_DELAY_MS);
    }
}

/// A chunk of a value carries its index in the stored key, counted from one; the bunch key is
/// index zero.
pub(crate) fn chunk_insert_delay(cind: Option<usize>) {
    if matches!(cind, Some(i) if i > 0) {
        pause(&CHUNK_DELAY_MS);
    }
}

pub(crate) fn list_delay() {
    pause(&LIST_DELAY_MS);
}

pub(crate) fn retire_delay() {
    pause(&RETIRE_DELAY_MS);
}

pub(crate) fn supervisor_panic() {
    if SUP_PANICS.load(Ordering::Relaxed) {
        panic!("The supervisor panicked starting up (test::hooks::set_supervisor_panic).");
    }
}

/// Is this syncer to stop now?  Counted when it is.
pub(crate) fn syncer_stop_armed() -> bool {
    SYNCER_STOPS.load(Ordering::Relaxed)
}

pub(crate) fn syncer_stops() -> bool {
    let stops = SYNCER_STOPS.load(Ordering::Relaxed);
    if stops {
        SYNCERS_STOPPED.fetch_add(1, Ordering::Relaxed);
    }
    stops
}

pub(crate) fn pair_hand_fails() -> bool {
    PAIR_HAND_FAILS.load(Ordering::Relaxed)
}

/// Is this collection to fail now?  Counted when it is.
pub(crate) fn collect_fails(fnum: u32) -> bool {
    let fails = COLLECT_FAILS.load(Ordering::Relaxed)
        || COLLECT_FAILS_FOR.load(Ordering::Relaxed) == fnum as u64;
    if fails {
        COLLECTS_FAILED.fetch_add(1, Ordering::Relaxed);
    }
    fails
}

pub(crate) fn collect_fails_for(fnum: u32) -> bool {
    COLLECT_FAILS_FOR.load(Ordering::Relaxed) == fnum as u64
}

pub(crate) fn dir_sync_fails() -> bool {
    DIR_SYNC_FAILS.load(Ordering::Relaxed)
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

/// Starts or stops recording the steps of a delete's tombstones, and clears the record.
pub fn set_trace(on: bool) {
    TRACING.store(on, Ordering::Relaxed);
    if let Ok(mut steps) = STEPS.lock() {
        steps.clear();
    }
}

/// The steps recorded since `set_trace(true)`.
pub fn trace_steps() -> Vec<Step> {
    match STEPS.lock() {
        Ok(steps)   => steps.clone(),
        Err(_)      => Vec::new(),
    }
}

pub(crate) fn trace(step: Step) {
    if TRACING.load(Ordering::Relaxed) {
        if let Ok(mut steps) = STEPS.lock() {
            steps.push(step);
        }
    }
}

static FIXED_STAMP: Mutex<Option<Duration>> = Mutex::new(None); // what the clock reads for a write
static STALE_STAMP: Mutex<Option<Duration>> = Mutex::new(None); // a write's stamp, unordered

/// Makes the clock read this time since the epoch for every write, as it reads for two puts in
/// one clock tick, or after a clock step back.  The process still orders its stamps.  `None`
/// restores the clock.
pub fn set_fixed_stamp(t: Option<Duration>) {
    if let Ok(mut s) = FIXED_STAMP.lock() {
        *s = t;
    }
}

pub(crate) fn fixed_stamp() -> Option<Duration> {
    match FIXED_STAMP.lock() {
        Ok(s)   => *s,
        Err(_)  => None,
    }
}

/// Stamps every write with this time since the epoch, past the process's ordering of its stamps,
/// as a write stamped before another and landing after it is stamped.  `None` restores the clock.
pub fn set_stale_stamp(t: Option<Duration>) {
    if let Ok(mut s) = STALE_STAMP.lock() {
        *s = t;
    }
}

pub(crate) fn stale_stamp() -> Option<Duration> {
    match STALE_STAMP.lock() {
        Ok(s)   => *s,
        Err(_)  => None,
    }
}
