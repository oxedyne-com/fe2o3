//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
    constant,
    publish::PublishConfig,
};

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
    sms::Provider as SmsProvider,
    http::{
        encoding,
        fields::{
            Cookie,
            SetCookieAttributes,
            SameSite,
        },
        fwd::ForwardedPolicy,
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
    Exact,      // e.g. `/admin`
    Prefix,     // any request whose path starts with `match_path`
    All,        // any path on the vhost, typically for a www -> canonical redirect
}

impl RedirectMatch {
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
    pub match_kind: RedirectMatch,
    pub match_path: String,         // ignored when `match_kind` is `All`
    // May contain the literal `{uri}`, which is replaced by the matched request path and query
    // string at redirect time.
    pub target:     String,
    pub status:     u16,            // normally 301 permanent or 302 temporary
}

impl RedirectRule {
    pub fn resolve_target(&self, request_uri: &str) -> String {
        if self.target.contains("{uri}") {
            self.target.replace("{uri}", request_uri)
        } else {
            self.target.clone()
        }
    }

    pub fn matches(&self, request_path: &str) -> bool {
        match self.match_kind {
            RedirectMatch::Exact    => request_path == self.match_path,
            RedirectMatch::Prefix   => request_path.starts_with(&self.match_path),
            RedirectMatch::All      => true,
        }
    }

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
    pub path:           String,                     // e.g. `/api/payments/checkout`
    pub upstream_host:  Option<String>,             // `None` when served by a handler
    pub upstream_port:  Option<u16>,                // 443 for `https://`, 80 for `http://`
    pub upstream_path:  Option<String>,             // e.g. `/v1/checkout/sessions`
    // True when the upstream URL used `https://`: dispatch opens a TLS connection when it is set
    // and a plain TCP connection otherwise. Defaults to true, so third-party API proxying keeps
    // the pre-feature semantics; the `http://` form is reserved for loopback app binaries where
    // TLS is unnecessary.
    pub upstream_tls:   bool,
    pub headers:        Vec<(String, String)>,      // `{file:...}` expanded at load time
    pub handler:        Option<String>,             // in-process; `None` for a proxy route
    pub config:         Vec<(String, String)>,      // handler config, `{file:}`/`{env:}` resolved
}

impl ApiRoute {
    /// Header values are stored as-is and may contain `{file:path}` placeholders. Call
    /// `resolve_headers` with the app root to expand them before use.
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

    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.config.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

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

    /// Resolve `{file:path}`, optional `{file?:path}`, and `{env:VAR}` or
    /// `{env:VAR:default}` placeholders in a config value.
    ///
    /// * `{file:path}` — replaced with the trimmed contents of the file,
    ///   resolved relative to `root`. Fails if the file cannot be read.
    /// * `{file?:path}` — the optional form: replaced with the trimmed
    ///   contents when the file exists, and with the empty string when it
    ///   is absent. Any read error other than not-found — a present file
    ///   the process may not read, say — still fails, so an unreadable key
    ///   is never silently dropped.
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

