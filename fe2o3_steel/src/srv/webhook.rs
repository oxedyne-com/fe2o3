/// Webhook handler infrastructure.
///
/// Steel provides the trait and registry; apps implement their own
/// handlers and register them before starting the server.

use crate::srv::cfg::WebhookRoute;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::{
    hmac::verify_hmac_sha256,
    http::{
        fields::HeaderFields,
        msg::HttpMessage,
        status::HttpStatus,
    },
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
/// Only called for handler-mode webhook routes; upstream-mode routes
/// are forwarded directly from the HTTPS dispatcher.
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
    let name = match &route.handler {
        Some(n) => n,
        None => {
            warn!("{}: webhook::dispatch called on an upstream-mode route.", id);
            return Ok(Some(HttpMessage::respond_with_text(
                HttpStatus::InternalServerError,
                "Webhook route misconfigured.",
            )));
        },
    };
    match registry.handlers.get(name) {
        Some(handler) => handler.handle(
            route, body, req_headers, tls_client, id,
        ).await,
        None => {
            warn!("{}: No registered webhook handler '{}'.", id, name);
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STRIPE WEBHOOK SIGNATURE VERIFICATION                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Default clock-skew tolerance, in seconds, for a Stripe webhook
/// timestamp. Stripe's own libraries default to five minutes.
pub const STRIPE_SIG_TOLERANCE_SECS: u64 = 300;

/// Verify a Stripe webhook signature.
///
/// Stripe signs each webhook with the endpoint's signing secret
/// (`whsec_...`) and sends the result in a `Stripe-Signature` header of
/// the form `t=<unix_ts>,v1=<hex_hmac>[,v1=<hex_hmac>...]` (there may be
/// several `v1` schemes during a secret rotation, and other schemes such
/// as `v0` which we ignore). The signed payload is the ASCII string
/// `"<t>.<raw request body>"`, and the tag is `HMAC-SHA256` of that
/// payload under the signing secret.
///
/// Verification succeeds when the recomputed HMAC matches any supplied
/// `v1` value (constant-time) **and** the timestamp is within
/// `tolerance_secs` of `now_secs`. The current time is passed in rather
/// than read from a clock so the check is deterministic and testable;
/// callers supply the wall-clock Unix seconds.
///
/// # Arguments
/// * `secret` -- the endpoint signing secret (the whole `whsec_...`
///   string as configured; Stripe HMACs against these raw bytes).
/// * `body` -- the exact raw request body bytes, unmodified.
/// * `sig_header` -- the value of the incoming `Stripe-Signature` header.
/// * `now_secs` -- the current time as Unix seconds.
/// * `tolerance_secs` -- the maximum allowed absolute difference between
///   `now_secs` and the header timestamp (see [`STRIPE_SIG_TOLERANCE_SECS`]).
///
/// # Returns
/// `Ok(())` when the signature is valid and fresh; otherwise a tagged
/// error describing the failure (never the signing secret).
pub fn verify_stripe_signature(
    secret:         &str,
    body:           &[u8],
    sig_header:     &str,
    now_secs:       u64,
    tolerance_secs: u64,
)
    -> Outcome<()>
{
    // Parse the comma-separated scheme=value pairs, collecting the
    // timestamp and every v1 tag.
    let mut ts: Option<&str> = None;
    let mut v1s: Vec<&str> = Vec::new();
    for part in sig_header.split(',') {
        let mut kv = part.splitn(2, '=');
        let scheme = match kv.next() {
            Some(s) => s.trim(),
            None    => continue,
        };
        let value = match kv.next() {
            Some(v) => v.trim(),
            None    => continue,
        };
        match scheme {
            "t"  => ts = Some(value),
            "v1" => v1s.push(value),
            _    => {},  // Ignore v0 and any future schemes.
        }
    }

    let ts = match ts {
        Some(t) => t,
        None    => return Err(err!(
            "Stripe-Signature header has no timestamp (t=).";
            Invalid, Input, Missing)),
    };
    if v1s.is_empty() {
        return Err(err!(
            "Stripe-Signature header has no v1 signature.";
            Invalid, Input, Missing));
    }

    // Enforce the timestamp tolerance before any HMAC work.
    let t_secs = match ts.parse::<u64>() {
        Ok(n)  => n,
        Err(_) => return Err(err!(
            "Stripe-Signature timestamp '{}' is not an integer.", ts;
            Invalid, Input, Mismatch)),
    };
    let skew = if now_secs >= t_secs {
        now_secs - t_secs
    } else {
        t_secs - now_secs
    };
    if skew > tolerance_secs {
        return Err(err!(
            "Stripe-Signature timestamp outside tolerance ({}s > {}s).",
            skew, tolerance_secs;
            Invalid, Input, Range));
    }

    // Signed payload is "<t>.<body>". Build the byte string.
    let mut signed = Vec::with_capacity(ts.len() + 1 + body.len());
    signed.extend_from_slice(ts.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(body);

    // Constant-time compare the recomputed HMAC against each v1 tag.
    for v1 in &v1s {
        let tag = match hex_decode(v1) {
            Some(bytes) => bytes,
            None        => continue,  // Malformed hex cannot match.
        };
        if verify_hmac_sha256(secret.as_bytes(), &signed, &tag) {
            return Ok(());
        }
    }

    Err(err!(
        "Stripe-Signature verification failed: no v1 tag matched.";
        Invalid, Input, Security))
}

/// Decode a lowercase or uppercase hex string into bytes. Returns
/// `None` on odd length or a non-hex character.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _           => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = nibble(bytes[i])?;
        let lo = nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}


#[cfg(test)]
mod tests {
    use super::*;
    use oxedyne_fe2o3_net::hmac::hmac_sha256;

    /// Hex-encode bytes for building a synthetic Stripe-Signature.
    fn hex_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from(b"0123456789abcdef"[(b >> 4) as usize]));
            out.push(char::from(b"0123456789abcdef"[(b & 0x0F) as usize]));
        }
        out
    }

    /// Build a valid Stripe-Signature header for a body at time `t`.
    fn sign(secret: &str, body: &[u8], t: u64) -> String {
        let mut signed = Vec::new();
        signed.extend_from_slice(t.to_string().as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(body);
        let tag = hmac_sha256(secret.as_bytes(), &signed);
        fmt!("t={},v1={}", t, hex_encode(&tag))
    }

    #[test]
    fn test_valid_signature_verifies() {
        let secret = "whsec_test_secret";
        let body = br#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let t = 1_700_000_000u64;
        let header = sign(secret, body, t);
        // Within tolerance of the signing time.
        assert!(verify_stripe_signature(
            secret, body, &header, t + 10, STRIPE_SIG_TOLERANCE_SECS).is_ok());
    }

    #[test]
    fn test_tampered_body_rejected() {
        let secret = "whsec_test_secret";
        let body = br#"{"amount":100}"#;
        let t = 1_700_000_000u64;
        let header = sign(secret, body, t);
        let tampered = br#"{"amount":999}"#;
        assert!(verify_stripe_signature(
            secret, tampered, &header, t, STRIPE_SIG_TOLERANCE_SECS).is_err());
    }

    #[test]
    fn test_wrong_secret_rejected() {
        let body = br#"{"amount":100}"#;
        let t = 1_700_000_000u64;
        let header = sign("whsec_real", body, t);
        assert!(verify_stripe_signature(
            "whsec_forged", body, &header, t, STRIPE_SIG_TOLERANCE_SECS).is_err());
    }

    #[test]
    fn test_expired_timestamp_rejected() {
        let secret = "whsec_test_secret";
        let body = br#"{"ok":true}"#;
        let t = 1_700_000_000u64;
        let header = sign(secret, body, t);
        // now is well past t + tolerance.
        let now = t + STRIPE_SIG_TOLERANCE_SECS + 1;
        assert!(verify_stripe_signature(
            secret, body, &header, now, STRIPE_SIG_TOLERANCE_SECS).is_err());
    }

    #[test]
    fn test_future_timestamp_rejected() {
        let secret = "whsec_test_secret";
        let body = br#"{"ok":true}"#;
        let t = 1_700_000_000u64 + STRIPE_SIG_TOLERANCE_SECS + 5;
        let header = sign(secret, body, t);
        // now is before the signing time by more than tolerance.
        let now = 1_700_000_000u64;
        assert!(verify_stripe_signature(
            secret, body, &header, now, STRIPE_SIG_TOLERANCE_SECS).is_err());
    }

    #[test]
    fn test_multiple_v1_one_matches() {
        let secret = "whsec_test_secret";
        let body = br#"{"ok":true}"#;
        let t = 1_700_000_000u64;
        let good = sign(secret, body, t);
        // Prepend a bogus v1 to simulate a rotation window; the real
        // one still matches.
        let header = fmt!("{},v1=deadbeef", good);
        assert!(verify_stripe_signature(
            secret, body, &header, t, STRIPE_SIG_TOLERANCE_SECS).is_ok());
    }

    #[test]
    fn test_missing_v1_rejected() {
        let secret = "whsec_test_secret";
        let body = br#"{"ok":true}"#;
        let header = "t=1700000000";
        assert!(verify_stripe_signature(
            secret, body, header, 1_700_000_000u64, STRIPE_SIG_TOLERANCE_SECS).is_err());
    }
}
