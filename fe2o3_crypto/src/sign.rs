use crate::keys::Keys;
// `pqc::dilithium` needs a mode feature to have a parameter set (see `pqc::mod`); the pure-Rust
// `Dilithium2_fe2o3` variant below needs it for the same reason.
#[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
use crate::pqc::dilithium as dilithium2_fe2o3;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::{
        BatchItem,
        Signer,
    },
};
// Used by the Dilithium arms of `verify_batch` (pq or a mode feature) and, unconditionally, by
// a test.
#[cfg(any(
    test,
    feature = "pq",
    feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3",
))]
use oxedyne_fe2o3_iop_crypto::sign::verify_each;
use oxedyne_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

use std::{
    collections::BTreeMap,
    convert::TryFrom,
    fmt::{
        self,
        Debug,
    },
    str,
};

#[cfg(feature = "batch")]
use curve25519_dalek::{
    constants::ED25519_BASEPOINT_POINT,
    traits::VartimeMultiscalarMul,
};
use curve25519_dalek::{
    edwards::{
        CompressedEdwardsY,
        EdwardsPoint,
    },
    scalar::Scalar,
    traits::IsIdentity,
};
use ed25519_dalek::{
    SigningKey,
    Signer as DalekSigner,
};
use sha2::{
    Digest,
    Sha512,
};

#[cfg(feature = "pq")]
use pqcrypto_dilithium::dilithium2;
#[cfg(feature = "pq")]
use pqcrypto_traits::sign::{
    DetachedSignature as _,
    PublicKey as _,
    SecretKey as _,
};
// Only `new_dilithium2_fe2o3` uses the old `rand_core`; it is mode-gated.
#[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
use rand_core_old::OsRng as OsRng_old;
use rand_core::OsRng;
use secrecy::{
    ExposeSecret,
    Secret,
};
// Only the Dilithium2_fe2o3 arm of `sign` zeroizes its own copy of the key; it is mode-gated.
#[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
use zeroize::Zeroize;

// Note: Need to use heap when zeroizing:
// https://benma.github.io/2020/10/16/rust-zeroize-move.html
// Applies here to the keys encapsulated by the variants.
/// Digital signature schemes.
#[derive(Clone)]
pub enum SignatureScheme { // Associated data: (public key, wrapped secret key)
    Ed25519(Keys< // SecretVec gets zeroed whenever dropped.
        {Self::ED25519_PK_LEN},
        {Self::ED25519_SK_LEN},
    >),
    /// The C reference implementation, wrapped. Absent without the `pq` feature.
    #[cfg(feature = "pq")]
    Dilithium2(Keys<
        {Self::DILITHIUM2_PK_LEN},
        {Self::DILITHIUM2_SK_LEN},
    >),
    /// Pure Rust impl based on https://github.com/quininer. Absent without a `mode0`..`mode3`
    /// feature, which is what gives it a parameter set.
    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    Dilithium2_fe2o3(Keys<
        {Self::DILITHIUM2_FE2O3_PK_LEN},
        {Self::DILITHIUM2_FE2O3_SK_LEN},
    >),
}

impl Debug for SignatureScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ed25519(..) => write!(f, "Ed25519"),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..) => write!(f, "Dilithium2"),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(..) => write!(f, "Dilithium2_fe2o3"),
        }
    }
}
    
impl InNamex for SignatureScheme {

