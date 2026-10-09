//! Specialised data structures for the Hematite ecosystem.
//! 
//! This crate provides several core data structures:
//! 
//! - Generation-swept maps ([`gen_map`]) - A cache map that drops what the last two compiles left untouched, under a byte ceiling
//! - Interval maps ([`interval`]) - Disjoint half-open integer ranges mapped to values
//! - Ring buffers ([`ring`]) - Fixed-size circular buffers with position tracking
//! - Stacks ([`stack`]) - Immutable stack implementation using Arc for thread-safety
//! - Trees ([`tree`]) - Generic tree structure with navigation and display capabilities
//! - Time utilities ([`time`]) - Timestamp wrappers and timing measurements
//!
//! The implementations emphasise:
//! - Memory safety through careful ownership management
//! - Thread-safety where appropriate (e.g. Arc-based stack)
//! - Clear error handling using the Hematite error framework
//! - Integration with Hematite serialisation via Jdat
//! - No use of unsafe code or unwrap
//!
//! Most types implement common traits like Clone, Debug, and Jdat serialisation where
//! appropriate.
//!
#![forbid(unsafe_code)]
pub mod digraph;
pub mod gen_map;
pub mod hll;
pub mod iblt;
pub mod interval;
pub mod ring;
pub mod stack;
pub mod time;
pub mod tree;
