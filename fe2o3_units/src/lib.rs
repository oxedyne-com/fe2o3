//! A library for working with scientific units, scales and dimensions.
//! 
//! Provides functionality for handling physical quantities with proper units and scale prefixes 
//! (e.g. milli, kilo, etc). Includes support for both decimal (SI) and binary scaling systems, 
//! significant figure tracking, and unit dimension validation.
//! 
//! Key features:
//! - SI unit system implementation with base units (metre, kilogram, second, etc)
//! - Decimal (SI) and binary prefix support (e.g. kilo/kibi, mega/mebi)
//! - Automatic scale normalisation and humanisation
//! - Dimensional analysis through the System trait
//! - Significant figure preservation in calculations
//! - Proper handling of zero values and edge cases
//!
//! # Examples
//! 
//! Working with data sizes using binary prefixes:
//! ```rust
//! use oxedyne_fe2o3_units::{Units, SI};
//! use oxedyne_fe2o3_core::prelude::*;
//! 
//! fn main() -> Outcome<()> {
//!     // Create a measurement representing 1024 bytes with 4 significant figures
//!     let data_size = res!(Units::<SI>::bytes(1024.0, 4));
//!     
//!     // Convert to a human-readable form (automatically scales to kibibytes)
//!     let human_readable = data_size.humanise();
//!     assert_eq!(human_readable.val(), 1.0);
//!     assert_eq!(human_readable.prefix(), "Ki");
//!     assert_eq!(human_readable.symbol(), "B");
//!     Ok(())
//! }
//! ```
//!
//! Working with SI units and decimal prefixes:
//! ```rust
//! use oxedyne_fe2o3_units::scale::Mag;
//! use oxedyne_fe2o3_core::prelude::*;
//! 
//! fn main() -> Outcome<()> {
//!     // Create a measurement of 1234000 microseconds with 4 significant figures 
//!     let time = res!(Mag::micro(1234000.0, 4));
//!     
//!     // Convert to a human-readable form (automatically scales to seconds)
//!     let human_time = time.humanise();
//!     assert_eq!(human_time.val(), 1.234);
//!     assert_eq!(human_time.prefix(), "");  // No prefix needed for base unit
//!     Ok(())
//! }
//! ```
//!
//! Custom unit systems can be created by implementing the System trait:
//! ```rust
//! use oxedyne_fe2o3_units::system::System;
//! 
//! #[derive(Clone, Debug, PartialEq)]
//! struct Currency {
//!     symbol: &'static str,
//! }
//! 
//! impl System for Currency {
//!     fn base_symbol(&self) -> &'static str {
//!         self.symbol
//!     }
//! }
//! 
//! impl std::fmt::Display for Currency {
//!     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//!         write!(f, "{}", self.symbol)
//!     }
//! }
//! ```
//!
#![forbid(unsafe_code)]
pub mod scale;
pub mod si;
pub mod system;
