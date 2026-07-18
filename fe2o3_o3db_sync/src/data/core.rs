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

/// A database key in its routed, byte form: either a whole value's key or one
/// chunk of a chunked value, tagged with its chunk index.
#[derive(Clone)]
pub enum Key {
    /// The key of a value stored in a single record.
    Complete(Vec<u8>),
    /// The key of one chunk of a chunked value, carrying its chunk index.
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

/// Zero-sized namespace for the record encoding helpers.
pub struct Encode;

impl Encode {

    /// Serialises a key and value `Dat` pair into their raw byte forms.
    pub fn encode_dat(
        k: Dat,
        v: Dat,
    )
        -> Outcome<(Vec<u8>, Vec<u8>)>
    {
        Ok((res!(k.as_bytes()), res!(v.as_bytes())))
    }

    /// Frames a key-value pair into the stored key and value byte layouts,
    /// stamping the current time into the metadata and appending checksums.
    /// Returns the stored key and value bytes, the chunk index (if any), the
    /// metadata, the cache-bot pool index, and the key and value lengths.
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
    
    /// Returns the key bytes by reference.
    pub fn as_bytes(&self) -> &Vec<u8> {
        match self {
            Self::Complete(byts) => byts,
            Self::Chunk(byts, _) => byts,
        }
    }

    /// Consumes the key and returns its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Complete(byts) => byts,
            Self::Chunk(byts, _) => byts,
        }
    }

    /// Returns the chunk index for a chunk key, or `None` for a complete key.
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

    /// Returns the length in bytes of the key.
    pub fn len(&self) -> usize {
        match self {
            Self::Complete(byts) => byts.len(),
            Self::Chunk(byts, _) => byts.len(),
        }
    }

}

/// A value read back from the store, paired with its metadata.
///
/// The boolean flag records whether the value was observed after a garbage
/// collection pass. A `None` payload signals that the key was not found.
#[derive(Clone, Debug)]
pub enum Value<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    /// A whole value stored in a single record.
    Complete(Option<(Dat, Meta<UIDL, UID>)>, bool),
    /// One chunk of a chunked value, carrying its chunk index.
    Chunk(Option<(Dat, Meta<UIDL, UID>)>, usize, bool),
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    Value<UIDL, UID>
{
    /// Builds a value, choosing the chunk or complete variant depending on
    /// whether a chunk index is supplied.
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

    /// Returns the value payload and metadata by reference, or `None` if the
    /// key was not found.
    pub fn data(&self) -> Option<&(Dat, Meta<UIDL, UID>)> {
        match self {
            Self::Complete(opt, ..) => opt.as_ref(),
            Self::Chunk(opt, ..) => opt.as_ref(),
        }
    }

    /// Returns the chunk index for a chunk value, or `None` for a complete value.
    pub fn index(&self) -> Option<usize> {
        match self {
            Self::Chunk(_, cind, _) => Some(*cind),
            _ => None,
        }
    }
}

/// Optional custom data-at-rest schemes supplied at database invocation. Any
/// field left `None` falls back to the hardwired default scheme.
#[derive(Clone, Debug, Default)]
pub struct RestSchemesInput<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    /// Optional custom encrypter for values at rest.
    pub enc:    Option<ENC>,
    /// Optional custom hasher for keys.
    pub hash:   Option<KH>,
    /// Optional custom pseudo-random hasher for cache distribution.
    pub prnd:   Option<PR>,
    /// Optional custom checksummer for integrity of data at rest.
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
    /// Bundles the optional custom schemes into a single input value.
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

/// The resolved data-at-rest schemes actually used by the database, each
/// expressed as a default-or-alternative wrapper so a call can defer to the
/// default, pick another default variant, or substitute a custom instance.
#[derive(Clone, Debug)]
pub struct RestSchemes<
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    /// Encrypter for values at rest.
    pub enc:    EncrypterDefAlt<EncryptionScheme, ENC>,
    /// Hasher for keys.
    pub hash:   HasherDefAlt<HashScheme, KH>,
    /// Pseudo-random hasher used to distribute values across caches.
    pub prnd:   HasherDefAlt<HashScheme, PR>,
    /// Checksummer for integrity of data at rest.
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
    /// Returns the value encrypter.
    pub fn encrypter(&self)             -> &EncrypterDefAlt<EncryptionScheme, ENC>  { &self.enc }
    /// Returns the key hasher.
    pub fn key_hasher(&self)            -> &HasherDefAlt<HashScheme, KH>            { &self.hash }
    /// Returns the pseudo-random hasher used for cache distribution.
    pub fn pseudorandom_hasher(&self)   -> &HasherDefAlt<HashScheme, PR>            { &self.prnd }
    /// Returns the checksummer.
    pub fn checksummer(&self)           -> &ChecksummerDefAlt<ChecksumScheme, CS>   { &self.csum }

    /// Replaces the key hasher with the given custom instance, returning self.
    pub fn set_key_hasher(mut self, hasher: KH) -> Self {
        self.hash = HasherDefAlt(DefAlt::Given(hasher));
        self
    }
}
