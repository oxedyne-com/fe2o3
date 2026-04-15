//! Block-device I/O accounting from `/proc/diskstats`.
//!
//! The kernel exposes one line per block device with cumulative
//! counters. Two samples separated in time let you compute
//! throughput, IOPS, and utilisation; see [`DiskStats::deltas`].

use crate::parse::{
    parse_num,
    read_to_string,
    tokens,
};

use oxedyne_fe2o3_core::prelude::*;

/// Sector size assumed when translating diskstats "sectors"
/// counters to bytes. The kernel interface has used a fixed
/// 512-byte sector forever; the on-disk physical sector size is
/// a separate concept.
pub const SECTOR_BYTES: u64 = 512;

/// One block device's cumulative counters, extracted from a
/// single line of `/proc/diskstats`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskLine {
    /// Device name, e.g. `sda`, `nvme0n1`, `dm-0`.
    pub name:         String,
    /// Cumulative reads completed successfully.
    pub reads:        u64,
    /// Cumulative sectors read.
    pub read_sectors: u64,
    /// Cumulative time spent on reads (ms).
    pub read_ms:      u64,
    /// Cumulative writes completed successfully.
    pub writes:       u64,
    /// Cumulative sectors written.
    pub write_sectors:u64,
    /// Cumulative time spent on writes (ms).
    pub write_ms:     u64,
    /// Cumulative time spent on all I/O (ms). Handy for a
    /// rough utilisation figure.
    pub io_ms:        u64,
}

/// A collection of block-device counters. Keeps them in the
/// order seen on disk so downstream renderers can display them
/// consistently.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskStats {
    pub devices: Vec<DiskLine>,
}

impl DiskStats {
    /// Read `/proc/diskstats`.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/diskstats"));
        Self::from_diskstats(&content)
    }

    /// Parse a `/proc/diskstats` body. Lines that do not have the
    /// expected column count are silently skipped (the kernel
    /// version has grown the format over time; we are liberal in
    /// what we accept).
    pub fn from_diskstats(content: &str) -> Outcome<Self> {
        let mut out = Self::default();
        for line in content.lines() {
            let toks = tokens(line);
            // Format: major minor name reads reads_merged read_sectors
            // read_ms writes writes_merged write_sectors write_ms
            // io_in_progress io_ms weighted_io_ms ...
            // Need at least 14 columns for the fields we use.
            if toks.len() < 14 {
                continue;
            }
            let name = toks[2].to_string();
            out.devices.push(DiskLine {
                name,
                reads:         res!(parse_num::<u64>(toks[3],  "disk.reads")),
                read_sectors:  res!(parse_num::<u64>(toks[5],  "disk.read_sectors")),
                read_ms:       res!(parse_num::<u64>(toks[6],  "disk.read_ms")),
                writes:        res!(parse_num::<u64>(toks[7],  "disk.writes")),
                write_sectors: res!(parse_num::<u64>(toks[9],  "disk.write_sectors")),
                write_ms:      res!(parse_num::<u64>(toks[10], "disk.write_ms")),
                io_ms:         res!(parse_num::<u64>(toks[12], "disk.io_ms")),
            });
        }
        Ok(out)
    }

    /// Compute per-device rate deltas between `prev` and `self`.
    /// Returns read/write bytes per second and an I/O utilisation
    /// fraction over the elapsed wall-clock interval in seconds.
    pub fn deltas(&self, prev: &Self, elapsed_s: f64) -> Vec<DiskDelta> {
        let mut out = Vec::with_capacity(self.devices.len());
        for dev in &self.devices {
            let p = prev.devices.iter().find(|d| d.name == dev.name);
            let p = match p {
                Some(p) => p,
                None    => continue,
            };
            let dr = dev.read_sectors.saturating_sub(p.read_sectors) * SECTOR_BYTES;
            let dw = dev.write_sectors.saturating_sub(p.write_sectors) * SECTOR_BYTES;
            let dio = dev.io_ms.saturating_sub(p.io_ms);
            let (rps, wps, util) = if elapsed_s > 0.0 {
                (
                    dr as f64 / elapsed_s,
                    dw as f64 / elapsed_s,
                    ((dio as f64) / 1000.0 / elapsed_s).min(1.0),
                )
            } else {
                (0.0, 0.0, 0.0)
            };
            out.push(DiskDelta {
                name:        dev.name.clone(),
                read_bps:    rps,
                write_bps:   wps,
                utilisation: util,
            });
        }
        out
    }
}

/// Per-device rate between two samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskDelta {
    /// Device name (as in `DiskLine::name`).
    pub name:        String,
    /// Read bytes per second over the interval.
    pub read_bps:    f64,
    /// Write bytes per second over the interval.
    pub write_bps:   f64,
    /// Fraction (0..=1) of the interval the device was busy with
    /// any I/O.
    pub utilisation: f64,
}
