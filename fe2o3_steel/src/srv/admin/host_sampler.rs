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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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

// At the default sample interval this is one hour of history, matching
// `TrafficRecorder::DEFAULT_HISTORY_CAPACITY`.
pub const DEFAULT_HISTORY_CAPACITY:     usize = 720;
pub const DEFAULT_SAMPLE_INTERVAL_SECS: u64 = 5;

/// Pairs a timestamp with the raw [`Snapshot`]. Rate-derived figures (CPU busy,
/// disk throughput) are computed by the consumer against the previous entry,
/// which keeps the sampler hot path free of arithmetic.
#[derive(Clone, Debug)]
pub struct HostSample {
    pub when_secs: u64,     // unix seconds
    pub snapshot:  Snapshot,
}

/// A timestamp plus the four already-derived series values: the shape emitted
/// by `/admin/host.json` and the shape persisted to ozone so history survives a
/// restart.
///
/// Derived because the useful figures need a pair of adjacent raw samples (CPU
/// busy fraction, disk B/s, net B/s). Persisting the reduced form keeps the
/// on-disk footprint small and sidesteps the need for ozone encoders over the
/// full `/proc`-derived struct tree.
#[derive(Clone, Copy, Debug)]
pub struct DerivedHostPoint {
    pub t_secs:     u64,    // unix seconds of the later sample of the pair
    pub cpu_pct:    f64,    // busy fraction over the preceding interval, per cent
    pub mem_pct:    f64,    // used fraction of total RAM, per cent
    pub disk_bps:   f64,    // aggregate disk throughput, bytes per second
    pub net_bps:    f64,    // aggregate non-loopback rx + tx, bytes per second
}

/// Bounded ring of host snapshots, cheaply cloneable via `Arc` and shared
/// between the periodic sampler task spawned in [`Server::start`] and every
/// dashboard request handler.
#[derive(Debug)]
pub struct HostSampler {
    history_capacity: usize,
    history:          RwLock<VecDeque<HostSample>>,  // newest last
    // Pre-restart points, loaded from ozone at start-up and rendered alongside
    // the live derived history so the Overview sparkline strip does not reset to
    // blank when Steel is restarted.
    persisted:        RwLock<Vec<DerivedHostPoint>>,
}

impl HostSampler {
    pub fn new() -> Self {
        Self {
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            history:          RwLock::new(
                VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            ),
            persisted:        RwLock::new(Vec::new()),
        }
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn history_capacity(&self) -> usize {
        self.history_capacity
    }

    /// Trims the oldest entry when the ring is already at capacity.
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

    /// Chronological order, oldest first.
    pub fn history_snapshot(&self) -> Outcome<Vec<HostSample>> {
        let hist = lock_read!(self.history);
        let mut out = Vec::with_capacity(hist.len());
        for s in hist.iter() {
            out.push(s.clone());
        }
        Ok(out)
    }

    pub fn latest(&self) -> Outcome<Option<HostSample>> {
        let hist = lock_read!(self.history);
        Ok(hist.back().cloned())
    }

    /// Each entry carries the later-of-pair timestamp, because the rate-based
    /// figures need two consecutive samples.
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

    pub fn seed_persisted(&self, points: Vec<DerivedHostPoint>) -> Outcome<()> {
        let mut slot = lock_write!(self.persisted);
        *slot = points;
        Ok(())
    }

    /// Persisted plus live, capped at the ring's history capacity. The merge
    /// drops persisted points at or after the oldest live derived timestamp, so
    /// a sample still present in the live ring is not double-counted.
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
