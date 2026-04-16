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
//!   chunk sizes from `fe2o3_net::constant`. Chunked transfer encoding is
//!   not supported: ACME API responses always carry a `Content-Length`
//!   header, and that is the only production caller for now.

use crate::{
    constant,
    http::{
        header::HttpMethod,
        msg::HttpMessage,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    pin::Pin,
    sync::Arc,
};

use tokio::{
    io::AsyncWriteExt,
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

/// Format an HTTP/1.1 request as wire bytes.
///
/// The caller supplies the method, the target server's host (which is written
/// into the `Host:` header), the request-target path (e.g. `/acme/new-nonce`
/// including any query string), a list of extra header name/value pairs, and
/// the request body. The function always emits `Host`, `Content-Length` and
/// `Connection: close` itself; callers should not include those in `headers`.
///
/// This is factored out so the request byte layout can be tested without
/// bringing up a TLS socket.
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
// │ HTTPS REQUEST                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Perform a single HTTPS request/response cycle against a remote server.
///
/// `host` / `port` identify the TCP endpoint. `method` / `path` / `headers` /
/// `body` form the request as described on [`format_request`]. `tls_config`
/// is the rustls client configuration the caller has built (typically
/// carrying a trust store of root CAs and no client auth).
///
/// The function opens a TCP connection, completes a TLS handshake using the
/// supplied config, writes the formatted request, reads one response from
/// the peer via [`HttpMessage::read`], and returns the parsed message.
///
/// Errors at each step are wrapped with `IO`, `Network` and (where
/// appropriate) `Wire` tags so callers can distinguish connect failures
/// from handshake failures from response-parse failures without text
/// inspection.
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
    // Format the request bytes up front so any failure from this point on is
    // a real network or TLS fault, not a local formatting bug.
    let request_bytes = format_request(method, host, path, headers, body);

    // TCP connect to the remote server.
    let tcp = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Failed to open a TCP connection to {}:{}.", host, port;
            IO, Network, Init)),
    };

    // TLS handshake. rustls needs the host name as a validated `ServerName`
    // so it can check SNI and verify the server certificate's SANs.
    let server_name = match ServerName::try_from(host.to_string()) {
        Ok(n) => n,
        Err(e) => return Err(err!(e,
            "Host {:?} is not a valid DNS name for TLS SNI.", host;
            IO, Network, Invalid, Input)),
    };
    let connector = TlsConnector::from(tls_config);
    let mut stream = match connector.connect(server_name, tcp).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "TLS handshake with {}:{} failed.", host, port;
            IO, Network, Init)),
    };

    // Write the full request and flush.
    match stream.write_all(&request_bytes).await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "Failed to write HTTP request body to {}:{}.", host, port;
            IO, Network, Wire, Write)),
    }
    match stream.flush().await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "Failed to flush HTTP request to {}:{}.", host, port;
            IO, Network, Wire, Write)),
    }

    // Read one response. The server will close the connection after
    // responding because we sent `Connection: close`.
    let result = HttpMessage::read::<
        { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
        { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
        _,
    >(
        Pin::new(&mut stream),
        &Vec::new(),
        Some(false),
        None,
    ).await;

    match result {
        Ok((Some(msg), _remnant)) => Ok(msg),
        Ok((None, _)) => Err(err!(
            "Server at {}:{} closed the connection before sending a \
            complete HTTP response.",
            host, port;
            IO, Network, Wire, Read, Missing)),
        Err(e) => Err(err!(e,
            "Failed to read or parse the HTTP response from {}:{}.", host, port;
            IO, Network, Wire, Read)),
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

    /// Count how many times `needle` appears in `haystack`.
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
}
