use crate::{
    app::{
        self,
        constant as app_const,
        https::AppWebHandler,
        repl::AppShellContext,
    },
    srv::{
        admin::{
            state::AdminState,
            traffic::TrafficRecorder,
        },
        cert::Certificate,
        cfg::ServerConfig,
        constant as srv_const,
        context::{
            new_db,
            Protocol,
            ServerContext,
            VhostRuntime,
        },
        dev::{
            cfg::DevConfig,
            refresh::DevRefreshManager,
        },
        id,
        server::Server,
        ws::{
            handler::AppWebSocketHandler,
            syntax::WebSocketSyntax,
        },
    },
};

use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_o3db_sync::O3db;

use oxedyne_fe2o3_core::{
    prelude::*,
    log::{
        bot::FileConfig,
        console::{
            LoggerConsole,
            StdoutLoggerConsole,
        },
    },
    path::NormalPath,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
};
use oxedyne_fe2o3_syntax::{
    msg::{
        MsgCmd,
    },
};
use oxedyne_fe2o3_tui::lib_tui::{
    repl::{
        Evaluation,
        ShellConfig,
    },
};

use std::{
    collections::HashMap,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        RwLock,
    },
    time::Duration,
};

use tokio;


impl AppShellContext {

    pub fn start_server(
        &mut self,
        _shell_cfg:  &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let root_path = Path::new(&self.app_cfg.app_root)
            .normalise()
            .absolute();

        info!("Reading server config...");
        let server_cfg = res!(ServerConfig::from_datmap(self.app_cfg.server_cfg.clone()));

        info!("Reading dev config...");
        let dev_cfg = res!(DevConfig::from_datmap(self.app_cfg.dev_cfg.clone()));

        // ┌───────────────────────┐
        // │ Determine mode.       │
        // └───────────────────────┘
        let mut dev_mode = false;
        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_arg("dev") {
                dev_mode = true;
                info!("Running in development mode.");
            }
        }

        // Ensure compatibility with existing websites.
        res!(app::dev::ensure_compatibility(&root_path));

        info!("Validating server config...");
        match server_cfg.validate(&root_path) {
            Ok(()) => info!("Server configuration validated successfully."),
            Err(e) => {
                warn!("Server configuration validation issues: {}", e);
                info!("Continuing with available routes...");
            }
        }

        if dev_mode {
            info!("Validating development config...");
            match dev_cfg.validate(&root_path) {
                Ok(()) => info!("Development configuration validated successfully."),
                Err(e) => {
                    warn!("Development configuration issues: {}", e);
                    info!("Some development features may be disabled.");
                }
            }
        }

        if self.stat.first && !dev_mode {
            return Ok(Evaluation::Error(fmt!(
                "You should update values in {} before running the server in production mode.",
                app_const::CONFIG_NAME,
            )));
        }

        // ┌───────────────────────┐
        // │ Reconfigure logging.  │
        // └───────────────────────┘
        
        let mut log_cfg = log_get_config!();
        let mut logger_console = StdoutLoggerConsole::new();
        let logger_console_thread = logger_console.go();
        log_cfg.console = Some(logger_console_thread.chan.clone());
        (log_cfg.level, _) = res!(self.app_cfg.server_log_level());
        log_cfg.file = Some(FileConfig::new(
            PathBuf::from(&root_path).join("www").join("logs"),
            self.app_cfg.app_name.clone(),
            "log".to_string(),
            0,
            Some(1_048_576), // Activate multiple log file archiving using this max size.
        ));
        debug!("log_cfg = {:?}", log_cfg);
        log_set_config!(log_cfg);
        println!("Server now logging at {:?}", log_get_file_path!());
        info!("┌───────────────────────┐");
        info!("│ New server session.   │");
        info!("└───────────────────────┘");

        // ┌───────────────────────┐
        // │ Parse vhosts + ACME.  │
        // └───────────────────────┘

        let vhosts_cfg = res!(server_cfg.get_vhosts());
        let acme_cfg = res!(server_cfg.get_acme());

        // ┌───────────────────────┐
        // │ Start per-vhost dbs.  │
        // └───────────────────────┘

