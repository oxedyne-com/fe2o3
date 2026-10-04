// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `math/shared.rs` and `math/fragment`, typst-library `math/` font queries, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! A maths font: a face to shape and outline with, its MATH table and metrics in ems, and the glyph
//! queries maths layout asks (advances, ink boxes, italics corrections, accent attachments, cut-in
//! kerning, constructions). Ems throughout; a caller multiplies by the font size in points.
//!
//! Faces are found by family name in the compilation's font book -- the faces this crate embeds, New
//! Computer Modern Math among them (Typst's default for equations), and every face the host supplies. A
//! family the book cannot supply falls through to the next in the list and, as in Typst, to the maths
//! fallback list.

use crate::fonts::{
	BookFace,
	FaceVariant,
	FontBook,
};
use crate::mathtable::{
	Construction,
	FontTables,
	Kern,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	font::Font,
	shape::{
		Dir,
		Feature,
	},
};
use oxedyne_fe2o3_graphics::transform::Transform;

use std::collections::HashMap;
use std::sync::{
	Arc,
	RwLock,
};

/// A corner of a glyph, for cut-in kerning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
	TopLeft,
	TopRight,
	BottomRight,
	BottomLeft,
}

impl Corner {
	/// The diagonally opposite corner: a script's corner facing the base's.
	pub fn inv(self) -> Self {
		match self {
			Corner::TopLeft		=> Corner::BottomRight,
			Corner::TopRight	=> Corner::BottomLeft,
			Corner::BottomRight	=> Corner::TopLeft,
			Corner::BottomLeft	=> Corner::TopRight,
		}
	}
}

/// The MATH constants Typst reads, in ems (the three plain numbers excepted).
#[derive(Clone, Copy, Debug, Default)]
pub struct Constants {
	pub space_width:							f64,
	pub script_percent_scale_down:				i16,
	pub script_script_percent_scale_down:		i16,
	pub display_operator_min_height:			f64,
	pub axis_height:							f64,
	pub accent_base_height:						f64,
	pub flattened_accent_base_height:			f64,
	pub subscript_shift_down:					f64,
	pub subscript_top_max:						f64,
	pub subscript_baseline_drop_min:			f64,
	pub superscript_shift_up:					f64,
	pub superscript_shift_up_cramped:			f64,
	pub superscript_bottom_min:					f64,
	pub superscript_baseline_drop_max:			f64,
	pub sub_superscript_gap_min:				f64,
	pub superscript_bottom_max_with_subscript:	f64,
	pub space_after_script:						f64,
	pub upper_limit_gap_min:					f64,
	pub upper_limit_baseline_rise_min:			f64,
	pub lower_limit_gap_min:					f64,
	pub lower_limit_baseline_drop_min:			f64,
	pub stack_top_shift_up:						f64,
	pub stack_top_display_style_shift_up:		f64,
	pub stack_bottom_shift_down:				f64,
	pub stack_bottom_display_style_shift_down:	f64,
	pub stack_gap_min:							f64,
	pub stack_display_style_gap_min:			f64,
	pub fraction_numerator_shift_up:			f64,
	pub fraction_numerator_display_style_shift_up:		f64,
	pub fraction_denominator_shift_down:		f64,
	pub fraction_denominator_display_style_shift_down:	f64,
	pub fraction_numerator_gap_min:				f64,
	pub fraction_num_display_style_gap_min:		f64,
	pub fraction_rule_thickness:				f64,
	pub fraction_denominator_gap_min:			f64,
	pub fraction_denom_display_style_gap_min:	f64,
	pub skewed_fraction_vertical_gap:			f64,
	pub skewed_fraction_horizontal_gap:			f64,
	pub overbar_vertical_gap:					f64,
	pub overbar_rule_thickness:					f64,
	pub overbar_extra_ascender:					f64,
	pub underbar_vertical_gap:					f64,
	pub underbar_rule_thickness:				f64,
	pub underbar_extra_descender:				f64,
	pub radical_vertical_gap:					f64,
	pub radical_display_style_vertical_gap:		f64,
	pub radical_rule_thickness:					f64,
	pub radical_extra_ascender:					f64,
	pub radical_kern_before_degree:				f64,
	pub radical_kern_after_degree:				f64,
	pub radical_degree_bottom_raise_percent:	f64,
}

