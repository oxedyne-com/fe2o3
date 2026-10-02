// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `layout/frame.rs`, cut down to what maths places, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! A maths frame -- Typst's `Frame` cut down to what maths layout places: glyph runs, stroked lines,
//! nested frames, laid-out external content and location tags, at points from the frame's top-left, y
//! down -- and its lowering to one IR horizontal box.
//!
//! The box sets everything on its baseline: glyph runs become text leaves, whatever their glyphs, raised
//! or lowered by their shift, the ink a rule needs becomes one graphic leaf, and external content keeps
//! its own nodes. Leaves are zero-width and seated by fixed glue, so overlapping parts (a script
//! over a base, a rule over a radicand) need no nested boxes.

use crate::diag::DiagnosticKind;
use crate::eval::realise::Tag;
use crate::eval::value::{
	ColorSpace,
	LineCap,
	Paint,
	Stroke,
};
use crate::eval::Engine;
use crate::flow::visual::{
	fix_stroke,
	pen_of,
	Curve,
	FixedStroke,
	P2,
};
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	DrawOp,
	Glue,
	Graphic,
	Leaf,
	Node,
	Sp,
};
use crate::math::font::{
	MathFont,
	ShapedGlyph,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::shape::Glyph;
use oxedyne_fe2o3_graphics::colour::Rgba;

use std::sync::Arc;

/// A stroke with every part decided, lengths in points.
pub type Pen = FixedStroke;

/// A stroke's unset parts from Typst's defaults, its paint from the text fill.
pub fn pen(stroke: &Stroke, font_size: f64, fill: Paint) -> Pen {
	let mut p = fix_stroke(stroke, font_size);
	if stroke.paint.is_none() {
		p.paint = fill;
	}
	p
}

/// A plain line pen: `paint` at `thickness`, butt caps.
pub fn line_pen(paint: Paint, thickness: f64) -> Pen {
	let mut p = fix_stroke(&Stroke::default(), 11.0);
	p.paint = paint;
	p.thickness = thickness;
	p.cap = LineCap::Butt;
	p
}

/// A run of glyphs from one font at one size, positioned from its baseline origin.
#[derive(Clone)]
pub struct GlyphRun {
	pub font:	Arc<MathFont>,
	pub size:	f64,
	pub fill:	Paint,
	pub text:	String,
	pub glyphs:	Vec<ShapedGlyph>,
}

impl std::fmt::Debug for GlyphRun {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GlyphRun").field("text", &self.text).field("size", &self.size).field("glyphs", &self.glyphs).finish()
	}
}

#[derive(Clone, Debug)]
pub enum FItem {
	Glyphs(GlyphRun),
	Line { to: P2, pen: Pen },			// a stroked segment from the item's position
	Rect { w: f64, h: f64, fill: Paint, pen: Option<Pen> },
	Frame(MFrame),
	Node { nodes: Vec<Node>, w: f64, h: f64 },	// laid-out content, top-left at the position
	Tag(Tag),
}

#[derive(Clone, Debug, Default)]
pub struct MFrame {
	pub w:			f64,
	pub h:			f64,
	pub baseline:	Option<f64>,
	pub items:		Vec<(P2, FItem)>,
}

impl MFrame {
	pub fn new(w: f64, h: f64) -> Self { Self { w, h, baseline: None, items: Vec::new() } }

	pub fn baseline(&self) -> f64 { self.baseline.unwrap_or(self.h) }
	pub fn has_baseline(&self) -> bool { self.baseline.is_some() }
	pub fn set_baseline(&mut self, b: f64) { self.baseline = Some(b); }
	pub fn ascent(&self) -> f64 { self.baseline() }
	pub fn descent(&self) -> f64 { self.h - self.baseline() }

	pub fn push(&mut self, at: P2, item: FItem) { self.items.push((at, item)); }

	pub fn push_frame(&mut self, at: P2, frame: MFrame) {
		if frame.items.is_empty() {
			return;
		}
		self.items.push((at, FItem::Frame(frame)));
	}

	pub fn translate(&mut self, dx: f64, dy: f64) {
		if dx == 0.0 && dy == 0.0 {
			return;
		}
		if let Some(b) = &mut self.baseline {
			*b += dy;
		}
		for (p, _) in self.items.iter_mut() {
			p.x += dx;
			p.y += dy;
		}
	}

