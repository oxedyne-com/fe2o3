//! Operator alerting, by email and by text message.
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
//!
//! # A channel that fails is reported through the other
//!
//! A channel can die as quietly as a host. From 2026-08-31 the SMS gateway
//! refused every text in the estate for want of credit, each refusal was
//! logged as sent, and it was found by an audit three weeks later. So a
//! refusal is a failure (see `oxedyne_fe2o3_net::sms`), and a channel that
//! starts failing is reported through the other one: text failures by mail,
//! and mail failures by text. The report comes when the failures start, again
//! once a day while they last, and once more when the channel delivers again
//! (see [`ChannelFailing`](AlertEvent::ChannelFailing)). An event that no
//! channel on the host carries at all, such as a notice on a host with no
//! mail, is logged as undelivered rather than dropped.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::cfg::{
    AlertConfig,
    SmsAlertConfig,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::{
    dkim::DkimSigner,
    http::{
        client::https_request,
        header::HttpHeadline,
    },
    imap::client::Security,
    smtp::client::{
        OutboundClient,
        SubmissionConfig,
    },
    sms::{
        Credential,
        Message as SmsMessage,
        Provider as SmsProvider,
        Receipt,
    },
};

use tokio_rustls::rustls::ClientConfig;

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

// How often a channel that keeps failing is reported again. Daily: a report read and forgotten
// comes back, and a channel dead over a long weekend is three messages, not three hundred.
const CHANNEL_REMIND: Duration = Duration::from_secs(86_400);

// How long one text may take, dial to receipt. A gateway that hangs must end as a failure in the
// log, not as a task that waits for ever with its text neither sent nor refused.
const SMS_TIMEOUT: Duration = Duration::from_secs(30);

/// A way this host reaches its operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Mail,
    Sms,
}

impl Channel {
    /// What the alerts on this channel are called in a message about it.
    pub fn alerts(&self) -> &'static str {
        match self {
            Self::Mail  => "alert mail",
            Self::Sms   => "text alerts",
        }
    }

    /// How an alert travels on this channel: "getting through by ...".
    pub fn by(&self) -> &'static str {
        match self {
            Self::Mail  => "mail",
            Self::Sms   => "text message",
        }
    }
}

/// Something worth waking an operator for.
#[derive(Clone, Debug)]
pub enum AlertEvent {
    // Started with databases configured but no master key, so DB-backed routes
    // answer 503 until somebody unseals. The one that matters most: the
    // websites are up, so nothing else looks wrong, and without this the
    // operator learns about it from a user complaint.
    SealedStart {
        db_count: usize,        // databases waiting on the key
    },
    // Rare by construction, and the audit trail an operator wants: who, when,
    // from where.
    Unsealed {
        admin: String,
        peer:  SocketAddr,
    },
    // Repeated failures to unwrap the wallet at the dashboard login, coalesced
    // into one message per burst rather than one per attempt. The login form
    // unseals, so it is worth guessing at, and an alerter that sent a message
    // per guess would be an amplifier pointed at the operator's mailbox.
    FailedUnseals {
        count:       u32,
        window_secs: u64,
        last_peer:   SocketAddr,
    },
    // The one event this host can raise about somebody else, and the reason
    // crate::srv::watch exists: a dead machine sends nothing, so the alarm has
    // to come from a live one.
    PeerDown {
        peer:       String,     // as configured
        url:        String,     // what was probed
        failures:   u32,        // consecutive failed probes
        down_secs:  u64,
        // Which machine noticed. Two watchers see one outage and send two
        // messages; without this they read as one message sent twice.
        noticed_by: String,
    },
    // Sent because an operator who was woken is owed the end of the story, and
    // because a recovery nobody announced is one somebody drives to the office
    // for.
    PeerRecovered {
        peer:       String,
        url:        String,
        away_secs:  u64,
        noticed_by: String,
    },
    // A peer that is answering but reports itself unwell: a health-body class has
    // stayed over its distress threshold. Distinct from `PeerDown` -- the box is
    // up, so this goes by email rather than waking somebody with an SMS -- and it
    // names the class that fired so the reader knows which resource is short.
    PeerDistress {
        peer:       String,
        url:        String,
        classes:    String,     // e.g. "mem_pct 94, swap_pct 71"
        since_secs: u64,
        noticed_by: String,
    },
    // The end of a distress episode, owed for the same reason as a recovery from
    // down: an operator told a box was unwell wants to know it came back under
    // its thresholds.
    PeerDistressCleared {
        peer:       String,
        url:        String,
        were_secs:  u64,        // how long the peer was distressed
        noticed_by: String,
    },
    // Proof that the alerting path itself still works, and the point of it is
    // that it is boring. A path used twice a year is broken when it is needed
    // -- an expired credential, a rotated key, a changed number, a dormant
    // account -- and it is discovered during the incident. This exercises every
    // leg on a schedule, so the failure is found on an ordinary afternoon
    // instead.
    Heartbeat {
        uptime_secs: u64,
        peers_ok:    usize,     // of `peers_total`, how many are answering
        peers_total: usize,
    },
    // One of this host's own channels has stopped delivering: a text refused for want of
    // credit, a relay that no longer takes the password. Told through the other channel, since
    // a channel can no more report its own failure than a host can report its own death.
    ChannelFailing {
        channel:    Channel,
        reason:     String,     // the provider's own words, from the latest failure
        missed:     String,     // the subject of the latest alert this channel lost
        failures:   u32,        // consecutive failed deliveries
        since_secs: u64,        // since the first of them
    },
    // The end of that story, owed for the same reason a recovery is: the operator told of a dead
    // channel wants to know it works again, and how much it lost.
    ChannelRestored {
        channel:    Channel,
        lost:       u32,        // the deliveries that failed in the episode
        were_secs:  u64,        // from the first of them to this success
    },
}

