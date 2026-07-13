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

impl ProcSelf {
    /// Cumulative CPU time (user plus system) consumed by the current
    /// process, in kernel clock ticks.  Read from `/proc/self/stat`
    /// fields `utime` and `stime`.  On Linux the tick rate is
    /// conventionally 100 Hz, so one tick is 10 ms; comparing two
    /// samples taken a known interval apart gives the fraction of a
    /// core the process is burning.  A process that blocks while idle
    /// accrues almost no ticks, whereas one that busy-polls accrues
    /// them continuously.
    pub fn cpu_ticks() -> Outcome<u64> {
        let content = res!(read_to_string("/proc/self/stat"));
        Self::cpu_ticks_from_stat(&content)
    }

    /// Parse `utime + stime` from a `/proc/self/stat` body.  The
    /// second field (`comm`) can itself contain spaces and brackets,
    /// so parsing begins after the final `)`.
    pub fn cpu_ticks_from_stat(content: &str) -> Outcome<u64> {
        let tail = match content.rfind(')') {
            Some(i) => &content[i + 1..],
            None    => return Err(err!(
                "No comm field terminator ')' found in /proc/self/stat body.";
                Input, Invalid, Decode)),
        };
        // After the `)`, the first token is field 3 (state), so utime
        // (field 14) and stime (field 15) sit at indices 11 and 12 of
        // this zero-based token list.
        let toks: Vec<&str> = tail.split_whitespace().collect();
        let utime = match toks.get(11) {
            Some(t) => res!(parse_num::<u64>(t, "utime")),
            None    => return Err(err!(
                "Missing utime field in /proc/self/stat.";
                Input, Missing, Decode)),
        };
        let stime = match toks.get(12) {
            Some(t) => res!(parse_num::<u64>(t, "stime")),
            None    => return Err(err!(
                "Missing stime field in /proc/self/stat.";
                Input, Missing, Decode)),
        };
        Ok(utime.saturating_add(stime))
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
