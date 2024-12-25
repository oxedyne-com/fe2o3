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
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
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

use tokio::io::{
    AsyncRead,
    AsyncWrite,
};


#[derive(Clone, Debug)]
pub enum Protocol<
    //EH:     EmailHandler,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    //Email {
    //    handler: EH,
    //},
    Web {
        web_handler:    WH,
        ws_handler:     WSH,
        ws_syntax:      SyntaxRef,
        dev_mode:       bool,
    },
}

pub struct ServerContext<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,        // Symmetric encryption of database.
    KH:     Hasher,           // Hashes database keys.
    DB:     Database<UIDL, UID, ENC, KH>, 
    //EH:     EmailHandler,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    pub cfg:        ServerConfig,
    pub root:       NormPathBuf,
    pub db:         Option<(Arc<RwLock<DB>>, UID)>,
    pub protocol:   Protocol<WH, WSH>,//Protocol<EH, WH, WSH>,
    phantom3:       PhantomData<ENC>,
    phantom4:       PhantomData<KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    //EH:     EmailHandler + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    //Clone for ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>
    Clone for ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
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
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    //EH:     EmailHandler + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    //ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    pub fn new(
        cfg:        ServerConfig,
        root:       NormPathBuf,
        db:         Option<(DB, UID)>,
        //protocol:   Protocol<EH, WH, WSH>,
        protocol:   Protocol<WH, WSH>,
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

    pub fn get_session_id(
        msg:        &HttpMessage,
        src_addr:   &SocketAddr,
    )
        -> Option<Sid>
    {
        match msg.header.fields.get_session_id() {
            Some(sid_string) => match Sid::parse_id(&sid_string) {
                Ok(n) => Some(n),
                Err(e) => {
                    error!(e, "The session cookie string '{}' in a message from \
                        {:?} cannot be decoded to a {}.",
                        sid_string, src_addr, std::any::type_name::<Sid>());
                    None
                },
            },
            None => None,
        }
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

pub fn no_db()
    -> Outcome<Option<(Arc<RwLock<O3db<
        { id::UID_LEN },
        id::Uid,
        EncryptionScheme,
        HashScheme,
        HashScheme,
        ChecksumScheme,
    >>>,
        id::Uid,
    )>>
{
    Ok(None)
}

pub fn new_ws_no_db<
    'a,
    S:      AsyncRead + AsyncWrite + Unpin,
    WSH:    WebSocketHandler,
>(
    stream: &'a mut S,
    ws_handler: WSH,
)
    -> Outcome<WebSocket<
        'a,
        { id::UID_LEN },
        id::Uid,
        EncryptionScheme,
        HashScheme,
        O3db<
            { id::UID_LEN },
            id::Uid,
            EncryptionScheme,
            HashScheme,
            HashScheme,
            ChecksumScheme,
        >,
        S,
        WSH,
    >>
{
    Ok(WebSocket::<
        '_,
        { id::UID_LEN },
        id::Uid,
        EncryptionScheme,
        HashScheme,
        O3db<
            { id::UID_LEN },
            id::Uid,
            EncryptionScheme,
            HashScheme,
            HashScheme,
            ChecksumScheme,
        >,
        S,
        WSH,
    >::new_client(
        stream,
        ws_handler,
        10,
        20,
    ))
}
