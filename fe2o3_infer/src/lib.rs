//! Convolutional inference on the CPU, with no dependency beyond `fe2o3_core`.
//!
//! This is the numerical floor: safe `f32` kernels behind one runtime dispatch,
//! so that a binary built for a stock target still reaches the vector unit of
//! the machine it lands on.
//!
//! # Layout
//!
//! Activations are held channels-last, `[N, H, W, C]`. In that layout a one by
//! one convolution *is* a matrix product with no gather in front of it, a
//! depthwise convolution vectorises over channels, and a per-channel scale or
//! slope is a unit-stride pass.
//!
//! # Safety
//!
//! The crate denies `unsafe` code with one documented exception, in
//! [`kern::run`], where calling a `#[target_feature]` function from an
//! unfeatured context requires the token even though the body is safe. There
//! are no raw pointers, no intrinsics, and no hand-written assembly anywhere.
#![deny(unsafe_code)]

#[macro_use]
pub mod macros;

pub mod kern;
pub mod tensor;
