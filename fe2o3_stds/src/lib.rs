//! A standards library providing commonly used enumerations and constants.
//! 
//! This crate serves as a central repository for standardised data structures used throughout the
//! Hematite ecosystem. It currently provides:
//! 
//! - A comprehensive enumeration of countries through the `regions::Country` enum
//! - Basic celestial body identifiers via `regions::CelestialBodies`
//! - ANSI terminal control sequences through the `chars::Term` struct
//!
//! The crate maintains zero dependencies to ensure it can serve as a reliable foundation for other
//! components. All data structures are designed to be efficient and easy to maintain, focussing on
//! enumerations that provide compile-time guarantees of correctness.
//!
#![forbid(unsafe_code)]
pub mod chars;
pub mod regions;