    /// Resolve `{file:path}` and optional `{file?:path}` placeholders.
    ///
    /// `{file:path}` fails on any read error. `{file?:path}` resolves to
    /// the empty string when the file is not found, but still fails on any
    /// other read error — a present-but-unreadable key must not be silently
    /// swallowed.
    fn resolve_file_only(value: &str, root: &Path) -> Outcome<String> {
        let mut result = value.to_string();
        loop {
            // Find the earliest of the two markers. The required '{file:' is
            // not a substring of the optional '{file?:' — the sixth byte is
            // ':' against '?' — so a plain search for one never matches the
            // other, and the two positions can never coincide.
            let required = result.find("{file:");
            let optional = result.find("{file?:");
            let (start, marker_len, is_optional) = match (required, optional) {
                (None,    None)              => break,
                (Some(r), None)              => (r, 6, false),
                (None,    Some(o))           => (o, 7, true),
                (Some(r), Some(o)) if o < r  => (o, 7, true),
                (Some(r), Some(_))           => (r, 6, false),
            };
            let opt_mark = if is_optional { "?" } else { "" };
            let end = match result[start..].find('}') {
                Some(i) => start + i,
                None => return Err(err!(
                    "Config: unclosed '{{file{}:' placeholder in value '{}'.",
                    opt_mark, value;
                    Invalid, Input)),
            };
            let rel_path = result[start + marker_len..end].to_string();
            let abs_path = root.join(&rel_path);
            let content = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s.trim().to_string(),
                // An absent optional file resolves to nothing. Only not-found
                // is tolerated, so a permissions failure on a present file
                // still errors below rather than yielding a silent empty key.
                Err(e) if is_optional
                    && e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(err!(e,
                    "Config: failed to read '{{file{}:{}}}' at '{:?}'.",
                    opt_mark, rel_path, abs_path;
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
    pub path:           String,                     // e.g. `/webhook/payments`
    pub handler:        Option<String>,             // `None` when the route forwards upstream
    pub upstream_host:  Option<String>,             // mutually exclusive with `handler`
    pub upstream_port:  Option<u16>,
    pub upstream_path:  Option<String>,             // the payload is POSTed here verbatim
    pub upstream_tls:   bool,                       // `https://`; false for loopback HTTP
    pub config:         Vec<(String, String)>,      // in-process mode only
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

    pub fn resolve_config(&mut self, root: &Path) -> Outcome<()> {
        for (_name, value) in &mut self.config {
            *value = res!(ApiRoute::resolve_file_refs(value, root));
        }
        Ok(())
    }

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
    pub path_prefix:    String,                     // `/` matches everything, `/api/` a subtree
    pub upstream_host:  String,                     // e.g. `127.0.0.1`, `localhost`
    pub upstream_port:  u16,
    pub upstream_tls:   bool,                       // default false; loopback rarely needs it
    // Whether to strip the path prefix before forwarding. When true, a request for
    // `/chat/api/v1/users` with prefix `/chat` is forwarded as `/api/v1/users`; when false the
    // full original path goes verbatim.
    pub strip_prefix:   bool,
}

impl ProxyRoute {
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

    pub fn matches(&self, request_path: &str) -> bool {
        request_path.starts_with(&self.path_prefix)
    }

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
// │ WEBSOCKET ROUTES                                                          │
// │                                                                           │
// │ A WebSocket route hands one path's upgrades to a WebSocket server of its   │
// │ own, on loopback.  Unlike ProxyRoute (a whole application behind a path    │
// │ prefix) it matches one exact path and forwards nothing else, so a site     │
// │ whose pages, files and API stay with Steel can still give a single         │
// │ endpoint to a separate process that speaks its own protocol.               │
// └───────────────────────────────────────────────────────────────────────────┘

/// A route that forwards the WebSocket upgrade on one path to an upstream
/// WebSocket server.
///
/// On an HTTP/1.1 `GET` with `Upgrade: websocket` whose path equals
/// [`WsRoute::path`], Steel opens a plain TCP connection to the upstream,
/// forwards the handshake, relays the `101 Switching Protocols` back to the
/// client, and then copies bytes in both directions until either end closes.
/// The frames themselves are never parsed: what the client sends is what the
/// upstream receives.
///
/// Checked before proxy routes, because a route naming one exact path is more
/// specific than a prefix that happens to contain it. A request to the same
/// path that is *not* an upgrade is left alone, and falls through to the rest
/// of the dispatch chain.
///
/// The upstream URL is `ws://host[:port]/path`. There is deliberately no
/// `wss://` form: this exists to reach a server on the same machine, whose
/// traffic never leaves the loopback interface, and an operator who needs TLS
/// to the upstream is not describing loopback and should say so with a
/// [`ProxyRoute`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WsRoute {
    pub path:           String,                     // matched exactly, e.g. `/ws`
    pub upstream_host:  String,                     // e.g. `127.0.0.1`
    pub upstream_port:  u16,                        // 80 where the URL gives none
    pub upstream_path:  String,                     // need not be the local one
}

impl WsRoute {
    /// Both fields are required: `path` is the local path, `upstream` the
    /// `ws://host[:port]/path` URL to forward it to.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let path = match m.get(&dat!("path")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "WsRoute: 'path' is required and must be a string.";
                Invalid, Input, Missing)),
        };
        let upstream = match m.get(&dat!("upstream")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!(
                "WsRoute '{}': 'upstream' is required and must be a string.", path;
                Invalid, Input, Missing)),
        };
        let (upstream_host, upstream_port, upstream_path) =
            res!(Self::parse_upstream(&upstream));
        Ok(Self {
            path,
            upstream_host,
            upstream_port,
            upstream_path,
        })
    }

    /// Parse a `ws://host[:port][/path]` URL into `(host, port, path)`.
    ///
    /// A `wss://` URL is refused with an explanation rather than quietly
    /// treated as plaintext, which is what a scheme this route cannot honour
    /// would otherwise become.
    pub fn parse_upstream(url: &str) -> Outcome<(String, u16, String)> {
        let rest = match url.strip_prefix("ws://") {
            Some(r) => r,
            None => {
                if url.starts_with("wss://") {
                    return Err(err!(
                        "WsRoute: 'wss://' upstream '{}' cannot be honoured: a ws_route \
                        forwards to a loopback server over plain TCP. Use a proxy_route \
                        with upstream_tls for a TLS upstream.", url;
                        Invalid, Input, Unimplemented));
                }
                return Err(err!(
                    "WsRoute: upstream URL must start with 'ws://'. Got: '{}'.", url;
                    Invalid, Input));
            },
        };
        let (host_port, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None    => (rest, "/"),
        };
        if host_port.is_empty() {
            return Err(err!(
                "WsRoute: upstream URL '{}' names no host.", url;
                Invalid, Input, Missing));
        }
        let (host, port) = match host_port.rfind(':') {
            Some(i) => {
                let p: u16 = match host_port[i + 1..].parse() {
                    Ok(n)  => n,
                    Err(_) => return Err(err!(
                        "WsRoute: invalid port in upstream URL '{}'.", url;
                        Invalid, Input)),
                };
                (host_port[..i].to_string(), p)
            }
            None => (host_port.to_string(), 80u16),
        };
        Ok((host, port, path.to_string()))
    }

    pub fn matches(&self, request_path: &str) -> bool {
        self.path == request_path
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
    pub session_prefix:     String,                 // e.g. "goose-"
    pub launch_command:     String,                 // e.g. "goose session"
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
    pub hostnames:              Vec<String>,            // the first is canonical
    pub public_dir_rel:         Option<String>,         // `None` for a pure-redirect vhost
    pub static_route_paths_rel: DaticleMap,             // URL path -> file or directory
    pub default_index_files:    Vec<String>,            // tried in order for a directory
    pub redirects:              Vec<RedirectRule>,      // before static file resolution
    // Relative to the app root. `None` means the vhost has no backing database, which is typical
    // for a pure-redirect vhost. When set, Steel opens and starts a dedicated Ozone instance
    // rooted here at server start-up.
    pub db_dir_rel:             Option<String>,
    pub api_routes:             Vec<ApiRoute>,          // local POST path -> upstream URL
    pub webhook_routes:         Vec<WebhookRoute>,      // local POST path -> named handler
    // Optional allow-list of outbound egress targets. When non-empty, every upstream this vhost
    // can reach -- an api route, a forwarded webhook route, a ws route or a proxy route -- must
    // match at least one entry or the server refuses to start. Entries are `host` or `host:port`;
    // `host` alone matches any port. An empty list -- the default -- means no allow-list is
    // configured and every upstream is permitted. Populating this on a vhost is a defence against
    // a compromised app config exfiltrating via an arbitrary upstream URL.
    pub egress_allowed:         Vec<String>,
    // Authorised signing keys for the signed-admin-login flow. Each entry binds a named operator
    // to a public key and a scope list; a SignedCommand with cmd = `"admin_login"` and a
    // signer_id matching one of these entries' public keys issues a dashboard session cookie
    // without a wallet passphrase. An empty list disables the feature for this vhost, leaving the
    // passphrase form as the only admin entry.
    pub admin_keys:             Vec<AdminKey>,
    // Optional URL of a script or stylesheet to inject into the `<head>` of every admin-served
    // page, so an operator can plug cross-app chrome onto a deployment without touching the Steel
    // source. `None` leaves the default `<head>` untouched. Taken as a raw URL and rendered as
    // `<script src="{url}" defer></script>`.
    pub head_injection_url:     Option<String>,
    // Each route forwards every request under a path prefix to an upstream server, with WebSocket
    // tunnelling and streaming responses. Checked after redirects but before static files and API
    // routes; the longest prefix wins.
    pub proxy_routes:           Vec<ProxyRoute>,
    // Each hands the upgrades arriving on one exact path to an upstream WebSocket server on
    // loopback, relaying the handshake and then the bytes. Checked before proxy routes, and only
    // for a request that is an upgrade. Empty by default, which is what every config written
    // before the field existed says.
    pub ws_routes:              Vec<WsRoute>,
    // Terminal session configuration. When present, enables the `term_new`, `term_list`,
    // `term_close` and `term_set_name` WS commands and the `/term/<session>` binary WS endpoint
    // for this vhost. `None` disables terminal features.
    pub term_config:            Option<TermConfig>,
    // The prose this vhost publishes: a directory of Markdown served as pages, a feed and a JSON
    // list, under a prefix of the site's choosing. `None` publishes nothing and serves none of
    // those paths, which is what a config with no `publish` block means and what every config
    // written before the block existed says.
    pub publish:                Option<PublishConfig>,
    // Who may administer this site from within it, at `/manage`. Each entry is a member's
    // username -- the same identifier the site's own login issues, which is the SHA-256 of the
    // member's passphrase. A member whose username is in this list, and who is signed in, reaches
    // the site console; everyone else is turned away from it.
    //
    // This is the operator's grant, and it lives here rather than in the site's database on
    // purpose. The operator owns the host and decides who runs each site; a site's own database,
    // which a content bug could reach, cannot mint its own administrators, so the blast radius of
    // such a bug stays content and never becomes authority. Empty -- the default, and what every
    // config written before this existed says -- means the site has no console.
    pub site_admins:            Vec<String>,
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
    pub name:           String,     // used in audit output and the dashboard's admin view
    pub public_key:     Vec<u8>,    // lowercase hex in the config file, bytes here
    pub scheme:         String,     // "Ed25519", "Dilithium2" or "Dilithium2_fe2o3"
    pub scopes:         Vec<String>,    // the wallet's own vocabulary; `"*"` is the wildcard
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
            publish:                None,
            egress_allowed:         Vec::new(),
            admin_keys:             Vec::new(),
            head_injection_url:     None,
            proxy_routes:           Vec::new(),
            ws_routes:              Vec::new(),
            term_config:            None,
            site_admins:            Vec::new(),
        }
    }
}

