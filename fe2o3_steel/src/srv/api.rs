//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

/// API handler infrastructure.
///
/// Mirrors the webhook handler pattern but for general-purpose API
/// endpoints. Steel provides the trait and registry; apps implement
/// their own handlers and register them via an `AppExtension` before
/// starting the server.
///
/// Webhooks are notifications from a third party, so the webhook
/// layer is happy to acknowledge with 200 and return `None` when
/// there is nothing to say back. API requests come from a client
/// that expects a response every time, so `ApiHandler::handle`
/// returns `HttpMessage` unconditionally.

use crate::{
    app::server::VhostDb,
    srv::{
        cfg::ApiRoute,
        id::Uid,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderFields,
        HeaderName,
    },
    header::HttpMethod,
    loc::HttpLocator,
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::HashMap,
    future::Future,
    net::{
        IpAddr,
        SocketAddr,
    },
    pin::Pin,
    sync::{
        Arc,
        RwLock,
    },
};
use tokio_rustls::rustls::ClientConfig;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ API HANDLER TRAIT                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// Apps implement this trait for each custom API endpoint they need -- a
/// checkout builder that validates a cart and proxies to a payment provider, a
/// geolocation lookup -- and register instances via an `AppExtension` before
/// starting Steel.
///
/// The handler receives the full incoming request, so it can inspect method,
/// query string, headers and body, and must always return a response.
pub trait ApiHandler: Send + Sync + 'static {
    fn handle<'a>(
        &'a self,
        route:          &'a ApiRoute,
        method:         HttpMethod,
        loc:            &'a HttpLocator,
        body:           &'a [u8],
        req_headers:    &'a HeaderFields,
        tls_client:     &'a Option<Arc<ClientConfig>>,
        id:             &'a str,
    ) -> Pin<Box<dyn Future<Output = Outcome<HttpMessage>> + Send + 'a>>;

    /// What Steel calls. A handler that needs the peer, the vhost's database or the right to
    /// decline overrides this; the default hands the request to `handle`, so every handler
    /// written before it is served as it was. `None` declines: Steel then serves the path from
    /// `www/` as though no route had matched, so a prefix handler on `/` leaves the static
    /// files to Steel.
    fn handle_ctx<'a>(
        &'a self,
        ctx: HandlerCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Outcome<Option<HttpMessage>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = res!(self.handle(
                ctx.route,
                ctx.method,
                ctx.loc,
                ctx.body,
                ctx.req_headers,
                ctx.tls_client,
                ctx.id,
            ).await);
            Ok(Some(resp))
        })
    }

    /// Called once for each `api_routes` entry that names this handler, when its vhost's database
    /// has been opened after unseal and before it is attached, so no request reaches the handler
    /// with that database until this returns. The place for start-up work over the store, which
    /// Steel's handlers otherwise see only per request: an index rebuilt by one prefix scan, a
    /// first load into an empty store. It runs on a blocking thread. An error is logged and the
    /// database is attached all the same, since other routes of the vhost may use it; a handler
    /// that failed here knows it, and answers accordingly.
    fn db_opened(&self, _route: &ApiRoute, _db: (Arc<RwLock<VhostDb>>, Uid)) -> Outcome<()> {
        Ok(())
    }
}

/// Everything `ApiHandler::handle` is given, and what it is not: the connection's peer, the
/// vhost's database and the method of a `PUT`, `DELETE` or `PATCH`. A `HEAD` arrives as `GET`
/// with `head_only` set in `loc.data`, as it does for `handle`.
pub struct HandlerCtx<'a> {
    pub route:          &'a ApiRoute,
    pub method:         HttpMethod,
    pub loc:            &'a HttpLocator,
    pub body:           &'a [u8],
    pub req_headers:    &'a HeaderFields,
    pub tls_client:     &'a Option<Arc<ClientConfig>>,
    pub id:             &'a str,
    pub peer:           SocketAddr,                         // the TCP peer, never a header
    pub db:             Option<(Arc<RwLock<VhostDb>>, Uid)>, // `None` while Steel is sealed
}

