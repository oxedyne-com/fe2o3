//! Embedded front-end assets and HTML layout helpers for the
//! admin dashboard.
//!
//! The dashboard's CSS is shipped as a single string compiled into
//! the `steel` binary via `include_str!`, then inlined into every
//! response inside a `<style>` tag. Inlining (rather than serving
//! `/admin/css/style.css` from a separate route) keeps the asset
//! pipeline trivial -- no extra route, no cache headers, no
//! authenticated-vs-unauthenticated path split for static files,
//! no separate disk directory to sync to a production host. The
//! stylesheet is small enough (~6 KB) that the per-response cost
//! is negligible.
//!
//! The visual style mirrors the Hematite documentation:
//!
//! - Primary red `rgb(243, 60, 87)` for accents, action buttons,
//!   and the header underline.
//! - Primary blue `rgb(171, 202, 222)` for inline code background.
//! - Light grey panels at `rgb(240, 240, 240)`.
//! - Libertinus Serif (with web fallback to Georgia) for body and
//!   headings, in keeping with the academic-technical-doc feel.
//! - Headings use small-caps and bold weight for the H1, italic
//!   greys for H2.
//!
//! Logo handling is deliberately text-only in v1: `Hematite`
//! followed by `Steel` in the accent colour. A real SVG mark can
//! be dropped in later by replacing the `brand_html` helper. The
//! complement Hematite docs at
//! `~/usr/complement/projects/oxedyne/projects/fe2o3/doc/Hematite/`
//! ship the canonical logo files; we deliberately do not pull
//! from that path at compile time so `fe2o3_steel` builds without
//! the complement tree being present.

use crate::srv::admin::AdminPrincipal;

use oxedyne_fe2o3_core::prelude::*;

/// Stylesheet inlined into every dashboard response. Read at
/// compile time from `assets/style.css` next to this file.
pub const STYLE_CSS: &str = include_str!("assets/style.css");

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NAV ENTRIES                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// One sidebar navigation entry. The `current` flag highlights
/// the entry corresponding to the currently rendered page.
pub struct NavEntry {
    pub label:  &'static str,
    pub href:   &'static str,
    pub group:  Option<&'static str>,
}

/// Standard nav layout for the dashboard. Sub-pages adjust the
/// `current` URL via [`render_layout`] to highlight the active
/// entry.
pub const NAV: &[NavEntry] = &[
    NavEntry { label: "Home",       href: "/admin",         group: Some("Dashboard") },
    NavEntry { label: "Traffic",    href: "/admin/traffic", group: None },
    NavEntry { label: "Ozone",      href: "/admin/ozone",   group: None },
    NavEntry { label: "Admins",     href: "/admin/admins",  group: Some("Management") },
    NavEntry { label: "Sign out",   href: "/admin/logout",  group: Some("Session") },
];

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LAYOUT                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Wrap a body fragment in the standard authenticated dashboard
/// layout: header bar with brand and signed-in user, sidebar with
/// nav entries, main content panel.
///
/// `current_path` is matched against each nav entry's `href` to
/// highlight the active link. `body_html` is the inner HTML for
/// the main content panel; callers are responsible for escaping
/// untrusted content within it.
pub fn render_layout(
    title:        &str,
    current_path: &str,
    principal:    &AdminPrincipal,
    body_html:    &str,
)
    -> String
{
    let nav_html = render_nav(current_path);
    fmt!(
        "<!doctype html>\n\
        <html lang=\"en\">\n\
        <head>\n\
        <meta charset=\"utf-8\">\n\
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
        <title>{title} - Steel admin</title>\n\
        <style>{css}</style>\n\
        </head>\n\
        <body>\n\
        <header class=\"steel-header\">\n\
            <div class=\"brand\">{brand}</div>\n\
            <div class=\"session\">\
                Signed in as <strong>{user}</strong>\
                &middot; <a href=\"/admin/logout\">Sign out</a>\
            </div>\n\
        </header>\n\
        <div class=\"page\">\n\
        {nav}\
        <main class=\"content\">\n\
        {body}\
        </main>\n\
        </div>\n\
        </body>\n\
        </html>\n",
        title    = html_escape(title),
        css      = STYLE_CSS,
        brand    = brand_html(),
        user     = html_escape(&principal.name),
        nav      = nav_html,
        body     = body_html,
    )
}

/// Standalone layout for unauthenticated pages (login form). No
/// sidebar, no header bar, just a centred card.
pub fn render_login_layout(
    title:     &str,
    body_html: &str,
)
    -> String
{
    fmt!(
        "<!doctype html>\n\
        <html lang=\"en\">\n\
        <head>\n\
        <meta charset=\"utf-8\">\n\
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
        <title>{title} - Steel admin</title>\n\
        <style>{css}</style>\n\
        </head>\n\
        <body class=\"login\">\n\
        <div class=\"login-card\">\n\
        <h1>{brand}</h1>\n\
        {body}\
        </div>\n\
        </body>\n\
        </html>\n",
        title = html_escape(title),
        css   = STYLE_CSS,
        brand = brand_html(),
        body  = body_html,
    )
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FRAGMENTS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the brand mark used in the header and login card. v1
/// is text-only: "Hematite Steel" with `Steel` in the accent
/// colour. Replace the function body to drop in a real SVG mark
/// when one is ready.
pub fn brand_html() -> String {
    "Hematite <span class=\"accent\">Steel</span>".to_string()
}

/// Render the sidebar with the nav entries from [`NAV`]. Entries
/// whose `href` matches `current_path` (or for which the
/// current path is a sub-path) are highlighted via the `current`
/// CSS class.
pub fn render_nav(current_path: &str) -> String {
    let mut out = String::new();
    out.push_str("<nav class=\"sidebar\">\n");
    let mut last_group: Option<&'static str> = None;
    for entry in NAV {
        if let Some(group) = entry.group {
            if last_group != Some(group) {
                out.push_str(&fmt!(
                    "<div class=\"group\">{}</div>\n",
                    html_escape(group),
                ));
                last_group = Some(group);
            }
        }
        let class = if is_current(current_path, entry.href) {
            " class=\"current\""
        } else {
            ""
        };
        out.push_str(&fmt!(
            "<a href=\"{}\"{}>{}</a>\n",
            entry.href,
            class,
            html_escape(entry.label),
        ));
    }
    out.push_str("</nav>\n");
    out
}

/// Decide whether a given nav entry should be highlighted as the
/// current page. The home entry (`/admin`) only matches an exact
/// `/admin`; every other entry matches its prefix so sub-pages
/// like `/admin/ozone?prefix=user:` still highlight `Ozone`.
fn is_current(current_path: &str, entry_href: &str) -> bool {
    if entry_href == "/admin" {
        current_path == "/admin"
    } else {
        current_path == entry_href
            || current_path.starts_with(&fmt!("{}/", entry_href))
            || current_path.starts_with(&fmt!("{}?", entry_href))
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HTML ESCAPE                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Escape a string for safe inclusion in an HTML text node or
/// attribute value. Replaces the five characters that can break
/// out of a text node into markup: `&`, `<`, `>`, `"`, `'`.
pub fn html_escape(s: &str) -> String {
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
