//! What a caller may and may not tell an upstream about the hop it took.
//!
//! A reverse proxy appends `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host` and RFC 7239
//! `Forwarded` describing the connection it actually accepted. If the caller's own copies ride
//! through as well, the upstream receives two values -- the caller's first, the hop's second -- and
//! `HeaderFields::get_one` returns `list[0]`. An upstream doing the obvious thing therefore reads
//! whatever the caller invented.
//!
//! These tests assert on the bytes the builders in `http::fwd` produce, and then on what the wire
//! parser makes of those bytes. Asserting on the string alone would prove what was written;
//! asserting through the parser proves what a reader of it actually gets, which is the property an
//! upstream depends on.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::HeaderName,
    fwd::{
        build_proxy_request_head,
        build_upgrade_request_head,
        ForwardedPolicy,
    },
    msg::HttpMessage,
};

use std::{
    net::SocketAddr,
    pin::Pin,
};

use tokio::io::AsyncWriteExt;


/// Through the real wire parser: building the message instead would skip the name normalisation the
/// parser does, and the point of these tests is what happens to bytes that arrived from outside.
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

/// The forwarding header name as the wire parser produces it.
fn xff() -> HeaderName {
    HeaderName::from("x-forwarded-for")
}

/// A caller's request carrying a full set of forged forwarding headers.
fn forged_upgrade() -> &'static str {
    "GET /ws HTTP/1.1\r\n\
    Host: app.example\r\n\
    Upgrade: websocket\r\n\
    X-Forwarded-For: 9.9.9.9\r\n\
    x-forwarded-for: 8.8.8.8, 7.7.7.7\r\n\
    X-FORWARDED-FOR: 6.6.6.6\r\n\
    X-Forwarded-Proto: http\r\n\
    X-Forwarded-Host: evil.example\r\n\
    Forwarded: for=9.9.9.9;proto=http;host=evil.example\r\n\
    \r\n"
}

/// With nobody trusted, every forged forwarding header is dropped and this hop's own appended.
///
/// The forgery is sent in three casings and as a chain, because a caller chooses all of that. What
/// the upstream must see is one value of each, and it must be the hop's.
#[tokio::test]
async fn test_upgrade_head_strips_a_forgery_00() -> Outcome<()> {
    let request = res!(parse_request(forged_upgrade()).await);
    let peer: SocketAddr = res!("203.0.113.7:51000".parse::<SocketAddr>(), Test);
    let head = build_upgrade_request_head(
        "/ws", "127.0.0.1", &request, &peer, &ForwardedPolicy::none());

    assert_eq!(values_of(&head, "x-forwarded-for"), vec![fmt!("203.0.113.7:51000")],
        "head was:\n{}", head);
    assert_eq!(values_of(&head, "x-forwarded-proto"), vec![fmt!("https")],
        "head was:\n{}", head);
    assert_eq!(values_of(&head, "x-forwarded-host"), vec![fmt!("app.example")],
        "the upstream must see the host the client addressed, once; head was:\n{}", head);
    assert_eq!(values_of(&head, "forwarded"),
        vec![fmt!("for=\"203.0.113.7:51000\";proto=https;host=\"app.example\"")],
        "head was:\n{}", head);
    for forged in ["9.9.9.9", "8.8.8.8", "7.7.7.7", "6.6.6.6", "evil.example"] {
        assert!(!head.contains(forged),
            "the forged value '{}' reached the upstream:\n{}", forged, head);
    }

    // The caller's own headers still ride through. A strip that took the rest with it would be a
    // different outage, and the upgrade cannot complete without these.
    assert_eq!(values_of(&head, "upgrade"), vec![fmt!("websocket")], "head was:\n{}", head);
    assert_eq!(values_of(&head, "connection"), vec![fmt!("Upgrade")], "head was:\n{}", head);
    assert_eq!(values_of(&head, "host"), vec![fmt!("127.0.0.1")],
        "the Host belongs to this hop, not to the caller's original; head was:\n{}", head);
    assert!(head.ends_with("\r\n\r\n"), "the head must end with a blank line:\n{}", head);
    Ok(())
}

