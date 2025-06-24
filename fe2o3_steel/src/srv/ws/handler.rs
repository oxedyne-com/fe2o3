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
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
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
            self.db,
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
    //dev_receiver: Arc<Mutex<Option<broadcast::Receiver<()>>>>,
}

impl AppWebSocketHandler {

    pub fn new(dev_manager: Option<Arc<DevRefreshManager>>) -> Self {
        Self {
            dev_manager,
            //dev_receiver: Arc::new(Mutex::new(None)),
        }
    }

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
                        if let Dat::Tup2(mut tup2) = std::mem::take(&mut cmdrx.vals[0]) {
                            let k = std::mem::take(&mut tup2[0]);
                            let v = std::mem::take(&mut tup2[1]);
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

