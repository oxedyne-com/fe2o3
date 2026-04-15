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

/// Point on the Overview sparkline strip: a timestamp plus the four
/// already-derived series values. This is the shape emitted by
/// `/admin/host.json` and the shape persisted to ozone so history
/// survives a restart.
///
/// Derived because the useful figures for the Overview sparkline
/// strip need a pair of adjacent raw samples (CPU busy fraction,
/// disk B/s, net B/s). Persisting the reduced form keeps the
/// on-disk footprint small and sidesteps the need for ozone
/// encoders over the full `/proc`-derived struct tree.
#[derive(Clone, Copy, Debug)]
pub struct DerivedHostPoint {
    /// Unix seconds at which the point's later-of-pair sample was taken.
    pub t_secs:     u64,
    /// CPU busy fraction over the preceding interval, in per cent.
    pub cpu_pct:    f64,
    /// Memory used as a fraction of total RAM, in per cent, at the
    /// later-of-pair timestamp.
    pub mem_pct:    f64,
    /// Aggregate disk throughput in bytes per second over the
    /// preceding interval.
    pub disk_bps:   f64,
    /// Aggregate non-loopback network throughput (rx + tx) in
    /// bytes per second over the preceding interval.
    pub net_bps:    f64,
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
    /// Pre-restart points, loaded from ozone at start-up. Rendered
    /// alongside the live derived history so the Overview sparkline
    /// strip does not reset to blank when Steel is restarted.
    persisted:        RwLock<Vec<DerivedHostPoint>>,
}

impl HostSampler {
    /// Construct a sampler with the default history capacity.
    pub fn new() -> Self {
        Self {
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            history:          RwLock::new(
                VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            ),
            persisted:        RwLock::new(Vec::new()),
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

    /// Compute the derived sparkline history from the live ring.
    /// Each entry uses the later-of-pair timestamp because the
    /// rate-based figures need two consecutive samples. Returns an
    /// empty `Vec` when the ring holds fewer than two entries.
    pub fn derived_history(&self) -> Outcome<Vec<DerivedHostPoint>> {
        let hist = lock_read!(self.history);
        if hist.len() < 2 {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(hist.len() - 1);
        let mut iter = hist.iter();
        let mut prev = match iter.next() {
            Some(p) => p,
            None => return Ok(out),
        };
        for curr in iter {
            let delta = curr.snapshot.delta(&prev.snapshot);
            let disk_bps: f64 = delta.disk.iter()
                .map(|d| d.read_bps + d.write_bps).sum();
            let net_bps: f64 = delta.net.iter()
                .filter(|n| n.name != "lo")
                .map(|n| n.rx_bps + n.tx_bps).sum();
            out.push(DerivedHostPoint {
                t_secs:     curr.when_secs,
                cpu_pct:    delta.cpu_busy * 100.0,
                mem_pct:    curr.snapshot.mem.used_fraction() * 100.0,
                disk_bps,
                net_bps,
            });
            prev = curr;
        }
        Ok(out)
    }

    /// Replace the persisted history with the supplied points. Used
    /// by start-up restore to prime the sparkline strip with the
    /// derived history saved by the previous run.
    pub fn seed_persisted(&self, points: Vec<DerivedHostPoint>) -> Outcome<()> {
        let mut slot = lock_write!(self.persisted);
        *slot = points;
        Ok(())
    }

    /// Combined persisted-plus-live derived history, capped at the
    /// ring's history capacity. The merge drops persisted points
    /// whose timestamp is at or after the oldest live derived
    /// timestamp, so a sample that is still present in the live
    /// ring is not double-counted.
    pub fn merged_derived_history(&self) -> Outcome<Vec<DerivedHostPoint>> {
        let live = res!(self.derived_history());
        let persisted = {
            let g = lock_read!(self.persisted);
            g.clone()
        };
        if live.is_empty() {
            return Ok(persisted);
        }
        if persisted.is_empty() {
            return Ok(live);
        }
        let cutoff = live.first().map(|p| p.t_secs).unwrap_or(0);
        let mut out: Vec<DerivedHostPoint> = persisted.into_iter()
            .filter(|p| p.t_secs < cutoff)
            .collect();
        out.extend(live);
        if out.len() > self.history_capacity {
            let excess = out.len() - self.history_capacity;
            out.drain(..excess);
        }
        Ok(out)
    }
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}