impl VhostConfig {
    pub fn primary_hostname(&self) -> &str {
        self.hostnames.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Has this vhost anywhere to keep a session?
    ///
    /// A session identifier is a key prefix into the vhost's own database:
    /// `sess:<sid>:...` for what a session holds, `sess_meta:<sid>` for the
    /// session itself. A vhost configured without a database has nowhere to put
    /// either, and its session commands already answer "no database available",
    /// so an identifier issued to one of its visitors can never be used for
    /// anything.
    ///
    /// Issuing one anyway is not free. A `Set-Cookie` on a static asset makes
    /// every response uncacheable by a shared cache, and a cookie set without a
    /// purpose is a cookie an operator has to account for to anyone who asks
    /// what it is for. So a vhost with no database mints none.
    pub fn uses_sessions(&self) -> bool {
        self.db_dir_rel.is_some()
    }

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
        // WebSocket routes (optional).
        let ws_routes = match m.get(&dat!("ws_routes")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Map(sub) => out.push(res!(WsRoute::from_datmap(sub))),
                        _ => return Err(err!(
                            "VhostConfig: 'ws_routes' entries must be maps.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'ws_routes' must be a list of maps.";
                Invalid, Input, Mismatch)),
        };
        let term_config = match m.get(&dat!("term_config")) {
            Some(Dat::Map(sub)) => Some(res!(TermConfig::from_datmap(sub))),
            None => None,
            _ => return Err(err!(
                "VhostConfig: 'term_config' must be a map.";
                Invalid, Input, Mismatch)),
        };
        // Absent means the vhost publishes nothing, which is what every config
        // written before this block existed says, and what most vhosts mean.
        let publish = match m.get(&dat!("publish")) {
            Some(Dat::Map(sub)) => Some(res!(PublishConfig::from_datmap(sub))),
            None => None,
            _ => return Err(err!(
                "VhostConfig: 'publish' must be a map.";
                Invalid, Input, Mismatch)),
        };
        // A list and a vek are both written as a list of strings, so both are
        // read, as everywhere else a list of strings is accepted here.
        let site_admins = match m.get(&dat!("site_admins")) {
            Some(Dat::List(list)) => {
                let mut out = Vec::new();
                for item in list {
                    match item {
                        Dat::Str(s) => out.push(s.clone()),
                        _ => return Err(err!(
                            "VhostConfig: 'site_admins' entries must be strings.";
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
                            "VhostConfig: 'site_admins' entries must be strings.";
                            Invalid, Input, Mismatch)),
                    }
                }
                out
            }
            None => Vec::new(),
            _ => return Err(err!(
                "VhostConfig: 'site_admins' must be a list of strings.";
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
            ws_routes,
            term_config,
            publish,
            site_admins,
        })
    }

    /// Every upstream this vhost's configuration can reach outward to, as
    /// `(kind, local path, host, port)`, where the kind and path name the
    /// route in an error message. A route served by an in-process handler
    /// reaches nothing and does not appear.
    ///
    /// This is the list `egress_allowed` is enforced against, and it is the
    /// only such list: a route kind added to [`VhostConfig`] without a line
    /// here is outside the allow-list, so add the line with the field.
    pub fn egress_targets(&self) -> Vec<(&'static str, &str, &str, u16)> {
        let mut out = Vec::new();
        for r in &self.api_routes {
            if let (Some(h), Some(p)) = (&r.upstream_host, &r.upstream_port) {
                out.push(("api route", r.path.as_str(), h.as_str(), *p));
            }
        }
        // A webhook route in forwarding mode POSTs the payload onward, so it carries a body out.
        for r in &self.webhook_routes {
            if let (Some(h), Some(p)) = (&r.upstream_host, &r.upstream_port) {
                out.push(("webhook route", r.path.as_str(), h.as_str(), *p));
            }
        }
        // Ws and proxy routes always name an upstream; there is no handler form to skip.
        for r in &self.ws_routes {
            out.push(("ws route", r.path.as_str(), r.upstream_host.as_str(), r.upstream_port));
        }
        for r in &self.proxy_routes {
            out.push((
                "proxy route",
                r.path_prefix.as_str(),
                r.upstream_host.as_str(),
                r.upstream_port,
            ));
        }
        out
    }

    /// Does `egress_allowed` permit a connection to this host and port?
    ///
    /// Entries are compared as `host` or `host:port`: a bare-host entry
    /// matches any port for that host, and a `host:port` entry requires an
    /// exact match. An empty list permits everything, since it means no
    /// allow-list was configured. The port is taken from the last colon and
    /// only when what follows it parses as one, so a bracketed IPv6 literal
    /// is read as the bare host it is rather than split down the middle.
    pub fn egress_permits(&self, host: &str, port: u16) -> bool {
        if self.egress_allowed.is_empty() {
            return true;
        }
        for entry in &self.egress_allowed {
            match entry.rsplit_once(':') {
                Some((eh, ep)) => match ep.parse::<u16>() {
                    Ok(n)  => if eh == host && n == port { return true; },
                    Err(_) => if entry == host { return true; },
                },
                None => if entry == host { return true; },
            }
        }
        false
    }

    /// Check every upstream this vhost can reach against the `egress_allowed`
    /// list, refusing the first that no entry permits. A no-op when the
    /// allow-list is empty, which is what a config that configures no
    /// allow-list means. See [`egress_targets`](Self::egress_targets) for
    /// what is covered and [`egress_permits`](Self::egress_permits) for how
    /// an entry is matched.
    pub fn validate_egress(&self) -> Outcome<()> {
        if self.egress_allowed.is_empty() {
            return Ok(());
        }
        for (kind, path, host, port) in self.egress_targets() {
            if !self.egress_permits(host, port) {
                return Err(err!(
                    "VhostConfig '{}': {} '{}' upstream {}:{} is not \
                    in the configured egress_allowed list ({:?}).",
                    self.primary_hostname(), kind, path, host, port,
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
/// challenge on the same port Steel is already listening on. The hostname of
/// an enabled mail listener is included automatically, as are any
/// `extra_domains`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcmeConfig {
    pub enabled:        bool,               // false loads certificates from disk instead
    pub contact_email:  String,             // registered with the ACME account, for notices
    pub directory_url:  String,             // defaults to the Let's Encrypt staging endpoint
    pub cache_dir_rel:  String,             // account key and certificates, from the app root
    // Additional hostnames to name in the certificate, beyond the vhost hostnames and the mail
    // listener. Steel can only issue for a name that resolves to it, but it need not be the
    // service that ultimately serves that name: where another daemon on the same host terminates
    // TLS for a hostname Steel does not route -- an MTA, say -- listing it here puts it in the
    // certificate Steel already renews, and that daemon can be pointed at the result. Without it
    // such a name has no renewal path at all, and the failure is silent until the certificate
    // expires.
    pub extra_domains:  Vec<String>,
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
            extra_domains:  Vec::new(),
        }
    }
}

impl AcmeConfig {
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
        match m.get(&dat!("extra_domains")) {
            Some(Dat::List(l)) => {
                for d in l {
                    if let Dat::Str(s) = d {
                        out.extra_domains.push(s.clone());
                    }
                }
            }
            Some(Dat::Vek(v)) => {
                for d in v.iter() {
                    if let Dat::Str(s) = d {
                        out.extra_domains.push(s.clone());
                    }
                }
            }
            _ => (),
        }
        Ok(out)
    }

    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("enabled"),       dat!(self.enabled));
        m.insert(dat!("contact_email"), dat!(self.contact_email.clone()));
        m.insert(dat!("directory_url"), dat!(self.directory_url.clone()));
        m.insert(dat!("cache_dir_rel"), dat!(self.cache_dir_rel.clone()));
        m.insert(dat!("extra_domains"), Dat::List(
            self.extra_domains.iter().map(|d| dat!(d.clone())).collect()
        ));
        m
    }

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
    pub enabled:            bool,           // false leaves the mail server unstarted
    pub hostname:           String,         // advertised in the greetings; the public MX name
    pub smtp_port:          u16,            // MX receive, standard 25
    pub submission_port:    u16,            // standard 587
    pub imap_port:          u16,            // implicit TLS, standard 993
    pub maildir_root:       String,         // per-user trees live at `<root>/<delivery_dir>/`
    pub users_file_rel:     String,         // JDAT user file: passwords and delivery dirs
    pub spool_dir_rel:      String,         // the outbound spool
    pub dkim_key_file:      String,         // PKCS#8 DER; empty disables DKIM signing
    pub dkim_selector:      String,         // published at `<selector>._domainkey.<domain>`
    /// Path to an RSA DKIM private key (PKCS#8 or PKCS#1 DER). Empty
    /// disables RSA signing.
    ///
    /// Signing with both an ed25519 and an RSA key, under two selectors, is
    /// what RFC 8463 asks for: ed25519 verification is still patchy in the
    /// wild, and a receiver that cannot verify a signature treats the message
    /// as *unsigned*, leaving DMARC to rest on SPF alone.
    ///
    /// Steel will not create this key. `ring` refuses to generate RSA keys,
    /// and hand-rolling the arithmetic to do so is not a road worth taking to
    /// save one command:
    ///
    /// ```text
    /// openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
    ///     -outform DER -out mail/dkim_rsa.key
    /// ```
    pub dkim_rsa_key_file:  String,
    pub dkim_rsa_selector:  String,         // must differ from `dkim_selector`; default "rsa"
    pub dkim_domain:        String,         // may differ from `hostname`
    pub local_domains:      Vec<String>,    // recipients outside this set are refused at RCPT TO
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
            dkim_rsa_key_file:  String::new(),
            dkim_rsa_selector:  String::new(),
            dkim_domain:        String::new(),
            local_domains:      Vec::new(),
        }
    }
}

