use crate::srv::{
    cfg::ServerConfig,
    dev::cfg::DevConfig,
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
    pub dev_cfg:            DaticleMap,
}

impl Config for AppConfig {
    /// Validates and resolves the app_root path. Special values:
    /// - "current" (or "here", "auto", "."): Use current working directory
    /// - "env": Read from STEEL_APP_ROOT environment variable
    /// - "~/path": Expand home directory
    /// - "./path": Relative to current directory
    /// - "/path": Absolute path (used as-is)
    fn check_and_fix(&mut self) -> Outcome<()> {
        self.app_root = match self.app_root.as_str() {
            "current" | "here" | "auto" | "." => {
                res!(std::env::current_dir())
                    .to_string_lossy()
                    .to_string()
            }
            "env" => {
                match std::env::var("STEEL_APP_ROOT") {
                    Ok(path) => path,
                    Err(_) => return Err(err!(
                        "STEEL_APP_ROOT environment variable not set when app_root is 'env'.";
                        Configuration, Missing, Invalid)),
                }
            }
            path if path.starts_with("~/") => {
                let home = match std::env::var("HOME") {
                    Ok(home) => home,
                    Err(_) => return Err(err!(
                        "HOME environment variable not set for path expansion.";
                        Configuration, Missing, Invalid)),
                };
                fmt!("{}{}", home, &path[1..])
            }
            path if path.starts_with("./") => {
                let cwd = res!(std::env::current_dir());
                cwd.join(&path[2..]).to_string_lossy().to_string()
            }
            path => path.to_string(),
        };

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

        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_root:           String::new(),
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
