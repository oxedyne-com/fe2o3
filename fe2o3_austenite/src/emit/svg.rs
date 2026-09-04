//! The SVG page writer.
//!
//! The geometry and paint are handed to `fe2o3_graphics`: [`Path::rect`] builds each box, and the
//! crate's own [`write_path_data`] and [`presentation`] render the `d` attribute and the fill or
//! stroke. This module writes only the element tree around them -- the `<svg>`, `<rect>` and
//! `<path>` -- which `fe2o3_graphics::svg` deliberately leaves to the caller, because the document
//! shape above a `<path>` is the caller's format, not that crate's.

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
		let attrs	= match placed.kind {
			PlacedKind::Rule		=> presentation(Some(Rgba::BLACK), None),
			PlacedKind::Reserved	=> presentation(None, Some((grey, &pen))),
		};
		out.push_str(&fmt!("  <path d=\"{}\" {}/>\n", d, attrs));
	}

	// The folio, as page furniture rather than shaped body text -- the viewer's default face renders
	// it. Real folios become shaped glyph runs once a font exists in Phase 1.
	let folio_x = w / 2;
	let folio_y = h.saturating_sub(page.geom.margin.to_pt() as usize / 2);
	out.push_str(&fmt!(
		"  <text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"9\" fill=\"#000000\">{}</text>\n",
		folio_x, folio_y, page.number));

	out.push_str("</svg>\n");
	Ok(out)
}