        // One Ozone instance per vhost that has a `db_dir_rel` configured.
        // Pure-redirect vhosts (no webroot, no database) simply skip this
        // step. The resulting map is keyed by each vhost's canonical
        // (primary) hostname in lowercase, matching what `db_for_vhost`
        // looks up on request dispatch.
        let uid = id::Uid::new(0);
        let mut vhost_dbs: HashMap<String, (Arc<RwLock<O3db<
            { id::UID_LEN },
            id::Uid,
            EncryptionScheme,
            HashScheme,
            HashScheme,
            ChecksumScheme,
        >>>, id::Uid)> = HashMap::new();

        for vh in &vhosts_cfg {
            let primary_lc = vh.primary_hostname().to_lowercase();
            let db_dir = match res!(vh.get_db_dir(&root_path)) {
                Some(p) => p,
                None => {
                    info!("Vhost '{}' has no database configured, skipping.",
                        primary_lc);
                    continue;
                }
            };
            info!("Starting database for vhost '{}' at {:?}...",
                primary_lc, db_dir);
            let mut db = res!(new_db(&db_dir, &self.db_enc_key));
            // Label is the vhost's canonical name so log output disambiguates
            // multi-vhost deployments.
            let label = fmt!("db_{}", primary_lc);
            res!(db.start(&label));
            res!(ok!(db.updated_api()).activate_gc(true));

            std::thread::sleep(Duration::from_millis(200));

            let (start, msgs) = res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
            info!("Vhost '{}': {} ping replies in {:?}.",
                primary_lc, msgs.len(), start.elapsed());

            vhost_dbs.insert(primary_lc, (Arc::new(RwLock::new(db)), uid));
        }

        // ┌───────────────────────┐
        // │ Certificates.         │
        // └───────────────────────┘

        if dev_mode {
            // Dev mode uses a single shared self-signed cert.
            let tls_dir = res!(server_cfg.get_tls_dir(&root_path, true));
            let cert_path = tls_dir.join("fullchain.pem");
            let key_path = tls_dir.join("privkey.pem");
            debug!("dev tls_dir = {:?}", tls_dir);
            if !cert_path.exists() || !key_path.exists() {
                info!("Development certificates not found -- generating self-signed cert.");
                res!(Certificate::new_dev(
                    &server_cfg,
                    &root_path,
                ));
            }
        } else if acme_cfg.enabled {
            info!("ACME is enabled; certificates will be issued on start-up via {}.",
                acme_cfg.directory_url);
        } else {
            // Production without ACME: require per-vhost cert files on disk.
            let tls_dir = res!(server_cfg.get_tls_dir(&root_path, false));
            for vh in &vhosts_cfg {
                let primary = vh.primary_hostname();
                let vh_dir = tls_dir.join(primary);
                let cert_path = vh_dir.join("fullchain.pem");
                let key_path = vh_dir.join("privkey.pem");
                if !cert_path.exists() || !key_path.exists() {
                    return Ok(Evaluation::Error(fmt!(
                        "Missing certificate for vhost '{}'. ACME is disabled, so \
                        Steel expects {:?} and {:?} to already exist. Either enable \
                        ACME in the acme section of config.jdat, or install the \
                        required PEM files and restart.",
                        primary, cert_path, key_path,
                    )));
                }
            }
        }

        if dev_mode {
            info!("Connect via: https://localhost:{}", server_cfg.server_port_tcp);
        } else if let Some(first) = vhosts_cfg.first() {
            info!("Connect via: https://{}", first.primary_hostname());
        }

        // ┌───────────────────────┐
        // │ Refresh the css and   │
        // │ javascript bundles.   │
        // └───────────────────────┘
        
        let js_bundles_map = if dev_mode && dev_cfg.has_js_bundling(&root_path) {
            info!("JavaScript bundling enabled.");
            res!(dev_cfg.get_js_bundles_map(&root_path))
        } else {
            info!("JavaScript bundling disabled or not configured.");
            Vec::new()
        };

        let js_import_aliases = res!(dev_cfg.get_js_import_aliases(&root_path));

        let css_paths = if dev_mode && dev_cfg.has_css_bundling(&root_path) {
            info!("CSS compilation enabled.");
            res!(dev_cfg.get_css_paths(&root_path))
        } else {
            info!("CSS compilation disabled or not configured.");
            (PathBuf::new(), PathBuf::new())
        };
        