/// How loudly an event should be delivered.
///
/// The distinction is the whole of the routing rule, and it exists because the
/// channels differ in cost and in how much they intrude. An operator who is
/// texted about routine events stops reading the texts, and the one that
/// mattered goes with the rest -- which is the same argument the module header
/// makes for keeping the event set small.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Critical,   // every channel, including the ones that cost money and wake somebody
    Notice,     // worth a record; mail only
}

impl AlertEvent {
    /// A peer going down is the only thing here that reaches a phone, plus the
    /// heartbeat that proves a phone still can be reached. Everything else is
    /// about this machine, and this machine can only report it while it is
    /// well enough to be read about later.
    pub fn severity(&self) -> Severity {
        match self {
            Self::PeerDown { .. }		=> Severity::Critical,
            Self::PeerRecovered { .. }		=> Severity::Critical,
            Self::Heartbeat { .. }		=> Severity::Critical,
            Self::SealedStart { .. }		=> Severity::Notice,
            Self::Unsealed { .. }		=> Severity::Notice,
            Self::FailedUnseals { .. }		=> Severity::Notice,
            // A distressed peer is up, so it is worth a record, not a phone call.
            Self::PeerDistress { .. }		=> Severity::Notice,
            Self::PeerDistressCleared { .. }	=> Severity::Notice,
            // Dead mail reaches the phone, as the heartbeat does, because it is about whether
            // the operator can be reached at all: every notice goes by mail alone. Dead texts
            // are told by mail, the one channel left.
            Self::ChannelFailing { channel: Channel::Mail, .. }	=> Severity::Critical,
            Self::ChannelFailing { channel: Channel::Sms, .. }	=> Severity::Notice,
            Self::ChannelRestored { .. }			=> Severity::Notice,
        }
    }

    /// The whole event in one line, for a channel that has no subject and no
    /// body and charges by the segment.
    ///
    /// Blunt on purpose, and written separately rather than trimmed from
    /// [`Self::body`]. It is read on a lock screen in the dark by somebody who
    /// wants to know whether to get up, so it leads with the machine and the
    /// verdict. How long it has been down, which machine noticed and what was
    /// probed are all in the email, which costs nothing to make longer; here
    /// they are noise in front of the one word that matters.
    pub fn short(&self, host: &str) -> String {
        match self {
            Self::PeerDown { peer, .. }         => fmt!("{} is DOWN", peer),
            Self::PeerRecovered { peer, .. }    => fmt!("{} is back", peer),
            Self::Heartbeat { peers_ok, peers_total, .. } => fmt!(
                "{}: alerting alive, {}/{} ok", host, peers_ok, peers_total),
            Self::ChannelFailing { channel, .. } => fmt!(
                "{}: {} FAILING", host, channel.alerts()),
            Self::ChannelRestored { channel, .. } => fmt!(
                "{}: {} working again", host, channel.alerts()),
            other                               => other.subject(host),
        }
    }

