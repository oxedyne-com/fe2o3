//! Read-only ozone browser for the admin dashboard.
//!
//! Lists the live `(key, meta)` entries in the current vhost's
//! ozone database, with optional prefix filter and limit, and
//! renders them as an HTML table. A second query parameter
//! `key=<urlencoded>` selects one key and triggers a
//! `Database::get` to populate a detail panel alongside the list.
//!
//! # Routes
//!
//! - `GET /admin/database` -- list all keys for the current vhost's
//!   ozone database, optionally filtered by `?prefix=<str>`,
//!   capped by `?limit=<n>`, and with `?key=<str>` selecting a
//!   single entry for the detail panel.
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
        OZONE_LOGO_SVG,
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
// │ QUERY AND DETAIL STATE                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parsed query string: scan options plus optional detail selector.
struct ListQuery {
    /// Scan parameters forwarded to `Database::scan`.
    scan:   ScanOpts,
    /// Key to fetch for the detail panel, if `?key=` was supplied.
    detail: Option<String>,
}

/// Non-generic snapshot of one key's detail, produced inside the
/// generic `handle_get` and passed to the non-generic renderer.
struct DetailView {
    /// The key string that was requested.
    key:     String,
    /// Fetch outcome rendered verbatim into the detail panel.
    outcome: DetailOutcome,
}

enum DetailOutcome {
    /// `Database::get` returned `Some` -- render value + meta.
    Found {
        value_jdat: String,
        meta_time:  u64,
        meta_user:  String,
    },
    /// `Database::get` returned `None`.
    Missing,
    /// `Database::get` returned `Err`. The structural error itself
    /// is logged; only a short user-facing message is shown.
    Error(String),
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch a `GET` request against an `/admin/database*` path.
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
    query:        &str,
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

