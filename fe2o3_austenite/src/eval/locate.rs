// U4 owns this file. A location is a hash of (where the realisation stands, element kind, span, ordinal among
// equal pairs), so the same element keeps its location from pass to pass; U0 wrote the hash so U8 can build
// against real locations.
//
// Where the realisation stands is a [`Place`], Typst's `Locator`: a position in the tree of layouts, hashed
// from the places above it. An element that lays a body of its own out is given its place when it is
// realised, as Typst gives a child its sublocator, and every layout of its body, a measurement or a second
// attempt in a region a footnote has shortened, starts afresh at that place. The same body then realises to
// the same locations however often it is laid out, as Typst's `Locator::relayout` has it, and a footnote
// within a block is placed once, not once for each layout of the block.

use crate::eval::content::ElemKind;
use crate::ledger::{
	AnchorId,
	AnchorKind,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location(pub u64);

impl Location {
	/// The ledger anchor that records where this location landed.
	pub fn anchor(self) -> AnchorId { AnchorId::new(AnchorKind::Location, fmt!("{:016x}", self.0)) }
}

/// A position in the tree of layouts: what a body is realised under. The root, where the document's own
/// level is realised, is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Place(pub u64);

impl Place {
	/// The place for content laid out on behalf of a located element, as Typst's `Locator::synthesize`: a
	/// footnote's entry, read after the note.
	pub fn synthesise(loc: Location) -> Self { Place(fnv(&[DOMAIN_SYNTH, loc.0])) }
}

// What a hash is for, so a location, a place and a synthesised place of one key never meet.
const DOMAIN_LOCATION:	u64	= 1;
const DOMAIN_PLACE:		u64	= 2;
const DOMAIN_SYNTH:		u64	= 3;

/// Hands out locations and places during one realisation, at one place; `reset` before each pass. The
/// ordinals count within the place, so another realisation at the same place, begun afresh with
/// [`Locator::new`], hands out the same ones.
#[derive(Clone, Debug, Default)]
pub struct Locator {
	place:	Place,
	seen:	HashMap<u64, u32>,
}

impl Locator {
	/// A locator for a realisation at `place`, counting from nothing.
	pub fn new(place: Place) -> Self { Self { place, seen: HashMap::new() } }

	pub fn reset(&mut self) { *self = Self::default(); }

	pub fn locate(&mut self, kind: ElemKind, span: Span) -> Location {
		Location(self.next_hash(DOMAIN_LOCATION, kind, span))
	}

	/// The place of an element met in this realisation, for a body it lays out later: Typst's sublocator.
	pub fn next(&mut self, kind: ElemKind, span: Span) -> Place {
		Place(self.next_hash(DOMAIN_PLACE, kind, span))
	}

	fn next_hash(&mut self, domain: u64, kind: ElemKind, span: Span) -> u64 {
		let base = fnv(&[domain, self.place.0, kind as u64, span.file.0 as u64, span.start as u64, span.end as u64]);
		let n = self.seen.entry(base).or_insert(0);
		let h = fnv(&[base, *n as u64]);
		*n += 1;
		h
	}
}

// Folds the words to the 64 bits a location and a place hold.
fn fnv(words: &[u64]) -> u64 {
	let mut h = Fingerprinter::new();
	for w in words {
		h.write_u64(*w);
	}
	h.finish().fold()
}
