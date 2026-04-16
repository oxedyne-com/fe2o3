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
    /// Exact path match, e.g. `/admin`.
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
// │ API ROUTES                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// An outbound API proxy route.
///
/// Maps a local POST path to an upstream HTTPS URL. Steel forwards the
/// request body verbatim and injects the configured headers (typically
/// containing secret credentials loaded from files at startup).
///
/// As an alternative to a remote upstream, a route may name an
/// in-process `handler` registered by an `AppExtension`. In that case
/// Steel dispatches the request to the registered `ApiHandler`
/// instead of proxying. The two modes are mutually exclusive: a
/// route either has `upstream*` set or `handler` set, never both.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiRoute {
    /// Local path to match (e.g. `/api/payments/checkout`).
    pub path:           String,
    /// Upstream hostname (e.g. `api.example.com`). `None` when the
    /// route is served by an in-process handler.
    pub upstream_host:  Option<String>,
    /// Upstream port (defaults to 443). `None` when handler-served.
    pub upstream_port:  Option<u16>,
    /// Upstream request path (e.g. `/v1/checkout/sessions`). `None`
    /// when handler-served.
    pub upstream_path:  Option<String>,
    /// Headers injected into the upstream request. Values have already been
    /// resolved (any `{file:...}` references expanded at config load time).
    pub headers:        Vec<(String, String)>,
    /// Name of an in-process API handler registered via `AppExtension`.
    /// `None` when the route is a proxy.
    pub handler:        Option<String>,
    /// Handler-specific configuration key-value pairs. Values support
    /// `{file:}` and `{env:}` placeholders, resolved at startup.
    /// Empty when the route is a proxy.
    pub config:         Vec<(String, String)>,
}

