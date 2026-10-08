//! Content fingerprints, the 128 bit keys of an incremental cache.
//!
//! A [`Fingerprinter`] is SipHash-1-3 with its 128 bit output under a fixed key (the reference
//! test key, bytes `00` to `0f`), so a fingerprint is the same on every run, platform and release;
//! the pinned vectors in `tests/fingerprint.rs` are the promise. It is not a cryptographic hash.
//! A fingerprint stands for a stream of bytes however the stream was cut into writes, and every
//! integer is written little-endian at its own width, so `usize` is eight bytes on a 32 bit target
//! too.

use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::BitXor;
use std::ops::BitXorAssign;
use std::sync::OnceLock;

// The reference test key of the SipHash paper.
const K0:	u64	= 0x0706_0504_0302_0100;
const K1:	u64	= 0x0f0e_0d0c_0b0a_0908;

/// A 128 bit content fingerprint. Its text form is the 32 hex digits of the value, `hi` first.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fingerprint(u128);

impl Fingerprint {
	pub const fn from_u128(v: u128) -> Self { Self(v) }

	pub const fn as_u128(self) -> u128 { self.0 }

	/// The fingerprint of a byte string.
	pub fn of(bytes: &[u8]) -> Self {
		let mut f = Fingerprinter::new();
		f.write(bytes);
		f.finish()
	}

	/// The halves xored into 64 bits, for a key that must stay a `u64`. A caller that can hold the
	/// whole fingerprint should.
	pub const fn fold(self) -> u64 { (self.0 as u64) ^ ((self.0 >> 64) as u64) }
}

// Xor folds the fingerprints of a set whose order does not matter, each element appearing once.
impl BitXor for Fingerprint {
	type Output = Self;
	fn bitxor(self, rhs: Self) -> Self { Self(self.0 ^ rhs.0) }
}

impl BitXorAssign for Fingerprint {
	fn bitxor_assign(&mut self, rhs: Self) { self.0 ^= rhs.0; }
}

impl fmt::Display for Fingerprint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:032x}", self.0)
	}
}

impl fmt::Debug for Fingerprint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Fingerprint({:032x})", self.0)
	}
}

// The value is already uniform, so one half is the whole hash a map needs.
impl Hash for Fingerprint {
	fn hash<H: Hasher>(&self, state: &mut H) {
		state.write_u64(self.0 as u64);
	}
}

/// A streaming fingerprint builder. A write of an integer is a write of its little-endian bytes,
/// and a float goes by its bit pattern, so `NaN` is not folded to one value.
#[derive(Clone, Debug)]
pub struct Fingerprinter {
	v0:		u64,
	v1:		u64,
	v2:		u64,
	v3:		u64,
	tail:	u64,	// Bytes waiting for a full block, little-endian.
	ntail:	u32,	// How many of them, 0 to 7.
	len:	u64,	// Bytes written.
}

impl Default for Fingerprinter {
	fn default() -> Self { Self::new() }
}

impl Fingerprinter {
	pub fn new() -> Self {
		Self {
			v0:		K0 ^ 0x736f_6d65_7073_6575,
			v1:		K1 ^ 0x646f_7261_6e64_6f6d ^ 0xee,	// The 128 bit output mode.
			v2:		K0 ^ 0x6c79_6765_6e65_7261,
			v3:		K1 ^ 0x7465_6462_7974_6573,
			tail:	0,
			ntail:	0,
			len:	0,
		}
	}

	#[inline(always)]
	fn round(&mut self) {
		self.v0 = self.v0.wrapping_add(self.v1);
		self.v1 = self.v1.rotate_left(13) ^ self.v0;
		self.v0 = self.v0.rotate_left(32);
		self.v2 = self.v2.wrapping_add(self.v3);
		self.v3 = self.v3.rotate_left(16) ^ self.v2;
		self.v0 = self.v0.wrapping_add(self.v3);
		self.v3 = self.v3.rotate_left(21) ^ self.v0;
		self.v2 = self.v2.wrapping_add(self.v1);
		self.v1 = self.v1.rotate_left(17) ^ self.v2;
		self.v2 = self.v2.rotate_left(32);
	}

	#[inline(always)]
	fn block(&mut self, m: u64) {
		self.v3 ^= m;
		self.round();
		self.v0 ^= m;
	}

	// Appends the low `n` bytes of `v`, 1 to 8 of them, the bytes above being zero.
	#[inline(always)]
	fn push(&mut self, v: u64, n: u32) {
		self.len = self.len.wrapping_add(n as u64);
		let have	= self.ntail;
		let t		= self.tail | (v << (8 * have));
		if have + n < 8 {
			self.tail	= t;
			self.ntail	= have + n;
			return;
		}
		self.block(t);
		let used	= 8 - have;
		self.ntail	= n - used;
		self.tail	= if used < 8 { v >> (8 * used) } else { 0 };
	}