    fn name_id(&self) -> Outcome<NamexId> {
	    Ok(match self {
            Self::Ed25519(..) =>
                res!(NamexId::try_from("9UQvATp4Zbv8IbWOivdhiQnex+ELo7sxOr8ntEZphMc=")),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..) =>
                res!(NamexId::try_from("W4+qt2Gd+9RQBxllcx10b4h/Ih3g9m76C+mj17TwUNw=")),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(..) =>
                res!(NamexId::try_from("zkSGGwLauv5FLpNoCse+3D7bKIdNh7PeBsfbjv/TSvQ=")),
        })
    }

    fn local_id(&self) -> LocalId {
	    match self {
            Self::Ed25519(..)           => LocalId(1),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..)        => LocalId(2),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(..)  => LocalId(3),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        let ids = match gname {
            "schemes" => [
	            ("Ed25519", "9UQvATp4Zbv8IbWOivdhiQnex+ELo7sxOr8ntEZphMc="),
                ("Dilithium2", "W4+qt2Gd+9RQBxllcx10b4h/Ih3g9m76C+mj17TwUNw="),
                ("Dilithium2_fe2o3", "zkSGGwLauv5FLpNoCse+3D7bKIdNh7PeBsfbjv/TSvQ="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for SignatureScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl Signer for SignatureScheme {

    #![allow(unused)]
    fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>> {
        match self {
            Self::Ed25519(keys) => match keys {
                Keys { pk: Some(pk), sks: Some(sks) } => { 
                    let skv = sks.expose_secret();
                    let sk_byts = res!(<[u8; Self::ED25519_SK_LEN]>::try_from(&skv[..]));
                    let signing_key = SigningKey::from_bytes(&sk_byts);
                    let verifying_key = signing_key.verifying_key();
                    if verifying_key.to_bytes() != pk[..] {
                        return Err(err!("Public key mismatch."; Invalid, Configuration));
                    }
                    let result = signing_key.sign(msg).to_bytes().to_vec();
                    Ok(result)
                },
                _ => Err(err!("Require both keys to sign."; Missing, Configuration)),
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let skv = sks.expose_secret(); // This gets zeroized automatically, ...
                    let mut sk = res!(dilithium2::SecretKey::from_bytes(&skv[..])); // this does not, so...
                    let result = dilithium2::detached_sign(msg, &sk).as_bytes().to_vec();
                    sk = res!(dilithium2::SecretKey::from_bytes(&vec![0; skv.len()])); // do it manually.
                    Ok(result)
                },
                _ => Err(err!("Require secret key to sign."; Missing, Configuration)),
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => match keys {
                Keys { sks: Some(sks), .. } => {
                    let skv = sks.expose_secret();
                    let mut sk = res!(<[u8; Self::DILITHIUM2_FE2O3_SK_LEN]>::try_from(&skv[..]));
                    let result = dilithium2_fe2o3::sign::sign(msg, &sk).to_vec();
                    sk.zeroize();
                    Ok(result)
                },
                _ => Err(err!("Require secret key to sign."; Missing, Configuration)),
            },
        }
    }

    fn verify(&self, msg: &[u8], sig: &[u8]) -> Outcome<bool> {
        Ok(match self {
            Self::Ed25519(keys) => match keys {
                Keys { pk: Some(pk), .. } => res!(verify_ed25519(&pk[..], msg, sig)),
                _ => return Err(err!("Require public key to verify."; Missing, Configuration)),
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => match keys {
                Keys { pk: Some(pk), .. } => { 
                    let pk = res!(dilithium2::PublicKey::from_bytes(&pk[..]));
                    let sig = res!(dilithium2::DetachedSignature::from_bytes(&sig));
                    match dilithium2::verify_detached_signature(&sig, msg, &pk) {
                        Ok(()) => true,
                        _ => false,
                    }
                },
                _ => return Err(err!("Require public key to verify."; Missing, Configuration)),
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => match keys {
                Keys { pk: Some(pk), .. } => {
                    let pk = res!(<[u8; Self::DILITHIUM2_FE2O3_PK_LEN]>::try_from(&pk[..]));
                    let sig = res!(<[u8; Self::DILITHIUM2_FE2O3_SIG_LEN]>::try_from(&sig[..]));
                    dilithium2_fe2o3::sign::verify(msg, &sig, &pk)
                },
                _ => return Err(err!("Require public key to verify."; Missing, Configuration)),
            },
        })
    }

    /// Checks many signatures at once, each against the public key its item
    /// carries.
    ///
    /// Ed25519 is checked by [`verify_batch_ed25519`], which decompresses each
    /// distinct public key once and, where the build carries the `batch`
    /// feature, puts the whole set to one verification equation. Either way it
    /// holds each item to the rules of [`verify_ed25519`] and accepts a set
    /// exactly when that accepts every member. The scheme's own keys are not
    /// consulted: every item names its own signer, which is what a batch drawn
    /// from a history signed by several people needs.
    ///
    /// The Dilithium schemes have no batch equation here, so they are checked
    /// one at a time and the result is the same as it always was.
    fn verify_batch(&self, items: &[BatchItem<'_>])
        -> Outcome<bool>
        where Self: Sized
    {
        match self {
            Self::Ed25519(..) => verify_batch_ed25519(items),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..) => verify_each(self, items),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(..) => verify_each(self, items),
        }
    }
}

/// Does `sig` verify as an Ed25519 signature by `public` over `msg`?
///
/// Verification is strict: RFC 8032 §5.1.7 with nothing left optional, plus the
/// one refusal the RFC leaves to the verifier.
///
/// - `public` and R must each be the canonical encoding of a point (§5.1.3), so
///   one key and one signature each have exactly one spelling.
/// - S must lie below the group order L, so a signature cannot be re-spelt as
///   S + L.
/// - A public key or R of small order is refused. The identity as a public key,
///   with R the identity and S zero, satisfies the group equation over every
///   message, so a verifier that took it would let anyone sign as that key.
/// - The group equation is the cofactored `[8][S]B = [8]R + [8][k]A`, the form
///   the RFC gives first. It is the only form a batch can check exactly, which is
///   what lets [`SignatureScheme::verify_batch`] promise to accept precisely the
///   signatures this accepts.
///
/// No honest signer is affected: a key and an R made by signing are canonical,
/// of large order and free of any small-order component. A key or signature of
/// the wrong length, or a key that encodes no curve point at all, is an error;
/// every other failure is `false`.
pub fn verify_ed25519(public: &[u8], msg: &[u8], sig: &[u8]) -> Outcome<bool> {
    let key = match res!(strict_key(public)) {
        Some(key) => key,
        None => return Ok(false),
    };
    let parts = match res!(strict_sig(sig)) {
        Some(parts) => parts,
        None => return Ok(false),
    };
    let k = Scalar::from_bytes_mod_order_wide(&challenge(&parts.r_bytes, public, msg));
    Ok(equation_holds(&key, &parts, &k))
}

/// R and S of a signature that has passed the strict checks.
struct SigParts {
    r_bytes:    [u8; 32],       // R as signed, which the challenge hashes
    r:          EdwardsPoint,
    s:          Scalar,
}

/// Decodes a public key for verification: an error where it has the wrong
/// length or encodes no point, `None` where a strict verifier refuses it.
fn strict_key(public: &[u8]) -> Outcome<Option<EdwardsPoint>> {
    let byts = match <[u8; SignatureScheme::ED25519_PK_LEN]>::try_from(public) {
        Ok(byts) => byts,
        Err(_) => return Err(err!(
            "An Ed25519 public key is {} bytes, and {} were given.",
            SignatureScheme::ED25519_PK_LEN, public.len();
            Invalid, Input, Size)),
    };
    let point = match CompressedEdwardsY(byts).decompress() {
        Some(point) => point,
        None => return Err(err!(
            "The {} bytes given as an Ed25519 public key encode no curve point.",
            SignatureScheme::ED25519_PK_LEN;
            Invalid, Input)),
    };
    if !is_canonical_point(&byts) || point.is_small_order() {
        return Ok(None);
    }
    Ok(Some(point))
}

/// Decodes a signature for verification: an error where it has the wrong
/// length, `None` where a strict verifier refuses it.
fn strict_sig(sig: &[u8]) -> Outcome<Option<SigParts>> {
    if sig.len() != SignatureScheme::ED25519_SIG_LEN {
        return Err(err!(
            "An Ed25519 signature is {} bytes, and {} were given.",
            SignatureScheme::ED25519_SIG_LEN, sig.len();
            Invalid, Input, Size));
    }
    let mut r_bytes = [0u8; 32];
    r_bytes.copy_from_slice(&sig[..32]);
    let mut s_bytes = [0u8; 32];
    s_bytes.copy_from_slice(&sig[32..]);
    if !is_canonical_point(&r_bytes) {
        return Ok(None);
    }
    let r = match CompressedEdwardsY(r_bytes).decompress() {
        Some(r) if !r.is_small_order() => r,
        _ => return Ok(None),
    };
    let s = match Option::<Scalar>::from(Scalar::from_canonical_bytes(s_bytes)) {
        Some(s) => s,
        None => return Ok(None),
    };
    Ok(Some(SigParts { r_bytes, r, s }))
}

/// SHA-512(R ‖ A ‖ M), the 64 bytes RFC 8032 reads as the integer k.
fn challenge(r_bytes: &[u8; 32], public: &[u8], msg: &[u8]) -> [u8; 64] {
    let mut h = Sha512::new();
    h.update(r_bytes);
    h.update(public);
    h.update(msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(&h.finalize());
    out
}

/// Is `[8]([S]B - R - [k]A)` the identity?
fn equation_holds(key: &EdwardsPoint, parts: &SigParts, k: &Scalar) -> bool {
    let sb_ka = EdwardsPoint::vartime_double_scalar_mul_basepoint(k, &-key, &parts.s);
    (sb_ka - parts.r).mul_by_cofactor().is_identity()
}

/// The field modulus p = 2^255 - 19, little endian, against which a compressed
/// curve point's y coordinate is measured for canonicity.
const FIELD_MODULUS: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

/// Reports whether 32 bytes are the *canonical* compressed encoding of an
/// Edwards point, which is to say the one and only encoding that point
/// compresses back to.
///
/// RFC 8032 §5.1.3 fails the decoding of any other, and `curve25519-dalek`
/// decompresses them without complaint, so the check is made here, for the
/// public key and for R alike. Two things make an encoding non-canonical:
///
/// - a y coordinate not less than p, which is reduced on decompression and so
///   compresses back to different bytes, and
/// - the sign bit set on a point whose x is zero, since -0 is 0 and compression
///   emits a clear sign bit. x is zero exactly when y is 1 or p - 1, which is
///   why only those two values are named.
fn is_canonical_point(bytes: &[u8]) -> bool {
    if bytes.len() != 32 {
        return false;
    }
    let negative = bytes[31] & 0x80 != 0;
    let mut y = [0u8; 32];
    y.copy_from_slice(bytes);
    y[31] &= 0x7f;
    // Little endian, so the comparison walks down from the top byte.
    for i in (0..32).rev() {
        if y[i] < FIELD_MODULUS[i] {
            break;
        }
        if y[i] > FIELD_MODULUS[i] {
            return false;
        }
        if i == 0 {
            return false; // Equal to p, which reduces to zero.
        }
    }
    if negative {
        // y = 1, and y = p - 1, are the two points whose x is zero.
        let one = y[0] == 0x01 && y[1..].iter().all(|b| *b == 0);
        let minus_one = y[0] == 0xec
            && y[1..31].iter().all(|b| *b == 0xff)
            && y[31] == 0x7f;
        if one || minus_one {
            return false;
        }
    }
    true
}

/// One item of a batch, decoded and passed by the strict checks.
struct Check {
    key:    usize,      // index into the batch's distinct keys
    parts:  SigParts,
    hram:   [u8; 64],   // the challenge, before reduction
}

// Domain separation for the batch coefficients
#[cfg(feature = "batch")]
const BATCH_DST: &[u8] = b"fe2o3 ed25519 batch/1";

/// Puts the checked items to one equation: the sum of each item's own
/// cofactored equation, weighted by a 128-bit coefficient.
///
/// ```text
///     [8]( [Σ z_i·S_i]B − Σ z_i·R_i − Σ_j (Σ_{i→j} z_i·k_i)·A_j ) = identity
/// ```
///
/// The coefficients are drawn by hashing every input, so a signer cannot pick a
/// signature knowing its coefficient, and nothing asks the operating system for
/// randomness, so this runs on wasm32. Each term is multiplied by the cofactor,
/// so a residue of small order, which the single check also absorbs, can
/// neither be cancelled nor exposed by a coefficient: the batch accepts a set
/// exactly when every member passes [`verify_ed25519`], short of a 2^-128
/// chance. The cofactorless batch it replaces accepted, whenever a coefficient
/// happened to annihilate it, a residue that the single check refused, and a
/// signer could arrange that by trying messages.
#[cfg(feature = "batch")]
fn holds(keys: &[EdwardsPoint], checks: &[Check]) -> bool {
    let mut h = Sha512::new();
    h.update(BATCH_DST);
    h.update(&(checks.len() as u64).to_le_bytes());
    for c in checks {
        h.update(&c.hram); // Binds R, A and the message.
        h.update(c.parts.s.as_bytes());
    }
    let seed = h.finalize();
    let mut b_coef = Scalar::ZERO;
    let mut a_coefs = vec![Scalar::ZERO; keys.len()];
    let mut scalars = Vec::with_capacity(1 + checks.len() + keys.len());
    let mut points = Vec::with_capacity(1 + checks.len() + keys.len());
    for (i, c) in checks.iter().enumerate() {
        let mut h = Sha512::new();
        h.update(&seed);
        h.update(&(i as u64).to_le_bytes());
        let mut z = [0u8; 16];
        z.copy_from_slice(&h.finalize()[..16]);
        let z = Scalar::from(u128::from_le_bytes(z));
        b_coef += z * c.parts.s;
        a_coefs[c.key] -= z * Scalar::from_bytes_mod_order_wide(&c.hram);
        scalars.push(-z);
        points.push(c.parts.r);
    }
    scalars.push(b_coef);
    points.push(ED25519_BASEPOINT_POINT);
    for (a_coef, key) in a_coefs.iter().zip(keys.iter()) {
        scalars.push(*a_coef);
        points.push(*key);
    }
    EdwardsPoint::vartime_multiscalar_mul(scalars.iter(), points.iter())
        .mul_by_cofactor()
        .is_identity()
}

/// Checks the items one at a time, each by its own equation. See the `batch`
/// variant above.
#[cfg(not(feature = "batch"))]
fn holds(keys: &[EdwardsPoint], checks: &[Check]) -> bool {
    for c in checks {
        let k = Scalar::from_bytes_mod_order_wide(&c.hram);
        if !equation_holds(&keys[c.key], &c.parts, &k) {
            return false;
        }
    }
    true
}

/// Checks a batch of Ed25519 signatures under the rules of [`verify_ed25519`],
/// decompressing each distinct public key once.
///
/// # The public key cache
///
/// Decompressing a public key costs a field inversion and a square root, and a
/// version control history is typically signed by a handful of people over
/// thousands of operations. Doing it once per *key* rather than once per
/// *signature* saves that much, and it does not depend on the batch equation
/// being available. The batch equation also gathers every signature by one key
/// into a single term, so a key costs one point in the sum however often it
/// signed.
///
/// # What the result means
///
/// `true` says every signature in the set holds. `false` says at least one does
/// not, and says nothing about which: a caller that must name the culprit
/// checks them again one at a time. A malformed key or signature is an error
/// rather than a `false`, matching what [`Signer::verify`] does with the same
/// bytes, so that a caller falling back on either outcome reproduces the same
/// message.
fn verify_batch_ed25519(items: &[BatchItem<'_>])
    -> Outcome<bool>
{
    if items.is_empty() {
        return Ok(true);
    }
    let mut index: BTreeMap<&[u8], usize> = BTreeMap::new();
    let mut keys: Vec<EdwardsPoint> = Vec::new();
    let mut checks: Vec<Check> = Vec::with_capacity(items.len());
    for item in items {
        // The key first and then the signature, the order `verify_ed25519`
        // takes them in, so an item earns the same error or `false` here.
        let key = match index.get(item.public) {
            Some(key) => *key,
            None => match res!(strict_key(item.public)) {
                Some(point) => {
                    keys.push(point);
                    index.insert(item.public, keys.len() - 1);
                    keys.len() - 1
                },
                None => return Ok(false),
            },
        };
        let parts = match res!(strict_sig(item.sig)) {
            Some(parts) => parts,
            None => return Ok(false),
        };
        let hram = challenge(&parts.r_bytes, item.public, item.msg);
        checks.push(Check { key, parts, hram });
    }
    Ok(holds(&keys, &checks))
}

impl KeyManager for SignatureScheme {

    /// Clone using the specified keys.
    fn clone_with_keys(&self, pk: Option<&[u8]>, sk: Option<&[u8]>) -> Outcome<Self> {
        Ok(match self {
            Self::Ed25519(..) => Self::Ed25519(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(<[u8; Self::ED25519_PK_LEN]>::try_from(&pk[..]))),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(
                        <[u8; Self::ED25519_SK_LEN]>::try_from(&sk[..])
                    ))),
                    None => None,
                },
            }),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..) => Self::Dilithium2(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(<[u8; Self::DILITHIUM2_PK_LEN]>::try_from(&pk[..]))),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(
                        <[u8; Self::DILITHIUM2_SK_LEN]>::try_from(&sk[..])
                    ))),
                    None => None,
                },
            }),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(..) => Self::Dilithium2_fe2o3(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(
                        <[u8; Self::DILITHIUM2_FE2O3_PK_LEN]>::try_from(&pk[..])
                    )),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(
                        <[u8; Self::DILITHIUM2_FE2O3_SK_LEN]>::try_from(&sk[..])
                    ))),
                    None => None,
                },
            }),
        })
    }

    fn get_public_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::Ed25519(keys) => match &keys.pk {
                Some(k) => Some(&k[..]),
                None => None,
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => match &keys.pk {
                Some(k) => Some(&k[..]),
                None => None,
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => match &keys.pk {
                Some(k) => Some(&k[..]),
                None => None,
            },
        })
    }

    fn get_secret_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::Ed25519(keys) => match &keys.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(&sk[..])
                },
                None => None,
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => match &keys.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(&sk[..])
                },
                None => None,
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => match &keys.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(&sk[..])
                },
                None => None,
            },
        })
    }

    fn set_public_key(mut self, pk: Option<&[u8]>) -> Outcome<Self> {
        match &mut self {
            Self::Ed25519(keys) => keys.pk = match pk {
                Some(pk) => Some(res!(<[u8; Self::ED25519_PK_LEN]>::try_from(&pk[..]))),
                None => None,
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => keys.pk = match pk {
                Some(pk) => Some(res!(<[u8; Self::DILITHIUM2_PK_LEN]>::try_from(&pk[..]))),
                None => None,
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => keys.pk = match pk {
                Some(pk) => Some(res!(<[u8; Self::DILITHIUM2_FE2O3_PK_LEN]>::try_from(&pk[..]))),
                None => None,
            },
        }
        Ok(self)
    }

    fn set_secret_key(mut self, sk: Option<&[u8]>) -> Outcome<Self> {
        match &mut self {
            Self::Ed25519(keys) => keys.sks = match sk {
                Some(sk) => Some(Secret::new(res!(<[u8; Self::ED25519_SK_LEN]>::try_from(&sk[..])))),
                None => None,
            },
            #[cfg(feature = "pq")]
            Self::Dilithium2(keys) => keys.sks = match sk {
                Some(sk) => Some(Secret::new(res!(<[u8; Self::DILITHIUM2_SK_LEN]>::try_from(&sk[..])))),
                None => None,
            },
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            Self::Dilithium2_fe2o3(keys) => keys.sks = match sk {
                Some(sk) => Some(Secret::new(res!(<[u8; Self::DILITHIUM2_FE2O3_SK_LEN]>::try_from(&sk[..])))),
                None => None,
            },
        }
        Ok(self)
    }
}

impl str::FromStr for SignatureScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match name {
            "Ed25519" => Self::new_ed25519(),
            #[cfg(feature = "pq")]
            "Dilithium2" => res!(Self::new_dilithium2()),
            // The name is a real one, and this build simply does not carry it. Saying so is not the
            // same as saying it does not exist, and a caller deserves to be told which it is.
            #[cfg(not(feature = "pq"))]
            "Dilithium2" => return Err(err!(
                "The signature scheme 'Dilithium2' is the C reference implementation, which this \
                build does not carry: it was built without the 'pq' feature, which needs a C \
                toolchain. The pure-Rust 'Dilithium2_fe2o3' is here and does the same job.";
            Invalid, Input, NoImpl)),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            "Dilithium2_fe2o3" => Self::new_dilithium2_fe2o3(),
            // The name is a real one, and this build simply does not carry it: it was built
            // without a `mode0`..`mode3` feature, which is what gives Dilithium2_fe2o3 a
            // parameter set.
            #[cfg(not(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3")))]
            "Dilithium2_fe2o3" => return Err(err!(
                "The signature scheme 'Dilithium2_fe2o3' needs one of the 'mode0'..'mode3' \
                features, which this build was not given.";
            Invalid, Input, NoImpl)),
            _ => return Err(err!(
                "The signature scheme '{}' is not recognised.", name;
            Invalid, Input)),
        })
    }
}

