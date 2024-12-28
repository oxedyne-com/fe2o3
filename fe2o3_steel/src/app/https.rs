use crate::srv::{
    cfg::ServerConfig,
    dev::refresh::HtmlModifier,
};

use oxedize_fe2o3_core::{
    prelude::*,
    file::{
        OsPath,
        PathState,
    },
    map::MapMut,
    path::NormalPath,
    rand::Rand,
};
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_net::{
    file::RequestPath,
    http::{
        fields::HeaderName,
        handler::WebHandler,
        loc::HttpLocator,
        msg::HttpMessage,
        status::HttpStatus,
    },
};

use std::{
    fmt::Debug,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        RwLock,
    },
};

use tokio::{
    self,
    io::AsyncReadExt,
};


#[derive(Clone, Debug)]
pub struct AppWebHandler<
    M: MapMut<String, OsPath> + Clone + Debug + Send + Sync,
>{
    // Config
    pub cfg:                    ServerConfig,
    // State
    pub public_dir:             PathBuf,
    pub static_routes:          M,
    pub default_index_files:    Vec<String>,
    pub dev_mode:               bool,
}

impl<
    M: MapMut<String, OsPath> + Clone + Debug + Send + Sync,
>
    AppWebHandler<M>
{
    pub fn new(
        cfg:                    ServerConfig,
        public_dir:             PathBuf,
        static_routes:          M,
        default_index_files:    Vec<String>,
        dev_mode:               bool,
    )
        -> Self
    {
        Self {
            cfg,
            public_dir,
            static_routes,
            default_index_files,
            dev_mode,
        }
    }

    pub fn err_id() -> String {
        Rand::generate_random_string(6, "abcdefghikmnpqrstuvw0123456789")
    }

    async fn router(
        &self,
        loc:    &HttpLocator,
        id:     &String, 
    )
        -> Outcome<PathBuf>
    {
        let route = loc.path.as_string();
        match self.static_routes.get(route) {
            Some(os_path) => match os_path {
                OsPath::Dir(path) => {
                    for filename in &self.default_index_files {
                        // path is already normalised and absolute.
                        let path = path.clone().join(filename);
                        match PathState::FileMustExist.validate(
                            &path,
                            "",
                        ) {
                            Ok(()) => return Ok(path),
                            Err(_) => continue,
                        }
                    }
                    return Err(err!(
                        "{}: Default files not found in directory {:?}.", id, path;
                        File, NotFound)); 
                }
                // The path has already been normalised and made absolute.
                OsPath::File(path) => return Ok(path.clone()),
            }
            None => {
                // TODO consider dynamic routes.
                let path = Path::new(route).normalise();
                if path.escapes() {
                    return Err(err!(
                        "ServerConfig: route path {} escapes the public directory {:?}.",
                        route, self.public_dir;
                        Invalid, Path));
                }
                return Ok(self.public_dir.clone().join(path));
            }
        }
    }
}

impl<
    M: MapMut<String, OsPath> + Clone + Debug + Send + Sync,
>
    WebHandler for AppWebHandler<M>
{

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
        loc:        HttpLocator,
        _response:   Option<HttpMessage>,
        _body:       Vec<u8>,
        _db:         Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:    &Option<SID>,
        id:         &String, 
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    {
        let dev_mode = self.dev_mode;
        let rpath = loc.path.clone();
        let id = id.to_string();

        async move {
            let abs_path = match self.router(&loc, &id).await {
                Ok(path) => path, // The path may not exist, but at least we have one.
                Err(e) => {
                    // Tap out early if the route is definitely not known.
                    error!(e);
                    return Ok(Some(
                        HttpMessage::respond_with_text(
                            HttpStatus::NotFound,
                            "File not found.",
                        ).with_field(
                            HeaderName::ContentType,
                            RequestPath::content_type(rpath.as_path()),
                        )
                    ));
                }
            };
            
            let id_clone = id.clone();
            let result = tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(async {
                    Ok(match tokio::fs::File::open(&abs_path).await {
                        Ok(mut file) => {
                            let mut contents = Vec::new();
                            match file.read_to_end(&mut contents).await {
                                Ok(_n) => {
                                    // Check for HTML content and dev mode.
                                    let content_type = RequestPath::content_type(abs_path.as_path());
                                    let content_type_str = content_type.to_string();
                                    let response = HttpMessage::new_response(HttpStatus::OK)
                                        .with_field(HeaderName::ContentType, content_type);

                                    if dev_mode && content_type_str.contains("text/html") {
                                        let contents_str = res!(String::from_utf8(contents.clone()));
                                        let modified =
                                            res!(HtmlModifier::inject_dev_refresh(&contents_str));
                                        response.with_body(modified.into_bytes())
                                    } else {
                                        response.with_body(contents)
                                    }
                                }
                                Err(e) => {
                                    let err_id = Self::err_id();
                                    error!(e.into(),
                                        "{}: While trying to server file '{:?}' (err_id: {})",
                                        id_clone, abs_path, err_id,
                                    );
                                    HttpMessage::respond_with_text(
                                        HttpStatus::InternalServerError,
                                        fmt!("Problem during request processing (err_id: {}).",
                                            err_id),
                                    )
                                }
                            }
                        }
                        Err(_e) => {
                            debug!("{}: File {:?} not found.", id_clone, abs_path);
                            HttpMessage::respond_with_text(
                                HttpStatus::NotFound,
                                "File not found.",
                            ).with_field(
                                HeaderName::ContentType,
                                RequestPath::content_type(abs_path.as_path()),
                            )
                        }
                    })
                })
            });

            match result.await {
                Ok(response) => match response {
                    Ok(http_msg) => Ok(Some(http_msg)),
                    Err(e) => Err(e),
                },
                Err(e) => Err(err!(e,
                    "{}: Error while executing async file read.", id;
                    IO, File, Read)),
            }
        }        
        
    }
    
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
        _loc:        HttpLocator,
        response:   Option<HttpMessage>,
        _body:       Vec<u8>,
        _db:         Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:    &Option<SID>,
        _id:         &String, 
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    {
        async move {
            Ok(response)
        }
    }
}
