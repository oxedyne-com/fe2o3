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
//! passkey, over `fe2o3_crypto`'s strict Ed25519 verifier.
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

use oxedyne_fe2o3_crypto::sign::verify_ed25519;

use ring::digest::{
    Context,
    SHA256,
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


/// The fixed head of authenticator data, which `credentials.create()` returns as well as
/// `credentials.get()`: an enrolment reads it from `getAuthenticatorData()` to check the
/// rpIdHash and the UP and UV flags before it stores a key. Only the head is read; the attested
/// credential data and extensions after it, which are CBOR, are not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthDataHead {
    pub rp_id_hash:     [u8; 32],
    pub user_present:   bool,   // UP
    pub user_verified:  bool,   // UV
    pub counter:        u32,    // signCount, big-endian on the wire
}

impl AuthDataHead {
    pub fn read(authenticator_data: &[u8]) -> Outcome<Self> {
        if authenticator_data.len() < AUTH_DATA_MIN_LEN {
            return Err(err!(
                "WebAuthn authenticatorData is {} bytes, need at least {}.",
                authenticator_data.len(), AUTH_DATA_MIN_LEN;
                Invalid, Input, Size));
        }
        let mut rp_id_hash = [0u8; 32];
        rp_id_hash.copy_from_slice(&authenticator_data[0..32]);
        let flags = authenticator_data[32];
        Ok(Self {
            rp_id_hash,
            user_present:   flags & FLAG_UP != 0,
            user_verified:  flags & FLAG_UV != 0,
            counter:        u32::from_be_bytes([
                authenticator_data[33],
                authenticator_data[34],
                authenticator_data[35],
                authenticator_data[36],
            ]),
        })
    }

    /// Was this authenticator data made for `rp_id`?
    pub fn rp_id_matches(&self, rp_id: &str) -> bool {
        self.rp_id_hash == sha256(rp_id.as_bytes())
    }
}

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
    // clientDataJSON: type, challenge, origin and crossOrigin. Parsed as JSON via jdat, the
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

    // crossOrigin: true means the ceremony ran in a frame whose origin differs
    // from the page above it, while `origin` still names the relying party. That
    // is how another site steers a user into an approval, so it is refused.
    // Absent (older browsers) or false passes; any other value is malformed.
    match cd.get(&dat!("crossOrigin")) {
        None | Some(Dat::Bool(false)) => (),
        Some(Dat::Bool(true)) => return Err(err!(
            "WebAuthn clientDataJSON crossOrigin is true for origin {:?}: the \
            ceremony ran in a cross-origin frame.", expect_origin;
            Invalid, Input, Security)),
        Some(other) => return Err(err!(
            "WebAuthn clientDataJSON crossOrigin for origin {:?} is not a boolean, \
            got {:?}.", expect_origin, other.kind();
            Invalid, Input, Decode)),
    }

    // authenticatorData: rpIdHash, flags and the signature counter.
    let head = res!(AuthDataHead::read(authenticator_data));
    if !head.rp_id_matches(rp_id) {
        return Err(err!(
            "WebAuthn rpIdHash does not match SHA-256({:?}).", rp_id;
            Invalid, Input, Mismatch, Security));
    }
    if !head.user_present {
        return Err(err!(
            "WebAuthn assertion has the user-present (UP) flag clear.";
            Invalid, Input, Security));
    }
    if !head.user_verified {
        return Err(err!(
            "WebAuthn assertion has the user-verified (UV) flag clear; \
            userVerification:'required' was expected.";
            Invalid, Input, Security));
    }
    let (counter, user_present, user_verified) =
        (head.counter, head.user_present, head.user_verified);

    // The signature is over authenticatorData || SHA-256(clientDataJSON).
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&sha256(client_data_json));

    let ok = match alg {
        CoseAlg::Es256 => verify_p256_sha256_asn1(stored_key, &signed, signature),
        // Hematite's one Ed25519 verifier, which refuses a small-order key that
        // `ring`'s would take as signing anything.
        CoseAlg::EdDsa => matches!(verify_ed25519(stored_key, &signed, signature), Ok(true)),
    };
    if !ok {
        return Err(err!(
            "WebAuthn assertion signature does not verify against the stored credential key.";
            Invalid, Input, Security));
    }

    Ok(VerifiedAssertion { counter, user_present, user_verified })
}


