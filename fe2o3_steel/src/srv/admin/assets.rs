//! Embedded front-end assets and HTML layout helpers for the
//! admin dashboard.
//!
//! The dashboard's CSS is shipped as a single string compiled into
//! the `steel` binary via `include_str!`, then inlined into every
//! response inside a `<style>` tag. Inlining (rather than serving
//! `/admin/css/style.css` from a separate route) keeps the asset
//! pipeline trivial -- no extra route, no cache headers, no
//! authenticated-vs-unauthenticated path split for static files,
//! no separate disk directory to sync to a production host.
//!
//! The same inlining treatment applies to the uPlot charting
//! library (vendored at `assets/uplot.js` + `assets/uplot.css`)
//! used by the traffic view.
//!
//! Palette and typography are defined in the CSS file. Headers
//! are deliberately text-only and minimal -- no logo, no brand
//! mark, no placeholder artwork.

use crate::srv::admin::AdminPrincipal;

use oxedyne_fe2o3_core::prelude::*;

/// Stylesheet inlined into every dashboard response. Read at
/// compile time from `assets/style.css` next to this file.
pub const STYLE_CSS: &str = include_str!("assets/style.css");

/// Minified uPlot JavaScript (https://github.com/leeoniya/uPlot,
/// MIT licensed, v1.6.32 vendored). Inlined into pages that draw
/// time-series charts. About 51 KB of source, perhaps half that
/// over the wire when the response is compressed; trivial
/// compared to a framework runtime.
pub const UPLOT_JS: &str = include_str!("assets/uplot.js");

/// Companion CSS shipped with uPlot. Covers axis styling,
/// crosshair rendering and legend layout.
pub const UPLOT_CSS: &str = include_str!("assets/uplot.css");

/// fe2o3 logo (SVG, ~3 KB), copied verbatim from the Hematite
/// asset tree. Text-right layout. Inlined into the sidebar
/// banner of every authenticated dashboard page and into the
/// login card. Dark mode recolours the wordmark via an SVG-
/// targeting CSS override in `style.css`.
pub const FE2O3_LOGO_SVG: &str = include_str!("assets/fe2o3_logo.svg");

/// Oxedyne umbrella logo (SVG, ~3 KB). Text-below-mark layout.
/// Inlined into the page header brand area as the top-left
/// chrome on every authenticated dashboard page.
pub const OXEDYNE_LOGO_SVG: &str = include_str!("assets/oxedyne_logo.svg");

/// Ozone database logo (SVG, ~3 KB). Text-right layout. Used
/// inline beside the `Database` page heading so the browser view
/// is visually identified with the underlying store.
pub const OZONE_LOGO_SVG: &str = include_str!("assets/ozone_logo.svg");

/// Oxanium variable font (TTF, ~43 KB). Served under
/// `/admin/assets/oxanium.ttf` so the browser caches it between
/// pages. CSS `@font-face` in `style.css` points at the same
/// route.
pub const FONT_OXANIUM_TTF: &[u8] = include_bytes!("assets/oxanium.ttf");

/// Inline SVG icons used by the header chrome. Kept tiny --
/// they are 20 x 20 glyphs, one path each, no fills that could
/// be theme-dependent. The sun and moon icons share outer
/// geometry so the theme toggle button animates cleanly.
pub const ICON_SUN_SVG: &str = r#"<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>"#;

/// Moon icon, used when dark mode is active so the toggle
/// clearly says "switch to light mode".
pub const ICON_MOON_SVG: &str = r#"<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>"#;

/// Inline JavaScript that reads the persisted theme preference
/// from `localStorage` on page load and applies the `dark` class
/// to `<html>`. Hooked to the theme toggle button by id. Runs
/// synchronously in `<head>` so the dark class is applied
/// before first paint -- no flash of light theme when the user
/// has dark saved.
pub const THEME_JS: &str = r#"
(function() {
    var KEY = 'steelAdminTheme';
    function apply(theme) {
        var root = document.documentElement;
        if (theme === 'dark') root.classList.add('dark');
        else root.classList.remove('dark');
        var btn = document.getElementById('theme-toggle');
        if (btn) btn.setAttribute('data-theme', theme);
    }
    try { apply(localStorage.getItem(KEY) || 'light'); } catch (e) {}
    window.addEventListener('DOMContentLoaded', function() {
        var btn = document.getElementById('theme-toggle');
        if (!btn) return;
        btn.addEventListener('click', function() {
            var cur = document.documentElement.classList.contains('dark') ? 'dark' : 'light';
            var next = cur === 'dark' ? 'light' : 'dark';
            try { localStorage.setItem(KEY, next); } catch (e) {}
            apply(next);
        });
    });
})();
"#;

