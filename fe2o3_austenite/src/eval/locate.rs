// U4 owns this file. A location is a hash of (element kind, span, ordinal among equal pairs), so the same
// element keeps its location from pass to pass; U0 wrote the hash so U8 can build against real locations.

use crate::eval::content::ElemKind;
use crate::ledger::{
	AnchorId,
	AnchorKind,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location(pub u64);

impl Location {
	/// The ledger anchor that records where this location landed.
	pub fn anchor(self) -> AnchorId { AnchorId::new(AnchorKind::Location, fmt!("{:016x}", self.0)) }
}

/// Hands out locations during one realisation; `reset` before each pass.
#[derive(Clone, Debug, Default)]
pub struct Locator {
	seen:	HashMap<u64, u32>,
}

impl Locator {
	pub fn reset(&mut self) { self.seen.clear(); }

	pub fn locate(&mut self, kind: ElemKind, span: Span) -> Location {
		let base = fnv(&[kind as u64, span.file.0 as u64, span.start as u64, span.end as u64]);
		let n = self.seen.entry(base).or_insert(0);
		let loc = Location(fnv(&[base, *n as u64]));
		*n += 1;
		loc
	}
}

fn fnv(words: &[u64]) -> u64 {
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for w in words {
		for b in w.to_le_bytes() {
			h ^= b as u64;
			h = h.wrapping_mul(0x0000_0100_0000_01b3);
		}
	}
	h
}