// The DER head of a P-256 `SubjectPublicKeyInfo`: SEQUENCE { SEQUENCE { id-ecPublicKey,
// prime256v1 }, BIT STRING (66 bytes, no unused bits) }, after which come the 65 bytes of the
// uncompressed point.
const SPKI_P256_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
// The DER head of an Ed25519 `SubjectPublicKeyInfo` (RFC 8410): SEQUENCE { SEQUENCE { id-Ed25519 },
// BIT STRING (33 bytes, no unused bits) }, after which come the 32 key bytes.
const SPKI_ED25519_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Turn the `SubjectPublicKeyInfo` a browser returns from `response.getPublicKey()` at enrolment
/// into the algorithm and key form [`verify_assertion`] takes: the 65-byte uncompressed SEC1 point
/// for a P-256 key, the 32 raw bytes for an Ed25519 one.
///
/// Only the two exact encodings are accepted. A browser emits nothing else for these algorithms,
/// so anything that differs -- an RSA or P-384 key, a compressed point, trailing bytes -- is
/// refused rather than parsed generously.
pub fn stored_key_from_spki(spki_der: &[u8]) -> Outcome<(CoseAlg, Vec<u8>)> {
    let p256_len = SPKI_P256_PREFIX.len() + 65;
    if spki_der.len() == p256_len && spki_der.starts_with(&SPKI_P256_PREFIX) {
        let point = &spki_der[SPKI_P256_PREFIX.len()..];
        if point[0] != 0x04 {
            return Err(err!(
                "WebAuthn SPKI: a P-256 key whose point starts 0x{:02x}, not 0x04 (uncompressed).",
                point[0];
                Invalid, Input, Security));
        }
        return Ok((CoseAlg::Es256, point.to_vec()));
    }
    let ed_len = SPKI_ED25519_PREFIX.len() + 32;
    if spki_der.len() == ed_len && spki_der.starts_with(&SPKI_ED25519_PREFIX) {
        return Ok((CoseAlg::EdDsa, spki_der[SPKI_ED25519_PREFIX.len()..].to_vec()));
    }
    Err(err!(
        "WebAuthn SPKI: {} bytes holding {}, where only a P-256 ({} bytes) or an Ed25519 ({} \
        bytes) key is accepted. It begins {}.",
        spki_der.len(), spki_key_kind(spki_der), p256_len, ed_len,
        spki_der.iter().take(26).map(|b| fmt!("{:02x}", b)).collect::<String>();
        Invalid, Input, Security))
}

