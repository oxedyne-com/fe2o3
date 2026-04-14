use crate::srv::{
    api::{
        self,
        ApiHandlerRegistry,
    },
    cfg::{
        ApiRoute,
        ServerConfig,
        WebhookRoute,
    },
    dev::refresh::HtmlModifier,
    webhook::{
        self,
        WebhookRegistry,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    file::{
        OsPath,
    },
    map::MapMut,
    path::NormalPath,
    rand::Rand,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_net::{
    file::RequestPath,
    http::{
        client::https_request,
        fields::HeaderName,
        handler::WebHandler,
        header::HttpMethod,
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
use tokio_rustls::rustls::ClientConfig;


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
    /// Outbound API proxy routes for this vhost.
    pub api_routes:             Vec<ApiRoute>,
    /// Incoming webhook routes for this vhost.
    pub webhook_routes:         Vec<WebhookRoute>,
    /// Registered webhook handler implementations.
    pub webhook_registry:       Arc<WebhookRegistry>,
    /// Registered in-process API handler implementations.
    pub api_handler_registry:   Arc<ApiHandlerRegistry>,
    /// TLS client config for outbound HTTPS requests to upstream APIs.
    pub tls_client:             Option<Arc<ClientConfig>>,
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
        api_routes:             Vec<ApiRoute>,
        webhook_routes:         Vec<WebhookRoute>,
        webhook_registry:       Arc<WebhookRegistry>,
        api_handler_registry:   Arc<ApiHandlerRegistry>,
        tls_client:             Option<Arc<ClientConfig>>,
    )
        -> Self
    {
        Self {
            cfg,
            public_dir,
            static_routes,
            default_index_files,
            dev_mode,
            api_routes,
            webhook_routes,
            webhook_registry,
            api_handler_registry,
            tls_client,
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

        // First try configured static routes.
        match self.static_routes.get(route) {
            Some(os_path) => match os_path {
                OsPath::Dir(path) => {
                    // Path is already normalised and absolute.
                    for filename in &self.default_index_files {
                        let candidate = path.clone().join(filename);
                        if candidate.exists() {
                            return Ok(candidate);
                        }
                    }
                    return Err(err!(
                        "{}: No default index files found in directory {:?}. \
                        Tried: {:?}", id, path, self.default_index_files;
                        File, NotFound)); 
                }
                OsPath::File(path) => return Ok(path.clone()),
            }
            None => {
                // Fallback: try to serve directly from public directory.
                let clean_path = if route.starts_with('/') {
                    &route[1..]
                } else {
                    route
                };
                
                let path = Path::new(clean_path).normalise();
                if path.escapes() {
                    return Err(err!(
                        "{}: Request path '{}' would escape the public directory.", 
                        id, route;
                        Invalid, Path, Security));
                }
                
                let full_path = self.public_dir.clone().join(path);
                
                // If it's a directory, try index files.
                if full_path.is_dir() {
                    for filename in &self.default_index_files {
                        let candidate = full_path.join(filename);
                        if candidate.exists() {
                            return Ok(candidate);
                        }
                    }
                }
                
                return Ok(full_path);
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
        let request_path = loc.path.as_string().to_string();
        let api_routes = self.api_routes.clone();
        let api_handler_registry = self.api_handler_registry.clone();
        let tls_client = self.tls_client.clone();

        async move {
            // Check API routes for a handler-mode match before falling
            // through to static file serving. Proxy-mode API routes are
            // POST-only, so we only match handler-mode routes here.
            if let Some(route) = api_routes.iter().find(|r|
                r.path == request_path && r.handler.is_some())
            {
                debug!("{}: GET {} -> api handler '{}'",
                    id, request_path,
                    route.handler.as_deref().unwrap_or("?"));
                let resp = res!(api::dispatch(
                    &api_handler_registry,
                    route,
                    HttpMethod::GET,
                    &loc,
                    &[],
                    &tls_client,
                    &id,
                ).await);
                return Ok(Some(resp));
            }

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
        loc:        HttpLocator,
        _response:  Option<HttpMessage>,
        body:       Vec<u8>,
        _db:        Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:   &Option<SID>,
        id:         &String,
    )
        -> impl std::future::Future<Output = Outcome<Option<HttpMessage>>> + Send
    {
        let request_path = loc.path.as_string().to_string();
        let id = id.to_string();
        let api_routes = self.api_routes.clone();
        let webhook_routes = self.webhook_routes.clone();
        let webhook_registry = self.webhook_registry.clone();
        let api_handler_registry = self.api_handler_registry.clone();
        let tls_client = self.tls_client.clone();

        async move {
            // Check webhook routes first.
            if let Some(wh) = webhook_routes.iter().find(|r| r.path == request_path) {
                debug!("{}: POST {} -> webhook handler '{}'",
                    id, request_path, wh.handler);
                return webhook::dispatch(
                    &webhook_registry, wh, &body, &tls_client, &id,
                ).await;
            }

            // Find a matching API route.
            let route = match api_routes.iter().find(|r| r.path == request_path) {
                Some(r) => r,
                None => {
                    debug!("{}: POST {} -- no matching API route.", id, request_path);
                    return Ok(Some(HttpMessage::respond_with_text(
                        HttpStatus::NotFound,
                        "No API route matches this path.",
                    )));
                }
            };

            // In-process handler path: dispatch to the registered ApiHandler.
            if route.handler.is_some() {
                debug!("{}: POST {} -> api handler '{}'",
                    id, request_path,
                    route.handler.as_deref().unwrap_or("?"));
                let resp = res!(api::dispatch(
                    &api_handler_registry,
                    route,
                    HttpMethod::POST,
                    &loc,
                    &body,
                    &tls_client,
                    &id,
                ).await);
                return Ok(Some(resp));
            }

            // Proxy path: forward to the upstream.
            let tls_cfg = match &tls_client {
                Some(cfg) => cfg.clone(),
                None => {
                    error!(err!(
                        "{}: API route '{}' matched but no TLS client is configured.",
                        id, request_path;
                        Init, Missing));
                    return Ok(Some(HttpMessage::respond_with_text(
                        HttpStatus::InternalServerError,
                        "Server TLS client not configured for outbound requests.",
                    )));
                }
            };

            let upstream_host = match &route.upstream_host {
                Some(h) => h,
                None => {
                    error!(err!(
                        "{}: API route '{}' is in proxy mode but has no upstream_host.",
                        id, request_path;
                        Init, Missing));
                    return Ok(Some(HttpMessage::respond_with_text(
                        HttpStatus::InternalServerError,
                        "API proxy route misconfigured.",
                    )));
                }
            };
            let upstream_port = route.upstream_port.unwrap_or(443);
            let upstream_path = match &route.upstream_path {
                Some(p) => p.as_str(),
                None    => "/",
            };

            // Build upstream headers.
            let mut hdrs: Vec<(&str, &str)> = Vec::new();
            for (name, value) in &route.headers {
                hdrs.push((name.as_str(), value.as_str()));
            }

            // Forward the Content-Type from the original request if present.
            // Different upstream APIs expect different content types
            // (form-urlencoded, JSON, XML, etc.) so we relay whatever
            // the caller sent rather than hard-coding a default.
            let ct_holder;
            if let Some(ct) = loc.data.get(&dat!("content_type")) {
                if let Dat::Str(s) = ct {
                    ct_holder = s.clone();
                    hdrs.push(("Content-Type", &ct_holder));
                }
            }

            debug!("{}: POST {} -> {}:{}{}", id, request_path,
                upstream_host, upstream_port, upstream_path);

            // Forward the request to the upstream.
            let upstream_resp = res!(https_request(
                upstream_host,
                upstream_port,
                HttpMethod::POST,
                upstream_path,
                &hdrs,
                &body,
                tls_cfg,
            ).await);

            Ok(Some(upstream_resp))
        }
    }
}
