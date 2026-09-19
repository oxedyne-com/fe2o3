//! P-256 verification, held to two oracles it does not itself define.
//!
//! - The NIST CAVP 186-4 SigVer vectors (`tests/data/ecdsa_verify_tests.txt`),
//!   the standards body's own accept/reject verdicts.
//! - `ring`, differentially: thousands of random signatures, sound and mutated,
//!   that must earn the same verdict from this crate as from `ring`. `ring` is
//!   the backend of `fe2o3_net::ecdsa`, so this is that module's verdict without
//!   the dependency cycle its crate would create.

#![cfg(feature = "p256")]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::p256::{
    P256_POINT_LEN,
    P256_SIG_LEN,
    verify_p256_prehashed,
    verify_p256_sha256_fixed,
};

/// Decodes a hex string from a vector file into bytes.
fn unhex(s: &str) -> Outcome<Vec<u8>> {
    match hex::decode(s) {
        Ok(v) => Ok(v),
        Err(e) => Err(err!("Cannot decode hex '{}': {:?}.", s, e; Invalid, Input)),
    }
}

/// One P-256 case lifted from the CAVP file.
struct Case {
    x:          Vec<u8>,    // public point abscissa
    y:          Vec<u8>,    // public point ordinate
    digest:     Vec<u8>,    // message digest, any hash width
    r:          Vec<u8>,    // signature r
    s:          Vec<u8>,    // signature s
    invalid:    bool,       // carries `Invalid = Y`
}

impl Case {
    /// The 65-byte uncompressed SEC1 public key, `0x04 || X || Y`.
    fn pubkey(&self) -> Vec<u8> {
        let mut pk = Vec::with_capacity(P256_POINT_LEN);
        pk.push(0x04);
        pk.extend_from_slice(&self.x);
        pk.extend_from_slice(&self.y);
        pk
    }

    /// The 64-byte fixed `r || s` signature.
    fn sig(&self) -> Vec<u8> {
        let mut sig = Vec::with_capacity(P256_SIG_LEN);
        sig.extend_from_slice(&self.r);
        sig.extend_from_slice(&self.s);
        sig
    }
}

/// Reads the `Curve = P-256` blocks out of the CAVP file.
///
/// A blank line ends a block. Only P-256 blocks are returned; the file's P-384
/// blocks are for a curve this crate does not carry.
fn read_p256_cases() -> Outcome<Vec<Case>> {
    let path = format!("{}/tests/data/ecdsa_verify_tests.txt", env!("CARGO_MANIFEST_DIR"));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Err(err!("Cannot read the CAVP vector file '{}': {}.", path, e; IO, Read)),
    };

    let mut cases = Vec::new();
    let mut curve:  Option<String>  = None;
    let mut x:      Option<Vec<u8>> = None;
    let mut y:      Option<Vec<u8>> = None;
    let mut digest: Option<Vec<u8>> = None;
    let mut r:      Option<Vec<u8>> = None;
    let mut s:      Option<Vec<u8>> = None;
    let mut invalid = false;

    // Emits the block accumulated so far, if it is a complete P-256 one.
    fn flush(
        curve:      &mut Option<String>,
        x:          &mut Option<Vec<u8>>,
        y:          &mut Option<Vec<u8>>,
        digest:     &mut Option<Vec<u8>>,
        r:          &mut Option<Vec<u8>>,
        s:          &mut Option<Vec<u8>>,
        invalid:    &mut bool,
        cases:      &mut Vec<Case>,
    )
        -> Outcome<()>
    {
        if curve.as_deref() == Some("P-256") {
            match (x.take(), y.take(), digest.take(), r.take(), s.take()) {
                (Some(x), Some(y), Some(digest), Some(r), Some(s)) => {
                    cases.push(Case { x, y, digest, r, s, invalid: *invalid });
                },
                _ => return Err(err!(
                    "A P-256 block in the CAVP file is missing a field."; Invalid, Input)),
            }
        }
        *curve = None;
        *x = None;
        *y = None;
        *digest = None;
        *r = None;
        *s = None;
        *invalid = false;
        Ok(())
    }

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            res!(flush(&mut curve, &mut x, &mut y, &mut digest, &mut r, &mut s,
                &mut invalid, &mut cases));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((key, val)) => {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "Curve"     => curve = Some(val.to_string()),
                    "X"         => x = Some(res!(unhex(val))),
                    "Y"         => y = Some(res!(unhex(val))),
                    "Digest"    => digest = Some(res!(unhex(val))),
                    "R"         => r = Some(res!(unhex(val))),
                    "S"         => s = Some(res!(unhex(val))),
                    "Invalid"   => invalid = val == "Y",
                    _           => {}, // Other keys are not needed here.
                }
            },
            None => return Err(err!("Unparsable line in the CAVP file: '{}'.", line; Invalid, Input)),
        }
    }
    // The file may not end with a blank line.
    res!(flush(&mut curve, &mut x, &mut y, &mut digest, &mut r, &mut s,
        &mut invalid, &mut cases));

    Ok(cases)
}

