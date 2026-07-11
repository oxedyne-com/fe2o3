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

use oxedyne_fe2o3_core::prelude::*;

use ring::signature::{
    UnparsedPublicKey,
    ECDSA_P256_SHA256_FIXED,
};


/// Verify an ECDSA P-256 signature over `msg` under `pubkey`.
///
/// Returns `true` when the signature is valid. A public key or signature of
/// the wrong length, or an ill-formed point, simply fails to verify -- the
/// call never panics, since `ring` reports every malformed input as an
/// ordinary verification failure.
///
/// # Arguments
/// * `pubkey` -- the 65-byte uncompressed SEC1 point `0x04 || X || Y`
///   (WebCrypto `exportKey('raw')`).
/// * `msg` -- the raw message bytes as signed; NOT pre-hashed, as SHA-256 is
///   applied internally.
/// * `sig` -- the 64-byte fixed-length `r || s` signature (IEEE P1363, the
///   form WebCrypto ECDSA signing emits).
pub fn verify_p256_sha256_fixed(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pubkey);
    key.verify(msg, sig).is_ok()
}


#[cfg(test)]
mod tests {
    use super::*;

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
}
