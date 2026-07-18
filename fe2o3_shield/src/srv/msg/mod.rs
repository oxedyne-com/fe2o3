//! Message subsystem for the Shield wire protocol.
//!
//! This module defines the on-wire packet format, multi-packet assembly and
//! validation, encoding and decoding, the handshake exchange and the protocol
//! syntax that binds them together.
pub mod assemble;
pub mod core;
pub mod decode;
pub mod encode;
pub mod handshake;
pub mod protocol;
pub mod packet;
pub mod syntax;
