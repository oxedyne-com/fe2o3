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

pub mod cache;
pub mod challenge;
pub mod client;
pub mod jose;
pub mod rfc8555;
pub mod trust;
