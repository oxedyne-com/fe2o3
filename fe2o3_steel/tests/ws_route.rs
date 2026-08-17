//! A `ws_route` in front of a real WebSocket server.
//!
//! The upstream in these tests is not a stub answering a fixed string: it is `fe2o3_net`'s own
//! WebSocket machinery, handshaking from the request the relay forwarded and framing its replies.
//! That is the point of the exercise -- a relay that satisfies a mock of itself has proved nothing,
//! whereas a browser's decoder and this upstream agree on the same RFC.
//!
//! The relay is driven directly rather than through a TLS listener, because what is under test is
//! the forwarding, and a certificate would only stand between the test and the bytes.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::{
    http::{
        fwd::ForwardedPolicy,
        msg::HttpMessage,
    },
    ws::{
        accept_key,
        accept_response,
        connect_request,
        encode_message,
        read_message,
        WebSocketLimits,
        WebSocketMessage,
    },
};
use oxedyne_fe2o3_steel::srv::{
    cfg::{
        VhostConfig,
        WsRoute,
    },
    wsproxy::tunnel_upgrade,
};

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};

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


/// Frames a client-side message, as a browser would: masked.
fn client_frame(msg: &WebSocketMessage) -> Outcome<Vec<u8>> {
    encode_message(msg, true, 1024, 4096)
}

/// What the upstream saw of the request the relay forwarded to it.
#[derive(Clone, Debug, Default)]
struct UpstreamSaw {
    path:       String,             // as the relay asked for it
    ws_key:     Option<String>,     // the `Sec-WebSocket-Key` the client chose, if it survived
    forwarded:  Option<String>,     // the `X-Forwarded-For` the relay added, if it did
    host:       Option<String>,     // the `Host` header, which the relay owns
}

/// Start an echo WebSocket server on loopback.
///
/// It handshakes with `fe2o3_net`'s own machinery, echoes every text message back, and answers the
/// text `bye` with a close frame before dropping the connection -- which is how the close-
/// propagation test gets an upstream that goes away.
async fn spawn_echo_upstream(saw: Arc<Mutex<UpstreamSaw>>) -> Outcome<SocketAddr> {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => return Err(err!(e, "Could not bind the upstream listener."; IO, Network, Init)),
    };
    let addr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => return Err(err!(e, "Upstream listener has no address."; IO, Network)),
    };
    tokio::spawn(async move {
        let (mut stream, _peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!(err!(e, "Upstream accept failed."; IO, Network));
                return;
            }
        };
        // Read the upgrade request the relay forwarded.
        let read = HttpMessage::read::<1024, 1024, _>(
            Pin::new(&mut stream),
            &Vec::new(),
            Some(true),
            None,
        ).await;
        let req = match read {
            Ok((Some(m), _)) => m,
            Ok((None, _)) => {
                error!(err!("Upstream saw no request at all."; IO, Network, Missing));
                return;
            }
            Err(e) => {
                error!(err!(e, "Upstream could not read the request."; IO, Network, Read));
                return;
            }
        };
        // Record what arrived, then answer it.
        {
            let mut guard = match saw.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.path = match &req.header.headline {
                oxedyne_fe2o3_net::http::header::HttpHeadline::Request { loc, .. } =>
                    loc.path.as_string().to_string(),
                _ => String::new(),
            };
            guard.ws_key = header_of(&req, "Sec-WebSocket-Key");
            guard.forwarded = header_of(&req, "X-Forwarded-For");
            guard.host = header_of(&req, "Host");
        }
        let response = match accept_response(&req) {
            Ok(r) => r,
            Err(e) => {
                error!(e);
                return;
            }
        };
        if let Err(e) = stream.write_all(response.as_bytes()).await {
            error!(err!(e, "Upstream could not answer the upgrade."; IO, Network, Write));
            return;
        }
        // Echo until told otherwise.
        let mut buffer = Vec::new();
        loop {
            let msg = match read_message(&mut stream, &mut buffer, 1024, WebSocketLimits::default()).await {
                Ok(Some(m)) => m,
                Ok(None) => return,
                Err(e) => {
                    error!(e);
                    return;
                }
            };
            let reply = match msg {
                WebSocketMessage::Text(txt) if txt == "bye" => {
                    let close = WebSocketMessage::Close(None, None);
                    match encode_message(&close, false, 1024, 4096) {
                        Ok(b) => {
                            let _ = stream.write_all(&b).await;
                            let _ = stream.shutdown().await;
                            return;
                        }
                        Err(e) => {
                            error!(e);
                            return;
                        }
                    }
                }
                WebSocketMessage::Text(txt) => WebSocketMessage::Text(txt),
                WebSocketMessage::Binary(byts) => WebSocketMessage::Binary(byts),
                WebSocketMessage::Close(_, _) => return,
                other => {
                    error!(err!("Upstream got an unexpected {:?}.", other; Test, Unexpected));
                    return;
                }
            };
            match encode_message(&reply, false, 1024, 4096) {
                Ok(b) => {
                    if let Err(e) = stream.write_all(&b).await {
                        error!(err!(e, "Upstream could not echo."; IO, Network, Write));
                        return;
                    }
                }
                Err(e) => {
                    error!(e);
                    return;
                }
            }
        }
    });
    Ok(addr)
}

