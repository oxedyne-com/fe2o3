// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `shapes.rs`, `transforms.rs`, `image.rs` and `repeat.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6d owns this file: images, shapes, curves and transforms into one atomic box each, over `image.rs`,
// `fe2o3_graphics` paths and `ir::TransformNode`/`ir::ClipNode`.
//
// Layout follows Typst 0.15's `typst-layout` (shapes.rs, transforms.rs, image.rs, repeat.rs) closely enough
// that the geometry can be checked against the `typst` oracle's SVG output shape for shape. Each element is
// laid out, as Typst lays it out, into a `Frame` of positioned items in points (y down), which is then
// lowered to IR: ink becomes one `Graphic` leaf, a laid-out body stays as nodes, and the two are overlaid
// in a vertical box whose layers back up over one another with negative glue.
//
// A transformed or clipped frame lowers to a `Node::Transform` or `Node::Clip` holding its material, which the
// driver places as a group; a raster's cover crop is still done on its pixels.
//
// An infinite region extent is `Sp(i32::MAX)` (anything from 2^30 up reads as infinite).

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::visual::{
	self as vis,
	fid,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	DashItem,
	HAlign,
	LineCap,
	LineJoin,
	Paint,
	Relative,
	Stroke,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::flow::Region;
use crate::image::{
	self as img,
	Decoded,
};
use crate::ir::{
	BoxNode,
	ClipNode,
	Dims,
	DrawOp,
	Glue,
	Graphic,
	LeafKind,
	Leaf,
	Node,
	RasterImage,
	Sp,
	Transform,
	TransformNode,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Ink,
	path::{
		Path,
		PathBuilder,
		Pt,
	},
	stroke::{
		Cap,
		Dash as PenDash,
		Join,
		Stroke as Pen,
	},
	svg_doc::SvgOp,
	transform::Transform as GTransform,
};

use std::sync::Arc;

const INF_SP:		i32	= 1 << 30;	// region extents from here up are infinite
const STROKE_TOL:	f32	= 0.01;		// flattening tolerance of a stroke outline, points
const RAW_UNIT:		f64	= 1.0 / 127.0;	// Typst's raw length unit in points: a round cap's centre nudge

// Geometry

/// A point in points, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P2 {
	pub x:	f64,
	pub y:	f64,
}

impl P2 {
	pub fn new(x: f64, y: f64) -> Self { Self { x, y } }
	fn add(self, o: P2) -> P2 { P2::new(self.x + o.x, self.y + o.y) }
	fn sub(self, o: P2) -> P2 { P2::new(self.x - o.x, self.y - o.y) }
	fn mul(self, k: f64) -> P2 { P2::new(self.x * k, self.y * k) }
	fn hypot(self) -> f64 { self.x.hypot(self.y) }
	fn apply(self, t: &Transform) -> P2 {
		P2::new(t.a * self.x + t.c * self.y + t.e, t.b * self.x + t.d * self.y + t.f)
	}
}

/// One step of a curve. Quadratics are raised to cubics as they are added, as Typst does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
	Move(P2),
	Line(P2),
	Cubic(P2, P2, P2),
	Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve(pub Vec<Seg>);

impl Curve {
	pub fn new() -> Self { Self::default() }
	pub fn move_(&mut self, p: P2) { self.0.push(Seg::Move(p)); }
	pub fn line(&mut self, p: P2) { self.0.push(Seg::Line(p)); }
	pub fn cubic(&mut self, a: P2, b: P2, p: P2) { self.0.push(Seg::Cubic(a, b, p)); }
	pub fn close(&mut self) { self.0.push(Seg::Close); }
	pub fn is_empty(&self) -> bool { self.0.is_empty() }

	pub fn rect(w: f64, h: f64) -> Self {
		let mut c = Curve::new();
		c.move_(P2::new(0.0, 0.0));
		c.line(P2::new(w, 0.0));
		c.line(P2::new(w, h));
		c.line(P2::new(0.0, h));
		c.close();
		c
	}

	/// Typst's ellipse inscribed in a `w` by `h` box at the origin.
	pub fn ellipse(w: f64, h: f64) -> Self {
		let rx	= w / 2.0;
		let ry	= h / 2.0;
		let m	= 0.551784;
		let mx	= m * rx;
		let my	= m * ry;
		let p	= |x: f64, y: f64| P2::new(x + rx, y + ry);
		let mut c = Curve::new();
		c.move_(p(-rx, 0.0));
		c.cubic(p(-rx, -my), p(-mx, -ry), p(0.0, -ry));
		c.cubic(p(mx, -ry), p(rx, -my), p(rx, 0.0));
		c.cubic(p(rx, my), p(mx, ry), p(0.0, ry));
		c.cubic(p(-mx, ry), p(-rx, my), p(-rx, 0.0));
		c
	}


	/// The curve as a `fe2o3_graphics` path, offset by `at`.
	pub fn to_path(&self, at: P2) -> Outcome<Path> {
		let f = |p: P2| Pt::new((p.x + at.x) as f32, (p.y + at.y) as f32);
		let mut b = PathBuilder::new();
		for s in &self.0 {
			match *s {
				Seg::Move(p)		=> b.move_to(f(p)),
				Seg::Line(p)		=> b.line_to(f(p)),
				Seg::Cubic(a, c, p)	=> b.cubic_to(f(a), f(c), f(p)),
				Seg::Close			=> b.close(),
			}
		}
		b.finish()
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
	NonZero,
	EvenOdd,
}

/// A stroke with every part decided, lengths in points: Typst's `FixedStroke`.
#[derive(Clone, Debug)]
pub struct FixedStroke {
	pub paint:			Paint,
	pub thickness:		f64,
	pub cap:			LineCap,
	pub join:			LineJoin,
	pub dash:			Option<(Vec<f64>, f64)>,	// pattern and phase
	pub miter_limit:	f64,
}

/// A curve with its fill and stroke.
#[derive(Clone, Debug)]
pub struct Shape {
	pub curve:		Curve,
	pub fill:		Option<Paint>,
	pub fill_rule:	FillRule,
	pub stroke:		Option<FixedStroke>,
}

/// What a frame holds at a position.
#[derive(Clone, Debug)]
pub enum Item {
	Shape(Shape),
	Raster { image: Arc<RasterImage>, w: f64, h: f64 },	// fills (0, 0) to (w, h) from its position
	Ops(Vec<DrawOp>),									// ready ink, in the frame's own coordinates
	Nodes { nodes: Vec<Node>, dims: Dims },				// laid-out content, its top-left at the position
	Group { frame: Box<Frame>, transform: Transform, clip: Option<Curve> },
}

/// Typst's frame: a sized box of positioned items. `baseline` is from the top; none means the bottom.
#[derive(Clone, Debug, Default)]
pub struct Frame {
	pub width:		f64,
	pub height:		f64,
	pub baseline:	Option<f64>,
	pub items:		Vec<(P2, Item)>,
}

impl Frame {
	pub fn new(width: f64, height: f64) -> Self { Self { width, height, baseline: None, items: Vec::new() } }

	pub fn push(&mut self, at: P2, item: Item) { self.items.push((at, item)); }

	/// Puts items behind everything already in the frame.
	pub fn prepend(&mut self, items: Vec<(P2, Item)>) {
		let mut v = items;
		v.append(&mut self.items);
		self.items = v;
	}

	pub fn translate(&mut self, d: P2) {
		if d.x == 0.0 && d.y == 0.0 {
			return;
		}
		if let Some(b) = &mut self.baseline {
			*b += d.y;
		}
		for (p, _) in &mut self.items {
			*p = p.add(d);
		}
	}

	/// Moves the contents but not the baseline.
	pub fn translate_visual(&mut self, d: P2) {
		for (p, _) in &mut self.items {
			*p = p.add(d);
		}
	}

	pub fn baseline(&self) -> f64 { self.baseline.unwrap_or(self.height) }

	/// Wraps the contents in a group drawn under `t`; the frame keeps its size.
	pub fn transform(&mut self, t: Transform) {
		if self.items.is_empty() {
			return;
		}
		let inner = Frame { width: self.width, height: self.height, baseline: None, items: std::mem::take(&mut self.items) };
		self.items.push((P2::default(), Item::Group { frame: Box::new(inner), transform: t, clip: None }));
	}

	pub fn clip(&mut self, c: Curve) {
		if self.items.is_empty() {
			return;
		}
		let inner = Frame { width: self.width, height: self.height, baseline: None, items: std::mem::take(&mut self.items) };
		self.items.push((P2::default(), Item::Group { frame: Box::new(inner), transform: Transform::identity(), clip: Some(c) }));
	}

	/// Resizes to `target`, placing the contents by `align` (0 start, 0.5 centre, 1 end, per axis).
	fn resize(&mut self, tw: f64, th: f64, ax: f64, ay: f64) {
		let d = P2::new((tw - self.width) * ax, (th - self.height) * ay);
		self.width	= tw;
		self.height	= th;
		self.translate(d);
	}

	/// Drops this frame's own ink, keeping laid-out content (whose ink its own layout decides) and size.
	fn hide(&mut self) {
		self.items.retain(|(_, i)| matches!(i, Item::Nodes { .. } | Item::Group { .. }));
		for (_, i) in &mut self.items {
			if let Item::Group { frame, .. } = i {
				frame.hide();
			}
		}
	}
}

/// The space an element's own layout sees: Typst's region, in points, possibly infinite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pod {
	pub w:			f64,
	pub h:			f64,
	pub expand_x:	bool,
	pub expand_y:	bool,
}

impl Pod {
	pub fn new(w: f64, h: f64, expand_x: bool, expand_y: bool) -> Self { Self { w, h, expand_x, expand_y } }

	fn finite(&self) -> bool { self.w.is_finite() && self.h.is_finite() }

	/// The pod as a flow region.
	pub fn region(&self) -> Region {
		let w = sp_of(self.w);
		let h = sp_of(self.h);
		Region { width: w, height: h, base: (w, h), expand_x: self.expand_x, expand_y: self.expand_y }
	}
}

fn pt_of(sp: Sp) -> f64 {
	if sp.0 >= INF_SP { f64::INFINITY } else { sp.to_pt() }
}

fn sp_of(pt: f64) -> Sp {
	if !pt.is_finite() || pt >= Sp(INF_SP).to_pt() { Sp(i32::MAX) } else { Sp::from_pt(pt) }
}

// Style access

/// The font size in points that resolves `em`: the chain's `text.size`, Typst's 11pt when none is set.
pub fn font_size(styles: &StyleChain) -> f64 { styles.font_size() }

/// A relative length against a whole, as Typst's `relative_to`: a ratio of an infinite whole is zero, not
/// NaN, so `0% + 5pt` in an unbounded region is 5pt.
pub fn rel_to(r: Relative, whole: f64, fs: f64) -> f64 {
	let part = r.rel.0 * whole;
	(if part.is_finite() { part } else { 0.0 }) + r.abs.resolve(fs)
}

