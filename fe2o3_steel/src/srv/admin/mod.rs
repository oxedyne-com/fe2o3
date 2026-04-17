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

/// Wildcard scope that matches every verb. Mirrors the CLI's
/// wildcard-matching logic in `AdminUser::has_scope`.
pub const SCOPE_WILDCARD: &str = "*";

/// Legacy CLI scope that authorises managing other admin entries.
/// Required, in addition to a dashboard scope, for the dashboard's
/// admin-management UI to become visible.
pub const SCOPE_ADMIN: &str = "admin";

/// Read-only dashboard access. Grants login plus traffic and ozone
/// browsing; no mutations.
pub const SCOPE_DASHBOARD_VIEW: &str = "dashboard.view";

/// Full dashboard access. Grants everything in [`SCOPE_DASHBOARD_VIEW`]
/// plus future v2 mutations (e.g. edit ozone values).
pub const SCOPE_DASHBOARD_ADMIN: &str = "dashboard.admin";

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ADMIN PRINCIPAL                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Identity and authorisation carried with every authenticated
/// dashboard request.
///
/// Built by [`auth`] on successful login and encoded into the signed
/// session cookie by [`session`]. Handlers in [`handler`] extract it
/// from the request and consult its methods to gate individual views
/// and actions.
#[derive(Clone, Debug)]
pub struct AdminPrincipal {
    /// Name of the wallet admin that unlocked the session. Copied
    /// from `AdminUser::name` at login time.
    pub name:       String,
    /// Snapshot of the wallet admin's scope list at login time.
    /// Not refreshed across the life of the session -- rotating an
    /// admin's scopes takes effect on their next login.
    pub scopes:     Vec<String>,
    /// Unix seconds at which the session expires.
    pub expires_at: u64,
}

impl AdminPrincipal {
    /// Returns `true` if this principal is authorised for `verb`.
    /// The wildcard scope `"*"` matches every verb.
    pub fn has_scope(&self, verb: &str) -> bool {
        self.scopes.iter().any(|s| s == SCOPE_WILDCARD || s == verb)
    }

    /// Returns `true` if this principal can log into the dashboard
    /// at all. Either of the two dashboard scopes (or the wildcard)
    /// suffices.
    pub fn can_view_dashboard(&self) -> bool {
        self.has_scope(SCOPE_DASHBOARD_VIEW)
            || self.has_scope(SCOPE_DASHBOARD_ADMIN)
    }

    /// Returns `true` if this principal can perform dashboard
    /// mutations. In v1 this gates nothing -- the v1 dashboard is
    /// read-only -- but handlers consult it so v2 write actions fall
    /// into place without a second auth pass.
    pub fn can_admin_dashboard(&self) -> bool {
        self.has_scope(SCOPE_DASHBOARD_ADMIN)
    }

    /// Returns `true` if this principal can manage other wallet admin
    /// entries. Requires the legacy CLI `admin` scope in addition to
    /// a dashboard scope, so that granting dashboard login does not
    /// automatically grant the power to enrol more admins.
    pub fn can_manage_admins(&self) -> bool {
        self.has_scope(SCOPE_ADMIN) && self.can_view_dashboard()
    }
}
