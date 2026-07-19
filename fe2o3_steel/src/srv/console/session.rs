//! Signed site-admin session cookies for the console.
//!
//! A site owner signs in at `/manage/login` with the operator's own wallet
//! passphrase -- the same one the `/admin` dashboard verifies -- and is given a
//! *site-admin* session. That session opens the `/manage` console and nothing
//! else.
//!
//! # What the session is, and what it is not
//!
//! It is a small record -- a kind tag, the admin's name, and an expiry --
//! encrypted with AES-256-GCM under the per-process dashboard session key, the
//! very [`EncryptionScheme`](oxedyne_fe2o3_crypto::enc::EncryptionScheme) the
//! operator session uses. Reusing that key buys two things for free: forging a
//! session needs the key, which never leaves the process, and a restart mints a
//! fresh key and so invalidates every outstanding manage session exactly as it
//! does every operator one.
//!
//! It is deliberately a *different cookie*, on a *different path*, in a
//! *different record layout* from the operator session. The operator cookie is
//! `Path=/admin`, so a browser never sends it here; this cookie is `Path=/`, so
//! it reaches `/manage`. The record carries a leading [`KIND_TAG`] the operator
//! record does not, so a blob minted for one use cannot be read as the other
//! even though both are sealed under the same key. The credential opens the
//! console -- the site's content -- and carries no operator scope: it can never
//! stand in for an `/admin` session.

