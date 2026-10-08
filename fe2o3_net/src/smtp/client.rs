//! Client-side SMTP, for the two different conversations a sender can have.
//!
//! **Delivery** ([`OutboundClient::deliver`]) is what a mail server does: look up the recipient
//! domain's MX, connect to the best-preference exchange on port 25, EHLO, opportunistic STARTTLS,
//! then MAIL/RCPT/DATA. Nobody authenticates -- the receiving server accepts the mail because it is
//! responsible for the recipient, not because it knows the sender.
//!
//! **Submission** ([`OutboundClient::submit`]) is what a mail *client* does, and it is a different
//! conversation with a different party: connect to the account holder's own provider on the
//! submission port, and prove who you are before the provider will carry anything. Without it a
//! sender can only talk to servers that already wanted the message; with it, a sender can post mail
//! through the account it holds a password for, which is how every desktop mail client works.
//!
//! No queue, no retry policy, no exponential backoff -- the caller is
//! expected to drive retries itself by enqueueing the message in a
//! spool directory and re-invoking the client. Keeps the abstraction
//! useful for both a "fire and forget" path and a real queue runner.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    addr::is_publicly_routable,
    dns_resolver,
    imap::client::Security,
    smtp::server::read_line,
    tls::{
        self,
        ClientStream,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
    },
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};

use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::rustls::ClientConfig;


// Generous, because some receiving MX hosts greylist or impose multi-second waits before 220.
// It bounds each step of a conversation, not the whole of it.
pub const SMTP_CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

// The reply to the final "." may take this long whatever the step deadline, as RFC 5321
// §4.5.3.2.6 allows. A sender that gives up while the receiver is still accepting sends the
// message again on its next try, and the receiver ends up with both.
const DATA_DONE_TIMEOUT: Duration = Duration::from_secs(600);

// Once the message is accepted, QUIT is a courtesy, and no step of it waits longer than this. A
// whole step spent on it could run out a caller's own deadline and report an accepted message as
// failed, which is a false alarm and, where the caller retries, a duplicate.
const QUIT_TIMEOUT: Duration = Duration::from_secs(2);

// The message goes over in pieces of this size, each held to the deadline on its own, so a large
// message on a slow link is bounded by its progress rather than cut off for its size.
const SEND_PIECE: usize = 64 * 1024;


/// Where to post a message, and how to prove you may.
///
/// The provider's submission service, not a recipient's MX: the host is the one the account lives
/// on, and the credential is the account's own. Port 587 conventionally starts in the clear and
/// upgrades with `STARTTLS`; port 465 is TLS from the first byte.
#[derive(Clone, Debug)]
pub struct SubmissionConfig {
    pub host:       String,     // also the name the certificate is validated against
    pub port:       u16,        // conventionally 587 (STARTTLS) or 465 (implicit TLS)
    pub security:   Security,
    pub user:       String,     // usually, but not always, the address being sent from
    // For a provider with two-factor authentication this is an application password, not the
    // password the human types into a browser.
    pub password:   String,
    // Each step: the connect, the TLS handshake, each command and each reply. The reply to the
    // message itself may take DATA_DONE_TIMEOUT where that is longer, and QUIT no more than
    // QUIT_TIMEOUT.
    pub timeout:    Duration,
    // Dialled instead of resolving `host`. The certificate is still validated against `host`, so
    // pinning the address weakens nothing -- and a server connecting on behalf of a user must vet
    // the address it dials rather than hand the name to the resolver twice.
    pub addr:       Option<SocketAddr>,
}

impl SubmissionConfig {

    /// The conventional deadline, and no pinned address.
    pub fn new(
        host:       impl Into<String>,
        port:       u16,
        security:   Security,
        user:       impl Into<String>,
        password:   impl Into<String>,
    )
        -> Self
    {
        Self {
            host:       host.into(),
            port,
            security,
            user:       user.into(),
            password:   password.into(),
            timeout:    SMTP_CLIENT_TIMEOUT,
            addr:       None,
        }
    }

    pub fn with_addr(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}


/// The addresses to try for a domain, from the A records of its mail exchanges, looked up with
/// `lookup`.
///
/// Failing to find one is permanent only where the DNS said so for good: the domain publishes a null
/// MX (RFC 7505), or every exchange is a name that does not exist (NXDOMAIN). An exchange that exists
/// but has no A record may be an IPv6-only host, which this client cannot reach and a later one might,
/// and a lookup that failed outright says nothing about the host, so both leave the failure transient.
/// Mail to a dead domain used to be retried for ever because none of this was told apart.
fn route<L>(mxs: &[dns_resolver::MxRecord], lookup: L) -> Outcome<Vec<DeliveryTarget>>
    where
        L: Fn(&str) -> Outcome<Vec<Ipv4Addr>>,
{
    if mxs.iter().all(|mx| mx.exchange.is_empty()) {
        return Err(err!(
            "The recipient domain publishes a null MX and accepts no mail (RFC 7505).";
            IO, Network, Missing, Permanent));
    }
    let mut targets: Vec<DeliveryTarget> = Vec::new();
    // Whether some exchange is not shown to be gone.
    let mut open = false;
    for mx in mxs {
        if mx.exchange.is_empty() {
            continue;
        }
        match lookup(&mx.exchange) {
            Ok(addrs) if addrs.is_empty()   => open = true,
            Ok(addrs)                       => {
                for ip in addrs {
                    targets.push(DeliveryTarget {
                        host:       mx.exchange.clone(),
                        addr:       IpAddr::V4(ip),
                        port:       25,
                        preference: mx.preference,
                    });
                }
            }
            Err(e) if is_permanent(&e)      => (),
            Err(_)                          => open = true,
        }
    }
    if !targets.is_empty() {
        Ok(targets)
    } else if open {
        Err(err!(
            "No reachable MX hosts for any of the configured recipients.";
            IO, Network, Missing))
    } else {
        Err(err!(
            "No mail exchange of the recipient domain exists (NXDOMAIN).";
            IO, Network, Missing, Permanent))
    }
}

/// One outbound delivery target after MX resolution, sorted into preference order by
/// [`OutboundClient::deliver`].
#[derive(Clone, Debug)]
struct DeliveryTarget {
    host:       String,     // MX exchange
    addr:       IpAddr,
    // Always 25 for a resolved exchange, which is the only port mail is delivered on. It is a
    // field rather than a literal so a fixture can stand in as an exchange on a loopback port;
    // nothing reads it from configuration and nothing should.
    port:       u16,
    preference: u16,        // as the MX record gave it
}

/// Which mail exchanges one delivery keeps away from, and the one it last dialled.
///
/// A caller that has found an exchange not answering (see [`OutboundClient::exchanges`]) names it by
/// address in [`Self::avoiding`], and `deliver_with` goes on to the next exchange of the recipient's
/// domain as though it were not listed. The delivery notes each exchange as it dials it, so a caller
/// that gives up on the delivery while it is under way, by dropping it at a deadline, still knows which
/// exchange it was waiting on. One `Dial` serves one delivery.
#[derive(Debug, Default)]
pub struct Dial {
    avoid:  Vec<IpAddr>,
    last:   Mutex<Option<IpAddr>>,
}

impl Dial {

    /// A dial that never connects to the exchanges at `avoid`.
    pub fn avoiding(avoid: Vec<IpAddr>) -> Self {
        Self { avoid, last: Mutex::new(None) }
    }

    /// Is the exchange at `ip` one this delivery keeps away from?
    pub fn skips(&self, ip: IpAddr) -> bool {
        self.avoid.contains(&ip)
    }

    /// Notes that the exchange at `ip` is the one now being dialled. A courier that is not
    /// [`OutboundClient`] calls this for each exchange it tries, so the caller can tell which one held
    /// a delivery up.
    pub fn dialling(&self, ip: IpAddr) -> Outcome<()> {
        let mut last = lock_mutex!(self.last);
        *last = Some(ip);
        Ok(())
    }

    /// The exchange most recently dialled, or `None` where none has been.
    pub fn last(&self) -> Outcome<Option<IpAddr>> {
        let last = lock_mutex!(self.last);
        Ok(*last)
    }
}

/// Per-process outbound SMTP client.
///
/// Holds a rustls `ClientConfig` initialised with the system trust
/// anchors so STARTTLS to any public MX validates correctly. Cheap to
/// clone -- the inner config is in an `Arc`.
#[derive(Clone)]
pub struct OutboundClient {
    // Sent in EHLO. Should be the public hostname of the sending server, the one that owns the
    // IP whose PTR lines up.
    pub hostname:       Arc<String>,
    pub tls_config:     Arc<ClientConfig>,  // for STARTTLS, built once
    // Whether `deliver` may dial an exchange whose address is not publicly routable. Off by
    // default, and only a fixture on the loopback, or a development host, has a reason to set it.
    pub allow_private_exchanges: bool,
}

impl OutboundClient {

    /// STARTTLS validation goes against the system CA bundle. A caller needing a custom root
    /// store builds the `ClientConfig` itself.
    pub fn with_system_roots(hostname: impl Into<String>) -> Outcome<Self> {
        let cfg = res!(Self::default_tls_config());
        Ok(Self {
            hostname:   Arc::new(hostname.into()),
            tls_config: Arc::new(cfg),
            allow_private_exchanges: false,
        })
    }

    /// The host's CA bundle, through [`crate::tls::default_client_config`], which every protocol
    /// client in this crate shares.
    pub fn default_tls_config() -> Outcome<ClientConfig> {
        tls::default_client_config()
    }

    /// Each MX in preference order until one succeeds. The queue id is the first accepting
    /// server's; where every host failed, the error is the last one's.
    ///
    /// An exchange at an address that is not publicly routable (see
    /// [`crate::addr::is_publicly_routable`]) is never dialled unless `allow_private_exchanges` is
    /// set. Where no exchange is left the error is permanent. Submission to a configured relay
    /// ([`Self::submit`]) is not filtered.
    pub async fn deliver(
        &self,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
    )
        -> Outcome<String>
    {
        self.deliver_with(mail_from, rcpt_to, body, &Dial::default()).await
    }

    /// [`Self::deliver`], keeping away from the exchanges `dial` names and noting in it each exchange
    /// it dials. Where every exchange that is left is one `dial` avoids, nothing is dialled and the
    /// failure is transient.
    pub async fn deliver_with(
        &self,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
        dial:       &Dial,
    )
        -> Outcome<String>
    {
        if rcpt_to.is_empty() {
            return Err(err!(
                "OutboundClient::deliver called with no recipients.";
                Invalid, Input, Missing));
        }

        // Group recipients by domain so each domain delivery is one
        // SMTP transaction. The MVP only handles the common case of
        // every recipient sharing one domain.
        let domain = res!(extract_domain(&rcpt_to[0]));
        for r in rcpt_to.iter().skip(1) {
            let other = res!(extract_domain(r));
            if !other.eq_ignore_ascii_case(&domain) {
                return Err(err!(
                    "OutboundClient::deliver: multi-domain delivery is \
                    not supported in the MVP (got '{}' and '{}').",
                    domain, other;
                    Invalid, Input));
            }
        }

        let targets = res!(Self::resolve(domain).await);
        self.deliver_to_exchanges(&targets, mail_from, rcpt_to, body, SMTP_CLIENT_TIMEOUT, dial).await
    }

