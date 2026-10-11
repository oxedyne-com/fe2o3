//! `ApiHandler::handle_ctx` and the loopback plain-HTTP listener, driven through `handle_https`
//! against a real socket.
//!
//! A probe handler owns the prefix route `/edit/*` and answers with what its `HandlerCtx` held:
//! the method, the peer, and whether the vhost's database was there. It declines a path ending
//! `.html`, which must then come from `www/`. The database map is the one Steel fills on unseal,
//! shared with the handler's `VhostDbRef`, so emptying and filling it here is what sealing and
//! unsealing do; the database put in it is opened but never started, since the handler only looks
//! for it. Every request goes through `serve_loopback`, which is also the test of that listener:
//! two vhosts picked by the Host header, no HSTS though the config asks for it, and a bind to an
//! address that is not loopback refused.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_steel::{
    app::{
        https::{
            AppWebHandler,
            VhostDbRef,
        },
        server::VhostDb,
    },
    srv::{
        api::{
            ApiHandler,
            ApiHandlerRegistry,
            HandlerCtx,
        },
        cfg::{
            ApiRoute,
            ServerConfig,
        },
        context::{
            self,
            Protocol,
            ServerContext,
            VhostDbs,
            VhostRuntime,
        },
        id,
        loopback,
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
use oxedyne_fe2o3_net::http::{
    fields::HeaderFields,
    header::HttpMethod,
    loc::HttpLocator,
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::AtomicUsize,
        Arc,
        RwLock,
    },
};

use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::TcpStream,
};
use tokio_rustls::rustls::ClientConfig;

const HOST:     &str = "edit.test";
const OTHER:    &str = "other.test";

/// Answers with what its context held, and declines a `.html` path.
struct Probe;

impl ApiHandler for Probe {
    fn handle<'a>(
        &'a self,
        _route:         &'a ApiRoute,
        _method:        HttpMethod,
        _loc:           &'a HttpLocator,
        _body:          &'a [u8],
        _req_headers:   &'a HeaderFields,
        _tls_client:    &'a Option<Arc<ClientConfig>>,
        _id:            &'a str,
    ) -> Pin<Box<dyn Future<Output = Outcome<HttpMessage>> + Send + 'a>> {
        Box::pin(async move {
            Ok(HttpMessage::respond_with_text(HttpStatus::InternalServerError, "handle, not handle_ctx"))
        })
    }

    fn handle_ctx<'a>(
        &'a self,
        ctx: HandlerCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Outcome<Option<HttpMessage>>> + Send + 'a>> {
        Box::pin(async move {
            let path = ctx.loc.path.as_string().to_string();
            if path.ends_with(".html") {
                return Ok(None);
            }
            Ok(Some(HttpMessage::respond_with_text(HttpStatus::OK, &fmt!(
                "probe {} {} db={} peer={} body={}",
                ctx.method, path, ctx.db.is_some(), ctx.peer.ip(),
                String::from_utf8_lossy(ctx.body),
            ))))
        })
    }
}

fn www(dir: &PathBuf, files: &[(&str, &str)]) -> Outcome<()> {
    for (rel, text) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            res!(std::fs::create_dir_all(parent), IO, File);
        }
        res!(std::fs::write(&p, text), IO, File);
    }
    Ok(())
}

#[test]
fn handle_ctx_and_the_loopback_listener() -> Outcome<()> {
    let runtime = res!(tokio::runtime::Runtime::new(), Init);
    runtime.block_on(async { drive().await })
}

