//! HMAC-SHA256 keyed message authentication.
//!
//! A thin wrapper over `ring::hmac` (the constant-time MAC already in
//! Hematite's dependency tree via this crate) exposing the two
//! operations downstream callers need: compute a tag, and verify a
//! tag in constant time. Webhook signature verification (e.g. Stripe's
//! `Stripe-Signature` scheme) is the motivating caller.
//!
//! Constant-time verification matters: comparing a computed tag against
//! an attacker-supplied one with an ordinary byte comparison leaks the
//! length of the matching prefix through timing, so verification must
//! use `ring::hmac::verify`, never `==`.

use oxedyne_fe2o3_core::prelude::*;

use ring::hmac as ring_hmac;


/// Compute the HMAC-SHA256 tag of `msg` under `key`.
///
/// Returns the 32-byte tag. The operation is infallible for any key
/// and message length, so no `Outcome` wrapper is needed.
///
/// # Arguments
/// * `key` -- the secret key bytes (any length; `ring` handles the
///   RFC 2104 key padding/hashing internally).
/// * `msg` -- the message to authenticate.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let k = ring_hmac::Key::new(ring_hmac::HMAC_SHA256, key);
    let tag = ring_hmac::sign(&k, msg);
    // The SHA-256 tag is always 32 bytes; copy it into a fixed array.
    let mut out = [0u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

/// Verify, in constant time, that `tag` is the HMAC-SHA256 of `msg`
/// under `key`.
///
/// Returns `true` when the tag matches. Uses `ring`'s constant-time
/// comparison so no timing side channel reveals how much of the tag
/// was correct. A `tag` of the wrong length simply fails to verify.
///
/// # Arguments
/// * `key` -- the secret key bytes.
/// * `msg` -- the authenticated message.
/// * `tag` -- the candidate tag to check against a freshly computed one.
pub fn verify_hmac_sha256(key: &[u8], msg: &[u8], tag: &[u8]) -> bool {
    let k = ring_hmac::Key::new(ring_hmac::HMAC_SHA256, key);
    ring_hmac::verify(&k, msg, tag).is_ok()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Decode a lowercase hex string into bytes for test vectors.
    fn from_hex(s: &str) -> Vec<u8> {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 2);
        let val = |c: u8| -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _           => 0,
            }
        };
        let mut i = 0;
        while i + 1 < bytes.len() {
            out.push((val(bytes[i]) << 4) | val(bytes[i + 1]));
            i += 2;
        }
        out
    }

    /// RFC 4231 test case 2: a known HMAC-SHA256 vector.
    ///
    /// Key = "Jefe", Data = "what do ya want for nothing?",
    /// expected tag =
    /// 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843.
    #[test]
    fn test_rfc4231_case2() {
        let key = b"Jefe";
        let msg = b"what do ya want for nothing?";
        let expected = from_hex(
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");

        let tag = hmac_sha256(key, msg);
        assert_eq!(&tag[..], &expected[..], "HMAC-SHA256 tag mismatch");

        // Constant-time verification of the same vector.
        assert!(verify_hmac_sha256(key, msg, &expected),
            "verify should accept the correct tag");

        // A tampered tag must be rejected.
        let mut bad = expected.clone();
        bad[0] ^= 0x01;
        assert!(!verify_hmac_sha256(key, msg, &bad),
            "verify should reject a tampered tag");

        // A tag of the wrong length must be rejected, not panic.
        assert!(!verify_hmac_sha256(key, msg, &expected[..16]),
            "verify should reject a short tag");
    }

    /// RFC 4231 test case 1: 20-byte 0x0b key, Data = "Hi There".
    #[test]
    fn test_rfc4231_case1() {
        let key = [0x0bu8; 20];
        let msg = b"Hi There";
        let expected = from_hex(
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        let tag = hmac_sha256(&key, msg);
        assert_eq!(&tag[..], &expected[..], "HMAC-SHA256 case 1 mismatch");
    }
}
