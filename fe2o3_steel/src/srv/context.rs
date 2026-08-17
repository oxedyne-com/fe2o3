use crate::srv::{
    admin::{
        state::AdminState,
        traffic::TrafficRecorder,
    },
    cfg::{
        ProxyRoute,
        RedirectRule,
        ServerConfig,
        WsRoute,
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

#[derive(Clone, Debug)]
pub struct VhostRuntime<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    pub hostnames:      Vec<String>,
    pub web_handler:    WH,
    pub ws_handler:     WSH,
    pub ws_syntax:      SyntaxRef,
    pub redirects:      Vec<RedirectRule>,
    pub proxy_routes:   Vec<ProxyRoute>,
    pub ws_routes:      Vec<WsRoute>,
    pub term_manager:   Option<Arc<crate::srv::ws::term::TerminalManager>>,
    pub uses_sessions:  bool,
}

impl<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
>
    VhostRuntime<WH, WSH>
{
    pub fn primary_hostname(&self) -> &str {
        self.hostnames.first().map(|s| s.as_str()).unwrap_or("")
    }

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

#[derive(Clone, Debug)]
pub enum Protocol<
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    Web {
        vhosts:         Arc<HashMap<String, Arc<VhostRuntime<WH, WSH>>>>,
        default_vhost:  String,
        dev_mode:       bool,
    },
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SERVER CONTEXT                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

pub type VhostDbs<const UIDL: usize, UID, DB> =
    Arc<RwLock<HashMap<String, (Arc<RwLock<DB>>, UID)>>>;

#[derive(Clone, Debug)]
pub struct VhostDbSpec {
    pub vhost_key:  String,
    pub db_dir:     std::path::PathBuf,
}

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
    pub vhost_dbs:  VhostDbs<UIDL, UID, DB>,
    pub protocol:   Protocol<WH, WSH>,
    pub traffic:    Option<Arc<TrafficRecorder>>,
    pub admin_state: Option<Arc<AdminState>>,
    pub db_specs:   Vec<VhostDbSpec>,
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
            cfg:            self.cfg.clone(),
            root:           self.root.clone(),
            vhost_dbs:      self.vhost_dbs.clone(),
            protocol:       self.protocol.clone(),
            traffic:        self.traffic.clone(),
            admin_state:    self.admin_state.clone(),
            db_specs:       self.db_specs.clone(),
            phantom3:       PhantomData,
            phantom4:       PhantomData,
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
    pub fn new(
        cfg:            ServerConfig,
        root:           NormPathBuf,
        vhost_dbs:      VhostDbs<UIDL, UID, DB>,
        db_specs:       Vec<VhostDbSpec>,
        protocol:       Protocol<WH, WSH>,
        traffic:        Option<Arc<TrafficRecorder>>,
        admin_state:    Option<Arc<AdminState>>,
    )
        -> Self
    {
        Self {
            cfg,
            root,
            vhost_dbs,
            db_specs,
            protocol,
            traffic,
            admin_state,
            phantom3:   PhantomData,
            phantom4:   PhantomData,
        }
    }

    pub fn db_for_vhost(
        &self,
        vhost_key: &str,
    )
        -> Option<(Arc<RwLock<DB>>, UID)>
    {
        let guard = match self.vhost_dbs.read() {
            Ok(g) => g,
            Err(_) => {
                // This returns `Option`, not `Outcome`, so a poisoned lock
                // cannot be propagated. Report it and answer as though the
                // vhost has no database: the caller degrades to a 404 or a
                // 503 rather than serving from a map nobody can read.
                fault!("The vhost database map lock is poisoned; treating \
                    '{}' as having no database.", vhost_key);
                return None;
            }
        };
        guard.get(&vhost_key.to_lowercase()).cloned()
    }

    pub fn is_sealed(&self) -> bool {
        match &self.admin_state {
            Some(state) => state.is_sealed(),
            None => false,
        }
    }

    pub fn db_pending_for_vhost(&self, vhost_key: &str) -> bool {
        if !self.is_sealed() {
            return false;
        }
        let key = vhost_key.to_lowercase();
        self.db_specs.iter().any(|spec| spec.vhost_key == key)
    }

    pub fn clone_self(&self) -> Self {
        self.clone()
    }

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
    // Start from the library's own production configuration and state only what a Steel server
    // deliberately wants different.  This was previously a full struct literal, and it carried
    // the values from `o3db_sync`'s *test* setup -- a 1.5 KB chunking threshold and a 64-byte
    // chunk size -- so every Steel store split any value past 1.5 KB into 64-byte pieces.  An
    // exhaustive literal is what let that happen and what kept it: it has to restate every field,
    // so a wrong one is invisible among the right ones, and a field added upstream cannot reach a
    // caller who has already spelled them all out.  Naming only the deviations makes each one a
    // decision, and everything else tracks the library.
    let cfg = OzoneConfig {
        // A Steel server may hold several stores, so it takes a tenth of the library's cache.
        cache_size_limit_bytes:         100_000_000,
        // One writer per zone: the server's stores are small and write-light.
        num_wbots_per_zone:             1,
        // Zone state is reported more often than the library default, so the dashboard's view of
        // a store is near-live rather than five seconds stale.
        zone_state_update_secs:         1,
        // No per-zone size caps: a Steel store grows with the application that owns it.
        zone_overrides:                 BTreeMap::new(),
        // Everything else -- the chunking above all -- is the library's own production default.
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

pub fn no_db()
    -> Outcome<HashMap<String, (Arc<RwLock<O3db<
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
    Ok(HashMap::new())
}

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
