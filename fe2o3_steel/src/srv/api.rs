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

use crate::srv::cfg::ApiRoute;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::HeaderFields,
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
