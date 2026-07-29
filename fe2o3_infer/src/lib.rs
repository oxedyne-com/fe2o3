//! Convolutional inference on the CPU, with no dependency beyond `fe2o3_core`.
//!
//! The crate carries three layers:
//!
//! - [`kern`] -- safe `f32` kernels behind one runtime dispatch, so that a
//!   binary built for a stock target still reaches the vector unit of the
//!   machine it lands on.
//! - [`onnx`] and [`graph`] -- a loader for the subset of ONNX that a small
//!   convolutional network uses, and a runner over the operators it yields.
//! - [`face`] -- the two things built on top: a face detector that returns
//!   boxes with five landmarks, and an embedder that turns an aligned crop
//!   into a vector two faces can be compared with.
//!
//! # Owning nothing
//!
//! Nothing here reads a file, opens a socket, or starts a thread. Weights
//! arrive as `&[u8]` and images arrive as pixels; where they came from and how
//! the work is spread across cores are the caller's business. That keeps the
//! crate usable from a scanner, a server or a test with equal ease.
//!
//! # Layout
//!
//! Activations are held channels-last, `[N, H, W, C]`. In that layout a one by
//! one convolution *is* a matrix product with no gather in front of it, a
//! depthwise convolution vectorises over channels, and a per-channel scale or
//! slope is a unit-stride pass. The loader permutes the weights once so that
//! the runner never has to transpose an activation.
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

pub mod face;
pub mod graph;
pub mod kern;
pub mod onnx;
pub mod prelude;
pub mod tensor;
