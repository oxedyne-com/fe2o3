//! Client-side TLS plumbing shared by the protocol clients.
//!
//! Every outbound protocol client in this crate faces the same two
//! problems: it must be able to speak plain TCP and TLS over one socket
//! (because STARTTLS upgrades in place), and it must validate the peer
//! against the host's trust anchors. [`ClientStream`] solves the first
//! and [`default_client_config`] the second, so SMTP, IMAP and anything
//! that follows share one implementation rather than each carrying its
//! own copy.
//!
//! Server-side TLS is a different concrete type -- `tokio_rustls`
//! distinguishes the client and server halves of a `TlsStream` -- so the
//! SMTP and IMAP servers keep their own `MaybeTls` and are unaffected.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    pin::Pin,
    sync::Arc,
    task::{
        Context,
        Poll,
    },
};

use tokio::{
    io::{
        AsyncRead,
        AsyncWrite,
        ReadBuf,
    },
    net::TcpStream,
};
use tokio_rustls::{
    rustls::{
        ClientConfig,
        pki_types::{
            CertificateDer,
            ServerName,
        },
        RootCertStore,
    },
    TlsConnector,
};


/// Either a plain TCP stream or a client-side TLS-wrapped TCP stream.
///
/// A protocol client holds one of these and can replace a `Plain` with a
/// `Tls` in place, which is exactly what a STARTTLS upgrade is.
pub enum ClientStream {
    /// Plain TCP, before any TLS handshake.
    Plain(TcpStream),
    /// Client-side TLS wrap.
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl ClientStream {

    /// Consume the wrapper and return the inner plain stream, if it has
    /// not already been wrapped in TLS.
    pub fn into_plain(self) -> Option<TcpStream> {
        match self {
            Self::Plain(s) => Some(s),
            Self::Tls(_)   => None,
        }
    }

    /// Whether the connection is protected.
    pub fn is_tls(&self) -> bool {
        matches!(self, Self::Tls(_))
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
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s)   => Pin::new(s.as_mut()).poll_read(cx, buf),
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
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s)   => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s)   => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s)   => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Wrap an established plain stream in client-side TLS, validating the
/// peer certificate against `cfg` for the name `host`.
pub async fn upgrade(
    plain:  TcpStream,
    host:   &str,
    cfg:    Arc<ClientConfig>,
)
    -> Outcome<ClientStream>
{
    let name = match ServerName::try_from(host.to_string()) {
        Ok(n)  => n,
        Err(_) => return Err(err!(
            "Cannot construct a TLS server name from '{}'.", host;
            Invalid, Input)),
    };
    let connector = TlsConnector::from(cfg);
    match connector.connect(name, plain).await {
        Ok(s)  => Ok(ClientStream::Tls(Box::new(s))),
        Err(e) => Err(err!(e,
            "TLS handshake to {}.", host;
            IO, Network, Init)),
    }
}

/// Load the host's CA bundle into a fresh rustls `ClientConfig`.
///
/// Callers needing a custom root store should build the `ClientConfig`
/// themselves; this is the "trust what the operating system trusts"
/// default that every public-internet client wants.
pub fn default_client_config() -> Outcome<ClientConfig> {
    let ca_paths = [
        "/etc/ssl/certs/ca-certificates.crt",	// Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",		// Fedora/RHEL
        "/etc/ssl/cert.pem",					// Alpine/macOS
    ];
    let ca_file = match ca_paths.iter().find(|p| std::path::Path::new(p).exists()) {
        Some(p) => *p,
        None => return Err(err!(
            "No system CA bundle found. Tried: {:?}", ca_paths;
            Init, Missing, File)),
    };
    let pem = match std::fs::read(ca_file) {
        Ok(d)  => d,
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
    Ok(ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth())
}

/// Iterate over every `-----BEGIN CERTIFICATE-----` block in `pem`,
/// returning the decoded DER bytes for each one. A tiny in-tree
/// substitute for `rustls_pemfile::certs` so the crate does not need the
/// extra dependency.
pub fn parse_pem_certificates(pem: &[u8]) -> Vec<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END:   &str = "-----END CERTIFICATE-----";
    let text = String::from_utf8_lossy(pem);
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut search_from = 0usize;
    while let Some(b) = text[search_from..].find(BEGIN) {
        let start = search_from + b + BEGIN.len();
        let e = match text[start..].find(END) {
            Some(i) => i,
            None    => break,
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
