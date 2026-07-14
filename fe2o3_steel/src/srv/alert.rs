//! Operator alerting by email.
//!
//! Steel raises an alert when something happens that a human needs to know
//! about and would not otherwise see: it came up with data still sealed, an
//! admin unsealed it, or somebody is guessing at the passphrase.
//!
//! # What is deliberately not alerted
//!
//! Routine events. An alert that fires on every sign-in, every restart, every
//! request, is an alert the operator learns to delete unread, and the one
//! message that mattered goes with the rest. The set below is small on
//! purpose, and each member is rare in normal operation.
//!
//! # The email notifies, it never authorises
//!
//! There is no approve-by-clicking link, and there never should be. The mail
//! says what happened and points at `/admin`; the human authenticates there.
//! An authorisation that arrives by email is an authorisation anybody who can
//! read, spoof or replay that email holds too.
//!
//! # The machine that alerts is the machine in trouble
//!
//! Worth being honest about: Steel is reporting on itself. A Steel that is
//! wedged, unreachable or dead sends nothing, and silence is indistinguishable
//! from health. That is why the alert is addressed *off* the network -- an
//! external mailbox at least survives the host -- and why alerting is a
//! complement to external monitoring, not a substitute for it.

use crate::srv::cfg::AlertConfig;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::{
    imap::client::Security,
    smtp::client::{
        OutboundClient,
        SubmissionConfig,
    },
};

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
    },
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

/// Something worth waking an operator for.
#[derive(Clone, Debug)]
pub enum AlertEvent {
    /// Steel started with databases configured but no master key, so
    /// DB-backed routes are answering 503 until somebody unseals.
    ///
    /// The one that matters most: the websites are up, so nothing else
    /// looks wrong, and without this the operator learns about it from a
    /// user complaint.
    SealedStart {
        /// Databases waiting on the key.
        db_count: usize,
    },
    /// An admin lifted the seal. Rare by construction, and the audit trail
    /// an operator wants: who, when, from where.
    Unsealed {
        admin: String,
        peer:  SocketAddr,
    },
    /// Repeated failures to unwrap the wallet at the dashboard login.
    ///
    /// Coalesced: one message per burst, not one per attempt. The login
    /// form unseals, so it is worth guessing at, and an alerter that sent
    /// a message per guess would be an amplifier pointed at the operator's
    /// mailbox.
    FailedUnseals {
        count:       u32,
        window_secs: u64,
        last_peer:   SocketAddr,
    },
}

impl AlertEvent {
    /// Subject line. Prefixed so the operator can filter on it.
    pub fn subject(&self, host: &str) -> String {
        match self {
            Self::SealedStart { db_count } => fmt!(
                "[steel:{}] SEALED at start -- {} database(s) shut", host, db_count),
            Self::Unsealed { admin, .. } => fmt!(
                "[steel:{}] unsealed by '{}'", host, admin),
            Self::FailedUnseals { count, .. } => fmt!(
                "[steel:{}] {} failed admin passphrase attempts", host, count),
        }
    }

    /// Body text. Plain, short, and it never asks the reader to click
    /// anything that would act on their behalf.
    pub fn body(&self, host: &str) -> String {
        match self {
            Self::SealedStart { db_count } => fmt!(
                "Steel on {host} started sealed.\n\n\
                The websites are serving normally -- static vhosts, redirects, \
                proxy routes and certificate renewal are all unaffected. But {n} \
                database(s) are shut because no wallet master key has been \
                supplied, and any route that needs one is answering 503.\n\n\
                Sign in at https://{host}/admin with an admin passphrase to \
                unseal. Nothing in this email authorises anything; you will be \
                asked to authenticate there.\n",
                host = host, n = db_count),
            Self::Unsealed { admin, peer } => fmt!(
                "Steel on {host} was unsealed.\n\n\
                Admin:  {admin}\n\
                From:   {peer}\n\n\
                The databases are open. If this was not you, treat the wallet \
                passphrase for '{admin}' as compromised: rotate it with \
                `admin --passwd`, and review admin-audit.log.\n",
                host = host, admin = admin, peer = peer),
            Self::FailedUnseals { count, window_secs, last_peer } => fmt!(
                "Steel on {host} refused {count} admin passphrase attempt(s) in \
                the last {mins} minute(s).\n\n\
                Most recent from: {peer}\n\n\
                The dashboard login unwraps the wallet master key, so this form \
                is worth guessing at. Each attempt costs the attacker an Argon2id \
                derivation and is rate limited per address, but a sustained \
                campaign is worth knowing about. Review admin-audit.log, and \
                consider binding the dashboard to localhost via admin_local_port \
                if it does not need to face the internet.\n",
                host = host, count = count, mins = window_secs / 60,
                peer = last_peer),
        }
    }
}