impl ApiRoute {
    /// Parse an API route from a `DaticleMap`.
    ///
    /// Header values are stored as-is and may contain `{file:path}`
    /// placeholders. Call `resolve_headers` with the app root to expand
    /// them before use.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        // Path (required).
        let path = match m.get(&dat!("path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "ApiRoute: 'path' field is required and must be a string.";
                Invalid, Input, Missing)),
        };
        // Either `upstream` (proxy) or `handler` (in-process). Not both.
        let upstream_str = match m.get(&dat!("upstream")) {
            Some(Dat::Str(s)) => Some(s.clone()),
            None              => None,
            _ => return Err(err!(
                "ApiRoute '{}': 'upstream' must be a string when present.", path;
                Invalid, Input, Mismatch)),
        };
        let handler = match m.get(&dat!("handler")) {
            Some(Dat::Str(s)) => Some(s.clone()),
            None              => None,
            _ => return Err(err!(
                "ApiRoute '{}': 'handler' must be a string when present.", path;
                Invalid, Input, Mismatch)),
        };
        match (&upstream_str, &handler) {
            (None, None) => return Err(err!(
                "ApiRoute '{}': must specify either 'upstream' (for proxy \
                routes) or 'handler' (for in-process routes).", path;
                Invalid, Input, Missing)),
            (Some(_), Some(_)) => return Err(err!(
                "ApiRoute '{}': 'upstream' and 'handler' are mutually \
                exclusive. A route is either a proxy or in-process, not \
                both.", path;
                Invalid, Input, Conflict)),
            _ => {}
        }
        // Parse upstream URL into host, port, path (proxy mode only).
        let (upstream_host, upstream_port, upstream_path) = match upstream_str {
            Some(url) => {
                let (h, p, up) = res!(Self::parse_upstream(&url));
                (Some(h), Some(p), Some(up))
            }
            None => (None, None, None),
        };
        // Headers (optional map of name -> value). Used in proxy mode for
        // headers injected into the upstream request; empty for handler mode.
        let headers = match m.get(&dat!("headers")) {
            Some(Dat::Map(sub)) => {
                let mut out = Vec::new();
                for (k, v) in sub.iter() {
                    let name = match k {
                        Dat::Str(s) => s.clone(),
                        _ => return Err(err!(
                            "ApiRoute '{}': header names must be strings.", path;
                            Invalid, Input, Mismatch)),
                    };
                    let raw_val = match v {
                        Dat::Str(s) => s.clone(),
                        _ => return Err(err!(
                            "ApiRoute '{}': header values must be strings.", path;
                            Invalid, Input, Mismatch)),
                    };
                    out.push((name, raw_val));
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "ApiRoute '{}': 'headers' must be a map.", path;
                Invalid, Input, Mismatch)),
        };
        // Handler-specific config (optional map). Only used in handler mode.
        let config = match m.get(&dat!("config")) {
            Some(Dat::Map(sub)) => {
                let mut out = Vec::new();
                for (k, v) in sub.iter() {
                    let name = match k {
                        Dat::Str(s) => s.clone(),
                        _ => continue,
                    };
                    let val = match v {
                        Dat::Str(s) => s.clone(),
                        _ => continue,
                    };
                    out.push((name, val));
                }
                out
            }
            None => Vec::new(),
            _ => Vec::new(),
        };
        Ok(Self {
            path,
            upstream_host,
            upstream_port,
            upstream_path,
            headers,
            handler,
            config,
        })
    }

    /// Look up a handler-config value by key.
    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.config.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Parse an `https://host[:port]/path` URL into components.
    fn parse_upstream(url: &str) -> Outcome<(String, u16, String)> {
        let rest = match url.strip_prefix("https://") {
            Some(r) => r,
            None => return Err(err!(
                "ApiRoute: upstream URL must start with 'https://'. Got: '{}'.", url;
                Invalid, Input)),
        };
        let (host_port, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None    => (rest, "/"),
        };
        let (host, port) = match host_port.rfind(':') {
            Some(i) => {
                let p: u16 = match host_port[i + 1..].parse() {
                    Ok(n)  => n,
                    Err(_) => return Err(err!(
                        "ApiRoute: invalid port in upstream URL '{}'.", url;
                        Invalid, Input)),
                };
                (host_port[..i].to_string(), p)
            }
            None => (host_port.to_string(), 443),
        };
        Ok((host, port, path.to_string()))
    }

    /// Expand `{file:path}` and `{env:}` placeholders in all header
    /// values and handler-config values by reading the referenced
    /// files relative to `root`. Must be called once at startup
    /// before the route is dispatched.
    pub fn resolve_headers(&mut self, root: &Path) -> Outcome<()> {
        for (_name, value) in &mut self.headers {
            *value = res!(Self::resolve_file_refs(value, root));
        }
        for (_name, value) in &mut self.config {
            *value = res!(Self::resolve_file_refs(value, root));
        }
        Ok(())
    }

    /// Resolve `{file:path}` and `{env:VAR}` or `{env:VAR:default}`
    /// placeholders in a config value.
    ///
    /// * `{file:path}` — replaced with the trimmed contents of the file,
    ///   resolved relative to `root`. Fails if the file cannot be read.
    /// * `{env:VAR}` — replaced with the value of environment variable
    ///   `VAR`. Fails if the variable is unset.
    /// * `{env:VAR:default}` — replaced with the env var value, or
    ///   `default` if the variable is unset or empty.
    ///
    /// Env placeholders are resolved first so they may appear inside
    /// `{file:...}` paths to parameterise file locations.
    pub fn resolve_file_refs(value: &str, root: &Path) -> Outcome<String> {
        // Pass 1: resolve all {env:} placeholders so env values can appear
        // inside {file:} paths.
        let intermediate = res!(Self::resolve_env_refs(value));
        // Pass 2: resolve all {file:} placeholders.
        Self::resolve_file_only(&intermediate, root)
    }

    /// Resolve only `{env:VAR[:default]}` placeholders.
    fn resolve_env_refs(value: &str) -> Outcome<String> {
        let mut result = value.to_string();
        while let Some(start) = result.find("{env:") {
            let end = match result[start..].find('}') {
                Some(i) => start + i,
                None => return Err(err!(
                    "Config: unclosed '{{env:' placeholder in value '{}'.", value;
                    Invalid, Input)),
            };
            let inner = result[start + 5..end].to_string();
            let (var_name, default) = match inner.find(':') {
                Some(i) => (&inner[..i], Some(&inner[i + 1..])),
                None    => (inner.as_str(), None),
            };
            let replacement = match std::env::var(var_name) {
                Ok(v) if !v.is_empty() => v,
                _ => match default {
                    Some(d) => d.to_string(),
                    None => return Err(err!(
                        "Config: environment variable '{}' is not set \
                        and '{{env:{}}}' has no default.",
                        var_name, inner;
                        Invalid, Input, Missing)),
                },
            };
            result.replace_range(start..=end, &replacement);
        }
        Ok(result)
    }

    /// Resolve only `{file:path}` placeholders.
    fn resolve_file_only(value: &str, root: &Path) -> Outcome<String> {
        let mut result = value.to_string();
        while let Some(start) = result.find("{file:") {
            let end = match result[start..].find('}') {
                Some(i) => start + i,
                None => return Err(err!(
                    "Config: unclosed '{{file:' placeholder in value '{}'.", value;
                    Invalid, Input)),
            };
            let rel_path = result[start + 6..end].to_string();
            let abs_path = root.join(&rel_path);
            let content = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s.trim().to_string(),
                Err(e) => return Err(err!(e,
                    "Config: failed to read '{{file:{}}}' at '{:?}'.",
                    rel_path, abs_path;
                    IO, File, Read)),
            };
            result.replace_range(start..=end, &content);
        }
        Ok(result)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WEBHOOK ROUTES                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// An incoming webhook route with a named handler.
