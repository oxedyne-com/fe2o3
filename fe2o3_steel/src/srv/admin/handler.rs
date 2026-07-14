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
        upload_head_html,
    },
    audit::{
        self,
        ADMIN_ANON,
        VERB_DASHBOARD_ADMIN_ADD,
        VERB_DASHBOARD_ADMIN_REMOVE,
        VERB_DASHBOARD_GUARD_BLACKLIST,
        VERB_DASHBOARD_GUARD_UNBLOCK,
        VERB_DASHBOARD_GUARD_WHITELIST,
        VERB_DASHBOARD_LOGIN,
        VERB_DASHBOARD_LOGOUT,
    },
    auth::{
        self,
        LoginOutcome,
    },
    guard::DEFAULT_SNAPSHOT_CAP,
    session::{
        self,
        SESSION_COOKIE_NAME,
    },
    state::AdminState,
};

use oxedyne_fe2o3_crypto::keystore::DEFAULT_WALLET_KDF_NAME;
use oxedyne_fe2o3_jdat::{
    file::JdatFile,
    string::enc::EncoderConfig,
};

use std::{
    net::SocketAddr,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
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
/// JSON feed for the Overview sparkline strip. Emits host
/// sampler history as four aligned series (CPU, memory, disk,
/// network). Used by the inline Overview JavaScript and by the
/// auto-refresh polling loop.
pub const PATH_HOST_JSON: &str = "/admin/host.json";
/// JSON feed for the Traffic view. Emits counters, chart series,
/// and recent requests so the auto-refresh loop can update the
/// chip row, chart, and table without reloading the page.
pub const PATH_TRAFFIC_JSON: &str = "/admin/traffic.json";
/// Security view (GET): renders the address guard counts and a
/// table of per-address entries with whitelist / blacklist /
/// unblock controls. POST performs the selected action.
pub const PATH_SECURITY: &str = "/admin/security";
/// Admin management view (GET): lists wallet admins and renders
/// add / remove forms.
/// (POST): performs the requested mutation.
pub const PATH_ADMINS:  &str = "/admin/admins";
/// Oxanium variable font. Served unauthenticated so the login
/// page can use it for headings before the visitor has a session.
pub const PATH_ASSET_OXANIUM: &str = "/admin/assets/oxanium.ttf";
/// Signed-admin-login challenge (GET): returns a JDAT body
/// describing the expected command name and the server's current
/// timestamp so a client can align its SignedCommand envelope.
pub const PATH_CHALLENGE: &str = "/admin/challenge";
/// Signed-admin-login submission (POST): accepts a JDAT-encoded
/// [`SignedCommand`] with `cmd = "admin_login"`, verifies it
/// against the vhost's configured `admin_keys`, and issues the
/// same admin session cookie the passphrase flow does.
pub const PATH_SIGNED_LOGIN: &str = "/admin/signed-login";

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch a `GET` request against a `/admin/*` path.
pub async fn handle_get(
    state:   &AdminState,
    path:    &str,
    headers: &Arc<HeaderFields>,
    peer:    SocketAddr,
    id:      &str,
)
    -> Outcome<HttpMessage>
{
    let _ = peer; // GET routes do not yet audit by address.
    debug!("{}: dashboard GET {}", id, path);
    match path {
        PATH_ASSET_OXANIUM => Ok(serve_font_oxanium()),
        PATH_LOGIN => Ok(render_login_form(state.seal_withholds_data(), None)),
        PATH_LOGOUT => Ok(handle_logout(state, headers)),
        PATH_ROOT => Ok(render_home(state, headers)),
        PATH_TRAFFIC => Ok(render_traffic(state, headers)),
        PATH_HOST_JSON => Ok(render_host_json(state, headers)),
        PATH_TRAFFIC_JSON => Ok(render_traffic_json(state, headers)),
        PATH_SECURITY => Ok(render_security(state, headers, None)),
        PATH_ADMINS => Ok(render_admins(state, headers, None)),
        PATH_CHALLENGE => Ok(
            crate::srv::admin::signed_login::handle_challenge(state),
        ),
        _ => Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Dashboard route not found.",
        )),
    }
}

/// Serve the Oxanium variable font with a long-cache header so
/// the browser keeps one copy across dashboard navigations.
fn serve_font_oxanium() -> HttpMessage {
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("font/ttf".to_string()),
        )
        .with_field(
            HeaderName::CacheControl,
            HeaderFieldValue::Generic("public, max-age=86400, immutable".to_string()),
        )
        .with_body(crate::srv::admin::assets::FONT_OXANIUM_TTF.to_vec())
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
    peer:     SocketAddr,
    id:       &str,
)
    -> Outcome<HttpMessage>
{
    debug!("{}: dashboard POST {}", id, path);
    match path {
        PATH_LOGIN => Ok(handle_login(state, body, peer)),
        PATH_SIGNED_LOGIN => Ok(handle_signed_login(state, body)),
        PATH_SECURITY => Ok(handle_security_post(state, _headers, body)),
        PATH_ADMINS => Ok(handle_admins_post(state, _headers, body)),
        _ => Ok(HttpMessage::respond_with_text(
            HttpStatus::NotFound,
            "Dashboard route not found.",
        )),
    }
}