    /// The addresses of the exchanges a message to `rcpt_to` would be dialled at, in preference order,
    /// found by the lookups `deliver` makes and without dialling any. An exchange this client would
    /// never dial, being at an address that is not publicly routable, is left out, so the list may be
    /// empty. A caller uses it to learn which exchanges it has found not answering before it sends.
    pub async fn exchanges(&self, rcpt_to: &str) -> Outcome<Vec<IpAddr>> {
        let domain = res!(extract_domain(rcpt_to));
        let mut targets = res!(Self::resolve(domain).await);
        targets.retain(|t| self.dialable(t));
        targets.sort_by_key(|t| t.preference);
        let mut ips: Vec<IpAddr> = Vec::new();
        for t in &targets {
            if !ips.contains(&t.addr) {
                ips.push(t.addr);
            }
        }
        Ok(ips)
    }

    // The exchanges of a domain: its MX records, each looked up for its A records.
    async fn resolve(domain: String) -> Outcome<Vec<DeliveryTarget>> {
        let mxs = res!(
            tokio::task::spawn_blocking(move || dns_resolver::lookup_mx(&domain)).await
                .map_err(|e| err!("MX lookup task join failure: {}.", e;
                    IO, Network, Init))
        );
        let mxs = res!(mxs);

        let targets = res!(
            tokio::task::spawn_blocking(move || route(&mxs, dns_resolver::lookup_a)).await
                .map_err(|e| err!("Exchange lookup task join failure: {}.", e;
                    IO, Network, Init))
        );
        targets
    }

    // Whether this client may dial the exchange: not at a loopback, private or otherwise unroutable
    // address, unless `allow_private_exchanges` is set.
    fn dialable(&self, t: &DeliveryTarget) -> bool {
        self.allow_private_exchanges || is_publicly_routable(&t.addr)
    }

    /// The delivery loop itself, given the exchanges rather than resolving them.
    ///
    /// Split out of [`Self::deliver`] for one reason: everything above this point needs a live MX
    /// lookup, so the loop below -- preference order, which failures are worth another exchange, and
    /// whether the collapsed error is permanent -- could not be exercised at all. It carries
    /// jarrah's outbound mail and had no test until 2026-08-17. Private, and takes the exchanges as
    /// an argument rather than reading them from anywhere: this is not a way to configure where mail
    /// goes, it is a way for a fixture to stand in as an exchange. `deadline` bounds each step of
    /// each conversation, and is an argument so a fixture need not wait a minute to see one pass. `dial`
    /// names the exchanges to keep away from and notes each one dialled.
    async fn deliver_to_exchanges(
        &self,
        targets:    &[DeliveryTarget],
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
        deadline:   Duration,
        dial:       &Dial,
    )
        -> Outcome<String>
    {
        if targets.is_empty() {
            return Err(err!(
                "No reachable MX hosts for any of the configured recipients.";
                IO, Network, Missing));
        }
        let mut targets: Vec<DeliveryTarget> = targets.to_vec();
        if !self.allow_private_exchanges {
            // An exchange whose address is loopback, private or otherwise unroutable is never
            // dialled, whatever name led to it: a record that points mail at the sender's own
            // network is no exchange for the recipient. Nothing a later try finds will differ,
            // so where nothing is left the failure is permanent and the address is suppressed.
            targets.retain(|t| {
                let routable = is_publicly_routable(&t.addr);
                if !routable {
                    warn!("Outbound SMTP: MX {} ({}) is not publicly routable and is not dialled.",
                        t.host, t.addr);
                }
                routable
            });
            if targets.is_empty() {
                let domain = rcpt_to.first()
                    .and_then(|r| extract_domain(r).ok())
                    .unwrap_or_default();
                return Err(err!(
                    "Every mail exchange of '{}' is at an address that is not publicly routable, \
                    so nothing was dialled (MX routing).", domain;
                    IO, Network, Security, Permanent));
            }
        }
        targets.sort_by_key(|t| t.preference);
        // An exchange the caller has found not answering is left for another time, so a delivery is
        // not held up again by the host that held up the last. Where every one is left, nothing has
        // been shown wrong with the recipient, and the failure is transient.
        let count = targets.len();
        targets.retain(|t| !dial.skips(t.addr));
        if targets.is_empty() {
            return Err(err!(
                "Every one of the {} mail exchanges is being avoided, so nothing was dialled.", count;
                IO, Network));
        }

        let mut last_err: Option<String> = None;
        // Whether any exchange refused this recipient with a 5xx. A permanent rejection -- an unknown
        // mailbox, a blocked sender -- is authoritative for the domain, so retrying another exchange or a
        // later sweep will not cure it, and the caller wants to suppress the address rather than keep
        // trying. A 4xx, a timeout or a connection error is transient and carries no such tag.
        let mut permanent = false;
        for tgt in &targets {
            res!(dial.dialling(tgt.addr));
            match self.try_one(tgt, mail_from, rcpt_to, body, deadline).await {
                Ok(qid) => return Ok(qid),
                Err(e) => {
                    if is_permanent(&e) {
                        permanent = true;
                    }
                    let msg = fmt!("MX {} ({}): {}", tgt.host, tgt.addr, e);
                    warn!("Outbound SMTP attempt failed: {}", msg);
                    last_err = Some(msg);
                }
            }
        }
        let last = last_err.unwrap_or_else(|| "(none)".to_string());
        // The permanence is carried on the collapsed error so a single failed `deliver` still tells the
        // caller whether to retry the address or give up on it, without unpicking the wrapped causes.
        if permanent {
            Err(err!(
                "All MX delivery attempts failed; last error: {}", last;
                IO, Network, Permanent))
        } else {
            Err(err!(
                "All MX delivery attempts failed; last error: {}", last;
                IO, Network))
        }
    }

    /// The conversation a mail client has, not the one a mail server has: the provider
    /// carries the message because the sender proved they hold the account, so the credential is
    /// not optional and neither is the encryption under it. The client refuses to send the password
    /// over a connection it could not secure -- a provider that offers no TLS on its submission
    /// port is not one a password may be spoken to, and failing loudly is the only safe answer.
    ///
    /// Unlike delivery, `rcpt_to` may span any number of domains: the provider, not this client,
    /// works out where each one goes. What comes back is whatever the provider said on accepting
    /// the message, which usually carries its queue id.
    pub async fn submit(
        &self,
        cfg:        &SubmissionConfig,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
    )
        -> Outcome<String>
    {
        if rcpt_to.is_empty() {
            return Err(err!(
                "OutboundClient::submit called with no recipients.";
                Invalid, Input, Missing));
        }

        let addr = match cfg.addr {
            Some(a) => a,
            None => {
                let host = cfg.host.clone();
                let ips = res!(
                    tokio::task::spawn_blocking(move || dns_resolver::lookup_a(&host)).await
                        .map_err(|e| err!("Submission host lookup task join failure: {}.", e;
                            IO, Network, Init))
                );
                let ips = res!(ips);
                match ips.first() {
                    Some(ip) => SocketAddr::new(IpAddr::V4(*ip), cfg.port),
                    None => return Err(err!(
                        "The submission host {} resolves to no address.", cfg.host;
                        IO, Network, Missing)),
                }
            },
        };

        let mut conv = res!(Conversation::connect(addr, &cfg.host, cfg.timeout).await);

        // TLS from the first byte, or in the clear until STARTTLS lifts it.
        if cfg.security == Security::ImplicitTls {
            conv = res!(conv.upgrade(self.tls_config.clone()).await);
        }

        let banner = res!(conv.reply().await);
        if banner.code != 220 {
            return Err(err!(
                "Expected a 220 banner from {}, got {} {}", cfg.host, banner.code, banner.text;
                IO, Network, Wire));
        }

        let mut ehlo = res!(self.ehlo(&mut conv).await);

        if cfg.security == Security::StartTls {
            let offered = ehlo.text.lines().any(|l| l.trim().eq_ignore_ascii_case("STARTTLS"));
            if !offered {
                return Err(err!(
                    "{} does not offer STARTTLS, so the account password cannot be sent to it \
                    without being readable on the wire.", cfg.host;
                    IO, Network, Invalid));
            }
            res!(conv.command("STARTTLS").await);
            let resp = res!(conv.reply().await);
            if resp.code != 220 {
                return Err(err!(
                    "{} refused STARTTLS: {} {}", cfg.host, resp.code, resp.text;
                    IO, Network, Wire));
            }
            conv = res!(conv.upgrade(self.tls_config.clone()).await);
            // The extension list before the upgrade cannot be trusted, and AUTH is usually only
            // offered after it, so ask again inside TLS.
            ehlo = res!(self.ehlo(&mut conv).await);
        }

        if cfg.security == Security::Plain {
            warn!("Submitting to {} without TLS: the account password will cross the wire in \
                the clear. Only a loopback test server should ever be reached this way.", cfg.host);
        }

        res!(authenticate(&mut conv, &ehlo, &cfg.user, &cfg.password).await);
        let queue_id = res!(transact(&mut conv, mail_from, rcpt_to, body).await);

        conv.quit().await;
        Ok(queue_id)
    }

