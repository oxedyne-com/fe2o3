//! ECDSA P-256 (NIST secp256r1) signature verification.
//!
//! A thin wrapper over `ring::signature` (already in Hematite's dependency
//! tree via this crate, and the same backend the ACME JWS signer uses)
//! exposing the one operation a downstream verifier needs: check a P-256
//! signature over a message given a public key.
//!
//! The motivating caller is a payment gateway verifying signatures from
//! browser device keypairs. WebCrypto exposes Ed25519 on some engines but
//! not others; where it is missing the browser falls back to ECDSA over
//! P-256 with SHA-256. This function accepts exactly the encodings that
//! WebCrypto emits, so the gateway can verify both an Ed25519 signature
//! (via [`crate`]'s Ed25519 path) and a P-256 signature uniformly.
//!
//! Accepted encodings:
//!
//! - Public key: the 65-byte uncompressed SEC1 point `0x04 || X || Y`, as
//!   produced by WebCrypto `exportKey('raw')` for an ECDSA P-256 key.
//! - Signature: the 64-byte fixed-length `r || s` form (IEEE P1363), which
//!   is what WebCrypto `crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' })`
//!   emits. This is `ring`'s `ECDSA_P256_SHA256_FIXED`.
//! - Message: the raw bytes as signed. It must NOT be pre-hashed --
//!   `ECDSA_P256_SHA256_FIXED` hashes the message with SHA-256 internally,
//!   matching WebCrypto's `hash: 'SHA-256'`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use ring::{
    rand::SystemRandom,
    signature::{
        EcdsaKeyPair,
        KeyPair,
        UnparsedPublicKey,
        ECDSA_P256_SHA256_ASN1,
        ECDSA_P256_SHA256_FIXED,
        ECDSA_P256_SHA256_FIXED_SIGNING,
    },
};


/// `pubkey` is the 65-byte uncompressed SEC1 point, `sig` the 64-byte `r || s`,
/// and `msg` the raw message rather than a digest, SHA-256 being applied within.
/// A wrong length or an ill-formed point fails to verify rather than panicking,
/// `ring` reporting every malformed input as an ordinary verification failure.
pub fn verify_p256_sha256_fixed(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pubkey);
    key.verify(msg, sig).is_ok()
}

/// The sibling of [`verify_p256_sha256_fixed`] for the ASN.1 DER signature form.
/// The key and message encodings are identical -- a 65-byte uncompressed SEC1
/// point and a raw (un-hashed) message -- but `sig` is the DER `SEQUENCE { r, s }`
/// rather than the 64-byte `r || s`. This is the shape a WebAuthn/CTAP
/// authenticator emits for an ES256 assertion (COSE algorithm `-7`), where the
/// fixed form of a browser's own WebCrypto key does not apply. As above, any
/// malformed input fails to verify rather than panicking.
pub fn verify_p256_sha256_asn1(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, pubkey);
    key.verify(msg, sig).is_ok()
}

/// A P-256 key pair that signs in the encodings [`verify_p256_sha256_fixed`] accepts.
///
/// The counterpart to the verifier above: where that checks what a browser produced, this produces
/// the same shapes from Rust -- a 65-byte uncompressed SEC1 public key and 64-byte `r || s`
/// signatures over SHA-256. A Rust client of a gateway that authenticates device keys needs it, and
/// so does any test that must present a real signature rather than a fixture.
///
/// The PKCS#8 bytes are retained so the key can be written to disk and reloaded: `ring` consumes
/// them at load time and does not hand them back.
pub struct P256KeyPair {
    pkcs8:      Vec<u8>,        // kept for persistence
    key_pair:   EcdsaKeyPair,   // ring's live key pair
    rng:        SystemRandom,   // ring wants one per signature
}

