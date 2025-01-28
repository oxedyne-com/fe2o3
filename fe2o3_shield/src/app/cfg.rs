use crate::srv::{
    cfg::ServerConfig,
};

use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};

use std::{
    collections::BTreeMap,
    path::Path,
};


#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct AppConfig {
    pub app_root:           String,
    pub app_name:           String,
    pub app_human_name:     String,
    pub app_description:    String,
    pub app_log_level:      String,
    pub kdf_name:           String,
    pub enc_name:           String,
    pub server_cfg:         DaticleMap,
}

impl Config for AppConfig {
    fn check_and_fix(&mut self) -> Outcome<()> {
        let app_root_path = Path::new(&self.app_root);
        res!(std::fs::create_dir_all(app_root_path));
        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_root:           String::new(),
            app_name:           fmt!("shield"),
            app_human_name:     fmt!("Shield Server"),
            app_description:    fmt!("A Hematite Shield Server app."),
            app_log_level:      fmt!("debug"),
            kdf_name:           fmt!("Argon2id_v0x13"),
            enc_name:           fmt!("AES-256-GCM"),
            server_cfg:         DaticleMap::new(),
        }
    }
}

impl AppConfig {

    pub fn new() -> Outcome<Self> {
        let mut cfg = Self::default();
        cfg.app_root = fmt!("{}", res!(std::env::current_dir()).display());
        cfg.server_cfg = try_extract_dat!(
            ServerConfig::to_datmap(ServerConfig::default()),
            Map,
        );
        Ok(cfg)
    }

    pub fn server_log_level(&self) -> Outcome<(LogLevel, String)> {
        let level_str = if let Some(dat) = self.server_cfg.get(&dat!("log_level")) {
            try_extract_dat!(dat, Str)
        } else {
            return Err(err!(
                "Log level key not found in server configuration: {:?}.",
                self.server_cfg;
            Configuration, Missing, Key));
        };
        let level = res!(LogLevel::from_str(&level_str));
        Ok((level, level_str.clone()))
    }
}
