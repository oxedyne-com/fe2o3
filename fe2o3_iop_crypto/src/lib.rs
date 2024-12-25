//! Provides interoperability traits and implementations for cryptographic operations.
//! 
//! This crate defines fundamental traits that allow cryptographic schemes to be used 
//! interchangeably through common interfaces:
//! 
//! - `KeyExchanger` for key exchange mechanisms
//! - `Encrypter` for encryption/decryption operations  
//! - `Signer` for digital signatures
//! - `KeyManager` for handling public/private key pairs
//! 
//! Each trait is implemented via a `DefAlt` pattern that supports fallback behaviour between
//! default and given implementations. The implementations handle proper error propagation and
//! maintain type safety throughout cryptographic operations.
//!
//! All trait implementations avoid unsafe code and unwrap operations to maintain robustness.
//!
#![forbid(unsafe_code)]
pub mod enc;
pub mod kem;
pub mod keys;
pub mod sign;
