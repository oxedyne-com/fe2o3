/// Plaintext HTTP listener.
///
/// Steel's primary listener is HTTPS on port 443 (or a development
/// equivalent). Some clients — notably browsers that do not default to
/// HTTPS-first mode — need an HTTP listener on port 80 that unconditionally
/// redirects to the HTTPS origin. This module provides that listener.
///
/// The redirect preserves the incoming `Host` header and request target
/// so deep links continue to work after the protocol upgrade.

use oxedyne_fe2o3_core::prelude::*;

use std::net::SocketAddr;

use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};


/// Maximum bytes read from a plaintext HTTP request before giving up.
/// We only need the request line and the `Host` header, so a small buffer
/// is plenty and bounds the damage from malicious clients.
const MAX_REQUEST_BYTES: usize = 8192;


/// Bind `server_address:port` and accept plaintext HTTP connections,
/// responding to each with a 301 redirect to the HTTPS equivalent.
///
/// This function loops forever and is intended to be spawned as a
/// background Tokio task alongside the main HTTPS accept loop. Per-
/// connection errors are logged but do not terminate the listener.
pub async fn run_redirect_listener(
    server_address: String,
    http_port:      u16,
    https_port:     u16,
)
    -> Outcome<()>
{
    let ip: std::net::IpAddr = match server_address.parse() {
        Ok(ip) => ip,
        Err(e) => return Err(err!(e,
            "Invalid server_address '{}' for plaintext HTTP listener.",
            server_address;
            Invalid, Input, Network)),
    };
    let addr = SocketAddr::new(ip, http_port);
    let listener = res!(TcpListener::bind(&addr).await, IO, Network);
    info!("Listening on: {} (plaintext HTTP, redirects to HTTPS)", addr);

    loop {
        let (stream, src_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                error!(err!(e,
                    "Plaintext HTTP accept aborted.";
                    IO, Network));
                continue;
            }
        };
        tokio::spawn(async move {
            if let Err(e) = handle_redirect(stream, src_addr, https_port).await {
                error!(err!(e,
                    "Plaintext HTTP connection from {} failed.", src_addr;
                    IO, Network));
            }
        });
    }
}

/// Handle a single plaintext HTTP connection by parsing just enough of
/// the request to extract the `Host` header and request target, then
/// writing a 301 response with a `Location` header pointing at the HTTPS
/// equivalent.
///
/// If the request is malformed the handler still replies with a safe
/// 301 to the root of whatever host it could identify, or an explanatory
/// plain-text 400 if nothing could be salvaged.
///
/// This is also used by the HTTPS listener when it detects plaintext
/// traffic on the TLS port (someone pasting `http://` into a browser
/// that does not upgrade automatically).
pub async fn handle_redirect(
    mut stream: TcpStream,
    _src_addr:  SocketAddr,
    https_port: u16,
)
    -> Outcome<()>
{
    // Read until we see the end-of-headers marker or hit the size cap.
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    loop {
        if buf.len() >= MAX_REQUEST_BYTES {
            break;
        }
        let n = match stream.read(&mut tmp).await {
            Ok(0)  => break,
            Ok(n)  => n,
            Err(_) => break,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    // Parse request line and Host header.
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");

    let mut host: Option<&str> = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Host:").or_else(|| line.strip_prefix("host:")) {
            host = Some(rest.trim());
            break;
        }
    }

    let location = match host {
        Some(h) if !h.is_empty() => {
            // Strip any incoming port, we always redirect to the HTTPS port.
            let host_only = match h.rfind(':') {
                Some(i) => &h[..i],
                None    => h,
            };
            if https_port == 443 {
                fmt!("https://{}{}", host_only, target)
            } else {
                fmt!("https://{}:{}{}", host_only, https_port, target)
            }
        }
        _ => {
            let body = "Bad Request: missing Host header.";
            let response = fmt!(
                "HTTP/1.1 400 Bad Request\r\n\
                Connection: close\r\n\
                Content-Type: text/plain; charset=utf-8\r\n\
                Content-Length: {}\r\n\
                \r\n\
                {}",
                body.len(), body,
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
            return Ok(());
        }
    };

    let body = fmt!("Redirecting to {}", location);
    let response = fmt!(
        "HTTP/1.1 301 Moved Permanently\r\n\
        Location: {}\r\n\
        Connection: close\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        Content-Length: {}\r\n\
        \r\n\
        {}",
        location, body.len(), body,
    );
    match stream.write_all(response.as_bytes()).await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "Failed to write plaintext HTTP redirect response.";
            IO, Network, Wire, Write)),
    }
    let _ = stream.shutdown().await;
    Ok(())
}
