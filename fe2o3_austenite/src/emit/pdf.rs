//! The PDF page writer.
//!
//! The parallel of [`super::svg`], over the very same placed frames. Boxes and graphics go to
//! `fe2o3_graphics`'s [`PdfWriter`] as fill and stroke operators. Text goes as text: each glyph is shown
//! by id from its face's own font file, embedded once and subset to the glyphs the document uses, with a
//! `/ToUnicode` from the same [`ShapedText::glyph_text`] the SVG text layer reads, so the PDF's text
//! selects, copies and searches. A face that cannot be embedded falls back to filled outlines in a Type-3
//! font, which still extracts.
//!
//! A document is one file across all its pages, not a string per page, so this module's entry point is
//! [`render_document`] rather than the per-page `render_page` the [`super::Emitter`] enum uses for
//! SVG.

use crate::font::ShapedText;
use crate::ir::{
	DrawOp,
	Graphic,
	LinkTarget,
	Sp,
};
use crate::page::{
	Page,
	PlacedKind,
};

use std::io::Write;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
	pdf::{
		OutlineItem,
		PdfPage,
		PdfStream,
		PdfWriter,
	},
	transform::Transform,
};

/// Renders a whole document -- every page -- as one PDF file, held in a buffer. A convenience for a
/// short run; a whole book streams to a file with [`stream_document`] instead, which never holds more
/// than one page's outlines. The bytes are the same either way.
pub fn render_document(pages: &[Page]) -> Outcome<Vec<u8>> {
	let mut writer = PdfWriter::new().with_compression(true);
	for page in pages {
		writer.add_page(res!(render_page(page)));
	}
	writer.to_bytes()
}

/// Opens a page-at-a-time PDF stream over `out`, for a document of exactly `total` pages.
///
/// This is the streaming half of the emitter, and the reason a whole-book compile is flat in memory:
/// the caller composes one page, calls [`write_page`] to serialise it to `out`, then drops the page's
/// frame, so neither the engine nor the writer ever holds every page's glyph outlines at once. Close
/// the stream with [`PdfStream::finish`] once all `total` pages are written. Compression is on, as
/// [`render_document`] leaves it, so the two produce identical bytes.
pub fn open_document<W: Write>(out: W, total: usize) -> Outcome<PdfStream<W>> {
	PdfStream::new(out, total, true)
}

/// As [`open_document`], but the file also carries a document outline (the viewer's bookmark side
/// panel), built by the caller from the heading table and the front-matter anchors. An empty outline
/// yields a file byte-identical to [`open_document`]'s.
pub fn open_document_with_outline<W: Write>(
	out:		W,
	total:		usize,
	outline:	Vec<OutlineItem>,
)
	-> Outcome<PdfStream<W>>
{
	PdfStream::new_with_outline(out, total, true, outline)
}

/// Renders one page's frame to the open PDF stream. The page's outlines live only for this call: the
/// [`PdfPage`] built here is written and dropped before returning, so the caller may drop the page's
/// frame the moment this returns.
pub fn write_page<W: Write>(stream: &mut PdfStream<W>, page: &Page) -> Outcome<()> {
	stream.page(&res!(render_page(page)))
}

/// Writes a page whose draw list was built elsewhere -- on a worker thread, so the SVG the same walk
/// produces runs off the writer's thread. The content stream is serialised here, in page order, because
/// that is where a font's object number and a Type-3 glyph's code are assigned deterministically.
pub fn write_built_page<W: Write>(stream: &mut PdfStream<W>, pdf_page: &PdfPage) -> Outcome<()> {
	stream.page(pdf_page)
}

/// Builds one page's draw list: a white ground, then each placed box as a fill or a stroke.
///
/// The coordinates are the engine's page frame -- top-left origin, y down -- and are handed on
/// unflipped, since `fe2o3_graphics::pdf` flips the whole page itself.
pub fn render_page(page: &Page) -> Outcome<PdfPage> {
	let w = page.geom.width.to_pt();
	let h = page.geom.height.to_pt();
	let mut out = PdfPage::new(w, h);

	// A white ground, matching the SVG writer's opaque background rectangle.
	out.fill(res!(Path::rect(Bounds::new(0.0, 0.0, w as f32, h as f32))), Rgba::WHITE);

	// A half-point grey pen outlines a reservation, so a proof shows where a resolved value will sit
	// without the box reading as content.
	let grey = Rgba::new(176, 176, 176, 255);

	for placed in &page.frame.placed {
		// Real text is shown glyph by glyph; a rule or a reservation is one rectangle.
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
		let path = res!(Path::rect(Bounds::new(x0, y0, x1, y1)));
		match &placed.kind {
			PlacedKind::Rule		=> out.fill(path, Rgba::BLACK),
			PlacedKind::Reserved	=> out.stroke(path, grey, 0.5),
			PlacedKind::Text(_)		=> continue,	// drawn above
			PlacedKind::Graphic(_)	=> continue,	// drawn above
		}
	}

	// The running head and folio arrive as `PlacedKind::Text` and are shown with the body, above. This writer adds no page furniture of its own.
	Ok(out)
}

