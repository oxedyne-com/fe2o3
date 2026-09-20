//! Rasterises a Pearl page to a pixmap, driving `fe2o3_graphics`'s anti-aliased [`Pixmap`] straight from
//! the one leaf walk `pearl.rs` exposes as [`PearlDoc::render_page_to`]. A [`PixmapSink`] is a
//! [`PageSink`](oxedyne_fe2o3_austenite::emit::pearl::PageSink): the SVG writer and this rasteriser share
//! that single walk, so a page reaches pixels without ever round-tripping through an SVG string. The one
//! new thing this module adds over the SVG arm is scaling by a caller-chosen DPI rather than a fixed
//! pixel width, and skipping the invisible selectable-text layer (see [`PixmapSink::text_layer`]).

use oxedyne_fe2o3_austenite::emit::pearl::{
	PageSink,
	PearlDoc,
	TselRun,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	Path,
};
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::stroke::Stroke;
use oxedyne_fe2o3_graphics::transform::Transform;
use oxedyne_fe2o3_text::base64;

/// The standard screen DPI Pearl's own SVG is authored at: one point equals one pixel, so a page comes
/// out at its media-box size in pixels.
pub const DEFAULT_DPI: f32 = 72.0;

/// A rasterised page: its pixels, and the pixel dimensions they were drawn at.
pub struct RasterPage {
	pub pixmap:		Pixmap,
	pub width_px:	usize,
	pub height_px:	usize,
}

/// A [`PageSink`] that draws a page's placed ink onto an anti-aliased [`Pixmap`] at a chosen DPI. Every
/// path arrives in the page's point frame; the sink applies the DPI scale as its device transform. The
/// selectable-text layer is invisible ink, so it is dropped rather than drawn.
pub struct PixmapSink {
	dpi:		f32,
	pixmap:		Option<Pixmap>,
	width_px:	usize,
	height_px:	usize,
}

impl PixmapSink {
	pub fn new(dpi: f32) -> Self {
		Self { dpi, pixmap: None, width_px: 0, height_px: 0 }
	}

	/// The finished page, or an error if no page was ever opened into the sink.
	pub fn finish(self) -> Outcome<RasterPage> {
		let pixmap = res!(self.pixmap.ok_or_else(|| err!(
			"finish called before a page was rendered into the pixmap sink."; Bug, Missing)));
		Ok(RasterPage { pixmap, width_px: self.width_px, height_px: self.height_px })
	}

	/// The pixel-per-point device scale for the sink's DPI.
	fn scale(&self) -> f32 {
		self.dpi / DEFAULT_DPI
	}

	/// The device transform for the sink's DPI.
	fn device(&self) -> Transform {
		let s = self.scale();
		Transform::scale(s, s)
	}

	/// The open pixmap, or an error naming the sink method that ran before [`PageSink::begin`].
	fn canvas(&mut self, who: &str) -> Outcome<&mut Pixmap> {
		Ok(res!(self.pixmap.as_mut().ok_or_else(|| err!(
			"pixmap sink {} was called before begin opened a page.", who; Bug, Missing))))
	}
}

impl PageSink for PixmapSink {
	fn begin(&mut self, w: usize, h: usize) -> Outcome<()> {
		let s = self.scale();
		// The media box scaled by the DPI, ceiled and floored at one, exactly as the media-box viewport
		// (whole points) scales -- so a page's pixel size is the SVG viewBox size times the DPI ratio.
		self.width_px	= (((w as f32) * s).ceil() as usize).max(1);
		self.height_px	= (((h as f32) * s).ceil() as usize).max(1);
		let mut pm = res!(Pixmap::new(self.width_px, self.height_px));
		res!(pm.fill_bounds(
			Bounds::new(0.0, 0.0, self.width_px as f32, self.height_px as f32), Rgba::WHITE, None));
		self.pixmap = Some(pm);
		Ok(())
	}

	fn fill(&mut self, path: &Path, colour: Rgba) -> Outcome<()> {
		let t = self.device();
		let pm = res!(self.canvas("fill"));
		res!(pm.fill_path(path, &t, colour, None));
		Ok(())
	}

	fn stroke(&mut self, path: &Path, colour: Rgba, pen: &Stroke) -> Outcome<()> {
		let t = self.device();
		let pm = res!(self.canvas("stroke"));
		if pen.dash.is_some() {
			// A dashed pen has no single-width fast path in `stroke_path`; bake it to its filled outline
			// first, exactly as the generic SVG rasteriser example does. Pearl's own leaves never carry a
			// dash, so this arm is future-proofing, not a path any current `.prl` reaches.
			let outline = res!(path.stroke(pen));
			res!(pm.fill_path(&outline, &t, colour, None));
		} else {
			res!(pm.stroke_path(path, &t, colour, None, pen));
		}
		Ok(())
	}

