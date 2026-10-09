// A location is a hash of (where the realisation stands, element kind, the fingerprint of the element's
// content, ordinal among the equal ones there). No source position enters it, so the same element keeps its
// location from pass to pass and across an edit elsewhere in the source; an edit changes the location of the
// edited element and of the elements that hold it, and of nothing else.
//
// Where the realisation stands is a [`Place`], Typst's `Locator`: a position in the tree of layouts, hashed
// from the places above it. An element that lays a body of its own out is given its place when it is
// realised, as Typst gives a child its sublocator, and every layout of its body, a measurement or a second
// attempt in a region a footnote has shortened, starts afresh at that place. The same body then realises to
// the same locations however often it is laid out, as Typst's `Locator::relayout` has it, and a footnote
// within a block is placed once, not once for each layout of the block.
//
// A place is keyed by the element's shell (its kind and the fields that hold no content) and its ordinal
// among the elements of that shell at the place above, never by what the element holds. Typing into a
// paragraph then moves the paragraph's own location and those of the elements that hold it, and leaves the
// place of the paragraph, so every located element inside it keeps its location and an introspector from
// the pass before still answers the questions those elements ask. Inserting or deleting a container of
// the same shell renumbers the places of the containers after it, a known limit that an alias from the
// renumbered place to the one the introspector recorded would remove.

use crate::eval::content::ElemKind;
use crate::ledger::{
	AnchorId,
	AnchorKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
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
/// ordinals count the elements of one kind and one fingerprint (the content's for a location, the shell's for
/// a place) within the place, so another realisation at the same place, begun afresh with [`Locator::new`],
/// hands out the same ones.
#[derive(Clone, Debug, Default)]
pub struct Locator {
	place:	Place,
	seen:	HashMap<u128, u32>,
}

impl Locator {
	/// A locator for a realisation at `place`, counting from nothing.
	pub fn new(place: Place) -> Self { Self { place, seen: HashMap::new() } }

	pub fn reset(&mut self) { *self = Self::default(); }

	pub fn place(&self) -> Place { self.place }

	/// The place and the ordinals handed out so far, sorted, for the flow's carry fingerprint: how many
	/// were handed out and how often, never which.
	pub fn carry(&self) -> (Place, Vec<u32>) {
		let mut counts: Vec<u32> = self.seen.values().copied().collect();
		counts.sort_unstable();
		(self.place, counts)
	}

	/// The location of an element of `kind` whose content has the fingerprint `fp`, met in this realisation.
	pub fn locate(&mut self, kind: ElemKind, fp: Fingerprint) -> Location {
		Location(self.next_hash(DOMAIN_LOCATION, kind, fp))
	}

	/// The place of an element met in this realisation, for a body it lays out later: Typst's sublocator. The
	/// element is told by its `shell`, which leaves out what it holds, and by its ordinal among the same
	/// shell here.
	pub fn next(&mut self, kind: ElemKind, shell: Fingerprint) -> Place {
		Place(self.next_hash(DOMAIN_PLACE, kind, shell))
	}

	fn next_hash(&mut self, domain: u64, kind: ElemKind, fp: Fingerprint) -> u64 {
		let mut h = Fingerprinter::new();
		h.write_u64(domain);
		h.write_u64(self.place.0);
		h.write_u64(kind as u64);
		h.write_fingerprint(fp);
		let base = h.finish();
		let n = self.seen.entry(base.as_u128()).or_insert(0);
		let mut h = Fingerprinter::new();
		h.write_fingerprint(base);
		h.write_u64(*n as u64);
		*n += 1;
		h.finish().fold()
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
