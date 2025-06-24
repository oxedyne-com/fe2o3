//! The database is instantiated with two types for each of the four schemes, e.g.
//! `EncryptionScheme` and `ENC`.  The first is a default Hematite `enum` that provides access to a
//! range of schemes, but is naturally limited.  The second can be a custom type that implements
//! `Encrypter`.  `oxedyne_fe2o3_iop_db::api::RestSchemesOverride` is used to specify an
//! `oxedyne_fe2o3_core::alt::Override` based on these two scheme types for encryption and key hashing (as
//! well as an optional change in the database chunking configuration).  This allows four
//! possibilities for changing the scheme used to write and read individual (k, v) pairs:
//! - a pass-through which defers to the current database-wide scheme,
//! - use of a different variant of the default scheme (e.g. `EncryptionScheme`),
//! - use of a different instance of the scheme given at invocation (e.g. `ENC`),
//! - no scheme (i.e. the identity transformation).
use crate::{
    prelude::*,
    data::cache::KeyVal,
    file::stored::{
        StoredKey,
        StoredValue,
    },
};

use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_iop_crypto::enc::EncrypterDefAlt;
use oxedyne_fe2o3_hash::{
    csum::{
        ChecksummerDefAlt,
        ChecksumScheme,
    },
    hash::{
        HasherDefAlt,
        HashScheme,
    },
};
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    daticle::Dat,
    id::NumIdDat,
};

use std::fmt;

#[derive(Clone)]
pub enum Key {
    Complete(Vec<u8>),
    Chunk(Vec<u8>, usize),
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(v) => write!(f, "Complete({:02x?})", v),
            Self::Chunk(v, i) => write!(f, "Chunk({:02x?}, {})", v, i),
        }
    }
}

pub struct Encode;

impl Encode {

    pub fn encode_dat(
        k: Dat,
        v: Dat,
    )
        -> Outcome<(Vec<u8>, Vec<u8>)>
    {
        Ok((res!(k.as_bytes()), res!(v.as_bytes())))
    }

    pub fn encode<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
        C: Checksummer,
    >(
        mut kv:     KeyVal<UIDL, UID>,
        csummer:    ChecksummerDefAlt<ChecksumScheme, C>,
    )
        -> Outcome<(
            Vec<u8>,
            Vec<u8>,
            Option<usize>,
            Meta<UIDL, UID>,
            usize,
            usize,
            usize,
        )>
    {
        res!(kv.stamp_time_now());
        let KeyVal { key, val, chash, meta, cbpind } = kv;
        // [1.1] Assemble the StoredKey, StoredValue and StoredIndex to be written to file.
        let cind = key.index(); 
        let mut kbuf = key.into_bytes();
        kbuf = res!(StoredKey::build_bytes(chash, kbuf, cind, &meta, csummer.clone()));
        let klen = kbuf.len();
        let mut vbuf = val;
        vbuf = res!(StoredValue::build_bytes(vbuf, csummer));
        let vlen = vbuf.len();
        Ok((kbuf, vbuf, cind, meta, cbpind, klen, vlen))
    }
}

impl Key {
    
    pub fn as_bytes(&self) -> &Vec<u8> {
        match self {
            Self::Complete(byts) => byts,
            Self::Chunk(byts, _) => byts,
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Complete(byts) => byts,
            Self::Chunk(byts, _) => byts,
        }
    }

    pub fn index(&self) -> Option<usize> {
        match self {
            Self::Chunk(_, cind) => Some(*cind),
            _ => None,
        }
    }

    //pub fn stored(&self, chash: alias::ChooseHash, meta: &Meta) -> Outcome<Vec<u8>> {
    //    let kbuf = self.as_bytes().clone();
    //    StoredKey::build_bytes(chash, kbuf, self.index(), meta)
    //}

    pub fn len(&self) -> usize {
        match self {
            Self::Complete(byts) => byts.len(),
            Self::Chunk(byts, _) => byts.len(),
        }
    }

}

#[derive(Clone, Debug)]
pub enum Value<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    Complete(Option<(Dat, Meta<UIDL, UID>)>, bool),
    Chunk(Option<(Dat, Meta<UIDL, UID>)>, usize, bool),
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    Value<UIDL, UID>
{
    pub fn new(
        data:   Option<(Dat, Meta<UIDL, UID>)>,
        cind:   Option<usize>,
        postgc: bool,
    )
        -> Self
    {
        match cind {
            Some(cind) => Self::Chunk(data, cind, postgc), 
            None => Self::Complete(data, postgc),
        }
    }

    pub fn data(&self) -> Option<&(Dat, Meta<UIDL, UID>)> {
        match self {
            Self::Complete(opt, ..) => opt.as_ref(),
            Self::Chunk(opt, ..) => opt.as_ref(),
        }
    }

    pub fn index(&self) -> Option<usize> {
        match self {
            Self::Chunk(_, cind, _) => Some(*cind),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RestSchemesInput<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    pub enc:    Option<ENC>,
    pub hash:   Option<KH>,
    pub prnd:   Option<PR>,
    pub csum:   Option<CS>,
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>
    RestSchemesInput<ENC, KH, PR, CS>
{
    pub fn new(
        enc:    Option<ENC>,
        hash:   Option<KH>,
        prnd:   Option<PR>,
        csum:   Option<CS>,
    )
        -> Self
    {
        Self {
            enc,
            hash,
            prnd,
            csum,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RestSchemes<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    pub enc:    EncrypterDefAlt<EncryptionScheme, ENC>,
    pub hash:   HasherDefAlt<HashScheme, KH>,
    pub prnd:   HasherDefAlt<HashScheme, PR>,
    pub csum:   ChecksummerDefAlt<ChecksumScheme, CS>,
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>
    Default for RestSchemes<ENC, KH, PR, CS>
{
    fn default() -> Self {
        Self {
            enc:    EncrypterDefAlt(DefAlt::None),
            hash:   HasherDefAlt(DefAlt::Default(HashScheme::new_seahash())),
            prnd:   HasherDefAlt(DefAlt::Default(HashScheme::new_seahash())),
            csum:   ChecksummerDefAlt(DefAlt::Default(ChecksumScheme::new_crc32())),
        }
    }
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>
    From<RestSchemesInput<ENC, KH, PR, CS>> for RestSchemes<ENC, KH, PR, CS>
{
    fn from(input: RestSchemesInput<ENC, KH, PR, CS>) -> Self {
        let mut result = Self::default();
        result.enc = EncrypterDefAlt::from(input.enc);
        if input.hash.is_some() { result.hash = HasherDefAlt::from(input.hash); }
        if input.prnd.is_some() { result.prnd = HasherDefAlt::from(input.prnd); }
        if input.csum.is_some() { result.csum = ChecksummerDefAlt::from(input.csum); }
        result
    }
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>
    RestSchemes<ENC, KH, PR, CS>
{
    pub fn encrypter(&self)             -> &EncrypterDefAlt<EncryptionScheme, ENC>  { &self.enc }
    pub fn key_hasher(&self)            -> &HasherDefAlt<HashScheme, KH>            { &self.hash }
    pub fn pseudorandom_hasher(&self)   -> &HasherDefAlt<HashScheme, PR>            { &self.prnd }
    pub fn checksummer(&self)           -> &ChecksummerDefAlt<ChecksumScheme, CS>   { &self.csum }

    pub fn set_key_hasher(mut self, hasher: KH) -> Self {
        self.hash = HasherDefAlt(DefAlt::Given(hasher));
        self
    }
}
