//! The SVG page writer.
//!
//! The geometry and paint are handed to `fe2o3_graphics`: [`Path::rect`] builds each box, a glyph's
//! outline arrives from `fe2o3_font` as a [`Path`] and is placed with [`Path::transform`], and the
//! crate's own [`write_path_data`] and [`presentation`] render the `d` attribute and the fill or
//! stroke. This module writes only the element tree around them -- the `<svg>`, `<rect>` and
//! `<path>` -- which `fe2o3_graphics::svg` deliberately leaves to the caller, because the document
//! shape above a `<path>` is the caller's format, not that crate's.

use crate::font::ShapedText;
use crate::ir::{
	DrawOp,
	Graphic,
	Sp,
};
use crate::page::{
	Page,
	PlacedKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
	stroke::Stroke,
	svg::{
		presentation,
		write_path_data,
	},
	transform::Transform,
};

/// Renders one page as a self-contained SVG document.
pub fn render_page(page: &Page) -> Outcome<String> {
	let size	= page.geom.media_box();
	let w		= size.x.as_usize();
	let h		= size.y.as_usize();

	// A half-point grey pen outlines a reservation, so a proof shows where a resolved value will sit
	// without the box reading as content.
	let pen		= res!(Stroke::new(0.5));
	let grey	= Rgba::new(176, 176, 176, 255);

	let mut out = String::new();
	out.push_str(&fmt!(
		"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
		w, h, w, h));
	out.push_str(&fmt!(
		"<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n", w, h));

	for placed in &page.frame.placed {
		// Real text is drawn glyph by glyph as filled outlines; a rule or a reservation as one
		// rectangle.
		if let PlacedKind::Text(shaped) = &placed.kind {
			res!(draw_text(&mut out, placed.x, placed.y, placed.dims.height, shaped));
			continue;
		}
		if let PlacedKind::Graphic(g) = &placed.kind {
			res!(draw_graphic(&mut out, placed.x, placed.y, g));
			continue;
		}

		let x0 = placed.x.to_pt() as f32;
		let y0 = placed.y.to_pt() as f32;
		let x1 = (placed.x + placed.dims.width).to_pt() as f32;
		let y1 = (placed.y + placed.dims.height + placed.dims.depth).to_pt() as f32;

		// A zero-area box has nothing to draw, and `Path::rect` would reject it.
		if x1 <= x0 || y1 <= y0 {
			continue;
		}
		let path	= res!(Path::rect(Bounds::new(x0, y0, x1, y1)));
		let d		= write_path_data(&path);
		let attrs	= match &placed.kind {
			PlacedKind::Rule		=> presentation(Some(Rgba::BLACK), None),
			PlacedKind::Reserved	=> presentation(None, Some((grey, &pen))),
			PlacedKind::Text(_)		=> continue,	// drawn above
			PlacedKind::Graphic(_)	=> continue,	// drawn above
		};
		out.push_str(&fmt!("  <path d=\"{}\" {}/>\n", d, attrs));
	}

	// The running head and folio are shaped runs placed into the frame's margins by
	// `doc::decorate`, so they arrive here as `PlacedKind::Text` and are drawn as glyph outlines with
	// the body, above. This writer adds no page furniture of its own.
	out.push_str("</svg>\n");
	Ok(out)
}

/// Draws a placed graphic: each op's path translated from the graphic's own frame to where the graphic
/// landed, then filled or stroked. The paths are already y down in points, so a translation suffices --
/// no flip, unlike a glyph outline.
fn draw_graphic(
	out:		&mut String,
	bx:			Sp,
	by:			Sp,
	graphic:	&Graphic,
)
	-> Outcome<()>
{
	let t = Transform::translate(bx.to_pt() as f32, by.to_pt() as f32);
	for op in &graphic.ops {
		match op {
			DrawOp::Fill { path, colour } => {
				let p = res!(path.transform(&t));
				out.push_str(&fmt!(
					"  <path d=\"{}\" {}/>\n", write_path_data(&p), presentation(Some(*colour), None)));
			},
			DrawOp::Stroke { path, colour, width } => {
				let pen	= res!(Stroke::new(*width));
				let p	= res!(path.transform(&t));
				out.push_str(&fmt!(
					"  <path d=\"{}\" {}/>\n", write_path_data(&p), presentation(None, Some((*colour, &pen)))));
			},
		}
	}
	Ok(())
}

/// Draws a placed run as filled glyph outlines. `height` is the face ascent, so `by + height` is the
/// baseline. `bx`/`by` are the box's top-left.
fn draw_text(
	out:	&mut String,
	bx:		Sp,
	by:		Sp,
	height:	Sp,
	shaped:	&ShapedText,
)
	-> Outcome<()>
{
	let base_x	= bx.to_pt() as f32;
	let base_y	= (by + height).to_pt() as f32;
	for glyph in &shaped.run().glyphs {
		let path = res!(shaped.outline(glyph));
		// A glyph with no ink -- a space -- carries an advance but nothing to fill.
		if path.is_empty() {
			continue;
		}
		// The outline is font-frame, y up; the page is y down. Flip in y, then move onto the baseline
		// at the glyph's own offset. The run is shaped in points, so no scale beyond the flip.
		let t = Transform::scale(1.0, -1.0)
			.then(&Transform::translate(base_x + glyph.x, base_y - glyph.y));
		let placed = res!(path.transform(&t));
		out.push_str(&fmt!(
			"  <path d=\"{}\" {}/>\n",
			write_path_data(&placed), presentation(Some(Rgba::BLACK), None)));
	}
	Ok(())
}
