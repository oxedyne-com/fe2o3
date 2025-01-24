use crate::{
    prelude::*,
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
            bot_file::GcControl,
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
    file::zdir::ZoneDir,
};

use oxedize_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
    id::NumIdDat,
};
use oxedize_fe2o3_hash::{
    csum::{
        ChecksummerDefAlt,
        ChecksumScheme,
    },
};
use oxedize_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
};
use oxedize_fe2o3_namex::id::{
    InNamex,
    NamexId,
};

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        Instant,
    },
};


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

        // Generate the hash.
        let hash = self.schemes().key_hasher()
            .or_hash(&[&kbuf], constant::KEY_HASH_SALT, schms2.map(|s| s.key_hasher()))
            .as_hashform();
        let (cbwind, chash) = res!(ChooseCache::<PR>::choose_cbot(
            &hash,
            nz,
            nc,
        ));

        // Keys and values are wrapped in a fixed width `Dat::BU8`, `Dat::BU16`,
        // `Dat::BU32`, or `Dat::BU64`.
        let new_key = res!(Dat::wrap_bytes_var(hash.as_vec()));
        Ok((
            new_key,
            cbwind,
            chash,
        ))
    }

    // Write API, for general public use.
    
    /// Insert key-value `Dat`icles using the given data scheme overrides.  A `Responder` channel
    /// is returned.  If the value is not chunked, this will carry a single `OzoneMsg::KeyExists`
    /// message indicating whether or not the key was already present.  When chunked, one
    /// `OzoneMsg::KeyChunkExists` will be sent for each chunk.  Receipt of all expected messages
    /// indicates operation completion.
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
    /// `OzoneMsg::Chunks` containing the number of chunks.  If there was no chunking this will be
    /// followed by an `OzoneMsg::KeyExists` message indicating whether the key was present.  In
    /// the case of chunking, an `OzoneMsg::KeyChunkExists` message will follow for the bunch key
    /// (with index 0) and each chunk, in undefined order.  These messages indicate successful
    /// completion of each operation.
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
        let msgs = res!(self.prepare_write_dat(
            k,
            v,
            user,
            schms2,
            resp.clone(),
        ));
        let nchunks = msgs.len(); 
        if resp.is_some() {
            res!(resp.send(OzoneMsg::Chunks(nchunks)));
        }
        res!(self.store_bytes(msgs));
        Ok(nchunks)
    }

    /// The key and value `Dat`icles are serialised here and then sent for final processing.
    pub fn prepare_write_dat(
        &self,
        k:      Dat,
        v:      Dat,
        user:   UID,
        schms2: Option<&RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
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
        )
    }

    /// This is the last step in preparing the serialised data for storage, where
    /// `RestSchemesOverride` is finally invoked.  This influences how the key is hashed, if and
    /// how the data is chunked, and if and how those chunks are encrypted.  `OzoneMsg`s are
    /// returned, ready for sending to `WriteBot`s.
    pub fn prepare_write(
        &self,
        k:          Vec<u8>,
        mut vbuf:   Vec<u8>,
        user:       UID,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        resp:       Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>>
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
        debug!("Encryption is on: {}", encryption_on);
        if encryption_on {
            vbuf = res!(
                self.schemes().encrypter().or_encrypt(&mut vbuf, schms2.map(|s| s.encrypter()))
            ); 
        }

        let mut msgs = Vec::new();
        let mut meta = Meta::new(user);
        res!(meta.stamp_time_now());

        // 4. Package the value, breaking into chunks if it is too big.
        if vbuf.len() >= chunk_threshold {
            let chunker = OzoneConfig::chunker(chunk_config);
            // 4.1 Chunk data.
            let (chunks, chunk_state) = res!(chunker.chunk(&vbuf));
            let datkeys = res!(chunker.keys(**resp.ticket(), &chunk_state));
            
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
        Ok(msgs)
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
        // 1. Normalise the key.
        let (kstored, cbwind, _chash) = res!(self.ozone_key_dat(k, schms2));
        let klen_cache = kstored.len();

        // 2. The value we use to indicate deletion is an unencrypted custom usr type.
        let v = Dat::Usr(id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
        let vstored = res!(v.as_bytes());

        // 3. Select a zone writer bot.
        let wbots = res!(self.chans().get_workers_of_type_in_zone(&WorkerType::Writer, cbwind.zind()));
        let (bot, bpind) = wbots.choose_bot(&ChooseBot::Randomly);

        // 4. Create the metadata.
        let mut meta = Meta::new(user);
        res!(meta.stamp_time_now());

        // 5. Send write request, with responder.
        match bot.send(OzoneMsg::Write {
            kstored,
            vstored,
            klen_cache,
            cind: None,
            meta,
            cbpind: **cbwind.bpind(),
            resp,
        }) {
            Err(e) => return Err(err!(e,
                "{}: While sending delete request to wbot {}.",
                self.ozid(), WorkerInd::new(*cbwind.zind(), bpind);
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
            (Some((Dat::Tup5u64(tup), meta)), _) =>
                // Fetch the chunks.
                match res!(self.fetch_chunks(&Dat::Tup5u64(tup), schms2)) {
                    Dat::BU8(v)   |
                    Dat::BU16(v)  |
                    Dat::BU32(v)  |
                    Dat::BU64(v)  => match Dat::from_bytes(&v) {
                        Err(e) => Err(err!(e,
                            "Could not form a Dat from the chunked value bytes, \
                            this could be due to the use of an encryption scheme \
                            differing from the one provided ({}).",
                            enc.or_debug(or_enc);
                            Decode, Bytes, Unexpected)),
                        Ok((dat, _)) => Ok(Some((dat, meta))),
                    },
                    dat => Err(err!( "Unexpected Dat {:?} returned.", dat;
                        Decode, Bytes, Unexpected)),
                },
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

    /// Activate garbage collection by sending a control message to the igbots via the zbots, via the supervisor.
    pub fn activate_gc(&self, on: bool) -> Outcome<()> {
        info!("Activating garbage collection...");
        let emsg = "garbage collection activation";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::GcControl(GcControl::On(on), resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        let (_, msgs) = res!(resp.recv_number(self.cfg().num_zones(), constant::USER_REQUEST_WAIT));
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
        warn!("All {} caches successfully cleared.", n);
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
        info!("Cache dump summary");
        info!("+-----------+--------------+--------------+");
        info!("|   Cache   |   Entries    |    Size [B]  |");
        info!("+-----------+--------------+--------------+");
        for (wind, cache) in &sorted {
            info!("|{:^11}|{:>13} |{:>13} |",
                fmt!("{}", wind),
                cache.map().len(),
                cache.get_size(),
            ); 
        }
        info!("+-----------+--------------+--------------+");
        for (wind, cache) in sorted {
            let mut total_size = 0;
            info!("{} cache dump of {} entries:", wind, cache.map().len()); 
            if cache.map().len() == 0 {
                info!(" No cache entries."); 
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

                        info!(" kbyt = {:02x?} vlen = {} floc = {:?}",
                            kbyt, vlen, mloc.file_location(),
                        );
                        total_size += size;
                    }
                }
            }
            info!("{} cache size estimate: {} [B]", wind, total_size);
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

    /// Dump all zone file states to the log file.
    pub fn dump_file_states(&self, wait: Wait) -> Outcome<()> {
        // Gather.
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
        // Display.
        for (wind, fstates) in sorted {
            info!("{} file states dump:", wind); 
            if fstates.map().len() == 0 {
                info!(" None"); 
            } else {
                for (fnum, fstat) in fstates.map() {
                    info!(" {:10} {:?} old = {:.1}%",
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

    pub fn list_files(&self, wait: Wait) -> Outcome<()> {

        info!("Directory listing for {} zones, key:", self.cfg().num_zones());
        info!(" Typ: f File | d Directory | s Symlink");
        info!(" Size: in bytes");
        info!(" Mod: seconds since last modified");
        info!(" Name: object label");

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
        for (zind, zmap) in map {
            let mut total_size = 0;
            info!("{:?} directory", zind);
            info!("+-----+--------------+--------------+-----------------------------------------------------");
            info!("| Typ |   Size [B]   |    Mod [s]   | Name");
            info!("+-----+--------------+--------------+-----------------------------------------------------");
            for (_key, entry) in zmap {
                info!(
                    "|  {}  |{:>13} |{:>13} | {}",
                    entry.typ,
                    entry.size,
                    entry.mods,
                    entry.name,
                );
                total_size += entry.size;
            }
            info!("+-----+--------------+--------------+-----------------------------------------------------");
            info!("|     |{:>13} |              |", total_size);
            info!("+-----+--------------+--------------+-----------------------------------------------------");
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

    /// Instruct the wbots to increment to their next live files, to provide a clean slate for
    /// testing.
    pub fn new_live_files(&self) -> Outcome<()> {
        let emsg = "new live files request";
        let resp = self.responder();
        if let Err(e) = self.chans().sup().send(
            OzoneMsg::NewLiveFile(None, resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send {} to supervisor.", self.ozid(), emsg;
                Channel, Write));
        }
        let (_, msgs) = res!(resp.recv_number(self.cfg().num_wbots(), constant::USER_REQUEST_WAIT));
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