        let refresh_manager = Arc::new(DevRefreshManager::new(
            &root_path,
            js_bundles_map,
            js_import_aliases,
            css_paths,
        ));
        res!(refresh_manager.refresh());

        // ┌───────────────────────┐
        // │ Initialise the dev    │
        // │ refresh functionality.│
        // └───────────────────────┘
        
        let rt = res!(tokio::runtime::Runtime::new());

        let ws_handler = AppWebSocketHandler::new(
            if dev_mode {
                let manager_clone = refresh_manager.clone();
                // The file watcher is synchronous blocking code (notify's
                // event loop). On single-CPU hosts Tokio's default runtime
                // has a single worker, so a blocking task spawned with
                // `rt.spawn` would hog that worker and starve the accept
                // loop. Use `spawn_blocking` to run it on the dedicated
                // blocking thread pool instead.
                rt.spawn_blocking(move || {
                    debug!("Starting dev refresh file watcher.");
                    if let Err(e) = manager_clone.watch() {
                        error!(err!(e,
                            "Failed to start development file watcher.";
                            Init));
                    }
                });
                Some(refresh_manager)
            } else {
                None
            }
        );

        // ┌───────────────────────┐
        // │ Start server.         │
        // └───────────────────────┘

        let ws_syntax = res!(WebSocketSyntax::new(
            &self.app_cfg.app_human_name,
            &app_const::VERSION,
            &self.app_cfg.app_description,
        ));

        // Build per-vhost runtimes: one AppWebHandler per vhost, each with its
        // own public_dir / static_routes / default index files. The resulting
        // map is keyed by every alias hostname so SNI dispatch finds the right
        // runtime regardless of which alias the client asked for.
        let mut vhost_map: HashMap<String, Arc<VhostRuntime<
            AppWebHandler<HashMap<String, oxedyne_fe2o3_core::file::OsPath>>,
            _,
        >>> = HashMap::new();
        let mut default_vhost_key: Option<String> = None;

        // Build a TLS client config for outbound API proxy requests.
        // Reads the system CA bundle so Steel can talk to any upstream.
        let tls_client = match build_outbound_tls_client() {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                warn!("{}", e);
                info!("Outbound API proxy will be unavailable.");
                None
            }
        };

        // Build the admin dashboard runtime. AdminState holds a
        // shared handle to the wallet (so dashboard login uses the
        // same admin list the CLI sees) plus an AES-256-GCM cipher
        // pre-keyed with a SHA3-256 derivation of the wallet master
        // key, used for stateless signed session cookies. The
        // TrafficRecorder is shared across every vhost so the
        // dashboard can present a single host-wide traffic view.
        let admin_state = res!(AdminState::new(
            self.wallet.clone(),
            &self.db_enc_key,
        ));
        let admin_state = Arc::new(admin_state);
        let traffic = TrafficRecorder::new_shared(0);
        info!("Admin dashboard runtime initialised \
            (session key derived; traffic ring capacity {}).",
            traffic.capacity());

        for vh in &vhosts_cfg {
            let public_dir = match res!(vh.get_public_dir(&root_path)) {
                Some(p) => p,
                None => PathBuf::new(),
            };
            let static_routes = res!(vh.get_static_route_paths(
                &root_path,
                HashMap::new(),
            ));
            let default_index_files = res!(vh.get_default_index_files());

            // Resolve {file:} placeholders in API route headers.
            let mut api_routes = vh.api_routes.clone();
            for route in &mut api_routes {
                res!(route.resolve_headers(root_path.as_ref()));
            }
            if !api_routes.is_empty() {
                info!("Vhost '{}': {} API route(s) configured.",
                    vh.primary_hostname(), api_routes.len());
            }

            // Resolve {file:} placeholders in webhook route config.
            let mut webhook_routes = vh.webhook_routes.clone();
            for route in &mut webhook_routes {
                res!(route.resolve_config(root_path.as_ref()));
            }
            if !webhook_routes.is_empty() {
                info!("Vhost '{}': {} webhook route(s) configured.",
                    vh.primary_hostname(), webhook_routes.len());
            }

            let web_handler = AppWebHandler::new(
                server_cfg.clone(),
                public_dir,
                static_routes,
                default_index_files,
                dev_mode,
                api_routes,
                webhook_routes,
                self.webhook_registry.clone(),
                self.api_handler_registry.clone(),
                tls_client.clone(),
                Some(admin_state.clone()),
                Some(traffic.clone()),
            );

            let runtime = Arc::new(VhostRuntime {
                hostnames:      vh.hostnames.clone(),
                web_handler,
                ws_handler:     ws_handler.clone(),
                ws_syntax:      ws_syntax.clone(),
                redirects:      vh.redirects.clone(),
            });

            let primary_lc = vh.primary_hostname().to_lowercase();
            if default_vhost_key.is_none() {
                default_vhost_key = Some(primary_lc.clone());
            }
            for h in &vh.hostnames {
                vhost_map.insert(h.to_lowercase(), runtime.clone());
            }
        }

        let default_vhost = match default_vhost_key {
            Some(k) => k,
            None => return Ok(Evaluation::Error(fmt!(
                "No vhosts configured -- at least one vhost is required."))),
        };

        let protocol = Protocol::Web {
            vhosts:         Arc::new(vhost_map),
            default_vhost,
            dev_mode,
        };

        let server_context = ServerContext::new(
            server_cfg,
            root_path.clone(),
            vhost_dbs,
            protocol,
        );

        let server = Server::new(server_context);

        info!("Starting server...");
        for line in srv_const::SPLASH.lines() {
            info!("{}", line);
        }

        match rt.block_on(server.start()) {
            Ok(()) => info!("Server stopped gracefully."),
            Err(e) => error!(Error::Upstream(Arc::new(e), ErrMsg {
                tags: &[ErrTag::IO, ErrTag::Thread],
                msg: fmt!("Result of the attempt to execute the server within the Tokio runtime."),
            })),
        }

        log_finish_wait!();

        Ok(Evaluation::Exit)
    }

}

