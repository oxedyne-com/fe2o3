//! A hashing library providing checksums, cryptographic hashes and proof-of-work functionality.
//! 
//! This crate is part of the Hematite collection and provides:
//! 
//! - Checksum generation and verification through the [`csum`] module
//! - Cryptographic hashing via the [`hash`] module, supporting algorithms like SHA3-256
//! - Key derivation functions in the [`kdf`] module, implementing Argon2
//! - A concurrent sharded hashmap in the [`map`] module for high-performance applications
//! - Proof-of-work mining and verification in the [`pow`] module
//! 
//! The crate integrates with Hematite's error handling system and Namex scheme identification,
//! whilst providing generic traits for custom algorithm implementations.
//! 
//! # Examples
//! 
//! ```
//! use oxedyne_fe2o3_hash::{
//!     hash::HashScheme,
//!     prelude::*,
//! };
//! 
//! // Create a SHA3-256 hasher.
//! let hasher = HashScheme::new_sha3_256();
//! let input = b"example data";
//! 
//! // Generate a hash with an empty salt.
//! let hash = hasher.hash(&[input], []);
//! ```
#![forbid(unsafe_code)]

use oxedyne_fe2o3_jdat::version::SemVer;


pub mod csum;
pub mod map;
pub mod hash;
pub mod kdf;
pub mod pow;

pub const VERSION: SemVer = SemVer::new(0,0,1);
