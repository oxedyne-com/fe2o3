use crate::{
    app::{
        constant as app_const,
        repl::AppShellContext,
    },
    srv::{
        cfg::ServerConfig,
        constant as srv_const,
        context::ServerContext,
        msg::syntax as srv_syntax,
        protocol::Protocol,
        schemes::WireSchemesInput,
        server::Server,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    alt::Alt,
    log::{
        bot::FileConfig,
        console::{
            LoggerConsole,
            StdoutLoggerConsole,
        },
    },
    path::NormalPath,
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_hash::csum::ChecksumScheme;
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};
use oxedize_fe2o3_net::id;
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
    path::{
        Path,
        PathBuf,
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
            .normalise() // Now a NormPathBuf.
            .absolute();
        debug!("Reading server config...");
        let mut server_cfg = res!(ServerConfig::from_datmap(self.app_cfg.server_cfg.clone()));
        info!("Validating server config...");
        res!(server_cfg.check_and_fix());
        res!(server_cfg.validate(&root_path));

        // ┌───────────────────────┐
        // │ Determine mode.       │
        // └───────────────────────┘
        let mut test_mode = false;
        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_arg("dev") {
                test_mode = true;
                info!("Running in test mode.");
            }
        }

        if self.stat.first && !test_mode {
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
        // │ Start server.         │
        // └───────────────────────┘
        
        let chunk_cfg = ServerConfig::new_chunk_cfg(1_000, 200, true, true);

        let protocol = res!(Protocol::new(
            &server_cfg,
            WireSchemesInput {
                enc:    Alt::Specific(None::<EncryptionScheme>),
                csum:   Alt::Specific(None::<ChecksumScheme>),
                powh:   Alt::Specific(ServerConfig::default_packet_pow_hash_scheme()),
                sign:   Alt::Specific(ServerConfig::default_packet_signature_scheme()),
                hsenc:  Alt::Specific(None::<EncryptionScheme>),
                chnk:   Some(chunk_cfg),
            },
            [0u8; 8],
            id::Mid::default(),
            id::Sid::default(),
            id::Uid::default(),
            test_mode,
        ));

        let server_context = ServerContext::new(
            server_cfg,
            root_path.clone(),
            Some((self.db.clone(), uid)),
            protocol,
        );

        let syntax = res!(srv_syntax::base_msg());
        let mut server = Server::new(server_context, syntax.clone());
        let rt = res!(tokio::runtime::Runtime::new());

        info!("Starting server...");
        for line in srv_const::SPLASH.lines() {
            info!("{}", line);
        }

        match rt.block_on(server.start()) {
            Ok(()) => info!("Server stopped gracefully."),
            Err(e) => error!(err!(e,
                "While running server within tokio runtime.";
                IO, Thread)),
        }

        log_finish_wait!();

        Ok(Evaluation::Exit)
    }
}
