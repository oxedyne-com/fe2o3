//! Real font metrics and shaped text, over `fe2o3_font`.
//!
//! `fe2o3_font` works in device pixels, which at the SVG boundary are points (the media box is whole
//! points). A text size is therefore a length in points carried as an `f32`, and the shaper's
//! advances and the face's metrics come back in points, converted to [`Sp`] at that boundary.

use crate::ir::{
	Dims,
	Metrics,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	set::FontSet,
	shape::{
		Dir,
		Glyph,
		Run,
	},
};
use oxedyne_fe2o3_graphics::path::Path;
use oxedyne_fe2o3_graphics::transform::Transform;

use std::sync::Arc;

/// Measures text by shaping it and summing advances; height and depth are the face's own. The
/// Phase 1 replacement for [`StubMetrics`](crate::ir::StubMetrics), which stays for running without
/// a font.
#[derive(Clone)]
pub struct FontMetrics {
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	f32,	// device points, the shaper's pixel
}

impl FontMetrics {
	pub fn new(fonts: Arc<FontSet>, role: Role, dir: Dir, size: Sp) -> Self {
		Self { fonts, role, dir, size: size.to_pt() as f32 }
	}
}

impl Metrics for FontMetrics {
	fn measure(&self, text: &str) -> Outcome<Dims> {
		let shaped = res!(ShapedText::shape(self.fonts.clone(), self.role, self.dir, self.size, text));
		Ok(shaped.dims())
	}

	fn shape(&self, text: &str) -> Outcome<Option<ShapedText>> {
		Ok(Some(res!(ShapedText::shape(self.fonts.clone(), self.role, self.dir, self.size, text))))
	}
}

/// Where a shaped run draws its glyphs from: a role in the reader's set, or a standalone font handed
/// in outside the set -- the maths font, which is not one of the reading roles. Both are shared (`Arc`)
/// because a run is cloned into the page frame and outlives the composition that placed it.
#[derive(Clone)]
enum Source {
	Set { fonts: Arc<FontSet>, role: Role },
	Solo { font: Arc<Font> },
}

impl Source {
	fn font(&self) -> &Font {
		match self {
			Source::Set { fonts, role }	=> fonts.get(*role),
			Source::Solo { font }		=> font,
		}
	}
}

/// A shaped run and the font handle to draw it, carried by a [`LeafKind::Text`](crate::ir::LeafKind)
/// so the emitter can outline each glyph.
#[derive(Clone)]
pub struct ShapedText {
	src:	Source,
	size:	f32,	// device points, the shaper's pixel and the outline's size
	run:	Run,
	dims:	Dims,
}

impl ShapedText {
	/// Shapes `text` in `role` at `size` points, keeping the run and the handle to draw it.
	pub fn new(
		fonts:	Arc<FontSet>,
		role:	Role,
		dir:	Dir,
		size:	Sp,
		text:	&str,
	)
		-> Outcome<Self>
	{
		Self::shape(fonts, role, dir, size.to_pt() as f32, text)
	}

	/// Shapes in the shaper's own unit, shared by measurement and placement.
	fn shape(
		fonts:	Arc<FontSet>,
		role:	Role,
		dir:	Dir,
		size:	f32,
		text:	&str,
	)
		-> Outcome<Self>
	{
		Self::from_source(Source::Set { fonts, role }, dir, size, text)
	}

	/// Shapes in a standalone font outside the reading set -- the maths font, whose glyphs a role's
	/// chain does not carry. The run seats and outlines against that same font.
	pub fn new_with_font(
		font:	Arc<Font>,
		dir:	Dir,
		size:	Sp,
		text:	&str,
	)
		-> Outcome<Self>
	{
		Self::from_source(Source::Solo { font }, dir, size.to_pt() as f32, text)
	}

	fn from_source(src: Source, dir: Dir, size: f32, text: &str) -> Outcome<Self> {
		let font	= src.font();
		let run		= res!(font.shape(text, size, dir));
		let vm		= res!(font.metrics(size));
		let dims	= Dims::new(
			Sp::from_pt(run.advance as f64),	// the width the shaper's advances sum to
			Sp::from_pt(vm.ascent as f64),		// height above the baseline
			Sp::from_pt(vm.descent as f64),		// depth below it
		);
		Ok(Self { src, size, run, dims })
	}

	pub fn dims(&self) -> Dims { self.dims }
	pub fn run(&self) -> &Run { &self.run }

	/// The size the run was shaped at, in device points.
	pub fn size(&self) -> f32 { self.size }

	/// One glyph's outline, in the font frame (origin at the glyph, y up); empty for a space.
	pub fn outline(&self, glyph: &Glyph) -> Outcome<Path> {
		self.src.font().outline(glyph.face, glyph.id, self.size)
	}

	/// The run's real ink extent above and below the baseline, taken from the glyph outlines rather
	/// than the font's global ascent and descent. Maths needs this: a maths font's global ascent spans
	/// its tallest construction -- a big integral, a three-storey brace -- not the single symbol in
	/// hand, so seating a fraction or a script by the font metric puts it wildly wrong. The outline is
	/// y up with the baseline at zero, so the top of the ink is the height and the bottom, when it dips
	/// below the baseline, is the depth.
	pub fn ink_extent(&self) -> Outcome<(Sp, Sp)> {
		let mut top = 0.0f32;	// greatest height above the baseline
		let mut bot = 0.0f32;	// greatest depth below it, as a positive number
		for glyph in &self.run.glyphs {
			let path = res!(self.outline(glyph));
			if let Some(b) = path.bounds(&Transform::IDENTITY) {
				if b.y1 > top { top = b.y1; }
				if -b.y0 > bot { bot = -b.y0; }
			}
		}
		Ok((Sp::from_pt(top as f64), Sp::from_pt(bot as f64)))
	}
}

impl std::fmt::Debug for ShapedText {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// The font set holds parsed faces that do not print; summarise the run instead.
		f.debug_struct("ShapedText")
			.field("size", &self.size)
			.field("glyphs", &self.run.glyphs.len())
			.field("dims", &self.dims)
			.finish()
	}
}
