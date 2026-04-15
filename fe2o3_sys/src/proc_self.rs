//! Resource footprint of the current process from
//! `/proc/self/status` and `/proc/self/stat`.

use crate::parse::{
    parse_num,
    read_to_string,
};

use oxedyne_fe2o3_core::prelude::*;

/// Fields extracted from `/proc/self/status`. All byte figures
/// arrive from the kernel as kB and are converted on ingest.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcSelf {
    /// Resident set size (physical RAM currently in use).
    pub rss_bytes:  u64,
    /// Virtual memory size.
    pub vsize_bytes:u64,
    /// Peak resident set size observed for this process.
    pub peak_rss:   u64,
    /// Number of threads in the process.
    pub threads:    u32,
    /// Number of voluntary context switches (cooperative).
    pub voluntary_ctxt: u64,
    /// Number of involuntary context switches (pre-empted).
    pub involuntary_ctxt: u64,
}

impl ProcSelf {
    /// Read `/proc/self/status` and parse the fields.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/self/status"));
        Self::from_status(&content)
    }

    /// Parse a `/proc/self/status` body.
    pub fn from_status(content: &str) -> Outcome<Self> {
        let mut out = Self::default();
        for line in content.lines() {
            let (name, rest) = match line.split_once(':') {
                Some(t) => t,
                None    => continue,
            };
            let rest = rest.trim();
            match name {
                "VmRSS" => {
                    let kb = res!(parse_kb(rest, "VmRSS"));
                    out.rss_bytes = kb.saturating_mul(1024);
                },
                "VmSize" => {
                    let kb = res!(parse_kb(rest, "VmSize"));
                    out.vsize_bytes = kb.saturating_mul(1024);
                },
                "VmPeak" => {
                    let kb = res!(parse_kb(rest, "VmPeak"));
                    out.peak_rss = kb.saturating_mul(1024);
                },
                "Threads" => {
                    out.threads = res!(parse_num::<u32>(rest, "Threads"));
                },
                "voluntary_ctxt_switches" => {
                    out.voluntary_ctxt = res!(parse_num::<u64>(rest, "voluntary_ctxt_switches"));
                },
                "nonvoluntary_ctxt_switches" => {
                    out.involuntary_ctxt = res!(parse_num::<u64>(rest, "nonvoluntary_ctxt_switches"));
                },
                _ => (),
            }
        }
        Ok(out)
    }
}

/// Parse a value of the form `<number> kB` into `u64` kilobytes.
fn parse_kb(rest: &str, field: &str) -> Outcome<u64> {
    let tok = match rest.split_whitespace().next() {
        Some(t) => t,
        None    => return Err(err!(
            "{} field is empty.", field;
            Input, Missing, Decode)),
    };
    parse_num::<u64>(tok, field)
}
