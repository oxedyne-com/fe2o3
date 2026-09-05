//! WebAuthn assertion verification -- the server half of the
//! `navigator.credentials.get()` ceremony.
//!
//! This is deliberately the *assertion* path only, and deliberately small. The
//! one-time *registration* ceremony (`credentials.create()`) carries the only
//! CBOR/COSE in WebAuthn -- the attestation object -- and a single-admin,
//! out-of-band enrolment never needs the server to parse it: the operator reads
//! the public key from `response.getPublicKey()` in the browser and records it
//! in an allowlist. So nothing here touches CBOR. What remains is a byte-layout
//! parse of `authenticatorData`, a handful of equality and flag checks on the
//! `clientDataJSON`, and one ECDSA (or EdDSA) signature verification over the
//! primitives this crate already carries.
//!
//! The signed message a platform authenticator produces is
//! `authenticatorData || SHA-256(clientDataJSON)`; ES256 (COSE `-7`) signs it as
//! a DER `SEQUENCE { r, s }` -- hence [`crate::ecdsa::verify_p256_sha256_asn1`],
//! the ASN.1 sibling of the fixed-form verifier a browser's own WebCrypto key
//! uses. ES256 is the near-universal default of Apple, Google and Windows
//! authenticators; EdDSA (COSE `-8`) is modelled too for the rare Ed25519
//! passkey, over `ring`'s Ed25519 verifier.
//!
//! A second downstream caller (a device-key admin surface, an operator console)
//! reuses this verbatim, which is why it lives here rather than in one app.
//!
//! Reference: WebAuthn Level 2, §5.8.1 (attested/authenticator data) and §7.2
//! (verifying an authentication assertion).

use crate::acme::jose::base64url_decode;
use crate::ecdsa::verify_p256_sha256_asn1;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
};

use ring::{
    digest::{
        Context,
        SHA256,
    },
    signature::{
        UnparsedPublicKey,
        ED25519,
    },
};


/// The COSE signature algorithm a stored credential was registered under. Only
/// the two a platform authenticator emits are modelled; the caller records which
/// at enrolment and passes it back on every assertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoseAlg {
    Es256,  // COSE -7: ECDSA P-256 + SHA-256, DER signature, 65-byte SEC1 key
    EdDsa,  // COSE -8: Ed25519, 64-byte signature, 32-byte key
}

/// A verified assertion. The caller advances its own per-credential replay guard
/// from `counter` (reject a value not greater than the stored one) and may record
/// which user-verification the authenticator asserted.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedAssertion {
    pub counter:        u32,    // authenticator signature counter, for replay defence
    pub user_present:   bool,   // UP flag was set
    pub user_verified:  bool,   // UV flag was set (PIN/biometric on this assertion)
}

// authenticatorData flag bits (WebAuthn §6.1).
const FLAG_UP: u8 = 0x01;   // user present
const FLAG_UV: u8 = 0x04;   // user verified

// rpIdHash(32) || flags(1) || signCount(4): the fixed head every assertion carries.
const AUTH_DATA_MIN_LEN: usize = 37;


fn sha256(data: &[u8]) -> [u8; 32] {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(ctx.finish().as_ref());
    out
}