use crate::srv::admin::{
	session::now_secs,
	state::AdminState,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_net::http::fields::{
	HeaderFields,
	HeaderFieldValue,
	HeaderName,
};
use oxedyne_fe2o3_text::base2x;

use std::sync::Arc;

/// Name of the cookie carrying a signed site-admin session.
///
/// Distinct from the operator session cookie so neither can ever be presented
/// where the other is expected.
pub const MANAGE_COOKIE_NAME: &str = "manage_session";

/// How long a site-admin session lives before it must be re-established, in
/// seconds. Matches the operator session's default lifetime.
pub const MANAGE_SESSION_TTL_SECS: u64 = 30 * 60;

/// Wire-format version prefix. Bumped when the record layout changes
/// incompatibly.
pub const MANAGE_FORMAT_VERSION: &str = "m1";

/// The record's leading tag, so a blob from any other use of the same session
/// key cannot be mistaken for a site-admin session.
const KIND_TAG: &[u8] = b"steel-site-admin-v1";

/// The greatest admin-name length the record will carry, in bytes.
const MAX_NAME_LEN: usize = 64;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENCODE                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Mint a signed site-admin session cookie value for `name`, expiring
/// [`MANAGE_SESSION_TTL_SECS`] from now.
///
/// The name is the admin entry whose passphrase unwrapped the wallet, kept only
/// so the console header can say who is signed in. It is never a secret and
/// never a scope.
pub fn encode(state: &AdminState, name: &str) -> Outcome<String> {
	let plain = res!(encode_record(name));
	let cipher = res!(state.session_enc.encrypt(&plain));
	let blob = base2x::HEMATITE64.to_string(&cipher);
	Ok(fmt!("{}.{}", MANAGE_FORMAT_VERSION, blob))
}

/// Serialise the tagged, length-prefixed plaintext record: the kind tag, the
/// name, and the expiry.
fn encode_record(name: &str) -> Outcome<Vec<u8>> {
	let nb = name.as_bytes();
	if nb.len() > MAX_NAME_LEN {
		return Err(err!(
			"Admin name is {} bytes; max is {} for a manage session.",
			nb.len(), MAX_NAME_LEN;
			Input, TooBig));
	}
	let exp = now_secs().saturating_add(MANAGE_SESSION_TTL_SECS);
	let mut out = Vec::with_capacity(KIND_TAG.len() + 2 + nb.len() + 8);
	out.extend_from_slice(KIND_TAG);
	out.extend_from_slice(&(nb.len() as u16).to_be_bytes());
	out.extend_from_slice(nb);
	out.extend_from_slice(&exp.to_be_bytes());
	Ok(out)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DECODE                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Decode and verify a manage session cookie value, returning the admin's name
/// when the ciphertext authenticates, the tag matches, and the expiry has not
/// passed.
///
/// A tagged error, rather than an `Option`, so the caller can log *why* a
/// session was rejected -- tampering, a wrong key, expiry, a bad format --
/// without any of it reaching the client.
pub fn decode(state: &AdminState, cookie: &str) -> Outcome<String> {
	let (ver, blob) = match cookie.split_once('.') {
		Some(p)	=> p,
		None	=> return Err(err!(
			"Manage session cookie has no version prefix.";
			Input, Invalid)),
	};
	if ver != MANAGE_FORMAT_VERSION {
		return Err(err!(
			"Manage session cookie version '{}' is not recognised (expected '{}').",
			ver, MANAGE_FORMAT_VERSION;
			Input, Invalid, Mismatch));
	}
	let cipher = res!(base2x::HEMATITE64.from_str(blob));
	let plain = res!(state.session_enc.decrypt(&cipher));
	decode_record(&plain)
}

/// Parse the tagged, length-prefixed plaintext record back into the admin's
/// name, refusing an expired record.
fn decode_record(bytes: &[u8]) -> Outcome<String> {
	// The tag first: a record that does not open with it is not a site-admin
	// session, whatever else it might decrypt to.
	if bytes.len() < KIND_TAG.len() || &bytes[..KIND_TAG.len()] != KIND_TAG {
		return Err(err!(
			"Manage session record is not tagged as a site-admin session.";
			Input, Invalid, Mismatch));
	}
	let mut p = KIND_TAG.len();

	if p + 2 > bytes.len() {
		return Err(err!(
			"Manage session record truncated reading the name length.";
			Input, Invalid, TooSmall));
	}
	let name_len = u16::from_be_bytes([bytes[p], bytes[p + 1]]) as usize;
	p += 2;
	if name_len > MAX_NAME_LEN {
		return Err(err!(
			"Manage session record claims a {}-byte name; max is {}.",
			name_len, MAX_NAME_LEN;
			Input, TooBig));
	}
	if p + name_len > bytes.len() {
		return Err(err!(
			"Manage session record truncated reading the name.";
			Input, Invalid, TooSmall));
	}
	let name = res!(std::str::from_utf8(&bytes[p..p + name_len]),
		Decode, String).to_string();
	p += name_len;

	if p + 8 != bytes.len() {
		return Err(err!(
			"Manage session record has an unexpected length after the name.";
			Input, Invalid));
	}
	let mut exp_arr = [0u8; 8];
	exp_arr.copy_from_slice(&bytes[p..p + 8]);
	let exp = u64::from_be_bytes(exp_arr);

	let now = now_secs();
	if exp <= now {
		return Err(err!(
			"Manage session expired at unix {} (now {}).", exp, now;
			Input, Invalid, Security));
	}
	Ok(name)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FROM A REQUEST                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// The raw manage session cookie value a request carries, if any.
///
/// The bytes as the browser sent them, unverified. Used both as the seed the
/// console's CSRF token is derived from -- it is `HttpOnly` and `SameSite=Strict`,
/// so no script reads it and no cross-site request sends it -- and by
/// [`authenticate`] on its way to a verified name.
pub fn cookie_value(headers: &Arc<HeaderFields>) -> Option<String> {
	if let Some(HeaderFieldValue::Cookie(cookies)) =
		headers.get_one(&HeaderName::Cookie)
	{
		for c in cookies {
			if c.key == MANAGE_COOKIE_NAME {
				return Some(c.val.clone());
			}
		}
	}
	None
}

/// The admin a request's manage session names, if it carries a valid one.
///
/// Every rejection -- no cookie, a tampered or forged one, an expired one --
/// flattens to `None`, logged at debug, so the gate can simply fall through to
/// the member paths.
pub fn authenticate(state: &AdminState, headers: &Arc<HeaderFields>) -> Option<String> {
	let value = cookie_value(headers)?;
	match decode(state, &value) {
		Ok(name)	=> Some(name),
		Err(e)		=> {
			debug!("console: manage session rejected: {}", e);
			None
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
	use super::*;
	use crate::srv::admin::{
		host_sampler::HostSampler,
		state::AdminState,
		traffic::TrafficRecorder,
	};
	use oxedyne_fe2o3_crypto::keystore::Wallet;
	use std::{
		path::PathBuf,
		sync::RwLock,
	};

	/// An unsealed admin state, mirroring the one the session tests use.
	fn mkstate() -> AdminState {
		AdminState::new(
			Arc::new(RwLock::new(Wallet::default())),
			PathBuf::from("./wallet.jdat"),
			Some([0u8; 32].to_vec()),
			1,
			None,
			TrafficRecorder::new_shared(0),
			HostSampler::new_shared(),
			crate::srv::admin::guard::new_shared().expect("addr guard"),
			crate::srv::admin::guard::new_shared().expect("auth guard"),
			Vec::new(),
			None,
		).expect("admin state")
	}

	/// A minted session round-trips to the name it was minted for.
	#[test]
	fn round_trip_00() -> Outcome<()> {
		let state = mkstate();
		let cookie = res!(encode(&state, "jason"));
		assert!(cookie.starts_with("m1."));
		assert_eq!(res!(decode(&state, &cookie)), fmt!("jason"));
		Ok(())
	}

	/// A tampered ciphertext fails to authenticate: the AES-GCM tag does not
	/// verify, so no forged name comes back.
	#[test]
	fn rejects_tampered_01() -> Outcome<()> {
		let state = mkstate();
		let cookie = res!(encode(&state, "jason"));
		let mut bytes = cookie.into_bytes();
		let idx = bytes.len() - 5;
		bytes[idx] ^= 0x01;
		let tampered = String::from_utf8_lossy(&bytes).into_owned();
		assert!(decode(&state, &tampered).is_err(), "a tampered manage cookie passed");
		Ok(())
	}

	/// Garbage in the cookie slot is refused, not read as a session.
	#[test]
	fn rejects_garbage_02() -> Outcome<()> {
		let state = mkstate();
		assert!(decode(&state, "not-a-cookie").is_err());
		assert!(decode(&state, "m1.").is_err());
		assert!(decode(&state, "m1.zzzz").is_err());
		assert!(decode(&state, "").is_err());
		Ok(())
	}

	/// A session sealed under one process's key is refused by another's: the
	/// key is per-process, so a manage cookie is not portable.
	#[test]
	fn not_portable_between_processes_03() -> Outcome<()> {
		let a = mkstate();
		let b = mkstate();
		let cookie = res!(encode(&a, "jason"));
		assert!(decode(&a, &cookie).is_ok());
		assert!(decode(&b, &cookie).is_err(), "a manage cookie crossed processes");
		Ok(())
	}
}
