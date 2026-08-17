use crate::srv::{
    cfg::ServerConfig,
    msg::{
        core::IdTypes,
        protocol::{
            Protocol,
            ProtocolTypes,
        },
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    path::NormPathBuf,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_net::id;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
};

use std::{
    collections::BTreeMap,
    marker::PhantomData,
    path::Path,
    sync::{
        Arc,
        RwLock,
    },
};


#[derive(Clone, Debug)]
pub struct ServerContext<
    const C: usize, // Length of user secret pow code.
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL>,
    // Database
    ENC:    Encrypter,      // Symmetric encryption of database.
    KH:     Hasher,         // Hashes database keys.
    DB:     Database<UL, <P::ID as IdTypes<ML, SL, UL>>::U, ENC, KH>,
> {
    pub cfg:        ServerConfig,
    pub root:       NormPathBuf,
    pub db:         Option<(Arc<RwLock<DB>>, <P::ID as IdTypes<ML, SL, UL>>::U)>,
    pub protocol:   Protocol<C, ML, SL, UL, P>,
    phantom3:       PhantomData<ENC>,
    phantom4:       PhantomData<KH>,
}

impl<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL> + 'static,
    // Database
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UL, <P::ID as IdTypes<ML, SL, UL>>::U, ENC, KH> + 'static, 
>
    ServerContext<C, ML, SL, UL, P, ENC, KH, DB>
{
    pub fn new(
        cfg:        ServerConfig,
        root:       NormPathBuf,
        db:         Option<(DB, <P::ID as IdTypes<ML, SL, UL>>::U)>,
        protocol:   Protocol<C, ML, SL, UL, P>,
    )
        -> Self
    {
        Self {
            cfg,
            root,
            db: db.map(|(db, uid)| (Arc::new(RwLock::new(db)), uid)),
            protocol,
            phantom3:   PhantomData,
            phantom4:   PhantomData,
        }
    }

    //pub fn clone_self(&self) -> Self {
    //    self.clone()
    //}

    pub fn err_id() -> String {
        Rand::generate_random_string(6, "abcdefghikmnpqrstuvw0123456789")
    }
}

pub fn new_db(
    db_root: &Path,
    enc_key: &[u8],
)
    -> Outcome<O3db<
        { id::UID_LEN },
        id::Uid,
        EncryptionScheme,
        HashScheme,
        HashScheme,
        ChecksumScheme,
    >>
{
    // Only the deviations from the library's own configuration are stated, so that a field
    // added upstream reaches this caller instead of failing to compile here, and so that
    // each value below reads as a decision rather than as one entry in a list of twenty.
    // The values are those this server has always used; nothing about the store changes.
    let cfg = OzoneConfig {
        // A Shield server's store is small, so it takes a tenth of the library's cache.
        cache_size_limit_bytes:         100_000_000,
        // Files
        data_file_max_bytes:            1_000_000,
        // Chunking. These are far below the library's own figures and match its *test*
        // setup, which is where Steel found the same pair and replaced them; they are
        // left alone here because changing them changes how values already in a Shield
        // store were split. Worth revisiting deliberately.
        rest_chunk_threshold:           1_500,
        rest_chunk_bytes:               64,
        // One writer per zone: the store is write-light.
        num_wbots_per_zone:             1,
        // Zone state is reported more often than the library default.
        zone_state_update_secs:         1,
        // No per-zone size caps.
        zone_overrides:                 BTreeMap::new(),
        ..Default::default()
    };


    let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(enc_key));
    let crc32 = ChecksumScheme::new_crc32();
    let schms_input = RestSchemesInput::new(
        Some(aes_gcm.clone()),
        None::<HashScheme>,
        None::<HashScheme>,
        Some(crc32.clone()),
    );
    O3db::new(
        &db_root,
        Some(cfg),
        schms_input,
        id::Uid::default(),
    )
}
