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
    console as site_console,
    publish::{
        self,
        PublishConfig,
        Subscription,
        comment as publish_comment,
        page as publish_page,
        send::MailSender,
        store as publish_store,
        subscribe as publish_subscribe,
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
    /// The site's own DKIM mail sender, shared across every vhost and
    /// built once from the server's mail configuration. `None` where
    /// the host has no mail configured, in which case newsletter
    /// signup answers "not set up" rather than recording a pending
    /// subscriber it could never confirm.
    pub mail:                   Option<Arc<MailSender>>,
    /// The members the operator has entrusted with this site, by
    /// username. A signed-in member on this list reaches the site
    /// console at `/manage`; an empty list means the site has no
    /// console and `/manage` means whatever it meant before.
    pub site_admins:            Arc<Vec<String>>,
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
        mail:                   Option<Arc<MailSender>>,
        site_admins:            Arc<Vec<String>>,
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
            mail,
            site_admins,
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
        // The path and the query are parsed apart, so a handler wanting the query must be handed
        // it: `request_path` never carries one, and splitting it on `?` finds nothing.
        let request_query = loc.query.clone();
        let api_routes = self.api_routes.clone();
        let api_handler_registry = self.api_handler_registry.clone();
        let tls_client = self.tls_client.clone();
        let admin_state = self.admin_state.clone();
        let publish = self.publish.clone();
        let site_admins = self.site_admins.clone();

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
                || request_path.starts_with("/admin/database/")
            {
                if let Some(state) = &admin_state {
                    let resp = res!(admin_ozone_view::handle_get(
                        state.as_ref(),
                        db.as_ref(),
                        &request_path,
                        &request_query,
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

            // The site console, at `/manage`, before static files. A site that
            // has content to manage claims the prefix -- so a signed-in member
            // can learn their id and ask to be an admin even before anyone is
            // one, which is the bootstrap. A site that neither publishes nor
            // has admins skips this and `/manage` means what it did before. The
            // gate is inside: a listed member manages, a signed-in member who
            // is not listed is shown their id, an anonymous visitor is sent home.
            if (publish.is_some() || !site_admins.is_empty())
                && site_console::owns(&request_path)
            {
                let resp = res!(site_console::handle_get(
                    site_admins.as_ref(),
                    admin_state.as_deref(),
                    publish.as_deref(),
                    db.as_ref(),
                    &request_path,
                    &request_query,
                    &req_headers,
                    &id,
                ).await);
                return Ok(Some(resp));
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
                    // The newsletter's public endpoints sit under the same prefix and touch the
                    // subscriber store, not the posts, so they are answered before the posts are
                    // read. A confirm or unsubscribe is a GET, since it is followed from an email;
                    // the sign-up form is a GET too, and its POST is handled in `handle_post`.
                    match cfg.subscription_of(&request_path) {
                        Some(Subscription::Subscribe) => {
                            return Ok(Some(publish_subscribe::subscribe_form(cfg.as_ref())));
                        }
                        Some(Subscription::Confirm) => {
                            let resp = res!(publish_subscribe::handle_confirm(
                                cfg.as_ref(), db.as_ref(), &request_query, &id));
                            return Ok(Some(resp));
                        }
                        Some(Subscription::Unsubscribe) => {
                            let resp = res!(publish_subscribe::handle_unsubscribe(
                                cfg.as_ref(), db.as_ref(), &request_query, &id));
                            return Ok(Some(resp));
                        }
                        None => {}
                    }
                    // A member's uploaded picture: bytes in the site's own database, asked for by
                    // the byline that points at it. Answered here, before the posts are read,
                    // because it is not a post and reading every post to serve a picture would be
                    // work for nothing.
                    if let Some(user) = request_path.strip_prefix(&cfg.avatar_prefix()) {
                        let found = match db.as_ref() {
                            Some(dbh) => match publish_store::get_avatar(dbh, user) {
                                Ok(found) => found,
                                Err(e) => {
                                    error!(e, "{}: publish: the picture of '{}' will not read",
                                        id, user);
                                    None
                                }
                            },
                            None => None,
                        };
                        return Ok(Some(match found {
                            Some((kind, bytes)) => {
                                info!("{}: publish: picture of '{}', {} bytes",
                                    id, user, bytes.len());
                                HttpMessage::new_response(HttpStatus::OK)
                                    .with_field(
                                        HeaderName::ContentType,
                                        HeaderFieldValue::Generic(kind),
                                    )
                                    // A picture changes when its owner changes it and not otherwise,
                                    // so it is worth an hour of a reader's cache -- long enough to
                                    // matter over a page of bylines, short enough that a new one
                                    // shows up the same day.
                                    .with_field(
                                        HeaderName::CacheControl,
                                        HeaderFieldValue::Generic(fmt!("public, max-age=3600")),
                                    )
                                    // An SVG is a document, and a document served from this origin
                                    // may carry script that runs as this site. The picture is drawn
                                    // in an `<img>`, where no browser runs it, but a URL can be
                                    // opened directly -- so the response is sandboxed and given
                                    // nothing it may fetch or execute. The same headers cost a PNG
                                    // nothing, so they are not conditional on the type: a rule that
                                    // applies sometimes is a rule that will one day be missed.
                                    .with_field(
                                        HeaderName::ContentSecurityPolicy,
                                        HeaderFieldValue::Generic(
                                            fmt!("default-src 'none'; style-src 'unsafe-inline'; \
                                                sandbox")),
                                    )
                                    // And it is what it says it is: no sniffing a mistyped upload
                                    // into something with a script in it.
                                    .with_field(
                                        HeaderName::XContentTypeOptions,
                                        HeaderFieldValue::Generic(fmt!("nosniff")),
                                    )
                                    .with_body(bytes)
                            }
                            None => HttpMessage::respond_with_text(
                                HttpStatus::NotFound,
                                "no such picture",
                            ),
                        }));
                    }
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
                    // The distinct authors the posts name, resolved to a face -- a display name and
                    // an avatar -- for an author row. Two requests draw one: the index, which renders
                    // its own filter, and the JSON, which hands the same faces to a page that renders
                    // the filter itself. A post view, the feed and the filter script want none of it,
                    // and a read per author on every one of those would be work nobody asked for. A
                    // directory has a database for none of this, and a post from one names no author
                    // regardless.
                    let author_names: Vec<String> =
                        if request_path == cfg.path || request_path == cfg.json_path() {
                            // Everyone the list names, for the filter's author row, and after them
                            // whoever else may write here, for the block above it saying what the
                            // site is about. A blog whose first post is not written yet still has
                            // someone who can say what it will be about, and this is how their
                            // description reaches an empty index.
                            let mut names: Vec<String> = posts.iter()
                                .map(|p| p.author.clone())
                                .filter(|a| !a.is_empty())
                                .collect();
                            names.extend(site_admins.iter().cloned());
                            if let Some(dbh) = db.as_ref() {
                                match publish_store::admins_get(dbh, &id) {
                                    Ok(granted) => names.extend(granted),
                                    Err(e) => warn!(
                                        "{}: publish: the granted admins will not read: {}", id, e),
                                }
                            }
                            names
                        } else {
                            // A post being read wants one face: whoever wrote it, for the byline and
                            // the note beneath it. The feed and the filter script want none.
                            publish_page::served_post(cfg.as_ref(), &posts, &request_path)
                                .map(|p| p.author.clone())
                                .filter(|a| !a.is_empty())
                                .into_iter()
                                .collect()
                        };
                    let authors = if author_names.is_empty() {
                        Vec::new()
                    } else {
                        match db.as_ref() {
                            Some(dbh) => publish_store::resolve_authors(dbh, &author_names),
                            None => Vec::new(),
                        }
                    };
                    // The conversation below the post, where the site has a database to keep one
                    // in. A comments read that fails costs the conversation and never the prose: a
                    // reader came for the post.
                    // The site's own switch, which the console sets; the config is only where it
                    // started. Closing stops new comments and does **not** hide the ones already
                    // published: a conversation that happened still happened, and taking it off the
                    // page would be a deletion nobody asked for.
                    let open = publish_comment::comments_open(db.as_ref(), cfg.comments);
                    let served = publish_page::served_post(cfg.as_ref(), &posts, &request_path);
                    let view = match (db.as_ref(), served) {
                        (Some(dbh), Some(post)) => {
                            match publish_comment::site_secret(dbh) {
                                Ok(secret) => {
                                    // The order a reader asked for, and the page of it they are on.
                                    let newest = publish_page::query_word(&request_query, "order")
                                        .as_deref() == Some("newest");
                                    let ranker = if newest {
                                        publish_comment::Ranker::Recent
                                    } else {
                                        publish_comment::Ranker::Chronological
                                    };
                                    let all = match publish_comment::public_for_post(
                                        dbh, &post.slug, ranker, &id,
                                    ) {
                                        Ok(items) => publish_comment::thread(items),
                                        Err(e) => {
                                            warn!("{}: publish: comments on '{}' will not read: {}",
                                                id, post.slug, e);
                                            Vec::new()
                                        }
                                    };
                                    let count = publish_comment::count_threads(&all);
                                    let want = publish_page::query_word(&request_query, "cpage")
                                        .and_then(|p| p.parse::<usize>().ok())
                                        .unwrap_or(1);
                                    let (threads, at, pages) = publish_comment::page_of(all, want);
                                    Some(publish_page::CommentsView {
                                        threads,
                                        count,
                                        page: at,
                                        pages,
                                        order: if newest { "newest" } else { "oldest" },
                                        path: cfg.path_of(&post.slug),
                                        challenge: publish_comment::pow_challenge(&post.slug, &secret),
                                        said: publish_page::said_of(&request_query),
                                        open,
                                        // Whoever holds the cookie for a comment on this page may
                                        // still correct it, if the window stands.
                                        editable: publish_page::edit_claim(&req_headers)
                                            .filter(|(cid, token)| {
                                                publish_comment::edit_token_ok(cid, &secret, token)
                                            }),
                                    })
                                }
                                Err(e) => {
                                    warn!("{}: publish: no comment secret: {}", id, e);
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    let resp = res!(publish_page::handle_get(
                        cfg.as_ref(),
                        &posts,
                        &authors,
                        &request_path,
                        &request_query,
                        view.as_ref(),
                        &id,
                    ));
                    // The tally, where a post was actually served to somebody who is neither its
                    // author nor a machine. It is kept here because this is the last place holding
                    // the database: the renderers take a slice of posts on purpose.
                    //
                    // A tally that cannot be written costs the tally and never the page. A reader
                    // asked for prose, and a counter failing is not their problem; it is logged and
                    // the response goes out regardless.
                    if let (Some(db), Some(post)) = (
                        db.as_ref(),
                        publish_page::served_post(cfg.as_ref(), &posts, &request_path),
                    ) {
                        let ua = match req_headers.get_one(&HeaderName::UserAgent) {
                            Some(HeaderFieldValue::Generic(s)) => Some(s.as_str()),
                            _ => None,
                        };
                        let seen = publish::counts_as_read(
                            site_console::session::cookie_value(&req_headers).is_some(),
                            ua,
                        );
                        if seen {
                            if let Err(e) = publish_store::reads_bump(db, &post.slug) {
                                warn!("{}: publish: the read of '{}' was not counted: {}",
                                    id, post.slug, e);
                            }
                        }
                    }
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
        let mail = self.mail.clone();
        let site_admins = self.site_admins.clone();

        async move {
            // The newsletter sign-up: a public POST under the published prefix,
            // answered before the console and the API routes. It touches the
            // subscriber store and the DKIM mail sender, not the posts, and
            // always answers the same themed page whether or not the address was
            // already known -- so the form is never an oracle for the list.
            if let Some(cfg) = &publish {
                if let Some(Subscription::Subscribe) = cfg.subscription_of(&request_path) {
                    let resp = res!(publish_subscribe::handle_subscribe(
                        cfg.as_ref(), db.as_ref(), &mail, &body, &id).await);
                    return Ok(Some(resp));
                }
                // A comment on a post: a public POST under the post's own path, so which post is
                // being commented on is carried by the URL and cannot be swapped in the body. The
                // reader is answered with a redirect back to the post, carrying what to tell them --
                // so a reload does not post the comment twice.
                // An edit: only from whoever holds the token this comment was answered with, and
                // only while the window stands.
                if let Some(slug) = cfg.comment_edit_slug(&request_path)
                    .filter(|_| publish_comment::comments_open(db.as_ref(), cfg.comments))
                {
                    if let Some(dbh) = db.as_ref() {
                        let secret = res!(publish_comment::site_secret(dbh));
                        let cid = site_console::form_field(&body, "id").unwrap_or_default();
                        let token = site_console::form_field(&body, "token").unwrap_or_default();
                        let source = site_console::form_field(&body, "body").unwrap_or_default();
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let ok = match res!(publish_comment::get(dbh, slug, &cid)) {
                            Some(c) => {
                                publish_comment::edit_token_ok(&cid, &secret, &token)
                                    && publish_comment::editable(&c, now)
                            }
                            None => false,
                        };
                        // One answer whether the token was wrong, the window had closed or the
                        // comment was never there: none of those is anybody's business to learn by
                        // asking.
                        let said = if ok
                            && res!(publish_comment::edit(dbh, slug, &cid, &source))
                        {
                            "edited"
                        } else {
                            "noedit"
                        };
                        return Ok(Some(publish_page::comment_posted(cfg.as_ref(), slug, said)));
                    }
                }
                // A preview: renders and stores nothing. Answered before the comment route, since
                // its path is the comment path with a suffix.
                if let Some(_slug) = cfg.comment_preview_slug(&request_path)
                    .filter(|_| publish_comment::comments_open(db.as_ref(), cfg.comments))
                {
                    if let Some(dbh) = db.as_ref() {
                        let secret = res!(publish_comment::site_secret(dbh));
                        let source = site_console::form_field(&body, "body").unwrap_or_default();
                        let html = res!(publish_comment::preview(
                            dbh,
                            &source,
                            Some(&peer.ip().to_string()),
                            &secret,
                            cfg.comment_rate_secs,
                        ));
                        return Ok(Some(publish_page::comment_preview(html)));
                    }
                }
                if let Some(slug) = cfg.comment_slug(&request_path)
                    .filter(|_| publish_comment::comments_open(db.as_ref(), cfg.comments))
                {
                    // The slug must name a post a reader can actually see. Without this the endpoint
                    // writes a record under any name at all -- an unauthenticated write to storage
                    // keyed on a string the sender chose, which is a way to fill a disk rather than a
                    // way to comment. Measured against a live site before it was fixed: a POST to
                    // /readme/anything/comment answered 303 and stored a comment.
                    let posts = match publish::read(cfg.as_ref(), db.as_ref(), &id) {
                        Ok(p) => p,
                        Err(e) => {
                            error!(e, "{}: publish: cannot read the posts to place a comment", id);
                            Vec::new()
                        }
                    };
                    if !posts.iter().any(|p| p.slug == slug) {
                        info!("{}: publish: a comment named no post of ours ('{}')", id, slug);
                        return Ok(Some(HttpMessage::respond_with_text(
                            HttpStatus::NotFound, "No such post.")));
                    }
                    let mut edit_cookie: Option<(String, String)> = None;
                    let said = match db.as_ref() {
                        Some(dbh) => {
                            let secret = res!(publish_comment::site_secret(dbh));
                            let f = |k: &str| site_console::form_field(&body, k).unwrap_or_default();
                            let sub = publish_comment::Submission {
                                slug,
                                parent:     site_console::form_field(&body, "parent"),
                                name:       f("name"),
                                email:      site_console::form_field(&body, "email"),
                                body:       f("body"),
                                honeypot:   f("website"),
                                challenge:  f("challenge"),
                                nonce:      f("nonce"),
                                from:       Some(peer.ip().to_string()),
                                now:        publish_comment::now_stamp(),
                            };
                            let got = res!(publish_comment::receive(
                                dbh,
                                &publish_comment::Moderator::default(),
                                (cfg.comment_rate_secs, cfg.comment_rate_hourly),
                                sub,
                                &secret,
                                &secret,
                                &id,
                            ));
                            // The token that lets its author correct it, handed back once, in a
                            // cookie that expires with the window. It names one comment and proves
                            // nothing else, so it is safe to hold in a browser.
                            if let Some(cid) = &got.1 {
                                edit_cookie = Some((
                                    cid.clone(),
                                    publish_comment::edit_token(cid, &secret),
                                ));
                            }
                            got.0.tell_reader().to_string()
                        }
                        None => "shut".to_string(),
                    };
                    let mut resp = publish_page::comment_posted(cfg.as_ref(), slug, &said);
                    if let Some((cid, token)) = edit_cookie {
                        resp = publish_page::with_edit_cookie(resp, &cid, &token);
                    }
                    return Ok(Some(resp));
                }
            }

            // The site console's writes, gated on a site admin's session and
            // guarded against cross-site forgery. It answers only the paths it
            // writes to and hands the rest back. Dispatched on the same terms as
            // its pages; the gate inside denies a site with no admins, since a
            // write needs a listed member and an empty list names none.
            if (publish.is_some() || !site_admins.is_empty())
                && site_console::owns(&request_path)
            {
                if let Some(resp) = res!(site_console::handle_post(
                    site_admins.as_ref(),
                    admin_state.as_deref(),
                    publish.as_deref(),
                    db.as_ref(),
                    &tls_client,
                    &mail,
                    &request_path,
                    &req_headers,
                    &body,
                    peer,
                    &id,
                ).await) {
                    return Ok(Some(resp));
                }
            }

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
