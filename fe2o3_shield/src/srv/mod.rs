//! Server-side implementation of the Shield protocol.
//!
//! This module houses the core protocol machinery: configuration and runtime
//! context, the guard system for DoS mitigation, the message subsystem
//! (packets, assembly and handshake), the proof-of-work engine, cryptographic
//! scheme selection and the UDP server loop itself.
pub mod cfg;
pub mod cmd;
pub mod constant;
pub mod context;
pub mod guard;
pub mod msg;
pub mod pow;
pub mod schemes;
pub mod server;
pub mod test;
