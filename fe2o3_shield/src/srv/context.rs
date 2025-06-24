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
    let cfg = OzoneConfig {
        // Key hashing
        bytes_before_hashing:           32,
        // Caches
        cache_size_limit_bytes:         100_000_000,
        init_load_caches:               true,
        // Files
        data_file_max_bytes:            1_000_000,
        // Chunking
        rest_chunk_threshold:           1_500,
        rest_chunk_bytes:               64,
        // Bots
        num_cbots_per_zone:             2,
        num_fbots_per_zone:             2,
        num_igbots_per_zone:            2,
        num_rbots_per_zone:             2,
        num_wbots_per_zone:             1,
        num_sbots:                      2,
        // Zones
        num_zones:                      2,
        zone_state_update_secs:         1, 
        zone_overrides:                 BTreeMap::new(),
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
