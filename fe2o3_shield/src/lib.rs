//! Shield (Signed Hash In Every Little Datagram) is a security-focused peer-to-peer networking
//! protocol built on UDP. It provides session management and syntax validation layers with an
//! emphasis on denial of service resistance through proof of work.
//! 
//! The crate contains both the Shield protocol library for building custom applications and a 
//! reference UDP server implementation. Key features include:
//!
//! - Post-quantum cryptography options for key exchange and signing
//! - Lightweight handshake procedure for establishing encrypted sessions 
//! - Proof of work validation to mitigate denial of service attacks
//! - Flexible choice of cryptographic primitives
//! - Modular design supporting custom protocol implementations
//!
//! The library is under active development. While core functionality is implemented, APIs may
//! change before the 1.0 release.
//!
#![forbid(unsafe_code)]
pub mod constant;
pub mod cfg;
pub mod core;
pub mod guard;
//pub mod id;
//pub mod keys;
pub mod msg;
pub mod packet;
pub mod pow;
pub mod schemes;
pub mod server;
//pub mod session;

pub use crate::core::Shield;
