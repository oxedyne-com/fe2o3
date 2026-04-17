//! Signed session cookies for the admin dashboard.
//!
//! Sessions are stateless: the principal's name, scopes and expiry
//! are encoded into a length-prefixed binary record, encrypted with
//! AES-256-GCM under the dashboard session key, and handed to the
//! browser as a cookie. No session table lives on disk or in memory,
//! so a restart rotates the session key and invalidates every
//! outstanding session at once.
//!
//! # Wire format
//!
//! The cookie string is:
//!
//! ```text
//! v1.<base2x(ciphertext)>
//! ```
//!
//! where the plaintext record is:
//!
//! ```text
//! u16 name_len BE
//! [name_len] bytes   -- admin name UTF-8
//! u8  num_scopes
//! for each scope:
//!     u16 len BE
//!     [len] bytes    -- scope string UTF-8
//! u64 exp BE         -- unix seconds at which the session expires
//! ```
//!
//! The ciphertext is the output of `EncryptionScheme::encrypt`,
//! which for AES-256-GCM is `ciphertext || tag || nonce`. Decoding
//! routes the same bytes back through `decrypt`, which verifies the
//! tag, strips the nonce, and returns the plaintext.

use crate::srv::admin::{
    AdminPrincipal,
    state::AdminState,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_text::base2x;

use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

/// Name of the cookie that carries a signed dashboard session.
pub const SESSION_COOKIE_NAME: &str = "steel_admin_sess";

/// Default session lifetime. Sliding: the handler refreshes the
/// expiry on every authenticated request so an idle session still
/// eventually expires.
pub const DEFAULT_SESSION_TTL_SECS: u64 = 30 * 60;

/// Wire-format version prefix. Bumped when the plaintext record
/// layout changes incompatibly.
pub const SESSION_FORMAT_VERSION: &str = "v1";

/// Maximum number of scopes a single session may carry. Bounds the
/// worst-case cookie size and gives the decoder an obvious reject
/// point before it allocates.
pub const MAX_SESSION_SCOPES: u8 = 32;

/// Maximum length of a single scope string in the cookie, in bytes.
pub const MAX_SCOPE_LEN: usize = 64;

/// Maximum length of the admin name in the cookie, in bytes.
pub const MAX_NAME_LEN: usize = 64;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENCODE                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Encode a principal into the signed cookie string.
///
/// The caller is responsible for setting `expires_at` on the
/// principal; [`session::refresh_principal`] is a convenience that
/// produces an expiry `DEFAULT_SESSION_TTL_SECS` ahead of "now".
pub fn encode_session(
    state:      &AdminState,
    principal:  &AdminPrincipal,
)
    -> Outcome<String>
{
    let plain = res!(encode_record(principal));
    let cipher = res!(state.session_enc.encrypt(&plain));
    let blob = base2x::HEMATITE64.to_string(&cipher);
    Ok(fmt!("{}.{}", SESSION_FORMAT_VERSION, blob))
}

/// Serialise a principal to the length-prefixed plaintext record
/// described in the module-level documentation.
fn encode_record(p: &AdminPrincipal) -> Outcome<Vec<u8>> {
    let name_bytes = p.name.as_bytes();
    if name_bytes.len() > MAX_NAME_LEN {
        return Err(err!(
            "Admin name is {} bytes; max is {} for session encoding.",
            name_bytes.len(), MAX_NAME_LEN;
            Input, TooBig));
    }
    if p.scopes.len() > MAX_SESSION_SCOPES as usize {
        return Err(err!(
            "Principal has {} scopes; session format supports up to {}.",
            p.scopes.len(), MAX_SESSION_SCOPES;
            Input, TooBig));
    }
    let mut out = Vec::with_capacity(
        2 + name_bytes.len() + 1 + p.scopes.len() * 10 + 8);
    out.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(name_bytes);
    out.push(p.scopes.len() as u8);
    for s in &p.scopes {
        let sb = s.as_bytes();
        if sb.len() > MAX_SCOPE_LEN {
            return Err(err!(
                "Scope '{}' is {} bytes; max is {} for session encoding.",
                s, sb.len(), MAX_SCOPE_LEN;
                Input, TooBig));
        }
        out.extend_from_slice(&(sb.len() as u16).to_be_bytes());
        out.extend_from_slice(sb);
    }
    out.extend_from_slice(&p.expires_at.to_be_bytes());
    Ok(out)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DECODE                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Decode and verify a session cookie, returning the embedded
/// principal if the ciphertext authenticates and the expiry has
/// not passed.
///
/// Returns a tagged error rather than `Option<_>` so the handler
/// can log *why* a session was rejected (tampering, expiry, bad
/// format) without leaking the distinction to the client.
pub fn decode_session(
    state:  &AdminState,
    cookie: &str,
)
    -> Outcome<AdminPrincipal>
{
    let (ver, blob) = match cookie.split_once('.') {
        Some(p) => p,
        None => return Err(err!(
            "Session cookie does not contain a version prefix.";
            Input, Invalid)),
    };
    if ver != SESSION_FORMAT_VERSION {
        return Err(err!(
            "Session cookie version '{}' is not recognised (expected '{}').",
            ver, SESSION_FORMAT_VERSION;
            Input, Invalid, Mismatch));
    }
    let cipher = res!(base2x::HEMATITE64.from_str(blob));
    let plain = res!(state.session_enc.decrypt(&cipher));
    let principal = res!(decode_record(&plain));

    let now = now_secs();
    if principal.expires_at <= now {
        return Err(err!(
            "Session expired at unix {} (now {}).",
            principal.expires_at, now;
            Input, Invalid, Security));
    }
    Ok(principal)
}

/// Parse the length-prefixed plaintext record into an
/// [`AdminPrincipal`].
fn decode_record(bytes: &[u8]) -> Outcome<AdminPrincipal> {
    let mut p = 0usize;
    let name_len = res!(read_u16(bytes, &mut p)) as usize;
    if name_len > MAX_NAME_LEN {
        return Err(err!(
            "Session record claims a {}-byte name; max is {}.",
            name_len, MAX_NAME_LEN;
            Input, TooBig));
    }
    let name_bytes = res!(read_slice(bytes, &mut p, name_len));
    let name = res!(std::str::from_utf8(name_bytes),
        Decode, String).to_string();

    let num_scopes = res!(read_u8(bytes, &mut p));
    if num_scopes > MAX_SESSION_SCOPES {
        return Err(err!(
            "Session record claims {} scopes; max is {}.",
            num_scopes, MAX_SESSION_SCOPES;
            Input, TooBig));
    }
    let mut scopes = Vec::with_capacity(num_scopes as usize);
    for _ in 0..num_scopes {
        let slen = res!(read_u16(bytes, &mut p)) as usize;
        if slen > MAX_SCOPE_LEN {
            return Err(err!(
                "Session record claims a {}-byte scope; max is {}.",
                slen, MAX_SCOPE_LEN;
                Input, TooBig));
        }
        let sb = res!(read_slice(bytes, &mut p, slen));
        let s = res!(std::str::from_utf8(sb),
            Decode, String).to_string();
        scopes.push(s);
    }

    let exp_bytes = res!(read_slice(bytes, &mut p, 8));
    let mut exp_arr = [0u8; 8];
    exp_arr.copy_from_slice(exp_bytes);
    let expires_at = u64::from_be_bytes(exp_arr);

    if p != bytes.len() {
        return Err(err!(
            "Session record has {} trailing bytes after the expiry.",
            bytes.len() - p;
            Input, Invalid));
    }
    Ok(AdminPrincipal {
        name,
        scopes,
        expires_at,
    })
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PRINCIPAL REFRESH                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// Produce a fresh copy of `principal` with `expires_at` advanced to
/// `now + DEFAULT_SESSION_TTL_SECS`. Used by the handler on every
/// authenticated request to implement sliding-expiry sessions.
pub fn refresh_principal(principal: &AdminPrincipal) -> AdminPrincipal {
    AdminPrincipal {
        name:       principal.name.clone(),
        scopes:     principal.scopes.clone(),
        expires_at: now_secs().saturating_add(DEFAULT_SESSION_TTL_SECS),
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Current unix time in seconds, clamped to zero on clock error.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_u8(bytes: &[u8], p: &mut usize) -> Outcome<u8> {
    if *p >= bytes.len() {
        return Err(err!(
            "Session record truncated at byte {} (reading u8).", *p;
            Input, Invalid, TooSmall));
    }
    let v = bytes[*p];
    *p += 1;
    Ok(v)
}

fn read_u16(bytes: &[u8], p: &mut usize) -> Outcome<u16> {
    if *p + 2 > bytes.len() {
        return Err(err!(
            "Session record truncated at byte {} (reading u16).", *p;
            Input, Invalid, TooSmall));
    }
    let v = u16::from_be_bytes([bytes[*p], bytes[*p + 1]]);
    *p += 2;
    Ok(v)
}

fn read_slice<'a>(
    bytes:  &'a [u8],
    p:      &mut usize,
    n:      usize,
)
    -> Outcome<&'a [u8]>
{
    if *p + n > bytes.len() {
        return Err(err!(
            "Session record truncated at byte {} (reading {} bytes).",
            *p, n;
            Input, Invalid, TooSmall));
    }
    let s = &bytes[*p..*p + n];
    *p += n;
    Ok(s)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srv::admin::state::derive_session_key;
    use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
    use oxedyne_fe2o3_crypto::keystore::Wallet;
    use std::sync::{Arc, RwLock};

    fn mkstate() -> AdminState {
        use crate::srv::admin::{
            host_sampler::HostSampler,
            traffic::TrafficRecorder,
        };
        use std::path::PathBuf;
        let master = [0u8; 32];
        let key = derive_session_key(&master).expect("derive");
        let enc = EncryptionScheme::new_aes_256_gcm_with_key(&key)
            .expect("aes");
        AdminState {
            wallet:         Arc::new(RwLock::new(Wallet::default())),
            wallet_path:    PathBuf::from("./wallet.jdat"),
            master_key:     master.to_vec(),
            session_enc:    enc,
            traffic:        TrafficRecorder::new_shared(0),
            host_sampler:   HostSampler::new_shared(),
        }
    }

    fn mkprincipal() -> AdminPrincipal {
        AdminPrincipal {
            name:       "alice".to_string(),
            scopes:     vec![
                "dashboard.view".to_string(),
                "dashboard.admin".to_string(),
            ],
            expires_at: now_secs() + DEFAULT_SESSION_TTL_SECS,
        }
    }

    #[test]
    fn round_trip() {
        let state = mkstate();
        let p = mkprincipal();
        let cookie = encode_session(&state, &p).expect("encode");
        assert!(cookie.starts_with("v1."));
        let p2 = decode_session(&state, &cookie).expect("decode");
        assert_eq!(p2.name, p.name);
        assert_eq!(p2.scopes, p.scopes);
        assert_eq!(p2.expires_at, p.expires_at);
    }

    #[test]
    fn rejects_tampered_cookie() {
        let state = mkstate();
        let p = mkprincipal();
        let cookie = encode_session(&state, &p).expect("encode");
        // Flip a byte in the ciphertext. Must still decode as a
        // valid cookie string but fail AES-GCM authentication.
        let mut bytes: Vec<u8> = cookie.into_bytes();
        let tamper_idx = bytes.len() - 5;
        bytes[tamper_idx] ^= 0x01;
        let tampered = String::from_utf8(bytes).expect("utf8");
        assert!(decode_session(&state, &tampered).is_err());
    }

    #[test]
    fn rejects_expired_cookie() {
        let state = mkstate();
        let mut p = mkprincipal();
        p.expires_at = 1;
        let cookie = encode_session(&state, &p).expect("encode");
        assert!(decode_session(&state, &cookie).is_err());
    }

    #[test]
    fn rejects_version_mismatch() {
        let state = mkstate();
        let p = mkprincipal();
        let cookie = encode_session(&state, &p).expect("encode");
        let swapped = cookie.replacen("v1.", "v2.", 1);
        assert!(decode_session(&state, &swapped).is_err());
    }
}