/// Ready-made `head_extra` fragment that pulls the uPlot
/// library and its stylesheet into a page. Pass to
/// [`render_layout`] when the body renders a chart. Inlined as
/// `<style>` and `<script>` tags so no extra asset route is
/// required.
pub fn upload_head_html() -> String {
    fmt!(
        "<style>{css}</style>\n\
        <script>{js}</script>\n",
        css = UPLOT_CSS,
        js  = UPLOT_JS,
    )
}

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
    NavEntry { label: "Overview",   href: "/admin",             group: Some("Dashboard") },
    NavEntry { label: "Database",   href: "/admin/database",    group: None },
    NavEntry { label: "Traffic",    href: "/admin/traffic",     group: None },
    NavEntry { label: "Security",   href: "/admin/security",    group: None },
    NavEntry { label: "Admins",     href: "/admin/admins",      group: Some("Management") },
    NavEntry { label: "Sign out",   href: "/admin/logout",      group: Some("Session") },
];

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LAYOUT                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Wrap a body fragment in the standard authenticated dashboard
/// layout: header bar with the current page label and signed-in
/// user, sidebar with nav entries, main content panel.
///
/// `current_path` is matched against each nav entry's `href` to
/// highlight the active link. `body_html` is the inner HTML for
/// the main content panel; callers are responsible for escaping
/// untrusted content within it. `head_extra` lets a specific page
/// inject extra `<script>` or `<link>` tags, used by the traffic
/// view to load the charting library.
pub fn render_layout(
    title:        &str,
    current_path: &str,
    principal:    &AdminPrincipal,
    body_html:    &str,
    head_extra:   &str,
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
        <title>{title} :: Steel admin</title>\n\
        <style>{css}</style>\n\
        <script>{theme_js}</script>\n\
        {head_extra}\
        </head>\n\
        <body>\n\
        <header class=\"steel-header\">\n\
            <div class=\"brand\">\
                <a href=\"/admin\" class=\"brand-logo\" aria-label=\"Oxedyne\">{oxedyne}</a>\
                <span class=\"brand-section\">{section}</span>\
            </div>\n\
            <div class=\"session\">\
                <button id=\"theme-toggle\" class=\"theme-toggle\" type=\"button\" aria-label=\"Toggle theme\">\
                    <span class=\"theme-icon-sun\">{sun}</span>\
                    <span class=\"theme-icon-moon\">{moon}</span>\
                </button>\
                <span class=\"session-user\">{user}</span>\
                <a class=\"session-signout\" href=\"/admin/logout\">Sign out</a>\
            </div>\n\
        </header>\n\
        <div class=\"page\">\n\
        <nav class=\"sidebar\">\n\
        <div class=\"sidebar-brand\">{fe2o3}</div>\n\
        {nav_entries}\
        </nav>\n\
        <main class=\"content\">\n\
        {body}\
        </main>\n\
        </div>\n\
        </body>\n\
        </html>\n",
        title       = html_escape(title),
        css         = STYLE_CSS,
        theme_js    = THEME_JS,
        head_extra  = head_extra,
        oxedyne     = OXEDYNE_LOGO_SVG,
        fe2o3       = FE2O3_LOGO_SVG,
        section     = html_escape(title),
        sun         = ICON_SUN_SVG,
        moon        = ICON_MOON_SVG,
        user        = html_escape(&principal.name),
        nav_entries = nav_html,
        body        = body_html,
    )
}

/// Standalone layout for unauthenticated pages (login form). No
/// sidebar, no header bar, just a centred card with the fe2o3
/// logo above the form.
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
        <title>{title} :: Steel admin</title>\n\
        <style>{css}</style>\n\
        <script>{theme_js}</script>\n\
        </head>\n\
        <body class=\"login\">\n\
        <div class=\"login-card\">\n\
        <div class=\"login-logo\">{logo}</div>\n\
        <div class=\"login-sub\">Admin dashboard</div>\n\
        {body}\
        </div>\n\
        </body>\n\
        </html>\n",
        title    = html_escape(title),
        css      = STYLE_CSS,
        theme_js = THEME_JS,
        logo     = FE2O3_LOGO_SVG,
        body     = body_html,
    )
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FRAGMENTS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Render the sidebar nav entry list from [`NAV`]. Called from
/// [`render_layout`] which supplies the surrounding `<nav>`
/// wrapper and the sidebar brand block above the entries.
/// Entries whose `href` matches `current_path` (or for which
/// the current path is a sub-path) are highlighted via the
/// `current` CSS class.
pub fn render_nav(current_path: &str) -> String {
    let mut out = String::new();
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
    out
}

/// Decide whether a given nav entry should be highlighted as the
/// current page. The home entry (`/admin`) only matches an exact
/// `/admin`; every other entry matches its prefix so sub-pages
/// like `/admin/database?prefix=user:` still highlight `Database`.
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