/// One shaped glyph in ems: Typst's `Glyph` without the source range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
	pub id:			u16,
	pub x_advance:	f64,
	pub x_offset:	f64,
	pub y_advance:	f64,
	pub y_offset:	f64,
}

/// A glyph's ink box in ems, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
	pub x0:	f64,
	pub y0:	f64,
	pub x1:	f64,
	pub y1:	f64,
}

pub struct MathFont {
	pub font:		Arc<Font>,
	pub tables:		FontTables,
	pub family:		String,
	pub weight:		u16,
	pub consts:		Constants,
	pub has_math:	bool,
	boxes:			RwLock<HashMap<u16, Option<GlyphBox>>>,
}

impl std::fmt::Debug for MathFont {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MathFont").field("family", &self.family).field("weight", &self.weight).finish()
	}
}

impl MathFont {
	/// The maths view of a book face, sharing its font so text and equations in one family are one font to
	/// a PDF.
	pub fn from_face(face: &BookFace) -> Outcome<Self> {
		let tables	= res!(FontTables::parse(res!(face.face()).bytes()));
		let mut mf = Self {
			font:		face.font.clone(),
			tables,
			family:		face.family.clone(),
			weight:		face.variant.weight,
			consts:		Constants::default(),
			has_math:	false,
			boxes:		RwLock::new(HashMap::new()),
		};
		mf.consts = res!(mf.constants());
		mf.has_math = mf.tables.math.is_some();
		Ok(mf)
	}

	pub fn upem(&self) -> f64 { self.tables.upem }

	/// Design units to ems.
	pub fn em(&self, du: f64) -> f64 { du / self.tables.upem }

