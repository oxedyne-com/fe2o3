//! An SVG document, read into flat drawing operations.
//!
//! Where [`crate::svg`] reads only the `d` of one `<path>`, this reads the element tree a whole file
//! carries: the `viewBox`, the nested `<g transform>` frames, the shapes (`<path>`, `<rect>`, `<circle>`,
//! `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`), and the `<use>` references that place an outline
//! held in `<defs>`. Two kinds of file are the case in hand, and the reader spans both. One is what a
//! typesetter emits -- Typst's cetz plots -- a regular subset of paths, `<use>` glyphs and
//! `translate`/`matrix` transforms with colour as `#rrggbb`. The other is what an illustrator emits --
//! Inkscape -- where presentation cascades down the group tree (`<g fill="#f00">` colours its children),
//! the same properties may arrive as a `style="fill:#f00"` attribute instead, primitives stand in for
//! paths, opacity rides its own attributes, and `fill-rule="evenodd"` asks a holed shape to fill with
//! its overlaps read as holes.
//!
//! Live `<text>`/`<tspan>` runs an Inkscape file keeps are read, but not shaped: this reader has no font,
//! so a run comes out as an [`SvgOp::Text`] carrying the string, the anchor point, the enclosing frame,
//! the size and the face hints, for a caller that does have a font set to shape to glyph outlines. An
//! embedded `<image>` (a base64 PNG or JPEG) is decoded here to straight RGBA and placed as an
//! [`SvgOp::Image`]. What is still left at the door: `<clipPath>` and markers (arrowheads);
//! `filter`/`pattern`; a `<text>` rotated or sheared by its frame keeps its position but is shaped upright;
//! and an `<image>` under a rotation or shear is placed by its axis-aligned bounds.
//! A gradient fill or stroke (`url(#id)`) comes out as an [`SvgOp::Shade`]: a linear or radial
//! [`Shading`] whose frame carries the gradient's `gradientTransform` and, for `objectBoundingBox`, the
//! shape's bounding box, its stops and every unset attribute taken along the `href` chain as Inkscape
//! writes them (SVG 1.1 13.2). Only `spreadMethod="pad"` is modelled (`reflect` and `repeat` are read as
//! pad), and a radial's focal point (`fx`/`fy`) is taken to be its centre. A reference to nothing draws
//! nothing rather than failing the read. A file reaching past all this is read as far as it fits and the rest is left.
//!
//! The one thing worth stating is what happens to a typesetter's text. It does not leave `<text>` in its
//! SVG; it bakes each glyph to an outline, files the outline once as a `<symbol>`, and places it with a
//! `<use>` whose enclosing group carries the position and the y-flip that turns a font's y-up outline the
//! right way up. So no font is needed to read that text back: the outlines are already in the file, and a
//! `<use>` is just a filled path fetched by id. An illustrator's `<text>`, by contrast, is still live
//! text, so the reader hands it on unshaped for the caller with a font to bake -- see [`SvgOp::Text`].
//!
//! The geometry comes out in the `viewBox`'s own units, y down, which for a Typst file are points. A
//! caller sizing the picture to a figure width scales every path by one factor; nothing here bakes that
//! in, so the picture is read once and drawn at whatever size is wanted.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::colour::{
	Gradient,
	Ink,
	Rgba,
	Shading,
	Stop,
};
use crate::path::{
	Bounds,
	Path,
	PathBuilder,
	Pt,
	Seg,
};
use crate::pixmap::Tone;
use crate::stroke::{
	Cap,
	Dash,
	Join,
	Stroke,
};
use crate::transform::Transform;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::base64;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Node,
	Xml,
};

use std::collections::HashMap;
use std::rc::Rc;

/// One drawing operation read from an SVG document: a filled or a stroked path, a live text run, or a
/// decoded raster, in the document's own frame (y down, `viewBox` units). Path geometry is flattened of
/// its element tree -- every transform baked in -- so a caller need only scale and place it. A text run,
/// which needs a font this reader has none of, is instead handed on with its own frame (`local`), for the
/// caller to shape; a raster carries its pixels and a placement rectangle already in the picture frame.
#[derive(Clone, Debug)]
pub enum SvgOp {
	Fill { path: Path, colour: Rgba },
	// A path filled with a gradient; a gradient stroke arrives as its stroked outline, filled.
	Shade { path: Path, shading: Shading },
	Stroke { path: Path, colour: Rgba, stroke: Stroke },
	// A live `<text>`/`<tspan>` run left unshaped: `local` maps its own frame (y down) to the picture
	// frame, `x`/`y` are the anchor and baseline in that frame, `size` the font-size in its units.
	Text {
		text:	String,
		local:	Transform,
		x:		f32,
		y:		f32,
		size:	f32,
		anchor:	Anchor,
		italic:	bool,
		bold:	bool,
		colour:	Rgba,
	},
	// A decoded raster and the rectangle it fills, top-left (x, y), w wide and h tall, in the picture frame.
	Image {
		rgba:	Vec<u8>,	// straight RGBA, row-major, top row first
		tone:	Tone,		// whether the embedded file held grey alone
		iw:		usize,		// image pixel width
		ih:		usize,		// image pixel height
		x:		f32,
		y:		f32,
		w:		f32,
		h:		f32,
	},
}

/// Where a `<text>` run's anchor point sits along the run: at its start, middle or end, per SVG's
/// `text-anchor`. The caller applies it once the run's advance is known from shaping.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
	Start,
	Middle,
	End,
}

/// A read SVG document: its drawing operations and the size of its `viewBox`, in the `viewBox`'s units.
#[derive(Clone, Debug)]
pub struct SvgPicture {
	pub ops:	Vec<SvgOp>,
	pub width:	f32,	// viewBox width, in its own units (points, for a typesetter's output)
	pub height:	f32,	// viewBox height
}

/// The store `<defs>` fills for the draw walk to draw from: outlines a `<use>` fetches by id, and the
/// gradients a `url(#id)` paint resolves to.
struct Defs {
	outlines:	HashMap<String, Path>,			// id -> concatenated outline, for <use>
	gradients:	HashMap<String, Rc<GradDef>>,	// id -> the gradient, its href chain resolved
}

/// A gradient element as written, before its `href` chain is followed: each attribute it sets, its stops
/// if it has any, and the id it links to.
struct GradSrc {
	radial:	bool,
	attrs:	HashMap<&'static str, String>,
	stops:	Option<Vec<Stop>>,
	href:	Option<String>,
}

