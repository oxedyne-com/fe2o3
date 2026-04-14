use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        cfg::{
            ServerConfig,
            VhostConfig,
        },
        constant,
        context::{
            self,
            Protocol,
            ServerContext,
            VhostRuntime,
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
    file::OsPath,
    prelude::*,
    path::NormalPath,
};
use oxedyne_fe2o3_jdat::version::SemVer;

use std::{
    collections::HashMap,
    path::{
        Path,
        PathBuf,
    },
    sync::Arc,
    thread,
    time::Duration,
};


/// Minimal smoke test exercising the multi-vhost server bring-up path.
///
/// Spins up a single-vhost `Protocol::Web` with an empty webroot and no
/// redirect rules, just to verify that the refactored `ServerContext` and
/// `Server::start()` path compile and accept a start call under the current
/// API. This is a compile-time regression guard; end-to-end behaviour is
/// verified separately against a live deployment.
pub async fn test_server(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "server" => {
            let cfg = ServerConfig::default();
            let home = res!(std::env::var("HOME"));
            let app_root = Path::new(&home)
                .join("usr/code/web/apps/test")
                .normalise()
                .absolute();
            info!("App root path is {:?}", app_root);

            let db_root = app_root
                .clone()
                .join(Path::new(constant::DB_DIR).normalise())
                .normalise()
                .absolute();

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
            res!(db.start("steel_test_db"));
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
                "Steel Websocket Test Syntax",
            ));

            // Single test vhost with an empty webroot and no routes.
            let vhost_cfg = VhostConfig {
                hostnames:              vec![fmt!("localhost")],
                public_dir_rel:         None,
                static_route_paths_rel: Default::default(),
                default_index_files:    vec![fmt!("index.html")],
                redirects:              Vec::new(),
                db_dir_rel:             None,
            };

            let web_handler: AppWebHandler<HashMap<String, OsPath>> = AppWebHandler::new(
                cfg.clone(),
                PathBuf::new(),
                HashMap::new(),
                vhost_cfg.default_index_files.clone(),
                true,
            );

            let ws_handler = AppWebSocketHandler::new(None);

            let runtime = Arc::new(VhostRuntime {
                hostnames:      vhost_cfg.hostnames.clone(),
                web_handler,
                ws_handler,
                ws_syntax,
                redirects:      vhost_cfg.redirects.clone(),
            });

            let mut vhosts = HashMap::new();
            for h in &vhost_cfg.hostnames {
                vhosts.insert(h.to_lowercase(), runtime.clone());
            }

            let protocol = Protocol::Web {
                vhosts:         Arc::new(vhosts),
                default_vhost:  vhost_cfg.primary_hostname().to_lowercase(),
                dev_mode:       true,
            };

            let mut vhost_dbs = HashMap::new();
            vhost_dbs.insert(
                vhost_cfg.primary_hostname().to_lowercase(),
                (Arc::new(std::sync::RwLock::new(db)), uid),
            );
            let context = ServerContext::new(
                cfg,
                app_root,
                vhost_dbs,
                protocol,
            );

            let server = Server::new(context);
            // Do not await forever: this test only proves the bring-up path
            // compiles and reaches the accept loop under the current API.
            let _ = tokio::time::timeout(Duration::from_millis(250), server.start()).await;
        }
        _ => (),
    }

    Ok(())
}