	fn constants(&self) -> Outcome<Constants> {
		let space = res!(self.shape(" ", &[]));
		let space_width = match space.first() {
			Some(g) if g.id != 0	=> g.x_advance,
			_						=> 5.0 / 18.0,
		};
		let m = &self.tables.metrics;
		let math = match &self.tables.math {
			Some(t)	=> t,
			None	=> {
				// Typst's fallback constants for a text face, after MathML Core.
				let u = self.em(m.underline_thickness as f64);
				let u = if u > 0.0 { u } else { 0.06 };
				let xh = self.em(m.x_height as f64);
				return Ok(Constants {
					space_width,
					script_percent_scale_down:			70,
					script_script_percent_scale_down:	50,
					axis_height:						xh / 2.0,
					accent_base_height:					xh,
					flattened_accent_base_height:		self.em(m.cap_height as f64),
					subscript_shift_down:	m.subscript_y_offset.map(|v| -self.em(v as f64)).unwrap_or(-0.2),
					subscript_top_max:					0.8 * xh,
					superscript_shift_up:	m.superscript_y_offset.map(|v| self.em(v as f64)).unwrap_or(0.5),
					superscript_bottom_min:				0.25 * xh,
					sub_superscript_gap_min:			4.0 * u,
					superscript_bottom_max_with_subscript:	0.8 * xh,
					space_after_script:					1.0 / 24.0,
					stack_gap_min:						3.0 * u,
					stack_display_style_gap_min:		7.0 * u,
					fraction_numerator_gap_min:			u,
					fraction_num_display_style_gap_min:	3.0 * u,
					fraction_rule_thickness:			u,
					fraction_denominator_gap_min:		u,
					fraction_denom_display_style_gap_min:	3.0 * u,
					skewed_fraction_horizontal_gap:		0.5,
					overbar_vertical_gap:				3.0 * u,
					overbar_rule_thickness:				u,
					overbar_extra_ascender:				u,
					underbar_vertical_gap:				3.0 * u,
					underbar_rule_thickness:			u,
					underbar_extra_descender:			u,
					radical_vertical_gap:				1.25 * u,
					radical_display_style_vertical_gap:	u + 0.25 * xh,
					radical_rule_thickness:				u,
					radical_extra_ascender:				u,
					radical_kern_before_degree:			5.0 / 18.0,
					radical_kern_after_degree:			-10.0 / 18.0,
					radical_degree_bottom_raise_percent:	0.6,
					..Constants::default()
				});
			}
		};
		let c = math.constants();
		let e = |v: i16| self.em(v as f64);
		Ok(Constants {
			space_width,
			script_percent_scale_down:					c.script_percent_scale_down,
			script_script_percent_scale_down:			c.script_script_percent_scale_down,
			display_operator_min_height:				self.em(c.display_operator_min_height as f64),
			axis_height:								e(c.axis_height),
			accent_base_height:							e(c.accent_base_height),
			flattened_accent_base_height:				e(c.flattened_accent_base_height),
			subscript_shift_down:						e(c.subscript_shift_down),
			subscript_top_max:							e(c.subscript_top_max),
			subscript_baseline_drop_min:				e(c.subscript_baseline_drop_min),
			superscript_shift_up:						e(c.superscript_shift_up),
			superscript_shift_up_cramped:				e(c.superscript_shift_up_cramped),
			superscript_bottom_min:						e(c.superscript_bottom_min),
			superscript_baseline_drop_max:				e(c.superscript_baseline_drop_max),
			sub_superscript_gap_min:					e(c.sub_superscript_gap_min),
			superscript_bottom_max_with_subscript:		e(c.superscript_bottom_max_with_subscript),
			space_after_script:							e(c.space_after_script),
			upper_limit_gap_min:						e(c.upper_limit_gap_min),
			upper_limit_baseline_rise_min:				e(c.upper_limit_baseline_rise_min),
			lower_limit_gap_min:						e(c.lower_limit_gap_min),
			lower_limit_baseline_drop_min:				e(c.lower_limit_baseline_drop_min),
			stack_top_shift_up:							e(c.stack_top_shift_up),
			stack_top_display_style_shift_up:			e(c.stack_top_display_style_shift_up),
			stack_bottom_shift_down:					e(c.stack_bottom_shift_down),
			stack_bottom_display_style_shift_down:		e(c.stack_bottom_display_style_shift_down),
			stack_gap_min:								e(c.stack_gap_min),
			stack_display_style_gap_min:				e(c.stack_display_style_gap_min),
			fraction_numerator_shift_up:				e(c.fraction_num_shift_up),
			fraction_numerator_display_style_shift_up:		e(c.fraction_num_display_shift_up),
			fraction_denominator_shift_down:			e(c.fraction_den_shift_down),
			fraction_denominator_display_style_shift_down:	e(c.fraction_den_display_shift_down),
			fraction_numerator_gap_min:					e(c.fraction_num_gap_min),
			fraction_num_display_style_gap_min:			e(c.fraction_num_display_gap_min),
			fraction_rule_thickness:					e(c.fraction_rule_thickness),
			fraction_denominator_gap_min:				e(c.fraction_denom_gap_min),
			fraction_denom_display_style_gap_min:		e(c.fraction_denom_display_gap_min),
			skewed_fraction_vertical_gap:				e(c.skewed_fraction_vertical_gap),
			skewed_fraction_horizontal_gap:				e(c.skewed_fraction_horizontal_gap),
			overbar_vertical_gap:						e(c.overbar_vertical_gap),
			overbar_rule_thickness:						e(c.overbar_rule_thickness),
			overbar_extra_ascender:						e(c.overbar_extra_ascender),
			underbar_vertical_gap:						e(c.underbar_vertical_gap),
			underbar_rule_thickness:					e(c.underbar_rule_thickness),
			underbar_extra_descender:					e(c.underbar_extra_descender),
			radical_vertical_gap:						e(c.radical_vertical_gap),
			radical_display_style_vertical_gap:			e(c.radical_display_style_vertical_gap),
			radical_rule_thickness:						e(c.radical_rule_thickness),
			radical_extra_ascender:						e(c.radical_extra_ascender),
			radical_kern_before_degree:					e(c.radical_kern_before_degree),
			radical_kern_after_degree:					e(c.radical_kern_after_degree),
			radical_degree_bottom_raise_percent:		c.radical_degree_bottom_raise_percent as f64 / 100.0,
		})
	}

