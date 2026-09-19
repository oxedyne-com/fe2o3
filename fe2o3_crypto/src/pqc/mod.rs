#![allow(unused)]

/// Dilithium, in pure Rust. It needs no C, so it is here whatever the target -- but the
/// algorithm itself has no size without a parameter set, so it needs one of the `mode0`..`mode3`
/// features. Without one it is absent rather than broken: every K/L/ETA/SETABITS/BETA/OMEGA-sized
/// item downstream, from packed lengths to sign/verify, has no value to take.
#[cfg(any(feature = "mode0", feature = "mode1", feature = "mode2", feature = "mode3"))]
pub mod dilithium;
pub mod macros_dilithium;

/// SABER rests on the C reference implementation this crate compiles, so it is absent without the
/// `pq` feature, along with the C toolchain that feature needs.
#[cfg(feature = "pq")]
pub mod macros_saber;
#[cfg(feature = "pq")]
pub mod saber;
