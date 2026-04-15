//! Per-interface network counters from `/proc/net/dev`.

use crate::parse::{
    parse_num,
    read_to_string,
    tokens,
};

use oxedyne_fe2o3_core::prelude::*;

/// Cumulative byte and packet counters for one interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetLine {
    /// Interface name, e.g. `lo`, `eth0`, `wlp4s0`.
    pub name:      String,
    /// Cumulative bytes received.
    pub rx_bytes:  u64,
    /// Cumulative packets received.
    pub rx_packets:u64,
    /// Cumulative receive errors.
    pub rx_errors: u64,
    /// Cumulative receive packets dropped.
    pub rx_drops:  u64,
    /// Cumulative bytes transmitted.
    pub tx_bytes:  u64,
    /// Cumulative packets transmitted.
    pub tx_packets:u64,
    /// Cumulative transmit errors.
    pub tx_errors: u64,
    /// Cumulative transmit packets dropped.
    pub tx_drops:  u64,
}

/// Collection of per-interface counters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetStats {
    pub interfaces: Vec<NetLine>,
}

impl NetStats {
    /// Read `/proc/net/dev`.
    pub fn sample() -> Outcome<Self> {
        let content = res!(read_to_string("/proc/net/dev"));
        Self::from_net_dev(&content)
    }

    /// Parse a `/proc/net/dev` body. The first two lines are
    /// header rows describing the column groups; we skip them.
    pub fn from_net_dev(content: &str) -> Outcome<Self> {
        let mut out = Self::default();
        for line in content.lines().skip(2) {
            let (name_part, rest) = match line.split_once(':') {
                Some(t) => t,
                None    => continue,
            };
            let toks = tokens(rest);
            // 16 columns: rx (bytes packets errs drop fifo frame
            // compressed multicast) then tx (same layout).
            if toks.len() < 16 {
                continue;
            }
            out.interfaces.push(NetLine {
                name:       name_part.trim().to_string(),
                rx_bytes:   res!(parse_num::<u64>(toks[0],  "net.rx_bytes")),
                rx_packets: res!(parse_num::<u64>(toks[1],  "net.rx_packets")),
                rx_errors:  res!(parse_num::<u64>(toks[2],  "net.rx_errors")),
                rx_drops:   res!(parse_num::<u64>(toks[3],  "net.rx_drops")),
                tx_bytes:   res!(parse_num::<u64>(toks[8],  "net.tx_bytes")),
                tx_packets: res!(parse_num::<u64>(toks[9],  "net.tx_packets")),
                tx_errors:  res!(parse_num::<u64>(toks[10], "net.tx_errors")),
                tx_drops:   res!(parse_num::<u64>(toks[11], "net.tx_drops")),
            });
        }
        Ok(out)
    }

    /// Per-interface throughput between two samples.
    pub fn deltas(&self, prev: &Self, elapsed_s: f64) -> Vec<NetDelta> {
        let mut out = Vec::with_capacity(self.interfaces.len());
        for iface in &self.interfaces {
            let p = prev.interfaces.iter().find(|i| i.name == iface.name);
            let p = match p {
                Some(p) => p,
                None    => continue,
            };
            let rx = iface.rx_bytes.saturating_sub(p.rx_bytes);
            let tx = iface.tx_bytes.saturating_sub(p.tx_bytes);
            let (rxps, txps) = if elapsed_s > 0.0 {
                (rx as f64 / elapsed_s, tx as f64 / elapsed_s)
            } else {
                (0.0, 0.0)
            };
            out.push(NetDelta {
                name:    iface.name.clone(),
                rx_bps:  rxps,
                tx_bps:  txps,
            });
        }
        out
    }
}

/// Per-interface throughput between two samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetDelta {
    pub name:   String,
    /// Bytes per second received over the interval.
    pub rx_bps: f64,
    /// Bytes per second transmitted over the interval.
    pub tx_bps: f64,
}