/// A caller that sends nothing still has this hop's account appended.
///
/// Stripping is not the whole job. If the strip ran and the append did not, the upstream would fall
/// back to its socket peer -- which behind a proxy is loopback, and loopback is the address a
/// trusted gateway path is written for.
#[tokio::test]
async fn test_this_hop_names_itself_when_the_caller_said_nothing_00() -> Outcome<()> {
    let raw = "GET /ws HTTP/1.1\r\n\
        Host: app.example\r\n\
        Upgrade: websocket\r\n\
        \r\n";
    let request = res!(parse_request(raw).await);
    let peer: SocketAddr = res!("198.51.100.200:51004".parse::<SocketAddr>(), Test);
    let head = build_upgrade_request_head(
        "/ws", "127.0.0.1", &request, &peer, &ForwardedPolicy::none());

    assert_eq!(values_of(&head, "x-forwarded-for"), vec![fmt!("198.51.100.200:51004")]);
    assert_eq!(values_of(&head, "x-forwarded-proto"), vec![fmt!("https")]);
    assert_eq!(values_of(&head, "x-forwarded-host"), vec![fmt!("app.example")]);
    assert_eq!(values_of(&head, "forwarded"),
        vec![fmt!("for=\"198.51.100.200:51004\";proto=https;host=\"app.example\"")]);
    Ok(())
}

/// The same properties on the HTTP proxy head, and the trusted branch on the same call site.
///
/// The string asserted on here is the one written to the upstream socket, headers and terminating
/// blank line included -- a caller writes exactly this and then the body.
#[tokio::test]
async fn test_http_proxy_head_strips_and_appends_00() -> Outcome<()> {
    let raw = "POST /api/thing HTTP/1.1\r\n\
        Host: app.example\r\n\
        Content-Type: application/json\r\n\
        Content-Length: 2\r\n\
        X-Forwarded-For: 9.9.9.9\r\n\
        X-Forwarded-Proto: http\r\n\
        X-Forwarded-Host: evil.example\r\n\
        Forwarded: for=9.9.9.9\r\n\
        \r\n{}";
    let request = res!(parse_request(raw).await);
    let peer: SocketAddr = res!("203.0.113.7:52000".parse::<SocketAddr>(), Test);

    let head = build_proxy_request_head(
        "POST", "/thing", "127.0.0.1", &request, &peer, &ForwardedPolicy::none(), 2);

    assert_eq!(values_of(&head, "x-forwarded-for"), vec![fmt!("203.0.113.7:52000")],
        "head was:\n{}", head);
    assert_eq!(values_of(&head, "x-forwarded-proto"), vec![fmt!("https")],
        "head was:\n{}", head);
    assert_eq!(values_of(&head, "x-forwarded-host"), vec![fmt!("app.example")],
        "head was:\n{}", head);
    assert_eq!(values_of(&head, "forwarded"),
        vec![fmt!("for=\"203.0.113.7:52000\";proto=https;host=\"app.example\"")],
        "head was:\n{}", head);
    assert!(!head.contains("9.9.9.9") && !head.contains("evil.example"),
        "a forgery reached the upstream:\n{}", head);

    // The hop's own framing, and the caller's content type untouched.
    assert!(head.starts_with("POST /thing HTTP/1.1\r\nHost: 127.0.0.1\r\n"), "head was:\n{}", head);
    assert_eq!(values_of(&head, "content-length"), vec![fmt!("2")], "head was:\n{}", head);
    assert_eq!(values_of(&head, "connection"), vec![fmt!("close")], "head was:\n{}", head);
    assert_eq!(values_of(&head, "content-type"), vec![fmt!("application/json")],
        "head was:\n{}", head);
    assert!(head.ends_with("\r\n\r\n"), "the head must end with a blank line:\n{}", head);

    // And with the peer trusted, the same call site preserves the chain. This is what a content
    // delivery network needs: strip unconditionally and the real client address is discarded
    // rather than preserved, which is the same bug wearing a safer face.
    let policy = res!(ForwardedPolicy::new(&[fmt!("203.0.113.7")]));
    let head = build_proxy_request_head(
        "POST", "/thing", "127.0.0.1", &request, &peer, &policy, 2);
    assert_eq!(values_of(&head, "x-forwarded-for"),
        vec![fmt!("9.9.9.9"), fmt!("203.0.113.7:52000")], "head was:\n{}", head);
    assert_eq!(values_of(&head, "x-forwarded-proto"), vec![fmt!("http"), fmt!("https")],
        "head was:\n{}", head);
    Ok(())
}