fn header_of(msg: &HttpMessage, name: &str) -> Option<String> {
    for (field_name, values) in msg.header.fields.iter() {
        if fmt!("{}", field_name).eq_ignore_ascii_case(name) {
            return values.first().map(|v| fmt!("{}", v));
        }
    }
    None
}

async fn read_header_block(stream: &mut tokio::io::DuplexStream) -> Outcome<String> {
    let mut accum: Vec<u8> = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) => return Err(err!(
                "The relay closed before sending a response header block.";
                IO, Network, Read, Missing)),
            Ok(n) => n,
            Err(e) => return Err(err!(e, "Reading the relayed response."; IO, Network, Read)),
        };
        accum.extend_from_slice(&buf[..n]);
        if accum.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if accum.len() > 65536 {
            return Err(err!("Response header block never ended."; IO, Network, TooBig));
        }
    }
    Ok(String::from_utf8_lossy(&accum).to_string())
}

/// The whole hop: a browser's handshake reaches a real WebSocket server through the relay, its
/// accept value checks out, and text goes both ways.
#[tokio::test]
async fn test_ws_route_relays_handshake_and_bytes_00() -> Outcome<()> {
    let saw = Arc::new(Mutex::new(UpstreamSaw::default()));
    let upstream = res!(spawn_echo_upstream(saw.clone()).await);

    let (mut browser, mut steel) = tokio::io::duplex(8192);
    let (request, key) = res!(connect_request("oxegen.example"));
    let src: SocketAddr = res!("203.0.113.9:51000".parse::<SocketAddr>(), Test);

    let relay = tokio::spawn(async move {
        tunnel_upgrade(
            &mut steel,
            &request,
            "127.0.0.1",
            upstream.port(),
            "/ws",
            src,
            &ForwardedPolicy::none(),
            "Test|ws",
        ).await
    });

    // The 101 arrives, carrying the accept value derived from the key the browser chose. Nothing
    // in the relay computes it: the upstream did.
    let head = res!(read_header_block(&mut browser).await);
    assert!(head.starts_with("HTTP/1.1 101 "),
        "expected a 101, got: {}", head.lines().next().unwrap_or(""));
    let expected = accept_key(&key);
    assert!(head.contains(&fmt!("Sec-WebSocket-Accept: {}", expected)),
        "the accept value must be the one derived from this handshake's key; got: {}", head);

    // A masked client frame goes up, and the echo comes back.
    let out = res!(client_frame(&WebSocketMessage::Text("oxe1:hello".to_string())));
    res!(browser.write_all(&out).await, IO, Network, Write);
    let mut buffer = Vec::new();
    match res!(read_message(&mut browser, &mut buffer, 1024, WebSocketLimits::default()).await) {
        Some(WebSocketMessage::Text(txt)) => assert_eq!(txt, "oxe1:hello"),
        other => return Err(err!("Expected the echo, got {:?}.", other; Test, Mismatch)),
    }

    // And a second one, to show the tunnel is still open rather than one-shot.
    let out = res!(client_frame(&WebSocketMessage::Text("second".to_string())));
    res!(browser.write_all(&out).await, IO, Network, Write);
    match res!(read_message(&mut browser, &mut buffer, 1024, WebSocketLimits::default()).await) {
        Some(WebSocketMessage::Text(txt)) => assert_eq!(txt, "second"),
        other => return Err(err!("Expected the second echo, got {:?}.", other; Test, Mismatch)),
    }

    // What the upstream saw: the client's key survived the hop (or the accept value above could
    // not have matched), the path is the configured upstream one, and the relay named itself.
    let seen = match saw.lock() {
        Ok(g) => g.clone(),
        Err(_) => return Err(err!("The upstream's record is poisoned."; Test, Poisoned)),
    };
    assert_eq!(seen.path, "/ws");
    assert_eq!(seen.ws_key.as_deref(), Some(key.as_str()));
    assert_eq!(seen.forwarded.as_deref(), Some("203.0.113.9:51000"));
    assert_eq!(seen.host.as_deref(), Some("127.0.0.1"),
        "the Host header belongs to this hop, not to the browser's original");

    // Closing from the far end ends the relay.
    let out = res!(client_frame(&WebSocketMessage::Text("bye".to_string())));
    res!(browser.write_all(&out).await, IO, Network, Write);
    match res!(read_message(&mut browser, &mut buffer, 1024, WebSocketLimits::default()).await) {
        Some(WebSocketMessage::Close(_, _)) => (),
        other => return Err(err!(
            "Expected the upstream's close frame, got {:?}.", other; Test, Mismatch)),
    }
    // And with the upstream gone, the client's side ends too.
    match res!(read_message(&mut browser, &mut buffer, 1024, WebSocketLimits::default()).await) {
        None => (),
        other => return Err(err!(
            "Expected the relay to close after the upstream did, got {:?}.", other;
            Test, Mismatch)),
    }
    match tokio::time::timeout(Duration::from_secs(5), relay).await {
        Ok(Ok(outcome)) => res!(outcome),
        Ok(Err(e)) => return Err(err!(e, "The relay task panicked."; Test)),
        Err(_) => return Err(err!(
            "The relay did not return after both ends closed."; Test, Timeout)),
    }
    Ok(())
}

