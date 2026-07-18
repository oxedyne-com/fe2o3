//! 256-bit node identifiers with XOR distance.
//!
//! The Kademlia id space is the full 256-bit range. Every routing decision is
//! expressed as a XOR distance between two [`NodeId`]s. The distance's position
//! in its binary expansion -- specifically the index of the most-significant
//! set bit -- selects which k-map in the routing table is responsible for a
//! given peer.

use oxedyne_fe2o3_core::prelude::*;

use std::{
	fmt,
	ops::BitXor,
};


/// The identifier length in bytes. // 256 bits.
pub const ID_LEN: usize = 32;

/// The identifier length in bits.
pub const ID_BITS: usize = ID_LEN * 8;


/// A 256-bit node identifier.
///
/// The byte ordering is big-endian in the logical sense: index `0` holds the
/// most-significant byte. `Ord` and `PartialOrd` follow the natural byte-wise
/// ordering and are only useful for deterministic iteration, not for XOR
/// distance comparison -- use [`NodeId::distance`] and its returned
/// [`Distance`] for that.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(pub [u8; ID_LEN]);

impl NodeId {
	/// Constructs an identifier from a raw 32-byte array.
	pub const fn from_bytes(bytes: [u8; ID_LEN]) -> Self {
		Self(bytes)
	}

	/// Constructs an identifier from a byte slice.
	///
	/// Returns an error if the slice is not exactly [`ID_LEN`] bytes.
	pub fn from_slice(bytes: &[u8]) -> Outcome<Self> {
		if bytes.len() != ID_LEN {
			return Err(err!(
				"NodeId requires exactly {} bytes, got {}.", ID_LEN, bytes.len();
			Invalid, Input, Size));
		}
		let mut arr = [0u8; ID_LEN];
		arr.copy_from_slice(bytes);
		Ok(Self(arr))
	}

	/// Returns the identifier as a byte slice.
	pub fn as_bytes(&self) -> &[u8; ID_LEN] {
		&self.0
	}

	/// The XOR distance between two identifiers.
	pub fn distance(&self, other: &Self) -> Distance {
		let mut out = [0u8; ID_LEN];
		for i in 0..ID_LEN {
			out[i] = self.0[i] ^ other.0[i];
		}
		Distance(out)
	}

	/// The index of the k-map that holds peers at the XOR distance between
	/// `self` and `other`.
	///
	/// The index is the bit-position of the distance's most-significant set
	/// bit, counted from the least-significant bit (so index `0` is the
	/// closest non-self bucket and index `255` is the furthest). If the two
	/// identifiers are equal the distance is zero and this returns `None` --
	/// a node should never appear in its own routing table.
	pub fn bucket_index(&self, other: &Self) -> Option<usize> {
		self.distance(other).bucket_index()
	}
}

impl BitXor for NodeId {
	type Output = Distance;

	fn bitxor(self, rhs: Self) -> Self::Output {
		self.distance(&rhs)
	}
}

impl fmt::Display for NodeId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for b in &self.0 {
			ok!(write!(f, "{:02x}", b));
		}
		Ok(())
	}
}


/// A 256-bit XOR distance between two [`NodeId`]s.
///
/// Comparison is byte-wise big-endian, which is equivalent to numeric
/// comparison of the corresponding 256-bit unsigned integer. Smaller values
/// are "closer" in the Kademlia sense.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Distance(pub [u8; ID_LEN]);

impl Distance {
	/// The zero distance. Two equal identifiers.
	pub const ZERO: Self = Self([0u8; ID_LEN]);

	/// Returns `true` if the distance is zero.
	pub fn is_zero(&self) -> bool {
		self.0.iter().all(|b| *b == 0)
	}

	/// The index of the most-significant set bit, counted from the
	/// least-significant bit.
	///
	/// Returns `None` if the distance is zero. For any non-zero distance the
	/// result is in `0..ID_BITS`.
	pub fn bucket_index(&self) -> Option<usize> {
		for (i, b) in self.0.iter().enumerate() {
			if *b != 0 {
				// Byte `i` is the most-significant non-zero byte. Within the
				// byte, the most-significant set bit sits at bit-position
				// (7 - leading_zeros). The overall bit-index from the LSB is
				// then (ID_BITS - 1 - 8*i - leading_zeros_within_byte).
				let byte_lz = b.leading_zeros() as usize;
				return Some(ID_BITS - 1 - (i * 8 + byte_lz));
			}
		}
		None
	}
}

impl fmt::Display for Distance {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for b in &self.0 {
			ok!(write!(f, "{:02x}", b));
		}
		Ok(())
	}
}