/// Every NIST CAVP P-256 verification vector earns the verdict NIST records:
/// a sound signature verifies, an `Invalid = Y` one does not.
///
/// The digests in the file span SHA-1 through SHA-512, so this also exercises the
/// prehash reduction across widths either side of the 32-byte scalar field. The
/// tallies are asserted too, so a silent parsing fault that dropped or duplicated
/// cases cannot pass unnoticed.
#[test]
fn cavp_p256_sigver_vectors() -> Outcome<()> {
    let cases = res!(read_p256_cases());
    assert_eq!(cases.len(), 85, "expected 85 P-256 CAVP blocks");

    let mut valid = 0usize;
    let mut invalid = 0usize;
    for (i, case) in cases.iter().enumerate() {
        let got = res!(verify_p256_prehashed(&case.pubkey(), &case.digest, &case.sig()));
        let want = !case.invalid;
        assert_eq!(got, want,
            "CAVP P-256 case {} (invalid={}): verifier said {}", i, case.invalid, got);
        if case.invalid { invalid += 1; } else { valid += 1; }
    }
    assert_eq!(valid, 21, "expected 21 valid P-256 vectors");
    assert_eq!(invalid, 64, "expected 64 invalid P-256 vectors");
    Ok(())
}

/// A wrong-width key or signature is a graceful `Ok(false)`, never a panic and
/// never an error, matching `fe2o3_net::ecdsa`.
#[test]
fn malformed_inputs_fail_gracefully() -> Outcome<()> {
    let pk = vec![0x04u8; P256_POINT_LEN];
    let sig = vec![0x00u8; P256_SIG_LEN];
    let msg = b"anything";
    assert!(!res!(verify_p256_sha256_fixed(&pk[..64], msg, &sig)), "short key");
    assert!(!res!(verify_p256_sha256_fixed(&pk, msg, &sig[..63])), "short sig");
    assert!(!res!(verify_p256_sha256_fixed(&[], msg, &sig)), "empty key");
    assert!(!res!(verify_p256_sha256_fixed(&pk, msg, &[])), "empty sig");
    // A well-formed 0x04-tagged but off-curve point is a failure, not an error.
    assert!(!res!(verify_p256_sha256_fixed(&pk, msg, &sig)), "all-zero point is off curve");
    Ok(())
}

