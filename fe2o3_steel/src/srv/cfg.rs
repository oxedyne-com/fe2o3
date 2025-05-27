use crate::srv::constant;

use oxedize_fe2o3_core::{
    prelude::*,
    file::{
        OsPath,
        PathState,
    },
    map::MapMut,
    path::{
        self,
        NormalPath,
        NormPathBuf,
    },
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};
use oxedize_fe2o3_net::{
    constant::SESSION_ID_KEY_LABEL,
    dns::Fqdn,
    http::{
        fields::{
            Cookie,
            SetCookieAttributes,
            SameSite,
        },
    },
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};


#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct ServerConfig {
    // TLS
    pub tls_dir_rel:                    String,
    pub tls_public_key_name:            String,
    pub tls_private_key_name:           String,
    pub tls_cert_name:                  String,
    pub tls_cert_address:               String,
    pub domain_names:                   Vec<String>,
    // Server
    pub log_level:                      String,
    pub num_server_bots:                u16,
    pub server_address:                 String,
    pub server_port_tcp:                u16,
    pub session_expiry_default_secs:    u32,
    pub ws_ping_interval_secs:          u8,
    pub server_max_errors_allowed:      u8,
    // Paths
    pub public_dir_rel:                 String, // Relative to the root directory.
    pub static_route_paths_rel:         DaticleMap, // Relative to the public directory.
    pub default_index_files:            Vec<String>, // Must be filenames, not paths.
    // Server policy
    pub server_accept_unknown_users:    bool,
}

impl Config for ServerConfig {}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // TLS
            tls_dir_rel:                    fmt!("./tls"),
            tls_public_key_name:            fmt!("pubkey"),
            tls_private_key_name:           fmt!("privkey"),
            tls_cert_name:                  fmt!("fullchain"),
            tls_cert_address:               fmt!("0.0.0.0"),
            domain_names:                   vec![
                fmt!("localhost."),
            ],
            // Server
            log_level:                      fmt!("debug"),
            num_server_bots:                1,
            server_address:                 fmt!("0.0.0.0"),
            server_port_tcp:                8443,
            session_expiry_default_secs:    604_800, // 1 week.
            ws_ping_interval_secs:          30,
            server_max_errors_allowed:      30,
            // Paths
            public_dir_rel:                 fmt!("./www/public"),
            static_route_paths_rel:         mapdat!{
                "/" => "./www/public/",
                // User can add if they need:
                //"/admin" => "./www/public/admin.html",
            }.get_map().unwrap_or(DaticleMap::new()),
            default_index_files:            vec![
                fmt!("index.html"),
                fmt!("index.htm"),
                fmt!("default.html"),
                fmt!("home.html"),
            ],
            // Server policy.
            server_accept_unknown_users:    false,
        }
    }
}

impl ServerConfig {

