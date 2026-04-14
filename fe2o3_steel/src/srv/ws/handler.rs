use crate::srv::{
    constant,
    context::{
        ServerContext,
    },
    dev::refresh::DevRefreshManager,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_hash::{
    kdf::KeyDerivationScheme,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::{
    api::Hasher,
    kdf::KeyDeriver,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_net::{
    http::{
        handler::WebHandler,
        msg::HttpMessage,
    },
    //smtp::handler::EmailHandler,
    ws::{
        WebSocket,
        core::WebSocketMessage,
        handler::WebSocketHandler,
    },
};
use oxedyne_fe2o3_syntax::{
    SyntaxRef,
    msg::{
        Msg,
        MsgCmd,
    },
};


use std::{
    sync::{
        Arc,
        RwLock,
        //Mutex,
    },
};

use tokio::{
    self,
    io::{
        AsyncRead,
        AsyncWrite,
    },
    sync::broadcast,
};


impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    //EH:     EmailHandler,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    //ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    pub async fn handle_websocket<
        'a,
        S: AsyncRead + AsyncWrite + Unpin,
    >(
        self,
        stream:     &'a mut S,
        ws_handler: WSH,
        ws_syntax:  SyntaxRef,
        vhost_db:   Option<(Arc<RwLock<DB>>, UID)>,
        request:    HttpMessage,
        id:         &String,
    )
        -> Outcome<()>
    {
        let mut ws = WebSocket::new_server(
            stream,
            ws_handler.clone(),
            constant::WEBSOCKET_CHUNK_SIZE,
            constant::WEBSOCKET_CHUNKING_THRESHOLD,
        );
        match ws.connect_as_server(request).await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: WebSocket handshake failed.", id;
                IO, Network, Wire)),
        };

        ws.listen(
            vhost_db,
            ws_syntax,
            Some(self.cfg.ws_ping_interval_secs),
            self.cfg.server_max_errors_allowed,
            id,
        ).await
    }
}

#[derive(Clone, Debug)]
pub struct AppWebSocketHandler {
    dev_manager: Option<Arc<DevRefreshManager>>,
    /// Session id of the browser that opened this WebSocket, as read from
    /// the HttpOnly session cookie at the upgrade request. Populated via
    /// `with_sid()` before the handshake. `None` means the client sent no
    /// session cookie; session-scoped commands will then reject.
    sid: Option<String>,
}

impl AppWebSocketHandler {

    pub fn new(dev_manager: Option<Arc<DevRefreshManager>>) -> Self {
        Self {
            dev_manager,
            sid: None,
        }
    }

    /// Build the scoped database key used by session-scoped commands, of
    /// the form `sess:<sid>:<user_key>`. Returns `None` when this handler
    /// has no session id attached.
    fn scoped_sess_key(&self, user_key: &str) -> Option<Dat> {
        self.sid.as_ref().map(|sid| {
            Dat::Str(fmt!("sess:{}:{}", sid, user_key))
        })
    }

    /// Build the session-metadata key `sess_meta:<sid>`. Returns `None`
    /// when this handler has no session id attached.
    fn sess_meta_key(&self) -> Option<Dat> {
        self.sid.as_ref().map(|sid| {
            Dat::Str(fmt!("sess_meta:{}", sid))
        })
    }

    /// Build the user record key `user:<username>`.
    fn user_key(username: &str) -> Dat {
        Dat::Str(fmt!("user:{}", username))
    }

    /// Current unix seconds, or zero if the clock is before the epoch.
    fn unix_secs_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Default KDF name used for user passphrase hashing. Hardcoded to
    /// match the wallet kdf for now; will be promoted to a per-app config
    /// value when the shell command surface grows a setting for it.
    const AUTH_KDF_NAME: &'static str = "Argon2id_v0x13";

