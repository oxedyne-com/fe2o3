//! Core functionality for the Hematite ecosystem.
//! 
//! This crate provides foundational types, traits and macros used throughout Hematite.
//! Key features include:
//!
//! - Error handling via [`Outcome<T>`] and [`Error<T>`] with support for tagging errors
//!   and chaining them together to preserve context during propagation
//! 
//! - A flexible logging system via [`Logger`] supporting multiple output targets and
//!   log levels with console and file support
//!
//! - Thread and bot management through [`ThreadController`] and message passing primitives 
//!   for reliable concurrent operations
//!
//! - Generic data structures and traits like [`Counter`], [`Map`], and [`Alt`] for 
//!   common programming patterns
//!
//! - Numeric type utilities via traits like [`Bound`], [`One`], and [`Zero`] with 
//!   checked arithmetic operations
//!
//! - String, path, and byte manipulation helpers with consistent error handling
//!
//! - Testing utilities with filtering and assertion support via [`test_it!`]
//!
//! - A procedural macro [`New`] for automatically implementing constructors
//!
//! # Error Handling Example
//!
//! ```
//! use fe2o3_core::prelude::*;
//!
//! fn validate_age(age: i32) -> Outcome<i32> {
//!     if age < 0 {
//!         return Err(err!(
//!             "Age cannot be negative, got {}", age
//!         ), Invalid, Input));
//!     }
//!     if age > 150 {
//!         return Err(err!(
//!             "Age seems unrealistic: {}", age
//!         ), Invalid, Range));
//!     }
//!     Ok(age)
//! }
//! ```
//!
//! The [`New`] derive macro provides automatic constructor generation:
//!
//! ```
//! use fe2o3_core::New;
//!
//! #[derive(New)]
//! struct Person {
//!     name: String,
//!     age: u32,
//! }
//! ```
//!
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused_assignments)]

#[macro_use]
pub mod macros {
    #[macro_use]
    pub mod error;
    #[macro_use]
    pub mod fallible;
    #[macro_use]
    pub mod meta;
    #[macro_use]
    pub mod string;
    #[macro_use]
    pub mod sync;
    #[macro_use]
    pub mod test;
}

//pub mod alias;
pub mod alt;
pub mod bool;
pub mod bot;
pub mod byte;
pub mod channels;
pub mod conv;
pub mod count;
pub mod error;
pub mod file;
pub mod id;
pub mod int;
pub mod log;
pub mod map;
pub mod mem;
pub mod ord;
pub mod path;
pub mod prelude;
pub mod rand;
pub mod string;
pub mod test;
pub mod time;
pub mod thread;

use error::Error;
pub use string::contains_str;

pub type Outcome<V> = std::result::Result<V, Error<error::ErrTag>>;

pub trait GenTag:
    Clone
    + std::fmt::Debug
    + Default
    + std::fmt::Display
    + Send
    + Sync
    + 'static
{}

pub fn format_type<T>(_: T) -> String {
    fmt!("{}", std::any::type_name::<T>())
}

pub use new::New; // The #[derive(New)] procedural macro for deriving a new function on a struct.
