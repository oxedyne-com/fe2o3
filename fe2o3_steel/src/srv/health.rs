//! The token-gated health body and the counters that feed it.
//!
//! Each Steel serves a small health body at a fixed path (see
//! [`crate::srv::cfg::ServerConfig::health_path`]): a flat map of integers read
//! from the host sampler's last sample, the address guard's live counts, and the
//! rolling admission counters below. It is the surface a peer [`Watcher`] reads
//! to tell *distress* -- a box that is answering but unwell -- from plain
//! liveness, which a `200` alone cannot show.
//!
//! # Why integers, and why a shared format
//!
//! The body is emitted here and parsed by the watcher there, both in this crate,
//! so the format is settled by agreement rather than by a schema: a flat JSON
//! object whose every value is an integer. Percentages are whole per cent; the
//! one-minute load average is carried times a hundred (a `load1` of `250` means
//! `2.50`), so a fractional load survives the integer contract without a float on
//! the wire.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::admin::host_sampler::HealthHostMetrics;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    sync::{
        Arc,
        Mutex,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

// The health-body field names, named once so the emitter, the parser and a
// watcher's threshold map all agree on the spelling.
pub const F_MEM_PCT:        &str = "mem_pct";
pub const F_SWAP_PCT:       &str = "swap_pct";
pub const F_DISK_IOPS:      &str = "disk_iops";
pub const F_LOAD1:          &str = "load1";        // 1-minute load average x100
pub const F_CONNS:          &str = "conns";
pub const F_R429_1M:        &str = "r429_1m";
pub const F_DROPPED_1M:     &str = "dropped_1m";
pub const F_GUARD_SELFTEST: &str = "guard_selftest";
pub const F_UPTIME_S:       &str = "uptime_s";
pub const F_SEALED:         &str = "sealed";

// How long a rolling counter keeps its per-second buckets. Two minutes so a
// one-minute query is always fully covered without an off-by-one at the boundary.
const ROLL_KEEP_SECS: u64 = 120;

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A count of events over a recent sliding window.
///
/// One bucket per unix second, trimmed to [`ROLL_KEEP_SECS`]. Increments take a
/// short mutex; the events counted -- a `429` emitted, a connection dropped at
/// admission -- are rare in normal operation and hot only under attack, so the
/// lock is not on any ordinary request's path. Cheaply cloneable via `Arc`, so
/// the increment site and the health reader share one counter.
#[derive(Debug)]
pub struct RollingCounter {
    buckets: Mutex<VecDeque<(u64, u64)>>,   // (unix second, count), newest last
}