    // Strip the path prefix. v1 supports only the list route; future revisions
    // will route /admin/database/<urlencoded_key> to a detail view.
    //
    // The query arrives as its own argument and is not cut out of the path: a
    // request's path and query are parsed apart, so `request_path` never holds
    // a `?`. This looked for one, never found it, and quietly read every
    // request as though it carried no query at all -- so the prefix box, the
    // limit box and the detail panel all did nothing, and the page always
    // listed the first `DEFAULT_LIST_LIMIT` keys of the whole database.
    let path_part = match request_path.strip_prefix("/admin/database") {
        Some(s) => s,
        None => return Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Ozone route not found.",
        )),
    };
    let _ = path_part; // Sub-path routing reserved for future use.

    let parsed = parse_query(query);

    // Resolve the per-vhost database. A vhost without a configured
    // ozone (typical for pure-redirect vhosts) renders a friendly
    // empty-state page rather than 500.
    let (db_arc, _uid) = match db {
        Some(t) => t,
        None => return Ok(render_no_db_page(&principal)),
    };

    let entries = {
        let guard = lock_read!(db_arc);
        match guard.scan(&parsed.scan, None) {
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

    // Scan returns `Dat::Empty` values; keep only the keys for
    // the list renderer so the render side stays non-generic.
    let keys: Vec<Dat> = entries.into_iter().map(|(k, _, _)| k).collect();

    // If a detail key was supplied, fetch it via `Database::get`
    // and marshal the result into a non-generic snapshot so the
    // renderer does not need to carry UID generics.
    let detail = match &parsed.detail {
        Some(key_str) => {
            let dat_key = Dat::Str(key_str.clone());
            let outcome = {
                let guard = lock_read!(db_arc);
                match guard.get(&dat_key, None) {
                    Ok(Some((val, meta))) => DetailOutcome::Found {
                        value_jdat: fmt!("{}", val),
                        meta_time:  meta.time.secs(),
                        meta_user:  fmt!("{:?}", meta.user),
                    },
                    Ok(None) => DetailOutcome::Missing,
                    Err(e) => {
                        error!(e, "{}: ozone get failed for key {:?}", id, key_str);
                        DetailOutcome::Error(
                            "Fetch failed; check the server log.".to_string(),
                        )
                    },
                }
            };
            Some(DetailView { key: key_str.clone(), outcome })
        },
        None => None,
    };

    Ok(render_list_page(&principal, &parsed, &keys, detail.as_ref()))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ QUERY PARSING                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parse the query string of a list request into a [`ListQuery`].
/// Recognises `prefix=<str>`, `limit=<u32>`, and `key=<str>` for
/// the detail panel. Unknown keys are ignored; malformed limits
/// fall back to the default.
fn parse_query(query: &str) -> ListQuery {
    let mut scan = ScanOpts::default();
    scan.limit = Some(DEFAULT_LIST_LIMIT);
    let mut detail: Option<String> = None;
    if query.is_empty() {
        return ListQuery { scan, detail };
    }
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = match kv.next() { Some(k) => k, None => continue };
        let v = kv.next().unwrap_or("");
        match k {
            "prefix" => {
                let decoded = url_decode(v);
                if !decoded.is_empty() {
                    scan.prefix = Some(Dat::Str(decoded));
                }
            },
            "limit" => {
                if let Ok(n) = v.parse::<usize>() {
                    scan.limit = Some(n.min(MAX_LIST_LIMIT));
                }
            },
            "key" => {
                let decoded = url_decode(v);
                if !decoded.is_empty() {
                    detail = Some(decoded);
                }
            },
            _ => (),
        }
    }
    ListQuery { scan, detail }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RENDER                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the key list plus an optional detail panel.
///
/// Each row in the key table is a link to `?prefix=...&limit=...
/// &key=<urlencoded>` so clicking a key refreshes the same page
/// with the detail panel populated. The prefix and limit inputs
/// are preserved so the list context survives a selection.
fn render_list_page(
    principal: &AdminPrincipal,
    parsed:    &ListQuery,
    keys:      &[Dat],
    detail:    Option<&DetailView>,
) -> HttpMessage {
    let prefix_raw = match &parsed.scan.prefix {
        Some(Dat::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let limit_val = parsed.scan.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let selected_key = detail.map(|d| d.key.as_str()).unwrap_or("");

    let form = fmt!(
        "<form class=\"steel-form\" method=\"GET\" action=\"/admin/database\">\n\
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
        </form>\n",
        prefix = html_escape(&prefix_raw),
        limit  = limit_val,
        maxlim = MAX_LIST_LIMIT,
    );

    let table = render_key_table(keys, &prefix_raw, limit_val, selected_key);
    let detail_html = match detail {
        Some(d) => render_detail_panel(d),
        None    => render_detail_placeholder(),
    };

    let body = fmt!(
        "<h1><span class=\"heading-logo\">{logo}</span>Database</h1>\n\
        <p class=\"meta\">Showing {count} entries (cap {limit}).</p>\n\
        {form}\
        <div class=\"ozone-split\">\n\
            <div class=\"ozone-list\">{table}</div>\n\
            <aside class=\"ozone-detail\">{detail}</aside>\n\
        </div>\n",
        logo   = OZONE_LOGO_SVG,
        count  = keys.len(),
        limit  = limit_val,
        form   = form,
        table  = table,
        detail = detail_html,
    );

    let html = render_layout(
        "Database",
        "/admin/database",
        principal,
        &body,
        "",
    );
    html_response(html)
}

/// Render the key list table. Each row is a link back to the
/// list view carrying the selected key in the query string.
fn render_key_table(
    keys:         &[Dat],
    prefix_raw:   &str,
    limit_val:    usize,
    selected_key: &str,
)
    -> String
{
    if keys.is_empty() {
        return "<p class=\"notice empty\">No keys match this filter.</p>".to_string();
    }
    let mut rows = String::new();
    for k in keys.iter() {
        let key_str = match k {
            Dat::Str(s) => s.clone(),
            other       => fmt!("{:?}", other),
        };
        let href = fmt!(
            "/admin/database?prefix={}&amp;limit={}&amp;key={}",
            url_encode(prefix_raw),
            limit_val,
            url_encode(&key_str),
        );
        let selected_attr = if key_str == selected_key {
            " class=\"selected\""
        } else {
            ""
        };
        rows.push_str(&fmt!(
            "<tr{sel}><td><a href=\"{href}\"><code>{label}</code></a></td></tr>\n",
            sel   = selected_attr,
            href  = href,
            label = html_escape(&key_str),
        ));
    }
    fmt!(
        "<table class=\"steel-table\">\n\
        <thead><tr><th>Key</th></tr></thead>\n\
        <tbody>\n{}</tbody>\n\
        </table>\n",
        rows,
    )
}

/// Render the right-hand detail panel for a selected key.
fn render_detail_panel(d: &DetailView) -> String {
    match &d.outcome {
        DetailOutcome::Found { value_jdat, meta_time, meta_user } => fmt!(
            "<h3>Selected key</h3>\n\
            <div class=\"field\"><span class=\"lbl\">Key</span>\
                <span class=\"val\"><code>{key}</code></span></div>\n\
            <div class=\"field\"><span class=\"lbl\">Modified (unix s)</span>\
                <span class=\"val\">{time}</span></div>\n\
            <div class=\"field\"><span class=\"lbl\">Meta user</span>\
                <span class=\"val\">{user}</span></div>\n\
            <div class=\"field\"><span class=\"lbl\">Value (JDAT)</span>\
                <pre class=\"val\">{value}</pre></div>\n",
            key   = html_escape(&d.key),
            time  = meta_time,
            user  = html_escape(meta_user),
            value = html_escape(value_jdat),
        ),
        DetailOutcome::Missing => fmt!(
            "<h3>Selected key</h3>\n\
            <div class=\"field\"><span class=\"lbl\">Key</span>\
                <span class=\"val\"><code>{key}</code></span></div>\n\
            <p class=\"notice empty\">Key not present in the database.</p>\n",
            key = html_escape(&d.key),
        ),
        DetailOutcome::Error(msg) => fmt!(
            "<h3>Selected key</h3>\n\
            <div class=\"field\"><span class=\"lbl\">Key</span>\
                <span class=\"val\"><code>{key}</code></span></div>\n\
            <p class=\"notice error\">{msg}</p>\n",
            key = html_escape(&d.key),
            msg = html_escape(msg),
        ),
    }
}

/// Render the placeholder shown when no key has been selected yet.
fn render_detail_placeholder() -> String {
    "<h3>Selected key</h3>\n\
    <p class=\"notice empty\">Pick a key from the list to see its \
    value and metadata.</p>\n".to_string()
}

/// Render the empty-state page returned when this vhost has no
/// configured ozone database (typical for pure-redirect vhosts).
fn render_no_db_page(principal: &AdminPrincipal) -> HttpMessage {
    let body = fmt!(
        "<h1><span class=\"heading-logo\">{}</span>Database</h1>\n\
        <p class=\"notice empty\">\
        This vhost does not have an ozone database configured. \
        Pure-redirect vhosts and static-only vhosts do not store \
        anything in ozone, so there is nothing to browse here.\
        </p>\n",
        OZONE_LOGO_SVG,
    );
    let html = render_layout(
        "Database",
        "/admin/database",
        principal,
        &body,
        "",
    );
    html_response(html)
}

/// Render the error page returned when scan itself fails. Keeps
/// the structural error out of the response body; the operator
/// reads the server log for the underlying cause.
fn render_error_page(principal: &AdminPrincipal, message: &str) -> HttpMessage {
    let body = fmt!(
        "<h1><span class=\"heading-logo\">{}</span>Database</h1>\n\
        <p class=\"notice error\">{}</p>\n",
        OZONE_LOGO_SVG,
        html_escape(message),
    );
    let html = render_layout(
        "Database",
        "/admin/database",
        principal,
        &body,
        "",
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

/// URL-encode a string for use in a query parameter. Percent-
/// escapes every byte that is not an unreserved ASCII character
/// per RFC 3986 section 2.3.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes().iter() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            other => out.push_str(&fmt!("%{:02X}", other)),
        }
    }
    out
}

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

