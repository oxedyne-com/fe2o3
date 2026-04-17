//! Signed-admin-login handler for the dashboard.
//!
//! Lets an operator authenticate to the admin dashboard by
//! presenting a [`SignedCommand`] rather than a wallet passphrase.
//! The inbound command names a `signer_id`, which the handler
//! looks up in the vhost's [`AdminKey`] list; a successful signature
//! verification against the matching public key, within the
//! freshness window, issues the same session cookie the passphrase
//! flow issues.
//!
//! The classical passphrase login stays available at `/admin/login`;
//! this is a parallel path. A Steel deployment that configures no
//! `admin_keys` never sees the new endpoints active.
//!
//! # Replay protection
//!
//! The handler owns a small [`NonceTracker`] that rejects duplicate
//! `(signer_id, nonce)` pairs within the freshness window. The
//! tracker evicts expired entries lazily on each insert, so no
//! background thread is required. The window is
//! [`SIGNED_LOGIN_FRESHNESS_SECS`] (120 s by default); a command
//! whose timestamp is outside this window is rejected up front
//! by [`SignedCommand::verify_fresh`].

use crate::srv::{
    admin::{
        AdminPrincipal,
        SCOPE_WILDCARD,
        SCOPE_DASHBOARD_VIEW,
        SCOPE_DASHBOARD_ADMIN,
        audit::{
            self,
            ADMIN_ANON,
            VERB_DASHBOARD_LOGIN,
        },
        state::AdminState,
    },
    cfg::AdminKey,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::command::SignedCommand;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::HashMap,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};


/// Freshness window for a signed-admin-login envelope. A command
/// whose timestamp sits more than this many seconds away from the
/// server's clock is rejected as stale.
pub const SIGNED_LOGIN_FRESHNESS_SECS: u64 = 120;

/// Command name the handler requires in the inbound envelope.
/// Other commands delivered to this endpoint are rejected.
pub const CMD_ADMIN_LOGIN: &str = "admin_login";


/// Session duration granted to a signed-login principal.
///
/// Matches the passphrase flow's default (one hour). The caller
/// refreshes by issuing a new SignedCommand when this expires; the
/// handler does not auto-renew.
pub const SIGNED_LOGIN_SESSION_SECS: u64 = 3600;


/// Small in-memory replay-window tracker.
///
/// Records the timestamp of every inbound `(signer_id, nonce)` pair
/// and rejects a re-presentation of the same pair. Lazily evicts
/// entries older than `window` on each insert. Suitable for the
/// single-process admin-login rate (one login per operator per
/// restart); not suitable as a general-purpose rate limiter.
#[derive(Debug)]
pub struct NonceTracker {
    seen:   HashMap<(Vec<u8>, [u8; 32]), u64>,
    window: Duration,
}

impl NonceTracker {
    /// Constructs a tracker with the given eviction window.
    pub fn new(window: Duration) -> Self {
        Self {
            seen:   HashMap::new(),
            window,
        }
    }

    /// Records `(signer_id, nonce)` as seen at `now`. Returns
    /// `Ok(())` if the pair was not previously seen inside the
    /// current window, or an error otherwise.
    pub fn record(
        &mut self,
        signer_id:  &[u8],
        nonce:      &[u8; 32],
        now:        u64,
    )
        -> Outcome<()>
    {
        self.evict_expired(now);
        let key = (signer_id.to_vec(), *nonce);
        if self.seen.contains_key(&key) {
            return Err(err!(
                "Signed-login nonce already seen for this signer inside \
                the {} s replay window.", self.window.as_secs();
                Invalid, Security, Duplicate));
        }
        self.seen.insert(key, now);
        Ok(())
    }

    /// Returns the current number of tracked entries. Diagnostic only.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    fn evict_expired(&mut self, now: u64) {
        let window_secs = self.window.as_secs();
        self.seen.retain(|_, ts| now.saturating_sub(*ts) <= window_secs);
    }
}


