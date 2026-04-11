use crate::srv::constant;

use oxedyne_fe2o3_core::{
    prelude::*,
    file::{
        OsPath,
        PathState,
    },
    map::MapMut,
    path::{
        NormalPath,
        NormPathBuf,
    },
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};
use oxedyne_fe2o3_net::{
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REDIRECT RULES                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// How a redirect rule's `match_path` is tested against an incoming request path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RedirectMatch {
    /// Exact path match, e.g. `/example`.
    Exact,
    /// Path prefix match; matches any request whose path starts with `match_path`.
    Prefix,
    /// Matches any path on the vhost, typically used for "www → canonical" redirects.
    All,
}

impl RedirectMatch {
    /// Parse a redirect match kind from its string form.
    pub fn from_str(s: &str) -> Outcome<Self> {
        match s {
            "exact"     => Ok(Self::Exact),
            "prefix"    => Ok(Self::Prefix),
            "all"       => Ok(Self::All),
            _ => Err(err!(
                "Unknown redirect match kind '{}'. Valid values are: exact, prefix, all.", s;
                Invalid, Input, String)),
        }
    }
}

/// A single redirect rule applied by a vhost before static file resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedirectRule {
    /// How `match_path` is matched against the incoming URL path.
    pub match_kind: RedirectMatch,
    /// The path pattern to match. Ignored when `match_kind` is `All`.
    pub match_path: String,
    /// Target URL to redirect to. May contain the literal string `{uri}`, which
    /// is replaced by the matched request path + query string at redirect time.
    pub target:     String,
    /// HTTP status code, normally `301` for permanent or `302` for temporary.
    pub status:     u16,
}

impl RedirectRule {
    /// Resolve the target URL for a given incoming request path, expanding the
    /// `{uri}` placeholder if present.
    pub fn resolve_target(&self, request_uri: &str) -> String {
        if self.target.contains("{uri}") {
            self.target.replace("{uri}", request_uri)
        } else {
            self.target.clone()
        }
    }

    /// Returns `true` if this rule matches the given request path.
    pub fn matches(&self, request_path: &str) -> bool {
        match self.match_kind {
            RedirectMatch::Exact    => request_path == self.match_path,
            RedirectMatch::Prefix   => request_path.starts_with(&self.match_path),
            RedirectMatch::All      => true,
        }
    }

