//! The HyperLogLog cardinality sketch.
//!
//! A [`HyperLogLog`] sketch uses $m = 2^p$ single-byte registers to estimate
//! the number of distinct 64-bit hashes it has observed. The sketch is fixed
//! size regardless of the true cardinality and merges with any other sketch
//! of the same precision by register-wise maximum.
//!
//! This crate does not bundle a hash function. Callers hash their inputs to
//! `u64` using whichever algorithm suits them (SeaHash, SipHash, SHA3
//! truncated, etc.) and call [`HyperLogLog::add_hash`]. Keeping the hash
//! choice external preserves the primitive's independence from any particular
//! authentication or cryptographic scheme.

use oxedyne_fe2o3_core::prelude::*;


/// The minimum precision parameter. With `p = 4` the sketch has 16 registers.
pub const P_MIN: u8 = 4;

/// The maximum precision parameter. With `p = 18` the sketch has 262144
/// registers. Above this the leading-zero range left in the 64-bit hash
/// collapses to fewer than 64 bits of entropy on the register side, which is
/// both wasteful and risks undercounting.
pub const P_MAX: u8 = 18;

/// The precision used by the distributed Ozone layer. 16384 registers, each
/// one byte -- a 16 KiB sketch, matching #raw("sec_ozone.typ") §"Network Size
/// Estimation: HyperLogLog".
pub const P_DEFAULT: u8 = 14;


/// A HyperLogLog cardinality sketch.
///
/// Holds $m = 2^p$ single-byte registers. Each register stores
/// $max(rho(h) : h mod m = j)$ where $rho(h)$ is the 1-based position of the
/// first set bit in the hash suffix after the register-selecting prefix, and
/// the max is taken over every hash observed so far that maps to register $j$.
#[derive(Clone, Debug)]
pub struct HyperLogLog {
	/// The precision parameter.
	p:			u8,
	/// The $m = 2^p$ register bytes.
	registers:	Vec<u8>,
}

impl HyperLogLog {
	/// Builds an empty sketch with precision `p`.
	///
	/// Validates `p ∈ [P_MIN, P_MAX]`. Allocates `2^p` zeroed bytes.
	pub fn new(p: u8) -> Outcome<Self> {
		if p < P_MIN || p > P_MAX {
			return Err(err!(
				"HyperLogLog precision p = {} out of range [{}, {}].",
				p, P_MIN, P_MAX;
			Invalid, Input));
		}
		let m = 1usize << p;
		Ok(Self {
			p,
			registers: vec![0u8; m],
		})
	}

	/// Constructs a sketch from raw register bytes.
	///
	/// Validates `p ∈ [P_MIN, P_MAX]` and that `bytes.len() == 2^p`. The
	/// bytes are copied into the sketch unchanged -- register values are not
	/// clamped; a malformed peer could feed a register above `64 - p + 1` and
	/// inflate the estimate. Callers exchanging sketches over the wire should
	/// apply their own rate limiting and reputation accounting before merging.
	pub fn from_bytes(p: u8, bytes: &[u8]) -> Outcome<Self> {
		if p < P_MIN || p > P_MAX {
			return Err(err!(
				"HyperLogLog precision p = {} out of range [{}, {}].",
				p, P_MIN, P_MAX;
			Invalid, Input));
		}
		let m = 1usize << p;
		if bytes.len() != m {
			return Err(err!(
				"HyperLogLog for p = {} needs {} register bytes, got {}.",
				p, m, bytes.len();
			Invalid, Input, Size));
		}
		Ok(Self {
			p,
			registers: bytes.to_vec(),
		})
	}

	/// Returns the precision parameter.
	pub fn precision(&self) -> u8 {
		self.p
	}

	/// Returns the number of registers, `m = 2^p`.
	pub fn m(&self) -> usize {
		self.registers.len()
	}

	/// Borrows the register bytes for serialisation.
	pub fn as_bytes(&self) -> &[u8] {
		&self.registers
	}

