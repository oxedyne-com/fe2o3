//! Admin dashboard for Steel.
//!
//! A self-contained web dashboard embedded inside the Steel server
//! binary. Operators reach it locally on a plaintext loopback listener
//! or remotely under `/admin` on the main vhost. The dashboard reuses
//! the wallet admin identities -- logging in is equivalent to handing
//! a passphrase to `steel admin` at the CLI -- so there is no second
//! user database to administer.
//!
//! # Capabilities
//!
//! - Browse and filter the ozone database associated with each vhost.
//! - Watch live traffic: recent requests, per-path and per-status
//!   counters, rate information.
//! - Manage wallet admin entries (add, remove, list) from the browser,
//!   mirroring the CLI's `admin` verbs.
//!
//! # Scopes
//!
//! Dashboard access is gated by the same scope strings used by the
//! CLI's `admin` verbs. The dashboard recognises:
//!
//! - [`SCOPE_DASHBOARD_VIEW`] -- read-only access; traffic and ozone
//!   browsing only.
//! - [`SCOPE_DASHBOARD_ADMIN`] -- full dashboard access; enables
//!   mutations in a future v2 (edit ozone values).
//! - [`SCOPE_ADMIN`] -- the existing CLI scope; required *in addition*
//!   to one of the dashboard scopes to see the admin-management UI.
//!
//! An admin holding only the wildcard `"*"` scope sees everything.
//!
//! # Submodules
//!
//! - [`auth`] -- login flow; verifies a passphrase against the loaded
//!   wallet and produces an [`AdminPrincipal`].
//! - [`session`] -- signed cookie format, encode/decode, principal
//!   extraction from an incoming request.
//! - [`traffic`] -- in-memory ring buffer of recent requests and the
//!   counters that feed the live dashboard views.
//! - [`ozone_view`] -- read-only ozone browsing, prefix scans, key
//!   detail lookup.
//! - [`assets`] -- embedded HTML, CSS, JavaScript and image assets
//!   served as the dashboard front end.
//! - [`handler`] -- HTTP dispatcher that maps `/admin/*` request paths
//!   to the appropriate view or action.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod assets;
pub mod audit;
pub mod auth;
pub mod guard;
pub mod handler;
pub mod host_sampler;
pub mod local_listener;
pub mod ozone_view;
pub mod persist;
pub mod session;
pub mod signed_login;
pub mod state;
pub mod traffic;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SCOPE CONSTANTS                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

pub const SCOPE_WILDCARD:           &str = "*";
pub const SCOPE_ADMIN:              &str = "admin";
pub const SCOPE_DASHBOARD_VIEW:     &str = "dashboard.view";
pub const SCOPE_DASHBOARD_ADMIN:    &str = "dashboard.admin";

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ADMIN PRINCIPAL                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Identity and authorisation carried with every authenticated
/// dashboard request.
#[derive(Clone, Debug)]
pub struct AdminPrincipal {
    pub name:       String,     // wallet admin that unlocked the session
    // Snapshot taken at login and never refreshed, so rotating an admin's scopes
    // takes effect only on their next login.
    pub scopes:     Vec<String>,
    pub expires_at: u64,        // unix seconds
}

impl AdminPrincipal {
    pub fn has_scope(&self, verb: &str) -> bool {
        self.scopes.iter().any(|s| s == SCOPE_WILDCARD || s == verb)
    }

    pub fn can_view_dashboard(&self) -> bool {
        self.has_scope(SCOPE_DASHBOARD_VIEW)
            || self.has_scope(SCOPE_DASHBOARD_ADMIN)
    }

    /// Gates dashboard mutations, such as the address-guard whitelist and
    /// blacklist actions.
    pub fn can_admin_dashboard(&self) -> bool {
        self.has_scope(SCOPE_DASHBOARD_ADMIN)
    }

    /// Requires the CLI `admin` scope on top of a dashboard scope, so granting
    /// dashboard login does not also grant the power to enrol more admins.
    pub fn can_manage_admins(&self) -> bool {
        self.has_scope(SCOPE_ADMIN) && self.can_view_dashboard()
    }
}
