/// Webhook handler infrastructure.
///
/// Steel provides the trait and registry; apps implement their own
/// handlers and register them before starting the server.

use crate::srv::cfg::WebhookRoute;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::HeaderFields,
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
// │ WEBHOOK HANDLER TRAIT                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Trait for handling incoming webhook requests.
///
/// Apps implement this trait for each webhook integration they need
/// (e.g. a payment provider forwarding a purchase confirmation to a
/// fulfilment upstream, or a notification service relaying an event)
/// and register instances with a [`WebhookRegistry`] before starting
/// Steel.
pub trait WebhookHandler: Send + Sync + 'static {
    /// Handle an incoming webhook POST body and return an HTTP response.
    ///
    /// # Arguments
    /// * `route` -- the matched webhook route config (path, handler
    ///   name, resolved config key-value pairs).
    /// * `body` -- the raw request body bytes.
    /// * `req_headers` -- the incoming request header fields, so
    ///   handlers can inspect signature headers (e.g. `Stripe-Signature`)
    ///   or content-type headers without a separate side channel.
    /// * `tls_client` -- shared TLS client config for outbound HTTPS
    ///   calls.
    /// * `id` -- connection identifier for logging.
    fn handle<'a>(
        &'a self,
        route:          &'a WebhookRoute,
        body:           &'a [u8],
        req_headers:    &'a HeaderFields,
        tls_client:     &'a Option<Arc<ClientConfig>>,
        id:             &'a str,
    ) -> Pin<Box<dyn Future<Output = Outcome<Option<HttpMessage>>> + Send + 'a>>;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WEBHOOK REGISTRY                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// A registry mapping handler names (from config) to handler implementations.
///
/// Built by the app before server startup. The stock `steel` binary creates
/// an empty registry; apps that need webhook handling register their handlers.
#[derive(Default)]
pub struct WebhookRegistry {
    handlers: HashMap<String, Box<dyn WebhookHandler>>,
}

impl WebhookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler under the given name.
    ///
    /// The name must match the `handler` field in the webhook route config.
    pub fn register<H: WebhookHandler>(&mut self, name: &str, handler: H) {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    /// Insert a handler already boxed (used by `AppExtension` wiring,
    /// which produces `Box<dyn WebhookHandler>` from its trait method
    /// rather than handing over concrete handler types).
    pub fn insert_boxed(&mut self, name: String, handler: Box<dyn WebhookHandler>) {
        self.handlers.insert(name, handler);
    }

    /// Returns `true` if a handler is registered for the given name.
    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }
}

// Manual Debug impl because Box<dyn WebhookHandler> is not Debug.
impl std::fmt::Debug for WebhookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookRegistry")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .finish()
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DISPATCH                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Dispatch an incoming webhook to the appropriate registered handler.
pub async fn dispatch(
    registry:       &WebhookRegistry,
    route:          &WebhookRoute,
    body:           &[u8],
    req_headers:    &HeaderFields,
    tls_client:     &Option<Arc<ClientConfig>>,
    id:             &str,
)
    -> Outcome<Option<HttpMessage>>
{
    match registry.handlers.get(&route.handler) {
        Some(handler) => handler.handle(
            route, body, req_headers, tls_client, id,
        ).await,
        None => {
            warn!("{}: No registered webhook handler '{}'.", id, route.handler);
            Ok(Some(HttpMessage::respond_with_text(
                HttpStatus::NotFound,
                "Unknown webhook handler.",
            )))
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPER UTILITIES (re-exported for app handlers)                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Percent-encode a string for use in application/x-www-form-urlencoded.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0x0F) as usize]));
            }
        }
    }
    out
}

/// Extract a JSON string value for a given key within a text block.
///
/// Looks for `"key": "value"` and returns the value. Handles `null`
/// as an empty string. No full JSON parser — just enough for extracting
/// fields from webhook payloads.
pub fn extract_value(block: &str, key: &str) -> Option<String> {
    let key_pos = block.find(key)?;
    let after_key = &block[key_pos + key.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if after_colon.starts_with('"') {
        let content = &after_colon[1..];
        let end = content.find('"').unwrap_or(content.len());
        Some(content[..end].to_string())
    } else if after_colon.starts_with("null") {
        Some(String::new())
    } else {
        let end = after_colon.find(|c: char| c == ',' || c == '}' || c == '\n')
            .unwrap_or(after_colon.len());
        Some(after_colon[..end].trim().to_string())
    }
}

/// Extract a JSON string value for `key` within a section starting at
/// `section_key`.
pub fn extract_json_string(json: &str, section_key: &str, key: &str) -> Option<String> {
    let section_start = json.find(section_key)?;
    let block = &json[section_start..json.len().min(section_start + 3000)];
    extract_value(block, key)
}