// Names the key an unaccepted SPKI holds, by the algorithm and curve object identifiers it
// carries, so the refusal says what was offered rather than only that it was wrong.
fn spki_key_kind(der: &[u8]) -> &'static str {
    const KNOWN: [(&[u8], &str); 7] = [
        (&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],     "a P-256 key in an unexpected encoding"),
        (&[0x2b, 0x81, 0x04, 0x00, 0x22],                       "a P-384 key"),
        (&[0x2b, 0x81, 0x04, 0x00, 0x23],                       "a P-521 key"),
        (&[0x2b, 0x81, 0x04, 0x00, 0x0a],                       "a secp256k1 key"),
        (&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01], "an RSA key"),
        (&[0x2b, 0x65, 0x71],                                   "an Ed448 key"),
        (&[0x2b, 0x65, 0x70],                                   "an Ed25519 key in an unexpected encoding"),
    ];
    for (oid, name) in KNOWN.iter() {
        if der.windows(oid.len()).any(|w| w == *oid) {
            return name;
        }
    }
    "no key this recognises"
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

    /// An EdDSA assertion signed by a real Ed25519 key verifies, and one "signed"
    /// for the identity as a key, with R the identity and S zero, is refused.
    /// That pair satisfies the cofactorless equation over any message, and
    /// `ring`'s Ed25519 verifier, which this path used before, takes it, as the
    /// second assertion shows.
    #[test]
    fn test_verify_assertion_eddsa_refuses_a_small_order_key() -> Outcome<()> {
        use oxedyne_fe2o3_crypto::sign::SignatureScheme;
        use oxedyne_fe2o3_iop_crypto::{
            keys::KeyManager,
            sign::Signer,
        };

        let challenge = b"eddsa-challenge";
        let ad = make_auth_data(RP_ID, FLAG_UP | FLAG_UV, 3);
        let cdj = client_data("webauthn.get", challenge, ORIGIN);
        let mut signed = ad.clone();
        signed.extend_from_slice(&sha256(&cdj));

        let key = SignatureScheme::new_ed25519();
        let public = match res!(key.get_public_key()) {
            Some(pk) => pk.to_vec(),
            None => return Err(err!("A new Ed25519 key has no public half."; Test, Missing)),
        };
        let sig = res!(key.sign(&signed));
        let v = res!(verify_assertion(
            CoseAlg::EdDsa, &public, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));
        assert_eq!(v.counter, 3);

        let mut identity = [0u8; 32];
        identity[0] = 0x01;
        let mut forged = [0u8; 64];
        forged[0] = 0x01;
        assert!(ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &identity)
            .verify(&signed, &forged).is_ok(),
            "ring's verifier takes the forgery, which is why this path no longer uses it");
        assert!(verify_assertion(
            CoseAlg::EdDsa, &identity, challenge, ORIGIN, RP_ID, &ad, &cdj, &forged).is_err(),
            "the identity key's forgery must be refused");
        Ok(())
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

    /// A ceremony run inside a cross-origin frame says so in `crossOrigin`. An
    /// assertion carrying `"crossOrigin":true` is refused even though every other
    /// check and the signature pass; `false`, or no member at all, still verifies.
    #[test]
    fn test_verify_assertion_refuses_cross_origin() -> Outcome<()> {
        let key = res!(Es256Key::new());
        let challenge = b"cross-origin-challenge";
        let ad = make_auth_data(RP_ID, FLAG_UP | FLAG_UV, 5);
        let sign_over = |cdj: &[u8]| -> Outcome<Vec<u8>> {
            let mut signed = ad.clone();
            signed.extend_from_slice(&sha256(cdj));
            key.sign(&signed)
        };
        let with_cross = |cross: &str| fmt!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"{}\",\
            \"crossOrigin\":{}}}",
            base64url_encode(challenge), ORIGIN, cross,
        ).into_bytes();

        let cdj = with_cross("true");
        let sig = res!(sign_over(&cdj));
        match verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig)
        {
            Ok(_) => return Err(err!(
                "An assertion with crossOrigin true verified."; Test, Security)),
            Err(e) => {
                let msg = fmt!("{}", e);
                assert!(msg.contains("crossOrigin") && msg.contains(ORIGIN),
                    "the refusal names the field and the origin: {}", msg);
            }
        }

        let cdj = with_cross("false");
        let sig = res!(sign_over(&cdj));
        res!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));

        let cdj = client_data("webauthn.get", challenge, ORIGIN);
        let sig = res!(sign_over(&cdj));
        res!(verify_assertion(
            CoseAlg::Es256, &key.pubkey, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));
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

    // ── SPKI extraction ───────────────────────────────────────────────────

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
            .collect()
    }

    /// A P-256 key generated by `ring`, encoded as an SPKI by `rcgen` rather than by this module,
    /// comes back as the very point `ring` reports, and an assertion it signs verifies against it.
    #[cfg(feature = "async")]
    #[test]
    fn test_stored_key_from_spki_p256_ring_key() -> Outcome<()> {
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
        let spki = match rcgen::KeyPair::from_der(pkcs8.as_ref()) {
            Ok(k) => k.public_key_der(),
            Err(e) => return Err(err!("rcgen load failed: {}.", e; Test, Init)),
        };
        let (alg, key) = res!(stored_key_from_spki(&spki));
        assert_eq!(alg, CoseAlg::Es256);
        assert_eq!(key.as_slice(), kp.public_key().as_ref());
        assert_eq!(key.len(), 65);

        let challenge = b"spki-p256";
        let ad = make_auth_data(RP_ID, FLAG_UP | FLAG_UV, 1);
        let cdj = client_data("webauthn.get", challenge, ORIGIN);
        let mut signed = ad.clone();
        signed.extend_from_slice(&sha256(&cdj));
        let sig = match kp.sign(&rng, &signed) {
            Ok(s) => s.as_ref().to_vec(),
            Err(e) => return Err(err!("ring sign failed: {}.", e; Test, Data)),
        };
        let v = res!(verify_assertion(alg, &key, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));
        assert!(v.user_verified);
        Ok(())
    }

    /// The same for an Ed25519 key.
    #[cfg(feature = "async")]
    #[test]
    fn test_stored_key_from_spki_ed25519_ring_key() -> Outcome<()> {
        use ring::signature::Ed25519KeyPair;
        let rng = SystemRandom::new();
        let pkcs8 = match Ed25519KeyPair::generate_pkcs8(&rng) {
            Ok(d) => d,
            Err(e) => return Err(err!("ring pkcs8 gen failed: {}.", e; Test, Init)),
        };
        let kp = match Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()) {
            Ok(k) => k,
            Err(e) => return Err(err!("ring pkcs8 load failed: {}.", e; Test, Init)),
        };
        let spki = match rcgen::KeyPair::from_der(pkcs8.as_ref()) {
            Ok(k) => k.public_key_der(),
            Err(e) => return Err(err!("rcgen load failed: {}.", e; Test, Init)),
        };
        let (alg, key) = res!(stored_key_from_spki(&spki));
        assert_eq!(alg, CoseAlg::EdDsa);
        assert_eq!(key.as_slice(), kp.public_key().as_ref());

        let challenge = b"spki-ed25519";
        let ad = make_auth_data(RP_ID, FLAG_UP | FLAG_UV, 2);
        let cdj = client_data("webauthn.get", challenge, ORIGIN);
        let mut signed = ad.clone();
        signed.extend_from_slice(&sha256(&cdj));
        let sig = kp.sign(&signed).as_ref().to_vec();
        let v = res!(verify_assertion(alg, &key, challenge, ORIGIN, RP_ID, &ad, &cdj, &sig));
        assert_eq!(v.counter, 2);
        Ok(())
    }

    /// Fixed encodings made elsewhere: a P-256 SPKI from `openssl ec -pubout -outform DER`, and
    /// the Ed25519 public key of RFC 8410 section 10.1.
    #[test]
    fn test_stored_key_from_spki_fixed_vectors() -> Outcome<()> {
        let p256 = hex("3059301306072a8648ce3d020106082a8648ce3d03010703420004f3bb3612ddeaa340\
            2901985c75ce5a1f71e4edf4587ae216401670c574f11250938a94e952f87af1459d7b47f6ca176c74ae\
            03d7fde4384d6e165e4f8fc6a9c3".split_whitespace().collect::<String>().as_str());
        let (alg, key) = res!(stored_key_from_spki(&p256));
        assert_eq!(alg, CoseAlg::Es256);
        assert_eq!(&key[..], &p256[26..]);

        let ed = hex("302a300506032b657003210019bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1");
        let (alg, key) = res!(stored_key_from_spki(&ed));
        assert_eq!(alg, CoseAlg::EdDsa);
        assert_eq!(key, hex("19bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1"));
        Ok(())
    }

    /// Anything other than the two exact encodings is refused, and the refusal names what it saw.
    #[test]
    fn test_stored_key_from_spki_refusals() {
        let p256 = hex("3059301306072a8648ce3d020106082a8648ce3d03010703420004f3bb3612ddeaa340\
            2901985c75ce5a1f71e4edf4587ae216401670c574f11250938a94e952f87af1459d7b47f6ca176c74ae\
            03d7fde4384d6e165e4f8fc6a9c3".split_whitespace().collect::<String>().as_str());
        let msg = |der: &[u8]| match stored_key_from_spki(der) {
            Ok(_)   => String::from("accepted"),
            Err(e)  => fmt!("{}", e),
        };

        let mut trailing = p256.clone();
        trailing.push(0);
        assert!(msg(&trailing).contains("P-256 key in an unexpected encoding"), "{}", msg(&trailing));

        let mut hybrid = p256.clone();
        hybrid[26] = 0x06;
        assert!(msg(&hybrid).contains("not 0x04"), "{}", msg(&hybrid));

        assert!(msg(&p256[..60]).contains("60 bytes"), "{}", msg(&p256[..60]));

        // The head of an RSA and of a P-384 SPKI; the key bytes do not matter to the refusal.
        let mut rsa = hex("30820122300d06092a864886f70d01010105000382010f00");
        rsa.extend_from_slice(&[0u8; 270]);
        assert!(msg(&rsa).contains("an RSA key"), "{}", msg(&rsa));
        let mut p384 = hex("3076301006072a8648ce3d020106052b81040022036200");
        p384.extend_from_slice(&[4u8; 97]);
        assert!(msg(&p384).contains("a P-384 key"), "{}", msg(&p384));

        assert!(msg(&[]).contains("no key this recognises"), "{}", msg(&[]));
    }

    /// The head of enrolment-style authenticator data -- head, then attested credential data --
    /// reads as the verifier reads it, and a short buffer is refused naming its length.
    #[test]
    fn test_auth_data_head_00() -> Outcome<()> {
        let mut ad = make_auth_data(RP_ID, FLAG_UP | FLAG_UV | 0x40, 0x0102_0304);
        ad.extend_from_slice(&[0xa5; 80]);
        let head = res!(AuthDataHead::read(&ad));
        assert!(head.rp_id_matches(RP_ID));
        assert!(!head.rp_id_matches("evil.example.com"));
        assert!(head.user_present);
        assert!(head.user_verified);
        assert_eq!(head.counter, 0x0102_0304);

        let head = res!(AuthDataHead::read(&make_auth_data(RP_ID, FLAG_UP, 0)));
        assert!(head.user_present && !head.user_verified);

        match AuthDataHead::read(&ad[..36]) {
            Ok(_)   => return Err(err!("A 36-byte head was accepted."; Test)),
            Err(e)  => assert!(fmt!("{}", e).contains("36 bytes"), "{}", e),
        }
        Ok(())
    }
}
