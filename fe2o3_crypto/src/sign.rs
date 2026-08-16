use crate::{
    keys::Keys,
    pqc::dilithium as dilithium2_fe2o3,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::{
        BatchItem,
        Signer,
        verify_each,
    },
};
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

use ed25519_dalek::{
    Signature,
    SigningKey,
    Signer as DalekSigner,
    Verifier,
    VerifyingKey,
};

#[cfg(feature = "pq")]
use pqcrypto_dilithium::dilithium2;
#[cfg(feature = "pq")]
use pqcrypto_traits::sign::{
    DetachedSignature as _,
    PublicKey as _,
    SecretKey as _,
};
use rand_core_old::OsRng as OsRng_old;
use rand_core::OsRng;
use secrecy::{
    ExposeSecret,
    Secret,
};
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
    /// The C reference implementation, wrapped. Absent without the `pq` feature; the pure-Rust
    /// `Dilithium2_fe2o3` below is always here.
    #[cfg(feature = "pq")]
    Dilithium2(Keys<
        {Self::DILITHIUM2_PK_LEN},
        {Self::DILITHIUM2_SK_LEN},
    >),
    Dilithium2_fe2o3(Keys< // Pure Rust impl based on https://github.com/quininer
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
            Self::Dilithium2_fe2o3(..) =>
                res!(NamexId::try_from("zkSGGwLauv5FLpNoCse+3D7bKIdNh7PeBsfbjv/TSvQ=")),
        })
    }

    fn local_id(&self) -> LocalId {
	    match self {
            Self::Ed25519(..)           => LocalId(1),
            #[cfg(feature = "pq")]
            Self::Dilithium2(..)        => LocalId(2),
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
                Keys { pk: Some(pk), .. } => { 
                    let verifying_key = res!(VerifyingKey::from_bytes(pk));
                    let signature = res!(Signature::from_slice(sig));
                    match verifying_key.verify(msg, &signature) {
                        Ok(()) => true,
                        _ => false,
                    }
                },
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
    /// feature, puts the whole set to one verification equation. The scheme's
    /// own keys are not consulted: every item names its own signer, which is
    /// what a batch drawn from a history signed by several people needs.
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
            Self::Dilithium2_fe2o3(..) => verify_each(self, items),
        }
    }
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
/// # Why this is here
///
/// Ed25519 has two verification equations that agree on every signature anyone
/// would produce and disagree on a handful nobody would. The single-signature
/// check in `ed25519-dalek` recomputes R and compares its *bytes* to the
/// signature's, which rejects a non-canonically encoded R; batch verification
/// decompresses R to a point instead, and a non-canonical encoding decompresses
/// to the same point as the canonical one. Without this check a batch would
/// therefore accept a re-encoded signature that checking one at a time refuses,
/// and the two would no longer be the same test.
///
/// Two things make an encoding non-canonical, and both are refused:
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

/// Puts the collected triples to the batch verification equation where the
/// build has one, and to the ordinary check one at a time where it does not.
///
/// The `batch` feature is what decides. Without it the public key cache above
/// still stands, so a build that cannot batch is still spared decompressing one
/// signer's key once per signature.
#[cfg(feature = "batch")]
fn check_collected(msgs: &[&[u8]], sigs: &[Signature], keys: &[VerifyingKey]) -> bool {
    ed25519_dalek::verify_batch(msgs, sigs, keys).is_ok()
}

/// Checks the collected triples one at a time. See the `batch` variant above.
#[cfg(not(feature = "batch"))]
fn check_collected(msgs: &[&[u8]], sigs: &[Signature], keys: &[VerifyingKey]) -> bool {
    for i in 0..sigs.len() {
        if keys[i].verify(msgs[i], &sigs[i]).is_err() {
            return false;
        }
    }
    true
}

/// Checks a batch of Ed25519 signatures, decompressing each distinct public key
/// once.
///
/// # The public key cache
///
/// Decompressing a public key costs a field inversion and a square root, and a
/// version control history is typically signed by a handful of people over
/// thousands of operations. Doing it once per *key* rather than once per
/// *signature* is the whole of the saving, and it does not depend on the batch
/// equation being available.
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
    let mut cache: BTreeMap<&[u8], VerifyingKey> = BTreeMap::new();
    let mut keys: Vec<VerifyingKey> = Vec::with_capacity(items.len());
    let mut sigs: Vec<Signature> = Vec::with_capacity(items.len());
    let mut msgs: Vec<&[u8]> = Vec::with_capacity(items.len());
    for item in items {
        let key = match cache.get(item.public) {
            Some(key) => *key,
            None => {
                let byts = res!(<[u8; SignatureScheme::ED25519_PK_LEN]>::try_from(item.public));
                let key = res!(VerifyingKey::from_bytes(&byts));
                cache.insert(item.public, key);
                key
            },
        };
        // Built first, so that a signature of the wrong length is the error it
        // has always been rather than a bare `false`.
        let sig = res!(Signature::from_slice(item.sig));
        // A non-canonically encoded R would part the batch equation from the
        // single one; see `is_canonical_point`.
        if !is_canonical_point(&item.sig[..32]) {
            return Ok(false);
        }
        keys.push(key);
        sigs.push(sig);
        msgs.push(item.msg);
    }
    Ok(check_collected(&msgs, &sigs, &keys))
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
            "Dilithium2_fe2o3" => Self::new_dilithium2_fe2o3(),
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
            LocalId(3) => Self::new_dilithium2_fe2o3(),
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
    // These are the C implementation's own sizes, so they can only be asked of it when it is here.
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_PK_LEN:        usize = dilithium2::public_key_bytes();
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_SK_LEN:        usize = dilithium2::secret_key_bytes();
    #[cfg(feature = "pq")]
    pub const DILITHIUM2_SIG_LEN:       usize = dilithium2::signature_bytes();
    pub const DILITHIUM2_FE2O3_PK_LEN:  usize = dilithium2_fe2o3::params::PUBLICKEYBYTES;
    pub const DILITHIUM2_FE2O3_SK_LEN:  usize = dilithium2_fe2o3::params::SECRETKEYBYTES;
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
        let cases: Vec<(&str, &[u8], &[u8], &[u8])> = vec![
            ("sound",               &a.public, &msg,         &sound),
            ("another's key",       &b.public, &msg,         &sound),
            ("altered message",     &a.public, &spoiled_msg, &sound),
            ("altered R",           &a.public, &msg,         &spoiled_sig_r),
            ("altered s",           &a.public, &msg,         &spoiled_sig_s),
            ("non canonical R",     &a.public, &msg,         &non_canonical_r),
        ];
        for (what, public, message, sig) in cases {
            let items = vec![BatchItem { public, msg: message, sig }];
            let batched = res!(algorithm.verify_batch(&items));
            let single = res!(singly(public, message, sig));
            assert_eq!(batched, single,
                "the batch and the single check disagree about the {} case", what);
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
