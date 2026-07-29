//! Convolutional inference on the CPU, with no dependency beyond `fe2o3_core`.
//!
//! The crate carries two layers so far:
//!
//! - [`kern`] -- safe `f32` kernels behind one runtime dispatch, so that a
//!   binary built for a stock target still reaches the vector unit of the
//!   machine it lands on.
//! - [`onnx`] and [`graph`] -- a loader for the subset of ONNX that a small
//!   convolutional network uses, and a runner over the operators it yields.
//!
//! # Owning nothing
//!
//! Nothing here reads a file, opens a socket, or starts a thread. Weights
//! arrive as `&[u8]`; where they came from and how the work is spread across
//! cores are the caller's business.
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

pub mod graph;
pub mod kern;
pub mod onnx;
pub mod tensor;
