//! File system utilities for the Hematite ecosystem.
//!
//! This crate provides tools for working with file system hierarchies, with a focus on directory tree
//! traversal and metadata handling. The primary type is [`FileTree`] which creates an in-memory
//! representation of a directory structure.
//!
//! # Key Features
//! - Tree-based directory structure representation using [`FileTree`]
//! - File metadata tracking through [`Attributes`] including modification times and sizes
//! - Support for both files and directories via [`Node`] variants
//! - Integration with Hematite's error handling through [`Outcome`]
//!
//! # Example
//! ```no_run
//! use oxedyne_fe2o3_file::tree::FileTree;
//!
//! // Create a file tree from a directory path
//! let tree = FileTree::new("/path/to/directory").unwrap();
//! ```
//!
//! The crate builds on Hematite's core data structures and error handling patterns to provide a
//! safe and maintainable approach to file system operations.
//!
//! # Archives and the formats built on them
//!
//! [`zip`] is an archive held in memory over bytes, written target-neutral, on the reading that an
//! archive is a filesystem in a file. It preserves the members it does not understand, byte for byte.
//!
//! [`office`] is the six Office formats -- `.docx`, `.xlsx`, `.pptx` and their OpenDocument
//! counterparts -- each of which is an archive of XML. They sit here rather than beside the document
//! tree in `fe2o3_text::doc`, where they would read more naturally, because they cannot: `fe2o3_jdat`
//! depends on `fe2o3_text`, so `fe2o3_text` can never depend on anything that depends on `fe2o3_jdat`,
//! and the archive does. This crate is on the other side of that line and can use both.

pub mod exif;
pub mod glob;
pub mod office;
pub mod tree;
pub mod zip;
