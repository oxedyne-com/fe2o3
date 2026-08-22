//! X25519 key agreement: encapsulating a session key to a named recipient.
//!
//! This is the classical half of [`oxedyne_fe2o3_iop_crypto::kem::KeyExchanger`],
//! beside the post-quantum [`crate::kem`], and it is deliberately not shaped on
//! it: that module's `encap` ignores the public key it is given and encapsulates
//! to the key the scheme itself holds, which is the opposite of what a caller
//! wrapping a secret for somebody else needs.
//!
//! # What it costs to build
//!
//! Nothing new in the dependency graph. `curve25519-dalek` is already there
//! through `ed25519-dalek`, and [`MontgomeryPoint::mul_clamped`] is public and
//! unfeatured, so the whole of X25519 is those two calls and a digest.
//!
//! # Where it is exercised
//!
//! In `ore_store`, not here. This crate's test target has not linked since some
//! time before 12026-08-22 -- `rust-lld` reports `jent_entropy_collector_alloc`
//! and three siblings undefined, jitter entropy symbols from the `pqcrypto`
//! C build -- so a test written beside this code could not be run. That is a
//! separate defect on the post-quantum path; the consequence here is only that
//! the tests live at the first downstream caller that links.

use crate::keys::Keys;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_crypto::{
    kem::KeyExchanger,
    keys::KeyManager,
};
use oxedyne_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};
use oxedyne_fe2o3_namex::id::{
    InNamex,
    LocalId,
    NamexId,
};

use std::{
    convert::TryFrom,
    fmt,
    str,
};

use curve25519_dalek::montgomery::MontgomeryPoint;
use rand_core::{
    OsRng,
    RngCore,
};
use secrecy::{
    ExposeSecret,
    Secret,
};


#[derive(Clone)]
pub enum AgreementScheme {
    X25519(Keys<
        {Self::X25519_PK_LEN},
        {Self::X25519_SK_LEN},
    >),
}

impl fmt::Display for AgreementScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for AgreementScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::X25519(..) => write!(f, "X25519"),
        }
    }
}

impl InNamex for AgreementScheme {

    fn name_id(&self) -> Outcome<NamexId> {
        Ok(match self {
            Self::X25519(..) =>
                res!(NamexId::try_from("HFN5dPSwWFeAktUWQEr0S9Zn1LyMSaurR4tPSPM9c0w=")),
        })
    }

    /// Version-dependent identifier for the agreement scheme, which is a far more
    /// compact alternative to the 256 bit Namex id.
    fn local_id(&self) -> LocalId {
        match self {
            Self::X25519(..) => LocalId(1),
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
                ("X25519", "HFN5dPSwWFeAktUWQEr0S9Zn1LyMSaurR4tPSPM9c0w="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for AgreementScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl KeyManager for AgreementScheme {

    fn clone_with_keys(&self, pk: Option<&[u8]>, sk: Option<&[u8]>) -> Outcome<Self> {
        Ok(match self {
            Self::X25519(..) => Self::X25519(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(<[u8; Self::X25519_PK_LEN]>::try_from(&pk[..]))),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(
                        <[u8; Self::X25519_SK_LEN]>::try_from(&sk[..])
                    ))),
                    None => None,
                },
            }),
        })
    }

    fn get_public_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::X25519(keys) => match &keys.pk {
                Some(k) => Some(&k[..]),
                None => None,
            },
        })
    }

    fn get_secret_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::X25519(keys) => match &keys.sks {
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
            Self::X25519(keys) => keys.pk = match pk {
                Some(pk) => Some(res!(<[u8; Self::X25519_PK_LEN]>::try_from(&pk[..]))),
                None => None,
            },
        }
        Ok(self)
    }

    fn set_secret_key(mut self, sk: Option<&[u8]>) -> Outcome<Self> {
        match &mut self {
            Self::X25519(keys) => keys.sks = match sk {
                Some(sk) => Some(Secret::new(res!(
                    <[u8; Self::X25519_SK_LEN]>::try_from(&sk[..])
                ))),
                None => None,
            },
        }
        Ok(self)
    }
}

impl KeyExchanger for AgreementScheme {

    /// Mints an ephemeral key pair, agrees a session key with `pk`, and hands
    /// back the ephemeral public key as the encapsulation of it.
    ///
    /// The ephemeral key is what makes this worth the thirty-two bytes it costs:
    /// under a static sender key, one leaked recipient secret opens every session
    /// key ever sent to that recipient. Here it opens only the encapsulations an
    /// attacker can still lay hands on. **That protects the encapsulation and not
    /// whatever was encrypted under the session key**, which is a distinction
    /// anybody describing this to a user has to keep.
    fn encap<
        const PK_LEN: usize,
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        pk: [u8; PK_LEN],
    )
        -> Outcome<(
            [u8; SESSION_KEY_LEN],
            [u8; CIPHERTEXT_LEN],
        )>
    {
        match self {
            Self::X25519(..) => {
                let theirs = res!(<[u8; Self::X25519_PK_LEN]>::try_from(&pk[..]));
                let mut eph_sk = [0u8; Self::X25519_SK_LEN];
                OsRng.fill_bytes(&mut eph_sk);
                let eph_pk = MontgomeryPoint::mul_base_clamped(eph_sk).to_bytes();
                let shared = res!(Self::agree(&eph_sk, &theirs));
                let session = res!(Self::derive(&eph_pk, &theirs, &shared));
                Ok((
                    res!(<[u8; SESSION_KEY_LEN]>::try_from(&session[..])),
                    res!(<[u8; CIPHERTEXT_LEN]>::try_from(&eph_pk[..])),
                ))
            },
        }
    }

    /// Recovers the session key from the ephemeral public key [`Self::encap`]
    /// published beside it.
    ///
    /// The recipient's own public key goes into the digest, and it is derived
    /// from the secret rather than read out of the pair, so a pair holding a
    /// public key that does not belong to its secret fails to agree rather than
    /// quietly deriving something the sender never derived.
    fn decap<
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        ciphertext: [u8; CIPHERTEXT_LEN],
    )
        -> Outcome<[u8; SESSION_KEY_LEN]>
    {
        match self {
            Self::X25519(keys) => match &keys.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    let eph_pk = res!(<[u8; Self::X25519_PK_LEN]>::try_from(&ciphertext[..]));
                    let ours = MontgomeryPoint::mul_base_clamped(*sk).to_bytes();
                    let shared = res!(Self::agree(sk, &eph_pk));
                    let session = res!(Self::derive(&eph_pk, &ours, &shared));
                    Ok(res!(<[u8; SESSION_KEY_LEN]>::try_from(&session[..])))
                },
                None => Err(err!(
                    "Require secret key to de-encapsulate.";
                Missing, Configuration)),
            },
        }
    }
}

