// U6c owns this file: grid and table into rows, with colspan/rowspan, gutters, per-cell fill/stroke/align
// (values, arrays cycling by column, or closures of x and y), lines, and repeating headers.
//
// Three stages, each Typst's. `resolve` places every cell (auto-placement skips occupied slots and never
// wraps a colspan), folds each cell's inset, align, fill and stroke over the grid's, and writes them back
// into the cell so a `show table.cell` rule reads them. `plan` sizes the tracks (relative, then auto from
// the cells' natural sizes with a span's excess on its last auto track, then fractions of what is left,
// shrinking overlarge auto columns fairly when nothing is left) and resolves every line segment by
// Typst's priorities: an explicit line over a cell's own stroke over the grid's, the later cell winning a
// tie. `lower` turns the plan into vertical material: one unbreakable box per group of rows a rowspan
// welds, zero or gutter glue between them as the legal page breaks, and the header armed as a
// `Node::RepeatHead`.
//
// A boundary line between two row groups is drawn by both: each group's box carries its own top and bottom
// rules, so a page break between them frames both pages, as Typst does. Coincident strokes draw twice.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::grid::{
	self as schema,
	fid,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Alignment,
	Dash,
	DashItem,
	Dict,
	LineCap,
	LineJoin,
	Length,
	Paint,
	Ratio,
	Relative,
	Stroke,
	Value,
};
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::{
	BoxNode,
	Dims,
	DrawOp,
	Glue,
	Graphic,
	Leaf,
	Node,
	Penalty,
	Sp,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
		PathBuilder,
		Pt,
	},
};

use std::sync::Arc;

