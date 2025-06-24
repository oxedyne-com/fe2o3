use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
    byte::{
        FromBytes,
        ToBytes,
    },
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    chunk::ChunkConfig,
    daticle::Dat,
    id::NumIdDat,
};
use oxedyne_fe2o3_namex::id::InNamex;


/// Metadata attached to every stored key instance.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Meta<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    pub time:   Timestamp,
    pub user:   UID,
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    FromBytes for Meta<UIDL, UID>
{
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        if buf.len() < Self::BYTE_LEN {
            return Err(<Self as FromBytes>::too_few(
                buf.len(), Self::BYTE_LEN, "Meta", file!(), line!()));
        }
        let (time, _n1) = res!(Timestamp::from_bytes(&buf));
        let (user, _n_uid) = res!(UID::from_bytes(&buf[Timestamp::BYTE_LEN..]));
        Ok((
            Self {
                time,
                user,
            },
            Self::BYTE_LEN,
        ))
    }
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    ToBytes for Meta<UIDL, UID>
{
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf = res!(self.time.to_bytes(buf));
        buf = res!(self.user.to_bytes(buf));
        Ok(buf)
    }
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    Meta<UIDL, UID>
{
    pub const BYTE_LEN: usize = Timestamp::BYTE_LEN + UIDL;

    pub fn new(uid: UID) -> Self {
        Self {
            user: uid,
            ..Default::default()
        }
    }

    pub fn stamp_time_now(&mut self) -> Outcome<()> {
        self.time = res!(Timestamp::now());
        Ok(())
    }

    pub fn clone_now(&self) -> Outcome<Self> {
        let mut result = self.clone();
        result.time = res!(Timestamp::now());
        Ok(result)
    }
}

/// A database can make use of two filters for the key (hash scheme) and the value (encryption
/// scheme).  `RestSchemesOverride` is used to specify an `oxedyne_fe2o3_core::alt::Override` based on
/// these two scheme types for encryption and key hashing (as well as an optional change in the
/// database chunking configuration).  This allows four possibilities for changing the scheme used
/// to write and read individual (k, v) pairs:
/// - a pass-through which defers to the current database-wide scheme,
/// - use of a different variant of the default scheme (e.g. `EncryptionScheme`),
/// - use of a different instance of the scheme given at invocation (e.g. `ENC`),
/// - no scheme (i.e. the identity transformation).
#[derive(Clone, Debug)]
pub struct RestSchemesOverride<
    ENC:    Encrypter,
    KH:     Hasher,
>{
    pub enc:    Override<EncryptionScheme, ENC>,
    pub hash:   Override<HashScheme, KH>,
    pub chnk:   Option<ChunkConfig>,
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
>
    Default for RestSchemesOverride<ENC, KH>
{
    fn default() -> Self {
        Self {
            enc:    Override::PassThrough,
            hash:   Override::PassThrough,
            chnk:   None,
        }
    }
}

impl<
    ENC:    Encrypter,
    KH:     Hasher,
>
    RestSchemesOverride<ENC, KH>
{
    /// Expresses the intention not to override the database defaults.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn encrypter(&self)     -> &Override<EncryptionScheme, ENC> { &self.enc }
    pub fn key_hasher(&self)    -> &Override<HashScheme, KH>        { &self.hash }
    pub fn chunk_config(&self)  -> &Option<ChunkConfig>             { &self.chnk }

    pub fn set_encrypter(mut self, enc: Override<EncryptionScheme, ENC>) -> Self {
        self.enc = enc;
        self
    }
    pub fn set_key_hasher(mut self, hash: Override<HashScheme, KH>) -> Self {
        self.hash = hash;
        self
    }
    pub fn set_chunk_config(mut self, chnk: Option<ChunkConfig>) -> Self {
        self.chnk = chnk;
        self
    }
}

/// A minimal, universal and synchronous (blocking) interface for a database.
///
pub trait Database<
    const UIDL: usize,        // User identifier byte length.
    UID:    NumIdDat<UIDL>,   // User identifier.            
    ENC:    Encrypter,        // Symmetric encryption of data at rest.
    KH:     Hasher,           // Hashes database keys.
>:
    std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    /// Insert a key-value pair of `Daticle`s into the database.  Returns whether the key already
    /// exists, and the number of chunks.
    fn insert(
        &self,
        key:    Dat,
        val:    Dat,
        user:   UID,
        or: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(bool, usize)>;

    /// Return a possible value, along with the key metadata.
    fn get(
        &self,
        key:    &Dat,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<(Dat, Meta<UIDL, UID>)>>;

    /// Deletes the given key and its value from the database, or at least marks it for deletion.
    fn delete(
        &self,
        key:    &Dat,
        user:   UID,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<bool>;
}
