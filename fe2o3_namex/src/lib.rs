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
//! ```rust
//! use oxedize_fe2o3_namex::{Namex, Entity};
//! use std::collections::BTreeMap;
//!
//! // Load an existing namex database
//! let namex = Namex::<BTreeMap<_,_>, BTreeMap<_,_>>::load("namex.jdat")?;
//!
//! // Export as JDAT or JSON
//! let jdat_lines = namex.export_jdat()?;
//! namex.to_file(jdat_lines, "export.jdat")?;
//! ```
//!
#![forbid(unsafe_code)]
pub mod db;
pub mod id;

pub use id::InNamex;
