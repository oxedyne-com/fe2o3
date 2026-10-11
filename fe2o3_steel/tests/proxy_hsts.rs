//! A proxied response over HTTPS carries the front's HSTS exactly once, as Steel's own responses do.
//!
//! TLS ends at the front, so the front is where `Strict-Transport-Security` belongs; the upstream
//! behind it speaks plain loopback HTTP and rightly sends none. The front fills the field only where
//! the upstream left it empty: an upstream's own policy is relayed as sent and never joined by a
//! second. (This release has no plain loopback listener, so every visitor came over TLS.)
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        api::ApiHandlerRegistry,
        cfg::{
            ProxyRoute,
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
        RwLock,
    },
};

use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
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

const HOST:     &str = "proxied.test";
const OWN:      &str = "max-age=60";                            // the upstream's own policy
const FRONT:    &str = "max-age=31536000; includeSubDomains";   // the front's
const MAX_AGE:  u32  = 31_536_000;

fn no_dbs<T>(_: fn(&Path, &[u8]) -> Outcome<T>)
    -> Arc<RwLock<HashMap<String, (Arc<RwLock<T>>, id::Uid)>>>
{
    Arc::new(RwLock::new(HashMap::new()))
}

/// A one-shot-per-connection upstream that sends its own HSTS only when the path holds `own`.
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
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tcp.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let line = head.lines().next().unwrap_or("").to_string();
                let own = match line.contains("own") {
                    true    => fmt!("Strict-Transport-Security: {}\r\n", OWN),
                    false   => String::new(),
                };
                let out = fmt!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n{}\
                    Content-Length: 2\r\nConnection: close\r\n\r\nok", own);
                let _ = tcp.write_all(out.as_bytes()).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    Ok(port)
}

/// Sends one GET on `stream` and reads one whole response, framed by its Content-Length: the head,
/// lower cased.
async fn ask<S>(stream: &mut S, path: &str) -> Outcome<String>
    where S: AsyncRead + AsyncWrite + Unpin
{
    let request = fmt!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", path, HOST);
    res!(stream.write_all(request.as_bytes()).await, Network, Write);
    res!(stream.flush().await, Network, Write);
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
                return Ok(head);
            }
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
        }
    }
    Err(err!("GET {} answered with no complete response: {:?}.",
        path, String::from_utf8_lossy(&reply); Test))
}

/// Every `strict-transport-security` value in a lower-cased head.
fn hsts(head: &str) -> Vec<String> {
    head.lines()
        .skip(1)
        .filter_map(|l| l.strip_prefix("strict-transport-security:"))
        .map(|v| v.trim().to_string())
        .collect()
}

#[test]
fn a_proxied_response_carries_the_fronts_hsts_once() -> Outcome<()> {
    let runtime = res!(tokio::runtime::Runtime::new(), Init);
    runtime.block_on(async { drive().await })
}

async fn drive() -> Outcome<()> {
    oxedyne_fe2o3_net::tls::ensure_crypto_provider();
    let cert = res!(rcgen::generate_simple_self_signed(vec![HOST.to_string()]), Init);
    let der = res!(cert.serialize_der(), Init);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    let server_tls = Arc::new(res!(TlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(der.clone())], key), Init));
    let mut roots = RootCertStore::empty();
    res!(roots.add(CertificateDer::from(der)), Init);
    let client_tls = Arc::new(ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth());

    let up_port = res!(upstream().await);
    let mut cfg = ServerConfig::default();
    cfg.hsts_max_age_secs = MAX_AGE;
    let web = AppWebHandler::<HashMap<String, OsPath>>::new(
        cfg.clone(),
        std::env::temp_dir().join(fmt!("steel_proxy_hsts_{}", std::process::id())),
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
    );
    let runtime = Arc::new(VhostRuntime {
        hostnames:      vec![HOST.to_string()],
        web_handler:    web,
        ws_handler:     AppWebSocketHandler::new(None),
        ws_syntax:      res!(WebSocketSyntax::new("steel_ws", &SemVer::new(0, 1, 0), "Proxy HSTS")),
        redirects:      Vec::new(),
        proxy_routes:   vec![ProxyRoute {
            path_prefix:    fmt!("/"),
            upstream_host:  fmt!("127.0.0.1"),
            upstream_port:  up_port,
            upstream_tls:   false,
            strip_prefix:   false,
        }],
        ws_routes:      Vec::new(),
        term_manager:   None,
        uses_sessions:  false,
        permissions_policy: None,
        tiles:          None,
        access_log:     false,
    });
    let mut vhosts = HashMap::new();
    vhosts.insert(HOST.to_string(), runtime);
    let protocol = Protocol::Web {
        vhosts:         Arc::new(vhosts),
        default_vhost:  HOST.to_string(),
        dev_mode:       true,
    };
    let root = std::env::temp_dir().normalise().absolute();
    let context = ServerContext::new(cfg, root, no_dbs(context::new_db), Vec::new(), protocol,
        None, None);

    // One request on a fresh TLS connection.
    let fetch = async |path: &str| -> Outcome<String> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let addr = res!(listener.local_addr(), Network, Init);
        let ctx = context.clone();
        let acceptor = TlsAcceptor::from(server_tls.clone());
        let server = tokio::spawn(async move {
            let (tcp, peer) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => return Err(err!(e, "Accept failed."; Network)),
            };
            let stream = match acceptor.accept(tcp).await {
                Ok(s) => s,
                Err(e) => return Err(err!(e, "TLS accept failed."; Network)),
            };
            ctx.handle_https(stream, Some(HOST.to_string()), peer).await
        });
        let tcp = res!(TcpStream::connect(addr).await, Network, Init);
        let name = res!(ServerName::try_from(HOST.to_string()), Invalid);
        let mut stream = res!(TlsConnector::from(client_tls.clone()).connect(name, tcp).await,
            Network);
        let head = res!(ask(&mut stream, path).await);
        let _ = server.await;
        Ok(head)
    };

    // Over HTTPS, an upstream that sends none is given the front's, once.
    let head = res!(fetch("/page").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert_eq!(hsts(&head), vec![FRONT.to_lowercase()], "{}", head);

    // An upstream's own policy is relayed as sent, never joined by a second.
    let head = res!(fetch("/own").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert_eq!(hsts(&head), vec![OWN.to_string()], "{}", head);
    Ok(())
}
