//! An Invertible Bloom Lookup Table primitive for the Hematite distributed
//! Ozone layer.
//!
//! An IBLT is a fixed-size sketch that supports insert, delete, subtract and
//! peeling-decode over fixed-length byte keys. Two IBLTs with the same
//! configuration subtract cell-wise into a third IBLT that encodes their
//! symmetric difference; the peeling decoder recovers the distinct keys (and
//! optional values) from that difference, in time linear in the difference's
//! size, not the input's. IBLTs shine when the expected difference is small
//! compared to the total dataset -- which is the steady-state case for
//! Ozone anti-entropy.
//!
//! # Sizing rule of thumb
//!
//! For a target symmetric difference of at most `d` entries, allocate
//! `num_cells ≈ 1.5 × d` with `num_hashes = 3`, or `num_cells ≈ 1.3 × d`
//! with `num_hashes = 4`. Below these thresholds the decode succeeds with
//! high probability; above them the peeling stalls and the caller must fall
//! back to a larger IBLT or a bulk transfer. See
//! [`iblt::DecodeOutcome::Incomplete`] for the failure path.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_data::iblt::{DecodeOutcome, Iblt, IbltConfig};
//!
//! # fn main() -> Outcome<()> {
//! let cfg = IbltConfig {
//!     num_cells:	80,
//!     num_hashes:	3,
//!     key_len:	8,
//!     value_len:	0,
//!     seed:		0xabcd_1234,
//! };
//!
//! let mut a = res!(Iblt::new(cfg));
//! let mut b = res!(Iblt::new(cfg));
//!
//! // A has {1..20}, B has {10..30}. Symmetric difference = 20 keys.
//! for i in 1u64..20 {
//!     res!(a.insert(&i.to_le_bytes(), &[]));
//! }
//! for i in 10u64..30 {
//!     res!(b.insert(&i.to_le_bytes(), &[]));
//! }
//!
//! // Diff = A minus B.
//! res!(a.subtract(&b));
//! match res!(a.decode()) {
//!     DecodeOutcome::Complete { inserted, deleted } => {
//!         // `inserted` = keys in A but not B; `deleted` = keys in B but not A.
//!         assert_eq!(inserted.len() + deleted.len(), 19);
//!     },
//!     DecodeOutcome::Incomplete { .. } => panic!("unexpected overload"),
//! }
//! # Ok(())
//! # }
//! ```
mod hash;
mod imp;

pub use imp::{
	DecodeOutcome,
	Iblt,
	IbltConfig,
};
