//! Raw cryptographic key types: [`SecretKey`], [`PublicKey`] and the
//! generic [`Keys`] pair.
//!
//! The multi-admin encrypted keystore (formerly co-located here) lives
//! in the sibling [`crate::keystore`] module.

use crate::scheme::SchemeTimestamp;

use oxedyne_fe2o3_core::{
    prelude::*,
    mem::Extract,
    rand::RanDef,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
};
use oxedyne_fe2o3_namex::id::LocalId as SchemeLocalId;

use std::fmt;

use rand_core::{
    OsRng,
    RngCore,
};

use secrecy::{
    ExposeSecret,
    Secret,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SECRET KEY                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A private key with `SchemeTimestamp` metadata.  A heap `Vec` is used since the key accomodates
/// different schemes, so the length of the key is not known at compile time.
pub struct SecretKey {
    pub key: Secret<Vec<u8>>,
    pub sts: SchemeTimestamp,
}

impl Clone for SecretKey {
    fn clone(&self) -> Self {
        Self {
            key: {
                let sk = self.key.expose_secret();
                Secret::new(sk.clone())
            },
            sts: self.sts.clone(),
        }
    }
}

impl Default for SecretKey {
    fn default() -> Self {
        Self {
            key: Secret::new(Vec::new()),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sk = self.key.expose_secret();
        write!(f, "SecretKey{{ key: [{}] bytes, sts: {:?}, k}}", sk.len(), self.sts)
    }
}

impl ToDat for SecretKey {
    fn to_dat(&self) -> Outcome<Dat> {
        let sk = self.key.expose_secret();
        Ok(tup2dat![
            Dat::bytdat(sk.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for SecretKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = SecretKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = Secret::new(try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64));
        result.sts =  res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl SecretKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Secret<Vec<u8>>,
    )
        -> Self
    {
        Self {
            sts,
            key,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Secret<Vec<u8>>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PUBLIC KEY                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A public key with `SchemeTimestamp` metadata, stored on the heap.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct PublicKey {
    pub sts: SchemeTimestamp, // Derived ordering starts with the first field here.
    pub key: Vec<u8>,
}

impl Default for PublicKey {
    fn default() -> Self {
        Self {
            key: Vec::new(),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey{{ key: [{}] bytes, sts: {:?}, k}}", self.key.len(), self.sts)
    }
}

impl ToDat for PublicKey {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            Dat::bytdat(self.key.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for PublicKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = PublicKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64);
        result.sts = res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl PublicKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Vec<u8>,
    )
        -> Self
    {
        Self {
            key,
            sts,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Vec<u8>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ KEYS PAIR                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// An optional public and private key pair with no additional metadata.
#[derive(Default)]
pub struct Keys<
    const PK_LEN: usize,
    const SK_LEN: usize,
> {
    pub pk:     Option<[u8; PK_LEN]>,
    pub sks:    Option<Secret<[u8; SK_LEN]>>,
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Clone for Keys<PK_LEN, SK_LEN>
{
    fn clone(&self) -> Self {
        Self {
            pk: self.pk.clone(),
            sks: match &self.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(Secret::new(sk.clone()))
                },
                None => None,
            },
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    RanDef for Keys<PK_LEN, SK_LEN>
{
    fn randef() -> Self {
        let mut pk = [0u8; PK_LEN];
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut pk);
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: Some(pk),
            sks: Some(Secret::new(sk)),
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Keys<PK_LEN, SK_LEN>
{
    pub fn new(
        pk: Option<[u8; PK_LEN]>,
        sks: Option<Secret<[u8; SK_LEN]>>,
    )
        -> Self
    {
        Self {
            pk,
            sks,
        }
    }

    pub fn randef_sk_only() -> Self {
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: None,
            sks: Some(Secret::new(sk)),
        }
    }

    pub fn randef_pk_only() -> Self {
        let mut pk = [0u8; PK_LEN];
        OsRng.fill_bytes(&mut pk);
        Self {
            pk: Some(pk),
            sks: None,
        }
    }
}