struct Get<'a> {
	elem:	&'a Content,
	styles:	&'a StyleChain,
	kind:	ElemKind,
	fs:		f64,
}

impl<'a> Get<'a> {
	fn val(&self, name: &str) -> Outcome<Option<Value>> {
		let id = res!(fid(self.kind, name));
		self.styles.resolve(self.elem, id)
	}

	fn rel(&self, r: Relative, whole: f64) -> f64 { rel_to(r, whole, self.fs) }

	fn bool(&self, name: &str, default: bool) -> Outcome<bool> {
		match res!(self.val(name)) {
			Some(Value::Bool(b))	=> Ok(b),
			_						=> Ok(default),
		}
	}

	fn content(&self, name: &str) -> Outcome<Option<Content>> {
		match res!(self.val(name)) {
			None | Some(Value::None)	=> Ok(None),
			Some(Value::Content(c))		=> Ok(Some(c)),
			Some(Value::Str(s))			=> Ok(Some(Content::text(&s))),
			Some(other)					=> Ok(Some(res!(other.cast::<Content>()))),
		}
	}
}

// Entry points

/// A visual element as one box, placeable inline or as a block. Its body, if any, is laid out by
/// [`crate::flow::layout_block`].
pub fn layout_visual(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Node>
{
	let frame = res!(layout_frame(engine, elem, styles, region));
	frame_to_node(engine, &frame, elem.span())
}

/// The element laid out into a frame, its body through [`crate::flow::layout_block`].
pub fn layout_frame(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
)
	-> Outcome<Frame>
{
	layout_frame_with(engine, elem, styles, region, &mut layout_body)
}

/// Lays a body out in a pod as the flow does, and wraps the nodes as a frame.
pub fn layout_body(engine: &mut Engine, body: &Content, styles: &StyleChain, pod: Pod) -> Outcome<Frame> {
	let nodes	= res!(crate::flow::layout_block(engine, body, styles, pod.region()));
	let dims	= vlist_dims(&nodes);
	let w		= if pod.expand_x { pod.w } else { dims.width.to_pt() };
	let h		= if pod.expand_y { pod.h } else { dims.vextent().to_pt() };
	let mut f	= Frame::new(w, h);
	f.baseline	= first_baseline(&nodes).map(|b| b.to_pt());
	f.push(P2::default(), Item::Nodes { nodes, dims });
	Ok(f)
}

/// As [`layout_frame`], with the routine that lays out a body passed in: what the element does around its
/// body is this file's, and how content becomes a frame is the caller's.
pub fn layout_frame_with<F>(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	region:	Region,
	body:	&mut F,
)
	-> Outcome<Frame>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Err(err!("layout_visual was given a sequence, not an element."; Bug, Invalid)),
	};
	let g = Get { elem, styles, kind, fs: font_size(styles) };
	let span = elem.span();
	// The body is laid out, however many times, at the element's place.
	let place = elem.place();
	let mut at_place = |engine: &mut Engine, b: &Content, s: &StyleChain, pod: Pod| engine.within(place, |e| body(e, b, s, pod));
	let body = &mut at_place;

	// Every visual element is shown by Typst as a block whose single child is the element's layouter; the
	// block's `width` and `height` (shapes and images have them) decide the pod.
	let base_w	= pt_of(region.base.0);
	let base_h	= pt_of(region.base.1);
	let mut pod	= Pod::new(pt_of(region.width), pt_of(region.height), false, false);
	if kind.field_id("width").is_some() && kind != ElemKind::Hide {
		if let Some(Value::Relative(r)) = res!(g.val("width")).map(|v| v).and_then(|v| cast_opt_rel(v)) {
			pod.w = g.rel(r, base_w);
			pod.expand_x = true;
		}
		match res!(g.val("height")) {
			Some(Value::Fraction(_)) => pod.expand_y = true,
			Some(v) => if let Some(Value::Relative(r)) = cast_opt_rel(v) {
				pod.h = g.rel(r, base_h);
				pod.expand_y = true;
			},
			None => (),
		}
	}

	let mut frame = match kind {
		ElemKind::Line		=> res!(layout_line(&g, pod, span, engine)),
		ElemKind::Polygon	=> res!(layout_polygon(&g, pod, span, engine)),
		ElemKind::Curve		=> res!(layout_curve(&g, pod, span, engine)),
		ElemKind::Rect | ElemKind::Square | ElemKind::Ellipse | ElemKind::Circle
							=> res!(layout_shape(engine, &g, pod, body)),
		ElemKind::Image		=> res!(layout_image(engine, &g, pod)),
		ElemKind::Move		=> res!(layout_move(engine, &g, pod, body)),
		ElemKind::Rotate | ElemKind::Scale | ElemKind::Skew
							=> res!(layout_transform(engine, &g, pod, body)),
		ElemKind::Repeat	=> res!(layout_repeat(engine, &g, pod, body)),
		// `hide` is shown as its body under the `hidden` style before flow sees it; laid out directly, it
		// is that body, hidden.
		ElemKind::Hide => {
			let b = match res!(g.content("body")) {
				Some(b)	=> b,
				None	=> Content::empty(),
			};
			let mut f = res!(body(engine, &b, styles, pod));
			f.hide();
			f
		},
		other => return Err(engine.error(DiagnosticKind::Unsupported, span, fmt!("{} is not a visual element flow can lay out", other.path()))),
	};

	// The block keeps the size it was given on an expanded axis.
	if pod.expand_x && pod.w.is_finite() {
		frame.width = pod.w;
	}
	if pod.expand_y && pod.h.is_finite() {
		frame.height = pod.h;
	}
	if res!(vis::is_hidden(styles)) {
		frame.hide();
	}
	Ok(frame)
}

fn cast_opt_rel(v: Value) -> Option<Value> {
	match v {
		Value::Auto | Value::None | Value::Fraction(_)	=> None,
		other => vis::cast_rel(other).ok().map(Value::Relative),
	}
}

// Strokes and paints

// Typst's default stroke: 1pt black, butt caps, miter joins, miter limit 4.
fn default_stroke() -> FixedStroke {
	FixedStroke {
		paint:			Paint::Color(black()),
		thickness:		1.0,
		cap:			LineCap::Butt,
		join:			LineJoin::Miter,
		dash:			None,
		miter_limit:	4.0,
	}
}

fn black() -> crate::eval::value::Color {
	crate::eval::value::Color { space: crate::eval::value::ColorSpace::Luma, c: [0.0, 0.0, 0.0, 0.0], alpha: 1.0 }
}

/// A stroke's unset parts filled from Typst's default, lengths resolved at the font size.
pub fn fix_stroke(s: &Stroke, fs: f64) -> FixedStroke {
	let d			= default_stroke();
	let thickness	= s.thickness.map(|l| l.resolve(fs)).unwrap_or(d.thickness);
	let dash = match &s.dash {
		Some(Some(dash)) => {
			let pattern = dash.array.iter().map(|i| match i {
				DashItem::Len(l)	=> l.resolve(fs),
				DashItem::Dot		=> thickness,
			}).collect::<Vec<_>>();
			// An empty array is the named "solid" pattern.
			if pattern.is_empty() { None } else { Some((pattern, dash.phase.resolve(fs))) }
		},
		_ => None,
	};
	FixedStroke {
		paint:			s.paint.clone().unwrap_or(d.paint),
		thickness,
		cap:			s.cap.unwrap_or(d.cap),
		join:			s.join.unwrap_or(d.join),
		dash,
		miter_limit:	s.miter_limit.unwrap_or(d.miter_limit),
	}
}

// `Smart<Option<Stroke>>` resolved against the fill: `auto` strokes only an unfilled shape.
fn smart_stroke(v: Option<Value>, filled: bool, fs: f64) -> Outcome<Option<FixedStroke>> {
	match v {
		None | Some(Value::Auto)	=> Ok(if filled { None } else { Some(default_stroke()) }),
		Some(Value::None)			=> Ok(None),
		Some(other)					=> Ok(Some(fix_stroke(&res!(vis::cast_stroke(other)), fs))),
	}
}

fn fill_rule(g: &Get) -> Outcome<FillRule> {
	match res!(g.val("fill-rule")) {
		Some(Value::Str(s)) if s.as_str() == "even-odd"	=> Ok(FillRule::EvenOdd),
		_												=> Ok(FillRule::NonZero),
	}
}

fn fill(g: &Get) -> Outcome<Option<Paint>> {
	match res!(g.val("fill")) {
		Some(v)	=> vis::fill_of(&v),
		None	=> Ok(None),
	}
}

fn point(g: &Get, v: &Value, pod: Pod) -> Outcome<P2> {
	let (x, y) = res!(vis::point_of(v));
	Ok(P2::new(g.rel(x, pod.w), g.rel(y, pod.h)))
}

// Line, polygon and curve

fn layout_line(g: &Get, pod: Pod, span: Span, engine: &mut Engine) -> Outcome<Frame> {
	let start = match res!(g.val("start")) {
		Some(v) if !v.is_none()	=> res!(point(g, &v, pod)),
		_						=> P2::default(),
	};
	let delta = match res!(g.val("end")) {
		Some(v) if !v.is_none() => res!(point(g, &v, pod)).sub(start),
		_ => {
			let length = match res!(g.val("length")) {
				Some(v)	=> res!(vis::cast_rel(v)),
				None	=> Relative { rel: crate::eval::value::Ratio(0.0), abs: crate::eval::value::Length::pt(30.0) },
			};
			let angle = match res!(g.val("angle")) {
				Some(v)	=> res!(vis::cast_angle(v)).0,
				None	=> 0.0,
			};
			// The length's two parts turn with the angle, each then resolved on its own axis.
			let x = Relative { rel: crate::eval::value::Ratio(length.rel.0 * angle.cos()),
				abs: crate::eval::value::Length { abs: length.abs.abs * angle.cos(), em: length.abs.em * angle.cos() } };
			let y = Relative { rel: crate::eval::value::Ratio(length.rel.0 * angle.sin()),
				abs: crate::eval::value::Length { abs: length.abs.abs * angle.sin(), em: length.abs.em * angle.sin() } };
			P2::new(g.rel(x, pod.w), g.rel(y, pod.h))
		},
	};
	let stroke = match res!(g.val("stroke")) {
		Some(v) if !v.is_auto()	=> fix_stroke(&res!(vis::cast_stroke(v)), g.fs),
		_						=> default_stroke(),
	};
	let end		= start.add(delta);
	let w		= start.x.max(end.x).max(0.0);
	let h		= start.y.max(end.y).max(0.0);
	if !w.is_finite() || !h.is_finite() {
		return Err(engine.error(DiagnosticKind::Type, span, "cannot create line with infinite length"));
	}
	let mut frame	= Frame::new(w, h);
	let mut c		= Curve::new();
	c.move_(P2::default());
	c.line(delta);
	frame.push(start, Item::Shape(Shape { curve: c, fill: None, fill_rule: FillRule::NonZero, stroke: Some(stroke) }));
	Ok(frame)
}

