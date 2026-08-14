//! That Steel actually uses the forwarding policy, on the path a request really takes.
//!
//! The policy itself, and the two request-head builders, live in `fe2o3_net::http::fwd` and are
//! tested there against the bytes they produce. What is left here is the question that crate cannot
//! answer: whether *Steel* reaches for them. A helper that behaves perfectly and is never called
//! strips nothing.
//!
//! So the WebSocket relay is driven end to end against a real socket -- Steel's `tunnel_upgrade`,
//! Steel's argument order, a capturing upstream recording what arrived -- and the configuration
//! that feeds the policy is loaded through `ServerConfig` the way a deployment loads it.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::http::{
    fwd::ForwardedPolicy,
    msg::HttpMessage,
};
use oxedyne_fe2o3_steel::srv::{
    cfg::ServerConfig,
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
    net::TcpListener,
};


/// Parse raw request bytes into an `HttpMessage` through the real wire parser.
///
/// Building the message rather than parsing it would skip the name normalisation the parser does,
/// and the point of these tests is what happens to bytes that arrived from outside.
async fn parse_request(raw: &str) -> Outcome<HttpMessage> {
    let (mut near, mut far) = tokio::io::duplex(8192);
    let bytes = raw.as_bytes().to_vec();
    tokio::spawn(async move {
        let _ = far.write_all(&bytes).await;
        let _ = far.flush().await;
    });
    let read = HttpMessage::read::<1024, 1024, _>(
        Pin::new(&mut near),
        &Vec::new(),
        Some(true),
        None,
    ).await;
    match res!(read) {
        (Some(msg), _) => Ok(msg),
        (None, _) => Err(err!("The test request did not parse."; Test, Invalid, Input)),
    }
}

/// Every value of a header, in the order it appears in a raw request head.
///
/// The comparison is case-insensitive on the name, so a head that names the field differently from
/// the test still has its values counted -- otherwise a strip that merely changed the case of a
/// forgery would read as a strip.
fn values_of(head: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in head.split("\r\n") {
        if let Some((held, value)) = line.split_once(':') {
            if held.trim().eq_ignore_ascii_case(name) {
                out.push(value.trim().to_string());
            }
        }
    }
    out
}

/// Run one upgrade through the relay and return the request head the upstream received.
///
/// The upstream is a bare socket that records what arrived and answers a `101`, so what is asserted
/// on is the wire, not a parse of it.
async fn relay_and_capture(
    raw_request:    &str,
    peer:           &str,
    policy:         ForwardedPolicy,
)
    -> Outcome<String>
{
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => return Err(err!(e, "Could not bind the capturing upstream."; IO, Network, Init)),
    };
    let addr = res!(listener.local_addr(), IO, Network);
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_far = seen.clone();
    tokio::spawn(async move {
        let mut stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(_) => return,
        };
        let mut accum: Vec<u8> = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = match stream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => return,
            };
            accum.extend_from_slice(&buf[..n]);
            if accum.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if accum.len() > 65536 {
                break;
            }
        }
        if let Ok(mut guard) = seen_far.lock() {
            *guard = String::from_utf8_lossy(&accum).to_string();
        }
        // Any 101 will do: nothing here tests the handshake.
        let _ = stream.write_all(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n").await;
        let _ = stream.shutdown().await;
    });

    let request = res!(parse_request(raw_request).await);
    let (browser, mut steel) = tokio::io::duplex(8192);
    let src: SocketAddr = res!(peer.parse::<SocketAddr>(), Test);
    let relay = tokio::spawn(async move {
        tunnel_upgrade(
            &mut steel, &request, "127.0.0.1", addr.port(), "/ws", src, &policy, "Test|fwd",
        ).await
    });
    // The upstream drops the connection after its 101, which ends the relay.
    let _ = tokio::time::timeout(Duration::from_secs(5), relay).await;
    drop(browser);

    let head = match seen.lock() {
        Ok(g) => g.clone(),
        Err(_) => return Err(err!("The upstream's record is poisoned."; Test, Poisoned)),
    };
    if head.is_empty() {
        return Err(err!("The upstream received nothing."; Test, Missing));
    }
    Ok(head)
}

