//! CPU time accounting from `/proc/stat`.
//!
//! The kernel reports cumulative tick counters (jiffies) per mode
//! per CPU. A single instantaneous read tells you how much time
//! the machine has spent in each mode since boot; two reads a few
//! seconds apart let you compute the busy percentage over that
//! interval via [`CpuTimes::busy_fraction`].

use crate::parse::{
    parse_num,
    read_to_string,
    tokens,
};

use oxedyne_fe2o3_core::prelude::*;

/// Absolute tick counts for the aggregate `cpu` line of
/// `/proc/stat`. All fields are cumulative since boot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CpuTimes {
    /// User-mode time (excluding nice).
    pub user:       u64,
    /// Niced user-mode time.
    pub nice:       u64,
    /// Kernel-mode time.
    pub system:     u64,
    /// Idle time.
    pub idle:       u64,
    /// Waiting on I/O.
    pub iowait:     u64,
    /// Servicing hardware interrupts.
    pub irq:        u64,
    /// Servicing soft interrupts.
    pub softirq:    u64,
    /// Involuntary wait (stolen by other tenants).
    pub steal:      u64,
    /// Virtual-guest time not already counted in `user`.
    pub guest:      u64,
    /// Niced virtual-guest time.
    pub guest_nice: u64,
}

impl CpuTimes {
    /// Read `/proc/stat` and return the aggregate `cpu` line.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/stat"));
        Self::from_stat(&content)
    }

    /// Parse the aggregate `cpu` line out of a `/proc/stat` body.
    pub fn from_stat(content: &str) -> Outcome<Self> {
        for line in content.lines() {
            if line.starts_with("cpu ") || line.starts_with("cpu\t") {
                return Self::from_line(line);
            }
        }
        Err(err!(
            "/proc/stat did not contain an aggregate 'cpu ' line.";
            Input, Missing, Decode))
    }

    /// Parse one `cpu...` line. Accepts the aggregate and
    /// per-cpu forms; the caller selects which one to pass in.
    pub fn from_line(line: &str) -> Outcome<Self> {
        let toks = tokens(line);
        if toks.len() < 5 {
            return Err(err!(
                "cpu line has only {} tokens: {:?}",
                toks.len(), line;
                Input, Size, Decode));
        }
        let get = |i: usize| -> Outcome<u64> {
            if i >= toks.len() {
                Ok(0)
            } else {
                parse_num::<u64>(toks[i], "cpu")
            }
        };
        Ok(Self {
            user:       res!(get(1)),
            nice:       res!(get(2)),
            system:     res!(get(3)),
            idle:       res!(get(4)),
            iowait:     res!(get(5)),
            irq:        res!(get(6)),
            softirq:    res!(get(7)),
            steal:      res!(get(8)),
            guest:      res!(get(9)),
            guest_nice: res!(get(10)),
        })
    }

    /// Total ticks since boot, across every mode.
    pub fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }

    /// Idle ticks since boot. Counts `idle` and `iowait`; anything
    /// else is considered busy.
    pub fn idle_total(&self) -> u64 {
        self.idle + self.iowait
    }

    /// Fraction (0..=1) of ticks spent busy over the interval
    /// between `prev` and `self`. Returns 0 if the interval is
    /// zero or the clock appears to have moved backwards.
    pub fn busy_fraction(&self, prev: &Self) -> f64 {
        let total_now  = self.total();
        let total_prev = prev.total();
        if total_now <= total_prev {
            return 0.0;
        }
        let idle_now  = self.idle_total();
        let idle_prev = prev.idle_total();
        let total_delta = (total_now - total_prev) as f64;
        let idle_delta  = idle_now.saturating_sub(idle_prev) as f64;
        let busy = total_delta - idle_delta;
        if busy <= 0.0 {
            0.0
        } else {
            busy / total_delta
        }
    }
}
