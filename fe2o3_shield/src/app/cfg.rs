use crate::srv::{
    cfg::ServerConfig,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::{
        self,
        Config,
    },
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
};


#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct AppConfig {
    pub app_root:           String,
    // Where Shield writes (its database and logs) and where its wallet sits. Each takes a
    // `cfg::Roots` spec, and an empty one is the app root.
    #[optional]
    pub state_root:         String,
    #[optional]
    pub secret_root:        String,
    pub app_name:           String,
    pub app_human_name:     String,
    pub app_description:    String,
    pub app_log_level:      String,
    pub kdf_name:           String,
    pub enc_name:           String,
    pub server_cfg:         DaticleMap,
}

impl Config for AppConfig {
    /// Resolves the app root by `cfg::resolve_root`, with `SHIELD_APP_ROOT` for `env`, and
    /// creates it; then checks the state and secret roots and creates the state root.
    fn check_and_fix(&mut self) -> Outcome<()> {
        self.app_root = res!(cfg::resolve_root(&self.app_root, "SHIELD_APP_ROOT")) // fixed: the documented env var name
            .to_string_lossy()
            .to_string();
        let app_root_path = Path::new(&self.app_root);
        res!(std::fs::create_dir_all(app_root_path));
        let roots = res!(self.roots());
        if !self.state_root.is_empty() {
            // The secret root is not created: it is the operator's, owned by root.
            res!(std::fs::create_dir_all(roots.state_root()));
        }
        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_root:           String::new(),
            state_root:         String::new(),
            secret_root:        String::new(),
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

    /// The app, state and secret roots, with `SHIELD_APP_ROOT`, `SHIELD_STATE_ROOT` and
    /// `SHIELD_SECRET_ROOT` for `env`.
    pub fn roots(&self) -> Outcome<cfg::Roots> {
        cfg::Roots::resolve(&self.app_root, &self.state_root, &self.secret_root,
            "SHIELD") // fixed: the documented env var prefix
    }

    /// Where the wallet sits: the secret root when one is named, otherwise beside the config.
    pub fn wallet_dir(&self, cfg_dir: &Path) -> Outcome<PathBuf> {
        if self.secret_root.is_empty() {
            Ok(cfg_dir.to_path_buf())
        } else {
            Ok(res!(self.roots()).secret_root().to_path_buf())
        }
    }

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