impl MailConfig {
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
        if let Some(Dat::Str(s)) = m.get(&dat!("dkim_rsa_key_file")) {
            out.dkim_rsa_key_file = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("dkim_rsa_selector")) {
            out.dkim_rsa_selector = s.clone();
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
// │ ALERT CONFIG                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Where an alert is posted, when it goes through a provider rather than
/// straight to the recipient's MX.
///
/// # Why this is usually the right choice
///
/// Delivering directly to the recipient's MX means a receiver decides whether
/// to trust a message that arrived, unannounced and unauthenticated, from a
/// server it has never heard of. Without a PTR record for the sending IP and
/// an SPF record naming it, a strict receiver -- Gmail, for one -- is entitled
/// to bin it. The message that says something is wrong is exactly the one that
/// must not land in a spam folder.
///
/// Submitting through the sender's own provider authenticates the sender, and
/// the provider's reputation carries the message the rest of the way. It also
/// serves the rule that the machine raising the alarm should not be the only
/// machine on the path.
///
/// # The credential must be readable while sealed
///
/// It cannot live in the wallet's encrypted secrets, because the most
/// important alert of all is the one saying Steel came up *sealed* -- and at
/// that moment there is no master key with which to decrypt anything. So the
/// password is a plain config value, and should be given as a `{file:...}`
/// reference to a file the server user alone can read, rather than written
/// into `config.jdat` in the clear.
#[derive(Clone, Debug)]
pub struct AlertSubmission {
    pub host:       String,         // also the name its certificate is validated against
    pub port:       u16,            // conventionally 587 (STARTTLS) or 465 (implicit TLS)
    pub security:   String,         // "starttls", "implicit", or "plain" (loopback only)
    pub user:       String,         // the account to authenticate as
    // That account's password. Supply as `{file:path}`; a provider with two-factor
    // authentication wants an application password here, not the one a human types into a
    // browser.
    pub password:   String,
}

/// Operator alerting by email. See `srv::alert`.
///
/// With no `submission` block, mail is delivered straight to the recipient's
/// MX by the in-tree SMTP client, so no relay and no local mail daemon are
/// required -- but deliverability then rests on this host having a PTR record
/// and the `from` domain having an SPF record that names it. With a
/// `submission` block, the message is posted through the sender's own
/// provider, which authenticates it. See [`AlertSubmission`].
#[derive(Clone, Debug)]
pub struct AlertConfig {
    pub enabled:                bool,                   // false sends no alert, ever
    pub from:                   String,                 // use a domain whose SPF names this host
    pub submission:             Option<AlertSubmission>,    // a provider, not the MX direct
    // Recipients. Address these off this machine: an alert delivered to a mailbox on the host it
    // is warning about is one the operator cannot read precisely when they need to.
    pub to:                     Vec<String>,
    pub ehlo_hostname:          String,                 // the name whose PTR matches the IP
    pub failed_threshold:       u32,                    // failures in the window before alerting
    // Window over which failures are counted. Failures further apart than this start a fresh
    // count, so a slow trickle does not eventually add up to something that reads as an attack.
    pub failed_window_secs:     u64,
    // Minimum gap between two failed-attempt alerts, so a sustained campaign produces a
    // sustained defence rather than a sustained mailbox.
    pub failed_cooldown_secs:   u64,
    // Where to send the text-message half, when there is one. Absent on most hosts: it belongs
    // on whichever machines do the watching, because a machine that has died cannot text
    // anybody about it.
    pub sms:                    Option<SmsAlertConfig>,
}

/// The text-message leg of alerting.
///
/// **Why a second channel at all.** Mail and a text fail for different reasons:
/// mail needs a working MX, a mailbox somebody reads and a spam filter that
/// lets it through; a text needs a funded account and a carrier. Two channels
/// that fail independently is the entire value of having two. A text is also
/// the only one that arrives with no data connection, which is the state a
/// phone is in exactly often enough to matter.
///
/// **The credential is not here.** Only the names of the environment variables
/// holding it. A configuration file is copied between machines, pasted into a
/// chat window to ask why a server will not start, and committed by accident;
/// an environment variable is none of those things by default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsAlertConfig {
    pub enabled:    bool,               // this leg alone; mail is unaffected
    pub provider:   SmsProvider,        // which gateway
    pub to:         Vec<String>,        // E.164, with the leading `+`
    // Sender, as the gateway wants it. Empty asks the gateway for its default, which is what an
    // account with one number should do rather than repeat itself in configuration.
    pub from:       String,
    pub user_env:   String,             // env var holding the gateway account identifier
    pub secret_env: String,             // env var holding the gateway secret
}

impl Default for SmsAlertConfig {
    fn default() -> Self {
        Self {
            enabled:    false,
            provider:   SmsProvider::ClickSend,
            to:         Vec::new(),
            from:       String::new(),
            user_env:   fmt!("STEEL_SMS_USER"),
            secret_env: fmt!("STEEL_SMS_SECRET"),
        }
    }
}

/// One machine this node watches, and where to ask.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchPeer {
    // What to call it in an alert: a person's name for the machine, not a hostname, since the
    // alert is read on a phone in the dark.
    pub name:     String,
    pub url:      String,   // the health URL, `https` unless `plain_ok` is set
    // Whether a plain `http` URL is acceptable for this one peer. Off unless the operator
    // writes it, and never a global switch: see `crate::srv::watch` for the single case it is
    // meant for.
    pub plain_ok: bool,
}

/// Watching the other machines in the estate.
///
/// See [`crate::srv::watch`] for why this is a mesh of peers rather than a
/// monitoring server, and why adding a machine is one line here and nothing
/// else anywhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchConfig {
    pub enabled:        bool,
    // The machines this node watches, not including itself: a node cannot report its own death,
    // which is the whole premise.
    pub peers:          Vec<WatchPeer>,
    pub interval_secs:  u64,                // seconds between rounds
    pub fail_threshold: u32,                // consecutive failures before a peer is called down
    pub timeout_secs:   u64,                // seconds to wait for a health answer
    // Seconds between reminders while a peer stays down. An alarm that fires every round is an
    // alarm that gets silenced, and the text leg costs money per message.
    pub repeat_secs:    u64,
    // Seconds between proof-of-life messages; zero switches them off. The alerting path is used
    // rarely by design, and a path used rarely is broken when it is needed -- an expired
    // credential, a rotated key, a changed number, a lapsed verification. This exercises every
    // leg on a schedule, so the failure is found on an ordinary afternoon.
    pub heartbeat_secs: u64,
}

/// Every string in a named list, whatever list shape the daticle used.
///
/// A jdat list can arrive as `Dat::List` or, when it was written with the `vek`
/// type tag, as `Dat::Vek`. They mean the same thing to a reader and a
/// configuration file uses whichever its author typed -- so a parser that knows
/// only one of them silently reads an empty list from a file that plainly has
/// entries in it. That cost a live deploy: `alerts.sms.to` was written in the
/// same `(vek|[...])` form as `alerts.to` beside it, and the SMS half read
/// nothing and refused to start.
fn strings_in(m: &DaticleMap, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, d: &Dat| {
        if let Dat::Str(s) = d {
            out.push(s.clone());
        }
    };
    match m.get(&dat!(key)) {
        Some(Dat::List(l))  => for d in l { push(&mut out, d); },
        Some(Dat::Vek(v))   => for d in v.iter() { push(&mut out, d); },
        _                   => {},
    }
    out
}

impl SmsAlertConfig {
    /// Parse an `SmsAlertConfig` from a `DaticleMap`.
    ///
    /// Every field falls back to the default, so an existing configuration that
    /// has never heard of this block keeps loading unchanged. What is *not*
    /// tolerated is an enabled block that cannot work: a gateway nobody
    /// recognises, or nobody to text. Both are refused at start-up, because the
    /// alternative is discovering them on the night the alert was needed.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let mut out = Self::default();
        if let Some(Dat::Bool(b)) = m.get(&dat!("enabled")) {
            out.enabled = *b;
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("provider")) {
            out.provider = res!(SmsProvider::from_id(s).ok_or_else(|| err!(
                "alerts.sms.provider is '{}'. Known gateways: {}.",
                s, SmsProvider::ALL.iter().map(|p| p.id())
                    .collect::<Vec<_>>().join(", ");
                Configuration, Invalid)));
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("from")) {
            out.from = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("user_env")) {
            out.user_env = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("secret_env")) {
            out.secret_env = s.clone();
        }
        out.to = strings_in(m, "to");
        if out.enabled {
            if out.to.is_empty() {
                return Err(err!(
                    "alerts.sms is enabled with no numbers in 'to'. An alerter \
                    with nobody to tell is worse than none, because it looks \
                    like cover.";
                    Configuration, Invalid, Missing));
            }
            // Checked here rather than at the first alert. A number that a
            // gateway will refuse is a text that never arrives, and the moment
            // it is discovered would otherwise be an outage at three in the
            // morning.
            for n in &out.to {
                if !oxedyne_fe2o3_net::sms::is_e164(n) {
                    return Err(err!(
                        "alerts.sms.to contains {:?}, which is not E.164. It \
                        needs a leading '+' and the country code, e.g. \
                        '+61400000000' -- every gateway refuses anything else, \
                        so this would be a text that never arrived.", n;
                        Configuration, Invalid, Input));
                }
            }
        }
        Ok(out)
    }
}