/// Verifies a signed-admin-login envelope and, on success, issues
/// the same admin session cookie the passphrase flow issues. A
/// failure returns a short JDAT error body rather than re-rendering
/// the passphrase login form -- the client is a programmatic caller,
/// not a browser form submission.
fn handle_signed_login(
    state: &AdminState,
    body:  &[u8],
)
    -> HttpMessage
{
    use crate::srv::admin::signed_login::{
        audit_signed_login,
        verify_signed_login,
        SignedLoginOutcome,
    };
    let outcome = verify_signed_login(state, body);
    audit_signed_login(&outcome);
    match outcome {
        SignedLoginOutcome::Ok(principal) => issue_session_cookie(
            state, &principal,
        ),
        _ => HttpMessage::respond_with_text(
            HttpStatus::Unauthorized,
            "Signed admin login rejected.",
        ),
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
    peer:  SocketAddr,
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
            return render_login_form(state.seal_withholds_data(), Some(
                "Login form did not include a passphrase."));
        },
    };

    let outcome = match auth::verify_passphrase(state, passphrase.as_bytes(), peer) {
        Ok(o) => o,
        Err(e) => {
            warn!("dashboard login: structural error during verify: {}", e);
            audit::append(
                ADMIN_ANON,
                VERB_DASHBOARD_LOGIN,
                "err",
                "reason=verify_structural_error",
            );
            return render_login_form(state.seal_withholds_data(), Some("Internal error during login."));
        },
    };

    match outcome {
        LoginOutcome::Ok(principal) => {
            audit::append(
                &principal.name,
                VERB_DASHBOARD_LOGIN,
                "ok",
                &fmt!("scopes={} src={}",
                    principal.scopes.join(","), peer.ip()),
            );
            issue_session_cookie(state, &principal)
        },
        LoginOutcome::BadCredentials => {
            audit::append(
                ADMIN_ANON,
                VERB_DASHBOARD_LOGIN,
                "err",
                &fmt!("reason=bad_credentials src={}", peer.ip()),
            );
            // Generic message: do not leak whether any admin exists.
            render_login_form(state.seal_withholds_data(), Some("Invalid credentials."))
        },
        LoginOutcome::NoDashboardScope { name } => {
            audit::append(
                &name,
                VERB_DASHBOARD_LOGIN,
                "err",
                &fmt!("reason=no_dashboard_scope src={}", peer.ip()),
            );
            warn!("dashboard login: admin '{}' authenticated but \
                holds no dashboard scope.", name);
            render_login_form(state.seal_withholds_data(), Some(
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
            return render_login_form(state.seal_withholds_data(), Some(
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
/// the visitor to the login form. Shows a four-card sparkline strip
/// fed by `/admin/host.json`, a welcome line, and a list of the
/// other live views.
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
    let host_block = render_host_sparkline_strip();
    let body = fmt!(
        "<h1>Overview</h1>\n\
        <p>Welcome, <strong>{}</strong>.</p>\n\
        <p class=\"meta\">Scopes: <code>{}</code></p>\n\
        {host}\
        <h2>Live views</h2>\n\
        <ul>\n\
            <li><a href=\"/admin/traffic\">Traffic</a> &mdash; \
                recent requests across every vhost on this host.</li>\n\
            <li><a href=\"/admin/database\">Database</a> &mdash; \
                browse keys and fetch values from this vhost's ozone database.</li>\n\
            <li><a href=\"/admin/security\">Security</a> &mdash; \
                address guard state and per-IP controls.</li>\n\
            <li><a href=\"/admin/admins\">Admins</a> &mdash; \
                manage wallet admin entries (requires <code>admin</code> scope).</li>\n\
        </ul>\n",
        html_escape(&principal.name),
        html_escape(&principal.scopes.join(" ")),
        host = host_block,
    );
    let head_extra = fmt!(
        "{uplot}\n<script>{spark}</script>\n<script>{refresh}</script>\n",
        uplot   = upload_head_html(),
        spark   = crate::srv::admin::assets::OVERVIEW_SPARKLINE_JS,
        refresh = crate::srv::admin::assets::AUTO_REFRESH_JS,
    );
    let html = render_layout(
        "Overview",
        "/admin",
        &principal,
        &body,
        &head_extra,
    );
    html_response(html)
}

/// Emit the four-card sparkline placeholder block. The inline JS
/// shipped alongside in `head_extra` fetches `/admin/host.json` and
/// populates each card's headline value and uPlot chart.
fn render_host_sparkline_strip() -> String {
    "<h2>Host resources</h2>\n\
    <p class=\"meta\">Last hour, sampled every 5 s. \
    Waiting on the first full pair of samples before charts draw.</p>\n\
    <div class=\"spark-row\">\n\
        <div class=\"spark-card\">\
            <div class=\"spark-header\">\
                <span class=\"spark-label\">CPU busy</span>\
                <span class=\"spark-value\" id=\"spark-cpu-val\">&mdash;</span>\
            </div>\
            <div class=\"spark-plot\" id=\"spark-cpu\"></div>\
        </div>\n\
        <div class=\"spark-card\">\
            <div class=\"spark-header\">\
                <span class=\"spark-label\">Memory used</span>\
                <span class=\"spark-value\" id=\"spark-mem-val\">&mdash;</span>\
            </div>\
            <div class=\"spark-plot\" id=\"spark-mem\"></div>\
        </div>\n\
        <div class=\"spark-card\">\
            <div class=\"spark-header\">\
                <span class=\"spark-label\">Disk I/O</span>\
                <span class=\"spark-value\" id=\"spark-disk-val\">&mdash;</span>\
            </div>\
            <div class=\"spark-plot\" id=\"spark-disk\"></div>\
        </div>\n\
        <div class=\"spark-card\">\
            <div class=\"spark-header\">\
                <span class=\"spark-label\">Network</span>\
                <span class=\"spark-value\" id=\"spark-net-val\">&mdash;</span>\
            </div>\
            <div class=\"spark-plot\" id=\"spark-net\"></div>\
        </div>\n\
    </div>\n".to_string()
}

/// Serialise the host sampler history as four time-aligned series.
///
/// The series are computed at the later-of-pair timestamp because
/// the rate-based figures (CPU busy, disk throughput, network
/// throughput) require two snapshots. Memory is a level metric and
/// is emitted at the same later-of-pair timestamp for alignment.
fn render_host_json(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> HttpMessage
{
    if extract_principal(state, headers).is_none() {
        return HttpMessage::respond_with_text(
            HttpStatus::Unauthorized,
            "Sign in required.",
        );
    }

    let merged = match state.host_sampler.merged_derived_history() {
        Ok(v) => v,
        Err(e) => {
            error!(e, "dashboard: host merged_derived_history failed");
            return HttpMessage::respond_with_text(
                HttpStatus::InternalServerError,
                "Host sampler error.",
            );
        },
    };

    // No points yet (fewer than two live samples and nothing persisted).
    // Emit an empty payload so the browser-side JS can render the
    // "warming up" state cleanly rather than throw on undefined.
    if merged.is_empty() {
        let empty = "{\"t\":[],\"cpu\":[],\"mem\":[],\"disk\":[],\"net\":[]}";
        return HttpMessage::new_response(HttpStatus::OK)
            .with_field(
                HeaderName::ContentType,
                HeaderFieldValue::Generic(
                    "application/json; charset=utf-8".to_string()),
            )
            .with_field(
                HeaderName::CacheControl,
                HeaderFieldValue::Generic("no-store".to_string()),
            )
            .with_body(empty.as_bytes().to_vec());
    }

    let mut ts = String::from("[");
    let mut cpu = String::from("[");
    let mut mem = String::from("[");
    let mut disk = String::from("[");
    let mut net = String::from("[");
    for p in &merged {
        if !ts.ends_with('[') {
            ts.push(',');
            cpu.push(',');
            mem.push(',');
            disk.push(',');
            net.push(',');
        }
        ts.push_str(&fmt!("{}", p.t_secs));
        cpu.push_str(&format_float(p.cpu_pct));
        mem.push_str(&format_float(p.mem_pct));
        disk.push_str(&format_float(p.disk_bps));
        net.push_str(&format_float(p.net_bps));
    }
    ts.push(']');
    cpu.push(']');
    mem.push(']');
    disk.push(']');
    net.push(']');

    let body = fmt!(
        "{{\"t\":{t},\"cpu\":{cpu},\"mem\":{mem},\"disk\":{disk},\"net\":{net}}}",
        t    = ts,
        cpu  = cpu,
        mem  = mem,
        disk = disk,
        net  = net,
    );

    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic(
                "application/json; charset=utf-8".to_string()),
        )
        .with_field(
            HeaderName::CacheControl,
            HeaderFieldValue::Generic("no-store".to_string()),
        )
        .with_body(body.into_bytes())
}

/// Format an `f64` as a JSON number with one decimal place. Used by
/// the host JSON emitter so the response size stays bounded and
/// matches what uPlot will render anyway. `NaN` and infinities fall
/// back to `0` so the emitted document is always valid JSON.
fn format_float(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let scaled = (v * 10.0).round() as i64;
    let whole = scaled / 10;
    let frac  = (scaled % 10).abs();
    fmt!("{}.{}", whole, frac)
}

/// JSON feed for the Traffic view. Returns counters, chart series,
/// and the recent-requests table in a single payload that the
/// auto-refresh client uses to update each section without
/// reloading the page.
fn render_traffic_json(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
)
    -> HttpMessage
{
    if extract_principal(state, headers).is_none() {
        return HttpMessage::respond_with_text(
            HttpStatus::Unauthorized,
            "Sign in required.",
        );
    }

    let counters = match state.traffic.counters_snapshot() {
        Ok(c) => c,
        Err(e) => {
            error!(e, "dashboard: traffic counters_snapshot() failed");
            crate::srv::admin::traffic::CountersSnapshot::default()
        },
    };
    let history = match state.traffic.history_snapshot() {
        Ok(v) => v,
        Err(e) => {
            error!(e, "dashboard: traffic history_snapshot() failed");
            Vec::new()
        },
    };
    let recent = match state.traffic.recent(50) {
        Ok(v) => v,
        Err(e) => {
            error!(e, "dashboard: traffic recent() failed");
            Vec::new()
        },
    };

    let mut status_keys: Vec<u16> = counters.by_status.keys().copied().collect();
    status_keys.sort();
    let rate_last = compute_rate_last(&history);
    let chart_json = build_chart_json(&history, &status_keys);

    // Build the counters JSON by hand to stay dependency-free.
    let mut by_status_json = String::from("[");
    for (i, s) in status_keys.iter().enumerate() {
        if i > 0 { by_status_json.push(','); }
        let count = counters.by_status.get(s).copied().unwrap_or(0);
        by_status_json.push_str(&fmt!(
            "{{\"code\":{c},\"count\":{n}}}",
            c = s,
            n = count,
        ));
    }
    by_status_json.push(']');

    let mut recent_json = String::from("[");
    for (i, r) in recent.iter().enumerate() {
        if i > 0 { recent_json.push(','); }
        recent_json.push_str(&fmt!(
            "{{\"method\":\"{m}\",\"vhost\":\"{v}\",\"path\":\"{p}\",\
            \"status\":{s},\"duration\":\"{d}\"}}",
            m = json_escape(&r.method),
            v = json_escape(&r.vhost),
            p = json_escape(&r.path),
            s = r.status,
            d = json_escape(&format_duration_us(r.duration_us)),
        ));
    }
    recent_json.push(']');

    let body = fmt!(
        "{{\"counters\":{{\"total\":{total},\"rate\":{rate},\
        \"by_status\":{by_status}}},\
        \"chart\":{chart},\"recent\":{recent}}}",
        total     = counters.total,
        rate      = format_float(rate_last),
        by_status = by_status_json,
        chart     = chart_json,
        recent    = recent_json,
    );

    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic(
                "application/json; charset=utf-8".to_string()),
        )
        .with_field(
            HeaderName::CacheControl,
            HeaderFieldValue::Generic("no-store".to_string()),
        )
        .with_body(body.into_bytes())
}

/// Escape a string for inclusion inside a JSON string literal. Only
/// covers the characters actually reachable through `RequestRecord`
/// (backslash, double quote, control characters). Not a general
/// JSON encoder.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&fmt!("\\u{:04x}", c as u32));
            },
            c => out.push(c),
        }
    }
    out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TRAFFIC                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the live traffic view. Reads the shared
/// `TrafficRecorder` from `AdminState` and emits:
///
/// - A headline card with the monotonic total, the per-status
///   summary, and the rate over the last sample window.
/// - A uPlot chart drawn from the periodic sample history,
///   showing request rate over time.
/// - The most recent 50 request records as a table.
///
/// The chart is drawn client-side by a small inline JavaScript
/// fragment that reads a JSON blob embedded in the page. No
/// polling or auto-refresh yet -- the view is a synchronous
/// snapshot of whatever the recorder contains at the moment of
/// the request.
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
    let history = match state.traffic.history_snapshot() {
        Ok(v) => v,
        Err(e) => {
            error!(e, "dashboard: traffic history_snapshot() failed");
            Vec::new()
        },
    };

    // Collect per-status codes in sorted order so the legend is
    // stable across refreshes.
    let mut status_keys: Vec<u16> = counters.by_status
        .keys().copied().collect();
    status_keys.sort();

    let rate_last = compute_rate_last(&history);

    // Build the headline strip.
    let mut headline_chips = String::new();
    headline_chips.push_str(&fmt!(
        "<div class=\"chip chip-total\">\
            <div class=\"chip-label\">Total</div>\
            <div class=\"chip-value\">{}</div>\
            <div class=\"chip-sub\">requests since startup</div>\
        </div>\n",
        counters.total,
    ));
    headline_chips.push_str(&fmt!(
        "<div class=\"chip\">\
            <div class=\"chip-label\">Rate</div>\
            <div class=\"chip-value\">{rate:.2}</div>\
            <div class=\"chip-sub\">requests / sec (last interval)</div>\
        </div>\n",
        rate = rate_last,
    ));
    for s in &status_keys {
        let count = counters.by_status.get(s).copied().unwrap_or(0);
        let cls = chip_class_for_status(*s);
        headline_chips.push_str(&fmt!(
            "<div class=\"chip {cls}\">\
                <div class=\"chip-label\">HTTP {status}</div>\
                <div class=\"chip-value\">{count}</div>\
                <div class=\"chip-sub\">{desc}</div>\
            </div>\n",
            cls    = cls,
            status = s,
            count  = count,
            desc   = status_description(*s),
        ));
    }

    // Build the chart data as JSON. Each series is the delta
    // between adjacent samples (so the chart shows
    // "requests-in-this-sample-interval", not cumulative
    // totals). Timestamps are unix seconds; uPlot expects
    // numeric x values.
    let chart_json = build_chart_json(&history, &status_keys);

    // Build the recent-requests table.
    let recent_html = if recent.is_empty() {
        "<p class=\"notice empty\">\
         No requests recorded yet. Interact with the dashboard \
         or visit any vhost and this table will populate.\
         </p>".to_string()
    } else {
        let mut rows = String::new();
        for r in &recent {
            let status_cls = chip_class_for_status(r.status);
            rows.push_str(&fmt!(
                "<tr>\
                <td><code>{method}</code></td>\
                <td>{vhost}</td>\
                <td class=\"path\"><code>{path}</code></td>\
                <td class=\"status {cls}\">{status}</td>\
                <td class=\"num\">{dur}</td>\
                </tr>\n",
                method = html_escape(&r.method),
                vhost  = html_escape(&r.vhost),
                path   = html_escape(&r.path),
                cls    = status_cls,
                status = r.status,
                dur    = format_duration_us(r.duration_us),
            ));
        }
        fmt!(
            "<h2>Recent requests</h2>\n\
            <table class=\"steel-table\">\n\
            <thead><tr>\
            <th>Method</th><th>Vhost</th><th>Path</th>\
            <th class=\"status\">Status</th><th class=\"num\">Duration</th>\
            </tr></thead>\n\
            <tbody id=\"traffic-recent-body\">{}</tbody>\n\
            </table>\n",
            rows,
        )
    };

    // Page body. The chart container has a fixed height so
    // uPlot can measure it before the JSON arrives.
    let body = fmt!(
        "<h1>Traffic</h1>\n\
        <div class=\"chip-row\" id=\"traffic-chip-row\">\n{chips}</div>\n\
        <h2>Requests per sample interval</h2>\n\
        <div id=\"traffic-chart\" class=\"chart-panel\"></div>\n\
        <script id=\"traffic-chart-data\" type=\"application/json\">\n\
        {chart_json}\n\
        </script>\n\
        <script>\n{chart_js}\n</script>\n\
        <script>\n{refresh_js}\n</script>\n\
        {recent}",
        chips      = headline_chips,
        chart_json = chart_json,
        chart_js   = TRAFFIC_CHART_JS,
        refresh_js = crate::srv::admin::assets::AUTO_REFRESH_JS,
        recent     = recent_html,
    );
    let html = render_layout(
        "Traffic",
        PATH_TRAFFIC,
        &principal,
        &body,
        &upload_head_html(),
    );
    html_response(html)
}

