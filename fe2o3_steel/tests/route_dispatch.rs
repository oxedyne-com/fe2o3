//! Route dispatch driven through `handle_https` against a real socket: an exact proxy route, and a
//! redirect that carries the query string.
//!
//! The unit tests in `srv::cfg` show that `ProxyRoute::matches` tells an exact route from a prefix
//! one, and that `RedirectRule::resolve_target` fills `{uri}`. What they cannot show is that the
//! dispatcher uses them, so this puts a TLS connection into `handle_https` with two vhosts: one whose
//! only route is an exact `/subscribe`, with a capturing upstream behind it that records every
//! request it is sent, and one that redirects everything to the first.
//!
//! What it holds for the proxy: the exact path reaches the upstream, with its body and one
//! `X-Forwarded-For` naming the client; the upstream's status and body come back untouched; the
//! neighbours of the path (`/subscribe.py`, `/subscribers.txt`, `/subscribe/`) never reach the
//! upstream; and an upstream that is down is a 502. For the redirect: the target names the path and
//! the query exactly as they arrived.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        api::ApiHandlerRegistry,
        cfg::{
            ProxyRoute,
            RedirectMatch,
            RedirectRule,
            ServerConfig,
        },
        context::{
            self,
            Protocol,
            ServerContext,
            VhostRuntime,
        },
        id,
        webhook::WebhookRegistry,
        ws::{
            handler::AppWebSocketHandler,
            syntax::WebSocketSyntax,
        },
    },
};

use oxedyne_fe2o3_core::{
    file::OsPath,
    path::NormalPath,
    prelude::*,
};
use oxedyne_fe2o3_jdat::version::SemVer;

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
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
use tokio_rustls::{
    rustls::{
        pki_types::{
            CertificateDer,
            PrivateKeyDer,
            PrivatePkcs8KeyDer,
            ServerName,
        },
        ClientConfig,
        RootCertStore,
        ServerConfig as TlsServerConfig,
    },
    TlsAcceptor,
    TlsConnector,
};

const HOST: &str = "subscribe.test";
const WWW: &str = "www.subscribe.test";

/// What the upstream was sent: the request line, the `X-Forwarded-For` lines, and the body.
#[derive(Clone, Debug)]
struct Seen {
    line:   String,
    xff:    Vec<String>,
    body:   String,
}

fn no_dbs<T>(_: fn(&Path, &[u8]) -> Outcome<T>)
    -> Arc<RwLock<HashMap<String, (Arc<RwLock<T>>, id::Uid)>>>
{
    Arc::new(RwLock::new(HashMap::new()))
}

