//! An `api_routes` proxy whose upstream cannot answer is a 502, not a connection closed on nothing.
//!
//! `forward_api_proxy` returned the outbound client's error, `handle_get` and `handle_post`
//! passed it up with `res!`, and the connection was dropped with no response: the visitor of an
//! `api_routes` entry whose upstream was down, or spoke something that is not HTTP, was told
//! nothing at all. A `proxy_routes` entry answers 502 in the same case; this holds `api_routes`
//! to it. Found while rehearsing ontheism.org's move to Steel, where the same empty reply first
//! showed as the refusal of an HTTP/1.0 upstream (see `api_http10.rs`).
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

/// A port nothing listens on: bound, noted and released.
async fn closed_port() -> Outcome<u16> {
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
    Ok(res!(listener.local_addr(), Network, Init).port())
}

/// An upstream that is down, or that answers with something that is not HTTP at all, is a 502.
#[test]
fn an_api_route_whose_upstream_cannot_answer_is_a_502() -> Outcome<()> {
    let web = res!(Scratch::new("api_502"));
    res!(web.put("index.html", "<p>home</p>"));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        let dead = res!(closed_port().await);
        // Speaks, but not HTTP.
        let babble = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let babble_port = res!(babble.local_addr(), Network, Init).port();
        tokio::spawn(async move {
            loop {
                let (mut tcp, _) = match babble.accept().await {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 1024];
                let _ = tcp.read(&mut buf).await;
                let _ = tcp.write_all(b"this is not a status line\r\n\r\n").await;
                let _ = tcp.shutdown().await;
            }
        });
        let mut site = Site::new(HOST, &web.0);
        site.api_routes = vec![
            route("/down",      "/x", dead),
            route("/babble",    "/x", babble_port),
        ];
        let rig = res!(Rig::new(vec![site]));
        for path in ["/down", "/babble"] {
            for method in ["GET", "POST"] {
                let r = res!(rig.fetch(HOST, method, path, &[], "{}").await);
                assert_eq!(r.status(), 502, "{} {} must be a 502: {:?}", method, path, r);
                assert!(r.body.contains("Bad Gateway"), "{} {}: {:?}", method, path, r);
            }
        }
        Ok(())
    })
}
