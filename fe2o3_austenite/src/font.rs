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
	set::FontSet,
	shape::{
		Dir,
		Glyph,
		Run,
	},
};
use oxedyne_fe2o3_graphics::path::Path;

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
}

/// A shaped run and the font handle to draw it, carried by a [`LeafKind::Text`](crate::ir::LeafKind)
/// so the emitter can outline each glyph. The set is shared (`Arc`) because a run is cloned into the
/// page frame and outlives the composition that placed it.
#[derive(Clone)]
pub struct ShapedText {
	fonts:	Arc<FontSet>,
	role:	Role,
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
		let font	= fonts.get(role);
		let run		= res!(font.shape(text, size, dir));
		let vm		= res!(font.metrics(size));
		let dims	= Dims::new(
			Sp::from_pt(run.advance as f64),	// the width the shaper's advances sum to
			Sp::from_pt(vm.ascent as f64),		// height above the baseline
			Sp::from_pt(vm.descent as f64),		// depth below it
		);
		Ok(Self { fonts, role, size, run, dims })
	}

	pub fn dims(&self) -> Dims { self.dims }
	pub fn run(&self) -> &Run { &self.run }

	/// The size the run was shaped at, in device points.
	pub fn size(&self) -> f32 { self.size }

	/// One glyph's outline, in the font frame (origin at the glyph, y up); empty for a space.
	pub fn outline(&self, glyph: &Glyph) -> Outcome<Path> {
		self.fonts.get(self.role).outline(glyph.face, glyph.id, self.size)
	}
}

impl std::fmt::Debug for ShapedText {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// The font set holds parsed faces that do not print; summarise the run instead.
		f.debug_struct("ShapedText")
			.field("role", &self.role)
			.field("size", &self.size)
			.field("glyphs", &self.run.glyphs.len())
			.field("dims", &self.dims)
			.finish()
	}
}