impl WatchConfig {
    /// Parse a `WatchConfig` from a `DaticleMap`.
    ///
    /// A peer with no name or no URL is refused rather than skipped: a watch
    /// list that silently watches four machines out of five is the failure this
    /// whole module exists to prevent.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let mut out = Self::default();
        if let Some(Dat::Bool(b)) = m.get(&dat!("enabled")) {
            out.enabled = *b;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("interval_secs")) {
            out.interval_secs = *n;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("timeout_secs")) {
            out.timeout_secs = *n;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("repeat_secs")) {
            out.repeat_secs = *n;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("heartbeat_secs")) {
            out.heartbeat_secs = *n;
        }
        if let Some(Dat::U32(n)) = m.get(&dat!("fail_threshold")) {
            out.fail_threshold = *n;
        }
        if let Some(Dat::List(l)) = m.get(&dat!("peers")) {
            for (i, d) in l.iter().enumerate() {
                let pm = match d {
                    Dat::Map(pm) => pm,
                    _ => return Err(err!(
                        "watch.peers entry {} is not a map.", i;
                        Configuration, Invalid, Input)),
                };
                let get = |k: &str| -> String {
                    match pm.get(&dat!(k)) {
                        Some(Dat::Str(v)) => v.clone(),
                        _ => String::new(),
                    }
                };
                let name = get("name");
                let url = get("url");
                if name.is_empty() || url.is_empty() {
                    return Err(err!(
                        "watch.peers entry {} needs both a 'name' and a 'url'.", i;
                        Configuration, Invalid, Missing));
                }
                // Absent means false, so every peer written before this key existed keeps
                // demanding TLS, which is the answer a silent config should give.
                let plain_ok = matches!(pm.get(&dat!("plain_ok")), Some(Dat::Bool(true)));
                out.peers.push(WatchPeer { name, url, plain_ok });
            }
        }
        if out.enabled {
            if out.peers.is_empty() {
                return Err(err!(
                    "watch is enabled with no peers. A watcher watching nothing \
                    looks like cover and is not.";
                    Configuration, Invalid, Missing));
            }
            if out.fail_threshold == 0 {
                return Err(err!(
                    "watch.fail_threshold is 0, which would call a machine down \
                    on a single dropped packet.";
                    Configuration, Invalid, Range));
            }
        }
        Ok(out)
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            enabled:        false,
            peers:          Vec::new(),
            interval_secs:  60,
            // Three, against a sixty second round: a machine is called down
            // after roughly three minutes of not answering. Long enough that a
            // restart or a certificate renewal does not raise the alarm, short
            // enough that fifty minutes of silence cannot happen again.
            fail_threshold: 3,
            timeout_secs:   10,
            repeat_secs:    900,
            // Monthly. Often enough that a dead path is caught before it
            // matters, rare enough that the message stays worth reading.
            heartbeat_secs: 2_592_000,
        }
    }
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled:                false,
            from:                   String::new(),
            submission:             None,
            to:                     Vec::new(),
            ehlo_hostname:          String::new(),
            failed_threshold:       5,
            failed_window_secs:     900,    // 15 minutes
            failed_cooldown_secs:   3_600,  // 1 hour
            sms:                    None,
        }
    }
}