// The attributes a gradient inherits along its href; the first group only from a gradient of its own kind.
const LINEAR_GEOM:	[&str; 4] = ["x1", "y1", "x2", "y2"];
const RADIAL_GEOM:	[&str; 3] = ["cx", "cy", "r"];
const SHARED:		[&str; 2] = ["gradientUnits", "gradientTransform"];

/// A gradient ready to place on a shape: its geometry (`x1 y1 x2 y2`, or `cx cy r` and a zero), read in
/// user units for `userSpaceOnUse` and as fractions of the box for `objectBoundingBox`.
#[derive(Debug)]
struct GradDef {
	radial:		bool,
	geom:		[f32; 4],
	user:		bool,		// gradientUnits="userSpaceOnUse"
	transform:	Transform,	// gradientTransform
	stops:		Vec<Stop>,
}

impl GradDef {
	/// The shading this gradient paints on a shape whose own-frame bounds are `bbox`, the shape placed in
	/// the picture by `local`. A bounding-box gradient on a shape with no width or height paints nothing,
	/// as SVG has it.
	fn shading(&self, bbox: Option<Bounds>, local: &Transform) -> Option<Shading> {
		let unit = if self.user {
			Transform::IDENTITY
		} else {
			let b = bbox?;
			let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
			if !(w > 0.0) || !(h > 0.0) {
				return None;
			}
			Transform { a: w, b: 0.0, c: 0.0, d: h, e: b.x0, f: b.y0 }
		};
		let g		= self.geom;
		let stops	= self.stops.clone();
		let grad = if !self.radial {
			Gradient::Linear { from: (g[0], g[1]), to: (g[2], g[3]), stops }
		} else if g[2] > 0.0 {
			Gradient::Radial { centre: (g[0], g[1]), radius: g[2], stops }
		} else {
			// A zero radius paints the last stop's colour (SVG 1.1 13.2.3): a gradient of no extent.
			Gradient::Linear { from: (g[0], g[1]), to: (g[0], g[1]), stops }
		};
		Some(Shading { grad, frame: self.transform.then(&unit).then(local) })
	}

	/// The flat mean of the stops, for a text run, which takes one colour.
	fn mean(&self) -> Rgba {
		Shading { grad: Gradient::Linear { from: (0.0, 0.0), to: (1.0, 0.0), stops: self.stops.clone() },
			frame: Transform::IDENTITY }.mean()
	}
}

/// A resolved `fill` or `stroke` paint.
#[derive(Clone)]
enum Fill {
	Flat(Rgba),
	Grad(Rc<GradDef>),
}

impl Fill {
	/// The one colour a consumer without shading paints this with.
	fn colour(&self) -> Rgba {
		match self {
			Self::Flat(c)	=> *c,
			Self::Grad(g)	=> g.mean(),
		}
	}
}

/// The presentation state inherited down the group tree: fill and stroke and their opacities, the pen's
/// width and joins, the dash pattern, the fill rule, and the cumulative group opacity. Every field
/// carries down to a child, which overrides only the ones its own attributes or `style` set. The initial
/// state is SVG's own defaults: a black fill, no stroke, full opacity, non-zero winding.
#[derive(Clone)]
struct Paint {
	fill:			Option<Fill>,	// resolved fill paint, or None for `fill:none`
	fill_opacity:	f32,
	even_odd:		bool,			// fill-rule="evenodd"
	stroke:			Option<Fill>,	// resolved stroke paint, or None for no stroke
	stroke_opacity:	f32,
	width:			f32,			// stroke width, in the element's own frame
	cap:			Cap,
	join:			Join,
	miter:			f32,
	dash:			Option<Vec<f32>>,
	dash_offset:	f32,
	opacity:		f32,			// cumulative group opacity, folded into every emitted alpha
	font_size:		f32,			// inherited font-size, in the element's own frame; 0 until a font sets one
	text_anchor:	Anchor,			// inherited text-anchor
	italic:			bool,			// inherited font-style: italic
	bold:			bool,			// inherited font-weight: bold
}

impl Default for Paint {
	fn default() -> Self {
		Self {
			fill:			Some(Fill::Flat(Rgba::BLACK)),
			fill_opacity:	1.0,
			even_odd:		false,
			stroke:			None,
			stroke_opacity:	1.0,
			width:			1.0,
			cap:			Cap::Butt,
			join:			Join::Miter,
			miter:			4.0,
			dash:			None,
			dash_offset:	0.0,
			opacity:		1.0,
			font_size:		0.0,
			text_anchor:	Anchor::Start,
			italic:			false,
			bold:			false,
		}
	}
}

/// Reads an SVG document into a flat [`SvgPicture`].
///
/// The tree is walked once in document order, a transform and a presentation state accumulated down each
/// branch, and every shape and `<use>` turned into an [`SvgOp`] with its transform already applied.
/// `<defs>` is read first, for the outlines a `<use>` will fetch and the mean colours a gradient fill
/// will resolve to, then skipped in the draw walk.
pub fn read_document(src: &str) -> Outcome<SvgPicture> {
	let xml		= res!(Xml::parse(src));
	let root	= res!(xml.root());
	if root.name.local() != "svg" {
		return Err(err!(
			"An SVG document's root is <svg>, but this one's is <{}>.", root.name.local();
			Invalid, Input));
	}

	// The viewBox sets the coordinate frame and the picture's size. A file with none falls back to its
	// width/height, and failing that to a unit box, so a malformed header still reads rather than stops.
	let (vx, vy, vw, vh) = res!(view_box(root));

	let mut defs = Defs { outlines: HashMap::new(), gradients: HashMap::new() };
	let mut srcs: HashMap<String, GradSrc> = HashMap::new();
	collect_defs(root, &mut defs, &mut srcs);
	defs.gradients = resolve_gradient_links(&srcs, vw, vh);

	// Everything is expressed relative to the viewBox origin, so the walk begins with a translation that
	// carries that origin to (0, 0); a Typst file's origin is already there and the translation is nil.
	let base = Transform::translate(-vx, -vy);
	let mut ops: Vec<SvgOp> = Vec::new();
	res!(walk(root, &base, &Paint::default(), &defs, &xml, &mut ops));

	Ok(SvgPicture { ops, width: vw, height: vh })
}