    /// Parse a redirect rule from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let match_kind_str = match m.get(&dat!("match_kind")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => fmt!("all"),
        };
        let match_kind = res!(RedirectMatch::from_str(&match_kind_str));
        let match_path = match m.get(&dat!("match_path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let target = match m.get(&dat!("target")) {
            Some(Dat::Str(s)) => s.clone(),
            None => return Err(err!(
                "RedirectRule: missing 'target' field.";
                Invalid, Input, Missing)),
            _ => return Err(err!(
                "RedirectRule: 'target' field must be a string.";
                Invalid, Input, Mismatch)),
        };
        let status = match m.get(&dat!("status")) {
            Some(Dat::U16(n)) => *n,
            Some(Dat::U32(n)) => *n as u16,
            Some(Dat::U64(n)) => *n as u16,
            _ => 301,
        };
        Ok(Self {
            match_kind,
            match_path,
            target,
            status,
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VHOST CONFIG                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Configuration for a single virtual host served by Steel.
///
/// A vhost is selected at TLS handshake time by its SNI hostname, and may carry
/// its own webroot, static routes, default index files, and redirect rules.
/// Multiple hostnames (e.g. `example.com` and a trailing-dot alias) are supported
/// by listing them all in `hostnames`; the first entry is the primary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VhostConfig {
    /// Hostnames answered by this vhost. The first entry is the canonical name.
    pub hostnames:              Vec<String>,
    /// Webroot directory for static file serving, relative to the app root.
    /// `None` for pure-redirect vhosts that never serve a file.
    pub public_dir_rel:         Option<String>,
    /// Named route overrides mapping URL paths to files or directories.
    pub static_route_paths_rel: DaticleMap,
    /// Default index files, tried in order when a directory is requested.
    pub default_index_files:    Vec<String>,
    /// Ordered list of redirect rules evaluated before static file resolution.
    pub redirects:              Vec<RedirectRule>,
}

impl Default for VhostConfig {
    fn default() -> Self {
        Self {
            hostnames:              vec![fmt!("localhost")],
            public_dir_rel:         Some(fmt!("./www/public")),
            static_route_paths_rel: DaticleMap::new(),
            default_index_files:    vec![
                fmt!("index.html"),
                fmt!("index.htm"),
                fmt!("default.html"),
                fmt!("home.html"),
            ],
            redirects:              Vec::new(),
        }
    }
}

impl VhostConfig {
    /// Primary (canonical) hostname.
    pub fn primary_hostname(&self) -> &str {
        self.hostnames.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Parse a vhost configuration from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        // Hostnames.
        let hostnames = match m.get(&dat!("hostnames")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'hostnames' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            Some(Dat::Vek(vek)) => {
                let mut out = Vec::new();
                for item in vek.iter() {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'hostnames' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => return Err(err!(
                "VhostConfig: 'hostnames' field is required.";
                Invalid, Input, Missing)),
            _ => return Err(err!(
                "VhostConfig: 'hostnames' must be a list of strings.";
                Invalid, Input, Mismatch)),
        };
        if hostnames.is_empty() {
            return Err(err!(
                "VhostConfig: 'hostnames' must contain at least one entry.";
                Invalid, Input, Missing));
        }
        // Public dir (optional).
        let public_dir_rel = match m.get(&dat!("public_dir_rel")) {
            Some(Dat::Str(s)) if s.is_empty() => None,
            Some(Dat::Str(s)) => Some(s.clone()),
            Some(Dat::Opt(opt)) => match opt.as_ref() {
                Some(Dat::Str(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        // Static routes.
        let static_route_paths_rel = match m.get(&dat!("static_route_paths_rel")) {
            Some(Dat::Map(sub)) => sub.clone(),
            None => DaticleMap::new(),
            _ => return Err(err!(
                "VhostConfig: 'static_route_paths_rel' must be a map.";
                Invalid, Input, Mismatch)),
        };
        // Default index files.
        let default_index_files = match m.get(&dat!("default_index_files")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'default_index_files' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            Some(Dat::Vek(vek)) => {
                let mut out = Vec::new();
                for item in vek.iter() {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'default_index_files' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => vec![
                fmt!("index.html"),
                fmt!("index.htm"),
            ],
            _ => return Err(err!(
                "VhostConfig: 'default_index_files' must be a list of strings.";
                Invalid, Input, Mismatch)),
        };
        // Redirect rules.
        let redirects = match m.get(&dat!("redirects")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Map(sub) => out.push(res!(RedirectRule::from_datmap(sub))),
                        _ => return Err(err!(
                            "VhostConfig: 'redirects' entries must be maps.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'redirects' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        Ok(Self {
            hostnames,
            public_dir_rel,
            static_route_paths_rel,
            default_index_files,
            redirects,
        })
    }

    /// Resolve the vhost's webroot to an absolute validated path, returning
    /// `None` for pure-redirect vhosts that have no webroot.
    pub fn get_public_dir(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<Option<PathBuf>>
    {
        let rel = match &self.public_dir_rel {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(None),
        };
        let path = Path::new(rel).normalise();
        if path.escapes() {
            return Err(err!(
                "VhostConfig: public directory {} escapes the directory {:?}.",
                rel, root;
                Invalid, Input, Path));
        }
        let path = root.clone().join(path).normalise().absolute().as_pathbuf();
        res!(PathState::DirMustExist.validate(
            &path,
            "",
        ));
        Ok(Some(path))
    }

    /// Validate and materialise the vhost's static route map.
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
                warn!("VhostConfig: Static route key is empty, skipping.");
                continue;
            }
            let path_str = try_extract_dat!(path_dat, Str);
            if path_str.is_empty() {
                warn!("VhostConfig: Static route '{}' path is empty, skipping.", route);
                continue;
            }
            let is_dir = path_str.ends_with("/");
            let path = Path::new(&path_str).normalise();
            if path.escapes() {
                warn!("VhostConfig: route '{}' target path '{}' escapes the directory \
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
                        warn!("VhostConfig: Directory '{}' for route '{}' not found, \
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
                        warn!("VhostConfig: File '{}' for route '{}' not found, skipping.",
                            path_str, route);
                        continue;
                    }
                }
            }
        }
        Ok(map)
    }

    /// Validate the `default_index_files` list.
    pub fn get_default_index_files(&self) -> Outcome<Vec<String>> {
        if self.default_index_files.is_empty() {
            warn!("VhostConfig: No default index files specified, using '{}'.",
                constant::DEFAULT_INDEX_FILE);
            return Ok(vec![fmt!("{}", constant::DEFAULT_INDEX_FILE)]);
        }
        let mut out = Vec::new();
        for filename in &self.default_index_files {
            if filename.is_empty() {
                return Err(err!(
                    "VhostConfig: Default index file entry is empty.";
                    Invalid, Input, Path));
            }
            if oxedyne_fe2o3_core::path::is_filename(filename) {
                out.push(filename.clone());
            } else {
                return Err(err!(
                    "VhostConfig: Default index file '{}' must be a filename, not a path.",
                    filename;
                    Invalid, Input, String));
            }
        }
        Ok(out)
    }

    /// Validate all `hostnames` as FQDNs.
    pub fn get_hostnames_fqdn(&self) -> Outcome<Vec<Fqdn>> {
        let mut out = Vec::new();
        for name in &self.hostnames {
            if name.is_empty() {
                return Err(err!(
                    "VhostConfig: hostname entry is empty.";
                    Invalid, Input, Missing));
            }
            let fqdn = match Fqdn::new(name) {
                Ok(fqdn) => fqdn,
                Err(e) => return Err(err!(e,
                    "While validating vhost hostname '{}'.", name;
                    Network)),
            };
            out.push(fqdn);
        }
        Ok(out)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACME CONFIG                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Configuration for Steel's built-in ACME (Let's Encrypt) client.
///
/// When `enabled` is `true`, Steel will request and automatically renew TLS
/// certificates for every configured vhost hostname via the TLS-ALPN-01
/// challenge on the same port Steel is already listening on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcmeConfig {
    /// Master switch for ACME. When `false`, certificates are loaded from disk.
    pub enabled:        bool,
    /// Contact email registered with the ACME account (e.g. for expiry notices).
    pub contact_email:  String,
    /// ACME directory URL. Defaults to the Let's Encrypt staging endpoint to
    /// prevent accidental rate-limit burns during development.
    pub directory_url:  String,
    /// Directory where the account key and issued certificates are persisted,
    /// relative to the app root.
    pub cache_dir_rel:  String,
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            enabled:        false,
            contact_email:  fmt!(""),
            // Staging by default, deliberately. Switch to production once
            // everything works end to end on staging.
            directory_url:  fmt!("https://acme-staging-v02.api.letsencrypt.org/directory"),
            cache_dir_rel:  fmt!("./tls/acme"),
        }
    }
}

impl AcmeConfig {
    /// Parse an ACME configuration from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let mut out = Self::default();
        if let Some(Dat::Bool(b)) = m.get(&dat!("enabled")) {
            out.enabled = *b;
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("contact_email")) {
            out.contact_email = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("directory_url")) {
            out.directory_url = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("cache_dir_rel")) {
            out.cache_dir_rel = s.clone();
        }
        Ok(out)
    }

    /// Convert the config to its `DaticleMap` representation.
    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("enabled"),       dat!(self.enabled));
        m.insert(dat!("contact_email"), dat!(self.contact_email.clone()));
        m.insert(dat!("directory_url"), dat!(self.directory_url.clone()));
        m.insert(dat!("cache_dir_rel"), dat!(self.cache_dir_rel.clone()));
        m
    }

    /// Resolve the ACME cache directory to an absolute validated path.
    pub fn get_cache_dir(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<PathBuf>
    {
        let path = Path::new(&self.cache_dir_rel).normalise();
        if path.escapes() {
            return Err(err!(
                "AcmeConfig: cache directory {} escapes the directory {:?}.",
                self.cache_dir_rel, root;
                Invalid, Input, Path));
        }
        let path = root.clone().join(path).normalise().absolute().as_pathbuf();
        res!(std::fs::create_dir_all(&path));
        Ok(path)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SERVER CONFIG                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Top-level server configuration. Fields here are shared across all vhosts;
/// per-site settings live on `VhostConfig` entries inside `vhosts`.
#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct ServerConfig {
    // --- TLS fallback (only used when acme.enabled = false) -----------------
    /// Directory holding per-vhost certificates when ACME is disabled,
    /// relative to the app root. Each vhost's certs live in the subdirectory
    /// `{tls_dir_rel}/{dev|prod}/{primary_hostname}/fullchain.pem` and
    /// `privkey.pem`.
    pub tls_dir_rel:                    String,

    // --- Server bind and policy (shared) ------------------------------------
    /// Logging level used by the server once running.
    pub log_level:                      String,
    /// Number of server bot workers.
    pub num_server_bots:                u16,
    /// IP address to bind to, typically `"0.0.0.0"`.
    pub server_address:                 String,
    /// TCP port to listen on.
    pub server_port_tcp:                u16,
    /// Default session lifetime in seconds.
    pub session_expiry_default_secs:    u32,
    /// WebSocket ping interval in seconds.
    pub ws_ping_interval_secs:          u8,
    /// Maximum consecutive errors allowed on a single connection.
    pub server_max_errors_allowed:      u8,
    /// Whether to accept users not known to the wallet.
    pub server_accept_unknown_users:    bool,

    // --- Virtual hosts ------------------------------------------------------
    /// Ordered list of virtual host configurations, stored as a `Dat::List`
    /// of `Dat::Map` entries and parsed via `get_vhosts()`.
    pub vhosts:                         Dat,

    // --- ACME ---------------------------------------------------------------
    /// ACME client configuration (as a daticle map, parsed via `get_acme()`).
    pub acme:                           DaticleMap,
}

impl Config for ServerConfig {}

impl Default for ServerConfig {
    fn default() -> Self {
        // Build a default single-vhost setup.
        let default_vhost = VhostConfig::default();
        let mut vhost_map = DaticleMap::new();
        let hostnames_list: Vec<Dat> = default_vhost
            .hostnames
            .iter()
            .map(|s| dat!(s.clone()))
            .collect();
        vhost_map.insert(dat!("hostnames"), Dat::List(hostnames_list));
        if let Some(ref p) = default_vhost.public_dir_rel {
            vhost_map.insert(dat!("public_dir_rel"), dat!(p.clone()));
        }
        let mut routes = DaticleMap::new();
        routes.insert(dat!("/"), dat!("./www/public/"));
        vhost_map.insert(dat!("static_route_paths_rel"), Dat::Map(routes));
        let idx_list: Vec<Dat> = default_vhost
            .default_index_files
            .iter()
            .map(|s| dat!(s.clone()))
            .collect();
        vhost_map.insert(dat!("default_index_files"), Dat::List(idx_list));
        vhost_map.insert(dat!("redirects"), Dat::List(Vec::new()));

        Self {
            tls_dir_rel:                    fmt!("./tls"),
            log_level:                      fmt!("debug"),
            num_server_bots:                1,
            server_address:                 fmt!("0.0.0.0"),
            server_port_tcp:                8443,
            session_expiry_default_secs:    604_800, // 1 week.
            ws_ping_interval_secs:          30,
            server_max_errors_allowed:      30,
            server_accept_unknown_users:    false,
            vhosts:                         Dat::List(vec![Dat::Map(vhost_map)]),
            acme:                           AcmeConfig::default().to_datmap(),
        }
    }
}

impl ServerConfig {

    /// Validate the whole server configuration: each vhost's webroot, static
    /// routes, default index files and hostnames, plus the ACME cache path.
    pub fn validate(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<()>
    {
        let vhosts = res!(self.get_vhosts());
        if vhosts.is_empty() {
            return Err(err!(
                "ServerConfig: at least one vhost must be defined.";
                Invalid, Input, Missing));
        }
        for vh in &vhosts {
            let _ = res!(vh.get_public_dir(root));
            let _ = res!(vh.get_static_route_paths(root, ()));
            let _ = res!(vh.get_default_index_files());
            let _ = res!(vh.get_hostnames_fqdn());
        }
        let _ = res!(self.get_acme());
        Ok(())
    }

    /// Parse and return all configured vhosts.
    pub fn get_vhosts(&self) -> Outcome<Vec<VhostConfig>> {
        let list = match &self.vhosts {
            Dat::List(items) => items,
            _ => return Err(err!(
                "ServerConfig: 'vhosts' must be a list of vhost maps.";
                Invalid, Input, Mismatch)),
        };
        let mut out = Vec::new();
        for (i, vh_dat) in list.iter().enumerate() {
            let vh_map = match vh_dat {
                Dat::Map(m) => m,
                _ => return Err(err!(
                    "ServerConfig: vhost entry {} is not a map.", i;
                    Invalid, Input, Mismatch)),
            };
            out.push(res!(VhostConfig::from_datmap(vh_map)));
        }
        Ok(out)
    }

    /// Parse and return the ACME configuration.
    pub fn get_acme(&self) -> Outcome<AcmeConfig> {
        AcmeConfig::from_datmap(&self.acme)
    }

    /// Build a default session cookie for the given session id string.
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

    /// Session lifetime as a `Duration`.
    pub fn session_expiry(&self) -> Duration {
        Duration::from_secs(self.session_expiry_default_secs as u64)
    }

    /// Parse `log_level` into a `LogLevel` enum.
    pub fn log_level(&self) -> Outcome<LogLevel> {
        LogLevel::from_str(&self.log_level)
    }

    /// Resolve the TLS directory for a given mode (dev or prod) to an absolute
    /// validated path. Used only when ACME is disabled.
    pub fn get_tls_dir(
        &self,
        root:       &NormPathBuf,
        dev_mode:   bool,
    )
        -> Outcome<PathBuf>
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
        Ok(tls_dir)
    }
}