impl str::FromStr for AgreementScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "X25519" => Ok(Self::new_x25519()),
            _ => Err(err!(
                "The key agreement scheme '{}' is not recognised.", name;
            Invalid, Input)),
        }
    }
}

impl TryFrom<LocalId> for AgreementScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: LocalId) -> std::result::Result<Self, Self::Error> {
        match n {
            LocalId(1) => Ok(Self::new_x25519()),
            _ => Err(err!(
                "The key agreement scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl AgreementScheme {

    pub const X25519_PK_LEN:            usize = 32;
    pub const X25519_SK_LEN:            usize = 32;
    pub const X25519_SESSION_KEY_LEN:   usize = 32;
    // The encapsulation is the ephemeral public key, which is what a
    // Diffie-Hellman KEM's ciphertext is.
    pub const X25519_CIPHERTEXT_LEN:    usize = 32;

    /// What goes into the digest ahead of the keys, so that a session key
    /// derived here can never collide with one derived by another protocol from
    /// the same shared secret.
    pub const X25519_KDF_TAG: &'static str = "FE2O3-X25519-SHA3-256-1";

    /// Mints a fresh key pair.
    pub fn new_x25519() -> Self {
        let mut sk = [0u8; Self::X25519_SK_LEN];
        OsRng.fill_bytes(&mut sk);
        let pk = MontgomeryPoint::mul_base_clamped(sk).to_bytes();
        Self::X25519(Keys::new(Some(pk), Some(Secret::new(sk))))
    }

    /// The scheme holding no keys, for a caller that is about to install its own.
    pub fn empty_x25519() -> Self {
        Self::X25519(Keys::default())
    }

    /// Takes a secret somebody already holds, deriving the public key from it
    /// rather than being told it.
    pub fn x25519_with_secret(sk: &[u8])
        -> Outcome<Self>
    {
        let sk = res!(<[u8; Self::X25519_SK_LEN]>::try_from(sk));
        let pk = MontgomeryPoint::mul_base_clamped(sk).to_bytes();
        Ok(Self::X25519(Keys::new(Some(pk), Some(Secret::new(sk)))))
    }

    /// The public key belonging to an X25519 secret.
    pub fn x25519_public_of(sk: &[u8])
        -> Outcome<[u8; Self::X25519_PK_LEN]>
    {
        let sk = res!(<[u8; Self::X25519_SK_LEN]>::try_from(sk));
        Ok(MontgomeryPoint::mul_base_clamped(sk).to_bytes())
    }

    /// The raw Diffie-Hellman, refusing the all zero result.
    ///
    /// A public key of small order drives every secret to the same point,
    /// whoever holds it, so an all zero agreement is not a shared secret at all;
    /// RFC 7748 §6.1 says to check for it and this is that check.
    fn agree(sk: &[u8; Self::X25519_SK_LEN], pk: &[u8; Self::X25519_PK_LEN])
        -> Outcome<[u8; Self::X25519_SESSION_KEY_LEN]>
    {
        let shared = MontgomeryPoint(*pk).mul_clamped(*sk).to_bytes();
        if shared.iter().all(|b| *b == 0) {
            return Err(err!(
                "The X25519 agreement came to zero, which means the public key it was \
                made against is of small order and agrees the same thing with every \
                secret. It is refused rather than used.";
            Invalid, Input, Key));
        }
        Ok(shared)
    }

    /// The session key, over a transcript that names both public keys.
    ///
    /// Both ends put in the same three values in the same order, so an attacker
    /// who substitutes either public key gets a different session key rather
    /// than one the other end will also derive.
    fn derive(
        eph:    &[u8; Self::X25519_PK_LEN],
        theirs: &[u8; Self::X25519_PK_LEN],
        shared: &[u8; Self::X25519_SESSION_KEY_LEN],
    )
        -> Outcome<[u8; Self::X25519_SESSION_KEY_LEN]>
    {
        let hashed = HashScheme::new_sha3_256().hash::<0>(&[
            Self::X25519_KDF_TAG.as_bytes(),
            &eph[..],
            &theirs[..],
            &shared[..],
        ], []);
        match hashed.as_hashform() {
            HashForm::Bytes32(bytes) => Ok(bytes),
            // SHA3-256 gives thirty-two bytes and nothing else reaches here. It
            // is an error rather than a fallback value because the one thing a
            // key derivation must never do quietly is hand back a constant.
            other => Err(err!(
                "SHA3-256 returned {:?} rather than thirty-two bytes, so no session \
                key was derived.", other;
            Bug, Mismatch)),
        }
    }
}
