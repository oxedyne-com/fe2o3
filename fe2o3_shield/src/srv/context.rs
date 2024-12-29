use crate::srv::{
    cfg::ServerConfig,
    id,
};

use oxedize_fe2o3_core::{
    prelude::*,
    id::ParseId,
    path::NormPathBuf,
    rand::Rand,
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    sign::Signer,
};
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::{
    api::Hasher,
    csum::Checksummer,
};
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_net::{
    http::{
        handler::WebHandler,
        msg::HttpMessage,
    },
    id::Sid,
    //smtp::handler::EmailHandler,
    ws::{
        WebSocket,
        handler::WebSocketHandler,
    },
};
use oxedize_fe2o3_o3db::{
    O3db,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
};
use oxedize_fe2o3_syntax::core::SyntaxRef;

use std::{
    collections::BTreeMap,
    marker::PhantomData,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        RwLock,
    },
};


#[derive(Clone, Debug)]
pub struct Protocol<
    // Data on the wire
	WENC:   Encrypter,      // Symmetric encryption of data on the wire.
	WCS:    Checksummer,    // Checks integrity of data on the wire.
    POWH:   Hasher,         // Packet validation proof of work hasher.
	SGN:    Signer,         // Digitally signs wire packets.
	HS:     Encrypter,      // Asymmetric encryption of symmetric encryption key during handshake.
> {
    pub cfg:    ServerConfig,
    pub schms:  WireSchemes<WENC, WCS, POWH, SGN, HS>,
}

pub struct ServerContext<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    // Database
    ENC:    Encrypter,      // Symmetric encryption of database.
    KH:     Hasher,         // Hashes database keys.
    DB:     Database<UIDL, UID, ENC, KH>, 
    // Wire
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter,
> {
    pub cfg:        ServerConfig,
    pub root:       NormPathBuf,
    pub db:         Option<(Arc<RwLock<DB>>, UID)>,
    pub protocol:   Protocol<WENC, WCS, POWH, SGN, HS>,
    phantom3:       PhantomData<ENC>,
    phantom4:       PhantomData<KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    // Database
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    // Wire
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    Clone for ServerContext<UIDL, UID, ENC, KH, DB, WENC, WCS,POWH, SGN, HS>
{
    fn clone(&self) -> Self {
        Self {
            cfg:        self.cfg.clone(),
            root:       self.root.clone(),
            db:         self.db.clone(),
            protocol:   self.protocol.clone(),
            phantom3:   PhantomData,
            phantom4:   PhantomData,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    // Database
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    // Wire
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    ServerContext<UIDL, UID, ENC, KH, DB, WENC, WCS,POWH, SGN, HS>
{
    pub fn new(
        cfg:        ServerConfig,
        root:       NormPathBuf,
        db:         Option<(DB, UID)>,
        protocol:   Protocol<WENC, WCS, POWH, SGN, HS>,
    )
        -> Self
    {
        Self {
            cfg,
            root,
            db:         db.map(|(db, uid)| (Arc::new(RwLock::new(db)), uid)),
            protocol,
            phantom3:   PhantomData,
            phantom4:   PhantomData,
        }
    }

    pub fn clone_self(&self) -> Self {
        self.clone()
    }

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
