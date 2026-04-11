use crate::{
    app::{
        self,
        constant as app_const,
        https::AppWebHandler,
        repl::AppShellContext,
    },
    srv::{
        cert::Certificate,
        cfg::ServerConfig,
        constant as srv_const,
        context::{
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
    sync::Arc,
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
        // │ Start database.       │
        // └───────────────────────┘
        
        info!("Starting database...");
        res!(self.db.start("database"));
        res!(ok!(self.db.updated_api()).activate_gc(true));

        std::thread::sleep(Duration::from_secs(1));

        let uid = id::Uid::new(0);

        // Ping all bots.
        let (start, msgs) = res!(self.db.api().ping_bots(app_const::GET_DATA_WAIT));
        info!("{} ping replies received in {:?}.", msgs.len(), start.elapsed());

        // ┌───────────────────────┐
        // │ Certificates.         │
        // └───────────────────────┘

        // Parse the vhosts and ACME config so we can decide the cert strategy.
        let vhosts_cfg = res!(server_cfg.get_vhosts());
        let acme_cfg = res!(server_cfg.get_acme());

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

            let web_handler = AppWebHandler::new(
                server_cfg.clone(),
                public_dir,
                static_routes,
                default_index_files,
                dev_mode,
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
            Some((self.db.clone(), uid)),
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
