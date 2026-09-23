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
//! The handler owns a small
//! [`NonceTracker`](oxedyne_fe2o3_net::guard::nonce::NonceTracker) that rejects duplicate
//! `(signer_id, nonce)` pairs within the freshness window, keyed by
//! the signer. The tracker evicts expired entries lazily on each
//! insert, so no background thread is required. The window is
//! [`SIGNED_LOGIN_FRESHNESS_SECS`] (120 s by default); a command
//! whose timestamp is outside this window is rejected up front
//! by [`SignedCommand::verify_fresh`].
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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

use std::time::{
    Duration,
    SystemTime,
    UNIX_EPOCH,
};


pub const SIGNED_LOGIN_FRESHNESS_SECS: u64 = 120;
pub const CMD_ADMIN_LOGIN:             &str = "admin_login";

// The signed-login session does not auto-renew: the caller presents a new
// SignedCommand once it expires.
pub const SIGNED_LOGIN_SESSION_SECS:   u64 = 3600;


/// Builds the challenge response, a JDAT map carrying:
///
/// - `server_timestamp`: the server's current unix seconds, for clients that
///   want to align their SignedCommand timestamp with the server's clock.
/// - `freshness_secs`: the size of the freshness window.
/// - `accept_cmd`: the string the inbound command must carry as its `cmd`
///   field (`"admin_login"`).
///
/// The endpoint does *not* issue a nonce -- nonces are client-generated and
/// carried in the SignedCommand itself, so replay protection happens at verify
/// time.
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


#[derive(Debug)]
pub enum SignedLoginOutcome {
    Ok(AdminPrincipal),
    MalformedBody { reason: String },   // body did not parse as a JDAT SignedCommand
    WrongCmd { got: String },           // `cmd` is not `admin_login`
    UnknownSigner,                      // signer not in this vhost's `admin_keys`
    BadSignature { reason: String },    // bad signature, or outside the freshness window
    ReplayedNonce,                      // nonce already seen inside the replay window
    NoDashboardScope { name: String },  // signature valid, no dashboard scope configured
}


/// Verifies a signed-admin-login envelope against the configured `admin_keys`.
/// Stateless apart from recording the nonce.
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


/// The envelope's `signer_id` is expected to be the public key bytes; the first
/// matching entry wins.
fn match_admin_key<'a>(
    admin_keys: &'a [AdminKey],
    signer_id:  &[u8],
)
    -> Option<&'a AdminKey>
{
    admin_keys.iter().find(|k| k.public_key.as_slice() == signer_id)
}


/// Writes the outcome to the admin audit log with a short reason tag, in the
/// passphrase flow's line format.
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
    use crate::srv::admin::{
        guard,
        host_sampler::HostSampler,
        traffic::TrafficRecorder,
    };

    use oxedyne_fe2o3_crypto::{
        keystore::Wallet,
        sign::SignatureScheme,
    };
    use oxedyne_fe2o3_iop_crypto::keys::KeyManager;

    use std::{
        path::PathBuf,
        sync::{
            Arc,
            RwLock,
        },
    };

    /// The replay guard is wired into the signed login, wherever the tracker
    /// lives: one signed envelope logs in once, and its second showing inside
    /// the window is refused.
    #[test]
    fn test_a_replayed_signed_login_is_refused_00() -> Outcome<()> {
        let key = SignatureScheme::new_ed25519();
        let public = match res!(key.get_public_key()) {
            Some(pk) => pk.to_vec(),
            None => return Err(err!("A new Ed25519 key has no public half."; Test, Missing)),
        };
        let state = res!(AdminState::new(
            Arc::new(RwLock::new(Wallet::default())),
            PathBuf::from("./wallet.jdat"),
            Some([0u8; 32].to_vec()),
            0,      // no databases
            None,   // no alerter
            TrafficRecorder::new_shared(0),
            HostSampler::new_shared(),
            res!(guard::new_shared()),
            res!(guard::new_shared()),
            vec![AdminKey {
                name:       "ops".to_string(),
                public_key: public.clone(),
                scheme:     "Ed25519".to_string(),
                scopes:     vec![SCOPE_WILDCARD.to_string()],
            }],
            None,
        ));
        let env = res!(SignedCommand::sign(public, CMD_ADMIN_LOGIN, Dat::Empty, &key));
        let body = res!(res!(env.to_dat()).as_bytes());
        match verify_signed_login(&state, &body) {
            SignedLoginOutcome::Ok(principal) => req!(principal.name, "ops".to_string()),
            other => return Err(err!("A fresh signed login earned {:?}.", other; Test)),
        }
        match verify_signed_login(&state, &body) {
            SignedLoginOutcome::ReplayedNonce => (),
            other => return Err(err!("The same signed login shown twice earned {:?}.", other; Test)),
        }
        Ok(())
    }
}
