//! What the watcher saw, kept where the dashboard can read it.
//!
//! The watcher's beliefs about each peer are its own and stay private to
//! [`crate::srv::watch`]. What it *saw* -- each probe's answer, its health body
//! with the measured `probe_ms` folded in, and the belief that probe left behind
//! -- goes into a bounded ring per peer behind the shared [`Fleet`] handle, which
//! the admin state holds too. The view reads a projection of a judgement already
//! made; it issues no request of its own, and nothing it does reaches back into
//! the watcher.
//!
//! # A body that will not parse
//!
//! The design (2026-09-21) counted a `200` whose body will not parse as a failed
//! probe. The watcher does not, and should not: a peer with no token serves an
//! ordinary page at its URL, and calling that down would page someone about a
//! healthy machine. So liveness keeps its rule, and the *page* marks a peer that
//! has a token and answered without a readable body as stale -- its numbers are
//! no longer current, which is the claim green would otherwise be making.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
    cfg::{
        WatchConfig,
        WatchPeer,
    },
    health::{
        F_SEALED_DBS,
        HealthBody,
    },
    watch::{
        is_cleared,
        is_over,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::ring::RingBuffer;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        RwLock,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

pub const RING_LEN:         usize = 60;     // an hour at the default 60 s round
// A reading older than this many rounds is stale. Two, so one late or lost round
// does not grey a row that the next round would have coloured.
pub const FRESH_ROUNDS:     u64 = 2;

pub fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The watcher's belief about a peer once a probe had been judged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerHealth {
    Up,
    Distressed,
    Down,
}

/// One probe of one peer, as the watcher saw it.
#[derive(Clone, Debug)]
pub struct ProbeSample {
    pub t_secs:     u64,                // unix seconds, when the probe was judged
    pub ok:         bool,               // answered with a 2xx
    pub probe_ms:   u64,                // measured by the watcher
    pub body:       Option<HealthBody>, // parsed, with `probe_ms` folded in
    pub health:     PeerHealth,         // the belief this probe left behind
}

/// A watched peer as the page may show it. The token is reduced to whether there
/// is one, so no path from here can put it on a page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetPeer {
    pub name:       String,
    pub host:       String,
    pub url:        String,
    pub has_token:  bool,
    pub distress:   BTreeMap<String, i64>,
    pub clear:      BTreeMap<String, i64>,
}

impl From<&WatchPeer> for FleetPeer {
    fn from(p: &WatchPeer) -> Self {
        Self {
            name:       p.name.clone(),
            host:       p.host.clone(),
            url:        p.url.clone(),
            has_token:  matches!(&p.token, Some(t) if !t.is_empty()),
            distress:   p.distress.clone(),
            clear:      p.clear.clone(),
        }
    }
}

/// The shared per-peer rings, and what the page needs to judge freshness.
#[derive(Debug)]
pub struct Fleet {
    whoami:         String,             // this host, the page's own row
    started_secs:   u64,
    interval_secs:  u64,                // the round, as the watcher runs it
    fail_threshold: u32,
    peers:          Vec<FleetPeer>,
    // Set by the watcher when its loop starts, so a watch list whose watcher never
    // started -- no alerter, no outbound TLS -- says so rather than showing every
    // peer as never heard from.
    watching:       AtomicBool,
    rings:          RwLock<Vec<RingBuffer<RING_LEN, ProbeSample>>>, // one per peer, in order
}

impl Fleet {
    /// `cfg` is the watch list with its tokens already resolved, or `None` on a
    /// host that watches nobody, which still has its own row to show.
    pub fn new(whoami: String, cfg: Option<&WatchConfig>) -> Self {
        let (peers, interval_secs, fail_threshold) = match cfg {
            Some(c) => (
                c.peers.iter().map(FleetPeer::from).collect::<Vec<_>>(),
                // The watcher's own floor on its round.
                c.interval_secs.max(5),
                c.fail_threshold.max(1),
            ),
            None => {
                let d = WatchConfig::default();
                (Vec::new(), d.interval_secs, d.fail_threshold)
            },
        };
        let rings = (0..peers.len()).map(|_| RingBuffer::default()).collect();
        Self {
            whoami,
            started_secs:   unix_secs(),
            interval_secs,
            fail_threshold,
            peers,
            watching:       AtomicBool::new(false),
            rings:          RwLock::new(rings),
        }
    }