impl RollingCounter {
    pub fn new() -> Self {
        Self { buckets: Mutex::new(VecDeque::new()) }
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn incr(&self) {
        self.add(1);
    }

    pub fn add(&self, n: u64) {
        let now = unix_secs();
        // A counter is not worth poisoning a process over: recover the buckets
        // from a poisoned lock rather than propagate, since a lost count is a far
        // smaller harm than a health route that stops answering.
        let mut b = match self.buckets.lock() {
            Ok(g)         => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match b.back_mut() {
            Some((sec, cnt)) if *sec == now => *cnt = cnt.saturating_add(n),
            _ => b.push_back((now, n)),
        }
        while let Some((sec, _)) = b.front() {
            if now.saturating_sub(*sec) > ROLL_KEEP_SECS {
                b.pop_front();
            } else {
                break;
            }
        }
    }

    /// Events counted in the last `window_secs` seconds.
    pub fn last(&self, window_secs: u64) -> u64 {
        let cutoff = unix_secs().saturating_sub(window_secs);
        let b = match self.buckets.lock() {
            Ok(g)         => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        b.iter().filter(|(sec, _)| *sec >= cutoff).map(|(_, cnt)| cnt).sum()
    }
}

impl Default for RollingCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// A flat map of integer health fields, emitted as JSON and parsed back the same.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthBody {
    pub fields: BTreeMap<String, i64>,
}

impl HealthBody {
    pub fn new() -> Self {
        Self { fields: BTreeMap::new() }
    }

    pub fn set(&mut self, key: &str, value: i64) {
        self.fields.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<i64> {
        self.fields.get(key).copied()
    }

    /// A flat JSON object of integers, keys in sorted order.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        for (i, (k, v)) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&fmt!("\"{}\":{}", k, v));
        }
        out.push('}');
        out
    }

    /// Assemble the health body from the pieces already kept elsewhere: the
    /// host sampler's last reading, the guard's live count and self-test, the
    /// rolling admission counters, and the two liveness scalars. Every value is
    /// an integer; a boolean rides as `0` or `1`.
    ///
    /// `host` is the sampler's reduced figures, `None` while the ring is still
    /// empty (the level fields are then simply absent rather than zeroed, so a
    /// watcher does not read a not-yet-sampled box as one at 0% memory).
    pub fn assemble(
        host:           Option<HealthHostMetrics>,
        conns:          usize,
        r429_1m:        u64,
        dropped_1m:     u64,
        guard_selftest: bool,
        uptime_s:       u64,
        sealed:         bool,
    )
        -> Self
    {
        let mut b = Self::new();
        if let Some(h) = host {
            b.set(F_MEM_PCT,   h.mem_pct);
            b.set(F_SWAP_PCT,  h.swap_pct);
            b.set(F_DISK_IOPS, h.disk_iops);
            b.set(F_LOAD1,     h.load1);
        }
        b.set(F_CONNS,          conns as i64);
        b.set(F_R429_1M,        r429_1m as i64);
        b.set(F_DROPPED_1M,     dropped_1m as i64);
        b.set(F_GUARD_SELFTEST, if guard_selftest { 1 } else { 0 });
        b.set(F_UPTIME_S,       uptime_s as i64);
        b.set(F_SEALED,         if sealed { 1 } else { 0 });
        b
    }

    /// Parse the flat integer object this crate emits. Deliberately narrow: it
    /// reads `{"key":int,...}` and nothing nested, because that is the whole of
    /// the format both ends agreed on, and a lenient parser would only hide a
    /// drift between them.
    pub fn parse(s: &str) -> Outcome<Self> {
        let trimmed = s.trim();
        let inner = res!(trimmed.strip_prefix('{')
            .and_then(|r| r.strip_suffix('}'))
            .ok_or_else(|| err!(
                "A health body must be a JSON object; got {:?}.", trimmed;
                Invalid, Input, Decode)));
        let mut out = Self::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (raw_key, raw_val) = res!(part.split_once(':').ok_or_else(|| err!(
                "A health-body field {:?} is not 'key:value'.", part;
                Invalid, Input, Decode)));
            let key = raw_key.trim().trim_matches('"').to_string();
            let val: i64 = res!(raw_val.trim().parse().map_err(|_| err!(
                "The health-body field '{}' has a non-integer value {:?}.",
                key, raw_val.trim();
                Invalid, Input, Decode)));
            out.fields.insert(key, val);
        }
        Ok(out)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips() {
        let mut b = HealthBody::new();
        b.set(F_MEM_PCT, 94);
        b.set(F_SWAP_PCT, 71);
        b.set(F_SEALED, 0);
        b.set(F_LOAD1, 250);
        let json = b.to_json();
        let back = HealthBody::parse(&json).expect("parse");
        assert_eq!(b, back);
        assert_eq!(back.get(F_MEM_PCT), Some(94));
        assert_eq!(back.get(F_LOAD1), Some(250));
    }

    #[test]
    fn parse_tolerates_whitespace() {
        let b = HealthBody::parse("{ \"mem_pct\" : 40 , \"conns\": 3 }").expect("parse");
        assert_eq!(b.get(F_MEM_PCT), Some(40));
        assert_eq!(b.get(F_CONNS), Some(3));
    }

    #[test]
    fn parse_rejects_a_non_object() {
        assert!(HealthBody::parse("not json").is_err());
        assert!(HealthBody::parse("{\"a\":notint}").is_err());
    }

    #[test]
    fn rolling_counter_sums_the_window() {
        let c = RollingCounter::new();
        c.incr();
        c.incr();
        c.add(5);
        // All within the same second, so a one-minute window sees all seven.
        assert_eq!(c.last(60), 7);
        // A zero-width window still sees the current second's bucket.
        assert_eq!(c.last(0), 7);
    }
}
