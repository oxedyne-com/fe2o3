//! JSON Web Signature (JWS) primitives for ACME, using ES256.
//!
//! ACME RFC 8555 §6.2 requires every authenticated request to the directory to
//! be wrapped in a JWS in the flattened JSON serialisation form defined by RFC
//! 7515 §7.2.2. Let's Encrypt and other common CAs accept account keys signed
//! with ES256 (ECDSA using P-256 and SHA-256), which is what this module
//! implements.
//!
//! The module deliberately does not know anything ACME-specific. It exposes a
//! [`JwsSigner`] that:
//!
//! - Generates or loads an ES256 key pair.
//! - Returns its PKCS#8 bytes for on-disk persistence via [`JwsSigner::pkcs8_bytes`].
//! - Exposes the public key as a JWK in [`JwsSigner::public_jwk`].
//! - Computes the RFC 7638 §3 thumbprint of the public JWK via
//!   [`JwsSigner::jwk_thumbprint_sha256`].
//! - Signs a caller-supplied protected header and payload via
//!   [`JwsSigner::sign_flattened`], returning the resulting flattened JWS as a
//!   `Dat::Map` ready to be serialised for an HTTP request body.
//!
//! Callers of this module -- the higher-level ACME client -- build the
//! protected header themselves (filling in `alg`, `url`, `nonce` and either
//! `jwk` or `kid`), pass it in, and receive the signed structure back.
//!
//! The signature is produced by `ring` using
//! `ECDSA_P256_SHA256_FIXED_SIGNING`, which emits the IEEE P1363 fixed-width
//! form (64 bytes: `r || s`) that JWS requires. No ASN.1 to P1363 conversion
//! is needed on our side.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;

use base64;
use ring::{
    digest::{
        Context,
        SHA256,
    },
    rand::SystemRandom,
    signature::{
        EcdsaKeyPair,
        KeyPair,
        ECDSA_P256_SHA256_FIXED_SIGNING,
    },
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ JWS SIGNER                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// An ES256 JWS signer.
///
/// Holds a live `ring::signature::EcdsaKeyPair` for signing plus a retained
/// copy of the original PKCS#8 bytes so the key can be persisted to disk and
/// reloaded via [`JwsSigner::from_pkcs8`]. `ring` consumes the PKCS#8 bytes
/// during load and does not expose them afterwards, so we keep our own copy.
pub struct JwsSigner {
    /// PKCS#8 serialised private key, retained for persistence.
    pkcs8:      Vec<u8>,
    /// Live ECDSA P-256 signing handle, tied to `ring`.
    key_pair:   EcdsaKeyPair,
    /// RNG used for the non-deterministic part of ECDSA signing.
    rng:        SystemRandom,
}

impl fmt::Debug for JwsSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwsSigner")
            .field("pkcs8",     &"<redacted>")
            .field("key_pair",  &"<redacted>")
            .field("rng",       &"SystemRandom")
            .finish()
    }
}

impl JwsSigner {