	/// Shapes maths text left to right in the `math` script, in ems, with OpenType features.
	pub fn shape(&self, text: &str, features: &[Feature]) -> Outcome<Vec<ShapedGlyph>> {
		let size = self.upem() as f32;
		let run = res!(self.font.shape_in(text, size, Dir::Ltr, features, *b"Zmth"));
		Ok(self.glyphs_of(&run))
	}

	/// Shapes running text in the script its characters have, as a quoted string in maths is set.
	pub fn shape_text(&self, text: &str, features: &[Feature]) -> Outcome<Vec<ShapedGlyph>> {
		let size = self.upem() as f32;
		let run = res!(self.font.shape_with(text, size, Dir::Ltr, features));
		Ok(self.glyphs_of(&run))
	}

	fn glyphs_of(&self, run: &oxedyne_fe2o3_font::shape::Run) -> Vec<ShapedGlyph> {
		let mut out = Vec::with_capacity(run.glyphs.len());
		let mut pen = 0.0f64;
		for g in &run.glyphs {
			out.push(ShapedGlyph {
				id:			g.id as u16,
				x_advance:	self.em(g.adv as f64),
				x_offset:	self.em(g.x as f64 - pen),
				y_advance:	0.0,
				y_offset:	self.em(g.y as f64),
			});
			pen += g.adv as f64;
		}
		out
	}

	/// Can the face draw every character of the text?
	pub fn covers(&self, text: &str) -> bool {
		match self.font.face(0) {
			Ok(face)	=> text.chars().all(|c| face.covers(c) || c == '\u{fe00}' || c == '\u{fe01}'),
			Err(_)		=> false,
		}
	}

	pub fn advance(&self, gid: u16) -> f64 { self.em(self.tables.advance(gid) as f64) }

	/// The glyph's control box in ems, as `ttf-parser` reports a glyph's bounding box; `None` for a glyph
	/// with no outline.
	pub fn bbox(&self, gid: u16) -> Outcome<Option<GlyphBox>> {
		{
			let cache = lock_read!(self.boxes);
			if let Some(b) = cache.get(&gid) {
				return Ok(*b);
			}
		}
		let path = res!(self.font.outline(0, gid as u32, self.upem() as f32));
		let b = path.bounds(&Transform::IDENTITY).map(|b| GlyphBox {
			x0:	self.em(b.x0 as f64),
			y0:	self.em(b.y0 as f64),
			x1:	self.em(b.x1 as f64),
			y1:	self.em(b.y1 as f64),
		});
		let mut cache = lock_write!(self.boxes);
		cache.insert(gid, b);
		Ok(b)
	}

	/// Ascent and descent of the glyph's ink box in ems, descent positive below the baseline.
	pub fn ascent_descent(&self, gid: u16) -> Outcome<Option<(f64, f64)>> {
		Ok(res!(self.bbox(gid)).map(|b| (b.y1, -b.y0)))
	}

	pub fn italics_correction(&self, gid: u16) -> Option<f64> {
		self.tables.math.as_ref()?.italics_correction(gid).map(|v| self.em(v as f64))
	}

	pub fn top_accent_attachment(&self, gid: u16) -> Option<f64> {
		self.tables.math.as_ref()?.top_accent_attachment(gid).map(|v| self.em(v as f64))
	}