    pub fn new_shared(whoami: String, cfg: Option<&WatchConfig>) -> Arc<Self> {
        Arc::new(Self::new(whoami, cfg))
    }

    pub fn whoami(&self) -> &str { &self.whoami }
    pub fn started_secs(&self) -> u64 { self.started_secs }
    pub fn interval_secs(&self) -> u64 { self.interval_secs }
    pub fn fail_threshold(&self) -> u32 { self.fail_threshold }
    pub fn peers(&self) -> &[FleetPeer] { &self.peers }

    pub fn set_watching(&self, on: bool) {
        self.watching.store(on, Ordering::Release);
    }

    pub fn is_watching(&self) -> bool {
        self.watching.load(Ordering::Acquire)
    }

    /// Keep one probe of peer `i`, displacing the oldest once the ring is full.
    pub fn record(&self, i: usize, sample: ProbeSample) -> Outcome<()> {
        let mut rings = lock_write!(self.rings, "Recording a probe of peer {}.", i);
        match rings.get_mut(i) {
            Some(ring) => {
                ring.set_and_adv(sample);
                Ok(())
            },
            None => Err(err!(
                "There is no ring for peer {}; the fleet was built for {} peer(s).",
                i, self.peers.len();
                Index, Missing)),
        }
    }

    /// Every peer with its samples, oldest first, copied out so no lock is held
    /// while a page is built from them.
    pub fn snapshot(&self) -> Outcome<Vec<(FleetPeer, Vec<ProbeSample>)>> {
        let rings = lock_read!(self.rings, "Reading the fleet's probe rings.");
        let mut out = Vec::with_capacity(self.peers.len());
        for (i, peer) in self.peers.iter().enumerate() {
            let samples = match rings.get(i) {
                Some(ring) => ring.iter_chrono().cloned().collect(),
                None => Vec::new(),
            };
            out.push((peer.clone(), samples));
        }
        Ok(out)
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WHAT THE PAGE MAY CLAIM                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The word at the head of a row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowState {
    Up,
    Distressed,
    Sealed,         // answering, with databases the seal holds shut
    Down,           // the watcher's own call, the one that alarms
    Stale,          // no current reading, not yet down
    Never,          // no reading since this host's watcher started
}

impl RowState {
    pub fn word(&self) -> &'static str {
        match self {
            Self::Up            => "up",
            Self::Distressed    => "distressed",
            Self::Sealed        => "sealed",
            Self::Down          => "down",
            Self::Stale         => "stale",
            Self::Never         => "never",
        }
    }

    /// May the row's numbers be shown as current and coloured? A stale row keeps
    /// its last numbers, dimmed and grey; the others have none to show.
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Up | Self::Distressed | Self::Sealed)
    }
}

/// What the page may say about one peer, and why when that is not `up`.
///
/// Only the samples decide it: the latest belief the watcher recorded, the age of
/// the last good reading against [`FRESH_ROUNDS`] rounds, and -- for a peer that
/// has a token -- whether the latest answer carried a body that parsed.
pub fn peer_state(
    peer:           &FleetPeer,
    samples:        &[ProbeSample],
    now_secs:       u64,
    interval_secs:  u64,
)
    -> (RowState, String)
{
    let latest = match samples.last() {
        Some(s) => s,
        None => return (RowState::Never, fmt!("not probed yet")),
    };
    if latest.health == PeerHealth::Down {
        return (RowState::Down, fmt!("called down by this host's watcher"));
    }
    let window = interval_secs.saturating_mul(FRESH_ROUNDS);
    if !peer.has_token {
        // Liveness alone: a good reading is any 2xx.
        return match samples.iter().rev().find(|s| s.ok) {
            None => (RowState::Never, fmt!("no answer since this host's watcher started")),
            Some(s) if now_secs.saturating_sub(s.t_secs) > window => (
                RowState::Stale,
                fmt!("no answer for {} s", now_secs.saturating_sub(s.t_secs))),
            Some(_) => (RowState::Up, String::new()),
        };
    }
    let last_read = samples.iter().rev().find(|s| s.body.is_some());
    if latest.ok && latest.body.is_none() {
        return match last_read {
            None => (RowState::Never, fmt!("answers, but its health body does not parse")),
            Some(_) => (RowState::Stale, fmt!("answered without a readable health body")),
        };
    }
    let body = match last_read {
        Some(s) if now_secs.saturating_sub(s.t_secs) > window => return (
            RowState::Stale,
            fmt!("no reading for {} s", now_secs.saturating_sub(s.t_secs))),
        Some(s) => match &s.body {
            Some(b) => b,
            None => return (RowState::Never, String::new()),
        },
        None => return (RowState::Never, fmt!("no reading since this host's watcher started")),
    };
    if body.get(F_SEALED_DBS).unwrap_or(0) > 0 {
        return (RowState::Sealed, fmt!("databases held shut awaiting an unseal"));
    }
    match latest.health {
        PeerHealth::Distressed => (RowState::Distressed, String::new()),
        _ => (RowState::Up, String::new()),
    }
}

