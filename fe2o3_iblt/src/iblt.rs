//! The Invertible Bloom Lookup Table.
//!
//! An [`Iblt`] holds `num_cells` cells, each consisting of four accumulators:
//! a key XOR, a value XOR, a 64-bit key-hash XOR used as a purity check and a
//! signed insert count. Every key is inserted into `num_hashes` cells chosen
//! by double-hashing. Symmetric difference between two IBLTs with the same
//! shape is computed by cellwise XOR on the byte accumulators and cellwise
//! subtraction on the counts; the peeling decoder then extracts keys from
//! "pure" cells -- those with `|count| == 1` and a key-hash fingerprint that
//! matches the recomputed hash of the extracted key -- and removes their
//! contributions from all cells, iterating until no pure cells remain.

use crate::hash::{
	hash_bytes,
	hash_pair,
};

use oxedyne_fe2o3_core::prelude::*;


/// The fixed number of bytes used for the purity-check fingerprint.
pub const FINGERPRINT_LEN: usize = 8;

/// The fixed number of bytes used for the signed count accumulator.
pub const COUNT_LEN: usize = 4;


/// Parameters shared by all cells of an [`Iblt`]. Returned by
/// [`Iblt::config`] and consumed by [`Iblt::from_bytes`] to restore an IBLT
/// from a serialised form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IbltConfig {
	/// Number of cells in the table.
	pub num_cells:	usize,
	/// Number of hash-derived cell indices each key maps to.
	pub num_hashes:	usize,
	/// Fixed key length in bytes.
	pub key_len:	usize,
	/// Fixed value length in bytes. Zero for key-only IBLTs.
	pub value_len:	usize,
	/// Seed shared by peers that intend to reconcile with one another.
	pub seed:		u64,
}


/// The outcome of a peeling decode.
#[derive(Clone, Debug)]
pub enum DecodeOutcome {
	/// Every cell was drained. `inserted` and `deleted` are the two halves of
	/// the symmetric difference between the IBLTs that produced the decoded
	/// table.
	Complete {
		/// Keys that had `count > 0` at extraction time, paired with their
		/// recovered values.
		inserted:	Vec<(Vec<u8>, Vec<u8>)>,
		/// Keys that had `count < 0` at extraction time, paired with their
		/// recovered values.
		deleted:	Vec<(Vec<u8>, Vec<u8>)>,
	},
	/// Decoding halted with non-empty cells remaining -- the IBLT was
	/// overloaded relative to the true symmetric-difference size. The partial
	/// results before the halt are preserved; callers can either fall back to
	/// a bulk transfer or retry with a larger IBLT.
	Incomplete {
		/// Keys extracted before decoding stalled.
		inserted:			Vec<(Vec<u8>, Vec<u8>)>,
		/// Keys extracted before decoding stalled.
		deleted:			Vec<(Vec<u8>, Vec<u8>)>,
		/// Number of cells still containing non-trivial state.
		remaining_cells:	usize,
	},
}


/// An Invertible Bloom Lookup Table over fixed-length keys and values.
#[derive(Clone, Debug)]
pub struct Iblt {
	cfg:			IbltConfig,
	/// XOR accumulator of inserted keys, `key_len` bytes per cell, flattened.
	key_xor:		Vec<u8>,
	/// XOR accumulator of inserted values, `value_len` bytes per cell,
	/// flattened. Empty when `value_len == 0`.
	value_xor:		Vec<u8>,
	/// 64-bit key-hash XOR for the purity check, one entry per cell.
	fp_xor:			Vec<u64>,
	/// Signed insert count per cell.
	count:			Vec<i32>,
}

