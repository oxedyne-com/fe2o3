//! Minimal async HTTPS client built on `tokio` + `tokio_rustls`.
//!
//! The pattern this module implements -- `TcpStream::connect` → TLS wrap via
//! `TlsConnector::from(Arc<ClientConfig>)` → write a `HttpMessage`-shaped
//! request → read a `HttpMessage` response -- already existed inside
//! `fe2o3_steel/tests/client.rs` for test harness purposes. This module hoists
//! it into `fe2o3_net` as a reusable primitive that any crate in the
//! workspace can call without reinventing it.
//!
//! Design choices kept deliberately small:
//!
//! - One request per connection, closed via `Connection: close`. No keep-alive,
//!   no pipelining, no HTTP/2. Sufficient for RFC 8555 ACME traffic and for
//!   the outbound HTTPS needs of SMTP webhooks, WebSocket handshakes to
//!   remote servers and similar short-lived call patterns.
//! - No trust store is bundled. The caller supplies an
//!   `Arc<rustls::ClientConfig>` that already carries whatever root anchors
//!   they want to trust, and `fe2o3_net` stays free of `webpki-roots` or
//!   `rustls-native-certs`. The ACME client under `fe2o3_net/src/acme/`
//!   compiles in its own pinned Let's Encrypt root anchors rather than
//!   pulling a generic trust store.
//! - Responses are read with `HttpMessage::read` using the existing default
//!   chunk sizes from `fe2o3_net::constant`. A body is framed by its
//!   `Content-Length`, by chunked transfer encoding, or, when a response says
//!   neither, by the end of the connection, which is how an HTTP/1.0 server such
//!   as Python's `http.server` ends a reply that has no length to state.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    constant,
    http::{
        header::HttpMethod,
        msg::{
            HttpMessage,
            ReadLimits,
        },
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
};

use tokio::{
    io::{
        AsyncRead,
        AsyncWrite,
        AsyncWriteExt,
    },
    net::TcpStream,
};
use tokio_rustls::{
    rustls::{
        pki_types::ServerName,
        ClientConfig,
    },
    TlsConnector,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REQUEST FORMATTING                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// `host` goes into the `Host:` header and `path` is the request target, query
/// string included. `Host`, `Content-Length` and `Connection: close` are always
/// written here, so a caller must not repeat them in `headers`.
///
/// Factored out so the byte layout can be tested without a TLS socket.
pub fn format_request(
    method:     HttpMethod,
    host:       &str,
    path:       &str,
    headers:    &[(&str, &str)],
    body:       &[u8],
)
    -> Vec<u8>
{
    let mut out = String::with_capacity(256 + body.len());
    out.push_str(&fmt!("{} {} HTTP/1.1\r\n", method, path));
    out.push_str(&fmt!("Host: {}\r\n", host));
    out.push_str("Connection: close\r\n");
    for (name, value) in headers {
        out.push_str(&fmt!("{}: {}\r\n", name, value));
    }
    out.push_str(&fmt!("Content-Length: {}\r\n", body.len()));
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE EXCHANGE                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The half of a request that does not care whether the stream beneath it is
/// TLS-wrapped, and so is shared by all four entry points below.
async fn exchange<S>(
    stream:         &mut S,
    method:         HttpMethod,
    request_bytes:  &[u8],
    peer:           &str,
    limits:         Option<&ReadLimits>,
)
    -> Outcome<HttpMessage>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match stream.write_all(request_bytes).await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "Failed to write HTTP request body to {}.", peer;
            IO, Network, Wire, Write)),
    }
    match stream.flush().await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "Failed to flush HTTP request to {}.", peer;
            IO, Network, Wire, Write)),
    }

    let result = HttpMessage::read_reply::<
        { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
        { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
        _,
    >(
        Pin::new(stream),
        &Vec::new(),
        method == HttpMethod::HEAD,
        limits,
    ).await;

    match result {
        Ok((Some(msg), _remnant)) => Ok(msg),
        Ok((None, _)) => Err(err!(
            "Server at {} closed the connection before sending a \
            complete HTTP response.",
            peer;
            IO, Network, Wire, Read, Missing)),
        // Kept distinct, so a caller that set `limits` can tell an answer larger than it
        // allows from one that broke off, without reading the text.
        Err(e) if e.tags().contains(&ErrTag::TooBig) => Err(err!(e,
            "The HTTP response from {} is larger than this caller reads.", peer;
            IO, Network, Wire, Read, TooBig)),
        Err(e) => Err(err!(e,
            "Failed to read or parse the HTTP response from {}.", peer;
            IO, Network, Wire, Read)),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HTTP REQUEST (PLAIN)                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// The sibling of [`https_request`] for an upstream over loopback or another
/// trusted segment, where TLS buys nothing: a proxied app binding
/// `127.0.0.1:<port>` need not present a certificate for traffic that never
/// leaves the host, and insisting on one would mean an internal CA to rotate and
/// a handshake on every hit.
pub async fn http_request(
    host:           &str,
    port:           u16,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
)
    -> Outcome<HttpMessage>
{
    http_request_limited(host, port, method, path, headers, body, None).await
}

/// [`http_request`] with the response bounded by `limits`, for a caller that
/// does not trust the peer to answer in proportion: a health probe, say, whose
/// every reply is kept.
pub async fn http_request_limited(
    host:           &str,
    port:           u16,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
    limits:         Option<&ReadLimits>,
)
    -> Outcome<HttpMessage>
{
    let request_bytes = format_request(method, host, path, headers, body);
    let peer = fmt!("{}:{}", host, port);

    let mut stream = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Failed to open a TCP connection to {}.", peer;
            IO, Network, Init)),
    };

    exchange(&mut stream, method, &request_bytes, &peer, limits).await
}

