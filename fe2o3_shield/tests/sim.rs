use oxedize_fe2o3_shield::{
    prelude::*,
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
    log::{
        console::MultiStreamLoggerConsole,
    },
};
use oxedize_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedize_fe2o3_iop_hash::kdf::KeyDeriver;

use std::{
    fs,
    path::Path,
    time::Duration,
    thread,
};


pub fn test_sim(_filter: &'static str) -> Outcome<()> {

    const NUM_PEERS: usize = 1;

    for peer in 1..=NUM_PEERS {
        let id = fmt!("{:03}", peer);
        test!("peer = {}", id);
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

        match res!(server::start_server::<MultiStreamLoggerConsole<_>>(
            &peer_cfg,
            &AppStatus::default(),
            res!(new_db(&db_root, &db_enc_key)),
            None,
            Some(id),
        )) {
            (_eval, Some((cmd_chan, handle))) => {

                msg!("100");
                thread::sleep(Duration::from_secs(3));

                msg!("110");
                res!(cmd_chan.send(Command::Finish));
                msg!("120");

                thread::sleep(Duration::from_secs(3));

                let rt = res!(tokio::runtime::Runtime::new());
                match rt.block_on(handle) {
                    Ok(server_result) => match server_result {
                        Ok(()) => info!(log_stream(), "Server stopped gracefully."),
                        Err(e) => error!(err!(e,
                            "While running server within tokio runtime."; IO, Thread)),
                    },
                    Err(join_err) => return Err(err!(join_err,
                        "While running server async tokio task."; Async)),
                }
            }
            (_eval, opt) => {
                return Err(err!(
                    "Unexpected server start response: opt = {:?}", opt;
                    Test, Unexpected));
            }
        }
        msg!("200");

    }

    Ok(())
}