	pub fn is_extended_shape(&self, gid: u16) -> bool {
		self.tables.math.as_ref().map(|t| t.is_extended_shape(gid)).unwrap_or(false)
	}

	/// The cut-in kern at a corner and a height (ems), or `None` when the glyph has no such table.
	pub fn kern_at_height(&self, gid: u16, corner: Corner, height: f64) -> Option<f64> {
		let info = self.tables.math.as_ref()?.kern_info(gid)?;
		let kern: &Kern = match corner {
			Corner::TopLeft		=> info.top_left.as_ref(),
			Corner::TopRight	=> info.top_right.as_ref(),
			Corner::BottomRight	=> info.bottom_right.as_ref(),
			Corner::BottomLeft	=> info.bottom_left.as_ref(),
		}?;
		Some(self.em(kern.at(height * self.upem()) as f64))
	}

	pub fn construction(&self, gid: u16, vertical: bool) -> Option<&Construction> {
		self.tables.math.as_ref()?.construction(gid, vertical)
	}

	pub fn min_connector_overlap(&self) -> Option<f64> {
		self.tables.math.as_ref().map(|t| self.em(t.min_connector_overlap() as f64))
	}

	pub fn has_feature(&self, tag: &[u8; 4]) -> bool { self.tables.has_feature(tag) }

	pub fn cap_height(&self) -> f64 { self.em(self.tables.metrics.cap_height as f64) }
	pub fn x_height(&self) -> f64 { self.em(self.tables.metrics.x_height as f64) }
	pub fn ascender(&self) -> f64 { self.em(self.tables.metrics.ascender as f64) }
	pub fn descender(&self) -> f64 { self.em(self.tables.metrics.descender as f64) }
}

/// Typst's maths fallback families, tried after the document's own when `text.fallback` is on.
pub const FALLBACKS: &[&str] = &[
	"new computer modern math",
	"libertinus serif",
	"twitter color emoji",
	"noto color emoji",
	"apple color emoji",
	"segoe ui emoji",
];

/// The face of `family` nearest `variant` in the book, if the book has the family.
pub fn select(book: &FontBook, family: &str, variant: FaceVariant) -> Outcome<Option<Arc<MathFont>>> {
	match book.select(&family.to_lowercase(), variant) {
		Some(id)	=> Ok(Some(res!(book.math_font(id)))),
		None		=> Ok(None),
	}
}

/// The face maths is set in: the first family of the list the book has, then the fallback families when
/// `fallback` is on, as Typst's `get_font`. A list that no face answers, with the fallback off, is an error.
pub fn resolve(book: &FontBook, families: &[String], fallback: bool, variant: FaceVariant) -> Outcome<Arc<MathFont>> {
	for fam in families {
		if let Some(f) = res!(select(book, fam, variant)) {
			return Ok(f);
		}
	}
	if fallback {
		for fam in FALLBACKS {
			if let Some(f) = res!(select(book, fam, variant)) {
				return Ok(f);
			}
		}
	}
	Err(err!("no font could be found"; Missing))
}

/// The faces to try, in order, for a character the first face lacks.
pub fn chain(book: &FontBook, families: &[String], fallback: bool, variant: FaceVariant) -> Outcome<Vec<Arc<MathFont>>> {
	let mut out: Vec<Arc<MathFont>> = Vec::new();
	let push = |f: Arc<MathFont>, out: &mut Vec<Arc<MathFont>>| {
		if !out.iter().any(|g| Arc::ptr_eq(g, &f)) {
			out.push(f);
		}
	};
	for fam in families {
		if let Some(f) = res!(select(book, fam, variant)) {
			push(f, &mut out);
		}
	}
	if fallback {
		for fam in FALLBACKS {
			if let Some(f) = res!(select(book, fam, variant)) {
				push(f, &mut out);
			}
		}
	}
	if out.is_empty() {
		out.push(res!(resolve(book, families, fallback, variant)));
	}
	Ok(out)
}
