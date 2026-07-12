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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CERTIFICATE EXPIRY                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// When the first certificate in a PEM chain expires, as Unix seconds.
///
/// A server that renews its own certificate has to know when the one it holds
/// runs out. The obvious shortcut -- ask the filesystem how old the file is --
/// is wrong in the one case that matters: a certificate restored from a backup,
/// or copied from another host, has a fresh mtime and an old expiry, and a
/// server trusting the mtime will serve an expired certificate and never notice.
/// The certificate itself is the only thing that knows.
///
/// This walks just enough DER to reach the field. An X.509 certificate is
///
/// ```text
/// Certificate  ::= SEQUENCE { tbsCertificate TBSCertificate, ... }
/// TBSCertificate ::= SEQUENCE {
///     version         [0] EXPLICIT Version DEFAULT v1,   -- optional
///     serialNumber        INTEGER,
///     signature           AlgorithmIdentifier,           -- SEQUENCE
///     issuer              Name,                          -- SEQUENCE
///     validity            Validity,                      -- SEQUENCE  <- here
///     ... }
/// Validity ::= SEQUENCE { notBefore Time, notAfter Time }
/// ```
///
/// so the walk is: into the certificate, into the TBS, skip the optional
/// version and the serial and the two SEQUENCEs before it, and take the second
/// time in the validity.
pub fn certificate_not_after(pem: &[u8]) -> Outcome<i64> {
    let der = match parse_pem_certificates(pem).into_iter().next() {
        Some(d) => d,
        None    => return Err(err!(
            "No certificate found in the PEM data.";
            Invalid, Input, Missing)),
    };

    let (tbs, _)      = res!(der_expect(&der, 0, TAG_SEQUENCE));   // Certificate
    let (fields, _)   = res!(der_expect(tbs, 0, TAG_SEQUENCE));    // TBSCertificate

    let mut pos = 0usize;
    // The version is [0] EXPLICIT and only present for v2 and v3. Every
    // certificate a public CA issues today is v3, but a v1 certificate is legal
    // and simply omits it.
    if fields.get(pos) == Some(&TAG_VERSION) {
        let (_, next) = res!(der_element(fields, pos));
        pos = next;
    }
    for tag in [TAG_INTEGER, TAG_SEQUENCE, TAG_SEQUENCE] {          // serial, sig alg, issuer
        let (_, next) = res!(der_expect(fields, pos, tag));
        pos = next;
    }
    let (validity, _) = res!(der_expect(fields, pos, TAG_SEQUENCE));

    // notBefore, then notAfter. Only the second is wanted.
    let (_, after_nb)     = res!(der_element(validity, 0));
    let tag = match validity.get(after_nb) {
        Some(t) => *t,
        None    => return Err(err!(
            "Certificate validity has a notBefore but no notAfter.";
            Invalid, Input, Missing)),
    };
    let (not_after, _) = res!(der_element(validity, after_nb));

    parse_asn1_time(not_after, tag)
}

/// Whether the certificate expires within `lead` seconds of now -- which is the
/// question a renewer is actually asking. An unparseable or unreadable
/// certificate is treated as expiring, because a server that cannot tell should
/// renew rather than gamble.
pub fn certificate_expires_within(pem: &[u8], lead_secs: i64) -> bool {
    let not_after = match certificate_not_after(pem) {
        Ok(t)  => t,
        Err(_) => return true,
    };
    let now = match std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
    {
        Ok(d)  => d.as_secs() as i64,
        Err(_) => return true,
    };
    not_after - now <= lead_secs
}


const TAG_INTEGER:  u8 = 0x02;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_UTCTIME:  u8 = 0x17;
const TAG_GENTIME:  u8 = 0x18;
/// `[0] EXPLICIT`, the context-specific constructed tag the version carries.
const TAG_VERSION:  u8 = 0xa0;

/// Read the DER element at `pos`: return its contents and the offset just past
/// it. Only the length encodings a certificate actually uses are handled --
/// indefinite lengths are not legal in DER.
fn der_element(buf: &[u8], pos: usize) -> Outcome<(&[u8], usize)> {
    if pos + 2 > buf.len() {
        return Err(err!(
            "DER element at {} runs past the end of the buffer.", pos;
            Invalid, Input, Decode));
    }
    let first = buf[pos + 1];
    let (len, header) = if first < 0x80 {
        (first as usize, 2usize)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || pos + 2 + n > buf.len() {
            return Err(err!(
                "DER element at {} has an unsupported length encoding.", pos;
                Invalid, Input, Decode));
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | buf[pos + 2 + i] as usize;
        }
        (len, 2 + n)
    };
    let start = pos + header;
    let end   = match start.checked_add(len) {
        Some(e) if e <= buf.len() => e,
        _ => return Err(err!(
            "DER element at {} claims {} bytes, past the end of the buffer.",
            pos, len;
            Invalid, Input, Decode)),
    };
    Ok((&buf[start..end], end))
}