/// Dials an address the caller has already vetted, rather than a host name this
/// would resolve for itself.
///
/// The distinction is the whole point. A server that connects somewhere its
/// user named must check the address first (see
/// [`crate::addr::resolve_public`]), and a check is worthless if the name is
/// then resolved a second time to dial it: the answer can change in between,
/// and DNS rebinding is precisely that trick. So the caller resolves once,
/// vets what came back, and hands the surviving address here. `host` is still
/// needed, but only for the `Host` header the origin server reads.
///
/// `limits` bounds the response, so a caller fetching a page on a user's
/// behalf can cap what it is willing to read.
pub async fn http_request_at(
    addr:           SocketAddr,
    host:           &str,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
    limits:         Option<&ReadLimits>,
)
    -> Outcome<HttpMessage>
{
    let request_bytes = format_request(method, host, path, headers, body);
    let peer = fmt!("{} ({})", host, addr);

    let mut stream = match TcpStream::connect(addr).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Failed to open a TCP connection to {}.", peer;
            IO, Network, Init)),
    };

    exchange(&mut stream, method, &request_bytes, &peer, limits).await
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HTTPS REQUEST                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// The request is formed as [`format_request`] describes, and `tls_config` is
/// the rustls configuration the caller built, normally a trust store of root CAs
/// and no client auth.
///
/// Each step's error is tagged `IO`, `Network` and, where it applies, `Wire`, so
/// a caller can tell a connect failure from a handshake failure from a
/// response-parse failure without reading the text.
pub async fn https_request(
    host:           &str,
    port:           u16,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
    tls_config:     Arc<ClientConfig>,
)
    -> Outcome<HttpMessage>
{
    https_request_limited(host, port, method, path, headers, body, tls_config, None).await
}

/// [`https_request`] with the response bounded by `limits`, as
/// [`http_request_limited`] is.
pub async fn https_request_limited(
    host:           &str,
    port:           u16,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
    tls_config:     Arc<ClientConfig>,
    limits:         Option<&ReadLimits>,
)
    -> Outcome<HttpMessage>
{
    // Format the request bytes up front so any failure from this point on is
    // a real network or TLS fault, not a local formatting bug.
    let request_bytes = format_request(method, host, path, headers, body);
    let peer = fmt!("{}:{}", host, port);

    // TCP connect to the remote server.
    let tcp = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Failed to open a TCP connection to {}.", peer;
            IO, Network, Init)),
    };

    let mut stream = res!(tls_wrap(tcp, host, &peer, tls_config).await);
    exchange(&mut stream, method, &request_bytes, &peer, limits).await
}