    fn response_text(
        syntax: SyntaxRef,
        cmd:    &str,
        vals:   Vec<Dat>,
    )
        -> Outcome<Option<WebSocketMessage>>
    {
        let mut response = res!(MsgCmd::new(syntax, cmd));
        for val in vals {
            response = res!(response.add_cmd_val(val));
        }
        trace!("Sending websocket message '{}'", response.to_string());
        return Ok(Some(WebSocketMessage::Text(response.to_string())));
    }

    fn check_syntax(
        syntax: SyntaxRef,
        msgcmd: &MsgCmd,
    )
        -> Outcome<()>
    {
        match syntax.get_cmd(&*msgcmd.name) {
            Some(cmd) => {
                let cmdcfg = cmd.config();
                if msgcmd.vals.len() != cmdcfg.vals.len() {
                    return Err(err!(
                        "The syntax '{}' command '{}' expects {} value(s), found {}.",
                        syntax.config().name,
                        msgcmd.name,
                        cmdcfg.vals.len(),
                        msgcmd.vals.len();
                        Input, Network, Mismatch));
                }
                for (i, (kind, _)) in cmdcfg.vals.iter().enumerate() {
                    if *kind != Kind::Unknown && *kind != msgcmd.vals[i].kind() {
                        return Err(err!(
                            "The syntax '{}' command '{}' expects value {} to be a '{:?}, found {:?}.",
                            syntax.config().name,
                            msgcmd.name,
                            i,
                            kind,
                            msgcmd.vals[i].kind();
                            Input, Network, Mismatch));
                    }
                }
            }
            None => {
                return Err(err!(
                    "No command '{}' found in syntax '{}'.",
                    msgcmd.name,
                    syntax.config().name;
                    Input, Network, Unknown));
            }
        }
        Ok(())
    }
}

/// `Syntax` accomodates multiple commands per message, we limit this to one here.
impl WebSocketHandler for AppWebSocketHandler {

    fn attach_sid(mut self, sid: Option<String>) -> Self {
        self.sid = sid;
        self
    }

