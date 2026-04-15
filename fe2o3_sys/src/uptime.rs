//! System uptime from `/proc/uptime`.

use crate::parse::{
    parse_num,
    read_to_string,
    tokens,
};

use oxedyne_fe2o3_core::prelude::*;

/// Seconds since boot and seconds spent idle (summed across
/// every CPU). Both are floating-point as reported by the
/// kernel so sub-second precision is preserved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Uptime {
    /// Wall-clock seconds since boot.
    pub seconds:     f64,
    /// Idle seconds, summed across every CPU.
    pub idle_total:  f64,
}

impl Uptime {
    /// Read `/proc/uptime`.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/uptime"));
        Self::from_line(content.trim())
    }

    /// Parse one `/proc/uptime` line of the form
    /// `<uptime_seconds> <idle_seconds>`.
    pub fn from_line(line: &str) -> Outcome<Self> {
        let toks = tokens(line);
        if toks.len() < 2 {
            return Err(err!(
                "uptime line has {} tokens: {:?}",
                toks.len(), line;
                Input, Size, Decode));
        }
        Ok(Self {
            seconds:    res!(parse_num::<f64>(toks[0], "uptime.seconds")),
            idle_total: res!(parse_num::<f64>(toks[1], "uptime.idle_total")),
        })
    }
}
