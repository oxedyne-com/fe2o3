use crate::srv::{
    cfg::ServerConfig,
    dev::cfg::DevConfig,
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
    // Where Steel writes (databases, logs, certificates, spool) and where its keys and `{file:}`
    // references sit. Each takes a `cfg::Roots` spec, and an empty one is the app root.
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
    pub dev_cfg:            DaticleMap,
}

impl Config for AppConfig {
    /// Resolves the app root by `cfg::resolve_root`, with `STEEL_APP_ROOT` for `env`, and
    /// creates it; then checks the state and secret roots and creates the state root.
    fn check_and_fix(&mut self) -> Outcome<()> {
        self.app_root = res!(cfg::resolve_root(&self.app_root, "STEEL_APP_ROOT")) // fixed: the documented env var name
            .to_string_lossy()
            .to_string();

        let app_root_path = Path::new(&self.app_root);

        if let Some(parent) = app_root_path.parent() {
            if !parent.exists() {
                return Err(err!(
                    "Parent directory '{:?}' does not exist for app_root '{:?}'. \
                    You may need to create the parent directories first.",
                    parent, app_root_path;
                    Path, Missing));
            }
        }

        match std::fs::create_dir_all(app_root_path) {
            Ok(()) => (),
            Err(e) => {
                let suggestion = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => fmt!(
                        "You may not have the necessary permissions to create \
                        directories at '{:?}'. Try running with elevated privileges \
                        or choose a different location.", app_root_path),
                    std::io::ErrorKind::NotFound => fmt!(
                        "The path '{:?}' cannot be created. Check that all parent \
                        directories exist and are accessible.", app_root_path),
                    std::io::ErrorKind::AlreadyExists => fmt!(
                        "A file already exists at '{:?}' preventing directory creation.",
                        app_root_path),
                    _ => fmt!(
                        "Unable to create directory at '{:?}'. Check path validity \
                        and permissions.", app_root_path),
                };
                return Err(err!(e, "{}", suggestion; IO, Path, Create));
            }
        }

        let roots = res!(self.roots());
        if !self.state_root.is_empty() {
            // The secret root is not created: it is the operator's, owned by root.
            if let Err(e) = std::fs::create_dir_all(roots.state_root()) {
                return Err(err!(e, "Creating the state root {:?}.", roots.state_root();
                    IO, Path, Create));
            }
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
            app_name:           fmt!("steel"),
            app_human_name:     fmt!("Steel Server"),
            app_description:    fmt!("A Hematite Steel Server app."),
            app_log_level:      fmt!("debug"),
            kdf_name:           fmt!("Argon2id_v0x13"),
            enc_name:           fmt!("AES-256-GCM"),
            server_cfg:         DaticleMap::new(),
            dev_cfg:            DaticleMap::new(),
        }
    }
}

impl AppConfig {

    /// The app, state and secret roots, with `STEEL_APP_ROOT`, `STEEL_STATE_ROOT` and
    /// `STEEL_SECRET_ROOT` for `env`.
    pub fn roots(&self) -> Outcome<cfg::Roots> {
        cfg::Roots::resolve(&self.app_root, &self.state_root, &self.secret_root,
            "STEEL") // fixed: the documented env var prefix
    }

    /// Where the wallet sits: the secret root when one is named, otherwise beside the config,
    /// so a second instance's `--config` still brings its own.
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
        cfg.dev_cfg = try_extract_dat!(
            DevConfig::to_datmap(DevConfig::default()),
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