    fn handle_text<
        const UIDL: usize,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &mut self,
        txt:    String,
        db:     Option<(Arc<RwLock<DB>>, UID)>,
        syntax: SyntaxRef,
        id:     &String,
    )
        -> Outcome<Option<WebSocketMessage>>
    {
        debug!("{}: AppWebSocketHandler received text message: '{}'", id, txt);

        let msgrx = Msg::new(syntax.clone());
        let msgrx = match msgrx.from_str(&txt, None) {
            Err(err) => {
                error!(err.clone());
                return Self::response_text(syntax, "error", vec![dat!(err.to_string())]);
            }
            Ok(msgrx) => msgrx,
        };

        if msgrx.cmds.len() != 1 {
            let err = err!(
                "Expected one command from syntax '{}', found {}.",
                syntax.config().name, msgrx.cmds.len();
                Invalid, Network, Input);
            error!(err.clone());
            return Self::response_text(syntax, "error", vec![dat!(err.to_string())]);
        }

        if let Some((cmd_name, mut cmdrx)) = msgrx.cmds.into_iter().next() {
            if let Err(err) = Self::check_syntax(syntax.clone(), &cmdrx) {
                error!(err.clone());
                return Self::response_text(syntax, "error", vec![dat!(err.to_string())]);
            }
            match cmd_name.as_str() {
                // ┌───────────────────────┐
                // │ DEVELOPMENT           │
                // └───────────────────────┘
                "dev_ping" => {
                    trace!("Received dev_ping");
                    if let Some(_manager) = &self.dev_manager {
                        return Self::response_text(syntax, "info", vec![dat!("pong")]);
                    } else {
                        return Self::response_text(syntax, "error",
                            vec![dat!("Dev mode not enabled.")]);
                    }
                }
                "dev_connect" => {
                    trace!("Received dev_connect");
                    if self.dev_manager.is_some() {
                        return Self::response_text(syntax, "info", vec![dat!("connected")]);
                    } else {
                        trace!("Not in dev mode");
                        return Self::response_text(syntax, "error",
                            vec![dat!("Dev mode not enabled.")]);
                    }
                }
                // ┌───────────────────────┐
                // │ GENERAL IO            │
                // └───────────────────────┘
                "echo" => return Ok(Some(WebSocketMessage::Text(txt))),
                // ┌───────────────────────┐
                // │ DATABASE IO           │
                // └───────────────────────┘
                "insert" => {
                    trace!("Received insert");
                    if let Some((ref db, uid)) = db {
                        let db = match db.write() {
                            Err(_err) => {
                                let err = err!(
                                    "While trying to access database.";
                                    Lock, Poisoned, Write);
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(v) => v,
                        };
                        {
                            let k = std::mem::take(&mut cmdrx.vals[0]);
                            let v = std::mem::take(&mut cmdrx.vals[1]);
                            let success = fmt!("Inserted value for key {} into database.", k);
                            match db.insert(
                                k,
                                v,
                                uid,
                                None,
                            ) {
                                Err(err) => {
                                    error!(err.clone());
                                    return Self::response_text(syntax,
                                        "error", vec![dat!(err.to_string())]);
                                }
                                Ok((exists, num_chunks)) => {
                                    let exists_txt = if exists {
                                        "exists"
                                    } else {
                                        "did not exist"
                                    };
                                    let txt = fmt!(
                                        "{} The key {}, {} chunks were used.",
                                        success, exists_txt, num_chunks,
                                    );
                                    return Self::response_text(syntax, "info", vec![dat!(txt)]);
                                }
                            }
                        }
                    }
                    let err = err!(
                        "Database not accessible for 'insert' command.";
                        Invalid, Network, Input);
                    error!(err.clone());
                    return Self::response_text(syntax, "error", vec![dat!(err.to_string())]);
                }
                "get_data" => {
                    if let Some((ref db, _uid)) = db {
                        let db = match db.read() {
                            Err(_err) => {
                                let err = err!(
                                    "While trying to access database.";
                                    Lock, Poisoned, Read);
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(v) => v,
                        };
                        match db.get(
                            &cmdrx.vals[0],
                            None,
                        ) {
                            Err(err) => {
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(Some((data, _meta))) => {
                                return Self::response_text(syntax, "data", vec![dat!(data)]);
                            }
                            Ok(None) => {
                                return Self::response_text(syntax, "data", vec![Dat::Empty]);
                            }
                        }
                    }
                    let err = err!(
                        "Database not accessible for 'get_data' command.";
                        Invalid, Network, Input);
                    error!(err.clone());
                    return Self::response_text(syntax, "error", vec![dat!(err.to_string())]);
                }
                // ┌───────────────────────┐
                // │ SESSION IO            │
                // └───────────────────────┘
                "sess_get" => {
                    trace!("{}: sess_get", id);
                    // The key the client sent is a user-facing string; the
                    // server prefixes it with `sess:<sid>:` so clients can
                    // never read outside their own session namespace.
                    let user_key = match &cmdrx.vals[0] {
                        Dat::Str(s) => s.clone(),
                        other => {
                            let err = err!(
                                "sess_get: key must be a string, got {:?}.",
                                other.kind();
                                Invalid, Network, Input);
                            error!(err.clone());
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    let scoped = match self.scoped_sess_key(&user_key) {
                        Some(k) => k,
                        None => {
                            let err = err!(
                                "sess_get: no session cookie attached to \
                                this connection.";
                                Invalid, Network, Input);
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    if let Some((ref db, _uid)) = db {
                        let db = match db.read() {
                            Err(_err) => {
                                let err = err!(
                                    "While trying to access database.";
                                    Lock, Poisoned, Read);
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(v) => v,
                        };
                        match db.get(&scoped, None) {
                            Err(err) => {
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(Some((data, _meta))) => {
                                return Self::response_text(syntax, "data",
                                    vec![dat!(data)]);
                            }
                            Ok(None) => {
                                return Self::response_text(syntax, "data",
                                    vec![Dat::Empty]);
                            }
                        }
                    }
                    let err = err!(
                        "Database not accessible for 'sess_get' command.";
                        Invalid, Network, Input);
                    error!(err.clone());
                    return Self::response_text(syntax, "error",
                        vec![dat!(err.to_string())]);
                }
                "sess_put" => {
                    trace!("{}: sess_put", id);
                    let user_key = match std::mem::take(&mut cmdrx.vals[0]) {
                        Dat::Str(s) => s,
                        other => {
                            let err = err!(
                                "sess_put: key must be a string, got {:?}.",
                                other.kind();
                                Invalid, Network, Input);
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    let value = std::mem::take(&mut cmdrx.vals[1]);
                    let scoped = match self.scoped_sess_key(&user_key) {
                        Some(k) => k,
                        None => {
                            let err = err!(
                                "sess_put: no session cookie attached to \
                                this connection.";
                                Invalid, Network, Input);
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    if let Some((ref db, uid)) = db {
                        let db = match db.write() {
                            Err(_err) => {
                                let err = err!(
                                    "While trying to access database.";
                                    Lock, Poisoned, Write);
                                error!(err.clone());
                                return Self::response_text(syntax,
                                    "error", vec![dat!(err.to_string())]);
                            }
                            Ok(v) => v,
                        };
                        match db.insert(scoped, value, uid, None) {
                            Err(err) => {
                                error!(err.clone());
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                            Ok((exists, num_chunks)) => {
                                let msg = fmt!(
                                    "sess_put ok, key {} exist, {} chunks.",
                                    if exists { "did" } else { "did not" },
                                    num_chunks,
                                );
                                return Self::response_text(syntax, "info",
                                    vec![dat!(msg)]);
                            }
                        }
                    }
                    let err = err!(
                        "Database not accessible for 'sess_put' command.";
                        Invalid, Network, Input);
                    error!(err.clone());
                    return Self::response_text(syntax, "error",
                        vec![dat!(err.to_string())]);
                }
                // ┌───────────────────────┐
                // │ AUTH                  │
                // └───────────────────────┘
                "register" => {
                    trace!("{}: register", id);
                    let username = match std::mem::take(&mut cmdrx.vals[0]) {
                        Dat::Str(s) => s,
                        other => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(fmt!("register: username must be Str, got {:?}.",
                                    other.kind()))]);
                        }
                    };
                    let passphrase = match std::mem::take(&mut cmdrx.vals[1]) {
                        Dat::Str(s) => s,
                        other => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(fmt!("register: passphrase must be Str, got {:?}.",
                                    other.kind()))]);
                        }
                    };
                    if username.is_empty() {
                        let err = err!(
                            "register: username must not be empty.";
                            Invalid, Network, Input);
                        return Self::response_text(syntax, "error",
                            vec![dat!(err.to_string())]);
                    }
                    let user_key = Self::user_key(&username);
                    // Guard against overwriting an existing user.
                    if let Some((ref db, _uid)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("register: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&user_key, None) {
                            Ok(Some(_)) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(fmt!(
                                        "register: user '{}' already exists.",
                                        username))]);
                            }
                            Ok(None) => (),
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    } else {
                        return Self::response_text(syntax, "error",
                            vec![dat!("register: no database available.")]);
                    }
                    // Derive the Argon2id hash of the passphrase.
                    let mut kdf = match KeyDerivationScheme::from_str(Self::AUTH_KDF_NAME) {
                        Ok(k) => k,
                        Err(err) => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    if let Err(err) = kdf.derive(passphrase.as_bytes()) {
                        return Self::response_text(syntax, "error",
                            vec![dat!(err.to_string())]);
                    }
                    let kdf_hash = match kdf.encode_to_string() {
                        Ok(s) => s,
                        Err(err) => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]);
                        }
                    };
                    // Build the user record.
                    let mut rec = DaticleMap::new();
                    rec.insert(dat!("kdf_name"), dat!(fmt!("{}", kdf)));
                    rec.insert(dat!("kdf_hash"), dat!(kdf_hash));
                    rec.insert(dat!("created_at"), Dat::U64(Self::unix_secs_now()));
                    let record = Dat::Map(rec);
                    // Write it.
                    if let Some((ref db, uid)) = db {
                        let db_w = match db.write() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("register: database write lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_w.insert(user_key, record, uid, None) {
                            Ok(_) => {
                                return Self::response_text(syntax, "info",
                                    vec![dat!(fmt!(
                                        "register: user '{}' created.", username))]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("register: no database available.")]);
                }
                "login" => {
                    trace!("{}: login", id);
                    let sid = match self.sid.clone() {
                        Some(s) => s,
                        None => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("login: no session cookie attached.")]);
                        }
                    };
                    let username = match std::mem::take(&mut cmdrx.vals[0]) {
                        Dat::Str(s) => s,
                        _ => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("login: username must be a string.")]);
                        }
                    };
                    let passphrase = match std::mem::take(&mut cmdrx.vals[1]) {
                        Dat::Str(s) => s,
                        _ => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("login: passphrase must be a string.")]);
                        }
                    };
                    let user_key = Self::user_key(&username);
                    let rec = if let Some((ref db, _)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("login: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&user_key, None) {
                            Ok(Some((data, _meta))) => data,
                            Ok(None) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!("login: invalid credentials.")]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    } else {
                        return Self::response_text(syntax, "error",
                            vec![dat!("login: no database available.")]);
                    };
                    // Extract kdf_name and kdf_hash from the stored record.
                    let (kdf_name, kdf_hash) = match &rec {
                        Dat::Map(m) => {
                            let name = match m.get(&dat!("kdf_name")) {
                                Some(Dat::Str(s)) => s.clone(),
                                _ => return Self::response_text(syntax, "error",
                                    vec![dat!("login: malformed user record (kdf_name).")]),
                            };
                            let hash = match m.get(&dat!("kdf_hash")) {
                                Some(Dat::Str(s)) => s.clone(),
                                _ => return Self::response_text(syntax, "error",
                                    vec![dat!("login: malformed user record (kdf_hash).")]),
                            };
                            (name, hash)
                        }
                        _ => return Self::response_text(syntax, "error",
                            vec![dat!("login: malformed user record (not a Map).")]),
                    };
                    // Rebuild the KDF and verify the passphrase.
                    let mut kdf = match KeyDerivationScheme::from_str(&kdf_name) {
                        Ok(k) => k,
                        Err(err) => return Self::response_text(syntax, "error",
                            vec![dat!(err.to_string())]),
                    };
                    if let Err(err) = kdf.decode_from_string(&kdf_hash) {
                        return Self::response_text(syntax, "error",
                            vec![dat!(err.to_string())]);
                    }
                    let ok = match kdf.verify(passphrase.as_bytes()) {
                        Ok(b) => b,
                        Err(err) => return Self::response_text(syntax, "error",
                            vec![dat!(err.to_string())]),
                    };
                    if !ok {
                        return Self::response_text(syntax, "error",
                            vec![dat!("login: invalid credentials.")]);
                    }
                    // Bind the session to the user.
                    let meta_key = Dat::Str(fmt!("sess_meta:{}", sid));
                    let mut meta = DaticleMap::new();
                    meta.insert(dat!("user"), dat!(username.clone()));
                    meta.insert(dat!("authenticated_at"),
                        Dat::U64(Self::unix_secs_now()));
                    let meta_rec = Dat::Map(meta);
                    if let Some((ref db, uid)) = db {
                        let db_w = match db.write() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("login: database write lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_w.insert(meta_key, meta_rec, uid, None) {
                            Ok(_) => {
                                return Self::response_text(syntax, "info",
                                    vec![dat!(fmt!(
                                        "login: authenticated as '{}'.", username))]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("login: no database available.")]);
                }
                "logout" => {
                    trace!("{}: logout", id);
                    let meta_key = match self.sess_meta_key() {
                        Some(k) => k,
                        None => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("logout: no session cookie attached.")]);
                        }
                    };
                    // Overwrite the session metadata with an empty map,
                    // marking the session as unauthenticated. Ozone does
                    // not currently expose a delete primitive on this
                    // surface; an empty record is treated as "not bound".
                    let empty_rec = Dat::Map(DaticleMap::new());
                    if let Some((ref db, uid)) = db {
                        let db_w = match db.write() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("logout: database write lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_w.insert(meta_key, empty_rec, uid, None) {
                            Ok(_) => {
                                return Self::response_text(syntax, "info",
                                    vec![dat!("logout: session unbound.")]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("logout: no database available.")]);
                }
                // ┌───────────────────────┐
                // │ USER IO               │
                // └───────────────────────┘
                "user_get" => {
                    trace!("{}: user_get", id);
                    let user_sub_key = match &cmdrx.vals[0] {
                        Dat::Str(s) => s.clone(),
                        other => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(fmt!(
                                    "user_get: key must be a string, got {:?}.",
                                    other.kind()))]);
                        }
                    };
                    // Look up the session's authenticated user.
                    let meta_key = match self.sess_meta_key() {
                        Some(k) => k,
                        None => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("user_get: no session cookie attached.")]);
                        }
                    };
                    let username = if let Some((ref db, _)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_get: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&meta_key, None) {
                            Ok(Some((Dat::Map(m), _))) => match m.get(&dat!("user")) {
                                Some(Dat::Str(s)) if !s.is_empty() => s.clone(),
                                _ => return Self::response_text(syntax, "error",
                                    vec![dat!("user_get: session is not authenticated.")]),
                            },
                            Ok(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_get: session is not authenticated.")]),
                            Err(err) => return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]),
                        }
                    } else {
                        return Self::response_text(syntax, "error",
                            vec![dat!("user_get: no database available.")]);
                    };
                    // Read the user-scoped key.
                    let scoped = Dat::Str(fmt!("user:{}:{}", username, user_sub_key));
                    if let Some((ref db, _)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_get: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&scoped, None) {
                            Ok(Some((data, _))) => {
                                return Self::response_text(syntax, "data",
                                    vec![dat!(data)]);
                            }
                            Ok(None) => {
                                return Self::response_text(syntax, "data",
                                    vec![Dat::Empty]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("user_get: no database available.")]);
                }
                "user_put" => {
                    trace!("{}: user_put", id);
                    let user_sub_key = match std::mem::take(&mut cmdrx.vals[0]) {
                        Dat::Str(s) => s,
                        other => {
                            return Self::response_text(syntax, "error",
                                vec![dat!(fmt!(
                                    "user_put: key must be a string, got {:?}.",
                                    other.kind()))]);
                        }
                    };
                    let value = std::mem::take(&mut cmdrx.vals[1]);
                    // Same authentication lookup as user_get.
                    let meta_key = match self.sess_meta_key() {
                        Some(k) => k,
                        None => {
                            return Self::response_text(syntax, "error",
                                vec![dat!("user_put: no session cookie attached.")]);
                        }
                    };
                    let username = if let Some((ref db, _)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_put: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&meta_key, None) {
                            Ok(Some((Dat::Map(m), _))) => match m.get(&dat!("user")) {
                                Some(Dat::Str(s)) if !s.is_empty() => s.clone(),
                                _ => return Self::response_text(syntax, "error",
                                    vec![dat!("user_put: session is not authenticated.")]),
                            },
                            Ok(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_put: session is not authenticated.")]),
                            Err(err) => return Self::response_text(syntax, "error",
                                vec![dat!(err.to_string())]),
                        }
                    } else {
                        return Self::response_text(syntax, "error",
                            vec![dat!("user_put: no database available.")]);
                    };
                    let scoped = Dat::Str(fmt!("user:{}:{}", username, user_sub_key));
                    if let Some((ref db, uid)) = db {
                        let db_w = match db.write() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("user_put: database write lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_w.insert(scoped, value, uid, None) {
                            Ok((exists, num_chunks)) => {
                                let msg = fmt!(
                                    "user_put ok, key {} exist, {} chunks.",
                                    if exists { "did" } else { "did not" },
                                    num_chunks,
                                );
                                return Self::response_text(syntax, "info",
                                    vec![dat!(msg)]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("user_put: no database available.")]);
                }
                "whoami" => {
                    trace!("{}: whoami", id);
                    // With no session cookie at all, report unauthenticated
                    // and stop. This is the expected state for a fresh
                    // connection that has not yet been through the anonymous
                    // session-issuance path.
                    let meta_key = match self.sess_meta_key() {
                        Some(k) => k,
                        None => {
                            let mut m = DaticleMap::new();
                            m.insert(dat!("authenticated"), Dat::Bool(false));
                            return Self::response_text(syntax, "data",
                                vec![Dat::Map(m)]);
                        }
                    };
                    if let Some((ref db, _)) = db {
                        let db_r = match db.read() {
                            Err(_) => return Self::response_text(syntax, "error",
                                vec![dat!("whoami: database read lock poisoned.")]),
                            Ok(v) => v,
                        };
                        match db_r.get(&meta_key, None) {
                            Ok(Some((data, _))) => {
                                let user_opt = match &data {
                                    Dat::Map(m) => match m.get(&dat!("user")) {
                                        Some(Dat::Str(s)) if !s.is_empty() => {
                                            Some(s.clone())
                                        }
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                let mut out = DaticleMap::new();
                                out.insert(dat!("authenticated"),
                                    Dat::Bool(user_opt.is_some()));
                                if let Some(u) = user_opt {
                                    out.insert(dat!("user"), dat!(u));
                                }
                                return Self::response_text(syntax, "data",
                                    vec![Dat::Map(out)]);
                            }
                            Ok(None) => {
                                let mut m = DaticleMap::new();
                                m.insert(dat!("authenticated"), Dat::Bool(false));
                                return Self::response_text(syntax, "data",
                                    vec![Dat::Map(m)]);
                            }
                            Err(err) => {
                                return Self::response_text(syntax, "error",
                                    vec![dat!(err.to_string())]);
                            }
                        }
                    }
                    return Self::response_text(syntax, "error",
                        vec![dat!("whoami: no database available.")]);
                }
                _ => {}
            }
        }
        unreachable!()
    }
    
    fn handle_binary<
        const UIDL: usize,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &mut self,
        byts:   Vec<u8>,
        _db:     Option<(Arc<RwLock<DB>>, UID)>,
        _syntax: SyntaxRef,
        id:     &String,
    )
        -> Outcome<Option<WebSocketMessage>>
    {
        debug!("{}: AppWebSocketHandler received binary message of length {}: {:02x?}",
            id, byts.len(), byts);
        let response = WebSocketMessage::Binary(byts); // Echo.
        Ok(Some(response))
    }

    fn dev_receiver(&self, id: &String) -> Outcome<Option<broadcast::Receiver<()>>> {
        if let Some(manager) = &self.dev_manager {
            debug!("{}: New client subscribed to dev refresh notifications.", id);
            Ok(Some(manager.get_receiver()))
        } else {
            debug!("{}: No dev receiver available to accept client refresh messages.", id);
            Ok(None)
        }
    }
}

