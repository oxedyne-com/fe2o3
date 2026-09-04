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

/// Whether a character takes the face of what surrounds it rather than choosing its own.
///
/// Two kinds do. A SPACE is in every face, so it has no opinion worth asking for and asking would
/// cut every run at every word. A COMBINING MARK is positioned against the letter it sits on, by the
/// face that drew that letter: an accent drawn by a face that never saw the base it belongs to lands
/// somewhere of its own choosing, which is how text acquires floating accents.
///
/// The mark test is the canonical combining class, which is the one fe2o3_text makes public. It is
/// not every mark -- a mark of class zero is not caught -- but it is every mark whose whole purpose
/// is to be placed against something else, which is the set that matters here.
fn neutral(ch: char) -> bool {
	ch.is_whitespace() || combining_class(ch) != 0
}

/// A stretch of a string that one face draws the whole of.
#[derive(Clone, Copy, Debug)]
struct Seg {
	/// Which face in the chain draws it.
	face:	u8,
	/// Where it starts in the string, in bytes.
	start:	usize,
	/// Where it ends, in bytes.
	end:	usize,
}

/// What the engine draws with: a face, and the faces to fall back to for what it cannot draw.
///
/// The first face is the one chosen for how it reads, and it draws nearly everything. The rest are
/// there for what it lacks. See the crate note: this is what lets the face at the head of the chain
/// be swapped without the swap costing coverage.
pub struct Font {
	/// The faces, in the order they are tried. Never empty.
	faces:	Vec<Face>,
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
	///
	/// The chain is never empty -- `chain` refuses an empty one and `new` builds one of exactly one --
	/// so the error is a statement that the type's own invariant broke, not a case a caller can meet.
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

	/// The vertical metrics at a size, in pixels.
	///
	/// They are the FIRST face's, and not the tallest of those that happened to be used. A line's
	/// height must not depend on whether an arrow in the middle of it came from further down the
	/// chain: text that changed its leading because of one character would ripple every time a
	/// document was edited. The faces behind the first are chosen to sit within its box.
	pub fn metrics(&self, size: f32) -> Outcome<Metrics> {
		res!(self.first()).metrics(size)
	}

	/// Which face draws a character: the first in the chain that can.
	///
	/// A character no face has is left with the first, which draws its own "not defined" glyph. That
	/// is the honest answer -- the reader is told something is missing rather than shown nothing.
	fn pick(&self, ch: char) -> u8 {
		for (i, face) in self.faces.iter().enumerate() {
			if face.covers(ch) {
				return i as u8;
			}
		}
		0
	}

	/// Cuts a string into the stretches each face draws.
	///
	/// Nearly every character simply asks the chain who draws it. The exception is the NEUTRAL ones,
	/// which take the face already in hand if it can draw them at all -- see [`neutral`]. A space is
	/// in every face, so asking the chain about it would always answer "the first", which would end
	/// the stretch either side of every space: a line of Arabic would be cut into words and shaped one
	/// at a time, losing the shaper's work across each join.
	///
	/// Stickiness must go no further than that. A face kept for anything it merely HAPPENS to cover
	/// never gives the reading face back: the wide face draws Latin perfectly well, so one arrow in a
	/// sentence would drag the whole of the rest of that sentence into the fallback, and the reader
	/// would watch the typeface change mid-line for no reason they could see.
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

	/// Shapes a string, each face in the chain drawing what the one before it could not.
	///
	/// The common case by far is a string one face draws the whole of, which is one shaping call and
	/// exactly what it has always been.
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