    async fn ehlo(&self, conv: &mut Conversation) -> Outcome<SmtpResponse> {
        res!(conv.command(&fmt!("EHLO {}", self.hostname)).await);
        let resp = res!(conv.reply().await);
        if resp.code != 250 {
            return Err(err!(
                "EHLO rejected: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        Ok(resp)
    }

    async fn try_one(
        &self,
        tgt:        &DeliveryTarget,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
        deadline:   Duration,
    )
        -> Outcome<String>
    {
        let addr = SocketAddr::new(tgt.addr, tgt.port);
        let mut conv = res!(Conversation::connect(addr, &tgt.host, deadline).await);

        // Read the 220 banner.
        let banner = res!(conv.reply().await);
        if banner.code != 220 {
            return Err(err!(
                "Expected 220 banner, got {} {}", banner.code, banner.text;
                IO, Network, Wire));
        }

        // EHLO, then look at extensions.
        let ehlo = res!(self.ehlo(&mut conv).await);
        let supports_starttls = ehlo.text.lines().any(|l| {
            l.trim().eq_ignore_ascii_case("STARTTLS")
        });

        // Opportunistic STARTTLS.
        if supports_starttls {
            res!(conv.command("STARTTLS").await);
            let resp = res!(conv.reply().await);
            if resp.code == 220 {
                conv = res!(conv.upgrade(self.tls_config.clone()).await);

                // Re-issue EHLO inside TLS.
                res!(conv.command(&fmt!("EHLO {}", self.hostname)).await);
                let _ = res!(conv.reply().await);
            }
        }

        let queue_id = res!(transact(&mut conv, mail_from, rcpt_to, body).await);

        conv.quit().await;
        Ok(queue_id)
    }
}

/// One SMTP conversation: the stream, and the deadline every step on it is held to.
///
/// Every read, write and handshake goes through here, so no step can wait for ever. Until
/// 2026-09-24 only the TCP connect was timed, and a server that took the connection and never
/// spoke held `submit` for as long as the socket lived, whatever its timeout said -- and with it
/// the text an alert sends beside its mail (D-06 audit A2).
struct Conversation {
    stream:     ClientStream,
    peer:       String,     // named in errors, and the name a certificate is validated against
    timeout:    Duration,   // per step
}

impl Conversation {

    async fn connect(addr: SocketAddr, peer: &str, deadline: Duration) -> Outcome<Self> {
        let plain = match timeout(deadline, TcpStream::connect(addr)).await {
            Ok(Ok(s))  => s,
            Ok(Err(e)) => return Err(err!(e,
                "Connecting to {} at {}.", peer, addr; IO, Network)),
            Err(_)     => return Err(err!(
                "Timeout connecting to {} at {}.", peer, addr; IO, Network, Timeout)),
        };
        Ok(Self { stream: ClientStream::Plain(plain), peer: peer.to_string(), timeout: deadline })
    }

    /// TLS over the plain stream, from the first byte or once STARTTLS has been agreed.
    async fn upgrade(self, tls_config: Arc<ClientConfig>) -> Outcome<Self> {
        let plain = match self.stream.into_plain() {
            Some(s) => s,
            None => return Err(err!(
                "A TLS upgrade was asked of the stream to {}, which is already encrypted.",
                self.peer; Invalid, Bug)),
        };
        let stream = res!(tls::upgrade(plain, &self.peer, tls_config, self.timeout).await);
        Ok(Self { stream, peer: self.peer, timeout: self.timeout })
    }

    /// All of one reply, however many lines it runs to, within the step deadline.
    async fn reply(&mut self) -> Outcome<SmtpResponse> {
        let deadline = self.timeout;
        self.reply_within(deadline).await
    }

    async fn reply_within(&mut self, deadline: Duration) -> Outcome<SmtpResponse> {
        match timeout(deadline, read_smtp_response(&mut self.stream)).await {
            Ok(r)  => r,
            Err(_) => Err(err!(
                "{} sent no reply within {:?}.", self.peer, deadline;
                IO, Network, Read, Timeout)),
        }
    }

    /// Ends the conversation once the message is accepted. The message is the receiver's by now,
    /// so a QUIT that stalls or fails is logged and changes nothing.
    async fn quit(mut self) {
        self.timeout = self.timeout.min(QUIT_TIMEOUT);
        let said = match self.command("QUIT").await {
            Ok(()) => self.reply().await.map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = said {
            debug!("{} accepted the message and then did not see QUIT through; the message \
                stands: {}", self.peer, e);
        }
    }

    /// CRLF-terminated, and flushed. The command is never named in an error, since AUTH carries
    /// the credential.
    async fn command(&mut self, cmd: &str) -> Outcome<()> {
        self.send(fmt!("{}\r\n", cmd).as_bytes(), "a command").await
    }

    /// Written and flushed, each piece within the deadline.
    async fn send(&mut self, bytes: &[u8], what: &str) -> Outcome<()> {
        for piece in bytes.chunks(SEND_PIECE) {
            match timeout(self.timeout, self.stream.write_all(piece)).await {
                Ok(Ok(()))  => (),
                Ok(Err(e))  => return Err(err!(e,
                    "Writing {} to {}.", what, self.peer; IO, Network, Write)),
                Err(_)      => return Err(err!(
                    "{} took no more of {} within {:?}.", self.peer, what, self.timeout;
                    IO, Network, Write, Timeout)),
            }
        }
        match timeout(self.timeout, self.stream.flush()).await {
            Ok(Ok(()))  => Ok(()),
            Ok(Err(e))  => Err(err!(e,
                "Flushing {} to {}.", what, self.peer; IO, Network, Write)),
            Err(_)      => Err(err!(
                "{} took no more of {} within {:?}.", self.peer, what, self.timeout;
                IO, Network, Write, Timeout)),
        }
    }
}

/// Walk MAIL/RCPT/DATA on a stream that is already open, secured and (where the server demands it)
/// authenticated. Delivery and submission differ in how they reach this point and not at all in
/// what they do once they are here, so they share the transaction rather than each keeping a copy
/// of it -- the second copy is where the dot-stuffing gets forgotten.
async fn transact(
    conv:       &mut Conversation,
    mail_from:  &str,
    rcpt_to:    &[String],
    body:       &[u8],
)
    -> Outcome<String>
{
    res!(conv.command(&fmt!("MAIL FROM:<{}>", mail_from)).await);
    let resp = res!(conv.reply().await);
    if resp.code / 100 != 2 {
        // A 5xx is a permanent refusal, tagged so the caller can suppress rather than retry; a 4xx is
        // transient and carries no such tag.
        if resp.code / 100 == 5 {
            return Err(err!(
                "MAIL FROM rejected: {} {}", resp.code, resp.text;
                IO, Network, Wire, Permanent));
        }
        return Err(err!(
            "MAIL FROM rejected: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }
    for r in rcpt_to {
        res!(conv.command(&fmt!("RCPT TO:<{}>", r)).await);
        let resp = res!(conv.reply().await);
        if resp.code / 100 != 2 {
            // A 5xx here is the no-such-mailbox case: permanent for this recipient, so it is tagged for
            // suppression. A 4xx (greylisting, a full mailbox) is transient and is not.
            if resp.code / 100 == 5 {
                return Err(err!(
                    "RCPT TO:<{}> rejected: {} {}", r, resp.code, resp.text;
                    IO, Network, Wire, Permanent));
            }
            return Err(err!(
                "RCPT TO:<{}> rejected: {} {}", r, resp.code, resp.text;
                IO, Network, Wire));
        }
    }
    res!(conv.command("DATA").await);
    let resp = res!(conv.reply().await);
    if resp.code != 354 {
        return Err(err!(
            "DATA rejected: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }

    // A line of the body that begins with a full stop would otherwise end the message, and the
    // terminator goes on a line of its own however the body ends.
    let mut stuffed = dot_stuff(body);
    if !body.ends_with(b"\r\n") {
        stuffed.extend_from_slice(b"\r\n");
    }
    stuffed.extend_from_slice(b".\r\n");
    res!(conv.send(&stuffed, "the message").await);

    // The receiver may be filtering the message before it answers, and giving up on it here is
    // how it comes to be delivered twice.
    let done = conv.timeout.max(DATA_DONE_TIMEOUT);
    let resp = res!(conv.reply_within(done).await);
    if resp.code / 100 != 2 {
        // A 5xx on the message itself -- refused content, a policy block -- will not be cured by resending
        // the same message, so it is tagged permanent for the caller to suppress on.
        if resp.code / 100 == 5 {
            return Err(err!(
                "Server rejected message: {} {}", resp.code, resp.text;
                IO, Network, Wire, Permanent));
        }
        return Err(err!(
            "Server rejected message: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }
    Ok(resp.text)
}

/// Is this a permanent rejection -- a 5xx from the receiving server -- that retrying will not
/// cure?
///
/// The one predicate a caller needs to tell "this address is bad, suppress it" from "the network
/// hiccupped, try again later". A permanent failure is tagged [`ErrTag::Permanent`] where the 5xx is
/// read off the wire in [`transact`], so this reads the tag rather than parse a status code out of a
/// message.
///
/// The tag is read from the whole chain, as [`Error::tags`] gathers it. `res!` wraps a cause in an
/// `Error::Upstream` carrying **no tags of its own**, so reading only the outer frame, as this did
/// until 2026-08-17, made the predicate answer `false` to every permanent failure there has ever
/// been: `submit` and `try_one` each pass `transact`'s error through one `res!`. Nothing was
/// suppressed, and `fe2o3_steel`'s subscriber list kept mailing addresses their servers had refused
/// outright.
pub fn is_permanent(e: &Error<ErrTag>) -> bool {
    e.tags().contains(&ErrTag::Permanent)
}

/// Prove to the provider that the sender holds the account.
///
/// `PLAIN` is preferred and `LOGIN` accepted, because between them they are what every provider
/// worth submitting through offers. Both hand over the password in base64, which is an encoding and
/// not a protection -- the only thing keeping it safe is the TLS underneath, which is why the
/// caller establishes that first and refuses to proceed without it. `ehlo` is the extension list
/// the server advertised inside TLS.
async fn authenticate(
    conv:       &mut Conversation,
    ehlo:       &SmtpResponse,
    user:       &str,
    password:   &str,
)
    -> Outcome<()>
{
    let mut mechanisms: Vec<String> = Vec::new();
    for line in ehlo.text.lines() {
        let l = line.trim();
        if l.len() >= 4 && l[..4].eq_ignore_ascii_case("AUTH") {
            for m in l[4..].split_whitespace() {
                mechanisms.push(m.to_uppercase());
            }
        }
    }
    if mechanisms.is_empty() {
        return Err(err!(
            "The server offers no AUTH mechanism, so there is no way to prove the account is \
            ours and it will not carry the message. It advertised: {}",
            ehlo.text.replace('\n', " | ");
            IO, Network, Missing));
    }

    if mechanisms.iter().any(|m| m == "PLAIN") {
        // RFC 4616: an authorisation identity we leave empty, then the account, then the password,
        // each separated by a NUL.
        let raw = fmt!("\0{}\0{}", user, password);
        let cmd = fmt!("AUTH PLAIN {}", base64::encode(raw.as_bytes()));
        res!(conv.command(&cmd).await);
        let resp = res!(conv.reply().await);
        return check_auth(&resp);
    }

    if mechanisms.iter().any(|m| m == "LOGIN") {
        res!(conv.command("AUTH LOGIN").await);
        let resp = res!(conv.reply().await);
        if resp.code != 334 {
            return Err(err!(
                "AUTH LOGIN was refused before the username: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        res!(conv.command(&base64::encode(user.as_bytes())).await);
        let resp = res!(conv.reply().await);
        if resp.code != 334 {
            return Err(err!(
                "The server rejected the username: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        res!(conv.command(&base64::encode(password.as_bytes())).await);
        let resp = res!(conv.reply().await);
        return check_auth(&resp);
    }

    Err(err!(
        "The server offers only {}, and this client can prove itself with PLAIN or LOGIN.",
        mechanisms.join(", ");
        IO, Network, Unimplemented))
}

/// Read the server's verdict on a login attempt, saying what a rejection usually means rather than
/// only that it happened. A wrong password and a password the provider will not accept from a
/// program look identical on the wire, and the second is the common case.
fn check_auth(resp: &SmtpResponse) -> Outcome<()> {
    if resp.code / 100 == 2 {
        return Ok(());
    }
    if resp.code == 535 || resp.code == 534 {
        return Err(err!(
            "The provider rejected the credential ({} {}). If the account has two-factor \
            authentication, an ordinary password will always be refused here and an application \
            password is required.", resp.code, resp.text;
            Invalid, Input, Unauthorised));
    }
    Err(err!(
        "Authentication failed: {} {}", resp.code, resp.text;
        IO, Network, Wire))
}

/// One parsed SMTP server response, potentially multi-line.
#[derive(Clone, Debug)]
struct SmtpResponse {
    code: u16,
    text: String,   // the text lines, joined by '\n'
}

/// A response ends at the line whose fourth byte is a space rather than a hyphen. Untimed, so it
/// is read only through [`Conversation::reply`].
async fn read_smtp_response(stream: &mut ClientStream) -> Outcome<SmtpResponse> {
    let mut text = String::new();
    let mut code: u16 = 0;
    loop {
        let line = match res!(read_line(stream).await) {
            Some(l) => l,
            None => return Err(err!(
                "Connection closed while reading SMTP response.";
                IO, Network, Read)),
        };
        if line.len() < 4 {
            return Err(err!(
                "SMTP response line too short: '{}'.", line;
                Invalid, Input, Decode));
        }
        let code_str = &line[..3];
        let sep = line.as_bytes()[3];
        let parsed: u16 = match code_str.parse() {
            Ok(n) => n,
            Err(_) => return Err(err!(
                "SMTP response code '{}' not numeric.", code_str;
                Invalid, Input, Decode)),
        };
        if code == 0 {
            code = parsed;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line[4..]);
        if sep == b' ' {
            break;
        }
        if sep != b'-' {
            return Err(err!(
                "SMTP response line has invalid separator: '{}'.", line;
                Invalid, Input, Decode));
        }
    }
    Ok(SmtpResponse { code, text })
}

/// RFC 5321 §4.5.2: any line whose first character is `.` gets a second one prepended, so the
/// receiver does not mistake it for the message terminator.
fn dot_stuff(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + body.len() / 64);
    let mut at_line_start = true;
    for &b in body {
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        at_line_start = b == b'\n';
    }
    out
}

/// Lowercased, from the last `@`.
fn extract_domain(addr: &str) -> Outcome<String> {
    match addr.rfind('@') {
        Some(i) => Ok(addr[i + 1..].to_lowercase()),
        None => Err(err!(
            "Address '{}' has no '@'.", addr;
            Invalid, Input, Mismatch)),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

// These drive the whole conversation against a stand-in provider on loopback, not
// the pieces of it in isolation. They live in the module rather than in `tests/`
// because `tests/main.rs` gates every case behind a single `filter` string, and on
// 2026-08-17 that filter was `"dns"` -- so the four submission cases in
// `tests/smtp_submit.rs` had never once run, while the harness reported them green.
// A test that cannot be switched off by editing one word somewhere else is worth
// more than a tidier home for it.
#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use tokio::{
        io::{
            AsyncBufReadExt,
            AsyncReadExt,
            BufReader,
        },
        net::TcpListener,
    };


    const USER: &str = "alice@example.com";
    const PASS: &str = "app-password-not-a-real-one";
    const EHLO: &str = "sender.test";
    const WAIT: Duration = Duration::from_secs(10);     // each step, where nothing should stall


    /// What the stand-in server does when it is spoken to.
    ///
    /// It poses as a submission provider for [`OutboundClient::submit`] and as an MX exchange for
    /// [`OutboundClient::deliver_to_exchanges`], which is honest: SMTP is the same protocol on both
    /// sides of the difference, and the difference is entirely in what the client does with it.
    #[derive(Clone, Copy)]
    struct Provider {
        mechs:      &'static str,   // advertised after `AUTH`; empty for no `AUTH` line at all
        starttls:   bool,           // advertise `STARTTLS` in the EHLO reply
        // Whether a `STARTTLS` command is answered 220. There is no TLS behind this fixture, so a
        // test that advertises the extension answers 454 -- which is the opportunistic case worth
        // covering anyway: a client offered TLS and refused it must deliver in the clear rather
        // than give up on the exchange.
        starttls_ok: bool,
        auth_ok:    bool,
        banner:     u16,            // 220 is ready for mail; 421 refuses the connection
        rcpt_code:  u16,            // 250 accepts the recipient
        data_code:  u16,            // 250 accepts the message
        stall:      &'static str,   // the command after which it goes silent; empty for never
        slow_done:  Duration,       // how long the reply to the final "." is held back
    }

    impl Provider {
        /// Offers both mechanisms this client can speak, and takes everything.
        fn accepting() -> Self {
            Self {
                mechs:       "PLAIN LOGIN",
                starttls:    false,
                starttls_ok: false,
                auth_ok:     true,
                banner:      220,
                rcpt_code:   250,
                data_code:   250,
                stall:       "",
                slow_done:   Duration::ZERO,
            }
        }

        /// An exchange, which advertises no `AUTH`: a delivering client is not expected to prove
        /// anything, and must not try.
        fn exchange() -> Self {
            Self { mechs: "", ..Self::accepting() }
        }

        fn ehlo_reply(&self) -> String {
            let mut out = fmt!("250-provider.example.com\r\n250-SIZE 35882577\r\n");
            if self.starttls {
                out.push_str("250-STARTTLS\r\n");
            }
            if !self.mechs.is_empty() {
                out.push_str(&fmt!("250-AUTH {}\r\n", self.mechs));
            }
            out.push_str("250 8BITMIME\r\n");
            out
        }
    }

    /// Every line the provider was sent, in order, including the DATA body.
    type Transcript = Arc<Mutex<Vec<String>>>;

    async fn provider(p: Provider) -> Outcome<(SocketAddr, Transcript)> {
        provider_on("127.0.0.1", p).await
    }

    // As `provider`, listening on another loopback address, so two exchanges can differ by address.
    async fn provider_on(host: &str, p: Provider) -> Outcome<(SocketAddr, Transcript)> {
        let listener = res!(TcpListener::bind(fmt!("{}:0", host)).await
            .map_err(|e| err!(e, "Binding the stand-in provider."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading the stand-in provider's address."; IO, Network)));

        let seen: Transcript = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();

        tokio::spawn(async move {
            let (sock, _) = match listener.accept().await {
                Ok(x)  => x,
                Err(_) => return,
            };
            let (r, mut w) = sock.into_split();
            let mut lines = BufReader::new(r).lines();

            let _ = w.write_all(fmt!("{} provider.example.com ESMTP\r\n",
                p.banner).as_bytes()).await;

            let mut in_data    = false;
            let mut await_user = false;
            let mut await_pass = false;
            let mut silent     = false;
            let verdict = if p.auth_ok {
                &b"235 2.7.0 Accepted\r\n"[..]
            } else {
                &b"535 5.7.8 Username and Password not accepted\r\n"[..]
            };

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut g) = log.lock() {
                    g.push(line.clone());
                }
                if !p.stall.is_empty() && line.to_uppercase().starts_with(p.stall) {
                    silent = true;
                }
                if silent {
                    continue;   // reads on, and answers nothing
                }
                if in_data {
                    if line == "." {
                        in_data = false;
                        tokio::time::sleep(p.slow_done).await;
                        let _ = w.write_all(fmt!("{} 2.0.0 Ok: queued as STANDIN1\r\n",
                            p.data_code).as_bytes()).await;
                    }
                    continue;
                }
                if await_user {
                    await_user = false;
                    await_pass = true;
                    let _ = w.write_all(b"334 UGFzc3dvcmQ6\r\n").await;      // "Password:"
                    continue;
                }
                if await_pass {
                    await_pass = false;
                    let _ = w.write_all(verdict).await;
                    continue;
                }

                let up = line.to_uppercase();
                if up.starts_with("EHLO") {
                    let _ = w.write_all(p.ehlo_reply().as_bytes()).await;
                } else if up.starts_with("STARTTLS") {
                    let _ = w.write_all(if p.starttls_ok {
                        &b"220 Go ahead\r\n"[..]
                    } else {
                        &b"454 4.7.0 TLS not available at the moment\r\n"[..]
                    }).await;
                } else if up.starts_with("AUTH PLAIN") {
                    let _ = w.write_all(verdict).await;
                } else if up.starts_with("AUTH LOGIN") {
                    await_user = true;
                    let _ = w.write_all(b"334 VXNlcm5hbWU6\r\n").await;      // "Username:"
                } else if up.starts_with("MAIL FROM") {
                    let _ = w.write_all(b"250 2.1.0 Ok\r\n").await;
                } else if up.starts_with("RCPT TO") {
                    let _ = w.write_all(fmt!("{} recipient\r\n", p.rcpt_code).as_bytes()).await;
                } else if up.starts_with("DATA") {
                    in_data = true;
                    let _ = w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await;
                } else if up.starts_with("QUIT") {
                    let _ = w.write_all(b"221 2.0.0 Bye\r\n").await;
                    return;
                } else {
                    let _ = w.write_all(b"250 2.0.0 Ok\r\n").await;
                }
            }
        });

        Ok((addr, seen))
    }

    fn cfg(addr: SocketAddr, security: Security) -> SubmissionConfig {
        SubmissionConfig::new("provider.example.com", addr.port(), security, USER, PASS)
            .with_addr(addr)
            .with_timeout(Duration::from_secs(10))
    }

    /// One line deliberately begins with a full stop, so dot-stuffing is under
    /// test in every case that gets as far as the body.
    fn body() -> Vec<u8> {
        let mut s = String::new();
        s.push_str("From: Alice <alice@example.com>\r\n");
        s.push_str("To: Bob <bob@example.net>\r\n");
        s.push_str("Subject: Hello\r\n");
        s.push_str("\r\n");
        s.push_str("A line.\r\n");
        s.push_str(".A line that begins with a full stop.\r\n");
        s.into_bytes()
    }

    fn lines_of(t: &Transcript) -> Outcome<Vec<String>> {
        match t.lock() {
            Ok(g)  => Ok(g.clone()),
            Err(_) => Err(err!("The provider's transcript was poisoned."; Lock, Poisoned)),
        }
    }

    /// Whether the password reached the provider, in the clear or base64, on any
    /// line. The only safe answer for a conversation that never authenticated.
    fn password_crossed(lines: &[String]) -> bool {
        for l in lines {
            if l.contains(PASS) {
                return true;
            }
            if let Ok(raw) = base64::decode(l.trim()) {
                if String::from_utf8_lossy(&raw).contains(PASS) {
                    return true;
                }
            }
            // AUTH PLAIN carries it inside the command.
            if let Some(b64) = l.split_whitespace().last() {
                if let Ok(raw) = base64::decode(b64) {
                    if String::from_utf8_lossy(&raw).contains(PASS) {
                        return true;
                    }
                }
            }
        }
        false
    }

    // The stand-in exchanges listen on the loopback, which a default client will not dial.
    async fn client() -> Outcome<OutboundClient> {
        let mut c = res!(OutboundClient::with_system_roots(EHLO));
        c.allow_private_exchanges = true;
        Ok(c)
    }

    // ── The submission conversation ───────────────────────────────

    /// The whole exchange, in order, with the credential in it: this is the shape
    /// every caller of `submit` depends on and none of them can see.
    #[tokio::test]
    async fn test_submission_speaks_the_conversation_in_order_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        let qid = res!(c.submit(
            &cfg(addr, Security::Plain),
            "alice@example.com",
            &[fmt!("bob@example.net")],
            &body(),
        ).await);
        req!(true, qid.contains("STANDIN1"), "the provider's queue id must come back");

        let lines = res!(lines_of(&seen));
        // Position matters: EHLO before AUTH, AUTH before MAIL FROM, and the
        // terminator after the body. A client that authenticates after offering
        // the envelope has told the provider who it is too late.
        let at = |want: &str| -> Option<usize> {
            lines.iter().position(|l| l.to_uppercase().starts_with(want))
        };
        let ehlo = res!(at("EHLO").ok_or_else(|| err!("No EHLO was sent."; Test, Missing)));
        let auth = res!(at("AUTH").ok_or_else(|| err!("No AUTH was sent."; Test, Missing)));
        let mail = res!(at("MAIL FROM").ok_or_else(|| err!("No MAIL FROM."; Test, Missing)));
        let rcpt = res!(at("RCPT TO").ok_or_else(|| err!("No RCPT TO."; Test, Missing)));
        let data = res!(at("DATA").ok_or_else(|| err!("No DATA."; Test, Missing)));
        req!(true, ehlo < auth, "AUTH came before EHLO");
        req!(true, auth < mail, "the envelope was offered before the login");
        req!(true, mail < rcpt, "RCPT TO came before MAIL FROM");
        req!(true, rcpt < data, "DATA came before RCPT TO");
        req!(true, lines.iter().any(|l| l == "."), "the DATA terminator never arrived");
        req!(true, lines.iter().any(|l| l.to_uppercase().starts_with("QUIT")),
            "the client did not say QUIT");
        Ok(())
    }

    /// `PLAIN` is offered first and must be the one chosen, with an empty
    /// authorisation identity, the account, then the password, NUL-separated.
    #[tokio::test]
    async fn test_auth_plain_encodes_the_credential_as_rfc4616_asks_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await);

        let lines = res!(lines_of(&seen));
        let auth = res!(lines.iter()
            .find(|l| l.to_uppercase().starts_with("AUTH PLAIN"))
            .ok_or_else(|| err!("The client never sent AUTH PLAIN: {:?}", lines;
                Test, Missing)));
        let b64 = auth["AUTH PLAIN ".len()..].trim().to_string();
        let raw = res!(base64::decode(&b64));
        req!(fmt!("\0{}\0{}", USER, PASS).into_bytes(), raw);
        Ok(())
    }

    /// A provider offering only `LOGIN` must be spoken to, not given up on: the
    /// account and the password go over on their own lines, base64 and nothing
    /// else.
    #[tokio::test]
    async fn test_auth_login_is_the_fallback_when_plain_is_absent_00() -> Outcome<()> {
        let p = Provider { mechs: "LOGIN", ..Provider::accepting() };
        let (addr, seen) = res!(provider(p).await);
        let c = res!(client().await);
        let qid = res!(c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await);
        req!(true, qid.contains("STANDIN1"));

        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH LOGIN")));
        req!(true, lines.iter().any(|l| base64::decode(l.trim())
            .map(|b| b == USER.as_bytes()).unwrap_or(false)),
            "the account never went over");
        req!(true, lines.iter().any(|l| base64::decode(l.trim())
            .map(|b| b == PASS.as_bytes()).unwrap_or(false)),
            "the password never went over");
        Ok(())
    }

    /// RFC 5321 §4.5.2 on the wire, not in a unit test of `dot_stuff`: a body line
    /// beginning with a full stop must arrive doubled, or it ends the message and
    /// the rest of it is read as commands.
    #[tokio::test]
    async fn test_a_leading_full_stop_arrives_doubled_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await);

        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.starts_with("..A line that begins")),
            "the line was not stuffed: {:?}", lines);
        // And the single-dot terminator is still exactly one dot.
        req!(1, lines.iter().filter(|l| *l == ".").count());
        Ok(())
    }

    /// A body that does not end in CRLF still gets a terminator on a line of its
    /// own, rather than one glued to the last line of the message.
    #[tokio::test]
    async fn test_a_body_without_a_trailing_crlf_still_terminates_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            b"Subject: x\r\n\r\nno trailing newline").await);

        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l == "no trailing newline"),
            "the last body line was mangled: {:?}", lines);
        req!(true, lines.iter().any(|l| l == "."), "no terminator on its own line");
        Ok(())
    }

    /// Submission may span domains -- the provider works out where each goes --
    /// and every recipient must get its own RCPT TO.
    #[tokio::test]
    async fn test_submission_offers_every_recipient_across_domains_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(
            &cfg(addr, Security::Plain),
            USER,
            &[fmt!("bob@example.net"), fmt!("carol@elsewhere.example"), fmt!("dan@third.test")],
            &body(),
        ).await);

        let lines = res!(lines_of(&seen));
        for who in ["bob@example.net", "carol@elsewhere.example", "dan@third.test"] {
            req!(true, lines.iter().any(|l| l == &fmt!("RCPT TO:<{}>", who)),
                "{} was not offered", who);
        }
        req!(3, lines.iter().filter(|l| l.to_uppercase().starts_with("RCPT TO")).count());
        Ok(())
    }

    // ── Refusals, and what must not have happened first ───────────

    /// A refused credential is an error, and the message must not be offered
    /// anyway. The provider said no; sending on regardless is how a message is
    /// reported sent and never arrives.
    #[tokio::test]
    async fn test_a_refused_credential_stops_the_conversation_00() -> Outcome<()> {
        let p = Provider { auth_ok: false, ..Provider::accepting() };
        let (addr, seen) = res!(provider(p).await);
        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await;
        if out.is_ok() {
            return Err(err!(
                "The provider refused the credential and the client reported success.";
                Test, Invalid));
        }
        // The 535 case names the application-password fix, because a wrong
        // password and a password the provider will not take from a program are
        // indistinguishable on the wire and the second is the common one.
        let msg = match out {
            Err(e) => fmt!("{}", e),
            Ok(_)  => String::new(),
        };
        req!(true, msg.to_lowercase().contains("application"),
            "the refusal did not name the fix: {}", msg);
        req!(false, msg.contains(PASS), "the password leaked into the error: {}", msg);

        let lines = res!(lines_of(&seen));
        req!(false, lines.iter().any(|l| l.to_uppercase().starts_with("MAIL FROM")),
            "the client offered the envelope after its login was rejected");
        Ok(())
    }

    /// A provider offering only a mechanism this client cannot speak gets no
    /// password at all -- not an attempt, and not a plaintext fallback.
    #[tokio::test]
    async fn test_an_unspeakable_mechanism_never_sees_the_password_00() -> Outcome<()> {
        let p = Provider { mechs: "XOAUTH2 GSSAPI", ..Provider::accepting() };
        let (addr, seen) = res!(provider(p).await);
        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await;
        if out.is_ok() {
            return Err(err!(
                "The client claimed to submit through a provider whose only mechanisms \
                it cannot speak."; Test, Invalid));
        }
        let lines = res!(lines_of(&seen));
        req!(false, password_crossed(&lines),
            "the password crossed the wire anyway: {:?}", lines);
        Ok(())
    }

    /// A provider advertising no `AUTH` line at all is refused for the same
    /// reason, and the message with it.
    #[tokio::test]
    async fn test_a_provider_offering_no_auth_is_refused_00() -> Outcome<()> {
        let p = Provider { mechs: "", ..Provider::accepting() };
        let (addr, seen) = res!(provider(p).await);
        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await;
        req!(true, out.is_err(), "a provider that cannot be authenticated to was used");
        let lines = res!(lines_of(&seen));
        req!(false, password_crossed(&lines));
        req!(false, lines.iter().any(|l| l.to_uppercase().starts_with("MAIL FROM")));
        Ok(())
    }

    /// `STARTTLS` was asked for and the provider does not offer it. The password
    /// would cross in the clear, so nothing crosses at all -- and the error says
    /// so, rather than only that something failed.
    #[tokio::test]
    async fn test_starttls_absent_means_the_password_is_withheld_00() -> Outcome<()> {
        let p = Provider { starttls: false, ..Provider::accepting() };
        let (addr, seen) = res!(provider(p).await);
        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::StartTls), USER,
            &[fmt!("bob@example.net")], &body()).await;
        let msg = match out {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "The client submitted a password to a provider offering no TLS.";
                Test, Invalid)),
        };
        req!(true, msg.contains("STARTTLS"), "the error did not name STARTTLS: {}", msg);
        let lines = res!(lines_of(&seen));
        req!(false, password_crossed(&lines),
            "the password crossed a connection that could not be secured: {:?}", lines);
        req!(false, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH")),
            "the client began to authenticate anyway");
        Ok(())
    }

    // ── Permanence, which is what a caller retries or suppresses on ──

    /// A 5xx on a recipient is authoritative: the caller suppresses the address
    /// rather than sweeping it again forever. This is the only thing
    /// [`is_permanent`] is for, and the only way a caller can tell the two apart.
    #[tokio::test]
    async fn test_a_5xx_recipient_refusal_is_permanent_00() -> Outcome<()> {
        let p = Provider { rcpt_code: 550, ..Provider::accepting() };
        let (addr, _) = res!(provider(p).await);
        let c = res!(client().await);
        match c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("nobody@example.net")], &body()).await
        {
            Ok(_)  => Err(err!("A 550 on RCPT TO was reported as a send."; Test, Invalid)),
            Err(e) => {
                req!(true, is_permanent(&e),
                    "a 550 was not tagged permanent, so the caller will retry it forever: {}", e);
                Ok(())
            },
        }
    }

