use crate::{
    app::{
        //cfg::AppConfig,
        constant as app_const,
        https::AppWebHandler,
        repl::AppShellContext,
        //smtps::AppEmailHandler,
    },
    srv::{
        cert::Certificate,
        cfg::ServerConfig,
        constant as srv_const,
        context::{
            Protocol,
            ServerContext,
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

use oxedize_fe2o3_core::{
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
use oxedize_fe2o3_jdat::{
    prelude::*,
};
use oxedize_fe2o3_syntax::{
    msg::{
        MsgCmd,
    },
};
use oxedize_fe2o3_tui::{
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
            .normalise() // Now a NormPathBuf.
            .absolute();
        debug!("Reading server config...");
        let server_cfg = res!(ServerConfig::from_datmap(self.app_cfg.server_cfg.clone()));
        info!("Validating server config...");
        res!(server_cfg.validate(&root_path));
        debug!("Reading dev config...");
        let dev_cfg = res!(DevConfig::from_datmap(self.app_cfg.dev_cfg.clone()));

        // ┌───────────────────────┐
        // │ Determine mode.       │
        // └───────────────────────┘
        let mut dev_mode = false;
        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_arg("dev") {
                dev_mode = true;
                info!("Running in development mode with self-signed certificates.");
                res!(dev_cfg.validate(&root_path));
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
        let mut log_cfg = get_log_config!();
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
        set_log_config!(log_cfg);
        println!("Server now logging at {:?}", get_log_file_path!());
        info!("┌───────────────────────┐");
        info!("│ New server session.   │");
        info!("└───────────────────────┘");

        // ┌───────────────────────┐
        // │ Start database.       │
        // └───────────────────────┘
        info!("Starting database...");
        res!(self.db.start());
        res!(ok!(self.db.updated_api()).activate_gc(true));

        std::thread::sleep(Duration::from_secs(1));

        let uid = id::Uid::new(0);

        // Ping all bots.
        let (start, msgs) = res!(self.db.api().ping_bots(app_const::GET_DATA_WAIT));
        info!("{} ping replies received in {:?}.", msgs.len(), start.elapsed());

        // ┌───────────────────────┐
        // │ Use self-signed       │
        // │ certificates.         │
        // └───────────────────────┘
        let (tls_dir, cert_path, key_path) = res!(server_cfg.get_tls_paths(&root_path, dev_mode));
        debug!("tls_dir = {:?}", tls_dir);
        debug!("cert_path = {:?}", cert_path);
        debug!("key_path = {:?}", key_path);
        let domains = res!(server_cfg.get_domain_names());

        if !cert_path.exists() || !key_path.exists() {
            if dev_mode {
                info!("Development certificates not found - generating self-signed certificates.");
                res!(Certificate::new_dev(
                    &server_cfg,
                    &root_path,
                ));
            } else {
                info!("Production certificates not found - generating self-signed certificates.");
                res!(Certificate::new_lets_encrypt(
                    &domains,
                    &tls_dir,
                ));
            }
        }

        if dev_mode {
            info!("Connect via: https://localhost:{}", server_cfg.server_port_tcp);
        } else {
            info!("Connect via: https://{}", domains[0].as_str());
        }

        // ┌───────────────────────┐
        // │ Refresh the css and   │
        // │ javascript bundles.   │
        // └───────────────────────┘
        let js_bundles_map = res!(dev_cfg.get_js_bundles_map(&root_path));
        let js_import_aliases = res!(dev_cfg.get_js_import_aliases(&root_path));
        let css_paths = res!(dev_cfg.get_css_paths(&root_path));
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
                rt.spawn(async move {
                    debug!("Starting dev refresh file watcher.");
                    if let Err(e) = manager_clone.watch() {
                        error!(err!(e, errmsg!(
                            "Failed to start development file watcher.",
                        )));
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

        let web_handler = AppWebHandler::new(
            server_cfg.clone(),
            res!(server_cfg.get_public_dir(&root_path)),
            res!(server_cfg.get_static_route_paths(
                &root_path,
                HashMap::new(),
            )),
            res!(server_cfg.get_default_index_files()),
            dev_mode,
        );
        let protocol = Protocol::Web {
            web_handler,
            ws_handler,
            ws_syntax,
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
