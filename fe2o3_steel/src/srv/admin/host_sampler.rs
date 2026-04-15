//! Host resource sampler for the admin dashboard.
//!
//! Periodically takes a snapshot via `fe2o3_sys::Snapshot::sample`
//! and keeps a bounded ring of recent readings. The dashboard
//! reads the ring to draw host-resource charts (CPU, memory,
//! disk, network, load average).
//!
//! The sampler is parallel to [`traffic::TrafficRecorder`]: same
//! bounded-ring shape, same fixed-interval sampler task, same
//! `Arc`-shared ownership between the server and the dashboard.
//! Constructed once in the TUI startup path and carried through
//! [`AdminState`](super::state::AdminState).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::snapshot::Snapshot;

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        RwLock,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

/// Default number of host snapshots kept in the ring. At the
/// default sample interval (5 s) this works out to one hour of
/// history, matching `TrafficRecorder::DEFAULT_HISTORY_CAPACITY`.
pub const DEFAULT_HISTORY_CAPACITY: usize = 720;

/// Default interval between host samples, in seconds.
pub const DEFAULT_SAMPLE_INTERVAL_SECS: u64 = 5;

/// One entry in the host-sampler history. Pairs a timestamp with
/// the raw [`Snapshot`]; rate-derived figures (CPU busy, disk
/// throughput) are computed by the consumer against the previous
/// entry because ring iteration is cheap and this keeps the
/// sampler hot path free of arithmetic.
#[derive(Clone, Debug)]
pub struct HostSample {
    /// Unix seconds at which the sample was taken.
    pub when_secs: u64,
    /// Raw metrics snapshot.
    pub snapshot:  Snapshot,
}

/// Bounded ring of host snapshots.
///
/// Cheaply cloneable via `Arc`; shared between the periodic
/// sampler task spawned in [`Server::start`] and every dashboard
/// request handler.
#[derive(Debug)]
pub struct HostSampler {
    /// Maximum number of samples retained.
    history_capacity: usize,
    /// Ring of samples, newest-last.
    history:          RwLock<VecDeque<HostSample>>,
}

impl HostSampler {
    /// Construct a sampler with the default history capacity.
    pub fn new() -> Self {
        Self {
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            history:          RwLock::new(
                VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            ),
        }
    }

    /// Wrap a fresh sampler in an `Arc` for shared ownership.
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Maximum number of samples retained.
    pub fn history_capacity(&self) -> usize {
        self.history_capacity
    }

    /// Take a fresh [`Snapshot`] and push it into the history.
    /// Trims the oldest entry when the ring reaches capacity.
    pub fn sample_now(&self) -> Outcome<()> {
        let snap = res!(Snapshot::sample());
        let entry = HostSample {
            when_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            snapshot: snap,
        };
        let mut hist = lock_write!(self.history);
        if hist.len() == self.history_capacity {
            hist.pop_front();
        }
        hist.push_back(entry);
        Ok(())
    }

    /// Clone the history ring in chronological order (oldest
    /// first). Cheap: one short read lock plus a per-sample
    /// clone.
    pub fn history_snapshot(&self) -> Outcome<Vec<HostSample>> {
        let hist = lock_read!(self.history);
        let mut out = Vec::with_capacity(hist.len());
        for s in hist.iter() {
            out.push(s.clone());
        }
        Ok(out)
    }

    /// Most recent sample, if any. Returns `None` before the
    /// sampler has been primed.
    pub fn latest(&self) -> Outcome<Option<HostSample>> {
        let hist = lock_read!(self.history);
        Ok(hist.back().cloned())
    }
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}
