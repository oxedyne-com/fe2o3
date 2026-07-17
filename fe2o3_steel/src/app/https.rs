use crate::srv::{
    admin::{
        handler as admin_handler,
        ozone_view as admin_ozone_view,
        state::AdminState,
        traffic::TrafficRecorder,
    },
    api::{
        self,
        ApiHandlerRegistry,
    },
    cache,
    cfg::{
        ApiRoute,
        ServerConfig,
        WebhookRoute,
    },
    dev::refresh::HtmlModifier,
    publish::{
        self,
        PublishConfig,
        page as publish_page,
        write as publish_write,
    },
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
        client::{
            http_request,
            https_request,
        },
        fields::{
            HeaderFields,
            HeaderFieldValue,
            HeaderName,
        },
        handler::WebHandler,
        header::HttpMethod,
        loc::HttpLocator,
        msg::HttpMessage,
        status::HttpStatus,
    },
};

use std::{
    fmt::Debug,
    net::SocketAddr,
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
    /// Shared admin dashboard runtime. Built once at server startup
    /// from the unlocked wallet, then handed to every vhost so the
    /// dashboard `/admin/*` routes can authenticate and authorise
    /// requests against the same admin list and session key. `None`
    /// disables the dashboard entirely (the route returns 404).
    pub admin_state:            Option<Arc<AdminState>>,
    /// Shared traffic recorder. Every request that reaches a vhost
    /// is logged here for the dashboard's traffic view. `None`
    /// disables traffic recording without affecting request
    /// handling.
    pub traffic:                Option<Arc<TrafficRecorder>>,
    /// The prose this vhost publishes. `None` publishes nothing, and
    /// the paths the block would have claimed fall through to the
    /// static file router like any others.
    pub publish:                Option<Arc<PublishConfig>>,
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
        admin_state:            Option<Arc<AdminState>>,
        traffic:                Option<Arc<TrafficRecorder>>,
        publish:                Option<Arc<PublishConfig>>,
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
            admin_state,
            traffic,
            publish,
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
        loc:            HttpLocator,
        _response:      Option<HttpMessage>,
        _body:          Vec<u8>,
        req_headers:    Arc<HeaderFields>,
        db:             Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:       &Option<SID>,
        peer:           SocketAddr,
        id:             &String,
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
        let admin_state = self.admin_state.clone();
        let publish = self.publish.clone();

        async move {
            // The dashboard owns the entire `/admin` and `/admin/*`
            // subtree on every vhost it is configured for. Dispatch
            // before any API/webhook/static route lookups so app
            // routes cannot accidentally shadow it.
            //
            // Two-stage dispatch: ozone-prefixed routes go to the
            // generic ozone_view module (which needs the per-vhost
            // db typed parameters in scope), all other dashboard
            // routes go to the non-generic handler module.
            if request_path == "/admin/database"
                || request_path.starts_with("/admin/database?")
                || request_path.starts_with("/admin/database/")
            {
                if let Some(state) = &admin_state {
                    let resp = res!(admin_ozone_view::handle_get(
                        state.as_ref(),
                        db.as_ref(),
                        &request_path,
                        &req_headers,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
                return Ok(Some(HttpMessage::respond_with_text(
                    HttpStatus::NotFound,
                    "Not found.",
                )));
            }
            if request_path == "/admin"
                || request_path.starts_with("/admin/")
            {
                if let Some(state) = &admin_state {
                    let resp = res!(admin_handler::handle_get(
                        state.as_ref(),
                        &request_path,
                        &req_headers,
                        peer,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
                // Dashboard not configured. Pretend the route does
                // not exist so we do not leak the existence of an
                // admin endpoint.
                return Ok(Some(HttpMessage::respond_with_text(
                    HttpStatus::NotFound,
                    "Not found.",
                )));
            }

            // The published prose owns its prefix and everything under
            // it, so a post named like a file on disk is still the
            // post. Dispatched before API and static routes for the
            // same reason the dashboard is: a prefix a vhost has
            // claimed should not be shadowed by what happens to sit in
            // its webroot. A vhost publishing nothing skips this
            // entirely and the paths mean whatever they meant before.
            if let Some(cfg) = &publish {
                if cfg.owns(&request_path) {
                    // The read is the only part that touches the database, so the generics stop here
                    // and the renderers take a slice of posts.
                    let posts = match publish::read(cfg.as_ref(), db.as_ref(), &id) {
                        Ok(posts) => posts,
                        Err(e) => {
                            // The site cannot read its own prose. That is the site's fault, and a
                            // reader should be told rather than shown an empty shelf that looks
                            // like the truth.
                            error!(e, "{}: publish: cannot read the posts", id);
                            return Ok(Some(HttpMessage::respond_with_text(
                                HttpStatus::InternalServerError,
                                "the posts cannot be read",
                            )));
                        }
                    };
                    let resp = res!(publish_page::handle_get(
                        cfg.as_ref(),
                        &posts,
                        &request_path,
                        &id,
                    ));
                    return Ok(Some(resp));
                }
            }

            // Check API routes before falling through to static file
            // serving. Handler-mode routes dispatch in-process;
            // proxy-mode routes forward to the upstream over TLS or
            // plain HTTP depending on the scheme flag. A proxy-mode
            // GET is useful for loopback app binaries that respond to
            // plain GETs (e.g. dynamic JSON endpoints), and for
            // third-party APIs that the app wants to mirror through
            // Steel so browser calls pick up the operator-supplied
            // headers (auth tokens, rate-limit keys).
            if let Some(route) = api_routes.iter().find(|r| r.path == request_path) {
                if route.handler.is_some() {
                    debug!("{}: GET {} -> api handler '{}'",
                        id, request_path,
                        route.handler.as_deref().unwrap_or("?"));
                    let resp = res!(api::dispatch(
                        &api_handler_registry,
                        route,
                        HttpMethod::GET,
                        &loc,
                        &[],
                        &req_headers,
                        &tls_client,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
                if route.upstream_host.is_some() {
                    let resp = res!(forward_api_proxy(
                        route,
                        HttpMethod::GET,
                        &loc,
                        &[],
                        &req_headers,
                        &tls_client,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
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
            let req_headers_clone = req_headers.clone();
            let static_max_age_secs = self.cfg.static_max_age_secs;
            let result = tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(async {
                    Ok(match tokio::fs::File::open(&abs_path).await {
                        Ok(mut file) => {
                            let content_type = RequestPath::content_type(abs_path.as_path());
                            let content_type_str = content_type.to_string();

                            // In development an entry document is rewritten on the
                            // way out to carry the refresh hook, so the file on disk
                            // is not the entity being sent, and no tag drawn from it
                            // would describe the response. Such a document is never
                            // cached, and never revalidated -- it is simply resent.
                            let cacheable = !(dev_mode && cache::is_document(&content_type_str));

                            // Ask the filesystem before reading the file. A client
                            // that already holds this entity needs no body, and
                            // reading one would be the very cost the tag exists to
                            // avoid.
                            let validators = if cacheable {
                                let meta = res!(file.metadata().await);
                                let etag = res!(cache::entity_tag(&meta));
                                let directive = cache::cache_control(
                                    &content_type_str,
                                    static_max_age_secs,
                                );
                                Some((etag, directive))
                            } else {
                                None
                            };

                            if let Some((etag, directive)) = &validators {
                                if cache::is_current(&req_headers_clone, etag) {
                                    debug!("{}: {:?} is unchanged; 304.", id_clone, abs_path);
                                    return Ok(res!(cache::not_modified(
                                        etag.clone(),
                                        directive.clone(),
                                    )));
                                }
                            }

                            let mut contents = Vec::new();
                            match file.read_to_end(&mut contents).await {
                                Ok(_n) => {
                                    let mut response = HttpMessage::new_response(HttpStatus::OK)
                                        .with_field(HeaderName::ContentType, content_type);

                                    if let Some((etag, directive)) = validators {
                                        response = response
                                            .with_field(HeaderName::ETag, res!(
                                                HeaderFieldValue::new(&HeaderName::ETag, &etag)))
                                            .with_field(HeaderName::CacheControl, res!(
                                                HeaderFieldValue::new(
                                                    &HeaderName::CacheControl, &directive)));
                                    }

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
        loc:            HttpLocator,
        _response:      Option<HttpMessage>,
        body:           Vec<u8>,
        req_headers:    Arc<HeaderFields>,
        db:             Option<(Arc<RwLock<DB>>, UID)>,
        _sid_opt:       &Option<SID>,
        peer:           SocketAddr,
        id:             &String,
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
        let admin_state = self.admin_state.clone();
        let publish = self.publish.clone();

        async move {
            // Dashboard `/admin/*` POST handlers (login form POST,
            // logout, future mutations). Same precedence rule as
            // GET: dashboard owns the subtree and dispatches before
            // any other lookup.
            if request_path == "/admin"
                || request_path.starts_with("/admin/")
            {
                if let Some(state) = &admin_state {
                    let resp = res!(admin_handler::handle_post(
                        state.as_ref(),
                        &request_path,
                        &body,
                        &req_headers,
                        peer,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
                return Ok(Some(HttpMessage::respond_with_text(
                    HttpStatus::NotFound,
                    "Not found.",
                )));
            }

            // The published prose owns its prefix for writes as it does
            // for reads. It answers only the paths it knows and hands
            // the rest back, so an app POSTing elsewhere under the
            // prefix is not swallowed here.
            if let Some(cfg) = &publish {
                if cfg.owns(&request_path) {
                    if let Some(resp) = res!(publish_write::handle_post(
                        cfg.as_ref(),
                        admin_state.as_ref(),
                        db.as_ref(),
                        &request_path,
                        &req_headers,
                        &id,
                    ).await) {
                        return Ok(Some(resp));
                    }
                }
            }

            // Check webhook routes first. Two dispatch branches
            // depending on the mode the route was configured in:
            //
            // - in-process `handler` -- the route names a registered
            //   `WebhookHandler` in the webhook registry; dispatch
            //   runs inside the Steel process as before.
            // - forwarded `upstream` -- the route carries an upstream
            //   URL; the raw body and most incoming headers are
            //   forwarded verbatim to the upstream so downstream
            //   signature verification (e.g. `Stripe-Signature`)
            //   still sees an unmodified payload.
            if let Some(wh) = webhook_routes.iter().find(|r| r.path == request_path) {
                if wh.is_upstream() {
                    debug!("{}: POST {} -> webhook upstream {:?}:{:?}",
                        id, request_path, wh.upstream_host, wh.upstream_port);
                    let resp = res!(forward_webhook(
                        wh,
                        &body,
                        &req_headers,
                        &tls_client,
                        &id,
                    ).await);
                    return Ok(Some(resp));
                }
                debug!("{}: POST {} -> webhook handler '{}'",
                    id, request_path,
                    wh.handler.as_deref().unwrap_or("?"));
                return webhook::dispatch(
                    &webhook_registry, wh, &body, &req_headers, &tls_client, &id,
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
                    &req_headers,
                    &tls_client,
                    &id,
                ).await);
                return Ok(Some(resp));
            }

            // Proxy path: forward to the upstream via the shared helper.
            let resp = res!(forward_api_proxy(
                route,
                HttpMethod::POST,
                &loc,
                &body,
                &req_headers,
                &tls_client,
                &id,
            ).await);
            Ok(Some(resp))
        }
    }
}

/// Forward an API request to the configured upstream. Shared by the
/// GET and POST proxy branches.
///
/// Honours the route's `upstream_tls` flag to pick between
/// `https_request` and `http_request`, carries the static headers
/// the route was configured with, and propagates a curated set of
/// incoming request headers so the upstream sees the same client
/// context the in-process handler would have seen. Hop-by-hop
/// headers (`Host`, `Connection`, `Content-Length`) are filtered
/// out because the outbound client regenerates them itself.
async fn forward_api_proxy(
    route:          &ApiRoute,
    method:         HttpMethod,
    loc:            &HttpLocator,
    body:           &[u8],
    req_headers:    &HeaderFields,
    tls_client:     &Option<Arc<ClientConfig>>,
    id:             &str,
)
    -> Outcome<HttpMessage>
{
    let upstream_host = match &route.upstream_host {
        Some(h) => h.as_str(),
        None => return Err(err!(
            "{}: API route '{}' is in proxy mode but has no upstream_host.",
            id, route.path;
            Init, Missing, Bug)),
    };
    let upstream_port = route.upstream_port.unwrap_or(
        if route.upstream_tls { 443 } else { 80 });
    let upstream_path = route.upstream_path
        .as_deref().unwrap_or("/");

    // Owned strings for headers built here so their `&str` refs live
    // until the outbound client has finished formatting the request.
    let mut owned: Vec<(String, String)> = Vec::new();

    // Route-configured headers (secret tokens, fixed auth). These
    // win against any incoming client header with the same name
    // below -- the operator's declared headers are authoritative.
    for (name, value) in &route.headers {
        owned.push((name.clone(), value.clone()));
    }

    // Propagate client headers that in-process handlers used to
    // have direct access to. The name list covers everything an
    // elearnity handler currently inspects plus the small set of
    // conventional "pass-through" headers browsers and CLIs send.
    // Hop-by-hop headers (`Host`, `Connection`, `Content-Length`)
    // are skipped because the outbound client regenerates them.
    let propagate: &[HeaderName] = &[
        HeaderName::Accept,
        HeaderName::AcceptLanguage,
        HeaderName::AcceptEncoding,
        HeaderName::ContentType,
        HeaderName::UserAgent,
        HeaderName::Authorization,
        HeaderName::Origin,
        HeaderName::Referer,
    ];
    for name in propagate {
        // Skip duplicates: a route-configured header with the same
        // name already sits in `owned`, so the operator's choice
        // wins over the client's.
        let name_str = fmt!("{}", name);
        if owned.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name_str)) {
            continue;
        }
        if let Some(HeaderFieldValue::Generic(v)) = req_headers.get_one(name) {
            owned.push((name_str, v.clone()));
        }
    }

    // Also let the dispatcher-resolved Content-Type override ride
    // through, because the POST branch already stamped it into
    // `loc.data` before reaching us and we want to respect the
    // original byte-level content-type.
    if let Some(ct) = loc.data.get(&dat!("content_type")) {
        if let Dat::Str(s) = ct {
            if !owned.iter().any(|(n, _)| n.eq_ignore_ascii_case("Content-Type")) {
                owned.push(("Content-Type".to_string(), s.clone()));
            }
        }
    }

    let hdrs: Vec<(&str, &str)> = owned.iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();

    debug!("{}: {} {} -> {}{}:{}{} ({} headers)",
        id, method, route.path,
        if route.upstream_tls { "https://" } else { "http://" },
        upstream_host, upstream_port, upstream_path, hdrs.len());

    if route.upstream_tls {
        let tls_cfg = match tls_client {
            Some(cfg) => cfg.clone(),
            None => return Err(err!(
                "{}: API route '{}' configured with https:// upstream but \
                no TLS client is available.", id, route.path;
                Init, Missing)),
        };
        https_request(
            upstream_host,
            upstream_port,
            method,
            upstream_path,
            &hdrs,
            body,
            tls_cfg,
        ).await
    } else {
        http_request(
            upstream_host,
            upstream_port,
            method,
            upstream_path,
            &hdrs,
            body,
        ).await
    }
}

/// Forward an incoming webhook POST to the upstream configured on a
/// `WebhookRoute`. Preserves the raw body bytes and propagates the
/// essential headers so signature verification on the upstream side
/// sees an unmodified payload.
///
/// Headers propagated: every header whose name is known to matter for
/// webhook providers (Stripe, GitHub, generic X-Signature, etc.).
/// Not every header: hop-by-hop headers (`Host`, `Connection`,
/// `Content-Length`) are regenerated by the outbound client, and
/// propagating them would double-up.
async fn forward_webhook(
    route:          &WebhookRoute,
    body:           &[u8],
    req_headers:    &HeaderFields,
    tls_client:     &Option<Arc<ClientConfig>>,
    id:             &str,
)
    -> Outcome<HttpMessage>
{
    let upstream_host = match &route.upstream_host {
        Some(h) => h.as_str(),
        None => return Err(err!(
            "{}: forward_webhook called on a route with no upstream_host.", id;
            Bug, Missing)),
    };
    let upstream_port = route.upstream_port.unwrap_or(
        if route.upstream_tls { 443 } else { 80 });
    let upstream_path = route.upstream_path
        .as_deref().unwrap_or("/");

    // Propagate the headers a typical webhook provider expects to see
    // verbatim. The list is deliberately short: Content-Type for the
    // JSON/form encoding, any Stripe-Signature / X-Hub-Signature style
    // header for signature verification, and a selection of common
    // auxiliaries (User-Agent, Request-Id, Idempotency-Key). Callers
    // that need a broader set can extend this list without touching
    // the dispatch shape.
    let mut hdrs: Vec<(String, String)> = Vec::new();
    let propagate_names: &[HeaderName] = &[
        HeaderName::ContentType,
        HeaderName::UserAgent,
    ];
    for name in propagate_names {
        if let Some(HeaderFieldValue::Generic(v)) =
            req_headers.get_one(name)
        {
            hdrs.push((fmt!("{}", name), v.clone()));
        }
    }
    // Propagate any non-standard header whose name begins with a
    // canonical signature prefix. This catches Stripe-Signature,
    // X-Hub-Signature, X-Signature, X-Hmac-Signature, and similar
    // without hardcoding the provider.
    for (name, values) in req_headers.iter() {
        if let HeaderName::NonStandard(n) = name {
            let lower = n.to_lowercase();
            let is_signature_like = lower.contains("signature")
                || lower.contains("idempotency")
                || lower == "x-request-id";
            if !is_signature_like {
                continue;
            }
            if let Some(HeaderFieldValue::Generic(v)) = values.first() {
                hdrs.push((n.clone(), v.clone()));
            }
        }
    }
    let hdr_refs: Vec<(&str, &str)> = hdrs.iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();

    debug!("{}: forwarding webhook to {}{}:{}{} (body {} bytes, {} headers)",
        id,
        if route.upstream_tls { "https://" } else { "http://" },
        upstream_host, upstream_port, upstream_path,
        body.len(), hdr_refs.len());

    if route.upstream_tls {
        let tls_cfg = match tls_client {
            Some(cfg) => cfg.clone(),
            None => return Err(err!(
                "{}: webhook route '{}' configured with https:// upstream \
                but no TLS client is available.", id, route.path;
                Init, Missing)),
        };
        https_request(
            upstream_host,
            upstream_port,
            HttpMethod::POST,
            upstream_path,
            &hdr_refs,
            body,
            tls_cfg,
        ).await
    } else {
        http_request(
            upstream_host,
            upstream_port,
            HttpMethod::POST,
            upstream_path,
            &hdr_refs,
            body,
        ).await
    }
}