    /// A 4xx is the greylisting case, and carries no such tag: retried later it
    /// usually succeeds, and suppressing the address would lose the mail.
    #[tokio::test]
    async fn test_a_4xx_recipient_refusal_is_transient_00() -> Outcome<()> {
        let p = Provider { rcpt_code: 451, ..Provider::accepting() };
        let (addr, _) = res!(provider(p).await);
        let c = res!(client().await);
        match c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await
        {
            Ok(_)  => Err(err!("A 451 on RCPT TO was reported as a send."; Test, Invalid)),
            Err(e) => {
                req!(false, is_permanent(&e),
                    "a 451 was tagged permanent, so the caller will suppress a good \
                    address: {}", e);
                Ok(())
            },
        }
    }

    /// And the same distinction on the message itself, where a policy block is
    /// permanent and a full mailbox is not.
    #[tokio::test]
    async fn test_a_refused_message_carries_its_permanence_00() -> Outcome<()> {
        let (addr, _) = res!(provider(Provider { data_code: 552, ..Provider::accepting() }).await);
        let c = res!(client().await);
        match c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await
        {
            Ok(_)  => return Err(err!("A 552 was reported as a send."; Test, Invalid)),
            Err(e) => req!(true, is_permanent(&e), "a 552 on the message was not permanent: {}", e),
        }

        let (addr, _) = res!(provider(Provider { data_code: 452, ..Provider::accepting() }).await);
        match c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await
        {
            Ok(_)  => Err(err!("A 452 was reported as a send."; Test, Invalid)),
            Err(e) => {
                req!(false, is_permanent(&e), "a 452 was tagged permanent: {}", e);
                Ok(())
            },
        }
    }