/// Coalescing state for failed passphrase attempts.
#[derive(Debug)]
struct FailureWindow {
    /// Failures counted since the window opened.
    count:  u32,
    /// When the first uncounted failure arrived.
    opened: Instant,
    /// Most recent source.
    last:   Option<SocketAddr>,
    /// When an alert was last sent, so a persistent attacker does not
    /// produce a persistent stream of email.
    sent:   Option<Instant>,
}

/// Sends [`AlertEvent`]s by email.
///
/// Cheap to clone: the configuration and the SMTP client are shared.
#[derive(Clone)]
pub struct Alerter {
    cfg:     Arc<AlertConfig>,
    client:  Arc<OutboundClient>,
    /// Where to post, when posting through a provider rather than delivering
    /// straight to the recipient's MX. Built once, at start-up.
    submission: Option<Arc<SubmissionConfig>>,
    /// Public hostname, used in subject lines and in the `/admin` link.
    host:    Arc<String>,
    failures: Arc<Mutex<FailureWindow>>,
}

impl std::fmt::Debug for Alerter {
    /// Written by hand because `OutboundClient` is not `Debug`, and because
    /// the recipient list is the only part of the configuration worth seeing
    /// in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alerter")
            .field("host", &self.host)
            .field("from", &self.cfg.from)
            .field("to", &self.cfg.to)
            .finish()
    }
}

impl Alerter {

    /// Build an alerter, or `None` when alerting is not configured.
    ///
    /// A misconfigured alerter is a start-up error rather than a silent
    /// no-op: an operator who has written an `alerts` block believes they
    /// will be told when something goes wrong, and the failure mode of
    /// discovering otherwise is that they find out from an outage.
    pub fn new(cfg: AlertConfig, host: String) -> Outcome<Option<Self>> {
        if !cfg.enabled {
            return Ok(None);
        }
        if cfg.to.is_empty() {
            return Err(err!(
                "Alerting is enabled but no recipient is configured. Set \
                'alerts.to' or disable alerting -- an alerter with nobody to \
                tell is worse than none, because it looks like cover.";
                Configuration, Invalid, Missing));
        }
        if cfg.from.is_empty() {
            return Err(err!(
                "Alerting is enabled but 'alerts.from' is empty.";
                Configuration, Invalid, Missing));
        }
        let ehlo = if cfg.ehlo_hostname.is_empty() {
            host.clone()
        } else {
            cfg.ehlo_hostname.clone()
        };
        let client = res!(OutboundClient::with_system_roots(ehlo));
        let submission = match &cfg.submission {
            Some(s) => {
                let security = match s.security.as_str() {
                    "implicit" => Security::ImplicitTls,
                    "plain"    => Security::Plain,
                    _          => Security::StartTls,
                };
                Some(Arc::new(SubmissionConfig::new(
                    s.host.clone(),
                    s.port,
                    security,
                    s.user.clone(),
                    s.password.clone(),
                )))
            }
            None => None,
        };
        Ok(Some(Self {
            cfg:      Arc::new(cfg),
            client:   Arc::new(client),
            submission,
            host:     Arc::new(host),
            failures: Arc::new(Mutex::new(FailureWindow {
                count:  0,
                opened: Instant::now(),
                last:   None,
                sent:   None,
            })),
        }))
    }

