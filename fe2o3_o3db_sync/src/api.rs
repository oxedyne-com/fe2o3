use crate::{
    prelude::*,
    test::hooks,
    base::{
        constant,
        id::{
            self,
            OzoneBotId,
        },
        index::{
            WorkerInd,
            ZoneInd,
        },
    },
    bots::{
        bot_zone::ZoneState,
        worker::{
            bot::WorkerType,
            bot_file::{
                Hold,
                GcControl,
            },
            bot_reader::ReadResult,
        },
    },
    comm::{
        channels::{
            BotChannels,
            ChooseBot,
            OzoneMsgCount,
        },
        msg::OzoneMsg,
        response::{
            Responder,
            Wait,
        },
    },
    data::{
        cache::{
            CacheEntry,
            KeyVal,
        },
        choose::ChooseCache,
        core::{
            Encode,
            Key,
            RestSchemes,
            Value,
        },
    },
    file::{
        core::FileEntry,
        floc::FileNum,
        state::{
            FileState,
            FileStateMap,
        },
        zdir::ZoneDir,
    },
};

use oxedyne_fe2o3_core::channels::Recv;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
    id::NumIdDat,
};
use oxedyne_fe2o3_hash::{
    csum::{
        ChecksummerDefAlt,
        ChecksumScheme,
    },
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_hash::api::HashForm;
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_namex::id::{
    InNamex,
    NamexId,
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        Instant,
    },
};


// What `OzoneApi::compact_now` did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompactReport {
    pub files_collected:    usize,  // files a collection was started on
    pub files_deleted:      usize,  // files removed for holding only old records
    pub bytes_before:       u64,    // length of the zone directories before the call rolled the live files
    pub bytes_after:        u64,    // the same once nothing sealed held an old byte
}

#[derive(Clone, Debug)]
pub struct OzoneApi<
    // Data at rest.
    const UIDL: usize,        // User id byte length.
    UID:    NumIdDat<UIDL>,   // User id.
    ENC:    Encrypter,        // Symmetric encryption of data at rest.
    KH:     Hasher,           // Hashes database keys.
	PR:     Hasher,           // Pseudo-randomiser hash to distribute cache data.
    CS:     Checksummer,      // Checks integrity of data at rest.
>{
    pub ozid:       OzoneBotId,
    pub db_root:    PathBuf,
    pub cfg:        OzoneConfig,
    pub chans:      BotChannels<UIDL, UID, ENC, KH>,
    pub schms:      RestSchemes<ENC, KH, PR, CS>,
}