fn layout_polygon(g: &Get, pod: Pod, span: Span, engine: &mut Engine) -> Outcome<Frame> {
	let mut pts = Vec::new();
	if let Some(Value::Array(a)) = res!(g.val("vertices")) {
		for v in a.iter() {
			pts.push(res!(point(g, v, pod)));
		}
	}
	let w = pts.iter().fold(0.0f64, |m, p| m.max(p.x));
	let h = pts.iter().fold(0.0f64, |m, p| m.max(p.y));
	if !w.is_finite() || !h.is_finite() {
		return Err(engine.error(DiagnosticKind::Type, span, "cannot create polygon with infinite size"));
	}
	let mut frame = Frame::new(w, h);
	let first = match pts.first() {
		Some(p)	=> *p,
		None	=> return Ok(frame),
	};
	let fill	= res!(fill(g));
	let stroke	= res!(smart_stroke(res!(g.val("stroke")), fill.is_some(), g.fs));
	let mut c	= Curve::new();
	c.move_(first);
	for p in &pts[1..] {
		c.line(*p);
	}
	c.close();
	frame.push(P2::default(), Item::Shape(Shape { curve: c, fill, fill_rule: res!(fill_rule(g)), stroke }));
	Ok(frame)
}

// Typst's `CurveBuilder`: quads raised to cubics, `auto` controls mirrored from the previous segment, a
// smooth close drawn as a cubic back to the start, bounds grown by each point and cubic extremum.
struct CurveBuilder {
	curve:				Curve,
	size:				P2,
	start_point:		P2,
	start_control_into:	P2,
	last_point:			P2,
	last_control_from:	P2,
	is_started:			bool,
	is_empty:			bool,
}

impl CurveBuilder {
	fn new() -> Self {
		Self {
			curve: Curve::new(), size: P2::default(), start_point: P2::default(),
			start_control_into: P2::default(), last_point: P2::default(),
			last_control_from: P2::default(), is_started: false, is_empty: true,
		}
	}

	fn expand(&mut self, p: P2) {
		self.size.x = self.size.x.max(p.x);
		self.size.y = self.size.y.max(p.y);
	}

	fn move_(&mut self, p: P2) {
		self.expand(p);
		self.start_point		= p;
		self.start_control_into	= p;
		self.last_point			= p;
		self.last_control_from	= p;
		self.is_started			= true;
		self.is_empty			= true;
	}

	fn start_component(&mut self) {
		self.curve.move_(self.start_point);
		self.is_empty	= false;
		self.is_started	= true;
	}

	fn line(&mut self, p: P2) {
		if self.is_empty {
			self.start_component();
			self.start_control_into = self.start_point;
		}
		self.curve.line(p);
		self.expand(p);
		self.last_point			= p;
		self.last_control_from	= p;
	}

	fn quad(&mut self, control: P2, end: P2) {
		let c1 = control_q2c(self.last_point, control);
		let c2 = control_q2c(end, control);
		self.cubic(c1, c2, end);
	}

	fn cubic(&mut self, c1: P2, c2: P2, end: P2) {
		if self.is_empty {
			self.start_component();
			self.start_control_into = mirror_c(self.start_point, c1);
		}
		self.curve.cubic(c1, c2, end);
		let (mx, my) = cubic_max(self.last_point, c1, c2, end);
		self.size.x = self.size.x.max(mx);
		self.size.y = self.size.y.max(my);
		self.last_point			= end;
		self.last_control_from	= mirror_c(end, c2);
	}

	fn close(&mut self, smooth: bool) {
		if self.is_started && !self.is_empty {
			if smooth {
				let (a, b, p) = (self.last_control_from, self.start_control_into, self.start_point);
				self.cubic(a, b, p);
			}
			self.curve.close();
			self.last_point			= self.start_point;
			self.last_control_from	= self.start_point;
		}
		self.is_started	= false;
		self.is_empty	= true;
	}
}

fn control_c2q(p: P2, c: P2) -> P2 { c.mul(1.5).sub(p.mul(0.5)) }
fn control_q2c(p: P2, c: P2) -> P2 { p.add(c.mul(2.0)).mul(1.0 / 3.0) }
fn mirror_c(p: P2, c: P2) -> P2 { p.mul(2.0).sub(c) }

// The largest x and y a cubic reaches: its end points and the roots of its derivative within (0, 1).
fn cubic_max(p0: P2, p1: P2, p2: P2, p3: P2) -> (f64, f64) {
	let axis = |a: f64, b: f64, c: f64, d: f64| -> f64 {
		let mut m = a.max(d);
		// B'(t)/3 = (b-a)(1-t)^2 + 2(c-b)(1-t)t + (d-c)t^2 = qa t^2 + qb t + qc.
		let qa = (b - a) - 2.0 * (c - b) + (d - c);
		let qb = 2.0 * ((c - b) - (b - a));
		let qc = b - a;
		let mut roots = Vec::new();
		if qa.abs() < 1e-12 {
			if qb.abs() > 1e-12 {
				roots.push(-qc / qb);
			}
		} else {
			let disc = qb * qb - 4.0 * qa * qc;
			if disc >= 0.0 {
				let s = disc.sqrt();
				roots.push((-qb + s) / (2.0 * qa));
				roots.push((-qb - s) / (2.0 * qa));
			}
		}
		for t in roots {
			if t > 0.0 && t < 1.0 {
				let u = 1.0 - t;
				let v = u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
				m = m.max(v);
			}
		}
		m
	};
	(axis(p0.x, p1.x, p2.x, p3.x), axis(p0.y, p1.y, p2.y, p3.y))
}

fn layout_curve(g: &Get, pod: Pod, span: Span, engine: &mut Engine) -> Outcome<Frame> {
	let mut b = CurveBuilder::new();
	let comps = match res!(g.val("components")) {
		Some(Value::Array(a))	=> a,
		_						=> Arc::new(Vec::new()),
	};
	for comp in comps.iter() {
		let c = match comp {
			Value::Content(c)	=> c,
			_					=> continue,
		};
		let k = match c.kind() {
			Some(k)	=> k,
			None	=> continue,
		};
		let cg = Get { elem: c, styles: g.styles, kind: k, fs: g.fs };
		let relative = if k == ElemKind::CurveClose { false } else { res!(cg.bool("relative", false)) };
		let at = |cg: &Get, name: &str, last: P2| -> Outcome<P2> {
			let v = match res!(cg.val(name)) {
				Some(v)	=> v,
				None	=> return Err(err!("curve component is missing `{}`", name; Input, Missing)),
			};
			let p = res!(point(cg, &v, pod));
			Ok(if relative { p.add(last) } else { p })
		};
		match k {
			ElemKind::CurveMove => {
				let p = res!(at(&cg, "start", b.last_point));
				b.move_(p);
			},
			ElemKind::CurveLine => {
				let p = res!(at(&cg, "end", b.last_point));
				b.line(p);
			},
			ElemKind::CurveQuad => {
				let end = res!(at(&cg, "end", b.last_point));
				let control = match res!(cg.val("control")) {
					Some(Value::Auto)				=> control_c2q(b.last_point, b.last_control_from),
					Some(Value::None) | None		=> end,
					Some(_)							=> res!(at(&cg, "control", b.last_point)),
				};
				b.quad(control, end);
			},
			ElemKind::CurveCubic => {
				let end = res!(at(&cg, "end", b.last_point));
				let c1 = match res!(cg.val("control-start")) {
					Some(Value::Auto)			=> b.last_control_from,
					Some(Value::None) | None	=> b.last_point,
					Some(_)						=> res!(at(&cg, "control-start", b.last_point)),
				};
				let c2 = match res!(cg.val("control-end")) {
					Some(Value::None) | None	=> end,
					Some(_)						=> res!(at(&cg, "control-end", b.last_point)),
				};
				b.cubic(c1, c2, end);
			},
			ElemKind::CurveClose => {
				let smooth = !matches!(res!(cg.val("mode")), Some(Value::Str(s)) if s.as_str() == "straight");
				b.close(smooth);
			},
			_ => (),
		}
	}
	let (curve, size) = (b.curve, b.size);
	let mut frame = Frame::new(size.x, size.y);
	if curve.is_empty() {
		return Ok(frame);
	}
	if !size.x.is_finite() || !size.y.is_finite() {
		return Err(engine.error(DiagnosticKind::Type, span, "cannot create curve with infinite size"));
	}
	let fill	= res!(fill(g));
	let stroke	= res!(smart_stroke(res!(g.val("stroke")), fill.is_some(), g.fs));
	frame.push(P2::default(), Item::Shape(Shape { curve, fill, fill_rule: res!(fill_rule(g)), stroke }));
	Ok(frame)
}

// Rect, square, ellipse, circle

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapeKind {
	Square,
	Rect,
	Circle,
	Ellipse,
}

impl ShapeKind {
	fn is_round(self) -> bool { matches!(self, ShapeKind::Circle | ShapeKind::Ellipse) }
	fn is_quadratic(self) -> bool { matches!(self, ShapeKind::Square | ShapeKind::Circle) }
}

// Four sides (left, top, right, bottom) of relative lengths. A side the value leaves unset folds onto the
// field's default, as Typst folds `Sides<Option<..>>`: `inset: (x: 2pt)` keeps the 5pt default above and below.
fn sides_rel(g: &Get, name: &str) -> Outcome<[Relative; 4]> {
	let default = match g.kind.field_id(name).and_then(|id| g.kind.field_spec(id)).and_then(|f| f.default.to_value()) {
		Some(v)	=> res!(vis::cast_rel(v)),
		None	=> Relative::default(),
	};
	let mut out = [default; 4];
	if let Some(v) = res!(g.val(name)) {
		let v = res!(vis::cast_sides_rel(v));
		for (k, s) in vis::sides_of(&v).iter().enumerate() {
			if let Some(x) = s {
				out[k] = res!(vis::cast_rel(x.clone()));
			}
		}
	}
	Ok(out)
}

fn quadratic_size(pod: Pod) -> Option<f64> {
	if pod.expand_x && pod.expand_y {
		Some(pod.w.min(pod.h))
	} else if pod.expand_x {
		Some(pod.w)
	} else if pod.expand_y {
		Some(pod.h)
	} else {
		None
	}
}