    pub fn validate(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<()>
    {
        let path = Path::new(&self.public_dir_rel).normalise();
        if path.escapes() {
            return Err(err!(
                "ServerConfig: public directory {} escapes the directory {:?}.",
                self.public_dir_rel, root;
                Invalid, Input, Path));
        }
        res!(PathState::DirMustExist.validate(
            root,
            &self.public_dir_rel,
        ));

        let _ = res!(self.get_tls_paths(root, true));
        let _ = res!(self.get_tls_paths(root, false));
        let _ = res!(self.get_static_route_paths(root, ()));
        let _ = res!(self.get_default_index_files());
        let _ = res!(self.get_domain_names());

        Ok(())
    }

    /// Return the absolute path given a relative path to a World Wide Web resource.
    pub fn get_public_dir(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<PathBuf>
    {
        let path = Path::new(&self.public_dir_rel).normalise();
        if path.escapes() {
            return Err(err!(
                "ServerConfig: public directory {} escapes the directory {:?}.",
                self.public_dir_rel, root;
                Invalid, Input, Path));
        }
        let path = root.clone().join(path).normalise().absolute().as_pathbuf();
        res!(PathState::DirMustExist.validate(
            &path,
            "",
        ));
        Ok(path)
    }

    pub fn session_cookie_default(&self, sid: String) -> Cookie {
        let session_cookie_attrs = [
            SetCookieAttributes::HttpOnly,
            SetCookieAttributes::MaxAge(self.session_expiry_default_secs),
            SetCookieAttributes::Path("/".to_string()),
            SetCookieAttributes::SameSite(SameSite::Lax),
            SetCookieAttributes::Secure,
        ];
        let session_cookie_attrs =
            BTreeSet::from_iter(session_cookie_attrs.iter().cloned());
        Cookie {
            key: SESSION_ID_KEY_LABEL.to_string(),
            val: sid,
            attrs: Some(session_cookie_attrs),
        }
    }

    pub fn session_expiry(&self) -> Duration {
        Duration::from_secs(self.session_expiry_default_secs as u64)
    }

    pub fn log_level(&self) -> Outcome<LogLevel> {
        LogLevel::from_str(&self.log_level)
    }

    pub fn get_static_route_paths<M: MapMut<String, OsPath>>(
        &self,
        root:       &NormPathBuf,
        mut map:    M,
    )
        -> Outcome<M>
    {
        for (route_dat, path_dat) in &self.static_route_paths_rel {
            let route = try_extract_dat!(route_dat, Str).clone();
            if route.is_empty() {
                warn!("ServerConfig: Static route key is empty, skipping.");
                continue;
            }
            let path_str = try_extract_dat!(path_dat, Str);
            if path_str.is_empty() {
                warn!("ServerConfig: Static route '{}' path is empty, skipping.", route);
                continue;
            }
    
            let is_dir = path_str.ends_with("/");
            let path = Path::new(&path_str).normalise();
            if path.escapes() {
                warn!("ServerConfig: route '{}' target path '{}' escapes the directory \
                    {:?}, skipping.",
                    route, path_str, root);
                continue;
            }
    
            let path = root.clone().join(path).normalise().absolute();
            if is_dir {
                match PathState::DirMustExist.validate(&path, "") {
                    Ok(()) => {
                        map.insert(route, OsPath::Dir(path.as_pathbuf()));
                    }
                    Err(_) => {
                        warn!("ServerConfig: Directory '{}' for route '{}' not found, \
                            skipping.",
                            path_str, route);
                        continue;
                    }
                }
            } else {
                match PathState::FileMustExist.validate(&path, "") {
                    Ok(()) => {
                        map.insert(route, OsPath::File(path.as_pathbuf()));
                    }
                    Err(_) => {
                        warn!("ServerConfig: File '{}' for route '{}' not found, skipping. \
                            If this should be a directory, ensure it ends with '/'.",
                            path_str, route);
                        continue;
                    }
                }
            }
        }
        Ok(map)
    }

    pub fn get_default_index_files(&self) -> Outcome<Vec<String>> {
        if self.default_index_files.len() == 0 {
            warn!("ServerConfig: No default index files have been specified, using '{}'.",
                constant::DEFAULT_INDEX_FILE);
            return Ok(vec![fmt!("{}", constant::DEFAULT_INDEX_FILE)]);
        }
        let mut result = Vec::new();
        for filename in &self.default_index_files {
            if filename.is_empty() {
                return Err(err!(
                    "ServerConfig: Default index file entry is empty.";
                    Invalid, Input, Path));
            }
            if path::is_filename(filename) {
                result.push(filename.clone());
            } else {
                return Err(err!(
                    "ServerConfig: The default index file '{}' must be a standalone file \
                    and not a path.", filename;
                    Invalid, Input, String));
            }
        }
        Ok(result)
    }

    pub fn get_domain_names(&self) -> Outcome<Vec<Fqdn>> {
        if self.domain_names.len() == 0 {
            return Err(err!(
                "ServerConfig: There must be at least one entry in the domain_names field, \
                such as 'localhost'.";
                Invalid, Input, Missing));
        }
        let mut result = Vec::new();
        for domain_name in &self.domain_names {
            msg!("name='{}'",domain_name);
            if domain_name.is_empty() {
                return Err(err!(
                    "ServerConfig: Domain name entry is empty.";
                    Invalid, Input, Path));
            }
            let fqdn = match Fqdn::new(domain_name) {
                Ok(fqdn) => fqdn,
                Err(e) => return Err(err!(e,
                    "While trying to validate domain name '{}'.", domain_name;
                Network)),
            };
            result.push(fqdn);
        }
        Ok(result)
    }

    pub fn get_tls_paths(
        &self,
        root:       &NormPathBuf,
        dev_mode:   bool,
    )
        -> Outcome<(PathBuf, PathBuf, PathBuf)>
    {
        let tls_dir_str = &self.tls_dir_rel;
        if tls_dir_str.is_empty() {
            return Err(err!(
                "ServerConfig: TLS directory is empty.";
                Invalid, Input, Missing));
        }
        let tls_dir = Path::new(tls_dir_str).normalise();
        if tls_dir.escapes() {
            return Err(err!(
                "ServerConfig: TLS directory {} escapes the directory {:?}.",
                tls_dir_str, root;
                Invalid, Input, Path));
        }
        let tls_dir = root.clone().join(tls_dir).normalise().absolute().as_pathbuf();
        let tls_dir = if dev_mode {
            res!(PathState::Create.validate(
                &tls_dir,
                constant::TLS_DIR_DEV,
            ));
            tls_dir.join(constant::TLS_DIR_DEV)
        } else {
            res!(PathState::Create.validate(
                &tls_dir,
                constant::TLS_DIR_PROD,
            ));
            tls_dir.join(constant::TLS_DIR_PROD)
        };
        if self.tls_cert_name.is_empty() {
            return Err(err!(
                "ServerConfig: TLS certificate name is empty.";
                Invalid, Input, Missing));
        }
        if self.tls_private_key_name.is_empty() {
            return Err(err!(
                "ServerConfig: TLS private_key_name is empty.";
                Invalid, Input, Missing));
        }
        let cert_path = tls_dir.join(&self.tls_cert_name).with_extension("pem");
        let key_path = tls_dir.join(&self.tls_private_key_name).with_extension("pem");
        Ok((tls_dir, cert_path, key_path))
    }
}