/// A `Host` that could not be a host is not repeated into a header this hop writes.
///
/// The value would otherwise be quoted into `Forwarded`, where a `"` ends the quoted string early
/// and the rest becomes parameters this hop never wrote.
#[tokio::test]
async fn test_an_unsafe_host_is_not_repeated_00() -> Outcome<()> {
    let raw = "GET /thing HTTP/1.1\r\n\
        Host: app.example\";proto=http;secret=\"x\r\n\
        \r\n";
    let request = res!(parse_request(raw).await);
    let peer: SocketAddr = res!("203.0.113.7:52001".parse::<SocketAddr>(), Test);
    let head = build_proxy_request_head(
        "GET", "/thing", "127.0.0.1", &request, &peer, &ForwardedPolicy::none(), 0);

    assert!(values_of(&head, "x-forwarded-host").is_empty(),
        "a host outside the host grammar is dropped, not passed on:\n{}", head);
    assert_eq!(values_of(&head, "forwarded"), vec![fmt!("for=\"203.0.113.7:52001\";proto=https")],
        "head was:\n{}", head);
    Ok(())
}

/// **The invariant, read the way an upstream reads it: this hop's value is last, either way.**
///
/// Untrusted, there is one value and it is this hop's. Trusted, the caller's chain is kept and this
/// hop's is appended after it. So `get_last` returns this hop's under both configurations, while
/// `get_one` returns the caller's whenever there is a caller's to return -- which is the whole
/// reason `get_last` exists.
///
/// The head is put back through the wire parser rather than scanned as a string, because what is
/// under test is what a reader of the message gets, not what the writer believed it wrote.
#[tokio::test]
async fn test_this_hops_value_is_last_under_either_policy_00() -> Outcome<()> {
    let request = res!(parse_request(forged_upgrade()).await);
    let peer: SocketAddr = res!("203.0.113.7:51000".parse::<SocketAddr>(), Test);

    // Untrusted: the caller's copies are gone, and the one value left is this hop's.
    let head = build_upgrade_request_head(
        "/ws", "127.0.0.1", &request, &peer, &ForwardedPolicy::none());
    let seen = res!(parse_request(&head).await);
    let last = res!(seen.header.fields.get_last(&xff()).ok_or_else(|| err!(
        "This hop always appends its own X-Forwarded-For."; Test, Missing)));
    assert_eq!(fmt!("{}", last), "203.0.113.7:51000",
        "untrusted: the last value must be this hop's; head was:\n{}", head);
    let first = res!(seen.header.fields.get_one(&xff()).ok_or_else(|| err!(
        "This hop always appends its own X-Forwarded-For."; Test, Missing)));
    assert_eq!(fmt!("{}", first), "203.0.113.7:51000",
        "untrusted: there is only one value, so first and last agree; head was:\n{}", head);

    // Trusted: the caller's chain survives, and this hop's value still comes last.
    let policy = res!(ForwardedPolicy::new(&[fmt!("203.0.113.0/24")]));
    let head = build_upgrade_request_head("/ws", "127.0.0.1", &request, &peer, &policy);
    let seen = res!(parse_request(&head).await);
    let last = res!(seen.header.fields.get_last(&xff()).ok_or_else(|| err!(
        "This hop always appends its own X-Forwarded-For."; Test, Missing)));
    assert_eq!(fmt!("{}", last), "203.0.113.7:51000",
        "trusted: the last value must still be this hop's; head was:\n{}", head);
    let first = res!(seen.header.fields.get_one(&xff()).ok_or_else(|| err!(
        "The caller's chain was preserved."; Test, Missing)));
    assert_ne!(fmt!("{}", first), "203.0.113.7:51000",
        "trusted: the first value is the caller's, which is the trap get_last avoids; head was:\n{}",
        head);
    Ok(())
}