    /// Subject line. Prefixed so the operator can filter on it.
    pub fn subject(&self, host: &str) -> String {
        match self {
            Self::SealedStart { db_count } => fmt!(
                "[steel:{}] SEALED at start -- {} database(s) shut", host, db_count),
            Self::Unsealed { admin, .. } => fmt!(
                "[steel:{}] unsealed by '{}'", host, admin),
            Self::FailedUnseals { count, .. } => fmt!(
                "[steel:{}] {} failed admin passphrase attempts", host, count),
            Self::PeerDown { peer, down_secs, .. } => fmt!(
                "[steel:{}] {} IS DOWN ({}m)", host, peer, down_secs / 60),
            Self::PeerRecovered { peer, away_secs, .. } => fmt!(
                "[steel:{}] {} recovered after {}m", host, peer, away_secs / 60),
            Self::PeerDistress { peer, classes, .. } => fmt!(
                "[steel:{}] {} in distress ({})", host, peer, classes),
            Self::PeerDistressCleared { peer, were_secs, .. } => fmt!(
                "[steel:{}] {} distress cleared after {}m", host, peer, were_secs / 60),
            Self::Heartbeat { peers_ok, peers_total, .. } => fmt!(
                "[steel:{}] alerting alive, {}/{} peers answering",
                host, peers_ok, peers_total),
            Self::ChannelFailing { channel, failures, .. } => fmt!(
                "[steel:{}] {} FAILING ({} in a row)", host, channel.alerts(), failures),
            Self::ChannelRestored { channel, were_secs, .. } => fmt!(
                "[steel:{}] {} working again after {}m", host, channel.alerts(), were_secs / 60),
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
            Self::PeerDown { peer, url, failures, down_secs, noticed_by } => fmt!(
                "{peer} is not answering.\n\n\
                Probed:   {url}\n\
                Failures: {failures} consecutive\n\
                Down for: {mins} minute(s)\n\
                Noticed by: {noticed_by}\n\n\
                This machine is reporting on another one, because a host that has \
                died cannot report its own death. Nothing has been restarted: a \
                watcher that repairs can flap a service in a loop and hide the \
                fault it was built to reveal, and a decision to restart belongs to \
                somebody who has read why it stopped.\n\n\
                If this is the Daimond gateway, its log is at \
                ~/usr/daimond-gateway/log/, the pane is held open after an exit, \
                and the last lines say what it said on the way out.\n",
                peer = peer, url = url, failures = failures,
                mins = down_secs / 60, noticed_by = noticed_by),
            Self::PeerRecovered { peer, url, away_secs, noticed_by } => fmt!(
                "{peer} is answering again after {mins} minute(s).\n\n\
                Probed:   {url}\n\
                Noticed by: {noticed_by}\n\n\
                Nothing here did that; it came back on its own or somebody fixed \
                it. Worth reading the log for what stopped it, because a fault \
                that cleared itself is a fault that can return.\n",
                peer = peer, url = url, mins = away_secs / 60, noticed_by = noticed_by),
            Self::PeerDistress { peer, url, classes, since_secs, noticed_by } => fmt!(
                "{peer} is answering but reports itself unwell.\n\n\
                Over threshold: {classes}\n\
                Probed:   {url}\n\
                For:      {mins} minute(s)\n\
                Noticed by: {noticed_by}\n\n\
                The box is up and serving; a resource it depends on is short. This \
                is not an outage, which is why it arrives by email and not as a \
                text -- but a box under sustained pressure is one on its way to an \
                outage, and it is cheaper to look now. The figures are read from \
                the box's own health body; nothing here has acted on it.\n",
                peer = peer, url = url, classes = classes,
                mins = since_secs / 60, noticed_by = noticed_by),
            Self::PeerDistressCleared { peer, url, were_secs, noticed_by } => fmt!(
                "{peer} is back under its thresholds after {mins} minute(s).\n\n\
                Probed:   {url}\n\
                Noticed by: {noticed_by}\n\n\
                The resource that was short has recovered. Worth a glance at what \
                drove it, because pressure that cleared itself can build again.\n",
                peer = peer, url = url, mins = were_secs / 60, noticed_by = noticed_by),
            Self::Heartbeat { uptime_secs, peers_ok, peers_total } => fmt!(
                "Alerting on {host} is alive. Nothing is wrong.\n\n\
                Uptime:  {days} day(s)\n\
                Peers:   {ok} of {total} answering\n\n\
                This message exists to prove the path still works. An alerting \
                route used twice a year is broken when it is needed -- an expired \
                credential, a rotated key, a changed number, a dormant account -- \
                and it is discovered during the incident. If these stop arriving, \
                the alerting is what has failed, not the estate.\n",
                host = host, days = uptime_secs / 86400,
                ok = peers_ok, total = peers_total),
            Self::ChannelFailing { channel, reason, missed, failures, since_secs } => fmt!(
                "Alerts from {host} are not getting through by {by}.\n\n\
                Failed:    {failures} in a row, over {mins} minute(s)\n\
                Last lost: {missed}\n\
                Why:       {reason}\n\n\
                {advice}\n\n\
                This report comes by another channel, because a channel cannot report \
                its own failure. It is repeated once a day while the failures go on, \
                and the first alert that gets through again ends it with a message of \
                its own.\n",
                host = host, by = channel.by(), failures = failures,
                mins = since_secs / 60, missed = missed, reason = reason,
                advice = match channel {
                    Channel::Sms => "A text needs a funded account and a working credential \
                        with the gateway, and the gateway's own words are above. Until it is \
                        fixed, a peer going down reaches you by mail alone, and mail needs the \
                        data connection a phone does not always have.",
                    Channel::Mail => "Mail needs a relay or a recipient's server that accepts \
                        it, and a working credential where it goes through a relay; the \
                        server's own words are above. Until it is fixed, a sealed start, an \
                        unseal, a burst of failed passphrases and a peer in distress reach \
                        nobody, because they go by mail alone.",
                }),
            Self::ChannelRestored { channel, lost, were_secs } => fmt!(
                "Alerts from {host} are getting through by {by} again.\n\n\
                Lost:  {lost} alert(s) in a row failed on this channel\n\
                Over:  {mins} minute(s)\n\n\
                Those alerts were not sent again. Each is in the log on {host}, under \
                \"ALERT NOT DELIVERED\".\n",
                host = host, by = channel.by(), lost = lost, mins = were_secs / 60),
        }
    }
}


/// Coalescing state for failed passphrase attempts.
#[derive(Debug)]
struct FailureWindow {
    count:  u32,                 // failures since the window opened
    opened: Instant,             // when the first uncounted failure arrived
    last:   Option<SocketAddr>,
    sent:   Option<Instant>,     // so a persistent attacker is not a stream of email
}

/// A run of failed deliveries on one channel.
#[derive(Clone, Debug)]
struct Failing {
    since:      Instant,    // the first failure of the run
    failures:   u32,        // consecutive failed deliveries
    told:       Instant,    // when the run was last reported
}

/// What this alerter has seen of its own channels, so a channel that stops delivering is
/// reported rather than only logged.
#[derive(Debug, Default)]
struct ChannelBook {
    mail:   Option<Failing>,
    sms:    Option<Failing>,
}

