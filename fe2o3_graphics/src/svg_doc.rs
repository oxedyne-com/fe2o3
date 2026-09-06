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
//! What is still left at the door: `<text>`/`<tspan>` (an Inkscape file keeps live text, needing a font
//! to shape, which this fontless reader has none of); an embedded `<image>` (a base64 raster, which the
//! flat fill/stroke op set cannot carry); `<clipPath>` and markers (arrowheads); and `filter`/`pattern`.
//! A gradient fill (`url(#id)`) resolves to the flat mean of its stops -- a true axial or radial shading
//! would need a paint the op set does not model -- and a reference to nothing draws nothing rather than
//! failing the read. A file reaching past all this is read as far as it fits and the rest is left.
//!
//! The one thing worth stating is what happens to a typesetter's text. It does not leave `<text>` in its
//! SVG; it bakes each glyph to an outline, files the outline once as a `<symbol>`, and places it with a
//! `<use>` whose enclosing group carries the position and the y-flip that turns a font's y-up outline the
//! right way up. So no font is needed to read that text back: the outlines are already in the file, and a
//! `<use>` is just a filled path fetched by id. That is why this module takes no font set.
//!
//! The geometry comes out in the `viewBox`'s own units, y down, which for a Typst file are points. A
//! caller sizing the picture to a figure width scales every path by one factor; nothing here bakes that
//! in, so the picture is read once and drawn at whatever size is wanted.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::colour::Rgba;
use crate::path::{
	Bounds,
	Path,
	PathBuilder,
	Pt,
	Seg,
};
use crate::stroke::{
	Cap,
	Dash,
	Join,
	Stroke,
};
use crate::transform::Transform;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};

use std::collections::HashMap;

/// One drawing operation read from an SVG document: a filled or a stroked path, in the document's own
/// frame (y down, `viewBox` units). The geometry is flattened of its element tree -- every transform is
/// baked into the path -- so a caller need only scale and place it.
#[derive(Clone, Debug)]
pub enum SvgOp {
	Fill { path: Path, colour: Rgba },
	Stroke { path: Path, colour: Rgba, stroke: Stroke },
}

/// A read SVG document: its drawing operations and the size of its `viewBox`, in the `viewBox`'s units.
#[derive(Clone, Debug)]
pub struct SvgPicture {
	pub ops:	Vec<SvgOp>,
	pub width:	f32,	// viewBox width, in its own units (points, for a typesetter's output)
	pub height:	f32,	// viewBox height
}

/// The store `<defs>` fills for the draw walk to draw from: outlines a `<use>` fetches by id, and the
/// flat mean colour a gradient `url(#id)` fill resolves to.
struct Defs {
	outlines:	HashMap<String, Path>,	// id -> concatenated outline, for <use>
	gradients:	HashMap<String, Rgba>,	// id -> flat mean of the gradient's stops
}

/// The presentation state inherited down the group tree: fill and stroke and their opacities, the pen's
/// width and joins, the dash pattern, the fill rule, and the cumulative group opacity. Every field
/// carries down to a child, which overrides only the ones its own attributes or `style` set. The initial
/// state is SVG's own defaults: a black fill, no stroke, full opacity, non-zero winding.
#[derive(Clone)]
struct Paint {
	fill:			Option<Rgba>,	// resolved fill colour, or None for `fill:none`
	fill_opacity:	f32,
	even_odd:		bool,			// fill-rule="evenodd"
	stroke:			Option<Rgba>,	// resolved stroke colour, or None for no stroke
	stroke_opacity:	f32,
	width:			f32,			// stroke width, in the element's own frame
	cap:			Cap,
	join:			Join,
	miter:			f32,
	dash:			Option<Vec<f32>>,
	dash_offset:	f32,
	opacity:		f32,			// cumulative group opacity, folded into every emitted alpha
}