/// As [`der_element`], but insisting on a tag. A certificate whose shape
/// departs from X.509 is not one to guess about.
fn der_expect(buf: &[u8], pos: usize, tag: u8) -> Outcome<(&[u8], usize)> {
    match buf.get(pos) {
        Some(t) if *t == tag => der_element(buf, pos),
        Some(t) => Err(err!(
            "Expected DER tag {:#04x} at {}, found {:#04x}.", tag, pos, t;
            Invalid, Input, Decode)),
        None => Err(err!(
            "Expected DER tag {:#04x} at {}, found the end of the buffer.",
            tag, pos;
            Invalid, Input, Decode)),
    }
}

/// Parse an ASN.1 `UTCTime` (`YYMMDDHHMMSSZ`) or `GeneralizedTime`
/// (`YYYYMMDDHHMMSSZ`) into Unix seconds.
fn parse_asn1_time(bytes: &[u8], tag: u8) -> Outcome<i64> {
    let s = match std::str::from_utf8(bytes) {
        Ok(s)  => s.trim_end_matches('Z'),
        Err(e) => return Err(err!(e,
            "Certificate time is not valid UTF-8.";
            Invalid, Input, Decode)),
    };
    let num = |a: usize, b: usize| -> Outcome<i64> {
        match s.get(a..b).and_then(|t| t.parse::<i64>().ok()) {
            Some(n) => Ok(n),
            None    => Err(err!(
                "Certificate time '{}' is malformed.", s;
                Invalid, Input, Decode)),
        }
    };
    let (year, off) = match tag {
        TAG_UTCTIME => {
            // Two digits, so the century is inferred: RFC 5280 §4.1.2.5.1 puts
            // 50-99 in the 1900s and 00-49 in the 2000s.
            let yy = res!(num(0, 2));
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2usize)
        }
        TAG_GENTIME => (res!(num(0, 4)), 4usize),
        other => return Err(err!(
            "Certificate validity has tag {:#04x}, which is neither a UTCTime \
            nor a GeneralizedTime.", other;
            Invalid, Input, Decode)),
    };
    let month = res!(num(off,     off + 2));
    let day   = res!(num(off + 2, off + 4));
    let hour  = res!(num(off + 4, off + 6));
    let min   = res!(num(off + 6, off + 8));
    // Seconds are optional in a UTCTime, though every CA emits them.
    let sec   = if s.len() >= off + 10 { res!(num(off + 8, off + 10)) } else { 0 };

    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + min * 60 + sec)
}

/// Days since the Unix epoch for a proleptic-Gregorian date. Howard Hinnant's
/// `days_from_civil`, which is exact and needs no table.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;                                    // [0, 399]
    let mp  = (m + 9) % 12;                                     // March = 0
    let doy = (153 * mp + 2) / 5 + d - 1;                       // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;            // [0, 146096]
    era * 146_097 + doe - 719_468
}


#[cfg(test)]
mod cert_tests {
    use super::*;

    /// A certificate whose expiry we choose, so the parser is checked against a
    /// known answer rather than against itself.
    fn cert_expiring(year: i32, month: u8, day: u8) -> Vec<u8> {
        use rcgen::{Certificate, CertificateParams};
        let mut params = CertificateParams::new(vec![fmt!("example.com")]);
        params.not_after = rcgen::date_time_ymd(year, month, day);
        let cert = Certificate::from_params(params).expect("test cert");
        cert.serialize_pem().expect("test cert pem").into_bytes()
    }

    #[test]
    fn test_not_after_is_read_from_the_certificate() {
        let pem = cert_expiring(2031, 3, 14);
        let t = certificate_not_after(&pem).expect("parse");
        assert_eq!(t, 1_931_212_800, "2031-03-14T00:00:00Z, got {}", t);

        // A GeneralizedTime, which is what a CA must use past 2049.
        let pem = cert_expiring(2060, 12, 31);
        let t = certificate_not_after(&pem).expect("parse");
        assert_eq!(t, 2_871_676_800, "2060-12-31T00:00:00Z, got {}", t);
    }

    #[test]
    fn test_expiry_is_the_question_a_renewer_asks() {
        let soon = cert_expiring(2026, 7, 20);   // in the past by the time this ages
        let far  = cert_expiring(2099, 1, 1);
        assert!(certificate_expires_within(&soon, 30 * 24 * 3600));
        assert!(!certificate_expires_within(&far, 30 * 24 * 3600));
    }

    #[test]
    fn test_rubbish_is_treated_as_expiring() {
        // A server that cannot tell must renew, not gamble.
        assert!(certificate_expires_within(b"not a certificate", 0));
        assert!(certificate_not_after(b"not a certificate").is_err());
    }

    #[test]
    fn test_days_from_civil_epoch_and_leap_years() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 7, 12), 20_646);
    }
}
