//! The fallback chain: a face, and the faces to fall back to for what it cannot draw.
//!
//! See the crate note on why a font is a chain and not a file. This module is what keeps the face
//! at the head of the chain swappable without the swap costing coverage.

use crate::face::{
	Face,
	Metrics,
};
use crate::shape::{
	Dir,
	Glyph,
	Run,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::prelude::*;
use oxedyne_fe2o3_text::unicode::norm::combining_class;

/// Whether a character takes the face of what surrounds it rather than choosing its own. A SPACE is
/// in every face, so asking would cut every run at every word; a COMBINING MARK is positioned by the
/// face that drew its base, so an accent drawn by a face that never saw that base floats. The mark
/// test is the canonical combining class -- not every mark, but every one meant to be placed against
/// something else.
fn neutral(ch: char) -> bool {
	ch.is_whitespace() || combining_class(ch) != 0
}

/// A stretch of a string that one face draws the whole of.
#[derive(Clone, Copy, Debug)]
struct Seg {
	face:	u8,	// which face in the chain draws it
	start:	usize,	// where it starts in the string, bytes
	end:	usize,	// where it ends, bytes
}

/// What the engine draws with: a face chosen for how it reads, and the faces to fall back to for
/// what it lacks. See the crate note on why a font is a chain, not a file.
pub struct Font {
	faces:	Vec<Face>,	// in the order they are tried; never empty
}

impl Font {

	/// A font of one face, which falls back to nothing.
	pub fn new(bytes: Vec<u8>) -> Outcome<Self> {
		Ok(Self {
			faces: vec![res!(Face::new(bytes))],
		})
	}

	/// A font of a face and the faces behind it, in the order they are to be tried.
	pub fn chain(faces: Vec<Face>) -> Outcome<Self> {
		if faces.is_empty() {
			return Err(err!(
				"A font is a chain of at least one face, and this chain is empty.";
			Invalid, Input, Missing));
		}
		if faces.len() > (u8::MAX as usize) + 1 {
			return Err(err!(
				"A chain of {} faces cannot be drawn: a glyph remembers which face drew it in one \
				byte.", faces.len();
			Invalid, Input, TooBig));
		}
		Ok(Self {
			faces,
		})
	}

	/// The face at the head of the chain: the one the reader actually reads.
	fn first(&self) -> Outcome<&Face> {
		match self.faces.first() {
			Some(face) => Ok(face),
			None => Err(err!("A font's chain of faces is empty, which its constructors refuse."; Bug)),
		}
	}

	/// The face at a place in the chain.
	fn face(&self, i: u8) -> Outcome<&Face> {
		match self.faces.get(i as usize) {
			Some(face) => Ok(face),
			None => Err(err!(
				"A glyph names face {} of a chain of {}.", i, self.faces.len(); Bug)),
		}
	}

	/// The vertical metrics at a size, in pixels. They are the FIRST face's, never the tallest used:
	/// a line's height must not change because one arrow in it came from further down the chain. The
	/// faces behind the first are chosen to sit within its box.
	pub fn metrics(&self, size: f32) -> Outcome<Metrics> {
		res!(self.first()).metrics(size)
	}

	/// Which face draws a character: the first in the chain that can. One no face has is left with the
	/// first, which draws its own "not defined" glyph -- the reader is told something is missing.
	fn pick(&self, ch: char) -> u8 {
		for (i, face) in self.faces.iter().enumerate() {
			if face.covers(ch) {
				return i as u8;
			}
		}
		0
	}

	/// Cuts a string into the stretches each face draws. Nearly every character asks the chain who
	/// draws it; the NEUTRAL ones (see [`neutral`]) take the face already in hand if it covers them at
	/// all, so a space does not end the stretch either side of it and cut a line of Arabic into words.
	/// Stickiness goes no further: a face kept for what it merely HAPPENS to cover would drag the rest
	/// of a sentence into the fallback over one arrow, and the reader would see the typeface change
	/// mid-line for no reason.
	fn segment(&self, text: &str) -> Vec<Seg> {
		let mut segs: Vec<Seg> = Vec::new();
		let mut cur: Option<(u8, usize)> = None;
		for (i, ch) in text.char_indices() {
			let held = match cur {
				Some((f, _))	=> neutral(ch)
					&& self.faces.get(f as usize).map_or(false, |x| x.covers(ch)),
				None		=> false,
			};
			let face = match cur {
				Some((f, _)) if held	=> f,
				_			=> self.pick(ch),
			};
			match cur {
				Some((f, _)) if f == face	=> {},
				Some((f, s))			=> {
					segs.push(Seg { face: f, start: s, end: i });
					cur = Some((face, i));
				},
				None				=> cur = Some((face, i)),
			}
		}
		if let Some((f, s)) = cur {
			segs.push(Seg { face: f, start: s, end: text.len() });
		}
		segs
	}

	/// Shapes a string, each face in the chain drawing what the one before it could not. The common
	/// case by far is a string one face draws the whole of: one shaping call.
	pub fn shape(&self, text: &str, size: f32, dir: Dir) -> Outcome<Run> {
		if text.is_empty() {
			return Ok(Run {
				glyphs:		Vec::new(),
				advance:	0.0,
				size,
			});
		}
		let segs = self.segment(text);
		if let [seg] = segs[..] {
			return res!(self.face(seg.face)).shape(text, size, dir, seg.face, 0);
		}

		// More than one face is needed, so each stretch is shaped by its own and the results are laid
		// end to end. Shaping stops at the join -- a ligature cannot span two faces anyway, since the
		// second face has never heard of the first's glyphs.
		let mut runs: Vec<Run> = Vec::with_capacity(segs.len());
		for seg in &segs {
			let sub = match text.get(seg.start..seg.end) {
				Some(s) => s,
				None => return Err(err!(
					"The stretch {}..{} is not a character boundary of the string being shaped.",
					seg.start, seg.end;
				Bug)),
			};
			runs.push(res!(res!(self.face(seg.face)).shape(sub, size, dir, seg.face, seg.start)));
		}

		// The stretches are laid out in VISUAL order, which for right-to-left text is the reverse of
		// the order they were cut in: the first stretch of an Arabic line sits at its right-hand end.
		// Within a stretch the shaper has already done this; across stretches it cannot, because it
		// never saw them together.
		let order: Vec<usize> = match dir {
			Dir::Ltr	=> (0..runs.len()).collect(),
			Dir::Rtl	=> (0..runs.len()).rev().collect(),
		};
		let mut glyphs = Vec::new();
		let mut pen = 0.0f32;
		for i in order {
			for g in &runs[i].glyphs {
				glyphs.push(Glyph {
					x: g.x + pen,
					..*g
				});
			}
			pen += runs[i].advance;
		}
		Ok(Run {
			glyphs,
			advance: pen,
			size,
		})
	}

	/// The outline of one glyph, drawn by the face that shaped it.
	pub fn outline(&self, face: u8, id: u32, size: f32) -> Outcome<Path> {
		res!(self.face(face)).outline(id, size)
	}
}