/// Draws a placed graphic: each op's path translated to where the graphic landed, then filled or
/// stroked. The paths are y down in points already, so only a translation is needed; the PDF writer
/// flips the whole page once, which leaves the graphic the right way up like the rest of the page.
fn draw_graphic(
	out:		&mut PdfPage,
	bx:			Sp,
	by:			Sp,
	graphic:	&Graphic,
)
	-> Outcome<()>
{
	let t = Transform::translate(bx.to_pt() as f32, by.to_pt() as f32);
	let ox = bx.to_pt();
	let oy = by.to_pt();
	// A linked graphic (the meta page's "Made with AI" chip) draws a clickable link annotation over its
	// placement box, in the same y-down engine frame the ink is placed in; the writer flips it into PDF
	// space. Only the mark carries the link, matching the template, where the words beside it are plain.
	// v0 draws only an external URI annotation here; an internal `LinkTarget::Anchor` needs the ledger to
	// resolve its destination page, which this per-graphic call does not hold, so it is left for a
	// follow-up (Pearl already carries the internal target for a reader that has the ledger).
	if let Some(LinkTarget::Uri(url)) = &graphic.link {
		let w = graphic.dims.width.to_pt();
		let h = (graphic.dims.height + graphic.dims.depth).to_pt();
		out.link(ox, oy, w, h, url.clone());
	}
	for op in &graphic.ops {
		match op {
			DrawOp::Fill { path, colour }			=> out.fill(res!(path.transform(&t)), *colour),
			DrawOp::Stroke { path, colour, width }	=> out.stroke(res!(path.transform(&t)), *colour, (*width).into()),
			DrawOp::Image { image, x, y, w, h } => {
				// The raster fills its rectangle at the graphic's placement; the PDF writer embeds it as an
				// image XObject, straight RGB with a soft mask only when a sample is translucent.
				let (rgb, alpha) = crate::image::split_rgba(image);
				out.image(
					rgb, alpha, image.width, image.height,
					ox + *x as f64, oy + *y as f64, *w as f64, *h as f64);
			},
		}
	}
	Ok(())
}