    /// Send an alert, without blocking the caller.
    ///
    /// Delivery runs on its own task. The request path must never wait on an
    /// MX lookup and an SMTP round trip, and must never fail because a
    /// mail server did not answer -- an alerter that can take the site down
    /// is a liability, not a safeguard.
    pub fn raise(&self, event: AlertEvent) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.send(&event).await {
                // Log loudly. This is the case where the operator believes
                // they are covered and are not.
                error!(e, "ALERT NOT DELIVERED. The event still happened: {}",
                    event.subject(&this.host));
            }
        });
    }

    /// Record a failed passphrase attempt, raising a coalesced alert once
    /// the burst crosses the configured threshold.
    ///
    /// One message per burst, then a cooldown. A brute-force attempt must
    /// not turn the alerter into a mail flood pointed at the operator.
    pub fn note_failed_unseal(&self, peer: SocketAddr) {
        if let Some(event) = self.record_failure(peer) {
            self.raise(event);
        }
    }

    /// Compose and send one alert.
    ///
    /// Posts through the configured provider when there is one, and otherwise
    /// delivers straight to the recipient's MX. The provider is the better
    /// road: a message that arrives unannounced and unauthenticated from a
    /// host with no PTR record is one a strict receiver may bin, and the alert
    /// saying something is wrong is precisely the one that must not land in a
    /// spam folder.
    async fn send(&self, event: &AlertEvent) -> Outcome<()> {
        let msg = self.compose(event);
        let queue_id = match &self.submission {
            Some(cfg) => res!(self.client.submit(
                cfg,
                &self.cfg.from,
                &self.cfg.to,
                msg.as_bytes(),
            ).await),
            None => res!(self.client.deliver(
                &self.cfg.from,
                &self.cfg.to,
                msg.as_bytes(),
            ).await),
        };
        info!("Alert sent ({}): {}", queue_id, event.subject(&self.host));
        Ok(())
    }

    /// Decide whether a failed attempt should raise an alert, and update the
    /// window. Separated from [`Self::note_failed_unseal`] so the coalescing
    /// rule can be tested without an SMTP server.
    fn record_failure(&self, peer: SocketAddr) -> Option<AlertEvent> {
        let threshold = self.cfg.failed_threshold;
        let cooldown = Duration::from_secs(self.cfg.failed_cooldown_secs);
        let window = Duration::from_secs(self.cfg.failed_window_secs);

        let mut f = match self.failures.lock() {
            Ok(g) => g,
            Err(_) => {
                fault!("The alerter's failure-window lock is poisoned; a failed \
                    passphrase attempt from {} was not counted.", peer);
                return None;
            }
        };
        // A burst is only a burst if it is recent. An attempt long after the
        // last one starts a fresh window rather than topping up a stale count,
        // so a slow trickle over weeks does not eventually trip the threshold
        // and read as an attack.
        if f.opened.elapsed() > window {
            f.count = 0;
            f.opened = Instant::now();
        }
        f.count = f.count.saturating_add(1);
        f.last = Some(peer);

        let cooled = match f.sent {
            Some(t) => t.elapsed() >= cooldown,
            None => true,
        };
        if f.count >= threshold && cooled {
            let event = AlertEvent::FailedUnseals {
                count:       f.count,
                window_secs: f.opened.elapsed().as_secs(),
                last_peer:   peer,
            };
            f.sent = Some(Instant::now());
            f.count = 0;
            f.opened = Instant::now();
            return Some(event);
        }
        None
    }

    /// Build an RFC 5322 message.
    fn compose(&self, event: &AlertEvent) -> String {
        let date = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => fmt!("{}", d.as_secs()),
            Err(_) => fmt!("0"),
        };
        fmt!(
            "From: {from}\r\n\
            To: {to}\r\n\
            Subject: {subject}\r\n\
            X-Steel-Host: {host}\r\n\
            X-Steel-Unix-Time: {date}\r\n\
            Content-Type: text/plain; charset=utf-8\r\n\
            \r\n\
            {body}",
            from    = self.cfg.from,
            to      = self.cfg.to.join(", "),
            subject = event.subject(&self.host),
            host    = self.host,
            date    = date,
            body    = event.body(&self.host).replace('\n', "\r\n"),
        )
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    fn mkalerter(threshold: u32, cooldown_secs: u64) -> Alerter {
        let cfg = AlertConfig {
            enabled:                true,
            from:                   "steel@example.com".to_string(),
            submission:             None,
            to:                     vec!["operator@example.com".to_string()],
            ehlo_hostname:          "example.com".to_string(),
            failed_threshold:       threshold,
            failed_window_secs:     900,
            failed_cooldown_secs:   cooldown_secs,
        };
        match Alerter::new(cfg, "example.com".to_string()) {
            Ok(Some(a)) => a,
            _ => panic!("alerter"),
        }
    }

    fn peer() -> SocketAddr {
        match "203.0.113.7:44321".parse() {
            Ok(p) => p,
            Err(_) => panic!("peer"),
        }
    }

    /// Below the threshold, nothing is raised. One wrong passphrase is a
    /// typo, not an attack, and an operator emailed about typos stops
    /// reading the emails.
    #[test]
    fn test_failures_below_the_threshold_are_silent_00() {
        let a = mkalerter(5, 3600);
        for _ in 0..4 {
            assert!(a.record_failure(peer()).is_none());
        }
        // The fifth crosses it.
        match a.record_failure(peer()) {
            Some(AlertEvent::FailedUnseals { count, .. }) => assert_eq!(count, 5),
            other => panic!("expected a coalesced alert, got {:?}", other),
        }
    }

    /// A brute-force run must produce one message per burst, not one per
    /// guess. An alerter that relays every attempt is an amplifier aimed at
    /// the operator's mailbox, and does the attacker's work for them.
    #[test]
    fn test_a_burst_coalesces_into_one_alert_00() {
        let a = mkalerter(5, 3600);
        let mut raised = 0;
        for _ in 0..100 {
            if a.record_failure(peer()).is_some() {
                raised += 1;
            }
        }
        assert_eq!(raised, 1,
            "100 guesses must yield one alert, not {}", raised);
    }

    /// Once the cooldown lapses, a continuing campaign alerts again --
    /// otherwise a single message would cover an attack running for days.
    #[test]
    fn test_a_lapsed_cooldown_alerts_again_00() {
        let a = mkalerter(2, 0); // zero cooldown: every burst reports
        let mut raised = 0;
        for _ in 0..10 {
            if a.record_failure(peer()).is_some() {
                raised += 1;
            }
        }
        assert_eq!(raised, 5, "ten failures at a threshold of two, no cooldown");
    }

    /// An alerter with nobody to tell is a start-up error, not a quiet
    /// no-op: it looks like cover, and the operator finds out it was not
    /// when something goes wrong and no message arrives.
    #[test]
    fn test_alerting_without_a_recipient_is_refused_00() {
        let cfg = AlertConfig {
            enabled: true,
            from:    "steel@example.com".to_string(),
            to:      Vec::new(),
            ..Default::default()
        };
        assert!(Alerter::new(cfg, "example.com".to_string()).is_err());
    }

    /// End to end, through a stand-in provider: the alert must actually be
    /// composed, authenticated and submitted, and arrive with the event in
    /// it. Alerting's failure mode is *looking like cover* -- an operator who
    /// believes they will be told, and is not -- so it is worth proving the
    /// message leaves the building rather than only that the code was called.
    #[test]
    fn test_an_alert_is_submitted_through_a_provider_00() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => panic!("bind: {}", e),
        };
        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(e) => panic!("addr: {}", e),
        };

        // A stand-in submission server: greet, advertise AUTH, accept the
        // credential, take the message, and hand back what it saw.
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let jh = std::thread::spawn(move || {
            let (sock, _) = match listener.accept() {
                Ok(x) => x,
                Err(_) => return,
            };
            let mut w = match sock.try_clone() {
                Ok(s) => s,
                Err(_) => return,
            };
            let mut lines = BufReader::new(sock).lines();
            let _ = w.write_all(b"220 provider.example.com ESMTP\r\n");
            let mut in_data = false;
            while let Some(Ok(line)) = lines.next() {
                if let Ok(mut g) = log.lock() {
                    g.push(line.clone());
                }
                if in_data {
                    if line == "." {
                        in_data = false;
                        let _ = w.write_all(b"250 2.0.0 Ok: queued as TEST1\r\n");
                    }
                    continue;
                }
                let upper = line.to_uppercase();
                if upper.starts_with("EHLO") {
                    let _ = w.write_all(
                        b"250-provider.example.com\r\n250-AUTH PLAIN\r\n250 8BITMIME\r\n");
                } else if upper.starts_with("AUTH PLAIN") {
                    let _ = w.write_all(b"235 2.7.0 Accepted\r\n");
                } else if upper.starts_with("DATA") {
                    in_data = true;
                    let _ = w.write_all(b"354 End data\r\n");
                } else if upper.starts_with("QUIT") {
                    let _ = w.write_all(b"221 2.0.0 Bye\r\n");
                    return;
                } else {
                    let _ = w.write_all(b"250 2.0.0 Ok\r\n");
                }
            }
        });

        let cfg = AlertConfig {
            enabled:                true,
            from:                   "steel@example.com".to_string(),
            submission:             Some(crate::srv::cfg::AlertSubmission {
                host:       "provider.example.com".to_string(),
                port:       addr.port(),
                security:   "plain".to_string(),
                user:       "steel@example.com".to_string(),
                password:   "app-password".to_string(),
            }),
            to:                     vec!["operator@elsewhere.example".to_string()],
            ehlo_hostname:          "example.com".to_string(),
            failed_threshold:       5,
            failed_window_secs:     900,
            failed_cooldown_secs:   3600,
        };
        let mut a = match Alerter::new(cfg, "example.com".to_string()) {
            Ok(Some(a)) => a,
            other => panic!("alerter: {:?}", other.is_err()),
        };
        // Pin the dialled address: the certificate name stays
        // provider.example.com, and no resolver is involved.
        a.submission = a.submission.map(|s| {
            Arc::new((*s).clone().with_addr(addr))
        });

        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all().build()
        {
            Ok(r) => r,
            Err(e) => panic!("runtime: {}", e),
        };
        let ev = AlertEvent::SealedStart { db_count: 3 };
        match rt.block_on(a.send(&ev)) {
            Ok(()) => (),
            Err(e) => panic!("the alert was not submitted: {}", e),
        }
        let _ = jh.join();

        let transcript = match seen.lock() {
            Ok(g) => g.join("\n"),
            Err(_) => panic!("lock"),
        };
        assert!(transcript.contains("AUTH PLAIN"),
            "the alerter must authenticate to the provider:\n{}", transcript);
        assert!(transcript.contains("MAIL FROM:<steel@example.com>"),
            "envelope sender missing:\n{}", transcript);
        assert!(transcript.contains("RCPT TO:<operator@elsewhere.example>"),
            "envelope recipient missing:\n{}", transcript);
        assert!(transcript.contains("SEALED at start -- 3 database(s) shut"),
            "the event did not reach the message:\n{}", transcript);
    }

    /// The message must never carry an action link. An authorisation that
    /// arrives by email is one that anybody able to read, spoof or replay
    /// the email holds too.
    #[test]
    fn test_the_email_notifies_but_never_authorises_00() {
        let ev = AlertEvent::SealedStart { db_count: 2 };
        let body = ev.body("example.com");
        assert!(body.contains("https://example.com/admin"),
            "the mail should point the operator at the dashboard");
        for bait in ["token=", "approve", "confirm=", "unseal?key", "click here"] {
            assert!(!body.to_lowercase().contains(bait),
                "the alert must not carry an authorising link ({})", bait);
        }
    }
}