	pub fn write_u8(&mut self, v: u8)		{ self.push(v as u64, 1); }
	pub fn write_u16(&mut self, v: u16)		{ self.push(v as u64, 2); }
	pub fn write_u32(&mut self, v: u32)		{ self.push(v as u64, 4); }
	pub fn write_i32(&mut self, v: i32)		{ self.write_u32(v as u32); }
	pub fn write_i64(&mut self, v: i64)		{ self.write_u64(v as u64); }
	pub fn write_usize(&mut self, v: usize)	{ self.write_u64(v as u64); }
	pub fn write_bool(&mut self, v: bool)	{ self.push(v as u64, 1); }
	pub fn write_f32(&mut self, v: f32)		{ self.write_u32(v.to_bits()); }
	pub fn write_f64(&mut self, v: f64)		{ self.write_u64(v.to_bits()); }

	pub fn write_u64(&mut self, v: u64) {
		if self.ntail == 0 {
			self.len = self.len.wrapping_add(8);
			self.block(v);
		} else {
			self.push(v, 8);
		}
	}

	pub fn write(&mut self, bytes: &[u8]) {
		let mut rest = bytes;
		if rest.is_empty() {
			return;
		}
		if self.ntail != 0 {
			let n		= ((8 - self.ntail) as usize).min(rest.len());
			let (a, b)	= rest.split_at(n);
			self.push(le_word(a), n as u32);
			rest		= b;
		}
		let mut chunks = rest.chunks_exact(8);
		for c in &mut chunks {
			self.len = self.len.wrapping_add(8);
			self.block(le_word(c));
		}
		let more = chunks.remainder();
		if !more.is_empty() {
			self.push(le_word(more), more.len() as u32);
		}
	}

	/// Writes the length, then the bytes, so "ab" then "c" differs from "a" then "bc".
	pub fn write_str(&mut self, s: &str) {
		self.write_u64(s.len() as u64);
		self.write(s.as_bytes());
	}

	/// Writes the 16 bytes of a fingerprint that stands for a part already hashed.
	pub fn write_fingerprint(&mut self, fp: Fingerprint) {
		self.write_u64(fp.0 as u64);
		self.write_u64((fp.0 >> 64) as u64);
	}

	/// Hashes what `f` writes on its own, then writes that fingerprint here. A part is then the
	/// same 16 bytes to its parent however long it is, and can be hashed once and reused.
	pub fn nest<F: FnOnce(&mut Fingerprinter)>(&mut self, f: F) {
		let mut part = Fingerprinter::new();
		f(&mut part);
		self.write_fingerprint(part.finish());
	}

	/// The fingerprint of everything written so far. The builder is not consumed, so it can go on.
	pub fn finish(&self) -> Fingerprint {
		let mut s = self.clone();
		s.block((s.len << 56) | s.tail);
		s.v2 ^= 0xee;
		s.round();
		s.round();
		s.round();
		let lo = s.v0 ^ s.v1 ^ s.v2 ^ s.v3;
		s.v1 ^= 0xdd;
		s.round();
		s.round();
		s.round();
		let hi = s.v0 ^ s.v1 ^ s.v2 ^ s.v3;
		Fingerprint(((hi as u128) << 64) | (lo as u128))
	}
}

// Up to eight bytes as one little-endian word.
#[inline(always)]
fn le_word(b: &[u8]) -> u64 {
	let mut w = 0u64;
	for (i, x) in b.iter().enumerate() {
		w |= (*x as u64) << (8 * i);
	}
	w
}

/// A fingerprint worked out on first use and kept. It is a cache beside a value, not a part of
/// it, so cloning gives an empty cell, and it takes no part in equality, hashing or debug output
/// (which would otherwise differ with whether the fingerprint had been read). A value that is
/// cloned and then changed therefore never reads the fingerprint of what it was.
#[derive(Default)]
pub struct LazyFingerprint {
	cell: OnceLock<Fingerprint>,
}

impl LazyFingerprint {
	pub const fn new() -> Self { Self { cell: OnceLock::new() } }

	pub fn get(&self) -> Option<Fingerprint> { self.cell.get().copied() }

	/// The kept fingerprint, worked out by `f` if there is none yet.
	pub fn get_or_init<F: FnOnce() -> Fingerprint>(&self, f: F) -> Fingerprint {
		*self.cell.get_or_init(f)
	}

	/// Forgets the fingerprint. A value changed in place must call this.
	pub fn clear(&mut self) { self.cell.take(); }
}

impl fmt::Debug for LazyFingerprint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("LazyFingerprint") }
}

impl Clone for LazyFingerprint {
	fn clone(&self) -> Self { Self::new() }
}

impl PartialEq for LazyFingerprint {
	fn eq(&self, _: &Self) -> bool { true }
}

impl Eq for LazyFingerprint {}

impl Hash for LazyFingerprint {
	fn hash<H: Hasher>(&self, _: &mut H) {}
}
