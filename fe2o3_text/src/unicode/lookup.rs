//! Lookup of a character property in a generated partition table.
//!
//! Each table is a partition of the code point space: a sorted array of the code point at which
//! each run begins, and a parallel array of the value that run takes. Every table begins at
//! U+0000, so a binary search of the starts always lands, and the lookup never has a failing path.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

/// A character property held in a generated partition table.
pub trait Partitioned: Copy + Sized + 'static {

	// The generated tables cover the whole code point space, so this value is
	// never taken; it is here only to keep the lookup total.
	const DEFAULT: Self;

	/// The run starts and run values of the property.
	fn table() -> (&'static [u32], &'static [Self]);

	fn of(c: char) -> Self {
		let (starts, vals) = Self::table();
		match vals.get(run(starts, c)) {
			Some(v) => *v,
			None => Self::DEFAULT,
		}
	}
}

pub fn run(starts: &[u32], c: char) -> usize {
	starts.partition_point(|s| *s <= (c as u32)).saturating_sub(1)
}

/// Zero where the table does not reach `c`.
pub fn flags(starts: &[u32], vals: &[u8], c: char) -> u8 {
	match vals.get(run(starts, c)) {
		Some(v) => *v,
		None => 0,
	}
}

pub fn find(keys: &[u32], c: char) -> Option<usize> {
	match keys.binary_search(&(c as u32)) {
		Ok(i) => Some(i),
		Err(_) => None,
	}
}

/// The half open slice `a..b`. A generated table cannot ask for a slice that is not there, so the
/// empty fallback keeps the caller free of a failing path; the `tables_are_consistent` test guards
/// the invariant.
pub fn pool(pool: &[char], a: usize, b: usize) -> &[char] {
	match pool.get(a..b) {
		Some(s) => s,
		None => &[],
	}
}

pub fn get<T: Copy>(vals: &[T], i: usize, dflt: T) -> T {
	match vals.get(i) {
		Some(v) => *v,
		None => dflt,
	}
}
