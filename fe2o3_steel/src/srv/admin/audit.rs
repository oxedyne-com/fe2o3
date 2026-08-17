//! Admin audit log.
//!
//! Append-only line-delimited file recording every privileged
//! action taken against the wallet, the dashboard, or anything
//! else the operator needs an after-the-fact paper trail for.
//!
//! Originally introduced for the CLI `admin --add` / `admin --remove`
//! verbs and the wallet-v2 migration; lifted out of `app/repl.rs`
//! into the admin module so the dashboard handler in
//! `srv/admin/handler.rs` can write to the same file using the same
//! line format.
//!
//! # Format
//!
//! One entry per line:
//!
//! ```text
//! <unix_seconds> <admin> <verb> <result> <detail>
//! ```
//!
//! - `unix_seconds` -- seconds since epoch when the event was recorded.
//! - `admin` -- name of the admin who triggered the action, or one of
//!   the sentinels `(unknown)` / `(anon)` when no identity was
//!   captured.
//! - `verb` -- dotted action name such as `admin.add`, `dashboard.login`.
//! - `result` -- `ok` or `err`.
//! - `detail` -- free-form key=value pairs, space-separated, no
//!   newlines. Quote values that contain spaces.
//!
//! The format is deliberately greppable rather than structured;
//! the audit log is read by humans in incident response, not by
//! parsers.
//!
//! # Failure handling
//!
//! Failures to open or write the log file are logged at `warn!` and
//! never propagated. The principle: the action being audited must
//! not fail because the audit log is unavailable. A dashboard login
//! that succeeds against a healthy wallet should still let the
//! operator in even if the disk holding the audit log is full.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

// Hosted here rather than in `app::constant` so the dashboard handler in the
// `srv` layer does not have to import from `app`.
pub const ADMIN_AUDIT_LOG_NAME: &str = "admin-audit.log";

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VERBS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

// CLI-side verbs (`admin.add`, `admin.remove`, `admin.passwd`, `admin.list`,
// `wallet.migrate`) stay as inline strings in `app/repl.rs` so the original call
// sites remain stable; dashboard verbs are declared here so handler call sites
// cannot drift on spelling.
pub const VERB_DASHBOARD_LOGIN:             &str = "dashboard.login";
pub const VERB_DASHBOARD_LOGOUT:            &str = "dashboard.logout";
pub const VERB_DASHBOARD_ADMIN_ADD:         &str = "dashboard.admin.add";
pub const VERB_DASHBOARD_ADMIN_REMOVE:      &str = "dashboard.admin.remove";
pub const VERB_DASHBOARD_GUARD_WHITELIST:   &str = "dashboard.guard.whitelist";
pub const VERB_DASHBOARD_GUARD_BLACKLIST:   &str = "dashboard.guard.blacklist";
pub const VERB_DASHBOARD_GUARD_UNBLOCK:     &str = "dashboard.guard.unblock";

pub const ADMIN_ANON: &str = "(anon)"; // unauthenticated visitor, e.g. a failed login

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ APPEND                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Failures are logged at `warn!` and swallowed. The audit log is never allowed
/// to break the action it is recording.
pub fn append(admin: &str, verb: &str, result: &str, detail: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = fmt!(
        "{} {} {} {} {}\n",
        secs, admin, verb, result, detail,
    );
    let path: PathBuf = Path::new("./").join(ADMIN_AUDIT_LOG_NAME);
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                warn!("Failed to write audit log line: {}", e);
            }
        },
        Err(e) => warn!("Failed to open audit log {:?}: {}", path, e),
    }
}