/// The `'static` requirement for UID, which propagates through the code base, is initially driven
/// by the channel send methods in this implementation.
impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    OzoneApi<UIDL, UID, ENC, KH, PR, CS>
{
    /// Create a new Ozone database API instance.
    pub fn new(
        ozid:       OzoneBotId,
        db_root:    PathBuf,
        cfg:        OzoneConfig,
        chans:      BotChannels<UIDL, UID, ENC, KH>,
        schms:      RestSchemes<ENC, KH, PR, CS>,
    )
        -> Self
    {
        Self {
            ozid,
            db_root,
            cfg,
            chans,
            schms,
        }
    }

    pub fn ozid(&self)          -> &OzoneBotId                      { &self.ozid }
    pub fn db_root(&self)       -> &Path                            { &self.db_root }
    pub fn cfg(&self)           -> &OzoneConfig                     { &self.cfg }
    pub fn schemes(&self)       -> &RestSchemes<ENC, KH, PR, CS>    { &self.schms }
    pub fn chans(&self)         -> &BotChannels<UIDL, UID, ENC, KH> { &self.chans }

    // Convenience.
    pub fn responder(&self) -> Responder<UIDL, UID, ENC, KH> { Responder::new(Some(&self.ozid())) }
    pub fn no_responder()   -> Responder<UIDL, UID, ENC, KH> { Responder::none(None) }

    // Key API.
    
    /// Prepare a key for the Ozone database.
    ///
    /// # Arguments
    /// * `k` - key `Dat` to be transformed into an Ozone key.
    /// * `enc` - optional `Encypter` which can contain a `Hasher`.  If this exists, this is the
    /// hash function that is used instead of the default.
    ///
    /// Returns the key bytes, the zone and a hash used to deterministically select bots.
    ///
    /// # Local errors
    /// * The encoded key length cannot be zero.
    pub fn ozone_key_dat(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(Vec<u8>, WorkerInd, alias::ChooseHash)>
    { 
        self.ozone_key(res!(k.as_bytes()), schms2)
    }
    
    /// Derives the chunk set identifier for a value from its key bytes.  A chunked value's chunk
    /// records are addressed by `Tup5u64([set_id, index, ..])`; deriving `set_id` from the key --
    /// rather than from a fresh random ticket per operation -- makes a later overwrite of the same
    /// key write its chunks under the same addresses, so the ordinary supersession path flags the
    /// superseded chunk records old and the collector reclaims them.  Seahash gives a stable
    /// 64-bit value across runs and builds; the dedicated salt keeps it distinct from the routing
    /// hash.  The collision probability matches the random ticket it replaces (~2^-64).
    pub fn chunk_set_id(kbuf: &[u8]) -> u64 {
        match HashScheme::new_seahash().hash(&[kbuf], constant::CHUNK_SET_ID_SALT).as_hashform() {
            HashForm::U64(h)    => h,
            // Seahash always yields a U64; fold any other form defensively into one.
            other               => {
                let v = other.as_vec();
                let mut buf = [0u8; 8];
                for (i, b) in v.iter().take(8).enumerate() {
                    buf[i] = *b;
                }
                u64::from_be_bytes(buf)
            },
        }
    }

    pub fn ozone_key(
        &self,
        kbuf:   Vec<u8>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(Vec<u8>, WorkerInd, alias::ChooseHash)>
    {
        self.keygen(
            kbuf,
            schms2,
            self.cfg().num_zones,
            self.cfg().num_cbots_per_zone,
        )
    }

    pub fn keygen(
        &self,
        kbuf:   Vec<u8>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        nz:     u16,
        nc:     u16,
    )
        -> Outcome<(Vec<u8>, WorkerInd, alias::ChooseHash)>
    {
        if kbuf.len() == 0 {
            return Err(err!("Key has length zero."; Input, Invalid));
        }

        // Compute the routing hash. This is *only* a routing signal
        // used to select the owning cbot and zone -- it does not
        // become the stored key form. The bytes that flow through
        // the rest of the write path, into the cache, and onto disk
        // are the original plaintext `kbuf`, so a later `scan` can
        // recover the user's original `Dat` key by parsing those
        // bytes with `Dat::from_bytes`.
        let hash = self.schemes().key_hasher()
            .or_hash(&[&kbuf], constant::KEY_HASH_SALT, schms2.map(|s| s.key_hasher()))
            .as_hashform();
        let (cbwind, chash) = res!(ChooseCache::<PR>::choose_cbot(
            &hash,
            nz,
            nc,
        ));

        Ok((
            kbuf,
            cbwind,
            chash,
        ))
    }

    // Write API, for general public use.
    
    /// Insert key-value `Dat`icles using the given data scheme overrides.  A `Responder` channel
    /// is returned, carrying the answers `store_dat_using_responder` describes; wait on them with
    /// `Responder::recv_store_ack`.
    ///
    /// # Arguments
    /// * `k` - key `Dat`cle.
    /// * `enc` - An optional `EncryptionScheme` that was used to store the value.  An error will
    /// be returned if the decryption does not yield a valid `Dat`icle.
    ///
    pub fn put(
        &self,
        key:    Dat,
        val:    Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        let sbots = self.chans().all_sbots();
        let (bot, bpind) = sbots.choose_bot(&ChooseBot::Randomly);
        match bot.send(OzoneMsg::Put {
            key,
            val,
            user,
            schms2: schms2.cloned(),
            resp:   resp.clone(),
        }) {
            Err(e) => Err(err!(e,
                "{}: While sending put request to sbot {}.", self.ozid(), bpind;
                Channel, Write)),
            _ => Ok(resp),
        }
    }

    // Write API, high level, used by ServerBots.
    
    /// The simplest storage entry point.  A default responder is automatically returned, which can
    /// be used to gain feedback on the operation (i.e. when it is successfully completed, and
    /// whether the key was present).
    ///
    /// # Arguments
    /// * `k` - key, a reference to a `Dat`.
    /// * `v` - value, as any type that can be converted to a `Dat` via `From`.
    /// * `user` - `User` number responsible for request.
    ///
    /// Returns a default `Responder` that contains the number of chunks (0 if not chunked).
    pub fn store(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        res!(self.store_dat_using_responder(k, v, user, None, resp.clone()));
        Ok(resp)
    }

    /// Store using data schemes overrides.  A default responder is automatically returned, which
    /// can be used to gain feedback on the operation (i.e. when it is successfully completed, and
    /// whether the key was present).
    ///
    /// # Arguments
    /// * `k` - key, a reference to a `Dat`.
    /// * `v` - value, as any type that can be converted to a `Dat` via `From`.
    /// * `user` - `User` number responsible for request.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    ///
    /// Returns a default `Responder` that contains the number of chunks (0 if not chunked).
    pub fn store_using_schemes(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        res!(self.store_dat_using_responder(k, v, user, schms2, resp.clone()));
        Ok(resp)
    }

    /// Store a value without a responder to provide feedback on the operation.
    ///
    /// # Arguments
    /// * `k` - key, a reference to a `Dat`.
    /// * `v` - value, as any type that can be converted to a `Dat` via `From`.
    /// * `user` - `User` number responsible for request.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    pub fn store_blindly(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<()>
    {
        let resp = Self::no_responder();
        res!(self.store_dat_using_responder(k, v, user, schms2, resp));
        Ok(())
    }

    /// Store value with a default responder and a custom chunk size.  The encoded value will be
    /// chunked if its size exceed the specified chunk size.
    ///
    /// # Arguments
    /// * `k` - key, a reference to a `Dat`.
    /// * `v` - value, as any type that can be converted to a `Dat` via `From`.
    /// * `user` - `User` number responsible for request.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing,
    /// encryption), including the chunker confgiuration.
    ///
    /// Returns a default `Responder` that contains the number of chunks (0 if not chunked).
    pub fn store_chunked(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(Responder<UIDL, UID, ENC, KH>, usize)>
    {
        let resp = self.responder();
        let num_chunks = res!(self.store_dat_using_responder(k, v, user, schms2, resp.clone()));
        Ok((resp, num_chunks))
    }

    /// The primary method for storing a key-value pair.
    ///
    /// # Data chunking
    /// Large values are chunked and spread across multiple zones.  The key points to a "bunch key"
    /// which contains the essential chunk information.  The bunch key must have an index of 0, and
    /// the chunks are indexed from 1.  Acquiring the bunch key thus allows all chunk keys to be
    /// reconstructed and retrieved.  The chunk values are stored as raw bytes wrapped in a
    /// `Dat::BU64`.  Chunking is performed when the size of the value to be stored exceeds
    /// some fraction of the maximum data file size, which can be customised via the given
    /// `Responder`.  Without encryption, the final chunk may be smaller than the rest.  With
    /// encryption this final chunk is padded with random bytes to the uniform size.
    ///
    /// ```ignore
    /// How data chunks are stored:
    ///                                                                    +-- this is the
    ///                                                                    |   "bunch key"
    /// "Store 'hello' -> 100 bytes" where chunk size is 30 bytes...       |
    ///                                                                    v
    ///  +---------------------------------------+          +---------------------------------------+
    ///  |            Dat::Str("hello")          | -------> |     PartKey((<id>, 0, 100, 4, 30))    |
    ///  +---------------------------------------+          +---------------------------------------+
    ///  +---------------------------------------+          +---------------------------------------+
    ///  |     PartKey((<id>, 1, 100, 4, 30))    | -------> |          Dat::BU64(<30 bytes>)        |
    ///  +---------------------------------------+          +---------------------------------------+
    ///  +---------------------------------------+          +---------------------------------------+
    ///  |     PartKey((<id>, 2, 100, 4, 30))    | -------> |          Dat::BU64(<30 bytes>)        |
    ///  +---------------------------------------+          +---------------------------------------+
    ///  +---------------------------------------+          +---------------------------------------+
    ///  |     PartKey((<id>, 3, 100, 4, 30))    | -------> |          Dat::BU64(<30 bytes>)        |
    ///  +---------------------------------------+          +---------------------------------------+
    ///  +---------------------------------------+          +---------------------------------------+
    ///  |     PartKey((<id>, 4, 100, 4, 30))    | -------> |          Dat::BU64(<10 bytes>)        |
    ///  +---------------------------------------+          +---------------------------------------+
    ///
    ///  ```
    /// Placing chunks inside (unencrypted) `Dat::BU64` wrappers allows the size to be known
    /// during cache initialisation if there is missing index file data.
    ///
    /// # Arguments
    /// * `k` - key, a reference to a `Dat`.
    /// * `v` - an owned value, any type that can be converted to a `Dat` via `From`.
    /// * `user` - `User` number responsible for request.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    /// * `resp` - a `Responder` channel.
    ///
    /// Returns the number of chunks.  The first message in the `Responder` will be an
    /// `OzoneMsg::Chunks` containing the number of chunks.  Each record written is then answered
    /// twice: `OzoneMsg::Written` once it is appended, and a final answer once it is durable under
    /// the sync policy and readable.  Without chunking the final answer is an `OzoneMsg::KeyExists`
    /// saying whether the key was present; with chunking it is an `OzoneMsg::KeyChunkExists` for
    /// the bunch key (with index 0) and for each chunk.  The answers to different records arrive
    /// in no particular order.  A writer that fails sends `OzoneMsg::Error` instead.
    /// `Responder::recv_store_ack` waits on all of it, each stage to its own deadline.
    ///
    /// # Local errors
    /// * The key must be transformable into an Ozone key.
    /// * The encoded value length must exceed zero.  This should not occur.
    /// * The chunk size in the responder cannot be zero.
    pub fn store_dat_using_responder(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<usize>
    {
        // A store of the deleted marker is a delete by another road, the one the distributed
        // adapters erase by, and it supersedes only the bunch key.  The chunks of a chunked value
        // live under keys of their own, so they are retired here, as `delete` retires them, or
        // their bytes would stay live in the data files for ever.
        let deleted = matches!(&v, Dat::Usr(kind, _) if *kind == id::usr_kind_id_deleted());
        // Any other store over a chunked value leaves the old chunks live too, unless the new value
        // writes the very keys they have.  A chunk key carries the length, count and size of its
        // value as well as the set identifier, so a value of another length shares none of them,
        // and the ordinary supersession never reaches them.  They are retired here, once the new
        // value is durable (below), and not before: retired first, a crash would leave the old
        // bunch key naming chunks that are gone.
        let old = if deleted { None } else { res!(self.chunk_set_of(&k, schms2)) };
        let (kbuf, vbuf) = res!(Encode::encode_dat(k.clone(), v));
        let (mut msgs, datkeys) = res!(self.prepare_write_keyed(kbuf, vbuf, user, schms2, resp.clone(), None));
        let nchunks = msgs.len();
        // Every tombstone this store sends is stamped with the store's own time (see
        // `tombstone_chunk_key_at`).
        let meta = res!(Self::write_meta(&msgs));
        let stale = match &old {
            Some(pkey) => Self::chunk_keys_of(pkey).into_iter().any(|ck| !datkeys.contains(&ck)),
            None => false,
        };
        let pkey = match old {
            Some(pkey) if stale => pkey,
            _ => {
                // The marker goes out beside the tombstones of the chunks it leaves, and their
                // answers are awaited once all are sent.
                let retiring = if deleted {
                    res!(self.reclaim_chunks_on_delete(&k, &meta, schms2))
                } else {
                    None
                };
                if resp.is_some() {
                    res!(resp.send(OzoneMsg::Chunks(nchunks)));
                }
                res!(self.store_bytes(msgs));
                if let Some((pkey, waits)) = retiring {
                    if let Err(e) = self.await_retired(&k, &pkey, waits) {
                        let e = err!(e,
                            "{}: Deleting {:?}: its tombstones were sent together, and the bunch \
                            key may have been deleted although a chunk was not confirmed retired.  \
                            The delete can be repeated while the bunch key is live, and chunks \
                            left behind wait for the orphan sweep.",
                            self.ozid(), k;
                            Write);
                        if resp.is_none() {
                            return Err(e);
                        }
                        res!(resp.send(OzoneMsg::Error(e)));
                    }
                }
                return Ok(nchunks);
            },
        };

        // The write is answered to a responder of our own, so that the old chunks can be retired
        // between its durability and the caller's answer.  The caller is then given the same
        // answers it would have had from the writers.
        if resp.is_some() {
            res!(resp.send(OzoneMsg::Chunks(nchunks)));
        }
        let own = self.responder();
        for (msg, _) in msgs.iter_mut() {
            if let OzoneMsg::Write { resp, .. } = msg {
                *resp = own.clone();
            }
        }
        res!(self.store_bytes(msgs));
        let acks = own.recv_write_acks(
            nchunks,
            constant::USER_REQUEST_TIMEOUT,
            constant::DURABILITY_TIMEOUT,
        );
        match acks {
            Err(e) => {
                // Passed on with its tags, as the writers would have sent it.
                if resp.is_none() {
                    return Err(e);
                }
                res!(resp.send(OzoneMsg::Error(e)));
            },
            Ok(acks) => {
                if let Err(e) = self.retire_chunks(&k, &pkey, &datkeys, &meta, schms2) {
                    // The new value is stored and durable; the old chunks are left to the orphan
                    // sweep, and the caller is not told its write failed.
                    warn!(sync_log::stream(),
                        "{}: The value stored at {:?} is durable, but the chunks of the value it \
                        replaced were not all retired, and wait for the orphan sweep: {}",
                        self.ozid(), k, e);
                }
                if resp.is_some() {
                    for _ in 0..nchunks {
                        res!(resp.send(OzoneMsg::Written));
                    }
                    for ack in acks {
                        res!(resp.send(ack));
                    }
                }
            },
        }
        Ok(nchunks)
    }

    /// Store forcing the chunk set identifier rather than deriving it from the key.  Test and
    /// migration support: it reproduces a value as an earlier build wrote it (a random
    /// per-operation set_id), so that reads of such a value can be exercised after the switch to
    /// key-derived identifiers.  Production writes never take this path.
    pub fn store_dat_using_responder_forcing_set_id(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        set_id: u64,
    )
        -> Outcome<usize>
    {
        let msgs = res!(self.prepare_write_dat(
            k,
            v,
            user,
            schms2,
            resp.clone(),
            Some(set_id),
        ));
        let nchunks = msgs.len();
        if resp.is_some() {
            res!(resp.send(OzoneMsg::Chunks(nchunks)));
        }
        res!(self.store_bytes(msgs));
        Ok(nchunks)
    }

    /// The key and value `Dat`icles are serialised here and then sent for final processing.  A
    /// `set_id_override` of `None` derives the chunk set identifier from the key (the ordinary
    /// path); `Some` forces it, for reproducing an earlier build's random-keyed values.
    pub fn prepare_write_dat(
        &self,
        k:              Dat,
        v:              Dat,
        user:           UID,
        schms2:         Option<&RestSchemesOverride<ENC, KH>>,
        resp:           Responder<UIDL, UID, ENC, KH>,
        set_id_override: Option<u64>,
    )
        -> Outcome<Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>>
    {
        //let (kbuf, vbuf) = res!(Encode::encode_dat(k, v));
        let (kbuf, vbuf) = res!(Encode::encode_dat(k.clone(), v.clone()));
        self.prepare_write(
            kbuf,
            vbuf,
            user,
            schms2,
            resp,
            set_id_override,
        )
    }

    /// This is the last step in preparing the serialised data for storage, where
    /// `RestSchemesOverride` is finally invoked.  This influences how the key is hashed, if and
    /// how the data is chunked, and if and how those chunks are encrypted.  `OzoneMsg`s are
    /// returned, ready for sending to `WriteBot`s.
    pub fn prepare_write(
        &self,
        k:          Vec<u8>,
        vbuf:       Vec<u8>,
        user:       UID,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        resp:       Responder<UIDL, UID, ENC, KH>,
        set_id_override: Option<u64>,
    )
        -> Outcome<Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>>
    {
        let (msgs, _) = res!(self.prepare_write_keyed(k, vbuf, user, schms2, resp, set_id_override));
        Ok(msgs)
    }

    /// `prepare_write`, and with the messages the keys of the records they write beyond the main
    /// key: the bunch key and then each chunk's, none for a value that is not chunked.
    fn prepare_write_keyed(
        &self,
        k:          Vec<u8>,
        mut vbuf:   Vec<u8>,
        user:       UID,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        resp:       Responder<UIDL, UID, ENC, KH>,
        set_id_override: Option<u64>,
    )
        -> Outcome<(Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>, Vec<Dat>)>
    {
        if vbuf.len() == 0 {
            return Err(err!(
                "{}: For key {:?}, the given value encoded length is zero.",
                self.ozid(), k;
                Input, Invalid));
        }

        // 1. Normalise the key.
        let (kbuf, cbwind, chash) = res!(self.ozone_key(k, schms2));

        // 3. Define chunking.
        let chunk_config = match schms2 {
            Some(schms2) => match schms2.chunk_config() {
                Some(cfg) => cfg.clone(),
                None => self.cfg().chunk_config(),
            },
            None => self.cfg().chunk_config(),
        };
        let chunk_threshold = chunk_config.threshold_bytes;

        let encryption_on = !(self.schemes().encrypter().or_is_identity(schms2.map(|s| s.encrypter())));
        debug!(sync_log::stream(), "Encryption is on: {}", encryption_on);
        if encryption_on {
            vbuf = res!(
                self.schemes().encrypter().or_encrypt(&mut vbuf, schms2.map(|s| s.encrypter()))
            ); 
        }

        let mut msgs = Vec::new();
        let mut keys = Vec::new();
        let mut meta = Meta::new(user);
        res!(meta.stamp_time_now());

        // 4. Package the value, breaking into chunks if it is too big.
        if vbuf.len() >= chunk_threshold {
            let chunker = OzoneConfig::chunker(chunk_config);
            // 4.1 Chunk data.
            let (chunks, chunk_state) = res!(chunker.chunk(&vbuf));
            // Address the chunks by a key-derived identifier, not the per-operation ticket, so an
            // overwrite of the same key supersedes the prior value's chunk records in place.  A
            // forced identifier (test/migration only) reproduces an earlier build's random keys.
            let set_id = match set_id_override {
                Some(id)    => id,
                None        => Self::chunk_set_id(&kbuf),
            };
            let datkeys = res!(chunker.keys(set_id, &chunk_state));
            keys = datkeys.clone();
            
            // 4.2 Store main key -> bunch key.
            let mut bkbuf = res!(datkeys[0].as_bytes());
            if encryption_on {
                bkbuf = res!(self.schemes().encrypter().or_encrypt(&bkbuf, schms2.map(|s| s.encrypter()))); 
                bkbuf = res!(Dat::wrap_bytes_var(bkbuf));
            }
            msgs.push((
                res!(Self::package_write(
                    KeyVal {
                        key:    Key::Chunk(kbuf, 0),
                        val:    bkbuf,
                        chash,
                        meta:   meta.clone(),
                        cbpind: **cbwind.bpind(),
                    },
                    resp.clone(),
                    self.schemes().checksummer().clone(),
                )),
                *cbwind.zind(),
            ));

            // 4.3 Store chunk keys -> chunk bytes.
            for (i, chunk) in chunks.into_iter().enumerate() {
                let (ckbuf, ccbwind, cchash) =
                    res!(self.ozone_key_dat(&datkeys[i+1], schms2));
                msgs.push((
                    res!(Self::package_write(
                        KeyVal {
                            key:    Key::Chunk(ckbuf, i + 1),
                            val:    chunk,
                            chash:  cchash,
                            meta:   meta.clone(),
                            cbpind: **ccbwind.bpind(),
                        },
                        resp.clone(),
                        self.schemes().checksummer().clone(),
                    )),
                    *ccbwind.zind(),
                ));
            }
        } else {
            // 3.1 No chunking, just a single block of data.
            if encryption_on {
                vbuf = res!(Dat::wrap_bytes_var(vbuf));
            }
            msgs.push((
                res!(Self::package_write(
                    KeyVal {
                        key:    Key::Complete(kbuf),
                        val:    vbuf,
                        chash,
                        meta:   meta.clone(),
                        cbpind: **cbwind.bpind(),
                    },
                    resp,
                    self.schemes().checksummer().clone(),
                )),
                *cbwind.zind(),
            ));
        }
        Ok((msgs, keys))
    }

    /// This is the write dispatch method, where `WriterBots` are chosen randomly.  Callers must
    /// ensure the value is wrapped in a `Dat::BU64`.
    ///
    /// # Local errors
    /// * The write request message cannot be sent via a `WriterBot` channel.
    pub fn store_bytes(
        &self,
        msgs: Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>,
    )
        -> Outcome<()>
    {
        for (msg, zind) in msgs {
            let wbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Writer, &zind));
            let (bot, bpind) = wbots.choose_bot(&ChooseBot::Randomly);
            match bot.send(msg) {
                Err(e) => return Err(err!(e,
                    "{}: While sending write request to wbot {}.",
                    self.ozid(), WorkerInd::new(zind, bpind);
                    Channel, Write)),
                _ => (),
            }
        }
        Ok(())
    }

    pub fn package_write(
        kv:         KeyVal<UIDL, UID>,
        resp:       Responder<UIDL, UID, ENC, KH>,
        csummer:    ChecksummerDefAlt<ChecksumScheme, CS>,
    )
        -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>>
    {
        let klen_cache = kv.key.len();
        let (kstored, vstored, cind, meta, cbpind, _, _) = res!(Encode::encode(kv, csummer));

        Ok(OzoneMsg::Write{
            kstored,
            vstored,
            klen_cache,
            cind,
            meta,
            cbpind,
            resp,
        })
    }

    pub fn delete_using_responder(
        &self,
        k:          &Dat,
        user:       UID,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        resp:       Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // 0. Stamp the delete once.  A delete removes what existed when it began: every tombstone
        //    it sends, the bunch key's and the chunks', carries this time, and the cache keeps the
        //    newer of two records of a key.  A value stored after the delete began is newer than
        //    all of them, so it survives whole, where tombstones stamped as each was sent would
        //    have taken the chunks it shares with the value deleted (a store of the same length
        //    writes the same chunk keys) and left its bunch key to the later tombstone.
        let mut meta = Meta::new(user);
        res!(meta.stamp_time_now());

        // 1. Normalise the key.  The cache hash belongs to the stored record, not merely to the
        //    routing decision, so it is carried through to the writer rather than dropped.
        let (kstored, cbwind, chash) = res!(self.ozone_key_dat(k, schms2));

        // 1a. If the value is chunked, the tombstone on the user key below supersedes only the
        //     bunch key; the chunk records live under their own keys and would leak forever (the
        //     whole reason a chunked value's chunks are never rewritten on delete).  So read the
        //     current bunch key, reconstruct each chunk key from the set_id it stores -- random
        //     for a pre-upgrade value, key-derived for a new one, either way exactly what
        //     `fetch_chunks` reconstructs to read them -- and tombstone each so the ordinary
        //     supersession path reclaims them.  Each carries a responder of its own, so that a
        //     failure names its chunk.  They are sent together with the bunch key's tombstone
        //     below, and awaited after it, so that the delete costs one durability round and not
        //     two.  The read is confined to the delete path, which is rare relative to writes,
        //     and only chunked values pay the fan-out.
        let retiring = res!(self.reclaim_chunks_on_delete(k, &meta, schms2));

        // 2. The value we use to indicate deletion is an unencrypted custom usr type.
        let v = Dat::Usr(id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
        let vstored = res!(v.as_bytes());

        // 3. Select a zone writer bot.
        let wbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Writer, cbwind.zind()));
        let (bot, bpind) = wbots.choose_bot(&ChooseBot::Randomly);

        // 4. Frame the tombstone through the same encoder an insertion goes through.
        let msg = res!(Self::package_write(
            KeyVal {
                key:    Key::Complete(kstored),
                val:    vstored,
                chash,
                meta,
                cbpind: **cbwind.bpind(),
            },
            resp,
            self.schemes().checksummer().clone(),
        ));

        // 5. Send write request, with responder.
        hooks::trace(hooks::Step::Sent);
        if let Err(e) = bot.send(msg) {
            return Err(err!(e,
                "{}: While sending delete request to wbot {}.",
                self.ozid(), WorkerInd::new(*cbwind.zind(), bpind);
                Channel, Write));
        }

        // 6. Wait for the chunk tombstones, sent before the bunch key's and answered beside it.  A
        //    delete that returns Ok has retired every chunk, and one that could not says which.
        if let Some((pkey, waits)) = retiring {
            if let Err(e) = self.await_retired(k, &pkey, waits) {
                return Err(err!(e,
                    "{}: Deleting {:?}: its tombstones were sent together, and the bunch key may \
                    have been deleted although a chunk was not confirmed retired.  The delete can \
                    be repeated while the bunch key is live, and chunks left behind wait for the \
                    orphan sweep.",
                    self.ozid(), k;
                    Write));
            }
        }
        Ok(())
    }

    /// Reads the current value at `k` and, if it is chunked, sends a tombstone for every chunk
    /// record, stamped with the time of `meta`, so the collector reclaims them.  Nothing is
    /// awaited: the part key and the waits come back for `await_retired`, so that the caller can
    /// send its own tombstone before waiting.  `None` for an unchunked or absent value.  Chunk
    /// keys are reconstructed from the part key exactly as `fetch_chunks` does, so this works for
    /// values written under either the old random set_id or the new key-derived one.
    fn reclaim_chunks_on_delete(
        &self,
        k:      &Dat,
        meta:   &Meta<UIDL, UID>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<(PartKey, Vec<(u64, Responder<UIDL, UID, ENC, KH>)>)>>
    {
        let pkey = match res!(self.chunk_set_of(k, schms2)) {
            Some(pkey)  => pkey,
            None        => return Ok(None), // Not chunked, or the key is absent: nothing extra to reclaim.
        };
        let waits = res!(self.send_retires(&pkey, &[], meta, schms2));
        Ok(Some((pkey, waits)))
    }

    /// The part key of the value now at `k`, when that value is chunked.  The read is a whole
    /// read, so a store pays it only where it must know.
    fn chunk_set_of(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<PartKey>>
    {
        let enc = self.schemes().encrypter();
        let or_enc = schms2.map(|s| s.encrypter());

        let resp = res!(self.fetch_using_schemes(k, schms2));
        match res!(resp.recv_daticle(enc, or_enc)) {
            (Some((Dat::Tup5u64(tup), _)), _) => Ok(Some(PartKey(tup))),
            _ => Ok(None),
        }
    }

    // The keys of a chunked value's chunk records, which `fetch_chunks` reconstructs likewise.
    fn chunk_keys_of(pkey: &PartKey) -> Vec<Dat> {
        let n = pkey.num_parts();
        (1..(n + 1)).map(|i| Dat::Tup5u64([
            pkey.set_id(),
            i,
            pkey.data_len(),
            n,
            pkey.part_size(),
        ])).collect()
    }

    /// Tombstones the chunk records of the value `pkey` names, except those under the keys in
    /// `keep`, and waits until each is answered.  Every tombstone is sent before any is waited
    /// for, so the chunks retire together.
    fn retire_chunks(
        &self,
        k:      &Dat,
        pkey:   &PartKey,
        keep:   &[Dat],
        meta:   &Meta<UIDL, UID>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<()>
    {
        let waits = res!(self.send_retires(pkey, keep, meta, schms2));
        self.await_retired(k, pkey, waits)
    }

    /// Sends a tombstone for each chunk record of the value `pkey` names, except those under the
    /// keys in `keep`, and returns the responder to wait on for each, with the chunk's number.
    /// Each has its own responder so that a failure names its chunk.
    fn send_retires(
        &self,
        pkey:   &PartKey,
        keep:   &[Dat],
        meta:   &Meta<UIDL, UID>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Vec<(u64, Responder<UIDL, UID, ENC, KH>)>>
    {
        let mut waits = Vec::new();
        for (i, ck) in Self::chunk_keys_of(pkey).into_iter().enumerate() {
            if keep.contains(&ck) {
                continue;
            }
            let resp = self.responder();
            res!(self.tombstone_chunk_key_at(&ck, meta.clone(), schms2, resp.clone()));
            waits.push(((i + 1) as u64, resp));
        }
        Ok(waits)
    }

    /// Waits until each of the tombstones `send_retires` sent is answered.
    fn await_retired(
        &self,
        k:      &Dat,
        pkey:   &PartKey,
        waits:  Vec<(u64, Responder<UIDL, UID, ENC, KH>)>,
    )
        -> Outcome<()>
    {
        let n = pkey.num_parts();
        for (i, resp) in waits {
            hooks::trace(hooks::Step::Waited);
            // Passed on with its own tags, so a write that landed unconfirmed can be told from one
            // that did not.
            if let Err(e) = resp.recv_write_acks(
                1,
                constant::USER_REQUEST_TIMEOUT,
                constant::DURABILITY_TIMEOUT,
            ) {
                return Err(err!(e,
                    "{}: Retiring the chunks of {:?}: the tombstone of chunk {} of {} was not \
                    acknowledged, so the value's chunks are not all retired.",
                    self.ozid(), k, i, n;
                    Write));
            }
        }
        Ok(())
    }

    // The metadata of a write prepared by `prepare_write_keyed`: every record of it carries the
    // one time.
    fn write_meta(
        msgs: &[(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)],
    )
        -> Outcome<Meta<UIDL, UID>>
    {
        match msgs.first() {
            Some((OzoneMsg::Write { meta, .. }, _)) => Ok(meta.clone()),
            other => Err(err!(
                "Expected a write message at the head of a prepared write, found {:?}.", other;
                Bug, Unexpected)),
        }
    }

    /// Writes an unencrypted deleted-kind tombstone at the `Key::Complete` form of a chunk-data
    /// key, dispatched to the writer of the key's routed zone under the given responder.  Because
    /// the stored-key bytes are the chunk key's bytes either way, the tombstone supersedes the
    /// chunk record at the same cache key, so the ordinary supersession collector flags the
    /// chunk's bytes old and reclaims them -- the only in-place way to retire a chunk record, since
    /// the store has no primitive that forgets a key without writing something at it.  Shared by
    /// the delete path, which reclaims a deleted value's chunks, and the orphan sweep, which
    /// reclaims chunk records no live bunch key references.
    ///
    /// `ck` must be the chunk's `Dat::Tup5u64` part key.  The tombstone is left unencrypted,
    /// exactly as an ordinary key delete leaves it, so a reader recognises it without the at-rest
    /// key.  It is stamped now; `tombstone_chunk_key_at` takes the time of the write it belongs to.
    pub fn tombstone_chunk_key(
        &self,
        ck:     &Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        let mut cmeta = Meta::new(user);
        res!(cmeta.stamp_time_now());
        self.tombstone_chunk_key_at(ck, cmeta, schms2, resp)
    }

    /// A chunk tombstone with the metadata of the delete or store that retires the chunk.  The
    /// cache keeps the newer of two records of a key, so a tombstone stamped with the time of the
    /// write it belongs to takes the chunks that write found and no newer chunk of a value stored
    /// since, where one stamped as it is sent could take a value begun after the write (QA A2-6).
    pub fn tombstone_chunk_key_at(
        &self,
        ck:     &Dat,
        cmeta:  Meta<UIDL, UID>,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        hooks::trace(hooks::Step::Sent);
        let (ckbuf, ccbwind, cchash) = res!(self.ozone_key_dat(ck, schms2));
        let tomb = Dat::Usr(id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
        let tvstored = res!(tomb.as_bytes());
        let msg = res!(Self::package_write(
            KeyVal {
                key:    Key::Complete(ckbuf),
                val:    tvstored,
                chash:  cchash,
                meta:   cmeta,
                cbpind: **ccbwind.bpind(),
            },
            resp,
            self.schemes().checksummer().clone(),
        ));
        let cwbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Writer, ccbwind.zind()));
        let (cbot, cbpind) = cwbots.choose_bot(&ChooseBot::Randomly);
        match cbot.send(msg) {
            Err(e) => Err(err!(e,
                "{}: While sending chunk tombstone for {:?} to wbot {}.",
                self.ozid(), ck, WorkerInd::new(*ccbwind.zind(), cbpind);
                Channel, Write)),
            _ => Ok(()),
        }
    }

    // Read API, for general public use.
    
    /// Get a `Dat`icle value using the given key and data scheme overrides.  The result is
    /// available asynchronously in the returned `Responder` channel.
    ///
    /// # Arguments
    /// * `k` - key `Dat`cle.
    /// * `enc` - An optional `EncryptionScheme` that was used to store the value.  An error will be returned if the decryption does not yield a valid `Dat`icle.
    ///
    pub fn get(
        &self,
        key:    &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        let sbots = self.chans().all_sbots();
        let (bot, bpind) = sbots.choose_bot(&ChooseBot::Randomly);
        // Send read request, with responder.
        match bot.send(OzoneMsg::Get {
            key:    key.clone(),
            schms2: schms2.cloned(),
            resp:   resp.clone(),
        }) {
            Err(e) => Err(err!(e,
                "{}: While sending get request to sbot {}.",
                self.ozid(), bpind;
                Channel, Write)),
            _ => Ok(resp),
        }
    }

    // Read API, high level, used by ServerBots.
    //
    /// Blocking retrieval of a `Dat`icle value using the given key and data scheme overrides.
    ///
    /// # Arguments
    /// * `k` - key `Dat` to be transformed into an Ozone key.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    ///
    pub fn get_wait(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<(Dat, Meta<UIDL, UID>)>>
    {
        let enc = self.schemes().encrypter();
        let or_enc = schms2.map(|s| s.encrypter());

        let resp = res!(self.fetch_using_schemes(k, schms2));
        match res!(resp.recv_daticle(enc, or_enc)) {
            (None, _) => Ok(None), // The key was not found.
            // The value was too large for a single record, so what is stored under this key is a
            // part key naming its chunks.  `fetch_chunks` gathers them, rejoins the bytes,
            // decrypts them and decodes them, so what it hands back is already the caller's
            // value -- fully formed, of whatever kind they stored.
            //
            // It was previously taken for raw bytes and decoded a SECOND time, which no chunked
            // value survives: a list, a map or a string fell through to a catch-all and came back
            // as an error, and a byte string -- the one kind the arms matched -- had its payload
            // read as though it were itself an encoding.  So every value large enough to be
            // chunked was written perfectly well and could not be read: an accumulating value,
            // such as a ledger, worked until the day it crossed the chunk size and then failed
            // for good.
            (Some((Dat::Tup5u64(tup), meta)), _) =>
                Ok(Some((res!(self.fetch_chunks(&Dat::Tup5u64(tup), schms2)), meta))),
            // The data received was in a single piece.
            (Some((dat, meta)), _) => Ok(Some((dat, meta))),
        }
    }

    // Read API, lower level.
    
    /// Fetch a value using the given key.  This is just a caller of `OzoneApi::fetch_using_responder`
    /// that provides a default `Responder`.  Default database schemes (e.g. encryption) are used.
    ///
    /// # Arguments
    /// * `k` - key `Dat` to be transformed into an Ozone key.
    ///
    /// Returns a default `Responder`.
    pub fn fetch(
        &self,
        k: &Dat,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        res!(self.fetch_using_responder(k, None, resp.clone()));
        Ok(resp)
    }

    /// Fetch a value using the given key and data schemes override.  This is just a caller of
    /// `OzoneApi::fetch_using_responder` that provides a default `Responder`.
    ///
    /// # Arguments
    /// * `k` - key `Dat` to be transformed into an Ozone key.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    ///
    /// Returns a default `Responder`.
    pub fn fetch_using_schemes(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        res!(self.fetch_using_responder(k, schms2, resp.clone()));
        Ok(resp)
    }

    pub fn fetch_using_key(
        &self,
        key:    Key,
        cbwind: WorkerInd,
    )
        -> Outcome<Responder<UIDL, UID, ENC, KH>>
    {
        let resp = self.responder();
        res!(self.fetch_using_key_and_responder(
            key,
            cbwind,
            resp.clone(),
        ));
        Ok(resp)
    }

    /// Fetch a value using the given key, scheme overrides and a customisable `Responder`.  The
    /// caller can use the `Responder` to wait for a single value, or an error.  An error will
    /// result if the value cannot be decoded into a `Dat`.  This can occur if the value was
    /// improperly stored or the given decrypter does not match the original encrypter.  If the
    /// value was chunked, a `PartKey` "bunch key" will be returned, which can be passed to
    /// `OzoneApi::fetch_chunks` to collect the chunks and re-assemble the value.
    ///
    /// # Arguments
    /// * `k` - key `Dat` to be transformed into an Ozone key.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    ///
    /// # Local errors
    /// * An error will result if the request cannot be sent to the randomly chosen `ReaderBot`.
    pub fn fetch_using_responder(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // Normalise the key.
        let (kbuf, cbwind, _chash) = res!(self.ozone_key_dat(k, schms2));
        let key = match k {
            Dat::Tup5u64(tup) => Key::Chunk(kbuf, try_into!(usize, PartKey(*tup).index())),
            _ => Key::Complete(kbuf),
        };
        self.fetch_using_key_and_responder(
            key,
            cbwind,
            resp,
        )
    }

    pub fn fetch_using_key_and_responder(
        &self,
        key:    Key,
        cbwind: WorkerInd, // CacheBot worker index.
        resp:   Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // Select a zone reader bot.
        let rbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Reader, cbwind.zind()));
        let (bot, bpind) = rbots.choose_bot(&ChooseBot::Randomly);
        // Send read request, with responder.
        match bot.send(OzoneMsg::Read(key, **cbwind.bpind(), resp)) {
            Err(e) => return Err(err!(e,
                "{}: While sending read request to rbot {}.",
                self.ozid(), WorkerInd::new(*cbwind.zind(), bpind);
                Channel, Write)),
            _ => Ok(()),
        }
    }

    /// Data is automatically chunked when stored, such that each chunk is accessed via its own
    /// `PartKey`.  However chunked data is not automatically reassembled.  A valid `PartKey`
    /// ("bunch key") passed to this method will perform the collection and reassembly.
    ///
    /// # Arguments
    /// * `k` - bunch key `PartKey` which provides all necessary chunk metrics.
    /// * `schms2` - `RestSchemesOverride` overrides database schemes (e.g. key hashing, encryption).
    ///
    /// # Local errors
    /// * The key must be a `PartKey`.
    /// * The `PartKey` part size must exceed zero.
    /// * The `PartKey` number of parts must exceed zero.
    /// * The `PartKey` index must be zero.
    /// * A read request cannot be sent to a randomly chosen `ReaderBot`.
    /// * A `Dat` value cannot be received from the `Responder`.
    /// * The `PartKey` key for a chunk cannot have an index value of zero.
    /// * The index of a chunk cannot exceed the expected number of chunks.  This can occur if the
    /// chunks were incorrectly stored.
    /// * The chunk value must be wrapped in a `Dat::BU64`.
    /// * The unwrapped length of all chunks must match the bunch key part size, except for the final
    /// chunk.  Note that `store_using_responder` pads the final chunk to the uniform size when
    /// encryption is used.
    /// * If the final chunk length differs from the part size, it must not exceed the part size.
    /// * An error will be raised if the total length of the chunk data received exceeds the
    /// expected capacity of the receiving receptable.  This error should not occur.
    /// * An error will occur if a chunk cannot be found.
    /// * An error will occur if the `ReaderBot` responds with an unexpected message.
    /// * The re-assembled data value must be an encoded `Dat`.
    ///
    pub fn fetch_chunks(
        &self,
        k:      &Dat,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Dat>
    {
        let self_id = self.ozid().clone();
        let enc = self.schemes().encrypter();
        let or_enc = schms2.map(|s| s.encrypter());
        let encryption_on = !(enc.or_is_identity(or_enc));

        match k {
            Dat::Tup5u64(tup) => {
                let pkey = PartKey(*tup);
                if pkey.part_size() == 0 {
                    return Err(err!(
                        "{}: Chunk size must exceed zero.", self_id;
                        Input, Invalid));
                }
                if pkey.num_parts() == 0 {
                    return Err(err!(
                        "{}: Number of chunks must exceed zero.", self_id;
                        Input, Invalid));
                }
                if pkey.index() != 0 {
                    return Err(err!(
                        "{}: Index in bunch key must be zero.", self_id;
                        Input, Invalid));
                }
                // 1. Send requests for chunks.
                let data_len = try_into!(usize, pkey.data_len());
                let chunk_size = try_into!(usize, pkey.part_size());
                let num_chunks = try_into!(usize, pkey.num_parts());
                let resp = self.responder();
                for i in 1..(pkey.num_parts() + 1) {
                    let k = Dat::Tup5u64([
                        pkey.set_id(),
                        i,
                        pkey.data_len(),
                        pkey.num_parts(),
                        pkey.part_size(),
                    ]);
                    let (kbuf, cbwind, _chash) = res!(self.ozone_key_dat(&k, schms2));

                    let rbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Reader, cbwind.zind()));
                    let (bot, bpind) = rbots.choose_bot(&ChooseBot::Randomly);
                    let key = Key::Chunk(kbuf, try_into!(usize, i));
                    match bot.send(OzoneMsg::Read(key, **cbwind.bpind(), resp.clone())) {
                        Err(e) => return Err(err!(e,
                            "{}: While sending chunk {} read request to rbot {}.",
                            self_id, i, WorkerInd::new(*cbwind.zind(), bpind);
                            Channel, Write)),
                        _ => (),
                    }
                }
                // 2. Collection and reassembly.
                let capacity = num_chunks * chunk_size;
                let mut joined = vec![0; capacity];
                for _ in 0..num_chunks {
                    match resp.recv_timeout(constant::USER_REQUEST_TIMEOUT) {
                        Err(e) => return Err(err!(e,
                            "{}: Could not read from chunk collection responder channel.", self_id;
                            IO, Channel, Read)),
                        Ok(OzoneMsg::Value(Value::Chunk(Some((Dat::BU8(v), _)), i, _)))    |
                        Ok(OzoneMsg::Value(Value::Chunk(Some((Dat::BU16(v), _)), i, _)))   |
                        Ok(OzoneMsg::Value(Value::Chunk(Some((Dat::BU32(v), _)), i, _)))   |
                        Ok(OzoneMsg::Value(Value::Chunk(Some((Dat::BU64(v), _)), i, _)))   => {
                            if i == 0 {
                                return Err(err!(
                                    "{}: For key {:?}, data chunk of size {} has an invalid \
                                    index of zero amongst an expected total of {} chunks.",
                                    self_id, k, v.len(), num_chunks;
                                    Invalid, Input));
                            }
                            if i > num_chunks {
                                return Err(err!(
                                    "{}: For key {:?}, data chunk of size {} with index {} \
                                    exceeds the expected number of chunks, {}.",
                                    self_id, k, v.len(), i, num_chunks;
                                    Invalid, Input));
                            }
                            let mut end = chunk_size * i;
                            let mut start = end - chunk_size;
                            if v.len() != chunk_size {
                                if i < num_chunks {
                                    return Err(err!(
                                        "{}: For key {:?}, data chunk {} of {} size of {} does \
                                        not match the size of {} specified by the \
                                        PartKey.",
                                        self_id, k, i, num_chunks, v.len(), chunk_size;
                                        Input, Size, Mismatch));
                                } else {
                                    if v.len() > chunk_size {
                                        return Err(err!(
                                            "{}: For key {:?}, the final data chunk {} size \
                                            of {} must be less than the size of the {} other \
                                            chunks, {} bytes.", 
                                            self_id, k, i, v.len(), num_chunks-1, chunk_size;
                                            Input, Size, Invalid));
                                    } else {
                                        start = chunk_size * (i-1);
                                        end = start + v.len();
                                    }
                                }
                            }
                            if end > capacity {
                                return Err(err!(
                                    "{}: For key {:?}, end location {} for retrieved data \
                                    (chunk {} of {}) of length {} exceeds the end location \
                                    of the expected reassembled data, {}.",
                                    self_id, k, end, i, num_chunks, chunk_size, capacity;
                                    Bug, Input, Size, Mismatch));
                            }
                            joined[start..end].copy_from_slice(&v[..]);
                        },
                        Ok(OzoneMsg::Value(Value::Chunk(None, i, _))) => return Err(err!(
                            "{}: For key {:?}, data chunk {} of {} was not found.",
                            self_id, k, i, num_chunks;
                            Missing, Data)),
                        Ok(msg) => return Err(err!(
                            "{}: Unrecognised chunk request response: {:?}", self_id, msg;
                            Invalid, Input)),
                    }
                }
                if encryption_on {
                    joined = res!(enc.or_decrypt(&joined[..data_len], or_enc)); 
                }
                match Dat::from_bytes(&joined) {
                    Err(e) => return Err(err!(e,
                        "{}: For key {:?}, a Dat could not be formed from the value bytes.  \
                        This could mean the data was not originally stored as a Dat, or the \
                        encrypter, {}, differs from that used to store the original data.",
                        self_id, k, enc.or_debug(or_enc);
                        Decode, Bytes)),
                    Ok((dat, _)) => return Ok(dat),
                }
            },
            _ => return Err(err!("{}: Key must be a PartKey.", self_id; Input, Invalid)),
        }
    }

    /// Walks every index file in every zone, so the cost is the size of the store rather than
    /// the size of the result.  The deadline is the ordinary user request one, which is what
    /// keeps "nothing on a request path may scan" enforceable rather than advisory: a walk big
    /// enough to matter fails here instead of stalling the caller.  A background walk that has
    /// deliberately accepted the cost says so at its call site with `scan_with_wait`.
    pub fn scan(
        &self,
        opts:   &ScanOpts,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>
    {
        self.scan_with_wait(opts, schms2, constant::USER_REQUEST_WAIT)
    }

    /// `scan` with the deadline named by the caller instead of taken from `USER_REQUEST_WAIT`.
    ///
    /// # Arguments
    ///
    /// * `wait` - how long every zone has, in total, to return its entries.  Only a caller that
    ///   knows it is off the request path should lengthen this.
    pub fn scan_with_wait(
        &self,
        opts:   &ScanOpts,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        wait:   Wait,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>
    {
        let nz = self.cfg().num_zones();
        let max_wait = wait.max_wait;
        let resp = self.responder();

        // Send one ScanRequest to a scan bot of every zone.
        for z in 0..nz {
            let zind = ZoneInd::new(z);
            let scbots = res!(self.chans().get_workers_of_type_in_zone(
                &WorkerType::Scan,
                &zind,
            ));
            let (bot, _) = scbots.choose_bot(&ChooseBot::Randomly);
            if let Err(e) = bot.send(OzoneMsg::ScanRequest {
                opts:   opts.clone(),
                schms2: schms2.cloned(),
                resp:   resp.clone(),
            }) {
                return Err(err!(e,
                    "{}: Cannot send scan request to scan bot in zone {}.",
                    self.ozid(), z;
                    Channel, Write));
            }
        }

        // Gather one ScanEntries response per zone.
        let (_, msgs) = match resp.recv_number(nz, wait) {
            Ok(v) => v,
            Err(e) => return Err(err!(e,
                "{}: A scan of all {} zones did not finish within {:?}.  A scan walks every \
                index file in every zone, so it takes longer the larger the store is, however \
                few entries match; a scan on a request path is what this deadline exists to \
                catch.  A caller that is deliberately off the request path should name its own \
                deadline with scan_with_wait rather than lengthening \
                constant::USER_REQUEST_TIMEOUT, which every user request shares.",
                self.ozid(), nz, max_wait;
                Channel, Timeout)),
        };
        let mut out: Vec<(Dat, Dat, Meta<UIDL, UID>)> = Vec::new();
        for msg in msgs {
            match msg {
                OzoneMsg::ScanEntries(entries) => {
                    out.extend(entries);
                },
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: Zone-level scan failure.", self.ozid();
                    Channel)),
                other => return Err(err!(
                    "{}: Unexpected response to scan request: {:?}",
                    self.ozid(), other;
                    Channel, Unexpected)),
            }
        }

        // Apply the global limit. Per-zone limits have already been
        // applied inside each igbot, so this cap tightens the
        // cross-zone merge rather than truncating any single zone.
        if let Some(lim) = opts.limit {
            if out.len() > lim {
                out.truncate(lim);
            }
        }
        Ok(out)
    }

    /// Explains a control operation that did not complete, since the bare shortfall from
    /// `recv_number` names neither the operation, nor the reason a healthy database can miss
    /// the deadline, nor the constant to change if it should not have.
    fn control_failure(
        &self,
        e:      Error<ErrTag>,
        opn:    &str,   // the operation attempted
        n:      usize,  // acknowledgements expected
        who:    &str,   // the bots expected to acknowledge
    )
        -> Error<ErrTag>
    {
        err!(e,
            "{}: The {} failed while waiting up to {:?} for all {} {} to acknowledge it.  A \
            bot acknowledges a control message only once it reaches it in its queue, so the \
            usual cause is a store large enough that the bots are still surveying its files \
            after startup, rather than any fault in the operation.  This deadline is \
            constant::CONTROL_REQUEST_TIMEOUT; raise that if a store legitimately needs \
            longer, and not constant::USER_REQUEST_TIMEOUT, which is deliberately short \
            because every user request shares it.",
            self.ozid(), opn, constant::CONTROL_REQUEST_TIMEOUT, n, who;
            Channel, Timeout)
    }

    /// Activate garbage collection by sending a control message to the igbots via the zbots, via the supervisor.
    pub fn activate_gc(&self, on: bool) -> Outcome<()> {
        info!(sync_log::stream(), "Activating garbage collection...");
        let emsg = "garbage collection activation";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::GcControl(GcControl::On(on), resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        // A control operation, not a user request: this runs once, at startup, behind whatever
        // initialisation the zone bots are still doing.  See constant::CONTROL_REQUEST_TIMEOUT.
        let nz = self.cfg().num_zones();
        let (_, msgs) = match resp.recv_number(nz, constant::CONTROL_REQUEST_WAIT) {
            Ok(v) => v,
            Err(e) => return Err(self.control_failure(e, emsg, nz, "zone bots")),
        };
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to {}.", self.ozid(), emsg;
                    Channel)),
                OzoneMsg::Ok => (),
                msg => return Err(err!(
                    "{}: Unexpected response to {}: {:?}", self.ozid(), emsg, msg;
                    Channel)),
            };
        }
        Ok(())
    }

    // Utility methods useful for situational awareness and testing.
    
    /// Command all cbots to clear their caches.
    pub fn clear_cache_values(&self, wait: Wait) -> Outcome<()> {
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::ClearCache(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send clear cache command to supervisor.", self.ozid();
                Channel, Write));
        }
        let n = self.cfg().num_caches();
        let (_, msgs) = res!(resp.recv_number(n, wait));
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to clear cache command.", self.ozid();
                    Channel)),
                OzoneMsg::Ok => (),
                msg => return Err(err!(
                    "{}: Unexpected response to clear cache command: {:?}", self.ozid(), msg;
                    Channel)),
            }
        }
        warn!(sync_log::stream(), "All {} caches successfully cleared.", n);
        Ok(())
    }

    /// Dump all cache contents to the log file.
    pub fn dump_caches(&self, wait: Wait) -> Outcome<()> {
        // Gather.
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::DumpCacheRequest(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send cache dump request to supervisor.", self.ozid();
                Channel, Write));
        }
        let n = self.cfg().num_caches();
        let (_, msgs) = res!(resp.recv_number(n, wait));
        let mut sorted = BTreeMap::new();
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to cache dump request.", self.ozid();
                    Channel)),
                OzoneMsg::DumpCacheResponse(wind, cache) => {
                    sorted.insert(wind, cache);
                },
                msg => return Err(err!(
                    "{}: Unexpected response to cache dump request: {:?}", self.ozid(), msg;
                    Channel)),
            }
        }
        // Display.
        info!(sync_log::stream(), "Cache dump summary");
        info!(sync_log::stream(), "+-----------+--------------+--------------+");
        info!(sync_log::stream(), "|   Cache   |   Entries    |    Size [B]  |");
        info!(sync_log::stream(), "+-----------+--------------+--------------+");
        for (wind, cache) in &sorted {
            info!(sync_log::stream(), "|{:^11}|{:>13} |{:>13} |",
                fmt!("{}", wind),
                cache.map().len(),
                cache.get_size(),
            ); 
        }
        info!(sync_log::stream(), "+-----------+--------------+--------------+");
        for (wind, cache) in sorted {
            let mut total_size = 0;
            info!(sync_log::stream(), "{} cache dump of {} entries:", wind, cache.map().len()); 
            if cache.map().len() == 0 {
                info!(sync_log::stream(), " No cache entries."); 
            } else {
                for (kbyt, centry) in cache.map() {
                    if let CacheEntry::LocatedValue(mloc, val) = centry {
                        //let (k, _) = res!(Dat::from_bytes(&kbyt));
                        let vlen = match val {
                            Some(v) => v.len(),
                            None => 0,
                        };
                        let size =
                            kbyt.len() +
                            vlen +
                            cache.mloc_size();

                        info!(sync_log::stream(), " kbyt = {:02x?} vlen = {} floc = {:?}",
                            kbyt, vlen, mloc.file_location(),
                        );
                        total_size += size;
                    }
                }
            }
            info!(sync_log::stream(), "{} cache size estimate: {} [B]", wind, total_size);
        }
            
        Ok(())
    }

    /// Returns cache entry for the given key.
    pub fn cache_entry_info(
        &self,
        k:          &Dat,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        timeout:    Duration,
    )
        -> Outcome<ReadResult<UIDL, UID>>
    {
        let (kbuf, cbwind, _chash) = res!(self.ozone_key_dat(k, schms2));
        let resp = self.responder();

        let cbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Cache, cbwind.zind()));
        let bot = res!(cbots.get_bot(**cbwind.bpind()));
        let key = match k {
            Dat::Tup5u64(tup) => Key::Chunk(kbuf, try_into!(usize, PartKey(*tup).index())),
            _ => Key::Complete(kbuf),
        };
        match bot.send(OzoneMsg::ReadCache(key, resp.clone())) {
            Err(e) => return Err(err!(e,
                "{}: While sending cache entry info request to cbot {}.",
                self.ozid(), cbwind;
                Channel, Write)),
            _ => (),
        }

        match res!(resp.recv_timeout(timeout)) {
            OzoneMsg::Error(e) => return Err(err!(e,
                "{}: In response to cache entry info request.", self.ozid();
                Channel)),
            OzoneMsg::ReadResult(readres) => Ok(readres),
            msg => Err(err!(
                "{}: Unexpected response to cache entry info request: {:?}", self.ozid(), msg;
                Channel)),
        }
    }

    pub fn collect_file_states(
        &self,
        wait: Wait,
    )
        -> Outcome<BTreeMap<WorkerInd, FileStateMap>>
    {
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::DumpFileStatesRequest(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send file state dump request to supervisor.", self.ozid();
                Channel, Write));
        }
        let n = self.cfg().num_filemaps();
        let (_, msgs) = res!(resp.recv_number(n, wait));
        let mut sorted = BTreeMap::new();
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to file state dump request.", self.ozid();
                    Channel)),
                OzoneMsg::DumpFileStatesResponse(wind, fstates) => {
                    sorted.insert(wind, fstates);
                },
                msg => return Err(err!(
                    "{}: Unexpected response to file states dump request: {:?}", self.ozid(), msg;
                    Channel)),
            }
        }
        Ok(sorted)
    }

    /// Dump all zone file states to the log file.
    pub fn dump_file_states(&self, wait: Wait) -> Outcome<()> {
        // Gather.
        let sorted = res!(self.collect_file_states(wait));
        // Display.
        for (wind, fstates) in sorted {
            info!(sync_log::stream(), "{} file states dump:", wind); 
            if fstates.map().len() == 0 {
                info!(sync_log::stream(), " None"); 
            } else {
                for (fnum, fstat) in fstates.map() {
                    info!(sync_log::stream(), " {:10} {:?} old = {:.1}%",
                        fnum, fstat,
                        100.0 * (fstat.get_old_sum() as f64)
                        / (self.cfg().data_file_max_bytes as f64),
                    );
                }
            }
        }
            
        Ok(())
    }

    /// Returns the number of messages in all channels for all zones.
    pub fn ozone_msg_count(&self) -> OzoneMsgCount {
        self.chans().msg_count()
    }

    /// Returns the file directory size, in-memory cache size and bot message queues for each zone, in bytes.
    pub fn ozone_state(&self, wait: Wait) -> Outcome<Vec<ZoneState>> {
        let emsg = "ozone state request";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::OzoneStateRequest(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        match res!(resp.recv_timeout(wait.max_wait)) {
            OzoneMsg::Error(e) => return Err(err!(e,
                "{}: In response to {}.", self.ozid(), emsg;
                Channel)),
            OzoneMsg::OzoneStateResponse(zstats) => Ok(zstats),
            msg => Err(err!(
                "{}: Unexpected response to {}: {:?}", self.ozid(), emsg, msg;
                Channel)),
        }
    }

    /// Ping the bots for proof of life.
    pub fn ping_bots(&self, wait: Wait) -> Outcome<(Instant, Vec<OzoneMsg<UIDL, UID, ENC, KH>>)> {
        let resp = self.responder();
        let self_id = self.ozid().clone();
        let n = res!(self.chans().send_to_all(OzoneMsg::Ping(self_id, resp.clone())));
        resp.recv_number(n, wait)
    }

    pub fn bot_error_count(&self, wait: Wait) -> Outcome<(usize, usize)> {
        let (_, msgs) = res!(self.ping_bots(wait));
        let mut nbots: usize    = 0;
        let mut errs:  usize    = 0;
        for msg in msgs {
            match msg {
                OzoneMsg::Pong(_ozid, n) => {
                    nbots += 1;
                    errs = errs.saturating_add(n);
                },
                msg => return Err(err!(
                    "{}: Unexpected response to bot ping: {:?}", self.ozid(), msg;
                    Channel)),
            }
        }
        Ok((errs, nbots))
    }

    /// Each zone's directory listing, read by its zone bot when the request arrives.
    pub fn collect_files(
        &self,
        wait: Wait,
    )
        -> Outcome<BTreeMap<ZoneInd, BTreeMap<String, FileEntry>>>
    {
        let emsg = "list files request";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::DumpFiles(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        let (_, msgs) = res!(resp.recv_number(self.cfg().num_zones(), wait));
        let mut map = BTreeMap::new();
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to {}.", self.ozid(), emsg;
                    Channel)),
                OzoneMsg::Files(zind, zmap) => map.insert(zind, zmap),
                msg => return Err(err!(
                    "{}: Unexpected response to {}: {:?}", self.ozid(), emsg, msg;
                    Channel)),
            };
        }
        Ok(map)
    }

    /// The accounting barrier.  When it returns `Ok`, every `UpdateData` and `ScheduleOld` caused by
    /// a write acknowledged before the call has reached its file bot, or its garbage buffer.  A cache
    /// bot answers its caller before it tells the file bot of the new record's file, and that file
    /// bot then tells the file bot of the old record's file, so each stage is asked in turn: the
    /// cache bots forward the request to every file bot (behind their `UpdateData`), and each file
    /// bot echoes it to every file bot (behind the `ScheduleOld` it forwarded).  Every file bot
    /// answers each echo, so a zone owes cache bots times file bots times file bots answers.
    fn settle(&self, deadline: Duration) -> Outcome<()> {
        let emsg = "accounting barrier";
        let resp = self.responder();
        let mut owed = 0usize;
        for z in 0..self.cfg().num_zones() {
            let zind = ZoneInd::new(z);
            let cbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Cache, &zind));
            let fbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::File, &zind));
            if let Err(e) = cbots.send_to_all(OzoneMsg::Settle(resp.clone())) {
                return Err(err!(e,
                    "{}: Cannot send the {} to the cache bots of zone {}.", self.ozid(), emsg, z;
                    Channel, Write));
            }
            owed += cbots.len() * fbots.len() * fbots.len();
        }
        let wait = Wait {
            max_wait:       deadline,
            check_interval: constant::CHECK_INTERVAL.min(deadline),
        };
        let (_, msgs) = match resp.recv_number(owed, wait) {
            Ok(v) => v,
            Err(e) => return Err(err!(e,
                "{}: The {} was not answered by all {} file bot echoes within {:?}.  A bot \
                answers only when it reaches the request in its queue.",
                self.ozid(), emsg, owed, deadline;
                Channel, Timeout)),
        };
        for msg in msgs {
            match msg {
                OzoneMsg::Ok => (),
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to the {}.", self.ozid(), emsg;
                    Channel)),
                msg => return Err(err!(
                    "{}: Unexpected response to the {}: {:?}", self.ozid(), emsg, msg;
                    Channel, Unexpected)),
            }
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn settle_for_test(&self, deadline: Duration) -> Outcome<()> {
        self.settle(deadline)
    }

    /// The physical length in bytes of every regular file in every zone directory, data, index and
    /// collection temporary alike, read when asked.  It is not the figure [`Self::ozone_state`] gives,
    /// the file bots' accounted size, which is pushed periodically and lags any write still draining.
    pub fn size_bytes(&self, wait: Wait) -> Outcome<u64> {
        let files = res!(self.collect_files(wait));
        let mut total = 0u64;
        for (_zind, zmap) in files {
            for (_name, entry) in zmap {
                if entry.typ == "f" {
                    total = total.saturating_add(entry.size);
                }
            }
        }
        Ok(total)
    }

    pub fn list_files(&self, wait: Wait) -> Outcome<()> {

        info!(sync_log::stream(), "Directory listing for {} zones, key:", self.cfg().num_zones());
        info!(sync_log::stream(), " Typ: f File | d Directory | s Symlink");
        info!(sync_log::stream(), " Size: in bytes");
        info!(sync_log::stream(), " Mod: seconds since last modified");
        info!(sync_log::stream(), " Name: object label");

        let map = res!(self.collect_files(wait));
        for (zind, zmap) in map {
            let mut total_size = 0;
            info!(sync_log::stream(), "{:?} directory", zind);
            info!(sync_log::stream(), "+-----+--------------+--------------+-----------------------------------------------------");
            info!(sync_log::stream(), "| Typ |   Size [B]   |    Mod [s]   | Name");
            info!(sync_log::stream(), "+-----+--------------+--------------+-----------------------------------------------------");
            for (_key, entry) in zmap {
                info!(sync_log::stream(), 
                    "|  {}  |{:>13} |{:>13} | {}",
                    entry.typ,
                    entry.size,
                    entry.mods,
                    entry.name,
                );
                total_size += entry.size;
            }
            info!(sync_log::stream(), "+-----+--------------+--------------+-----------------------------------------------------");
            info!(sync_log::stream(), "|     |{:>13} |              |", total_size);
            info!(sync_log::stream(), "+-----+--------------+--------------+-----------------------------------------------------");
        }
        Ok(())
    }

    pub fn get_zone_dirs(&self) -> Outcome<BTreeMap<ZoneInd, ZoneDir>> {
        let emsg = "zone directories request";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::GetZoneDir(resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        // Deliberately a user request deadline and not the control one: this reports what the
        // zone bots already hold, it is callable at any time rather than only during startup,
        // and a caller wanting an answer is better told quickly that there is none.
        let (_, msgs) = res!(resp.recv_number(self.cfg().num_zones(), constant::USER_REQUEST_WAIT));
        let mut map = BTreeMap::new();
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to {}.", self.ozid(), emsg;
                    Channel)),
                OzoneMsg::ZoneDir(zind, zdir) => map.insert(zind, zdir),
                msg => return Err(err!(
                    "{}: Unexpected response to {}: {:?}", self.ozid(), emsg, msg;
                    Channel)),
            };
        }
        Ok(map)
    }

    /// Collects every old byte that a write acknowledged before the call left behind, and returns
    /// once none is held by a sealed file, or fails at the deadline naming each file that still
    /// holds one and why.
    ///
    /// The live files are rolled first, since a live file is never collected, then the accounting
    /// barrier makes every supersession of an acknowledged write visible to the file bots.  Every
    /// quarter of a second the file bots are ordered to collect what they hold, `Force::Yes`, which
    /// waives the collector's trigger fraction and its switches and nothing that protects a read:
    /// a file with a reader, a move in flight or a writer still draining is asked again, not
    /// collected.  A zone bot that cannot pass the order on fails the call at once, not at the
    /// deadline.
    pub fn compact_now(&self, deadline: Duration) -> Outcome<CompactReport> {
        let emsg = "compaction";
        let start = Instant::now();
        let left = |start: &Instant| deadline.saturating_sub(start.elapsed());
        let wait = |d: Duration| Wait {
            max_wait:       d,
            check_interval: constant::CHECK_INTERVAL.min(d),
        };
        let bytes_before = res!(self.size_bytes(wait(deadline)));
        res!(self.new_live_files_within(wait(left(&start).max(Duration::from_millis(1)))));
        res!(self.settle(left(&start)));

        let resp = self.responder();
        let chan = res!(resp.channel().ok_or_else(|| err!(
            "{}: The responder for {} has no channel.", self.ozid(), emsg; Channel, Missing)));
        let mut collected = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        let mut waiting: BTreeMap<WorkerInd, Vec<(FileNum, Hold)>> = BTreeMap::new();
        let mut ordered: Option<Instant> = None;
        loop {
            if start.elapsed() > deadline {
                // The files that still hold old bytes, from the state the last pass saw.
                let states = res!(self.collect_file_states(constant::USER_REQUEST_WAIT));
                let mut held = Vec::new();
                for (wind, fmap) in &states {
                    for (fnum, fstat) in fmap.map() {
                        if Self::holds_old_bytes(fstat) {
                            let why = match waiting.get(wind).and_then(|w| w.iter().find(|(f, _)| f == fnum)) {
                                Some((_, hold)) => hold.why(),
                                None if fstat.gc_active() => "collecting",
                                None => "not yet ordered",
                            };
                            held.push(fmt!("file {} of zone {} ({})", fnum, wind.z() + 1, why));
                        }
                    }
                }
                return Err(err!(
                    "{}: The {} did not finish within {:?}.  Sealed files still holding old bytes: {}.",
                    self.ozid(), emsg, deadline, held.join(", ");
                    Channel, Timeout));
            }
            if ordered.map_or(true, |t| t.elapsed() >= Duration::from_millis(250)) {
                if let Err(e) = self.chans().sup().send(
                    OzoneMsg::GcControl(GcControl::Compact, resp.clone())
                ) {
                    return Err(err!(e,
                        "{}: Cannot send the {} order to supervisor.", self.ozid(), emsg;
                        Channel, Write));
                }
                ordered = Some(Instant::now());
            }
            // Reports, until the file bots are quiet for a moment.  A failed order ends the wait.
            while let Recv::Result(Ok(msg)) = chan.recv_timeout(Duration::from_millis(50)) {
                match msg {
                    OzoneMsg::CompactReport { wind, started, deleted: gone, waiting: held } => {
                        for fnum in started {
                            collected.insert((wind, fnum));
                        }
                        for fnum in gone {
                            deleted.insert((wind, fnum));
                        }
                        waiting.insert(wind, held);
                    },
                    OzoneMsg::Error(e) => return Err(err!(e,
                        "{}: In response to the {} order.", self.ozid(), emsg;
                        Channel)),
                    msg => return Err(err!(
                        "{}: Unexpected response to the {} order: {:?}", self.ozid(), emsg, msg;
                        Channel, Unexpected)),
                }
            }
            let states = res!(self.collect_file_states(wait(left(&start).max(Duration::from_millis(1)))));
            let mut pending = false;
            for (_wind, fmap) in &states {
                for (_fnum, fstat) in fmap.map() {
                    pending |= Self::holds_old_bytes(fstat);
                }
            }
            if !pending {
                break;
            }
        }
        // A deleted file's handle in a reader is dropped by a notice queued behind whatever the
        // reader was doing, and the call is not done while that handle keeps the bytes allocated.
        res!(self.readers_caught_up(wait(left(&start).max(Duration::from_millis(1)))));
        let bytes_after = res!(self.size_bytes(wait(left(&start).max(Duration::from_millis(1)))));
        Ok(CompactReport {
            files_collected:    collected.len(),
            files_deleted:      deleted.len(),
            bytes_before,
            bytes_after,
        })
    }

    // Returns once every reader has handled all that was queued to it before the call.
    fn readers_caught_up(&self, wait: Wait) -> Outcome<()> {
        let emsg = "reader barrier";
        let resp = self.responder();
        let mut n = 0;
        for pool in self.chans().get_all_workers_of_type(&WorkerType::Reader) {
            for i in 0..pool.len() {
                let bot = res!(pool.get_bot(i));
                if let Err(e) = bot.send(OzoneMsg::Ping(self.ozid().clone(), resp.clone())) {
                    return Err(err!(e,
                        "{}: Cannot send the {} to reader {}.", self.ozid(), emsg, i;
                        Channel, Write));
                }
                n += 1;
            }
        }
        let (_, msgs) = res!(resp.recv_number(n, wait));
        for msg in msgs {
            match msg {
                OzoneMsg::Pong(..) => (),
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to the {}.", self.ozid(), emsg; Channel)),
                msg => return Err(err!(
                    "{}: Unexpected response to the {}: {:?}", self.ozid(), emsg, msg;
                    Channel, Unexpected)),
            }
        }
        Ok(())
    }

    // Does a sealed file hold bytes that a collection would remove, or is one at work on it?
    fn holds_old_bytes(fstat: &FileState) -> bool {
        !fstat.is_live() && (
            fstat.gc_active() ||
            fstat.get_old_sum() > 0 ||
            (!fstat.data_map_empty() && fstat.is_all_old())
        )
    }

    /// Instruct the wbots to increment to their next live files, to provide a clean slate for
    /// testing.
    pub fn new_live_files(&self) -> Outcome<()> {
        self.new_live_files_within(constant::CONTROL_REQUEST_WAIT)
    }

    // As `new_live_files`, for the writers to answer within the given wait.  A caller with a
    // deadline of its own passes what is left of it, since the writers answer only once they
    // reach the order in their queues.
    fn new_live_files_within(&self, wait: Wait) -> Outcome<()> {
        let emsg = "new live files request";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::NewLiveFile(None, resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        // A control operation like `activate_gc`, and queued behind the same zone bot work.
        let nw = self.cfg().num_wbots();
        let max_wait = wait.max_wait;
        let (_, msgs) = match resp.recv_number(nw, wait) {
            Ok(v) => v,
            Err(e) if max_wait >= constant::CONTROL_REQUEST_TIMEOUT => return Err(
                self.control_failure(e, emsg, nw, "writer bots")),
            Err(e) => return Err(err!(e,
                "{}: The {} was not acknowledged by all {} writer bots within the {:?} that the \
                caller's deadline left.  A writer acknowledges it only once it reaches the order \
                in its queue.",
                self.ozid(), emsg, nw, max_wait;
                Channel, Timeout)),
        };
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "{}: In response to {}.", self.ozid(), emsg;
                    Channel)),
                OzoneMsg::Ok => (),
                msg => return Err(err!(
                    "{}: Unexpected response to {}: {:?}", self.ozid(), emsg, msg;
                    Channel)),
            };
        }
        Ok(())
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>
    InNamex for OzoneApi<UIDL, UID, ENC, KH, PR, CS>
{
    fn name_id(&self) -> Outcome<NamexId> {
        NamexId::try_from(constant::NAMEX_ID)
    }
}
