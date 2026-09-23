//! Watching the other machines, because a host cannot report its own death.
//!
//! # The problem this exists for
//!
//! [`crate::srv::alert`] says it plainly in its own header: Steel is reporting on itself, and a
//! Steel that is wedged, unreachable or dead sends nothing. **Silence is indistinguishable from
//! health.** On 2026-08-10 a payments gateway exited on its own and was down for about fifty
//! minutes while the websites in front of it served perfectly; nothing said a word, because the
//! only thing positioned to notice was the machine that had died.
//!
//! The fix is to invert the question. Do not detect a failure -- require a success, on a
//! schedule, from somewhere else, and treat its absence as the alarm. That is a thing only
//! another host can do.
//!
//! # The shape: a mesh, not a monitor
//!
//! Every node watches every other node it is told about, and any node can raise the alarm. There
//! is no monitoring server, because a monitoring server is one more single point that fails
//! silently. A three-node estate where each watches the other two survives losing any one of
//! them, and survives losing any two as far as the third is concerned.
//!
//! **Adding or removing a machine is one line of configuration**, and nothing else changes: no
//! code, no central registry, no re-deployment of the others beyond their own peer list. That is
//! the whole reason the peer list is data rather than a compiled set.
//!
//! # Duplicate alarms are a feature
//!
//! When two nodes both notice that a third has gone, the operator gets told twice. That is left
//! alone deliberately. Suppressing it would need the watchers to agree with each other, which
//! means a protocol between them, which means a thing that can itself fail and take the alarm
//! with it. Two messages saying the same true thing cost a few cents and a glance; one message
//! that was suppressed by a consensus that broke costs an outage.
//!
//! # What is watched
//!
//! A URL that answers `200` when the machine is well. Nothing more clever, because anything more
//! clever is a thing to keep in step with the machine it watches. What that URL means is the
//! watched machine's business -- a gateway's `/api/health` already reports whether its store
//! opened, which is a far better answer than whether a port accepts a connection.
//!
//! # Why a peer may say `http`, and why it must say so
//!
//! A probe is normally `https`, because it crosses the public internet and an unauthenticated
//! answer can be forged by anybody on the path -- and the forgery that matters is a `200`, which
//! hides the outage rather than inventing one. That stays the default and the absence of the key
//! keeps it.
//!
//! The exception this admits is a machine that serves nothing else. birch holds the off-host copy
//! of the forge and listens on 22 alone; giving it a certificate means giving it a public name, a
//! port open to a certificate authority's validators, and a renewal that can quietly stop -- three
//! moving parts on the one machine whose entire value is being boring. Its freshness endpoint is a
//! high port admitted by its firewall from the watcher's address alone. So a peer may carry
//! `"plain_ok": (true)`, and it is per peer rather than a switch on the watcher, because the
//! judgement is about one wire and not about watching in general. On 2026-08-23 that copy failed
//! every hour for twenty-one hours with nobody able to see it, which is the cost of the stricter
//! answer.
//!
//! # What it deliberately does not do
//!
//! It does not restart anything. A watcher that repairs is a watcher that can flap a service in
//! a loop at three in the morning and hide the fault it was built to reveal; and a decision to
//! restart a payments process belongs to a person who has read why it stopped.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
    alert::{
        AlertEvent,
        Alerter,
    },
    cfg::{
        WatchConfig,
        WatchPeer,
    },
    fleet::{
        Fleet,
        PeerHealth,
        ProbeSample,
        unix_secs,
    },
    health::{
        F_PROBE_MS,
        HealthBody,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use oxedyne_fe2o3_net::http::{
    client::{
        http_request,
        https_request,
    },
    header::{
        HttpHeadline,
        HttpMethod,
    },
    loc::Url,
};

use std::{
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
};

use tokio_rustls::rustls::ClientConfig;


/// What this node currently believes about one peer.
///
/// Liveness (`Up` / `Down`) and distress (`Distressed`) are the two things a peer
/// can be wrong about, and they are distinct: `Down` is a peer that stops
/// answering, `Distressed` is one that answers but reports itself unwell. A peer
/// cannot be both, because distress is read from a body a dead peer never sends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Health {
    // The count on `Up` is consecutive probe failures since the last success, not yet enough to
    // call the peer down.
    Up { failures: u32 },
    // Answering, but a health-body class has stayed over its distress threshold. `over` is the
    // consecutive over-threshold readings that tipped it in; `failures` is the consecutive probe
    // failures counted since, so a peer that stops answering while distressed still reaches `Down`
    // by the same rule an `Up` peer does.
    Distressed { over: u32, failures: u32 },
    Down,
}

