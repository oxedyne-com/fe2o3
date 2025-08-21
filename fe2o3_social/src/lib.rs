//! Social network graph generation and analysis.
//! 
//! This crate provides tools for generating realistic social networks using a stub matching
//! algorithm with configurable population profiles and geographic distributions.
//! 
//! Key components:
//! - Social network graph generation with memory-mapped storage
//! - Person modelling with demographic attributes
//! - Cultural and gender classifications
//! - Memory-mapped graph storage for large networks
//!
#![deny(unsafe_code)]
// Exception: Allow unsafe only for memory mapping in specific, well-documented cases
pub mod culture;
pub mod graph;
pub mod mmap_graph;
pub mod mmap_index;
pub mod person;