impl Iblt {
	/// Constructs an empty IBLT with the given configuration.
	pub fn new(cfg: IbltConfig) -> Outcome<Self> {
		if cfg.num_cells == 0 {
			return Err(err!(
				"IBLT num_cells must be greater than zero.";
			Invalid, Input));
		}
		if cfg.num_hashes == 0 {
			return Err(err!(
				"IBLT num_hashes must be greater than zero.";
			Invalid, Input));
		}
		if cfg.num_hashes > cfg.num_cells {
			return Err(err!(
				"IBLT num_hashes ({}) cannot exceed num_cells ({}).",
				cfg.num_hashes, cfg.num_cells;
			Invalid, Input));
		}
		if cfg.key_len == 0 {
			return Err(err!(
				"IBLT key_len must be greater than zero.";
			Invalid, Input));
		}
		Ok(Self {
			cfg,
			key_xor:	vec![0u8; cfg.num_cells * cfg.key_len],
			value_xor:	vec![0u8; cfg.num_cells * cfg.value_len],
			fp_xor:		vec![0u64; cfg.num_cells],
			count:		vec![0i32; cfg.num_cells],
		})
	}

	/// Returns the shared configuration.
	pub fn config(&self) -> IbltConfig {
		self.cfg
	}

	/// Inserts `(key, value)`, or re-inserts with the same sign if already
	/// present. Length mismatches are errors.
	pub fn insert(&mut self, key: &[u8], value: &[u8]) -> Outcome<()> {
		self.apply(key, value, 1)
	}

	/// Records a deletion of `(key, value)`. After subtraction with another
	/// IBLT this is what distinguishes "B has an extra copy" from "A has an
	/// extra copy" during decoding.
	pub fn delete(&mut self, key: &[u8], value: &[u8]) -> Outcome<()> {
		self.apply(key, value, -1)
	}

	/// Subtracts another IBLT in place. Both IBLTs must share the same
	/// [`IbltConfig`]; mismatches are errors.
	pub fn subtract(&mut self, other: &Self) -> Outcome<()> {
		if self.cfg != other.cfg {
			return Err(err!(
				"IBLT subtract requires matching configuration.";
			Invalid, Input, Mismatch));
		}
		for (dst, src) in self.key_xor.iter_mut().zip(other.key_xor.iter()) {
			*dst ^= *src;
		}
		for (dst, src) in self.value_xor.iter_mut().zip(other.value_xor.iter()) {
			*dst ^= *src;
		}
		for (dst, src) in self.fp_xor.iter_mut().zip(other.fp_xor.iter()) {
			*dst ^= *src;
		}
		for (dst, src) in self.count.iter_mut().zip(other.count.iter()) {
			*dst = dst.wrapping_sub(*src);
		}
		Ok(())
	}

	/// Runs the peeling decoder, draining the IBLT of every entry it can
	/// recover.
	///
	/// Mutates the IBLT in place: on return the cells that contributed to a
	/// recovered entry have been reduced, and the remaining cells (if any)
	/// are those that could not be peeled.
	pub fn decode(&mut self) -> Outcome<DecodeOutcome> {
		let mut inserted: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
		let mut deleted: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

		// Queue of cell indices that may currently be pure. We re-enqueue
		// the `num_hashes` cells touched by each extraction.
		let mut queue: Vec<usize> = (0..self.cfg.num_cells).collect();

		while let Some(idx) = queue.pop() {
			if !self.cell_is_pure(idx) {
				continue;
			}
			let (sign, key, value) = res!(self.extract_cell(idx));
			// Remove this entry from every cell it affects.
			let cell_idxs = self.cells_for(&key);
			for &ci in &cell_idxs {
				self.apply_at(ci, &key, &value, -sign);
			}
			// Any of those cells might now be pure.
			for ci in cell_idxs {
				queue.push(ci);
			}
			if sign > 0 {
				inserted.push((key, value));
			} else {
				deleted.push((key, value));
			}
		}

		// Count residual cells -- any cell still holding state is a failure.
		let mut remaining = 0usize;
		for i in 0..self.cfg.num_cells {
			if !self.cell_is_empty(i) {
				remaining += 1;
			}
		}
		Ok(if remaining == 0 {
			DecodeOutcome::Complete { inserted, deleted }
		} else {
			DecodeOutcome::Incomplete {
				inserted,
				deleted,
				remaining_cells: remaining,
			}
		})
	}

	/// Returns `true` if every cell is at its identity state (no keys, no
	/// values, zero fingerprint, zero count).
	pub fn is_empty(&self) -> bool {
		self.count.iter().all(|&c| c == 0)
			&& self.fp_xor.iter().all(|&f| f == 0)
			&& self.key_xor.iter().all(|&b| b == 0)
			&& self.value_xor.iter().all(|&b| b == 0)
	}

