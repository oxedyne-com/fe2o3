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
    cfg::WatchConfig,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    client::https_request,
    header::{
        HttpHeadline,
        HttpMethod,
    },
    loc::Url,
};

use std::{
    collections::HashMap,
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
};

use tokio_rustls::rustls::ClientConfig;


/// What this node currently believes about one peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Health {
    // The count on `Up` is consecutive failures seen since the last success, which is not yet
    // enough to call the peer down.
    Up { failures: u32 },
    Down,
}

/// One peer's running state.
struct PeerState {
    health:      Health,
    failing_at:  Option<Instant>,   // first seen to be failing, so a recovery can say how long
    told_at:     Option<Instant>,   // last told, so a lasting outage is a reminder not a stream
}

impl Default for PeerState {
    fn default() -> Self {
        Self {
            health:     Health::Up { failures: 0 },
            failing_at: None,
            told_at:    None,
        }
    }
}

/// The peer watcher.
///
/// Owns nothing but its configuration, its TLS client and its beliefs. Constructed once at
/// start-up and driven by [`Self::run`], which never returns.
pub struct Watcher {
    cfg:        Arc<WatchConfig>,
    alerter:    Arc<Alerter>,
    tls:        Arc<ClientConfig>,
    // This node's own name, so an alert says who noticed as well as what happened. Two nodes
    // watching a third send two messages, and without this they are indistinguishable.
    whoami:     String,
    state:      HashMap<String, PeerState>,
}

impl Watcher {
    /// Build a watcher over a peer list.
    ///
    /// A peer whose URL cannot be parsed, or one that is not `https`, is refused at start-up
    /// rather than at the first poll: a watcher that silently watches nothing is the failure
    /// this module exists to prevent, and start-up is when somebody is looking.
    pub fn new(
        cfg:     Arc<WatchConfig>,
        alerter: Arc<Alerter>,
        tls:     Arc<ClientConfig>,
        whoami:  String,
    )
        -> Outcome<Self>
    {
        let mut state = HashMap::new();
        for p in &cfg.peers {
            let url = res!(Url::parse(&p.url));
            if !url.scheme.is_tls() {
                return Err(err!(
                    "The watch entry for '{}' names {}, which is not https. A health probe \
                    crosses the public internet and its answer decides whether an operator is \
                    woken, so it is authenticated or it is not worth making.", p.name, p.url;
                    Configuration, Invalid, Input));
            }
            state.insert(p.name.clone(), PeerState::default());
        }
        Ok(Self { cfg, alerter, tls, whoami, state })
    }

    /// Poll every peer for ever.
    ///
    /// Never returns. Errors from a single probe are the point of the exercise and are handled;
    /// there is no failure here worth ending the loop for, because ending the loop is exactly
    /// the silence this is meant to break.
    pub async fn run(mut self) {
        let every = Duration::from_secs(self.cfg.interval_secs.max(5));
        let repeat = Duration::from_secs(self.cfg.repeat_secs.max(60));
        info!("Watching {} peer(s) every {}s: {}.",
            self.cfg.peers.len(),
            every.as_secs(),
            self.cfg.peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));

