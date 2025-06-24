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
pub mod tree;