/// The viewBox as `(min-x, min-y, width, height)`, or a fallback drawn from `width`/`height`.
fn view_box(root: &Elem) -> Outcome<(f32, f32, f32, f32)> {
	if let Some(vb) = root.attr("viewBox") {
		let n = numbers(vb);
		if n.len() == 4 {
			return Ok((n[0], n[1], n[2], n[3]));
		}
	}
	let w = root.attr("width").map(length_pt).unwrap_or(0.0);
	let h = root.attr("height").map(length_pt).unwrap_or(0.0);
	if w > 0.0 && h > 0.0 {
		return Ok((0.0, 0.0, w, h));
	}
	Err(err!(
		"An SVG document needs a viewBox or a width and height to set its size; this one has neither.";
		Invalid, Input, Missing))
}

/// Gathers the `<defs>` store in one descent: every element with an `id` that yields an outline (for a
/// `<use>` to fetch), and every gradient as written (for a `url(#id)` paint to resolve to). The search
/// descends the whole tree, since Inkscape files an id-bearing shape wherever it likes, not only under
/// `<defs>`. The gradients are settled by [`resolve_gradient_links`] once every one is known.
fn collect_defs(elem: &Elem, out: &mut Defs, srcs: &mut HashMap<String, GradSrc>) {
	for child in elem.elems() {
		let name = child.name.local();
		match name {
			"linearGradient" | "radialGradient" => {
				if let Some(id) = child.attr("id") {
					srcs.insert(id.to_string(), gradient_src(child, name == "radialGradient"));
				}
			},
			_ => {
				// Any id-bearing element that carries an outline can be the target of a <use>. The
				// outline is baked in the frame a <use> expects: the element's own transform applied,
				// which a font glyph relies on to carry its em-square scale (`scale(0.015625)`).
				if let Some(id) = child.attr("id") {
					let mut pb = PathBuilder::new();
					gather_paths(child, &Transform::IDENTITY, &mut pb);
					if let Ok(path) = pb.finish() {
						if !path.is_empty() {
							out.outlines.insert(id.to_string(), path);
						}
					}
				}
			},
		}
		collect_defs(child, out, srcs);
	}
}

/// Settles every gradient along its `href` chain, as SVG 1.1 13.2 has a gradient take its stops, and any
/// attribute it leaves unset, from the one it links to: the geometry only from a gradient of its own kind,
/// the units, the transform and the stops from either. Inkscape writes every positioned gradient this way,
/// a stopless link over a shared swatch. A chain is followed at most once round the whole set, so a cycle
/// ends; a gradient that finds no stops is left out, and a fill naming it is unpainted. `vw`/`vh` are the
/// viewport a `userSpaceOnUse` percentage is read against.
fn resolve_gradient_links(srcs: &HashMap<String, GradSrc>, vw: f32, vh: f32)
	-> HashMap<String, Rc<GradDef>>
{
	let mut out = HashMap::new();
	for (id, own) in srcs {
		let mut attrs: HashMap<&'static str, String> = HashMap::new();
		let mut stops: Option<Vec<Stop>> = None;
		let mut cur = own;
		for _ in 0..=srcs.len() {
			let same_kind = cur.radial == own.radial;
			for (k, v) in &cur.attrs {
				let geom = LINEAR_GEOM.contains(k) || RADIAL_GEOM.contains(k);
				if (!geom || same_kind) && !attrs.contains_key(k) {
					attrs.insert(*k, v.clone());
				}
			}
			if stops.is_none() {
				stops = cur.stops.clone();
			}
			match cur.href.as_ref().and_then(|h| srcs.get(h)) {
				Some(next)	=> cur = next,
				None		=> break,
			}
		}
		let stops = match stops {
			Some(st) if !st.is_empty()	=> st,
			_							=> continue,
		};
		let user = attrs.get("gradientUnits").map(|u| u.trim() == "userSpaceOnUse").unwrap_or(false);
		let transform = attrs.get("gradientTransform").map(|t| parse_transform(t))
			.unwrap_or(Transform::IDENTITY);
		// A coordinate in user units, or a fraction of the box; a percentage is of the viewport in user
		// space (the radius of its normalised diagonal) and of the box otherwise.
		let diag = ((vw * vw + vh * vh) / 2.0).sqrt();
		let coord = |k: &str, default: f32, span: f32| -> f32 {
			match attrs.get(k) {
				Some(v) => {
					let v = v.trim();
					match v.strip_suffix('%') {
						Some(p)	=> number(p) / 100.0 * if user { span } else { 1.0 },
						None	=> length_num(v),
					}
				},
				None => default * if user { span } else { 1.0 },
			}
		};
		let geom = if own.radial {
			[coord("cx", 0.5, vw), coord("cy", 0.5, vh), coord("r", 0.5, diag), 0.0]
		} else {
			[coord("x1", 0.0, vw), coord("y1", 0.0, vh), coord("x2", 1.0, vw), coord("y2", 0.0, vh)]
		};
		out.insert(id.clone(), Rc::new(GradDef { radial: own.radial, geom, user, transform, stops }));
	}
	out
}

