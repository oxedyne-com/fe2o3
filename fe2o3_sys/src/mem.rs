//! Memory accounting from `/proc/meminfo`.

use crate::parse::{
    parse_num,
    read_to_string,
};

use oxedyne_fe2o3_core::prelude::*;

/// Snapshot of the key memory figures reported by the kernel.
/// All values are in bytes; the `/proc/meminfo` kB values are
/// multiplied on ingest so downstream consumers never have to
/// worry about the unit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemInfo {
    /// Total usable RAM.
    pub total:      u64,
    /// Free RAM (no use by kernel or user).
    pub free:       u64,
    /// Kernel's best guess at actually free RAM including
    /// reclaimable page cache and buffers.
    pub available:  u64,
    /// Block-device buffers.
    pub buffers:    u64,
    /// Page cache.
    pub cached:     u64,
    /// Total swap space configured.
    pub swap_total: u64,
    /// Swap currently in use.
    pub swap_free:  u64,
}

impl MemInfo {
    /// Read `/proc/meminfo` and parse the key fields.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/meminfo"));
        Self::from_meminfo(&content)
    }

    /// Parse a `/proc/meminfo` body. Each line has the form
    /// `Name: <value> kB`; unknown names are ignored.
    pub fn from_meminfo(content: &str) -> Outcome<Self> {
        let mut out = Self::default();
        for line in content.lines() {
            let (name, rest) = match line.split_once(':') {
                Some(t) => t,
                None    => continue,
            };
            let mut iter = rest.split_whitespace();
            let value = match iter.next() {
                Some(v) => v,
                None    => continue,
            };
            let kb = res!(parse_num::<u64>(value, name.trim()));
            let bytes = kb.saturating_mul(1024);
            match name.trim() {
                "MemTotal"     => out.total      = bytes,
                "MemFree"      => out.free       = bytes,
                "MemAvailable" => out.available  = bytes,
                "Buffers"      => out.buffers    = bytes,
                "Cached"       => out.cached     = bytes,
                "SwapTotal"    => out.swap_total = bytes,
                "SwapFree"     => out.swap_free  = bytes,
                _              => (),
            }
        }
        if out.total == 0 {
            return Err(err!(
                "/proc/meminfo did not contain a MemTotal field.";
                Input, Missing, Decode));
        }
        Ok(out)
    }

    /// Bytes actually in use by processes and the kernel,
    /// approximated as `total - available`. Prefer this over
    /// `total - free` which double-counts page cache as "used".
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Used memory as a fraction (0..=1) of total.
    pub fn used_fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f64 / self.total as f64
        }
    }

    /// Swap currently in use, in bytes.
    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}