impl AlertConfig {
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let mut out = Self::default();
        if let Some(Dat::Bool(b)) = m.get(&dat!("enabled")) {
            out.enabled = *b;
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("from")) {
            out.from = s.clone();
        }
        if let Some(Dat::Str(s)) = m.get(&dat!("ehlo_hostname")) {
            out.ehlo_hostname = s.clone();
        }
        if let Some(Dat::Map(sm)) = m.get(&dat!("submission")) {
            let get_str = |k: &str| -> String {
                match sm.get(&dat!(k)) {
                    Some(Dat::Str(v)) => v.clone(),
                    _ => String::new(),
                }
            };
            let host = get_str("host");
            if host.is_empty() {
                return Err(err!(
                    "alerts.submission is present but has no 'host'.";
                    Configuration, Invalid, Missing));
            }
            let port = match sm.get(&dat!("port")) {
                Some(Dat::U16(n)) => *n,
                _ => 587,
            };
            let security = match get_str("security").as_str() {
                "" => fmt!("starttls"),
                other => other.to_string(),
            };
            match security.as_str() {
                "starttls" | "implicit" | "plain" => (),
                other => return Err(err!(
                    "alerts.submission.security is '{}'; expected 'starttls', \
                    'implicit' or 'plain'.", other;
                    Configuration, Invalid)),
            }
            out.submission = Some(AlertSubmission {
                host,
                port,
                security,
                user:       get_str("user"),
                password:   get_str("password"),
            });
        }
        out.to = strings_in(m, "to");
        if let Some(Dat::U32(n)) = m.get(&dat!("failed_threshold")) {
            out.failed_threshold = *n;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("failed_window_secs")) {
            out.failed_window_secs = *n;
        }
        if let Some(Dat::U64(n)) = m.get(&dat!("failed_cooldown_secs")) {
            out.failed_cooldown_secs = *n;
        }
        if out.failed_threshold == 0 {
            return Err(err!(
                "alerts.failed_threshold is 0, which would raise an alert on \
                every failed attempt and turn the alerter into an amplifier \
                pointed at the operator's mailbox.";
                Configuration, Invalid, Range));
        }
        if let Some(Dat::Map(sm)) = m.get(&dat!("sms")) {
            out.sms = Some(res!(SmsAlertConfig::from_datmap(sm)));
        }
        Ok(out)
    }

    /// Expand `{file:path}` and `{env:VAR}` placeholders in the submission
    /// credential, so the password need not be written into `config.jdat`
    /// in the clear.
    ///
    /// Not the wallet's encrypted secrets: the alert that matters most is the
    /// one saying Steel came up *sealed*, and at that moment there is no
    /// master key to decrypt anything with. The credential has to be readable
    /// before the wallet is open, which means a file the server user alone
    /// can read.
    pub fn resolve_secrets(&mut self, root: &Path) -> Outcome<()> {
        if let Some(sub) = &mut self.submission {
            sub.password = res!(ApiRoute::resolve_file_refs(&sub.password, root));
            sub.user = res!(ApiRoute::resolve_file_refs(&sub.user, root));
        }
        Ok(())
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
    // Directory holding per-vhost certificates when ACME is disabled, relative to the app root.
    // Each vhost's certs live in `{tls_dir_rel}/{dev|prod}/{primary_hostname}/fullchain.pem` and
    // `privkey.pem`.
    pub tls_dir_rel:                    String,

    // --- Server bind and policy (shared) ------------------------------------
    pub log_level:                      String,         // used by the server once running
    pub server_address:                 String,         // typically "0.0.0.0"
    pub server_port_tcp:                u16,            // the primary HTTPS port
    // Optional plaintext HTTP listener port. When non-zero, Steel binds this port too and
    // answers every incoming HTTP request with a `301 Moved Permanently` to the equivalent HTTPS
    // URL on the primary port. Typically 80 in production and 0 in local development. Defaults
    // to 0.
    pub server_port_tcp_plaintext:      u16,
    // `Strict-Transport-Security` `max-age` in seconds, injected into every HTTPS response when
    // non-zero. 31536000, one year, is conventional for production. Defaults to 0, no HSTS.
    pub hsts_max_age_secs:              u32,
    // `Cache-Control` `max-age` in seconds for static assets, which is how long a browser may
    // reuse one without asking. Entry documents are excluded and always revalidate, since a
    // deploy that changes one is invisible to anyone still holding the old copy. Raise this above
    // zero only when asset filenames carry a content hash: an asset cached under a stable name
    // outlives the deploy that replaced it. Defaults to 0, which revalidates everything -- cheap,
    // because the entity tag turns an unchanged asset into a bodiless 304.
    #[optional]
    pub static_max_age_secs:            u32,
    // `Cache-Control` `max-age` in seconds for an asset whose filename carries a content hash,
    // which is a promise that the file cannot change under that name. Such a response also says
    // `immutable`, so a browser does not revalidate it even on a manual reload. Entry documents
    // are excluded whatever their name. Defaults to one year, the conventional value and the
    // longest RFC 9111 5.2.2.1 suggests anyone use. Set to 0 if a build here emits hash-shaped
    // names that it then overwrites in place, which would otherwise leave a browser holding a
    // stale copy for a year.
    #[optional]
    pub fingerprint_max_age_secs:       u32,
    // Whether to encode eligible responses with gzip when the client says it will accept one.
    // Markup, script, stylesheets, JSON, SVG and WebAssembly typically go out at a third to a half
    // of their raw weight; formats that carry their own compression are never encoded twice.
    // Defaults to true.
    #[optional]
    pub compression_enabled:            bool,
    // Smallest response body, in bytes, worth encoding. A gzip member costs eighteen bytes of
    // framing before it encodes anything, so under about a kilobyte the saving is noise. Defaults
    // to 1024.
    #[optional]
    pub compression_min_bytes:          u64,
    // Optional plaintext HTTP listener bound to `127.0.0.1` for the admin dashboard only. When
    // non-zero, Steel binds this port on the loopback interface and serves the `/admin/*` routes
    // without TLS: SSH-tunnel to the host and reach the dashboard without going through the
    // public TLS chain, which is what an expired cert, a broken ACME or an emergency needs.
    // Anything other than `/admin*` returns 404. Defaults to 0, disabled.
    #[optional]
    pub admin_local_port:               u16,
    pub session_expiry_default_secs:    u32,            // seconds
    pub ws_ping_interval_secs:          u8,             // seconds
    pub server_max_errors_allowed:      u8,             // consecutive, on one connection
    // Whether to issue a session cookie to unauthenticated clients on first contact. When true,
    // Steel generates a fresh session id for any incoming request that does not already carry
    // one and attaches it as an `HttpOnly`, `Secure`, `SameSite=Lax` cookie, which is what makes
    // session-scoped WebSocket commands work for anonymous browsers. When false, requests
    // without a session cookie are still served, but session-scoped commands reject until the
    // client obtains a session id some other way.
    pub allow_anonymous_sessions:       bool,
    // ── Hardening knobs ───────────────────────────────────────────────────
    //
    // Every field below is `#[optional]` so on-disk configs from
    // earlier Steel builds continue to load. Missing fields fall
    // through to the Default impl (which reproduces the pre-feature
    // "permissive" behaviour for each: no size/time limits, headers
    // enabled, empty CSP, empty guard block).

    // Maximum bytes accepted in the HTTP request header block before the reader returns `413
    // Content Too Large`. Zero disables the limit.
    #[optional]
    pub http_max_header_bytes:          u64,
    // Maximum bytes accepted in the HTTP request body before the reader returns `413 Content Too
    // Large`. Zero disables the limit.
    #[optional]
    pub http_max_body_bytes:            u64,
    // Wall-clock budget for the HTTP header read phase, in milliseconds. A slow client that fails
    // to finish sending its header block within this window is disconnected with a `Timeout`
    // error. Zero disables the deadline.
    #[optional]
    pub http_header_read_timeout_ms:    u64,
    // When true, Steel injects a baseline set of security response headers into every HTTPS
    // response: `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`,
    // `Permissions-Policy`.
    #[optional]
    pub security_headers_enabled:       bool,
    #[optional]
    pub content_security_policy:        String,         // empty sends no CSP header
    #[optional]
    pub addr_guard:                     DaticleMap,     // empty map restores the defaults
    // URL path prefixes routed through the tighter auth-path rate limiter.
    #[optional]
    pub auth_path_prefixes:             Vec<String>,
    // Maximum average requests per second permitted against the auth path prefixes.
    #[optional]
    pub auth_rps_max:                   u64,
    // The immediate peers entitled to speak the forwarding headers -- `X-Forwarded-For`,
    // `X-Forwarded-Proto`, `X-Forwarded-Host` and RFC 7239 `Forwarded` -- written either as a
    // bare address, `198.51.100.7`, or as a prefix, `198.51.100.0/24`.
    //
    // Empty means trust nobody, which means a caller's copies of those headers are stripped
    // before Steel appends its own. That is the default, and it is the correct setting for a host
    // facing the public directly: nothing sits in front of Steel there, so nothing in front of
    // Steel is entitled to name the client.
    //
    // Deleting this field, or emptying a populated one, is not tidying. It reads like hardening a
    // later reader can drop, and it is the opposite. A forged `X-Forwarded-For` copied through
    // arrives ahead of Steel's own, and the obvious way to read a repeated header --
    // `HeaderFields::get_one` -- returns the first. An upstream address guard keyed on that
    // counts a fresh allowance for every fresh invented address, so it is not a weaker limit but
    // no limit at all, while looking configured. A forged `X-Forwarded-Proto: http` read the same
    // way tells an upstream that a TLS request arrived in plaintext, and an upstream that
    // redirects plaintext to HTTPS on that basis loops.
    //
    // Populate it only when something really does sit in front -- a CDN, a load balancer --
    // naming that thing's egress addresses. Stripping unconditionally would then discard the real
    // client address rather than preserve it, replacing every client with the CDN's egress, which
    // is the same bug wearing a safer face. When the peer is named here the caller's chain is
    // preserved and Steel's value appended to it; Steel's own value is last in either case, which
    // is why an upstream should read these headers with `HeaderFields::get_last`.
    //
    // Entries are parsed at start-up, so a typo is a start-up failure rather than a silently
    // empty allow-list. The policy itself lives in `oxedyne_fe2o3_net::http::fwd`; what stays
    // here is the configuration.
    #[optional]
    pub trusted_proxies:                Vec<String>,

    // --- Virtual hosts ------------------------------------------------------
    // Stored as a `Dat::List` of `Dat::Map` entries and parsed via `get_vhosts()`.
    pub vhosts:                         Dat,

    // --- ACME ---------------------------------------------------------------
    pub acme:                           DaticleMap,     // parsed via `get_acme()`

    // --- Mail ---------------------------------------------------------------
    // Parsed via `get_mail()`. An empty map disables the mail server entirely.
    pub mail:                           DaticleMap,

    // --- Alerts -------------------------------------------------------------
    // Operator alerting, parsed via `get_alerts()`. Absent, or an empty map, disables alerting
    // entirely.
    //
    // `#[optional]` because a config block for a feature nobody has switched on must not be
    // mandatory. Without it, `from_datmap` treats the field as required and every existing
    // `config.jdat` in the world becomes invalid the moment a new block is added to this struct
    // -- which is a fine way to take a production server down while adding a feature it does not
    // even use. Any block added here in future should be `#[optional]` too.
    #[optional]
    pub alerts:                         DaticleMap,

    // --- Watch --------------------------------------------------------------
    // The other machines this node watches, parsed via `get_watch()`. Absent, or an empty map,
    // means this node watches nobody -- which is the right default, since most hosts in an estate
    // are watched rather than watching.
    //
    // `#[optional]`, per the note above, and this block is the reason that note was worth writing
    // down: it was added to a struct backing two live production configurations that had never
    // heard of it.
    #[optional]
    pub watch:                          DaticleMap,
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
            server_address:                 fmt!("0.0.0.0"),
            server_port_tcp:                8443,
            server_port_tcp_plaintext:      0,      // disabled by default
            hsts_max_age_secs:              0,      // disabled by default
            static_max_age_secs:            0,      // revalidate every asset
            fingerprint_max_age_secs:       31_536_000, // one year, for a hashed name
            compression_enabled:            true,
            compression_min_bytes:          encoding::MIN_BYTES_DEFAULT as u64,
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
            trusted_proxies:                Vec::new(), // Trust nobody: always strip.
            vhosts:                         Dat::List(vec![Dat::Map(vhost_map)]),
            acme:                           AcmeConfig::default().to_datmap(),
            mail:                           DaticleMap::new(),
            alerts:                         DaticleMap::new(),
            watch:                          DaticleMap::new(),
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
            // Egress allow-list check: a vhost naming an upstream
            // outside its configured allow-list is refused at
            // start-up, whichever kind of route names it. The check
            // is a no-op when the allow-list is empty.
            res!(vh.validate_egress());
        }
        let _ = res!(self.get_acme());
        // A mistyped trusted proxy must be a start-up failure. An entry that failed to parse and
        // was skipped would leave an allow-list that looks populated and trusts nobody -- or, read
        // the other way round, an operator who believes their CDN is named here when it is not.
        let _ = res!(self.get_forwarded_policy());
        Ok(())
    }

    /// Which immediate peers are entitled to speak the forwarding headers.
    ///
    /// See [`trusted_proxies`](Self::trusted_proxies). An empty list yields a policy that trusts
    /// nobody, which strips every caller-supplied forwarding header.
    pub fn get_forwarded_policy(&self) -> Outcome<ForwardedPolicy> {
        ForwardedPolicy::new(&self.trusted_proxies)
    }

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

    /// The mail config regardless of `enabled`.
    ///
    /// [`get_mail`](Self::get_mail) gates on `enabled` because it answers "should the mail *server*
    /// (the listeners) start?". Sending a newsletter is a different question: a site can hold a DKIM
    /// identity and an outbound client to send with, without binding SMTP-receive/submission/IMAP and
    /// becoming an MX. So the newsletter sender reads the block here, `enabled` or not, and is built
    /// whenever there is a hostname and a signing key.
    pub fn get_mail_any(&self) -> Outcome<Option<MailConfig>> {
        if self.mail.is_empty() {
            return Ok(None);
        }
        Ok(Some(res!(MailConfig::from_datmap(&self.mail))))
    }

    /// The watch block, parsed, or `None` when this node watches nobody.
    ///
    /// See [`crate::srv::watch`]: most hosts in an estate are watched rather
    /// than watching, so absent is the ordinary answer.
    pub fn get_watch(&self) -> Outcome<Option<WatchConfig>> {
        if self.watch.is_empty() {
            return Ok(None);
        }
        let cfg = res!(WatchConfig::from_datmap(&self.watch));
        if !cfg.enabled {
            return Ok(None);
        }
        Ok(Some(cfg))
    }

    /// Parse the `alerts` block. An empty map, or an `enabled: false` map, disables alerting.
    pub fn get_alerts(&self) -> Outcome<Option<AlertConfig>> {
        if self.alerts.is_empty() {
            return Ok(None);
        }
        let cfg = res!(AlertConfig::from_datmap(&self.alerts));
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


#[cfg(test)]
mod tests {
    use super::*;

    /// A field added to `ServerConfig` without `#[optional]` invalidates every
    /// config file already on disk, which is an outage rather than a feature.
    /// The compression and fingerprint settings are new, so a config written
    /// before they existed must still load and must come up with the defaults.
    ///
    /// The map below still names `num_server_bots`, which is no longer a field,
    /// and that is deliberate: `from_datmap` reads the keys it knows and does
    /// not object to the rest, so this is also the check that REMOVING a field
    /// leaves every config already written for it loading unchanged. Nine
    /// `config.jdat` files across the apps set that key and none needs editing.
    #[test]
    fn a_config_written_before_these_fields_existed_still_loads() -> Outcome<()> {
        let mut m = DaticleMap::new();
        m.insert(dat!("tls_dir_rel"),                 dat!("./tls"));
        m.insert(dat!("log_level"),                   dat!("debug"));
        m.insert(dat!("num_server_bots"),             Dat::U16(1));
        m.insert(dat!("server_address"),              dat!("0.0.0.0"));
        m.insert(dat!("server_port_tcp"),             Dat::U16(8443));
        m.insert(dat!("server_port_tcp_plaintext"),   Dat::U16(0));
        m.insert(dat!("hsts_max_age_secs"),           Dat::U32(0));
        m.insert(dat!("session_expiry_default_secs"), Dat::U32(604_800));
        m.insert(dat!("ws_ping_interval_secs"),       Dat::U8(30));
        m.insert(dat!("server_max_errors_allowed"),   Dat::U8(30));
        m.insert(dat!("allow_anonymous_sessions"),    Dat::Bool(true));
        m.insert(dat!("vhosts"),                      Dat::List(Vec::new()));
        m.insert(dat!("acme"),                        Dat::Map(DaticleMap::new()));
        m.insert(dat!("mail"),                        Dat::Map(DaticleMap::new()));

        let cfg = res!(ServerConfig::from_datmap(m));
        assert!(cfg.compression_enabled,
            "compression must be on for a config that says nothing about it");
        assert_eq!(cfg.compression_min_bytes, 1024);
        assert_eq!(cfg.fingerprint_max_age_secs, 31_536_000);
        Ok(())
    }

    /// A session identifier is a key prefix into the vhost's own database, so a
    /// vhost without one has nowhere to keep a session and issues none.
    #[test]
    fn only_a_vhost_with_a_database_keeps_sessions() {
        let mut vh = VhostConfig::default();
        assert!(vh.uses_sessions(), "the default vhost is configured with a database");
        vh.db_dir_rel = None;
        assert!(!vh.uses_sessions(), "a static vhost has nowhere to keep a session");
    }

    fn vh(allowed: &[&str]) -> VhostConfig {
        let mut vh = VhostConfig::default();
        vh.egress_allowed = allowed.iter().map(|s| s.to_string()).collect();
        vh
    }

    fn api_up(path: &str, host: &str, port: u16) -> ApiRoute {
        ApiRoute {
            path:           path.to_string(),
            upstream_host:  Some(host.to_string()),
            upstream_port:  Some(port),
            upstream_path:  Some(fmt!("/")),
            upstream_tls:   true,
            headers:        Vec::new(),
            handler:        None,
            config:         Vec::new(),
        }
    }

    fn api_handler(path: &str) -> ApiRoute {
        ApiRoute {
            path:           path.to_string(),
            upstream_host:  None,
            upstream_port:  None,
            upstream_path:  None,
            upstream_tls:   true,
            headers:        Vec::new(),
            handler:        Some(fmt!("some_handler")),
            config:         Vec::new(),
        }
    }

    fn hook_up(path: &str, host: &str, port: u16) -> WebhookRoute {
        WebhookRoute {
            path:           path.to_string(),
            handler:        None,
            upstream_host:  Some(host.to_string()),
            upstream_port:  Some(port),
            upstream_path:  Some(fmt!("/")),
            upstream_tls:   true,
            config:         Vec::new(),
        }
    }

    fn hook_handler(path: &str) -> WebhookRoute {
        WebhookRoute {
            path:           path.to_string(),
            handler:        Some(fmt!("some_handler")),
            upstream_host:  None,
            upstream_port:  None,
            upstream_path:  None,
            upstream_tls:   true,
            config:         Vec::new(),
        }
    }

    fn ws_up(path: &str, host: &str, port: u16) -> WsRoute {
        WsRoute {
            path:           path.to_string(),
            upstream_host:  host.to_string(),
            upstream_port:  port,
            upstream_path:  fmt!("/"),
        }
    }

    fn proxy_up(prefix: &str, host: &str, port: u16) -> ProxyRoute {
        ProxyRoute {
            path_prefix:    prefix.to_string(),
            upstream_host:  host.to_string(),
            upstream_port:  port,
            upstream_tls:   false,
            strip_prefix:   false,
        }
    }

    /// The route kind whose enforcement has always worked, kept here so a
    /// rewrite of the check cannot quietly drop it.
    #[test]
    fn an_api_route_outside_the_allowlist_is_refused() {
        let mut v = vh(&["127.0.0.1"]);
        v.api_routes = vec![api_up("/api/pay", "evil.example.com", 443)];
        assert!(v.validate_egress().is_err(),
            "an api_route to a host the operator did not name must be refused");
    }

    /// A ws_route dials an upstream of its own, so an operator's allow-list
    /// that does not name that upstream must refuse it.
    #[test]
    fn a_ws_route_outside_the_allowlist_is_refused() -> Outcome<()> {
        let mut v = vh(&["127.0.0.1"]);
        v.ws_routes = vec![ws_up("/ws", "evil.example.com", 9000)];
        let e = res!(v.validate_egress().err().ok_or_else(|| err!(
            "a ws_route to a host the operator did not name must be refused"; Test)));
        let msg = fmt!("{}", e);
        assert!(msg.contains("evil.example.com"),
            "the refusal must name the host it refused, got: {}", msg);
        Ok(())
    }

    /// A proxy_route forwards a whole path prefix to its own upstream, which
    /// is the broadest outward reach a vhost has.
    #[test]
    fn a_proxy_route_outside_the_allowlist_is_refused() {
        let mut v = vh(&["127.0.0.1"]);
        v.proxy_routes = vec![proxy_up("/chat/", "evil.example.com", 8080)];
        assert!(v.validate_egress().is_err(),
            "a proxy_route to a host the operator did not name must be refused");
    }

    /// A webhook route in forwarding mode POSTs the payload onward, which is
    /// egress carrying a body.
    #[test]
    fn a_forwarded_webhook_outside_the_allowlist_is_refused() {
        let mut v = vh(&["127.0.0.1"]);
        v.webhook_routes = vec![hook_up("/webhook/pay", "evil.example.com", 443)];
        assert!(v.validate_egress().is_err(),
            "a forwarded webhook to a host the operator did not name must be refused");
    }

    /// A refusal that catches a legitimate configuration is a refusal an
    /// operator switches off, so a vhost with no allow-list at all keeps
    /// reaching every upstream it names.
    #[test]
    fn no_allowlist_permits_every_upstream() -> Outcome<()> {
        let mut v = vh(&[]);
        v.api_routes     = vec![api_up("/api/pay", "api.example.com", 443)];
        v.webhook_routes = vec![hook_up("/webhook/pay", "hooks.example.com", 443)];
        v.ws_routes      = vec![ws_up("/ws", "ws.example.com", 9000)];
        v.proxy_routes   = vec![proxy_up("/chat/", "chat.example.com", 8080)];
        res!(v.validate_egress());
        Ok(())
    }

    /// An allow-list that names every upstream permits them all, in both
    /// entry forms: a bare host for any port, and `host:port` for one.
    #[test]
    fn a_correct_allowlist_permits_every_upstream() -> Outcome<()> {
        let mut v = vh(&["127.0.0.1", "api.example.com", "ws.example.com:9000"]);
        v.api_routes     = vec![
            api_up("/api/pay", "api.example.com", 443),
            api_handler("/api/local"),
        ];
        v.webhook_routes = vec![
            hook_up("/webhook/pay", "127.0.0.1", 7000),
            hook_handler("/webhook/local"),
        ];
        v.ws_routes      = vec![ws_up("/ws", "ws.example.com", 9000)];
        v.proxy_routes   = vec![proxy_up("/chat/", "127.0.0.1", 8080)];
        res!(v.validate_egress());
        Ok(())
    }

    /// A `host:port` entry is the operator saying one port and not another.
    #[test]
    fn a_port_specific_entry_refuses_another_port() {
        let mut v = vh(&["ws.example.com:9000"]);
        v.ws_routes = vec![ws_up("/ws", "ws.example.com", 9001)];
        assert!(v.validate_egress().is_err(),
            "an entry naming port 9000 must not permit port 9001");
    }

    /// Every outward reach the configuration has, so a route kind added later
    /// without a line in `egress_targets` is a failing test rather than a
    /// silent hole in the allow-list.
    #[test]
    fn the_enumeration_covers_all_four_route_kinds() {
        let mut v = vh(&[]);
        v.api_routes     = vec![api_up("/api/pay", "a.example.com", 443), api_handler("/api/x")];
        v.webhook_routes = vec![hook_up("/webhook/pay", "b.example.com", 443), hook_handler("/w")];
        v.ws_routes      = vec![ws_up("/ws", "c.example.com", 9000)];
        v.proxy_routes   = vec![proxy_up("/chat/", "d.example.com", 8080)];
        let hosts: Vec<&str> = v.egress_targets().iter().map(|t| t.2).collect();
        assert_eq!(
            hosts,
            vec!["a.example.com", "b.example.com", "c.example.com", "d.example.com"],
            "every route with an upstream must be enumerated, and only those");
    }

    /// An IPv6 upstream is written bracketed, and the brackets contain colons:
    /// an allow-list entry read as `host:port` at the first colon it finds
    /// never matches one, which is an allow-list that silently allows nothing.
    #[test]
    fn a_bracketed_ipv6_entry_is_a_host_not_a_host_and_port() -> Outcome<()> {
        let mut v = vh(&["[::1]"]);
        v.ws_routes = vec![ws_up("/ws", "[::1]", 9000)];
        res!(v.validate_egress());
        v.ws_routes = vec![ws_up("/ws", "[2001:db8::1]", 9000)];
        assert!(v.validate_egress().is_err(),
            "an entry naming the loopback must not permit another address");
        Ok(())
    }

    /// A unique, empty scratch directory under the system temp root.
    fn scratch_dir(tag: &str) -> Outcome<PathBuf> {
        let nanos = match std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
        {
            Ok(d)  => d.as_nanos(),
            Err(_) => 0,
        };
        let dir = std::env::temp_dir().join(fmt!(
            "fe2o3_steel_file_resolver_{}_{}_{}", tag, std::process::id(), nanos));
        res!(std::fs::create_dir_all(&dir));
        Ok(dir)
    }

    /// `{file:path}` fails when the file is missing; `{file?:path}` yields the
    /// empty string on that one io kind — not-found — and nothing else. The
    /// two optional cases below straddle the branch: a missing file must give
    /// `""` while a read that fails for any *other* reason must still error,
    /// so the test fails if the not-found guard is dropped (every error
    /// swallowed) or inverted (not-found not swallowed).
    #[test]
    fn optional_file_marker_swallows_only_not_found() -> Outcome<()> {
        let root = res!(scratch_dir("optmark"));
        let present = "present.key";
        // A trailing newline proves the resolver trims, as the required form does.
        res!(std::fs::write(root.join(present), "s3cret\n"));

        // Required form: present resolves to the trimmed contents, ...
        assert_eq!(
            res!(ApiRoute::resolve_file_only(&fmt!("{{file:{}}}", present), &root)),
            "s3cret",
            "{{file:PRESENT}} must resolve to the trimmed file contents");
        // ... and missing hard-errors exactly as before this change.
        assert!(
            ApiRoute::resolve_file_only("{file:absent.key}", &root).is_err(),
            "{{file:MISSING}} must still hard-error");

        // Optional form: present behaves identically to the required form.
        assert_eq!(
            res!(ApiRoute::resolve_file_only(&fmt!("{{file?:{}}}", present), &root)),
            "s3cret",
            "{{file?:PRESENT}} must resolve to the trimmed file contents");
        // The load-bearing case: a not-found file resolves to the empty string.
        assert_eq!(
            res!(ApiRoute::resolve_file_only("{file?:absent.key}", &root)),
            "",
            "{{file?:MISSING}} must resolve to \"\", not error");

        // A read that fails for a reason other than not-found must still
        // error, so a present-but-unreadable key is never silently dropped.
        // Traversing through a regular file yields ENOTDIR, an io error whose
        // kind is not NotFound and therefore must not be swallowed. This holds
        // for any user, needing no permission games.
        res!(std::fs::write(root.join("notdir"), "x"));
        assert!(
            ApiRoute::resolve_file_only("{file?:notdir/child}", &root).is_err(),
            "{{file?:...}} must error on a non-not-found io error (ENOTDIR here)");

        // The literal present-but-unreadable case, where the platform allows
        // it to be constructed: a mode-000 file read by a non-root user fails
        // with PermissionDenied, which the optional form must not swallow.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = root.join("locked.key");
            res!(std::fs::write(&locked, "nope\n"));
            res!(std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)));
            // Skip the assertion if the test runs as root, where mode-000 is
            // still readable and the read would succeed.
            if std::fs::read_to_string(&locked).is_err() {
                assert!(
                    ApiRoute::resolve_file_only("{file?:locked.key}", &root).is_err(),
                    "{{file?:PRESENT_BUT_UNREADABLE}} must error, not yield \"\"");
            }
            let _ = std::fs::set_permissions(
                &locked, std::fs::Permissions::from_mode(0o600));
        }

        // Both markers may co-occur in one value, and the required finder must
        // not mis-match the optional marker: here the absent optional collapses
        // to nothing while the present required resolves around it.
        assert_eq!(
            res!(ApiRoute::resolve_file_only(
                &fmt!("a{{file?:absent.key}}b{{file:{}}}c", present), &root)),
            "abs3cretc",
            "the required finder must not mis-read '{{file?:' as '{{file:'");

        // The optional marker also works through the public wired entry point,
        // which runs the env pass first.
        assert_eq!(
            res!(ApiRoute::resolve_file_refs("{file?:absent.key}", &root)),
            "",
            "the optional marker must resolve through resolve_file_refs too");

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
