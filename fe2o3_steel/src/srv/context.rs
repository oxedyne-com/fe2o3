use crate::srv::{
    cfg::{
        RedirectRule,
        ServerConfig,
    },
    id,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    id::ParseId,
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
use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_net::{
    http::{
        handler::WebHandler,
        msg::HttpMessage,
    },
    id::Sid,
    ws::{
        WebSocket,
        handler::WebSocketHandler,
    },
};
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
};
use oxedyne_fe2o3_syntax::core::SyntaxRef;

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VHOST RUNTIME                                                             │
// │                                                                           │
// │ One per configured vhost. Carries everything the request path needs to    │
// │ serve that specific site: handlers, hostnames for validation, redirects.  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Runtime data for a single virtual host.
///
/// Instances are built once at startup from `VhostConfig` entries and stored
/// in `Protocol::Web::vhosts`, keyed by every alias hostname.
#[derive(Clone, Debug)]
pub struct VhostRuntime<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    /// All hostnames this vhost answers to. The first is the canonical one.
    pub hostnames:      Vec<String>,
    /// Static file request handler for this vhost.
    pub web_handler:    WH,
    /// WebSocket request handler (may be shared across vhosts in practice).
    pub ws_handler:     WSH,
    /// WebSocket protocol syntax.
    pub ws_syntax:      SyntaxRef,
    /// Ordered redirect rules evaluated before the static file router.
    pub redirects:      Vec<RedirectRule>,
}

impl<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
>
    VhostRuntime<WH, WSH>
{
    /// Returns the canonical (primary) hostname of this vhost.
    pub fn primary_hostname(&self) -> &str {
        self.hostnames.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Returns `true` if `host` matches any hostname registered for this vhost.
    /// Comparison is case-insensitive.
    pub fn accepts_host(&self, host: &str) -> bool {
        let host_lc = host.to_lowercase();
        // Strip any :port suffix.
        let host_lc = match host_lc.find(':') {
            Some(i) => host_lc[..i].to_string(),
            None => host_lc,
        };
        self.hostnames.iter().any(|h| h.to_lowercase() == host_lc)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROTOCOL                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// The protocol dialect the server is speaking. Currently only `Web`.
#[derive(Clone, Debug)]
pub enum Protocol<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    /// HTTPS + WebSocket, multi-vhost.
    Web {
        /// Map from hostname (lower case) to the runtime for that vhost.
        /// Every alias hostname has its own entry pointing at the same Arc.
        vhosts:         Arc<HashMap<String, Arc<VhostRuntime<WH, WSH>>>>,
        /// Primary hostname of the vhost used when SNI is absent or unknown.
        default_vhost:  String,
        /// Global development mode flag.
        dev_mode:       bool,
    },
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SERVER CONTEXT                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Shared state threaded through the server. Cheaply cloneable.
pub struct ServerContext<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,        // Symmetric encryption of database.
    KH:     Hasher,           // Hashes database keys.
    DB:     Database<UIDL, UID, ENC, KH>,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    pub cfg:        ServerConfig,
    pub root:       NormPathBuf,
    pub db:         Option<(Arc<RwLock<DB>>, UID)>,
    pub protocol:   Protocol<WH, WSH>,
    phantom3:       PhantomData<ENC>,
    phantom4:       PhantomData<KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
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
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    /// Create a new server context.
    pub fn new(
        cfg:        ServerConfig,
        root:       NormPathBuf,
        db:         Option<(DB, UID)>,
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

    /// Clone the context (explicit alias for situations where type inference
    /// on the derived impl causes issues).
    pub fn clone_self(&self) -> Self {
        self.clone()
    }

    /// Look up the vhost runtime for a given SNI hostname. Returns the default
    /// vhost when SNI is absent or the name is not registered.
    pub fn vhost_for(&self, sni: Option<&str>) -> Arc<VhostRuntime<WH, WSH>> {
        match &self.protocol {
            Protocol::Web { vhosts, default_vhost, .. } => {
                if let Some(name) = sni {
                    if let Some(vh) = vhosts.get(&name.to_lowercase()) {
                        return vh.clone();
                    }
                }
                // Fall through to default.
                match vhosts.get(&default_vhost.to_lowercase()) {
                    Some(vh) => vh.clone(),
                    None => {
                        // Should not happen if startup validated properly.
                        // Return the first entry if any; otherwise panic is
                        // impossible here because start-up would have failed.
                        vhosts.values().next().cloned().expect(
                            "ServerContext::vhost_for: no vhosts configured \
                            -- this should have been rejected at start-up.",
                        )
                    }
                }
            }
        }
    }

    /// Generate a short random identifier for error messages.
    pub fn err_id() -> String {
        Rand::generate_random_string(6, "abcdefghikmnpqrstuvw0123456789")
    }

    /// Extract the session id cookie from an HTTP message, if valid.
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

/// Construct a fresh Ozone database handle for a Steel application.
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

/// Return a typed `None` matching the database type parameters used by Steel.
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

/// Construct a WebSocket client helper without a database handle.
pub fn new_ws_no_db<
    'a,
    S:      AsyncRead + AsyncWrite + Unpin,
    WSH:    WebSocketHandler,
>(
    stream:     &'a mut S,
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