/// Shows a placed run: each glyph by id from its embedded face, or as a filled outline when the face
/// cannot be embedded. `height` is the line's own HBox height -- the face
/// ascent for an ordinary line, or the cap height for a first line raised under the block-edge model
/// (see `linebreak::set_lines`), whose glyphs then carry a compensating negative shift -- so
/// `by + height` is the baseline either way. `bx`/`by` are the box's top-left.
fn draw_text(
	out:	&mut PdfPage,
	bx:		Sp,
	by:		Sp,
	height:	Sp,
	shaped:	&ShapedText,
)
	-> Outcome<()>
{
	let base_x	= bx.to_pt() as f32;
	let base_y	= (by + height).to_pt() as f32;

	// The source scalar(s) each glyph stands for, for the font's /ToUnicode -- the very mapping the SVG
	// writer's selectable text layer draws on, so the two never disagree about what a glyph stands for.
	let texts = shaped.glyph_text();

	for (glyph, text) in shaped.run().glyphs.iter().zip(texts.into_iter()) {
		// The pen: x the glyph's left, y its baseline, in the engine's top-left y-down frame.
		let x = base_x + glyph.x;
		let y = base_y - glyph.y;
		if let Some(prog) = res!(shaped.program(glyph)) {
			// Every glyph is shown, a space included: its text is what puts the word gap into a copy.
			let gid = match u16::try_from(glyph.id) {
				Ok(g)	=> g,
				Err(_)	=> return Err(err!(
					"Glyph id {} exceeds the 16 bits a font program can index.", glyph.id; Invalid, Range)),
			};
			out.text(prog, gid, x, y, shaped.size(), shaped.colour(), text);
			continue;
		}
		// The writer stores this canonical outline once and shows it at the run's point size. A glyph with
		// no ink -- a space -- has an empty outline and is skipped, exactly as the SVG writer skips it, so the
		// two arms place the same marks; the viewer infers word gaps from the glyph positions.
		let outline = res!(shaped.outline_canonical(glyph));
		if outline.is_empty() {
			continue;
		}
		// The writer flips the outline back to y up within the page's y-flip, so the glyph reads upright.
		out.glyph(outline, x, y, shaped.size(), glyph.adv, shaped.colour(), text);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ir::{
		DrawOp,
		Dims,
		Graphic,
		Sp,
	};
	use crate::page::{
		Frame,
		Page,
		PageGeometry,
		Placed,
		PlacedKind,
	};
	use std::sync::Arc;

	#[test]
	fn text_embeds_its_font_and_extracts_via_tounicode() -> Outcome<()> {
		// A shaped word is shown from its embedded CFF face, and the font's /ToUnicode CMap maps its glyph
		// ids back to the source characters, so a viewer extracts the real word. Built uncompressed, so the
		// CMap is readable straight from the bytes.
		use crate::font::ShapedText;
		use oxedyne_fe2o3_font::{
			face::Role,
			shape::Dir,
		};

		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let shaped	= res!(ShapedText::new(fonts, Role::Body, Dir::Ltr, Sp::from_pt(11.0), "Oxegen"));
		let tdims	= shaped.dims();
		let mut frame = Frame::new();
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0), tdims, PlacedKind::Text(shaped)));
		let page	= Page::new(1, geom, frame);

		let pdf_page = res!(render_page(&page));
		let mut w	= PdfWriter::new();	// uncompressed, so the CMap is legible in the bytes
		w.add_page(pdf_page);
		let bytes	= res!(w.to_bytes());
		let text	= String::from_utf8_lossy(&bytes);

		assert!(text.contains("/Subtype /Type0"), "the word is shown from a composite font");
		assert!(text.contains("/Subtype /CIDFontType0 "), "over a CFF CIDFont");
		assert!(text.contains("/Subtype /CIDFontType0C"), "whose program is embedded");
		assert!(text.contains("+LibertinusSerif-Regular"), "under a subset tag and its PostScript name");
		assert!(!text.contains("/Subtype /Type3"), "no glyph falls back to outlines");
		assert!(text.contains(" Tj\n") || text.contains(" TJ\n"), "the glyphs are shown with text operators");
		assert!(text.contains("beginbfchar"), "a ToUnicode CMap carries character mappings");
		// The distinct letters of "Oxegen" appear as UTF-16BE destinations in the CMap.
		for (ch, hex) in [('O', "004F"), ('x', "0078"), ('e', "0065"), ('g', "0067"), ('n', "006E")] {
			assert!(text.contains(&fmt!("> <{}>", hex)),
				"the CMap maps '{}' (U+{}) so the word is extractable", ch, hex);
		}
		Ok(())
	}

	#[test]
	fn a_linked_graphic_emits_a_link_annotation() -> Outcome<()> {
		// A placed graphic carrying a link (the meta page's "Made with AI" chip) draws a PDF link annotation
		// over its box; a graphic with no link draws none, so the SVG-style plain image is unchanged.
		let geom	= PageGeometry::new(Sp::from_pt(200.0), Sp::from_pt(300.0), Sp::from_pt(20.0));
		let rect	= res!(Path::rect(Bounds::new(0.0, 0.0, 36.0, 36.0)));
		let graphic	= Graphic::new(
			vec![DrawOp::Fill { path: rect, colour: Rgba::BLACK }],
			Dims::new(Sp::from_pt(36.0), Sp::from_pt(36.0), Sp::ZERO))
			.with_link("https://need2know.ai/with-ai/doc".to_string());
		let mut frame = Frame::new();
		frame.push(Placed::new(
			Sp::from_pt(50.0), Sp::from_pt(80.0), graphic.dims,
			PlacedKind::Graphic(Arc::new(graphic))));
		let page	= Page::new(1, geom, frame);
		let bytes	= res!(render_document(&[page]));
		let text	= String::from_utf8_lossy(&bytes);
		assert!(text.contains("/Subtype /Link"), "a link annotation is emitted, found: {}", text);
		assert!(text.contains("/S /URI /URI (https://need2know.ai/with-ai/doc)"), "the URI action is written");
		// The box top-left (50, 80), 36 by 36, flips on a 300pt page to [50, 300-116, 86, 300-80] = [50 184 86 220].
		assert!(text.contains("/Rect [50 184 86 220]"), "the rectangle flips into PDF space, found: {}", text);
		Ok(())
	}

	#[test]
	fn an_unlinked_graphic_emits_no_annotation() -> Outcome<()> {
		let geom	= PageGeometry::new(Sp::from_pt(200.0), Sp::from_pt(300.0), Sp::from_pt(20.0));
		let rect	= res!(Path::rect(Bounds::new(0.0, 0.0, 36.0, 36.0)));
		let graphic	= Graphic::new(
			vec![DrawOp::Fill { path: rect, colour: Rgba::BLACK }],
			Dims::new(Sp::from_pt(36.0), Sp::from_pt(36.0), Sp::ZERO));
		let mut frame = Frame::new();
		frame.push(Placed::new(
			Sp::from_pt(50.0), Sp::from_pt(80.0), graphic.dims,
			PlacedKind::Graphic(Arc::new(graphic))));
		let page	= Page::new(1, geom, frame);
		let bytes	= res!(render_document(&[page]));
		let text	= String::from_utf8_lossy(&bytes);
		assert!(!text.contains("/Annots"), "no annotation array without a link");
		assert!(!text.contains("/Subtype /Link"), "no link annotation without a link");
		Ok(())
	}
}
