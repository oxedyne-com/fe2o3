//! The 256-bit placement threshold.
//!
//! A [`Threshold`] encodes the right-hand side of OAM's placement inequality
//!
//! $ "XOR"("peer_id", H("record")) < T $
//!
//! where `T = floor(2^256 * n / N)`. The threshold is computed once from the
//! configuration and then applied to many records, so peer-side placement
//! decisions reduce to a single 32-byte bytewise comparison.
//!
//! Three cases are represented explicitly so the saturation boundary is
//! unambiguous:
//!
//! - [`Threshold::None`] -- `n = 0`. No peer is a holder.
//! - [`Threshold::Bounded`] -- `0 < n < N`. The placement inequality uses the
//!   stored 256-bit value.
//! - [`Threshold::All`] -- `n >= N` (or degenerate `N = 0`). Every peer is a
//!   holder, independent of the XOR distance. Treated as "threshold equals
//!   `2^256`", which would not fit in a 256-bit word, so it is held as a
//!   sentinel instead.

use oxedyne_fe2o3_core::prelude::*;
use crate::kademlia::id::{
	Distance,
	ID_LEN,
	NodeId,
};


/// Number of 64-bit limbs used to represent the 256-bit threshold during
/// computation. Big-endian; limb index `0` is most-significant.
const LIMBS: usize = 4;


/// A 256-bit placement threshold with an explicit saturation boundary.
///
/// Comparison is strict less-than in the `Bounded` case, mirroring the
/// inequality in the Ozone chapter of the Hematite specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Threshold {
	/// `n = 0`. No peer holds any record.
	None,
	/// `0 < n < N`. A peer holds the record if its XOR distance to the record
	/// hash is strictly less than the wrapped 256-bit value.
	Bounded([u8; ID_LEN]),
	/// `n >= N` (or `N = 0`). Every peer holds every record.
	All,
}

impl Threshold {
	/// Computes `T = floor(2^256 * n / network_size)` for the standard case
	/// `0 < n < network_size`.
	///
	/// The two boundary cases are encoded explicitly:
	///
	/// - `n == 0` -- returns [`Threshold::None`]. The probability of any peer
	///   holding any record is zero.
	/// - `network_size == 0` or `n >= network_size` -- returns
	///   [`Threshold::All`]. Every peer holds every record.
	pub fn from_params(n: u64, network_size: u64) -> Self {
		if n == 0 {
			return Self::None;
		}
		if network_size == 0 || n >= network_size {
			return Self::All;
		}
		// Long division of the dividend `n * 2^256` by `network_size`.
		//
		// Dividend, big-endian u64 limbs: [n, 0, 0, 0, 0]. Five limbs because
		// `n * 2^256` occupies bits 256..(256 + 64); zero-padding the
		// low 256 bits yields the 5-limb representation.
		//
		// For `n < network_size` the top quotient limb is zero, so the
		// bottom four limbs fit in a 256-bit threshold without truncation.
		let dividend: [u64; LIMBS + 1] = [n, 0, 0, 0, 0];
		let divisor = network_size as u128;
		let mut quotient = [0u64; LIMBS + 1];
		let mut rem: u128 = 0;
		for i in 0..=LIMBS {
			let combined = (rem << 64) | (dividend[i] as u128);
			quotient[i] = (combined / divisor) as u64;
			rem = combined % divisor;
		}
		// `quotient[0]` must be zero because `n < network_size`. The
		// meaningful quotient is the lower four limbs. Assemble them
		// big-endian into the 32-byte representation.
		let mut out = [0u8; ID_LEN];
		for i in 0..LIMBS {
			let start = i * 8;
			out[start..start + 8].copy_from_slice(&quotient[i + 1].to_be_bytes());
		}
		Self::Bounded(out)
	}

	/// Returns `true` if a peer at the given XOR distance from the record hash
	/// is a holder under this threshold.
	pub fn contains(&self, distance: &Distance) -> bool {
		match self {
			Self::None => false,
			Self::All => true,
			Self::Bounded(t) => distance.0.as_slice() < t.as_slice(),
		}
	}

	/// Returns the stored 256-bit threshold value in the `Bounded` case.
	///
	/// Returns `None` for the saturation boundaries `None` and `All`, which
	/// have no finite 256-bit representation.
	pub fn as_bytes(&self) -> Option<&[u8; ID_LEN]> {
		match self {
			Self::Bounded(t) => Some(t),
			_ => None,
		}
	}

	/// Returns the threshold as a [`NodeId`]-shaped value when bounded. Handy
	/// for callers that want to drive iterator comparisons against XOR
	/// distances without rewrapping the bytes.
	pub fn as_node_id(&self) -> Option<NodeId> {
		self.as_bytes().map(|b| NodeId::from_bytes(*b))
	}
}