/// One gradient element as written: the attributes it sets, its stops (each `stop-opacity` folded into
/// the alpha, an offset short of the one before it raised to that one, as SVG 1.1 13.2.4 has it) or none
/// when it carries none, and the id its `href` names.
fn gradient_src(grad: &Elem, radial: bool) -> GradSrc {
	let mut attrs = HashMap::new();
	let keys: &[&'static str] = if radial { &RADIAL_GEOM } else { &LINEAR_GEOM };
	for k in keys.iter().chain(SHARED.iter()) {
		if let Some(v) = grad.attr(k) {
			attrs.insert(*k, v.to_string());
		}
	}
	let mut stops: Vec<Stop> = Vec::new();
	let mut last = 0.0f32;
	for stop in grad.elems() {
		if stop.name.local() != "stop" {
			continue;
		}
		let style	= stop.attr("style").unwrap_or("");
		let col		= prop(stop, style, "stop-color")
			.and_then(|v| paint_colour(&v, None))
			.unwrap_or(Rgba::BLACK);
		let op = prop(stop, style, "stop-opacity")
			.map(|v| number(&v).clamp(0.0, 1.0))
			.unwrap_or(1.0);
		let at = match stop.attr("offset").map(|o| o.trim()) {
			Some(o) => match o.strip_suffix('%') {
				Some(p)	=> number(p) / 100.0,
				None	=> number(o),
			},
			None => 0.0,
		};
		let at = at.clamp(0.0, 1.0).max(last);
		last = at;
		stops.push(Stop { at, colour: Ink::Rgb(fade(col, op)) });
	}
	let href = grad.attr("xlink:href").or_else(|| grad.attr("href"))
		.and_then(|h| h.trim().strip_prefix('#'))
		.map(|h| h.to_string());
	GradSrc { radial, attrs, stops: if stops.is_empty() { None } else { Some(stops) }, href }
}

/// Appends every shape outline held under an element into one builder, descending through any groups and
/// composing each element's own `transform` as it goes. Primitive shapes are turned to paths on the way,
/// so a `<use>` of a group of circles fetches them all, and a glyph's em-square scale rides down with it.
fn gather_paths(elem: &Elem, ctx: &Transform, pb: &mut PathBuilder) {
	let local = child_transform(elem, ctx);
	// The element's own shape, when it is one, in its baked frame, before its children.
	if let Some(path) = shape_path(elem) {
		if let Ok(placed) = path.transform(&local) {
			append_path(pb, &placed);
		}
	}
	for child in elem.elems() {
		gather_paths(child, &local, pb);
	}
}

/// Replays one path's segments into a builder, so several outlines become one.
fn append_path(pb: &mut PathBuilder, path: &Path) {
	for seg in path.segs() {
		match *seg {
			Seg::MoveTo(p)			=> pb.move_to(p),
			Seg::LineTo(p)			=> pb.line_to(p),
			Seg::QuadTo(c, p)		=> pb.quad_to(c, p),
			Seg::CubicTo(c0, c1, p)	=> pb.cubic_to(c0, c1, p),
			Seg::Close				=> pb.close(),
		}
	}
}

/// Walks the draw tree, emitting an [`SvgOp`] for every painted shape and `<use>`.
///
/// `ctx` maps this element's local frame to the picture frame; a group's `transform` composes onto it for
/// its children. `paint` is the presentation state inherited to this point; each element overrides only
/// what its own attributes or `style` set, and hands the rest down. `<defs>` and the gradient elements
/// are the store, gathered already, so they draw nothing here.
fn walk(
	elem:	&Elem,
	ctx:	&Transform,
	paint:	&Paint,
	defs:	&Defs,
	xml:	&Xml,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	for child in elem.elems() {
		let name = child.name.local();
		match name {
			"defs" | "symbol" | "linearGradient" | "radialGradient" | "clipPath"
				| "marker" | "mask" | "pattern" | "filter" | "title" | "desc" | "metadata"
				=> {},	// the store and the unrendered furniture, not drawn in place
			"g" | "a" | "svg" => {
				let local = child_transform(child, ctx);
				let sub = resolve_paint(paint, child, defs);
				res!(walk(child, &local, &sub, defs, xml, ops));
			},
			"use" => {
				let sub = resolve_paint(paint, child, defs);
				res!(emit_use(child, ctx, &sub, defs, ops));
			},
			"text" => {
				// A live text run, handed on unshaped; its `<tspan>` children are read here, not descended.
				let sub = resolve_paint(paint, child, defs);
				res!(emit_text(child, ctx, &sub, defs, xml, ops));
			},
			"image" => {
				res!(emit_image(child, ctx, ops));
			},
			_ => {
				// A shape is painted with its resolved state; an unmodelled container may still hold
				// drawable children, so it is descended with its own state resolved.
				let sub = resolve_paint(paint, child, defs);
				if let Some(shape) = shape_path(child) {
					let local = child_transform(child, ctx);
					res!(emit_shape(shape, &local, &sub, ops));
				} else {
					let local = child_transform(child, ctx);
					res!(walk(child, &local, &sub, defs, xml, ops));
				}
			},
		}
	}
	Ok(())
}

/// Resolves an element's presentation state from the inherited one: its `style` properties (which win)
/// and its presentation attributes (which fall back), each overriding only what it names.
fn resolve_paint(base: &Paint, elem: &Elem, defs: &Defs) -> Paint {
	let mut p	= base.clone();
	let style	= elem.attr("style").unwrap_or("");

	if let Some(v) = prop(elem, style, "fill") {
		p.fill = resolve_fill(&v, defs);
	}
	if let Some(v) = prop(elem, style, "fill-opacity") {
		p.fill_opacity = number(&v).clamp(0.0, 1.0);
	}
	if let Some(v) = prop(elem, style, "fill-rule") {
		p.even_odd = v.trim() == "evenodd";
	}
	if let Some(v) = prop(elem, style, "stroke") {
		p.stroke = resolve_fill(&v, defs);
	}
	if let Some(v) = prop(elem, style, "stroke-opacity") {
		p.stroke_opacity = number(&v).clamp(0.0, 1.0);
	}
	if let Some(v) = prop(elem, style, "stroke-width") {
		p.width = number(&v).max(0.0);
	}
	if let Some(v) = prop(elem, style, "stroke-linecap") {
		p.cap = match v.trim() {
			"round"		=> Cap::Round,
			"square"	=> Cap::Square,
			_			=> Cap::Butt,
		};
	}
	if let Some(v) = prop(elem, style, "stroke-linejoin") {
		p.join = match v.trim() {
			"round"	=> Join::Round,
			"bevel"	=> Join::Bevel,
			_		=> Join::Miter,
		};
	}
	if let Some(v) = prop(elem, style, "stroke-miterlimit") {
		p.miter = number(&v).max(1.0);
	}
	if let Some(v) = prop(elem, style, "stroke-dasharray") {
		let pattern = numbers(&v);
		if pattern.is_empty() || pattern.iter().all(|&x| x <= 0.0) || v.trim() == "none" {
			p.dash = None;
		} else {
			p.dash = Some(pattern);
		}
	}
	if let Some(v) = prop(elem, style, "stroke-dashoffset") {
		p.dash_offset = number(&v);
	}
	if let Some(v) = prop(elem, style, "opacity") {
		// Group opacity is not a paint of its own; it scales everything the subtree draws.
		p.opacity *= number(&v).clamp(0.0, 1.0);
	}
	// The font state cascades like the paint, so a `<g font-size=.. text-anchor=middle>` sets it for the
	// `<text>` runs beneath, which is where an Inkscape file often keeps it rather than on the text itself.
	if let Some(v) = prop(elem, style, "font-size") {
		let n = length_num(&v);
		if n > 0.0 {
			p.font_size = n;
		}
	}
	if let Some(v) = prop(elem, style, "font-style") {
		let t = v.trim();
		p.italic = t == "italic" || t == "oblique";
	}
	if let Some(v) = prop(elem, style, "font-weight") {
		let t = v.trim();
		p.bold = t == "bold" || t == "bolder"
			|| t.parse::<f32>().map(|w| w >= 600.0).unwrap_or(false);
	}
	if let Some(v) = prop(elem, style, "text-anchor") {
		p.text_anchor = parse_anchor(&v);
	}
	p
}

/// SVG's `text-anchor` (or the `text-align` shorthand Inkscape sometimes writes) as an [`Anchor`].
fn parse_anchor(v: &str) -> Anchor {
	match v.trim() {
		"middle" | "center"	=> Anchor::Middle,
		"end" | "right"		=> Anchor::End,
		_					=> Anchor::Start,
	}
}

/// A property's value, from the element's `style` first and its presentation attribute second, so the
/// `style` wins the SVG cascade as it should. Returned owned, since a `style` value is a slice of a
/// larger string this does not keep.
fn prop(elem: &Elem, style: &str, name: &str) -> Option<String> {
	if let Some(v) = style_prop(style, name) {
		return Some(v);
	}
	elem.attr(name).map(|s| s.to_string())
}

/// One declaration's value from a `style` attribute -- `name:value;name:value` -- or `None`. The scan is
/// literal: property names in these files carry no whitespace or comments to normalise.
fn style_prop(style: &str, name: &str) -> Option<String> {
	for decl in style.split(';') {
		let mut it = decl.splitn(2, ':');
		let key = it.next().unwrap_or("").trim();
		if key == name {
			if let Some(val) = it.next() {
				return Some(val.trim().to_string());
			}
		}
	}
	None
}

/// Resolves a `fill`/`stroke` value to a paint, or `None` for `none`, an unknown paint, or a reference
/// to nothing. A `url(#id)` paint resolves to the named gradient.
fn resolve_fill(v: &str, defs: &Defs) -> Option<Fill> {
	let s = v.trim();
	if s.is_empty() || s == "none" || s == "context-fill" || s == "context-stroke" {
		return None;
	}
	if let Some(rest) = s.strip_prefix("url(") {
		// The id inside `url(#id)`, taking everything up to the closing parenthesis and dropping the hash.
		let id = rest.split(')').next().unwrap_or("").trim().trim_start_matches('#');
		return defs.gradients.get(id).map(|g| Fill::Grad(g.clone()));
	}
	paint_colour(s, Some(defs)).map(Fill::Flat)
}

/// Places a `<use>`'s referenced outline: its target fetched by id, offset by the element's `x`/`y`, and
/// filled with the resolved fill. A reference to no known outline draws nothing rather than failing the
/// read.
fn emit_use(
	elem:	&Elem,
	ctx:	&Transform,
	paint:	&Paint,
	defs:	&Defs,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	let href = match elem.attr("xlink:href").or_else(|| elem.attr("href")) {
		Some(h)	=> h.trim_start_matches('#'),
		None	=> return Ok(()),
	};
	let outline = match defs.outlines.get(href) {
		Some(p)	=> p.clone(),
		None	=> return Ok(()),
	};
	let x		= elem.attr("x").map(number).unwrap_or(0.0);
	let y		= elem.attr("y").map(number).unwrap_or(0.0);
	let local	= child_transform(elem, ctx);
	let local	= Transform::translate(x, y).then(&local);
	emit_shape(outline, &local, paint, ops)
}

/// The font state inherited down a `<text>` and reset by each `<tspan>`: the size, the anchor, the face
/// hints, and the colour the run is painted. The size begins at zero -- a run that never gets one cannot
/// be shaped and is dropped -- and the colour at the inherited fill.
#[derive(Clone)]
struct TextState {
	size:	f32,		// font-size, in the element's own units
	anchor:	Anchor,
	italic:	bool,
	bold:	bool,
	colour:	Rgba,
}

impl TextState {
	/// The starting state a `<text>` inherits, all cascaded down the group tree to this point: the font
	/// size, the anchor, the face hints, and the fill colour.
	fn from_paint(paint: &Paint) -> Self {
		Self {
			size:	paint.font_size,
			anchor:	paint.text_anchor,
			italic:	paint.italic,
			bold:	paint.bold,
			colour:	paint.fill.as_ref().map(Fill::colour).unwrap_or(Rgba::BLACK),
		}
	}
}

/// Resolves a `<text>` or `<tspan>`'s font state from the inherited one: its `style` properties (which
/// win) and its presentation attributes (which fall back), each overriding only what it names. A
/// `fill:none` on labelled outline text falls back to the stroke colour, so the glyphs still show.
fn text_state(base: &TextState, elem: &Elem, defs: &Defs) -> TextState {
	let mut s	= base.clone();
	let style	= elem.attr("style").unwrap_or("");

	if let Some(v) = prop(elem, style, "font-size") {
		let n = length_num(&v);
		if n > 0.0 {
			s.size = n;
		}
	}
	if let Some(v) = prop(elem, style, "font-style") {
		let t = v.trim();
		s.italic = t == "italic" || t == "oblique";
	}
	if let Some(v) = prop(elem, style, "font-weight") {
		let t = v.trim();
		s.bold = t == "bold" || t == "bolder"
			|| t.parse::<f32>().map(|w| w >= 600.0).unwrap_or(false);
	}
	// `text-anchor` is the SVG property; `text-align` is the shorthand Inkscape writes on a `<tspan>`.
	if let Some(v) = prop(elem, style, "text-anchor").or_else(|| prop(elem, style, "text-align")) {
		s.anchor = parse_anchor(&v);
	}
	// Fill wins the colour; a declared `none` falls back to the stroke so outline text still paints; a
	// stroke alone, with no fill declared, likewise sets the colour.
	if let Some(v) = prop(elem, style, "fill") {
		match resolve_fill(&v, defs) {
			Some(c)	=> s.colour = c.colour(),
			None	=> {
				if let Some(sc) = prop(elem, style, "stroke").and_then(|w| resolve_fill(&w, defs)) {
					s.colour = sc.colour();
				}
			},
		}
	} else if let Some(v) = prop(elem, style, "stroke") {
		if let Some(c) = resolve_fill(&v, defs) {
			s.colour = c.colour();
		}
	}
	s
}

/// Reads a live `<text>` into text runs: its direct text at its own anchor, and each `<tspan>` at the
/// tspan's anchor (falling back to the text's) with the tspan's own font state. The reader shapes none of
/// it -- it carries no font -- so each run is an [`SvgOp::Text`] the caller with a font set bakes.
fn emit_text(
	elem:	&Elem,
	ctx:	&Transform,
	paint:	&Paint,
	defs:	&Defs,
	xml:	&Xml,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	let local	= child_transform(elem, ctx);
	let base	= text_state(&TextState::from_paint(paint), elem, defs);
	let tx		= first_number(elem.attr("x"));
	let ty		= first_number(elem.attr("y"));

	for kid in &elem.kids {
		match kid {
			Node::Elem(e) if e.name.local() == "tspan" => {
				let st	= text_state(&base, e, defs);
				let sx	= e.attr("x").map(|v| first_number(Some(v))).unwrap_or(tx);
				let sy	= e.attr("y").map(|v| first_number(Some(v))).unwrap_or(ty);
				push_text_run(ops, &xml.text_of(e), &local, sx, sy, &st);
			},
			Node::Text(span) => {
				push_text_run(ops, &xml.text(span), &local, tx, ty, &base);
			},
			_ => {},
		}
	}
	Ok(())
}

/// Queues one text run, dropping a run with no size to shape at or no visible characters. A tab or a
/// newline `xml:space="preserve"` leaves in the content -- a wrapped label keeps a line break inside its
/// run -- becomes a space, since the run sets on one line and the shaper would otherwise draw the control
/// character as a missing-glyph box.
fn push_text_run(
	ops:	&mut Vec<SvgOp>,
	text:	&str,
	local:	&Transform,
	x:		f32,
	y:		f32,
	st:		&TextState,
) {
	let cleaned: String = text
		.chars()
		.map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
		.collect();
	let cleaned = cleaned.trim();
	if st.size <= 0.0 || cleaned.is_empty() {
		return;
	}
	ops.push(SvgOp::Text {
		text:	cleaned.to_string(),
		local:	*local,
		x,
		y,
		size:	st.size,
		anchor:	st.anchor,
		italic:	st.italic,
		bold:	st.bold,
		colour:	st.colour,
	});
}

/// Decodes an embedded `<image>` -- a `data:...;base64,` PNG or JPEG -- and places its rectangle in the
/// picture frame. A file reference, an unreadable payload or an unknown raster draws nothing rather than
/// failing the whole read. The placement is mapped through the element's frame by its corners, so a
/// translate or a scale is exact; a rotation or a shear is approximated by the axis-aligned bounds.
fn emit_image(elem: &Elem, ctx: &Transform, ops: &mut Vec<SvgOp>) -> Outcome<()> {
	let href = match elem.attr("xlink:href").or_else(|| elem.attr("href")) {
		Some(h)	=> h,
		None	=> return Ok(()),
	};
	let payload = match href.find("base64,") {
		Some(i)	=> &href[i + "base64,".len()..],
		None	=> return Ok(()),	// a file reference carries no bytes to decode here
	};
	// Inkscape wraps the payload across lines; the decoder refuses whitespace, so strip it first.
	let clean: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
	let bytes = match base64::decode(&clean) {
		Ok(b)	=> b,
		Err(_)	=> return Ok(()),
	};
	let (pm, tone) = match crate::sniff::sniff(&bytes) {
		Some(crate::sniff::ImageKind::Png)	=> res!(crate::pixmap::Pixmap::from_png_toned(&bytes)),
		Some(crate::sniff::ImageKind::Jpeg)	=> res!(crate::pixmap::Pixmap::from_jpeg_toned(&bytes)),
		_					=> return Ok(()),	// neither PNG nor JPEG by its bytes
	};
	let (iw, ih) = (pm.width(), pm.height());
	let rgba = pm.into_data();

	let x = first_number(elem.attr("x"));
	let y = first_number(elem.attr("y"));
	let w = first_number(elem.attr("width"));
	let h = first_number(elem.attr("height"));
	if w <= 0.0 || h <= 0.0 {
		return Ok(());
	}
	let local	= child_transform(elem, ctx);
	let p0		= local.apply(Pt::new(x, y));
	let p1		= local.apply(Pt::new(x + w, y + h));
	ops.push(SvgOp::Image {
		rgba,
		tone,
		iw,
		ih,
		x:	p0.x.min(p1.x),
		y:	p0.y.min(p1.y),
		w:	(p1.x - p0.x).abs(),
		h:	(p1.y - p0.y).abs(),
	});
	Ok(())
}

/// Emits a shape's fill and stroke ops under a resolved presentation state. A path with a fill paints it
/// under the stroke, the order SVG draws them; either may be absent. An even-odd fill has its geometry
/// reordered to fill the same way under the non-zero rule the engine draws with. Opacity is folded into
/// each emitted colour's alpha.
fn emit_shape(
	shape:	Path,
	local:	&Transform,
	paint:	&Paint,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	let placed	= res!(shape.transform(local));
	let bbox	= shape.bounds(&Transform::IDENTITY);	// the box an objectBoundingBox gradient spans

	match &paint.fill {
		Some(Fill::Flat(colour)) => {
			let colour = fade(*colour, paint.fill_opacity * paint.opacity);
			if colour.a > 0 {
				let path = if paint.even_odd { res!(placed.even_odd_as_non_zero()) } else { placed.clone() };
				ops.push(SvgOp::Fill { path, colour });
			}
		},
		Some(Fill::Grad(g)) => {
			if let Some(sh) = g.shading(bbox, local) {
				let shading	= sh.faded(paint.fill_opacity * paint.opacity);
				let path	= if paint.even_odd { res!(placed.even_odd_as_non_zero()) } else { placed.clone() };
				ops.push(SvgOp::Shade { path, shading });
			}
		},
		None => {},
	}
	match &paint.stroke {
		Some(Fill::Flat(colour)) => {
			let colour = fade(*colour, paint.stroke_opacity * paint.opacity);
			if colour.a > 0 {
				let stroke = res!(pen(paint, local));
				ops.push(SvgOp::Stroke { path: placed, colour, stroke });
			}
		},
		Some(Fill::Grad(g)) => {
			// A shaded stroke is its outline filled: the pen's area as a path, under the same shading.
			if let Some(sh) = g.shading(bbox, local) {
				let shading	= sh.faded(paint.stroke_opacity * paint.opacity);
				let stroke	= res!(pen(paint, local));
				let path	= res!(placed.stroke(&stroke));
				ops.push(SvgOp::Shade { path, shading });
			}
		},
		None => {},
	}
	Ok(())
}

/// Builds the pen a shape's stroke wants, its width and dash scaled from the element's own frame into the
/// picture frame by the placement transform's scale, so a stroke inside a shrunk group keeps its true
/// thickness rather than the raw attribute's.
fn pen(paint: &Paint, local: &Transform) -> Outcome<Stroke> {
	let s			= local.scale_factor().max(f32::MIN_POSITIVE);
	let width		= (paint.width * s).max(f32::MIN_POSITIVE);
	let mut stroke	= res!(Stroke::new(width));
	stroke = stroke
		.with_cap(paint.cap)
		.with_join(paint.join)
		.with_miter_limit(paint.miter.max(1.0));
	if let Some(pattern) = &paint.dash {
		let scaled: Vec<f32> = pattern.iter().map(|&x| x * s).collect();
		if scaled.iter().any(|&x| x > 0.0) {
			stroke = stroke.with_dash(Dash::new(scaled).with_offset(paint.dash_offset * s));
		}
	}
	Ok(stroke)
}

/// Scales a colour's alpha by an opacity factor, for the fill-opacity, stroke-opacity and group opacity
/// the flat op set folds into the one alpha it carries.
fn fade(c: Rgba, factor: f32) -> Rgba {
	let a = ((c.a as f32) * factor.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8;
	Rgba::new(c.r, c.g, c.b, a)
}

/// The path a shape element describes, in its own coordinates, or `None` when the element is not a shape
/// this reader draws. The primitives are turned to the same paths the drawing crate builds them from.
fn shape_path(elem: &Elem) -> Option<Path> {
	match elem.name.local() {
		"path" => {
			let d = elem.attr("d")?;
			crate::svg::path_data(d).ok()
		},
		"rect" => {
			let x	= elem.attr("x").map(number).unwrap_or(0.0);
			let y	= elem.attr("y").map(number).unwrap_or(0.0);
			let w	= elem.attr("width").map(number).unwrap_or(0.0);
			let h	= elem.attr("height").map(number).unwrap_or(0.0);
			if w <= 0.0 || h <= 0.0 {
				return None;
			}
			let b = Bounds::new(x, y, x + w, y + h);
			// A rounded rect takes the radius given; either radius alone sets both, as SVG does.
			let rx = elem.attr("rx").map(number);
			let ry = elem.attr("ry").map(number);
			let r = match (rx, ry) {
				(Some(a), Some(b))	=> a.max(b),
				(Some(a), None)		=> a,
				(None, Some(b))		=> b,
				(None, None)		=> 0.0,
			};
			if r > 0.0 {
				Path::round_rect(b, r).ok()
			} else {
				Path::rect(b).ok()
			}
		},
		"circle" => {
			let cx	= elem.attr("cx").map(number).unwrap_or(0.0);
			let cy	= elem.attr("cy").map(number).unwrap_or(0.0);
			let r	= elem.attr("r").map(number).unwrap_or(0.0);
			if r <= 0.0 {
				return None;
			}
			Path::circle(cx, cy, r).ok()
		},
		"ellipse" => {
			let cx	= elem.attr("cx").map(number).unwrap_or(0.0);
			let cy	= elem.attr("cy").map(number).unwrap_or(0.0);
			let rx	= elem.attr("rx").map(number).unwrap_or(0.0);
			let ry	= elem.attr("ry").map(number).unwrap_or(0.0);
			if rx <= 0.0 || ry <= 0.0 {
				return None;
			}
			Path::ellipse(cx, cy, rx, ry).ok()
		},
		"line" => {
			let x1	= elem.attr("x1").map(number).unwrap_or(0.0);
			let y1	= elem.attr("y1").map(number).unwrap_or(0.0);
			let x2	= elem.attr("x2").map(number).unwrap_or(0.0);
			let y2	= elem.attr("y2").map(number).unwrap_or(0.0);
			let mut pb = PathBuilder::new();
			pb.move_to(Pt::new(x1, y1));
			pb.line_to(Pt::new(x2, y2));
			pb.finish().ok()
		},
		"polyline" | "polygon" => {
			let pts = numbers(elem.attr("points")?);
			if pts.len() < 4 {
				return None;
			}
			let mut pb = PathBuilder::new();
			pb.move_to(Pt::new(pts[0], pts[1]));
			let mut i = 2;
			while i + 1 < pts.len() {
				pb.line_to(Pt::new(pts[i], pts[i + 1]));
				i += 2;
			}
			// A polygon closes back onto its first point; a polyline is left open.
			if elem.name.local() == "polygon" {
				pb.close();
			}
			pb.finish().ok()
		},
		_ => None,
	}
}

/// The transform an element's own `transform` attribute composes onto the inherited frame.
fn child_transform(elem: &Elem, ctx: &Transform) -> Transform {
	match elem.attr("transform") {
		Some(t)	=> parse_transform(t).then(ctx),
		None	=> *ctx,
	}
}

/// Parses an SVG `transform` list -- `translate`, `matrix`, `scale`, `rotate`, `skewX`, `skewY` -- into one
/// affine map.
///
/// The list reads left to right and the leftmost function is the outermost, so a point is carried by the
/// rightmost first. Folding each function on the left of the running result builds exactly that order.
fn parse_transform(s: &str) -> Transform {
	let mut acc = Transform::IDENTITY;
	let bytes = s.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		// The function name, up to its opening parenthesis.
		let name_start = i;
		while i < bytes.len() && bytes[i] != b'(' {
			i += 1;
		}
		if i >= bytes.len() {
			break;
		}
		let name = s[name_start..i].trim();
		i += 1; // past '('
		let args_start = i;
		while i < bytes.len() && bytes[i] != b')' {
			i += 1;
		}
		let args = numbers(&s[args_start..i.min(bytes.len())]);
		if i < bytes.len() {
			i += 1; // past ')'
		}
		let f = function(name, &args);
		acc = f.then(&acc);
	}
	acc
}

/// One transform function as a matrix; an unrecognised or malformed one is the identity, so it is a
/// no-op rather than a fault.
fn function(name: &str, a: &[f32]) -> Transform {
	match name {
		"translate" => match a.len() {
			1	=> Transform::translate(a[0], 0.0),
			n if n >= 2	=> Transform::translate(a[0], a[1]),
			_	=> Transform::IDENTITY,
		},
		"scale" => match a.len() {
			1	=> Transform::scale(a[0], a[0]),
			n if n >= 2	=> Transform::scale(a[0], a[1]),
			_	=> Transform::IDENTITY,
		},
		"rotate" => match a.len() {
			1 => Transform::rotate(a[0].to_radians()),
			// A three-argument rotate turns about a centre: translate to it, rotate, translate back.
			n if n >= 3 => Transform::translate(-a[1], -a[2])
				.then(&Transform::rotate(a[0].to_radians()))
				.then(&Transform::translate(a[1], a[2])),
			_ => Transform::IDENTITY,
		},
		"matrix" if a.len() >= 6 => Transform {
			a: a[0], b: a[1], c: a[2], d: a[3], e: a[4], f: a[5],
		},
		// A shear by an angle in degrees: skewX slides x by tan(a) y, skewY slides y by tan(a) x.
		"skewX" if !a.is_empty() => Transform { c: a[0].to_radians().tan(), ..Transform::IDENTITY },
		"skewY" if !a.is_empty() => Transform { b: a[0].to_radians().tan(), ..Transform::IDENTITY },
		_ => Transform::IDENTITY,
	}
}

/// One `fill`/`stroke`/`stop-color` colour: a `#rgb`/`#rrggbb`/`#rrggbbaa`, an `rgb(...)`, or a named
/// colour. `none` and the unresolved are the caller's concern; this returns `None` for anything it cannot
/// read as a colour. A `defs` is taken only so a caller may share this for stop colours, which never
/// reference a gradient of their own.
fn paint_colour(s: &str, _defs: Option<&Defs>) -> Option<Rgba> {
	let s = s.trim();
	if s.is_empty() || s == "none" {
		return None;
	}
	if let Some(hex) = s.strip_prefix('#') {
		return Rgba::from_hex(hex).ok();
	}
	if let Some(rest) = s.strip_prefix("rgb") {
		let inner = rest.trim_start_matches('a').trim_start_matches('(').trim_end_matches(')');
		let n = numbers(inner);
		if n.len() >= 3 {
			let a = if n.len() >= 4 {
				// The fourth is a 0..1 alpha in rgba(); scaled to a byte.
				(n[3].clamp(0.0, 1.0) * 255.0).round() as u8
			} else {
				255
			};
			return Some(Rgba::new(
				n[0].clamp(0.0, 255.0) as u8,
				n[1].clamp(0.0, 255.0) as u8,
				n[2].clamp(0.0, 255.0) as u8,
				a,
			));
		}
	}
	named_colour(s)
}

/// The handful of named colours a plot or an illustration might carry, beyond the hex the files otherwise
/// use.
fn named_colour(name: &str) -> Option<Rgba> {
	match name {
		"black"			=> Some(Rgba::BLACK),
		"white"			=> Some(Rgba::WHITE),
		"red"			=> Some(Rgba::opaque(255, 0, 0)),
		"green"			=> Some(Rgba::opaque(0, 128, 0)),
		"lime"			=> Some(Rgba::opaque(0, 255, 0)),
		"blue"			=> Some(Rgba::opaque(0, 0, 255)),
		"navy"			=> Some(Rgba::opaque(0, 0, 128)),
		"cyan" | "aqua"	=> Some(Rgba::opaque(0, 255, 255)),
		"magenta" | "fuchsia"	=> Some(Rgba::opaque(255, 0, 255)),
		"grey" | "gray"	=> Some(Rgba::opaque(128, 128, 128)),
		"silver"		=> Some(Rgba::opaque(192, 192, 192)),
		"maroon"		=> Some(Rgba::opaque(128, 0, 0)),
		"yellow"		=> Some(Rgba::opaque(255, 255, 0)),
		"orange"		=> Some(Rgba::opaque(255, 165, 0)),
		"purple"		=> Some(Rgba::opaque(128, 0, 128)),
		"teal"			=> Some(Rgba::opaque(0, 128, 128)),
		"olive"			=> Some(Rgba::opaque(128, 128, 0)),
		"transparent" | "none"	=> Some(Rgba::TRANSPARENT),
		_				=> None,
	}
}

/// A length attribute in points, dropping a `pt` unit suffix; other units are read as their number.
fn length_pt(s: &str) -> f32 {
	let t = s.trim().trim_end_matches("pt");
	number(t)
}

/// A length as its bare number, dropping any trailing unit letters or a percent sign -- a `font-size`
/// arrives as `3.5278px`, whose unit `length_pt` would not shed. The value keeps the element's own units.
fn length_num(s: &str) -> f32 {
	numbers(s).first().copied().unwrap_or(0.0)
}

/// The first number of an attribute -- an `x`/`y` may carry a whitespace-separated list -- or zero for
/// an absent or unparseable one.
fn first_number(v: Option<&str>) -> f32 {
	match v {
		Some(s)	=> numbers(s).first().copied().unwrap_or(0.0),
		None	=> 0.0,
	}
}

/// One number, or zero when the text does not parse.
fn number(s: &str) -> f32 {
	s.trim().parse::<f32>().unwrap_or(0.0)
}

/// Every number in a run of numbers separated by whitespace, commas or a leading minus, in order.
fn numbers(s: &str) -> Vec<f32> {
	let mut out: Vec<f32> = Vec::new();
	let bytes = s.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		let c = bytes[i];
		if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() {
			let start = i;
			// A sign only opens a number; a following sign closes the previous one.
			if c == b'-' || c == b'+' {
				i += 1;
			}
			let mut seen_dot = false;
			let mut seen_exp = false;
			while i < bytes.len() {
				let d = bytes[i];
				if d.is_ascii_digit() {
					i += 1;
				} else if d == b'.' && !seen_dot && !seen_exp {
					seen_dot = true;
					i += 1;
				} else if (d == b'e' || d == b'E') && !seen_exp {
					seen_exp = true;
					i += 1;
					if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
						i += 1;
					}
				} else {
					break;
				}
			}
			if let Ok(n) = s[start..i].parse::<f32>() {
				out.push(n);
			}
		} else {
			i += 1;
		}
	}
	out
}
