//! A TLS front for `handle_https` holding one or more vhosts, for tests that ask what a vhost
//! answers on the wire rather than what one of its functions returns.
//!
//! One request is one fresh TLS connection into the real dispatcher, so what comes back has passed
//! through everything a visitor's request does: the SNI lookup, the redirect and proxy tables, the
//! handler and the response headers added last.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![allow(dead_code)]

use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        admin::{
            guard,
            host_sampler::HostSampler,
            state::AdminState,
            traffic::TrafficRecorder,
        },
        api::ApiHandlerRegistry,
        cfg::{
            ApiRoute,
            ServerConfig,
            WebhookRoute,
        },
        context::{
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
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    keystore::Wallet,
};
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::version::SemVer;
use oxedyne_fe2o3_o3db_sync::O3db;

use std::{
    collections::HashMap,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        atomic::{
            AtomicU64,
            Ordering,
        },
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

type Db = O3db<
    { id::UID_LEN },
    id::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

type Handler = AppWebHandler<HashMap<String, OsPath>>;

type Ctx = ServerContext<
    { id::UID_LEN },
    id::Uid,
    EncryptionScheme,
    HashScheme,
    Db,
    Handler,
    AppWebSocketHandler,
>;

static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// A directory that removes itself.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Outcome<Self> {
        let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(fmt!("steel-{}-{}-{}", label, std::process::id(), seq));
        let _ = std::fs::remove_dir_all(&dir);
        res!(std::fs::create_dir_all(&dir), IO, File);
        Ok(Self(dir))
    }

    /// Writes a file under the directory, making the directories above it.
    pub fn put(&self, rel: &str, text: &str) -> Outcome<()> {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            res!(std::fs::create_dir_all(parent), IO, File);
        }
        res!(std::fs::write(&path, text), IO, File);
        Ok(())
    }

    pub fn dir(&self, rel: &str) -> Outcome<()> {
        res!(std::fs::create_dir_all(self.0.join(rel)), IO, File);
        Ok(())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What one vhost of the rig is made of.
pub struct Site {
    pub host:               String,
    pub public_dir:         PathBuf,
    pub api_routes:         Vec<ApiRoute>,
    pub webhook_routes:     Vec<WebhookRoute>,
    pub admin_dashboard:    bool,
}

impl Site {
    pub fn new(host: &str, public_dir: &Path) -> Self {
        Self {
            host:               host.to_string(),
            public_dir:         public_dir.to_path_buf(),
            api_routes:         Vec::new(),
            webhook_routes:     Vec::new(),
            admin_dashboard:    true,
        }
    }
}

/// One whole response: the head with its names lower-cased, and the body.
#[derive(Clone, Debug)]
pub struct Reply {
    pub head: String,
    pub body: String,
}

impl Reply {
    pub fn status(&self) -> u16 {
        self.head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
    }

    /// The first field of this name, if the response carries one.
    pub fn field(&self, name: &str) -> Option<String> {
        let want = fmt!("{}:", name.to_lowercase());
        self.head.lines()
            .skip(1)
            .find_map(|l| l.to_lowercase().starts_with(&want).then(|| {
                l[want.len()..].trim().to_string()
            }))
    }
}

pub struct Rig {
    context:    Ctx,
    server_tls: Arc<TlsServerConfig>,
    client_tls: Arc<ClientConfig>,
}

impl Rig {
    pub fn new(sites: Vec<Site>) -> Outcome<Self> {
        Self::with_config(sites, ServerConfig::default())
    }

    /// As `new`, with a server config of the caller's choosing (small limits, short deadlines).
    pub fn with_config(sites: Vec<Site>, cfg: ServerConfig) -> Outcome<Self> {
        oxedyne_fe2o3_net::tls::ensure_crypto_provider();
        let names: Vec<String> = sites.iter().map(|s| s.host.clone()).collect();
        let cert = res!(rcgen::generate_simple_self_signed(names), Init);
        let der = res!(cert.serialize_der(), Init);
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
        let server_tls = res!(TlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(der.clone())], key), Init);
        let mut roots = RootCertStore::empty();
        res!(roots.add(CertificateDer::from(der)), Init);
        let client_tls = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();

        // A real dashboard state, so that a vhost which serves `/admin` has something to serve:
        // without one every vhost answers 404 there, whatever it is set to, and a test of the
        // setting could not tell one answer from the other.
        let admin = res!(AdminState::new(
            Arc::new(RwLock::new(Wallet::default())),
            PathBuf::from("./wallet.jdat"),
            Some([0u8; 32].to_vec()),
            0,
            None,
            TrafficRecorder::new_shared(0),
            HostSampler::new_shared(),
            res!(guard::new_shared()),
            res!(guard::new_shared()),
            Vec::new(),
            None,
        ));
        let admin = Arc::new(admin);

        let mut vhosts = HashMap::new();
        let mut first = String::new();
        for site in sites {
            if first.is_empty() {
                first = site.host.clone();
            }
            let web: Handler = AppWebHandler::new(
                cfg.clone(),
                site.public_dir.clone(),
                HashMap::new(),
                vec![fmt!("index.html")],
                false,
                site.api_routes.clone(),
                site.webhook_routes.clone(),
                Arc::new(WebhookRegistry::new()),
                Arc::new(ApiHandlerRegistry::new()),
                None,
                Some(admin.clone()),
                None,
                None,
                None,
                Arc::new(Vec::new()),
            ).with_admin_dashboard(site.admin_dashboard);
            let runtime = Arc::new(VhostRuntime {
                hostnames:      vec![site.host.clone()],
                web_handler:    web,
                ws_handler:     AppWebSocketHandler::new(None),
                ws_syntax:      res!(WebSocketSyntax::new(
                    "steel_ws", &SemVer::new(0, 1, 0), "Vhost rig")),
                redirects:      Vec::new(),
                proxy_routes:   Vec::new(),
                ws_routes:      Vec::new(),
                term_manager:   None,
                uses_sessions:  false,
                permissions_policy: None,
                tiles:          None,
                access_log:     false,
            });
            vhosts.insert(site.host.to_lowercase(), runtime);
        }
        let protocol = Protocol::Web {
            vhosts:         Arc::new(vhosts),
            default_vhost:  first,
            dev_mode:       false,
        };
        let root = std::env::temp_dir().normalise().absolute();
        let dbs: Arc<RwLock<HashMap<String, (Arc<RwLock<Db>>, id::Uid)>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let context = ServerContext::new(cfg, root, dbs, Vec::new(), protocol, None,
            Some(admin));
        Ok(Self {
            context,
            server_tls: Arc::new(server_tls),
            client_tls: Arc::new(client_tls),
        })
    }

    /// One request over a fresh TLS connection to the named vhost. `fields` are extra request
    /// header lines, `Name: value`.
    pub async fn fetch(
        &self,
        host:       &str,
        method:     &str,
        path:       &str,
        fields:     &[&str],
        body:       &str,
    )
        -> Outcome<Reply>
    {
        self.fetch_as(host, "HTTP/1.1", method, path, fields, body).await
    }

    /// As [`Rig::fetch`], naming the HTTP version the request line carries.
    pub async fn fetch_as(
        &self,
        host:       &str,
        version:    &str,
        method:     &str,
        path:       &str,
        fields:     &[&str],
        body:       &str,
    )
        -> Outcome<Reply>
    {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let addr = res!(listener.local_addr(), Network, Init);
        let ctx = self.context.clone();
        let acceptor = TlsAcceptor::from(self.server_tls.clone());
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
        let mut stream = res!(TlsConnector::from(self.client_tls.clone()).connect(name, tcp).await,
            Network);
        let mut request = fmt!("{} {} {}\r\nHost: {}\r\n", method, path, version, host);
        for f in fields {
            request.push_str(f);
            request.push_str("\r\n");
        }
        request.push_str(&fmt!("Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body));
        res!(stream.write_all(request.as_bytes()).await, Network, Write);
        res!(stream.flush().await, Network, Write);
        // One whole response, framed by its Content-Length and no further: Steel holds a
        // connection open for the next request, so reading to the end would wait out its
        // header-read timeout on every answer. A response with no Content-Length ends at its head.
        let mut reply = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(i) = reply.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&reply[..i]).to_lowercase();
                // A HEAD answer states the length of a body it does not send.
                let want = match method == "HEAD" {
                    true    => 0,
                    false   => head.lines()
                        .filter_map(|l| l.strip_prefix("content-length:"))
                        .filter_map(|v| v.trim().parse::<usize>().ok())
                        .next()
                        .unwrap_or(0),
                };
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
            None => return Err(err!("{} {} on {} answered with no complete head: {:?}.",
                method, path, host, String::from_utf8_lossy(&reply); Test)),
        };
        Ok(Reply {
            head: String::from_utf8_lossy(&reply[..split]).to_string(),
            body: String::from_utf8_lossy(&reply[split + 4..]).to_string(),
        })
    }
}