fn layout_shape<F>(engine: &mut Engine, g: &Get, pod: Pod, body_fn: &mut F) -> Outcome<Frame>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let kind = match g.kind {
		ElemKind::Square	=> ShapeKind::Square,
		ElemKind::Circle	=> ShapeKind::Circle,
		ElemKind::Ellipse	=> ShapeKind::Ellipse,
		_					=> ShapeKind::Rect,
	};
	let body = res!(g.content("body"));
	let mut frame;
	if let Some(child) = body {
		let mut inset = res!(sides_rel(g, "inset"));
		if kind.is_round() {
			// Round shapes pad their body further, so it sits inside the curve.
			for s in &mut inset {
				s.rel.0 += 0.5 - std::f64::consts::SQRT_2 / 4.0;
			}
		}
		let has_inset = inset.iter().any(|s| s.rel.0 != 0.0 || s.abs.abs != 0.0 || s.abs.em != 0.0);
		// Resolved against the region with Typst's summing: each axis's parts summed, then resolved.
		let sum_x = Relative { rel: crate::eval::value::Ratio(inset[0].rel.0 + inset[2].rel.0),
			abs: crate::eval::value::Length { abs: inset[0].abs.abs + inset[2].abs.abs, em: inset[0].abs.em + inset[2].abs.em } };
		let sum_y = Relative { rel: crate::eval::value::Ratio(inset[1].rel.0 + inset[3].rel.0),
			abs: crate::eval::value::Length { abs: inset[1].abs.abs + inset[3].abs.abs, em: inset[1].abs.em + inset[3].abs.em } };
		let mut inner = pod;
		if has_inset {
			inner.w = pod.w - g.rel(sum_x, pod.w);
			inner.h = pod.h - g.rel(sum_y, pod.h);
		}
		if kind.is_quadratic() {
			let length = match quadratic_size(inner) {
				Some(l)	=> l,
				None => {
					let f = res!(body_fn(engine, &child, g.styles, inner));
					f.width.max(f.height).min(inner.w.min(inner.h))
				},
			};
			inner = Pod::new(length, length, true, true);
		}
		frame = res!(body_fn(engine, &child, g.styles, inner));
		if has_inset {
			// Typst's `pad::grow`: the padded size is what, padded again, gives the frame's size.
			let pw = (frame.width + sum_x.abs.resolve(g.fs)) / (1.0 - sum_x.rel.0);
			let ph = (frame.height + sum_y.abs.resolve(g.fs)) / (1.0 - sum_y.rel.0);
			let left	= g.rel(inset[0], pw);
			let top		= g.rel(inset[1], ph);
			frame.width		= pw;
			frame.height	= ph;
			frame.translate(P2::new(left, top));
		}
	} else {
		// The default size a shape takes with no body and no forced size.
		let dw = 45.0f64.min(pod.w);
		let dh = 30.0f64.min(pod.h);
		let (w, h) = if kind.is_quadratic() {
			let l = quadratic_size(pod).unwrap_or(dw.min(dh));
			(l, l)
		} else {
			(if pod.expand_x { pod.w } else { dw }, if pod.expand_y { pod.h } else { dh })
		};
		frame = Frame::new(w, h);
	}

	let fill	= res!(fill(g));
	let outset	= res!(sides_rel(g, "outset"));
	if kind.is_round() {
		let stroke = res!(smart_stroke(res!(g.val("stroke")), fill.is_some(), g.fs));
		if fill.is_some() || stroke.is_some() {
			let l	= g.rel(outset[0], frame.width);
			let t	= g.rel(outset[1], frame.height);
			let r	= g.rel(outset[2], frame.width);
			let b	= g.rel(outset[3], frame.height);
			let shape = Shape {
				curve:		Curve::ellipse(frame.width + l + r, frame.height + t + b),
				fill,
				fill_rule:	FillRule::NonZero,
				stroke,
			};
			frame.prepend(vec![(P2::new(-l, -t), Item::Shape(shape))]);
		}
	} else {
		// Sides of strokes: `auto` strokes an unfilled rect all round; a set side left unset is unstroked.
		let strokes: [Option<FixedStroke>; 4] = match res!(g.val("stroke")) {
			None | Some(Value::Auto) => if fill.is_none() {
				[Some(default_stroke()), Some(default_stroke()), Some(default_stroke()), Some(default_stroke())]
			} else {
				[None, None, None, None]
			},
			Some(v) => {
				let v = res!(vis::cast_stroke_sides(v));
				let parts = vis::sides_of(&v);
				let mut out: [Option<FixedStroke>; 4] = [None, None, None, None];
				for k in 0..4 {
					out[k] = match &parts[k] {
						Some(Value::Stroke(s))	=> Some(fix_stroke(s, g.fs)),
						_						=> None,
					};
				}
				out
			},
		};
		let mut radius = [Relative::default(); 4];
		if let Some(v) = res!(g.val("radius")) {
			let v = res!(vis::cast_corners_rel(v));
			for (k, c) in vis::corners_of(&v).iter().enumerate() {
				if let Some(x) = c {
					radius[k] = res!(vis::cast_rel(x.clone()));
				}
			}
		}
		if fill.is_some() || strokes.iter().any(Option::is_some) {
			let l	= g.rel(outset[0], frame.width);
			let t	= g.rel(outset[1], frame.height);
			let r	= g.rel(outset[2], frame.width);
			let b	= g.rel(outset[3], frame.height);
			let size	= P2::new(frame.width + l + r, frame.height + t + b);
			let shapes	= styled_rect(size, &radius, g.fs, fill, &strokes);
			frame.prepend(shapes.into_iter().map(|s| (P2::new(-l, -t), Item::Shape(s))).collect());
		}
	}
	Ok(frame)
}

// Rectangles with radii and per-side strokes: Typst's `styled_rect`, `segmented_rect` and corner geometry.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Corner {
	TopLeft,
	TopRight,
	BottomRight,
	BottomLeft,
}

impl Corner {
	fn idx(self) -> usize {
		match self {
			Corner::TopLeft		=> 0,
			Corner::TopRight	=> 1,
			Corner::BottomRight	=> 2,
			Corner::BottomLeft	=> 3,
		}
	}

	fn next_cw(self) -> Corner {
		match self {
			Corner::TopLeft		=> Corner::TopRight,
			Corner::TopRight	=> Corner::BottomRight,
			Corner::BottomRight	=> Corner::BottomLeft,
			Corner::BottomLeft	=> Corner::TopLeft,
		}
	}

	fn next_ccw(self) -> Corner {
		match self {
			Corner::TopLeft		=> Corner::BottomLeft,
			Corner::TopRight	=> Corner::TopLeft,
			Corner::BottomRight	=> Corner::TopRight,
			Corner::BottomLeft	=> Corner::BottomRight,
		}
	}

	// The sides before and after the corner, clockwise, as indices into left, top, right, bottom.
	fn side_ccw(self) -> usize {
		match self {
			Corner::TopLeft		=> 0,
			Corner::TopRight	=> 1,
			Corner::BottomRight	=> 2,
			Corner::BottomLeft	=> 3,
		}
	}

	fn side_cw(self) -> usize {
		match self {
			Corner::TopLeft		=> 1,
			Corner::TopRight	=> 2,
			Corner::BottomRight	=> 3,
			Corner::BottomLeft	=> 0,
		}
	}
}

const ALL_CORNERS: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft];

fn styled_rect(size: P2, radius: &[Relative; 4], fs: f64, fill: Option<Paint>, strokes: &[Option<FixedStroke>; 4]) -> Vec<Shape> {
	let uniform = strokes_uniform(strokes);
	let no_radius = radius.iter().all(|r| r.rel.0 == 0.0 && r.abs.abs == 0.0 && r.abs.em == 0.0);
	if uniform && no_radius {
		return vec![Shape { curve: Curve::rect(size.x, size.y), fill, fill_rule: FillRule::NonZero, stroke: strokes[1].clone() }];
	}
	segmented_rect(size, radius, fs, fill, strokes)
}

fn stroke_same(a: &FixedStroke, b: &FixedStroke) -> bool {
	paint_same(&a.paint, &b.paint) && a.thickness == b.thickness && a.cap == b.cap && a.join == b.join
		&& a.dash == b.dash && a.miter_limit == b.miter_limit
}

fn paint_same(a: &Paint, b: &Paint) -> bool {
	match (a, b) {
		(Paint::Color(x), Paint::Color(y))			=> x == y,
		(Paint::Gradient(x), Paint::Gradient(y))	=> x == y,
		(Paint::Tiling(x), Paint::Tiling(y))		=> Arc::ptr_eq(x, y),
		_											=> false,
	}
}

fn strokes_uniform(s: &[Option<FixedStroke>; 4]) -> bool {
	s.iter().all(|x| match (x, &s[0]) {
		(None, None)		=> true,
		(Some(a), Some(b))	=> stroke_same(a, b),
		_					=> false,
	})
}

// Half the stroke width of each side, `None` for an unstroked side, as Typst 0.15 keeps them: a rounded
// corner next to an unstroked side borrows its neighbour's width when the radius has room, so the stroke
// tapers into the curve with a proper cap rather than a sliver.
struct ControlPoints {
	radius:			f64,
	stroke_after:	Option<f64>,
	stroke_before:	Option<f64>,
	corner:			Corner,
	size:			P2,
	same:			bool,
}

impl ControlPoints {
	fn rotate(&self, p: P2) -> P2 {
		let sx = self.size.x.signum();
		let sy = self.size.y.signum();
		match self.corner {
			Corner::TopLeft		=> P2::new(sx * p.x, sy * p.y),
			Corner::TopRight	=> P2::new(self.size.x - sx * p.y, sy * p.x),
			Corner::BottomRight	=> P2::new(self.size.x - sx * p.x, self.size.y - sy * p.y),
			Corner::BottomLeft	=> P2::new(sx * p.y, self.size.y - sy * p.x),
		}
	}
	fn reuse_after_for_cap(&self) -> Option<f64> { self.stroke_after.filter(|s| 2.0 * s < self.radius) }
	fn reuse_before_for_cap(&self) -> Option<f64> { self.stroke_before.filter(|s| 2.0 * s < self.radius) }
	fn wb(&self) -> f64 { self.stroke_before.or(self.reuse_after_for_cap()).unwrap_or(0.0) }
	fn wa(&self) -> f64 { self.stroke_after.or(self.reuse_before_for_cap()).unwrap_or(0.0) }
	fn outer(&self) -> P2 { self.rotate(P2::new(-self.wb(), -self.wa())) }
	fn center_outer(&self) -> P2 {
		let r = self.radius_outer();
		self.rotate(P2::new(r - self.wb(), r - self.wa()))
	}
	fn center(&self) -> P2 {
		let r = self.radius();
		self.rotate(P2::new(r, r))
	}
	fn center_inner(&self) -> P2 {
		let r = self.radius_inner();
		self.rotate(P2::new(self.wb() + r, self.wa() + r))
	}
	fn radius_outer(&self) -> f64 { self.radius }
	fn radius(&self) -> f64 { (self.radius - self.wb().min(self.wa())).max(0.0) }
	fn radius_inner(&self) -> f64 { (self.radius - 2.0 * self.wb().max(self.wa())).max(0.0) }
	fn mid_outer(&self) -> P2 {
		let c_i	= self.center_inner();
		let c_o	= self.center_outer();
		let o	= self.outer();
		let r	= self.radius_outer();
		let a = (o.x - c_i.x).powi(2) + (o.y - c_i.y).powi(2);
		let b = 2.0 * (o.x - c_i.x) * (c_i.x - c_o.x) + 2.0 * (o.y - c_i.y) * (c_i.y - c_o.y);
		let c = (c_i.x - c_o.x).powi(2) + (c_i.y - c_o.y).powi(2) - r.powi(2);
		let t = (-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a);
		c_i.add(o.sub(c_i).mul(t))
	}
	fn mid(&self) -> P2 {
		let center	= self.center();
		let diff	= self.outer().sub(center);
		center.add(diff.mul(self.radius() / diff.hypot()))
	}
	fn mid_inner(&self) -> P2 {
		let center	= self.center_inner();
		let diff	= self.outer().sub(center);
		center.add(diff.mul(self.radius_inner() / diff.hypot()))
	}
	fn arc_outer(&self) -> bool { self.radius_outer() > 0.0 }
	fn arc(&self) -> bool { self.radius() > 0.0 }
	fn arc_inner(&self) -> bool { self.radius_inner() > 0.0 }
	fn start_outer(&self) -> P2 { self.rotate(P2::new(-self.wb(), self.radius_outer() - self.wa())) }
	fn start(&self) -> P2 { self.rotate(P2::new(0.0, self.radius())) }
	fn start_inner(&self) -> P2 { self.rotate(P2::new(self.wb(), self.wa() + self.radius_inner())) }
	fn end_outer(&self) -> P2 { self.rotate(P2::new(self.radius_outer() - self.wb(), -self.wa())) }
	fn end(&self) -> P2 { self.rotate(P2::new(self.radius(), 0.0)) }
	fn end_inner(&self) -> P2 { self.rotate(P2::new(self.wb() + self.radius_inner(), self.wa())) }