impl ChannelBook {
    /// Fold one delivery's outcome into the book, returning the report it is owed, if any.
    ///
    /// `failure` is the provider's words for a failed delivery and `None` for a success;
    /// `missed` is the subject of the alert concerned. The first failure of a run is reported,
    /// then one each [`CHANNEL_REMIND`] while the run lasts, and the first success after it
    /// ends the run with a report of its own. Each of those happens once, which is what stops a
    /// report whose own delivery fails from setting off another.
    fn note(
        &mut self,
        ch:         Channel,
        failure:    Option<&str>,
        missed:     &str,
        now:        Instant,
    )
        -> Option<AlertEvent>
    {
        let slot = match ch {
            Channel::Mail   => &mut self.mail,
            Channel::Sms    => &mut self.sms,
        };
        let why = match failure {
            Some(w) => w,
            None => return match slot.take() {
                Some(f) => Some(AlertEvent::ChannelRestored {
                    channel:    ch,
                    lost:       f.failures,
                    were_secs:  now.saturating_duration_since(f.since).as_secs(),
                }),
                None => None,
            },
        };
        match slot {
            None => {
                *slot = Some(Failing { since: now, failures: 1, told: now });
                Some(AlertEvent::ChannelFailing {
                    channel:    ch,
                    reason:     why.to_string(),
                    missed:     missed.to_string(),
                    failures:   1,
                    since_secs: 0,
                })
            },
            Some(f) => {
                f.failures = f.failures.saturating_add(1);
                if now.saturating_duration_since(f.told) < CHANNEL_REMIND {
                    return None;
                }
                f.told = now;
                Some(AlertEvent::ChannelFailing {
                    channel:    ch,
                    reason:     why.to_string(),
                    missed:     missed.to_string(),
                    failures:   f.failures,
                    since_secs: now.saturating_duration_since(f.since).as_secs(),
                })
            },
        }
    }
}

/// The channels one event goes by on this host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Route {
    mail:   bool,
    sms:    bool,
}

impl Route {
    fn is_empty(&self) -> bool {
        !self.mail && !self.sms
    }
}

