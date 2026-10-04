// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `math/mod.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U7 owns this file: an equation's realised maths into IR, a port of Typst 0.15's
// `typst-layout/src/math/mod.rs`. An inline equation becomes horizontal boxes a paragraph sets, split
// after relations and binary operators so a line may break there; a block equation becomes its box, with
// its number placed beside it when `numbering` is set.
//
// The styles arrive with the equation's built-in show-set styles (display size, the maths font at weight
// 450, centring; `lib::math::show_set`) beneath the document's own show-set rules, as realisation applies
// them, so `show math.equation: set text(font: ..)` selects the maths font.

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::model::lookup;
use crate::eval::lib::numbering;
use crate::eval::content::display;
use crate::eval::styles::{
	StyleChain,
};
use crate::eval::value::{
	HAlign,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::flow::visual::P2;
use crate::flow::Region;
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Node,
	Sp,
};
use crate::math::font::{
	resolve as resolve_font,
	MathFont,
};
use crate::math::fragment::Frag;
use crate::math::frame::{
	to_node,
	FItem,
	MFrame,
};
use crate::math::item::{
	Item,
	Kind,
};
use crate::math::layout::{
	into_frame,
	Builder,
	Ctx,
};
use crate::math::props;
use crate::math::resolve::Resolver;
use crate::math::class::MathClass;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

const NUMBER_GUTTER: f64 = 0.5;	// ems between an equation and its number