/// A cell's colour, from the same thresholds the alarm uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Plain,  // no threshold, so no claim
    Green,
    Amber,  // above the clear boundary, under distress: an alarm here would not yet clear
    Red,
}

impl Tone {
    pub fn word(&self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Green => "green",
            Self::Amber => "amber",
            Self::Red   => "red",
        }
    }
}

/// Red at or over `distress`, amber over the clear boundary, green at or under it.
/// The clear boundary is `clear` where one is given and `distress` otherwise,
/// exactly as the watcher reads it, so a field with no `clear` has no amber.
pub fn tone(value: i64, distress: Option<i64>, clear: Option<i64>) -> Tone {
    let d = match distress {
        Some(d) => d,
        None => return Tone::Plain,
    };
    if is_over(value, d) {
        return Tone::Red;
    }
    if is_cleared(value, clear.unwrap_or(d)) {
        Tone::Green
    } else {
        Tone::Amber
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn peer(has_token: bool) -> FleetPeer {
        FleetPeer {
            name:       fmt!("jarrah"),
            host:       fmt!("jarrah"),
            url:        fmt!("https://example.test/_steel/health"),
            has_token,
            distress:   BTreeMap::new(),
            clear:      BTreeMap::new(),
        }
    }

    fn body(fields: &[(&str, i64)]) -> HealthBody {
        let mut b = HealthBody::new();
        for (k, v) in fields {
            b.set(k, *v);
        }
        b
    }

    fn sample(t_secs: u64, ok: bool, b: Option<HealthBody>, health: PeerHealth) -> ProbeSample {
        ProbeSample { t_secs, ok, probe_ms: 80, body: b, health }
    }

    fn fleet_of(n: usize) -> Fleet {
        let mut cfg = WatchConfig::default();
        for i in 0..n {
            cfg.peers.push(WatchPeer {
                name:       fmt!("peer{}", i),
                host:       fmt!("peer{}", i),
                url:        fmt!("https://peer{}.test/_steel/health", i),
                plain_ok:   false,
                distress:   BTreeMap::new(),
                clear:      BTreeMap::new(),
                token:      Some(fmt!("secret")),
            });
        }
        Fleet::new(fmt!("karri"), Some(&cfg))
    }

    /// The ring keeps the last sixty probes and hands them back oldest first, however
    /// many times it has wrapped.
    #[test]
    fn a_peer_ring_holds_the_last_sixty_probes_oldest_first() -> Outcome<()> {
        let fleet = fleet_of(2);
        for t in 1..=(RING_LEN as u64 + 10) {
            res!(fleet.record(0, sample(t, true, None, PeerHealth::Up)));
        }
        res!(fleet.record(1, sample(500, true, None, PeerHealth::Up)));
        let snap = res!(fleet.snapshot());
        let times: Vec<u64> = snap[0].1.iter().map(|s| s.t_secs).collect();
        assert_eq!(times.len(), RING_LEN, "the ring must be bounded at {}", RING_LEN);
        assert_eq!(times.first(), Some(&11), "the ten oldest must have been displaced");
        assert_eq!(times.last(), Some(&(RING_LEN as u64 + 10)));
        assert!(times.windows(2).all(|w| w[0] < w[1]), "the samples must read oldest first");
        assert_eq!(snap[1].1.len(), 1, "each peer has a ring of its own");
        assert!(fleet.record(2, sample(1, true, None, PeerHealth::Up)).is_err(),
            "a peer the fleet was not built for is refused, not silently dropped");
        Ok(())
    }

    /// The token is never copied into what the page can reach.
    #[test]
    fn a_fleet_peer_keeps_whether_there_is_a_token_and_not_the_token() {
        let fleet = fleet_of(1);
        assert!(fleet.peers()[0].has_token);
        assert!(!fmt!("{:?}", fleet.peers()).contains("secret"));
    }

    /// A peer that has a token and answers `200` with a body that does not parse is up to the
    /// alarm and stale to the page: its last numbers are no longer current.
    #[test]
    fn an_unparseable_body_from_a_token_peer_reads_stale() {
        let p = peer(true);
        let good = sample(1_000, true, Some(body(&[("mem_pct", 40)])), PeerHealth::Up);
        let garbled = sample(1_060, true, None, PeerHealth::Up);
        let (state, note) = peer_state(&p, &[good.clone(), garbled.clone()], 1_070, 60);
        assert_eq!(state, RowState::Stale, "the latest answer carried no readable body");
        assert!(note.contains("readable"), "the note must say why, got '{}'", note);
        assert!(!state.is_fresh());

        // A peer that has never served a readable body has nothing stale to show.
        let (state, _) = peer_state(&p, &[garbled.clone()], 1_070, 60);
        assert_eq!(state, RowState::Never);

        // The same answer from a peer with no token is ordinary liveness.
        let (state, _) = peer_state(&peer(false), &[garbled], 1_070, 60);
        assert_eq!(state, RowState::Up);

        // And a readable body in the latest answer is fresh again.
        let back = sample(1_120, true, Some(body(&[("mem_pct", 41)])), PeerHealth::Up);
        let (state, _) = peer_state(&p, &[good, back], 1_130, 60);
        assert_eq!(state, RowState::Up);
    }

    /// Older than two rounds without a good reading is stale, before the watcher has counted
    /// enough failures to call the peer down; once it has, the row says down.
    #[test]
    fn freshness_follows_the_age_of_the_last_good_reading() {
        let p = peer(true);
        let good = sample(1_000, true, Some(body(&[("mem_pct", 40)])), PeerHealth::Up);
        let miss = sample(1_120, false, None, PeerHealth::Up);
        assert_eq!(peer_state(&p, &[good.clone()], 1_110, 60).0, RowState::Up);
        assert_eq!(peer_state(&p, &[good.clone(), miss.clone()], 1_121, 60).0, RowState::Stale);
        let dead = sample(1_180, false, None, PeerHealth::Down);
        assert_eq!(peer_state(&p, &[good, miss, dead], 1_181, 60).0, RowState::Down);
        assert_eq!(peer_state(&p, &[], 1_181, 60).0, RowState::Never);
    }

    /// Sealed is read from the databases the seal holds shut, so a box with no database -- which
    /// runs sealed for ever by design -- reads up, not sealed.
    #[test]
    fn sealed_means_databases_held_shut() {
        let p = peer(true);
        let no_dbs = sample(1_000, true, Some(body(&[("sealed", 1), ("sealed_dbs", 0)])),
            PeerHealth::Up);
        assert_eq!(peer_state(&p, &[no_dbs], 1_001, 60).0, RowState::Up);
        let shut = sample(1_000, true, Some(body(&[("sealed", 1), ("sealed_dbs", 2)])),
            PeerHealth::Up);
        assert_eq!(peer_state(&p, &[shut], 1_001, 60).0, RowState::Sealed);
        let hot = sample(1_000, true, Some(body(&[("mem_pct", 95)])), PeerHealth::Distressed);
        assert_eq!(peer_state(&p, &[hot], 1_001, 60).0, RowState::Distressed);
    }

    /// The colours are the alarm's own comparisons: red at the distress value, amber in the
    /// dead-band, green at or under clear, and nothing where no threshold was set.
    #[test]
    fn tones_follow_the_alarm_thresholds() {
        assert_eq!(tone(90, Some(90), Some(75)), Tone::Red, "at the threshold is over it");
        assert_eq!(tone(89, Some(90), Some(75)), Tone::Amber);
        assert_eq!(tone(76, Some(90), Some(75)), Tone::Amber);
        assert_eq!(tone(75, Some(90), Some(75)), Tone::Green, "at clear is cleared");
        assert_eq!(tone(89, Some(90), None), Tone::Green, "no clear map, no dead-band");
        assert_eq!(tone(99, None, Some(75)), Tone::Plain, "a clear without a distress is no claim");
    }
}
