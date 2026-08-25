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
        alert::{
            AlertEvent,
            Alerter,
        },
        cert::Certificate,
        cfg::ServerConfig,
        constant as srv_const,
        context::{
            new_db,
            Protocol,
            ServerContext,
            VhostDbSpec,
            VhostDbs,
            VhostRuntime,
        },
        dev::{
            cfg::DevConfig,
            refresh::DevRefreshManager,
        },
        id,
        server::Server,
        stop,
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


pub const RUNTIME_STOP_SECS: u64 = 2;


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
        // │ Note per-vhost dbs.   │
        // └───────────────────────┘

        // One Ozone instance per vhost that has a `db_dir_rel` configured.
        // Pure-redirect vhosts (no webroot, no database) simply have no
        // entry. The map is keyed by each vhost's canonical (primary)
        // hostname in lowercase, matching what `db_for_vhost` looks up on
        // request dispatch.
        //
        // The databases are *not* opened here. Ozone is encrypted with the
        // wallet master key, and Steel starts sealed -- no passphrase has
        // been supplied, so no key exists yet. We record what to open and
        // leave the map empty; `open_dbs_on_unseal` fills it in once an
        // admin unseals, which may be seconds later (the operator typed a
        // passphrase at the shell) or hours later (a cold restart, unsealed
        // from the dashboard on someone's phone). Either way the listeners
        // bind and the static vhosts serve in the meantime.
        let uid = id::Uid::new(0);
        let vhost_dbs: VhostDbs<{ id::UID_LEN }, id::Uid, O3db<
            { id::UID_LEN },
            id::Uid,
            EncryptionScheme,
            HashScheme,
            HashScheme,
            ChecksumScheme,
        >> = Arc::new(RwLock::new(HashMap::new()));

        let mut db_specs: Vec<VhostDbSpec> = Vec::new();
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
            info!("Vhost '{}' has a database at {:?}; it will be opened on unseal.",
                primary_lc, db_dir);
            db_specs.push(VhostDbSpec {
                vhost_key:  primary_lc,
                db_dir,
            });
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
        // same admin list the CLI sees), the wallet's on-disk path
        // (so the admin-management UI can call Wallet::save), the
        // recovered master key (so it can call Wallet::enrol
        // without re-prompting), an AES-256-GCM cipher pre-keyed
        // with a SHA3-256 derivation of the master key (for
        // session cookies), and the shared TrafficRecorder. Both
        // AdminState and ServerContext hold the same Arc to the
        // recorder so dashboard reads and request-pipeline writes
        // see one consistent view.
        let traffic = TrafficRecorder::new_shared(0);
        let host_sampler = crate::srv::admin::host_sampler::HostSampler::new_shared();
        let addr_guard = res!(crate::srv::admin::guard::new_shared_with(
            server_cfg.get_addr_guard_settings(),
        ));
        // Build the tighter auth-path guard from the general
        // settings, then override the rps cap with the auth-specific
        // value. Everything else (throttle spacing, cooldown, strike
        // count) stays aligned with the general guard so operators
        // only need to tune one number for the common case.
        let mut auth_settings = server_cfg.get_addr_guard_settings();
        auth_settings.rps_max = server_cfg.auth_rps_max;
        let auth_guard = res!(crate::srv::admin::guard::new_shared_with(
            auth_settings,
        ));
        // The periodic traffic and host samplers are spawned inside
        // Server::start (not here) because this function runs in a
        // sync context -- the tokio runtime `rt` has been built but
        // not entered yet, so calling `tokio::spawn` here would
        // panic with "there is no reactor running". Server::start
        // is invoked via `rt.block_on` which makes the runtime
        // current for its body, so any tokio::spawn call from
        // there works.
        let wallet_path_for_admin = Path::new(&self.app_cfg.app_root)
            .join(app_const::WALLET_NAME);
        // Pull the signed-admin-login configuration off the primary
        // vhost, if any. A Steel process can host any number of
        // vhosts, but the admin dashboard is a single cross-vhost
        // surface, so the admin_keys list and head_injection_url
        // are sourced from the canonical vhost (the first entry in
        // the config). This mirrors how the wallet and master key
        // are already a process-wide concern.
        let (admin_keys_cfg, head_injection_url_cfg) = {
            let vhosts = res!(server_cfg.get_vhosts());
            match vhosts.into_iter().next() {
                Some(v) => (v.admin_keys, v.head_injection_url),
                None    => (Vec::new(), None),
            }
        };
        // Operator alerting. The public hostname of the primary vhost is what
        // an alert names, and what its `/admin` link points at.
        // What an alert calls this machine.
        //
        // THE MACHINE, not the first website it happens to serve. This used to
        // be `vhosts.first()`, so a host serving several sites announced itself
        // as whichever one sorted first in the config -- karri reported an
        // outage on jarrah as "seen from need2know.ai", naming a website that
        // has nothing to do with either end of the sentence. An operator reads
        // this half-awake and has to know which box to open.
        //
        // The kernel's own answer, read the way `fe2o3_sys` reads everything
        // else about a host, with the first vhost kept only as a fallback for a
        // system that does not carry one.
        let alert_host = {
            let machine = std::fs::read_to_string("/proc/sys/kernel/hostname")
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if !machine.is_empty() {
                machine
            } else {
                let vhosts = res!(server_cfg.get_vhosts());
                match vhosts.first() {
                    Some(v) => v.primary_hostname().to_string(),
                    None    => String::new(),
                }
            }
        };
        let alerter = match res!(server_cfg.get_alerts()) {
            Some(mut cfg) => {
                // Expand `{file:...}` in the submission credential, so the
                // password is not sitting in config.jdat in the clear.
                res!(cfg.resolve_secrets(root_path.as_ref()));
                // Sign alerts with the host's own DKIM keys, when it has any.
                // An unsigned message from a domain that signs everything else
                // is what a spam filter is entitled to distrust, and the alert
                // is the one message that has to arrive.
                let dkim = match res!(server_cfg.get_mail()) {
                    Some(mail_cfg) => res!(
                        crate::srv::server::load_dkim_signers(&mail_cfg, &root_path)),
                    None => Vec::new(),
                };
                let to = cfg.to.join(", ");
                let via = match &cfg.submission {
                    Some(s) => fmt!("via {}:{}", s.host, s.port),
                    None    => fmt!("direct to the recipient's MX"),
                };
                let signed = if dkim.is_empty() {
                    fmt!("unsigned")
                } else {
                    fmt!("DKIM-signed with {} key(s)", dkim.len())
                };
                let a = res!(Alerter::new(
                    cfg, alert_host.clone(), dkim, tls_client.clone()));
                info!("Alerting enabled; operator alerts go to {} ({}, {}).",
                    to, via, signed);
                a
            }
            None => {
                info!("Alerting is not configured. Steel will not tell anybody \
                    when it comes up sealed or when its passphrase is guessed at.");
                None
            }
        };

        // Watching the other machines. This is the half of alerting that a host
        // cannot do for itself: a machine that has died sends nothing, and
        // silence reads exactly like health. See `srv::watch`.
        //
        // Requires an alerter, because a watcher with nothing to report through
        // is a thread that discovers an outage and keeps it to itself. Requires
        // an outbound TLS client for the same practical reason. Both absences
        // are said out loud rather than logged at debug, since the operator who
        // configured a watch believes they are covered.
        match res!(server_cfg.get_watch()) {
            Some(wcfg) => match (&alerter, &tls_client) {
                (Some(a), Some(tls)) => {
                    let w = res!(crate::srv::watch::Watcher::new(
                        Arc::new(wcfg),
                        Arc::new(a.clone()),
                        tls.clone(),
                        alert_host.to_string(),
                    ));
                    // `rt.spawn`, NOT `tokio::spawn`. This function is sync: the
                    // runtime is built at the top and is not current until
                    // `block_on` further down, so `tokio::spawn` here panics with
                    // "there is no reactor running" -- exactly as the comment
                    // beside the samplers above says it will. That panic took a
                    // live server into a restart loop on 2026-08-10, because the
                    // code compiles perfectly and fails only when a server is
                    // actually started. A change to this function is not tested
                    // until something has been started with it.
                    rt.spawn(w.run());
                },
                (None, _) => warn!("A watch list is configured but alerting is not, \
                    so nothing would be told about a peer going down. The watcher \
                    was not started."),
                (_, None) => warn!("A watch list is configured but this Steel has no \
                    outbound TLS client, so it cannot probe anything. The watcher \
                    was not started."),
            },
            None => {},
        }

        // The site's own newsletter sender: the DKIM identities and an outbound SMTP client, built
        // from the same mail configuration the mail server and the alerter use, and shared by every
        // vhost. `None` where no mail is configured -- newsletter signup then answers "not set up"
        // rather than recording a pending subscriber it could never confirm. A build failure warns and
        // disables the newsletter rather than taking the server down: the websites do not depend on it.
        let mail_sender = match res!(server_cfg.get_mail_any()) {
            // Built whenever there is a sending identity -- a hostname to greet with and at least one
            // DKIM key -- independent of whether the mail *server* (the listeners) is enabled. This is
            // what lets a site send its newsletter without becoming an MX.
            Some(mail_cfg) if !mail_cfg.hostname.is_empty()
                && (!mail_cfg.dkim_key_file.is_empty() || !mail_cfg.dkim_rsa_key_file.is_empty()) => {
                let dkim = res!(
                    crate::srv::server::load_dkim_signers(&mail_cfg, &root_path));
                let domain = if mail_cfg.dkim_domain.is_empty() {
                    mail_cfg.hostname.clone()
                } else {
                    mail_cfg.dkim_domain.clone()
                };
                let default_from = fmt!("news@{}", domain);
                match crate::srv::publish::send::MailSender::new(
                    mail_cfg.hostname.clone(), dkim.clone(), default_from.clone())
                {
                    Ok(s) => {
                        info!("Newsletter sender ready (default from {}, {} DKIM key(s)).",
                            default_from, dkim.len());
                        Some(Arc::new(s))
                    }
                    Err(e) => {
                        warn!("Building the newsletter sender failed ({}); newsletter \
                            signup will report itself unavailable.", e);
                        None
                    }
                }
            }
            _ => {
                info!("No mail configured, so the newsletter is unavailable; signup says so.");
                None
            }
        };

        let admin_state = res!(AdminState::new(
            self.wallet.clone(),
            wallet_path_for_admin,
            self.db_enc_key.clone(),
            db_specs.len(),
            alerter,
            traffic.clone(),
            host_sampler.clone(),
            addr_guard.clone(),
            auth_guard.clone(),
            admin_keys_cfg,
            head_injection_url_cfg,
        ));
        let admin_state = Arc::new(admin_state);
        info!("Admin dashboard runtime initialised \
            (traffic ring capacity {}; host sampler capacity {}).",
            traffic.capacity(),
            host_sampler.history_capacity());
        if admin_state.seal_withholds_data() {
            warn!("Steel is SEALED: no wallet master key has been supplied, so the \
                {} configured database(s) are shut and routes that need them will \
                answer 503. Static vhosts, redirects, proxy routes and certificate \
                renewal all serve normally. Sign in at /admin with an admin \
                passphrase to unseal.",
                db_specs.len());
            // The alert that matters most. The websites are up, so nothing
            // looks wrong from outside, and without this the operator learns
            // that the databases are shut from a user complaint. Queued on the
            // runtime handle because `raise` spawns, and the runtime is not
            // current until `block_on` below.
            if let Some(a) = admin_state.alerter().cloned() {
                let n = db_specs.len();
                rt.spawn(async move {
                    a.raise(AlertEvent::SealedStart { db_count: n });
                });
            }
        } else if admin_state.is_sealed() {
            info!("Steel is sealed -- no wallet master key has been supplied -- but \
                no vhost has a database configured, so nothing is waiting on it and \
                everything serves normally.");
        }

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

            // Resolve {file:} placeholders in API route headers and in
            // in-process handler config (e.g. a Stripe secret key).
            let mut api_routes = vh.api_routes.clone();
            for route in &mut api_routes {
                res!(route.resolve_headers(root_path.as_ref()));
                res!(route.resolve_config(root_path.as_ref()));
            }
            if !api_routes.is_empty() {
                info!("Vhost '{}': {} API route(s) configured.",
                    vh.primary_hostname(), api_routes.len());
            }
            if !vh.proxy_routes.is_empty() {
                info!("Vhost '{}': {} proxy route(s) configured.",
                    vh.primary_hostname(), vh.proxy_routes.len());
            }
            for route in &vh.ws_routes {
                info!("Vhost '{}': ws route {} -> ws://{}:{}{}",
                    vh.primary_hostname(), route.path,
                    route.upstream_host, route.upstream_port, route.upstream_path);
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

            // Resolve {file:}/{env:} placeholders in the publish module's destination credentials, so a
            // token reaches the sender resolved and never sits in the config in the clear.
            let mut publish = vh.publish.clone();
            if let Some(p) = &mut publish {
                res!(p.resolve_secrets(root_path.as_ref()));
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
                publish.map(Arc::new),
                mail_sender.clone(),
                Arc::new(vh.site_admins.clone()),
            );

            // One terminal manager per vhost, built only where the config asks
            // for it. It is shared two ways: attached to the WS syntax handler
            // for the `term_*` management commands, and held on the runtime so
            // the `/term/` bridge router can see whether the feature exists at
            // all -- the presence of this is the config gate that decides
            // whether an upgrade to `/term/<name>` is dispatched or refused.
            let term_manager = vh.term_config.as_ref().map(|tc| Arc::new(
                crate::srv::ws::term::TerminalManager::new(
                    &tc.session_prefix,
                    &tc.launch_command,
                )
            ));
            let runtime = Arc::new(VhostRuntime {
                hostnames:      vh.hostnames.clone(),
                web_handler,
                ws_handler:     match &term_manager {
                    Some(tm) => ws_handler.clone().with_term_manager(tm.clone()),
                    None     => ws_handler.clone(),
                },
                ws_syntax:      ws_syntax.clone(),
                redirects:      vh.redirects.clone(),
                proxy_routes:   vh.proxy_routes.clone(),
                ws_routes:      vh.ws_routes.clone(),
                term_manager:   term_manager.clone(),
                uses_sessions:  vh.uses_sessions(),
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
            vhost_dbs.clone(),
            db_specs.clone(),
            protocol,
            Some(traffic.clone()),
            Some(admin_state.clone()),
        );

        let server = Server::new(server_context);

        // From here until the stores are shut, a stop request is answered
        // by the wind-up below rather than by the listener ending the
        // process itself. Set before the opener is spawned, not after the
        // listeners bind, so that a signal arriving while a store is being
        // opened still finds somebody who will close it.
        stop::serving(true);

        // The database opener waits for the master key and then opens
        // every configured Ozone instance. Spawned on the runtime handle
        // *before* `block_on` drives it, so it is already waiting on the
        // unseal signal by the time the first request can arrive.
        //
        // When the operator has already supplied a passphrase (via the
        // shell's `unseal`, or `STEEL_ADMIN_PASS`) the key is present and
        // this returns without waiting -- so the familiar start-up path is
        // unchanged apart from the databases now opening in parallel with
        // the listeners binding, rather than before them.
        // Cloned, not moved: the same map is what the wind-up reads to
        // find the stores it has to close.
        rt.spawn(open_dbs_on_unseal(
            admin_state.clone(),
            vhost_dbs.clone(),
            db_specs,
            uid,
        ));

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

        // The runtime is shut down, not dropped. Dropping a runtime waits
        // for its blocking pool, and in dev mode that pool holds the file
        // watcher -- `notify`'s own event loop, which by design never
        // returns. A stop therefore came all the way home, closed
        // everything, said so, and then hung for ever in the drop, one
        // line before the end of the function. Nothing here needs those
        // threads: what mattered has already finished inside `block_on`,
        // and asynchronous tasks are dropped at their await points.
        rt.shutdown_timeout(Duration::from_secs(RUNTIME_STOP_SECS));

        // The point of all of it. Whatever brought the server home --
        // a stop request or a failure -- every store it holds is shut
        // before the process ends, rather than being killed open. After
        // the runtime has gone, so the periodic persist task cannot be
        // writing into a database while it is being closed.
        let closed = close_vhost_dbs(&vhost_dbs);
        info!("{} database(s) closed.", closed);
        stop::serving(false);

        log_finish_wait!();

        Ok(Evaluation::Exit)
    }

}

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

    let mut config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    // Advertise HTTP/1.1 via ALPN so CDN-fronted servers (e.g.
    // Fireworks.ai behind Cloudflare) don't close the connection
    // after the TLS handshake when no protocol is negotiated.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DATABASE OPENER AND CLOSER                                                │
