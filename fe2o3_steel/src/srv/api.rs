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

use crate::srv::cfg::ApiRoute;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    header::HttpMethod,
    loc::HttpLocator,
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
};
use tokio_rustls::rustls::ClientConfig;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ API HANDLER TRAIT                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// Trait for handling incoming API requests.
///
/// Apps implement this trait for each custom API endpoint they need
/// -- for example a checkout builder that validates a cart and
/// proxies to a payment provider, or a geolocation lookup -- and
/// register instances via an `AppExtension` before starting Steel.
///
/// The handler receives the full incoming request so it can inspect
/// method, query string, headers and body, and must always return a
/// response.
pub trait ApiHandler: Send + Sync + 'static {
    /// Handle an incoming API request and return an HTTP response.
    ///
    /// # Arguments
    /// * `route`      -- the matched API route config (path, handler
    ///                   name, resolved handler-config key-value pairs).
    /// * `method`     -- the HTTP method of the incoming request.
    /// * `loc`        -- the parsed request location (path, query
    ///                   string, parsed fields).
    /// * `body`       -- the raw request body bytes.
    /// * `tls_client` -- shared TLS client config for outbound HTTPS
    ///                   calls the handler may need to make.
    /// * `id`         -- connection identifier for logging.
    fn handle<'a>(
        &'a self,
        route:      &'a ApiRoute,
        method:     HttpMethod,
        loc:        &'a HttpLocator,
        body:       &'a [u8],
        tls_client: &'a Option<Arc<ClientConfig>>,
        id:         &'a str,
    ) -> Pin<Box<dyn Future<Output = Outcome<HttpMessage>> + Send + 'a>>;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ API HANDLER REGISTRY                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// A registry mapping handler names (from config) to API handler
/// implementations.
///
/// Built by the app, usually via an `AppExtension::api_handlers`
/// return value, before server startup. Stock Steel starts with an
/// empty registry.
#[derive(Default)]
pub struct ApiHandlerRegistry {
    handlers: HashMap<String, Box<dyn ApiHandler>>,
}

impl ApiHandlerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler under the given name.
    ///
    /// The name must match the `handler` field in the corresponding
    /// `api_routes` entry of `config.jdat`.
    pub fn register<H: ApiHandler>(&mut self, name: &str, handler: H) {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    /// Insert a handler already boxed (used by `AppExtension` wiring).
    pub fn insert_boxed(&mut self, name: String, handler: Box<dyn ApiHandler>) {
        self.handlers.insert(name, handler);
    }

    /// Returns `true` if a handler is registered for the given name.
    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// Look up a handler by name.
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

/// Dispatch an incoming API request to the appropriate registered
/// handler.
///
/// Called from the HTTPS server when an `ApiRoute` has its `handler`
/// field set (meaning the route should be served by an in-process
/// handler rather than proxied to a remote upstream).
pub async fn dispatch(
    registry:   &ApiHandlerRegistry,
    route:      &ApiRoute,
    method:     HttpMethod,
    loc:        &HttpLocator,
    body:       &[u8],
    tls_client: &Option<Arc<ClientConfig>>,
    id:         &str,
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
        Some(handler) => handler.handle(route, method, loc, body, tls_client, id).await,
        None => {
            warn!("{}: No registered API handler '{}'.", id, handler_name);
            Ok(HttpMessage::respond_with_text(
                HttpStatus::NotFound,
                "Unknown API handler.",
            ))
        }
    }
}