	/// Serialises the IBLT into a compact byte buffer.
	///
	/// Format:
	///
	/// - 5 × u64 little-endian: `num_cells`, `num_hashes`, `key_len`,
	///   `value_len`, `seed`.
	/// - `num_cells × (key_len + value_len + 8 + 4)` bytes: per-cell
	///   `key_xor || value_xor || fp_xor_le || count_le`.
	pub fn to_bytes(&self) -> Vec<u8> {
		let per_cell = self.cfg.key_len + self.cfg.value_len
			+ FINGERPRINT_LEN + COUNT_LEN;
		let mut out = Vec::with_capacity(8 * 5 + per_cell * self.cfg.num_cells);
		out.extend_from_slice(&(self.cfg.num_cells as u64).to_le_bytes());
		out.extend_from_slice(&(self.cfg.num_hashes as u64).to_le_bytes());
		out.extend_from_slice(&(self.cfg.key_len as u64).to_le_bytes());
		out.extend_from_slice(&(self.cfg.value_len as u64).to_le_bytes());
		out.extend_from_slice(&self.cfg.seed.to_le_bytes());
		for i in 0..self.cfg.num_cells {
			let kr = self.key_range(i);
			out.extend_from_slice(&self.key_xor[kr]);
			let vr = self.value_range(i);
			out.extend_from_slice(&self.value_xor[vr]);
			out.extend_from_slice(&self.fp_xor[i].to_le_bytes());
			out.extend_from_slice(&self.count[i].to_le_bytes());
		}
		out
	}

	/// Parses the serialised form produced by [`Iblt::to_bytes`].
	pub fn from_bytes(bytes: &[u8]) -> Outcome<Self> {
		if bytes.len() < 8 * 5 {
			return Err(err!(
				"IBLT serialised form too short: {} bytes.", bytes.len();
			Invalid, Input, Size));
		}
		let read_u64 = |off: usize| -> u64 {
			let mut buf = [0u8; 8];
			buf.copy_from_slice(&bytes[off..off + 8]);
			u64::from_le_bytes(buf)
		};
		let num_cells	= read_u64(0)	as usize;
		let num_hashes	= read_u64(8)	as usize;
		let key_len		= read_u64(16)	as usize;
		let value_len	= read_u64(24)	as usize;
		let seed		= read_u64(32);
		let cfg = IbltConfig { num_cells, num_hashes, key_len, value_len, seed };

		let per_cell = key_len + value_len + FINGERPRINT_LEN + COUNT_LEN;
		let body_len = res!(num_cells.checked_mul(per_cell).ok_or_else(|| err!(
			"IBLT dimensions overflow: num_cells * per_cell.";
		Invalid, Input, Size)));
		let expected = 40 + body_len;
		if bytes.len() != expected {
			return Err(err!(
				"IBLT serialised form length mismatch: got {}, expected {}.",
				bytes.len(), expected;
			Invalid, Input, Size));
		}

		let mut iblt = res!(Self::new(cfg));
		let mut off = 40;
		for i in 0..num_cells {
			let kr = iblt.key_range(i);
			iblt.key_xor[kr].copy_from_slice(&bytes[off..off + key_len]);
			off += key_len;
			let vr = iblt.value_range(i);
			iblt.value_xor[vr].copy_from_slice(&bytes[off..off + value_len]);
			off += value_len;
			let mut fp_buf = [0u8; 8];
			fp_buf.copy_from_slice(&bytes[off..off + FINGERPRINT_LEN]);
			iblt.fp_xor[i] = u64::from_le_bytes(fp_buf);
			off += FINGERPRINT_LEN;
			let mut c_buf = [0u8; 4];
			c_buf.copy_from_slice(&bytes[off..off + COUNT_LEN]);
			iblt.count[i] = i32::from_le_bytes(c_buf);
			off += COUNT_LEN;
		}
		Ok(iblt)
	}

	// --- internals ----------------------------------------------------