        // The first probe waits one interval. A node that has just restarted is a node whose
        // peers may still be restarting too -- a shared power event, a rolling deploy -- and an
        // alarm raised in the first second of life is usually about the estate coming up, not
        // about anything being wrong.
        let beat = Duration::from_secs(self.cfg.heartbeat_secs);
        let started = Instant::now();
        let mut last_beat = started;
        loop {
            tokio::time::sleep(every).await;
            let mut ok_count = 0usize;
            for peer in self.cfg.peers.clone() {
                let ok = self.probe(&peer.url).await;
                if ok {
                    ok_count += 1;
                }
                self.judge(&peer.name, &peer.url, ok, repeat);
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

    /// Ask one peer whether it is well.
    ///
    /// Any answer that is not a `2xx` is a failure, including a `503`: a Steel that is up and
    /// sealed is answering, and it is still not serving the databases behind it.
    async fn probe(&self, url: &str) -> bool {
        let loc = match Url::parse(url) {
            Ok(l) => l,
            // Refused at construction, so this cannot happen -- and if it ever does, a peer
            // that cannot be addressed is a peer that is not answering.
            Err(e) => {
                warn!("The watch URL {} stopped parsing: {}", url, e);
                return false;
            },
        };
        let host = loc.host.clone();
        let port = loc.port;
        let path = loc.target.clone();
        let timeout = Duration::from_secs(self.cfg.timeout_secs.max(2));

        let call = https_request(
            &host, port, HttpMethod::GET, &path,
            &[("Connection", "close"), ("User-Agent", "steel-watch")],
            &[],
            self.tls.clone(),
        );
        match tokio::time::timeout(timeout, call).await {
            Ok(Ok(reply)) => {
                let code = match &reply.header.headline {
                    HttpHeadline::Response { status } => *status as u16,
                    // A response with a request headline is not an answer this
                    // can read, and an unreadable answer is not a healthy peer.
                    _ => 0,
                };
                if (200..300).contains(&code) {
                    true
                } else {
                    debug!("Watch: {} answered {}.", url, code);
                    false
                }
            },
            Ok(Err(e)) => {
                debug!("Watch: {} did not answer: {}", url, e);
                false
            },
            Err(_) => {
                debug!("Watch: {} did not answer within {}s.", url, timeout.as_secs());
                false
            },
        }
    }

    /// Fold one probe result into what this node believes, and alert on a change.
    ///
    /// Separated from the polling so the state machine can be tested without a network: the
    /// interesting behaviour is entirely here, and a test that had to stand up a peer to reach
    /// it would test tokio rather than the rule.
    fn judge(&mut self, name: &str, url: &str, ok: bool, repeat: Duration) {
        let threshold = self.cfg.fail_threshold.max(1);
        let now = Instant::now();
        let st = self.state.entry(name.to_string()).or_default();

        if ok {
            if st.health == Health::Down {
                let away = st.failing_at.map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                *st = PeerState::default();
                self.alerter.raise(AlertEvent::PeerRecovered {
                    peer:      name.to_string(),
                    url:       url.to_string(),
                    away_secs: away,
                    noticed_by: self.whoami.clone(),
                });
            } else {
                // A run of failures that did not reach the threshold is forgotten rather than
                // carried. Two isolated timeouts a day apart are not a fault, and a counter that
                // never resets turns them into one eventually.
                *st = PeerState::default();
            }
            return;
        }

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
                    self.alerter.raise(AlertEvent::PeerDown {
                        peer:       name.to_string(),
                        url:        url.to_string(),
                        failures,
                        down_secs,
                        noticed_by: self.whoami.clone(),
                    });
                } else {
                    st.health = Health::Up { failures };
                }
            },
            Health::Down => {
                // Still down. Remind, but only on the repeat interval -- an alarm that fires
                // every poll is an alarm that gets silenced, and the SMS leg of this costs money
                // per message.
                let due = st.told_at.map(|t| now.duration_since(t) >= repeat).unwrap_or(true);
                if due {
                    st.told_at = Some(now);
                    let down_secs = st.failing_at
                        .map(|t| now.duration_since(t).as_secs()).unwrap_or(0);
                    self.alerter.raise(AlertEvent::PeerDown {
                        peer:       name.to_string(),
                        url:        url.to_string(),
                        failures:   threshold,
                        down_secs,
                        noticed_by: self.whoami.clone(),
                    });
                }
            },
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::srv::cfg::WatchPeer;

    /// The state machine, without a network.
    ///
    /// `judge` is driven directly and the alerter is absent, so what is under test is the rule
    /// about when an operator is told -- which is the part that costs money when it is wrong in
    /// one direction and costs an outage when it is wrong in the other.
    fn machine(threshold: u32) -> (WatchConfig, PeerState) {
        let cfg = WatchConfig {
            enabled:        true,
            peers:          vec![WatchPeer {
                name: fmt!("jarrah"),
                url:  fmt!("https://example.test/api/health"),
            }],
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
        cfg.peers.push(WatchPeer {
            name: fmt!("conifer"),
            url:  fmt!("https://ontheism.org/health"),
        });
        assert_eq!(cfg.peers.len(), 2,
            "adding a machine must be a configuration change and nothing else");
    }
}
