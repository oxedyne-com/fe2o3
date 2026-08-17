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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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

pub const DEFAULT_RING_CAPACITY:        usize = 10_000; // roughly 1-2 MiB live
// Past this many distinct paths, a vhost's further paths fold into the
// `_other` bucket, bounding worst-case memory when a caller probes unique URLs.
pub const MAX_PATHS_PER_VHOST:          usize = 256;
pub const OTHER_PATH_BUCKET:            &str = "_other";
// At the default sample interval the history spans one hour. Five seconds
// trades chart smoothness against CPU cost on a quiet host.
pub const DEFAULT_HISTORY_CAPACITY:     usize = 720;
pub const DEFAULT_SAMPLE_INTERVAL_SECS: u64 = 5;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REQUEST RECORD                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Snapshot of a single request as it leaves the handler pipeline. All fields
/// are owned, so the record can outlive the request without keeping borrows
/// alive.
#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub when_ns:        u64,            // unix nanoseconds at completion
    pub vhost:          String,         // lowercased hostname, as keyed in vhost_dbs
    pub method:         String,
    pub path:           String,         // includes the query string
    pub status:         u16,
    pub peer:           String,         // IP and port
    pub bytes:          Option<u64>,    // response body length, when known
    pub duration_us:    u64,            // accept to final write, microseconds
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COUNTERS                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Per-vhost counters, aggregated since the recorder was created, i.e. since
/// Steel started. Counts never decrement, so the dashboard can compute a rate
/// of change by sampling and subtracting across two fetches.
#[derive(Clone, Debug, Default)]
pub struct VhostCounters {
    pub total:      u64,
    pub by_status:  HashMap<u16, u64>,      // `{200 => 1234, 404 => 12, ...}`
    pub by_path:    HashMap<String, u64>,   // capped, overflow into `_other`
}

#[derive(Clone, Debug, Default)]
pub struct CountersSnapshot {
    pub total:      u64,
    pub by_status:  HashMap<u16, u64>,
    pub by_vhost:   HashMap<String, VhostCounters>,
}

/// One point in the bounded traffic history: the monotonic totals at a
/// particular unix second, so the dashboard can difference adjacent samples and
/// draw a requests-per-interval chart.
#[derive(Clone, Debug)]
pub struct TrafficSample {
    pub when_secs:  u64,                // unix seconds
    pub total:      u64,                // cumulative across every vhost
    pub by_status:  HashMap<u16, u64>,  // cumulative at this instant
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TRAFFIC RECORDER                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Thread-safe ring buffer plus counters, cheaply cloneable via `Arc`. A typical
/// deployment constructs one and stores it both in the request pipeline (via
/// `ServerContext`) and in the admin state, for the dashboard.
#[derive(Debug)]
pub struct TrafficRecorder {
    capacity:           usize,          // older entries drop once at capacity
    ring:               RwLock<VecDeque<RequestRecord>>,   // newest last
    // A distinct lock from the ring, so dashboard reads of counters and of
    // records do not contend on the same lock.
    counters:           RwLock<CountersSnapshot>,
    total:              AtomicU64,      // also in `counters`; atomic for cheap reads
    // Periodic counter samples, newest last, populated by a background sampling
    // task and read by the dashboard when drawing time-series charts.
    history:            RwLock<VecDeque<TrafficSample>>,
    history_capacity:   usize,
}

impl TrafficRecorder {
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

    pub fn new_shared(capacity: usize) -> Arc<Self> {
        Arc::new(Self::new(capacity))
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn history_capacity(&self) -> usize {
        self.history_capacity
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Takes two short write locks, one on the ring and one on the counters. Any
    /// lock poisoning surfaces as an error; the hot-path call site logs it and
    /// continues rather than aborting the request.
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

    /// Up to `limit` most recent records, newest first. A `limit` of zero returns
    /// everything currently in the ring.
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

    pub fn counters_snapshot(&self) -> Outcome<CountersSnapshot> {
        let ctr = lock_read!(self.counters);
        Ok(ctr.clone())
    }

    /// Meant for a background task on a fixed interval. Trims the oldest entry
    /// when the ring reaches `history_capacity`.
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

    /// Chronological order, oldest first.
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

/// Current unix time in nanoseconds, clamped to zero on clock error.
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