/// A browser that goes away takes the tunnel with it, rather than leaving the relay holding a
/// socket nobody is reading.
#[tokio::test]
async fn test_ws_route_client_close_ends_the_tunnel_00() -> Outcome<()> {
    let saw = Arc::new(Mutex::new(UpstreamSaw::default()));
    let upstream = res!(spawn_echo_upstream(saw).await);

    let (mut browser, mut steel) = tokio::io::duplex(8192);
    let (request, _key) = res!(connect_request("oxegen.example"));
    let src: SocketAddr = res!("203.0.113.9:51001".parse::<SocketAddr>(), Test);

    let relay = tokio::spawn(async move {
        tunnel_upgrade(
            &mut steel, &request, "127.0.0.1", upstream.port(), "/ws", src,
            &ForwardedPolicy::none(), "Test|ws",
        ).await
    });

    let head = res!(read_header_block(&mut browser).await);
    assert!(head.starts_with("HTTP/1.1 101 "), "expected a 101, got: {}", head);

    // Drop the browser end.
    drop(browser);

    match tokio::time::timeout(Duration::from_secs(5), relay).await {
        Ok(Ok(outcome)) => res!(outcome),
        Ok(Err(e)) => return Err(err!(e, "The relay task panicked."; Test)),
        Err(_) => return Err(err!(
            "The relay did not return after the client went away."; Test, Timeout)),
    }
    Ok(())
}

