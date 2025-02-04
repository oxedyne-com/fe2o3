use oxedize_fe2o3_shield::{
    //prelude::*,
    app::{
        cfg::AppConfig,
        constant,
        server,
        tui::AppStatus,
    },
    srv::{
        cmd::Command,
        context::new_db,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    channels::Recv,
    log::{
        console::{
            switch_to_logger_console,
            MultiStreamLoggerConsole,
            StdoutLoggerConsole,
        },
    },
};
use oxedize_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedize_fe2o3_iop_hash::kdf::KeyDeriver;

use std::{
    fs,
    path::Path,
    time::{
        Duration,
        Instant,
    },
    thread,
};


pub fn test_sim(_filter: &'static str) -> Outcome<()> {
    let rt = res!(tokio::runtime::Runtime::new());
    rt.block_on(run_test_sim(_filter))
}

pub async fn run_test_sim(_filter: &'static str) -> Outcome<()> {

    test!("Reconfiguring log to stream to multiple channels...");
    res!(switch_to_logger_console::<MultiStreamLoggerConsole<_>>());
    let mut log_cfg = log_get_config!();
    log_cfg.file = None;
    log_set_config!(log_cfg);

    const NUM_PEERS: usize = 1;

    let mut server_handles = Vec::new();

    for peer in 1..=NUM_PEERS {
        let id = fmt!("{:03}", peer);
        msg!("Starting peer: {}", id);
        let peer_dir = fmt!("sims/{}", id);

        if res!(fs::exists(&peer_dir)) {
            res!(fs::remove_dir_all(&peer_dir));
        }
        res!(fs::create_dir_all(&peer_dir));

        let mut peer_cfg = res!(AppConfig::new());
        peer_cfg.app_root = peer_dir.clone();

        let app_root = Path::new(&peer_cfg.app_root);
        let db_root = app_root.join(constant::DB_DIR);

        let mut db_kdf = res!(KeyDerivationScheme::from_str(&peer_cfg.kdf_name));
        res!(db_kdf.derive(id.as_bytes())); // Use the id as the db encryption passphrase.
        let db_enc_key = res!(db_kdf.get_hash()).to_vec();

        match res!(server::start_server(
            &peer_cfg,
            &AppStatus::default(),
            res!(new_db(&db_root, &db_enc_key)),
            None,
            Some(id.clone()),
        ).await) {
            (_eval, Some((cmd_chan, handle))) => {
                server_handles.push((id, cmd_chan, handle));
            }
            (_eval, opt) => {
                return Err(err!(
                    "Unexpected server start response: opt = {:?}", opt;
                    Test, Unexpected));
            }
        }
    }

    thread::sleep(Duration::from_secs(5));

    let id = fmt!("001");
    match log_get_streams!(Duration::from_secs(2)) {
        Ok(Some(streams_map)) => {
            let unlocked_map = lock_read!(streams_map);
            if let Some(log_stream_chan) = unlocked_map.get(&id) {
                // Print out log stream for 5s.
                let duration = Duration::from_secs(5);
                let send_finish = Duration::from_secs(4);
                let start_time = Instant::now();
                let mut finish_sent = false;

                while start_time.elapsed() < duration {
                    match log_stream_chan.try_recv() {
                        Recv::Empty => {}
                        Recv::Result(Ok(line)) => {
                            println!("{}", line);
                        }
                        Recv::Result(Err(e)) => return Err(err!(e,
                            "While reading command channel."; Channel, Read)),
                    }
                    if !finish_sent && start_time.elapsed() > send_finish {
                        for (_, cmd_chan, _) in &server_handles {
                            res!(cmd_chan.send(Command::Finish));
                        }
                        finish_sent = true;
                    }
                }
            }
        }
        Ok(None) => {
            return Err(err!("Could not acquire log streams map."; Channel, Missing)); 
        }
        Err(e) => {
            return Err(err!(e, "While requesting log streams map."; Channel)); 
        }
    }

    res!(switch_to_logger_console::<StdoutLoggerConsole<_>>());

    thread::sleep(Duration::from_secs(1));

    // Wait for all servers to finish
    for (id, _, handle) in server_handles {
        match handle.await {
            Ok(server_result) => match server_result {
                Ok(()) => info!("Server {} stopped gracefully.", id),
                Err(e) => error!(err!(e,
                    "While running server {} task.", id; IO, Thread)),
            },
            Err(join_err) => return Err(err!(join_err,
                "While awaiting server {} task completion.", id; Async)),
        }
    }

    Ok(())
}
