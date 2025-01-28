use crate::{
    prelude::*,
    app::{
        cfg::AppConfig,
        constant as app_const,
        tui::AppStatus,
    },
    srv::{
        cfg::ServerConfig,
        cmd::Command,
        constant as srv_const,
        context::ServerContext,
        msg::{
            protocol::{
                DefaultProtocolTypes,
                Protocol,
                ProtocolMode,
            },
            syntax as srv_syntax,
        },
        schemes::WireSchemesInput,
        server::{
            LOG_STREAM_ID,
            Server,
        },
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    alt::Alt,
    channels::Simplex,
    log::{
        bot::FileConfig,
        console::LoggerConsole,
    },
    path::NormalPath,
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};
use oxedize_fe2o3_net::id;
use oxedize_fe2o3_o3db_sync::O3db;
use oxedize_fe2o3_syntax::{
    msg::{
        MsgCmd,
    },
};
use oxedize_fe2o3_tui::lib_tui::{
    repl::Evaluation,
};

use std::{
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

use tokio;


pub fn start_server<
    L: LoggerConsole<ErrTag>,
>(
    app_cfg:        &AppConfig,
    stat:           &AppStatus,
    mut db:         O3db<
                        { id::UID_LEN },
                        id::Uid,
                        EncryptionScheme,
                        HashScheme,
                        HashScheme,
                        ChecksumScheme,
                    >,
    cmd:            Option<&MsgCmd>,
    test_stream:    Option<String>,
)
    -> Outcome<(
        Evaluation,
        Option<(
            Simplex<Command>,
            tokio::task::JoinHandle<Outcome<()>>,
        )>,
    )>
{
    let root_path = Path::new(&app_cfg.app_root)
        .normalise() // Now a NormPathBuf.
        .absolute();
    debug!("Reading server config...");
    let mut server_cfg = res!(ServerConfig::from_datmap(app_cfg.server_cfg.clone()));
    info!("Validating server config...");
    res!(server_cfg.check_and_fix());
    res!(server_cfg.validate(&root_path));

    // ┌───────────────────────┐
    // │ Determine mode.       │
    // └───────────────────────┘
    let mut mode = ProtocolMode::Production;
    if let Some(msg_cmd) = cmd {
        if msg_cmd.has_arg("dev") {
            mode = ProtocolMode::Dev;
            info!("Running in dev mode.");
        }
    }

    if stat.first && matches!(mode, ProtocolMode::Production) {
        return Ok((
            Evaluation::Error(fmt!(
                "You should update values in {} before running the server in production mode.",
                app_const::CONFIG_NAME,
            )),
            None,
        ));
    }

    // ┌───────────────────────┐
    // │ Reconfigure logging.  │
    // └───────────────────────┘
    let mut log_cfg = get_log_config!();
    // Console:
    let mut logger_console = L::new();
    let logger_console_thread = logger_console.go();
    log_cfg.console = Some(logger_console_thread.chan.clone());
    // File::
    if test_stream.is_none() {
        log_cfg.file = Some(FileConfig::new(
            PathBuf::from(&root_path).join("www").join("logs"),
            app_cfg.app_name.clone(),
            "log".to_string(),
            0,
            Some(1_048_576), // Activate multiple log file archiving using this max size.
        ));
    } else {
        // Testing.
        log_cfg.file = None;
    };
    (log_cfg.level, _) = res!(app_cfg.server_log_level());
    set_log_config!(log_cfg);
    if test_stream.is_none() {
        println!("Server now logging at {:?}", get_log_file_path!());
    } else {
        test!(log_stream(), "Server logs now streaming to multiple channels.");
    }
    info!(log_stream(), "┌───────────────────────┐");
    info!(log_stream(), "│ New server session.   │");
    info!(log_stream(), "└───────────────────────┘");
    msg!("1000");

    // ┌───────────────────────┐
    // │ Start database.       │
    // └───────────────────────┘
    info!(log_stream(), "Starting database...");
    res!(db.start());
    res!(ok!(db.updated_api()).activate_gc(true));

    std::thread::sleep(Duration::from_secs(1));

    let uid = id::Uid::new(0);

    // Ping all bots.
    let (start, msgs) = res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
    info!(log_stream(), "{} ping replies received in {:?}.", msgs.len(), start.elapsed());

    msg!("1010");
    // ┌───────────────────────┐
    // │ Start server.         │
    // └───────────────────────┘
    
    let chunk_cfg = ServerConfig::new_chunk_cfg(1_000, 200, true, true);

    let protocol: Protocol<
        8,
        {id::MID_LEN},
        {id::SID_LEN},
        {id::UID_LEN},
        DefaultProtocolTypes<
            {id::MID_LEN},
            {id::SID_LEN},
            {id::UID_LEN},
        >,
    > =
        res!(Protocol::new(
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
            if test_stream.is_some() { ProtocolMode::Test } else { mode },
        ));

    let server_context = ServerContext::new(
        server_cfg,
        root_path.clone(),
        Some((db.clone(), uid)),
        protocol,
    );

    let syntax = res!(srv_syntax::base_msg());
    let (mut server, cmd_chan) = Server::new(server_context, syntax.clone());
    let rt = res!(tokio::runtime::Runtime::new());

    info!(log_stream(), "Starting server...");
    for line in srv_const::SPLASH.lines() {
        info!(log_stream(), "{}", line);
    }

    msg!("1020");
    let handle = rt.spawn(LOG_STREAM_ID.scope(
        if let Some(stream) = test_stream {
            stream
        } else {
            fmt!("main")
        },
        async move { server.start().await },
    ));

    //log_finish_wait!();

    msg!("1030");
    Ok((Evaluation::Exit, Some((cmd_chan, handle))))
}