/// An upstream that refuses the upgrade has its refusal relayed verbatim. A client is entitled to
/// read the answer the server gave, not one this hop invented.
#[tokio::test]
async fn test_ws_route_relays_a_refusal_00() -> Outcome<()> {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => return Err(err!(e, "Could not bind the grumpy upstream."; IO, Network, Init)),
    };
    let addr = res!(listener.local_addr(), IO, Network);
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").await;
            let _ = stream.shutdown().await;
        }
    });

    let (mut browser, mut steel) = tokio::io::duplex(8192);
    let (request, _key) = res!(connect_request("oxegen.example"));
    let src: SocketAddr = res!("203.0.113.9:51002".parse::<SocketAddr>(), Test);
    let relay = tokio::spawn(async move {
        tunnel_upgrade(
            &mut steel, &request, "127.0.0.1", addr.port(), "/ws", src,
            &ForwardedPolicy::none(), "Test|ws",
        ).await
    });

    let head = res!(read_header_block(&mut browser).await);
    assert!(head.starts_with("HTTP/1.1 403 "),
        "the upstream's refusal must reach the client unchanged; got: {}", head);
    match tokio::time::timeout(Duration::from_secs(5), relay).await {
        Ok(Ok(outcome)) => res!(outcome),
        Ok(Err(e)) => return Err(err!(e, "The relay task panicked."; Test)),
        Err(_) => return Err(err!("The relay did not return."; Test, Timeout)),
    }
    Ok(())
}

/// An upstream that is not listening is an error, not a hang and not a silent 101.
#[tokio::test]
async fn test_ws_route_errors_when_upstream_is_absent_00() -> Outcome<()> {
    // Bind and drop, so the port is one nothing is listening on.
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => return Err(err!(e, "Could not bind to find a free port."; IO, Network, Init)),
    };
    let addr = res!(listener.local_addr(), IO, Network);
    drop(listener);

    let (_browser, mut steel) = tokio::io::duplex(8192);
    let (request, _key) = res!(connect_request("oxegen.example"));
    let src: SocketAddr = res!("203.0.113.9:51003".parse::<SocketAddr>(), Test);
    let result = tunnel_upgrade(
        &mut steel, &request, "127.0.0.1", addr.port(), "/ws", src,
        &ForwardedPolicy::none(), "Test|ws",
    ).await;
    assert!(result.is_err(), "an absent upstream must be reported");
    Ok(())
}

/// The configuration a `ws_route` is written in: the URL's parts, and the defaults.
#[test]
fn test_ws_route_parses_its_upstream_00() -> Outcome<()> {
    let route = res!(WsRoute::from_datmap(&mapdat!{
        "path"      => "/ws",
        "upstream"  => "ws://127.0.0.1:9080/ws",
    }.get_map().unwrap_or_default()));
    assert_eq!(route.path, "/ws");
    assert_eq!(route.upstream_host, "127.0.0.1");
    assert_eq!(route.upstream_port, 9080);
    assert_eq!(route.upstream_path, "/ws");
    assert!(route.matches("/ws"));
    assert!(!route.matches("/ws/"), "the match is exact, not a prefix");
    assert!(!route.matches("/wsx"));

    // No port and no path: the HTTP default port, and the root.
    let route = res!(WsRoute::from_datmap(&mapdat!{
        "path"      => "/socket",
        "upstream"  => "ws://gateway.internal",
    }.get_map().unwrap_or_default()));
    assert_eq!(route.upstream_host, "gateway.internal");
    assert_eq!(route.upstream_port, 80);
    assert_eq!(route.upstream_path, "/");

    // The local path and the upstream path need not agree.
    let route = res!(WsRoute::from_datmap(&mapdat!{
        "path"      => "/ws",
        "upstream"  => "ws://127.0.0.1:9080/gateway/v0",
    }.get_map().unwrap_or_default()));
    assert_eq!(route.upstream_path, "/gateway/v0");
    Ok(())
}