/// The TLS sibling of [`http_request_at`], and vetted for the same reason: the
/// address is dialled as given, while `host` names the certificate that must
/// validate and fills the `Host` header. Pinning the address does not weaken
/// the TLS check -- the server still has to present a certificate for the name
/// the caller asked for.
pub async fn https_request_at(
    addr:           SocketAddr,
    host:           &str,
    method:         HttpMethod,
    path:           &str,
    headers:        &[(&str, &str)],
    body:           &[u8],
    tls_config:     Arc<ClientConfig>,
    limits:         Option<&ReadLimits>,
)
    -> Outcome<HttpMessage>
{
    let request_bytes = format_request(method, host, path, headers, body);
    let peer = fmt!("{} ({})", host, addr);

    let tcp = match TcpStream::connect(addr).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Failed to open a TCP connection to {}.", peer;
            IO, Network, Init)),
    };

    let mut stream = res!(tls_wrap(tcp, host, &peer, tls_config).await);
    exchange(&mut stream, method, &request_bytes, &peer, limits).await
}

/// rustls needs the host name as a validated `ServerName`, so that it can send
/// the right SNI and check the server certificate's SANs against it.
async fn tls_wrap(
    tcp:            TcpStream,
    host:           &str,
    peer:           &str,
    tls_config:     Arc<ClientConfig>,
)
    -> Outcome<tokio_rustls::client::TlsStream<TcpStream>>
{
    let server_name = match ServerName::try_from(host.to_string()) {
        Ok(n) => n,
        Err(e) => return Err(err!(e,
            "Host {:?} is not a valid DNS name for TLS SNI.", host;
            IO, Network, Invalid, Input)),
    };
    let connector = TlsConnector::from(tls_config);
    match connector.connect(server_name, tcp).await {
        Ok(s) => Ok(s),
        Err(e) => Err(err!(e,
            "TLS handshake with {} failed.", peer;
            IO, Network, Init)),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    /// Split the wire bytes at the `\r\n\r\n` boundary between header block
    /// and body so assertions can inspect them separately. The header block
    /// keeps the `\r\n` that terminates its last header line so that every
    /// header line in the returned string ends with `\r\n` consistently
    /// (the empty-line half of the separator is dropped). Returns
    /// `(header_block, body)`.
    fn split_wire(bytes: &[u8]) -> (String, Vec<u8>) {
        let sep = b"\r\n\r\n";
        let pos = bytes.windows(sep.len())
            .position(|w| w == sep)
            .expect("wire bytes did not contain an HTTP header/body separator");
        let header = String::from_utf8(bytes[..pos + 2].to_vec())
            .expect("header block was not valid UTF-8");
        let body = bytes[pos + sep.len()..].to_vec();
        (header, body)
    }

    fn count_occurrences(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    /// A GET request with no body must emit the correct request line, a
    /// `Host` header, `Connection: close`, and `Content-Length: 0`, with an
    /// empty body.
    #[test]
    fn test_format_request_get_no_body() -> Outcome<()> {
        let bytes = format_request(
            HttpMethod::GET,
            "acme-v02.api.letsencrypt.org",
            "/directory",
            &[],
            &[],
        );
        let (header, body) = split_wire(&bytes);

        if !header.starts_with("GET /directory HTTP/1.1\r\n") {
            return Err(err!(
                "Expected request line 'GET /directory HTTP/1.1', got first \
                line: {:?}.",
                header.lines().next().unwrap_or("");
                Test, Mismatch));
        }
        if !header.contains("Host: acme-v02.api.letsencrypt.org\r\n") {
            return Err(err!(
                "Missing or wrong Host header in:\n{}", header;
                Test, Missing));
        }
        if !header.contains("Connection: close\r\n") {
            return Err(err!(
                "Missing Connection: close header in:\n{}", header;
                Test, Missing));
        }
        if !header.contains("Content-Length: 0\r\n") {
            return Err(err!(
                "Missing Content-Length: 0 header in:\n{}", header;
                Test, Missing));
        }
        if !body.is_empty() {
            return Err(err!(
                "Expected empty body for a GET request, got {} bytes.",
                body.len();
                Test, Mismatch));
        }
        Ok(())
    }

    /// A POST with a body must emit the correct Content-Length and place the
    /// body bytes verbatim after the header terminator.
    #[test]
    fn test_format_request_post_with_body() -> Outcome<()> {
        let payload = br#"{"protected":"...","payload":"...","signature":"..."}"#;
        let bytes = format_request(
            HttpMethod::POST,
            "acme-v02.api.letsencrypt.org",
            "/acme/new-order",
            &[("Content-Type", "application/jose+json")],
            payload,
        );
        let (header, body) = split_wire(&bytes);

        if !header.starts_with("POST /acme/new-order HTTP/1.1\r\n") {
            return Err(err!(
                "Expected request line 'POST /acme/new-order HTTP/1.1', got \
                first line: {:?}.",
                header.lines().next().unwrap_or("");
                Test, Mismatch));
        }
        if !header.contains("Content-Type: application/jose+json\r\n") {
            return Err(err!(
                "Missing or wrong Content-Type header in:\n{}", header;
                Test, Missing));
        }
        let expected_len_line = fmt!("Content-Length: {}\r\n", payload.len());
        if !header.contains(&expected_len_line) {
            return Err(err!(
                "Missing or wrong {:?} header in:\n{}",
                expected_len_line, header;
                Test, Mismatch));
        }
        if body != payload {
            return Err(err!(
                "Body bytes did not round-trip: expected {} bytes, got {}.",
                payload.len(), body.len();
                Test, Mismatch));
        }
        Ok(())
    }

    /// Custom headers supplied by the caller must appear in the header block,
    /// without duplicating `Host`, `Connection` or `Content-Length`.
    #[test]
    fn test_format_request_custom_headers() -> Outcome<()> {
        let bytes = format_request(
            HttpMethod::POST,
            "example.test",
            "/acme/order/1",
            &[
                ("Content-Type",    "application/jose+json"),
                ("User-Agent",      "hematite-acme/0.5"),
                ("Accept",          "application/json"),
            ],
            b"{}",
        );
        let (header, _body) = split_wire(&bytes);

        // Our three custom headers must each appear exactly once.
        for name in ["Content-Type", "User-Agent", "Accept"] {
            let line_prefix = fmt!("{}: ", name);
            if count_occurrences(&header, &line_prefix) != 1 {
                return Err(err!(
                    "Expected exactly one {:?} header in:\n{}",
                    line_prefix, header;
                    Test, Mismatch));
            }
        }

        // Managed headers must still appear exactly once.
        for needle in [
            "Host: example.test\r\n",
            "Connection: close\r\n",
            "Content-Length: 2\r\n",
        ] {
            if count_occurrences(&header, needle) != 1 {
                return Err(err!(
                    "Expected exactly one occurrence of {:?} in:\n{}",
                    needle, header;
                    Test, Mismatch));
            }
        }
        Ok(())
    }

    /// Header block must always end with an empty line (`\r\n\r\n`), even
    /// when no custom headers are supplied.
    #[test]
    fn test_format_request_terminator() -> Outcome<()> {
        let bytes = format_request(
            HttpMethod::GET,
            "example.test",
            "/",
            &[],
            &[],
        );
        let sep = b"\r\n\r\n";
        if !bytes.windows(sep.len()).any(|w| w == sep) {
            return Err(err!(
                "Formatted request does not contain the CRLFCRLF header \
                terminator required by RFC 7230 §3.";
                Test, Missing));
        }
        Ok(())
    }

    // ── Replies from an HTTP/1.0 server ──────────────────────────────────────────────────────

    /// A one-shot loopback server: reads the request head, writes `reply` in pieces, then holds
    /// the connection open for `hold` before closing it. Returns its port.
    async fn serve_once(
        reply:  Vec<u8>,
        hold:   std::time::Duration,
    )
        -> Outcome<u16>
    {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = res!(TcpListener::bind("127.0.0.1:0").await);
        let port = res!(listener.local_addr()).port();
        tokio::spawn(async move {
            let (mut tcp, _) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut buf = [0u8; 2048];
            let _ = tcp.read(&mut buf).await;
            for piece in reply.chunks(1_300) {
                let _ = tcp.write_all(piece).await;
                let _ = tcp.flush().await;
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            tokio::time::sleep(hold).await;
            let _ = tcp.shutdown().await;
        });
        Ok(port)
    }

    fn run<F: std::future::Future<Output = Outcome<()>>>(f: F) -> Outcome<()> {
        let rt = res!(tokio::runtime::Runtime::new());
        rt.block_on(f)
    }

    /// Python's `http.server` answers `HTTP/1.0`. The reply was refused as an unrecognised
    /// version, so a caller proxying to such a server got an error where the page should be.
    #[test]
    fn test_an_http_1_0_reply_with_a_length_is_read() -> Outcome<()> {
        run(async {
            let port = res!(serve_once(
                b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
                std::time::Duration::ZERO,
            ).await);
            let msg = res!(http_request("127.0.0.1", port, HttpMethod::GET, "/", &[], &[]).await);
            assert_eq!(msg.header.version, crate::http::header::HttpVersion::Http1_0);
            assert_eq!(msg.body, b"hello");
            Ok(())
        })
    }

    /// With no length and no chunking an HTTP/1.0 body ends where the connection does (RFC 9112
    /// 6.3). It is longer than one read and arrives in several writes, so it can only be whole if
    /// the client reads to the close.
    #[test]
    fn test_a_body_that_ends_with_the_connection_is_read_whole() -> Outcome<()> {
        run(async {
            let body: Vec<u8> = (0..12_345u32).map(|i| b'a' + (i % 26) as u8).collect();
            let mut reply = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\n".to_vec();
            reply.extend_from_slice(&body);
            let port = res!(serve_once(reply, std::time::Duration::ZERO).await);
            let msg = res!(http_request("127.0.0.1", port, HttpMethod::GET, "/", &[], &[]).await);
            assert_eq!(msg.body.len(), body.len());
            assert_eq!(msg.body, body);
            Ok(())
        })
    }

    /// A reply to `HEAD`, and a `204` or `304`, has no body whatever the framing says, so the
    /// client must not wait for a connection the server is holding open.
    #[test]
    fn test_a_reply_that_has_no_body_is_not_waited_for() -> Outcome<()> {
        run(async {
            let hold = std::time::Duration::from_secs(4);
            let cases: [(HttpMethod, &[u8]); 3] = [
                (HttpMethod::HEAD, b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\n"),
                (HttpMethod::GET,  b"HTTP/1.0 204 No Content\r\n\r\n"),
                (HttpMethod::GET,  b"HTTP/1.1 304 Not Modified\r\n\r\n"),
            ];
            for (method, reply) in cases {
                let port = res!(serve_once(reply.to_vec(), hold).await);
                let started = std::time::Instant::now();
                let msg = res!(http_request("127.0.0.1", port, method, "/", &[], &[]).await);
                assert!(msg.body.is_empty());
                assert!(started.elapsed() < std::time::Duration::from_secs(2),
                    "{:?} {:?} waited {:?} for a body that cannot exist",
                    method, String::from_utf8_lossy(reply), started.elapsed());
            }
            Ok(())
        })
    }

    /// A caller's bound on the body holds for one with no length to check it against.
    #[test]
    fn test_a_body_that_ends_with_the_connection_obeys_the_limit() -> Outcome<()> {
        run(async {
            let mut reply = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
            reply.extend_from_slice(&[b'x'; 20_000]);
            let port = res!(serve_once(reply, std::time::Duration::ZERO).await);
            let limits = ReadLimits { max_body_bytes: Some(1_000), ..ReadLimits::default() };
            let e = match http_request_limited("127.0.0.1", port, HttpMethod::GET, "/", &[], &[],
                Some(&limits)).await
            {
                Ok(m) => return Err(err!("A {} byte body passed a limit of 1000.", m.body.len();
                    Test, Mismatch)),
                Err(e) => e,
            };
            assert!(e.tags().contains(&ErrTag::TooBig), "{}", e);
            Ok(())
        })
    }

    /// A body framed by a chunked encoding or by a `Content-Length` ends where the framing says,
    /// not where the connection does. A chunked reply reaches the framing check with its
    /// `Transfer-Encoding` already taken off, so a check made afterwards sees no framing at all and
    /// reads to the close, which both waits out a server holding the connection open and throws
    /// the decoded body away.
    #[test]
    fn test_a_framed_reply_is_not_read_to_the_close() -> Outcome<()> {
        run(async {
            let hold = std::time::Duration::from_secs(4);
            let cases: [(&[u8], &[u8]); 3] = [
                (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
                    b"hello world"),
                (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", b""),
                (b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", b""),
            ];
            for (reply, want) in cases {
                let port = res!(serve_once(reply.to_vec(), hold).await);
                let started = std::time::Instant::now();
                let msg = res!(http_request("127.0.0.1", port, HttpMethod::GET, "/", &[], &[]).await);
                assert_eq!(msg.body, want, "{:?}", String::from_utf8_lossy(reply));
                assert!(started.elapsed() < std::time::Duration::from_secs(2),
                    "{:?} waited {:?} for a close its framing had made unnecessary",
                    String::from_utf8_lossy(reply), started.elapsed());
            }
            Ok(())
        })
    }
}