/// A forged `X-Forwarded-For` never reaches the upstream when no peer is trusted.
///
/// The forgery is sent in three casings and as a chain, because a caller chooses all of that. What
/// the upstream must see is one value, and it must be the address Steel accepted the connection
/// from.
#[tokio::test]
async fn test_forged_x_forwarded_for_is_stripped_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        X-Forwarded-For: 9.9.9.9\r\n\
        x-forwarded-for: 8.8.8.8, 7.7.7.7\r\n\
        X-FORWARDED-FOR: 6.6.6.6\r\n\
        \r\n";
    let head = res!(relay_and_capture(raw, "203.0.113.7:51000", ForwardedPolicy::none()).await);

    let seen = values_of(&head, "x-forwarded-for");
    assert_eq!(seen, vec![fmt!("203.0.113.7:51000")],
        "the upstream must see one X-Forwarded-For, this hop's; got {:?} in head:\n{}",
        seen, head);
    for forged in ["9.9.9.9", "8.8.8.8", "7.7.7.7", "6.6.6.6"] {
        assert!(!head.contains(forged),
            "the forged address '{}' reached the upstream:\n{}", forged, head);
    }
    Ok(())
}

/// A forged `X-Forwarded-Proto` never reaches the upstream when no peer is trusted.
///
/// An upstream that reads the first value and finds `http` believes a TLS request arrived in
/// plaintext. One that redirects plaintext to HTTPS on that basis redirects a request that is
/// already HTTPS, and the client comes back to the same answer.
#[tokio::test]
async fn test_forged_x_forwarded_proto_is_stripped_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        X-Forwarded-Proto: http\r\n\
        \r\n";
    let head = res!(relay_and_capture(raw, "203.0.113.7:51001", ForwardedPolicy::none()).await);

    let seen = values_of(&head, "x-forwarded-proto");
    assert_eq!(seen, vec![fmt!("https")],
        "the upstream must see one X-Forwarded-Proto, this hop's; got {:?} in head:\n{}",
        seen, head);
    Ok(())
}

/// A forged `X-Forwarded-Host` and a forged RFC 7239 `Forwarded` are stripped too, and this hop's
/// own account of both is appended.
///
/// `X-Forwarded-Host` is the worst of the set left alone: Steel replaces `Host` with the upstream's
/// own, so without this the upstream has no truthful source for the host the client addressed --
/// only the caller's, which nothing checks.
#[tokio::test]
async fn test_forged_host_and_forwarded_are_stripped_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        X-Forwarded-Host: evil.example\r\n\
        Forwarded: for=9.9.9.9;proto=http;host=evil.example\r\n\
        \r\n";
    let head = res!(relay_and_capture(raw, "203.0.113.7:51002", ForwardedPolicy::none()).await);

    let seen = values_of(&head, "x-forwarded-host");
    assert_eq!(seen, vec![fmt!("app.example")],
        "the upstream must see the host the client addressed, once; got {:?} in head:\n{}",
        seen, head);
    let seen = values_of(&head, "forwarded");
    assert_eq!(seen, vec![fmt!("for=\"203.0.113.7:51002\";proto=https;host=\"app.example\"")],
        "the upstream must see one Forwarded, this hop's; got {:?} in head:\n{}", seen, head);
    assert!(!head.contains("evil.example"),
        "the forged host reached the upstream:\n{}", head);
    Ok(())
}

/// With the peer trusted, the caller's chain is preserved and this hop's value appended after it.
///
/// This is what a content delivery network needs: strip unconditionally and the real client address
/// is discarded rather than preserved, which is the same bug wearing a safer face. Steel's own
/// value is still last, so a downstream reader taking the last value is right under either
/// configuration.
#[tokio::test]
async fn test_trusted_peer_chain_is_preserved_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        X-Forwarded-For: 198.51.100.34\r\n\
        X-Forwarded-Proto: https\r\n\
        \r\n";
    let policy = res!(ForwardedPolicy::new(&[fmt!("203.0.113.0/24")]));
    let head = res!(relay_and_capture(raw, "203.0.113.7:51003", policy).await);

    let seen = values_of(&head, "x-forwarded-for");
    assert_eq!(seen, vec![fmt!("198.51.100.34"), fmt!("203.0.113.7:51003")],
        "a trusted peer's chain is kept and this hop appended to it; got {:?} in head:\n{}",
        seen, head);
    let seen = values_of(&head, "x-forwarded-proto");
    assert_eq!(seen, vec![fmt!("https"), fmt!("https")],
        "the proto chain is kept the same way; got {:?} in head:\n{}", seen, head);
    Ok(())
}

