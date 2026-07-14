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
    /// The required members of an EC key are `crv`, `kty`, `x` and `y`. They
    /// are handed to [`jwk_thumbprint_sha256_of`] deliberately out of
    /// lexicographic order, so that the canonicalisation -- not the caller --
    /// is what puts them in the order RFC 7638 §3 mandates.
    pub fn jwk_thumbprint_sha256(&self) -> Outcome<[u8; 32]> {
        let (x, y) = res!(self.public_key_xy());
        let x_b64 = base64url_encode(&x);
        let y_b64 = base64url_encode(&y);
        jwk_thumbprint_sha256_of(&[
            ("kty", "EC"),
            ("x",   &x_b64),
            ("y",   &y_b64),
            ("crv", "P-256"),
        ])
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

/// Build the RFC 7638 §3 canonical JSON form of a JWK from its required
/// members.
///
/// The canonical form is defined as an exact byte sequence, not merely a
/// structurally equivalent JSON document: the **required** members only (for
/// an EC key `crv`, `kty`, `x` and `y`; for an RSA key `e`, `kty` and `n`),
/// lexicographically ordered by member name, with no whitespace and no line
/// breaks. A CA recomputes this string from the JWK we send it and hashes the
/// result, so a single stray space or a reordered member silently invalidates
/// every challenge we ever answer.
///
/// The members are sorted here rather than trusted from the caller, and the
/// string is assembled by hand rather than handed to a general-purpose JSON
/// serialiser, because no serialiser guarantees the byte-exactness the RFC
/// requires. Members are taken as `(name, value)` pairs because every required
/// member of every key type RFC 7638 covers has a string value.
pub fn jwk_canonical_string(members: &[(&str, &str)]) -> Outcome<String> {
    if members.is_empty() {
        return Err(err!(
            "An RFC 7638 canonical JWK needs at least one required member, \
            none were supplied.";
            Invalid, Input, Missing));
    }
    let mut sorted = members.to_vec();
    // Lexicographic order by member name. Rust's `str` ordering compares by
    // byte, which for UTF-8 is equivalent to ordering by code point, and the
    // member names RFC 7638 defines are all ASCII in any case.
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for pair in sorted.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(err!(
                "Duplicate JWK member name {:?}; the canonical form would \
                silently drop one of them.", pair[0].0;
                Invalid, Input, Conflict));
        }
    }
    let mut out = String::from("{");
    for (i, (name, value)) in sorted.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(name));
        out.push_str("\":\"");
        out.push_str(&json_escape(value));
        out.push('"');
    }
    out.push('}');
    Ok(out)
}

/// Compute the RFC 7638 §3 SHA-256 thumbprint of a JWK given its required
/// members, which are canonicalised by [`jwk_canonical_string`] first.
pub fn jwk_thumbprint_sha256_of(members: &[(&str, &str)]) -> Outcome<[u8; 32]> {
    let canonical = res!(jwk_canonical_string(members));
    Ok(sha256(canonical.as_bytes()))
}

/// Escape a string for inclusion in a JSON string literal, per RFC 8259 §7.
///
/// Base64url payloads and the short ASCII tokens RFC 7638 uses as member
/// names never need escaping, so in practice this is the identity function;
/// it exists so that the canonical form stays valid JSON no matter what it is
/// handed.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'             => out.push_str("\\\""),
            '\\'            => out.push_str("\\\\"),
            '\n'            => out.push_str("\\n"),
            '\r'            => out.push_str("\\r"),
            '\t'            => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&fmt!("\\u{:04x}", c as u32)),
            c               => out.push(c),
        }
    }
    out
}

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
// │ TEST VECTORS                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// A fixed P-256 account key in PKCS#8, used to pin the EC thumbprint path
/// against an externally-derived expected value. Shared with the
/// [`crate::acme::rfc8555`] tests so the key authorisation and dns-01 vectors
/// chain off the very same key.
///
/// Generated once, outside this crate, with:
///
/// ```text
/// openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 \
///     -outform DER -out k.der
/// openssl pkcs8 -topk8 -nocrypt -inform DER -in k.der -outform DER
/// ```
///
/// This is a throwaway test key and guards nothing.
#[cfg(test)]
pub(crate) const TEST_P256_PKCS8: [u8; 138] = [
    0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86,
    0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d,
    0x03, 0x01, 0x07, 0x04, 0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
    0x36, 0x69, 0x61, 0xe3, 0x3c, 0xdb, 0xf0, 0x14, 0x7f, 0xc3, 0xc0, 0x0c,
    0x8b, 0xea, 0xfd, 0xa5, 0xa4, 0x6d, 0x21, 0xfa, 0xed, 0xa2, 0x06, 0x98,
    0x8a, 0x36, 0xc5, 0xc2, 0xa8, 0x87, 0xc3, 0x39, 0xa1, 0x44, 0x03, 0x42,
    0x00, 0x04, 0x70, 0xc0, 0x18, 0x21, 0x82, 0x6e, 0xec, 0x0d, 0x9a, 0x35,
    0x34, 0xeb, 0xba, 0xb4, 0x96, 0x04, 0x53, 0x30, 0xaf, 0xcb, 0xb2, 0x55,
    0x16, 0x06, 0xbe, 0xba, 0xe0, 0xb1, 0x4c, 0xfc, 0x23, 0xa4, 0x2a, 0xda,
    0xb2, 0xda, 0x17, 0x2f, 0x8c, 0x8c, 0xbf, 0x16, 0x87, 0xce, 0xe3, 0xb3,
    0x1f, 0x59, 0xe1, 0xcb, 0x82, 0x33, 0x7b, 0x55, 0xdc, 0x70, 0xd0, 0x1a,
    0x76, 0x31, 0x14, 0x32, 0x1b, 0xc6,
];