/// Sends [`AlertEvent`]s by email and, for the critical ones, by text message.
///
/// Cheap to clone: the configuration, the SMTP client and the channel book are shared.
#[derive(Clone)]
pub struct Alerter {
    cfg:     Arc<AlertConfig>,
    client:  Arc<OutboundClient>,
    // Where to post, when posting through a provider rather than delivering
    // straight to the recipient's MX. Built once, at start-up.
    submission: Option<Arc<SubmissionConfig>>,
    // DKIM identities to sign the alert with, when the host is configured to
    // sign its mail at all. An unsigned message from a domain that signs
    // everything else is exactly what a spam filter is entitled to distrust --
    // and the alert is the one message that has to arrive. The alerter posts
    // straight through the SMTP client rather than through the mail handler, so
    // it has to sign for itself.
    dkim:       Vec<Arc<DkimSigner>>,
    host:    Arc<String>,   // public, used in subject lines and the `/admin` link
    failures: Arc<Mutex<FailureWindow>>,
    channels: Arc<Mutex<ChannelBook>>,  // shared by every clone, so every delivery counts
    // Outbound TLS, for the SMS gateway. Absent on a host with no outbound
    // client, in which case the mail leg still works and the text leg says so
    // rather than failing silently.
    tls:     Option<Arc<ClientConfig>>,
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
    pub fn new(
        cfg:    AlertConfig,
        host:   String,
        dkim:   Vec<Arc<DkimSigner>>,
        tls:    Option<Arc<ClientConfig>>,
    )
        -> Outcome<Option<Self>>
    {
        if !cfg.enabled {
            return Ok(None);
        }
        // Mail is the usual channel and not the only one. A host that cannot
        // send mail -- its provider blocks outbound port 25 and it has no relay
        // to submit through -- still has a phone to reach, and refusing to
        // alert at all because one channel is unavailable would leave it silent
        // for the reason it most needs to speak.
        let texts = cfg.sms.as_ref().map(|s| s.enabled && !s.to.is_empty()).unwrap_or(false);
        let mails = !cfg.to.is_empty() && !cfg.from.is_empty();
        if !mails && !texts {
            return Err(err!(
                "Alerting is enabled but has no way to reach anybody: set \
                'alerts.from' and 'alerts.to' for mail, or 'alerts.sms' for \
                text messages, or disable alerting. An alerter with nobody to \
                tell is worse than none, because it looks like cover.";
                Configuration, Invalid, Missing));
        }
        if !mails && texts {
            warn!("Alerting has no mail recipient, so only a peer going down or coming back, \
                and the heartbeat, reach anybody: they go by text message. A sealed start, an \
                unseal, a burst of failed passphrases, a peer in distress and a failing text \
                channel go by mail alone, so on this host they reach nobody and are logged as \
                undelivered. Give 'alerts' a 'from' and a 'to', with a 'submission' relay \
                where this host cannot send direct.");
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
            tls,
            dkim,
            host:     Arc::new(host),
            failures: Arc::new(Mutex::new(FailureWindow {
                count:  0,
                opened: Instant::now(),
                last:   None,
                sent:   None,
            })),
            channels: Arc::new(Mutex::new(ChannelBook::default())),
        }))
    }

    /// Does this alerter have a mail recipient? Without one, a `Notice` event -- distress among
    /// them -- has no channel at all.
    pub fn sends_mail(&self) -> bool {
        !self.cfg.to.is_empty() && !self.cfg.from.is_empty()
    }

    /// Does this alerter have a text-message gateway and a number to text?
    pub fn sends_sms(&self) -> bool {
        self.cfg.sms.as_ref().map(|s| s.enabled && !s.to.is_empty()).unwrap_or(false)
    }

    /// The channels an event goes by on this host: mail when there is a recipient, a text when
    /// the event is critical and there is a gateway, and never the channel whose failure the
    /// event reports.
    fn route(&self, event: &AlertEvent) -> Route {
        let failing = match event {
            AlertEvent::ChannelFailing { channel, .. } => Some(*channel),
            _ => None,
        };
        Route {
            mail:   self.sends_mail() && failing != Some(Channel::Mail),
            sms:    self.sends_sms()
                        && event.severity() == Severity::Critical
                        && failing != Some(Channel::Sms),
        }
    }

    /// Send an alert, without blocking the caller.
    ///
    /// Delivery runs on its own task. The request path must never wait on an
    /// MX lookup and an SMTP round trip, and must never fail because a
    /// mail server did not answer -- an alerter that can take the site down
    /// is a liability, not a safeguard.
    ///
    /// An event that no channel on this host carries, such as a notice on a host with no mail
    /// recipient, is logged as an error rather than dropped: it is an alert nobody will get.
    pub fn raise(&self, event: AlertEvent) {
        let route = self.route(&event);
        if route.is_empty() {
            fault!("ALERT NOT DELIVERED: no channel on this host carries it. {} goes by {}, \
                and this host has {}. The event still happened: {}",
                match event.severity() {
                    Severity::Critical  => "A critical alert",
                    Severity::Notice    => "A notice",
                },
                match event.severity() {
                    Severity::Critical  => "mail and text message",
                    Severity::Notice    => "mail alone",
                },
                match (self.sends_mail(), self.sends_sms()) {
                    (true, true)    => "both, one of which is the channel this reports on",
                    (true, false)   => "mail alone, the channel this reports on",
                    (false, true)   => "no mail recipient",
                    (false, false)  => "neither",
                },
                event.subject(&self.host));
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.deliver(event, route).await;
        });
    }

    /// Deliver one event by each channel its route names, and account for each.
    ///
    /// Both legs are attempted, and neither is allowed to prevent the other. They fail for
    /// different reasons -- mail needs a working MX and a mailbox somebody reads, a text needs a
    /// funded account and a carrier -- and an alerter that abandoned the second because the
    /// first threw would have exactly one channel on the night both were needed.
    async fn deliver(&self, event: AlertEvent, route: Route) {
        if route.mail {
            match self.send(&event).await {
                Ok(()) => self.settle(Channel::Mail, None, &event),
                Err(e) => {
                    self.settle(Channel::Mail, Some(e.plain()), &event);
                    // Log loudly. This is the case where the operator believes they are
                    // covered and are not.
                    error!(e, "ALERT NOT DELIVERED BY MAIL. The event still happened: {}",
                        event.subject(&self.host));
                },
            }
        }
        if route.sms {
            match self.send_sms(&event).await {
                Ok(()) => self.settle(Channel::Sms, None, &event),
                Err(e) => {
                    self.settle(Channel::Sms, Some(e.plain()), &event);
                    error!(e, "ALERT NOT DELIVERED BY SMS. The event still happened: {}",
                        event.subject(&self.host));
                },
            }
        }
    }

    /// Record how a channel did, and raise the report that is owed: a channel that has started
    /// failing, is still failing a day on, or has just recovered is told through the others.
    fn settle(&self, ch: Channel, failure: Option<String>, event: &AlertEvent) {
        let report = {
            let mut book = lock_mutex_or_recover!(self.channels,
                "The alerter's channel book was poisoned; carrying on with what it held.");
            book.note(ch, failure.as_deref(), &event.subject(&self.host), Instant::now())
        };
        if let Some(r) = report {
            self.raise(r);
        }
    }

    /// Send the one-line form of an event as a text message, to every configured number.
    ///
    /// Every number is tried, and the alert fails if the gateway refused any of them. A text
    /// is logged as sent only on the gateway's receipt for it.
    ///
    /// **The credential is read from the environment, never from the
    /// configuration file.** A configuration file is copied between machines,
    /// pasted into a chat window to ask why a server will not start, and
    /// committed by accident; an environment variable is none of those things
    /// by default. The two names are configurable so a host can carry more than
    /// one account without a second Steel.
    async fn send_sms(&self, event: &AlertEvent) -> Outcome<()> {
        let sms = match &self.cfg.sms {
            Some(s) if s.enabled => s,
            _ => return Err(err!(
                "An SMS alert cannot be sent: no SMS gateway is configured."; Configuration, Missing)),
        };
        let tls = match &self.tls {
            Some(t) => t.clone(),
            None => return Err(err!(
                "An SMS alert cannot be sent: this Steel has no outbound TLS client, so it \
                cannot reach {}.", sms.provider.id();
                Init, Missing)),
        };
        let user = res!(std::env::var(&sms.user_env).map_err(|_| err!(
            "The SMS gateway account is read from ${}, which is not set. The alert was not \
            sent as a text.", sms.user_env;
            Configuration, Missing)));
        let secret = res!(std::env::var(&sms.secret_env).map_err(|_| err!(
            "The SMS gateway secret is read from ${}, which is not set. The alert was not \
            sent as a text.", sms.secret_env;
            Configuration, Missing)));

        let cred = Credential { user: &user, secret: &secret };
        let text = event.short(&self.host);
        let mut outcomes = Vec::with_capacity(sms.to.len());
        for to in &sms.to {
            let got = text_one(sms, &cred, to, &text, tls.clone()).await;
            outcomes.push((to.clone(), got));
        }
        tally_texts(sms.provider, outcomes, &event.subject(&self.host))
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
        let mut msg = self.compose(event).into_bytes();

        // Sign with every configured key, as the mail path does. A key that
        // will not sign is skipped rather than fatal: an alert that goes out
        // unsigned still reaches the operator, and an alert that does not go
        // out at all reaches nobody.
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs(),
            Err(_) => 0,
        };
        for signer in &self.dkim {
            match signer.sign(&msg, &[], now) {
                Ok(b) => msg = b,
                Err(e) => warn!("Signing an alert with the {} key for selector \
                    '{}' failed; sending it unsigned: {}",
                    signer.algorithm(), signer.selector(), e),
            }
        }
        let msg = String::from_utf8_lossy(&msg).into_owned();
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


