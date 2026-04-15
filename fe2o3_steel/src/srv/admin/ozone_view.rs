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
    AdminPrincipal,
    assets::{
        html_escape,
        render_layout,
    },
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
        None => return Ok(render_no_db_page(&principal)),
    };

    let entries = {
        let guard = lock_read!(db_arc);
        match guard.scan(&opts, None) {
            Ok(v) => v,
            Err(e) => {
                error!(e, "{}: ozone view scan failed", id);
                return Ok(render_error_page(
                    &principal,
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
    Ok(render_list_page(&principal, &opts, &keys))
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
    principal: &AdminPrincipal,
    opts:      &ScanOpts,
    keys:      &[Dat],
) -> HttpMessage {
    let prefix_attr = match &opts.prefix {
        Some(Dat::Str(s)) => html_escape(s),
        _ => String::new(),
    };
    let limit_attr = opts.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let body_table = if keys.is_empty() {
        "<p class=\"notice empty\">No keys match this filter.</p>".to_string()
    } else {
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
        fmt!(
            "<table class=\"steel-table\">\n\
            <thead><tr><th>Key</th></tr></thead>\n\
            <tbody>\n{}</tbody>\n\
            </table>\n",
            rows,
        )
    };
    let body = fmt!(
        "<h1>Ozone browser</h1>\n\
        <p class=\"meta\">Showing {count} entries (cap {limit}).</p>\n\
        <form class=\"steel-form\" method=\"GET\" action=\"/admin/ozone\">\n\
        <div class=\"row\">\n\
            <div>\n\
                <label for=\"prefix\">Prefix</label>\n\
                <input type=\"text\" id=\"prefix\" name=\"prefix\" \
                    value=\"{prefix}\" placeholder=\"e.g. user:\">\n\
            </div>\n\
            <div>\n\
                <label for=\"limit\">Limit</label>\n\
                <input type=\"number\" id=\"limit\" name=\"limit\" \
                    value=\"{limit}\" min=\"1\" max=\"{maxlim}\">\n\
            </div>\n\
        </div>\n\
        <button type=\"submit\">Search</button>\n\
        </form>\n\
        {table}",
        count   = keys.len(),
        limit   = limit_attr,
        prefix  = prefix_attr,
        maxlim  = MAX_LIST_LIMIT,
        table   = body_table,
    );
    let html = render_layout(
        "Ozone",
        "/admin/ozone",
        principal,
        &body,
    );
    html_response(html)
}

/// Render the empty-state page returned when this vhost has no
/// configured ozone database (typical for pure-redirect vhosts).
fn render_no_db_page(principal: &AdminPrincipal) -> HttpMessage {
    let body = "<h1>Ozone browser</h1>\n\
        <p class=\"notice empty\">\
        This vhost does not have an ozone database configured. \
        Pure-redirect vhosts and static-only vhosts do not store \
        anything in ozone, so there is nothing to browse here.\
        </p>\n".to_string();
    let html = render_layout(
        "Ozone",
        "/admin/ozone",
        principal,
        &body,
    );
    html_response(html)
}

/// Render the error page returned when scan itself fails. Keeps
/// the structural error out of the response body; the operator
/// reads the server log for the underlying cause.
fn render_error_page(principal: &AdminPrincipal, message: &str) -> HttpMessage {
    let body = fmt!(
        "<h1>Ozone browser</h1>\n\
        <p class=\"notice error\">{}</p>\n",
        html_escape(message),
    );
    let html = render_layout(
        "Ozone",
        "/admin/ozone",
        principal,
        &body,
    );
    html_response(html)
}

/// Build a 200 OK HTML response. Mirrors the helper in
/// `handler.rs`; copied locally so each render module can build
/// responses without re-importing the same helper.
fn html_response(body: String) -> HttpMessage {
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

