//! Read-only ozone browser for the admin dashboard.
//!
//! Lists the live `(key, meta)` entries in the current vhost's
//! ozone database, with optional prefix filter and limit, and
//! renders them as an HTML table. Values are not fetched at scan
//! time -- the upstream `Database::scan` API returns them as
//! `Dat::Empty` -- so the list view is cheap even on large
//! databases. Future revisions will add a per-key detail view
//! that calls `Database::get` to materialise the value on demand.
//!
//! # Routes
//!
//! - `GET /admin/ozone` -- list all keys for the current vhost's
//!   ozone database, optionally filtered by `?prefix=<str>` and
//!   capped by `?limit=<n>`.
//!
//! # Auth
//!
//! Same gate as every other authenticated dashboard view: the
//! request must carry a valid session cookie whose principal
//! holds either `dashboard.view` or `dashboard.admin`. Failures
//! 303-redirect to `/admin/login`.

use crate::srv::admin::{
    handler::{
        extract_principal,
        redirect_to_login,
    },
    state::AdminState,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    ScanOpts,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    daticle::Dat,
    id::NumIdDat,
};
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFields,
        HeaderFieldValue,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::sync::{
    Arc,
    RwLock,
};

/// Default cap on the number of keys returned by a single list
/// request. Bounds the wire response when the operator does not
/// supply an explicit `?limit=` parameter.
pub const DEFAULT_LIST_LIMIT: usize = 500;

/// Hard upper bound on the limit the operator can request. Stops
/// a malicious or accidental `?limit=999999999` from materialising
/// the whole database into memory at once.
pub const MAX_LIST_LIMIT: usize = 5_000;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch a `GET` request against an `/admin/ozone*` path.
///
/// Generic over the database scheme so this function can run
/// against whatever ozone configuration the caller's vhost
/// happens to use. The trait bound is `Database<UIDL, UID, ENC, KH>`
/// so the call sites in `app/https.rs` (which already carry these
/// generics through `WebHandler::handle_get`) can pass `_db`
/// straight in.
pub async fn handle_get<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
>(
    state:        &AdminState,
    db:           Option<&(Arc<RwLock<DB>>, UID)>,
    request_path: &str,
    headers:      &Arc<HeaderFields>,
    id:           &str,
)
    -> Outcome<HttpMessage>
{
    debug!("{}: ozone view GET {}", id, request_path);

    // Auth gate first -- never reveal vhost db contents to an
    // unauthenticated visitor.
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return Ok(redirect_to_login()),
    };

    // Strip the path prefix and pull the query string off the
    // back. v1 supports only the list route; future revisions
    // will route /admin/ozone/<urlencoded_key> to a detail view.
    let after_prefix = match request_path.strip_prefix("/admin/ozone") {
        Some(s) => s,
        None => return Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Ozone route not found.",
        )),
    };
    let (path_part, query) = match after_prefix.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (after_prefix, None),
    };
    let _ = path_part; // Sub-path key detail view lands in v2.

    let opts = parse_list_opts(query);

    // Resolve the per-vhost database. A vhost without a configured
    // ozone (typical for pure-redirect vhosts) renders a friendly
    // empty-state page rather than 500.
    let (db_arc, _uid) = match db {
        Some(t) => t,
        None => return Ok(render_no_db_page(&principal.name)),
    };

    let entries = {
        let guard = lock_read!(db_arc);
        match guard.scan(&opts, None) {
            Ok(v) => v,
            Err(e) => {
                error!(e, "{}: ozone view scan failed", id);
                return Ok(render_error_page(
                    &principal.name,
                    "Scan failed; check the server log.",
                ));
            },
        }
    };

    // The v1 list view shows keys only -- scan returns Dat::Empty
    // values and we ignore the per-entry meta (the render side is
    // not generic over UID byte length). Drop into a Vec<Dat> of
    // just the keys so the renderer stays simple and non-generic.
    let keys: Vec<Dat> = entries.into_iter().map(|(k, _, _)| k).collect();
    Ok(render_list_page(&principal.name, &opts, &keys))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ QUERY PARSING                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parse the query string of a list request into [`ScanOpts`].
