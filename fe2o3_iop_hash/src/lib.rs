//! Interoperability layer for hash functions, checksums and key derivation.
//! 
//! This crate provides traits that define common interfaces for different types of hashing:
//! 
//! - [`Hasher`](api::Hasher) - Core trait for hash functions that transform input data into 
//!   fixed-size outputs, with optional salting.
//! 
//! - [`Checksummer`](csum::Checksummer) - Trait for incremental checksum calculation and 
//!   verification over arbitrary data streams.
//! 
//! - [`KeyDeriver`](kdf::KeyDeriver) - Trait for password hashing and key derivation functions,
//!   supporting salt generation and verification.
//! 
//! All traits require `Send + Sync` for thread safety and implement `InNamex` for registration
//! in Hematite's universal name codex system.
//!
#![forbid(unsafe_code)]
pub mod api;
pub mod csum;
pub mod kdf;