///
/// When Steel receives a POST at the configured `path`, it dispatches to
/// the handler identified by `handler`. The `config` map carries handler-
/// specific settings (API keys, upstream URLs, identifiers, etc.) whose
/// values support the same `{file:path}` secret placeholder syntax as
/// API routes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebhookRoute {
    /// Local path to match (e.g. `/webhook/payments`).
    pub path:       String,
    /// Handler name (e.g. `payments_forwarder`).
    pub handler:    String,
    /// Handler-specific configuration.
    pub config:     Vec<(String, String)>,
}

impl WebhookRoute {
    /// Parse a webhook route from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let path = match m.get(&dat!("path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "WebhookRoute: 'path' is required and must be a string.";
                Invalid, Input, Missing)),
        };
        let handler = match m.get(&dat!("handler")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "WebhookRoute: 'handler' is required and must be a string.";
                Invalid, Input, Missing)),
        };
        let config = match m.get(&dat!("config")) {
            Some(Dat::Map(sub)) => {
                let mut out = Vec::new();
                for (k, v) in sub.iter() {
                    let name = match k {
                        Dat::Str(s) => s.clone(),
                        _ => continue,
                    };
                    let val = match v {
                        Dat::Str(s) => s.clone(),
                        _ => continue,
                    };
                    out.push((name, val));
                }
                out
            }
            None => Vec::new(),
            _ => Vec::new(),
        };
        Ok(Self { path, handler, config })
    }

    /// Expand `{file:path}` placeholders in all config values.
    pub fn resolve_config(&mut self, root: &Path) -> Outcome<()> {
        for (_name, value) in &mut self.config {
            *value = res!(ApiRoute::resolve_file_refs(value, root));
        }
        Ok(())
    }

    /// Look up a config value by key.
    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.config.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VHOST CONFIG                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Configuration for a single virtual host served by Steel.
