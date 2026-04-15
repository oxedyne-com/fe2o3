//! HTTP dispatcher for the admin dashboard.
//!
//! Routes `/admin/*` paths to login, logout, traffic view, ozone
//! browser and admin-management handlers. Extracts the session
//! cookie via [`session`](super::session) on every request and
//! rejects any authenticated path whose principal lacks the needed
//! scope.
//!
//! The same dispatcher is reused by the loopback plaintext listener
//! added in task #8 -- localhost gets the same routes, the same
//! login gate, and the same session cookie format.
//!
//! Filled in by task #7.