	fn image(&mut self, png_base64: &str, x: f32, y: f32, w: f32, h: f32) -> Outcome<()> {
		if w <= 0.0 || h <= 0.0 {
			return Ok(());
		}
		let s = self.scale();
		// Decode the embedded PNG the way `svg_doc::emit_image` does -- strip any wrapping whitespace, then
		// straight to a pixmap -- so the direct route matches the old SVG-reparse route byte for byte.
		let clean: String = png_base64.chars().filter(|c| !c.is_whitespace()).collect();
		let bytes = res!(base64::decode(&clean));
		let img = res!(Pixmap::from_png(&bytes));
		let iw = img.width();
		let ih = img.height();
		if iw == 0 || ih == 0 {
			return Ok(());
		}
		let dw	= (w * s).max(1.0);
		let dh	= (h * s).max(1.0);
		let ox	= x * s;
		let oy	= y * s;
		let px0	= ox.floor().max(0.0) as usize;
		let py0	= oy.floor().max(0.0) as usize;
		let px1	= ((ox + dw).ceil() as usize).min(self.width_px);
		let py1	= ((oy + dh).ceil() as usize).min(self.height_px);
		let pm = res!(self.canvas("image"));
		// Nearest-neighbour, as the graphics crate's own SVG rasteriser example does: enough fidelity for a
		// document's embedded raster, and it pulls in no resampler.
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
		Ok(())
	}

	// The invisible, selectable text layer places no ink -- every visible glyph has already arrived as its
	// own `fill` from the leaf's stored outline -- so a visual sink drops it. In a browser it is `fill:
	// transparent`; drawing it here would double the ink.
	fn text_layer(&mut self, _runs: &[TselRun]) -> Outcome<()> {
		Ok(())
	}

	fn end(&mut self) -> Outcome<()> {
		Ok(())
	}
}

/// Renders one page of a Pearl document to a pixmap at `dpi` dots per inch, driving the shared leaf walk
/// through a [`PixmapSink`].
pub fn render_page_to_pixmap(doc: &PearlDoc, idx: usize, dpi: f32) -> Outcome<RasterPage> {
	let mut sink = PixmapSink::new(dpi);
	res!(doc.render_page_to(idx, &mut sink));
	sink.finish()
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

	use oxedyne_fe2o3_graphics::svg_doc::{
		self,
		SvgOp,
	};

	// The old route to pixels, kept here as a reference oracle: `render_page` to an SVG string, re-parsed
	// by the graphics crate's SVG-document reader, then blitted onto a pixmap. The direct `PixmapSink`
	// must reproduce this byte for byte -- that is what proves the leaf-walk refactor changed nothing.
	fn render_via_svg_reparse(doc: &PearlDoc, idx: usize, dpi: f32) -> Outcome<RasterPage> {
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
						let outline = res!(path.stroke(&stroke));
						res!(pm.fill_path(&outline, &t, colour, None));
					} else {
						let pen = res!(Stroke::new(stroke.width));
						let pen = pen.with_cap(stroke.cap).with_join(stroke.join);
						res!(pm.stroke_path(&path, &t, colour, None, &pen));
					}
				},
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

	// A checked-in sample .prl rasters at 96 DPI to a pixmap whose dimensions are exactly the SVG viewBox
	// scaled by 96/72 -- proving the DPI scaling landed, not just that some raster came out -- and whose
	// pixels are not all the white the canvas starts on.
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

	// The refactor's raster proof: the direct `PixmapSink` produces the SAME pixels, byte for byte, as the
	// old route that re-parsed `render_page`'s SVG. Checked across every page of the checked-in fixtures,
	// at 72 and 144 DPI, so both the one-to-one and a scaled case are covered.
	#[test]
	fn test_direct_sink_matches_the_svg_reparse_route_byte_for_byte_02() -> Outcome<()> {
		for name in ["keystone.prl", "manuscript.prl"] {
			let path = fmt!("{}/../fe2o3_austenite/web/pearl-reader/samples/{}",
				env!("CARGO_MANIFEST_DIR"), name);
			let doc		= res!(PearlDoc::read_file(&path));
			let pages	= res!(doc.page_count());
			for dpi in [72.0_f32, 144.0_f32] {
				for idx in 0..pages {
					let direct	= res!(render_page_to_pixmap(&doc, idx, dpi));
					let oracle	= res!(render_via_svg_reparse(&doc, idx, dpi));
					assert_eq!(direct.width_px, oracle.width_px,
						"{} page {} at {} dpi: width differs", name, idx, dpi);
					assert_eq!(direct.height_px, oracle.height_px,
						"{} page {} at {} dpi: height differs", name, idx, dpi);
					assert_eq!(direct.pixmap.data(), oracle.pixmap.data(),
						"{} page {} at {} dpi: the direct sink and the SVG-reparse route drew different pixels",
						name, idx, dpi);
				}
			}
		}
		Ok(())
	}
}