/// One request, written exactly as given, with what the server did to the connection afterwards.
#[derive(Clone, Debug)]
pub struct Exchange {
    pub reply:  Reply,
    pub closed: bool,   // the server ended the connection within the patience given
}

impl Rig {

    /// A request as raw text, with nothing added (no `Connection` field, no `Content-Length`), over
    /// a fresh TLS connection to the named vhost. After one whole response is read, the connection
    /// is watched for `patience`: `closed` says whether the server ended it in that time.
    pub async fn exchange(
        &self,
        host:       &str,
        raw:        &str,
        patience:   std::time::Duration,
    )
        -> Outcome<Exchange>
    {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, Network, Init);
        let addr = res!(listener.local_addr(), Network, Init);
        let ctx = self.context.clone();
        let acceptor = TlsAcceptor::from(self.server_tls.clone());
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
        let mut stream = res!(TlsConnector::from(self.client_tls.clone()).connect(name, tcp).await,
            Network);
        res!(stream.write_all(raw.as_bytes()).await, Network, Write);
        res!(stream.flush().await, Network, Write);
        let mut reply = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut closed = false;
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
                Ok(0) | Err(_) => {
                    closed = true;
                    break;
                },
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
        }
        if !closed {
            closed = match tokio::time::timeout(patience, stream.read(&mut chunk)).await {
                Ok(Ok(0)) | Ok(Err(_)) => true,
                Ok(Ok(_))               => false,
                Err(_)                  => false,
            };
        }
        drop(stream);
        let _ = server.await;
        let split = match reply.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i,
            None => return Err(err!("{:?} on {} answered with no complete head: {:?}.",
                raw, host, String::from_utf8_lossy(&reply); Test)),
        };
        Ok(Exchange {
            reply: Reply {
                head: String::from_utf8_lossy(&reply[..split]).to_string(),
                body: String::from_utf8_lossy(&reply[split + 4..]).to_string(),
            },
            closed,
        })
    }
}

/// The runtime these tests drive `async` code with from a plain `#[test]`.
pub fn runtime() -> Outcome<tokio::runtime::Runtime> {
    Ok(res!(tokio::runtime::Runtime::new(), Init))
}
