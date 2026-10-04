//! An `api_routes` proxy to an HTTP/1.0 upstream, such as Python's `http.server`.
//!
//! The rehearsal of ontheism.org's move to Steel found an `api_routes` entry pointing at
//! `http.server` answering the visitor with nothing at all. The outbound client refused the
//! upstream's `HTTP/1.0` status line as an unrecognised version, the error left `handle_get`
//! unhandled, and the connection was dropped.
//!
//! A real HTTP/1.0 upstream is stood up here on loopback, written byte by byte rather than with a
//! library so that it says exactly what `http.server` says, and a vhost proxies four paths to it
//! through `handle_https` over TLS: a reply with a `Content-Length`; one with none, which
//! HTTP/1.0 ends by closing the connection; a `POST` echoed back; and a `204`. What is held: the
//! visitor gets the status and the whole body, in HTTP/1.1 and not in the upstream's version; and
//! a visitor who asks Steel itself in HTTP/1.0 is answered too, since one parser reads both.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::ApiRoute;

use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::TcpListener,
};

const HOST: &str = "api.test";

fn body_of(len: usize) -> String {
    (0..len).map(|i| (b'a' + (i % 26) as u8) as char).collect()
}

/// An upstream that answers `HTTP/1.0` and closes after each reply, as `http.server` does.
async fn upstream() -> Outcome<u16> {
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
    let port = res!(listener.local_addr(), Network, Init).port();
    tokio::spawn(async move {
        loop {
            let (mut tcp, _) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => return,
            };
            tokio::spawn(async move {
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
                let head = String::from_utf8_lossy(&buf[..split]).to_string();
                let want = head.lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < split + 4 + want {
                    match tcp.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let body = buf[split + 4..].to_vec();
                let line = head.lines().next().unwrap_or("").to_string();
                let mut out: Vec<u8> = Vec::new();
                if line.contains(" /with-length ") {
                    let text = body_of(9_000);
                    out.extend_from_slice(fmt!("HTTP/1.0 200 OK\r\nServer: BaseHTTP/0.6 Python/3\r\n\
                        Content-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                        text.len(), text).as_bytes());
                } else if line.contains(" /no-length ") {
                    out.extend_from_slice(fmt!("HTTP/1.0 200 OK\r\nServer: BaseHTTP/0.6 Python/3\r\n\
                        Content-Type: text/plain\r\n\r\n{}", body_of(9_000)).as_bytes());
                } else if line.contains(" /echo ") {
                    out.extend_from_slice(fmt!("HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\
                        Content-Length: {}\r\n\r\n", body.len()).as_bytes());
                    out.extend_from_slice(&body);
                } else if line.contains(" /empty ") {
                    out.extend_from_slice(b"HTTP/1.0 204 No Content\r\n\r\n");
                } else {
                    out.extend_from_slice(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                }
                let _ = tcp.write_all(&out).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    Ok(port)
}

fn route(path: &str, upstream_path: &str, port: u16) -> ApiRoute {
    ApiRoute {
        path:           path.to_string(),
        upstream_host:  Some("127.0.0.1".to_string()),
        upstream_port:  Some(port),
        upstream_path:  Some(upstream_path.to_string()),
        upstream_tls:   false,
        headers:        Vec::new(),
        handler:        None,
        config:         Vec::new(),
    }
}

#[test]
fn an_api_route_reaches_an_http_1_0_upstream() -> Outcome<()> {
    let web = res!(Scratch::new("api_http10"));
    res!(web.put("index.html", "<p>home</p>"));
    res!(web.put("file.txt", "a plain file"));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        let port = res!(upstream().await);
        let mut site = Site::new(HOST, &web.0);
        site.api_routes = vec![
            route("/h10/with-length",   "/with-length", port),
            route("/h10/no-length",     "/no-length",   port),
            route("/h10/echo",          "/echo",        port),
            route("/h10/empty",         "/empty",       port),
        ];
        let rig = res!(Rig::new(vec![site]));

        // A reply that states its length, and one that does not and ends with the connection.
        for path in ["/h10/with-length", "/h10/no-length"] {
            let r = res!(rig.fetch(HOST, "GET", path, &[], "").await);
            assert!(r.head.starts_with("HTTP/1.1 200"),
                "GET {} must be answered in HTTP/1.1 with the upstream's 200: {:?}", path, r.head);
            assert_eq!(r.body.len(), 9_000, "GET {} lost part of the body", path);
            assert_eq!(r.body, body_of(9_000), "GET {}", path);
            assert_eq!(r.field("content-length").as_deref(), Some("9000"), "GET {}", path);
        }

        // A POST whose body the upstream echoes.
        let r = res!(rig.fetch(HOST, "POST", "/h10/echo",
            &["Content-Type: text/plain"], "round trip 0123456789").await);
        assert!(r.head.starts_with("HTTP/1.1 200"), "{:?}", r.head);
        assert_eq!(r.body, "round trip 0123456789");

        // A status that has no body ends at its head, with the connection left open by the
        // upstream or not.
        let r = res!(rig.fetch(HOST, "GET", "/h10/empty", &[], "").await);
        assert!(r.head.starts_with("HTTP/1.1 204"), "{:?}", r.head);
        assert!(r.body.is_empty(), "{:?}", r);

        // Steel is asked in HTTP/1.0 as well, and answers in HTTP/1.1, as RFC 9112 2.3 has a
        // server do; the same parser reads both, and the request used to be dropped unread.
        let r = res!(rig.fetch_as(HOST, "HTTP/1.0", "GET", "/file.txt", &[], "").await);
        assert!(r.head.starts_with("HTTP/1.1 200"), "{:?}", r.head);
        assert_eq!(r.body, "a plain file");
        let r = res!(rig.fetch_as(HOST, "HTTP/1.0", "GET", "/h10/no-length", &[], "").await);
        assert_eq!(r.body, body_of(9_000));
        Ok(())
    })
}