	/// Resizes to `w` by `h`, moving the contents by the alignments (0 start, 0.5 centre, 1 end), and
	/// returns how far they moved.
	pub fn resize(&mut self, w: f64, h: f64, ax: f64, ay: f64) -> (f64, f64) {
		let d = ((w - self.w) * ax, (h - self.h) * ay);
		self.w = w;
		self.h = h;
		self.translate(d.0, d.1);
		d
	}
}

// Lowering

enum Prim {
	Text { x: f64, y: f64, shaped: ShapedText },
	Node { x: f64, y: f64, w: f64, h: f64, nodes: Vec<Node> },
	Anchor { x: f64, id: crate::ledger::AnchorId },
}

/// The frame as one horizontal box on its baseline.
pub fn to_node(engine: &mut Engine, frame: &MFrame, span: Span) -> Outcome<Node> {
	let mut prims = Vec::new();
	let mut ink = Vec::new();
	res!(flatten(engine, frame, 0.0, 0.0, &mut prims, &mut ink, span));
	let base = frame.baseline();
	let mut list = Vec::new();
	if !ink.is_empty() {
		// One zero-width graphic whose frame is the whole box: its top is the box's top.
		let dims = Dims::new(Sp::ZERO, Sp::from_pt(base), Sp::from_pt(frame.h - base));
		list.push(Node::Leaf(Leaf::graphic(Graphic::new(ink, dims))));
	}
	// The glyphs stay in the frame's own order, which is Typst's: a row after its row, a base before its
	// scripts, a numerator before its denominator, so the file's text reads as the equation does. The glue
	// that seats each leaf therefore goes back as well as forward.
	let mut cursor = 0.0;
	for p in prims {
		let x = px(&p);
		if (x - cursor).abs() > 1e-9 {
			list.push(Node::Glue(Glue::fixed(Sp::from_pt(x - cursor))));
			cursor = x;
		}
		match p {
			Prim::Text { y, shaped, .. } => {
				let leaf = Leaf::text_dims(shaped, Dims::new(Sp::ZERO, Sp::ZERO, Sp::ZERO))
					.with_shift(Sp::from_pt(y - base))
					.with_span(span.range());
				list.push(Node::Leaf(leaf));
			}
			Prim::Node { y, w, h, nodes, .. } => {
				let above = base - y;
				let inner = Node::VBox(BoxNode::new(nodes, Dims::new(Sp::from_pt(w), Sp::from_pt(h), Sp::ZERO)));
				list.push(Node::VBox(BoxNode::new(vec![inner],
					Dims::new(Sp::from_pt(w), Sp::from_pt(above), Sp::from_pt(h - above)))));
				cursor += w;
			}
			Prim::Anchor { id, .. } => list.push(Node::Anchor(id)),
		}
	}
	if (frame.w - cursor).abs() > 1e-9 {
		list.push(Node::Glue(Glue::fixed(Sp::from_pt(frame.w - cursor))));
	}
	let dims = Dims::new(Sp::from_pt(frame.w), Sp::from_pt(base), Sp::from_pt(frame.h - base));
	Ok(Node::HBox(BoxNode::new(list, dims)))
}

fn px(p: &Prim) -> f64 {
	match p {
		Prim::Text { x, .. } | Prim::Node { x, .. } | Prim::Anchor { x, .. }	=> *x,
	}
}

fn flatten(
	engine:	&mut Engine,
	frame:	&MFrame,
	ox:		f64,
	oy:		f64,
	prims:	&mut Vec<Prim>,
	ink:	&mut Vec<DrawOp>,
	span:	Span,
)
	-> Outcome<()>
{
	for (p, item) in &frame.items {
		let x = ox + p.x;
		let y = oy + p.y;
		match item {
			FItem::Glyphs(run) => prims.push(Prim::Text { x, y, shaped: res!(glyph_text(run)) }),
			FItem::Line { to, pen } => {
				let mut c = Curve::new();
				c.move_(P2::new(0.0, 0.0));
				c.line(*to);
				res!(stroke_ink(engine, &c, pen, x, y, ink, span));
			}
			FItem::Rect { w, h, fill, pen } => {
				let c = Curve::rect(*w, *h);
				if let Some(colour) = res!(rgba(engine, fill, span)) {
					ink.push(DrawOp::Fill { path: res!(c.to_path(P2::new(x, y))), colour });
				}
				if let Some(pen) = pen {
					res!(stroke_ink(engine, &c, pen, x, y, ink, span));
				}
			}
			FItem::Frame(f) => res!(flatten(engine, f, x, y, prims, ink, span)),
			FItem::Node { nodes, w, h } => prims.push(Prim::Node { x, y, w: *w, h: *h, nodes: nodes.clone() }),
			FItem::Tag(Tag::Start(c)) => if let Some(loc) = c.location() {
				prims.push(Prim::Anchor { x, id: loc.anchor() });
			},
			FItem::Tag(Tag::End(_)) => (),
		}
	}
	Ok(())
}