/// Builds a [`SignedCommand`]-flow challenge. The response body is
/// a JDAT map carrying:
///
/// - `server_timestamp`: the server's current unix seconds, for
///   clients that want to align their SignedCommand timestamp with
///   the server's clock.
/// - `freshness_secs`: the size of the freshness window.
/// - `accept_cmd`: the string the inbound command must carry as its
///   `cmd` field (`"admin_login"`).
///
/// The endpoint does *not* issue a nonce -- nonces are client-
/// generated and carried in the SignedCommand itself. Replay
/// protection happens at verify time.
pub fn handle_challenge(_state: &AdminState) -> HttpMessage {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut m = DaticleMap::new();
    m.insert(dat!("server_timestamp"),  dat!(now));
    m.insert(dat!("freshness_secs"),    dat!(SIGNED_LOGIN_FRESHNESS_SECS));
    m.insert(dat!("accept_cmd"),        dat!(CMD_ADMIN_LOGIN.to_string()));
    let body = Dat::Map(m);
    let bytes = match body.as_bytes() {
        Ok(b) => b,
        Err(e) => {
            error!(e, "signed-login challenge: JDAT encoding failed");
            return HttpMessage::respond_with_text(
                HttpStatus::InternalServerError,
                "Challenge encoding failed.",
            );
        },
    };
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("application/jdat".to_string()),
        )
        .with_body(bytes)
}


/// Outcome of [`verify_signed_login`].
#[derive(Debug)]
pub enum SignedLoginOutcome {
    /// Verified; the caller should issue a session cookie for this
    /// principal.
    Ok(AdminPrincipal),
    /// Request body failed to parse as a JDAT [`SignedCommand`].
    MalformedBody { reason: String },
    /// The envelope's `cmd` is not `"admin_login"`.
    WrongCmd { got: String },
    /// Envelope signer not listed in this vhost's `admin_keys`.
    UnknownSigner,
    /// Signature did not verify, or fell outside the freshness
    /// window.
    BadSignature { reason: String },
    /// Nonce already presented inside the replay window.
    ReplayedNonce,
    /// Signer is known and signature valid, but the configured
    /// scopes do not include a dashboard scope.
    NoDashboardScope { name: String },
}


/// Verifies a signed-admin-login envelope against the configured
/// `admin_keys` and nonce tracker. Stateless apart from the nonce
/// tracker update on success.
pub fn verify_signed_login(
    state:  &AdminState,
    body:   &[u8],
)
    -> SignedLoginOutcome
{
    // Parse the envelope.
    let (dat, _) = match Dat::from_bytes(body) {
        Ok(v) => v,
        Err(e) => return SignedLoginOutcome::MalformedBody {
            reason: fmt!("JDAT decode failed: {}", e),
        },
    };
    let env = match SignedCommand::from_dat(dat) {
        Ok(e) => e,
        Err(e) => return SignedLoginOutcome::MalformedBody {
            reason: fmt!("SignedCommand extraction failed: {}", e),
        },
    };
    if env.cmd != CMD_ADMIN_LOGIN {
        return SignedLoginOutcome::WrongCmd { got: env.cmd };
    }

    // Match signer against the configured admin_keys list.
    let admin_key = match match_admin_key(&state.admin_keys, &env.signer_id) {
        Some(a) => a,
        None    => return SignedLoginOutcome::UnknownSigner,
    };

    // Verify signature + freshness.
    if let Err(e) = env.verify_fresh(
        &admin_key.public_key,
        Duration::from_secs(SIGNED_LOGIN_FRESHNESS_SECS),
    ) {
        return SignedLoginOutcome::BadSignature {
            reason: fmt!("{}", e),
        };
    }

    // Reject replays.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    {
        let mut tracker = match state.nonce_tracker.lock() {
            Ok(t) => t,
            Err(_) => return SignedLoginOutcome::BadSignature {
                reason: "nonce tracker poisoned".to_string(),
            },
        };
        if tracker.record(&env.signer_id, &env.nonce, now).is_err() {
            return SignedLoginOutcome::ReplayedNonce;
        }
    }

    // Scope check: ensure the caller can actually use the dashboard.
    let scopes = admin_key.scopes.clone();
    let has_dashboard_scope = scopes.iter().any(|s|
        s == SCOPE_WILDCARD
        || s == SCOPE_DASHBOARD_VIEW
        || s == SCOPE_DASHBOARD_ADMIN
    );
    if !has_dashboard_scope {
        return SignedLoginOutcome::NoDashboardScope {
            name: admin_key.name.clone(),
        };
    }

    let expires_at = now.saturating_add(SIGNED_LOGIN_SESSION_SECS);
    SignedLoginOutcome::Ok(AdminPrincipal {
        name:       admin_key.name.clone(),
        scopes,
        expires_at,
    })
}


