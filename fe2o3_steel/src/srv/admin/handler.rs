//! HTTP dispatcher for the admin dashboard.
//!
//! Routes `/admin/*` paths to login, logout, and authenticated views.
//! Extracts the session cookie via [`session`](super::session) on
//! every authenticated request and rejects any path whose principal
//! lacks the needed scope.
//!
//! The same dispatcher is reused by the loopback plaintext listener
//! added in task #8 -- localhost gets the same routes, the same
//! login gate, and the same session cookie format.

use crate::srv::admin::{
    AdminPrincipal,
    assets::{
        html_escape,
        render_layout,
        render_login_layout,
    },
    audit::{
        self,
        ADMIN_ANON,
        VERB_DASHBOARD_LOGIN,
        VERB_DASHBOARD_LOGOUT,
    },
    auth::{
        self,
        LoginOutcome,
    },
    session::{
        self,
        SESSION_COOKIE_NAME,
    },
    state::AdminState,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        Cookie,
        HeaderFields,
        HeaderFieldValue,
        HeaderName,
        SameSite,
        SetCookieAttributes,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::BTreeSet,
    sync::Arc,
};

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ROUTE PATHS                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dashboard root. Authenticated landing page.
pub const PATH_ROOT:    &str = "/admin";
/// Login form (GET) and login submission (POST).
pub const PATH_LOGIN:   &str = "/admin/login";
/// Logout (GET): clears the session cookie and redirects to login.
pub const PATH_LOGOUT:  &str = "/admin/logout";
/// Traffic view (GET): renders recent requests and per-vhost
/// counters from the shared `TrafficRecorder`.
pub const PATH_TRAFFIC: &str = "/admin/traffic";

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch a `GET` request against a `/admin/*` path.
pub async fn handle_get(
    state:   &AdminState,
    path:    &str,
    headers: &Arc<HeaderFields>,
    id:      &str,
)
    -> Outcome<HttpMessage>
{
    debug!("{}: dashboard GET {}", id, path);
    match path {
        PATH_LOGIN => Ok(render_login_form(None)),
        PATH_LOGOUT => Ok(handle_logout(state, headers)),
        PATH_ROOT => Ok(render_home(state, headers)),
        PATH_TRAFFIC => Ok(render_traffic(state, headers)),
        _ => Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Dashboard route not found.",
        )),
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ POST                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch a `POST` request against a `/admin/*` path.
pub async fn handle_post(
    state:    &AdminState,
    path:     &str,
    body:     &[u8],
    _headers: &Arc<HeaderFields>,
    id:       &str,
)
    -> Outcome<HttpMessage>
{
    debug!("{}: dashboard POST {}", id, path);
    match path {
        PATH_LOGIN => Ok(handle_login(state, body)),
        _ => Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Dashboard route not found.",
        )),
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOGIN                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Process a login form submission. Body is
/// `application/x-www-form-urlencoded` carrying a single `passphrase`
/// field. On success, set the session cookie and 303-redirect to
/// `/admin`. On failure, re-render the login form with a generic
/// error message.
///
/// Every outcome -- success, bad credentials, missing dashboard
/// scope, structural error -- writes one line to the admin audit
/// log. The actor name is the unlocked admin where known and
/// `(anon)` otherwise; the response to the client is deliberately
/// generic regardless of outcome so the audit log is the only
/// place where "wrong passphrase" and "no dashboard scope" can
/// be told apart.
fn handle_login(
    state: &AdminState,
    body:  &[u8],
)
    -> HttpMessage
{
    let passphrase = match extract_form_field(body, "passphrase") {
        Some(p) => p,
        None => {
            audit::append(
                ADMIN_ANON,
                VERB_DASHBOARD_LOGIN,
                "err",
                "reason=missing_passphrase_field",
            );
            return render_login_form(Some(
                "Login form did not include a passphrase."));
        },
    };

    let outcome = match auth::verify_passphrase(state, passphrase.as_bytes()) {
        Ok(o) => o,
        Err(e) => {
            warn!("dashboard login: structural error during verify: {}", e);
            audit::append(
                ADMIN_ANON,
                VERB_DASHBOARD_LOGIN,
                "err",
                "reason=verify_structural_error",
            );
            return render_login_form(Some("Internal error during login."));
        },
    };

    match outcome {
        LoginOutcome::Ok(principal) => {
            audit::append(
                &principal.name,
                VERB_DASHBOARD_LOGIN,
                "ok",
                &fmt!("scopes={}", principal.scopes.join(",")),
            );
            issue_session_cookie(state, &principal)
        },
        LoginOutcome::BadCredentials => {
            audit::append(
                ADMIN_ANON,
                VERB_DASHBOARD_LOGIN,
                "err",
                "reason=bad_credentials",
            );
            // Generic message: do not leak whether any admin exists.
            render_login_form(Some("Invalid credentials."))
        },
        LoginOutcome::NoDashboardScope { name } => {
            audit::append(
                &name,
                VERB_DASHBOARD_LOGIN,
                "err",
                "reason=no_dashboard_scope",
            );
            warn!("dashboard login: admin '{}' authenticated but \
                holds no dashboard scope.", name);
            render_login_form(Some(
                "Authenticated, but this admin is not authorised \
                to use the dashboard. Ask an operator to grant \
                'dashboard.view' or 'dashboard.admin'."))
        },
    }
}