/// Recognises `prefix=<urlencoded str>` and `limit=<u32>`. Unknown
/// keys are ignored; malformed limits fall back to the default.
fn parse_list_opts(query: Option<&str>) -> ScanOpts {
    let mut opts = ScanOpts::default();
    opts.limit = Some(DEFAULT_LIST_LIMIT);
    let q = match query {
        Some(s) => s,
        None => return opts,
    };
    for pair in q.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = match kv.next() { Some(k) => k, None => continue };
        let v = kv.next().unwrap_or("");
        match k {
            "prefix" => {
                let decoded = url_decode(v);
                if !decoded.is_empty() {
                    opts.prefix = Some(Dat::Str(decoded));
                }
            },
            "limit" => {
                if let Ok(n) = v.parse::<usize>() {
                    opts.limit = Some(n.min(MAX_LIST_LIMIT));
                }
            },
            _ => (),
        }
    }
    opts
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RENDER                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the key list as an HTML table with the prefix filter
/// form above it. v1 shows keys only; values and metadata land
/// in the per-key detail view in a later commit.
fn render_list_page(
    actor: &str,
    opts:  &ScanOpts,
    keys:  &[Dat],
) -> HttpMessage {
    let prefix_attr = match &opts.prefix {
        Some(Dat::Str(s)) => html_escape(s),
        _ => String::new(),
    };
    let limit_attr = opts.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let mut rows = String::new();
    for k in keys.iter() {
        let key_str = match k {
            Dat::Str(s) => s.clone(),
            other => fmt!("{:?}", other),
        };
        rows.push_str(&fmt!(
            "<tr><td><code>{}</code></td></tr>\n",
            html_escape(&key_str),
        ));
    }
    let body = fmt!(
        "<!doctype html>\n\
        <html><head><title>Ozone browser</title>\n\
        <meta charset=\"utf-8\">\n\
        <style>\n\
        body {{ font-family: serif; max-width: 60rem; margin: 2rem auto; \
                padding: 0 1rem; color: #222; }}\n\
        h1 {{ font-variant-caps: small-caps; }}\n\
        form {{ margin-bottom: 1rem; }}\n\
        table {{ border-collapse: collapse; width: 100%; }}\n\
        td, th {{ border-bottom: 1px solid rgb(220,220,220); \
                  padding: 0.4rem 0.6rem; text-align: left; }}\n\
        th {{ background: rgb(240,240,240); }}\n\
        code {{ font-family: monospace; }}\n\
        .nav a {{ color: rgb(243,60,87); margin-right: 1rem; }}\n\
        </style>\n\
        </head><body>\n\
        <p class=\"nav\"><a href=\"/admin\">Home</a> \
        <a href=\"/admin/logout\">Sign out</a></p>\n\
        <h1>Ozone browser</h1>\n\
        <p>Signed in as <strong>{}</strong>. \
        Showing {} entries (cap {}).</p>\n\
        <form method=\"GET\" action=\"/admin/ozone\">\n\
        <label>Prefix: <input type=\"text\" name=\"prefix\" value=\"{}\"></label>\n\
        <label>Limit: <input type=\"number\" name=\"limit\" value=\"{}\" min=\"1\" max=\"{}\"></label>\n\
        <button type=\"submit\">Search</button>\n\
        </form>\n\
        <table><thead><tr><th>Key</th></tr></thead><tbody>\n\
        {}\
        </tbody></table>\n\
        </body></html>\n",
        html_escape(actor),
        keys.len(),
        limit_attr,
        prefix_attr,
        limit_attr,
        MAX_LIST_LIMIT,
        rows,
    );
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
        )
        .with_body(body.into_bytes())
}

/// Render the empty-state page returned when this vhost has no
/// configured ozone database (typical for pure-redirect vhosts).
fn render_no_db_page(actor: &str) -> HttpMessage {
    let body = fmt!(
        "<!doctype html>\n\
        <html><head><title>Ozone browser</title></head>\n\
        <body>\n\
        <p><a href=\"/admin\">Home</a></p>\n\
        <h1>Ozone browser</h1>\n\
        <p>Signed in as <strong>{}</strong>.</p>\n\
        <p>This vhost does not have an ozone database configured. \
        Pure-redirect vhosts and static-only vhosts do not store \
        anything in ozone, so there is nothing to browse here.</p>\n\
        </body></html>\n",
        html_escape(actor),
    );
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
        )
        .with_body(body.into_bytes())
}

/// Render the error page returned when scan itself fails. Keeps
/// the structural error out of the response body; the operator
/// reads the server log for the underlying cause.
fn render_error_page(actor: &str, message: &str) -> HttpMessage {
    let body = fmt!(
        "<!doctype html>\n\
        <html><head><title>Ozone browser</title></head>\n\
        <body>\n\
        <p><a href=\"/admin\">Home</a></p>\n\
        <h1>Ozone browser</h1>\n\
        <p>Signed in as <strong>{}</strong>.</p>\n\
        <p style=\"color: rgb(243,60,87);\">{}</p>\n\
        </body></html>\n",
        html_escape(actor),
        html_escape(message),
    );
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
        )
        .with_body(body.into_bytes())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// URL-decode a query parameter value.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            },
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_nibble(bytes[i + 1]);
                let lo = hex_nibble(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    },
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    },
                }
            },
            b => {
                out.push(b);
                i += 1;
            },
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + b - b'a'),
        b'A'..=b'F' => Some(10 + b - b'A'),
        _ => None,
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&'  => out.push_str("&amp;"),
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _    => out.push(c),
        }
    }
    out
}