/// Verify a WebAuthn authentication assertion, in the order that fails cheapest
/// and most specifically first.
///
/// # Arguments
///
/// - `alg`, `stored_key`: the credential's algorithm and public key as recorded
///   at enrolment -- a 65-byte uncompressed SEC1 point for ES256, a 32-byte key
///   for EdDSA.
/// - `expect_challenge`: the raw challenge bytes the server issued for this
///   login (one-time, short TTL, consumed by the caller). Compared against the
///   base64url `challenge` inside `clientDataJSON`.
/// - `expect_origin`: the exact `origin` the ceremony must have run at, e.g.
///   `https://admin.example.com` -- the anti-phishing bind.
/// - `rp_id`: the relying-party id, e.g. `admin.example.com`; its SHA-256 must
///   equal the first 32 bytes of `authenticator_data`.
/// - `authenticator_data`, `client_data_json`, `signature`: the three fields the
///   browser returns from `credentials.get()`.
pub fn verify_assertion(
    alg:                CoseAlg,
    stored_key:         &[u8],
    expect_challenge:   &[u8],
    expect_origin:      &str,
    rp_id:              &str,
    authenticator_data: &[u8],
    client_data_json:   &[u8],
    signature:          &[u8],
)
    -> Outcome<VerifiedAssertion>
{
    // clientDataJSON: type, challenge and origin. Parsed as JSON via jdat, the
    // same decoder the app dialects use.
    let cfg = DecoderConfig::<(), ()>::json(None);
    let text = String::from_utf8_lossy(client_data_json).to_string();
    let cd = match res!(Dat::decode_string_with_config(text, &cfg)) {
        Dat::Map(m) => m,
        other => return Err(err!(
            "WebAuthn clientDataJSON is not a JSON object, got {:?}.", other.kind();
            Invalid, Input, Decode)),
    };
    let cd_str = |k: &str| match cd.get(&dat!(k)) {
        Some(Dat::Str(s)) => Some(s.clone()),
        _ => None,
    };

    match cd_str("type").as_deref() {
        Some("webauthn.get") => (),
        other => return Err(err!(
            "WebAuthn clientDataJSON type is {:?}, expected \"webauthn.get\".", other;
            Invalid, Input, Mismatch)),
    }

    let challenge_b64 = res!(cd_str("challenge").ok_or_else(|| err!(
        "WebAuthn clientDataJSON carries no challenge."; Invalid, Input, Missing)));
    let got_challenge = res!(base64url_decode(&challenge_b64));
    if got_challenge != expect_challenge {
        return Err(err!(
            "WebAuthn assertion challenge does not match the one issued; \
            possible replay or a stale login.";
            Invalid, Input, Mismatch, Security));
    }

    match cd_str("origin").as_deref() {
        Some(o) if o == expect_origin => (),
        other => return Err(err!(
            "WebAuthn assertion origin is {:?}, expected {:?} (anti-phishing bind).",
            other, expect_origin;
            Invalid, Input, Mismatch, Security)),
    }

    // authenticatorData: rpIdHash, flags and the signature counter.
    if authenticator_data.len() < AUTH_DATA_MIN_LEN {
        return Err(err!(
            "WebAuthn authenticatorData is {} bytes, need at least {}.",
            authenticator_data.len(), AUTH_DATA_MIN_LEN;
            Invalid, Input, Size));
    }
    if authenticator_data[0..32] != sha256(rp_id.as_bytes()) {
        return Err(err!(
            "WebAuthn rpIdHash does not match SHA-256({:?}).", rp_id;
            Invalid, Input, Mismatch, Security));
    }
    let flags = authenticator_data[32];
    let user_present  = flags & FLAG_UP != 0;
    let user_verified = flags & FLAG_UV != 0;
    if !user_present {
        return Err(err!(
            "WebAuthn assertion has the user-present (UP) flag clear.";
            Invalid, Input, Security));
    }
    if !user_verified {
        return Err(err!(
            "WebAuthn assertion has the user-verified (UV) flag clear; \
            userVerification:'required' was expected.";
            Invalid, Input, Security));
    }
    let counter = u32::from_be_bytes([
        authenticator_data[33],
        authenticator_data[34],
        authenticator_data[35],
        authenticator_data[36],
    ]);

    // The signature is over authenticatorData || SHA-256(clientDataJSON).
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&sha256(client_data_json));

    let ok = match alg {
        CoseAlg::Es256 => verify_p256_sha256_asn1(stored_key, &signed, signature),
        CoseAlg::EdDsa => {
            UnparsedPublicKey::new(&ED25519, stored_key).verify(&signed, signature).is_ok()
        }
    };
    if !ok {
        return Err(err!(
            "WebAuthn assertion signature does not verify against the stored credential key.";
            Invalid, Input, Security));
    }

    Ok(VerifiedAssertion { counter, user_present, user_verified })
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::acme::jose::base64url_encode;

    use ring::{
        rand::SystemRandom,
        signature::{
            EcdsaKeyPair,
            KeyPair,
            ECDSA_P256_SHA256_ASN1_SIGNING,
        },
    };

    const RP_ID:  &str = "admin.sideye.oxegen.io";
    const ORIGIN: &str = "https://admin.sideye.oxegen.io";

    /// Build the `authenticatorData` head an authenticator would emit: the
    /// SHA-256 of the rpId, a flags byte, and a big-endian counter. Assertions
    /// carry no attested credential data, so 37 bytes is the whole of it.
    fn make_auth_data(rp_id: &str, flags: u8, counter: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(AUTH_DATA_MIN_LEN);
        v.extend_from_slice(&sha256(rp_id.as_bytes()));
        v.push(flags);
        v.extend_from_slice(&counter.to_be_bytes());
        v
    }

    fn client_data(ty: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
        // The browser emits compact JSON; the exact bytes are what gets hashed,
        // so build them directly rather than through an encoder.
        fmt!(
            "{{\"type\":\"{}\",\"challenge\":\"{}\",\"origin\":\"{}\"}}",
            ty, base64url_encode(challenge), origin,
        ).into_bytes()
    }

    struct Es256Key {
        kp:     EcdsaKeyPair,
        rng:    SystemRandom,
        pubkey: Vec<u8>,
    }

    impl Es256Key {
        fn new() -> Outcome<Self> {
            let rng = SystemRandom::new();
            let pkcs8 = match EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng) {
                Ok(d) => d,
                Err(e) => return Err(err!("ring pkcs8 gen failed: {}.", e; Test, Init)),
            };
            let kp = match EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng) {
                Ok(k) => k,
                Err(e) => return Err(err!("ring pkcs8 load failed: {}.", e; Test, Init)),
            };
            let pubkey = kp.public_key().as_ref().to_vec();
            Ok(Self { kp, rng, pubkey })
        }

        fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>> {
            match self.kp.sign(&self.rng, msg) {
                Ok(s) => Ok(s.as_ref().to_vec()),
                Err(e) => Err(err!("ring sign failed: {}.", e; Test, Data)),
            }
        }

        /// Produce a valid (authenticatorData, clientDataJSON, signature) triple.
        fn assert_over(&self, challenge: &[u8], flags: u8, counter: u32)
            -> Outcome<(Vec<u8>, Vec<u8>, Vec<u8>)>
        {
            let ad = make_auth_data(RP_ID, flags, counter);
            let cdj = client_data("webauthn.get", challenge, ORIGIN);
            let mut signed = ad.clone();
            signed.extend_from_slice(&sha256(&cdj));
            let sig = res!(self.sign(&signed));
            Ok((ad, cdj, sig))
        }
    }

    /// A well-formed ES256 assertion verifies, and its counter and flags come back.
    #[test]
    fn test_verify_assertion_es256_ok() -> Outcome<()> {
        let key = res!(Es256Key::new());
        let challenge = b"one-time-challenge-01";
        let (ad, cdj, sig) = res!(key.assert_over(challenge, FLAG_UP | FLAG_UV, 7));
        let v = res!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));
        assert_eq!(v.counter, 7);
        assert!(v.user_present && v.user_verified);
        Ok(())
    }

    /// Each guard rejects the thing it guards: a wrong challenge (replay), a wrong
    /// origin (phishing), a wrong rpId, a clear UV flag, a tampered signature, and
    /// a key that did not sign it.
    #[test]
    fn test_verify_assertion_rejections() -> Outcome<()> {
        let key = res!(Es256Key::new());
        let challenge = b"the-real-challenge";

        // Wrong challenge -- what a replayed assertion from an earlier login looks like.
        let (ad, cdj, sig) = res!(key.assert_over(challenge, FLAG_UP | FLAG_UV, 1));
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, b"a-different-challenge", ORIGIN, RP_ID,
            &ad, &cdj, &sig).is_err(),
            "a mismatched challenge must be rejected");

        // Wrong expected origin.
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, "https://evil.example.com", RP_ID,
            &ad, &cdj, &sig).is_err(),
            "a mismatched origin must be rejected");

        // Wrong rpId -> rpIdHash mismatch.
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, "other.example.com",
            &ad, &cdj, &sig).is_err(),
            "a mismatched rpId must be rejected");

        // UV flag clear, though the signature is otherwise valid.
        let (ad_no_uv, cdj2, sig2) = res!(key.assert_over(challenge, FLAG_UP, 2));
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad_no_uv, &cdj2, &sig2)
            .is_err(),
            "a clear UV flag must be rejected");

        // Tampered signature.
        let mut bad_sig = sig.clone();
        let last = bad_sig.len() - 1;
        bad_sig[last] ^= 0x01;
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &bad_sig)
            .is_err(),
            "a tampered signature must be rejected");

        // A different key did not sign this assertion.
        let other = res!(Es256Key::new());
        assert!(verify_assertion(
            CoseAlg::Es256, &other.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig)
            .is_err(),
            "the wrong credential key must be rejected");

        Ok(())
    }

    /// A short authenticatorData fails gracefully rather than panicking.
    #[test]
    fn test_verify_assertion_short_auth_data() -> Outcome<()> {
        let key = res!(Es256Key::new());
        let challenge = b"c";
        let (_ad, cdj, sig) = res!(key.assert_over(challenge, FLAG_UP | FLAG_UV, 1));
        let short = vec![0u8; 10];
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &short, &cdj, &sig)
            .is_err(),
            "a truncated authenticatorData must be rejected, not panic");
        Ok(())
    }

    /// Non-JSON clientDataJSON is refused, not a panic.
    #[test]
    fn test_verify_assertion_junk_client_data() -> Outcome<()> {
        let key = res!(Es256Key::new());
        let challenge = b"c";
        let (ad, _cdj, sig) = res!(key.assert_over(challenge, FLAG_UP | FLAG_UV, 1));
        assert!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad,
            b"not json at all", &sig).is_err(),
            "junk clientDataJSON must be refused");
        Ok(())
    }
}