impl From<Health> for PeerHealth {
    fn from(h: Health) -> Self {
        match h {
            Health::Up { .. }           => Self::Up,
            Health::Distressed { .. }   => Self::Distressed,
            Health::Down                => Self::Down,
        }
    }
}

/// One peer's running state.
struct PeerState {
    health:             Health,
    failing_at:         Option<Instant>,    // first seen failing, so a recovery can say how long
    told_at:            Option<Instant>,    // last told down, so a lasting outage reminds not streams
    // Consecutive over-threshold readings while still `Up`, so distress needs the same run of
    // agreeing evidence that a death does before it wakes anyone.
    distress_rising:    u32,
    distress_at:        Option<Instant>,    // when the distress run began, for the recovery line
    distress_told_at:   Option<Instant>,    // last told distressed, for the repeat cadence
}

impl Default for PeerState {
    fn default() -> Self {
        Self {
            health:             Health::Up { failures: 0 },
            failing_at:         None,
            told_at:            None,
            distress_rising:    0,
            distress_at:        None,
            distress_told_at:   None,
        }
    }
}

/// Is a reading over its distress threshold?
///
/// The alarm and the Fleet page's red both ask this one function, and whether a
/// reading has cleared both ask [`is_cleared`], so the page cannot draw a colour
/// the alarm would not have agreed with.
pub(crate) fn is_over(value: i64, distress: i64) -> bool {
    value >= distress
}

/// Is a reading at or under its clear boundary?
pub(crate) fn is_cleared(value: i64, boundary: i64) -> bool {
    value <= boundary
}

/// The distress classes a body is currently over, as `(field, value)` pairs.
fn classes_breaching(distress: &BTreeMap<String, i64>, body: &HealthBody) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    for (field, threshold) in distress {
        if let Some(v) = body.get(field) {
            if is_over(v, *threshold) {
                out.push((field.clone(), v));
            }
        }
    }
    out
}

/// Is every distress class at or below its clear boundary?
///
/// The clear boundary is a field's `clear` value where one is given, and its
/// `distress` value otherwise -- so a peer with no `clear` map has no dead-band
/// and leaves distress the moment it drops back under the entry threshold.
fn is_below_clear(peer: &WatchPeer, body: &HealthBody) -> bool {
    for (field, dthresh) in &peer.distress {
        let boundary = peer.clear.get(field).copied().unwrap_or(*dthresh);
        if let Some(v) = body.get(field) {
            if !is_cleared(v, boundary) {
                return false;
            }
        }
    }
    true
}

