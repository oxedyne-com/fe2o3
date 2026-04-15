//! Read-only ozone browser.
//!
//! Exposes three operations over every per-vhost ozone database that
//! Steel currently has open:
//!
//! - List vhosts with an open database.
//! - Prefix scan: list keys under a given `<app>:<entity>[:...]`
//!   prefix, paginated.
//! - Key detail: fetch a single key's value and pretty-print it in
//!   both JDAT and JSON form.
//!
//! No mutations in v1. Edit and delete land in v2 behind an explicit
//! `dashboard.admin` check plus a second confirmation.
//!
//! Filled in by task #5.