/// Client-side glue that reads the chart JSON blob embedded in
/// the page and draws a stacked requests-per-interval chart
/// with uPlot. Exposes `window.steelTrafficRefresh()` so the
/// auto-refresh polling loop can repaint the chip row, chart
/// and recent-requests table without reloading the page.
const TRAFFIC_CHART_JS: &str = r#"
(function() {
    var el = document.getElementById('traffic-chart');
    var dataEl = document.getElementById('traffic-chart-data');
    if (!el || typeof uPlot === 'undefined') return;
    var chart = null;
    var chartSeriesLabels = [];
    var palette = ['#f33c57', '#1976d2', '#2e7d32', '#ed6c02',
                   '#6a1b9a', '#00838f', '#455a64'];
    function classForStatus(s) {
        if (s >= 200 && s < 300) return 'chip-ok';
        if (s >= 300 && s < 400) return 'chip-redirect';
        if (s >= 400 && s < 500) return 'chip-client';
        if (s >= 500 && s < 600) return 'chip-server';
        return '';
    }
    function descForStatus(s) {
        if (s >= 200 && s < 300) return 'success';
        if (s >= 300 && s < 400) return 'redirect';
        if (s >= 400 && s < 500) return 'client error';
        if (s >= 500 && s < 600) return 'server error';
        return 'informational';
    }
    function buildChart(payload) {
        var series = [{}];
        chartSeriesLabels = [];
        for (var i = 0; i < payload.series.length; i++) {
            series.push({
                label:  payload.series[i].label,
                stroke: palette[i % palette.length],
                width:  2,
                fill:   palette[i % palette.length] + '22',
                paths:  uPlot.paths.stepped({align: 1}),
            });
            chartSeriesLabels.push(payload.series[i].label);
        }
        var data = [payload.t];
        for (var j = 0; j < payload.series.length; j++) {
            data.push(payload.series[j].values);
        }
        var opts = {
            width:  el.clientWidth || 800,
            height: 260,
            scales: {x: {time: true}},
            axes: [
                {stroke: '#666', grid: {stroke: '#eee'}},
                {stroke: '#666', grid: {stroke: '#eee'}},
            ],
            series: series,
            legend: {live: false},
        };
        chart = new uPlot(opts, data, el);
    }
    function chartSchemaMatches(payload) {
        if (payload.series.length !== chartSeriesLabels.length) return false;
        for (var i = 0; i < payload.series.length; i++) {
            if (payload.series[i].label !== chartSeriesLabels[i]) return false;
        }
        return true;
    }
    function updateChart(payload) {
        if (!payload.t || payload.t.length < 2) {
            if (!chart) {
                el.innerHTML = '<p class="notice empty">Not enough samples yet. '
                    + 'The chart appears after two sample intervals '
                    + '(~10 seconds at the default cadence).</p>';
            }
            return;
        }
        if (!chart || !chartSchemaMatches(payload)) {
            if (chart) { chart.destroy(); chart = null; }
            el.innerHTML = '';
            buildChart(payload);
            return;
        }
        var data = [payload.t];
        for (var j = 0; j < payload.series.length; j++) {
            data.push(payload.series[j].values);
        }
        chart.setData(data);
    }
    function updateChips(counters) {
        var row = document.getElementById('traffic-chip-row');
        if (!row) return;
        var parts = [];
        parts.push(
            '<div class="chip chip-total">'
            + '<div class="chip-label">Total</div>'
            + '<div class="chip-value">' + counters.total + '</div>'
            + '<div class="chip-sub">requests since startup</div>'
            + '</div>'
        );
        parts.push(
            '<div class="chip">'
            + '<div class="chip-label">Rate</div>'
            + '<div class="chip-value">' + counters.rate.toFixed(2) + '</div>'
            + '<div class="chip-sub">requests / sec (last interval)</div>'
            + '</div>'
        );
        counters.by_status.forEach(function(entry) {
            parts.push(
                '<div class="chip ' + classForStatus(entry.code) + '">'
                + '<div class="chip-label">HTTP ' + entry.code + '</div>'
                + '<div class="chip-value">' + entry.count + '</div>'
                + '<div class="chip-sub">' + descForStatus(entry.code) + '</div>'
                + '</div>'
            );
        });
        row.innerHTML = parts.join('');
    }
    function updateRecent(recent) {
        var body = document.getElementById('traffic-recent-body');
        if (!body) return;
        if (!recent.length) return;
        var html = '';
        for (var i = 0; i < recent.length; i++) {
            var r = recent[i];
            html += '<tr>'
                + '<td><code>' + r.method + '</code></td>'
                + '<td>' + r.vhost + '</td>'
                + '<td class="path"><code>' + r.path + '</code></td>'
                + '<td class="status ' + classForStatus(r.status) + '">' + r.status + '</td>'
                + '<td class="num">' + r.duration + '</td>'
                + '</tr>';
        }
        body.innerHTML = html;
    }
    function refreshFrom(payload) {
        if (!payload) return;
        updateChips(payload.counters);
        updateChart(payload.chart);
        updateRecent(payload.recent);
    }
    function refresh() {
        fetch('/admin/traffic.json', { credentials: 'same-origin' })
            .then(function(r) { return r.ok ? r.json() : null; })
            .then(refreshFrom)
            .catch(function() {});
    }
    // First paint: prefer the inline blob for a zero-RTT render,
    // fall back to the JSON feed if the blob is missing or empty.
    if (dataEl) {
        try {
            var payload = JSON.parse(dataEl.textContent);
            updateChart(payload);
        } catch (e) {
            refresh();
        }
    } else {
        refresh();
    }
    window.steelTrafficRefresh = refresh;
    window.addEventListener('resize', function() {
        if (chart) {
            chart.setSize({ width: el.clientWidth, height: 260 });
        }
    });
})();
"#;