/// An equation as one node: a block equation's box (numbered when it is), or an inline equation's
/// pieces in one horizontal box.
pub fn layout_equation(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Node>
{
	if res!(is_block(elem, styles)) {
		let mut nodes = res!(layout_equation_block(engine, elem, styles, region));
		if nodes.len() == 1 {
			if let Some(n) = nodes.pop() {
				return Ok(n);
			}
		}
		let dims = crate::flow::visual::vlist_dims(&nodes);
		return Ok(Node::VBox(BoxNode::new(nodes, dims)));
	}
	let pieces = res!(layout_equation_inline(engine, elem, styles, region));
	let mut w = Sp::ZERO;
	let mut h = Sp::ZERO;
	let mut d = Sp::ZERO;
	for p in &pieces {
		match p {
			Node::HBox(b) => {
				w += b.dims.width;
				h = h.max(b.dims.height);
				d = d.max(b.dims.depth);
			}
			Node::Glue(g) => w += g.natural,
			_ => (),
		}
	}
	Ok(Node::HBox(BoxNode::new(pieces, Dims::new(w, h, d))))
}

/// Is the equation set as a block?
pub fn is_block(elem: &Content, styles: &StyleChain) -> Outcome<bool> {
	Ok(matches!(res!(efield(elem, styles, "block")), Some(Value::Bool(true))))
}

// An equation field, its own value else the chain's.
fn efield(elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
	match ElemKind::Equation.field_id(name) {
		Some(id)	=> styles.resolve(elem, id),
		None		=> Ok(None),
	}
}

// The maths font the styles select, a warning when it carries no MATH table, and the chain with its
// script percentages.
fn prepare(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<(StyleChain, Arc<MathFont>)> {
	let st = styles.clone();
	let book = res!(engine.fonts.book());
	let font = res!(resolve_font(&book, &res!(props::families(&st)), res!(props::fallback(&st)), res!(props::face_variant(&st))));
	if !font.has_math {
		let d = crate::diag::Diagnostic::warning(DiagnosticKind::Lint, elem.span(), "current font is not designed for math")
			.with_hint("rendering may be poor");
		engine.diags.push(d);
	}
	let st = props::chain_with(&st, vec![props::set_script_scale(
		font.consts.script_percent_scale_down, font.consts.script_script_percent_scale_down)]);
	Ok((st, font))
}

fn body(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	match res!(efield(elem, styles, "body")) {
		Some(v)	=> display(engine, v, elem.span()),
		None	=> Ok(Content::empty()),
	}
}

fn region_size(region: Region) -> (f64, f64) {
	let f = |s: Sp| if s.0 >= (1 << 30) { f64::INFINITY } else { s.to_pt() };
	(f(region.base.0), f(region.base.1))
}

/// An inline equation as the pieces a paragraph sets: horizontal boxes, split after a relation or binary
/// operator, with fixed glue between them where Typst puts a space (a break opportunity).
pub fn layout_equation_inline(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Vec<Node>>
{
	let (st, font) = res!(prepare(engine, elem, styles));
	let b = res!(body(engine, elem, &st));
	let item = {
		let mut r = Resolver::new(engine);
		res!(r.resolve_into_item(&b, &st))
	};
	let mut ctx = Ctx::new(engine, region_size(region), font.clone());
	let mut pieces: Vec<Piece> = if !item.is_multiline() {
		par_items(res!(ctx.layout_into_fragments(&item, &st)))
	} else {
		vec![Piece::Frame(res!(ctx.layout_into_fragment(&item, &st)).into_frame())]
	};
	if pieces.is_empty() {
		pieces.push(Piece::Frame(MFrame::new(0.0, 0.0)));
	}
	let size = props::font_size(&st);
	let slack = res!(props::leading(&st)) * 0.7;
	let mut out = Vec::with_capacity(pieces.len());
	for p in pieces {
		match p {
			Piece::Space(w)			=> out.push(Node::Glue(Glue::fixed(Sp::from_pt(w)))),
			Piece::Frame(mut frame)	=> {
				let top = edge(res!(props::top_edge(&st)), &font, size, frame.ascent(), true);
				let bottom = edge(res!(props::bottom_edge(&st)), &font, size, frame.descent(), false);
				let ascent = top.max(frame.ascent() - slack);
				let descent = bottom.max(frame.descent() - slack);
				let b = frame.baseline();
				frame.translate(0.0, ascent - b);
				frame.h = ascent + descent;
				out.push(res!(to_node(engine, &frame, elem.span())));
			}
		}
	}
	Ok(out)
}

// A text edge in points: a font metric, a length, or the frame's own bounds. A bottom edge is a depth.
fn edge(e: props::Edge, font: &MathFont, size: f64, bounds: f64, top: bool) -> f64 {
	let sign = if top { 1.0 } else { -1.0 };
	match e {
		props::Edge::Ascender	=> sign * font.ascender() * size,
		props::Edge::CapHeight	=> sign * font.cap_height() * size,
		props::Edge::XHeight	=> sign * font.x_height() * size,
		props::Edge::Baseline	=> 0.0,
		props::Edge::Descender	=> sign * font.descender() * size,
		props::Edge::Bounds		=> bounds,
		props::Edge::Length(l)	=> sign * l,
	}
}

enum Piece {
	Frame(MFrame),
	Space(f64),
}

// Typst's `into_par_items`: a new piece after each relation or binary operator not followed by a
// closing delimiter (or a second relation), spaces after such a break kept visible.
fn par_items(frags: Vec<Frag>) -> Vec<Piece> {
	let mut items = Vec::new();
	let mut x = 0.0f64;
	let mut ascent = 0.0f64;
	let mut descent = 0.0f64;
	let mut frame = MFrame::new(0.0, 0.0);
	let mut empty = true;
	let mut space_visible = false;
	let finish = |frame: &mut MFrame, x: f64, a: f64, d: f64| {
		frame.w = x;
		frame.h = a + d;
		frame.set_baseline(0.0);
		frame.translate(0.0, a);
	};
	let mut iter = frags.into_iter().peekable();
	while let Some(f) = iter.next() {
		if space_visible {
			if let Frag::Space(w) = f {
				items.push(Piece::Space(w));
				continue;
			}
		}
		let class = f.class();
		let y = f.ascent();
		ascent = ascent.max(y);
		descent = descent.max(f.descent());
		let w = f.width();
		frame.push_frame(P2::new(x, -y), f.into_frame());
		x += w;
		empty = false;
		let next = iter.peek().map(|n| n.class());
		let brk = match class {
			MathClass::Binary	=> next != Some(MathClass::Closing),
			MathClass::Relation	=> !matches!(next, Some(MathClass::Relation | MathClass::Closing)),
			_					=> false,
		};
		if brk {
			let mut prev = std::mem::take(&mut frame);
			finish(&mut prev, x, ascent, descent);
			items.push(Piece::Frame(prev));
			empty = true;
			x = 0.0;
			ascent = 0.0;
			descent = 0.0;
			space_visible = true;
			if let Some(n) = iter.peek() {
				if !matches!(n, Frag::Space(_)) {
					items.push(Piece::Space(0.0));
				}
			}
		} else {
			space_visible = false;
		}
	}
	if !empty {
		finish(&mut frame, x, ascent, descent);
		items.push(Piece::Frame(frame));
	}
	items
}

/// A block equation as nodes: one box, or with `block.breakable` on and several rows, one box per row
/// with the leading between them as glue so the page may break there.
pub fn layout_equation_block(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Vec<Node>>
{
	let (st, font) = res!(prepare(engine, elem, styles));
	let b = res!(body(engine, elem, &st));
	let item = {
		let mut r = Resolver::new(engine);
		res!(r.resolve_into_item(&b, &st))
	};
	let (rw, _) = region_size(region);
	let mut ctx = Ctx::new(engine, region_size(region), font.clone());
	let builder = match &item {
		Item::Comp(c) => match &c.kind {
			Kind::Multiline { rows, .. }	=> res!(ctx.multiline(rows, &c.styles)),
			_								=> from_frame(into_frame(res!(ctx.layout_into_fragments(&item, &st)))),
		},
		_ => from_frame(into_frame(res!(ctx.layout_into_fragments(&item, &st)))),
	};
	let numbering = match res!(efield(elem, &st, "numbering")) {
		None | Some(Value::None)	=> None,
		Some(n)						=> Some(n),
	};
	let span = elem.span();
	match numbering {
		None if res!(props::breakable(&st)) && builder.frames.len() > 1 => {
			// One box per row, the gaps between rows as glue the page may break at.
			let mut out = Vec::new();
			let mut bottom = 0.0f64;
			for (i, (row, pos)) in builder.frames.into_iter().enumerate() {
				if i > 0 {
					out.push(Node::Glue(Glue::fixed(Sp::from_pt(pos.y - bottom))));
				}
				bottom = pos.y + row.h;
				let mut f = MFrame::new(builder.w, row.h);
				f.set_baseline(row.baseline());
				f.push_frame(P2::new(pos.x, 0.0), row);
				out.push(res!(to_node(engine, &f, span)));
			}
			Ok(out)
		}
		None => {
			let frame = builder.build(true);
			Ok(vec![res!(to_node(engine, &frame, span))])
		}
		Some(n) => {
			let number = res!(number_frame(engine, elem, &st, &n, region));
			let size = props::font_size(&st);
			let full = number.w + NUMBER_GUTTER * size;
			let (nh, nv) = res!(number_align(elem, &st));
			let eq_align = match res!(props::align_x(&st)) {
				Some(HAlign::Center)					=> 0.5,
				Some(HAlign::Right) | Some(HAlign::End)	=> 1.0,
				_										=> 0.0,
			};
			let frame = add_number(builder, number, (nh, nv), eq_align, rw, full);
			Ok(vec![res!(to_node(engine, &frame, span))])
		}
	}
}

fn from_frame(frame: MFrame) -> Builder {
	Builder { w: frame.w, h: frame.h, frames: vec![(frame, P2::new(0.0, 0.0))] }
}

// The number's horizontal side (0 start, 1 end) and vertical placement (0 top, 0.5 horizon, 1 bottom).
fn number_align(elem: &Content, styles: &StyleChain) -> Outcome<(f64, f64)> {
	let (x, y) = match res!(efield(elem, styles, "number-align")) {
		Some(Value::Alignment(a))	=> (a.x, a.y),
		_							=> (None, None),
	};
	let h = match x {
		Some(HAlign::Start) | Some(HAlign::Left)	=> 0.0,
		_											=> 1.0,
	};
	let v = match y {
		Some(VAlign::Top)		=> 0.0,
		Some(VAlign::Bottom)	=> 1.0,
		_						=> 0.5,
	};
	Ok((h, v))
}

fn number_frame(engine: &mut Engine, elem: &Content, styles: &StyleChain, numbering_v: &Value, region: Region) -> Outcome<MFrame> {
	// The number is the equation counter at the equation's own location, read through the model's lookup
	// seam so a counter update and the previous pass's layout facts reach it. An equation with no
	// location reads the counter's initial state, as the first pass does.
	let content = match elem.location() {
		Some(loc)	=> res!(lookup::display_counter(
			engine, &lookup::elem_counter(ElemKind::Equation), loc, numbering_v, elem.span())),
		None		=> {
			let shown = res!(numbering::apply(engine, numbering_v, &[0]));
			res!(display(engine, shown, elem.span()))
		}
	};
	let nodes = res!(crate::flow::layout_block(engine, &content, styles, Region {
		expand_x:	false,
		expand_y:	false,
		..region
	}));
	let dims = crate::flow::visual::vlist_dims(&nodes);
	let (w, h) = (dims.width.to_pt(), dims.vextent().to_pt());
	let mut f = MFrame::new(w, h);
	if let Some(b) = crate::flow::visual::first_baseline(&nodes) {
		f.set_baseline(b.to_pt());
	}
	f.push(P2::new(0.0, 0.0), FItem::Node { nodes, w, h });
	Ok(f)
}

// Typst's `add_equation_number` and `resize_equation`.
fn add_number(builder: Builder, number: MFrame, align: (f64, f64), eq_align: f64, region_w: f64, full: f64) -> MFrame {
	let first = builder.frames.first().map(|(f, p)| (f.h, *p, f.baseline())).unwrap_or((builder.h, P2::default(), 0.0));
	let last = builder.frames.last().map(|(f, p)| (f.h, *p, f.baseline())).unwrap_or((builder.h, P2::default(), 0.0));
	let lines = builder.frames.len();
	let mut eq = builder.build(true);
	let width = if region_w.is_finite() { region_w } else { eq.w + 2.0 * full };
	let multiline = lines >= 2;
	let offset = if align.1 == 0.5 && multiline {
		let h = eq.h.max(number.h);
		eq.resize(width, h, eq_align, 0.5)
	} else {
		let above = if !multiline || align.1 == 0.0 { (number.baseline() - first.2).max(0.0) } else { 0.0 };
		let below = if !multiline || align.1 == 1.0 { ((number.h - number.baseline()) - (last.0 - last.2)).max(0.0) } else { 0.0 };
		let h = eq.h + above + below;
		let o = eq.resize(width, h, eq_align, 0.0);
		eq.translate(0.0, above);
		(o.0, o.1 + above)
	};
	let shift = if eq_align == 0.0 && align.0 == 0.0 {
		full
	} else if eq_align == 1.0 && align.0 == 1.0 {
		-full
	} else {
		0.0
	};
	eq.translate(shift, 0.0);
	let x = if align.0 == 0.0 { 0.0 } else { eq.w - number.w };
	let baselines = |(_, pos, base): (f64, P2, f64)| offset.1 + pos.y + base - number.baseline();
	let y = if align.1 == 0.0 {
		baselines(first)
	} else if align.1 == 0.5 && !multiline {
		baselines(first)
	} else if align.1 == 0.5 {
		(eq.h - number.h) / 2.0
	} else {
		baselines(last)
	};
	eq.push_frame(P2::new(x, y), number);
	eq
}
