//! Numerical types and operations for the Hematite ecosystem.
//! 
//! This crate provides numerical primitives and utilities including:
//! - Wrapper types for floating point numbers that implement total ordering and hashing
//! - Support for arbitrary-precision integers and decimals
//! - A comprehensive string parser for various numerical formats
//! - Traits and utilities for common numerical operations
//! 
//! The crate avoids unsafe code and unwrap operations, favouring explicit error handling
//! through the Hematite `Outcome` type.
//!
#![forbid(unsafe_code)]
#[macro_use]
pub mod macros;

pub mod float;
pub mod int;
pub mod prelude;
pub mod string;

pub use num_bigint::BigInt;
pub use bigdecimal::BigDecimal;
