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

/// Options controlling a [`Database::scan`] invocation.
///
/// All fields have sensible defaults via [`Default`], so the common case
/// "give me every entry" is `ScanOpts::default()`.
///
/// # Semantics
///
/// - `prefix`: restrict the returned entries to those whose key matches
///   the given `Dat`. For `Dat::Str(s)` prefixes the match is a string
///   prefix. For other `Dat` variants the match is equality. `None`
///   returns every entry.
/// - `limit`: cap the returned vector at this many entries. `None` means
///   no cap; callers that don't trust the database size should always
///   set a bound.
/// - `include_values`: when `true` the scan reads each value payload
///   from disk (and decrypts if the database is encrypted). When
///   `false` the returned values are `Dat::Empty`, but keys and `Meta`
///   are still populated; this mode is much cheaper and is the right
///   choice when the caller is paginating keys for display and will
///   fetch values individually on demand.
#[derive(Clone, Debug, Default)]
pub struct ScanOpts {
    /// Optional key prefix filter.
    pub prefix:         Option<Dat>,
    /// Optional hard cap on the returned entry count.
    pub limit:          Option<usize>,
    /// Whether to read and decrypt value payloads.
    pub include_values: bool,
}

impl ScanOpts {
    /// Construct a scan-everything options value.
    pub fn all() -> Self {
        Self::default()
    }

    /// Shorthand for "scan every entry whose key is a `Dat::Str` starting
    /// with `prefix`".
    pub fn with_str_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix:         Some(Dat::Str(prefix.into())),
            limit:          None,
            include_values: false,
        }
    }

    /// Fluent setter for `include_values`.
    pub fn include_values(mut self, yes: bool) -> Self {
        self.include_values = yes;
        self
    }

    /// Fluent setter for `limit`.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
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

    /// Walk the database, returning `(key, value, meta)` triples for
    /// every entry that satisfies `opts`.
    ///
    /// The scan is a best-effort point-in-time snapshot: entries
    /// written after the scan begins may or may not appear; entries
    /// deleted or overwritten mid-scan return their latest live state
    /// (stale versions are filtered via the database's own
    /// reconciliation -- implementations must not return superseded
    /// values).
    ///
    /// `or` behaves as for [`Database::get`], supplying a per-call
    /// override of the encryption and hashing schemes. Scans with
    /// `include_values == true` use it to decrypt each value; scans
    /// with `include_values == false` leave the `or` argument alone.
    ///
    /// # Cost and scale
    ///
    /// Scan is expected to be O(database size); it is not a
    /// low-latency operation. Callers that only need a handful of
    /// entries matching a tight prefix should still bound the scan
    /// via `opts.limit` because implementations are free to evaluate
    /// the prefix filter after the walk rather than during it.
    fn scan(
        &self,
        opts:   &ScanOpts,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>;
}
