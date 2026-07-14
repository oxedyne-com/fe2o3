#![allow(unused)]

/// Dilithium, in pure Rust. It needs no C, so it is here whatever the target.
pub mod dilithium;
pub mod macros_dilithium;

/// SABER rests on the C reference implementation this crate compiles, so it is absent without the
/// `pq` feature, along with the C toolchain that feature needs.
#[cfg(feature = "pq")]
pub mod macros_saber;
#[cfg(feature = "pq")]
pub mod saber;
