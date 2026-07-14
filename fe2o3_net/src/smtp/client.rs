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


/// Default per-IO timeout for outbound SMTP. Chosen generously because
/// some receiving MX hosts greylist or impose multi-second waits before
/// 220.
pub const SMTP_CLIENT_TIMEOUT: Duration = Duration::from_secs(60);


/// Where to post a message, and how to prove you may.
///
/// The provider's submission service, not a recipient's MX: the host is the one the account lives
/// on, and the credential is the account's own. Port 587 conventionally starts in the clear and
/// upgrades with `STARTTLS`; port 465 is TLS from the first byte.
#[derive(Clone, Debug)]
pub struct SubmissionConfig {
    /// Submission host. Also the name the certificate is validated against.
    pub host:       String,
    /// Submission port, conventionally 587 (STARTTLS) or 465 (implicit TLS).
    pub port:       u16,
    /// Transport protection.
    pub security:   Security,
    /// The account to authenticate as. Usually, but not always, the address being sent from.
    pub user:       String,
    /// The account's password. For a provider with two-factor authentication this is an
    /// application password, not the password the human types into a browser.
    pub password:   String,
    /// Per-IO deadline.
    pub timeout:    Duration,
    /// Connect to this address instead of resolving `host`. The certificate is still validated
    /// against `host`, so pinning the address weakens nothing -- and a server connecting on behalf
    /// of a user must vet the address it dials rather than hand the name to the resolver twice.
    pub addr:       Option<SocketAddr>,
}

impl SubmissionConfig {

    /// A submission target with the conventional deadline and no pinned address.
    ///
    /// # Arguments
    /// * `host` - The provider's submission host.
    /// * `port` - The submission port.
    /// * `security` - How the connection is protected.
    /// * `user` - The account to authenticate as.
    /// * `password` - That account's password.
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

    /// Dial this address rather than resolving the host.
    pub fn with_addr(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Use this per-IO deadline.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}


/// One outbound delivery target after MX resolution. Sorted in
/// preference order by [`OutboundClient::deliver`].
#[derive(Clone, Debug)]
struct DeliveryTarget {
    /// MX exchange host name.
    host:       String,
    /// Resolved IP address.
    addr:       IpAddr,
    /// Original MX preference.
    preference: u16,
}

/// Per-process outbound SMTP client.
///
/// Holds a rustls `ClientConfig` initialised with the system trust
/// anchors so STARTTLS to any public MX validates correctly. Cheap to
/// clone -- the inner config is in an `Arc`.
#[derive(Clone)]
pub struct OutboundClient {
    /// Hostname to send in EHLO. Should be the public hostname of the
    /// sending server (the one that owns the IP whose PTR lines up).
    pub hostname:       Arc<String>,
    /// Rustls config used for STARTTLS. Built once via
    /// [`OutboundClient::default_tls_config`].
    pub tls_config:     Arc<ClientConfig>,
}

impl OutboundClient {

    /// Build an outbound client whose STARTTLS validation uses the
    /// system CA bundle. Convenience wrapper -- callers that need a
    /// custom root store should construct the `ClientConfig`
    /// themselves.
    pub fn with_system_roots(hostname: impl Into<String>) -> Outcome<Self> {
        let cfg = res!(Self::default_tls_config());
        Ok(Self {
            hostname:   Arc::new(hostname.into()),
            tls_config: Arc::new(cfg),
        })
    }

    /// Load the host's CA bundle into a fresh rustls `ClientConfig`.
    /// Delegates to [`crate::tls::default_client_config`], which every
    /// protocol client in this crate shares.
    pub fn default_tls_config() -> Outcome<ClientConfig> {
        tls::default_client_config()
    }

