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
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    task::{
        Context,
        Poll,
    },
    time::Duration,
};

use tokio_rustls::rustls::{
    self,
    ClientConfig,
    pki_types::{
        CertificateDer,
        ServerName,
    },
    RootCertStore,
};
use tokio::{
    io::{
        AsyncRead,
        AsyncWrite,
        AsyncWriteExt,
        ReadBuf,
    },
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::TlsConnector;


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

    /// Load `/etc/ssl/certs/ca-certificates.crt` (or the closest
    /// equivalent) into a fresh rustls `ClientConfig`.
    pub fn default_tls_config() -> Outcome<ClientConfig> {
        let ca_paths = [
            "/etc/ssl/certs/ca-certificates.crt",   // Debian/Ubuntu
            "/etc/pki/tls/certs/ca-bundle.crt",     // Fedora/RHEL
            "/etc/ssl/cert.pem",                    // Alpine/macOS
        ];
        let ca_file = match ca_paths.iter().find(|p| std::path::Path::new(p).exists()) {
            Some(p) => *p,
            None => return Err(err!(
                "No system CA bundle found. Tried: {:?}", ca_paths;
                Init, Missing, File)),
        };
        let pem = match std::fs::read(ca_file) {
            Ok(d) => d,
            Err(e) => return Err(err!(e,
                "Failed to read CA bundle '{}'.", ca_file;
                IO, File, Read)),
        };
        let mut store = RootCertStore::empty();
        let mut count = 0u32;
        for der in parse_pem_certificates(&pem) {
            let cert = CertificateDer::from(der);
            if store.add(cert).is_ok() {
                count += 1;
            }
        }
        if count == 0 {
            return Err(err!(
                "CA bundle '{}' contained no usable certificates.", ca_file;
                Init, Invalid, File));
        }
        let cfg = ClientConfig::builder()
            .with_root_certificates(store)
            .with_no_client_auth();
        Ok(cfg)
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
                let server_name = match ServerName::try_from(tgt.host.clone()) {
                    Ok(n) => n,
                    Err(_) => return Err(err!(
                        "Cannot construct ServerName for '{}'.", tgt.host;
                        Invalid, Input)),
                };
                let connector = TlsConnector::from(self.tls_config.clone());
                let tls = match connector.connect(server_name, plain).await {
                    Ok(s) => s,
                    Err(e) => return Err(err!(e,
                        "TLS handshake to {}.", tgt.host;
                        IO, Network, Init)),
                };
                stream = ClientStream::Tls(Box::new(tls));

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

/// Either a plain TCP stream or a client-side TLS-wrapped TCP stream.
///
/// The SMTP server's `MaybeTls` holds a *server-side* `TlsStream`,
/// which is a different concrete type than the *client-side* one
/// produced by `TlsConnector::connect`. Rather than make the server
/// enum generic, we keep a small dedicated variant here for the
/// outbound client.
pub enum ClientStream {
    /// Plain TCP, before STARTTLS.
    Plain(TcpStream),
    /// Client-side TLS wrap, after STARTTLS.
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl ClientStream {
    /// Consume the wrapper and return the inner plain stream, if any.
    pub fn into_plain(self) -> Option<TcpStream> {
        match self {
            ClientStream::Plain(s) => Some(s),
            ClientStream::Tls(_)   => None,
        }
    }
}

impl AsyncRead for ClientStream {
    fn poll_read(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
        buf:    &mut ReadBuf<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            ClientStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ClientStream::Tls(s)   => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
        buf:    &[u8],
    )
        -> Poll<std::io::Result<usize>>
    {
        match self.get_mut() {
            ClientStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ClientStream::Tls(s)   => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            ClientStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ClientStream::Tls(s)   => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            ClientStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ClientStream::Tls(s)   => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
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

/// Iterate over every `-----BEGIN CERTIFICATE-----` block in `pem`,
/// returning the decoded DER bytes for each one. A tiny in-tree
/// substitute for `rustls_pemfile::certs` so fe2o3_net does not need
/// the extra crate.
fn parse_pem_certificates(pem: &[u8]) -> Vec<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END:   &str = "-----END CERTIFICATE-----";
    let text = String::from_utf8_lossy(pem);
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut search_from = 0usize;
    while let Some(b) = text[search_from..].find(BEGIN) {
        let start = search_from + b + BEGIN.len();
        let e = match text[start..].find(END) {
            Some(i) => i,
            None => break,
        };
        let block = &text[start..start + e];
        let stripped: String = block.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(der) = base64::decode(&stripped) {
            out.push(der);
        }
        search_from = start + e + END.len();
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