	// The cap where a stroke begins at this corner, its side before unstroked.
	fn start_cap(&self, curve: &mut Curve, cap: LineCap) {
		let small = self.reuse_after_for_cap().is_none();
		if cap == LineCap::Butt || self.stroke_before.is_some() || (self.radius != 0.0 && small) {
			curve.line(self.mid_outer());
		} else if cap == LineCap::Square {
			let (s, e)	= (self.mid_inner(), self.mid_outer());
			let off		= line_normal(s, e).mul(self.wa());
			curve.line(s.add(off));
			curve.line(e.add(off));
			curve.line(e);
		} else {
			let (s, e)	= (self.mid_inner(), self.mid_outer());
			let centre	= s.add(e).mul(0.5).sub(line_normal(s, e).mul(RAW_UNIT));
			curve.arc(s, centre, e);
		}
	}

	// The cap where a stroke ends at this corner, its side after unstroked.
	fn end_cap(&self, curve: &mut Curve, cap: LineCap) {
		let small = self.reuse_before_for_cap().is_none();
		if cap == LineCap::Butt || self.stroke_after.is_some() || (self.radius != 0.0 && small) {
			curve.line(self.mid_inner());
		} else if cap == LineCap::Square {
			let (s, e)	= (self.mid_outer(), self.mid_inner());
			let off		= line_normal(s, e).mul(self.wb());
			curve.line(s.add(off));
			curve.line(e.add(off));
			curve.line(e);
		} else {
			let (s, e)	= (self.mid_outer(), self.mid_inner());
			let centre	= s.add(e).mul(0.5).sub(line_normal(s, e).mul(RAW_UNIT));
			curve.arc(s, centre, e);
		}
	}
}

// The unit normal of the line from `a` to `b`, turned a quarter anticlockwise (y down: (x, y) to (y, -x)).
fn line_normal(a: P2, b: P2) -> P2 {
	let d = b.sub(a);
	let n = P2::new(d.y, -d.x);
	let l = n.hypot();
	if l == 0.0 { n } else { n.mul(1.0 / l) }
}

// The control points of a cubic approximating the circular arc from `start` to `end` about `center`.
fn bezier_arc_control(start: P2, center: P2, end: P2) -> [P2; 2] {
	let a	= start.sub(center);
	let b	= end.sub(center);
	let q1	= a.x * a.x + a.y * a.y;
	let q2	= q1 + a.x * b.x + a.y * b.y;
	let k2	= (4.0 / 3.0) * ((2.0 * q1 * q2).sqrt() - q2) / (a.x * b.y - a.y * b.x);
	[
		P2::new(center.x + a.x - k2 * a.y, center.y + a.y + k2 * a.x),
		P2::new(center.x + b.x + k2 * b.y, center.y + b.y - k2 * b.x),
	]
}

impl Curve {
	fn arc(&mut self, start: P2, center: P2, end: P2) {
		let c = bezier_arc_control(start, center, end);
		self.cubic(c[0], c[1], end);
	}
	fn arc_move(&mut self, start: P2, center: P2, end: P2) {
		self.move_(start);
		self.arc(start, center, end);
	}
	fn arc_line(&mut self, start: P2, center: P2, end: P2) {
		self.line(start);
		self.arc(start, center, end);
	}
}

fn corners_control_points(size: P2, radius: &[f64; 4], strokes: &[Option<FixedStroke>; 4], widths: &[Option<f64>; 4]) -> [ControlPoints; 4] {
	ALL_CORNERS.map(|corner| ControlPoints {
		radius:			radius[corner.idx()],
		stroke_before:	widths[corner.side_ccw()],
		stroke_after:	widths[corner.side_cw()],
		corner,
		size,
		same: match (&strokes[corner.side_ccw()], &strokes[corner.side_cw()]) {
			(Some(a), Some(b)) => {
				// A solid stroke is filled, so only its paint and dash matter; a dashed one is stroked
				// whole, so its cap and width must agree too.
				let solid = a.dash.is_none();
				let filled_same = paint_same(&a.paint, &b.paint) && a.dash == b.dash;
				let stroked_same = a.cap == b.cap && a.thickness == b.thickness;
				filled_same && (solid || stroked_same)
			},
			(None, None)	=> true,
			_				=> false,
		},
	})
}

// Typst's `Option<Abs>::min`: `None` orders first.
fn opt_min(a: Option<f64>, b: Option<f64>) -> Option<f64> {
	match (a, b) {
		(Some(x), Some(y))	=> Some(x.min(y)),
		_					=> None,
	}
}

fn segmented_rect(size: P2, radius: &[Relative; 4], fs: f64, fill: Option<Paint>, strokes: &[Option<FixedStroke>; 4]) -> Vec<Shape> {
	let mut res		= Vec::new();
	let widths		= strokes.clone().map(|s| s.map(|s| s.thickness / 2.0));
	let base		= size.x.abs().min(size.y.abs()) / 2.0;
	// Each corner's largest radius: half the short side, plus the thinner of its two strokes.
	let corner_max	= [
		base + opt_min(widths[0], widths[1]).unwrap_or(0.0),
		base + opt_min(widths[1], widths[2]).unwrap_or(0.0),
		base + opt_min(widths[2], widths[3]).unwrap_or(0.0),
		base + opt_min(widths[3], widths[0]).unwrap_or(0.0),
	];
	let mut rad = [0.0f64; 4];
	for k in 0..4 {
		rad[k] = rel_to(radius[k], corner_max[k] * 2.0, fs).min(corner_max[k]);
	}
	let corners	= corners_control_points(size, &rad, strokes, &widths);
	let get		= |c: Corner| &corners[c.idx()];

	let mut stroke_insert = 0;
	if let Some(fill) = fill {
		let mut curve = Curve::new();
		let c = get(Corner::TopLeft);
		if c.arc() {
			curve.arc_move(c.start(), c.center(), c.end());
		} else {
			curve.move_(c.center());
		}
		for corner in [Corner::TopRight, Corner::BottomRight, Corner::BottomLeft] {
			let c = get(corner);
			if c.arc() {
				curve.arc_line(c.start(), c.center(), c.end());
			} else {
				curve.line(c.center());
			}
		}
		curve.close();
		res.push(Shape { curve, fill: Some(fill), fill_rule: FillRule::NonZero, stroke: None });
		stroke_insert += 1;
	}

	let current = corners.iter().find(|c| !c.same).map(|c| c.corner);
	if let Some(mut current) = current {
		// Several segments: from a corner where the sides differ, clockwise round the others.
		let mut last = current;
		for _ in 0..4 {
			current = current.next_cw();
			if get(current).same {
				continue;
			}
			let start	= last;
			let end		= current;
			last		= current;
			let stroke = match &strokes[start.side_cw()] {
				Some(s)	=> s,
				None	=> continue,
			};
			let start_cap	= stroke.cap;
			let end_cap		= match &strokes[end.side_ccw()] {
				Some(s)	=> s.cap,
				None	=> start_cap,
			};
			let (shape, ontop) = segment(start, end, start_cap, end_cap, &corners, stroke);
			if ontop {
				res.push(shape);
			} else {
				res.insert(stroke_insert, shape);
				stroke_insert += 1;
			}
		}
	} else if let Some(stroke) = &strokes[1] {
		let (shape, _) = segment(Corner::TopLeft, Corner::TopLeft, stroke.cap, stroke.cap, &corners, stroke);
		res.push(shape);
	}
	res
}

/// The per-side strokes of a container's `stroke` field: a side with none, or a field that is `none` or `auto`, is
/// unstroked.
pub fn side_strokes(stroke: Option<Value>, fs: f64) -> Outcome<[Option<FixedStroke>; 4]> {
	let mut out: [Option<FixedStroke>; 4] = [None, None, None, None];
	match stroke {
		None | Some(Value::None) | Some(Value::Auto)	=> (),
		Some(v) => {
			let v = res!(vis::cast_stroke_sides(v));
			let parts = vis::sides_of(&v);
			for k in 0..4 {
				if let Some(Value::Stroke(s)) = &parts[k] {
					out[k] = Some(fix_stroke(s, fs));
				}
			}
		},
	}
	Ok(out)
}

