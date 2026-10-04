//! What an `api_routes` proxy and a webhook forwarder do with an upstream that will not finish.
//!
//! An upstream that states no `Content-Length`, is not chunked and never closes the connection has
//! a body that ends nowhere. Since the outbound client learned to read such a body to the end of the
//! connection (for HTTP/1.0 servers such as Python's `http.server`), the forwarders, which set no
//! limit and no deadline, held the visitor's request open for ever and buffered whatever arrived
//! without bound. They are bounded now: the reply's size by the server's `http_max_*` limits (a
//! `502 Bad Gateway` over them), and the whole call by `upstream_timeout_ms` (a `504 Gateway
//! Timeout` over it).
//!
//! Each case stands up a misbehaving upstream on loopback and asks through `handle_https`, with the
//! whole ask under a deadline of its own so that a hang is a failed assertion and not a stuck test.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Reply,
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::{
    ApiRoute,
    ServerConfig,
    WebhookRoute,
};

use std::{
    sync::{
        atomic::{
            AtomicUsize,
            Ordering,
        },
        Arc,
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

const HOST: &str = "limits.test";
const CAP: usize = 64 * 1024;               // the reply size limit the rig's config sets
const SPILL: usize = 64 * 1024 * 1024;      // what the endless upstream would send, if let

#[derive(Clone, Copy, Debug)]
enum Upstream {
    Hangs,      // answers `HTTP/1.0 200` with a few bytes, then says nothing and holds the line
    Endless,    // answers `HTTP/1.0 200`, then sends without stopping and never closes
}

/// A loopback upstream; the second value counts the bytes it managed to send.
async fn upstream(kind: Upstream) -> Outcome<(u16, Arc<AtomicUsize>)> {
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
    let port = res!(listener.local_addr(), Network, Init).port();
    let sent = Arc::new(AtomicUsize::new(0));
    let counter = sent.clone();
    tokio::spawn(async move {
        loop {
            let (mut tcp, _) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let counter = counter.clone();
            tokio::spawn(async move {
                // The request head, then whatever body it promised.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let split = loop {
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i;
                    }
                    match tcp.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..split]).to_lowercase();
                let want = head.lines()
                    .filter_map(|l| l.strip_prefix("content-length:"))
                    .filter_map(|v| v.trim().parse::<usize>().ok())
                    .next()
                    .unwrap_or(0);
                while buf.len() < split + 4 + want {
                    match tcp.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                if tcp.write_all(b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\npartial").await.is_err() {
                    return;
                }
                match kind {
                    Upstream::Hangs => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    },
                    Upstream::Endless => {
                        let block = [b'x'; 16 * 1024];
                        while counter.load(Ordering::SeqCst) < SPILL {
                            match tcp.write_all(&block).await {
                                Ok(()) => { counter.fetch_add(block.len(), Ordering::SeqCst); },
                                Err(_) => return,   // Steel has hung up on it
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    },
                }
            });
        }
    });
    Ok((port, sent))
}

fn config() -> ServerConfig {
    let mut cfg = ServerConfig::default();
    cfg.http_max_body_bytes = CAP as u64;
    cfg.upstream_timeout_ms = 500;
    cfg
}

fn api(port: u16) -> ApiRoute {
    ApiRoute {
        path:           "/proxy".to_string(),
        upstream_host:  Some("127.0.0.1".to_string()),
        upstream_port:  Some(port),
        upstream_path:  Some("/x".to_string()),
        upstream_tls:   false,
        headers:        Vec::new(),
        handler:        None,
        config:         Vec::new(),
    }
}

fn hook(port: u16) -> WebhookRoute {
    WebhookRoute {
        path:           "/hook".to_string(),
        handler:        None,
        upstream_host:  Some("127.0.0.1".to_string()),
        upstream_port:  Some(port),
        upstream_path:  Some("/x".to_string()),
        upstream_tls:   false,
        config:         Vec::new(),
    }
}

/// One request through a vhost whose route points at the upstream, under a deadline of its own.
fn ask(label: &str, kind: Upstream, method: &str, path: &str) -> Outcome<(Reply, usize)> {
    let web = res!(Scratch::new(label));
    res!(web.put("index.html", "<p>home</p>"));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        let (port, sent) = res!(upstream(kind).await);
        let mut site = Site::new(HOST, &web.0);
        site.api_routes = vec![api(port)];
        site.webhook_routes = vec![hook(port)];
        let rig = res!(Rig::with_config(vec![site], config()));
        let fields: &[&str] = &[];
        let body = if method == "POST" { "{}" } else { "" };
        let reply = match tokio::time::timeout(
            Duration::from_secs(8),
            rig.fetch(HOST, method, path, fields, body),
        ).await {
            Ok(r) => res!(r),
            Err(_) => return Err(err!(
                "{} {} through a {:?} upstream was still unanswered after 8 s: the forwarder has \
                no deadline.", method, path, kind; Test, Timeout)),
        };
        Ok((reply, sent.load(Ordering::SeqCst)))
    })
}

#[test]
fn an_api_route_answers_504_when_its_upstream_never_finishes() -> Outcome<()> {
    let (r, _) = res!(ask("api_hangs", Upstream::Hangs, "GET", "/proxy"));
    assert_eq!(r.status(), 504, "a reply that never completes must be a 504: {:?}", r.head);
    Ok(())
}

#[test]
fn an_api_route_answers_502_and_stops_reading_when_the_reply_has_no_end() -> Outcome<()> {
    let (r, sent) = res!(ask("api_endless", Upstream::Endless, "GET", "/proxy"));
    assert_eq!(r.status(), 502, "a reply over the size limit must be a 502: {:?}", r.head);
    assert!(sent < SPILL / 2,
        "the forwarder read {} bytes of an endless reply with a {} byte limit", sent, CAP);
    Ok(())
}

#[test]
fn an_api_post_answers_504_when_its_upstream_never_finishes() -> Outcome<()> {
    let (r, _) = res!(ask("api_post_hangs", Upstream::Hangs, "POST", "/proxy"));
    assert_eq!(r.status(), 504, "{:?}", r.head);
    Ok(())
}

#[test]
fn a_webhook_forward_answers_504_when_its_upstream_never_finishes() -> Outcome<()> {
    let (r, _) = res!(ask("hook_hangs", Upstream::Hangs, "POST", "/hook"));
    assert_eq!(r.status(), 504, "a reply that never completes must be a 504: {:?}", r.head);
    Ok(())
}

#[test]
fn a_webhook_forward_answers_502_and_stops_reading_when_the_reply_has_no_end() -> Outcome<()> {
    let (r, sent) = res!(ask("hook_endless", Upstream::Endless, "POST", "/hook"));
    assert_eq!(r.status(), 502, "a reply over the size limit must be a 502: {:?}", r.head);
    assert!(sent < SPILL / 2,
        "the forwarder read {} bytes of an endless reply with a {} byte limit", sent, CAP);
    Ok(())
}
