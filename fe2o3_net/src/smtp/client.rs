//! Client-side SMTP for outbound delivery.
//!
//! Given an envelope (sender, recipients) and an RFC 5322 message body,
//! the client looks up the recipient domain's MX, opens a TCP
//! connection to the best-preference exchange, performs an EHLO and
//! opportunistic STARTTLS handshake, and walks MAIL/RCPT/DATA. Used by
//! the steel submission handler to relay outbound mail and by simple
//! programmatic senders.
//!
//! No queue, no retry policy, no exponential backoff -- the caller is
//! expected to drive retries itself by enqueueing the message in a
//! spool directory and re-invoking the client. Keeps the abstraction
//! useful for both a "fire and forget" path and a real queue runner.

use crate::{
    dns_resolver,
    smtp::server::read_line,
    tls::{
        self,
        ClientStream,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::IpAddr,
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

        // MAIL FROM.
        res!(write_command(&mut stream, &fmt!("MAIL FROM:<{}>", mail_from)).await);
        let resp = res!(read_smtp_response(&mut stream).await);
        if resp.code / 100 != 2 {
            return Err(err!(
                "MAIL FROM rejected: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        // RCPT TO each.
        for r in rcpt_to {
            res!(write_command(&mut stream, &fmt!("RCPT TO:<{}>", r)).await);
            let resp = res!(read_smtp_response(&mut stream).await);
            if resp.code / 100 != 2 {
                return Err(err!(
                    "RCPT TO:<{}> rejected: {} {}", r, resp.code, resp.text;
                    IO, Network, Wire));
            }
        }
        // DATA.
        res!(write_command(&mut stream, "DATA").await);
        let resp = res!(read_smtp_response(&mut stream).await);
        if resp.code != 354 {
            return Err(err!(
                "DATA rejected: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        // Send the body, applying dot-stuffing.
        let stuffed = dot_stuff(body);
        if let Err(e) = stream.write_all(&stuffed).await {
            return Err(err!(e, "Writing DATA body."; IO, Network, Write));
        }
        // Always end with CRLF if not already.
        if !body.ends_with(b"\r\n") {
            if let Err(e) = stream.write_all(b"\r\n").await {
                return Err(err!(e,
                    "Writing CRLF tail."; IO, Network, Write));
            }
        }
        if let Err(e) = stream.write_all(b".\r\n").await {
            return Err(err!(e, "Writing DATA terminator."; IO, Network, Write));
        }
        if let Err(e) = stream.flush().await {
            return Err(err!(e, "Flushing DATA."; IO, Network, Write));
        }
        let resp = res!(read_smtp_response(&mut stream).await);
        if resp.code / 100 != 2 {
            return Err(err!(
                "Server rejected message: {} {}", resp.code, resp.text;
                IO, Network, Wire));
        }
        // QUIT.
        let _ = write_command(&mut stream, "QUIT").await;
        let _ = read_smtp_response(&mut stream).await;

        Ok(resp.text)
    }
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