/// An untrusted peer that sends nothing still has this hop's account appended.
///
/// Stripping is not the whole job. If the strip ran and the append did not, the upstream would fall
/// back to its socket peer -- which behind Steel is loopback, and loopback is the address a trusted
/// gateway path is written for.
#[tokio::test]
async fn test_this_hop_names_itself_when_the_caller_said_nothing_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        \r\n";
    let head = res!(relay_and_capture(raw, "198.51.100.200:51004", ForwardedPolicy::none()).await);

    assert_eq!(values_of(&head, "x-forwarded-for"), vec![fmt!("198.51.100.200:51004")]);
    assert_eq!(values_of(&head, "x-forwarded-proto"), vec![fmt!("https")]);
    assert_eq!(values_of(&head, "x-forwarded-host"), vec![fmt!("app.example")]);
    assert_eq!(values_of(&head, "forwarded"),
        vec![fmt!("for=\"198.51.100.200:51004\";proto=https;host=\"app.example\"")]);
    Ok(())
}

/// The caller's own headers still reach the upstream. A strip that took the rest with it would be a
/// different outage.
#[tokio::test]
async fn test_the_callers_other_headers_still_ride_through_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
        Sec-WebSocket-Version: 13\r\n\
        Cookie: sid=abc123\r\n\
        X-Forwarded-For: 9.9.9.9\r\n\
        \r\n";
    let head = res!(relay_and_capture(raw, "203.0.113.7:51005", ForwardedPolicy::none()).await);

    assert_eq!(values_of(&head, "sec-websocket-key"), vec![fmt!("dGhlIHNhbXBsZSBub25jZQ==")]);
    assert_eq!(values_of(&head, "sec-websocket-version"), vec![fmt!("13")]);
    assert_eq!(values_of(&head, "cookie"), vec![fmt!("sid=abc123")]);
    assert_eq!(values_of(&head, "host"), vec![fmt!("127.0.0.1")],
        "the Host belongs to this hop, not to the caller's original");
    Ok(())
}

/// A configuration written before `trusted_proxies` existed still loads, and trusts nobody.
///
/// A field added without `#[optional]` is required, and a required field added to a struct backing
/// two live production configurations is an outage rather than a feature.
#[test]
fn test_a_config_without_trusted_proxies_still_loads_00() -> Outcome<()> {
    let mut m = DaticleMap::new();
    m.insert(dat!("tls_dir_rel"),                 dat!("./tls"));
    m.insert(dat!("log_level"),                   dat!("debug"));
    m.insert(dat!("num_server_bots"),             Dat::U16(1));
    m.insert(dat!("server_address"),              dat!("0.0.0.0"));
    m.insert(dat!("server_port_tcp"),             Dat::U16(8443));
    m.insert(dat!("server_port_tcp_plaintext"),   Dat::U16(0));
    m.insert(dat!("hsts_max_age_secs"),           Dat::U32(0));
    m.insert(dat!("session_expiry_default_secs"), Dat::U32(604_800));
    m.insert(dat!("ws_ping_interval_secs"),       Dat::U8(30));
    m.insert(dat!("server_max_errors_allowed"),   Dat::U8(30));
    m.insert(dat!("allow_anonymous_sessions"),    Dat::Bool(true));
    m.insert(dat!("vhosts"),                      Dat::List(Vec::new()));
    m.insert(dat!("acme"),                        Dat::Map(DaticleMap::new()));
    m.insert(dat!("mail"),                        Dat::Map(DaticleMap::new()));

    let cfg = res!(ServerConfig::from_datmap(m));
    assert!(cfg.trusted_proxies.is_empty(),
        "a config that names no trusted proxy trusts none");
    let policy = res!(cfg.get_forwarded_policy());
    assert!(policy.is_empty());
    assert!(!policy.trusts(&res!("127.0.0.1:4000".parse::<SocketAddr>(), Test)),
        "loopback is not trusted by default -- that exemption is what the hole was written into");
    Ok(())
}

/// A mistyped trusted proxy is a start-up failure, not an allow-list that silently trusts nobody.
#[test]
fn test_a_mistyped_trusted_proxy_is_refused_00() -> Outcome<()> {
    let mut cfg = ServerConfig::default();
    cfg.trusted_proxies = vec![fmt!("198.51.100.0/24"), fmt!("not-an-address")];
    assert!(cfg.get_forwarded_policy().is_err(),
        "an entry that cannot be parsed must be reported, not skipped");

    cfg.trusted_proxies = vec![fmt!("198.51.100.0/24")];
    let policy = res!(cfg.get_forwarded_policy());
    assert!(policy.trusts(&res!("198.51.100.1:80".parse::<SocketAddr>(), Test)));
    Ok(())
}