/// A grid or table as vertical material: rows that may break across pages.
pub fn layout_grid(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Vec<Node>>
{
	let grid = res!(resolve(engine, elem, styles));
	let plan = res!(plan(engine, &grid, styles, region, &mut FlowCells));
	lower(engine, &grid, &plan)
}

// Cell layout seam

/// How a cell's content is measured and set. The grid owns the geometry; the content is the block flow's.
pub trait CellLayout {
	/// The natural size of a cell's content in `region`, which does not expand.
	fn measure(&mut self, engine: &mut Engine, cell: &Content, styles: &StyleChain, region: Region) -> Outcome<Dims>;

	/// The cell's content set into exactly its spanned area, as a vertical list.
	fn layout(&mut self, engine: &mut Engine, cell: &Content, styles: &StyleChain, region: Region) -> Outcome<Vec<Node>>;
}

/// Cells laid out by the block flow, through realisation, so show rules on the cell apply.
pub struct FlowCells;

impl CellLayout for FlowCells {
	fn measure(&mut self, engine: &mut Engine, cell: &Content, styles: &StyleChain, region: Region) -> Outcome<Dims> {
		crate::flow::measure(engine, cell, styles, region)
	}

	fn layout(&mut self, engine: &mut Engine, cell: &Content, styles: &StyleChain, region: Region) -> Outcome<Vec<Node>> {
		crate::flow::layout_block(engine, cell, styles, region)
	}
}

// Vocabulary

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing {
	Auto,
	Rel(Relative),
	Fr(f64),
}

impl Sizing {
	fn from_value(v: &Value) -> Option<Sizing> {
		match v {
			Value::Auto			=> Some(Sizing::Auto),
			Value::Length(l)	=> Some(Sizing::Rel(Relative { rel: Ratio(0.0), abs: *l })),
			Value::Ratio(r)		=> Some(Sizing::Rel(Relative { rel: *r, abs: Length::zero() })),
			Value::Relative(r)	=> Some(Sizing::Rel(*r)),
			Value::Fraction(f)	=> Some(Sizing::Fr(f.0)),
			_					=> None,
		}
	}

	fn is_auto(&self) -> bool { matches!(self, Sizing::Auto) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sides<T> {
	pub left:	T,
	pub top:	T,
	pub right:	T,
	pub bottom:	T,
}

impl<T: Clone> Sides<T> {
	pub fn splat(v: T) -> Self { Sides { left: v.clone(), top: v.clone(), right: v.clone(), bottom: v } }
}

/// A resolved pen: every part of a stroke settled, the thickness in scaled points.
#[derive(Clone, Debug)]
pub struct Pen {
	pub paint:		Paint,
	pub thickness:	Sp,
	pub cap:		LineCap,
	pub join:		LineJoin,
	pub dash:		Option<Dash>,
	pub miter:		f64,
}

impl Pen {
	fn same(&self, o: &Pen) -> bool {
		self.thickness == o.thickness && self.cap == o.cap && self.join == o.join && self.dash == o.dash
			&& self.miter == o.miter && same_paint(&self.paint, &o.paint)
	}
}

fn same_paint(a: &Paint, b: &Paint) -> bool {
	match (a, b) {
		(Paint::Color(x), Paint::Color(y))			=> x == y,
		(Paint::Gradient(x), Paint::Gradient(y))	=> x == y,
		(Paint::Tiling(x), Paint::Tiling(y))		=> Arc::ptr_eq(x, y),
		_											=> false,
	}
}

/// Which part of the grid a run of rows belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
	Body,
	Header { level: i64, repeat: bool },
	Footer { repeat: bool },
}

/// A header or footer: the content rows `start..end` it occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
	pub start:	usize,
	pub end:	usize,
	pub part:	Part,
}

/// A placed cell, in content coordinates, with its properties folded over the grid's.
#[derive(Clone, Debug)]
pub struct Cell {
	pub x:			usize,
	pub y:			usize,
	pub colspan:	usize,
	pub rowspan:	usize,
	pub elem:		Content,			// the cell element, resolved fields written in
	pub fill:		Option<Paint>,
	pub stroke:		Sides<Option<Stroke>>,
	pub prioritised:	Sides<bool>,	// the cell's own stroke set this side
	pub inset:		Sides<Relative>,
	pub align:		Alignment,
}

/// An explicit `hline` or `vline`, in track coordinates.
#[derive(Clone, Debug)]
pub struct Line {
	pub at:		usize,				// the track boundary it is drawn on
	pub start:	usize,				// first track it covers
	pub end:	usize,				// one past the last
	pub stroke:	Option<Stroke>,		// `None`: `stroke: none`, which removes cell strokes there
}

/// A grid with every cell placed. Tracks interleave gutters when any gutter is given, as in Typst: content
/// column `x` is track `2x` then.
#[derive(Clone, Debug)]
pub struct CellGrid {
	pub cols:		Vec<Sizing>,
	pub rows:		Vec<Sizing>,
	pub gutter:		bool,
	pub n_cols:		usize,
	pub n_rows:		usize,
	pub cells:		Vec<Cell>,
	pub hlines:		Vec<Line>,
	pub vlines:		Vec<Line>,
	pub sections:	Vec<Section>,	// headers and the footer, in row order
	pub font_pt:	f64,
	pub span:		Span,
	parents:		Vec<Option<usize>>,	// the cell covering each (column track, row track)
}

impl CellGrid {
	/// The track of content column or row `i`.
	pub fn track(&self, i: usize) -> usize { if self.gutter { 2 * i } else { i } }

	/// The tracks a span of `n` content columns or rows starting at `i` covers, gutters between included.
	pub fn track_span(&self, i: usize, n: usize) -> (usize, usize) {
		let a = self.track(i);
		(a, self.track(i + n - 1) + 1)
	}

	/// The cell covering a track position, when one does.
	pub fn parent(&self, xt: usize, yt: usize) -> Option<&Cell> {
		let w = self.cols.len();
		if xt >= w || yt >= self.rows.len() {
			return None;
		}
		self.parents.get(yt * w + xt).copied().flatten().and_then(|i| self.cells.get(i))
	}

	fn cell_tracks(&self, c: &Cell) -> ((usize, usize), (usize, usize)) {
		(self.track_span(c.x, c.colspan), self.track_span(c.y, c.rowspan))
	}
}

// Resolution

fn int_of(v: &Value) -> Option<i64> {
	match v {
		Value::Int(i)	=> Some(*i),
		_				=> None,
	}
}

/// The font size in force, for `em` lengths. `text.size` is read by name so the text schema stays U6a's.
pub fn font_size_pt(styles: &StyleChain) -> f64 {
	match ElemKind::Text.field_id("size").and_then(|id| styles.get(ElemKind::Text, id)) {
		Some(Value::Length(l))	=> l.resolve(11.0),
		_						=> 11.0,
	}
}

/// A per-cell property as Typst's `Celled`: a value, an array cycling by column, or a function of x and y.
fn celled(engine: &mut Engine, v: &Value, x: usize, y: usize, span: Span) -> Outcome<Value> {
	match v {
		Value::Func(f) => {
			let mut args = Args::new(span);
			args.push(span, Value::Int(x as i64));
			args.push(span, Value::Int(y as i64));
			engine.call_func(f, args)
		}
		Value::Array(a) => match a.len() {
			0	=> Ok(Value::None),
			n	=> Ok(a.get(x % n).cloned().unwrap_or(Value::None)),
		},
		other => Ok(other.clone()),
	}
}

fn paint_of(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Option<Paint>> {
	match v {
		Value::None | Value::Auto	=> Ok(None),
		Value::Color(c)				=> Ok(Some(Paint::Color(*c))),
		Value::Gradient(g)			=> Ok(Some(Paint::Gradient(g.clone()))),
		Value::Tiling(t)			=> Ok(Some(Paint::Tiling(t.clone()))),
		other => Err(engine.error(span, fmt!("expected color, gradient, tiling, or none, found {}", other.ty().name()))),
	}
}

/// A stroke-like value (length, paint or stroke) as a partial stroke.
fn stroke_of(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Stroke> {
	match v {
		Value::Length(l)	=> Ok(Stroke { thickness: Some(*l), ..Stroke::default() }),
		Value::Color(_) | Value::Gradient(_) | Value::Tiling(_)
			=> Ok(Stroke { paint: res!(paint_of(engine, span, v)), ..Stroke::default() }),
		Value::Stroke(s)	=> Ok((**s).clone()),
		Value::Dict(d)		=> stroke_dict(engine, span, d),
		other => Err(engine.error(span, fmt!(
			"expected length, color, gradient, tiling, dictionary, or stroke, found {}", other.ty().name()))),
	}
}

/// `(paint: .., thickness: .., cap: .., join: .., dash: .., miter-limit: ..)`, the dictionary form of a stroke.
fn stroke_dict(engine: &mut Engine, span: Span, d: &Dict) -> Outcome<Stroke> {
	let mut s = Stroke::default();
	for (k, v) in d.iter() {
		match k {
			"paint"			=> s.paint = res!(paint_of(engine, span, v)),
			"thickness"		=> match v {
				Value::Length(l)	=> s.thickness = Some(*l),
				Value::Auto			=> (),
				other => return Err(engine.error(span, fmt!("expected length, found {}", other.ty().name()))),
			},
			"cap"			=> s.cap = match v {
				Value::Str(t) if t.as_str() == "butt"	=> Some(LineCap::Butt),
				Value::Str(t) if t.as_str() == "round"	=> Some(LineCap::Round),
				Value::Str(t) if t.as_str() == "square"	=> Some(LineCap::Square),
				_ => None,
			},
			"join"			=> s.join = match v {
				Value::Str(t) if t.as_str() == "miter"	=> Some(LineJoin::Miter),
				Value::Str(t) if t.as_str() == "round"	=> Some(LineJoin::Round),
				Value::Str(t) if t.as_str() == "bevel"	=> Some(LineJoin::Bevel),
				_ => None,
			},
			"dash"			=> s.dash = Some(res!(dash_of(engine, span, v))),
			"miter-limit"	=> s.miter_limit = match v {
				Value::Float(f)	=> Some(*f),
				Value::Int(i)	=> Some(*i as f64),
				_				=> None,
			},
			other => return Err(engine.error(span, fmt!("unexpected key \"{}\", valid keys are \"paint\", \"thickness\", \"cap\", \"join\", \"dash\", and \"miter-limit\"", other))),
		}
	}
	Ok(s)
}

fn dash_of(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Option<Dash>> {
	let pattern = |a: &[f64]| a.iter().map(|x| DashItem::Len(Length::pt(*x))).collect::<Vec<_>>();
	let named = |s: &str| -> Option<Vec<DashItem>> {
		Some(match s {
			"solid"					=> return None,
			"dotted"				=> vec![DashItem::Dot, DashItem::Len(Length::pt(2.0))],
			"densely-dotted"		=> vec![DashItem::Dot, DashItem::Len(Length::pt(1.0))],
			"loosely-dotted"		=> vec![DashItem::Dot, DashItem::Len(Length::pt(4.0))],
			"dashed"				=> pattern(&[3.0, 3.0]),
			"densely-dashed"		=> pattern(&[3.0, 2.0]),
			"loosely-dashed"		=> pattern(&[3.0, 6.0]),
			"dash-dotted"			=> vec![DashItem::Len(Length::pt(3.0)), DashItem::Len(Length::pt(2.0)),
				DashItem::Dot, DashItem::Len(Length::pt(2.0))],
			"densely-dash-dotted"	=> vec![DashItem::Len(Length::pt(3.0)), DashItem::Len(Length::pt(1.0)),
				DashItem::Dot, DashItem::Len(Length::pt(1.0))],
			"loosely-dash-dotted"	=> vec![DashItem::Len(Length::pt(3.0)), DashItem::Len(Length::pt(4.0)),
				DashItem::Dot, DashItem::Len(Length::pt(4.0))],
			_						=> return None,
		})
	};
	match v {
		Value::None					=> Ok(None),
		Value::Str(s)				=> Ok(named(s).map(|array| Dash { array, phase: Length::zero() })),
		Value::Array(a)				=> {
			let mut array = Vec::new();
			for item in a.iter() {
				match item {
					Value::Length(l)						=> array.push(DashItem::Len(*l)),
					Value::Str(s) if s.as_str() == "dot"	=> array.push(DashItem::Dot),
					other => return Err(engine.error(span, fmt!("expected length or \"dot\", found {}", other.ty().name()))),
				}
			}
			Ok(Some(Dash { array, phase: Length::zero() }))
		}
		other => Err(engine.error(span, fmt!("expected string, array, dictionary, or none, found {}", other.ty().name()))),
	}
}

/// `a` over `b`, part by part: Typst's stroke fold.
fn fold_stroke(a: &Stroke, b: &Stroke) -> Stroke {
	Stroke {
		paint:			a.paint.clone().or_else(|| b.paint.clone()),
		thickness:		a.thickness.or(b.thickness),
		cap:			a.cap.or(b.cap),
		join:			a.join.or(b.join),
		dash:			a.dash.clone().or_else(|| b.dash.clone()),
		miter_limit:	a.miter_limit.or(b.miter_limit),
	}
}

/// A stroke side as written: unspecified (`None`), explicitly none (`Some(None)`), or a stroke.
type SideStroke = Option<Option<Stroke>>;

/// A stroke value by side: `none`, a stroke-like value for all four, or a dictionary of sides with `x`,
/// `y` and `rest`.
fn sides_stroke(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Sides<SideStroke>> {
	let one = |engine: &mut Engine, v: &Value| -> Outcome<SideStroke> {
		match v {
			Value::None	=> Ok(Some(None)),
			v			=> Ok(Some(Some(res!(stroke_of(engine, span, v))))),
		}
	};
	match v {
		Value::Auto	=> Ok(Sides::splat(None)),
		Value::Dict(d) if is_sides_dict(d) => {
			let mut s: Sides<SideStroke> = Sides::splat(None);
			let get = |engine: &mut Engine, k: &str| -> Outcome<SideStroke> {
				match d.get(k) {
					Some(v)	=> one(engine, v),
					None	=> Ok(None),
				}
			};
			let rest	= res!(get(engine, "rest"));
			let x		= res!(get(engine, "x"));
			let y		= res!(get(engine, "y"));
			s.left		= res!(get(engine, "left")).or_else(|| x.clone()).or_else(|| rest.clone());
			s.right		= res!(get(engine, "right")).or_else(|| x.clone()).or_else(|| rest.clone());
			s.top		= res!(get(engine, "top")).or_else(|| y.clone()).or_else(|| rest.clone());
			s.bottom	= res!(get(engine, "bottom")).or_else(|| y.clone()).or_else(|| rest.clone());
			Ok(s)
		}
		v => Ok(Sides::splat(res!(one(engine, v)))),
	}
}

/// `a` over `b` side by side, a stroke over a stroke part by part.
fn fold_sides(a: &Sides<SideStroke>, b: &Sides<SideStroke>) -> Sides<SideStroke> {
	let one = |a: &SideStroke, b: &SideStroke| match (a, b) {
		(Some(Some(x)), Some(Some(y)))	=> Some(Some(fold_stroke(x, y))),
		(Some(x), _)					=> Some(x.clone()),
		(None, y)						=> y.clone(),
	};
	Sides {
		left:	one(&a.left, &b.left),
		top:	one(&a.top, &b.top),
		right:	one(&a.right, &b.right),
		bottom:	one(&a.bottom, &b.bottom),
	}
}

fn is_sides_dict(d: &Dict) -> bool {
	d.keys().all(|k| matches!(k, "left" | "top" | "right" | "bottom" | "x" | "y" | "rest"))
}

fn sides_inset(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Sides<Option<Relative>>> {
	let rel = |engine: &mut Engine, v: &Value| -> Outcome<Option<Relative>> {
		match v {
			Value::Length(l)	=> Ok(Some(Relative { rel: Ratio(0.0), abs: *l })),
			Value::Ratio(r)		=> Ok(Some(Relative { rel: *r, abs: Length::zero() })),
			Value::Relative(r)	=> Ok(Some(*r)),
			Value::None			=> Ok(Some(Relative::default())),
			other => Err(engine.error(span, fmt!("expected relative length, found {}", other.ty().name()))),
		}
	};
	match v {
		Value::Auto	=> Ok(Sides::splat(None)),
		Value::Dict(d) => {
			let get = |engine: &mut Engine, k: &str| -> Outcome<Option<Relative>> {
				match d.get(k) {
					Some(v)	=> rel(engine, v),
					None	=> Ok(None),
				}
			};
			let rest	= res!(get(engine, "rest"));
			let x		= res!(get(engine, "x"));
			let y		= res!(get(engine, "y"));
			Ok(Sides {
				left:	res!(get(engine, "left")).or(x).or(rest),
				right:	res!(get(engine, "right")).or(x).or(rest),
				top:	res!(get(engine, "top")).or(y).or(rest),
				bottom:	res!(get(engine, "bottom")).or(y).or(rest),
			})
		}
		v => Ok(Sides::splat(res!(rel(engine, v)))),
	}
}

fn align_of(engine: &mut Engine, span: Span, v: &Value) -> Outcome<Alignment> {
	match v {
		Value::Auto | Value::None	=> Ok(Alignment::default()),
		Value::Alignment(a)			=> Ok(*a),
		other => Err(engine.error(span, fmt!("expected alignment or auto, found {}", other.ty().name()))),
	}
}

fn rel_value(r: Relative) -> Value {
	if r.rel.0 == 0.0 {
		Value::Length(r.abs)
	} else {
		Value::Relative(r)
	}
}

fn side_stroke_value(s: &Option<Stroke>) -> Value {
	match s {
		Some(s)	=> Value::Stroke(Arc::new(s.clone())),
		None	=> Value::None,
	}
}

/// Grows the occupancy map to `rows` rows.
fn ensure_rows(occ: &mut Vec<Option<usize>>, cols: usize, rows: usize) {
	if occ.len() < cols * rows {
		occ.resize(cols * rows, None);
	}
}

fn is_free(occ: &[Option<usize>], i: usize) -> bool {
	occ.get(i).map(|s| s.is_none()).unwrap_or(true)
}

/// A child cell's position and spans before its properties are resolved.
struct Placing {
	elem:		Content,
	x:			usize,
	y:			usize,
	colspan:	usize,
	rowspan:	usize,
}

/// Places one cell by Typst's rules: both coordinates fixed; x fixed, the first free row in that column;
/// y fixed, the first free column in that row; neither, the first free slot from the automatic index,
/// never wrapping a colspan to the next row.
#[allow(clippy::too_many_arguments)]
fn place_cell(
	engine:	&mut Engine,
	styles:	&StyleChain,
	c:		usize,
	occ:	&mut Vec<Option<usize>>,
	auto:	&mut usize,
	first:	usize,
	placed:	&mut Vec<Placing>,
	elem:	Content,
)
	-> Outcome<()>
{
	let span	= elem.span();
	let fx		= int_of(&schema::resolve(styles, &elem, fid::X));
	let fy		= int_of(&schema::resolve(styles, &elem, fid::Y));
	let colspan	= int_of(&schema::resolve(styles, &elem, fid::COLSPAN)).unwrap_or(1).max(1) as usize;
	let rowspan	= int_of(&schema::resolve(styles, &elem, fid::ROWSPAN)).unwrap_or(1).max(1) as usize;
	for v in [fx, fy].into_iter().flatten() {
		if v < 0 {
			return Err(engine.error(span, "number must be at least zero"));
		}
	}
	let index = match (fx, fy) {
		(Some(x), Some(y)) => {
			let (x, y) = (x as usize, y as usize);
			if x >= c {
				return Err(engine.error(span, fmt!("cell could not be placed at invalid column {}", x)));
			}
			let i = y * c + x;
			if !is_free(occ, i) {
				return Err(engine.error_hint(span,
					fmt!("attempted to place a second cell at column {}, row {}", x, y),
					"try specifying your cells in a different order"));
			}
			i
		}
		(Some(x), None) => {
			let x = x as usize;
			if x >= c {
				return Err(engine.error(span, fmt!("cell could not be placed at invalid column {}", x)));
			}
			// Within a header or footer the search starts at its first row.
			let mut y = first;
			while !is_free(occ, y * c + x) {
				y += 1;
			}
			y * c + x
		}
		(None, Some(y)) => {
			let y = y as usize;
			match (0..c).find(|x| is_free(occ, y * c + x)) {
				Some(x)	=> y * c + x,
				None	=> return Err(engine.error_hint(span,
					fmt!("cell could not be placed in row {} because it was full", y),
					"try specifying your cells in a different order")),
			}
		}
		(None, None) => {
			let mut i = *auto;
			while !is_free(occ, i) {
				i += 1;
			}
			i
		}
	};
	let (x, y) = (index % c, index / c);
	if colspan > c - x {
		return Err(engine.error_hint(span,
			"cell's colspan would cause it to exceed the available column(s)",
			"try placing the cell in another position or reducing its colspan"));
	}
	ensure_rows(occ, c, y + rowspan);
	for dy in 0..rowspan {
		for dx in 0..colspan {
			let j = (y + dy) * c + x + dx;
			if let Some(Some(_)) = occ.get(j) {
				return Err(engine.error_hint(span,
					fmt!("cell would span a previously placed cell at column {}, row {}", x + dx, y + dy),
					"try specifying your cells in a different order or reducing the cell's rowspan or colspan"));
			}
		}
	}
	let n = placed.len();
	for dy in 0..rowspan {
		for dx in 0..colspan {
			if let Some(s) = occ.get_mut((y + dy) * c + x + dx) {
				*s = Some(n);
			}
		}
	}
	if fx.is_none() && fy.is_none() {
		// A full-width cell leaves no room beside it, so its rowspan pushes the automatic index down too.
		*auto = if colspan == c { index + colspan * rowspan } else { index + colspan };
	}
	placed.push(Placing { elem, x, y, colspan, rowspan });
	Ok(())
}

/// An `hline` or `vline` as found, before the row count is known.
struct PendingLine {
	vertical:	bool,
	at:			usize,
	start:		usize,
	end:		Option<usize>,
	stroke:		Option<Stroke>,
	after:		bool,	// `position: bottom` or `end`: before the gutter rather than after it
	span:		Span,
}

fn pending_line(
	engine:		&mut Engine,
	styles:		&StyleChain,
	elem:		&Content,
	vertical:	bool,
	c:			usize,
	auto:		usize,
)
	-> Outcome<PendingLine>
{
	let span = elem.span();
	let at = match schema::resolve(styles, elem, fid::LINE_AT) {
		Value::Int(i) if i >= 0 => i as usize,
		Value::Int(_) => return Err(engine.error(span, "number must be at least zero")),
		// After the latest automatically positioned cell: below its row, or right of it.
		_ if vertical	=> if auto == 0 { 0 } else { (auto - 1) % c + 1 },
		_				=> auto.div_ceil(c),
	};
	let start = match schema::resolve(styles, elem, fid::LINE_START) {
		Value::Int(i) if i >= 0	=> i as usize,
		_						=> 0,
	};
	let end = match schema::resolve(styles, elem, fid::LINE_END) {
		Value::Int(i) if i >= 0	=> Some(i as usize),
		_						=> None,
	};
	// A line's stroke is not folded with its default: parts it leaves unset come from the cell strokes it
	// overrides (`hline(stroke: red)` over a 1.5pt cell edge draws red at 1.5pt, as in Typst).
	let stroke = match schema::resolve(styles, elem, fid::LINE_STROKE) {
		Value::None	=> None,
		v			=> Some(res!(stroke_of(engine, span, &v))),
	};
	let after = match schema::resolve(styles, elem, fid::LINE_POS) {
		Value::Alignment(a) => match (vertical, a.x, a.y) {
			(true, Some(crate::eval::value::HAlign::End), _)			=> true,
			(true, Some(crate::eval::value::HAlign::Right), _)		=> true,
			(false, _, Some(crate::eval::value::VAlign::Bottom))		=> true,
			_														=> false,
		},
		_ => false,
	};
	Ok(PendingLine { vertical, at, start, end, stroke, after, span })
}

/// Places every child and resolves every cell's properties.
pub fn resolve(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<CellGrid> {
	let span	= elem.span();
	let kind	= match elem.kind() {
		Some(k @ (ElemKind::Grid | ElemKind::Table))	=> k,
		_ => return Err(err!("layout_grid expects a grid or table element, found {:?}", elem.kind(); Input, Invalid)),
	};
	let (_, head_k, foot_k, hline_k, vline_k) = schema::family_of(kind);
	let font_pt = font_size_pt(styles);

	let sizings = |engine: &mut Engine, id| -> Outcome<Vec<Sizing>> {
		match schema::resolve(styles, elem, id) {
			Value::Array(a) => {
				let mut out = Vec::new();
				for v in a.iter() {
					match Sizing::from_value(v) {
						Some(s)	=> out.push(s),
						None	=> return Err(engine.error(span, fmt!(
							"expected auto, relative length, or fraction, found {}", v.ty().name()))),
					}
				}
				Ok(out)
			}
			Value::Int(n) if n >= 0	=> Ok(vec![Sizing::Auto; n as usize]),
			v => match Sizing::from_value(&v) {
				Some(s)	=> Ok(vec![s]),
				None	=> Ok(Vec::new()),
			},
		}
	};
	let col_t	= res!(sizings(engine, fid::COLUMNS));
	let row_t	= res!(sizings(engine, fid::ROWS));
	let col_g	= res!(sizings(engine, fid::COL_GUTTER));
	let row_g	= res!(sizings(engine, fid::ROW_GUTTER));
	let c		= col_t.len().max(1);

	let children = match schema::resolve(styles, elem, fid::CHILDREN) {
		Value::Array(a)	=> a,
		_				=> Arc::new(Vec::new()),
	};

	let mut occ:		Vec<Option<usize>>	= Vec::new();
	let mut placed:		Vec<Placing>		= Vec::new();
	let mut pending:	Vec<PendingLine>	= Vec::new();
	let mut sections:	Vec<Section>		= Vec::new();
	let mut auto		= 0usize;

	for child in children.iter() {
		let child = match child {
			Value::Content(ch)	=> ch.clone(),
			_					=> continue,
		};
		let k = match child.kind() {
			Some(k)	=> k,
			None	=> continue,	// construction wraps every non-element child in a cell
		};
		if k == hline_k || k == vline_k {
			pending.push(res!(pending_line(engine, styles, &child, k == vline_k, c, auto)));
		} else if k == head_k || k == foot_k {
			let is_head = k == head_k;
			// A header or footer starts on the first wholly free row at or after the automatic position.
			let mut row = auto.div_ceil(c);
			while (0..c).any(|x| !is_free(&occ, row * c + x)) {
				row += 1;
			}
			let first		= placed.len();
			let mut local	= row * c;
			let at			= if is_head { fid::HEAD_CHILDREN } else { fid::FOOT_CHILDREN };
			let parts = match schema::resolve(styles, &child, at) {
				Value::Array(a)	=> a,
				_				=> Arc::new(Vec::new()),
			};
			for p in parts.iter() {
				if let Value::Content(pc) = p {
					match pc.kind() {
						Some(pk) if pk == hline_k || pk == vline_k => pending.push(
							res!(pending_line(engine, styles, pc, pk == vline_k, c, local))),
						_ => res!(place_cell(engine, styles, c, &mut occ, &mut local, row, &mut placed, pc.clone())),
					}
				}
			}
			let end = placed[first..].iter().map(|p| p.y + p.rowspan).max().unwrap_or(row).max(row);
			let start = placed[first..].iter().map(|p| p.y).min().unwrap_or(row).min(row);
			let repeat = matches!(schema::resolve(styles, &child, fid::REPEAT), Value::Bool(true));
			let part = if is_head {
				let level = int_of(&schema::resolve(styles, &child, fid::LEVEL)).unwrap_or(1);
				Part::Header { level, repeat }
			} else {
				Part::Footer { repeat }
			};
			sections.push(Section { start, end, part });
			auto = end * c;
		} else {
			res!(place_cell(engine, styles, c, &mut occ, &mut auto, 0, &mut placed, child));
		}
	}

	let mut n_rows = row_t.len();
	for p in &placed {
		n_rows = n_rows.max(p.y + p.rowspan);
	}
	for s in &sections {
		n_rows = n_rows.max(s.end);
	}
	for s in &sections {
		if let Part::Footer { .. } = s.part {
			if s.end != n_rows {
				return Err(engine.error(span, "footer must end at the last row"));
			}
		}
	}

	// Every slot no cell covers holds an empty cell, as in Typst: it takes the grid's inset, fill and
	// stroke like any other.
	let (cell_k, ..) = schema::family_of(kind);
	ensure_rows(&mut occ, c, n_rows);
	for i in 0..c * n_rows {
		if is_free(&occ, i) {
			let empty = Content::new(cell_k, vec![(fid::BODY, Value::Content(Content::empty()))], span);
			placed.push(Placing { elem: empty, x: i % c, y: i / c, colspan: 1, rowspan: 1 });
		}
	}

	// Tracks: a missing size repeats the last given, else auto for content and zero for gutter.
	let gutter = !col_g.is_empty() || !row_g.is_empty();
	let zero = Sizing::Rel(Relative::default());
	let pick = |v: &[Sizing], i: usize, d: Sizing| v.get(i).or(v.last()).copied().unwrap_or(d);
	let mut cols = Vec::new();
	for x in 0..c {
		cols.push(pick(&col_t, x, Sizing::Auto));
		if gutter && x + 1 < c {
			cols.push(pick(&col_g, x, zero));
		}
	}
	let mut rows = Vec::new();
	for y in 0..n_rows {
		rows.push(pick(&row_t, y, Sizing::Auto));
		if gutter && y + 1 < n_rows {
			rows.push(pick(&row_g, y, zero));
		}
	}

	// Properties: the cell's own value, else the grid's resolved at the cell's position.
	let g_fill		= schema::resolve(styles, elem, fid::FILL);
	let g_align		= schema::resolve(styles, elem, fid::ALIGN);
	let g_inset		= schema::resolve(styles, elem, fid::INSET);
	let g_stroke	= schema::resolve(styles, elem, fid::STROKE);
	// A table's side left unspecified is a stroke with no part set: drawn as `1pt + black`, but in a fold
	// at a shared edge it yields every part to the neighbour's specified stroke. So `stroke: (bottom:
	// olive)` draws olive between rows and black only where no neighbour speaks, as Typst does.
	let stroke_default: Sides<SideStroke> = match kind {
		ElemKind::Table	=> Sides::splat(Some(Some(Stroke::default()))),
		_				=> Sides::splat(None),
	};
	let mut cells = Vec::with_capacity(placed.len());
	for p in placed {
		let cspan	= p.elem.span();
		let mut el	= p.elem;
		let fill = match schema::resolve(styles, &el, fid::CELL_FILL) {
			Value::Auto	=> {
				let v = res!(celled(engine, &g_fill, p.x, p.y, cspan));
				res!(paint_of(engine, cspan, &v))
			}
			v => res!(paint_of(engine, cspan, &v)),
		};
		let own_a	= res!(align_of(engine, cspan, &schema::resolve(styles, &el, fid::CELL_ALIGN)));
		let grid_a	= {
			let v = res!(celled(engine, &g_align, p.x, p.y, cspan));
			res!(align_of(engine, cspan, &v))
		};
		let align = Alignment { x: own_a.x.or(grid_a.x), y: own_a.y.or(grid_a.y) };
		let own_i	= res!(sides_inset(engine, cspan, &schema::resolve(styles, &el, fid::CELL_INSET)));
		let grid_i	= {
			let v = res!(celled(engine, &g_inset, p.x, p.y, cspan));
			res!(sides_inset(engine, cspan, &v))
		};
		let inset = Sides {
			left:	own_i.left.or(grid_i.left).unwrap_or_default(),
			top:	own_i.top.or(grid_i.top).unwrap_or_default(),
			right:	own_i.right.or(grid_i.right).unwrap_or_default(),
			bottom:	own_i.bottom.or(grid_i.bottom).unwrap_or_default(),
		};
		let own_s	= res!(sides_stroke(engine, cspan, &schema::resolve(styles, &el, fid::CELL_STROKE)));
		let grid_s	= {
			let v = res!(celled(engine, &g_stroke, p.x, p.y, cspan));
			let s = res!(sides_stroke(engine, cspan, &v));
			match &g_stroke {
				Value::Func(_) | Value::Array(_)	=> s,
				_									=> fold_sides(&s, &stroke_default),
			}
		};
		let folded = fold_sides(&own_s, &grid_s);
		let stroke = Sides {
			left:	folded.left.flatten(),
			top:	folded.top.flatten(),
			right:	folded.right.flatten(),
			bottom:	folded.bottom.flatten(),
		};
		let prio = |s: &SideStroke| matches!(s, Some(Some(_)));
		let prioritised = Sides {
			left:	prio(&own_s.left),
			top:	prio(&own_s.top),
			right:	prio(&own_s.right),
			bottom:	prio(&own_s.bottom),
		};

		// Written back, so `show table.cell: it => ..` reads the resolved values as in Typst.
		el.set(fid::X,			Value::Int(p.x as i64));
		el.set(fid::Y,			Value::Int(p.y as i64));
		el.set(fid::COLSPAN,	Value::Int(p.colspan as i64));
		el.set(fid::ROWSPAN,	Value::Int(p.rowspan as i64));
		el.set(fid::CELL_FILL,	match &fill {
			Some(Paint::Color(c))		=> Value::Color(*c),
			Some(Paint::Gradient(g))	=> Value::Gradient(g.clone()),
			Some(Paint::Tiling(t))		=> Value::Tiling(t.clone()),
			None						=> Value::None,
		});
		el.set(fid::CELL_ALIGN, if align == Alignment::default() { Value::Auto } else { Value::Alignment(align) });
		let mut d = Dict::new();
		d.insert("top",		rel_value(inset.top));
		d.insert("right",	rel_value(inset.right));
		d.insert("bottom",	rel_value(inset.bottom));
		d.insert("left",	rel_value(inset.left));
		el.set(fid::CELL_INSET, Value::dict(d));
		let mut d = Dict::new();
		d.insert("top",		side_stroke_value(&stroke.top));
		d.insert("right",	side_stroke_value(&stroke.right));
		d.insert("bottom",	side_stroke_value(&stroke.bottom));
		d.insert("left",	side_stroke_value(&stroke.left));
		el.set(fid::CELL_STROKE, Value::dict(d));

		cells.push(Cell {
			x: p.x, y: p.y, colspan: p.colspan, rowspan: p.rowspan, elem: el,
			fill, stroke, prioritised, inset, align,
		});
	}

	let mut grid = CellGrid {
		cols, rows, gutter, n_cols: c, n_rows, cells,
		hlines: Vec::new(), vlines: Vec::new(), sections, font_pt, span, parents: Vec::new(),
	};

	// Parents over tracks.
	let (w, h) = (grid.cols.len(), grid.rows.len());
	let mut parents = vec![None; w * h];
	for (i, cell) in grid.cells.iter().enumerate() {
		let ((x0, x1), (y0, y1)) = grid.cell_tracks(cell);
		for yt in y0..y1 {
			for xt in x0..x1 {
				if let Some(s) = parents.get_mut(yt * w + xt) {
					*s = Some(i);
				}
			}
		}
	}
	grid.parents = parents;

	// Lines into track coordinates.
	for l in pending {
		let (limit, other) = if l.vertical { (c, n_rows) } else { (n_rows, c) };
		if l.at > limit {
			let what = if l.vertical { "vertical line at invalid column" } else { "horizontal line at invalid row" };
			return Err(engine.error(l.span, fmt!("cannot place {} {}", what, l.at)));
		}
		let end = l.end.unwrap_or(other).min(other);
		if l.start >= end {
			continue;
		}
		let at = if l.at == limit {
			if l.vertical { grid.cols.len() } else { grid.rows.len() }
		} else if l.after {
			// `bottom` or `end`: after row or column `at`, before the gutter that follows it.
			grid.track(l.at) + 1
		} else {
			grid.track(l.at)
		};
		let (s, e) = grid.track_span(l.start, end - l.start);
		let line = Line { at, start: s, end: e, stroke: l.stroke };
		if l.vertical {
			grid.vlines.push(line);
		} else {
			grid.hlines.push(line);
		}
	}
	Ok(grid)
}

// Planning

/// A cell set at its place, in the grid's own frame (top left origin, y down).
#[derive(Clone, Debug)]
pub struct PlacedCell {
	pub index:	usize,	// into `CellGrid::cells`
	pub x:		Sp,
	pub y:		Sp,
	pub w:		Sp,
	pub h:		Sp,
	pub nodes:	Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct FillRect {
	pub x:		Sp,
	pub y:		Sp,
	pub w:		Sp,
	pub h:		Sp,
	pub paint:	Paint,
}

/// One drawn line segment, its ends already extended by half its thickness as Typst draws them.
#[derive(Clone, Debug)]
pub struct Segment {
	pub vertical:	bool,
	pub at:			Sp,		// x of a vertical segment, y of a horizontal one
	pub from:		Sp,
	pub to:			Sp,
	pub pen:		Pen,
}

/// Rows that stay together: tracks `start..end`, never split by a page break.
#[derive(Clone, Debug)]
pub struct Group {
	pub start:		usize,
	pub end:		usize,
	pub part:		Part,
	pub top:		Sp,
	pub height:		Sp,
	pub cells:		Vec<PlacedCell>,
	pub fills:		Vec<FillRect>,
	pub lines:		Vec<Segment>,
}

/// The grid laid out: track sizes and the row groups, in the grid's frame.
#[derive(Clone, Debug)]
pub struct GridPlan {
	pub width:	Sp,
	pub cols:	Vec<Sp>,
	pub rows:	Vec<Sp>,
	pub groups:	Vec<Group>,
}

fn rel_sp(r: &Relative, whole: Sp, font_pt: f64) -> Sp {
	Sp::from_pt(r.resolve(whole.to_pt(), font_pt))
}

fn sum(v: &[Sp]) -> Sp {
	v.iter().fold(Sp::ZERO, |a, b| a + *b)
}

fn share(total: Sp, part: f64, whole: f64) -> Sp {
	if whole <= 0.0 {
		return Sp::ZERO;
	}
	Sp((total.0 as f64 * part / whole).round() as i32)
}

/// Sizes every track and sets every cell, then groups the rows and resolves fills and lines.
pub fn plan<L: CellLayout>(
	engine:	&mut Engine,
	grid:	&CellGrid,
	styles:	&StyleChain,
	region:	Region,
	cells:	&mut L,
)
	-> Outcome<GridPlan>
{
	let fpt		= grid.font_pt;
	let cols	= res!(size_columns(engine, grid, styles, region, cells));
	let width	= sum(&cols);
	let rows	= res!(size_rows(engine, grid, styles, region, &cols, cells));

	let mut xs = vec![Sp::ZERO];
	for w in &cols {
		let last = xs.last().copied().unwrap_or(Sp::ZERO);
		xs.push(last + *w);
	}
	let mut ys = vec![Sp::ZERO];
	for h in &rows {
		let last = ys.last().copied().unwrap_or(Sp::ZERO);
		ys.push(last + *h);
	}
	let at = |v: &[Sp], i: usize| v.get(i).copied().unwrap_or(Sp::ZERO);

	// Groups: header and footer sections whole, then body rows welded by any rowspan crossing them. A
	// gutter track between groups belongs to neither: it is the glue the page may break at.
	let mut groups: Vec<Group> = Vec::new();
	let section_of = |yt: usize| grid.sections.iter().find(|s| {
		let (a, b) = grid.track_span(s.start, (s.end - s.start).max(1));
		s.end > s.start && yt >= a && yt < b
	});
	let mut yt = 0;
	while yt < grid.rows.len() {
		if grid.gutter && yt % 2 == 1 {
			yt += 1;
			continue;
		}
		let (mut end, part) = match section_of(yt) {
			Some(s)	=> (grid.track_span(s.start, s.end - s.start).1, s.part),
			None	=> (yt + 1, Part::Body),
		};
		// Extend over any rowspan that crosses the group's lower boundary.
		loop {
			let mut grown = end;
			for c in &grid.cells {
				let (_, (y0, y1)) = grid.cell_tracks(c);
				if y0 < end && y1 > end && y0 >= yt {
					grown = grown.max(y1);
				}
			}
			if grown == end {
				break;
			}
			end = grown;
		}
		groups.push(Group {
			start: yt, end, part, top: at(&ys, yt), height: at(&ys, end) - at(&ys, yt),
			cells: Vec::new(), fills: Vec::new(), lines: Vec::new(),
		});
		yt = end;
	}

	for g in groups.iter_mut() {
		for (i, c) in grid.cells.iter().enumerate() {
			let ((x0, x1), (y0, y1)) = grid.cell_tracks(c);
			if y0 < g.start || y0 >= g.end {
				continue;
			}
			let (x, y) = (at(&xs, x0), at(&ys, y0));
			let (w, h) = (at(&xs, x1) - x, at(&ys, y1) - y);
			if let Some(paint) = &c.fill {
				g.fills.push(FillRect { x, y, w, h, paint: paint.clone() });
			}
			let r = Region { width: w, height: h, base: (w, h), expand_x: true, expand_y: true };
			let nodes = res!(cells.layout(engine, &c.elem, styles, r));
			g.cells.push(PlacedCell { index: i, x, y, w, h, nodes });
		}
		// Vertical lines first, horizontal over them, as Typst draws. A group also inks the row gutter below
		// it, which only an explicit line crosses: the gutter is glue, and glue carries no ink.
		let mut ink_end = g.end;
		while ink_end < grid.rows.len() && grid.gutter && ink_end % 2 == 1 {
			ink_end += 1;
		}
		for bx in 0..=grid.cols.len() {
			let mut run: Option<(usize, Pen)> = None;
			for yt in g.start..=ink_end {
				let pen = if yt < ink_end { vline_pen(grid, bx, yt, fpt) } else { None };
				run = flush(run, pen, yt, &mut g.lines, |a, b, p| Segment {
					vertical: true, at: at(&xs, bx),
					from: at(&ys, a) - half(p.thickness), to: at(&ys, b) + half(p.thickness), pen: p,
				});
			}
		}
		for by in g.start..=g.end {
			let mut run: Option<(usize, Pen)> = None;
			for xt in 0..=grid.cols.len() {
				let pen = if xt < grid.cols.len() { hline_pen(grid, by, xt, fpt) } else { None };
				run = flush(run, pen, xt, &mut g.lines, |a, b, p| Segment {
					vertical: false, at: at(&ys, by),
					from: at(&xs, a) - half(p.thickness), to: at(&xs, b) + half(p.thickness), pen: p,
				});
			}
		}
	}
	Ok(GridPlan { width, cols, rows, groups })
}

fn half(t: Sp) -> Sp { Sp(t.0 / 2) }

/// Extends the current run of equal pens, or closes it into a segment and starts another.
fn flush<F: Fn(usize, usize, Pen) -> Segment>(
	run:	Option<(usize, Pen)>,
	pen:	Option<Pen>,
	i:		usize,
	out:	&mut Vec<Segment>,
	make:	F,
)
	-> Option<(usize, Pen)>
{
	match (run, pen) {
		(Some((a, p)), Some(q)) if p.same(&q)	=> Some((a, p)),
		(Some((a, p)), next) => {
			out.push(make(a, i, p));
			next.map(|q| (i, q))
		}
		(None, next) => next.map(|q| (i, q)),
	}
}

fn pen_of(s: &Stroke, font_pt: f64) -> Option<Pen> {
	let thickness = Sp::from_pt(s.thickness.map(|l| l.resolve(font_pt)).unwrap_or(1.0));
	if thickness <= Sp::ZERO {
		return None;
	}
	Some(Pen {
		paint:		s.paint.clone().unwrap_or(Paint::Color(crate::eval::value::Color {
			space: crate::eval::value::ColorSpace::Luma, c: [0.0; 4], alpha: 1.0 })),
		thickness,
		cap:		s.cap.unwrap_or(LineCap::Butt),
		join:		s.join.unwrap_or(LineJoin::Miter),
		dash:		s.dash.clone().flatten(),
		miter:		s.miter_limit.unwrap_or(4.0),
	})
}

/// The two cells' strokes at a shared edge: the prioritised one over the other, the later cell winning a
/// tie; either alone when the other gives none.
fn cell_edge(first: Option<(Option<Stroke>, bool)>, second: Option<(Option<Stroke>, bool)>) -> Option<Stroke> {
	let (fs, fp) = first.unwrap_or((None, false));
	let (ss, sp) = second.unwrap_or((None, false));
	let (hi, lo) = if fp && !sp { (fs, ss) } else { (ss, fs) };
	match (hi, lo) {
		(Some(a), Some(b))	=> Some(fold_stroke(&a, &b)),
		(Some(a), None)		=> Some(a),
		(None, b)			=> b,
	}
}

/// The explicit line at a boundary covering a track, the last given winning.
fn explicit<'a>(lines: &'a [Line], at: usize, t: usize) -> Option<&'a Line> {
	lines.iter().rev().find(|l| l.at == at && t >= l.start && t < l.end)
}

fn with_line(line: Option<&Line>, cell: Option<Stroke>) -> Option<Stroke> {
	match line {
		Some(Line { stroke: Some(s), .. })	=> Some(match &cell {
			Some(c)	=> fold_stroke(s, c),
			None	=> s.clone(),
		}),
		Some(Line { stroke: None, .. })		=> None,
		None								=> cell,
	}
}

fn vline_pen(grid: &CellGrid, bx: usize, yt: usize, fpt: f64) -> Option<Pen> {
	let w = grid.cols.len();
	if bx != 0 && bx != w {
		// A colspan through this boundary hides the line there, explicit or not.
		// In a gutter the cell that decides is the one past a gutter column and above a gutter row.
		let (ax, ay) = if grid.gutter { (bx + bx % 2, yt - yt % 2) } else { (bx, yt) };
		if let Some(p) = grid.parent(ax, ay) {
			if grid.track(p.x) < bx {
				return None;
			}
		}
	}
	let left	= if bx > 0 { grid.parent(bx - 1, yt).map(|c| (c.stroke.right.clone(), c.prioritised.right)) } else { None };
	let right	= if bx < w { grid.parent(bx, yt).map(|c| (c.stroke.left.clone(), c.prioritised.left)) } else { None };
	let stroke	= with_line(explicit(&grid.vlines, bx, yt), cell_edge(left, right));
	stroke.and_then(|s| pen_of(&s, fpt))
}

fn hline_pen(grid: &CellGrid, by: usize, xt: usize, fpt: f64) -> Option<Pen> {
	let h = grid.rows.len();
	if by != 0 && by != h {
		let (ax, ay) = if grid.gutter { (xt - xt % 2, by + by % 2) } else { (xt, by) };
		if let Some(p) = grid.parent(ax, ay) {
			if grid.track(p.y) < by {
				return None;
			}
		}
	}
	let above	= if by > 0 { grid.parent(xt, by - 1).map(|c| (c.stroke.bottom.clone(), c.prioritised.bottom)) } else { None };
	let below	= if by < h { grid.parent(xt, by).map(|c| (c.stroke.top.clone(), c.prioritised.top)) } else { None };
	let stroke	= with_line(explicit(&grid.hlines, by, xt), cell_edge(above, below));
	stroke.and_then(|s| pen_of(&s, fpt))
}

/// Column widths: relative against the region's base, auto from the cells, fractions from what remains.
fn size_columns<L: CellLayout>(
	engine:	&mut Engine,
	grid:	&CellGrid,
	styles:	&StyleChain,
	region:	Region,
	cells:	&mut L,
)
	-> Outcome<Vec<Sp>>
{
	let fpt = grid.font_pt;
	let mut rcols	= vec![Sp::ZERO; grid.cols.len()];
	let mut rel		= Sp::ZERO;
	let mut fr		= 0.0;
	for (i, s) in grid.cols.iter().enumerate() {
		match s {
			Sizing::Auto	=> (),
			Sizing::Rel(r)	=> {
				rcols[i] = rel_sp(r, region.base.0, fpt);
				rel += rcols[i];
			}
			Sizing::Fr(f)	=> fr += f,
		}
	}
	let available = region.width - rel;
	if available < Sp::ZERO {
		return Ok(rcols);
	}

	// Auto columns, left to right; a colspan's excess over what it already covers lands on its last auto.
	let mut auto	= Sp::ZERO;
	let mut count	= 0usize;
	for x in 0..grid.cols.len() {
		if !grid.cols[x].is_auto() {
			continue;
		}
		let mut resolved = Sp::ZERO;
		for cell in &grid.cells {
			let ((x0, x1), (y0, y1)) = grid.cell_tracks(cell);
			if x < x0 || x >= x1 {
				continue;
			}
			let last_auto = (x0..x1).rev().find(|i| grid.cols[*i].is_auto());
			if last_auto != Some(x) {
				continue;
			}
			// A cell spanning only relative rows knows its height; otherwise the region's is the guess.
			let mut height	= Sp::ZERO;
			let mut fixed	= true;
			for yt in y0..y1 {
				match grid.rows.get(yt) {
					Some(Sizing::Rel(r))	=> height += rel_sp(r, region.base.1, fpt),
					_						=> fixed = false,
				}
			}
			if !fixed {
				height = region.base.1;
			}
			let r = Region { width: available, height, base: (available, height), expand_x: false, expand_y: false };
			let size = res!(cells.measure(engine, &cell.elem, styles, r));
			let covered = sum(&rcols[x0..x1]) - rcols[x];
			let need = size.width - covered;
			if need > resolved {
				resolved = need;
			}
		}
		rcols[x] = resolved;
		auto += resolved;
		count += 1;
	}

	let remaining = available - auto;
	if remaining >= Sp::ZERO {
		if fr > 0.0 {
			for (i, s) in grid.cols.iter().enumerate() {
				if let Sizing::Fr(f) = s {
					rcols[i] = share(remaining, *f, fr);
				}
			}
		}
	} else {
		shrink_auto(grid, &mut rcols, available, count);
	}
	Ok(rcols)
}

/// Typst's fair shrink: auto columns no wider than an equal share keep their width, the rest share what
/// is left equally, repeated until nothing changes.
fn shrink_auto(grid: &CellGrid, rcols: &mut [Sp], available: Sp, count: usize) {
	let mut last;
	let mut fair		= i64::MIN;
	let mut redistribute	= available.0 as i64;
	let mut overlarge	= count as i64;
	let mut changed		= true;
	while changed && overlarge > 0 {
		changed	= false;
		last	= fair;
		fair	= redistribute / overlarge;
		for (i, s) in grid.cols.iter().enumerate() {
			let w = rcols[i].0 as i64;
			if s.is_auto() && w <= fair && w > last {
				redistribute -= w;
				overlarge -= 1;
				changed = true;
			}
		}
	}
	for (i, s) in grid.cols.iter().enumerate() {
		if s.is_auto() && (rcols[i].0 as i64) > fair {
			rcols[i] = Sp(fair as i32);
		}
	}
}

/// Row heights: relative against the region's base, auto from the cells (a rowspan's excess on its last
/// auto row), fractions sharing what the region has left.
fn size_rows<L: CellLayout>(
	engine:	&mut Engine,
	grid:	&CellGrid,
	styles:	&StyleChain,
	region:	Region,
	cols:	&[Sp],
	cells:	&mut L,
)
	-> Outcome<Vec<Sp>>
{
	let fpt = grid.font_pt;
	let mut rrows	= vec![Sp::ZERO; grid.rows.len()];
	let mut fr		= 0.0;
	for (i, s) in grid.rows.iter().enumerate() {
		match s {
			Sizing::Rel(r)	=> rrows[i] = rel_sp(r, region.base.1, fpt),
			Sizing::Fr(f)	=> fr += f,
			Sizing::Auto	=> (),
		}
	}
	// Natural heights, measured once per cell at its spanned width.
	let mut natural = Vec::with_capacity(grid.cells.len());
	for cell in &grid.cells {
		let ((x0, x1), (y0, y1)) = grid.cell_tracks(cell);
		if !(y0..y1).any(|y| grid.rows[y].is_auto()) {
			natural.push(Sp::ZERO);
			continue;
		}
		let w = sum(&cols[x0..x1]);
		let r = Region { width: w, height: region.height, base: (w, region.base.1), expand_x: true, expand_y: false };
		let d = res!(cells.measure(engine, &cell.elem, styles, r));
		natural.push(d.vextent());
	}
	for y in 0..grid.rows.len() {
		if !grid.rows[y].is_auto() {
			continue;
		}
		let mut h = Sp::ZERO;
		for (cell, nat) in grid.cells.iter().zip(&natural) {
			let (_, (y0, y1)) = grid.cell_tracks(cell);
			if y < y0 || y >= y1 {
				continue;
			}
			let last_auto = (y0..y1).rev().find(|i| grid.rows[*i].is_auto());
			if last_auto != Some(y) {
				continue;
			}
			let covered = sum(&rrows[y0..y1]) - rrows[y];
			let need = *nat - covered;
			if need > h {
				h = need;
			}
		}
		rrows[y] = h;
	}
	if fr > 0.0 {
		let remaining = region.height - sum(&rrows);
		if remaining > Sp::ZERO {
			for (i, s) in grid.rows.iter().enumerate() {
				if let Sizing::Fr(f) = s {
					rrows[i] = share(remaining, *f, fr);
				}
			}
		}
	}
	Ok(rrows)
}

// Lowering

fn rgba(engine: &mut Engine, span: Span, p: &Paint) -> Outcome<Rgba> {
	match p {
		Paint::Color(c)		=> c.to_rgba(),
		Paint::Gradient(g)	=> {
			// `DrawOp` carries a flat colour only; the first stop stands in, and the loss is said.
			engine.warn(span, "gradients in grid fills and strokes are drawn in their first colour");
			match g.stops.first() {
				Some((c, _))	=> c.to_rgba(),
				None			=> Ok(Rgba::BLACK),
			}
		}
		Paint::Tiling(_)	=> {
			engine.warn(span, "tilings in grid fills and strokes are not drawn yet");
			Ok(Rgba::TRANSPARENT)
		}
	}
}

fn f(sp: Sp) -> f32 { sp.to_pt() as f32 }

fn segment_path(s: &Segment) -> Outcome<Path> {
	let mut b = PathBuilder::new();
	if s.vertical {
		b.move_to(Pt::new(f(s.at), f(s.from)));
		b.line_to(Pt::new(f(s.at), f(s.to)));
	} else {
		b.move_to(Pt::new(f(s.from), f(s.at)));
		b.line_to(Pt::new(f(s.to), f(s.at)));
	}
	b.finish()
}

/// A drawing the size of a group's box, placed at its top left, which the vertical list then steps back
/// over so what follows starts at the same top.
fn overlay(ops: Vec<DrawOp>, w: Sp, h: Sp, list: &mut Vec<Node>) {
	if ops.is_empty() {
		return;
	}
	list.push(Node::Leaf(Leaf::graphic(Graphic::new(ops, Dims::new(w, h, Sp::ZERO)))));
	list.push(Node::Glue(Glue::fixed(-h)));
}

/// One row group as an unbreakable box: its fills, then each cell's content at its place, then its lines.
fn group_box(engine: &mut Engine, grid: &CellGrid, plan: &GridPlan, g: &Group) -> Outcome<BoxNode> {
	let span	= grid.span;
	let w		= plan.width;
	let mut list = Vec::new();

	let mut ops = Vec::new();
	for r in &g.fills {
		let c = res!(rgba(engine, span, &r.paint));
		let y = r.y - g.top;
		let path = res!(Path::rect(Bounds::new(f(r.x), f(y), f(r.x + r.w), f(y + r.h))));
		ops.push(DrawOp::Fill { path, colour: c });
	}
	overlay(ops, w, g.height, &mut list);

	// Each cell stepped to its place: down, across, its box, then back up to the group's top.
	for pc in &g.cells {
		let y = pc.y - g.top;
		list.push(Node::Glue(Glue::fixed(y)));
		let cell = Node::VBox(BoxNode::new(pc.nodes.clone(), Dims::new(pc.w, pc.h, Sp::ZERO)));
		list.push(Node::HBox(BoxNode::new(
			vec![Node::Glue(Glue::fixed(pc.x)), cell],
			Dims::new(pc.x + pc.w, pc.h, Sp::ZERO))));
		list.push(Node::Glue(Glue::fixed(-(y + pc.h))));
	}

	let mut ops = Vec::new();
	for s in &g.lines {
		if s.pen.dash.is_some() {
			engine.warn(span, "dashed grid lines are drawn solid");
		}
		let c = res!(rgba(engine, span, &s.pen.paint));
		let mut s2 = s.clone();
		if !s.vertical {
			s2.at = s.at - g.top;
		} else {
			s2.from = s.from - g.top;
			s2.to = s.to - g.top;
		}
		ops.push(DrawOp::Stroke { path: res!(segment_path(&s2)), colour: c, width: f(s.pen.thickness) });
	}
	overlay(ops, w, g.height, &mut list);
	list.push(Node::Glue(Glue::fixed(g.height)));
	Ok(BoxNode::new(list, Dims::new(w, g.height, Sp::ZERO)))
}

/// The plan as vertical material. Rows break between groups only; a header is welded to the row after it
/// and armed to repeat, a subheader replacing any header of its level or deeper; the footer is welded to
/// the row before it.
pub fn lower(engine: &mut Engine, grid: &CellGrid, plan: &GridPlan) -> Outcome<Vec<Node>> {
	let mut out: Vec<Node> = Vec::new();
	let mut active: Vec<(i64, BoxNode)> = Vec::new();
	let mut armed	= false;
	let mut weld	= false;
	let mut prev_end: Option<usize> = None;
	for g in &plan.groups {
		let bx = res!(group_box(engine, grid, plan, g));
		if let Some(pe) = prev_end {
			if matches!(g.part, Part::Footer { .. }) || weld {
				out.push(Node::Penalty(Penalty::new(Penalty::INFINITY, false)));
			}
			let gap = sum(&plan.rows[pe..g.start]);
			out.push(Node::Glue(Glue::fixed(gap)));
		}
		weld = false;
		out.push(Node::VBox(bx.clone()));
		if let Part::Header { level, repeat } = g.part {
			weld = true;
			active.retain(|(l, _)| *l < level);
			if repeat {
				active.push((level, bx));
			}
			if !active.is_empty() {
				let h = active.iter().fold(Sp::ZERO, |a, (_, b)| a + b.dims.height);
				let list = active.iter().map(|(_, b)| Node::VBox(b.clone())).collect();
				out.push(Node::RepeatHead(Some(Box::new(BoxNode::new(list, Dims::new(plan.width, h, Sp::ZERO))))));
				armed = true;
			} else if armed {
				out.push(Node::RepeatHead(None));
				armed = false;
			}
		}
		prev_end = Some(g.end);
	}
	if armed {
		out.push(Node::RepeatHead(None));
	}
	Ok(out)
}
