use crate::srv::cfg::ServerConfig;

use oxedyne_fe2o3_core::{
    prelude::*,
    path::{
        //NormalPath,
        NormPathBuf,
    },
    rand::Rand,
};
//use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
//use oxedyne_fe2o3_iop_db::api::Database;
//use oxedyne_fe2o3_iop_hash::api::Hasher;
//use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_net::{
    //file::RequestPath,
    smtp::handler::EmailHandler,
};

//use std::{
//    sync::{
//        Arc,
//        RwLock,
//    },
//};
//
//use tokio::{
//    self,
//    io::AsyncReadExt,
//};


#[derive(Clone, Debug)]
pub struct AppEmailHandler {
    pub cfg:    ServerConfig,
    pub root:   NormPathBuf,
}

impl AppEmailHandler {
    pub fn err_id() -> String {
        Rand::generate_random_string(6, "abcdefghikmnpqrstuvw0123456789")
    }
}

impl EmailHandler for AppEmailHandler {

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
    //    _response:   Option<HttpMessage>,
    //    _body:       Vec<u8>,
    //    _db:         Arc<RwLock<DB>>,
    //    _sid_opt:    &Option<SID>,
    //    id:         &String, 
    //)
    //    -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    //{
    //    let abs_path = if loc.path.as_str() == "/" {
    //        self.cfg.default_www_root_file(&self.root)
    //    } else {
    //        // Rather than raise an error, just remove relative components from path like "./",
    //        // "../../../", etc.
    //        let rel_path = loc.path.as_path().normalise().remove_relative();
    //        self.cfg.public_www_abs_path(&self.root, rel_path)
    //    };

    //    let id = id.clone();

    //    async move {
    //        let result = tokio::task::spawn_blocking(move || {
    //            tokio::runtime::Handle::current().block_on(async {
    //                match tokio::fs::File::open(&abs_path).await {
    //                    Ok(mut file) => {
    //                        let mut contents = Vec::new();
    //                        match file.read_to_end(&mut contents).await {
    //                            Ok(_n) => {
    //                                HttpMessage::new_response(HttpStatus::OK)
    //                                .with_field(
    //                                    HeaderName::ContentType,
    //                                    RequestPath::content_type(abs_path.as_path()),
    //                                ).with_body(contents)
    //                            }
    //                            Err(e) => {
    //                                let err_id = Self::err_id();
    //                                error!(e.into(), "{}: While trying to server file '{:?}' (err_id: {})",
    //                                    id, abs_path, err_id);
    //                                HttpMessage::respond_with_text(
    //                                    HttpStatus::InternalServerError,
    //                                    fmt!("Problem during request processing (err_id: {}).", err_id),
    //                                )
    //                            }
    //                        }
    //                    }
    //                    Err(_e) => {
    //                        debug!("{}: File {:?} not found.", id, abs_path);
    //                        HttpMessage::respond_with_text(
    //                            HttpStatus::NotFound,
    //                            "File not found.",
    //                        ).with_field(
    //                            HeaderName::ContentType,
    //                            RequestPath::content_type(abs_path.as_path()),
    //                        )
    //                    }
    //                }
    //            })
    //        });

    //        match result.await {
    //            Ok(response) => Ok(Some(response)),
    //            Err(e) => Err(err!(e, errmsg!(
    //                "Error while executing async file read.",
    //            ), IO, File, Read)),
    //        }
    //    }        
    //    
    //}
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
    //    _loc:        HttpLocator,
    //    response:   Option<HttpMessage>,
    //    _body:       Vec<u8>,
    //    _db:         Arc<RwLock<DB>>,
    //    _sid_opt:    &Option<SID>,
    //    _id:         &String, 
    //)
    //    -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    //{
    //    async move {
    //        Ok(response)
    //    }
    //}
}
