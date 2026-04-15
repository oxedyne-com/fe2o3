//! In-memory traffic recorder.
//!
//! Holds a bounded ring buffer of recent HTTP requests and a small
//! set of per-vhost / per-status counters. Populated from the request
//! pipeline in `srv/https.rs`; read by [`handler`](super::handler)
//! when the operator opens the traffic view.
//!
//! The buffer shape is chosen with a second consumer in mind: once
//! `fe2o3_net::guard::AddressGuard` lands (after extraction from
//! `fe2o3_shield`), it will feed from the same counters to drive
//! rate-limiting and blacklist transitions. Counters are therefore
//! updated on the hot path under a short write lock; snapshots for
//! the dashboard copy out once under a read lock.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        HashMap,
        VecDeque,
    },
    sync::{
        Arc,
        RwLock,
        atomic::{
            AtomicU64,
            Ordering,
        },
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

/// Default ring buffer capacity -- 10k entries, roughly 1-2 MiB of
/// live memory at typical sizes. Tunable via `AdminConfig` once the
/// admin config block lands.
pub const DEFAULT_RING_CAPACITY: usize = 10_000;

/// Upper bound on the number of distinct paths a single vhost may
/// keep individual counters for. Beyond this, further paths fold
/// into a single `_other` bucket. Bounds worst-case memory when a
/// caller probes unique URLs.
pub const MAX_PATHS_PER_VHOST: usize = 256;

/// Bucket name used when a vhost's distinct-path count exceeds
/// `MAX_PATHS_PER_VHOST`.
pub const OTHER_PATH_BUCKET: &str = "_other";

/// Default number of periodic samples the sampler keeps. At the
/// default sample interval this works out to one hour of history.
pub const DEFAULT_HISTORY_CAPACITY: usize = 720;

/// Default interval between counter samples. Five seconds is a
/// reasonable trade-off between chart smoothness and CPU cost on
/// a quiet host.
pub const DEFAULT_SAMPLE_INTERVAL_SECS: u64 = 5;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REQUEST RECORD                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Snapshot of a single request as it leaves the handler pipeline.
///
/// All fields owned so the record can outlive the request without
/// keeping borrows alive.
#[derive(Clone, Debug)]
pub struct RequestRecord {
    /// Unix nanoseconds at which the request completed.
    pub when_ns:        u64,
    /// Vhost the request was routed to. Lowercased hostname,
    /// matches the key used in `ServerContext::vhost_dbs`.
    pub vhost:          String,
    /// HTTP method ("GET", "POST", ...).
    pub method:         String,
    /// Request path, including query string.
    pub path:           String,
    /// Final response status code.
    pub status:         u16,
    /// Remote peer's IP address and port as a string.
    pub peer:           String,
    /// Response body length in bytes, if known.
    pub bytes:          Option<u64>,
    /// Wall-clock duration from accept to final write, in
    /// microseconds.
    pub duration_us:    u64,
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COUNTERS                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Per-vhost counters, aggregated since the recorder was created
/// (i.e. since Steel started). Counts never decrement, so these
/// are suitable for rate-of-change computation by the dashboard
/// (sample-then-subtract across two fetches).
#[derive(Clone, Debug, Default)]
pub struct VhostCounters {
    /// Total requests that hit this vhost.
    pub total:      u64,
    /// Per-status breakdown: `{200 => 1234, 404 => 12, ...}`.
    pub by_status:  HashMap<u16, u64>,
    /// Per-path breakdown, capped at [`MAX_PATHS_PER_VHOST`]
    /// entries. Overflow folds into [`OTHER_PATH_BUCKET`].
    pub by_path:    HashMap<String, u64>,
}

/// Snapshot of counter state at a moment in time. Returned by
/// [`TrafficRecorder::counters_snapshot`].
#[derive(Clone, Debug, Default)]
pub struct CountersSnapshot {
    /// Total requests across every vhost.
    pub total:      u64,
    /// Overall per-status breakdown.
    pub by_status:  HashMap<u16, u64>,
    /// Per-vhost counters.
    pub by_vhost:   HashMap<String, VhostCounters>,
}

/// One point in the bounded traffic history. Captures the
/// monotonic totals at a particular unix second so the dashboard
/// can compute deltas between adjacent samples and draw a
/// requests-per-interval chart.
#[derive(Clone, Debug)]
pub struct TrafficSample {
    /// Unix seconds at which the sample was taken.
    pub when_secs:  u64,
    /// Cumulative total requests across every vhost.
    pub total:      u64,
    /// Cumulative per-status breakdown at this instant.
    pub by_status:  HashMap<u16, u64>,
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TRAFFIC RECORDER                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Thread-safe ring buffer plus counters.
///
/// Cheaply cloneable via `Arc`. A typical deployment constructs
/// one and stores it both in the request pipeline (via
/// `ServerContext`) and in the admin state (for the dashboard).
#[derive(Debug)]
pub struct TrafficRecorder {
    /// Fixed ring buffer capacity. Older entries are dropped when
    /// a new entry arrives at capacity.
    capacity:           usize,
    /// Recent request records, newest-last. Length never exceeds
    /// `capacity`.
    ring:               RwLock<VecDeque<RequestRecord>>,
    /// Rolling counter state. Distinct lock from the ring so that
    /// dashboard reads of counters and records do not contend on
    /// the same lock.
    counters:           RwLock<CountersSnapshot>,
    /// Monotonic total, also visible via `counters`, but kept as
    /// an atomic so lock-free reads stay cheap.
    total:              AtomicU64,
    /// Bounded ring of periodic counter samples, newest-last.
    /// Populated by a background sampling task; read by the
    /// dashboard traffic view to draw time-series charts.
    history:            RwLock<VecDeque<TrafficSample>>,
    /// Maximum number of samples kept in `history`.
    history_capacity:   usize,
}

impl TrafficRecorder {
    /// Construct a fresh recorder with the given ring capacity.
    /// A zero capacity is treated as [`DEFAULT_RING_CAPACITY`].
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 {
            DEFAULT_RING_CAPACITY
        } else {
            capacity
        };
        Self {
            capacity:           cap,
            ring:               RwLock::new(VecDeque::with_capacity(cap)),
            counters:           RwLock::new(CountersSnapshot::default()),
            total:              AtomicU64::new(0),
            history:            RwLock::new(
                VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            ),
            history_capacity:   DEFAULT_HISTORY_CAPACITY,
        }
    }

    /// Shortcut that wraps a fresh recorder in an `Arc`. Most
    /// call sites share the recorder between the request pipeline
    /// and the dashboard so the `Arc` saves a wrap later.
    pub fn new_shared(capacity: usize) -> Arc<Self> {
        Arc::new(Self::new(capacity))
    }

    /// Capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Maximum number of periodic samples kept in history.
    pub fn history_capacity(&self) -> usize {
        self.history_capacity
    }

    /// Monotonic total request count since the recorder was
    /// created. Lock-free.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Record a single completed request.
    ///
    /// Takes two short write locks -- one on the ring and one on
    /// the counters. Any lock poisoning surfaces as an error; the
    /// hot-path call site logs and continues rather than
    /// aborting the request.
    pub fn record(&self, rec: RequestRecord) -> Outcome<()> {
        // Counters first so a poisoned ring does not leave us
        // with a stale count.
        {
            let mut ctr = lock_write!(self.counters);
            ctr.total = ctr.total.saturating_add(1);
            *ctr.by_status.entry(rec.status).or_insert(0) += 1;
            let vh = ctr.by_vhost
                .entry(rec.vhost.clone())
                .or_insert_with(VhostCounters::default);
            vh.total = vh.total.saturating_add(1);
            *vh.by_status.entry(rec.status).or_insert(0) += 1;
            if vh.by_path.contains_key(&rec.path)
                || vh.by_path.len() < MAX_PATHS_PER_VHOST
            {
                *vh.by_path.entry(rec.path.clone()).or_insert(0) += 1;
            } else {
                *vh.by_path.entry(OTHER_PATH_BUCKET.to_string())
                    .or_insert(0) += 1;
            }
        }
        self.total.fetch_add(1, Ordering::Relaxed);
        {
            let mut ring = lock_write!(self.ring);
            if ring.len() == self.capacity {
                ring.pop_front();
            }
            ring.push_back(rec);
        }
        Ok(())
    }

    /// Return up to `limit` most recent records, newest first.
    /// `limit` of zero returns everything currently in the ring.
    pub fn recent(&self, limit: usize) -> Outcome<Vec<RequestRecord>> {
        let ring = lock_read!(self.ring);
        let take = if limit == 0 { ring.len() } else { limit.min(ring.len()) };
        let mut out = Vec::with_capacity(take);
        // Iterate newest-first by walking back from the end.
        for rec in ring.iter().rev().take(take) {
            out.push(rec.clone());
        }
        Ok(out)
    }

    /// Clone the current counter state for the dashboard to read.
    pub fn counters_snapshot(&self) -> Outcome<CountersSnapshot> {
        let ctr = lock_read!(self.counters);
        Ok(ctr.clone())
    }

    /// Record a periodic sample of the current counter state into
    /// the history ring. Intended to be called from a background
    /// task at a fixed interval; the dashboard reads from the
    /// history when drawing time-series charts. Trims the oldest
    /// entry when the ring reaches `history_capacity`.
    pub fn sample_now(&self) -> Outcome<()> {
        let ctr = lock_read!(self.counters);
        let sample = TrafficSample {
            when_secs:  SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            total:      ctr.total,
            by_status:  ctr.by_status.clone(),
        };
        drop(ctr);
        let mut hist = lock_write!(self.history);
        if hist.len() == self.history_capacity {
            hist.pop_front();
        }
        hist.push_back(sample);
        Ok(())
    }

    /// Clone the full history ring in chronological order
    /// (oldest first). Cheap: one short read lock plus a
    /// per-sample clone.
    pub fn history_snapshot(&self) -> Outcome<Vec<TrafficSample>> {
        let hist = lock_read!(self.history);
        let mut out = Vec::with_capacity(hist.len());
        for s in hist.iter() {
            out.push(s.clone());
        }
        Ok(out)
    }
}

impl Default for TrafficRecorder {
    fn default() -> Self {
        Self::new(DEFAULT_RING_CAPACITY)
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Current unix time in nanoseconds, clamped to zero on clock
/// error. Call-site helper for the request pipeline.
pub fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    fn mkrec(vhost: &str, path: &str, status: u16) -> RequestRecord {
        RequestRecord {
            when_ns:        now_ns(),
            vhost:          vhost.to_string(),
            method:         "GET".to_string(),
            path:           path.to_string(),
            status,
            peer:           "127.0.0.1:0".to_string(),
            bytes:          Some(42),
            duration_us:    123,
        }
    }

    #[test]
    fn record_and_recent() {
        let r = TrafficRecorder::new(4);
        r.record(mkrec("a", "/", 200)).expect("rec 1");
        r.record(mkrec("a", "/x", 200)).expect("rec 2");
        r.record(mkrec("a", "/y", 404)).expect("rec 3");
        let recent = r.recent(0).expect("recent");
        assert_eq!(recent.len(), 3);
        // Newest first.
        assert_eq!(recent[0].path, "/y");
        assert_eq!(recent[2].path, "/");
    }

    #[test]
    fn ring_evicts_oldest() {
        let r = TrafficRecorder::new(2);
        r.record(mkrec("a", "/1", 200)).expect("rec 1");
        r.record(mkrec("a", "/2", 200)).expect("rec 2");
        r.record(mkrec("a", "/3", 200)).expect("rec 3");
        let recent = r.recent(0).expect("recent");
        assert_eq!(recent.len(), 2);
        // Oldest ("/1") must have been dropped.
        let paths: Vec<&str> = recent.iter().map(|r| r.path.as_str()).collect();
        assert!(!paths.contains(&"/1"));
        assert!(paths.contains(&"/3"));
    }

    #[test]
    fn counters_track_status_and_vhost() {
        let r = TrafficRecorder::new(100);
        r.record(mkrec("a", "/", 200)).expect("rec");
        r.record(mkrec("a", "/", 200)).expect("rec");
        r.record(mkrec("a", "/", 404)).expect("rec");
        r.record(mkrec("b", "/", 500)).expect("rec");
        let snap = r.counters_snapshot().expect("snap");
        assert_eq!(snap.total, 4);
        assert_eq!(snap.by_status.get(&200).copied(), Some(2));
        assert_eq!(snap.by_status.get(&404).copied(), Some(1));
        assert_eq!(snap.by_status.get(&500).copied(), Some(1));
        let vh_a = snap.by_vhost.get("a").expect("vhost a");
        assert_eq!(vh_a.total, 3);
        let vh_b = snap.by_vhost.get("b").expect("vhost b");
        assert_eq!(vh_b.total, 1);
        assert_eq!(r.total(), 4);
    }

    #[test]
    fn per_vhost_path_bucket_saturates() {
        let r = TrafficRecorder::new(10_000);
        // Fill the per-vhost path map to its cap.
        for i in 0..(MAX_PATHS_PER_VHOST + 5) {
            let p = fmt!("/p{}", i);
            r.record(mkrec("a", &p, 200)).expect("rec");
        }
        let snap = r.counters_snapshot().expect("snap");
        let vh_a = snap.by_vhost.get("a").expect("vhost a");
        // Cap observed, plus the _other overflow bucket.
        assert!(vh_a.by_path.len() <= MAX_PATHS_PER_VHOST + 1);
        assert_eq!(
            vh_a.by_path.get(OTHER_PATH_BUCKET).copied(),
            Some(5),
        );
    }

    #[test]
    fn recent_honours_limit() {
        let r = TrafficRecorder::new(100);
        for i in 0..10 {
            r.record(mkrec("a", &fmt!("/p{}", i), 200)).expect("rec");
        }
        let three = r.recent(3).expect("recent");
        assert_eq!(three.len(), 3);
        assert_eq!(three[0].path, "/p9");
        assert_eq!(three[2].path, "/p7");
    }
}