impl<'a> HandlerCtx<'a> {
    /// The client's address. Behind a Steel on this host the TCP peer is loopback, and the
    /// front Steel's `X-Forwarded-For` -- which it rewrites, keeping no client-sent copy -- names
    /// the client as its last entry. From any other peer the header is the client's own claim,
    /// so the peer is the answer.
    pub fn client(&self) -> IpAddr {
        if !self.peer.ip().is_loopback() {
            return self.peer.ip();
        }
        let last = match self.req_headers.get_last(&HeaderName::from("x-forwarded-for")) {
            Some(HeaderFieldValue::Generic(v))  => v.rsplit(',').next().map(|s| s.trim().to_string()),
            Some(other)                         => Some(other.to_string()),
            None                                => None,
        };
        match last.and_then(|s| s.parse::<IpAddr>().ok()) {
            Some(ip)    => ip,
            None        => self.peer.ip(),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ API HANDLER REGISTRY                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Maps handler names, as written in config, to API handler implementations.
/// Built by the app, usually from `AppExtension::api_handlers`, before server
/// startup. Stock Steel starts with an empty registry.
#[derive(Default)]
pub struct ApiHandlerRegistry {
    handlers: HashMap<String, Box<dyn ApiHandler>>,
}

impl ApiHandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// The name must match the `handler` field in the corresponding `api_routes`
    /// entry of `config.jdat`.
    pub fn register<H: ApiHandler>(&mut self, name: &str, handler: H) {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    pub fn insert_boxed(&mut self, name: String, handler: Box<dyn ApiHandler>) {
        self.handlers.insert(name, handler);
    }

    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&dyn ApiHandler> {
        self.handlers.get(name).map(|b| b.as_ref())
    }
}

// Manual Debug impl because Box<dyn ApiHandler> is not Debug.
impl std::fmt::Debug for ApiHandlerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiHandlerRegistry")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .finish()
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DISPATCH                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Called from the HTTPS server when an `ApiRoute` has its `handler` field set,
/// meaning the route is served by an in-process handler rather than proxied to a
/// remote upstream.
pub async fn dispatch(
    registry:       &ApiHandlerRegistry,
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
    let handler_name = match &route.handler {
        Some(n) => n,
        None => {
            warn!("{}: API route '{}' reached dispatch with no handler name.",
                id, route.path);
            return Ok(HttpMessage::respond_with_text(
                HttpStatus::InternalServerError,
                "API route misconfigured: no handler name.",
            ));
        }
    };
    match registry.handlers.get(handler_name) {
        Some(handler) => handler.handle(
            route, method, loc, body, req_headers, tls_client, id,
        ).await,
        None => {
            warn!("{}: No registered API handler '{}'.", id, handler_name);
            Ok(HttpMessage::respond_with_text(
                HttpStatus::NotFound,
                "Unknown API handler.",
            ))
        }
    }
}

/// Steel's own dispatch, through `handle_ctx`. `None` when the handler declines, so the caller
/// falls through to static serving.
pub async fn dispatch_ctx(
    registry:   &ApiHandlerRegistry,
    ctx:        HandlerCtx<'_>,
)
    -> Outcome<Option<HttpMessage>>
{
    let handler_name = match &ctx.route.handler {
        Some(n) => n,
        None => {
            warn!("{}: API route '{}' reached dispatch with no handler name.",
                ctx.id, ctx.route.path);
            return Ok(Some(HttpMessage::respond_with_text(
                HttpStatus::InternalServerError,
                "API route misconfigured: no handler name.",
            )));
        }
    };
    match registry.handlers.get(handler_name) {
        Some(handler) => handler.handle_ctx(ctx).await,
        None => {
            warn!("{}: No registered API handler '{}'.", ctx.id, handler_name);
            Ok(Some(HttpMessage::respond_with_text(
                HttpStatus::NotFound,
                "Unknown API handler.",
            )))
        }
    }
}
