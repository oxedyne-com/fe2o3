//! Lookup of a character property in a generated partition table.
//!
//! Each table is a partition of the code point space: a sorted array of the code point at which
//! each run begins, and a parallel array of the value that run takes. Every table begins at
//! U+0000, so a binary search of the starts always lands, and the lookup never has a failing path.

use oxedyne_fe2o3_core::prelude::*;

/// A character property held in a generated partition table.
pub trait Partitioned: Copy + Sized + 'static {

	/// The value a code point outside the table would take. The generated tables cover the whole
	/// code point space, so this only keeps the lookup total.
	const DEFAULT: Self;

	/// The run starts and run values of the property.
	fn table() -> (&'static [u32], &'static [Self]);

	/// Returns the property value of `c`.
	fn of(c: char) -> Self {
		let (starts, vals) = Self::table();
		match vals.get(run(starts, c)) {
			Some(v) => *v,
			None => Self::DEFAULT,
		}
	}
}

/// Returns the index of the run containing `c`.
pub fn run(starts: &[u32], c: char) -> usize {
	starts.partition_point(|s| *s <= (c as u32)).saturating_sub(1)
}

/// Returns the byte a partition table of flags gives to `c`, or zero if the table does not reach
/// it.
pub fn flags(starts: &[u32], vals: &[u8], c: char) -> u8 {
	match vals.get(run(starts, c)) {
		Some(v) => *v,
		None => 0,
	}
}

/// Returns the position of `c` in a sorted key array, if it is there.
pub fn find(keys: &[u32], c: char) -> Option<usize> {
	match keys.binary_search(&(c as u32)) {
		Ok(i) => Some(i),
		Err(_) => None,
	}
}

/// Returns the half open slice `a..b` of a character pool. A generated table cannot ask for a
/// slice that is not there, and the empty fallback keeps the caller free of a failing path; the
/// `tables_are_consistent` test guards the invariant.
pub fn pool(pool: &[char], a: usize, b: usize) -> &[char] {
	match pool.get(a..b) {
		Some(s) => s,
		None => &[],
	}
}

/// Returns the element of a static table at `i`, or `dflt` if the index is past the end, which a
/// generated table cannot produce.
pub fn get<T: Copy>(vals: &[T], i: usize, dflt: T) -> T {
	match vals.get(i) {
		Some(v) => *v,
		None => dflt,
	}
}
