use crate::{
    http::{
        fields::HeaderFields,
        header::HttpMethod,
        msg::HttpMessage,
        loc::HttpLocator,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        RwLock,
    },
};


pub trait WebHandler:
    Clone
    + std::fmt::Debug
    + Send
    + Sync
{
    /// Handle a `GET`.
    ///
    /// `peer` is the address the request arrived from. A handler that cannot
    /// see who it is talking to cannot audit a failed login, attribute an
    /// abusive request, or refuse one address while serving another -- so the
    /// address is passed in rather than left to be reconstructed from headers
    /// a client controls.
    fn handle_get<
        const SIDL: usize,
        const UIDL: usize,
        SID:    NumIdDat<SIDL> + 'static,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &self,
        loc:            HttpLocator,
        response:       Option<HttpMessage>,
        body:           Vec<u8>,
        req_headers:    Arc<HeaderFields>,
        db:             Option<(Arc<RwLock<DB>>, UID)>,
        sid_opt:        &Option<SID>,
        peer:           SocketAddr,
        id:             &String,
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send;

    /// Handle a `POST`. See [`WebHandler::handle_get`] for `peer`.
    fn handle_post<
        const SIDL: usize,
        const UIDL: usize,
        SID:    NumIdDat<SIDL> + 'static,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &self,
        loc:            HttpLocator,
        response:       Option<HttpMessage>,
        body:           Vec<u8>,
        req_headers:    Arc<HeaderFields>,
        db:             Option<(Arc<RwLock<DB>>, UID)>,
        sid_opt:        &Option<SID>,
        peer:           SocketAddr,
        id:             &String,
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send;

    /// Handle a `PUT`, `DELETE` or `PATCH`. The default answers nothing, which the server
    /// turns into a `405`, so a handler that serves only `GET` and `POST` need not say so.
    fn handle_other<
        const SIDL: usize,
        const UIDL: usize,
        SID:    NumIdDat<SIDL> + 'static,
        UID:    NumIdDat<UIDL> + 'static,
        ENC:    Encrypter,
        KH:     Hasher,
        DB:     Database<UIDL, UID, ENC, KH>,
    >(
        &self,
        _method:        HttpMethod,
        _loc:           HttpLocator,
        _body:          Vec<u8>,
        _req_headers:   Arc<HeaderFields>,
        _db:            Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:       &Option<SID>,
        _peer:          SocketAddr,
        _id:            &String,
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    {
        std::future::ready(Ok(None))
    }
}
