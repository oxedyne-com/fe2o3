//! Aggregate [`Snapshot`] grouping every metric module.
//!
//! Producers call [`Snapshot::sample`] on a regular cadence; two
//! consecutive snapshots can be compared via [`Snapshot::delta`]
//! to derive rate-based figures (CPU busy percentage, disk
//! throughput, network throughput) without each consumer having
//! to juggle separate prev/curr pairs.

use crate::{
    cpu::CpuTimes,
    disk::{
        DiskDelta,
        DiskStats,
    },
    load::LoadAvg,
    mem::MemInfo,
    net::{
        NetDelta,
        NetStats,
    },
    proc_self::ProcSelf,
    uptime::Uptime,
};

use oxedyne_fe2o3_core::prelude::*;

/// One host-stats reading. Fields are captured independently, so
/// a failure in any single `/proc` file surfaces as the crate
/// error for that file; no partial snapshots.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Monotonic timestamp, nanoseconds since an arbitrary epoch
    /// (typically Instant::now() mapped to an integer). Stored as
    /// plain `u64` so [`Snapshot`] remains `Default`-able and
    /// trivially serialisable.
    pub monotonic_ns: u64,
    /// Aggregate CPU time counters.
    pub cpu:      CpuTimes,
    /// Memory figures.
    pub mem:      MemInfo,
    /// Load averages.
    pub load:     LoadAvg,
    /// Per-device block I/O counters.
    pub disk:     DiskStats,
    /// Per-interface network counters.
    pub net:      NetStats,
    /// Uptime counters.
    pub uptime:   Uptime,
    /// Current process footprint.
    pub proc_self:ProcSelf,
}

impl Snapshot {
    /// Take a fresh reading. Any `/proc` read failure aborts the
    /// snapshot and is returned; callers deciding whether to
    /// retry should inspect the error kind.
    pub fn sample() -> Outcome<Self> {
        Ok(Self {
            monotonic_ns: now_ns(),
            cpu:       res!(CpuTimes::sample()),
            mem:       res!(MemInfo::sample()),
            load:      res!(LoadAvg::sample()),
            disk:      res!(DiskStats::sample()),
            net:       res!(NetStats::sample()),
            uptime:    res!(Uptime::sample()),
            proc_self: res!(ProcSelf::sample()),
        })
    }

    /// Elapsed wall-clock seconds between `prev` and `self`,
    /// derived from the `monotonic_ns` fields.
    pub fn elapsed_since(&self, prev: &Self) -> f64 {
        if self.monotonic_ns <= prev.monotonic_ns {
            return 0.0;
        }
        (self.monotonic_ns - prev.monotonic_ns) as f64 / 1_000_000_000.0
    }

    /// Derive rate-based figures from the difference between
    /// this snapshot and `prev`.
    pub fn delta(&self, prev: &Self) -> SnapshotDelta {
        let elapsed = self.elapsed_since(prev);
        SnapshotDelta {
            elapsed_s:       elapsed,
            cpu_busy:        self.cpu.busy_fraction(&prev.cpu),
            disk:            self.disk.deltas(&prev.disk, elapsed),
            net:             self.net.deltas(&prev.net, elapsed),
        }
    }
}

/// Rate-based figures computed between two snapshots.
#[derive(Clone, Debug, Default)]
pub struct SnapshotDelta {
    /// Elapsed wall-clock seconds between samples.
    pub elapsed_s: f64,
    /// CPU busy fraction (0..=1) over the interval.
    pub cpu_busy:  f64,
    /// Per-device I/O rates.
    pub disk:      Vec<DiskDelta>,
    /// Per-interface network rates.
    pub net:       Vec<NetDelta>,
}

/// Nanoseconds since an arbitrary monotonic epoch. Uses
/// `std::time::Instant::now()` elapsed against a process-lifetime
/// base captured on first call.
fn now_ns() -> u64 {
    use std::{
        sync::OnceLock,
        time::Instant,
    };
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}