/// A one-shot-per-connection HTTP upstream. It answers 400 when the body says `bad` and 200
/// otherwise, closing after each reply as `http.server` does, and records what it was sent.
async fn upstream(seen: Arc<Mutex<Vec<Seen>>>) -> Outcome<u16> {
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
    let port = res!(listener.local_addr(), Network, Init).port();
    tokio::spawn(async move {
        loop {
            let (mut tcp, _) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let seen = seen.clone();
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
                let body = String::from_utf8_lossy(&buf[split + 4..]).to_string();
                let xff = head.lines()
                    .filter_map(|l| l.split_once(':'))
                    .filter(|(k, _)| k.trim().eq_ignore_ascii_case("x-forwarded-for"))
                    .map(|(_, v)| v.trim().to_string())
                    .collect();
                let (code, text, reply) = if body.contains("bad") {
                    (400, "Bad Request", "{\"message\": \"Invalid request.\"}")
                } else {
                    (200, "OK", "{\"message\": \"Subscribed. Thank you.\"}")
                };
                if let Ok(mut g) = seen.lock() {
                    g.push(Seen {
                        line:   head.lines().next().unwrap_or("").to_string(),
                        xff,
                        body,
                    });
                }
                let out = fmt!("HTTP/1.0 {} {}\r\nContent-Type: application/json\r\n\
                    Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code, text, reply.len(), reply);
                let _ = tcp.write_all(out.as_bytes()).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    Ok(port)
}

/// A port nothing listens on: bound, noted and released.
async fn closed_port() -> Outcome<u16> {
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
    Ok(res!(listener.local_addr(), Network, Init).port())
}

fn route(path: &str, exact: bool, port: u16) -> ProxyRoute {
    ProxyRoute {
        path_prefix:    path.to_string(),
        exact,
        upstream_host:  "127.0.0.1".to_string(),
        upstream_port:  port,
        upstream_tls:   false,
        strip_prefix:   false,
    }
}

#[test]
fn an_exact_proxy_route_and_a_redirect_that_keeps_its_query() -> Outcome<()> {
    let runtime = res!(tokio::runtime::Runtime::new(), Init);
    runtime.block_on(async { drive().await })
}

async fn drive() -> Outcome<()> {
    oxedyne_fe2o3_net::tls::ensure_crypto_provider();
    let cert = res!(rcgen::generate_simple_self_signed(vec![HOST.to_string(), WWW.to_string()]), Init);
    let der = res!(cert.serialize_der(), Init);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    let server_tls = res!(TlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(der.clone())], key), Init);
    let mut roots = RootCertStore::empty();
    res!(roots.add(CertificateDer::from(der)), Init);
    let client_tls = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let tls = (Arc::new(server_tls), Arc::new(client_tls));

    let seen = Arc::new(Mutex::new(Vec::new()));
    let up_port = res!(upstream(seen.clone()).await);
    let dead_port = res!(closed_port().await);

    let cfg = ServerConfig::default();
    let web = || -> AppWebHandler<HashMap<String, OsPath>> {
        AppWebHandler::new(
            cfg.clone(),
            std::env::temp_dir().join(fmt!("steel_route_dispatch_{}", std::process::id())),
            HashMap::new(),
            vec![fmt!("index.html")],
            true,
            Vec::new(),
            Vec::new(),
            Arc::new(WebhookRegistry::new()),
            Arc::new(ApiHandlerRegistry::new()),
            None,
            None,
            None,
            None,
            None,
            Arc::new(Vec::new()),
        )
    };
    let vhost = |name: &str, redirects: Vec<RedirectRule>, proxy_routes: Vec<ProxyRoute>|
        -> Outcome<Arc<VhostRuntime<_, _>>>
    {
        Ok(Arc::new(VhostRuntime {
            hostnames:      vec![name.to_string()],
            web_handler:    web(),
            ws_handler:     AppWebSocketHandler::new(None),
            ws_syntax:      res!(WebSocketSyntax::new("steel_ws", &SemVer::new(0, 1, 0), "Route test")),
            redirects,
            proxy_routes,
            ws_routes:      Vec::new(),
            term_manager:   None,
            uses_sessions:  false,
            permissions_policy: None,
            tiles:          None,
            access_log:     false,
        }))
    };
    let mut vhosts = HashMap::new();
    vhosts.insert(HOST.to_string(), res!(vhost(HOST, Vec::new(), vec![
        route("/subscribe", true, up_port),
        route("/down", true, dead_port),
    ])));
    // The www vhost sends everything to the first, as the live configs do.
    vhosts.insert(WWW.to_string(), res!(vhost(WWW, vec![RedirectRule {
        match_kind: RedirectMatch::All,
        match_path: String::new(),
        target:     fmt!("https://{}{{uri}}", HOST),
        status:     301,
    }], Vec::new())));
    let protocol = Protocol::Web {
        vhosts:         Arc::new(vhosts),
        default_vhost:  HOST.to_string(),
        dev_mode:       true,
    };
    let root = std::env::temp_dir().normalise().absolute();
    let context = ServerContext::new(cfg, root, no_dbs(context::new_db), Vec::new(), protocol,
        None, None);

    // One request over a fresh TLS connection: the head, lower cased, and the body.
    let fetch = async |host: &str, method: &str, path: &str, body: &str| -> Outcome<(String, String)> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let addr = res!(listener.local_addr(), Network, Init);
        let ctx = context.clone();
        let acceptor = TlsAcceptor::from(tls.0.clone());
        let host_owned = host.to_string();
        let server = tokio::spawn(async move {
            let (tcp, peer) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => return Err(err!(e, "Accept failed."; Network)),
            };
            let stream = match acceptor.accept(tcp).await {
                Ok(s) => s,
                Err(e) => return Err(err!(e, "TLS accept failed."; Network)),
            };
            ctx.handle_https(stream, Some(host_owned), peer).await
        });
        let tcp = res!(TcpStream::connect(addr).await, Network, Init);
        let name = res!(ServerName::try_from(host.to_string()), Invalid);
        let mut stream = res!(TlsConnector::from(tls.1.clone()).connect(name, tcp).await, Network);
        let request = fmt!("{} {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
            X-Forwarded-For: 9.9.9.9\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            method, path, host, body.len(), body);
        res!(stream.write_all(request.as_bytes()).await, Network, Write);
        res!(stream.flush().await, Network, Write);
        // Read one whole response, framed by its Content-Length, and no further. Steel keeps a
        // connection open for the next request even when this one said `Connection: close`, so
        // reading to the end would wait out its header-read timeout (fifteen seconds) on every
        // answer it gives itself rather than relays.
        let mut reply = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(i) = reply.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&reply[..i]).to_lowercase();
                let want = head.lines()
                    .filter_map(|l| l.strip_prefix("content-length:"))
                    .filter_map(|v| v.trim().parse::<usize>().ok())
                    .next()
                    .unwrap_or(0);
                if reply.len() >= i + 4 + want {
                    break;
                }
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
        }
        drop(stream);
        let _ = server.await;
        let split = match reply.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return Err(err!("No complete response: {:?}.",
                String::from_utf8_lossy(&reply); Test)),
        };
        Ok((
            String::from_utf8_lossy(&reply[..split]).to_lowercase(),
            String::from_utf8_lossy(&reply[split + 4..]).to_string(),
        ))
    };
    let hits = || seen.lock().map(|g| g.len()).unwrap_or(usize::MAX);

    // The exact path reaches the upstream, body intact, status and body coming back untouched.
    let (head, body) = res!(fetch(HOST, "POST", "/subscribe", "{\"email\": \"a@b.example\"}").await);
    assert!(head.starts_with("http/1.0 200") || head.starts_with("http/1.1 200"), "{}", head);
    assert!(body.contains("Subscribed. Thank you."), "{}", body);
    assert_eq!(hits(), 1);
    let first = match seen.lock() {
        Ok(g) => g[0].clone(),
        Err(_) => return Err(err!("The capture lock is poisoned."; Test)),
    };
    assert_eq!(first.line, "POST /subscribe HTTP/1.1");
    assert_eq!(first.body, "{\"email\": \"a@b.example\"}");
    // One forwarded address, this hop's, and the forged one the caller sent is gone.
    assert_eq!(first.xff, vec![fmt!("127.0.0.1")], "{:?}", first);

    // An error status and its body are passed through, not replaced.
    let (head, body) = res!(fetch(HOST, "POST", "/subscribe", "bad").await);
    assert!(head.contains(" 400"), "{}", head);
    assert!(body.contains("Invalid request."), "{}", body);
    assert_eq!(hits(), 2);

    // The neighbours of the path never reach the upstream.
    for path in ["/subscribe.py", "/subscribers.txt", "/subscribe/", "/subscribe/x", "/subscribe?x=1x"] {
        let (head, _) = res!(fetch(HOST, "GET", path, "").await);
        let proxied = path == "/subscribe?x=1x";
        assert!(proxied || head.contains(" 404"), "{} should be a 404, got: {}", path, head);
        assert_eq!(hits(), if proxied { 3 } else { 2 }, "{} reached the upstream", path);
    }

    // An upstream that is down is a 502.
    let (head, _) = res!(fetch(HOST, "POST", "/down", "{}").await);
    assert!(head.contains(" 502"), "{}", head);

    // A redirect carries the query string as well as the path, byte for byte, and no more.
    let (head, _) = res!(fetch(WWW, "GET", "/credits.html?ref=x&y=1", "").await);
    assert!(head.starts_with("http/1.1 301"), "{}", head);
    assert!(head.contains("location: https://subscribe.test/credits.html?ref=x&y=1\r\n")
        || head.ends_with("location: https://subscribe.test/credits.html?ref=x&y=1"), "{}", head);
    let (head, _) = res!(fetch(WWW, "GET", "/", "").await);
    assert!(head.contains("location: https://subscribe.test/\r\n")
        || head.ends_with("location: https://subscribe.test/"), "{}", head);
    Ok(())
}