fn stroke_ink(engine: &mut Engine, c: &Curve, pen: &Pen, x: f64, y: f64, ink: &mut Vec<DrawOp>, span: Span) -> Outcome<()> {
	if pen.thickness <= 0.0 {
		return Ok(());
	}
	if let Some(colour) = res!(rgba(engine, &pen.paint, span)) {
		let path = res!(c.to_path(P2::new(x, y)));
		let p = res!(pen_of(pen));
		ink.push(DrawOp::Fill { path: res!(path.stroke(&p)), colour });
	}
	Ok(())
}

// A glyph run as a text run of its own glyphs, whatever they are. A variant, an assembly part and a
// feature-selected form are glyphs of the font like any other: set as the font's glyph ids at their
// offsets they draw as the font draws them and keep the text they stand for, so the page's text and a
// PDF's /ToUnicode carry a script's digit, a stretched delimiter and an accent as characters, as Typst's
// do. A glyph for each character stands for that character; otherwise the run's first glyph stands for
// its whole text.
fn glyph_text(run: &GlyphRun) -> Outcome<ShapedText> {
	let colour = match &run.fill {
		Paint::Color(c) if is_black(c)	=> Rgba::BLACK,
		Paint::Color(c)					=> res!(c.to_rgba()),
		_								=> Rgba::BLACK,
	};
	let starts: Vec<usize> = run.text.char_indices().map(|(i, _)| i).collect();
	let each = starts.len() == run.glyphs.len();
	let (mut px, mut py) = (0.0f64, 0.0f64);
	let mut glyphs = Vec::with_capacity(run.glyphs.len());
	for (i, g) in run.glyphs.iter().enumerate() {
		glyphs.push(Glyph {
			id:		g.id as u32,
			face:	0,
			x:		((px + g.x_offset) * run.size) as f32,
			y:		((py + g.y_offset) * run.size) as f32,	// up from the baseline
			adv:	(g.x_advance * run.size) as f32,
			cluster: if each { starts[i] } else { 0 },
		});
		px += g.x_advance;
		py += g.y_advance;
	}
	let vm = res!(run.font.font.metrics(run.size as f32));
	let dims = Dims::new(Sp::from_pt(px * run.size), Sp::from_pt(vm.ascent as f64), Sp::from_pt(vm.descent as f64));
	Ok(ShapedText::from_glyphs(run.font.font.clone(), run.size as f32, glyphs, run.text.clone(), colour, dims))
}

fn is_black(c: &crate::eval::value::Color) -> bool {
	c.alpha >= 1.0 && match c.space {
		ColorSpace::Luma	=> c.c[0] == 0.0,
		ColorSpace::Rgb		=> c.c[0] == 0.0 && c.c[1] == 0.0 && c.c[2] == 0.0,
		_					=> false,
	}
}

/// A paint as a flat colour: black directly, other colours through the colour library, a gradient by its
/// middle stop and a tiling not at all, each of the last two with a warning.
pub fn rgba(engine: &mut Engine, p: &Paint, span: Span) -> Outcome<Option<Rgba>> {
	match p {
		Paint::Color(c) if is_black(c)	=> Ok(Some(Rgba::BLACK)),
		Paint::Color(c)					=> Ok(Some(res!(c.to_rgba()))),
		Paint::Gradient(g) => {
			engine.warn(DiagnosticKind::Unsupported, span, "gradients are drawn in one flat colour: the drawing layer has no shading");
			match g.stops.get(g.stops.len() / 2) {
				Some((c, _))	=> Ok(Some(res!(c.to_rgba()))),
				None			=> Ok(None),
			}
		}
		Paint::Tiling(_) => {
			engine.warn(DiagnosticKind::Unsupported, span, "tiling paint is not drawn: the drawing layer has no tiling");
			Ok(None)
		}
	}
}
