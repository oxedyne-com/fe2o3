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
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
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
        SocketAddr,
    },
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::rustls::ClientConfig;


// Generous, because some receiving MX hosts greylist or impose multi-second waits before 220.
pub const SMTP_CLIENT_TIMEOUT: Duration = Duration::from_secs(60);


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
    pub timeout:    Duration,   // per IO
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
}

impl OutboundClient {

    /// STARTTLS validation goes against the system CA bundle. A caller needing a custom root
    /// store builds the `ClientConfig` itself.
    pub fn with_system_roots(hostname: impl Into<String>) -> Outcome<Self> {
        let cfg = res!(Self::default_tls_config());
        Ok(Self {
            hostname:   Arc::new(hostname.into()),
            tls_config: Arc::new(cfg),
        })
    }

    /// The host's CA bundle, through [`crate::tls::default_client_config`], which every protocol
    /// client in this crate shares.
    pub fn default_tls_config() -> Outcome<ClientConfig> {
        tls::default_client_config()
    }

    /// Each MX in preference order until one succeeds. The queue id is the first accepting
    /// server's; where every host failed, the error is the last one's.
    pub async fn deliver(
        &self,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
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

        // MX lookup, then resolve each MX host to an A record.
        let mxs = res!(
            tokio::task::spawn_blocking(move || dns_resolver::lookup_mx(&domain)).await
                .map_err(|e| err!("MX lookup task join failure: {}.", e;
                    IO, Network, Init))
        );
        let mxs = res!(mxs);

        let mut targets: Vec<DeliveryTarget> = Vec::new();
        for mx in &mxs {
            let exchange = mx.exchange.clone();
            let pref = mx.preference;
            let addrs_outcome = tokio::task::spawn_blocking(move || {
                dns_resolver::lookup_a(&exchange)
            }).await;
            let addrs = match addrs_outcome {
                Ok(Ok(v)) => v,
                _ => continue,
            };
            for ip in addrs {
                targets.push(DeliveryTarget {
                    host:       mx.exchange.clone(),
                    addr:       IpAddr::V4(ip),
                    port:       25,
                    preference: pref,
                });
            }
        }
        self.deliver_to_exchanges(&targets, mail_from, rcpt_to, body).await
    }

