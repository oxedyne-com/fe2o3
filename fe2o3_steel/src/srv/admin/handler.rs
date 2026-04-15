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
/// the visitor to the login form. v1 home content is deliberately
/// minimal -- traffic, ozone browser and admin management land in
/// subsequent commits.
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
        "<!doctype html>\n\
        <html><head><title>Steel admin</title></head>\n\
        <body>\n\
        <h1>Steel admin dashboard</h1>\n\
        <p>Signed in as <strong>{}</strong>.</p>\n\
        <p>Scopes: <code>{}</code></p>\n\
        <p>This is the v1 placeholder landing page. Traffic, ozone \
        and admin-management views arrive in subsequent commits.</p>\n\
        <p><a href=\"/admin/logout\">Sign out</a></p>\n\
        </body></html>\n",
        html_escape(&principal.name),
        html_escape(&principal.scopes.join(" ")),
    );
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

/// Build the HTML login form. `error_msg`, when present, is rendered
/// above the form -- the wording is deliberately generic to avoid
/// leaking which axis (no admin / wrong passphrase / no dashboard
/// scope) caused the failure.
fn render_login_form(error_msg: Option<&str>) -> HttpMessage {
    let error_html = match error_msg {
        Some(msg) => fmt!(
            "<p style=\"color: rgb(243, 60, 87);\">{}</p>",
            html_escape(msg),
        ),
        None => String::new(),
    };
    let body = fmt!(
        "<!doctype html>\n\
        <html><head>\n\
        <title>Steel admin login</title>\n\
        <meta charset=\"utf-8\">\n\
        <style>\n\
        body {{ font-family: serif; max-width: 32rem; margin: 4rem auto; \
                padding: 0 1rem; color: #222; }}\n\
        h1 {{ font-variant-caps: small-caps; font-weight: bold; }}\n\
        form {{ background: rgb(240, 240, 240); padding: 1.5rem; }}\n\
        input[type=password] {{ width: 100%; padding: 0.5rem; \
                font-size: 1rem; }}\n\
        button {{ background: rgb(243, 60, 87); color: white; \
                border: 0; padding: 0.5rem 1rem; font-size: 1rem; \
                cursor: pointer; }}\n\
        </style>\n\
        </head><body>\n\
        <h1>Steel admin</h1>\n\
        {}\
        <form method=\"POST\" action=\"/admin/login\">\n\
        <p><label for=\"passphrase\">Wallet passphrase:</label></p>\n\
        <p><input type=\"password\" id=\"passphrase\" name=\"passphrase\" \
            autofocus required></p>\n\
        <p><button type=\"submit\">Sign in</button></p>\n\
        </form>\n\
        </body></html>\n",
        error_html,
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

/// Escape a string for safe inclusion in HTML body text. Replaces
/// the five characters that can break out of a text node into
/// markup or attribute syntax. Sufficient for the dashboard's
/// error message rendering; richer output goes through proper
/// templating in task #6.
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
