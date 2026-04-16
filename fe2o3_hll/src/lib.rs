//! A HyperLogLog cardinality-sketch primitive for the Hematite distributed
//! Ozone layer.
//!
//! The sketch estimates the number of distinct 64-bit hashes it has observed,
//! in fixed space regardless of the true cardinality. Two sketches of equal
//! precision merge by register-wise maximum into a sketch that represents
//! the union of their inputs -- without revealing which input supplied which
//! element.
//!
//! # Hash function is the caller's choice
//!
//! This crate does not bundle a hash function. [`sketch::HyperLogLog::add_hash`]
//! takes a `u64` directly. Callers pick a hash appropriate to their domain:
//! SeaHash for generic uniformly-distributed bytes, SipHash for keyed
//! resistance to adversarial inputs, or a truncated cryptographic hash when
//! the sketch is part of a larger authenticated protocol. The sketch itself
//! makes no guarantee about adversarial resistance -- if your inputs come
//! from a potentially hostile source, use a keyed hash.
//!
//! # Distributed Ozone usage
//!
//! See #raw("sec_ozone.typ") §"Network Size Estimation: HyperLogLog". Every
//! peer keeps a 16 KiB sketch (precision `p = 14`) tracking the peer ids it
//! has observed. Every few hours peers swap sketches with a small sample of
//! the network, merge, and update their local estimate of $N$. The merged
//! result converges to within ~2% of the true cardinality within 5-10 rounds.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_hll::sketch::{HyperLogLog, P_DEFAULT};
//!
//! # fn main() -> Outcome<()> {
//! let mut sketch = res!(HyperLogLog::new(P_DEFAULT));
//! for i in 0u64..1_000 {
//!     // Caller-chosen hash: here we use a trivial mixer for demonstration.
//!     let h = i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
//!     sketch.add_hash(h);
//! }
//! let estimate = sketch.estimate_rounded();
//! assert!(estimate > 900 && estimate < 1100, "estimate = {}", estimate);
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]

pub mod sketch;