/// The RFC 7638 §3 thumbprint of [`TEST_P256_PKCS8`], base64url-encoded.
///
/// Derived **independently of this crate**, from the DER above. The public
/// point is the trailing 65 bytes of the SubjectPublicKeyInfo
/// (`04 || X || Y`), giving:
///
/// ```text
/// x = cMAYIYJu7A2aNTTrurSWBFMwr8uyVRYGvrrgsUz8I6Q
/// y = Ktqy2hcvjIy_FofO47MfWeHLgjN7Vdxw0Bp2MRQyG8Y
/// ```
///
/// and therefore the RFC 7638 canonical string
///
/// ```text
/// {"crv":"P-256","kty":"EC","x":"cMAYIYJu7A2aNTTrurSWBFMwr8uyVRYGvrrgsUz8I6Q","y":"Ktqy2hcvjIy_FofO47MfWeHLgjN7Vdxw0Bp2MRQyG8Y"}
/// ```
///
/// whose SHA-256, base64url-encoded without padding, is the value below.
/// Re-derive with `openssl` and `python3`:
///
/// ```text
/// openssl pkey -inform DER -in k8.der -pubout -outform DER -out pub.der
/// python3 -c '
/// import hashlib, base64
/// p = open("pub.der","rb").read()[-65:]
/// b = lambda v: base64.urlsafe_b64encode(v).rstrip(b"=").decode()
/// c = chr(123)+chr(34)+"crv"+chr(34)+":"+chr(34)+"P-256"+chr(34)+","+chr(34)+"kty"+chr(34)+":"+chr(34)+"EC"+chr(34)+","+chr(34)+"x"+chr(34)+":"+chr(34)+b(p[1:33])+chr(34)+","+chr(34)+"y"+chr(34)+":"+chr(34)+b(p[33:65])+chr(34)+chr(125)
/// print(b(hashlib.sha256(c.encode()).digest()))'
/// ```
#[cfg(test)]
pub(crate) const TEST_P256_THUMBPRINT_B64: &str =
    "rIV82OX7WtoQ9t9CvXXciOOey0zuRuaonj8p-bQghoA";


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

    // ---- RFC 7638 external oracles ---------------------------------------

    /// The RSA key from the RFC 7638 §3.1 worked example. `e`, `kty` and `n`
    /// are the required members of an RSA JWK; everything else in the RFC's
    /// example JWK (`alg`, `kid`, `use`) is excluded from the canonical form
    /// by §3, and this vector is precisely what proves we exclude them.
    const RFC7638_RSA_N: &str = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4\
        cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5Js\
        GY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZg\
        nYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lF\
        d2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";

    /// The canonical string RFC 7638 §3.1 prints for that key, byte for byte.
    const RFC7638_RSA_CANONICAL: &str = "{\"e\":\"AQAB\",\"kty\":\"RSA\",\"n\":\"0vx7\
        agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1\
        L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6C\
        f0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajr\
        n1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw\
        0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw\"}";

    /// The thumbprint RFC 7638 §3.1 publishes for that key.
    const RFC7638_RSA_THUMBPRINT_B64: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";

    /// **External oracle, RFC 7638 §3.1.** The canonical string we build for
    /// the RFC's own RSA example must equal the one the RFC prints, byte for
    /// byte. This is the test that a self-consistently wrong implementation
    /// cannot pass: reordering the members, inserting whitespace, or padding
    /// the base64 all change these bytes, while leaving a determinism test
    /// perfectly happy.
    ///
    /// Note the members are handed over deliberately unsorted, so the
    /// canonicaliser -- not this test -- is what establishes the order.
    #[test]
    fn test_rfc7638_rsa_canonical_string_matches_the_rfc() -> Outcome<()> {
        let canonical = res!(jwk_canonical_string(&[
            ("n",   RFC7638_RSA_N),
            ("kty", "RSA"),
            ("e",   "AQAB"),
        ]));
        if canonical != RFC7638_RSA_CANONICAL {
            return Err(err!(
                "RFC 7638 §3.1 canonical string mismatch.\n  ours: {}\n  rfc:  {}",
                canonical, RFC7638_RSA_CANONICAL;
                Test, Mismatch));
        }
        Ok(())
    }

    /// **External oracle, RFC 7638 §3.1.** The SHA-256 thumbprint of the
    /// RFC's RSA example must equal the value the RFC publishes.
    #[test]
    fn test_rfc7638_rsa_thumbprint_matches_the_rfc() -> Outcome<()> {
        let tp = res!(jwk_thumbprint_sha256_of(&[
            ("n",   RFC7638_RSA_N),
            ("kty", "RSA"),
            ("e",   "AQAB"),
        ]));
        let got = base64url_encode(&tp);
        if got != RFC7638_RSA_THUMBPRINT_B64 {
            return Err(err!(
                "RFC 7638 §3.1 thumbprint mismatch: got {:?}, RFC publishes {:?}.",
                got, RFC7638_RSA_THUMBPRINT_B64;
                Test, Mismatch));
        }
        Ok(())
    }

    /// The canonicaliser must sort by member name regardless of the order it
    /// is given them in, and must emit no whitespace whatsoever.
    #[test]
    fn test_jwk_canonical_string_sorts_and_omits_whitespace() -> Outcome<()> {
        let canonical = res!(jwk_canonical_string(&[
            ("y",   "YY"),
            ("kty", "EC"),
            ("crv", "P-256"),
            ("x",   "XX"),
        ]));
        let expected = "{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"XX\",\"y\":\"YY\"}";
        if canonical != expected {
            return Err(err!(
                "Canonical string was {:?}, expected {:?}.", canonical, expected;
                Test, Mismatch));
        }
        if canonical.contains(' ') || canonical.contains('\n') || canonical.contains('\t') {
            return Err(err!(
                "Canonical string contains whitespace: {:?}.", canonical;
                Test, Invalid));
        }
        Ok(())
    }

    /// A duplicate member name would silently drop data from the hash input,
    /// so it must be refused rather than canonicalised.
    #[test]
    fn test_jwk_canonical_string_rejects_duplicate_members() -> Outcome<()> {
        match jwk_canonical_string(&[("kty", "EC"), ("kty", "RSA")]) {
            Ok(s) => Err(err!(
                "Duplicate member name was accepted, producing {:?}.", s;
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    /// **External oracle, EC path.** The thumbprint of the fixed P-256 key in
    /// [`TEST_P256_PKCS8`], computed through the real production path
    /// (`from_pkcs8` -> `jwk_thumbprint_sha256`), must equal the value derived
    /// independently with `openssl` and `python3`. See the doc comment on
    /// [`TEST_P256_THUMBPRINT_B64`] for the derivation.
    #[test]
    fn test_ec_thumbprint_matches_external_oracle() -> Outcome<()> {
        let signer = res!(JwsSigner::from_pkcs8(&TEST_P256_PKCS8));
        let tp = res!(signer.jwk_thumbprint_sha256());
        let got = base64url_encode(&tp);
        if got != TEST_P256_THUMBPRINT_B64 {
            return Err(err!(
                "EC thumbprint for the pinned P-256 key was {:?}, but the \
                externally-derived value is {:?}.", got, TEST_P256_THUMBPRINT_B64;
                Test, Mismatch));
        }
        Ok(())
    }

    /// The JWK the signer publishes and the JWK the thumbprint is taken over
    /// must describe the same key: a CA recomputes the thumbprint from the
    /// `jwk` header we send, so any drift between the two breaks every
    /// challenge. Pins both against the same external oracle.
    #[test]
    fn test_public_jwk_agrees_with_thumbprint_input() -> Outcome<()> {
        let signer = res!(JwsSigner::from_pkcs8(&TEST_P256_PKCS8));
        let jwk = res!(signer.public_jwk());
        let kty = res!(map_get_str(&jwk, "kty"));
        let crv = res!(map_get_str(&jwk, "crv"));
        let x   = res!(map_get_str(&jwk, "x"));
        let y   = res!(map_get_str(&jwk, "y"));

        // Rebuild the thumbprint from the *published* JWK members alone.
        let tp = res!(jwk_thumbprint_sha256_of(&[
            ("kty", &kty),
            ("crv", &crv),
            ("x",   &x),
            ("y",   &y),
        ]));
        let got = base64url_encode(&tp);
        if got != TEST_P256_THUMBPRINT_B64 {
            return Err(err!(
                "Thumbprint taken over the published JWK is {:?}, but the \
                externally-derived value is {:?}.", got, TEST_P256_THUMBPRINT_B64;
                Test, Mismatch));
        }
        Ok(())
    }

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