/// What a `ws_route` refuses to be configured as. Each of these would otherwise become a surprise
/// at runtime, on a connection an operator is watching.
#[test]
fn test_ws_route_refuses_bad_configuration_00() -> Outcome<()> {
    let missing_upstream = mapdat!{ "path" => "/ws" };
    assert!(WsRoute::from_datmap(&missing_upstream.get_map().unwrap_or_default()).is_err(),
        "a route with no upstream is not a route");

    let missing_path = mapdat!{ "upstream" => "ws://127.0.0.1:9080/ws" };
    assert!(WsRoute::from_datmap(&missing_path.get_map().unwrap_or_default()).is_err(),
        "a route with no local path claims nothing");

    let tls = mapdat!{ "path" => "/ws", "upstream" => "wss://example.com/ws" };
    assert!(WsRoute::from_datmap(&tls.get_map().unwrap_or_default()).is_err(),
        "wss:// must be refused rather than quietly spoken as plaintext");

    let http = mapdat!{ "path" => "/ws", "upstream" => "http://127.0.0.1:9080/ws" };
    assert!(WsRoute::from_datmap(&http.get_map().unwrap_or_default()).is_err(),
        "a scheme other than ws:// must be refused");

    let bad_port = mapdat!{ "path" => "/ws", "upstream" => "ws://127.0.0.1:notaport/ws" };
    assert!(WsRoute::from_datmap(&bad_port.get_map().unwrap_or_default()).is_err(),
        "a port that is not a number must be refused");

    let no_host = mapdat!{ "path" => "/ws", "upstream" => "ws:///ws" };
    assert!(WsRoute::from_datmap(&no_host.get_map().unwrap_or_default()).is_err(),
        "an upstream with no host must be refused");
    Ok(())
}

/// A vhost written before `ws_routes` existed must still parse, and must have none. This is the
/// property that keeps every deployed config working: a new field that is required breaks them all.
#[test]
fn test_vhost_without_ws_routes_still_parses_00() -> Outcome<()> {
    let vhost = mapdat!{
        "hostnames"         => listdat!["example.com"],
        "public_dir_rel"    => "./www",
    };
    let cfg = res!(VhostConfig::from_datmap(&vhost.get_map().unwrap_or_default()));
    assert!(cfg.ws_routes.is_empty(), "a config that names no ws_routes has none");

    let with_routes = mapdat!{
        "hostnames"         => listdat!["example.com"],
        "public_dir_rel"    => "./www",
        "ws_routes"         => listdat![mapdat!{
            "path"      => "/ws",
            "upstream"  => "ws://127.0.0.1:9080/ws",
        }],
    };
    let cfg = res!(VhostConfig::from_datmap(&with_routes.get_map().unwrap_or_default()));
    assert_eq!(cfg.ws_routes.len(), 1);
    assert_eq!(cfg.ws_routes[0].upstream_port, 9080);

    // A malformed entry is a start-up failure, not a route silently dropped.
    let broken = mapdat!{
        "hostnames" => listdat!["example.com"],
        "ws_routes" => listdat!["/ws"],
    };
    assert!(VhostConfig::from_datmap(&broken.get_map().unwrap_or_default()).is_err(),
        "a ws_routes entry that is not a map must fail the parse");
    Ok(())
}

/// The relay never has to be told the client's address twice: `TcpStream` is only mentioned here so
/// the test file's imports match what a caller uses.
#[allow(dead_code)]
fn _unused(_: TcpStream) {}
