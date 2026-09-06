//! An SVG document, read into flat drawing operations.
//!
//! Where [`crate::svg`] reads only the `d` of one `<path>`, this reads the element tree a whole file
//! carries: the `viewBox`, the nested `<g transform>` frames, the `<path>` fills and strokes, and the
//! `<use>` references that place a glyph outline held in `<defs>`. It is deliberately narrow. The files
//! it exists for are the SVG a typesetter emits -- Typst's cetz plots are the case in hand -- and that
//! output is a small, regular subset: paths, groups, symbol-defined glyphs referenced by `<use>`, and
//! `translate`/`matrix` transforms, with colour as `#rrggbb` or `#rrggbbaa` and no gradient, clip,
//! pattern or filter. A file reaching past that subset is read as far as it fits and the rest is left.
//!
//! The one thing worth stating is what happens to text. A typesetter does not leave `<text>` in its
//! SVG; it bakes each glyph to an outline, files the outline once as a `<symbol>`, and places it with a
//! `<use>` whose enclosing group carries the position and the y-flip that turns a font's y-up outline
//! the right way up. So no font is needed to read the text back: the outlines are already in the file,
//! and a `<use>` is just a filled path fetched by id. That is why this module takes no font set.
//!
//! The geometry comes out in the `viewBox`'s own units, y down, which for a Typst file are points. A
//! caller sizing the picture to a figure width scales every path by one factor; nothing here bakes that
//! in, so the picture is read once and drawn at whatever size is wanted.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::colour::Rgba;
use crate::path::{
	Path,
	PathBuilder,
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

/// Reads an SVG document into a flat [`SvgPicture`].
///
/// The tree is walked once in document order, a transform accumulated down each branch, and every
/// `<path>` and `<use>` turned into an [`SvgOp`] with its transform already applied. `<defs>` is read
/// first, for the glyph outlines a `<use>` will fetch, then skipped in the draw walk.
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

	// The glyph outlines a <use> fetches, keyed by their symbol id.
	let mut defs: HashMap<String, Path> = HashMap::new();
	collect_symbols(root, &mut defs);

	// Everything is expressed relative to the viewBox origin, so the walk begins with a translation that
	// carries that origin to (0, 0); a Typst file's origin is already there and the translation is nil.
	let base = Transform::translate(-vx, -vy);
	let mut ops: Vec<SvgOp> = Vec::new();
	res!(walk(root, &base, &defs, &mut ops));

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

/// Walks `<defs>` and gathers every `<symbol>`'s outline, keyed by its `id`.
///
/// A symbol may hold more than one `<path>`; they are concatenated into the one outline, since a `<use>`
/// draws the symbol whole. Symbols nest nowhere in a typesetter's output, so the search is a plain
/// descent with no transform to carry: a symbol's outline is in its own frame and the `<use>` places it.
fn collect_symbols(elem: &Elem, out: &mut HashMap<String, Path>) {
	for child in elem.elems() {
		match child.name.local() {
			"symbol" => {
				if let Some(id) = child.attr("id") {
					let mut pb = PathBuilder::new();
					gather_paths(child, &mut pb);
					if let Ok(path) = pb.finish() {
						if !path.is_empty() {
							out.insert(id.to_string(), path);
						}
					}
				}
			},
			_ => collect_symbols(child, out),
		}
	}
}

/// Appends every `<path>` outline held under an element into one builder, descending through any groups.
fn gather_paths(elem: &Elem, pb: &mut PathBuilder) {
	for child in elem.elems() {
		match child.name.local() {
			"path" => {
				if let Some(d) = child.attr("d") {
					if let Ok(path) = crate::svg::path_data(d) {
						append_path(pb, &path);
					}
				}
			},
			_ => gather_paths(child, pb),
		}
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

/// Walks the draw tree, emitting an [`SvgOp`] for every painted `<path>` and `<use>`.
///
/// `ctx` maps this element's local frame to the picture frame; a `<g transform>` composes onto it for
/// its children. `<defs>` and `<symbol>` are the glyph store, gathered already, so they draw nothing here.
fn walk(
	elem:	&Elem,
	ctx:	&Transform,
	defs:	&HashMap<String, Path>,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	for child in elem.elems() {
		match child.name.local() {
			"defs" | "symbol"	=> {},	// the glyph store, not drawn in place
			"g" => {
				let local = child_transform(child, ctx);
				res!(walk(child, &local, defs, ops));
			},
			"path" => {
				if let Some(d) = child.attr("d") {
					let local	= child_transform(child, ctx);
					let path	= res!(crate::svg::path_data(d));
					let placed	= res!(path.transform(&local));
					res!(emit_paint(child, placed, ops));
				}
			},
			"use" => res!(emit_use(child, ctx, defs, ops)),
			_ => {
				// An unmodelled container may still hold drawable children; a leaf is simply skipped.
				let local = child_transform(child, ctx);
				res!(walk(child, &local, defs, ops));
			},
		}
	}
	Ok(())
}

/// Places a `<use>`'s referenced outline: its symbol fetched by id, offset by the element's `x`/`y`, and
/// filled. A `<use>` in a typesetter's output is always a glyph, so it is filled, never stroked; a
/// reference to no known symbol draws nothing rather than failing the read.
fn emit_use(
	elem:	&Elem,
	ctx:	&Transform,
	defs:	&HashMap<String, Path>,
	ops:	&mut Vec<SvgOp>,
)
	-> Outcome<()>
{
	let href = match elem.attr("xlink:href").or_else(|| elem.attr("href")) {
		Some(h)	=> h.trim_start_matches('#'),
		None	=> return Ok(()),
	};
	let outline = match defs.get(href) {
		Some(p)	=> p,
		None	=> return Ok(()),
	};
	let x		= elem.attr("x").map(number).unwrap_or(0.0);
	let y		= elem.attr("y").map(number).unwrap_or(0.0);
	let local	= Transform::translate(x, y).then(ctx);
	let placed	= res!(outline.transform(&local));
	let colour	= paint(elem.attr("fill")).unwrap_or(Some(Rgba::BLACK));
	if let Some(colour) = colour {
		ops.push(SvgOp::Fill { path: placed, colour });
	}
	Ok(())
}

/// Turns a painted `<path>`'s `fill` and `stroke` presentation into fill and stroke ops. A path with
/// neither paints nothing; a path with both paints the fill under the stroke, the order SVG draws them.
fn emit_paint(elem: &Elem, path: Path, ops: &mut Vec<SvgOp>) -> Outcome<()> {
	// The default fill is black, so a path with no fill attribute is filled; `fill="none"` turns it off.
	let fill = match elem.attr("fill") {
		Some(v)	=> res!(paint_value(v)),
		None	=> Some(Rgba::BLACK),
	};
	if let Some(colour) = fill {
		ops.push(SvgOp::Fill { path: path.clone(), colour });
	}
	if let Some(colour) = res!(stroke_paint(elem)) {
		let stroke = res!(stroke_of(elem));
		ops.push(SvgOp::Stroke { path, colour, stroke });
	}
	Ok(())
}

/// The stroke colour, or `None` when the path is unstroked (`stroke` absent or `none`).
fn stroke_paint(elem: &Elem) -> Outcome<Option<Rgba>> {
	match elem.attr("stroke") {
		Some(v)	=> paint_value(v),
		None	=> Ok(None),
	}
}

/// Builds the pen a `<path>`'s stroke presentation describes: width, caps, joins, miter limit and dashes.
fn stroke_of(elem: &Elem) -> Outcome<Stroke> {
	let width	= elem.attr("stroke-width").map(number).unwrap_or(1.0).max(f32::MIN_POSITIVE);
	let mut pen	= res!(Stroke::new(width));
	if let Some(cap) = elem.attr("stroke-linecap") {
		pen = pen.with_cap(match cap {
			"round"		=> Cap::Round,
			"square"	=> Cap::Square,
			_			=> Cap::Butt,
		});
	}
	if let Some(join) = elem.attr("stroke-linejoin") {
		pen = pen.with_join(match join {
			"round"	=> Join::Round,
			"bevel"	=> Join::Bevel,
			_		=> Join::Miter,
		});
	}
	if let Some(ml) = elem.attr("stroke-miterlimit") {
		pen = pen.with_miter_limit(number(ml).max(1.0));
	}
	// A dash array of nothing, or of all zeros, is a solid line and carries no pattern.
	if let Some(da) = elem.attr("stroke-dasharray") {
		let pattern = numbers(da);
		if !pattern.is_empty() && pattern.iter().any(|&x| x > 0.0) {
			let offset	= elem.attr("stroke-dashoffset").map(number).unwrap_or(0.0);
			pen			= pen.with_dash(Dash::new(pattern).with_offset(offset));
		}
	}
	Ok(pen)
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

/// A paint value that may be absent: `None` for an absent attribute, `Some(None)` for `none`,
/// `Some(Some(colour))` for a colour.
fn paint(v: Option<&str>) -> Option<Option<Rgba>> {
	v.map(|s| paint_value(s).unwrap_or(None))
}

/// One `fill`/`stroke` value: `none` and unrecognised paints turn it off, a `#rrggbb`/`#rrggbbaa` or a
/// named colour turn it on. Gradients and patterns (`url(...)`) are outside the subset and turn it off.
fn paint_value(v: &str) -> Outcome<Option<Rgba>> {
	let s = v.trim();
	if s.is_empty() || s == "none" || s.starts_with("url(") {
		return Ok(None);
	}
	if let Some(hex) = s.strip_prefix('#') {
		return Ok(Some(res!(Rgba::from_hex(hex))));
	}
	Ok(named_colour(s))
}

/// The handful of named colours a plot might carry, beyond the hex the subset otherwise uses.
fn named_colour(name: &str) -> Option<Rgba> {
	match name {
		"black"		=> Some(Rgba::BLACK),
		"white"		=> Some(Rgba::WHITE),
		"red"		=> Some(Rgba::opaque(255, 0, 0)),
		"green"		=> Some(Rgba::opaque(0, 128, 0)),
		"blue"		=> Some(Rgba::opaque(0, 0, 255)),
		"grey" | "gray"	=> Some(Rgba::opaque(128, 128, 128)),
		"yellow"	=> Some(Rgba::opaque(255, 255, 0)),
		"orange"	=> Some(Rgba::opaque(255, 165, 0)),
		"transparent"	=> Some(Rgba::TRANSPARENT),
		_			=> None,
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