	/// Incorporates a 64-bit hash into the sketch.
	///
	/// The top `p` bits select the register; the remaining `64 - p` bits are
	/// scanned for the position of the first set bit, which if greater than
	/// the current register value replaces it.
	pub fn add_hash(&mut self, hash: u64) {
		let p = self.p as u32;
		// Register index from the top p bits.
		let idx = (hash >> (64 - p)) as usize;
		// Suffix = low (64 - p) bits shifted into the top of a u64 so that
		// the suffix's MSB sits at bit 63 and its LSB at bit p. The bottom p
		// bits are zero by construction.
		let suffix = hash << p;
		let rho = if suffix == 0 {
			// No set bit in the suffix; the conventional cap.
			(64 - p + 1) as u8
		} else {
			// leading_zeros counts zero bits from bit 63 downward. Since the
			// suffix has at least one set bit in positions p..64, the count
			// sits in [0, 63 - p]; +1 for the 1-based rho convention leaves
			// rho in [1, 64 - p].
			suffix.leading_zeros() as u8 + 1
		};
		if rho > self.registers[idx] {
			self.registers[idx] = rho;
		}
	}

	/// Merges `other` into `self` by register-wise maximum.
	///
	/// Returns an error if the two sketches have different precision.
	pub fn merge(&mut self, other: &Self) -> Outcome<()> {
		if self.p != other.p {
			return Err(err!(
				"Cannot merge HyperLogLog sketches with different precision \
				(self p = {}, other p = {}).",
				self.p, other.p;
			Invalid, Input, Mismatch));
		}
		for (dst, src) in self.registers.iter_mut().zip(other.registers.iter()) {
			if *src > *dst {
				*dst = *src;
			}
		}
		Ok(())
	}

	/// Returns the current cardinality estimate.
	///
	/// The formula is:
	///
	/// - Raw: $E = alpha_m dot m^2 / sum_j 2^(-M_j)$.
	/// - Linear counting for small cardinalities: if $E <= 5/2 dot m$ and at
	///   least one register is zero, return $m dot ln(m / z)$ where $z$ is
	///   the number of zero registers. This is the well-known HLL small-range
	///   correction.
	///
	/// At `p = 14` the expected standard error is approximately
	/// $1.04 / sqrt(m) ≈ 0.008$, i.e. around 0.8%. The spec's 2% target at
	/// $10^6$ peers is comfortably within that.
	pub fn estimate(&self) -> f64 {
		let m = self.registers.len() as f64;
		let alpha = alpha_m(self.registers.len());

		let mut sum = 0.0f64;
		let mut zeros = 0usize;
		for &r in &self.registers {
			if r == 0 {
				zeros += 1;
			}
			// 2^(-r) computed as ldexp(1, -r). For r up to ~65 this is well
			// within f64 precision.
			sum += (-(r as f64)).exp2();
		}
		let raw = alpha * m * m / sum;

		// Linear counting correction for small cardinalities.
		let small_threshold = 2.5f64 * m;
		if raw <= small_threshold && zeros > 0 {
			return m * (m / zeros as f64).ln();
		}
		raw
	}

	/// Convenience wrapper around [`HyperLogLog::estimate`] that rounds to
	/// the nearest non-negative integer.
	pub fn estimate_rounded(&self) -> u64 {
		let e = self.estimate();
		if e < 0.0 {
			0
		} else {
			e.round() as u64
		}
	}

	/// Resets every register to zero without reallocating.
	pub fn clear(&mut self) {
		for r in self.registers.iter_mut() {
			*r = 0;
		}
	}
}

/// The $alpha_m$ bias-correction constant from the original HyperLogLog
/// paper. For $m in {16, 32, 64}$ the constants are tabulated; for larger
/// $m$ the formula $alpha_m = 0.7213 / (1 + 1.079 / m)$ is used.
fn alpha_m(m: usize) -> f64 {
	match m {
		16	=> 0.673,
		32	=> 0.697,
		64	=> 0.709,
		_	=> 0.7213 / (1.0 + 1.079 / m as f64),
	}
}