/// Dial the gateway for one number and read its answer, which is a receipt only when the
/// gateway took the message.
async fn text_one(
    sms:    &SmsAlertConfig,
    cred:   &Credential<'_>,
    to:     &str,
    text:   &str,
    tls:    Arc<ClientConfig>,
)
    -> Outcome<Receipt>
{
    let m = SmsMessage { to, from: &sms.from, body: text };
    let call = res!(sms.provider.request(cred, &m));
    let headers: Vec<(&str, &str)> = call.headers.iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let dial = https_request(
        &call.host, call.port, call.method, &call.path, &headers, &call.body, tls);
    let reply = match tokio::time::timeout(SMS_TIMEOUT, dial).await {
        Ok(r) => res!(r),
        Err(_) => return Err(err!(
            "{} did not answer within {}s, so the text is not known to have been sent.",
            sms.provider.id(), SMS_TIMEOUT.as_secs();
            Network, Timeout)),
    };
    let status = match &reply.header.headline {
        HttpHeadline::Response { status } => *status as u16,
        // Not a response at all, which is no receipt.
        _ => 0,
    };
    // The whole answer is read, status and body. Two of these gateways answer a rejected
    // credential with 200 and an object explaining themselves, and one answers an unfunded
    // account with 200 and a refusal inside the message record, so a sender that trusted the
    // status would log every alert as delivered while none of them was.
    sms.provider.parse(status, &reply.body)
}

