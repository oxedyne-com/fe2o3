use oxedize_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        cfg::ServerConfig,
        constant,
        context::{
            self,
            Protocol,
            ServerContext,
        },
        id,
        server::Server,
        ws::{
            handler::WebSocketDatabaseHandler,
            syntax::WebSocketSyntax,
        },
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    path::NormalPath,
};
use oxedize_fe2o3_jdat::version::SemVer;

use std::{
    path::Path,
    thread,
    time::Duration,
};

pub async fn test_server(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "server" => {
            let cfg = ServerConfig::default();
            let home = res!(std::env::var("HOME"));
            let app_root = Path::new(home).join("usr/code/web/apps/test").normalise().absolute();
            info!("App root path is {:?}", app_root);

            let db_root = app_root
                .clone()
                .join(Path::new(constant::DB_DIR).normalise())
                .normalise().absolute();

            let db_enc_key: [u8; 32] = [
                0x68, 0xee, 0x9a, 0xd0, 0x6a, 0x77, 0x91, 0x4b,
                0xef, 0x52, 0x2c, 0x28, 0x55, 0x5b, 0xe2, 0x8e,
                0xe8, 0xa5, 0xeb, 0xa2, 0xef, 0xe0, 0xe7, 0x74,
                0x82, 0x61, 0x0f, 0x17, 0x24, 0xc4, 0x0d, 0xd2,
            ];

            let mut db = res!(context::new_db(&db_root, &db_enc_key));

            // Since we are testing, delete all existing data and index files.
            let files = res!(db.find_all_data_files());
            test!("Found {} files", files.len());
            for file in files {
                test!("  Deleting {:?}", file);
                res!(std::fs::remove_file(file));
            }

            test!("Starting db...");
            res!(db.start());
            thread::sleep(Duration::from_secs(1));

            let gc_on = false;
            res!(ok!(db.updated_api()).activate_gc(gc_on));

            thread::sleep(Duration::from_secs(1));

            // Ping all database bots.
            let (start, msgs) = res!(db.api().ping_bots(constant::GET_DATA_WAIT));
            test!("{} ping replies received in {:?}.", msgs.len(), start.elapsed());

            let uid = id::Uid::new(0);

            let ws_syntax = res!(WebSocketSyntax::new(
                "steel_ws",
                &SemVer::new(0, 1, 0),
                "Steel Websocket Test Sytnax",
            ));

            let protocol = Protocol::Web{
                handler: AppWebHandler {
                    cfg: cfg.clone(),
                    root: app_root.clone(),
                },
                ws_handler: WebSocketDatabaseHandler,
                ws_syntax,
            };

            let context = ServerContext::new(
                cfg,
                app_root,
                Some((db, uid)),
                protocol,
            );

            let server = Server::new(context);
            let result = server.start().await;
            res!(result);
        }
        _ => (),
    }

    Ok(())
}
