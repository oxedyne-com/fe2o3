//! This crate provides cryptographic primitives and implementations focused on post-quantum security.
//! 
//! With the advent of quantum computers, many widely-used cryptographic algorithms based on
//! classical hard problems like integer factorisation will become vulnerable. This crate
//! implements quantum-resistant schemes that were selected as finalists in the NIST Post-Quantum
//! Cryptography standardisation process.
//! 
//! # Key Features
//! 
//! - SABER key encapsulation mechanism (KEM) for quantum-resistant key exchange
//!   - Includes LightSaber, Saber and FireSaber variants
//!   - Pure Rust and C reference implementations
//!   - Constant-time operations where possible
//! 
//! - Dilithium digital signature scheme for quantum-resistant signatures
//!   - Pure Rust implementation
//!   - Based on module lattice problems
//!   - Configurable security levels
//! 
//! - Classical cryptographic primitives
//!   - AES-256-GCM for symmetric encryption
//!   - Ed25519 for classical digital signatures
//! 
//! - Generic traits and types
//!   - `EncryptionScheme` for symmetric encryption
//!   - `KeyExchangeScheme` for key exchange/encapsulation
//!   - `SignatureScheme` for digital signatures
//!   - Safe key management with `Keys` type
//! 
//! The implementations aim to be memory safe and avoid panics while maintaining efficiency.
//! Post-quantum schemes are implemented based on reference implementations and validated against
//! test vectors.
//! 
//! # Example Usage
//! 
//! ```ignore
//! use oxedize_fe2o3_crypto::{EncryptionScheme, SignatureScheme};
//! use oxedize_fe2o3_core::prelude::*;
//! 
//! // Create a post-quantum signature scheme
//! let scheme = res!(SignatureScheme::new_dilithium2());
//! 
//! // Sign a message
//! let message = b"Hello, post-quantum world!";
//! let signature = res!(scheme.sign(message));
//! 
//! // Verify the signature
//! assert!(res!(scheme.verify(message, &signature)));
//!
//#![forbid(unsafe_code)] // Unfortunately need to remove for c interop. TODO oxedize everything!
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
//#![allow(unused)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

//#[macro_use]
//pub mod macros_dilithium;
//pub mod macros_saber;

pub mod enc;
pub mod kem;
pub mod keys;
pub mod pqc;
pub mod scheme;
pub mod sign;
//pub mod wasm;

use oxedize_fe2o3_jdat::version::SemVer;

pub const VERSION: SemVer = SemVer::new(0,0,1);