/// Account for one alert's texts: each accepted text is logged as sent, and the alert fails
/// when any number's was refused or not sent, naming each number and the provider's words.
///
/// One refusal among several numbers fails the alert. More than one number is configured
/// because any of them may be the one that is read, and a refusal is a person not told.
fn tally_texts(
    provider:   SmsProvider,
    outcomes:   Vec<(String, Outcome<Receipt>)>,
    subject:    &str,
)
    -> Outcome<()>
{
    let total = outcomes.len();
    if total == 0 {
        return Err(err!(
            "SMS alerting is enabled with no numbers to send to."; Configuration, Missing));
    }
    let mut refused = Vec::new();
    for (to, got) in outcomes {
        match got {
            Ok(r)   => info!("Alert texted to {} ({} {}, id {}): {}",
                to, provider.id(), r.status, r.id, subject),
            Err(e)  => refused.push(fmt!("{}: {}", to, e.plain())),
        }
    }
    if refused.is_empty() {
        return Ok(());
    }
    Err(err!("{} of {} text(s) were not sent. {}", refused.len(), total, refused.join("; ");
        Network))
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
            sms:                    None,
        };
        match Alerter::new(cfg, "example.com".to_string(), Vec::new(), None) {
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
        assert!(Alerter::new(cfg, "example.com".to_string(), Vec::new(), None).is_err());
    }

    /// A stand-in submission server on loopback. For each of `conns` connections in turn it
    /// greets, advertises AUTH, accepts the credential, takes the message and records every
    /// line it was sent.
    fn stand_in_provider(conns: usize)
        -> (SocketAddr, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>)
    {
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
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let jh = std::thread::spawn(move || {
            for _ in 0..conns {
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
                        break;
                    } else {
                        let _ = w.write_all(b"250 2.0.0 Ok\r\n");
                    }
                }
            }
        });
        (addr, seen, jh)
    }

    /// An alerter that submits its mail to a stand-in at `addr`, with the given text leg.
    fn alerter_via(addr: SocketAddr, sms: Option<SmsAlertConfig>) -> Alerter {
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
            sms,
        };
        let mut a = match Alerter::new(cfg, "example.com".to_string(), Vec::new(), None) {
            Ok(Some(a)) => a,
            other => panic!("alerter: {:?}", other.is_err()),
        };
        // Pin the dialled address: the certificate name stays
        // provider.example.com, and no resolver is involved.
        a.submission = a.submission.map(|s| {
            Arc::new((*s).clone().with_addr(addr))
        });
        a
    }

    /// The whole of what a stand-in was sent, one line per line.
    fn transcript(seen: &Arc<Mutex<Vec<String>>>) -> String {
        match seen.lock() {
            Ok(g) => g.join("\n"),
            Err(_) => panic!("lock"),
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => panic!("runtime: {}", e),
        }
    }

    /// End to end, through a stand-in provider: the alert must actually be
    /// composed, authenticated and submitted, and arrive with the event in
    /// it. Alerting's failure mode is *looking like cover* -- an operator who
    /// believes they will be told, and is not -- so it is worth proving the
    /// message leaves the building rather than only that the code was called.
    #[test]
    fn test_an_alert_is_submitted_through_a_provider_00() {
        let (addr, seen, jh) = stand_in_provider(1);
        let a = alerter_via(addr, None);
        let ev = AlertEvent::SealedStart { db_count: 3 };
        match runtime().block_on(a.send(&ev)) {
            Ok(()) => (),
            Err(e) => panic!("the alert was not submitted: {}", e),
        }
        let _ = jh.join();

        let transcript = transcript(&seen);
        assert!(transcript.contains("AUTH PLAIN"),
            "the alerter must authenticate to the provider:\n{}", transcript);
        assert!(transcript.contains("MAIL FROM:<steel@example.com>"),
            "envelope sender missing:\n{}", transcript);
        assert!(transcript.contains("RCPT TO:<operator@elsewhere.example>"),
            "envelope recipient missing:\n{}", transcript);
        assert!(transcript.contains("SEALED at start -- 3 database(s) shut"),
            "the event did not reach the message:\n{}", transcript);
    }

    fn texting(to: &str) -> SmsAlertConfig {
        SmsAlertConfig {
            enabled:    true,
            to:         vec![to.to_string()],
            ..Default::default()
        }
    }

    /// An alerter with or without each channel. Nothing is dialled by one of these.
    fn alerter_with(mail: bool, sms: bool) -> Alerter {
        let cfg = AlertConfig {
            enabled:    true,
            from:       if mail { fmt!("steel@example.com") } else { String::new() },
            to:         if mail { vec![fmt!("operator@example.com")] } else { Vec::new() },
            sms:        if sms { Some(texting("+61400000000")) } else { None },
            ..Default::default()
        };
        match Alerter::new(cfg, "karri".to_string(), Vec::new(), None) {
            Ok(Some(a)) => a,
            other => panic!("alerter: {:?}", other.is_err()),
        }
    }

    fn jarrah_down() -> AlertEvent {
        AlertEvent::PeerDown {
            peer:       fmt!("jarrah"),
            url:        fmt!("https://daimond.oxedyne.com/api/health"),
            failures:   3,
            down_secs:  180,
            noticed_by: fmt!("karri"),
        }
    }

    fn failing(channel: Channel) -> AlertEvent {
        AlertEvent::ChannelFailing {
            channel,
            reason:     fmt!("refused"),
            missed:     fmt!("[steel:karri] jarrah IS DOWN (3m)"),
            failures:   1,
            since_secs: 0,
        }
    }

    // A text refused for want of credit, in the shape ClickSend answered with from 2026-08-31:
    // success for the call, a refusal for the message.
    const UNFUNDED: &[u8] = br#"{"http_code":200,"response_code":"SUCCESS",
        "response_msg":"Messages queued for delivery.","data":{"total_price":0,"total_count":1,
        "queued_count":0,"messages":[{"direction":"out","to":"+61400000000",
        "body":"jarrah is DOWN","from":"","message_id":"4C1F2D3E","message_parts":1,
        "message_price":"0.0000","status":"INSUFFICIENT_CREDIT"}]}}"#;
    const ACCEPTED: &[u8] = br#"{"http_code":200,"response_code":"SUCCESS","data":{"messages":[
        {"message_id":"ABC-123","status":"SUCCESS","message_parts":1,"message_price":"0.0790"}]}}"#;

    /// The failure path from the gateway's own answer. An unfunded account's refusal, read by
    /// the parser the text leg uses, fails the alert and names the number and the vendor's
    /// status, and the report it is owed carries those words by mail.
    #[test]
    fn test_a_refused_text_fails_the_alert_and_is_reported_by_mail_00() {
        let subject = "[steel:karri] jarrah IS DOWN (3m)";
        let refused = SmsProvider::ClickSend.parse(200, UNFUNDED);
        assert!(refused.is_err(), "an unfunded account's answer read as a receipt");
        let words = match tally_texts(
            SmsProvider::ClickSend, vec![(fmt!("+61400000000"), refused)], subject)
        {
            Ok(()) => panic!("a refused text was counted as sent"),
            Err(e) => e.plain(),
        };
        assert!(words.contains("1 of 1") && words.contains("+61400000000")
            && words.contains("INSUFFICIENT_CREDIT"),
            "the failure must name the number and the vendor's status: {}", words);

        // One refusal among two numbers still fails the alert: that number's person was not told.
        let outcomes = vec![
            (fmt!("+61400000001"), SmsProvider::ClickSend.parse(200, ACCEPTED)),
            (fmt!("+61400000000"), SmsProvider::ClickSend.parse(200, UNFUNDED)),
        ];
        match tally_texts(SmsProvider::ClickSend, outcomes, subject) {
            Ok(()) => panic!("a refusal among several numbers was counted as sent"),
            Err(e) => assert!(e.plain().contains("1 of 2"), "{}", e.plain()),
        }
        let taken = vec![(fmt!("+61400000001"), SmsProvider::ClickSend.parse(200, ACCEPTED))];
        assert!(tally_texts(SmsProvider::ClickSend, taken, subject).is_ok());

        // The report goes by mail, never by the channel that refused, and says why.
        let both = alerter_with(true, true);
        let mut book = ChannelBook::default();
        let report = match book.note(Channel::Sms, Some(&words), subject, Instant::now()) {
            Some(r) => r,
            None => panic!("the first refusal of a run owes a report"),
        };
        assert_eq!(both.route(&report), Route { mail: true, sms: false });
        let body = report.body("karri");
        assert!(body.contains("INSUFFICIENT_CREDIT") && body.contains("jarrah IS DOWN"),
            "the report must carry the vendor's words and the alert that was lost:\n{}", body);
    }

    /// A channel's run of failures is told when it starts, once a day while it lasts, and when
    /// it ends -- not once per failure, and not for the other channel.
    #[test]
    fn test_a_channel_run_is_told_once_reminded_daily_and_closed_00() {
        let mut book = ChannelBook::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let why = "clicksend refused the message: INSUFFICIENT_CREDIT";

        assert!(book.note(Channel::Sms, None, "x", t0).is_none(), "a working channel owes nothing");
        match book.note(Channel::Sms, Some(why), "[steel:karri] jarrah IS DOWN (3m)", at(60)) {
            Some(AlertEvent::ChannelFailing {
                channel: Channel::Sms, reason, missed, failures: 1, since_secs: 0,
            }) => {
                assert_eq!(reason, why);
                assert_eq!(missed, "[steel:karri] jarrah IS DOWN (3m)");
            },
            other => panic!("the first failure must be reported, got {:?}", other),
        }
        for i in 2..10u64 {
            assert!(book.note(Channel::Sms, Some(why), "x", at(60 + i * 900)).is_none(),
                "failure {} of a run already told was reported again", i);
        }
        assert!(book.note(Channel::Mail, None, "x", at(9_000)).is_none(),
            "mail working says nothing about the texts");
        match book.note(Channel::Sms, Some(why), "y", at(60 + 86_400)) {
            Some(AlertEvent::ChannelFailing { failures: 10, since_secs: 86_400, missed, .. }) =>
                assert_eq!(missed, "y", "a reminder names the latest alert lost"),
            other => panic!("a day on, a failure must remind, got {:?}", other),
        }
        assert!(book.note(Channel::Sms, Some(why), "z", at(60 + 86_401)).is_none());
        match book.note(Channel::Sms, None, "w", at(60 + 90_000)) {
            Some(AlertEvent::ChannelRestored { channel: Channel::Sms, lost: 11, were_secs: 90_000 }) => (),
            other => panic!("the first success after a run must close it, got {:?}", other),
        }
        assert!(book.note(Channel::Sms, None, "v", at(60 + 90_060)).is_none(), "the run is over");
    }

    /// Mail when there is a recipient, a text for what is critical, and never the channel whose
    /// failure is the news. With no mail, a notice has nowhere to go.
    #[test]
    fn test_each_event_goes_by_the_channels_it_should_00() {
        let both = alerter_with(true, true);
        let texts_only = alerter_with(false, true);
        let distress = AlertEvent::PeerDistress {
            peer:       fmt!("jarrah"),
            url:        fmt!("https://oxedyne.com/_steel/health"),
            classes:    fmt!("swap_pct 31"),
            since_secs: 180,
            noticed_by: fmt!("conifer"),
        };
        let restored = AlertEvent::ChannelRestored { channel: Channel::Sms, lost: 3, were_secs: 600 };

        assert_eq!(both.route(&jarrah_down()), Route { mail: true, sms: true });
        assert_eq!(both.route(&distress), Route { mail: true, sms: false });
        assert_eq!(both.route(&failing(Channel::Sms)), Route { mail: true, sms: false },
            "failing texts are told by mail");
        assert_eq!(both.route(&failing(Channel::Mail)), Route { mail: false, sms: true },
            "failing mail is told by text");
        assert_eq!(both.route(&restored), Route { mail: true, sms: false });

        assert_eq!(texts_only.route(&jarrah_down()), Route { mail: false, sms: true });
        assert!(texts_only.route(&distress).is_empty());
        assert!(texts_only.route(&AlertEvent::SealedStart { db_count: 1 }).is_empty());
        assert!(texts_only.route(&failing(Channel::Sms)).is_empty());
    }

    /// An event with no channel is logged as undelivered and goes no further. There is no
    /// runtime here, so a delivery task spawned for it would panic.
    #[test]
    fn test_an_event_with_no_channel_is_not_sent_anywhere_00() {
        let texts_only = alerter_with(false, true);
        texts_only.raise(AlertEvent::SealedStart { db_count: 2 });
    }

    /// THE PATH THAT WAS MISSING: a text that fails is reported by mail. The text leg here
    /// fails for want of an outbound TLS client, a failure met before any gateway is asked. The
    /// alert still goes by mail, and a second message follows it saying the texts are failing,
    /// why, and which alert was lost.
    #[test]
    fn test_a_failed_text_is_reported_by_mail_00() {
        let (addr, seen, jh) = stand_in_provider(2);
        let a = alerter_via(addr, Some(texting("+61400000000")));
        let subjects = |seen: &Arc<Mutex<Vec<String>>>| -> usize {
            match seen.lock() {
                Ok(g) => g.iter().filter(|l| l.starts_with("Subject:")).count(),
                Err(_) => 0,
            }
        };
        runtime().block_on(async {
            a.raise(AlertEvent::PeerDown {
                peer:       fmt!("birch"),
                url:        fmt!("https://oregami.oxegen.io/health"),
                failures:   3,
                down_secs:  180,
                noticed_by: fmt!("conifer"),
            });
            let deadline = Instant::now() + Duration::from_secs(20);
            while subjects(&seen) < 2 && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        let transcript = transcript(&seen);
        assert!(transcript.contains("Subject: [steel:example.com] birch IS DOWN (3m)"),
            "the alert itself must still go by mail:\n{}", transcript);
        assert!(transcript.contains("Subject: [steel:example.com] text alerts FAILING (1 in a row)"),
            "the failed text must be reported by mail:\n{}", transcript);
        assert!(transcript.contains("no outbound TLS client"),
            "the report must say why:\n{}", transcript);
        assert!(transcript.contains("Last lost: [steel:example.com] birch IS DOWN (3m)"),
            "the report must name the alert that was lost:\n{}", transcript);
        let _ = jh.join();
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
