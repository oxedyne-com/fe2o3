//! A tile served end to end: a TLS connection into `handle_https`, a real PMTiles archive
//! behind the route, and the log file read afterwards for any trace of where the viewer looked.
//!
//! The archive is `fe2o3_geom/tests/data/protomaps/sample.pmtiles`, written by the reference
//! Python writer from Protomaps tiles.  The log runs at trace level into a file of its own, so
//! anything the server would say about the request is there to be found; a sentinel line
//! logged beside the request proves the file is the one being written.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        api::ApiHandlerRegistry,
        cfg::{
            ServerConfig,
            TileConfig,
        },
        context::{
            self,
            Protocol,
            ServerContext,
            VhostRuntime,
        },
        id,
        tiles::{
            TileService,
            TileSource,
        },
        webhook::WebhookRegistry,
        ws::{
            handler::AppWebSocketHandler,
            syntax::WebSocketSyntax,
        },
    },
};

use oxedyne_fe2o3_core::{
    file::OsPath,
    log::bot::FileConfig,
    path::NormalPath,
    prelude::*,
};
use oxedyne_fe2o3_jdat::version::SemVer;
use oxedyne_fe2o3_net::http::encoding;

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
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

const HOST: &str = "tiles.test";
const SENTINEL: &str = "tile-test-sentinel-7f3a";

fn data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fe2o3_geom/tests/data/protomaps").join(rel)
}

/// An empty database map of the type the server holds, taken from the constructor's own
/// signature, since the tile route never reaches a database.
fn no_dbs<T>(_: fn(&Path, &[u8]) -> Outcome<T>)
    -> Arc<RwLock<HashMap<String, (Arc<RwLock<T>>, id::Uid)>>>
{
    Arc::new(RwLock::new(HashMap::new()))
}

#[test]
fn a_tile_is_served_through_https_and_no_log_line_names_it() -> Outcome<()> {
    // The log, at trace level, into a file of its own.
    let dir = std::env::temp_dir().join(fmt!("steel_tiles_https_{}", std::process::id()));
    res!(std::fs::create_dir_all(&dir), File, Write);
    let mut log_cfg = log_get_config!();
    log_cfg.level = res!(LogLevel::from_str("trace"));
    let file_cfg = FileConfig::new(dir.clone(), "tiles".to_string(), "log".to_string(), 0, None);
    let log_path = file_cfg.path();
    log_cfg.file = Some(file_cfg);
    log_set_config!(log_cfg);

    let runtime = res!(tokio::runtime::Runtime::new(), Init);
    let outcome = runtime.block_on(async { serve_and_ask().await });
    log_finish_wait!();
    let body = res!(outcome);

    let log = res!(std::fs::read_to_string(&log_path), File, Read);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(log.contains(SENTINEL), "The log file was not the one written: {:?}", log_path);
    for needle in ["13/6729/4865", "/t/sample", "6729", "12/2957/2545"] {
        assert!(!log.contains(needle), "The log names a tile ({}):\n{}", needle, log);
    }
    // And the tile itself was the archive's.
    let want = res!(std::fs::read(data("13_6729_4865.mvt")), File, Read);
    assert_eq!(body, want);
    Ok(())
}

async fn serve_and_ask() -> Outcome<Vec<u8>> {
    oxedyne_fe2o3_net::tls::ensure_crypto_provider();
    // A self-signed certificate for the vhost, trusted by the client alone.
    let cert = res!(rcgen::generate_simple_self_signed(vec![HOST.to_string()]), Init);
    let der = res!(cert.serialize_der(), Init);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    let server_tls = res!(TlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(der.clone())], key), Init);
    let mut roots = RootCertStore::empty();
    res!(roots.add(CertificateDer::from(der)), Init);
    let client_tls = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let tls = (Arc::new(server_tls), Arc::new(client_tls));

    // A vhost carrying only the tile route, over the real sample archive, access log off.
    let mut builds = BTreeMap::new();
    builds.insert("sample".to_string(), data("sample.pmtiles"));
    let tile_cfg = TileConfig {
        prefix:         "/t".to_string(),
        current:        "sample".to_string(),
        builds,
        allow_origins:  vec!["https://oxegen.io".to_string()],
        attribution:    "© OpenStreetMap".to_string(),
    };
    let tiles = res!(TileService::<TileSource>::new(&tile_cfg, HOST, 0, TileSource::open));
    let cfg = ServerConfig::default();
    let web_handler: AppWebHandler<HashMap<String, OsPath>> = AppWebHandler::new(
        cfg.clone(),
        PathBuf::new(),
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
        web_handler,
        ws_handler:     AppWebSocketHandler::new(None),
        ws_syntax:      res!(WebSocketSyntax::new("steel_ws", &SemVer::new(0, 1, 0), "Tiles test")),
        redirects:      Vec::new(),
        proxy_routes:   Vec::new(),
        ws_routes:      Vec::new(),
        term_manager:   None,
        uses_sessions:  false,
        permissions_policy: None,
        tiles:          Some(Arc::new(tiles)),
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

    // One request over a fresh TLS connection, answered by `handle_https`: the head, lower
    // cased, and the body.
    let fetch = async |path: &str| -> Outcome<(String, Vec<u8>)> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let addr = res!(listener.local_addr(), Network, Init);
        let ctx = context.clone();
        let acceptor = TlsAcceptor::from(tls.0.clone());
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
        let mut stream = res!(TlsConnector::from(tls.1.clone()).connect(name, tcp).await, Network);
        let request = fmt!("GET {} HTTP/1.1\r\nHost: {}\r\nAccept-Encoding: gzip\r\n\
            Cookie: session=abc123\r\nConnection: close\r\n\r\n", path, HOST);
        res!(stream.write_all(request.as_bytes()).await, Network, Write);
        res!(stream.flush().await, Network, Write);
        let mut reply = Vec::new();
        // The server closes after a `Connection: close` request; a close without
        // close_notify still leaves the bytes read.
        let _ = stream.read_to_end(&mut reply).await;
        let _ = server.await;
        let split = match reply.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return Err(err!("No complete response: {:?}.",
                String::from_utf8_lossy(&reply); Test)),
        };
        Ok((String::from_utf8_lossy(&reply[..split]).to_lowercase(), reply[split + 4..].to_vec()))
    };

    info!("{}: asking for a tile", SENTINEL);
    let (head, body) = res!(fetch("/t/sample/13/6729/4865.mvt").await);
    assert!(head.starts_with("http/1.1 200"), "{}", head);
    assert!(head.contains("content-encoding: gzip"), "{}", head);
    assert!(!head.contains("set-cookie"), "{}", head);
    let plain = res!(encoding::gunzip(&body));
    // A tile in the run of three the writer made from one stored content: the open ocean the
    // reference reader decompressed to 75 bytes.
    let (head2, body2) = res!(fetch("/t/sample/12/2957/2545.mvt").await);
    assert!(head2.starts_with("http/1.1 200"), "{}", head2);
    assert_eq!(res!(encoding::gunzip(&body2)).len(), 75);
    // A tile the archive does not hold is an empty answer, not an error.
    let (head3, _) = res!(fetch("/t/sample/13/0/0.mvt").await);
    assert!(head3.starts_with("http/1.1 204"), "{}", head3);
    info!("{}: done", SENTINEL);
    Ok(plain)
}
