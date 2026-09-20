//! Rasterises a Pearl page to a PNG, driving `fe2o3_graphics`'s anti-aliased [`Pixmap`] from the flat
//! drawing operations `svg_doc::read_document` reads out of the very SVG [`PearlDoc::render_page`]
//! reconstructs. Nothing here re-renders a page from the document's leaves directly: it walks
//! `pearl.rs`'s own SVG output through the graphics crate's own SVG-document reader -- the same loop
//! `fe2o3_graphics/examples/svg_raster.rs` already proves out for an arbitrary SVG file. The one new
//! thing this module adds is scaling by a caller-chosen DPI rather than a fixed pixel width, and skipping
//! the invisible selectable-text layer (see [`render_page_to_pixmap`]'s own comment).

use oxedyne_fe2o3_austenite::emit::pearl::PearlDoc;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::Bounds;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::stroke::Stroke;
use oxedyne_fe2o3_graphics::svg_doc::{
	self,
	SvgOp,
};
use oxedyne_fe2o3_graphics::transform::Transform;

/// The standard screen DPI Pearl's own SVG is authored at: one point equals one pixel, so a page comes
/// out at its media-box size in pixels.
pub const DEFAULT_DPI: f32 = 72.0;

/// A rasterised page: its pixels, and the pixel dimensions they were drawn at.
pub struct RasterPage {
	pub pixmap:		Pixmap,
	pub width_px:	usize,
	pub height_px:	usize,
}

/// Renders one page of a Pearl document to a pixmap at `dpi` dots per inch.
///
/// The route is `PearlDoc::render_page` (the format's own SVG reconstruction) through
/// `svg_doc::read_document` (the graphics crate's flat op reader) onto a fresh [`Pixmap`]. A `<text
/// class="tsel">` run -- the invisible, selectable text layer `pearl.rs` appends once per page, after
/// every page's own ink -- is dropped here rather than drawn or marked: it is `fill: transparent` in a
/// browser (a CSS class rule this flat op reader never sees, since it does not descend into `<style>`),
/// and every visible glyph has already arrived as its own `Fill` path from the leaf's stored outline, so
/// drawing the tsel run too would double the ink, and a placeholder baseline marker -- what
/// `svg_raster.rs`'s generic example draws for unshaped text -- would sit directly on top of glyphs that
/// are already correctly rendered.
pub fn render_page_to_pixmap(doc: &PearlDoc, idx: usize, dpi: f32) -> Outcome<RasterPage> {
	let svg	= res!(doc.render_page(idx));
	let pic	= res!(svg_doc::read_document(&svg));

	let s			= dpi / DEFAULT_DPI;
	let width_px	= ((pic.width * s).ceil() as usize).max(1);
	let height_px	= ((pic.height * s).ceil() as usize).max(1);
	let t			= Transform::scale(s, s);

	let mut pm = res!(Pixmap::new(width_px, height_px));
	res!(pm.fill_bounds(
		Bounds::new(0.0, 0.0, width_px as f32, height_px as f32), Rgba::WHITE, None));

	for op in pic.ops {
		match op {
			SvgOp::Fill { path, colour } => {
				res!(pm.fill_path(&path, &t, colour, None));
			},
			SvgOp::Stroke { path, colour, stroke } => {
				if stroke.dash.is_some() {
					// A dashed pen has no single-width fast path in `stroke_path`; bake it to its filled
					// outline first, exactly as the generic SVG rasteriser example does.
					let outline = res!(path.stroke(&stroke));
					res!(pm.fill_path(&outline, &t, colour, None));
				} else {
					let pen = res!(Stroke::new(stroke.width));
					let pen = pen.with_cap(stroke.cap).with_join(stroke.join);
					res!(pm.stroke_path(&path, &t, colour, None, &pen));
				}
			},
			// The invisible tsel layer; see this function's own comment.
			SvgOp::Text { .. } => {},
			SvgOp::Image { rgba, iw, ih, x, y, w, h } => {
				if iw == 0 || ih == 0 || w <= 0.0 || h <= 0.0 {
					continue;
				}
				let img	= res!(Pixmap::from_data(iw, ih, rgba));
				let dw	= (w * s).max(1.0);
				let dh	= (h * s).max(1.0);
				let ox	= x * s;
				let oy	= y * s;
				let px0	= ox.floor().max(0.0) as usize;
				let py0	= oy.floor().max(0.0) as usize;
				let px1	= ((ox + dw).ceil() as usize).min(width_px);
				let py1	= ((oy + dh).ceil() as usize).min(height_px);
				// Nearest-neighbour, as the graphics crate's own SVG rasteriser example does: enough
				// fidelity for a document's embedded raster, and it pulls in no resampler.
				for py in py0..py1 {
					for px in px0..px1 {
						let u = (((px as f32) + 0.5 - ox) / dw) * (iw as f32);
						let v = (((py as f32) + 0.5 - oy) / dh) * (ih as f32);
						if u < 0.0 || v < 0.0 {
							continue;
						}
						let sx = (u as usize).min(iw - 1);
						let sy = (v as usize).min(ih - 1);
						if let Some(c) = img.pixel(sx, sy) {
							pm.blend_pixel(px, py, c);
						}
					}
				}
			},
		}
	}

	Ok(RasterPage { pixmap: pm, width_px, height_px })
}