/// Typst's `clip_rect`: the path a clipping container cuts its contents to, in the container's own frame. It
/// is the container's rectangle grown by its `outset`, with the corners rounded by `radius`, and drawn at the
/// inner edge of the stroke, so a stroke is never clipped away and nothing shows outside it. `outset` is
/// `[left, top, right, bottom]` in points.
pub fn clip_path(
	w:			f64,
	h:			f64,
	fs:			f64,
	radius:		Option<Value>,
	stroke:		Option<Value>,
	outset:		[f64; 4],
)
	-> Outcome<Path>
{
	let strokes	= res!(side_strokes(stroke, fs));
	let mut rel = [Relative::default(); 4];
	if let Some(v) = radius {
		let v = res!(vis::cast_corners_rel(v));
		for (k, c) in vis::corners_of(&v).iter().enumerate() {
			if let Some(x) = c {
				rel[k] = res!(vis::cast_rel(x.clone()));
			}
		}
	}
	let size		= P2::new(w + outset[0] + outset[2], h + outset[1] + outset[3]);
	let widths		= strokes.clone().map(|s| s.map(|s| s.thickness / 2.0));
	let base		= size.x.abs().min(size.y.abs()) / 2.0;
	let corner_max	= [
		base + opt_min(widths[0], widths[1]).unwrap_or(0.0),
		base + opt_min(widths[1], widths[2]).unwrap_or(0.0),
		base + opt_min(widths[2], widths[3]).unwrap_or(0.0),
		base + opt_min(widths[3], widths[0]).unwrap_or(0.0),
	];
	let mut rad = [0.0f64; 4];
	for k in 0..4 {
		rad[k] = rel_to(rel[k], corner_max[k] * 2.0, fs).min(corner_max[k]);
	}
	let corners	= corners_control_points(size, &rad, &strokes, &widths);
	let mut curve = Curve::new();
	let first = &corners[Corner::TopLeft.idx()];
	if first.arc_inner() {
		curve.arc_move(first.start_inner(), first.center_inner(), first.end_inner());
	} else {
		curve.move_(first.center_inner());
	}
	for corner in [Corner::TopRight, Corner::BottomRight, Corner::BottomLeft] {
		let c = &corners[corner.idx()];
		if c.arc_inner() {
			curve.arc_line(c.start_inner(), c.center_inner(), c.end_inner());
		} else {
			curve.line(c.center_inner());
		}
	}
	curve.close();
	curve.to_path(P2::new(-outset[0], -outset[1]))
}

fn curve_segment(start: Corner, end: Corner, corners: &[ControlPoints; 4], curve: &mut Curve) {
	let c = &corners[start.idx()];
	if start == end || !c.arc() {
		curve.move_(c.end());
	} else {
		curve.arc_move(c.mid(), c.center(), c.end());
	}
	let mut current = start.next_cw();
	while current != end {
		let c = &corners[current.idx()];
		if c.arc() {
			curve.arc_line(c.start(), c.center(), c.end());
		} else {
			curve.line(c.end());
		}
		current = current.next_cw();
	}
	let c = &corners[end.idx()];
	if !c.arc() {
		curve.line(c.start());
	} else if start == end {
		curve.arc_line(c.start(), c.center(), c.end());
	} else {
		curve.arc_line(c.start(), c.center(), c.mid());
	}
}

fn segment(
	start:		Corner,
	end:		Corner,
	start_cap:	LineCap,
	end_cap:	LineCap,
	corners:	&[ControlPoints; 4],
	stroke:		&FixedStroke,
)
	-> (Shape, bool)
{
	let fill_corner = |c: &ControlPoints| c.stroke_before != c.stroke_after || c.radius() < c.wb();
	let fill_corners = || {
		if fill_corner(&corners[start.idx()]) || fill_corner(&corners[end.idx()]) {
			return true;
		}
		let mut current = start.next_cw();
		while current != end {
			if fill_corner(&corners[current.idx()]) {
				return true;
			}
			current = current.next_cw();
		}
		false
	};
	let solid		= stroke.dash.is_none();
	let use_fill	= solid && fill_corners();
	let shape = if use_fill {
		fill_segment(start, end, start_cap, end_cap, corners, stroke)
	} else {
		let mut curve = Curve::new();
		curve_segment(start, end, corners, &mut curve);
		Shape { curve, fill: None, fill_rule: FillRule::NonZero, stroke: Some(stroke.clone()) }
	};
	(shape, use_fill)
}

fn fill_segment(
	start:		Corner,
	end:		Corner,
	start_cap:	LineCap,
	end_cap:	LineCap,
	corners:	&[ControlPoints; 4],
	stroke:		&FixedStroke,
)
	-> Shape
{
	let mut curve = Curve::new();
	if start == end {
		let c = &corners[start.idx()];
		curve.move_(c.end_inner());
		curve.line(c.end_outer());
	} else {
		let c = &corners[start.idx()];
		if c.arc_inner() {
			curve.arc_move(c.end_inner(), c.center_inner(), c.mid_inner());
		} else {
			curve.move_(c.end_inner());
		}
		c.start_cap(&mut curve, start_cap);
		if c.arc_outer() {
			curve.arc_line(c.mid_outer(), c.center_outer(), c.end_outer());
		}
	}
	let mut current = start.next_cw();
	while current != end {
		let c = &corners[current.idx()];
		if c.arc_outer() {
			curve.arc_line(c.start_outer(), c.center_outer(), c.end_outer());
		} else {
			curve.line(c.outer());
		}
		current = current.next_cw();
	}
	if start == end {
		let c = &corners[end.idx()];
		if c.arc_outer() {
			curve.arc_line(c.start_outer(), c.center_outer(), c.end_outer());
		} else {
			curve.line(c.outer());
			curve.line(c.end_outer());
		}
		if c.arc_inner() {
			curve.arc_line(c.end_inner(), c.center_inner(), c.start_inner());
		} else {
			curve.line(c.center_inner());
		}
	} else {
		let c = &corners[end.idx()];
		if c.arc_outer() {
			curve.arc_line(c.start_outer(), c.center_outer(), c.mid_outer());
		} else {
			curve.line(c.outer());
		}
		c.end_cap(&mut curve, end_cap);
		if c.arc_inner() {
			curve.arc_line(c.mid_inner(), c.center_inner(), c.start_inner());
		}
	}
	let mut current = end.next_ccw();
	while current != start {
		let c = &corners[current.idx()];
		if c.arc_inner() {
			curve.arc_line(c.end_inner(), c.center_inner(), c.start_inner());
		} else {
			curve.line(c.center_inner());
		}
		current = current.next_ccw();
	}
	curve.close();
	Shape { curve, fill: Some(stroke.paint.clone()), fill_rule: FillRule::NonZero, stroke: None }
}

// Images

fn layout_image(engine: &mut Engine, g: &Get, pod: Pod) -> Outcome<Frame> {
	let span = g.elem.span();
	let (data, ext) = match res!(g.val("source")) {
		Some(Value::Bytes(b)) => ((*b).clone(), None),
		Some(Value::Str(p)) => {
			let path = res!(crate::eval::import::resolve_path(engine, &p, span.file, span));
			let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_string());
			(res!(crate::eval::import::read_file(engine, &path, span)), ext)
		},
		_ => return Err(engine.error(DiagnosticKind::Type, span, "image source must be a path or bytes")),
	};
	let format = match res!(g.val("format")) {
		Some(Value::Str(s))	=> img::format_named(&s),
		_ => match ext.as_deref().map(img::format_named) {
			Some(Ok(f))	=> Ok(f),
			_			=> img::detect_format(&data),
		},
	};
	let format = match format {
		Ok(f)	=> f,
		Err(e)	=> return Err(engine.error(DiagnosticKind::Type, span, last_msg(&e))),
	};
	let decoded = match img::decode(&data, format) {
		Ok(d)	=> d,
		Err(e)	=> return Err(engine.error(DiagnosticKind::Type, span, fmt!("failed to decode image ({})", last_msg(&e)))),
	};
	let (nw, nh)	= decoded.size_pt();
	let px_ratio	= nw / nh;
	let region_ratio = pod.w / pod.h;
	let wide		= px_ratio > region_ratio;
	let (tw, th) = if pod.expand_x && pod.expand_y {
		(pod.w, pod.h)
	} else if pod.expand_x {
		(pod.w, pod.h.min(pod.w / px_ratio))
	} else if pod.expand_y {
		(pod.w.min(pod.h * px_ratio), pod.h)
	} else {
		(nw.min(pod.w).min(pod.h * px_ratio), nh.min(pod.h).min(pod.w / px_ratio))
	};
	let fit = match res!(g.val("fit")) {
		Some(Value::Str(s))	=> s.to_string(),
		_					=> "cover".to_string(),
	};
	let (fw, fh) = match fit.as_str() {
		"stretch" => (tw, th),
		_ => if wide == (fit == "contain") { (tw, tw / px_ratio) } else { (th * px_ratio, th) },
	};
	let mut frame = Frame::new(fw, fh);
	let cover_clip = fit == "cover" && (fw > tw + 1e-9 || fh > th + 1e-9);
	match decoded {
		Decoded::Raster { image, .. } => {
			if cover_clip {
				// The visible part of a covering raster is cropped from its pixels, so it needs no clip: the
				// crop is the centred window of the fitted image the target shows.
				let (cropped, cw, ch) = crop_centred(&image, fw, fh, tw, th);
				frame = Frame::new(tw, th);
				frame.push(P2::default(), Item::Raster { image: Arc::new(cropped), w: cw, h: ch });
				return Ok(frame);
			}
			frame.push(P2::default(), Item::Raster { image: Arc::new(image), w: fw, h: fh });
		},
		Decoded::Vector { picture, .. } => {
			let sx = (fw / picture.width.max(1e-9) as f64) as f32;
			let sy = (fh / picture.height.max(1e-9) as f64) as f32;
			let ops = res!(svg_ops(engine, g.styles, picture.ops, GTransform::scale(sx, sy)));
			frame.push(P2::default(), Item::Ops(ops));
		},
	}
	frame.resize(tw, th, 0.5, 0.5);
	if cover_clip {
		frame.clip(Curve::rect(tw, th));
	}
	Ok(frame)
}

// The centred window of a raster fitted at `fw` by `fh` that a `tw` by `th` target shows, as whole pixels,
// and the size in points the kept pixels cover.
fn crop_centred(image: &RasterImage, fw: f64, fh: f64, tw: f64, th: f64) -> (RasterImage, f64, f64) {
	let kx	= image.width as f64 / fw;
	let ky	= image.height as f64 / fh;
	let cw	= ((tw * kx).round() as usize).clamp(1, image.width);
	let ch	= ((th * ky).round() as usize).clamp(1, image.height);
	let x0	= (image.width - cw) / 2;
	let y0	= (image.height - ch) / 2;
	let mut rgba = Vec::with_capacity(cw * ch * 4);
	for y in y0..y0 + ch {
		let row = (y * image.width + x0) * 4;
		rgba.extend_from_slice(&image.rgba[row..row + cw * 4]);
	}
	(RasterImage { width: cw, height: ch, rgba }, tw, th)
}