async fn drive() -> Outcome<()> {
    let base = std::env::temp_dir().join(fmt!("steel_handler_ctx_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let www_edit = base.join("www_edit");
    let www_other = base.join("www_other");
    res!(www(&www_edit, &[("index.html", "edit index"), ("edit/page.html", "static page")]));
    res!(www(&www_other, &[("index.html", "other index")]));

    // The map Steel fills on unseal, shared by the context and the handler.
    let dbs: VhostDbs<{ id::UID_LEN }, id::Uid, VhostDb> = Arc::new(RwLock::new(HashMap::new()));

    let mut cfg = ServerConfig::default();
    cfg.hsts_max_age_secs = 31_536_000;
    let mut registry = ApiHandlerRegistry::new();
    registry.register("probe", Probe);
    let registry = Arc::new(registry);
    let route = ApiRoute {
        path:           fmt!("/edit/*"),
        upstream_host:  None,
        upstream_port:  None,
        upstream_path:  None,
        upstream_tls:   false,
        headers:        Vec::new(),
        handler:        Some(fmt!("probe")),
        config:         Vec::new(),
    };
    let web = |dir: &PathBuf, routes: Vec<ApiRoute>, name: &str|
        -> AppWebHandler<HashMap<String, OsPath>>
    {
        AppWebHandler::new(
            cfg.clone(),
            dir.clone(),
            HashMap::new(),
            vec![fmt!("index.html")],
            true,
            routes,
            Vec::new(),
            Arc::new(WebhookRegistry::new()),
            registry.clone(),
            None,
            None,
            None,
            None,
            None,
            Arc::new(Vec::new()),
        ).with_vhost_db(VhostDbRef::new(dbs.clone(), name))
    };
    let vhost = |name: &str, web_handler| -> Outcome<Arc<VhostRuntime<_, _>>> {
        Ok(Arc::new(VhostRuntime {
            hostnames:      vec![name.to_string()],
            web_handler,
            ws_handler:     AppWebSocketHandler::new(None),
            ws_syntax:      res!(WebSocketSyntax::new("steel_ws", &SemVer::new(0, 1, 0), "Ctx test")),
            redirects:      Vec::new(),
            proxy_routes:   Vec::new(),
            ws_routes:      Vec::new(),
            term_manager:   None,
            uses_sessions:  false,
            permissions_policy: None,
            tiles:          None,
            access_log:     false,
        }))
    };
    let mut vhosts = HashMap::new();
    vhosts.insert(HOST.to_string(), res!(vhost(HOST, web(&www_edit, vec![route], HOST))));
    vhosts.insert(OTHER.to_string(), res!(vhost(OTHER, web(&www_other, Vec::new(), OTHER))));
    let protocol = Protocol::Web {
        vhosts:         Arc::new(vhosts),
        default_vhost:  HOST.to_string(),
        dev_mode:       true,
    };
    let root = std::env::temp_dir().normalise().absolute();
    let context = ServerContext::new(cfg.clone(), root, dbs.clone(), Vec::new(), protocol,
        None, None);

    // A bind anywhere but loopback is refused, and a loopback one is served.
    for addr in ["0.0.0.0:0", "192.0.2.1:0", "[::]:0"] {
        let addr: SocketAddr = res!(addr.parse(), Invalid);
        assert!(loopback::bind_loopback(addr).await.is_err(), "{} was bound", addr);
    }
    let listener = res!(loopback::bind_loopback(res!("127.0.0.1:0".parse(), Invalid)).await);
    let addr = res!(listener.local_addr(), Network, Init);
    let inflight = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(context.clone().serve_loopback(listener, inflight));

    // One request on a fresh connection: the head, lower cased, and the body.
    let fetch = async |host: &str, method: &str, path: &str, body: &str|
        -> Outcome<(String, String)>
    {
        let mut tcp = res!(TcpStream::connect(addr).await, Network, Init);
        let request = fmt!("{} {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\
            Connection: close\r\n\r\n{}", method, path, host, body.len(), body);
        res!(tcp.write_all(request.as_bytes()).await, Network, Write);
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
            match tcp.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
        }
        let split = match reply.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return Err(err!("No complete response to {} {}: {:?}.", method, path,
                String::from_utf8_lossy(&reply); Test)),
        };
        Ok((
            String::from_utf8_lossy(&reply[..split]).to_lowercase(),
            String::from_utf8_lossy(&reply[split + 4..]).to_string(),
        ))
    };

    // Sealed: the handler runs with the peer set and no database.
    let (head, body) = res!(fetch(HOST, "GET", "/edit/doc", "").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert_eq!(body, "probe GET /edit/doc db=false peer=127.0.0.1 body=");
    // Over plain HTTP no HSTS is sent, though the config asks for it.
    assert!(!head.contains("strict-transport-security"), "{}", head);

    // A handler that declines a GET leaves the path to www/.
    let (head, body) = res!(fetch(HOST, "GET", "/edit/page.html", "").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert_eq!(body, "static page");

    // Unsealed: the database the map now holds reaches the handler.
    let db_dir = base.join("db");
    res!(std::fs::create_dir_all(&db_dir), IO, File);
    let db = res!(context::new_db(&db_dir, &[7u8; 32]));
    {
        let mut g = res!(dbs.write().map_err(|_| err!("The db map lock is poisoned."; Test)));
        g.insert(HOST.to_string(), (Arc::new(RwLock::new(db)), id::Uid::default()));
    }
    let (_, body) = res!(fetch(HOST, "GET", "/edit/doc", "").await);
    assert_eq!(body, "probe GET /edit/doc db=true peer=127.0.0.1 body=");

    // PUT and DELETE reach the handler route, the body with them.
    let (head, body) = res!(fetch(HOST, "PUT", "/edit/doc", "new text").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert_eq!(body, "probe PUT /edit/doc db=true peer=127.0.0.1 body=new text");
    let (_, body) = res!(fetch(HOST, "DELETE", "/edit/a/b", "").await);
    assert_eq!(body, "probe DELETE /edit/a/b db=true peer=127.0.0.1 body=");

    // Elsewhere, and for a method no route takes, a 405 that says what is allowed.
    for (method, path) in [("PUT", "/index.html"), ("DELETE", "/elsewhere"), ("OPTIONS", "/edit/doc")] {
        let (head, _) = res!(fetch(HOST, method, path, "").await);
        assert!(head.starts_with("http/1.1 405"), "{} {}: {}", method, path, head);
        assert!(head.contains("allow: get, head, post"), "{} {}: {}", method, path, head);
    }

    // The Host header picks the vhost on a plain connection.
    let (_, body) = res!(fetch(OTHER, "GET", "/", "").await);
    assert_eq!(body, "other index");
    let (_, body) = res!(fetch(&fmt!("{}:{}", HOST, addr.port()), "GET", "/", "").await);
    assert_eq!(body, "edit index");
    // The other vhost has no handler route, so its /edit is no handler's.
    let (head, _) = res!(fetch(OTHER, "PUT", "/edit/doc", "x").await);
    assert!(head.starts_with("http/1.1 405"), "{}", head);

    server.abort();
    {
        let mut g = res!(dbs.write().map_err(|_| err!("The db map lock is poisoned."; Test)));
        g.clear();
    }
    let _ = std::fs::remove_dir_all(&base);
    Ok(())
}
