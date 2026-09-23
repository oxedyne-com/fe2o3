//! Host resource sampler for the admin dashboard.
//!
//! Periodically takes a snapshot via `fe2o3_sys::Snapshot::sample`
//! and keeps a bounded ring of recent readings. The dashboard
//! reads the ring to draw host-resource charts (CPU, memory,
//! disk, network, load average).
//!
//! The sampler is parallel to [`super::traffic::TrafficRecorder`]: same
//! bounded-ring shape, same fixed-interval sampler task, same
//! `Arc`-shared ownership between the server and the dashboard.
//! Constructed once in the TUI startup path and carried through
//! [`AdminState`](super::state::AdminState).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::{
    resident::Resident,
    snapshot::Snapshot,
};

use std::{
    collections::VecDeque,
    path::{
        Path,
        PathBuf,
    },
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
// A resident scan reads every process's status file, so it rides every sixth
// tick rather than every one: half a watcher's default round, so no probe reads
// a figure more than thirty seconds old.
pub const RESIDENT_SAMPLE_SECS:         u64 = 30;

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

/// The four host figures the health body carries, already reduced to the
/// integers it emits. See [`HostSampler::health_metrics`].
#[derive(Clone, Copy, Debug)]
pub struct HealthHostMetrics {
    pub mem_pct:    i64,
    pub swap_pct:   i64,
    pub disk_iops:  i64,
    pub load1:      i64,    // 1-minute load average x100
}

/// Bounded ring of host snapshots, cheaply cloneable via `Arc` and shared
/// between the periodic sampler task spawned in [`crate::srv::server::Server::start`] and every
/// dashboard request handler.
#[derive(Debug)]
pub struct HostSampler {
    history_capacity: usize,
    history:          RwLock<VecDeque<HostSample>>,  // newest last
    // Pre-restart points, loaded from ozone at start-up and rendered alongside
    // the live derived history so the Overview sparkline strip does not reset to
    // blank when Steel is restarted.
    persisted:        RwLock<Vec<DerivedHostPoint>>,
    residents:        Vec<String>,  // `health_residents`, the processes to report
    // The last resident scan and when it was taken, in unix seconds. Read by the
    // health body, so a request formats figures rather than walking `/proc`.
    resident_scan:    RwLock<(u64, Vec<Resident>)>,
    // The filesystem whose fullness the health body reports as `disk_pct`: the
    // app root, where the databases and logs grow. `None` reports none.
    disk_path:        Option<PathBuf>,
    disk_pct:         RwLock<Option<i64>>,
}

impl HostSampler {
    pub fn new() -> Self {
        Self {
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            history:          RwLock::new(
                VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            ),
            persisted:        RwLock::new(Vec::new()),
            residents:        Vec::new(),
            resident_scan:    RwLock::new((0, Vec::new())),
            disk_path:        None,
            disk_pct:         RwLock::new(None),
        }
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// A sampler that also reads what the health body needs beyond the snapshot:
    /// the named processes, every [`RESIDENT_SAMPLE_SECS`], and how full the
    /// filesystem holding `disk_path` is, every tick.
    pub fn new_shared_for_health(
        residents:  Vec<String>,
        disk_path:  Option<PathBuf>,
    )
        -> Arc<Self>
    {
        let mut s = Self::new();
        s.residents = residents;
        s.disk_path = disk_path;
        Arc::new(s)
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
        let when_secs = entry.when_secs;
        {
            let mut hist = lock_write!(self.history);
            if hist.len() == self.history_capacity {
                hist.pop_front();
            }
            hist.push_back(entry);
        }
        // Each read below is tried whatever became of the one before it, and the
        // first failure is reported once both have had their turn.
        let mut failed = None;
        if let Some(path) = &self.disk_path {
            // One `statvfs` call. A failure leaves the field absent rather than
            // stale, so a watcher never reads an old figure as a current one.
            let pct = disk_used_pct(path);
            let mut slot = lock_write!(self.disk_pct);
            match pct {
                Ok(p) => *slot = Some(p),
                Err(e) => {
                    *slot = None;
                    failed = Some(e);
                },
            }
        }
        if !self.residents.is_empty() {
            let due = {
                let scan = lock_read!(self.resident_scan);
                scan.0 == 0 || when_secs.saturating_sub(scan.0) >= RESIDENT_SAMPLE_SECS
            };
            if due {
                let found = Resident::sample(&self.residents);
                let mut scan = lock_write!(self.resident_scan);
                match found {
                    Ok(f) => *scan = (when_secs, f),
                    Err(e) => {
                        // The attempt is stamped, so a failing scan waits out the
                        // cadence like a good one and warns once a scan, not once a
                        // tick.
                        scan.0 = when_secs;
                        if failed.is_none() {
                            failed = Some(e);
                        }
                    },
                }
            }
        }
        match failed {
            Some(e) => Err(e),
            None    => Ok(()),
        }
    }

    /// The last resident scan; empty until the first has run, or when no
    /// residents are configured.
    pub fn residents(&self) -> Outcome<Vec<Resident>> {
        let scan = lock_read!(self.resident_scan);
        Ok(scan.1.clone())
    }

    /// How full the app root's filesystem was at the last tick, or `None` when
    /// it has not been read or the read failed.
    pub fn disk_pct(&self) -> Outcome<Option<i64>> {
        let slot = lock_read!(self.disk_pct);
        Ok(*slot)
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

    /// The integer-ready host figures for the health body, or `None` when the
    /// ring is empty. `disk_iops` needs two adjacent samples for its rate and is
    /// zero until a second sample has landed; the level figures need only the
    /// latest.
    pub fn health_metrics(&self) -> Outcome<Option<HealthHostMetrics>> {
        let hist = lock_read!(self.history);
        let last = match hist.back() {
            Some(s) => s,
            None    => return Ok(None),
        };
        let mem_pct  = (last.snapshot.mem.used_fraction() * 100.0).round() as i64;
        let swap_pct = if last.snapshot.mem.swap_total == 0 {
            0
        } else {
            (last.snapshot.mem.swap_used() as f64
                / last.snapshot.mem.swap_total as f64 * 100.0).round() as i64
        };
        // Load average times a hundred, so a fractional load survives the
        // integer contract: 250 is a load of 2.50.
        let load1 = (last.snapshot.load.one * 100.0).round() as i64;
        // Completed reads + writes per second, summed over every device, from the
        // two most recent samples.
        let disk_iops = match hist.len() >= 2 {
            true => {
                let prev = &hist[hist.len() - 2];
                let elapsed = last.when_secs.saturating_sub(prev.when_secs);
                if elapsed == 0 {
                    0
                } else {
                    let mut ops: u64 = 0;
                    for dev in &last.snapshot.disk.devices {
                        if let Some(p) = prev.snapshot.disk.devices.iter()
                            .find(|d| d.name == dev.name)
                        {
                            ops = ops
                                .saturating_add(dev.reads.saturating_sub(p.reads))
                                .saturating_add(dev.writes.saturating_sub(p.writes));
                        }
                    }
                    (ops / elapsed) as i64
                }
            },
            false => 0,
        };
        Ok(Some(HealthHostMetrics { mem_pct, swap_pct, disk_iops, load1 }))
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

/// Per cent of a filesystem in use, as `df` reports it: blocks used over the
/// blocks an unprivileged writer could ever have, rounded up. The blocks reserved
/// for root count as neither, which is why a disk `df` shows at 100% still takes
/// root's writes -- and why an application that is not root finds it full.
pub fn used_pct(blocks: u64, free: u64, avail: u64) -> i64 {
    let used = blocks.saturating_sub(free);
    let usable = used.saturating_add(avail);
    if usable == 0 {
        return 0;
    }
    let pct = used.saturating_mul(100).saturating_add(usable - 1) / usable;
    pct.min(100) as i64
}

#[cfg(unix)]
fn disk_used_pct(path: &Path) -> Outcome<i64> {
    let st = match nix::sys::statvfs::statvfs(path) {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Reading the filesystem usage of {:?} for the health body's disk_pct.", path;
            IO, File, Read)),
    };
    Ok(used_pct(st.blocks() as u64, st.blocks_free() as u64, st.blocks_available() as u64))
}

#[cfg(not(unix))]
fn disk_used_pct(path: &Path) -> Outcome<i64> {
    Err(err!(
        "Reading the filesystem usage of {:?} is implemented for Unix only.", path;
        Unimplemented))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The figure `df` would print for the same counts, reserved blocks and all.
    #[test]
    fn disk_use_is_read_as_df_reads_it() {
        // 100 blocks, 20 free of which 5 are reserved for root: 80 used of 95 usable.
        assert_eq!(used_pct(100, 20, 15), 85, "80 of 95 is 84.2, which df rounds up");
        assert_eq!(used_pct(100, 5, 0), 100, "full to an unprivileged writer");
        assert_eq!(used_pct(100, 100, 100), 0);
        assert_eq!(used_pct(0, 0, 0), 0, "an empty filesystem is not a division by zero");
    }

    /// The sampler reads the filesystem it was pointed at on its own tick, so the
    /// health body only formats the figure.
    #[cfg(unix)]
    #[test]
    fn a_tick_reads_the_disk_the_sampler_was_given() -> Outcome<()> {
        let sampler = HostSampler::new_shared_for_health(Vec::new(), Some(PathBuf::from("/")));
        assert_eq!(res!(sampler.disk_pct()), None, "nothing read before the first tick");
        res!(sampler.sample_now());
        let pct = res!(res!(sampler.disk_pct()).ok_or_else(|| err!(
            "the first tick did not read the disk"; Test, Missing)));
        assert!((0..=100).contains(&pct), "got {}", pct);
        Ok(())
    }
}
