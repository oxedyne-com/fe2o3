use crate::{
    ws::core::WebSocketMessage,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    id::NumIdDat,
};
use oxedyne_fe2o3_syntax::SyntaxRef;

use std::{
    sync::{
        Arc,
        RwLock,
    },
};

use tokio::sync::broadcast;


pub trait WebSocketHandler:
    Clone
    + std::fmt::Debug
    + Send
    + Sync
{
    const DEV_REFRESH_MSG: &'static str = "dev_refresh";

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
        -> Outcome<Option<WebSocketMessage>>;
    
    fn handle_binary<
        const UIDL: usize,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &mut self,
        byts:   Vec<u8>,
        db:     Option<(Arc<RwLock<DB>>, UID)>,
        syntax: SyntaxRef,
        id:     &String,
    )
        -> Outcome<Option<WebSocketMessage>>;

    fn dev_receiver(&self, id: &String) -> Outcome<Option<broadcast::Receiver<()>>> {
        debug!("{}: No dev receiver has been defined to accept client refresh messages.", id);
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct WebSocketEchoHandler;

impl WebSocketHandler for WebSocketEchoHandler {
    fn handle_text<
        const UIDL: usize,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &mut self,
        txt:    String,
        _db:     Option<(Arc<RwLock<DB>>, UID)>,
        _syntax: SyntaxRef,
        id:     &String,
    )
        -> Outcome<Option<WebSocketMessage>>
    {
        trace!("{}: WebSocketEchoHandler received text message: '{}'", id, txt);
        let response = WebSocketMessage::Text(txt); // Echo.
        Ok(Some(response))
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
        trace!("{}: WebSocketEchoHandler received binary message of length {}: {:02x?}",
            id, byts.len(), byts);
        let response = WebSocketMessage::Binary(byts); // Echo.
        Ok(Some(response))
    }
}

#[derive(Clone, Debug)]
pub struct WebSocketSinkHandler;

impl WebSocketHandler for WebSocketSinkHandler {
    fn handle_text<
        const UIDL: usize,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &mut self,
        txt:    String,
        _db:     Option<(Arc<RwLock<DB>>, UID)>,
        _syntax: SyntaxRef,
        id:     &String,
    )
        -> Outcome<Option<WebSocketMessage>>
    {
        trace!("{}: WebSocketSinkHandler received text message: '{}'", id, txt);
        Ok(None)
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
        trace!("{}: WebSocketSinkHandler received binary message of length {}: {:02x?}",
            id, byts.len(), byts);
        Ok(None)
    }
}