/// The strongest check: this crate and `ring` agree, signature by signature, over
/// thousands of random cases -- sound ones both accept, one-bit mutations of the
/// signature, message or key both reject.
///
/// Native only: it mints keys and signs with `ring`, which needs the operating
/// system's randomness and does not exist on wasm. The verify path under test
/// carries neither dependency.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn differential_against_ring() -> Outcome<()> {
    use rand::Rng;
    use rand::RngCore;
    use ring::{
        rand::SystemRandom,
        signature::{
            EcdsaKeyPair,
            KeyPair,
            UnparsedPublicKey,
            ECDSA_P256_SHA256_FIXED,
            ECDSA_P256_SHA256_FIXED_SIGNING,
        },
    };

    /// `ring`'s verdict, the oracle `fe2o3_net::ecdsa::verify_p256_sha256_fixed`
    /// wraps.
    fn ring_ok(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> bool {
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pubkey).verify(msg, sig).is_ok()
    }

    let sysrng = SystemRandom::new();
    let mut rng = rand::thread_rng();

    const ITERS: usize = 3000;
    let mut sound_checked = 0usize;
    let mut mutated_checked = 0usize;

    for iter in 0..ITERS {
        // A fresh key each round, so the set spans many keys, not many signatures
        // of one.
        let pkcs8 = match EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &sysrng) {
            Ok(doc) => doc,
            Err(e) => return Err(err!("ring keygen failed at iter {}: {}.", iter, e; Test, Init)),
        };
        let kp = match EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &sysrng) {
            Ok(kp) => kp,
            Err(e) => return Err(err!("ring key load failed at iter {}: {}.", iter, e; Test, Init)),
        };
        let pubkey = kp.public_key().as_ref().to_vec();

        // A random message, sometimes empty.
        let mlen = rng.gen_range(0..256usize);
        let mut msg = vec![0u8; mlen];
        rng.fill_bytes(&mut msg);

        let sig = match kp.sign(&sysrng, &msg) {
            Ok(s) => s.as_ref().to_vec(),
            Err(e) => return Err(err!("ring sign failed at iter {}: {}.", iter, e; Test, Data)),
        };

        // Sound: both accept, and this crate says true.
        let mine = res!(verify_p256_sha256_fixed(&pubkey, &msg, &sig));
        let theirs = ring_ok(&pubkey, &msg, &sig);
        assert!(theirs, "ring rejected its own signature at iter {}", iter);
        assert_eq!(mine, theirs, "sound case disagreement at iter {}", iter);
        assert!(mine, "this crate rejected a sound signature at iter {}", iter);
        sound_checked += 1;

        // Mutated: flip one bit in the signature, the message, or the key (past
        // the 0x04 tag), preserving length so the crypto, not a size check, is
        // what decides. Both verifiers must agree, and must reject.
        let (mut pk_m, mut msg_m, mut sig_m) = (pubkey.clone(), msg.clone(), sig.clone());
        match rng.gen_range(0..4u8) {
            0 => {
                // r half of the signature.
                let bit = rng.gen_range(0..(32 * 8));
                sig_m[bit / 8] ^= 1 << (bit % 8);
            },
            1 => {
                // s half of the signature.
                let bit = rng.gen_range(0..(32 * 8));
                sig_m[32 + bit / 8] ^= 1 << (bit % 8);
            },
            2 => {
                // A byte of the message; skip when the message is empty.
                if msg_m.is_empty() {
                    sig_m[0] ^= 0x01;
                } else {
                    let i = rng.gen_range(0..msg_m.len());
                    let bit = rng.gen_range(0..8u32);
                    msg_m[i] ^= 1 << bit;
                }
            },
            _ => {
                // The public point, keeping the 0x04 tag.
                let i = rng.gen_range(1..P256_POINT_LEN);
                let bit = rng.gen_range(0..8u32);
                pk_m[i] ^= 1 << bit;
            },
        }
        let mine_m = res!(verify_p256_sha256_fixed(&pk_m, &msg_m, &sig_m));
        let theirs_m = ring_ok(&pk_m, &msg_m, &sig_m);
        assert_eq!(mine_m, theirs_m,
            "mutated case disagreement at iter {}: mine={} ring={}", iter, mine_m, theirs_m);
        assert!(!mine_m, "a one-bit mutation still verified at iter {}", iter);
        mutated_checked += 1;
    }

    assert_eq!(sound_checked, ITERS);
    assert_eq!(mutated_checked, ITERS);
    Ok(())
}