    /// Resolve the recipient domain's MX records, then attempt
    /// delivery against each in preference order until one succeeds.
    /// Returns the queue id assigned by the first server that accepted
    /// the message, or the last error if every host failed.
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
                    preference: pref,
                });
            }
        }
        if targets.is_empty() {
            return Err(err!(
                "No reachable MX hosts for any of the configured recipients.";
                IO, Network, Missing));
        }
        targets.sort_by_key(|t| t.preference);

        let mut last_err: Option<String> = None;
        for tgt in &targets {
            match self.try_one(tgt, mail_from, rcpt_to, body).await {
                Ok(qid) => return Ok(qid),
                Err(e) => {
                    let msg = fmt!("MX {} ({}): {}", tgt.host, tgt.addr, e);
                    warn!("Outbound SMTP attempt failed: {}", msg);
                    last_err = Some(msg);
                }
            }
        }
        Err(err!(
            "All MX delivery attempts failed; last error: {}",
            last_err.unwrap_or_else(|| "(none)".to_string());
            IO, Network))
    }

    /// Post a message through the account holder's own provider, authenticating first.
    ///
    /// This is the conversation a mail client has, not the one a mail server has: the provider
    /// carries the message because the sender proved they hold the account, so the credential is
    /// not optional and neither is the encryption under it. The client refuses to send the password
    /// over a connection it could not secure -- a provider that offers no TLS on its submission
    /// port is not one a password may be spoken to, and failing loudly is the only safe answer.
    ///
    /// # Arguments
    /// * `cfg` - The provider, the port, and the credential.
    /// * `mail_from` - The envelope sender.
    /// * `rcpt_to` - The envelope recipients. Unlike delivery, these may span any number of
    ///   domains: the provider, not this client, works out where each one goes.
    /// * `body` - The RFC 5322 message.
    ///
    /// # Returns
    /// Whatever the provider said when it accepted the message, which usually carries its queue id.
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

    /// Greet the server and return what it says it can do.
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
        let addr = std::net::SocketAddr::new(tgt.addr, 25);
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
///
/// # Arguments
/// * `stream` - The open conversation.
/// * `mail_from` - The envelope sender.
/// * `rcpt_to` - The envelope recipients.
/// * `body` - The RFC 5322 message.
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
        return Err(err!(
            "MAIL FROM rejected: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }
    for r in rcpt_to {
        res!(write_command(stream, &fmt!("RCPT TO:<{}>", r)).await);
        let resp = res!(read_smtp_response(stream).await);
        if resp.code / 100 != 2 {
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
        return Err(err!(
            "Server rejected message: {} {}", resp.code, resp.text;
            IO, Network, Wire));
    }
    Ok(resp.text)
}

/// Prove to the provider that the sender holds the account.
///
/// `PLAIN` is preferred and `LOGIN` accepted, because between them they are what every provider
/// worth submitting through offers. Both hand over the password in base64, which is an encoding and
/// not a protection -- the only thing keeping it safe is the TLS underneath, which is why the
/// caller establishes that first and refuses to proceed without it.
///
/// # Arguments
/// * `stream` - The open, secured conversation.
/// * `ehlo` - The extension list the server advertised inside TLS.
/// * `user` - The account to authenticate as.
/// * `password` - That account's password.
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

/// One parsed SMTP server response (potentially multi-line).
#[derive(Clone, Debug)]
struct SmtpResponse {
    /// Numeric code.
    code: u16,
    /// Concatenated text lines, joined by '\n'.
    text: String,
}

/// Read one full multi-line SMTP response (terminated by a line whose
/// fourth byte is a space rather than a hyphen).
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

/// Write one CRLF-terminated SMTP command.
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

/// Apply RFC 5321 §4.5.2 dot stuffing to a raw RFC 5322 message.
///
/// Any line whose first character is `.` gets a second `.` prepended
/// so the receiver does not mistake it for the message terminator.
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

/// Extract the domain part of an `local@domain` address.
fn extract_domain(addr: &str) -> Outcome<String> {
    match addr.rfind('@') {
        Some(i) => Ok(addr[i + 1..].to_lowercase()),
        None => Err(err!(
            "Address '{}' has no '@'.", addr;
            Invalid, Input, Mismatch)),
    }
}
