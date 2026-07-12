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
    /// Upstream hostname (e.g. `api.example.com`, `127.0.0.1`). `None`
    /// when the route is served by an in-process handler.
    pub upstream_host:  Option<String>,
    /// Upstream port (defaults to 443 for `https://`, 80 for `http://`).
    /// `None` when handler-served.
    pub upstream_port:  Option<u16>,
    /// Upstream request path (e.g. `/v1/checkout/sessions`). `None`
    /// when handler-served.
    pub upstream_path:  Option<String>,
    /// `true` when the upstream URL used `https://`. Proxy dispatch
    /// opens a TLS connection when this is set and a plain TCP
    /// connection otherwise. Defaults to `true` so third-party API
    /// proxying keeps the pre-feature semantics; the new
    /// `http://` form is reserved for loopback app binaries where
    /// TLS is unnecessary.
    pub upstream_tls:   bool,
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
        // Parse upstream URL into host, port, path, scheme (proxy mode only).
        let (upstream_host, upstream_port, upstream_path, upstream_tls) = match upstream_str {
            Some(url) => {
                let (h, p, up, tls) = res!(Self::parse_upstream(&url));
                (Some(h), Some(p), Some(up), tls)
            }
            None => (None, None, None, true),
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
            upstream_tls,
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
    /// Parse an upstream URL into `(host, port, path, tls)`. Accepts
    /// both `https://` and `http://`; the former sets `tls = true` and
    /// defaults the port to 443, the latter sets `tls = false` and
    /// defaults the port to 80. Plain HTTP is intended for loopback
    /// upstreams only -- a public API reached over HTTP is a separate
    /// security mistake and Steel does not make it easier to do.
    pub fn parse_upstream(url: &str) -> Outcome<(String, u16, String, bool)> {
        let (rest, tls, default_port) = if let Some(r) = url.strip_prefix("https://") {
            (r, true, 443u16)
        } else if let Some(r) = url.strip_prefix("http://") {
            (r, false, 80u16)
        } else {
            return Err(err!(
                "ApiRoute: upstream URL must start with 'https://' or 'http://'. \
                Got: '{}'.", url;
                Invalid, Input));
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
            None => (host_port.to_string(), default_port),
        };
        Ok((host, port, path.to_string(), tls))
    }

    /// Expand `{file:path}` and `{env:}` placeholders in all header
    /// values by reading the referenced files relative to `root`. Used
    /// in proxy mode for headers injected into the upstream request.
    /// Handler-config values are resolved separately by
    /// [`ApiRoute::resolve_config`]; a route with an in-process handler
    /// should have both called at startup. Must be called once before
    /// the route is dispatched.
    pub fn resolve_headers(&mut self, root: &Path) -> Outcome<()> {
        for (_name, value) in &mut self.headers {
            *value = res!(Self::resolve_file_refs(value, root));
        }
        Ok(())
    }

    /// Expand `{file:path}` and `{env:}` placeholders in all
    /// handler-config values by reading the referenced files relative
    /// to `root`. Mirrors [`WebhookRoute::resolve_config`] so an
    /// in-process API handler can read a resolved secret (e.g. a
    /// Stripe key) out of its `config` map. Must be called once at
    /// startup before the route is dispatched.
    pub fn resolve_config(&mut self, root: &Path) -> Outcome<()> {
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
    pub path:           String,
    /// In-process handler name (e.g. `payments_forwarder`). `None`
    /// when the route forwards to an upstream URL instead.
    pub handler:        Option<String>,
    /// Upstream hostname when the route forwards instead of
    /// dispatching in-process. Mutually exclusive with `handler`.
    pub upstream_host:  Option<String>,
    /// Upstream port when forwarding.
    pub upstream_port:  Option<u16>,
    /// Upstream request path when forwarding (the hook payload is
    /// POSTed here verbatim).
    pub upstream_path:  Option<String>,
    /// `true` when the upstream URL used `https://`; `false` for
    /// plain HTTP loopback upstreams. Default `true` mirrors the
    /// `ApiRoute` conservative default.
    pub upstream_tls:   bool,
    /// Handler-specific configuration (in-process mode only).
    pub config:         Vec<(String, String)>,
}

impl WebhookRoute {
    /// Parse a webhook route from a `DaticleMap`.
    ///
    /// Accepts either an in-process `handler` field or an
    /// `upstream` URL; exactly one of the two is required, and
    /// setting both is a configuration error. The `upstream` URL
    /// follows the same `https://` / `http://` grammar as
    /// [`ApiRoute::parse_upstream`].
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let path = match m.get(&dat!("path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "WebhookRoute: 'path' is required and must be a string.";
                Invalid, Input, Missing)),
        };
        let handler = match m.get(&dat!("handler")) {
            Some(Dat::Str(s)) => Some(s.clone()),
            None              => None,
            _ => return Err(err!(
                "WebhookRoute '{}': 'handler' must be a string when present.", path;
                Invalid, Input, Mismatch)),
        };
        let upstream_str = match m.get(&dat!("upstream")) {
            Some(Dat::Str(s)) => Some(s.clone()),
            None              => None,
            _ => return Err(err!(
                "WebhookRoute '{}': 'upstream' must be a string when present.", path;
                Invalid, Input, Mismatch)),
        };
        match (&handler, &upstream_str) {
            (None, None) => return Err(err!(
                "WebhookRoute '{}': must specify either 'handler' (for \
                in-process webhooks) or 'upstream' (for forwarded \
                webhooks).", path;
                Invalid, Input, Missing)),
            (Some(_), Some(_)) => return Err(err!(
                "WebhookRoute '{}': 'handler' and 'upstream' are mutually \
                exclusive. A webhook route is either in-process or \
                forwarded, not both.", path;
                Invalid, Input, Conflict)),
            _ => (),
        }
        let (upstream_host, upstream_port, upstream_path, upstream_tls) = match upstream_str {
            Some(url) => {
                let (h, p, up, tls) = res!(ApiRoute::parse_upstream(&url));
                (Some(h), Some(p), Some(up), tls)
            }
            None => (None, None, None, true),
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
        Ok(Self {
            path,
            handler,
            upstream_host,
            upstream_port,
            upstream_path,
            upstream_tls,
            config,
        })
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

    /// True when the route forwards to an upstream instead of
    /// dispatching to an in-process handler.
    pub fn is_upstream(&self) -> bool {
        self.upstream_host.is_some()
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROXY ROUTES                                                              │
// │                                                                           │
// │ A reverse-proxy route forwards all requests under a path prefix to an     │
// │ upstream server.  Unlike ApiRoute (exact path match, buffered response),  │
// │ ProxyRoute uses prefix matching, supports WebSocket upgrade tunneling,     │
// │ and streams response bodies without buffering — making it suitable for     │
// │ proxying full web applications including those that use SSE or WebSocket  │
// │ for real-time communication.                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// A reverse-proxy route that forwards all requests under a path prefix
/// to an upstream server.
///
/// When a request's path starts with `path_prefix`, Steel connects to
/// the upstream over TCP (optionally TLS), forwards the request, and
/// streams the response back to the client.  WebSocket upgrade requests
/// are transparently tunnelled: Steel connects to the upstream, forwards
/// the upgrade handshake, then bidirectionally pipes raw bytes between
/// client and upstream for the lifetime of the WebSocket connection.
///
/// Proxy routes are checked after redirect rules but before static file
/// serving and API routes.  When multiple proxy routes match, the longest
/// prefix wins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyRoute {
    /// Path prefix to match (e.g. `/` matches everything, `/api/` matches
    /// all paths starting with `/api/`).
    pub path_prefix:    String,
    /// Upstream hostname or IP address (e.g. `127.0.0.1`, `localhost`).
    pub upstream_host:  String,
    /// Upstream TCP port (e.g. 3000).
    pub upstream_port:  u16,
    /// Whether to use TLS when connecting to the upstream.  Default false
    /// — loopback proxies typically do not need TLS.
    pub upstream_tls:   bool,
    /// Whether to strip the path prefix before forwarding.  When true,
    /// a request for `/chat/api/v1/users` with prefix `/chat` is
    /// forwarded as `/api/v1/users`.  When false, the full original
    /// path is forwarded verbatim.
    pub strip_prefix:   bool,
}

impl ProxyRoute {
    /// Parse a proxy route from a `DaticleMap`.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let path_prefix = match m.get(&dat!("path_prefix")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "ProxyRoute: 'path_prefix' is required and must be a string.";
                Invalid, Input, Missing)),
        };
        let upstream_host = match m.get(&dat!("upstream_host")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "ProxyRoute '{}': 'upstream_host' is required and must be a string.",
                path_prefix;
                Invalid, Input, Missing)),
        };
        let upstream_port = match m.get(&dat!("upstream_port")) {
            Some(Dat::U16(n)) => *n,
            Some(Dat::U32(n)) => *n as u16,
            Some(Dat::U64(n)) => *n as u16,
            Some(Dat::I64(n)) => *n as u16,
            _ => return Err(err!(
                "ProxyRoute '{}': 'upstream_port' is required and must be a number.",
                path_prefix;
                Invalid, Input, Missing)),
        };
        let upstream_tls = match m.get(&dat!("upstream_tls")) {
            Some(Dat::Bool(b)) => *b,
            None => false,
            _ => return Err(err!(
                "ProxyRoute '{}': 'upstream_tls' must be a boolean when present.",
                path_prefix;
                Invalid, Input, Mismatch)),
        };
        let strip_prefix = match m.get(&dat!("strip_prefix")) {
            Some(Dat::Bool(b)) => *b,
            None => false,
            _ => return Err(err!(
                "ProxyRoute '{}': 'strip_prefix' must be a boolean when present.",
                path_prefix;
                Invalid, Input, Mismatch)),
        };
        Ok(Self {
            path_prefix,
            upstream_host,
            upstream_port,
            upstream_tls,
            strip_prefix,
        })
    }

    /// Returns `true` if the given request path matches this proxy route's
    /// prefix.
    pub fn matches(&self, request_path: &str) -> bool {
        request_path.starts_with(&self.path_prefix)
    }

    /// Compute the upstream request path, stripping the prefix if configured.
    pub fn upstream_path_for(&self, request_path: &str) -> String {
        if self.strip_prefix {
            if let Some(stripped) = request_path.strip_prefix(&self.path_prefix) {
                if stripped.is_empty() {
                    "/".to_string()
                } else if stripped.starts_with('/') {
                    stripped.to_string()
                } else {
                    fmt!("/{}", stripped)
                }
            } else {
                request_path.to_string()
            }
        } else {
            request_path.to_string()
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TERMINAL CONFIG                                                           │
// │                                                                           │
// │ Enables terminal session management for a vhost.  When configured,        │
// │ Steel adds term_* commands to the WS syntax protocol and a binary         │
// │ WS endpoint at /term/<session> for bidirectional terminal I/O.            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Configuration for the terminal session manager.
///
/// When present in a [`VhostConfig`], enables terminal features:
/// creating, listing, closing and renaming tmux-backed sessions,
/// plus a binary WS endpoint for terminal I/O bridging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermConfig {
    /// Prefix for tmux session names (e.g. "goose-").
    pub session_prefix:     String,
    /// Command to launch in new sessions (e.g. "goose session").
    pub launch_command:     String,
}

impl TermConfig {

    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let session_prefix = match m.get(&dat!("session_prefix")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "term-".to_string(),
        };
        let launch_command = match m.get(&dat!("launch_command")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => "/bin/bash".to_string(),
        };
        Ok(Self {
            session_prefix,
            launch_command,
        })
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
    /// Optional allow-list of outbound egress targets. When
    /// non-empty, every `api_routes` upstream must match at
    /// least one entry or the server refuses to start. Entries
    /// are `host` or `host:port` strings; `host` alone matches
    /// any port. Empty list (the default) means "no allow-list
    /// configured" and every upstream is permitted, matching the
    /// pre-feature behaviour. Populating this field on a vhost is
    /// a defence against a compromised app config exfiltrating
    /// via an arbitrary upstream URL.
    pub egress_allowed:         Vec<String>,
    /// Authorised signing keys for the signed-admin-login flow.
    /// Each entry binds a named operator to a public key and a
    /// scope list; a #raw("SignedCommand") with #raw("cmd") =
    /// `"admin_login"` and #raw("signer_id") matching one of these
    /// entries' public keys issues a dashboard session cookie
    /// without a wallet passphrase. Empty list means the feature
    /// is disabled for this vhost and the classical
    /// passphrase-form login is the only admin entry.
    pub admin_keys:             Vec<AdminKey>,
    /// Optional URL of a script or stylesheet resource to inject
    /// into the `<head>` of every admin-served page. An operator
    /// uses this to plug an Oxegen-style header bar or similar
    /// cross-app chrome onto a Steel deployment without touching
    /// the Steel source. `None` leaves the default `<head>`
    /// untouched. Interpreted as a raw URL, rendered as
    /// `<script src="{url}" defer></script>`.
    pub head_injection_url:     Option<String>,
    /// Reverse-proxy routes.  Each route forwards all requests under
    /// a path prefix to an upstream server, with WebSocket tunneling
    /// and streaming response support.  Checked after redirects but
    /// before static files and API routes; longest prefix wins.
    pub proxy_routes:           Vec<ProxyRoute>,
    /// Terminal session configuration.  When present, enables the
    /// `term_new`, `term_list`, `term_close` and `term_set_name`
    /// WS commands and the `/term/<session>` binary WS endpoint
    /// for this vhost.  `None` disables terminal features.
    pub term_config:            Option<TermConfig>,
}

/// A single entry in a vhost's [`VhostConfig::admin_keys`] list.
///
/// Names a public key, a human-readable identity and a scope list.
/// The signed-admin-login flow looks up an inbound
/// #raw("SignedCommand")'s #raw("signer_id") against these entries'
/// public keys; a match yields the matching name and scopes for the
/// session cookie. Scopes use the same vocabulary as
/// [`AdminUser::scopes`](oxedyne_fe2o3_crypto::keystore::AdminUser)
/// so the dashboard gates requests identically regardless of whether
/// the admin authenticated via passphrase or signature.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminKey {
    /// Human-readable identity for the key's holder. Used in audit
    /// log output and the dashboard's admin view.
    pub name:           String,
    /// Raw public key bytes. Encoded as lowercase hex in the config
    /// file for human-readability; parsed into bytes at load time.
    pub public_key:     Vec<u8>,
    /// Signature scheme name, matching one of
    /// [`SignatureScheme`](oxedyne_fe2o3_crypto::sign::SignatureScheme)'s
    /// `Debug` output strings (`"Ed25519"`, `"Dilithium2"`,
    /// `"Dilithium2_fe2o3"`).
    pub scheme:         String,
    /// Scopes granted to a session authenticated with this key. Uses
    /// the same vocabulary as the wallet's admin entries; `"*"` is
    /// the wildcard.
    pub scopes:         Vec<String>,
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
            egress_allowed:         Vec::new(),
            admin_keys:             Vec::new(),
            head_injection_url:     None,
            proxy_routes:           Vec::new(),
            term_config:            None,
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
        // Egress allow-list (optional).
        let egress_allowed = match m.get(&dat!("egress_allowed")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'egress_allowed' entries must be strings.";
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
                            "VhostConfig: 'egress_allowed' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'egress_allowed' must be a list of strings.";
                Invalid, Input, Mismatch)),
        };
        // Authorised signed-admin-login keys (optional).
        let admin_keys = match m.get(&dat!("admin_keys")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::with_capacity(list.len());
                for item in list {
                    out.push(res!(AdminKey::from_dat(item.clone())));
                }
                out
            }
            Some(Dat::Vek(vek)) => {
                let mut out = Vec::with_capacity(vek.len());
                for item in vek.iter() {
                    out.push(res!(AdminKey::from_dat(item.clone())));
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'admin_keys' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        // Head-injection URL (optional).
        let head_injection_url = match m.get(&dat!("head_injection_url")) {
            Some(Dat::Str(s)) => Some(s.clone()),
            None => None,
            _ => return Err(err!(
                "VhostConfig: 'head_injection_url' must be a string.";
                Invalid, Input, Mismatch)),
        };
        // Reverse proxy routes (optional).
        let proxy_routes = match m.get(&dat!("proxy_routes")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Map(sub) => out.push(res!(ProxyRoute::from_datmap(sub))),
                        _ => return Err(err!(
                            "VhostConfig: 'proxy_routes' entries must be maps.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'proxy_routes' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        let term_config = match m.get(&dat!("term_config")) {
            Some(Dat::Map(sub)) => Some(res!(TermConfig::from_datmap(sub))),
            None => None,
            _ => return Err(err!(
                "VhostConfig: 'term_config' must be a map.";
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
            egress_allowed,
            admin_keys,
            head_injection_url,
            proxy_routes,
            term_config,
        })
    }

    /// Check every `api_routes` upstream against the `egress_allowed`
    /// list. Returns `Ok(())` when the allow-list is empty (no
    /// enforcement) or when every upstream matches at least one
    /// entry. Entries are compared as `host` or `host:port`: a
    /// bare-host entry matches any port for that host, and a
    /// `host:port` entry requires an exact match.
    pub fn validate_egress(&self) -> Outcome<()> {
        if self.egress_allowed.is_empty() {
            return Ok(());
        }
        for route in &self.api_routes {
            let (h, p) = match (&route.upstream_host, &route.upstream_port) {
                (Some(h), Some(p)) => (h.clone(), *p),
                _ => continue, // handler-served route; no outbound
            };
            let mut ok = false;
            for entry in &self.egress_allowed {
                if let Some((eh, ep)) = entry.split_once(':') {
                    if eh == h.as_str() {
                        if let Ok(ep_n) = ep.parse::<u16>() {
                            if ep_n == p {
                                ok = true;
                                break;
                            }
                        }
                    }
                } else if entry == h.as_str() {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err(err!(
                    "VhostConfig '{}': api route '{}' upstream {}:{} is not \
                    in the configured egress_allowed list ({:?}).",
                    self.primary_hostname(), route.path, h, p,
                    self.egress_allowed;
                    Invalid, Input, Security, Configuration));
            }
        }
        Ok(())
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
    #[optional]
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
    // ── Hardening knobs ───────────────────────────────────────────────────
    //
    // Every field below is `#[optional]` so on-disk configs from
    // earlier Steel builds continue to load. Missing fields fall
    // through to the Default impl (which reproduces the pre-feature
    // "permissive" behaviour for each: no size/time limits, headers
    // enabled, empty CSP, empty guard block).

    /// Maximum bytes accepted in the HTTP request header block before
    /// the reader returns `413 Content Too Large`. A value of `0`
    /// disables the limit.
    #[optional]
    pub http_max_header_bytes:          u64,
    /// Maximum bytes accepted in the HTTP request body before the
    /// reader returns `413 Content Too Large`. A value of `0`
    /// disables the limit.
    #[optional]
    pub http_max_body_bytes:            u64,
    /// Wall-clock budget for the HTTP header read phase, in
    /// milliseconds. A slow client that fails to finish sending its
    /// header block within this window is disconnected with a
    /// `Timeout` error. A value of `0` disables the deadline.
    #[optional]
    pub http_header_read_timeout_ms:    u64,
    /// When `true`, Steel injects a baseline set of security
    /// response headers into every HTTPS response: `X-Content-Type-Options`,
    /// `X-Frame-Options`, `Referrer-Policy`, `Permissions-Policy`.
    #[optional]
    pub security_headers_enabled:       bool,
    /// Optional `Content-Security-Policy` header value.
    #[optional]
    pub content_security_policy:        String,
    /// Per-IP address guard tuning. Empty map restores defaults.
    #[optional]
    pub addr_guard:                     DaticleMap,
    /// URL path prefixes routed through the tighter auth-path
    /// rate limiter.
    #[optional]
    pub auth_path_prefixes:             Vec<String>,
    /// Maximum average requests per second permitted against the
    /// auth path prefixes.
    #[optional]
    pub auth_rps_max:                   u64,

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
            addr_guard:                     DaticleMap::new(),
            auth_path_prefixes:             vec![
                fmt!("/login"),
                fmt!("/admin/login"),
            ],
            auth_rps_max:                   5,
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
            // Egress allow-list check: a vhost whose API proxy
            // routes target an upstream outside its configured
            // allow-list is refused at start-up. The check is a
            // no-op when the allow-list is empty.
            res!(vh.validate_egress());
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

    /// Parse the `addr_guard` map block into runtime settings for the
    /// per-IP address guard. Every field is optional; a missing or
    /// unrecognised field falls back to the module default, and an
    /// entirely empty map restores every default.
    pub fn get_addr_guard_settings(
        &self,
    )
        -> crate::srv::admin::guard::AddrGuardSettings
    {
        use crate::srv::admin::guard::AddrGuardSettings;
        let mut s = AddrGuardSettings::default();
        let take_u64 = |key: &str| -> Option<u64> {
            match self.addr_guard.get(&dat!(key)) {
                Some(Dat::U64(v)) => Some(*v),
                Some(Dat::U32(v)) => Some(*v as u64),
                Some(Dat::U16(v)) => Some(*v as u64),
                Some(Dat::U8(v))  => Some(*v as u64),
                _ => None,
            }
        };
        if let Some(v) = take_u64("rps_max") {
            s.rps_max = v;
        }
        if let Some(v) = take_u64("tint_min_ms") {
            s.tint_min = Duration::from_millis(v);
        }
        if let Some(v) = take_u64("tsunset_base_secs") {
            s.tsunset_base = Duration::from_secs(v);
        }
        if let Some(v) = take_u64("tsunset_spread_secs") {
            s.tsunset_spread = Duration::from_secs(v);
        }
        if let Some(v) = take_u64("blist_cnt") {
            s.blist_cnt = v.min(u16::MAX as u64) as u16;
        }
        s
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


impl AdminKey {
    /// Parses a single admin-key entry from a `Dat` map. Expected
    /// shape:
    ///
    /// ```text
    /// {
    ///     "name":       "alice",
    ///     "scheme":     "Ed25519",
    ///     "public_key": "<base2x HEMATITE64 bytes>",
    ///     "scopes":     ["*"],
    /// }
    /// ```
    ///
    /// `public_key` is the canonical fe2o3 byte-string encoding --
    /// [`base2x::HEMATITE64`](oxedyne_fe2o3_text::base2x::HEMATITE64) --
    /// matching what `oxegen keygen` prints.
    pub fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut dat = dat;
        if dat.kind() != oxedyne_fe2o3_jdat::kind::Kind::Map
            && dat.kind() != oxedyne_fe2o3_jdat::kind::Kind::OrdMap
        {
            return Err(err!(
                "admin_keys entry must be a map, got {:?}.", dat.kind();
                Invalid, Input, Mismatch));
        }
        let name = match dat.map_remove_must(&dat!("name")) {
            Ok(Dat::Str(s)) => s,
            Ok(other) => return Err(err!(
                "admin_keys entry 'name' must be a string, got {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
            Err(_) => return Err(err!(
                "admin_keys entry missing 'name'.";
                Invalid, Input, Missing)),
        };
        let scheme = match dat.map_remove_must(&dat!("scheme")) {
            Ok(Dat::Str(s)) => s,
            Ok(other) => return Err(err!(
                "admin_keys entry 'scheme' must be a string, got {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
            Err(_) => "Ed25519".to_string(),	// default
        };
        let public_key_enc = match dat.map_remove_must(&dat!("public_key")) {
            Ok(Dat::Str(s)) => s,
            _ => return Err(err!(
                "admin_keys entry '{}' missing or non-string 'public_key'.",
                name;
                Invalid, Input, Mismatch)),
        };
        let public_key = match oxedyne_fe2o3_text::base2x::HEMATITE64
            .from_str(&public_key_enc)
        {
            Ok(b) => b,
            Err(e) => return Err(err!(e,
                "admin_keys entry '{}' 'public_key' is not valid \
                base2x HEMATITE64.", name;
                Invalid, Input, Decode)),
        };
        let scopes = match dat.map_remove_must(&dat!("scopes")) {
            Ok(Dat::List(list)) => {
                let mut out = Vec::with_capacity(list.len());
                for item in list {
                    match item {
                        Dat::Str(s) => out.push(s),
                        other => return Err(err!(
                            "admin_keys entry '{}' scope must be a string, \
                            got {:?}.", name, other.kind();
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            Ok(other) => return Err(err!(
                "admin_keys entry '{}' 'scopes' must be a list, got {:?}.",
                name, other.kind();
                Invalid, Input, Mismatch)),
            Err(_) => Vec::new(),
        };
        Ok(Self { name, scheme, public_key, scopes })
    }
}