impl TryFrom<&LocalId> for SignatureScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: &LocalId) -> std::result::Result<Self, Self::Error> {
        Ok(match *n {
            LocalId(1) => Self::new_ed25519(),
            #[cfg(feature = "pq")]
            LocalId(2) => res!(Self::new_dilithium2()),
            #[cfg(not(feature = "pq"))]
            LocalId(2) => return Err(err!(
                "The signature scheme with local id 2 is Dilithium2, the C reference \
                implementation, which this build does not carry: it was built without the 'pq' \
                feature. The pure-Rust Dilithium2_fe2o3, local id 3, is here.";
            Invalid, Input, NoImpl)),
            #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
            LocalId(3) => Self::new_dilithium2_fe2o3(),
            #[cfg(not(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3")))]
            LocalId(3) => return Err(err!(
                "The signature scheme with local id 3 is Dilithium2_fe2o3, which needs one of the \
                'mode0'..'mode3' features, which this build was not given.";
            Invalid, Input, NoImpl)),
            _ => return Err(err!(
                "The signature scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        })
    }
}

impl SignatureScheme {

    //pub const USR_VERSION: SemVer = SemVer::new(0,0,1);
    pub const SCHEMES: [&'static str; 3] = [
        "<EdDSA|Ed25519>",
        "<Dilithium|Dilithium2>",
        "<Dilithium|Dilithium2_fe2o3>",
    ];

    pub const ED25519_PK_LEN:           usize = ed25519_dalek::PUBLIC_KEY_LENGTH;
    pub const ED25519_SK_LEN:           usize = ed25519_dalek::SECRET_KEY_LENGTH;
    pub const ED25519_SIG_LEN:          usize = ed25519_dalek::SIGNATURE_LENGTH;
    // These are the C implementation's own sizes, so they can only be asked of it when it is here.
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_PK_LEN:        usize = dilithium2::public_key_bytes();
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_SK_LEN:        usize = dilithium2::secret_key_bytes();
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_SIG_LEN:       usize = dilithium2::signature_bytes();
    // These sizes come from the mode feature's parameter set, so they only exist with one.
    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    pub const DILITHIUM2_FE2O3_PK_LEN:  usize = dilithium2_fe2o3::params::PUBLICKEYBYTES;
    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    pub const DILITHIUM2_FE2O3_SK_LEN:  usize = dilithium2_fe2o3::params::SECRETKEYBYTES;
    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    pub const DILITHIUM2_FE2O3_SIG_LEN: usize = dilithium2_fe2o3::params::SIG_SIZE_PACKED;

    pub fn new_ed25519() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let keys = Keys {
            pk: Some(signing_key.verifying_key().to_bytes()),
            sks: Some(Secret::new(signing_key.to_bytes())),
        };
        Self::Ed25519(keys)
    }

    pub fn empty_ed25519() -> Self {
        Self::Ed25519(Keys::default())
    }

    #[cfg(feature = "pq")]
    pub fn new_dilithium2() -> Outcome<Self> {
        let (pk, sk) = dilithium2::keypair();
        const PK_LEN: usize = dilithium2::public_key_bytes();
        const SK_LEN: usize = dilithium2::secret_key_bytes();
        let keys = Keys {
            pk: Some(res!(<[u8; PK_LEN]>::try_from(&(pk.as_bytes())[..]))),
            sks: Some(Secret::new(res!(<[u8; SK_LEN]>::try_from(&(sk.as_bytes())[..])))),
        };
        Ok(Self::Dilithium2(keys))
    }

    #[cfg(feature = "pq")]
    pub fn empty_dilithium2() -> Self {
        Self::Dilithium2(Keys::default())
    }

    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    pub fn new_dilithium2_fe2o3() -> Self {
        let (mut pk, mut sk) = (
            [0; Self::DILITHIUM2_FE2O3_PK_LEN],
            [0; Self::DILITHIUM2_FE2O3_SK_LEN],
        );
        dilithium2_fe2o3::sign::keypair(&mut OsRng_old, &mut pk, &mut sk);
        let keys = Keys {
            pk: Some(pk),
            sks: Some(Secret::new(sk)),
        };
        Self::Dilithium2_fe2o3(keys)
    }

    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    pub fn empty_dilithium2_fe2o3() -> Self {
        Self::Dilithium2_fe2o3(Keys::default())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A key pair together with the public key bytes, which is what a batch item
    /// wants and what the scheme hands back only as an `Option`.
    struct Pair {
        /// The scheme, holding both keys.
        scheme: SignatureScheme,
        /// The public key bytes.
        public: Vec<u8>,
    }

    /// Mints an Ed25519 pair for a test.
    fn pair() -> Outcome<Pair> {
        let scheme = SignatureScheme::new_ed25519();
        let public = match res!(scheme.get_public_key()) {
            Some(pk) => pk.to_vec(),
            None => return Err(err!("A minted Ed25519 pair has no public key."; Bug, Missing)),
        };
        Ok(Pair { scheme, public })
    }

    /// Checks one signature the way a caller checks one, so that a test can ask
    /// whether the batch and the single agree.
    fn singly(public: &[u8], msg: &[u8], sig: &[u8]) -> Outcome<bool> {
        let bound = res!(SignatureScheme::empty_ed25519().clone_with_keys(Some(public), None));
        bound.verify(msg, sig)
    }

    /// A batch of sound signatures by several signers holds, and an empty batch
    /// holds vacuously.
    #[test]
    fn a_batch_of_sound_ed25519_signatures_holds() -> Outcome<()> {
        let a = res!(pair());
        let b = res!(pair());
        let msgs: Vec<Vec<u8>> = (0..8u8).map(|i| vec![i; 64 + i as usize]).collect();
        let mut sigs = Vec::new();
        for (i, msg) in msgs.iter().enumerate() {
            let who = if i % 2 == 0 { &a } else { &b };
            sigs.push(res!(who.scheme.sign(msg)));
        }
        let items: Vec<BatchItem<'_>> = (0..msgs.len())
            .map(|i| BatchItem {
                public: if i % 2 == 0 { &a.public } else { &b.public },
                msg:    &msgs[i],
                sig:    &sigs[i],
            })
            .collect();
        let algorithm = SignatureScheme::empty_ed25519();
        assert!(res!(algorithm.verify_batch(&items)));
        assert!(res!(algorithm.verify_batch(&[])), "an empty batch holds vacuously");
        Ok(())
    }

    /// One tampered signature anywhere in the batch fails it, and checking the
    /// same items one at a time then finds exactly that one.
    ///
    /// This is what lets a caller batch at all. The batch is permitted to be
    /// silent about which member failed only because the fallback is guaranteed
    /// to find it; a batch that failed while every member passed singly would
    /// leave a caller with a refusal it cannot explain.
    #[test]
    fn a_tampered_signature_fails_the_batch_and_is_found_singly() -> Outcome<()> {
        let a = res!(pair());
        let msgs: Vec<Vec<u8>> = (0..6u8).map(|i| vec![i; 40]).collect();
        let algorithm = SignatureScheme::empty_ed25519();
        for spoiled in 0..msgs.len() {
            let mut sigs = Vec::new();
            for (i, msg) in msgs.iter().enumerate() {
                let mut sig = res!(a.scheme.sign(msg));
                if i == spoiled {
                    sig[40] ^= 0x01; // Within s, so the encoding stays well formed.
                }
                sigs.push(sig);
            }
            let items: Vec<BatchItem<'_>> = (0..msgs.len())
                .map(|i| BatchItem { public: &a.public, msg: &msgs[i], sig: &sigs[i] })
                .collect();
            assert!(!res!(algorithm.verify_batch(&items)),
                "the batch holding a tampered signature at {} was accepted", spoiled);
            let mut bad = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if !res!(singly(item.public, item.msg, item.sig)) {
                    bad.push(i);
                }
            }
            assert_eq!(bad, vec![spoiled], "the fallback named the wrong signature");
        }
        Ok(())
    }

    /// The public key cache does not let a second signature by a signer already
    /// in the batch go unchecked.
    ///
    /// A cache keyed on the public key is exactly the shape of mistake where the
    /// *key* is remembered as having been checked rather than the *signature*,
    /// and every signature after the first by that signer is then waved through.
    /// Every item here is signed by one key and every one of them is tampered
    /// with in turn.
    #[test]
    fn the_key_cache_does_not_wave_a_repeat_signer_through() -> Outcome<()> {
        let a = res!(pair());
        let algorithm = SignatureScheme::empty_ed25519();
        let msgs: Vec<Vec<u8>> = (0..5u8).map(|i| vec![i; 32]).collect();
        for spoiled in 0..msgs.len() {
            let mut sigs = Vec::new();
            for (i, msg) in msgs.iter().enumerate() {
                let mut sig = res!(a.scheme.sign(msg));
                if i == spoiled {
                    sig[40] ^= 0x01;
                }
                sigs.push(sig);
            }
            let items: Vec<BatchItem<'_>> = (0..msgs.len())
                .map(|i| BatchItem { public: &a.public, msg: &msgs[i], sig: &sigs[i] })
                .collect();
            assert!(!res!(algorithm.verify_batch(&items)),
                "signature {} by an already cached key was not checked", spoiled);
        }
        Ok(())
    }

    /// Swapping two signatures between messages fails the batch, which a batch
    /// that only counted signatures would not catch.
    #[test]
    fn signatures_swapped_between_messages_fail_the_batch() -> Outcome<()> {
        let a = res!(pair());
        let one = b"the first message".to_vec();
        let two = b"the second message".to_vec();
        let sig_one = res!(a.scheme.sign(&one));
        let sig_two = res!(a.scheme.sign(&two));
        let items = vec![
            BatchItem { public: &a.public, msg: &one, sig: &sig_two },
            BatchItem { public: &a.public, msg: &two, sig: &sig_one },
        ];
        assert!(!res!(SignatureScheme::empty_ed25519().verify_batch(&items)));
        Ok(())
    }

    /// The batch accepts a signature exactly when checking it alone does.
    ///
    /// The two are different equations, and the reason a batch may stand in for
    /// the single check is that they accept the same set. Every one-bit change
    /// tried here has to be refused by both or accepted by both.
    #[test]
    fn the_batch_and_the_single_check_agree() -> Outcome<()> {
        let a = res!(pair());
        let b = res!(pair());
        let msg = b"a message worth signing".to_vec();
        let sound = res!(a.scheme.sign(&msg));
        let algorithm = SignatureScheme::empty_ed25519();

        // Each case is (what it is, public key, message, signature).
        let mut spoiled_msg = msg.clone();
        spoiled_msg[0] ^= 0x01;
        let mut spoiled_sig_r = sound.clone();
        spoiled_sig_r[0] ^= 0x01;
        let mut spoiled_sig_s = sound.clone();
        spoiled_sig_s[40] ^= 0x01;
        let non_canonical_r = {
            // A y coordinate of p itself, which decompression reduces and which
            // therefore compresses back to different bytes.
            let mut sig = sound.clone();
            sig[..32].copy_from_slice(&FIELD_MODULUS);
            sig
        };
        // The identity as a key, R the identity and S zero: the cofactorless
        // equation holds for every message.
        let mut identity = [0u8; 32];
        identity[0] = 0x01;
        let mut forged = [0u8; 64];
        forged[0] = 0x01;
        // A point of order four, y = 0, as a key.
        let order_four = [0u8; 32];
        let cases: Vec<(&str, &[u8], &[u8], &[u8], bool)> = vec![
            ("sound",               &a.public,      &msg,         &sound,           true),
            ("another's key",       &b.public,      &msg,         &sound,           false),
            ("altered message",     &a.public,      &spoiled_msg, &sound,           false),
            ("altered R",           &a.public,      &msg,         &spoiled_sig_r,   false),
            ("altered s",           &a.public,      &msg,         &spoiled_sig_s,   false),
            ("non canonical R",     &a.public,      &msg,         &non_canonical_r, false),
            ("identity key forgery",&identity,      &msg,         &forged,          false),
            ("order four key",      &order_four,    &msg,         &forged,          false),
        ];
        for (what, public, message, sig, verdict) in cases {
            let items = vec![BatchItem { public, msg: message, sig }];
            let batched = res!(algorithm.verify_batch(&items));
            let single = res!(singly(public, message, sig));
            assert_eq!(batched, single,
                "the batch and the single check disagree about the {} case", what);
            assert_eq!(single, verdict, "the {} case earned the wrong verdict", what);
            assert_eq!(res!(verify_ed25519(public, message, sig)), single,
                "the free function and the scheme disagree about the {} case", what);
        }
        Ok(())
    }

    /// The canonicity test admits the encodings compression produces and refuses
    /// the ones it does not.
    #[test]
    fn canonical_points_are_told_from_the_rest() -> Outcome<()> {
        let mut zero = [0u8; 32];
        assert!(is_canonical_point(&zero), "y = 0 is canonical");
        zero[31] |= 0x80;
        assert!(is_canonical_point(&zero), "y = 0 with the sign bit set is a real encoding");

        assert!(!is_canonical_point(&FIELD_MODULUS), "y = p reduces to zero");
        let mut over = FIELD_MODULUS;
        over[0] = 0xee;
        assert!(!is_canonical_point(&over), "y = p + 1 reduces to one");
        let mut under = FIELD_MODULUS;
        under[0] = 0xec;
        assert!(is_canonical_point(&under), "y = p - 1 is canonical");
        under[31] |= 0x80;
        assert!(!is_canonical_point(&under), "y = p - 1 has x = 0, so it has no negative");

        let mut one = [0u8; 32];
        one[0] = 0x01;
        assert!(is_canonical_point(&one), "y = 1 is the identity");
        one[31] |= 0x80;
        assert!(!is_canonical_point(&one), "y = 1 has x = 0, so it has no negative");

        let mut two = [0u8; 32];
        two[0] = 0x02;
        two[31] |= 0x80;
        assert!(is_canonical_point(&two), "y = 2 has a negative like any other");

        assert!(!is_canonical_point(&[0u8; 31]), "a short encoding is not one");
        assert!(!is_canonical_point(&[0u8; 33]), "a long encoding is not one");
        Ok(())
    }

    /// A build without the batch equation gives the same answers as one with it.
    ///
    /// This is the wasm32 path stated as a property. A browser build that leaves
    /// `batch` off -- or any build that does -- falls back to `verify_each`, and
    /// what must not differ between the two is which signatures are accepted.
    /// The test compares the two here, in one build, so that a divergence shows
    /// up on a developer's machine rather than only in a browser.
    #[test]
    fn the_batchless_path_agrees_with_the_batch() -> Outcome<()> {
        let a = res!(pair());
        let b = res!(pair());
        let msgs: Vec<Vec<u8>> = (0..4u8).map(|i| vec![i; 48]).collect();
        let algorithm = SignatureScheme::empty_ed25519();
        // Every subset of the four having its signature spoiled, sixteen in all,
        // so the two paths are compared on sound sets and unsound ones alike.
        for spoiled in 0..16u8 {
            let mut sigs = Vec::new();
            for (i, msg) in msgs.iter().enumerate() {
                let who = if i % 2 == 0 { &a } else { &b };
                let mut sig = res!(who.scheme.sign(msg));
                if spoiled & (1 << i) != 0 {
                    sig[40] ^= 0x01;
                }
                sigs.push(sig);
            }
            let items: Vec<BatchItem<'_>> = (0..msgs.len())
                .map(|i| BatchItem {
                    public: if i % 2 == 0 { &a.public } else { &b.public },
                    msg:    &msgs[i],
                    sig:    &sigs[i],
                })
                .collect();
            let batched = res!(algorithm.verify_batch(&items));
            let each = res!(verify_each(&algorithm, &items));
            assert_eq!(batched, each,
                "the two paths disagree about the set spoiled at {:04b}", spoiled);
            assert_eq!(batched, spoiled == 0,
                "the set spoiled at {:04b} was not judged on its merits", spoiled);
        }
        Ok(())
    }

    /// A scheme with no batch equation still answers the batch, one signature at
    /// a time, so a caller need not ask which scheme it holds.
    #[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
    #[test]
    fn a_scheme_without_a_batch_equation_still_answers() -> Outcome<()> {
        let scheme = SignatureScheme::new_dilithium2_fe2o3();
        let public = match res!(scheme.get_public_key()) {
            Some(pk) => pk.to_vec(),
            None => return Err(err!("A minted pair has no public key."; Bug, Missing)),
        };
        let msg = b"a message worth signing".to_vec();
        let sound = res!(scheme.sign(&msg));
        let mut spoiled = sound.clone();
        spoiled[0] ^= 0x01;
        assert!(res!(scheme.verify_batch(&[
            BatchItem { public: &public, msg: &msg, sig: &sound },
        ])));
        assert!(!res!(scheme.verify_batch(&[
            BatchItem { public: &public, msg: &msg, sig: &sound },
            BatchItem { public: &public, msg: &msg, sig: &spoiled },
        ])));
        Ok(())
    }
}
