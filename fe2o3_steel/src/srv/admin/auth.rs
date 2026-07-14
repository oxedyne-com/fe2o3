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

use crate::srv::{
    admin::{
        AdminPrincipal,
        session::{
            DEFAULT_SESSION_TTL_SECS,
            now_secs,
        },
        state::AdminState,
    },
    alert::AlertEvent,
};

use oxedyne_fe2o3_core::prelude::*;

use std::net::SocketAddr;

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
    peer:       SocketAddr,
)
    -> Outcome<LoginOutcome>
{
    // `unseal` performs the wallet unlock, and installs the recovered
    // master key if the process is still sealed. A dashboard login is
    // therefore the same act as an unseal: the passphrase that proves
    // who you are is the passphrase that unwraps the key. That is what
    // lets an admin bring a cold-started Steel's databases up from a
    // browser, with no terminal and no database behind the login form.
    //
    // A wrong passphrase fails the unwrap, so it can neither log in
    // nor unseal. There is no separate credential to get out of step.
    //
    // Note the unseal is not gated on dashboard scope, while the
    // session below is. This is not an escalation: every admin in the
    // wallet holds their own wrap of the master key, so any of them
    // can already recover it by definition. Scope governs what the
    // dashboard will *show* them, not whether they are trusted with
    // the key they already have.
    let unsealed = match state.unseal(passphrase) {
        Ok(u) => u,
        Err(_) => {
            // A wrong passphrase. Count it: this form unwraps the wallet
            // master key, so it is worth guessing at, and a burst of
            // guesses is something the operator should hear about.
            if let Some(alerter) = state.alerter() {
                alerter.note_failed_unseal(peer);
            }
            return Ok(LoginOutcome::BadCredentials);
        }
    };

    // Alert only when this login is what actually lifted the seal. Every
    // subsequent sign-in is a routine login, and alerting on those would
    // bury the one message that mattered.
    if unsealed.lifted {
        if let Some(alerter) = state.alerter() {
            alerter.raise(AlertEvent::Unsealed {
                admin: unsealed.name.clone(),
                peer,
            });
        }
    }

    let name = unsealed.name;
    let scopes = unsealed.scopes;

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
