//! Interoperability layer for databases in the Hematite ecosystem.
//! 
//! This crate provides core traits and structures that define common database behaviours,
//! particularly around key-value operations and metadata handling. It includes:
//! 
//! - The `Database` trait defining standard database operations with encryption and hashing support
//! - Metadata structures for tracking timestamps and user information
//! - Scheme override capabilities for customising encryption and hashing behaviours
//! 
//! The crate serves as an abstraction layer between database implementations and consumers,
//! ensuring consistent interfaces whilst allowing flexibility in specific implementations.
//!
#![forbid(unsafe_code)]
pub mod api;
