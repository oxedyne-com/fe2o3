//! Relaying a WebSocket upgrade to an upstream server.
//!
//! Two kinds of route need the same thing done. A [`ProxyRoute`](crate::srv::cfg::ProxyRoute)
//! carries a whole application, WebSocket endpoints included; a
//! [`WsRoute`](crate::srv::cfg::WsRoute) carries one path to a server that speaks its own
//! protocol. Both forward the handshake and then get out of the way.
//!
//! Getting out of the way is the point. After the `101` the connection is no longer HTTP, and the
//! frames on it are between the client and the upstream: this module never parses one. It copies
//! bytes in both directions until an end closes, which is also why nothing here needs to know
//! which sub-protocol, extensions or message sizes the two ends agreed on.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    header::HttpHeadline,
    msg::HttpMessage,
};

use std::net::SocketAddr;

use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
    },
    net::TcpStream,
};


/// Largest upstream response header block accepted while waiting for the `101`.
const MAX_RESPONSE_HEADER_BYTES: usize = 64 * 1024;

/// The upstream path a request is forwarded to, query included.
///
/// The query must ride through verbatim: an upstream that dispatches on a query parameter never
/// sees it otherwise, and silently gets the default.
pub fn upstream_target(request: &HttpMessage, base_path: &str) -> Outcome<String> {
    let query = match &request.header.headline {
        HttpHeadline::Request { loc, .. } => loc.query.clone(),
        _ => return Err(err!(
            "A websocket upgrade must be an HTTP request, not a response.";
            Invalid, Bug)),
    };
    Ok(match query.is_empty() {
        true  => base_path.to_string(),
        false => fmt!("{}?{}", base_path, query),
    })
}

/// Forward `request` to the upstream WebSocket server at `host:port` as a `GET` of
/// `upstream_path`, relay the response to `client`, and then copy bytes both ways until either end
/// closes.
///
/// Every header the client sent is forwarded except the three this hop owns -- `Host`,
/// `Connection` and `Content-Length` -- so `Sec-WebSocket-Key`, `Sec-WebSocket-Version`,
/// `Sec-WebSocket-Protocol` and any cookies reach the upstream untouched. The upstream therefore
/// computes the `Sec-WebSocket-Accept` the client will check, and this hop never has to.
///
/// Returns once a direction closes. A client that goes away closes the upstream's write half, and
/// an upstream that goes away closes the client's, so neither end is left holding a socket the
/// other has abandoned.
pub async fn tunnel_upgrade<S>(
    client:         &mut S,
    request:        &HttpMessage,
    host:           &str,
    port:           u16,
    upstream_path:  &str,
    src_addr:       SocketAddr,
    id:             &str,
)
    -> Outcome<()>
    where S: AsyncRead + AsyncWrite + Unpin,
{
    // Connect to the upstream.
    let mut upstream = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "{}: ws relay: failed to connect to {}:{}.", id, host, port;
            IO, Network, Init)),
    };

    // Reconstruct the upgrade request for the upstream.
    let mut req = String::with_capacity(512);
    req.push_str(&fmt!("GET {} HTTP/1.1\r\n", upstream_path));
    req.push_str(&fmt!("Host: {}\r\n", host));
    for (name, values) in request.header.fields.iter() {
        let name_str = fmt!("{}", name);
        if name_str.eq_ignore_ascii_case("host")
            || name_str.eq_ignore_ascii_case("connection")
            || name_str.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        for value in values {
            req.push_str(&fmt!("{}: {}\r\n", name_str, value));
        }
    }
    req.push_str("Connection: Upgrade\r\n");
    req.push_str(&fmt!("X-Forwarded-For: {}\r\n", src_addr));
    req.push_str("X-Forwarded-Proto: https\r\n");
    req.push_str("\r\n");

    match upstream.write_all(req.as_bytes()).await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "{}: ws relay: failed to send the upgrade request upstream.", id;
            IO, Network, Wire, Write)),
    }
    match upstream.flush().await {
        Ok(()) => (),
        Err(e) => return Err(err!(e,
            "{}: ws relay: failed to flush the upstream connection.", id;
            IO, Network, Wire, Write)),
    }

    // Read the upstream's response -- a 101 if it accepted -- and forward it verbatim, whatever it
    // says. A refusal is the upstream's answer to give, and the client is entitled to read it.
    let mut buf = vec![0u8; 8192];
    let mut accum: Vec<u8> = Vec::new();
    loop {
        let n = match upstream.read(&mut buf).await {
            Ok(0) => {
                return Err(err!(
                    "{}: ws relay: the upstream closed before answering the upgrade.", id;
                    IO, Network, Wire, Read, Missing));
            }
            Ok(n) => n,
            Err(e) => return Err(err!(e,
                "{}: ws relay: error reading the upstream response.", id;
                IO, Network, Wire, Read)),
        };
        accum.extend_from_slice(&buf[..n]);
        if let Some(pos) = accum.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_end = pos + 4;
            let response_bytes = &accum[..header_end];
            let extra_bytes = &accum[header_end..];

            match client.write_all(response_bytes).await {
                Ok(()) => (),
                Err(e) => return Err(err!(e,
                    "{}: ws relay: failed to forward the upgrade response.", id;
                    IO, Network, Wire, Write)),
            }
            // Anything the upstream sent after its headers is already a frame, and belongs to the
            // client as much as the headers did.
            if !extra_bytes.is_empty() {
                match client.write_all(extra_bytes).await {
                    Ok(()) => (),
                    Err(e) => return Err(err!(e,
                        "{}: ws relay: failed to forward the upstream's first frames.", id;
                        IO, Network, Wire, Write)),
                }
            }
            match client.flush().await {
                Ok(()) => (),
                Err(e) => return Err(err!(e,
                    "{}: ws relay: failed to flush the client connection.", id;
                    IO, Network, Wire, Write)),
            }
            break;
        }
        if accum.len() > MAX_RESPONSE_HEADER_BYTES {
            return Err(err!(
                "{}: ws relay: the upstream response headers exceed {} bytes.",
                id, MAX_RESPONSE_HEADER_BYTES;
                IO, Network, Input, TooBig));
        }
    }

    // Copy bytes both ways until a direction ends.
    let (mut client_r, mut client_w) = tokio::io::split(client);
    let (mut upstream_r, mut upstream_w) = upstream.into_split();

    log!(log_get_level!(), "{}: ws relay: tunnel to {}:{} established.", id, host, port);

    tokio::select! {
        // Client -> upstream.
        res = tokio::io::copy(&mut client_r, &mut upstream_w) => {
            match res {
                Ok(_) => log!(log_get_level!(),
                    "{}: ws relay: client -> upstream closed.", id),
                Err(e) => log!(log_get_level!(),
                    "{}: ws relay: client -> upstream error: {}", id, e),
            }
            let _ = upstream_w.shutdown().await;
        }
        // Upstream -> client.
        res = tokio::io::copy(&mut upstream_r, &mut client_w) => {
            match res {
                Ok(_) => log!(log_get_level!(),
                    "{}: ws relay: upstream -> client closed.", id),
                Err(e) => log!(log_get_level!(),
                    "{}: ws relay: upstream -> client error: {}", id, e),
            }
            let _ = client_w.shutdown().await;
        }
    }

    log!(log_get_level!(), "{}: ws relay: tunnel closed.", id);
    Ok(())
}