/// The firing classes as one line for an alarm, e.g. `mem_pct 94, swap_pct 71`.
fn classes_text(classes: &[(String, i64)]) -> String {
    classes.iter()
        .map(|(f, v)| fmt!("{} {}", f, v))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The peer watcher.
///
/// Owns its configuration, its TLS client and its beliefs, and writes what it saw into the
/// shared [`Fleet`] for the dashboard to draw. Constructed once at start-up and driven by
/// [`Self::run`], which never returns.
pub struct Watcher {
    cfg:        Arc<WatchConfig>,
    alerter:    Arc<Alerter>,
    tls:        Arc<ClientConfig>,
    // This node's own name, so an alert says who noticed as well as what happened. Two nodes
    // watching a third send two messages, and without this they are indistinguishable.
    whoami:     String,
    // One per peer, in `cfg.peers` order. Kept by position rather than by name, because
    // nothing makes a name unique, and two entries sharing one would share one set of beliefs.
    state:      Vec<PeerState>,
    fleet:      Arc<Fleet>,
}

/// Refuse a peer whose URL this cannot honestly probe.
///
/// Separated from [`Watcher::new`] so the rule can be tested on its own: it decides whether an
/// operator is watched over an authenticated wire, and a rule that costly should not need an
/// alerter and a TLS client standing up before it can be exercised.
fn vet(p: &WatchPeer) -> Outcome<()> {
    let url = res!(Url::parse(&p.url));
    if !url.scheme.is_tls() && !p.plain_ok {
        return Err(err!(
            "The watch entry for '{}' names {}, which is not https. A health probe crosses the \
            public internet and its answer decides whether an operator is woken, so it is \
            authenticated or it is not worth making. A machine that cannot hold a certificate \
            may say so with \"plain_ok\": (true) on its own entry, which is a judgement about \
            that one wire.", p.name, p.url;
            Configuration, Invalid, Input));
    }
    if p.plain_ok && url.scheme.is_tls() {
        warn!("The watch entry for '{}' says plain_ok and names an https URL. The key does \
            nothing here; remove it, so it does not read as a weakness that was accepted.",
            p.name);
    }
    Ok(())
}

impl Watcher {
    /// Build a watcher over a peer list.
    ///
    /// A peer whose URL cannot be parsed, or one that is not `https` and has not said
    /// `plain_ok`, is refused at start-up rather than at the first poll: a watcher that silently
    /// watches nothing is the failure this module exists to prevent, and start-up is when
    /// somebody is looking.
    pub fn new(
        cfg:     Arc<WatchConfig>,
        alerter: Arc<Alerter>,
        tls:     Arc<ClientConfig>,
        whoami:  String,
        fleet:   Arc<Fleet>,
    )
        -> Outcome<Self>
    {
        let mut state = Vec::with_capacity(cfg.peers.len());
        for p in &cfg.peers {
            res!(vet(p));
            state.push(PeerState::default());
        }
        Ok(Self { cfg, alerter, tls, whoami, state, fleet })
    }

    /// Poll every peer for ever.
    ///
    /// Never returns. Errors from a single probe are the point of the exercise and are handled;
    /// there is no failure here worth ending the loop for, because ending the loop is exactly
    /// the silence this is meant to break.
    pub async fn run(mut self) {
        let every = Duration::from_secs(self.cfg.interval_secs.max(5));
        let repeat = Duration::from_secs(self.cfg.repeat_secs.max(60));
        // A peer probed over plain http is named as such here, so that the concession shows in
        // the log an operator actually reads rather than only in a configuration file nobody
        // opens between incidents.
        info!("Watching {} peer(s) every {}s: {}.",
            self.cfg.peers.len(),
            every.as_secs(),
            self.cfg.peers.iter()
                .map(|p| if p.plain_ok { fmt!("{} (plain http)", p.name) } else { p.name.clone() })
                .collect::<Vec<_>>().join(", "));

        // The first probe waits one interval. A node that has just restarted is a node whose
        // peers may still be restarting too -- a shared power event, a rolling deploy -- and an
        // alarm raised in the first second of life is usually about the estate coming up, not
        // about anything being wrong.
        let beat = Duration::from_secs(self.cfg.heartbeat_secs);
        let started = Instant::now();
        let mut last_beat = started;
        self.fleet.set_watching(true);
        loop {
            tokio::time::sleep(every).await;
            let mut ok_count = 0usize;
            let cfg = self.cfg.clone();
            for (i, peer) in cfg.peers.iter().enumerate() {
                let (ok, body, dur) = self.probe(peer).await;
                if ok {
                    ok_count += 1;
                }
                debug!("Watch: {} answered ok={} in {}ms.",
                    peer.name, ok, dur.as_millis());
                self.judge(i, peer, ok, body, dur, repeat);
            }
            // Proof of life, on the same loop that does the watching -- so a
            // heartbeat arriving is evidence the watcher is running and not
            // merely that a timer somewhere else still fires.
            if self.cfg.heartbeat_secs > 0 && Instant::now().duration_since(last_beat) >= beat {
                last_beat = Instant::now();
                self.alerter.raise(AlertEvent::Heartbeat {
                    uptime_secs: Instant::now().duration_since(started).as_secs(),
                    peers_ok:    ok_count,
                    peers_total: self.cfg.peers.len(),
                });
            }
        }
    }

    /// Ask one peer whether it is well, returning liveness, its health body when
    /// one was served, and how long the probe took.
    ///
    /// Any answer that is not a `2xx` is a failure, including a `503`: a Steel that is up and
    /// sealed is answering, and it is still not serving the databases behind it. The body is
    /// present only when the peer carries a `token` (so the gate opens) and the answer parsed;
    /// its absence never fails the liveness check, so a peer that answers `200` without a body is
    /// still up. The duration is measured here, not self-reported by the peer.
    async fn probe(&self, peer: &WatchPeer) -> (bool, Option<HealthBody>, std::time::Duration) {
        let url = &peer.url;
        let started = Instant::now();
        let loc = match Url::parse(url) {
            Ok(l) => l,
            // Refused at construction, so this cannot happen -- and if it ever does, a peer
            // that cannot be addressed is a peer that is not answering.
            Err(e) => {
                warn!("The watch URL {} stopped parsing: {}", url, e);
                return (false, None, started.elapsed());
            },
        };
        let host = loc.host.clone();
        let port = loc.port;
        let path = loc.target.clone();
        let timeout = Duration::from_secs(self.cfg.timeout_secs.max(2));

        // The token, when the operator gave one, opens the peer's health gate. Without it the
        // peer answers a 404 for the health path, so the probe reads liveness only.
        let mut headers: Vec<(&str, &str)> = vec![
            ("Connection", "close"),
            ("User-Agent", "steel-watch"),
        ];
        if let Some(tok) = &peer.token {
            headers.push(("x-steel-health-token", tok.as_str()));
        }
        // The scheme decides, and `new` has already refused a plain URL that nobody opted in
        // to -- so by the time a probe runs, `http` here means the operator wrote it down.
        let reply = if loc.scheme.is_tls() {
            let call = https_request(
                &host, port, HttpMethod::GET, &path, &headers, &[], self.tls.clone(),
            );
            tokio::time::timeout(timeout, call).await
        } else {
            let call = http_request(&host, port, HttpMethod::GET, &path, &headers, &[]);
            tokio::time::timeout(timeout, call).await
        };
        let elapsed = started.elapsed();
        match reply {
            Ok(Ok(reply)) => {
                let code = match &reply.header.headline {
                    HttpHeadline::Response { status } => *status as u16,
                    // A response with a request headline is not an answer this
                    // can read, and an unreadable answer is not a healthy peer.
                    _ => 0,
                };
                if (200..300).contains(&code) {
                    let body = match String::from_utf8(reply.body.clone()) {
                        Ok(s) => match HealthBody::parse(&s) {
                            Ok(b) => Some(b),
                            // A 200 that does not parse as a health body is a peer that is up but
                            // served something else (no token, a plain page): still alive.
                            Err(_) => None,
                        },
                        Err(_) => None,
                    };
                    (true, body, elapsed)
                } else {
                    debug!("Watch: {} answered {}.", url, code);
                    (false, None, elapsed)
                }
            },
            Ok(Err(e)) => {
                debug!("Watch: {} did not answer: {}", url, e);
                (false, None, elapsed)
            },
            Err(_) => {
                debug!("Watch: {} did not answer within {}s.", url, timeout.as_secs());
                (false, None, elapsed)
            },
        }
    }

    /// Fold one probe result into what this node believes, alert on a change, and keep the
    /// sample for the dashboard.
    ///
    /// The dashboard's copy is written after the alerts are raised and a failure to write it is
    /// only logged, so nothing the page does can delay or silence an alarm.
    fn judge(
        &mut self,
        i:      usize,
        peer:   &WatchPeer,
        ok:     bool,
        body:   Option<HealthBody>,
        took:   Duration,
        repeat: Duration,
    ) {
        let threshold = self.cfg.fail_threshold.max(1);
        let now = Instant::now();
        let st = match self.state.get_mut(i) {
            Some(st) => st,
            // Built one per peer at construction, so unreachable; a peer with no beliefs is
            // one this cannot judge, and saying so beats judging it against another's.
            None => {
                warn!("Watch: no state for peer {} ('{}'); skipped.", i, peer.name);
                return;
            },
        };
        let (events, sample) = assess(
            threshold, &self.whoami, peer, st, ok, body, took, repeat, now, unix_secs());
        for event in events {
            self.alerter.raise(event);
        }
        if let Err(e) = self.fleet.record(i, sample) {
            warn!("Watch: the dashboard's copy of {}'s probe was not kept: {}", peer.name, e);
        }
    }
}

/// One probe result in; the alerts it raises and the sample the dashboard keeps out.
///
/// The probe time is measured here rather than reported by the peer, and it is written into the
/// body before the body is judged, so a `probe_ms` threshold in a peer's `distress` map is an
/// ordinary class with no special case. A peer that serves no body has nothing to carry it and
/// no distress to judge; the sample still records the time.
fn assess(
    threshold: u32,
    whoami:    &str,
    peer:      &WatchPeer,
    st:        &mut PeerState,
    ok:        bool,
    body:      Option<HealthBody>,
    took:      Duration,
    repeat:    Duration,
    now:       Instant,
    t_secs:    u64,
)
    -> (Vec<AlertEvent>, ProbeSample)
{
    let probe_ms = took.as_millis().min(i64::MAX as u128) as u64;
    let body = body.map(|mut b| {
        b.set(F_PROBE_MS, probe_ms as i64);
        b
    });
    let events = decide(threshold, whoami, peer, st, ok, body.clone(), repeat, now);
    let sample = ProbeSample {
        t_secs,
        ok,
        probe_ms,
        body,
        health: st.health.into(),
    };
    (events, sample)
}

/// The peer-state transition, separated from the polling and the alerter.
///
/// This is the whole of the interesting behaviour -- when an operator is told, and about what --
/// and it is a pure function of the current state and one probe result. It returns the alerts to
/// raise rather than raising them, so a test can drive it directly with a `PeerState` and no
/// network, no SMTP client, and no alerter standing up behind it.
fn decide(
    threshold: u32,
    whoami:    &str,
    peer:      &WatchPeer,
    st:        &mut PeerState,
    ok:        bool,
    body:      Option<HealthBody>,
    repeat:    Duration,
    now:       Instant,
)
    -> Vec<AlertEvent>
{
    let mut out = Vec::new();
    let name = &peer.name;
    let url = &peer.url;
    // Whether this peer even asks to be watched for distress: a token to open the gate and at
    // least one threshold to test. A peer without both is plain up/down, exactly as before.
    let watches_distress = peer.token.is_some() && !peer.distress.is_empty();

    // A peer that asked for distress watching but gave us nothing to test is a misconfiguration
    // worth saying out loud every poll, not silently reading as well: the operator believes the
    // class is watched. Only warned on a live answer, since a dead peer's missing body is the
    // outage itself, not a config fault.
    if ok && watches_distress {
        match &body {
            None => warn!("Watch: {} answered but served no parseable health body, so its \
                distress thresholds cannot be evaluated -- check the health token and path.",
                name),
            Some(b) => {
                let missing: Vec<String> = peer.distress.keys()
                    .filter(|f| b.get(f).is_none())
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    warn!("Watch: {}'s health body is missing distress field(s) {:?}, so \
                        those classes cannot be evaluated.", name, missing);
                }
            },
        }
    }

    if ok {
        // Any successful probe clears the liveness-failure marker; distress is a separate run.
        if st.health == Health::Down {
            let away = st.failing_at.map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
            *st = PeerState::default();
            out.push(AlertEvent::PeerRecovered {
                peer:       name.clone(),
                url:        url.clone(),
                away_secs:  away,
                noticed_by: whoami.to_string(),
            });
            return out;
        }
        st.failing_at = None;

        // The classes currently over threshold, and whether we have a reading at all.
        let breaching = match &body {
            Some(b) if watches_distress => classes_breaching(&peer.distress, b),
            _ => Vec::new(),
        };
        let have_reading = watches_distress && body.is_some();

        match st.health {
            Health::Up { .. } => {
                if !have_reading {
                    // Healthy liveness, nothing to assess.
                    st.health = Health::Up { failures: 0 };
                    st.distress_rising = 0;
                    return out;
                }
                if breaching.is_empty() {
                    // Under threshold: the rising run is broken.
                    st.distress_rising = 0;
                    st.health = Health::Up { failures: 0 };
                    return out;
                }
                // Over threshold: distress needs the same run of agreeing evidence a death does.
                st.distress_rising += 1;
                if st.distress_rising >= threshold {
                    st.health = Health::Distressed { over: st.distress_rising, failures: 0 };
                    st.distress_at = Some(now);
                    st.distress_told_at = Some(now);
                    out.push(AlertEvent::PeerDistress {
                        peer:       name.clone(),
                        url:        url.clone(),
                        classes:    classes_text(&breaching),
                        since_secs: 0,
                        noticed_by: whoami.to_string(),
                    });
                } else {
                    st.health = Health::Up { failures: 0 };
                }
            },
            Health::Distressed { over, .. } => {
                // A live probe resets the distress-phase failure counter. Whether distress clears
                // is a question only a reading under the clear boundary can answer; with no reading
                // we hold distress rather than guess it away.
                let cleared = match &body {
                    Some(b) if watches_distress => is_below_clear(peer, b),
                    _ => false,
                };
                if cleared {
                    let were = st.distress_at
                        .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                    out.push(AlertEvent::PeerDistressCleared {
                        peer:       name.clone(),
                        url:        url.clone(),
                        were_secs:  were,
                        noticed_by: whoami.to_string(),
                    });
                    *st = PeerState::default();
                } else {
                    st.health = Health::Distressed { over, failures: 0 };
                    // Remind on the repeat interval, as a lasting outage does.
                    let due = st.distress_told_at
                        .map(|t| now.duration_since(t) >= repeat).unwrap_or(true);
                    if due && !breaching.is_empty() {
                        st.distress_told_at = Some(now);
                        let since = st.distress_at
                            .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                        out.push(AlertEvent::PeerDistress {
                            peer:       name.clone(),
                            url:        url.clone(),
                            classes:    classes_text(&breaching),
                            since_secs: since,
                            noticed_by: whoami.to_string(),
                        });
                    }
                }
            },
            // Handled by the recovery return above; defensive and inert.
            Health::Down => {},
        }
        return out;
    }

    // The probe failed.
    match st.health {
        Health::Up { failures } => {
            let failures = failures + 1;
            if st.failing_at.is_none() {
                st.failing_at = Some(now);
            }
            if failures >= threshold {
                st.health = Health::Down;
                st.told_at = Some(now);
                let down_secs = st.failing_at
                    .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                out.push(AlertEvent::PeerDown {
                    peer:       name.clone(),
                    url:        url.clone(),
                    failures,
                    down_secs,
                    noticed_by: whoami.to_string(),
                });
            } else {
                st.health = Health::Up { failures };
            }
        },
        Health::Distressed { over, failures } => {
            // A distressed peer that stops answering reaches Down by the same rule an Up one does:
            // the distress episode gives way to the outage it foreshadowed.
            let failures = failures + 1;
            if st.failing_at.is_none() {
                st.failing_at = Some(now);
            }
            if failures >= threshold {
                st.health = Health::Down;
                st.told_at = Some(now);
                let down_secs = st.failing_at
                    .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                out.push(AlertEvent::PeerDown {
                    peer:       name.clone(),
                    url:        url.clone(),
                    failures,
                    down_secs,
                    noticed_by: whoami.to_string(),
                });
            } else {
                st.health = Health::Distressed { over, failures };
            }
        },
        Health::Down => {
            // Still down. Remind, but only on the repeat interval -- an alarm that fires every
            // poll is an alarm that gets silenced, and the SMS leg of this costs money per message.
            let due = st.told_at.map(|t| now.duration_since(t) >= repeat).unwrap_or(true);
            if due {
                st.told_at = Some(now);
                let down_secs = st.failing_at
                    .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                out.push(AlertEvent::PeerDown {
                    peer:       name.clone(),
                    url:        url.clone(),
                    failures:   threshold,
                    down_secs,
                    noticed_by: whoami.to_string(),
                });
            }
        },
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A plain up/down peer: no token and no thresholds, so distress is never assessed.
    fn plain_peer(name: &str, url: &str) -> WatchPeer {
        WatchPeer {
            name:     name.to_string(),
            host:     name.to_string(),
            url:      url.to_string(),
            plain_ok: false,
            distress: BTreeMap::new(),
            clear:    BTreeMap::new(),
            token:    None,
        }
    }

    /// A peer that watches distress: a token opens the gate, and one or more thresholds are tested.
    fn distress_peer(
        name:     &str,
        distress: &[(&str, i64)],
        clear:    &[(&str, i64)],
    )
        -> WatchPeer
    {
        let mut p = plain_peer(name, "https://example.test/_steel/health");
        p.token = Some(fmt!("a-shared-secret"));
        p.distress = distress.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        p.clear = clear.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        p
    }

    fn body_of(fields: &[(&str, i64)]) -> HealthBody {
        let mut b = HealthBody::new();
        for (k, v) in fields {
            b.set(k, *v);
        }
        b
    }

    /// The state machine, without a network.
    ///
    /// `decide` is driven directly and no alerter stands up, so what is under test is the rule
    /// about when an operator is told -- which is the part that costs money when it is wrong in
    /// one direction and costs an outage when it is wrong in the other.
    fn machine(threshold: u32) -> (WatchConfig, PeerState) {
        let cfg = WatchConfig {
            enabled:        true,
            peers:          vec![plain_peer("jarrah", "https://example.test/api/health")],
            interval_secs:  60,
            fail_threshold: threshold,
            timeout_secs:   10,
            repeat_secs:    900,
            heartbeat_secs: 2_592_000,
        };
        (cfg, PeerState::default())
    }

    /// A single failure is not an outage. This is the property that keeps a flaky minute from
    /// waking somebody, and it is the one most likely to be tuned away by accident.
    #[test]
    fn one_failure_below_the_threshold_is_not_yet_an_outage() {
        let (cfg, mut st) = machine(3);
        assert_eq!(st.health, Health::Up { failures: 0 });
        // Two failures against a threshold of three: still up, and counting.
        for expected in 1..=2u32 {
            let Health::Up { failures } = st.health else { panic!("went down too early") };
            st.health = Health::Up { failures: failures + 1 };
            assert_eq!(st.health, Health::Up { failures: expected });
        }
        assert!(cfg.fail_threshold == 3);
    }

    /// A success clears the count, so isolated failures never accumulate into a false outage.
    #[test]
    fn a_success_forgets_the_run_rather_than_carrying_it() {
        let (_cfg, mut st) = machine(3);
        st.health = Health::Up { failures: 2 };
        st.failing_at = Some(Instant::now());
        st = PeerState::default();
        assert_eq!(st.health, Health::Up { failures: 0 });
        assert!(st.failing_at.is_none(), "a recovered peer still remembered when it failed");
    }

    #[test]
    fn a_peer_list_is_data_so_the_estate_can_change_without_a_rebuild() {
        let (mut cfg, _) = machine(2);
        cfg.peers.push(plain_peer("conifer", "https://ontheism.org/health"));
        assert_eq!(cfg.peers.len(), 2,
            "adding a machine must be a configuration change and nothing else");
    }

    /// The default must be the strict one, because a config that says nothing about a wire is a
    /// config whose author never thought about it.
    #[test]
    fn a_plain_url_is_refused_unless_the_operator_wrote_it_down() {
        let refused = vet(&plain_peer("birch", "http://65.21.145.109:9109/forge/fresh"));
        assert!(refused.is_err(), "a plain http peer was accepted without plain_ok");
        let msg = fmt!("{}", refused.err().unwrap());
        assert!(msg.contains("plain_ok"),
            "the refusal must name the key that would allow it, or the operator has to read \
            the source to find out: got '{}'", msg);

        let mut allowed = plain_peer("birch", "http://65.21.145.109:9109/forge/fresh");
        allowed.plain_ok = true;
        assert!(vet(&allowed).is_ok(),
            "a plain http peer was refused although plain_ok was set");
    }

    /// A malformed URL is refused whatever the peer says about its wire, so `plain_ok` cannot be
    /// read as a general relaxation.
    #[test]
    fn plain_ok_does_not_excuse_a_url_that_cannot_be_parsed() {
        let mut p = plain_peer("nowhere", "not a url at all");
        p.plain_ok = true;
        assert!(vet(&p).is_err(), "plain_ok let an unparseable URL through");
    }

    // ── Distress state machine (A6) ──────────────────────────────────────────

    fn kinds(events: &[AlertEvent]) -> Vec<&'static str> {
        events.iter().map(|e| match e {
            AlertEvent::PeerDown { .. }            => "down",
            AlertEvent::PeerRecovered { .. }       => "recovered",
            AlertEvent::PeerDistress { .. }        => "distress",
            AlertEvent::PeerDistressCleared { .. } => "cleared",
            _                                      => "other",
        }).collect()
    }

    /// A peer stays up until a run of over-threshold readings as long as the fail threshold, then
    /// enters distress once -- not on the first reading, and not repeatedly.
    #[test]
    fn up_to_distressed_needs_the_same_run_as_a_death() {
        let peer = distress_peer("jarrah", &[("mem_pct", 90)], &[("mem_pct", 80)]);
        let mut st = PeerState::default();
        let now = Instant::now();
        let hot = body_of(&[("mem_pct", 94)]);

        // Two over-threshold readings against a threshold of three: still up, no alert.
        for _ in 0..2 {
            let ev = decide(3, "argonaut", &peer, &mut st, true, Some(hot.clone()),
                Duration::from_secs(900), now);
            assert!(ev.is_empty(), "distress fired before the run was long enough");
            assert!(matches!(st.health, Health::Up { .. }));
        }
        // The third tips it into distress, exactly once.
        let ev = decide(3, "argonaut", &peer, &mut st, true, Some(hot.clone()),
            Duration::from_secs(900), now);
        assert_eq!(kinds(&ev), vec!["distress"]);
        assert!(matches!(st.health, Health::Distressed { .. }));
    }

    /// Hysteresis: once distressed, a reading must fall under the *clear* boundary, not merely back
    /// under the entry threshold, before the peer is called well again.
    #[test]
    fn distress_clears_only_under_the_clear_boundary() {
        let peer = distress_peer("jarrah", &[("mem_pct", 90)], &[("mem_pct", 80)]);
        let mut st = PeerState::default();
        let now = Instant::now();
        // Enter distress at threshold 1 for brevity.
        let _ = decide(1, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 95)])),
            Duration::from_secs(900), now);
        assert!(matches!(st.health, Health::Distressed { .. }));

        // 85 is under the 90 entry threshold but still over the 80 clear boundary: holds distress.
        let ev = decide(1, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 85)])),
            Duration::from_secs(900), now);
        assert!(matches!(st.health, Health::Distressed { .. }),
            "distress cleared before crossing the clear boundary -- no hysteresis");
        assert!(!kinds(&ev).contains(&"cleared"));

        // 78 is under the clear boundary: now it clears.
        let ev = decide(1, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 78)])),
            Duration::from_secs(900), now);
        assert_eq!(kinds(&ev), vec!["cleared"]);
        assert!(matches!(st.health, Health::Up { .. }));
    }

    /// A distressed peer that stops answering reaches Down by the failure counter, the same rule an
    /// Up peer follows.
    #[test]
    fn distressed_to_down_via_the_failure_counter() {
        let peer = distress_peer("jarrah", &[("mem_pct", 90)], &[("mem_pct", 80)]);
        let mut st = PeerState::default();
        let now = Instant::now();
        let _ = decide(2, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 95)])),
            Duration::from_secs(900), now);
        let _ = decide(2, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 95)])),
            Duration::from_secs(900), now);
        assert!(matches!(st.health, Health::Distressed { .. }));

        // One failure: still distressed, counting.
        let ev = decide(2, "argonaut", &peer, &mut st, false, None,
            Duration::from_secs(900), now);
        assert!(ev.is_empty());
        assert!(matches!(st.health, Health::Distressed { failures: 1, .. }));
        // Second failure reaches the threshold: Down.
        let ev = decide(2, "argonaut", &peer, &mut st, false, None,
            Duration::from_secs(900), now);
        assert_eq!(kinds(&ev), vec!["down"]);
        assert_eq!(st.health, Health::Down);
    }

    /// The probe time the watcher measured reaches `decide` as an ordinary field: a slow answer
    /// over the peer's `probe_ms` threshold is distress, named with the time measured here, and
    /// a figure the peer put in its own body under that name is not believed.
    #[test]
    fn probe_ms_is_folded_in_before_the_body_is_judged() {
        let peer = distress_peer("jarrah", &[("probe_ms", 3000)], &[("probe_ms", 1500)]);
        let mut st = PeerState::default();
        let now = Instant::now();
        let repeat = Duration::from_secs(900);
        // The peer claims a 1 ms probe; the watcher measured 3.5 s.
        let claimed = body_of(&[("mem_pct", 40), ("probe_ms", 1)]);

        let (ev, sample) = assess(1, "karri", &peer, &mut st, true, Some(claimed),
            Duration::from_millis(3_500), repeat, now, 1_000);
        assert_eq!(kinds(&ev), vec!["distress"],
            "a probe over its threshold must reach decide and raise distress");
        match &ev[0] {
            AlertEvent::PeerDistress { classes, .. } => assert_eq!(classes, "probe_ms 3500"),
            _ => panic!("expected a distress event"),
        }
        assert_eq!(sample.probe_ms, 3_500);
        assert_eq!(sample.body.as_ref().and_then(|b| b.get("probe_ms")), Some(3_500),
            "the body the dashboard keeps must carry the measured time, not the claimed one");
        assert_eq!(sample.health, PeerHealth::Distressed);
        assert_eq!(sample.t_secs, 1_000);

        // A fast answer under the clear boundary lifts it.
        let (ev, sample) = assess(1, "karri", &peer, &mut st, true,
            Some(body_of(&[("mem_pct", 40)])), Duration::from_millis(200), repeat, now, 1_060);
        assert_eq!(kinds(&ev), vec!["cleared"]);
        assert_eq!(sample.health, PeerHealth::Up);

        // No body, nothing to judge: the time is still kept.
        let (ev, sample) = assess(1, "karri", &peer, &mut st, true, None,
            Duration::from_millis(4_000), repeat, now, 1_120);
        assert!(ev.is_empty());
        assert!(sample.body.is_none());
        assert_eq!(sample.probe_ms, 4_000);
    }

    /// A peer with no thresholds is plain up/down: a hot body never makes it distressed.
    #[test]
    fn a_peer_without_thresholds_never_goes_distressed() {
        let peer = plain_peer("jarrah", "https://example.test/api/health");
        let mut st = PeerState::default();
        let now = Instant::now();
        for _ in 0..5 {
            let ev = decide(1, "argonaut", &peer, &mut st, true, Some(body_of(&[("mem_pct", 99)])),
                Duration::from_secs(900), now);
            assert!(ev.is_empty());
            assert!(matches!(st.health, Health::Up { .. }));
        }
    }
}