impl Default for Paint {
	fn default() -> Self {
		Self {
			fill:			Some(Rgba::BLACK),
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
	collect_defs(root, &mut defs);

	// Everything is expressed relative to the viewBox origin, so the walk begins with a translation that
	// carries that origin to (0, 0); a Typst file's origin is already there and the translation is nil.
	let base = Transform::translate(-vx, -vy);
	let mut ops: Vec<SvgOp> = Vec::new();
	res!(walk(root, &base, &Paint::default(), &defs, &mut ops));

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
/// `<use>` to fetch), and every gradient's flat mean colour (for a `url(#id)` fill to resolve to). The
/// search descends the whole tree, since Inkscape files an id-bearing shape wherever it likes, not only
/// under `<defs>`.
fn collect_defs(elem: &Elem, out: &mut Defs) {
	for child in elem.elems() {
		let name = child.name.local();
		match name {
			"linearGradient" | "radialGradient" => {
				if let Some(id) = child.attr("id") {
					if let Some(c) = gradient_mean(child) {
						out.gradients.insert(id.to_string(), c);
					}
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
		collect_defs(child, out);
	}
}

/// The flat mean colour of a gradient's stops: each stop's `stop-color` weighted equally, its
/// `stop-opacity` folded into the alpha. A gradient with no stops of its own yields nothing, and the
/// caller leaves such a fill unpainted. This is the flat approximation a true axial or radial shading is
/// reduced to, since the flat op set carries no paint that varies across a shape.
fn gradient_mean(grad: &Elem) -> Option<Rgba> {
	let mut r = 0.0f32;
	let mut g = 0.0f32;
	let mut b = 0.0f32;
	let mut a = 0.0f32;
	let mut n = 0.0f32;
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
		r += col.r as f32;
		g += col.g as f32;
		b += col.b as f32;
		a += (col.a as f32) * op;
		n += 1.0;
	}
	if n < 1.0 {
		return None;
	}
	Some(Rgba::new(
		(r / n).round() as u8,
		(g / n).round() as u8,
		(b / n).round() as u8,
		(a / n).round().clamp(0.0, 255.0) as u8,
	))
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
				res!(walk(child, &local, &sub, defs, ops));
			},
			"use" => {
				let sub = resolve_paint(paint, child, defs);
				res!(emit_use(child, ctx, &sub, defs, ops));
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
					res!(walk(child, &local, &sub, defs, ops));
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
	p
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

/// Resolves a `fill`/`stroke` value to a colour, or `None` for `none`, an unknown paint, or a reference
/// to nothing. A `url(#id)` fill resolves to the flat mean of the named gradient's stops.
fn resolve_fill(v: &str, defs: &Defs) -> Option<Rgba> {
	let s = v.trim();
	if s.is_empty() || s == "none" || s == "context-fill" || s == "context-stroke" {
		return None;
	}
	if let Some(rest) = s.strip_prefix("url(") {
		// The id inside `url(#id)`, taking everything up to the closing parenthesis and dropping the hash.
		let id = rest.split(')').next().unwrap_or("").trim().trim_start_matches('#');
		return defs.gradients.get(id).copied();
	}
	paint_colour(s, Some(defs))
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
	let placed = res!(shape.transform(local));

	if let Some(colour) = paint.fill {
		let colour = fade(colour, paint.fill_opacity * paint.opacity);
		if colour.a > 0 {
			let path = if paint.even_odd {
				res!(placed.even_odd_as_non_zero())
			} else {
				placed.clone()
			};
			ops.push(SvgOp::Fill { path, colour });
		}
	}
	if let Some(colour) = paint.stroke {
		let colour = fade(colour, paint.stroke_opacity * paint.opacity);
		if colour.a > 0 {
			let stroke = res!(pen(paint, local));
			ops.push(SvgOp::Stroke { path: placed, colour, stroke });
		}
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

/// Parses an SVG `transform` list -- `translate`, `matrix`, `scale`, `rotate` -- into one affine map.
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