// An SVG's ops under `t`: paths and rasters mapped, live text shaped under the element's styles.
fn svg_ops(engine: &mut Engine, styles: &StyleChain, ops: Vec<SvgOp>, t: GTransform) -> Outcome<Vec<DrawOp>> {
	let mut out = Vec::with_capacity(ops.len());
	for op in ops {
		match op {
			SvgOp::Fill { path, colour } => out.push(DrawOp::Fill { path: res!(path.transform(&t)), colour: colour.into() }),
			SvgOp::Stroke { path, colour, stroke } => {
				let outline = res!(path.stroke(&stroke));
				out.push(DrawOp::Fill { path: res!(outline.transform(&t)), colour: colour.into() });
			},
			SvgOp::Text { text, local, x, y, size, anchor, colour, .. } => {
				let shaped = res!(crate::flow::text::shape(engine, &text, styles));
				// The run is shaped at the chain's size and scaled to the SVG's own font size.
				let k		= if shaped.size() > 0.0 { size / shaped.size() } else { 1.0 };
				let advance	= shaped.dims().width.to_pt() as f32 * k;
				let pen_x	= match anchor {
					oxedyne_fe2o3_graphics::svg_doc::Anchor::Start	=> x,
					oxedyne_fe2o3_graphics::svg_doc::Anchor::Middle	=> x - advance / 2.0,
					oxedyne_fe2o3_graphics::svg_doc::Anchor::End	=> x - advance,
				};
				for glyph in &shaped.run().glyphs {
					let o = res!(shaped.outline(glyph));
					if o.is_empty() {
						continue;
					}
					let place = GTransform::scale(k, -k)
						.then(&GTransform::translate(pen_x + glyph.x * k, y - glyph.y * k))
						.then(&local)
						.then(&t);
					out.push(DrawOp::Fill { path: res!(o.transform(&place)), colour: colour.into() });
				}
			},
			SvgOp::Image { rgba, iw, ih, x, y, w, h } => {
				let p0 = t.apply(Pt::new(x, y));
				let p1 = t.apply(Pt::new(x + w, y + h));
				out.push(DrawOp::Image {
					image:	Arc::new(RasterImage { width: iw, height: ih, rgba }),
					x:		p0.x,
					y:		p0.y,
					w:		p1.x - p0.x,
					h:		p1.y - p0.y,
				});
			},
		}
	}
	Ok(out)
}

// Transforms

fn origin_fraction(h: HAlign, v: VAlign) -> (f64, f64) {
	let x = match h {
		HAlign::Start | HAlign::Left	=> 0.0,
		HAlign::Center					=> 0.5,
		HAlign::End | HAlign::Right		=> 1.0,
	};
	let y = match v {
		VAlign::Top		=> 0.0,
		VAlign::Horizon	=> 0.5,
		VAlign::Bottom	=> 1.0,
	};
	(x, y)
}

fn body_of(g: &Get) -> Outcome<Content> {
	Ok(res!(g.content("body")).unwrap_or_default())
}

fn layout_move<F>(engine: &mut Engine, g: &Get, pod: Pod, body_fn: &mut F) -> Outcome<Frame>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let body		= res!(body_of(g));
	let mut frame	= res!(body_fn(engine, &body, g.styles, pod));
	let dx = match res!(g.val("dx")) { Some(v) => res!(vis::cast_rel(v)), None => Relative::default() };
	let dy = match res!(g.val("dy")) { Some(v) => res!(vis::cast_rel(v)), None => Relative::default() };
	let d = P2::new(g.rel(dx, pod.w), g.rel(dy, pod.h));
	// Typst moves the contents only: the size and the baseline stay where the body put them.
	frame.translate_visual(d);
	Ok(frame)
}

/// The corners of a `w` by `h` box under `t`: the offset that brings the image's top-left to the origin,
/// and the size of its bounding box.
fn bounding_box(w: f64, h: f64, t: &Transform) -> (P2, P2) {
	let pts = [P2::new(0.0, 0.0), P2::new(w, 0.0), P2::new(0.0, h), P2::new(w, h)]
		.map(|p| {
			// Typst's `transform_inf`: an infinite coordinate stays infinite rather than turning to NaN.
			let x = if p.x.is_infinite() || p.y.is_infinite() { p.x } else { p.apply(t).x };
			let y = if p.x.is_infinite() || p.y.is_infinite() { p.y } else { p.apply(t).y };
			P2::new(x, y)
		});
	let min_x = pts.iter().fold(f64::INFINITY, |m, p| m.min(p.x));
	let min_y = pts.iter().fold(f64::INFINITY, |m, p| m.min(p.y));
	let max_x = pts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.x));
	let max_y = pts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.y));
	(P2::new(-min_x, -min_y), P2::new((max_x - min_x).abs(), (max_y - min_y).abs()))
}

fn skew(ax: f64, ay: f64) -> Transform {
	Transform { a: 1.0, b: ay.tan(), c: ax.tan(), d: 1.0, e: 0.0, f: 0.0 }
}

fn layout_transform<F>(engine: &mut Engine, g: &Get, pod: Pod, body_fn: &mut F) -> Outcome<Frame>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let span	= g.elem.span();
	let body	= res!(body_of(g));
	let (h, v)	= res!(vis::origin_of(res!(g.val("origin"))));
	let origin	= origin_fraction(h, v);
	let reflow	= res!(g.bool("reflow", false));
	let (ts, size) = match g.kind {
		ElemKind::Rotate => {
			let angle = match res!(g.val("angle")) {
				Some(v)	=> res!(vis::cast_angle(v)).0,
				None	=> 0.0,
			};
			let size = if pod.finite() {
				bounding_box(pod.w, pod.h, &Transform::rotate(-angle)).1
			} else {
				P2::new(f64::INFINITY, f64::INFINITY)
			};
			(Transform::rotate(angle), size)
		},
		ElemKind::Skew => {
			let ax = match res!(g.val("ax")) { Some(v) => res!(vis::cast_angle(v)).0, None => 0.0 };
			let ay = match res!(g.val("ay")) { Some(v) => res!(vis::cast_angle(v)).0, None => 0.0 };
			let t = skew(ax, ay);
			let size = if pod.finite() {
				bounding_box(pod.w, pod.h, &t).1
			} else {
				P2::new(f64::INFINITY, f64::INFINITY)
			};
			(t, size)
		},
		_ => {
			let (sx, sy) = res!(resolve_scale(engine, g, pod, &body, body_fn));
			let size = P2::new(
				if pod.w.is_finite() { (pod.w / sx).abs() } else { pod.w },
				if pod.h.is_finite() { (pod.h / sy).abs() } else { pod.h },
			);
			(Transform::scale(sx, sy), size)
		},
	};
	let _ = span;
	let mut frame = if reflow {
		let measured	= res!(body_fn(engine, &body, g.styles, Pod::new(size.x, size.y, false, false)));
		res!(body_fn(engine, &body, g.styles, Pod::new(measured.width, measured.height, true, true)))
	} else {
		res!(body_fn(engine, &body, g.styles, pod))
	};
	let x	= frame.width * origin.0;
	let y	= frame.height * origin.1;
	let t	= Transform::translate(x, y).then(&ts).then(&Transform::translate(-x, -y));
	if reflow {
		// The baseline follows the transformed baseline's middle point for scaling (not mirrored) and
		// skewing; a rotation clears it, as its direction after turning is not clear.
		let keep = match g.kind {
			ElemKind::Rotate	=> false,
			ElemKind::Scale		=> ts.d >= 0.0,
			_					=> true,
		};
		let new_base = match (frame.baseline, keep) {
			(Some(b), true)	=> Some(P2::new(frame.width / 2.0, b).apply(&t).y),
			_				=> None,
		};
		let (offset, bb) = bounding_box(frame.width, frame.height, &t);
		frame.transform(t);
		frame.translate_visual(offset);
		frame.width		= bb.x;
		frame.height	= bb.y;
		match new_base {
			Some(b)			=> frame.baseline = Some(b + offset.y),
			None if !keep	=> frame.baseline = None,
			None			=> (),
		}
	} else {
		frame.transform(t);
	}
	Ok(frame)
}

fn resolve_scale<F>(engine: &mut Engine, g: &Get, pod: Pod, body: &Content, body_fn: &mut F) -> Outcome<(f64, f64)>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let span = g.elem.span();
	let x = res!(g.val("x"));
	let y = res!(g.val("y"));
	let needs_size = matches!(x, Some(Value::Length(_))) || matches!(y, Some(Value::Length(_)));
	let size = if needs_size {
		let f = res!(body_fn(engine, body, g.styles, Pod::new(pod.w, pod.h, false, false)));
		P2::new(f.width, f.height)
	} else {
		P2::default()
	};
	let axis = |v: Option<Value>, whole: f64| -> Outcome<Option<f64>> {
		match v {
			Some(Value::Auto)		=> Ok(None),
			Some(Value::Ratio(r))	=> Ok(Some(r.0)),
			Some(Value::Length(l))	=> Ok(Some(l.resolve(g.fs) / whole)),
			None					=> Ok(Some(1.0)),
			Some(other)				=> Err(err!("expected ratio, length, or auto, found {}", other.ty().name();
				Input, Mismatch)),
		}
	};
	match (res!(axis(x, size.x)), res!(axis(y, size.y))) {
		(None, None)			=> Err(engine.error(DiagnosticKind::Type, span, "x and y cannot both be auto")),
		(Some(a), Some(b))		=> Ok((a, b)),
		(None, Some(v)) | (Some(v), None)	=> Ok((v, v)),
	}
}

// Repeat

fn layout_repeat<F>(engine: &mut Engine, g: &Get, pod: Pod, body_fn: &mut F) -> Outcome<Frame>
	where F: FnMut(&mut Engine, &Content, &StyleChain, Pod) -> Outcome<Frame>
{
	let span	= g.elem.span();
	let body	= res!(body_of(g));
	let piece	= res!(body_fn(engine, &body, g.styles, Pod::new(pod.w, pod.h, false, false)));
	let w_fill	= pod.w;
	if !w_fill.is_finite() || !piece.height.is_finite() {
		return Err(engine.error(DiagnosticKind::Type, span, "repeat with no size restrictions"));
	}
	let mut frame = Frame::new(w_fill, piece.height);
	if piece.baseline.is_some() {
		frame.baseline = Some(piece.baseline());
	}
	let mut gap = match res!(g.val("gap")) {
		Some(Value::Length(l))	=> l.resolve(g.fs),
		_						=> 0.0,
	};
	let width		= piece.width;
	let count		= ((w_fill + gap) / (width + gap)).floor();
	let remaining	= (w_fill + gap) % (width + gap);
	let justify		= res!(g.bool("justify", true));
	if justify {
		gap += remaining / (count - 1.0);
	}
	let mut offset = 0.0;
	if count == 1.0 || !justify {
		offset += res!(align_x(g.styles)) * remaining;
	}
	if width > 0.0 {
		for _ in 0..(count.max(0.0) as usize).min(1000) {
			frame.push(P2::new(offset, 0.0), Item::Group {
				frame:		Box::new(piece.clone()),
				transform:	Transform::identity(),
				clip:		None,
			});
			offset += width + gap;
		}
	}
	Ok(frame)
}

// The horizontal alignment in force (`align`'s `alignment`), as a fraction: start is 0 in left-to-right text.
fn align_x(styles: &StyleChain) -> Outcome<f64> {
	let v = match ElemKind::Align.field_id("alignment") {
		Some(id)	=> res!(styles.get(ElemKind::Align, id)),
		None		=> None,
	};
	Ok(match v {
		Some(Value::Alignment(a)) => match a.x {
			Some(HAlign::Center)					=> 0.5,
			Some(HAlign::Right) | Some(HAlign::End)	=> 1.0,
			_										=> 0.0,
		},
		_ => 0.0,
	})
}