    /// The delivery loop itself, given the exchanges rather than resolving them.
    ///
    /// Split out of [`Self::deliver`] for one reason: everything above this point needs a live MX
    /// lookup, so the loop below -- preference order, which failures are worth another exchange, and
    /// whether the collapsed error is permanent -- could not be exercised at all. It carries
    /// jarrah's outbound mail and had no test until 2026-08-17. Private, and takes the exchanges as
    /// an argument rather than reading them from anywhere: this is not a way to configure where mail
    /// goes, it is a way for a fixture to stand in as an exchange.
    async fn deliver_to_exchanges(
        &self,
        targets:    &[DeliveryTarget],
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
    )
        -> Outcome<String>
    {
        if targets.is_empty() {
            return Err(err!(
                "No reachable MX hosts for any of the configured recipients.";
                IO, Network, Missing));
        }
        let mut targets: Vec<DeliveryTarget> = targets.to_vec();
        targets.sort_by_key(|t| t.preference);

        let mut last_err: Option<String> = None;
        // Whether any exchange refused this recipient with a 5xx. A permanent rejection -- an unknown
        // mailbox, a blocked sender -- is authoritative for the domain, so retrying another exchange or a
        // later sweep will not cure it, and the caller wants to suppress the address rather than keep
        // trying. A 4xx, a timeout or a connection error is transient and carries no such tag.
        let mut permanent = false;
        for tgt in &targets {
            match self.try_one(tgt, mail_from, rcpt_to, body).await {
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

        let connect = TcpStream::connect(addr);
        let plain = match timeout(cfg.timeout, connect).await {
            Ok(Ok(s))  => s,
            Ok(Err(e)) => return Err(err!(e,
                "Connecting to the submission host {} at {}.", cfg.host, addr; IO, Network)),
            Err(_)     => return Err(err!(
                "Timeout connecting to the submission host {} at {}.", cfg.host, addr;
                IO, Network)),
        };

        // TLS from the first byte, or in the clear until STARTTLS lifts it.
        let mut stream = match cfg.security {
            Security::ImplicitTls =>
                res!(tls::upgrade(plain, &cfg.host, self.tls_config.clone()).await),
            _ => ClientStream::Plain(plain),
        };

        let banner = res!(read_smtp_response(&mut stream).await);
        if banner.code != 220 {
            return Err(err!(
                "Expected a 220 banner from {}, got {} {}", cfg.host, banner.code, banner.text;
                IO, Network, Wire));
        }

        let mut ehlo = res!(self.ehlo(&mut stream).await);

        if cfg.security == Security::StartTls {
            let offered = ehlo.text.lines().any(|l| l.trim().eq_ignore_ascii_case("STARTTLS"));
            if !offered {
                return Err(err!(
                    "{} does not offer STARTTLS, so the account password cannot be sent to it \
                    without being readable on the wire.", cfg.host;
                    IO, Network, Invalid));
            }
            res!(write_command(&mut stream, "STARTTLS").await);
            let resp = res!(read_smtp_response(&mut stream).await);
            if resp.code != 220 {
                return Err(err!(
                    "{} refused STARTTLS: {} {}", cfg.host, resp.code, resp.text;
                    IO, Network, Wire));
            }
            let plain = match stream.into_plain() {
                Some(s) => s,
                None => return Err(err!(
                    "STARTTLS response received on an already-encrypted stream.";
                    Invalid, Bug)),
            };
            stream = res!(tls::upgrade(plain, &cfg.host, self.tls_config.clone()).await);
            // The extension list before the upgrade cannot be trusted, and AUTH is usually only
            // offered after it, so ask again inside TLS.
            ehlo = res!(self.ehlo(&mut stream).await);
        }

        if cfg.security == Security::Plain {
            warn!("Submitting to {} without TLS: the account password will cross the wire in \
                the clear. Only a loopback test server should ever be reached this way.", cfg.host);
        }

        res!(authenticate(&mut stream, &ehlo, &cfg.user, &cfg.password).await);
        let queue_id = res!(transact(&mut stream, mail_from, rcpt_to, body).await);

        let _ = write_command(&mut stream, "QUIT").await;
        let _ = read_smtp_response(&mut stream).await;
        Ok(queue_id)
    }

    async fn ehlo(&self, stream: &mut ClientStream) -> Outcome<SmtpResponse> {
        res!(write_command(stream, &fmt!("EHLO {}", self.hostname)).await);
        let resp = res!(read_smtp_response(stream).await);
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
    )
        -> Outcome<String>
    {
        let addr = std::net::SocketAddr::new(tgt.addr, tgt.port);
        let connect = TcpStream::connect(addr);
        let plain = match timeout(SMTP_CLIENT_TIMEOUT, connect).await {
            Ok(Ok(s))  => s,
            Ok(Err(e)) => return Err(err!(e,
                "Connecting to {}.", addr; IO, Network)),
            Err(_)     => return Err(err!(
                "Timeout connecting to {}.", addr;
                IO, Network)),
        };

        let mut stream = ClientStream::Plain(plain);

        // Read the 220 banner.
        let banner = res!(read_smtp_response(&mut stream).await);
        if banner.code != 220 {
            return Err(err!(
                "Expected 220 banner, got {} {}", banner.code, banner.text;
                IO, Network, Wire));
        }

        // EHLO, then look at extensions.
        res!(write_command(&mut stream, &fmt!("EHLO {}", self.hostname)).await);
        let ehlo = res!(read_smtp_response(&mut stream).await);
        if ehlo.code != 250 {
            return Err(err!(
                "EHLO rejected: {} {}", ehlo.code, ehlo.text;
                IO, Network, Wire));
        }
        let supports_starttls = ehlo.text.lines().any(|l| {
            l.trim().eq_ignore_ascii_case("STARTTLS")
        });

        // Opportunistic STARTTLS.
        if supports_starttls {
            res!(write_command(&mut stream, "STARTTLS").await);
            let resp = res!(read_smtp_response(&mut stream).await);
            if resp.code == 220 {
                let plain = match stream.into_plain() {
                    Some(s) => s,
                    None => return Err(err!(
                        "STARTTLS response received on already-TLS stream.";
                        Invalid, Bug)),
                };
                stream = res!(tls::upgrade(plain, &tgt.host, self.tls_config.clone()).await);

                // Re-issue EHLO inside TLS.
                res!(write_command(&mut stream, &fmt!("EHLO {}", self.hostname)).await);
                let _ = res!(read_smtp_response(&mut stream).await);
            }
        }

        let queue_id = res!(transact(&mut stream, mail_from, rcpt_to, body).await);

        // QUIT.
        let _ = write_command(&mut stream, "QUIT").await;
        let _ = read_smtp_response(&mut stream).await;

        Ok(queue_id)
    }
}

/// Walk MAIL/RCPT/DATA on a stream that is already open, secured and (where the server demands it)
/// authenticated. Delivery and submission differ in how they reach this point and not at all in
/// what they do once they are here, so they share the transaction rather than each keeping a copy
/// of it -- the second copy is where the dot-stuffing gets forgotten.
async fn transact(
    stream:     &mut ClientStream,
    mail_from:  &str,
    rcpt_to:    &[String],
    body:       &[u8],
)
    -> Outcome<String>
{
    res!(write_command(stream, &fmt!("MAIL FROM:<{}>", mail_from)).await);
    let resp = res!(read_smtp_response(stream).await);
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
        res!(write_command(stream, &fmt!("RCPT TO:<{}>", r)).await);
        let resp = res!(read_smtp_response(stream).await);
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
    res!(write_command(stream, "DATA").await);
    let resp = res!(read_smtp_response(stream).await);
    if resp.code != 354 {
        return Err(err!(
            "DATA rejected: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }

    // A line of the body that begins with a full stop would otherwise end the message.
    let stuffed = dot_stuff(body);
    if let Err(e) = stream.write_all(&stuffed).await {
        return Err(err!(e, "Writing DATA body."; IO, Network, Write));
    }
    if !body.ends_with(b"\r\n") {
        if let Err(e) = stream.write_all(b"\r\n").await {
            return Err(err!(e, "Writing CRLF tail."; IO, Network, Write));
        }
    }
    if let Err(e) = stream.write_all(b".\r\n").await {
        return Err(err!(e, "Writing DATA terminator."; IO, Network, Write));
    }
    if let Err(e) = stream.flush().await {
        return Err(err!(e, "Flushing DATA."; IO, Network, Write));
    }

    let resp = res!(read_smtp_response(stream).await);
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
/// The whole chain is walked, not only the outermost frame. `res!` wraps a cause in an
/// `Error::Upstream` carrying **no tags of its own**, and `Error::tags` reports one frame's tags
/// rather than the chain's -- so every `res!` between the 5xx and the caller hid the tag. Reading
/// only the outer frame, as this did until 2026-08-17, made the predicate answer `false` to every
/// permanent failure there has ever been: `submit` and `try_one` each pass `transact`'s error
/// through one `res!`. Nothing was suppressed, and `fe2o3_steel`'s subscriber list kept mailing
/// addresses their servers had refused outright.
pub fn is_permanent(e: &Error<ErrTag>) -> bool {
    if e.tags().contains(&ErrTag::Permanent) {
        return true;
    }
    let mut cause = std::error::Error::source(e);
    while let Some(c) = cause {
        if let Some(inner) = c.downcast_ref::<Error<ErrTag>>() {
            if inner.tags().contains(&ErrTag::Permanent) {
                return true;
            }
        }
        cause = c.source();
    }
    false
}

/// Prove to the provider that the sender holds the account.
///
/// `PLAIN` is preferred and `LOGIN` accepted, because between them they are what every provider
/// worth submitting through offers. Both hand over the password in base64, which is an encoding and
/// not a protection -- the only thing keeping it safe is the TLS underneath, which is why the
/// caller establishes that first and refuses to proceed without it. `ehlo` is the extension list
/// the server advertised inside TLS.
async fn authenticate(
    stream:     &mut ClientStream,
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
        res!(write_command(stream, &cmd).await);
        let resp = res!(read_smtp_response(stream).await);
        return check_auth(&resp);
    }

    if mechanisms.iter().any(|m| m == "LOGIN") {
        res!(write_command(stream, "AUTH LOGIN").await);
        let resp = res!(read_smtp_response(stream).await);
        if resp.code != 334 {
            return Err(err!(
                "AUTH LOGIN was refused before the username: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        res!(write_command(stream, &base64::encode(user.as_bytes())).await);
        let resp = res!(read_smtp_response(stream).await);
        if resp.code != 334 {
            return Err(err!(
                "The server rejected the username: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        res!(write_command(stream, &base64::encode(password.as_bytes())).await);
        let resp = res!(read_smtp_response(stream).await);
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

/// A response ends at the line whose fourth byte is a space rather than a hyphen.
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

/// CRLF-terminated, and flushed.
async fn write_command(stream: &mut ClientStream, cmd: &str) -> Outcome<()> {
    let line = fmt!("{}\r\n", cmd);
    if let Err(e) = stream.write_all(line.as_bytes()).await {
        return Err(err!(e, "Writing SMTP command."; IO, Network, Write));
    }
    if let Err(e) = stream.flush().await {
        return Err(err!(e, "Flushing SMTP command."; IO, Network, Write));
    }
    Ok(())
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
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
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
            let verdict = if p.auth_ok {
                &b"235 2.7.0 Accepted\r\n"[..]
            } else {
                &b"535 5.7.8 Username and Password not accepted\r\n"[..]
            };

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut g) = log.lock() {
                    g.push(line.clone());
                }
                if in_data {
                    if line == "." {
                        in_data = false;
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

    async fn client() -> Outcome<OutboundClient> {
        OutboundClient::with_system_roots(EHLO)
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
            &[fmt!("bob@example.net")], &body()).await);
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
            "a@example.com", &[fmt!("bob@example.net")], &body()).await);
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
            &[fmt!("bob@example.net")], &body()).await);
        req!(true, res!(lines_of(&saw_good)).iter().any(|l| l == "."));
        req!(true, res!(lines_of(&saw_bad)).is_empty(),
            "a less-preferred exchange was used while a better one worked");
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
            &[fmt!("nobody@example.net")], &body()).await
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
            &[fmt!("bob@example.net")], &body()).await
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
            &[fmt!("nobody@example.net")], &body()).await
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
            &[fmt!("bob@example.net")], &body()).await);
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
            &[fmt!("bob@example.net")], &body()).await
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
            &[fmt!("nobody@example.net")], &body()).await
        {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!("A 550 was a delivery."; Test, Invalid)),
        };
        req!(true, msg.contains(&host), "the failing exchange was not named: {}", msg);
        req!(true, msg.contains("550"), "the server's code was dropped: {}", msg);
        Ok(())
    }
}