impl P256KeyPair {
    pub fn generate() -> Outcome<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = match EcdsaKeyPair::generate_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &rng,
        ) {
            Ok(doc) => doc.as_ref().to_vec(),
            Err(e) => return Err(err!(
                "ring could not generate a P-256 PKCS#8 document: {}.", e;
                Init, Unknown)),
        };
        let key_pair = res!(Self::load_pair(&pkcs8, &rng));
        Ok(Self { pkcs8, key_pair, rng })
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Outcome<Self> {
        let rng = SystemRandom::new();
        let key_pair = res!(Self::load_pair(pkcs8, &rng));
        Ok(Self {
            pkcs8:  pkcs8.to_vec(),
            key_pair,
            rng,
        })
    }

    /// The bytes round-trip through [`P256KeyPair::from_pkcs8`].
    pub fn pkcs8_bytes(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public key as the 65-byte uncompressed SEC1 point `0x04 || X || Y`, the same encoding
    /// WebCrypto `exportKey('raw')` yields and [`verify_p256_sha256_fixed`] expects.
    pub fn public_key(&self) -> Vec<u8> {
        self.key_pair.public_key().as_ref().to_vec()
    }

    /// The 64-byte fixed-length `r || s` form. `msg` is the raw message, not a
    /// digest: SHA-256 is applied within, matching WebCrypto's
    /// `sign({ name: 'ECDSA', hash: 'SHA-256' })`.
    pub fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>> {
        match self.key_pair.sign(&self.rng, msg) {
            Ok(sig) => Ok(sig.as_ref().to_vec()),
            Err(e) => Err(err!(
                "ring could not produce a P-256 signature: {}.", e;
                Unknown)),
        }
    }

    fn load_pair(pkcs8: &[u8], rng: &SystemRandom) -> Outcome<EcdsaKeyPair> {
        match EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8, rng) {
            Ok(kp) => Ok(kp),
            Err(e) => Err(err!(
                "ring rejected the supplied P-256 PKCS#8 bytes: {}.", e;
                Init, Invalid, Input)),
        }
    }
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
            ECDSA_P256_SHA256_FIXED_SIGNING,
        },
    };

    /// Round-trip a self-consistent vector generated with `ring`: create a
    /// P-256 key pair, sign a message, export the raw (65-byte uncompressed)
    /// public key and the 64-byte fixed signature, then verify. A tampered
    /// signature, message and key must all be rejected, and wrong-length
    /// inputs must fail gracefully rather than panic.
    #[test]
    fn test_p256_verify_round_trip() -> Outcome<()> {
        let rng = SystemRandom::new();

        // Fresh P-256 key pair.
        let pkcs8 = match EcdsaKeyPair::generate_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &rng,
        ) {
            Ok(doc) => doc,
            Err(e) => return Err(err!(
                "ring failed to generate a P-256 PKCS#8 document: {}.", e;
                Test, Init)),
        };
        let key_pair = match EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8.as_ref(),
            &rng,
        ) {
            Ok(kp) => kp,
            Err(e) => return Err(err!(
                "ring rejected its own freshly-generated P-256 PKCS#8: {}.", e;
                Test, Init)),
        };

        // The raw public key is the 65-byte uncompressed SEC1 point, exactly
        // what WebCrypto exportKey('raw') yields.
        let pubkey = key_pair.public_key().as_ref().to_vec();
        assert_eq!(pubkey.len(), 65, "P-256 raw public key must be 65 bytes");
        assert_eq!(pubkey[0], 0x04, "uncompressed SEC1 point must start with 0x04");

        // Sign a message. ring's FIXED variant hashes with SHA-256 internally
        // and emits the 64-byte r || s form.
        let msg = b"payment gateway device-key challenge";
        let sig = match key_pair.sign(&rng, msg) {
            Ok(s) => s.as_ref().to_vec(),
            Err(e) => return Err(err!(
                "ring failed to sign the P-256 test message: {}.", e;
                Test, Data)),
        };
        assert_eq!(sig.len(), 64, "P-256 fixed signature must be 64 bytes");

        // A valid signature verifies.
        assert!(verify_p256_sha256_fixed(&pubkey, msg, &sig),
            "verify should accept a valid P-256 signature");

        // A tampered signature is rejected.
        let mut bad_sig = sig.clone();
        bad_sig[0] ^= 0x01;
        assert!(!verify_p256_sha256_fixed(&pubkey, msg, &bad_sig),
            "verify should reject a tampered signature");

        // A tampered message is rejected.
        let mut bad_msg = msg.to_vec();
        bad_msg[0] ^= 0x01;
        assert!(!verify_p256_sha256_fixed(&pubkey, &bad_msg, &sig),
            "verify should reject a tampered message");

        // A tampered public key is rejected.
        let mut bad_key = pubkey.clone();
        bad_key[1] ^= 0x01; // Perturb the X coordinate, keep the 0x04 tag.
        assert!(!verify_p256_sha256_fixed(&bad_key, msg, &sig),
            "verify should reject a wrong public key");

        // Wrong-length inputs must fail gracefully, not panic.
        assert!(!verify_p256_sha256_fixed(&pubkey[..64], msg, &sig),
            "verify should reject a short public key");
        assert!(!verify_p256_sha256_fixed(&pubkey, msg, &sig[..63]),
            "verify should reject a short signature");
        assert!(!verify_p256_sha256_fixed(&[], msg, &sig),
            "verify should reject an empty public key");

        Ok(())
    }

    /// The ASN.1 sibling verifies what `ring`'s DER signer produces, in the shape a WebAuthn
    /// authenticator emits: a 65-byte SEC1 key, a raw message, and a DER `SEQUENCE { r, s }`
    /// signature. A tampered signature, message and key must all be rejected, the fixed-form
    /// verifier must NOT accept a DER signature (the two encodings are distinct), and
    /// wrong-length inputs must fail gracefully rather than panic.
    #[test]
    fn test_p256_verify_asn1_round_trip() -> Outcome<()> {
        use ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;

        let rng = SystemRandom::new();

        let pkcs8 = match EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng) {
            Ok(doc) => doc,
            Err(e) => return Err(err!(
                "ring failed to generate a P-256 ASN.1 PKCS#8 document: {}.", e; Test, Init)),
        };
        let key_pair = match EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            pkcs8.as_ref(),
            &rng,
        ) {
            Ok(kp) => kp,
            Err(e) => return Err(err!(
                "ring rejected its own freshly-generated ASN.1 PKCS#8: {}.", e; Test, Init)),
        };

        let pubkey = key_pair.public_key().as_ref().to_vec();
        assert_eq!(pubkey.len(), 65, "P-256 raw public key must be 65 bytes");
        assert_eq!(pubkey[0], 0x04, "uncompressed SEC1 point must start with 0x04");

        // The ASN.1 signer emits a DER SEQUENCE, variable length (~70-72 bytes),
        // never the fixed 64.
        let msg = b"webauthn.get assertion over authenticatorData || SHA-256(clientDataJSON)";
        let sig = match key_pair.sign(&rng, msg) {
            Ok(s) => s.as_ref().to_vec(),
            Err(e) => return Err(err!(
                "ring failed to sign the P-256 ASN.1 test message: {}.", e; Test, Data)),
        };
        assert_ne!(sig.len(), 64, "the DER form is not the 64-byte fixed form");
        assert_eq!(sig[0], 0x30, "a DER SEQUENCE begins with the 0x30 tag");

        // A valid DER signature verifies under the ASN.1 verifier.
        assert!(verify_p256_sha256_asn1(&pubkey, msg, &sig),
            "asn1 verify should accept a valid DER signature");

        // The fixed-form verifier must not accept a DER signature: the encodings
        // are distinct and must not be interchangeable.
        assert!(!verify_p256_sha256_fixed(&pubkey, msg, &sig),
            "the fixed verifier must reject a DER-encoded signature");

        // A tampered signature is rejected (perturb r, past the DER header).
        let mut bad_sig = sig.clone();
        let last = bad_sig.len() - 1;
        bad_sig[last] ^= 0x01;
        assert!(!verify_p256_sha256_asn1(&pubkey, msg, &bad_sig),
            "asn1 verify should reject a tampered signature");

        // A tampered message is rejected.
        let mut bad_msg = msg.to_vec();
        bad_msg[0] ^= 0x01;
        assert!(!verify_p256_sha256_asn1(&pubkey, &bad_msg, &sig),
            "asn1 verify should reject a tampered message");

        // A tampered public key is rejected.
        let mut bad_key = pubkey.clone();
        bad_key[1] ^= 0x01;
        assert!(!verify_p256_sha256_asn1(&bad_key, msg, &sig),
            "asn1 verify should reject a wrong public key");

        // Wrong-length and empty inputs must fail gracefully, not panic.
        assert!(!verify_p256_sha256_asn1(&pubkey[..64], msg, &sig),
            "asn1 verify should reject a short public key");
        assert!(!verify_p256_sha256_asn1(&pubkey, msg, &[]),
            "asn1 verify should reject an empty signature");
        assert!(!verify_p256_sha256_asn1(&pubkey, msg, &sig[..2]),
            "asn1 verify should reject a truncated DER signature");

        Ok(())
    }

    /// A loaded key's public point must be the one that is actually in the DER. The expected `x`
    /// and `y` below were derived from the same DER by `openssl` (see the documentation on
    /// `TEST_P256_PKCS8`), so this checks the loader against a tool that is not us.
    #[test]
    fn test_p256_keypair_public_key_matches_openssl_00() -> Outcome<()> {
        let kp = res!(P256KeyPair::from_pkcs8(&crate::acme::jose::TEST_P256_PKCS8));
        let pk = kp.public_key();
        assert_eq!(pk.len(), 65);
        assert_eq!(pk[0], 0x04);
        assert_eq!(
            base64url_encode(&pk[1..33]),
            "cMAYIYJu7A2aNTTrurSWBFMwr8uyVRYGvrrgsUz8I6Q",
        );
        assert_eq!(
            base64url_encode(&pk[33..65]),
            "Ktqy2hcvjIy_FofO47MfWeHLgjN7Vdxw0Bp2MRQyG8Y",
        );
        Ok(())
    }

    /// What this crate signs, this crate verifies -- in the encodings a browser uses. The signature
    /// must be the 64-byte fixed form, and a tampered message must fail.
    #[test]
    fn test_p256_keypair_sign_verifies_00() -> Outcome<()> {
        let kp = res!(P256KeyPair::generate());
        let msg = b"verify:sid-of-the-oxedation";
        let sig = res!(kp.sign(msg));
        assert_eq!(sig.len(), 64, "the fixed form is 64 bytes of r || s");
        assert!(verify_p256_sha256_fixed(&kp.public_key(), msg, &sig),
            "a freshly-signed message must verify");
        assert!(!verify_p256_sha256_fixed(&kp.public_key(), b"verify:another-sid", &sig),
            "the signature must not verify over a different message");
        Ok(())
    }

    /// A key written to disk and read back is the same key: same public point, and signatures made
    /// after the reload still verify.
    #[test]
    fn test_p256_keypair_pkcs8_round_trip_00() -> Outcome<()> {
        let kp = res!(P256KeyPair::generate());
        let reloaded = res!(P256KeyPair::from_pkcs8(kp.pkcs8_bytes()));
        assert_eq!(kp.public_key(), reloaded.public_key());
        let msg = b"a message signed after the reload";
        let sig = res!(reloaded.sign(msg));
        assert!(verify_p256_sha256_fixed(&kp.public_key(), msg, &sig),
            "the reloaded key must be the same key");
        Ok(())
    }

    /// Bytes that are not a P-256 PKCS#8 document must be refused, with an error rather than a
    /// panic.
    #[test]
    fn test_p256_keypair_rejects_junk_pkcs8_00() -> Outcome<()> {
        assert!(P256KeyPair::from_pkcs8(b"not a key at all").is_err(),
            "junk PKCS#8 must be refused");
        Ok(())
    }
}