/// Matches an envelope's `signer_id` (expected to be the public key
/// bytes) against the configured `admin_keys` list. Returns the
/// first matching entry.
fn match_admin_key<'a>(
    admin_keys: &'a [AdminKey],
    signer_id:  &[u8],
)
    -> Option<&'a AdminKey>
{
    admin_keys.iter().find(|k| k.public_key.as_slice() == signer_id)
}


/// Logs the signed-login outcome to the admin audit log with a short
/// reason tag, matching the passphrase flow's audit line format.
pub fn audit_signed_login(outcome: &SignedLoginOutcome) {
    match outcome {
        SignedLoginOutcome::Ok(principal) => audit::append(
            &principal.name,
            VERB_DASHBOARD_LOGIN,
            "ok",
            &fmt!("signed; scopes={}", principal.scopes.join(",")),
        ),
        SignedLoginOutcome::MalformedBody { reason } => audit::append(
            ADMIN_ANON, VERB_DASHBOARD_LOGIN, "err",
            &fmt!("signed; reason=malformed_body: {}", reason),
        ),
        SignedLoginOutcome::WrongCmd { got } => audit::append(
            ADMIN_ANON, VERB_DASHBOARD_LOGIN, "err",
            &fmt!("signed; reason=wrong_cmd: got={}", got),
        ),
        SignedLoginOutcome::UnknownSigner => audit::append(
            ADMIN_ANON, VERB_DASHBOARD_LOGIN, "err",
            "signed; reason=unknown_signer",
        ),
        SignedLoginOutcome::BadSignature { reason } => audit::append(
            ADMIN_ANON, VERB_DASHBOARD_LOGIN, "err",
            &fmt!("signed; reason=bad_signature: {}", reason),
        ),
        SignedLoginOutcome::ReplayedNonce => audit::append(
            ADMIN_ANON, VERB_DASHBOARD_LOGIN, "err",
            "signed; reason=replayed_nonce",
        ),
        SignedLoginOutcome::NoDashboardScope { name } => audit::append(
            name, VERB_DASHBOARD_LOGIN, "err",
            "signed; reason=no_dashboard_scope",
        ),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_tracker_accepts_distinct_and_rejects_repeat() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let signer = b"alice".to_vec();
        let n1 = [0x11u8; 32];
        let n2 = [0x22u8; 32];
        res!(t.record(&signer, &n1, 1000));
        res!(t.record(&signer, &n2, 1000));
        assert!(t.record(&signer, &n1, 1000).is_err(),
            "re-presenting the same nonce inside the window must fail");
        Ok(())
    }

    #[test]
    fn nonce_tracker_evicts_after_window() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let signer = b"alice".to_vec();
        let n = [0x33u8; 32];
        res!(t.record(&signer, &n, 1000));
        // Same signer + nonce, but 61 seconds later: eviction kicks
        // in on the insert and the record succeeds.
        res!(t.record(&signer, &n, 1061));
        Ok(())
    }

    #[test]
    fn nonce_tracker_scopes_by_signer() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let a = b"alice".to_vec();
        let b = b"bob".to_vec();
        let n = [0x44u8; 32];
        res!(t.record(&a, &n, 1000));
        // Different signer, same nonce -- allowed.
        res!(t.record(&b, &n, 1000));
        Ok(())
    }
}