/// Renders every page of `doc` to PNG bytes, in page order.
pub fn render_all_pages_to_png(doc: &PearlDoc, dpi: f32) -> Outcome<Vec<Vec<u8>>> {
	let pages = res!(doc.page_count());
	let mut out = Vec::with_capacity(pages);
	for idx in 0..pages {
		let raster = res!(render_page_to_pixmap(doc, idx, dpi));
		out.push(res!(raster.pixmap.to_png()));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	// A checked-in sample .prl rasters at 96 DPI to a pixmap whose dimensions are exactly the SVG
	// viewBox scaled by 96/72 -- proving the DPI scaling landed, not just that some raster came out --
	// and whose pixels are not all the white the canvas starts on.
	#[test]
	fn test_a_sample_prl_rasters_to_a_correctly_sized_non_blank_png_00() -> Outcome<()> {
		let path = concat!(env!("CARGO_MANIFEST_DIR"),
			"/../fe2o3_austenite/web/pearl-reader/samples/keystone.prl");
		let doc = res!(PearlDoc::read_file(path));
		assert_eq!(res!(doc.page_count()), 1, "keystone.prl is a one-page fixture");

		let dpi		= 96.0;
		let raster	= res!(render_page_to_pixmap(&doc, 0, dpi));

		let svg	= res!(doc.render_page(0));
		let pic	= res!(svg_doc::read_document(&svg));
		let want_w = ((pic.width * (dpi / DEFAULT_DPI)).ceil() as usize).max(1);
		let want_h = ((pic.height * (dpi / DEFAULT_DPI)).ceil() as usize).max(1);
		assert_eq!(raster.width_px, want_w, "pixel width did not scale by the chosen DPI");
		assert_eq!(raster.height_px, want_h, "pixel height did not scale by the chosen DPI");

		let has_ink = raster.pixmap.data().chunks(4).any(|px| px != [255, 255, 255, 255]);
		assert!(has_ink, "the rasterised page is a blank white canvas");

		let png = res!(raster.pixmap.to_png());
		assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "to_png did not write a PNG signature");
		Ok(())
	}

	// A multi-page document renders one PNG per page, and the count matches `page_count`.
	#[test]
	fn test_render_all_pages_matches_page_count_01() -> Outcome<()> {
		let path = concat!(env!("CARGO_MANIFEST_DIR"),
			"/../fe2o3_austenite/web/pearl-reader/samples/manuscript.prl");
		let doc		= res!(PearlDoc::read_file(path));
		let pages	= res!(doc.page_count());
		assert!(pages > 1, "manuscript.prl should be a multi-page fixture");

		let pngs = res!(render_all_pages_to_png(&doc, DEFAULT_DPI));
		assert_eq!(pngs.len(), pages, "one PNG was not written per page");
		for png in &pngs {
			assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
		}
		Ok(())
	}
}
