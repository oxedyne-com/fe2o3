//! Delays and failures a test puts in the store's way, so that a slow or failing disk, or a slow
//! start, can be reproduced on a machine that has none of them.  Each is off until a test sets it,
//! and each is process-wide, so a test that sets one runs in a test binary of its own.

use crate::file::core::FileType;

use oxedyne_fe2o3_jdat::Dat;

use std::{
    collections::BTreeMap,
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
static REPLAY_FILE_MS:   AtomicU64  = AtomicU64::new(0);      // before an igbot replays a file at start
static READ_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // between a reader's pin and its read
static TOMB_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // before a cache bot enters a chunk tombstone
static CHUNK_DELAY_MS:   AtomicU64  = AtomicU64::new(0);      // before a cache bot enters a chunk
static LIST_DELAY_MS:    AtomicU64  = AtomicU64::new(0);      // between a directory's listing and its opens
static RETIRE_DROPS:     AtomicBool = AtomicBool::new(false); // a durable head's retire is never sent
static RETIRES_DROPPED:  AtomicU64  = AtomicU64::new(0);      // dropped by the switch above
static OWED_DROPS:       AtomicBool = AtomicBool::new(false); // a held discard's barrier request is lost
static SUP_PANICS:       AtomicBool = AtomicBool::new(false); // the supervisor panics starting up
static BARRIER_FAILS:    AtomicBool = AtomicBool::new(false); // every durability barrier fails
static BARRIERS_FAILED:  AtomicU64  = AtomicU64::new(0);      // failed by the switch above
static BARRIER_FAILS_FOR: AtomicU64 = AtomicU64::new(u64::MAX); // barriers of this one live file fail
static SYNCER_STOPS:     AtomicBool = AtomicBool::new(false); // syncers stop after their next batch
static SYNCERS_STOPPED:  AtomicU64  = AtomicU64::new(0);      // stopped by the switch above
static PAIR_HAND_FAILS:  AtomicBool = AtomicBool::new(false); // new live pairs cannot be handed over
static COLLECT_FAILS:    AtomicBool = AtomicBool::new(false); // every collection fails before it commits
static COLLECTS_FAILED:  AtomicU64  = AtomicU64::new(0);      // failed by the switch above
static COLLECT_FAILS_FOR: AtomicU64 = AtomicU64::new(u64::MAX); // collections of this one file fail
static DIR_SYNC_FAILS:   AtomicBool = AtomicBool::new(false); // the directory sync after a collection's data rename fails
static COMPACT_FAILS:    AtomicBool = AtomicBool::new(false); // the zone bots cannot pass on a compaction order
static RECORD_BYTES_READ: AtomicU64 = AtomicU64::new(0);      // read from data files by the reader bots
static HEAD_INSERT_FAILS: AtomicBool = AtomicBool::new(false); // a cache bot cannot enter a store's head
static HEAD_HAND_FAILS:  AtomicBool = AtomicBool::new(false); // a writer cannot hand a written head to its syncer
static HEAD_HOLD:        AtomicBool = AtomicBool::new(false); // a store waits before it sends its head
static HEADS_HELD:       AtomicU64  = AtomicU64::new(0);      // stores that have reached the hold above
static SWEEP_HOLD:       AtomicBool = AtomicBool::new(false); // a sweep waits between its two scans
static SWEEPS_HELD:      AtomicU64  = AtomicU64::new(0);      // sweeps that have reached the hold above
static FAULT_RECORD_FAILS: AtomicBool = AtomicBool::new(false); // the fault record cannot be written
static FAULT_RECORD_AT:  AtomicU64  = AtomicU64::new(0);      // the nth fault record write fails, 0 none
static FAULT_RECORDS:    AtomicU64  = AtomicU64::new(0);      // fault record writes since the above was set
static REPLAY_SYNC_FAILS: AtomicBool = AtomicBool::new(false); // the flush before replay fails
static REPLAY_FLUSH_WAY: AtomicU64 = AtomicU64::new(0);      // 0 as the kernel allows, 1 syncfs, 2 per file
static REPLAY_FLUSHES:   AtomicU64  = AtomicU64::new(0);      // syncfs or file syncs made before replay
static DATA_TEAR_AT:     AtomicU64  = AtomicU64::new(u64::MAX); // the next data append fails after this many bytes
static INDEX_TEAR_AT:    AtomicU64  = AtomicU64::new(u64::MAX); // the next index append fails after this many bytes
static WRITES_TORN:      AtomicU64  = AtomicU64::new(0);      // appends failed by the two above
static CUT_BACK_FAILS:   AtomicBool = AtomicBool::new(false); // a torn append cannot be cut back off
static DURABILITY_MS:    AtomicU64  = AtomicU64::new(0);      // a store's durability timeout, when set
static KERNEL_RELEASE:   Mutex<Option<String>> = Mutex::new(None); // in place of the running kernel's
static DELETE_OLD_ORDER: AtomicBool = AtomicBool::new(false); // a whole-file delete unlinks its data file first
static DELETE_ABORT_AT:  AtomicU64  = AtomicU64::new(0);      // the process aborts after the first unlink of this delete, 0 never
static DELETES_BEGUN:    AtomicU64  = AtomicU64::new(0);      // whole-file deletes counted for the above
static HEAD_DROPS:       AtomicBool = AtomicBool::new(false); // a chunked store never sends its head
static SYNCER_CHUNK_US:  AtomicU64  = AtomicU64::new(0);      // before a syncer takes in a store's chunk
static BARRIERS:         Mutex<BTreeMap<(usize, usize), u64>> = Mutex::new(BTreeMap::new()); // completed, by (zone, writer)
static HALT:             Mutex<Option<(Halt, std::path::PathBuf)>> = Mutex::new(None); // where to wait for a kill
static HOLD_SKIP:        AtomicU64  = AtomicU64::new(0);      // records appended before the held one
static HOLD_MS:          AtomicU64  = AtomicU64::new(0);      // the hold, zero when disarmed
static HOLDS_TAKEN:      AtomicU64  = AtomicU64::new(0);      // records held so far

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

/// Holds an init garbage bot this long before it replays each file at start, as a large file on a
/// slow disk would, so that replay is still being queued for a while after the zones begin.
pub fn set_replay_file_delay(d: Duration) {
    REPLAY_FILE_MS.store(millis(d), Ordering::Relaxed);
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

/// Drops the retire of the chunk set a durable head displaced, as a crash between the two would.
/// The cache bot that would send it goes on serving; `retires_dropped` counts the drops.
pub fn set_retire_drop(on: bool) {
    RETIRE_DROPS.store(on, Ordering::SeqCst);
}

pub fn retires_dropped() -> u64 {
    RETIRES_DROPPED.load(Ordering::SeqCst)
}

/// Loses every barrier request a held discard makes, as a crash between the hold and the barrier
/// it owes would, so a test can hold the window open: only the policy, a seal, `sync_now` or a
/// close moves a mark.
pub fn set_owed_drop(on: bool) {
    OWED_DROPS.store(on, Ordering::SeqCst);
}

pub(crate) fn owed_dropped() -> bool {
    OWED_DROPS.load(Ordering::SeqCst)
}

/// Holds ONE record's append for `d`, as a writer stuck in a write to a disk that has stopped
/// completing would be held: the first `skip` records appended from now pass, the next is held,
/// and then the hook disarms itself.  The held record is appended once the hold ends, after its
/// caller has been told that whether it lands is not known (WSTALL, 2026-10-16), which is the case
/// a caller retrying an unknown outcome has to survive.  `Duration::ZERO` disarms it.
pub fn set_write_hold(skip: u64, d: Duration) {
    HOLD_MS.store(0, Ordering::Relaxed);
    HOLD_SKIP.store(skip, Ordering::Relaxed);
    HOLD_MS.store(millis(d), Ordering::Relaxed);
}

/// How many records `set_write_hold` has held so far, each counted when its hold begins.
pub fn write_holds_taken() -> u64 {
    HOLDS_TAKEN.load(Ordering::Relaxed)
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

/// Holds every chunked store, its chunks durable and in the cache, before it sends its head, until
/// the hold is lifted, so that a test can write between the two.  `heads_held` counts the stores
/// that have reached it.  It covers one put at a time: the hold blocks the server bot driving the
/// put, so a second put held at once that shares that server bot cannot advance to the hold.
pub fn set_head_hold(on: bool) {
    HEAD_HOLD.store(on, Ordering::SeqCst);
}

pub fn heads_held() -> u64 {
    HEADS_HELD.load(Ordering::SeqCst)
}

pub(crate) fn head_hold() {
    if HEAD_HOLD.load(Ordering::SeqCst) {
        HEADS_HELD.fetch_add(1, Ordering::SeqCst);
        while HEAD_HOLD.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Holds every orphan sweep between its scan of the live keys and its scan of the chunks, until
/// the hold is lifted, so that a test can make a whole store between the two.  `sweeps_held`
/// counts the sweeps that have reached it.
pub fn set_sweep_hold(on: bool) {
    SWEEP_HOLD.store(on, Ordering::SeqCst);
}

pub fn sweeps_held() -> u64 {
    SWEEPS_HELD.load(Ordering::SeqCst)
}

pub(crate) fn sweep_hold() {
    if SWEEP_HOLD.load(Ordering::SeqCst) {
        SWEEPS_HELD.fetch_add(1, Ordering::SeqCst);
        while SWEEP_HOLD.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Makes the durable record of a store's fault, and its refusal marker, fail to be written, as a
/// failing disk would.
pub fn set_fault_record_fails(on: bool) {
    FAULT_RECORD_FAILS.store(on, Ordering::SeqCst);
}

pub(crate) fn fault_record_fails() -> bool {
    FAULT_RECORD_FAILS.load(Ordering::SeqCst)
}

/// Makes only the `n`th write of a fault record from now fail, counting from one, and every other
/// write, the refusal marker's among them, succeed.  Zero turns it off.
pub fn set_fault_record_fails_at(n: u64) {
    FAULT_RECORDS.store(0, Ordering::SeqCst);
    FAULT_RECORD_AT.store(n, Ordering::SeqCst);
}

// Counts a write of the fault record, and says whether it is the one set to fail.
pub(crate) fn fault_record_write_fails() -> bool {
    let at = FAULT_RECORD_AT.load(Ordering::SeqCst);
    let n = FAULT_RECORDS.fetch_add(1, Ordering::SeqCst) + 1;
    at != 0 && n == at
}

/// Makes the flush an open gives the store's files before replaying them fail, whichever way it
/// is made, as a write-back error left by a crashed run would.
pub fn set_replay_sync_failure(on: bool) {
    REPLAY_SYNC_FAILS.store(on, Ordering::SeqCst);
}

// The answer of a flush before replay: the real call has been made, and the hook replaces its
// answer with `injected` when a test has asked, so the error path is the real one.
pub(crate) fn replay_sync<E>(real: Result<(), E>, injected: E) -> Result<(), E> {
    match REPLAY_SYNC_FAILS.load(Ordering::SeqCst) {
        true    => Err(injected),
        false   => real,
    }
}

/// Gives the open a kernel release of `rel` in place of the running kernel's, as
/// `/proc/sys/kernel/osrelease` would, and `Some("")` for one that cannot be read.
pub fn set_kernel_release(rel: Option<&str>) {
    let mut r = match KERNEL_RELEASE.lock() { Ok(r) => r, Err(p) => p.into_inner() };
    *r = rel.map(String::from);
}

pub(crate) fn kernel_release() -> Option<String> {
    match KERNEL_RELEASE.lock() {
        Ok(r)   => r.clone(),
        Err(p)  => p.into_inner().clone(),
    }
}

/// Where a process that a test means to kill stops, to wait for the kill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Halt {
    Synced,         // a barrier's syncs have returned, its mark is not yet published
    RecordSynced,   // the fault record's temporary file is synced, not yet renamed into place
    Recorded,       // the fault record is in place, the refusal marker not yet written
    Quarantined,    // a cut's data tail is in quarantine, the data file not yet cut
    DataCut,        // a cut's data file is cut, its index not yet
}

/// Stops the process at `at`, the first time it gets there: `flag` is written, naming the halt,
/// and the thread sleeps until the test kills the process.  `None` stops nowhere.
pub fn set_halt(at: Option<(Halt, std::path::PathBuf)>) {
    let mut h = match HALT.lock() { Ok(h) => h, Err(p) => p.into_inner() };
    *h = at;
}

pub(crate) fn halt(at: Halt) {
    let flag = {
        let mut h = match HALT.lock() { Ok(h) => h, Err(p) => p.into_inner() };
        match h.as_ref() {
            Some((want, _)) if *want == at => h.take().map(|(_, flag)| flag),
            _ => None,
        }
    };
    if let Some(flag) = flag {
        let _ = std::fs::write(&flag, format!("{:?}", at));
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
}

/// How the flush before replay is made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayFlush {
    Kernel,     // `syncfs` where the kernel reports write-back errors through it, else per file
    Syncfs,     // `syncfs` once per filesystem, wherever the platform has it
    PerFile,    // a sync of each data and index file
}

/// Forces the way the flush before replay is made, so that a test can exercise both.
pub fn set_replay_flush(way: ReplayFlush) {
    let n = match way {
        ReplayFlush::Kernel     => 0,
        ReplayFlush::Syncfs     => 1,
        ReplayFlush::PerFile    => 2,
    };
    REPLAY_FLUSH_WAY.store(n, Ordering::SeqCst);
}

pub(crate) fn replay_flush() -> ReplayFlush {
    match REPLAY_FLUSH_WAY.load(Ordering::SeqCst) {
        1 => ReplayFlush::Syncfs,
        2 => ReplayFlush::PerFile,
        _ => ReplayFlush::Kernel,
    }
}

/// The `syncfs` calls and file syncs made by every flush before replay since the process began.
pub fn replay_flushes() -> u64 {
    REPLAY_FLUSHES.load(Ordering::SeqCst)
}

pub(crate) fn count_replay_flush() {
    REPLAY_FLUSHES.fetch_add(1, Ordering::SeqCst);
}

/// Makes the durability barriers of live file `fnum` fail, as `set_barrier_failure` does every
/// barrier, so that a test can fail the writes to a file of their own while every other syncs.
pub fn set_barrier_failure_for(fnum: Option<u32>) {
    BARRIER_FAILS_FOR.store(fnum.map(u64::from).unwrap_or(u64::MAX), Ordering::Relaxed);
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

/// Shapes a collection's whole-file delete as a crash between its two unlinks finds it.  With
/// `old_order` the data file goes first, as every build before 2026-10-11 did, and a crash leaves
/// an index without its data file.  With `abort_at` n > 0 the process aborts straight after the
/// first unlink of the nth such delete, which no handler can answer: the store's files are then
/// as a process crash leaves them.
pub fn set_delete_cut(old_order: bool, abort_at: u64) {
    DELETE_OLD_ORDER.store(old_order, Ordering::Relaxed);
    DELETES_BEGUN.store(0, Ordering::Relaxed);
    DELETE_ABORT_AT.store(abort_at, Ordering::Relaxed);
}

/// Makes every store that sends a head after its chunks, the forcing path's included, fail once
/// its chunks are written and before it sends its head, as a crash between the two leaves it.
pub fn set_head_drop(on: bool) {
    HEAD_DROPS.store(on, Ordering::SeqCst);
}

pub(crate) fn head_drops() -> bool {
    HEAD_DROPS.load(Ordering::SeqCst)
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

/// Makes the writer's next append to a live file of type `typ` fail once `after` of its bytes are
/// in the file, as a disk that fills or errors part-way through a write would.  It fires once;
/// `None` disarms it.
pub fn set_write_tear(typ: FileType, after: Option<u64>) {
    let at = after.unwrap_or(u64::MAX);
    match typ {
        FileType::Data  => DATA_TEAR_AT.store(at, Ordering::SeqCst),
        FileType::Index => INDEX_TEAR_AT.store(at, Ordering::SeqCst),
    }
}

/// How many appends `set_write_tear` has failed so far.
pub fn writes_torn() -> u64 {
    WRITES_TORN.load(Ordering::SeqCst)
}

/// How many bytes the append now starting may write before it fails, if it is to fail.  An
/// append no longer than that is written whole and disarms the tear without counting it.
pub(crate) fn write_tears(typ: &FileType) -> Option<u64> {
    let at = match typ {
        FileType::Data  => DATA_TEAR_AT.swap(u64::MAX, Ordering::SeqCst),
        FileType::Index => INDEX_TEAR_AT.swap(u64::MAX, Ordering::SeqCst),
    };
    if at == u64::MAX {
        return None;
    }
    Some(at)
}

// Counts a tear once the append has actually failed short (QA APPENDSEEK F3).
pub(crate) fn write_torn() {
    WRITES_TORN.fetch_add(1, Ordering::SeqCst);
}

/// Makes cutting a torn append back off its live file fail, as a disk that refuses the truncate
/// as well would.
pub fn set_cut_back_failure(on: bool) {
    CUT_BACK_FAILS.store(on, Ordering::SeqCst);
}

pub(crate) fn cut_back_fails() -> bool {
    CUT_BACK_FAILS.load(Ordering::SeqCst)
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

/// Holds every syncer this long before it takes in each of a store's chunks, so that a syncer
/// lagging its writer through a long put can be reproduced.
pub fn set_syncer_chunk_delay(d: Duration) {
    SYNCER_CHUNK_US.store(d.as_micros().min(u64::MAX as u128) as u64, Ordering::Relaxed);
}

pub(crate) fn syncer_chunk_delay() {
    let us = SYNCER_CHUNK_US.load(Ordering::Relaxed);
    if us > 0 {
        thread::sleep(Duration::from_micros(us));
    }
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

pub(crate) fn write_hold() {
    let ms = HOLD_MS.load(Ordering::Relaxed);
    if ms == 0 {
        return;
    }
    // A record still to pass takes one off the count; only the first to find it at nought, and
    // the hold still armed, is held.
    if HOLD_SKIP.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1)).is_ok() {
        return;
    }
    if HOLD_MS.compare_exchange(ms, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        HOLDS_TAKEN.fetch_add(1, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(ms));
    }
}

pub(crate) fn read_delay() {
    pause(&READ_DELAY_MS);
}

pub(crate) fn insert_delay() {
    pause(&INSERT_DELAY_MS);
}

pub(crate) fn replay_file_delay() {
    pause(&REPLAY_FILE_MS);
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

pub(crate) fn retire_dropped() -> bool {
    let on = RETIRE_DROPS.load(Ordering::SeqCst);
    if on {
        RETIRES_DROPPED.fetch_add(1, Ordering::SeqCst);
    }
    on
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

// The order of a whole-file delete's two unlinks.
pub(crate) fn delete_order() -> [FileType; 2] {
    match DELETE_OLD_ORDER.load(Ordering::Relaxed) {
        true    => [FileType::Data, FileType::Index],
        false   => [FileType::Index, FileType::Data],
    }
}

// Called after the first unlink of a whole-file delete.
pub(crate) fn delete_first_unlinked() {
    let at = DELETE_ABORT_AT.load(Ordering::Relaxed);
    if at > 0 && DELETES_BEGUN.fetch_add(1, Ordering::SeqCst) + 1 == at {
        eprintln!("test::hooks: aborting after the first unlink of whole-file delete {}.", at);
        std::process::abort();
    }
}

pub(crate) fn collect_fails_for(fnum: u32) -> bool {
    COLLECT_FAILS_FOR.load(Ordering::Relaxed) == fnum as u64
}

pub(crate) fn dir_sync_fails() -> bool {
    DIR_SYNC_FAILS.load(Ordering::Relaxed)
}

/// The durability barriers completed since the process began, over every store and writer, so
/// that a test counts barriers rather than timing them.
pub fn barriers() -> u64 {
    barriers_by_writer().values().sum()
}

/// The barriers completed since the process began, by (zone, writer).
pub fn barriers_by_writer() -> BTreeMap<(usize, usize), u64> {
    match BARRIERS.lock() {
        Ok(m)   => m.clone(),
        Err(p)  => p.into_inner().clone(),
    }
}

pub(crate) fn barrier_made(zone: usize, bot: usize) {
    let mut m = match BARRIERS.lock() {
        Ok(m)   => m,
        Err(p)  => p.into_inner(),
    };
    *m.entry((zone, bot)).or_insert(0) += 1;
}

/// Is the disk to fail this sync?  Counted when it is.
pub(crate) fn sync_fails(fnum: u32) -> bool {
    let fails = BARRIER_FAILS.load(Ordering::Relaxed)
        || u64::from(fnum) == BARRIER_FAILS_FOR.load(Ordering::Relaxed);
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

/// The bytes of every record a reader bot has read from a data file for a key, so a test can
/// tell a lookup answered from the index from one that read values.  Counted always, since it
/// costs one relaxed addition per read.
pub fn record_bytes_read() -> u64 {
    RECORD_BYTES_READ.load(Ordering::Relaxed)
}

pub(crate) fn count_record_read(n: usize) {
    RECORD_BYTES_READ.fetch_add(n as u64, Ordering::Relaxed);
}
