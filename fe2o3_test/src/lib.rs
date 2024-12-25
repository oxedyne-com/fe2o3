//! A testing utility crate for the Hematite ecosystem that provides a structured way to organise and run
//! tests.
//! 
//! # Test Organisation
//! 
//! Tests in Hematite projects typically follow this structure:
//! 
//! - A `tests/main.rs` file as the entry point that configures logging and runs all test modules
//! - Individual test modules (e.g. `tests/map.rs`, `tests/string.rs`) containing related test functions
//! - Use of the `test_it!` macro to implement filterable, grouped test cases
//! 
//! # Example Usage
//! 
//! In your `tests/main.rs`:
//! ```rust
//! mod map;
//! mod string;
//! 
//! use oxedize_fe2o3_core::prelude::*;
//! 
//! #[test]
//! fn main() -> Outcome<()> {
//!     // Set up logging before running tests
//!     set_log_level!("test");
//!     
//!     let outcome = run_tests();
//!     
//!     // Allow logger thread to complete before exiting
//!     log_finish_wait!();
//!     
//!     outcome
//! }
//! 
//! fn run_tests() -> Outcome<()> {
//!     let filter = "all";  // Or specific test group
//!     
//!     res!(map::test_map_func(filter));
//!     res!(string::test_string_func(filter));
//!     
//!     Ok(())
//! }
//! ```
//! 
//! In your test modules (e.g. `tests/map.rs`):
//! ```rust
//! use oxedize_fe2o3_core::{prelude::*, test::test_it};
//! 
//! pub fn test_map_func(filter: &'static str) -> Outcome<()> {
//!     // Run test cases that match the filter
//!     res!(test_it(filter, &["Map Find 000", "all", "map"], || {
//!         // Your test code here
//!         Ok(())
//!     }));
//!     
//!     // Additional test cases...
//!     res!(test_it(filter, &["Map Find 010", "all", "map"], || {
//!         // More test code
//!         Ok(())
//!     }));
//!     
//!     Ok(())
//! }
//! ```
//! 
//! The `test_it` macro allows you to:
//! - Group related test cases together in a single test function
//! - Filter which tests run using tags
//! - Provide descriptive names for test cases
//! - Ensure proper error handling and logging throughout the test suite
//!
#![forbid(unsafe_code)]
pub mod data;
pub mod error;