/// Build a `rustls::ClientConfig` for outbound HTTPS requests by
/// reading the system CA certificate bundle. Returns an error if the
/// bundle cannot be found or contains no usable certificates.
///
/// Used internally by `start_server` for the API proxy and exposed
/// so that app extensions can use the same CA store for one-shot
/// CLI subcommands that need to make outbound HTTPS calls.
pub fn build_outbound_tls_client()
    -> Outcome<Arc<tokio_rustls::rustls::ClientConfig>>
{
    use tokio_rustls::rustls::{
        ClientConfig,
        RootCertStore,
        pki_types::CertificateDer,
    };

    // Common system CA bundle paths.
    let ca_paths = [
        "/etc/ssl/certs/ca-certificates.crt",   // Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",     // Fedora/RHEL
        "/etc/ssl/cert.pem",                    // Alpine/macOS
    ];
    let ca_file = match ca_paths.iter().find(|p| Path::new(p).exists()) {
        Some(p) => *p,
        None => return Err(err!(
            "No system CA bundle found. Tried: {:?}", ca_paths;
            Init, Missing, File)),
    };

    info!("Loading system CA certificates from '{}'...", ca_file);
    let pem_data = match std::fs::read(ca_file) {
        Ok(d) => d,
        Err(e) => return Err(err!(e,
            "Failed to read CA bundle '{}'.", ca_file;
            IO, File, Read)),
    };

    let mut store = RootCertStore::empty();
    let mut count = 0u32;
    // Parse PEM-encoded certificates.
    let mut cursor = &pem_data[..];
    loop {
        match rustls_pemfile::read_one(&mut cursor) {
            Ok(Some(rustls_pemfile::Item::X509Certificate(cert))) => {
                let der = CertificateDer::from(cert);
                match store.add(der) {
                    Ok(()) => count += 1,
                    Err(_) => (), // Skip malformed certs silently.
                }
            }
            Ok(Some(_)) => continue, // Skip non-certificate items.
            Ok(None) => break,       // End of file.
            Err(_) => break,         // Parse error; stop.
        }
    }
    if count == 0 {
        return Err(err!(
            "CA bundle '{}' contained no usable certificates.", ca_file;
            Init, Invalid, File));
    }
    info!("Loaded {} CA certificate(s) for outbound HTTPS.", count);

    let config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    Ok(Arc::new(config))
}
