//use crate::{
//    http::{
//        msg::HttpMessage,
//        loc::HttpLocator,
//    },
//};
//
//use oxedyne_fe2o3_core::prelude::*;
//use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
//use oxedyne_fe2o3_iop_db::api::Database;
//use oxedyne_fe2o3_iop_hash::api::Hasher;
//use oxedyne_fe2o3_jdat::id::NumIdDat;
//
//use std::{
//    sync::{
//        Arc,
//        RwLock,
//    },
//};


pub trait EmailHandler:
    Clone
    + std::fmt::Debug
    + Send
    + Sync
{
    //fn handle_get<
    //    const SIDL: usize,
    //    const UIDL: usize,
    //    SID:    NumIdDat<SIDL> + 'static,
    //    UID:    NumIdDat<UIDL> + 'static,
    //    ENC:    Encrypter,
    //    KH:     Hasher,
    //    DB:     Database<UIDL, UID, ENC, KH>,
    //>(
    //    &self,
    //    loc:        HttpLocator,
    //    response:   Option<HttpMessage>,
    //    body:       Vec<u8>,
    //    db:         Arc<RwLock<DB>>,
    //    sid_opt:    &Option<SID>,
    //    id:         &String, 
    //)
    //    -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send;
    //
    //fn handle_post<
    //    const SIDL: usize,
    //    const UIDL: usize,
    //    SID:    NumIdDat<SIDL> + 'static,
    //    UID:    NumIdDat<UIDL> + 'static,
    //    ENC:    Encrypter,
    //    KH:     Hasher,
    //    DB:     Database<UIDL, UID, ENC, KH>,
    //>(
    //    &self,
    //    loc:        HttpLocator,
    //    response:   Option<HttpMessage>,
    //    body:       Vec<u8>,
    //    db:         Arc<RwLock<DB>>,
    //    sid_opt:    &Option<SID>,
    //    id:         &String, 
    //)
    //    -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send;
}