/// Compute the requests-per-second rate over the last sample
/// interval in `history`. Returns 0.0 when there are fewer than
/// two samples.
fn compute_rate_last(history: &[crate::srv::admin::traffic::TrafficSample]) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }
    let last = &history[history.len() - 1];
    let prev = &history[history.len() - 2];
    if last.when_secs <= prev.when_secs {
        return 0.0;
    }
    let dt = (last.when_secs - prev.when_secs) as f64;
    let dn = last.total.saturating_sub(prev.total) as f64;
    if dt == 0.0 { 0.0 } else { dn / dt }
}

/// Serialise the traffic history into the JSON shape the chart
/// script expects: `{t: [unix_ts...], series: [{label, values}]}`
/// where each `values` array is the delta in that status's
/// counter since the previous sample (i.e. "requests in this
/// sample interval").
fn build_chart_json(
    history:     &[crate::srv::admin::traffic::TrafficSample],
    status_keys: &[u16],
)
    -> String
{
    if history.len() < 2 {
        return "{\"t\": [], \"series\": []}".to_string();
    }
    let mut ts = String::from("[");
    let mut deltas: Vec<Vec<u64>> = status_keys.iter()
        .map(|_| Vec::with_capacity(history.len() - 1))
        .collect();
    for w in history.windows(2) {
        let prev = &w[0];
        let curr = &w[1];
        if !ts.ends_with('[') {
            ts.push(',');
        }
        ts.push_str(&fmt!("{}", curr.when_secs));
        for (i, s) in status_keys.iter().enumerate() {
            let a = prev.by_status.get(s).copied().unwrap_or(0);
            let b = curr.by_status.get(s).copied().unwrap_or(0);
            deltas[i].push(b.saturating_sub(a));
        }
    }
    ts.push(']');

    let mut series = String::from("[");
    for (i, s) in status_keys.iter().enumerate() {
        if i > 0 { series.push(','); }
        let values = deltas[i].iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        series.push_str(&fmt!(
            "{{\"label\":\"{label}\",\"values\":[{values}]}}",
            label  = fmt!("HTTP {}", s),
            values = values,
        ));
    }
    series.push(']');

    fmt!("{{\"t\":{},\"series\":{}}}", ts, series)
}

