//! ECDSA P-256 (NIST secp256r1) signature *verification*, pure Rust and
//! wasm-clean.
//!
//! # Why this exists, and why it is verify only
//!
//! A browser or phone signs with a key it will not export: a WebCrypto
//! `CryptoKey` minted `extractable: false`, or a Secure Enclave / Android
//! Keystore handle. The key never crosses into wasm, so signing and key
//! generation stay on the host side of the boundary and are not offered here.
//! What a downloaded wasm client *does* need is to *check* a signature on-device,
//! rather than trust a server to have checked it, and that is the one operation
//! this module provides.
//!
//! The only other P-256 in Hematite is `fe2o3_net::ecdsa`, a thin wrapper over
//! `ring`, which is native only and cannot run in a browser. This module is its
//! wasm-reachable peer: it rests on the RustCrypto `p256` crate, pure Rust with
//! no `getrandom` on the verify path, and so compiles to
//! `wasm32-unknown-unknown`. It is gated behind the crate's `p256` feature and
//! is off by default, leaving a native build unchanged.
//!
//! # Accepted encodings
//!
//! These match exactly what WebCrypto emits, so the same bytes a browser puts on
//! the wire verify here without reshaping:
//!
//! - Public key: the 65-byte uncompressed SEC1 point `0x04 || X || Y`, as
//!   `exportKey('raw')` yields for an ECDSA P-256 key.
//! - Signature: the 64-byte fixed-length `r || s` form (IEEE P1363), as
//!   `crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' })` emits.
//! - Message: the raw bytes as signed, NOT a digest. SHA-256 is applied within,
//!   via `fe2o3_hash`, so no second hasher is compiled onto this path.
//!
//! # What a false means
//!
//! A malformed input -- a key or signature of the wrong length, an off-curve or
//! identity point, an `r` or `s` outside `[1, n-1]` -- is a verification
//! *failure*, `Ok(false)`, not an error. That mirrors `fe2o3_net::ecdsa` and
//! lets a caller treat "this does not verify" uniformly, however the bytes went
//! wrong. Nothing here ever accepts what it cannot fully parse and check.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::sha256;

// Leading `::` so the crate `p256` is meant, not this module of the same name.
use ::p256::ecdsa::{
    Signature,
    VerifyingKey,
    signature::hazmat::PrehashVerifier,
};

/// Length of an uncompressed SEC1 P-256 point, `0x04 || X || Y`.
pub const P256_POINT_LEN: usize = 65;

/// Length of a raw `r || s` P-256 signature.
pub const P256_SIG_LEN: usize = 64;

/// Verify a P-256 / SHA-256 signature in the encodings WebCrypto emits.
///
/// `pubkey` is the 65-byte uncompressed SEC1 point, `sig` the 64-byte `r || s`,
/// and `msg` the raw message rather than a digest: SHA-256 is applied within,
/// matching WebCrypto's `hash: 'SHA-256'`. See the module header for how a
/// malformed input is reported.
pub fn verify_p256_sha256_fixed(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> Outcome<bool> {
    let digest = sha256::digest(msg);
    verify_p256_prehashed(pubkey, &digest, sig)
}

/// The prehash primitive underneath [`verify_p256_sha256_fixed`]: verify a
/// P-256 signature over a digest that has already been computed.
///
/// `digest` is the message digest as a big-endian byte string. It need not be 32
/// bytes: a digest shorter or longer than the curve's scalar field is reduced
/// per FIPS 186-4 / SEC1 (a short one is taken whole, a long one truncated to its
/// leftmost 256 bits), which is what lets a caller present, say, a SHA-1 or
/// SHA-512 digest and get the answer NIST's own vectors expect. The WebCrypto
/// path hands it a 32-byte SHA-256 digest. `pubkey` and `sig` are as for
/// [`verify_p256_sha256_fixed`].
pub fn verify_p256_prehashed(pubkey: &[u8], digest: &[u8], sig: &[u8]) -> Outcome<bool> {
    // A wrong width is a failure to verify, not an error; see the module header.
    if pubkey.len() != P256_POINT_LEN {
        return Ok(false);
    }
    if sig.len() != P256_SIG_LEN {
        return Ok(false);
    }
    // An off-curve or identity point, or an out-of-range r or s, parses as a
    // failure rather than propagating: the point never lands where a signature
    // could hold.
    let vk = match VerifyingKey::from_sec1_bytes(pubkey) {
        Ok(vk) => vk,
        Err(_) => return Ok(false),
    };
    let signature = match Signature::from_slice(sig) {
        Ok(sig) => sig,
        Err(_) => return Ok(false),
    };
    Ok(vk.verify_prehash(digest, &signature).is_ok())
}