    /// Generate a fresh ES256 key pair.
    pub fn new_es256() -> Outcome<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = match EcdsaKeyPair::generate_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &rng,
        ) {
            Ok(doc) => doc.as_ref().to_vec(),
            Err(_) => return Err(err!(
                "ring::signature::EcdsaKeyPair::generate_pkcs8 failed to \
                produce a fresh ES256 key pair.";
                Init, Unknown)),
        };
        let key_pair = match EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &pkcs8,
            &rng,
        ) {
            Ok(kp) => kp,
            Err(e) => return Err(err!(
                "ring::signature::EcdsaKeyPair::from_pkcs8 rejected the \
                freshly-generated PKCS#8 document: {}.", e;
                Init, Invalid)),
        };
        Ok(Self {
            pkcs8,
            key_pair,
            rng,
        })
    }

    /// Load an existing ES256 key pair from its PKCS#8 encoding.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Outcome<Self> {
        let rng = SystemRandom::new();
        let key_pair = match EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &rng,
        ) {
            Ok(kp) => kp,
            Err(e) => return Err(err!(
                "ring::signature::EcdsaKeyPair::from_pkcs8 rejected the \
                supplied PKCS#8 bytes: {}.", e;
                Init, Invalid, Input)),
        };
        Ok(Self {
            pkcs8:  pkcs8.to_vec(),
            key_pair,
            rng,
        })
    }

    /// Return the retained PKCS#8 serialisation of the private key for on-disk
    /// persistence. The bytes round-trip through [`JwsSigner::from_pkcs8`].
    pub fn pkcs8_bytes(&self) -> &[u8] {
        &self.pkcs8
    }

    /// Return the public key as a JWK, shaped as a `Dat::Map` with the keys
    /// `kty`, `crv`, `x` and `y`.
    ///
    /// The resulting `Dat` can be embedded directly into a JWS protected
    /// header when a new ACME account is being registered (RFC 8555 §6.2
    /// requires `jwk` on those requests; authenticated follow-up requests use
    /// `kid` instead).
    pub fn public_jwk(&self) -> Outcome<Dat> {
        let (x, y) = res!(self.public_key_xy());
        Ok(mapdat!{
            "kty" => "EC",
            "crv" => "P-256",
            "x"   => base64url_encode(&x),
            "y"   => base64url_encode(&y),
        })
    }

    /// Compute the RFC 7638 §3 thumbprint of the public JWK using SHA-256.
    ///
    /// The canonical input form for a P-256 EC key is byte-exact:
    ///
    /// ```text
    /// {"crv":"P-256","kty":"EC","x":"<b64url-x>","y":"<b64url-y>"}
    /// ```
    ///
    /// with the required members in lexicographic order and no whitespace.
    /// We construct that string by hand rather than relying on a generic JSON
    /// serialiser because the RFC defines the canonical form in terms of the
    /// exact byte sequence, not a structural equivalent.
    pub fn jwk_thumbprint_sha256(&self) -> Outcome<[u8; 32]> {
        let (x, y) = res!(self.public_key_xy());
        let canonical = fmt!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            base64url_encode(&x),
            base64url_encode(&y),
        );
        Ok(sha256(canonical.as_bytes()))
    }

    /// Produce a JWS in the flattened JSON serialisation form defined in RFC
    /// 7515 §7.2.2.
    ///
    /// `protected_header` must be a `Dat::Map` holding the ACME-required
    /// fields (`alg`, `url`, `nonce`, and either `jwk` or `kid`). This
    /// function serialises it to compact JSON, base64url-encodes it, and signs
    /// the `"<b64 header>.<b64 payload>"` signing input per RFC 7515 §5.1.
    ///
    /// `payload_bytes` may be empty, which ACME RFC 8555 §6.3 uses to mark a
    /// request as "POST-as-GET".
    ///
    /// The returned `Dat::Map` has the keys `protected`, `payload` and
    /// `signature`, each a base64url-encoded string, and is ready to be
    /// serialised into the HTTP request body with `.json()`.
    pub fn sign_flattened(
        &self,
        protected_header:   &Dat,
        payload_bytes:      &[u8],
    )
        -> Outcome<Dat>
    {
        let header_json = res!(protected_header.json());
        let header_b64  = base64url_encode(header_json.as_bytes());
        let payload_b64 = base64url_encode(payload_bytes);
        let signing_input = fmt!("{}.{}", header_b64, payload_b64);
        let sig = match self.key_pair.sign(&self.rng, signing_input.as_bytes()) {
            Ok(s) => s,
            Err(_) => return Err(err!(
                "ring::signature::EcdsaKeyPair::sign failed to produce an \
                ES256 signature for the JWS signing input.";
                Unknown)),
        };
        let sig_b64 = base64url_encode(sig.as_ref());
        Ok(mapdat!{
            "protected" => header_b64,
            "payload"   => payload_b64,
            "signature" => sig_b64,
        })
    }

    /// Extract the raw `(x, y)` coordinates of the public key. Each is 32 bytes.
    ///
    /// `ring` exposes the public key as an uncompressed SEC1 point
    /// `0x04 || x || y`, a total of 65 bytes for P-256. We split and validate
    /// the shape here so callers can treat the coordinates as plain byte arrays.
    fn public_key_xy(&self) -> Outcome<([u8; 32], [u8; 32])> {
        let pk = self.key_pair.public_key().as_ref();
        if pk.len() != 65 || pk[0] != 0x04 {
            return Err(err!(
                "ring::signature::EcdsaKeyPair::public_key returned a \
                representation that is not a 65-byte uncompressed SEC1 \
                point (got {} bytes, leading byte {:#04x}).",
                pk.len(), pk[0];
                Invalid, Size, Unknown));
        }
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(&pk[1..33]);
        y.copy_from_slice(&pk[33..65]);
        Ok((x, y))
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// URL-safe base64 without padding, as required by RFC 7515 §2.
pub fn base64url_encode(bytes: &[u8]) -> String {
    base64::encode_config(bytes, base64::URL_SAFE_NO_PAD)
}

/// URL-safe base64 decoder that tolerates missing padding.
pub fn base64url_decode(s: &str) -> Outcome<Vec<u8>> {
    match base64::decode_config(s, base64::URL_SAFE_NO_PAD) {
        Ok(v) => Ok(v),
        Err(e) => Err(err!(e,
            "Failed to decode {:?} as URL-safe base64 without padding.", s;
            Invalid, Input, Decode)),
    }
}

/// SHA-256 hash of the input, returned as a 32-byte array.
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let digest = ctx.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    use ring::signature::{
        UnparsedPublicKey,
        ECDSA_P256_SHA256_FIXED,
    };

    /// Pull one string-valued entry out of a `Dat::Map` by key, returning an
    /// `Outcome` error on absence or type mismatch. Used by the tests below.
    fn map_get_str(dat: &Dat, key: &str) -> Outcome<String> {
        match dat {
            Dat::Map(m) => match m.get(&dat!(key.to_string())) {
                Some(Dat::Str(s)) => Ok(s.clone()),
                Some(other) => Err(err!(
                    "Expected Dat::Str at key {:?}, found {:?}.", key, other;
                    Invalid, Mismatch)),
                None => Err(err!(
                    "Missing key {:?} in Dat::Map.", key;
                    Missing, Input)),
            },
            _ => Err(err!(
                "Expected Dat::Map, found {:?}.", dat;
                Invalid, Mismatch)),
        }
    }

    /// Generate a fresh signer, produce a flattened JWS, then verify the
    /// signature against the signer's own public key via `ring`. End-to-end
    /// correctness of sign + serialise.
    #[test]
    fn test_sign_verify_round_trip() -> Outcome<()> {
        let signer = res!(JwsSigner::new_es256());
        let header = mapdat!{
            "alg"   => "ES256",
            "nonce" => "deadbeef",
            "url"   => "https://example.test/acme/new-order",
        };
        let payload = b"{\"identifiers\":[{\"type\":\"dns\",\"value\":\"example.test\"}]}";

        let jws = res!(signer.sign_flattened(&header, payload));

        // Recover the signing input the way a verifier would.
        let prot_b64 = res!(map_get_str(&jws, "protected"));
        let load_b64 = res!(map_get_str(&jws, "payload"));
        let sig_b64  = res!(map_get_str(&jws, "signature"));
        let signing_input = fmt!("{}.{}", prot_b64, load_b64);
        let sig_bytes = res!(base64url_decode(&sig_b64));

        // Verify with ring directly, using the raw public key bytes.
        let pk_bytes = signer.key_pair.public_key().as_ref();
        let verifier = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pk_bytes);
        match verifier.verify(signing_input.as_bytes(), &sig_bytes) {
            Ok(()) => (),
            Err(_) => return Err(err!(
                "ring rejected a signature that JwsSigner::sign_flattened had \
                just produced for the same public key.";
                Test, Unknown)),
        }

        // Payload must round-trip through base64url.
        let decoded_payload = res!(base64url_decode(&load_b64));
        if decoded_payload != payload {
            return Err(err!(
                "JWS payload did not round-trip: expected {} bytes, got {}.",
                payload.len(), decoded_payload.len();
                Test, Mismatch));
        }
        Ok(())
    }

    /// Empty payload -- the RFC 8555 §6.3 "POST-as-GET" case -- must produce a
    /// valid JWS whose payload field is the empty base64url string.
    #[test]
    fn test_sign_post_as_get() -> Outcome<()> {
        let signer = res!(JwsSigner::new_es256());
        let header = mapdat!{
            "alg"   => "ES256",
            "nonce" => "nonceval",
            "url"   => "https://example.test/acme/order/1",
        };
        let jws = res!(signer.sign_flattened(&header, b""));
        let load_b64 = res!(map_get_str(&jws, "payload"));
        if !load_b64.is_empty() {
            return Err(err!(
                "POST-as-GET payload must be base64url-encoded empty bytes \
                (the empty string), got {:?}.", load_b64;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Generate, serialise to PKCS#8, reload, and verify that a signature
    /// produced by the reloaded signer verifies against the original signer's
    /// public key. Confirms the PKCS#8 round-trip preserves identity.
    #[test]
    fn test_pkcs8_round_trip() -> Outcome<()> {
        let first  = res!(JwsSigner::new_es256());
        let pkcs8  = first.pkcs8_bytes().to_vec();
        let second = res!(JwsSigner::from_pkcs8(&pkcs8));

        // Both signers should expose the same public key bytes.
        let pk1 = first.key_pair.public_key().as_ref().to_vec();
        let pk2 = second.key_pair.public_key().as_ref().to_vec();
        if pk1 != pk2 {
            return Err(err!(
                "PKCS#8 round-trip produced a signer with a different public \
                key (orig {} bytes, reload {} bytes).", pk1.len(), pk2.len();
                Test, Mismatch));
        }

        // And the retained pkcs8 bytes should be equal to what we loaded.
        if second.pkcs8_bytes() != pkcs8.as_slice() {
            return Err(err!(
                "PKCS#8 round-trip: retained bytes in reloaded signer \
                differ from the input.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// The JWK thumbprint must be deterministic and exactly 32 bytes wide.
    /// Two calls on the same signer must yield identical output.
    #[test]
    fn test_jwk_thumbprint_deterministic() -> Outcome<()> {
        let signer = res!(JwsSigner::new_es256());
        let t1 = res!(signer.jwk_thumbprint_sha256());
        let t2 = res!(signer.jwk_thumbprint_sha256());
        if t1 != t2 {
            return Err(err!(
                "JwsSigner::jwk_thumbprint_sha256 is not deterministic on \
                the same signer.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// `public_jwk` must return a `Dat::Map` with the four required ES256 JWK
    /// members, each of the expected type and shape.
    #[test]
    fn test_public_jwk_shape() -> Outcome<()> {
        let signer = res!(JwsSigner::new_es256());
        let jwk = res!(signer.public_jwk());
        let kty = res!(map_get_str(&jwk, "kty"));
        let crv = res!(map_get_str(&jwk, "crv"));
        let x   = res!(map_get_str(&jwk, "x"));
        let y   = res!(map_get_str(&jwk, "y"));
        if kty != "EC" {
            return Err(err!("Expected kty == \"EC\", got {:?}.", kty;
                Test, Mismatch));
        }
        if crv != "P-256" {
            return Err(err!("Expected crv == \"P-256\", got {:?}.", crv;
                Test, Mismatch));
        }
        // Base64url of a 32-byte coordinate is 43 characters when unpadded.
        if x.len() != 43 || y.len() != 43 {
            return Err(err!(
                "Expected base64url-encoded 32-byte coordinates (43 chars), \
                got x={} y={}.", x.len(), y.len();
                Test, Mismatch));
        }
        let x_bytes = res!(base64url_decode(&x));
        let y_bytes = res!(base64url_decode(&y));
        if x_bytes.len() != 32 || y_bytes.len() != 32 {
            return Err(err!(
                "Decoded JWK coordinates are not 32 bytes each: x={} y={}.",
                x_bytes.len(), y_bytes.len();
                Test, Size));
        }
        Ok(())
    }
}