/// Pick a CSS class for a status chip based on the HTTP class.
fn chip_class_for_status(status: u16) -> &'static str {
    match status {
        200..=299 => "chip-ok",
        300..=399 => "chip-redirect",
        400..=499 => "chip-client",
        500..=599 => "chip-server",
        _         => "",
    }
}

/// Human-readable one-word description for a status code class.
fn status_description(status: u16) -> &'static str {
    match status {
        200..=299 => "success",
        300..=399 => "redirect",
        400..=499 => "client error",
        500..=599 => "server error",
        _         => "informational",
    }
}

/// Format a microsecond duration as a compact human string.
fn format_duration_us(us: u64) -> String {
    if us < 1_000 {
        fmt!("{} \u{00b5}s", us)
    } else if us < 1_000_000 {
        fmt!("{:.1} ms", (us as f64) / 1_000.0)
    } else {
        fmt!("{:.2} s", (us as f64) / 1_000_000.0)
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ADMIN MANAGEMENT                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Outcome of an admin-management mutation, threaded back into
/// the rendered list view as a notice banner above the form.
struct AdminFlash {
    /// `true` for a green "ok" notice, `false` for a red error.
    ok:      bool,
    message: String,
}

/// Render the admin management view: list of wallet admins and
/// add / remove forms. Authorisation requires both a dashboard
/// scope (so the visitor can see the dashboard at all) and the
/// legacy `admin` scope (which gates admin enrolment in both the
/// CLI and the dashboard). A visitor with only `dashboard.view`
/// or only `dashboard.admin` is sent the "forbidden" flavour of
/// the page rather than a redirect to login -- they are signed
/// in, just not authorised for this verb.
fn render_admins(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
    flash:   Option<AdminFlash>,
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    if !principal.can_manage_admins() {
        return render_admin_forbidden(&principal);
    }

    // Snapshot the admin list out of the wallet under a short
    // read lock so the rendering does not hold the lock across
    // any HTML formatting.
    let admins_snapshot: Vec<(String, u64, Vec<String>)> = {
        let w = match state.wallet.read() {
            Ok(g) => g,
            Err(_) => return render_admins_error(
                &principal,
                "Wallet lock is poisoned.",
            ),
        };
        w.admins().iter()
            .map(|a| (a.name.clone(), a.expires_at, a.scopes.clone()))
            .collect()
    };

    let flash_html = render_flash(flash.as_ref());
    let mut rows = String::new();
    for (name, expires_at, scopes) in &admins_snapshot {
        let expiry = if *expires_at == 0 {
            "never".to_string()
        } else {
            fmt!("unix {}", expires_at)
        };
        let scopes_html = if scopes.is_empty() {
            "<em>(none)</em>".to_string()
        } else {
            html_escape(&scopes.join(", "))
        };
        let safe_name = html_escape(name);
        rows.push_str(&fmt!(
            "<tr>\
            <td><strong>{name}</strong></td>\
            <td>{expiry}</td>\
            <td><code>{scopes}</code></td>\
            <td><form method=\"POST\" action=\"/admin/admins\" \
                    onsubmit=\"return confirm('Remove admin {name}?');\">\
                <input type=\"hidden\" name=\"action\" value=\"remove\">\
                <input type=\"hidden\" name=\"name\" value=\"{name_attr}\">\
                <button type=\"submit\" class=\"primary\">Remove</button>\
                </form></td>\
            </tr>\n",
            name      = safe_name,
            expiry    = expiry,
            scopes    = scopes_html,
            name_attr = safe_name,
        ));
    }
    let body = fmt!(
        "<h1>Admin management</h1>\n\
        {flash}\
        <p>Wallet currently holds <strong>{count}</strong> admin entries. \
        Adding a new admin enrols their password against the same \
        wallet master key the CLI <code>admin --add</code> verb \
        uses; removing an admin revokes their password immediately. \
        Every action here is recorded in <code>admin-audit.log</code> \
        alongside the CLI events.</p>\n\
        <h2>Existing admins</h2>\n\
        <table class=\"steel-table\">\n\
        <thead><tr>\
            <th>Name</th><th>Expires</th><th>Scopes</th><th>Actions</th>\
        </tr></thead>\n\
        <tbody>{rows}</tbody>\n\
        </table>\n\
        <h2>Add an admin</h2>\n\
        <form class=\"steel-form\" method=\"POST\" action=\"/admin/admins\">\n\
        <input type=\"hidden\" name=\"action\" value=\"add\">\n\
        <label for=\"new_name\">Name</label>\n\
        <input type=\"text\" id=\"new_name\" name=\"name\" required \
            autocomplete=\"off\">\n\
        <label for=\"new_password\">Password</label>\n\
        <input type=\"password\" id=\"new_password\" name=\"password\" \
            required autocomplete=\"new-password\">\n\
        <label for=\"new_scopes\">Scopes (comma-separated)</label>\n\
        <input type=\"text\" id=\"new_scopes\" name=\"scopes\" \
            value=\"dashboard.view\" \
            placeholder=\"dashboard.view, admin\">\n\
        <p class=\"meta\">Well-known scopes: \
        <code>admin</code>, <code>dashboard.view</code>, \
        <code>dashboard.admin</code>. Use <code>*</code> for \
        operator-level access.</p>\n\
        <label for=\"new_expires_in\">Expires in (seconds, 0 = never)</label>\n\
        <input type=\"number\" id=\"new_expires_in\" name=\"expires_in\" \
            value=\"0\" min=\"0\">\n\
        <button type=\"submit\">Add admin</button>\n\
        </form>\n",
        flash = flash_html,
        count = admins_snapshot.len(),
        rows  = rows,
    );
    let html = render_layout(
        "Admins",
        PATH_ADMINS,
        &principal,
        &body,
        "",
    );
    html_response(html)
}

/// Render the "you are signed in but not allowed here" page
/// shown to a principal whose scopes do not include both
/// `admin` and a dashboard scope.
fn render_admin_forbidden(principal: &AdminPrincipal) -> HttpMessage {
    let body = "<h1>Admin management</h1>\n\
        <p class=\"notice error\">\
        You are signed in to the dashboard but your admin entry \
        does not hold the <code>admin</code> scope. Admin \
        management requires both a dashboard scope \
        (<code>dashboard.view</code> or <code>dashboard.admin</code>) \
        and the <code>admin</code> scope. Ask another operator to \
        grant <code>admin</code> via <code>./steel admin --add</code> \
        if you need to enrol new admin entries from this dashboard.\
        </p>\n".to_string();
    let html = render_layout(
        "Admins",
        PATH_ADMINS,
        principal,
        &body,
        "",
    );
    html_response(html)
}

/// Render an error variant of the admins page when the wallet
/// itself cannot be read.
fn render_admins_error(principal: &AdminPrincipal, message: &str) -> HttpMessage {
    let body = fmt!(
        "<h1>Admin management</h1>\n\
        <p class=\"notice error\">{}</p>\n",
        html_escape(message),
    );
    let html = render_layout(
        "Admins",
        PATH_ADMINS,
        principal,
        &body,
        "",
    );
    html_response(html)
}

/// Render the flash banner above the admin list, if any.
fn render_flash(flash: Option<&AdminFlash>) -> String {
    match flash {
        None => String::new(),
        Some(f) => fmt!(
            "<p class=\"notice {}\">{}</p>\n",
            if f.ok { "" } else { "error" },
            html_escape(&f.message),
        ),
    }
}

/// Dispatch a `POST /admin/admins` form submission. The `action`
/// field selects between `add` and `remove`. Authorisation is the
/// same as for the GET view: dashboard scope plus `admin`.
fn handle_admins_post(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
    body:    &[u8],
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    if !principal.can_manage_admins() {
        return render_admin_forbidden(&principal);
    }

    let action = extract_form_field(body, "action").unwrap_or_default();
    let flash = match action.as_str() {
        "add" => handle_admin_add(state, &principal, body),
        "remove" => handle_admin_remove(state, &principal, body),
        _ => AdminFlash {
            ok:      false,
            message: "Unknown action.".to_string(),
        },
    };
    render_admins(state, headers, Some(flash))
}

/// Add a new wallet admin entry from form fields. Validates the
/// inputs, computes `expires_at` from the optional `expires_in`
/// duration, takes a write lock on the wallet, calls
/// `Wallet::enrol`, saves the wallet to disk, and emits an
/// audit log line.
fn handle_admin_add(
    state:     &AdminState,
    principal: &AdminPrincipal,
    body:      &[u8],
)
    -> AdminFlash
{
    let new_name = extract_form_field(body, "name").unwrap_or_default();
    let new_pass = extract_form_field(body, "password").unwrap_or_default();
    let scopes_raw = extract_form_field(body, "scopes").unwrap_or_default();
    let expires_in_raw = extract_form_field(body, "expires_in")
        .unwrap_or_default();

    if new_name.is_empty() || new_pass.is_empty() {
        return AdminFlash {
            ok:      false,
            message: "Name and password are required.".to_string(),
        };
    }
    let new_scopes: Vec<String> = scopes_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let expires_in: u64 = expires_in_raw.parse::<u64>().unwrap_or(0);
    let expires_at = if expires_in == 0 {
        0
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_add(expires_in)
    };

    // Enrolling wraps the master key under the new admin's password, so
    // it cannot be done while sealed. In practice this is unreachable
    // from the dashboard -- signing in *is* the unseal -- but the state
    // is shared, and a caller reaching here without a key deserves a
    // straight answer rather than a panic.
    let master_key = match state.master_key() {
        Ok(k) => k,
        Err(_) => return AdminFlash {
            ok:      false,
            message: "Steel is sealed: no master key is loaded, so a new \
                admin cannot be enrolled.".to_string(),
        },
    };
    let result = {
        let mut w = match state.wallet.write() {
            Ok(g) => g,
            Err(_) => return AdminFlash {
                ok:      false,
                message: "Wallet lock is poisoned.".to_string(),
            },
        };
        let enrol_res = w.enrol(
            &master_key,
            new_name.clone(),
            new_pass.as_bytes(),
            new_scopes.clone(),
            expires_at,
            DEFAULT_WALLET_KDF_NAME,
        );
        if let Err(e) = enrol_res {
            audit::append(
                &principal.name,
                VERB_DASHBOARD_ADMIN_ADD,
                "err",
                &fmt!("target={} reason={}", new_name, e),
            );
            return AdminFlash {
                ok:      false,
                message: fmt!("Failed to enrol '{}': {}", new_name, e),
            };
        }
        w.save(
            &state.wallet_path,
            "  ",
            Some(EncoderConfig::<(), ()>::default()),
        )
    };
    if let Err(e) = result {
        audit::append(
            &principal.name,
            VERB_DASHBOARD_ADMIN_ADD,
            "err",
            &fmt!("target={} reason=save_failed:{}", new_name, e),
        );
        return AdminFlash {
            ok:      false,
            message: fmt!(
                "Admin enrolled in memory but the wallet could not be \
                saved to disk: {}", e),
        };
    }
    audit::append(
        &principal.name,
        VERB_DASHBOARD_ADMIN_ADD,
        "ok",
        &fmt!(
            "target={} scopes={} expires_at={}",
            new_name, new_scopes.join(","), expires_at,
        ),
    );
    AdminFlash {
        ok:      true,
        message: fmt!("Added admin '{}'.", new_name),
    }
}

/// Remove a wallet admin entry by name. Validates the input,
/// takes a write lock, calls `Wallet::remove_by_name`, saves,
/// and audit-logs.
fn handle_admin_remove(
    state:     &AdminState,
    principal: &AdminPrincipal,
    body:      &[u8],
)
    -> AdminFlash
{
    let target = extract_form_field(body, "name").unwrap_or_default();
    if target.is_empty() {
        return AdminFlash {
            ok:      false,
            message: "Missing target name.".to_string(),
        };
    }
    let result = {
        let mut w = match state.wallet.write() {
            Ok(g) => g,
            Err(_) => return AdminFlash {
                ok:      false,
                message: "Wallet lock is poisoned.".to_string(),
            },
        };
        let remove_res = w.remove_by_name(&target);
        if let Err(e) = remove_res {
            audit::append(
                &principal.name,
                VERB_DASHBOARD_ADMIN_REMOVE,
                "err",
                &fmt!("target={} reason={}", target, e),
            );
            return AdminFlash {
                ok:      false,
                message: fmt!("Failed to remove '{}': {}", target, e),
            };
        }
        w.save(
            &state.wallet_path,
            "  ",
            Some(EncoderConfig::<(), ()>::default()),
        )
    };
    if let Err(e) = result {
        audit::append(
            &principal.name,
            VERB_DASHBOARD_ADMIN_REMOVE,
            "err",
            &fmt!("target={} reason=save_failed:{}", target, e),
        );
        return AdminFlash {
            ok:      false,
            message: fmt!(
                "Admin removed in memory but the wallet could not be \
                saved to disk: {}", e),
        };
    }
    audit::append(
        &principal.name,
        VERB_DASHBOARD_ADMIN_REMOVE,
        "ok",
        &fmt!("target={}", target),
    );
    AdminFlash {
        ok:      true,
        message: fmt!("Removed admin '{}'.", target),
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SECURITY                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// One-shot flash banner threaded back into the security page after
/// a POST mutation.
struct SecurityFlash {
    ok:      bool,
    message: String,
}

/// Render the Security view: a chip row of per-state counts, a table
/// of observed addresses with whitelist / blacklist / unblock buttons,
/// and a manual blacklist form for operators that need to pre-block
/// a known-bad IP.
///
/// The read path only needs `dashboard.view`; any mutations from the
/// accompanying POST handler require `dashboard.admin`.
fn render_security(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
    flash:   Option<SecurityFlash>,
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    let snap = match state.addr_guard.snapshot(DEFAULT_SNAPSHOT_CAP) {
        Ok(s) => s,
        Err(e) => {
            error!(e, "dashboard: addr guard snapshot failed");
            return render_security_error(&principal, "Address guard is unavailable.");
        },
    };

    let flash_html = match flash.as_ref() {
        Some(f) => fmt!(
            "<p class=\"notice {}\">{}</p>\n",
            if f.ok { "" } else { "error" },
            html_escape(&f.message),
        ),
        None => String::new(),
    };

    let can_mutate = principal.can_admin_dashboard();
    let chip_row = fmt!(
        "<div class=\"chip-row\">\n\
            <div class=\"chip\">\
                <div class=\"chip-label\">Monitored</div>\
                <div class=\"chip-value\">{mon}</div>\
                <div class=\"chip-sub\">rate-limited only</div>\
            </div>\n\
            <div class=\"chip\">\
                <div class=\"chip-label\">Throttled</div>\
                <div class=\"chip-value\">{thr}</div>\
                <div class=\"chip-sub\">under active cooldown</div>\
            </div>\n\
            <div class=\"chip\">\
                <div class=\"chip-label\">Blacklisted</div>\
                <div class=\"chip-value\">{bl}</div>\
                <div class=\"chip-sub\">dropping every packet</div>\
            </div>\n\
            <div class=\"chip\">\
                <div class=\"chip-label\">Whitelisted</div>\
                <div class=\"chip-value\">{wl}</div>\
                <div class=\"chip-sub\">always allowed</div>\
            </div>\n\
        </div>\n",
        mon = snap.counts.monitor,
        thr = snap.counts.throttle,
        bl  = snap.counts.blacklist,
        wl  = snap.counts.whitelist,
    );

    // Stable ordering: blacklist first so attacks rise to the top,
    // then throttle, then monitor, then whitelist; within each state
    // by descending total_reqs. Using a local Vec::sort because the
    // snapshot itself emits entries in shard-traversal order.
    let mut entries = snap.entries;
    entries.sort_by(|a, b| {
        let rank = |label: &str| -> u8 {
            match label {
                "blacklist" => 0,
                "throttle"  => 1,
                "monitor"   => 2,
                "whitelist" => 3,
                _           => 4,
            }
        };
        let ra = rank(a.state);
        let rb = rank(b.state);
        if ra != rb { return ra.cmp(&rb); }
        b.total_reqs.cmp(&a.total_reqs)
    });

    let table_html = if entries.is_empty() {
        "<p class=\"notice empty\">No addresses observed yet.</p>\n".to_string()
    } else {
        let mut rows = String::new();
        for e in &entries {
            let ip = fmt!("{}", e.ip);
            let ip_attr = html_escape(&ip);
            let actions = if can_mutate {
                fmt!(
                    "<form method=\"POST\" action=\"/admin/security\" \
                            class=\"inline-form\">\
                        <input type=\"hidden\" name=\"ip\" value=\"{ip}\">\
                        <button type=\"submit\" name=\"action\" value=\"whitelist\">\
                            Whitelist</button>\
                        <button type=\"submit\" name=\"action\" value=\"blacklist\">\
                            Blacklist</button>\
                        <button type=\"submit\" name=\"action\" value=\"unblock\">\
                            Reset</button>\
                    </form>",
                    ip = ip_attr,
                )
            } else {
                "<em>view only</em>".to_string()
            };
            rows.push_str(&fmt!(
                "<tr>\
                <td><code>{ip}</code></td>\
                <td class=\"pill pill-{state_class}\">{state_label}</td>\
                <td>{total}</td>\
                <td>{thrcnt}</td>\
                <td>{actions}</td>\
                </tr>\n",
                ip          = html_escape(&ip),
                state_class = e.state,
                state_label = e.state,
                total       = e.total_reqs,
                thrcnt      = e.throttle_cnt,
                actions     = actions,
            ));
        }
        fmt!(
            "<table class=\"steel-table\">\n\
            <thead><tr>\
                <th>IP</th><th>State</th><th>Requests</th>\
                <th>Throttles</th><th>Actions</th>\
            </tr></thead>\n\
            <tbody>{rows}</tbody>\n\
            </table>\n",
            rows = rows,
        )
    };

    let manual_form_html = if can_mutate {
        "<h2>Block a specific address</h2>\n\
        <form class=\"steel-form\" method=\"POST\" action=\"/admin/security\">\n\
        <input type=\"hidden\" name=\"action\" value=\"blacklist\">\n\
        <label for=\"ip\">IP address</label>\n\
        <input type=\"text\" id=\"ip\" name=\"ip\" required \
            placeholder=\"1.2.3.4 or ::1\" autocomplete=\"off\">\n\
        <button type=\"submit\">Add to blacklist</button>\n\
        </form>\n".to_string()
    } else {
        String::new()
    };

    let body = fmt!(
        "<h1>Security</h1>\n\
        {flash}\
        <p>The address guard runs before the TLS handshake on every \
        incoming TCP connection. Blacklisted and throttled addresses \
        are dropped at the accept loop so they cost the server nothing \
        more than a SYN/ACK.</p>\n\
        {chips}\
        <h2>Observed addresses</h2>\n\
        <p class=\"meta\">Total observed: <strong>{total}</strong>. \
        Snapshot cap: {cap}. Showing {shown} rows.</p>\n\
        {table}\
        {manual}",
        flash  = flash_html,
        chips  = chip_row,
        total  = snap.counts.total,
        cap    = DEFAULT_SNAPSHOT_CAP,
        shown  = entries.len(),
        table  = table_html,
        manual = manual_form_html,
    );

    let html = render_layout("Security", PATH_SECURITY, &principal, &body, "");
    html_response(html)
}

/// Render an error variant of the security page.
fn render_security_error(principal: &AdminPrincipal, message: &str) -> HttpMessage {
    let body = fmt!(
        "<h1>Security</h1>\n\
        <p class=\"notice error\">{}</p>\n",
        html_escape(message),
    );
    let html = render_layout("Security", PATH_SECURITY, principal, &body, "");
    html_response(html)
}

/// Dispatch a `POST /admin/security` form submission. Requires
/// `dashboard.admin`: guard mutations are privileged. Actions are
/// `whitelist`, `blacklist`, and `unblock`; each takes a single
/// `ip` field.
fn handle_security_post(
    state:   &AdminState,
    headers: &Arc<HeaderFields>,
    body:    &[u8],
)
    -> HttpMessage
{
    let principal = match extract_principal(state, headers) {
        Some(p) => p,
        None => return redirect_to_login(),
    };
    if !principal.can_admin_dashboard() {
        return render_security_error(
            &principal,
            "You need the dashboard.admin scope to mutate the address guard.",
        );
    }

    let action_raw = extract_form_field(body, "action").unwrap_or_default();
    let ip_raw = extract_form_field(body, "ip").unwrap_or_default();
    let flash = apply_security_action(state, &principal, &action_raw, &ip_raw);
    render_security(state, headers, Some(flash))
}

/// Parse the `ip` form field, apply `action_raw`, emit an audit log
/// entry, and return a flash banner summarising the outcome.
fn apply_security_action(
    state:      &AdminState,
    principal:  &AdminPrincipal,
    action_raw: &str,
    ip_raw:     &str,
)
    -> SecurityFlash
{
    let ip = match ip_raw.parse::<std::net::IpAddr>() {
        Ok(ip) => ip,
        Err(e) => {
            audit::append(
                &principal.name,
                audit_verb_for(action_raw),
                "err",
                &fmt!("ip={} reason=parse:{}", ip_raw, e),
            );
            return SecurityFlash {
                ok:      false,
                message: fmt!("Not a valid IP address: '{}'.", ip_raw),
            };
        },
    };
    let (verb, result) = match action_raw {
        "whitelist" => (
            VERB_DASHBOARD_GUARD_WHITELIST,
            state.addr_guard.whitelist(&ip),
        ),
        "blacklist" => (
            VERB_DASHBOARD_GUARD_BLACKLIST,
            state.addr_guard.blacklist(&ip),
        ),
        "unblock" => (
            VERB_DASHBOARD_GUARD_UNBLOCK,
            state.addr_guard.unblock(&ip),
        ),
        other => {
            return SecurityFlash {
                ok:      false,
                message: fmt!("Unknown security action '{}'.", other),
            };
        },
    };
    match result {
        Ok(()) => {
            audit::append(&principal.name, verb, "ok", &fmt!("ip={}", ip));
            SecurityFlash {
                ok:      true,
                message: fmt!("{} {}.", action_label(action_raw), ip),
            }
        },
        Err(e) => {
            audit::append(
                &principal.name,
                verb,
                "err",
                &fmt!("ip={} reason={}", ip, e),
            );
            SecurityFlash {
                ok:      false,
                message: fmt!(
                    "Failed to {} {}: {}",
                    action_raw, ip, e,
                ),
            }
        },
    }
}

/// Map a form `action` value to the audit verb that should be
/// recorded when parsing fails before we know which branch to take.
fn audit_verb_for(action_raw: &str) -> &'static str {
    match action_raw {
        "whitelist" => VERB_DASHBOARD_GUARD_WHITELIST,
        "blacklist" => VERB_DASHBOARD_GUARD_BLACKLIST,
        "unblock"   => VERB_DASHBOARD_GUARD_UNBLOCK,
        _           => VERB_DASHBOARD_GUARD_UNBLOCK,
    }
}

/// Past-tense label used in the security flash banner so each
/// successful action reads as a natural sentence.
fn action_label(action_raw: &str) -> &'static str {
    match action_raw {
        "whitelist" => "Whitelisted",
        "blacklist" => "Blacklisted",
        "unblock"   => "Reset",
        _           => "Applied",
    }
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
///
/// When Steel is sealed the form says so, and says what signing in
/// will do. This is the cold-start path: the process is up and serving
/// its static sites, but the databases are shut until an admin's
/// passphrase unwraps the master key. The operator needs to know that
/// this form is the thing standing between them and their databases --
/// otherwise a sealed Steel looks like a healthy one that has
/// mysteriously lost its data.
fn render_login_form(sealed: bool, error_msg: Option<&str>) -> HttpMessage {
    let error_html = match error_msg {
        Some(msg) => fmt!(
            "<p class=\"notice error\">{}</p>",
            html_escape(msg),
        ),
        None => String::new(),
    };
    let sealed_html = if sealed {
        "<p class=\"notice warn\">\
        <strong>Steel is sealed.</strong> The websites are serving, but the \
        databases are shut: no master key is loaded. Signing in with an admin \
        passphrase unseals them.\
        </p>\n"
    } else {
        ""
    };
    let body = fmt!(
        "{sealed}{error}\
        <form class=\"steel-form\" method=\"POST\" action=\"/admin/login\">\n\
        <label for=\"passphrase\">Wallet passphrase</label>\n\
        <input type=\"password\" id=\"passphrase\" name=\"passphrase\" \
            autofocus required>\n\
        <button type=\"submit\">{action}</button>\n\
        </form>\n",
        sealed = sealed_html,
        error  = error_html,
        action = if sealed { "Sign in and unseal" } else { "Sign in" },
    );
    let html = render_login_layout(
        if sealed { "Sign in and unseal" } else { "Sign in" },
        &body,
    );
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