    // ── Wire shape ────────────────────────────────────────────────

    /// A multi-line greeting or EHLO reply is one response, and the client must
    /// read to the line whose fourth byte is a space rather than stopping at the
    /// first.
    #[tokio::test]
    async fn test_a_multiline_ehlo_is_read_as_one_response_00() -> Outcome<()> {
        // `Provider::ehlo_reply` sends four lines, three continued. Reaching AUTH
        // at all proves the mechanism list was read out of the last-but-one.
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH PLAIN")),
            "the mechanism list was not read out of the continued EHLO reply");
        Ok(())
    }

    /// The EHLO name the caller configured is the one that goes over: it is what
    /// a receiving server checks against the sending IP's PTR.
    #[tokio::test]
    async fn test_the_configured_ehlo_name_is_the_one_sent_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), USER, &[fmt!("bob@example.net")],
            &body()).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l == &fmt!("EHLO {}", EHLO)),
            "EHLO did not carry the configured name: {:?}", lines);
        Ok(())
    }

    /// The envelope sender is the address the caller gave, angle-bracketed, and
    /// not the login -- the two differ whenever a person sends as an alias.
    #[tokio::test]
    async fn test_the_envelope_sender_is_not_the_login_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        res!(c.submit(&cfg(addr, Security::Plain), "alias@example.com",
            &[fmt!("bob@example.net")], &body()).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l == "MAIL FROM:<alias@example.com>"),
            "the envelope sender was rewritten: {:?}", lines);
        Ok(())
    }

    /// Nothing at all happens without a recipient: no connection, no banner read,
    /// no credential offered to a conversation that cannot carry a message.
    #[tokio::test]
    async fn test_no_recipient_is_refused_before_dialling_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(Provider::accepting()).await);
        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::Plain), USER, &[], &body()).await;
        req!(true, out.is_err(), "a message with no recipient was submitted");
        req!(true, res!(lines_of(&seen)).is_empty(),
            "the client dialled a provider for a message it could not send");
        Ok(())
    }

    /// A greeting that is not a 220 is not a provider ready to take mail, and the
    /// client must stop there rather than talk over it.
    #[tokio::test]
    async fn test_a_refused_banner_stops_before_ehlo_00() -> Outcome<()> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding a refusing provider."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading its address."; IO, Network)));
        let seen: Transcript = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            if let Ok((sock, _)) = listener.accept().await {
                let (r, mut w) = sock.into_split();
                let _ = w.write_all(b"554 no service here\r\n").await;
                let mut buf = Vec::new();
                let mut r = r;
                let _ = r.read_to_end(&mut buf).await;
                if let Ok(mut g) = log.lock() {
                    g.push(String::from_utf8_lossy(&buf).into_owned());
                }
            }
        });

        let c = res!(client().await);
        let out = c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await;
        let msg = match out {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A 554 greeting was treated as a provider ready for mail."; Test, Invalid)),
        };
        req!(true, msg.contains("220"), "the error did not say what was expected: {}", msg);
        let said = res!(lines_of(&seen)).join("");
        req!(false, said.to_uppercase().contains("EHLO"),
            "the client talked over a refusing banner: {:?}", said);
        Ok(())
    }

    // ── The pieces, where the whole conversation cannot reach them ──

    #[test]
    fn test_dot_stuff_doubles_only_at_a_line_start_00() -> Outcome<()> {
        req!(b"..a\r\n".to_vec(),        dot_stuff(b".a\r\n"));
        req!(b"a.b\r\n".to_vec(),        dot_stuff(b"a.b\r\n"));
        req!(b"x\r\n..y\r\n".to_vec(),   dot_stuff(b"x\r\n.y\r\n"));
        // Two dots on their own line become three: the receiver unstuffs one.
        req!(b"...\r\n".to_vec(),        dot_stuff(b"..\r\n"));
        Ok(())
    }

    #[test]
    fn test_extract_domain_takes_the_last_at_00() -> Outcome<()> {
        req!(fmt!("example.com"), res!(extract_domain("a@example.com")));
        req!(fmt!("example.com"), res!(extract_domain("A@Example.COM")));
        // A quoted local part may itself contain an '@'.
        req!(fmt!("example.com"), res!(extract_domain("\"odd@name\"@example.com")));
        req!(true, extract_domain("no-at-sign").is_err());
        Ok(())
    }

    /// The tag survives being wrapped. `res!` builds an `Error::Upstream` with no
    /// tags of its own, so a predicate reading one frame answers `false` however
    /// many 5xx there were underneath -- which is what made this inert.
    #[test]
    fn test_permanence_survives_a_wrapping_frame_00() -> Outcome<()> {
        let inner: Error<ErrTag> = err!(
            "RCPT TO:<nobody@example.net> rejected: 550 no such mailbox";
            IO, Network, Wire, Permanent);
        req!(true, is_permanent(&inner), "the tag is not read at the innermost frame");

        let once = Error::Upstream(std::sync::Arc::new(inner), ErrMsg {
            tags: &[],
            msg:  errmsg!(),
        });
        req!(true, is_permanent(&once), "one wrapping frame hid the tag");

        let twice = Error::Upstream(std::sync::Arc::new(once), ErrMsg {
            tags: &[],
            msg:  errmsg!(),
        });
        req!(true, is_permanent(&twice), "two wrapping frames hid the tag");

        // And a transient failure stays transient however deep it is, or every
        // greylisted address would be suppressed.
        let soft: Error<ErrTag> = err!(
            "RCPT TO:<bob@example.net> rejected: 451 try later";
            IO, Network, Wire);
        let wrapped = Error::Upstream(std::sync::Arc::new(soft), ErrMsg {
            tags: &[],
            msg:  errmsg!(),
        });
        req!(false, is_permanent(&wrapped), "a 451 was read as permanent");
        Ok(())
    }

    /// Delivery is one SMTP transaction per domain, and the MVP does one domain.
    /// Refusing loudly beats delivering to the first domain and dropping the rest.
    #[tokio::test]
    async fn test_delivery_refuses_a_mixed_domain_envelope_00() -> Outcome<()> {
        let c = res!(client().await);
        req!(true, c.deliver("a@example.com", &[], b"x").await.is_err(),
            "delivery with no recipient was accepted");
        req!(true, c.deliver("a@example.com",
            &[fmt!("b@one.example"), fmt!("c@two.example")], b"x").await.is_err(),
            "delivery accepted two domains in one transaction");
        Ok(())
    }

    // ── A domain with nowhere to deliver to ──

    fn mx(exchange: &str) -> dns_resolver::MxRecord {
        dns_resolver::MxRecord { preference: 10, exchange: exchange.to_string() }
    }

    /// What the resolver says of a name that does not exist.
    fn nxdomain() -> Outcome<Vec<Ipv4Addr>> {
        Err(err!("The name does not exist (DNS RCODE 3, NXDOMAIN)."; IO, Network, Missing, Permanent))
    }

    /// RFC 7505. The two stale messages on karri were stuck behind this: the domain says it takes
    /// no mail, and the sender read that as a reason to ask again in thirty seconds, for ever. No
    /// exchange is looked up, because there is none.
    #[test]
    fn test_a_null_mx_is_a_permanent_failure_00() -> Outcome<()> {
        match route(&[mx("")], |_| Err(err!("a null MX was looked up."; IO, Network))) {
            Ok(t) => Err(err!("A null MX gave targets: {:?}.", t; Test, Mismatch)),
            Err(e) => {
                req!(true, is_permanent(&e), "a null MX was read as transient");
                Ok(())
            }
        }
    }

    /// NXDOMAIN for every exchange: the name is not there, so the domain has nowhere to deliver.
    #[test]
    fn test_exchanges_that_do_not_exist_are_permanent_00() -> Outcome<()> {
        match route(&[mx("mx1.gone.example"), mx("mx2.gone.example")], |_| nxdomain()) {
            Ok(t) => Err(err!("Absent exchanges gave targets: {:?}.", t; Test, Mismatch)),
            Err(e) => {
                req!(true, is_permanent(&e), "exchanges that do not exist were read as transient");
                Ok(())
            }
        }
    }

    /// NOERROR with no A record, which is also what an IPv6-only exchange looks like. The name is
    /// there, so nothing says it never will be deliverable: the message waits, it is not bounced.
    #[test]
    fn test_an_exchange_with_no_a_record_is_transient_00() -> Outcome<()> {
        match route(&[mx("mx1.v6only.example")], |_| Ok(Vec::new())) {
            Ok(t) => Err(err!("An exchange with no A record gave targets: {:?}.", t; Test, Mismatch)),
            Err(e) => {
                req!(false, is_permanent(&e), "an exchange with no A record was read as permanent");
                Ok(())
            }
        }
    }

    /// A timeout or a SERVFAIL says nothing about the domain, so the message waits.
    #[test]
    fn test_a_failed_lookup_is_not_permanent_00() -> Outcome<()> {
        match route(&[mx("mx1.slow.example")], |_| Err(err!("Timed out."; IO, Network, Timeout))) {
            Ok(t) => Err(err!("A failed lookup gave targets: {:?}.", t; Test, Mismatch)),
            Err(e) => {
                req!(false, is_permanent(&e), "a failed DNS lookup was read as permanent");
                Ok(())
            }
        }
    }

    /// One exchange gone and another that exists but has no A: the second is not shown to be gone.
    #[test]
    fn test_one_absent_exchange_does_not_condemn_the_others_00() -> Outcome<()> {
        let looked = |host: &str| if host.starts_with("gone") { nxdomain() } else { Ok(Vec::new()) };
        match route(&[mx("gone.example"), mx("mx2.v6only.example")], looked) {
            Ok(t) => Err(err!("Gave targets: {:?}.", t; Test, Mismatch)),
            Err(e) => {
                req!(false, is_permanent(&e), "an exchange with no A record was condemned with its sibling");
                Ok(())
            }
        }
    }

    /// An exchange that resolves is a target even when its sibling does not exist.
    #[test]
    fn test_a_resolving_exchange_is_a_target_00() -> Outcome<()> {
        let looked = |host: &str| if host.starts_with("gone") {
            nxdomain()
        } else {
            Ok(vec![Ipv4Addr::new(192, 0, 2, 25)])
        };
        let t = res!(route(&[mx("gone.example"), mx("mx2.up.example")], looked));
        req!(1, t.len(), "the resolving exchange was not the one target");
        req!(true, t[0].addr == IpAddr::V4(Ipv4Addr::new(192, 0, 2, 25)), "wrong address");
        req!(25, t[0].port, "mail goes to port 25");
        Ok(())
    }

    // ── Delivery to an exchange, which carries jarrah's outbound mail ──

    /// A stand-in exchange, and the target that points at it. `preference` is what
    /// the MX record would have said.
    async fn exchange_at(p: Provider, preference: u16) -> Outcome<(DeliveryTarget, Transcript)> {
        let (addr, seen) = res!(provider(p).await);
        Ok((DeliveryTarget {
            host:       fmt!("mx{}.example.net", preference),
            addr:       addr.ip(),
            port:       addr.port(),
            preference,
        }, seen))
    }

    /// The conversation a mail *server* has: no credential, because the receiving
    /// exchange takes the message for being responsible for the recipient. A
    /// delivering client that tries to log in is doing a mail client's job.
    #[tokio::test]
    async fn test_delivery_never_authenticates_00() -> Outcome<()> {
        // The exchange advertises AUTH anyway, which real ones commonly do.
        let (tgt, seen) = res!(exchange_at(Provider::accepting(), 10).await);
        let c = res!(client().await);
        let qid = res!(c.deliver_to_exchanges(&[tgt], "postmaster@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        req!(true, qid.contains("STANDIN1"));

        let lines = res!(lines_of(&seen));
        req!(false, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH")),
            "a delivering client tried to authenticate: {:?}", lines);
        req!(false, password_crossed(&lines));
        req!(true, lines.iter().any(|l| l == "MAIL FROM:<postmaster@example.com>"));
        req!(true, lines.iter().any(|l| l == "RCPT TO:<bob@example.net>"));
        req!(true, lines.iter().any(|l| l == "."), "the message was never terminated");
        // Dot-stuffing is the same on this path, and it is a different call site.
        req!(true, lines.iter().any(|l| l.starts_with("..A line that begins")));
        Ok(())
    }

    /// Preference order, which is the whole point of holding a list: the lowest
    /// number is tried first.
    ///
    /// The preferred exchange refuses at `RCPT TO` rather than at the banner,
    /// deliberately. A banner-refusing fixture records nothing, so "its transcript
    /// is empty" is satisfied by *never having been dialled* as much as by having
    /// been -- and an assertion that cannot fail proves nothing. Written that way
    /// first, this case passed with `sort_by_key` deleted.
    #[tokio::test]
    async fn test_exchanges_are_tried_in_preference_order_00() -> Outcome<()> {
        let dead = Provider { rcpt_code: 550, ..Provider::exchange() };
        let c = res!(client().await);

        // The refusing exchange is preferred, so it must be spoken to first and
        // the message must still land on the second.
        let (bad,  saw_bad)  = res!(exchange_at(dead, 10).await);
        let (good, saw_good) = res!(exchange_at(Provider::exchange(), 20).await);
        let qid = res!(c.deliver_to_exchanges(&[good.clone(), bad.clone()],
            "a@example.com", &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        req!(true, qid.contains("STANDIN1"));
        req!(true, res!(lines_of(&saw_bad)).iter().any(|l| l.starts_with("RCPT TO")),
            "the preferred exchange was skipped: it was never offered the recipient");
        req!(true, res!(lines_of(&saw_good)).iter().any(|l| l == "."),
            "the message did not reach the second exchange");

        // With the preferences swapped the good one is used first, and the
        // refusing one is never dialled at all.
        let (good, saw_good) = res!(exchange_at(Provider::exchange(), 10).await);
        let (bad,  saw_bad)  = res!(exchange_at(dead, 20).await);
        res!(c.deliver_to_exchanges(&[bad, good], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        req!(true, res!(lines_of(&saw_good)).iter().any(|l| l == "."));
        req!(true, res!(lines_of(&saw_bad)).is_empty(),
            "a less-preferred exchange was used while a better one worked");
        Ok(())
    }

    /// A stand-in exchange on a loopback address of its own, which a `Dial` can name.
    async fn exchange_on(host: &str, preference: u16) -> Outcome<(DeliveryTarget, Transcript)> {
        let (addr, seen) = res!(provider_on(host, Provider::exchange()).await);
        Ok((DeliveryTarget {
            host:       fmt!("mx{}.example.net", preference),
            addr:       addr.ip(),
            port:       addr.port(),
            preference,
        }, seen))
    }

    /// An exchange the caller has found not answering is not dialled, and the next one takes the
    /// message. Both would accept it: only the skip keeps the preferred one out, so an empty transcript
    /// there means it was never dialled, and the dial notes only the exchange that was.
    #[tokio::test]
    async fn test_a_skipped_exchange_is_not_dialled_00() -> Outcome<()> {
        let c = res!(client().await);
        let (slow, saw_slow) = res!(exchange_on("127.0.0.1", 10).await);
        let (good, saw_good) = res!(exchange_on("127.0.0.2", 20).await);
        let dial = Dial::avoiding(vec![slow.addr]);
        req!(true, dial.skips(slow.addr));
        req!(false, dial.skips(good.addr));
        let qid = res!(c.deliver_to_exchanges(&[slow.clone(), good.clone()], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &dial).await);
        req!(true, qid.contains("STANDIN1"));
        req!(true, res!(lines_of(&saw_slow)).is_empty(), "an exchange the dial avoids was dialled");
        req!(true, res!(lines_of(&saw_good)).iter().any(|l| l == "."),
            "the message did not reach the exchange that was not avoided");
        req!(Some(good.addr), res!(dial.last()), "the dial did not note the exchange it reached");
        Ok(())
    }

    /// Where every exchange is avoided nothing is dialled and the failure is transient, since nothing
    /// is shown wrong with the recipient.
    #[tokio::test]
    async fn test_every_exchange_skipped_is_a_transient_failure_00() -> Outcome<()> {
        let c = res!(client().await);
        let (a, saw_a) = res!(exchange_on("127.0.0.1", 10).await);
        let (b, saw_b) = res!(exchange_on("127.0.0.2", 20).await);
        let dial = Dial::avoiding(vec![a.addr, b.addr]);
        match c.deliver_to_exchanges(&[a, b], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &dial).await
        {
            Ok(q) => return Err(err!("A delivery avoiding every exchange gave {}.", q; Test, Mismatch)),
            Err(e) => req!(false, is_permanent(&e), "every exchange avoided was read as permanent"),
        }
        req!(true, res!(lines_of(&saw_a)).is_empty() && res!(lines_of(&saw_b)).is_empty(),
            "an exchange was dialled though all were avoided");
        req!(None::<IpAddr>, res!(dial.last()), "the dial noted an exchange that was not dialled");
        Ok(())
    }

    /// A 5xx recipient refusal is authoritative for the domain: it survives the
    /// collapse of every per-exchange error into one, so the caller suppresses the
    /// address instead of sweeping it forever. This is the path `deliver`'s own
    /// re-tagging was written for, and which never worked.
    #[tokio::test]
    async fn test_a_permanent_refusal_survives_the_collapse_00() -> Outcome<()> {
        let dead = Provider { rcpt_code: 550, ..Provider::exchange() };
        let (a, _) = res!(exchange_at(dead, 10).await);
        let (b, _) = res!(exchange_at(dead, 20).await);
        let c = res!(client().await);
        match c.deliver_to_exchanges(&[a, b], "a@example.com",
            &[fmt!("nobody@example.net")], &body(), WAIT, &Dial::default()).await
        {
            Ok(_)  => Err(err!("Two 550s were reported as a delivery."; Test, Invalid)),
            Err(e) => {
                req!(true, is_permanent(&e),
                    "a 550 from every exchange was not permanent, so the address is \
                    retried forever: {}", e);
                Ok(())
            },
        }
    }

    /// Greylisting is the common case and must not suppress anybody: a 4xx from
    /// every exchange collapses to a transient error.
    #[tokio::test]
    async fn test_a_transient_refusal_stays_transient_through_the_collapse_00() -> Outcome<()> {
        let busy = Provider { rcpt_code: 450, ..Provider::exchange() };
        let (a, _) = res!(exchange_at(busy, 10).await);
        let (b, _) = res!(exchange_at(busy, 20).await);
        let c = res!(client().await);
        match c.deliver_to_exchanges(&[a, b], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await
        {
            Ok(_)  => Err(err!("Two 450s were reported as a delivery."; Test, Invalid)),
            Err(e) => {
                req!(false, is_permanent(&e),
                    "greylisting was read as permanent, so a good address is \
                    suppressed: {}", e);
                Ok(())
            },
        }
    }

    /// One exchange refusing permanently is enough, even where another failed for
    /// a reason that carries no verdict. The address is bad; which exchange said
    /// so does not change that, and the flag must survive a later attempt.
    #[tokio::test]
    async fn test_one_permanent_refusal_among_failures_is_enough_00() -> Outcome<()> {
        let (dead, _)     = res!(exchange_at(
            Provider { rcpt_code: 550, ..Provider::exchange() }, 10).await);
        let (refusing, _) = res!(exchange_at(
            Provider { banner: 421, ..Provider::exchange() }, 20).await);
        let c = res!(client().await);
        match c.deliver_to_exchanges(&[dead, refusing], "a@example.com",
            &[fmt!("nobody@example.net")], &body(), WAIT, &Dial::default()).await
        {
            Ok(_)  => Err(err!("A 550 and a 421 were reported as a delivery."; Test, Invalid)),
            Err(e) => {
                req!(true, is_permanent(&e),
                    "the permanent refusal was lost behind a later transient one: {}", e);
                Ok(())
            },
        }
    }

    /// Opportunistic means opportunistic: an exchange that advertises `STARTTLS`
    /// and then will not do it still gets the mail, in the clear. Delivery has no
    /// credential to protect, and refusing here would silently stop mail to any
    /// exchange having a bad day with its certificate.
    #[tokio::test]
    async fn test_a_refused_starttls_still_delivers_in_the_clear_00() -> Outcome<()> {
        let p = Provider { starttls: true, starttls_ok: false, ..Provider::exchange() };
        let (tgt, seen) = res!(exchange_at(p, 10).await);
        let c = res!(client().await);
        let qid = res!(c.deliver_to_exchanges(&[tgt], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        req!(true, qid.contains("STANDIN1"), "a refused STARTTLS stopped the delivery");

        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.to_uppercase() == "STARTTLS"),
            "the offer was advertised and not taken up: {:?}", lines);
        req!(true, lines.iter().any(|l| l == "."), "the message never arrived");
        Ok(())
    }

    /// No exchanges is a named error, not a silent success. A resolver that found
    /// nothing must not look like a delivery.
    #[tokio::test]
    async fn test_no_exchange_is_a_named_failure_00() -> Outcome<()> {
        let c = res!(client().await);
        let msg = match c.deliver_to_exchanges(&[], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await
        {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "Delivery with nowhere to deliver reported success."; Test, Invalid)),
        };
        req!(true, msg.contains("No reachable MX"), "the error did not say why: {}", msg);
        Ok(())
    }

    /// Where every exchange failed, the caller gets the last one's words -- and
    /// the exchange they came from, because "delivery failed" without a host is
    /// not something an operator can act on.
    #[tokio::test]
    async fn test_a_collapsed_error_names_an_exchange_00() -> Outcome<()> {
        let (a, _) = res!(exchange_at(
            Provider { rcpt_code: 550, ..Provider::exchange() }, 10).await);
        let host = a.host.clone();
        let c = res!(client().await);
        let msg = match c.deliver_to_exchanges(&[a], "a@example.com",
            &[fmt!("nobody@example.net")], &body(), WAIT, &Dial::default()).await
        {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!("A 550 was a delivery."; Test, Invalid)),
        };
        req!(true, msg.contains(&host), "the failing exchange was not named: {}", msg);
        req!(true, msg.contains("550"), "the server's code was dropped: {}", msg);
        Ok(())
    }

    // ── Exchanges that are not publicly routable ──

    /// A target at `addr`, on the port of a stand-in that would answer if it were dialled.
    fn target_at(addr: IpAddr, port: u16) -> DeliveryTarget {
        DeliveryTarget {
            host:       fmt!("mx10.example.net"),
            addr,
            port,
            preference: 10,
        }
    }

    /// An MX that points at the sender's own network is a request forgery, or a misconfigured
    /// record: either way the message must not be offered there, and the address must not be
    /// retried. Each target here shares its port with a live stand-in, so a client that dialled
    /// would reach it, and the stand-in's transcript says whether it did.
    #[tokio::test]
    async fn test_an_exchange_in_private_space_is_refused_without_a_connection_00() -> Outcome<()> {
        let c = res!(OutboundClient::with_system_roots(EHLO));
        req!(false, c.allow_private_exchanges, "the default must be to refuse private exchanges");
        for ip in [
            "127.0.0.1",            // loopback
            "10.1.2.3",             // private
            "169.254.169.254",      // the cloud metadata service
            "fd00::1",              // unique local
            "::ffff:127.0.0.1",     // loopback, spelled as a mapped IPv6 address
        ] {
            let (stand_in, seen) = res!(provider(Provider::exchange()).await);
            let tgt = target_at(res!(ip.parse::<IpAddr>().map_err(|e|
                err!(e, "The test address {}.", ip; Test, Invalid))), stand_in.port());
            match c.deliver_to_exchanges(&[tgt], "a@example.com",
                &[fmt!("bob@example.net")], &body(), STALL, &Dial::default()).await
            {
                Ok(_)  => return Err(err!(
                    "Delivery to {} was reported as a success.", ip; Test, Invalid)),
                Err(e) => {
                    req!(true, is_permanent(&e),
                        "{} was refused as transient, so the address is retried: {}", ip, e);
                    let msg = fmt!("{}", e);
                    req!(true, msg.contains("routable"),
                        "the refusal for {} did not say why: {}", ip, msg);
                },
            }
            // The stand-in is reached by 127.0.0.1 and ::ffff:127.0.0.1 alike, so a client that
            // dialled either would have spoken to it. Give a late connection time to arrive.
            tokio::time::sleep(Duration::from_millis(100)).await;
            req!(true, res!(lines_of(&seen)).is_empty(),
                "{} was dialled although it is not publicly routable", ip);
        }
        Ok(())
    }

    /// A client that opts in may deliver to an exchange on the loopback, which is how the
    /// stand-ins of these tests, and a development host, are reached.
    #[tokio::test]
    async fn test_a_client_that_allows_private_exchanges_delivers_to_one_00() -> Outcome<()> {
        let (tgt, seen) = res!(exchange_at(Provider::exchange(), 10).await);
        let mut c = res!(OutboundClient::with_system_roots(EHLO));
        c.allow_private_exchanges = true;
        let qid = res!(c.deliver_to_exchanges(&[tgt], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        req!(true, qid.contains("STANDIN1"));
        req!(true, res!(lines_of(&seen)).iter().any(|l| l == "."), "the message never arrived");
        Ok(())
    }

    // ── A peer that stops talking ─────────────────────────────────

    /// A server that takes every connection and never says a word, as a wedged relay does while
    /// its kernel still completes the handshake. Each connection is held open, and silent.
    async fn mute_server() -> Outcome<SocketAddr> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding the mute server."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading the mute server's address."; IO, Network)));
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        Ok(addr)
    }

    /// Runs one conversation against a peer that stalls, and requires it to fail, to fail
    /// within a few deadlines rather than hang, and to say that it timed out.
    async fn fails_in_time<F>(what: &str, conversation: F) -> Outcome<()>
    where
        F: std::future::Future<Output = Outcome<String>>,
    {
        let start = std::time::Instant::now();
        let out = match timeout(Duration::from_secs(10), conversation).await {
            Ok(o)  => o,
            Err(_) => return Err(err!(
                "{}: still waiting after 10s, on a deadline of {:?}.", what, STALL;
                Test, Timeout)),
        };
        let took = start.elapsed();
        let msg = match out {
            Ok(_)  => return Err(err!(
                "{}: a peer that stopped talking was reported to have taken the message.", what;
                Test, Invalid)),
            Err(e) => fmt!("{}", e),
        };
        req!(true, took < Duration::from_secs(5),
            "{}: took {:?} to fail on a deadline of {:?}", what, took, STALL);
        req!(true, msg.contains("within"), "{}: the error did not say it timed out: {}", what, msg);
        Ok(())
    }

    const STALL: Duration = Duration::from_millis(500);

    /// THE HANG THAT WAS D-06 A2: a relay that takes the connection and never greets. `submit`
    /// waited on the banner for as long as the socket lived, whatever its timeout said, and an
    /// alert's text waited behind it. The same peer met with TLS from the first byte stalls the
    /// handshake instead, which is timed by the same deadline.
    #[tokio::test]
    async fn test_a_server_that_never_speaks_fails_submission_in_time_00() -> Outcome<()> {
        let addr = res!(mute_server().await);
        let c = res!(client().await);
        for security in [Security::Plain, Security::ImplicitTls] {
            let cfg = cfg(addr, security).with_timeout(STALL);
            res!(fails_in_time(&fmt!("{:?}", security),
                c.submit(&cfg, USER, &[fmt!("bob@example.net")], &body())).await);
        }
        Ok(())
    }

    /// A server that greets, takes the login and then goes silent is held to the same deadline:
    /// every reply is timed, not only the first.
    #[tokio::test]
    async fn test_a_server_that_stops_mid_conversation_fails_in_time_00() -> Outcome<()> {
        let (addr, seen) = res!(provider(
            Provider { stall: "MAIL FROM", ..Provider::accepting() }).await);
        let c = res!(client().await);
        let cfg = cfg(addr, Security::Plain).with_timeout(STALL);
        res!(fails_in_time("stalled at MAIL FROM",
            c.submit(&cfg, USER, &[fmt!("bob@example.net")], &body())).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH")),
            "the stall came before the login, so a later step was not tested: {:?}", lines);
        req!(false, lines.iter().any(|l| l.to_uppercase().starts_with("RCPT TO")),
            "the client talked on past a reply that never came: {:?}", lines);
        Ok(())
    }

    /// Delivery meets the same mute peer as an exchange, and is held to its deadline too.
    #[tokio::test]
    async fn test_an_exchange_that_never_speaks_fails_delivery_in_time_00() -> Outcome<()> {
        let addr = res!(mute_server().await);
        let tgt = DeliveryTarget {
            host:       fmt!("mx10.example.net"),
            addr:       addr.ip(),
            port:       addr.port(),
            preference: 10,
        };
        let c = res!(client().await);
        res!(fails_in_time("delivery",
            c.deliver_to_exchanges(&[tgt], "a@example.com", &[fmt!("bob@example.net")],
                &body(), STALL, &Dial::default())).await);
        Ok(())
    }

    // ── The ends of the transaction, which must not be cut short ──

    /// A receiver that filters the message before it answers the final "." is waited for,
    /// beyond the step deadline. Cut off at the step deadline, as it was from 4f0e16c until
    /// D-06 audit R1, a receiver that accepted late was sent the message again on every retry.
    #[tokio::test]
    async fn test_a_slow_acceptance_is_waited_for_00() -> Outcome<()> {
        let slow = Provider { slow_done: STALL * 3, ..Provider::exchange() };
        let (tgt, seen) = res!(exchange_at(slow, 10).await);
        let c = res!(client().await);
        let qid = res!(c.deliver_to_exchanges(&[tgt], "a@example.com",
            &[fmt!("bob@example.net")], &body(), STALL, &Dial::default()).await);
        req!(true, qid.contains("STANDIN1"), "the late acceptance was not read: {}", qid);
        req!(1, res!(lines_of(&seen)).iter().filter(|l| *l == ".").count());

        let (addr, _) = res!(provider(
            Provider { slow_done: STALL * 3, ..Provider::accepting() }).await);
        let cfg = cfg(addr, Security::Plain).with_timeout(STALL);
        let qid = res!(c.submit(&cfg, USER, &[fmt!("bob@example.net")], &body()).await);
        req!(true, qid.contains("STANDIN1"), "the late acceptance was not read: {}", qid);
        Ok(())
    }

    /// Once the message is accepted, a receiver that goes silent at QUIT cannot turn the send
    /// into a failure, nor hold it for a whole step: the caller would report an accepted message
    /// as lost, and send it again.
    #[tokio::test]
    async fn test_a_silent_quit_does_not_fail_an_accepted_message_00() -> Outcome<()> {
        let mute_at_quit = Provider { stall: "QUIT", ..Provider::exchange() };
        let c = res!(client().await);

        let (tgt, seen) = res!(exchange_at(mute_at_quit, 10).await);
        let start = std::time::Instant::now();
        let qid = res!(c.deliver_to_exchanges(&[tgt], "a@example.com",
            &[fmt!("bob@example.net")], &body(), WAIT, &Dial::default()).await);
        let took = start.elapsed();
        req!(true, qid.contains("STANDIN1"));
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.to_uppercase() == "QUIT"),
            "the client never said QUIT");
        req!(true, took < WAIT / 2, "delivery waited {:?} on a silent QUIT", took);

        let (addr, _) = res!(provider(
            Provider { stall: "QUIT", ..Provider::accepting() }).await);
        let start = std::time::Instant::now();
        let qid = res!(c.submit(&cfg(addr, Security::Plain), USER,
            &[fmt!("bob@example.net")], &body()).await);
        let took = start.elapsed();
        req!(true, qid.contains("STANDIN1"));
        req!(true, took < WAIT / 2, "submission waited {:?} on a silent QUIT", took);
        Ok(())
    }
}
