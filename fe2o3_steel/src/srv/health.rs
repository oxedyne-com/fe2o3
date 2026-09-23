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
use oxedyne_fe2o3_sys::resident::Resident;

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

// ── Fleet fields ─────────────────────────────────────────────────────────────
//
// The fields the Fleet view added (2026-09-23), beside the originals above.
pub const F_DISK_PCT:       &str = "disk_pct";      // the app root's filesystem, as `df` puts it
pub const F_SEALED_DBS:     &str = "sealed_dbs";    // databases the seal holds shut
pub const F_MAIL_DOWN:      &str = "mail_down";     // mail listeners asked for, not bound
// Written by the watcher into the body it read, never served by a peer: the time
// the probe took, measured at the watching end.
pub const F_PROBE_MS:       &str = "probe_ms";

// The per-process figures for the services named in `health_residents` are a list
// in spirit, and the format has no lists. Each resident is therefore flattened
// into keys of the form `res.<name>.<figure>` -- `res.steel.rss_kb`,
// `res.daimond_gateway.cap_pct` -- so a resident is an ordinary field to the
// parser and an ordinary threshold to a watcher's `distress` map. A name is
// letters, digits, `_`, `-` and `.`, so none can break the object, and the split
// back into name and figure is taken at the last dot. `procs` is always present,
// so a service that is not running reads as zero processes rather than as a
// service nobody asked about; `rss_kb` is in the kernel's kB, which are KiB;
// `cap_kb` and `cap_pct` appear only when the service's cgroup sets a limit.
pub const RES_PREFIX:       &str = "res.";
pub const RES_PROCS:        &str = "procs";
pub const RES_RSS_KB:       &str = "rss_kb";
pub const RES_CAP_KB:       &str = "cap_kb";
pub const RES_CAP_PCT:      &str = "cap_pct";
pub const RES_NAME_MAX:     usize = 64;

/// Can this name ride in a flattened resident key?
pub fn is_resident_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= RES_NAME_MAX
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

pub fn resident_key(name: &str, figure: &str) -> String {
    fmt!("{}{}.{}", RES_PREFIX, name, figure)
}

/// The resident name and figure a flattened key carries, or `None` for any other
/// key. The split is at the last dot, since a name may hold dots and a figure
/// never does.
pub fn split_resident_key(key: &str) -> Option<(&str, &str)> {
    let rest = ok!(key.strip_prefix(RES_PREFIX));
    let (name, figure) = ok!(rest.rsplit_once('.'));
    if name.is_empty() || figure.is_empty() {
        return None;
    }
    Some((name, figure))
}

/// One resident's figures as a body carries them, keyed by figure.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResidentFigures {
    pub name:       String,
    pub figures:    BTreeMap<String, i64>,
}

impl ResidentFigures {
    pub fn get(&self, figure: &str) -> Option<i64> {
        self.figures.get(figure).copied()
    }
}

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

impl HealthBody {
    /// Flatten one resident into its `res.<name>.*` keys. A name that cannot
    /// ride in a key is skipped: configuration refuses one at start-up, so
    /// reaching here with one is a caller bypassing that check.
    pub fn set_resident(&mut self, r: &Resident) {
        if !is_resident_name(&r.name) {
            return;
        }
        let clamp = |v: u64| -> i64 { v.min(i64::MAX as u64) as i64 };
        self.set(&resident_key(&r.name, RES_PROCS),  r.procs as i64);
        self.set(&resident_key(&r.name, RES_RSS_KB), clamp(r.rss_kib));
        if let Some(cap) = r.cap_kib {
            self.set(&resident_key(&r.name, RES_CAP_KB), clamp(cap));
        }
        if let Some(pct) = r.cap_pct() {
            self.set(&resident_key(&r.name, RES_CAP_PCT), clamp(pct));
        }
    }

    /// The residents a body carries, gathered back from their flattened keys, in
    /// name order.
    pub fn residents(&self) -> Vec<ResidentFigures> {
        let mut by_name: BTreeMap<&str, BTreeMap<String, i64>> = BTreeMap::new();
        for (k, v) in &self.fields {
            if let Some((name, figure)) = split_resident_key(k) {
                by_name.entry(name).or_default().insert(figure.to_string(), *v);
            }
        }
        by_name.into_iter()
            .map(|(name, figures)| ResidentFigures { name: name.to_string(), figures })
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Residents flatten into ordinary integer keys, survive the wire, and gather
    /// back into the same figures -- including a name that holds a dot, and a
    /// service that is not running, which must still say so.
    #[test]
    fn residents_flatten_and_gather_back_across_the_wire() -> Outcome<()> {
        let running = Resident {
            name:       fmt!("daimond_gateway"),
            procs:      1,
            rss_kib:    412_000,
            cap_kib:    Some(524_288),
        };
        let uncapped = Resident {
            name:       fmt!("python3.11"),
            procs:      2,
            rss_kib:    90_000,
            cap_kib:    None,
        };
        let absent = Resident { name: fmt!("steel"), ..Resident::default() };
        let mut body = HealthBody::assemble(None, 3, 0, 0, true, 60, false);
        for r in [&running, &uncapped, &absent] {
            body.set_resident(r);
        }

        let json = body.to_json();
        assert!(json.contains("\"res.daimond_gateway.rss_kb\":412000"), "got {}", json);
        assert!(json.contains("\"res.daimond_gateway.cap_pct\":78"), "got {}", json);
        let back = res!(HealthBody::parse(&json));
        assert_eq!(back, body, "the flattened body must round-trip exactly");

        let gathered = back.residents();
        let names: Vec<&str> = gathered.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["daimond_gateway", "python3.11", "steel"]);
        assert_eq!(gathered[0].get(RES_CAP_KB), Some(524_288));
        assert_eq!(gathered[0].get(RES_CAP_PCT), Some(78));
        assert_eq!(gathered[1].get(RES_RSS_KB), Some(90_000),
            "a dot in the name must not split it: the split is at the last dot");
        assert_eq!(gathered[1].get(RES_CAP_PCT), None, "no cap, no percentage");
        assert_eq!(gathered[2].get(RES_PROCS), Some(0),
            "a service that is not running still reports, as zero processes");
        Ok(())
    }

    /// A resident key is recognised only in its full shape, and a name that could
    /// break the object is refused before it can become one.
    #[test]
    fn resident_keys_split_only_in_their_own_shape() {
        assert_eq!(split_resident_key("res.steel.rss_kb"), Some(("steel", "rss_kb")));
        assert_eq!(split_resident_key("res.a.b.cap_pct"), Some(("a.b", "cap_pct")));
        assert_eq!(split_resident_key("mem_pct"), None);
        assert_eq!(split_resident_key("res.steel"), None);
        assert_eq!(split_resident_key("res..rss_kb"), None);
        assert!(is_resident_name("daimond_gateway"));
        assert!(!is_resident_name(""));
        assert!(!is_resident_name("a\"b"), "a quote would end the JSON key");
        assert!(!is_resident_name("a:b"), "a colon would split the field");
        assert!(!is_resident_name("a,b"), "a comma would split the object");
        let mut b = HealthBody::new();
        b.set_resident(&Resident { name: fmt!("bad,name"), procs: 1, ..Resident::default() });
        assert!(b.fields.is_empty(), "an unsafe name must never reach the body");
    }

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