/// Build the success response: set the session cookie and redirect
/// to the dashboard root. The cookie is `HttpOnly`, `SameSite=Strict`
/// and `Secure` so it cannot be read by page JavaScript or sent on
/// cross-site requests.
fn issue_session_cookie(
    state:     &AdminState,
    principal: &AdminPrincipal,
)
    -> HttpMessage
{
    let cookie_value = match session::encode_session(state, principal) {
        Ok(s) => s,
        Err(e) => {
            error!(e, "dashboard login: failed to encode session cookie");
            return render_login_form(Some(
                "Login succeeded but session encoding failed."));
        },
    };
    let cookie = build_session_cookie(cookie_value, false);
    HttpMessage::new_response(HttpStatus::SeeOther)
        .with_field(
            HeaderName::Location,
            HeaderFieldValue::Generic(PATH_ROOT.to_string()),
        )
        .set_cookie(cookie)
}

/// Build a `Set-Cookie` value carrying `cookie_value` under
/// [`SESSION_COOKIE_NAME`]. The cookie is constrained to the
/// `/admin` path, marked `HttpOnly`, `Secure` and `SameSite=Strict`
/// so page JavaScript cannot read it and cross-site requests
/// cannot present it. Setting `clear` to `true` produces a
/// `Max-Age=0` cookie used by the logout flow to evict the
/// browser's stored copy.
fn build_session_cookie(cookie_value: String, clear: bool) -> Cookie {
    let mut attrs: BTreeSet<SetCookieAttributes> = BTreeSet::new();
    attrs.insert(SetCookieAttributes::Path("/admin".to_string()));
    attrs.insert(SetCookieAttributes::HttpOnly);
    attrs.insert(SetCookieAttributes::Secure);
    attrs.insert(SetCookieAttributes::SameSite(SameSite::Strict));
    if clear {
        attrs.insert(SetCookieAttributes::MaxAge(0));
    }
    Cookie {
        key:    SESSION_COOKIE_NAME.to_string(),
        val:    cookie_value,
        attrs:  Some(attrs),
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOGOUT                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Clear the session cookie by setting it to an empty value with a
/// `Max-Age=0`, then redirect to the login form. Stateless logout:
/// because sessions are not stored server-side, dropping the cookie
/// is sufficient.
///
/// Audits the logout under the actor's name when a valid session
/// cookie is present, and as `(anon)` otherwise. This lets the
/// audit log distinguish "Alice signed out" from "an unauthorised
/// visitor hit /admin/logout while not signed in".
fn handle_logout(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> HttpMessage
{
    let actor = match extract_principal(state, headers) {
        Some(p) => p.name,
        None => ADMIN_ANON.to_string(),
    };
    audit::append(&actor, VERB_DASHBOARD_LOGOUT, "ok", "");
    let cookie = build_session_cookie(String::new(), true);
    HttpMessage::new_response(HttpStatus::SeeOther)
        .with_field(
            HeaderName::Location,
            HeaderFieldValue::Generic(PATH_LOGIN.to_string()),
        )
        .set_cookie(cookie)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HOME                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Authenticated landing page. Validates the session cookie, and on
/// failure (no cookie / tampered / expired / unknown version) sends
/// the visitor to the login form. The home page is a brief
/// orientation panel pointing the operator at the live views.
fn render_home(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    let body = fmt!(
        "<h1>Dashboard</h1>\n\
        <p>Welcome, <strong>{}</strong>.</p>\n\
        <p class=\"meta\">Scopes: <code>{}</code></p>\n\
        <h2>Live views</h2>\n\
        <ul>\n\
            <li><a href=\"/admin/traffic\">Traffic</a> &mdash; \
                recent requests across every vhost on this host.</li>\n\
            <li><a href=\"/admin/ozone\">Ozone</a> &mdash; \
                browse the keys stored in this vhost's ozone database.</li>\n\
            <li><a href=\"/admin/admins\">Admins</a> &mdash; \
                manage wallet admin entries (requires <code>admin</code> scope).</li>\n\
        </ul>\n\
        <h2>About this dashboard</h2>\n\
        <p>This is the Steel admin dashboard, served from inside \
        the Steel server binary. Every request through this \
        process is reflected in the Traffic view; every privileged \
        action you take here is recorded in the same \
        <code>admin-audit.log</code> file the CLI <code>admin</code> \
        verbs write to.</p>\n",
        html_escape(&principal.name),
        html_escape(&principal.scopes.join(" ")),
    );
    let html = render_layout("Dashboard", "/admin", &principal, &body);
    html_response(html)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TRAFFIC                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the live traffic view. Reads the shared
/// `TrafficRecorder` from `AdminState` and emits a counters
/// summary plus the most recent N records as an HTML table.
/// v1 is intentionally read-only and synchronous (no
/// auto-refresh); a richer view with charts and live updates
/// can come later.
fn render_traffic(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    // Take both snapshots up front so the HTML rendering does
    // not hold either lock.
    let recent = match state.traffic.recent(50) {
        Ok(v) => v,
        Err(e) => {
            error!(e, "dashboard: traffic recent() failed");
            Vec::new()
        },
    };
    let counters = match state.traffic.counters_snapshot() {
        Ok(c) => c,
        Err(e) => {
            error!(e, "dashboard: traffic counters_snapshot() failed");
            crate::srv::admin::traffic::CountersSnapshot::default()
        },
    };

    // Render counters: total + per-status table.
    let mut status_rows = String::new();
    let mut statuses: Vec<(u16, u64)> = counters.by_status
        .iter()
        .map(|(s, n)| (*s, *n))
        .collect();
    statuses.sort_by_key(|(s, _)| *s);
    for (s, n) in statuses {
        status_rows.push_str(&fmt!(
            "<tr><td><code>{}</code></td><td>{}</td></tr>\n",
            s, n,
        ));
    }
    let counters_html = if counters.total == 0 {
        "<p class=\"notice empty\">\
         No requests recorded yet. As soon as a request lands on \
         this Steel binary the counters and recent-requests table \
         below will populate.\
         </p>".to_string()
    } else {
        fmt!(
            "<p class=\"meta\">Total requests since startup: \
            <strong>{}</strong>.</p>\n\
            <h2>By status</h2>\n\
            <table class=\"steel-table\">\n\
            <thead><tr><th>Status</th><th>Count</th></tr></thead>\n\
            <tbody>{}</tbody>\n\
            </table>\n",
            counters.total,
            status_rows,
        )
    };

    // Render the recent-requests table.
    let recent_html = if recent.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for r in &recent {
            rows.push_str(&fmt!(
                "<tr>\
                <td><code>{}</code></td>\
                <td>{}</td>\
                <td><code>{}</code></td>\
                <td>{}</td>\
                <td>{} \u{00b5}s</td>\
                </tr>\n",
                html_escape(&r.method),
                html_escape(&r.vhost),
                html_escape(&r.path),
                r.status,
                r.duration_us,
            ));
        }
        fmt!(
            "<h2>Recent requests</h2>\n\
            <table class=\"steel-table\">\n\
            <thead><tr>\
            <th>Method</th><th>Vhost</th><th>Path</th>\
            <th>Status</th><th>Duration</th>\
            </tr></thead>\n\
            <tbody>{}</tbody>\n\
            </table>\n",
            rows,
        )
    };

    let body = fmt!(
        "<h1>Traffic</h1>\n\
        {counters}\
        {recent}",
        counters = counters_html,
        recent   = recent_html,
    );
    let html = render_layout("Traffic", PATH_TRAFFIC, &principal, &body);
    html_response(html)
}

/// Build a 200 OK response with a UTF-8 HTML body. Centralises
/// the content-type wiring so the per-page render functions stay
/// concerned only with their own markup.
fn html_response(body: String) -> HttpMessage {
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
        )
        .with_body(body.into_bytes())
}

/// Build a 303 redirect to the login form. Used for any
/// authenticated route that is hit without a valid session.
/// `pub` so submodules can reuse this redirect without
/// reimplementing the response shape.
pub fn redirect_to_login() -> HttpMessage {
    HttpMessage::new_response(HttpStatus::SeeOther)
        .with_field(
            HeaderName::Location,
            HeaderFieldValue::Generic(PATH_LOGIN.to_string()),
        )
}

/// Extract the admin session cookie from the request header fields,
/// decode and verify it via [`session::decode_session`], and return
/// the embedded [`AdminPrincipal`] only if the principal is still
/// authorised to see the dashboard. Any failure -- missing cookie,
/// tampered cookie, expired cookie, missing dashboard scope -- is
/// flattened to `None` so the caller can simply 303 to login.
///
/// `pub` so other dashboard submodules (`ozone_view`, future
/// admin-management UI) can share the same auth gate rather than
/// each implementing cookie verification independently.
pub fn extract_principal(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> Option<AdminPrincipal>
{
    let cookie_value = read_cookie(headers, SESSION_COOKIE_NAME)?;
    let principal = match session::decode_session(state, &cookie_value) {
        Ok(p) => p,
        Err(e) => {
            debug!("dashboard: session cookie rejected: {}", e);
            return None;
        },
    };
    if !principal.can_view_dashboard() {
        debug!("dashboard: principal '{}' lacks dashboard scope",
            principal.name);
        return None;
    }
    Some(principal)
}

/// Walk the request's `Cookie` header and return the value of the
/// first cookie whose key matches `name`. Returns `None` when the
/// header is absent, has no Cookie field, or no entry matches.
fn read_cookie(headers: &Arc<HeaderFields>, name: &str) -> Option<String> {
    if let Some(HeaderFieldValue::Cookie(cookies)) =
        headers.get_one(&HeaderName::Cookie)
    {
        for c in cookies {
            if c.key == name {
                return Some(c.val.clone());
            }
        }
    }
    None
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOGIN FORM                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Build the HTML login form. `error_msg`, when present, is
/// rendered above the form -- the wording is deliberately generic
/// to avoid leaking which axis (no admin / wrong passphrase / no
/// dashboard scope) caused the failure.
fn render_login_form(error_msg: Option<&str>) -> HttpMessage {
    let error_html = match error_msg {
        Some(msg) => fmt!(
            "<p class=\"notice error\">{}</p>",
            html_escape(msg),
        ),
        None => String::new(),
    };
    let body = fmt!(
        "{error}\
        <form class=\"steel-form\" method=\"POST\" action=\"/admin/login\">\n\
        <label for=\"passphrase\">Wallet passphrase</label>\n\
        <input type=\"password\" id=\"passphrase\" name=\"passphrase\" \
            autofocus required>\n\
        <button type=\"submit\">Sign in</button>\n\
        </form>\n",
        error = error_html,
    );
    let html = render_login_layout("Sign in", &body);
    html_response(html)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Extract a single field value from an
/// `application/x-www-form-urlencoded` body. Returns the first
/// matching key's value, URL-decoded; returns `None` if the key
/// is absent. Designed for tiny login-style bodies where one or
/// two fields are expected.
fn extract_form_field(body: &[u8], key: &str) -> Option<String> {
    let s = std::str::from_utf8(body).ok()?;
    for pair in s.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = kv.next()?;
        let v = kv.next().unwrap_or("");
        if url_decode(k) == key {
            return Some(url_decode(v));
        }
    }
    None
}

/// Decode an `x-www-form-urlencoded` value. Replaces `+` with space
/// and `%XX` with the corresponding byte. Invalid escapes pass
/// through unchanged.
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