///
/// A vhost is selected at TLS handshake time by its SNI hostname, and may carry
/// its own webroot, static routes, default index files, redirect rules and
/// Ozone database. Multiple hostnames (e.g. `example.com` and a trailing-dot
/// alias) are supported by listing them all in `hostnames`; the first entry
/// is the primary.
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
    /// Database directory for this vhost's Ozone instance, relative to the
    /// app root. `None` means the vhost has no backing database (typical for
    /// pure-redirect vhosts). When set, Steel opens and starts a dedicated
    /// Ozone instance rooted here at server start-up.
    pub db_dir_rel:             Option<String>,
    /// Outbound API proxy routes. Each route maps a local POST path to an
    /// upstream HTTPS URL with injected headers (typically secret credentials).
    pub api_routes:             Vec<ApiRoute>,
    /// Incoming webhook routes. Each route maps a local POST path to a
    /// named handler with handler-specific configuration.
    pub webhook_routes:         Vec<WebhookRoute>,
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
            db_dir_rel:             Some(fmt!("./o3db")),
            api_routes:             Vec::new(),
            webhook_routes:         Vec::new(),
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
        // Database directory (optional).
        let db_dir_rel = match m.get(&dat!("db_dir_rel")) {
            Some(Dat::Str(s)) if s.is_empty() => None,
            Some(Dat::Str(s)) => Some(s.clone()),
            Some(Dat::Opt(opt)) => match opt.as_ref() {
                Some(Dat::Str(s)) => Some(s.clone()),
                _ => None,
            },
            None => None,
            _ => return Err(err!(
                "VhostConfig: 'db_dir_rel' must be a string.";
                Invalid, Input, Mismatch)),
        };
        // API proxy routes (optional).
        let api_routes = match m.get(&dat!("api_routes")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Map(sub) => out.push(res!(ApiRoute::from_datmap(sub))),
                        _ => return Err(err!(
                            "VhostConfig: 'api_routes' entries must be maps.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'api_routes' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        // Webhook routes (optional).
        let webhook_routes = match m.get(&dat!("webhook_routes")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Map(sub) => out.push(res!(WebhookRoute::from_datmap(sub))),
                        _ => return Err(err!(
                            "VhostConfig: 'webhook_routes' entries must be maps.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'webhook_routes' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        Ok(Self {
            hostnames,
            public_dir_rel,
            static_route_paths_rel,
            default_index_files,
            redirects,
            db_dir_rel,
            api_routes,
            webhook_routes,
        })
    }

    /// Resolve the vhost's database directory to an absolute path, creating
    /// it if it does not yet exist. Returns `None` when the vhost has no
    /// configured database. Unlike `get_public_dir`, this tolerates a missing
    /// directory and creates it: Ozone expects a writable root and will
    /// populate it on first start-up.
    /// Supports both relative (anchored at `root`) and absolute paths.
    pub fn get_db_dir(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<Option<PathBuf>>
    {
        let rel = match &self.db_dir_rel {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(None),
        };
        let path = if Path::new(rel).is_absolute() {
            PathBuf::from(rel)
        } else {
            let norm = Path::new(rel).normalise();
            if norm.escapes() {
                return Err(err!(
                    "VhostConfig: database directory {} escapes the directory {:?}.",
                    rel, root;
                    Invalid, Input, Path));
            }
            root.clone().join(norm).normalise().absolute().as_pathbuf()
        };
        res!(std::fs::create_dir_all(&path));
        Ok(Some(path))
    }

    /// Resolve the vhost's webroot to an absolute validated path, returning
    /// `None` for pure-redirect vhosts that have no webroot. Supports both
    /// relative paths (anchored at `root`) and absolute paths (used as-is).
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
        let path = if Path::new(rel).is_absolute() {
            PathBuf::from(rel)
        } else {
            let norm = Path::new(rel).normalise();
            if norm.escapes() {
                return Err(err!(
                    "VhostConfig: public directory {} escapes the directory {:?}.",
                    rel, root;
                    Invalid, Input, Path));
            }
            root.clone().join(norm).normalise().absolute().as_pathbuf()
        };
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
// │ MAIL CONFIG                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Hematite mail listener configuration.
///
/// When present (and `enabled = true`), Steel binds three TCP ports
/// alongside the HTTPS listener: SMTP receive, SMTP submission, and
/// IMAP. All three share the rustls cert resolver Steel uses for
/// HTTPS so a single ACME-issued cert covers every protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailConfig {
    /// Master switch. When `false`, the mail server is not started.
    pub enabled:            bool,
    /// Hostname the SMTP and IMAP servers advertise in their
    /// greetings. Should be the public MX hostname.
    pub hostname:           String,
    /// MX-receive port. Standard 25.
    pub smtp_port:          u16,
    /// Submission port. Standard 587.
    pub submission_port:    u16,
    /// Implicit-TLS IMAP port. Standard 993.
    pub imap_port:          u16,
    /// Maildir storage root. Per-user trees live underneath as
    /// `<root>/<delivery_dir>/`.
    pub maildir_root:       String,
    /// Path to the JDAT user file (passwords + delivery dirs).
    pub users_file_rel:     String,
    /// Path to the outbound spool directory.
    pub spool_dir_rel:      String,
    /// Path to the DKIM private key file (PKCS#8 DER form). Empty
    /// disables DKIM signing.
    pub dkim_key_file:      String,
    /// DKIM selector to publish under
    /// `<selector>._domainkey.<dkim_domain>`.
    pub dkim_selector:      String,
    /// Domain to sign for. May differ from `hostname` if mail is
    /// sent on behalf of a user-facing domain via a separate MX.
    pub dkim_domain:        String,
    /// Domains the receive path will accept mail for. Recipients
    /// outside this set are rejected at `RCPT TO` time.
    pub local_domains:      Vec<String>,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self {
            enabled:            false,
            hostname:           String::new(),
            smtp_port:          25,
            submission_port:    587,
            imap_port:          993,
            maildir_root:       String::new(),
            users_file_rel:     String::new(),
            spool_dir_rel:      String::new(),
            dkim_key_file:      String::new(),
            dkim_selector:      String::new(),
            dkim_domain:        String::new(),
            local_domains:      Vec::new(),
        }
    }
}

impl MailConfig {
    /// Parse a `MailConfig` from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let mut out = Self::default();
        if let Some(Dat::Bool(b)) = m.get(&dat!("enabled")) {
            out.enabled = *b;
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("hostname")) {
            out.hostname = s.clone();
        }
        if let Some(Dat::U16(n)) = m.get(&dat!("smtp_port")) {
            out.smtp_port = *n;
        }
        if let Some(Dat::U16(n)) = m.get(&dat!("submission_port")) {
            out.submission_port = *n;
        }
        if let Some(Dat::U16(n)) = m.get(&dat!("imap_port")) {
            out.imap_port = *n;
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("maildir_root")) {
            out.maildir_root = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("users_file_rel")) {
            out.users_file_rel = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("spool_dir_rel")) {
            out.spool_dir_rel = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("dkim_key_file")) {
            out.dkim_key_file = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("dkim_selector")) {
            out.dkim_selector = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("dkim_domain")) {
            out.dkim_domain = s.clone();
        }
        match m.get(&dat!("local_domains")) {
            Some(Dat::List(l)) => {
                for d in l {
                    if let Dat::Str(s) = d {
                        out.local_domains.push(s.clone());
                    }
                }
            }
            Some(Dat::Vek(v)) => {
                for d in v.iter() {
                    if let Dat::Str(s) = d {
                        out.local_domains.push(s.clone());
                    }
                }
            }
            _ => (),
        }
        Ok(out)
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
    /// Primary TCP port for HTTPS traffic.
    pub server_port_tcp:                u16,
    /// Optional plaintext HTTP listener port. When non-zero, Steel binds
    /// this port too and responds to every incoming HTTP request with a
    /// `301 Moved Permanently` redirect to the equivalent HTTPS URL on
    /// the primary port. Typically set to `80` in production and `0`
    /// (disabled) in local development. Defaults to `0`.
    pub server_port_tcp_plaintext:      u16,
    /// `Strict-Transport-Security` `max-age` in seconds, injected into
    /// every HTTPS response when non-zero. A value of `31536000` (one
    /// year) is conventional for production. Defaults to `0` (no HSTS).
    pub hsts_max_age_secs:              u32,
    /// Optional plaintext HTTP listener bound to `127.0.0.1` for the
    /// admin dashboard only. When non-zero, Steel binds this port on
    /// the loopback interface and serves the `/admin/*` routes
    /// without TLS. Use case: SSH-tunnel to the host and reach the
    /// dashboard locally without going through the public TLS chain
    /// (useful when a cert has expired, when ACME is broken, or when
    /// the operator wants emergency access). Anything other than
    /// `/admin*` returns 404. Defaults to `0` (disabled).
    pub admin_local_port:               u16,
    /// Default session lifetime in seconds.
    pub session_expiry_default_secs:    u32,
    /// WebSocket ping interval in seconds.
    pub ws_ping_interval_secs:          u8,
    /// Maximum consecutive errors allowed on a single connection.
    pub server_max_errors_allowed:      u8,
    /// Whether to issue a session cookie to unauthenticated clients on
    /// first contact. When `true`, Steel generates a fresh session id for
    /// any incoming request that does not already carry one and attaches
    /// it as an `HttpOnly`, `Secure`, `SameSite=Lax` cookie. This makes
    /// session-scoped WebSocket commands work for anonymous browsers.
    /// When `false`, requests without a session cookie are still served
    /// but session-scoped commands will reject until the client obtains
    /// a session id through some other mechanism.
    pub allow_anonymous_sessions:       bool,
    /// Maximum bytes accepted in the HTTP request header block before
    /// the reader returns `413 Content Too Large`. Bounds memory
    /// exposure to oversized headers such as cookie stuffing. A value
    /// of `0` disables the limit.
    pub http_max_header_bytes:          u64,
    /// Maximum bytes accepted in the HTTP request body before the
    /// reader returns `413 Content Too Large`. Checked against the
    /// `Content-Length` header up front so oversize requests are
    /// rejected before any body bytes are read. A value of `0`
    /// disables the limit.
    pub http_max_body_bytes:            u64,
    /// Wall-clock budget for the HTTP header read phase, in
    /// milliseconds. A slow client that fails to finish sending its
    /// header block within this window is disconnected with a
    /// `Timeout` error. A value of `0` disables the deadline.
    pub http_header_read_timeout_ms:    u64,
    /// When `true`, Steel injects a baseline set of security
    /// response headers into every HTTPS response: `X-Content-Type-Options`,
    /// `X-Frame-Options`, `Referrer-Policy`, `Permissions-Policy`.
    /// Recommended on for production deployments.
    pub security_headers_enabled:       bool,
    /// Optional `Content-Security-Policy` header value. When
    /// non-empty, Steel emits this string verbatim as the CSP
    /// header on every HTTPS response. Defaults to empty because
    /// CSP is app-specific and a strict default would break
    /// existing front ends. Tighten on a per-deployment basis.
    pub content_security_policy:        String,

    // --- Virtual hosts ------------------------------------------------------
    /// Ordered list of virtual host configurations, stored as a `Dat::List`
    /// of `Dat::Map` entries and parsed via `get_vhosts()`.
    pub vhosts:                         Dat,

    // --- ACME ---------------------------------------------------------------
    /// ACME client configuration (as a daticle map, parsed via `get_acme()`).
    pub acme:                           DaticleMap,

    // --- Mail ---------------------------------------------------------------
    /// Mail listener configuration (as a daticle map, parsed via `get_mail()`).
    /// Empty map disables the mail server entirely.
    pub mail:                           DaticleMap,
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
        if let Some(ref p) = default_vhost.db_dir_rel {
            vhost_map.insert(dat!("db_dir_rel"), dat!(p.clone()));
        }

        Self {
            tls_dir_rel:                    fmt!("./tls"),
            log_level:                      fmt!("debug"),
            num_server_bots:                1,
            server_address:                 fmt!("0.0.0.0"),
            server_port_tcp:                8443,
            server_port_tcp_plaintext:      0,      // disabled by default
            hsts_max_age_secs:              0,      // disabled by default
            admin_local_port:               0,      // disabled by default
            session_expiry_default_secs:    604_800, // 1 week.
            ws_ping_interval_secs:          30,
            server_max_errors_allowed:      30,
            allow_anonymous_sessions:       true,
            http_max_header_bytes:          16 * 1024,            // 16 KiB
            http_max_body_bytes:            8 * 1024 * 1024,      // 8 MiB
            http_header_read_timeout_ms:    15_000,               // 15 s
            security_headers_enabled:       true,
            content_security_policy:        String::new(),
            vhosts:                         Dat::List(vec![Dat::Map(vhost_map)]),
            acme:                           AcmeConfig::default().to_datmap(),
            mail:                           DaticleMap::new(),
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

    /// Parse and return the mail configuration. Returns `None` if no
    /// mail block is configured (`mail = {}` in JDAT).
    pub fn get_mail(&self) -> Outcome<Option<MailConfig>> {
        if self.mail.is_empty() {
            return Ok(None);
        }
        let cfg = res!(MailConfig::from_datmap(&self.mail));
        if !cfg.enabled {
            return Ok(None);
        }
        Ok(Some(cfg))
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
