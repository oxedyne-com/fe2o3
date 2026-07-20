//! A universal name codex system for distributed identifier management.
//!
//! Namex provides a decentralised approach to mapping 32-byte identifiers to named entities,
//! allowing consistent identification of schemes, formats, and other specifications across
//! distributed systems. The system supports:
//!
//! - Entity storage with rich metadata including names, descriptions and cross-references
//! - Multiple names per entity with language support
//! - Hierarchical tagging for flexible categorisation
//! - JDAT-based storage format with JSON export capability
//! - Distributed operation through file sharing and merging
//!
//! # Example
//!
//! ```no_run
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_namex::{
//!     db::{Entity, MapKey, Namex},
//!     id::NamexId,
//! };
//! use std::collections::BTreeMap;
//!
//! fn example() -> Outcome<()> {
//!     // Load an existing namex database.
//!     let namex = res!(Namex::<
//!         BTreeMap<MapKey, Entity>,
//!         BTreeMap<NamexId, Entity>,
//!     >::load("namex.jdat"));
//!
//!     // Export as JDAT or JSON.
//!     let jdat_lines = res!(namex.export_jdat());
//!     res!(namex.to_file(jdat_lines, "export.jdat"));
//!
//!     Ok(())
//! }
//! ```
//!
#![forbid(unsafe_code)]
pub mod db;
pub mod id;

pub use id::InNamex;