// └───────────────────────────────────────────────────────────────────────────┘

type VhostDb = O3db<
    { id::UID_LEN },
    id::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

fn close_vhost_dbs(
    vhost_dbs: &VhostDbs<{ id::UID_LEN }, id::Uid, VhostDb>,
) -> usize {
    let dbs = lock_read_or_recover!(vhost_dbs,
        "The per-vhost database map is poisoned. Closing what it holds anyway: \
        an open store is the thing worth minding here.");
    if dbs.is_empty() {
        info!("No database is open, so there is none to close.");
        return 0;
    }
    let mut closed = 0;
    for (vhost, (db, _uid)) in dbs.iter() {
        info!("Closing the database for vhost '{}'...", vhost);
        let db = lock_read_or_recover!(db,
            "The database handle for vhost '{}' is poisoned. Closing it anyway.",
            vhost);
        match db.close() {
            Ok(()) => {
                info!("Closed the database for vhost '{}'.", vhost);
                closed += 1;
            },
            Err(e) => error!(e,
                "Closing the database for vhost '{}'. Its files may need a \
                repair pass on the next start.", vhost),
        }
    }
    closed
}

async fn open_dbs_on_unseal(
    admin_state:    Arc<AdminState>,
    vhost_dbs:      VhostDbs<{ id::UID_LEN }, id::Uid, O3db<
                        { id::UID_LEN },
                        id::Uid,
                        EncryptionScheme,
                        HashScheme,
                        HashScheme,
                        ChecksumScheme,
                    >>,
    db_specs:       Vec<VhostDbSpec>,
    uid:            id::Uid,
) {
    if db_specs.is_empty() {
        return;
    }

    let enc_key = match admin_state.await_master_key().await {
        Ok(k) => k,
        Err(e) => {
            error!(e, "Waiting for the wallet master key. No database will \
                be opened; Steel remains sealed.");
            return;
        }
    };

    let outcome = tokio::task::spawn_blocking(move || -> Outcome<usize> {
        let mut opened = 0;
        for spec in &db_specs {
            info!("Starting database for vhost '{}' at {:?}...",
                spec.vhost_key, spec.db_dir);
            let mut db = res!(new_db(&spec.db_dir, &enc_key));
            // Label is the vhost's canonical name so log output
            // disambiguates multi-vhost deployments.
            let label = fmt!("db_{}", spec.vhost_key);
            res!(db.start(&label));
            res!(ok!(db.updated_api()).activate_gc(true));

            std::thread::sleep(Duration::from_millis(200));

            let (start, msgs) = res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
            info!("Vhost '{}': {} ping replies in {:?}.",
                spec.vhost_key, msgs.len(), start.elapsed());

            // Publish as soon as each database is up, rather than
            // batching at the end: a vhost whose database is ready has
            // no reason to keep answering 503 while a later one starts.
            let mut guard = lock_write!(vhost_dbs,
                "Attaching the database for vhost '{}'.", spec.vhost_key);
            guard.insert(
                spec.vhost_key.clone(),
                (Arc::new(RwLock::new(db)), uid),
            );
            opened += 1;
        }
        Ok(opened)
    }).await;

    match outcome {
        Ok(Ok(n)) => info!("Unsealed: {} database(s) open and attached.", n),
        Ok(Err(e)) => error!(e, "Opening the per-vhost databases after unseal."),
        Err(e) => error!(err!(e,
            "The database opener task failed to join.";
            Thread, Panic),
            "Opening the per-vhost databases after unseal."),
    }
}
