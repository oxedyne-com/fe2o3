//! Load-average sampling from `/proc/loadavg`.

use crate::parse::{
    parse_num,
    read_to_string,
    tokens,
};

use oxedyne_fe2o3_core::prelude::*;

/// The three classic load averages plus the runnable / total
/// task counter the kernel writes after them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadAvg {
    /// One-minute load average.
    pub one:      f64,
    /// Five-minute load average.
    pub five:     f64,
    /// Fifteen-minute load average.
    pub fifteen:  f64,
    /// Number of currently runnable scheduling entities.
    pub runnable: u64,
    /// Total number of scheduling entities.
    pub total:    u64,
}

impl LoadAvg {
    /// Read `/proc/loadavg`.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/loadavg"));
        Self::from_line(content.trim())
    }

    /// Parse one `/proc/loadavg` line of the form
    /// `<one> <five> <fifteen> <runnable>/<total> <last_pid>`.
    pub fn from_line(line: &str) -> Outcome<Self> {
        let toks = tokens(line);
        if toks.len() < 4 {
            return Err(err!(
                "loadavg line has {} tokens: {:?}",
                toks.len(), line;
                Input, Size, Decode));
        }
        let one     = res!(parse_num::<f64>(toks[0], "loadavg.one"));
        let five    = res!(parse_num::<f64>(toks[1], "loadavg.five"));
        let fifteen = res!(parse_num::<f64>(toks[2], "loadavg.fifteen"));
        let (runnable, total) = match toks[3].split_once('/') {
            Some((r, t)) => (
                res!(parse_num::<u64>(r, "loadavg.runnable")),
                res!(parse_num::<u64>(t, "loadavg.total")),
            ),
            None => (0, 0),
        };
        Ok(Self { one, five, fifteen, runnable, total })
    }
}
