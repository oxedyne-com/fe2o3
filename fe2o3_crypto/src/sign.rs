use crate::{
    keys::Keys,
    pqc::dilithium as dilithium2_fe2o3,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::Signer,
};
use oxedize_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

use std::{
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

use pqcrypto_dilithium::dilithium2;
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
    Dilithium2(Keys< // Wrapper reference c impl.
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
            Self::Dilithium2(..) =>
                res!(NamexId::try_from("W4+qt2Gd+9RQBxllcx10b4h/Ih3g9m76C+mj17TwUNw=")),
            Self::Dilithium2_fe2o3(..) =>
                res!(NamexId::try_from("zkSGGwLauv5FLpNoCse+3D7bKIdNh7PeBsfbjv/TSvQ=")),
        })
    }

    fn local_id(&self) -> LocalId {
	    match self {
            Self::Ed25519(..)           => LocalId(1),
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
            _ => return Err(err!(errmsg!(
                "The Namex group name '{}' is not recognised for SignatureScheme.", gname,
            ), Invalid, Input)),
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
                        return Err(err!(errmsg!("Public key mismatch."), Invalid, Configuration));
                    }
                    let result = signing_key.sign(msg).to_bytes().to_vec();
                    Ok(result)
                },
                _ => Err(err!(errmsg!("Require both keys to sign."), Missing, Configuration)),
            },
            Self::Dilithium2(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let skv = sks.expose_secret(); // This gets zeroized automatically, ...
                    let mut sk = res!(dilithium2::SecretKey::from_bytes(&skv[..])); // this does not, so...
                    let result = dilithium2::detached_sign(msg, &sk).as_bytes().to_vec();
                    sk = res!(dilithium2::SecretKey::from_bytes(&vec![0; skv.len()])); // do it manually.
                    Ok(result)
                },
                _ => Err(err!(errmsg!("Require secret key to sign."), Missing, Configuration)),
            },
            Self::Dilithium2_fe2o3(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let skv = sks.expose_secret();
                    let mut sk = res!(<[u8; Self::DILITHIUM2_FE2O3_SK_LEN]>::try_from(&skv[..]));
                    let result = dilithium2_fe2o3::sign::sign(msg, &sk).to_vec();
                    sk.zeroize();
                    Ok(result)
                },
                _ => Err(err!(errmsg!("Require secret key to sign."), Missing, Configuration)),
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
                _ => return Err(err!(errmsg!("Require public key to verify."), Missing, Configuration)),
            },
            Self::Dilithium2(keys) => match keys {
                Keys { pk: Some(pk), .. } => { 
                    let pk = res!(dilithium2::PublicKey::from_bytes(&pk[..]));
                    let sig = res!(dilithium2::DetachedSignature::from_bytes(&sig));
                    match dilithium2::verify_detached_signature(&sig, msg, &pk) {
                        Ok(()) => true,
                        _ => false,
                    }
                },
                _ => return Err(err!(errmsg!("Require public key to verify."), Missing, Configuration)),
            },
            Self::Dilithium2_fe2o3(keys) => match keys {
                Keys { pk: Some(pk), .. } => { 
                    let pk = res!(<[u8; Self::DILITHIUM2_FE2O3_PK_LEN]>::try_from(&pk[..]));
                    let sig = res!(<[u8; Self::DILITHIUM2_FE2O3_SIG_LEN]>::try_from(&sig[..]));
                    dilithium2_fe2o3::sign::verify(msg, &sig, &pk)
                },
                _ => return Err(err!(errmsg!("Require public key to verify."), Missing, Configuration)),
            },
        })
    }
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
                    Some(sk) => Some(Secret::new(res!(<[u8; Self::ED25519_SK_LEN]>::try_from(&sk[..])))),
                    None => None,
                },
            }),
            Self::Dilithium2(..) => Self::Dilithium2(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(<[u8; Self::DILITHIUM2_PK_LEN]>::try_from(&pk[..]))),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(<[u8; Self::DILITHIUM2_SK_LEN]>::try_from(&sk[..])))),
                    None => None,
                },
            }),
            Self::Dilithium2_fe2o3(..) => Self::Dilithium2_fe2o3(Keys {
                pk: match pk {
                    Some(pk) => Some(res!(<[u8; Self::DILITHIUM2_FE2O3_PK_LEN]>::try_from(&pk[..]))),
                    None => None,
                },
                sks: match sk {
                    Some(sk) => Some(Secret::new(res!(<[u8; Self::DILITHIUM2_FE2O3_SK_LEN]>::try_from(&sk[..])))),
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
            "Dilithium2" => res!(Self::new_dilithium2()),
            "Dilithium2_fe2o3" => Self::new_dilithium2_fe2o3(),
            _ => return Err(err!(errmsg!(
                "The signature scheme '{}' is not recognised.", name,
            ), Invalid, Input)),
        })
    }
}

impl TryFrom<&LocalId> for SignatureScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: &LocalId) -> std::result::Result<Self, Self::Error> {
        Ok(match *n {
            LocalId(1) => Self::new_ed25519(),
            LocalId(2) => res!(Self::new_dilithium2()),
            LocalId(3) => Self::new_dilithium2_fe2o3(),
            _ => return Err(err!(errmsg!(
                "The signature scheme with local id {} is not recognised.", n,
            ), Invalid, Input)),
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
    pub const DILITHIUM2_PK_LEN:        usize = dilithium2::public_key_bytes();
    pub const DILITHIUM2_SK_LEN:        usize = dilithium2::secret_key_bytes();
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
