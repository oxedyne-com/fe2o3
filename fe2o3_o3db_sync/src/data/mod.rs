//! In-memory data handling: caches, key routing and record encoding.
//!
//! [`cache`] holds the per-zone value cache and its entries, [`choose`]
//! implements the deterministic hash-based selection of the owning cache bot
//! and zone, and [`core`] defines the key, value and rest-scheme types along
//! with the encode/decode of records.

pub mod cache;
pub mod choose;
pub mod core;
//pub mod user;
