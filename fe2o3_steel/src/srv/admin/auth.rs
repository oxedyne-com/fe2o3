//! Dashboard login flow.
//!
//! Verifies a passphrase against the loaded wallet and produces an
//! [`AdminPrincipal`] on success. The wallet is already resident in
//! memory -- it was loaded at Steel start-up by the TUI unlock
//! prompt -- so login reuses the same `Wallet::unlock` path the CLI
//! uses.
//!
//! Unlike the CLI unlock, which is a one-time event at start-up,
//! dashboard login also gates on *scope*: the passphrase must not
//! only unwrap an admin entry, the matched entry must also hold
//! one of the dashboard scopes. An admin whose scope list gates
//! CLI-only verbs is therefore still recognised by the wallet but
//! refused by the dashboard.

use crate::srv::admin::{
    AdminPrincipal,
    session::{
        DEFAULT_SESSION_TTL_SECS,
        now_secs,
    },
    state::AdminState,
};

use oxedyne_fe2o3_core::prelude::*;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOGIN OUTCOME                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Result of a dashboard login attempt.
///
/// Distinct from a plain `Outcome` so the handler can distinguish
/// "wrong password" (re-prompt with a generic error) from "correct
/// password but no dashboard scope" (explicit refusal with operator
/// guidance).
#[derive(Debug)]
pub enum LoginOutcome {
    /// Login succeeded; principal is ready for session encoding.
    Ok(AdminPrincipal),
    /// No admin entry unwrapped with the supplied passphrase. The
    /// message is intentionally generic so the response to the
    /// client does not leak whether any admin exists.
    BadCredentials,
    /// An admin entry unwrapped, but the matched entry holds
    /// neither `dashboard.view` nor `dashboard.admin`. The wallet
    /// accepts the passphrase; the dashboard refuses the session.
    /// The name is returned for audit-log purposes; handlers must
    /// not echo it to the client.
    NoDashboardScope { name: String },
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOGIN                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Verify a passphrase against the wallet and, on success, build an
/// [`AdminPrincipal`] with a fresh sliding-TTL expiry.
///
/// Returns a [`LoginOutcome`] describing the high-level result.
/// Structural errors (poisoned lock, wallet corruption) propagate
/// as `Err(_)`; user-visible refusals are conveyed through the
/// enum variants so the caller can respond and audit-log them
/// differently.
pub fn verify_passphrase(
    state:      &AdminState,
    passphrase: &[u8],
)
    -> Outcome<LoginOutcome>
{
    let wallet = lock_read!(state.wallet);
    let unlocked = match wallet.unlock(passphrase) {
        Ok(u) => u,
        Err(_) => return Ok(LoginOutcome::BadCredentials),
    };
    drop(wallet);

    let scopes = unlocked.admin_scopes.clone();
    let name = unlocked.admin_name.clone();

    // Reuse the principal-side scope check so the dashboard
    // access rule lives in exactly one place.
    let probe = AdminPrincipal {
        name:       name.clone(),
        scopes:     scopes.clone(),
        expires_at: 0,
    };
    if !probe.can_view_dashboard() {
        return Ok(LoginOutcome::NoDashboardScope { name });
    }

    Ok(LoginOutcome::Ok(AdminPrincipal {
        name,
        scopes,
        expires_at: now_secs().saturating_add(DEFAULT_SESSION_TTL_SECS),
    }))
}