	fn apply(&mut self, key: &[u8], value: &[u8], sign: i32) -> Outcome<()> {
		if key.len() != self.cfg.key_len {
			return Err(err!(
				"IBLT key length mismatch: got {}, expected {}.",
				key.len(), self.cfg.key_len;
			Invalid, Input, Size));
		}
		if value.len() != self.cfg.value_len {
			return Err(err!(
				"IBLT value length mismatch: got {}, expected {}.",
				value.len(), self.cfg.value_len;
			Invalid, Input, Size));
		}
		let cells = self.cells_for(key);
		for ci in cells {
			self.apply_at(ci, key, value, sign);
		}
		Ok(())
	}

	/// Applies `(sign × (key, value))` to a specific cell without re-deriving
	/// the target cells. Used both by the public insert/delete paths (which
	/// walk every target cell) and by the decoder (which removes a recovered
	/// entry from the cells it affected).
	fn apply_at(&mut self, ci: usize, key: &[u8], value: &[u8], sign: i32) {
		let kr = self.key_range(ci);
		for (dst, src) in self.key_xor[kr].iter_mut().zip(key.iter()) {
			*dst ^= *src;
		}
		let vr = self.value_range(ci);
		for (dst, src) in self.value_xor[vr].iter_mut().zip(value.iter()) {
			*dst ^= *src;
		}
		self.fp_xor[ci] ^= self.fingerprint(key);
		self.count[ci] = self.count[ci].wrapping_add(sign);
	}

	fn cells_for(&self, key: &[u8]) -> Vec<usize> {
		let (h1, h2) = hash_pair(key, self.cfg.seed);
		let m = self.cfg.num_cells as u64;
		let mut out = Vec::with_capacity(self.cfg.num_hashes);
		// Double hashing. To keep the k hashes truly distinct even when h2 is
		// a factor of m (unlikely for typical seeds but possible), guard with
		// a linear-probe fallback that advances by one cell until a fresh
		// index is found. This preserves correctness for pathological seeds
		// without distorting typical behaviour.
		for i in 0..self.cfg.num_hashes {
			let base = h1.wrapping_add((i as u64).wrapping_mul(h2));
			let mut idx = (base % m) as usize;
			while out.contains(&idx) {
				idx = (idx + 1) % self.cfg.num_cells;
			}
			out.push(idx);
		}
		out
	}

	fn fingerprint(&self, key: &[u8]) -> u64 {
		hash_bytes(key, self.cfg.seed ^ 0xc2b2_ae3d_27d4_eb4f)
	}

	fn cell_is_pure(&self, ci: usize) -> bool {
		let c = self.count[ci];
		if c != 1 && c != -1 {
			return false;
		}
		let kr = self.key_range(ci);
		let key = &self.key_xor[kr];
		self.fingerprint(key) == self.fp_xor[ci]
	}

	fn cell_is_empty(&self, ci: usize) -> bool {
		if self.count[ci] != 0 {
			return false;
		}
		if self.fp_xor[ci] != 0 {
			return false;
		}
		let kr = self.key_range(ci);
		if self.key_xor[kr].iter().any(|&b| b != 0) {
			return false;
		}
		let vr = self.value_range(ci);
		if self.value_xor[vr].iter().any(|&b| b != 0) {
			return false;
		}
		true
	}

	fn extract_cell(&self, ci: usize) -> Outcome<(i32, Vec<u8>, Vec<u8>)> {
		let sign = self.count[ci];
		if sign != 1 && sign != -1 {
			return Err(err!(
				"IBLT cell {} is not pure (count = {}).", ci, sign;
			Invalid, Input, Bug));
		}
		let kr = self.key_range(ci);
		let key = self.key_xor[kr].to_vec();
		let vr = self.value_range(ci);
		let value = self.value_xor[vr].to_vec();
		Ok((sign, key, value))
	}

	fn key_range(&self, ci: usize) -> std::ops::Range<usize> {
		let start = ci * self.cfg.key_len;
		start..start + self.cfg.key_len
	}

	fn value_range(&self, ci: usize) -> std::ops::Range<usize> {
		let start = ci * self.cfg.value_len;
		start..start + self.cfg.value_len
	}
}
