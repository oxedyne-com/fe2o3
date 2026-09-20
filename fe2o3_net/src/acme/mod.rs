//! ACME (Automatic Certificate Management Environment) client, RFC 8555.
//!
//! This module implements the subset of the ACME protocol needed to obtain and
//! renew TLS server certificates from a certificate authority such as Let's
//! Encrypt using the `tls-alpn-01` challenge type.
//!
//! The submodules are layered from low-level primitives upward:
//!
//! - [`jose`] provides the ES256 JSON Web Signature primitive that every ACME
//!   request is wrapped in.
//!
//! Further submodules covering RFC 8555 message types, the ACME client state
//! machine, the TLS-ALPN-01 challenge cert generator and the renewal loop are
//! added incrementally on top of [`jose`].
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

// `jose`, `cache` and `rfc8555` are tokio-free primitives (JWS, the on-disk cache
// and the RFC 8555 message types); `jose` in particular is reused by the
// tokio-free `webauthn` verifier. The client state machine, the renewal/trust
// TLS glue and the rcgen-backed challenge cert generator are the async/TLS half.
pub mod cache;
#[cfg(feature = "async")]
pub mod challenge;
#[cfg(feature = "async")]
pub mod client;
pub mod jose;
pub mod rfc8555;
#[cfg(feature = "async")]
pub mod trust;