// Lowering to IR

/// A frame as one IR node: ink alone is one `Graphic` leaf; with laid-out content the layers are
/// overlaid in a vertical box (see the file header).
pub fn frame_to_node(engine: &mut Engine, frame: &Frame, span: Span) -> Outcome<Node> {
	let mut layers: Vec<Layer> = Vec::new();
	res!(collect_layers(engine, frame, P2::default(), &mut layers, span));
	let w		= frame.width;
	let h		= frame.height;
	let base	= frame.baseline().clamp(0.0, h.max(0.0));
	let dims	= Dims::new(Sp::from_pt(w), Sp::from_pt(base), Sp::from_pt(h - base));
	let only_ink = layers.iter().all(|l| matches!(l, Layer::Ink(_)));
	if only_ink {
		let mut ops = Vec::new();
		for l in layers {
			if let Layer::Ink(o) = l {
				ops.extend(o);
			}
		}
		// The graphic's frame is the box's: y down from the top, the baseline `base` below it.
		return Ok(Node::Leaf(Leaf::graphic(Graphic::new(ops, dims)).with_span(span.range())));
	}
	let full = Dims::new(Sp::from_pt(w), Sp::from_pt(h), Sp::ZERO);
	let mut list = Vec::new();
	let n = layers.len();
	for (i, l) in layers.into_iter().enumerate() {
		let layer = match l {
			Layer::Ink(ops) => Node::VBox(BoxNode::new(vec![
				Node::Leaf(Leaf::graphic(Graphic::new(ops, full)).with_span(span.range())),
			], full)),
			Layer::Nodes(at, nodes, d) => {
				let inner = Node::VBox(BoxNode::new(nodes, Dims::new(d.width, d.vextent(), Sp::ZERO)));
				Node::VBox(BoxNode::new(vec![
					Node::Glue(Glue::fixed(Sp::from_pt(at.y))),
					Node::HBox(BoxNode::new(vec![
						Node::Glue(Glue::fixed(Sp::from_pt(at.x))),
						inner,
					], Dims::new(Sp::from_pt(at.x) + d.width, d.vextent(), Sp::ZERO))),
				], full))
			},
		};
		list.push(layer);
		if i + 1 < n {
			list.push(Node::Glue(Glue::fixed(-full.height)));
		}
	}
	Ok(Node::VBox(BoxNode::new(list, dims)))
}

enum Layer {
	Ink(Vec<DrawOp>),
	Nodes(P2, Vec<Node>, Dims),
}

fn push_ink(layers: &mut Vec<Layer>, ops: Vec<DrawOp>) {
	if let Some(Layer::Ink(v)) = layers.last_mut() {
		v.extend(ops);
		return;
	}
	layers.push(Layer::Ink(ops));
}

fn collect_layers(engine: &mut Engine, frame: &Frame, at: P2, layers: &mut Vec<Layer>, span: Span) -> Outcome<()> {
	for (p, item) in &frame.items {
		let pos = at.add(*p);
		match item {
			Item::Shape(s) => {
				let ops = res!(shape_ops(engine, s, pos, span));
				push_ink(layers, ops);
			},
			Item::Raster { image, w, h } => push_ink(layers, vec![DrawOp::Image {
				image:	image.clone(),
				x:		pos.x as f32,
				y:		pos.y as f32,
				w:		*w as f32,
				h:		*h as f32,
			}]),
			Item::Ops(ops) => {
				let t = GTransform::translate(pos.x as f32, pos.y as f32);
				let mut v = Vec::with_capacity(ops.len());
				for op in ops {
					v.push(res!(op_transform(op, &t, span, engine)));
				}
				push_ink(layers, v);
			},
			Item::Nodes { nodes, dims } => layers.push(Layer::Nodes(pos, nodes.clone(), *dims)),
			Item::Group { frame: inner, transform, clip } => {
				let identity = *transform == Transform::identity();
				if identity && clip.is_none() {
					res!(collect_layers(engine, inner, pos, layers, span));
					continue;
				}
				let node	= res!(frame_to_node(engine, inner, span));
				let t		= Transform::translate(pos.x, pos.y).then(transform);
				let dims	= Dims::new(Sp::from_pt(inner.width), Sp::from_pt(inner.height), Sp::ZERO);
				let mut out = match clip {
					Some(c) => {
						let path = res!(c.to_path(P2::default()));
						res!(wrap_clip(ClipNode { list: vec![node], dims, path: Some(path) }))
					},
					None => node,
				};
				if !identity || pos != P2::default() {
					out = res!(wrap_transform(engine, TransformNode { transform: t, list: vec![out], dims }, span));
				}
				match out {
					Node::Leaf(Leaf { kind: LeafKind::Graphic(gr), .. }) => push_ink(layers, gr.ops.clone()),
					other => layers.push(Layer::Nodes(P2::default(), vec![other], dims)),
				}
			},
		}
	}
	Ok(())
}

/// Transformed material as a node: the transform node itself, which the driver places as a group.
pub fn wrap_transform(_engine: &mut Engine, t: TransformNode, _span: Span) -> Outcome<Node> {
	Ok(Node::Transform(t))
}

/// Clipped material as a node: the clip node itself, which the driver places as a clipping group.
pub fn wrap_clip(c: ClipNode) -> Outcome<Node> {
	Ok(Node::Clip(c))
}

fn op_transform(op: &DrawOp, t: &GTransform, span: Span, engine: &mut Engine) -> Outcome<DrawOp> {
	Ok(match op {
		DrawOp::Fill { path, colour }			=> DrawOp::Fill { path: res!(path.transform(t)), colour: *colour },
		DrawOp::Stroke { path, colour, width }	=> DrawOp::Stroke {
			path:	res!(path.transform(t)),
			colour:	*colour,
			width:	*width * t.scale_factor(),
		},
		DrawOp::Image { image, x, y, w, h } => {
			if t.b != 0.0 || t.c != 0.0 || t.a < 0.0 || t.d < 0.0 {
				return Err(engine.error(DiagnosticKind::Unsupported, span,
					"a raster image cannot yet be rotated, skewed or mirrored: the drawing layer places \
					rasters upright"));
			}
			let p0 = t.apply(Pt::new(*x, *y));
			let p1 = t.apply(Pt::new(*x + *w, *y + *h));
			DrawOp::Image { image: image.clone(), x: p0.x, y: p0.y, w: p1.x - p0.x, h: p1.y - p0.y }
		},
	})
}

/// A shape's fill and stroke as draw ops at `at`. A stroke is outlined and filled, so caps, joins, dashes
/// and the miter limit are drawn exactly whatever the emitter supports.
pub fn shape_ops(engine: &mut Engine, s: &Shape, at: P2, span: Span) -> Outcome<Vec<DrawOp>> {
	let mut ops = Vec::new();
	let path = res!(s.curve.to_path(at));
	if let Some(p) = &s.fill {
		if let Some(colour) = res!(paint_ink(engine, p, span)) {
			let fill_path = match s.fill_rule {
				FillRule::EvenOdd	=> res!(path.even_odd_as_non_zero()),
				FillRule::NonZero	=> path.clone(),
			};
			ops.push(DrawOp::Fill { path: fill_path, colour });
		}
	}
	if let Some(st) = &s.stroke {
		if st.thickness > 0.0 {
			if let Some(colour) = res!(paint_ink(engine, &st.paint, span)) {
				let pen = res!(pen_of(st));
				ops.push(DrawOp::Fill { path: res!(path.stroke(&pen)), colour });
			}
		}
	}
	Ok(ops)
}

/// The `fe2o3_graphics` pen for a fixed stroke.
pub fn pen_of(st: &FixedStroke) -> Outcome<Pen> {
	let mut pen = res!(Pen::new(st.thickness as f32));
	pen = pen
		.with_cap(match st.cap {
			LineCap::Butt	=> Cap::Butt,
			LineCap::Round	=> Cap::Round,
			LineCap::Square	=> Cap::Square,
		})
		.with_join(match st.join {
			LineJoin::Miter	=> Join::Miter,
			LineJoin::Round	=> Join::Round,
			LineJoin::Bevel	=> Join::Bevel,
		})
		.with_miter_limit((st.miter_limit as f32).max(1.0))
		.with_tolerance(STROKE_TOL);
	if let Some((pattern, phase)) = &st.dash {
		if pattern.iter().sum::<f64>() > 0.0 {
			pen = pen.with_dash(PenDash::new(pattern.iter().map(|x| *x as f32).collect()).with_offset(*phase as f32));
		}
	}
	Ok(pen)
}

/// A paint as a flat ink in the colour's own space. A gradient is drawn in its middle stop's colour and a
/// tiling not at all, each with a warning, because the drawing layer carries flat colours only.
pub fn paint_ink(engine: &mut Engine, p: &Paint, span: Span) -> Outcome<Option<Ink>> {
	match p {
		Paint::Color(c) => Ok(Some(c.to_ink())),
		Paint::Gradient(g) => {
			engine.warn(DiagnosticKind::Unsupported, span, "gradients are drawn in one flat colour: the drawing layer has no shading");
			match g.stops.get(g.stops.len() / 2) {
				Some((c, _))	=> Ok(Some(c.to_ink())),
				None			=> Ok(None),
			}
		},
		Paint::Tiling(_) => {
			engine.warn(DiagnosticKind::Unsupported, span, "tiling paint is not drawn: the drawing layer has no tiling");
			Ok(None)
		},
	}
}

// Node lists

/// A vertical list's natural size: its widest box and the sum of its extents.
pub fn vlist_dims(nodes: &[Node]) -> Dims {
	let mut w = Sp::ZERO;
	let mut h = Sp::ZERO;
	for n in nodes {
		let nw = match n {
			Node::HBox(b) | Node::VBox(b)	=> b.dims.width,
			Node::Leaf(l)					=> l.dims.width,
			_								=> Sp::ZERO,
		};
		if nw > w {
			w = nw;
		}
		h += n.vextent();
	}
	Dims::new(w, h, Sp::ZERO)
}

/// The first baseline in a vertical list: the first line's, from the list's top.
pub fn first_baseline(nodes: &[Node]) -> Option<Sp> {
	let mut y = Sp::ZERO;
	for n in nodes {
		match n {
			Node::HBox(b)	=> return Some(y + b.dims.height),
			Node::Leaf(l)	=> return Some(y + l.dims.height),
			Node::VBox(b)	=> if let Some(inner) = first_baseline(&b.list) {
				return Some(y + inner);
			},
			_ => (),
		}
		y += n.vextent();
	}
	None
}

// The innermost message of an error, which is what a diagnostic reports.
fn last_msg(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}
