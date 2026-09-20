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
use crate::memo::{
	Fnv,
	Memo,
};
use crate::page::{
	Page,
	Placed,
	PlacedKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
	pixmap::Pixmap,
	stroke::Stroke,
	svg::{
		presentation,
		write_path_data,
	},
	transform::Transform,
};
use oxedyne_fe2o3_text::base64;
use oxedyne_fe2o3_text::xml::write::escape as xml_escape;

/// Renders one page as a self-contained SVG document. This is the whole-frame path, unchanged in its
/// bytes: it renders every placed item -- body and furniture alike -- as one slice.
pub fn render_page(page: &Page) -> Outcome<String> {
	let mut ink		= String::new();
	let mut tspans	= String::new();
	let mut seen	= false;
	res!(render_slice(&page.frame.placed, &mut ink, &mut tspans, &mut seen));
	Ok(assemble(page, &ink, &tspans))
}

/// Renders one page through the emit memo. The body frame (`placed[..body_len]`) is content-hashed;
/// a hit reuses its previously rendered ink and selectable-text tspans, a miss renders and stores them.
/// The furniture beyond the body -- the running head and folio, whose folio differs page to page -- is
/// always drawn fresh and concatenated on, so the assembled bytes are identical to a whole-frame render
/// of the same page. A page with no recorded body split (`body_len == usize::MAX`, every non-memo path)
/// treats its whole frame as body, so a first, cold compile caches exactly what it drew.
pub fn render_page_memo(page: &Page, memo: &mut Memo) -> Outcome<String> {
	let split	= page.body_len();
	let body	= &page.frame.placed[..split];
	let furn	= &page.frame.placed[split..];

	let key = page_key(page, body);
	let (body_ink, body_tspans, seen_after) = match memo.page_lookup(key) {
		Some(e)	=> (e.body_ink, e.body_tspans, e.seen_text),
		None	=> {
			let mut ink		= String::new();
			let mut tspans	= String::new();
			let mut seen	= false;
			res!(render_slice(body, &mut ink, &mut tspans, &mut seen));
			memo.page_store(key, ink.clone(), tspans.clone(), seen);
			(ink, tspans, seen)
		},
	};

	// The furniture, drawn fresh, its selectable tspans continuing the body's interword spacing.
	let mut furn_ink	= String::new();
	let mut furn_tspans	= String::new();
	let mut seen		= seen_after;
	res!(render_slice(furn, &mut furn_ink, &mut furn_tspans, &mut seen));

	let ink		= fmt!("{}{}", body_ink, furn_ink);
	let tspans	= fmt!("{}{}", body_tspans, furn_tspans);
	Ok(assemble(page, &ink, &tspans))
}

/// The stable identity of a whole page for the changed-only delta ([`crate::delta`]): the content hash of
/// its ENTIRE frame -- body and furniture (running head, folio) alike -- so two compiles yield the same id
/// for a page exactly when it renders to the same SVG bytes. Deliberately a superset of [`page_key`], which
/// hashes the body alone for the emit memo (whose furniture is always redrawn fresh); the delta needs the
/// folio in the key, since the consumer caches and reuses the whole page SVG by this id, and a page whose
/// only change is its printed folio renders differently and must be resent.
pub(crate) fn page_id(page: &Page) -> u64 {
	page_key(page, &page.frame.placed)
}

/// The content key of a page's body frame: its geometry and every body-placed item's position, size and
/// ink. Furniture is excluded (it hashes nothing here); the verso mirror shift is already baked into the
/// placed positions, so a page that changes parity hashes differently and misses, which is correct -- its
/// body sits at different coordinates. Theme is not in the key because it is baked into the shaped runs
/// (a run's glyph ids and colour already reflect it) and into the caller's global fingerprint besides.
fn page_key(page: &Page, body: &[Placed]) -> u64 {
	let mut h = Fnv::new();
	h.write(b"page");
	let size = page.geom.media_box();
	h.write_usize(size.x.as_usize());
	h.write_usize(size.y.as_usize());
	for p in body {
		h.write_i32(p.x.raw());
		h.write_i32(p.y.raw());
		h.write_i32(p.dims.width.raw());
		h.write_i32(p.dims.height.raw());
		h.write_i32(p.dims.depth.raw());
		match &p.kind {
			PlacedKind::Rule		=> h.write_u8(0),
			PlacedKind::Reserved	=> h.write_u8(1),
			PlacedKind::Text(s)		=> { h.write_u8(2); s.hash_into(&mut h); },
			PlacedKind::Graphic(g)	=> { h.write_u8(3); hash_graphic(g, &mut h); },
		}
	}
	h.finish()
}

/// Folds a placed graphic into a page key: each op by its kind, its paint, and its geometry. A path is
/// hashed by the very `d` string the writer emits, so two paths hash alike exactly when they draw alike.
fn hash_graphic(g: &Graphic, h: &mut Fnv) {
	for op in &g.ops {
		match op {
			DrawOp::Fill { path, colour } => {
				h.write_u8(0);
				h.write_str(&write_path_data(path));
				h.write(&[colour.r, colour.g, colour.b, colour.a]);
			},
			DrawOp::Stroke { path, colour, width } => {
				h.write_u8(1);
				h.write_str(&write_path_data(path));
				h.write(&[colour.r, colour.g, colour.b, colour.a]);
				h.write_f32(*width);
			},
			DrawOp::Image { image, x, y, w, h: ht } => {
				h.write_u8(2);
				h.write_usize(image.width);
				h.write_usize(image.height);
				h.write(&image.rgba);
				h.write_f32(*x);
				h.write_f32(*y);
				h.write_f32(*w);
				h.write_f32(*ht);
			},
		}
	}
}

/// Wraps the visible ink and the selectable tspans of a page in the SVG document shell -- the `<svg>`
/// viewport, the white backing rectangle, the `.tsel` style, and the one page-wide `<text>` layer -- so
/// every render path, memo or not, produces the same bytes for the same content.
fn assemble(page: &Page, ink: &str, tspans: &str) -> String {
	let size	= page.geom.media_box();
	let w		= size.x.as_usize();
	let h		= size.y.as_usize();
	let mut out = String::new();
	out.push_str(&fmt!(
		"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
		w, h, w, h));
	out.push_str(&fmt!(
		"<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n", w, h));
	// `.tsel` (Typst.ts's own name for the same idea) is the selectable text layer's class: transparent,
	// so it draws nothing over the glyph outlines below, and pointer-events left at the SVG default
	// (not `none`) so a mouse drag still hits real text nodes rather than only outline paths.
	out.push_str("<style>.tsel { fill: transparent; }</style>\n");
	out.push_str(ink);
	if !tspans.is_empty() {
		out.push_str(&fmt!("  <text class=\"tsel\">{}</text>\n", tspans));
	}
	out.push_str("</svg>\n");
	out
}

/// Renders one slice of a page's placed items into two buffers: `ink` gets the visible glyph outlines,
/// rules and graphics; `tspans` gets the invisible selectable text layer. `seen_text` threads across
/// slices, so the furniture's first run takes its leading interword space exactly as it would in a
/// single-pass whole-frame render. This is the per-item logic [`render_page`] always ran, split out so
/// the body and the furniture can be rendered -- and the body cached -- independently.
///
/// The visible pass runs first, in item order, then the selectable pass: Austenite's SVG carries only
/// glyph outlines, which a browser can render but neither select nor search, so Typst.ts's answer -- a
/// transparent text layer at the same baseline positions -- is mirrored on top. Every run's tspans join
/// ONE page-wide `<text>` (assembled by [`assemble`]): Chromium's `window.find` was tested to fail across
/// a boundary between two sibling `<text>` elements once their tspans carry per-glyph `x`/`y`, so a single
/// element sidesteps it. The line breaker leaves the interword gap as pure position, not a space glyph, so
/// a single invisible space is inserted ahead of every run but the first to restore it for search and copy.
fn render_slice(
	placed:		&[Placed],
	ink:		&mut String,
	tspans:		&mut String,
	seen_text:	&mut bool,
)
	-> Outcome<()>
{
	// A half-point grey pen outlines a reservation, so a proof shows where a resolved value will sit
	// without the box reading as content.
	let pen		= res!(Stroke::new(0.5));
	let grey	= Rgba::new(176, 176, 176, 255);

	for p in placed {
		// Real text is drawn glyph by glyph as filled outlines; a rule or a reservation as one rectangle.
		if let PlacedKind::Text(shaped) = &p.kind {
			res!(draw_text(ink, p.x, p.y, p.dims.height, shaped));
			continue;
		}
		if let PlacedKind::Graphic(g) = &p.kind {
			res!(draw_graphic(ink, p.x, p.y, g));
			continue;
		}

		let x0 = p.x.to_pt() as f32;
		let y0 = p.y.to_pt() as f32;
		let x1 = (p.x + p.dims.width).to_pt() as f32;
		let y1 = (p.y + p.dims.height + p.dims.depth).to_pt() as f32;

		// A zero-area box has nothing to draw, and `Path::rect` would reject it.
		if x1 <= x0 || y1 <= y0 {
			continue;
		}
		let path	= res!(Path::rect(Bounds::new(x0, y0, x1, y1)));
		let d		= write_path_data(&path);
		let attrs	= match &p.kind {
			PlacedKind::Rule		=> presentation(Some(Rgba::BLACK), None),
			PlacedKind::Reserved	=> presentation(None, Some((grey, &pen))),
			PlacedKind::Text(_)		=> continue,	// drawn above
			PlacedKind::Graphic(_)	=> continue,	// drawn above
		};
		ink.push_str(&fmt!("  <path d=\"{}\" {}/>\n", d, attrs));
	}

	for p in placed {
		if let PlacedKind::Text(shaped) = &p.kind {
			if res!(run_text_layer(tspans, p.x, p.y, p.dims.height, shaped, *seen_text)) {
				*seen_text = true;
			}
		}
	}
	Ok(())
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
	let ox = bx.to_pt() as f32;
	let oy = by.to_pt() as f32;
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
			DrawOp::Image { image, x, y, w, h } => {
				// The raster is re-encoded to PNG and embedded as a base64 data URI in an `<image>`. Its
				// frame is the page's own -- top-left, y down -- so the rectangle is placed directly, with
				// no flip; `preserveAspectRatio="none"` lets the box already sized to the aspect fill.
				let pm	= res!(Pixmap::from_data(image.width, image.height, image.rgba.clone()));
				let png	= res!(pm.to_png());
				let b64	= base64::encode(&png);
				out.push_str(&fmt!(
					"  <image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" \
						href=\"data:image/png;base64,{}\"/>\n",
					ox + *x, oy + *y, *w, *h, b64));
			},
		}
	}
	Ok(())
}

/// Draws a placed run as filled glyph outlines. `height` is the line's own HBox height -- the face
/// ascent for an ordinary line, or the cap height for a first line raised under the block-edge model
/// (see `linebreak::set_lines`), whose glyphs then carry a compensating negative shift -- so
/// `by + height` is the baseline either way. `bx`/`by` are the box's top-left.
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
			write_path_data(&placed), presentation(Some(shaped.colour()), None)));
	}
	Ok(())
}

/// Appends a placed run's invisible, selectable tspans to the page's one `.tsel` text buffer: one
/// `<tspan>` per inked glyph, each carrying the source text [`ShapedText::glyph_text`] maps that glyph
/// to, its own `font-size` (runs on a page differ -- a heading against a caption), and sitting exactly
/// on that glyph's own baseline position -- the same `(base_x + glyph.x, base_y - glyph.y)`
/// [`draw_text`] paints the outline at, so the invisible character and the visible one it stands in for
/// never drift apart. The mapping is the very one the PDF writer's `/ToUnicode` CMap uses, not a fresh
/// derivation, so the two extraction paths can never disagree about what a glyph says.
///
/// `sep` asks for a leading space, ahead of this run's own tspans, standing in for the interword gap the
/// line breaker never gives a glyph of its own (see the call site in `render_page`). Returns whether
/// anything was appended, so the caller only counts a run that actually carried a character towards
/// "there was a previous run to space this one from".
fn run_text_layer(
	buf:	&mut String,
	bx:		Sp,
	by:		Sp,
	height:	Sp,
	shaped:	&ShapedText,
	sep:	bool,
)
	-> Outcome<bool>
{
	let base_x	= bx.to_pt() as f32;
	let base_y	= (by + height).to_pt() as f32;
	let size	= shaped.size();
	let texts	= shaped.glyph_text();

	let mut spans = String::new();
	for (glyph, text) in shaped.run().glyphs.iter().zip(texts.iter()) {
		// A glyph with no text of its own -- a later part of a ligature or decomposed mark, already
		// claimed by an earlier glyph at the same cluster -- contributes no span; the earlier one already
		// carries the character.
		if text.is_empty() {
			continue;
		}
		spans.push_str(&fmt!(
			"<tspan x=\"{}\" y=\"{}\" font-size=\"{}\">{}</tspan>",
			base_x + glyph.x, base_y - glyph.y, size, xml_escape(text)));
	}
	// A run with nothing inked -- entirely spaces -- has nothing to select.
	if spans.is_empty() {
		return Ok(false);
	}
	if sep {
		buf.push_str(&fmt!("<tspan x=\"{}\" y=\"{}\" font-size=\"{}\"> </tspan>", base_x, base_y, size));
	}
	buf.push_str(&spans);
	Ok(true)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ir::{
		LeafKind,
		Node,
	};
	use crate::linebreak::break_paragraph;
	use crate::page::{
		Frame,
		PageGeometry,
		Placed,
	};
	use oxedyne_fe2o3_font::{
		face::Role,
		shape::Dir,
	};
	use std::sync::Arc;

	/// A body paragraph set with a fill draws its glyphs in that colour, and a default paragraph stays
	/// black. The paragraph is broken by [`break_paragraph`] with the fill threaded from the theme, then
	/// its text leaves are placed and rendered, so the test exercises the whole thread from the line
	/// breaker to the SVG paint. Black is asserted free of red -- the byte-identity the all-black corpus
	/// rests on, since a black run's paint is exactly what it was before text carried a colour.
	#[test]
	fn a_paragraph_renders_in_its_set_fill() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let measure	= Sp::from_pt(400.0);
		let size	= Sp::from_pt(11.0);
		let leading	= Sp::from_pt(13.2);

		// A run set red renders red glyphs.
		let rnodes	= res!(break_paragraph(
			fonts.clone(), Role::Body, Dir::Ltr, size, "Red prose here", measure, leading, false, Rgba::opaque(255, 0, 0), None));
		let rsvg	= res!(render_text_leaves(geom, &rnodes));
		assert!(rsvg.contains("fill=\"#ff0000\""), "a paragraph set red must draw red glyphs, found: {}", rsvg);

		// The default fill (black) renders black glyphs and never red.
		let bnodes	= res!(break_paragraph(
			fonts.clone(), Role::Body, Dir::Ltr, size, "Black prose here", measure, leading, false, Rgba::BLACK, None));
		let bsvg	= res!(render_text_leaves(geom, &bnodes));
		assert!(bsvg.contains("fill=\"#000000\""), "a default paragraph must draw black glyphs");
		assert!(!bsvg.contains("fill=\"#ff0000\""), "a default paragraph must never draw red");
		Ok(())
	}

	/// The selectable text layer carries the same word the outlines draw, transparent, and the outlines
	/// are unmoved by its presence -- the visible ink stays exactly what it was, this test's own name for
	/// why the oracle's PDF hash (built from the same outlines, on the same path) is untouched by an
	/// SVG-only addition.
	#[test]
	fn a_run_gets_an_invisible_selectable_twin() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let shaped	= res!(crate::font::ShapedText::new(fonts, Role::Body, Dir::Ltr, Sp::from_pt(11.0), "Oxegen"));
		let dims	= shaped.dims();
		let mut frame = Frame::new();
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0), dims, PlacedKind::Text(shaped)));
		let svg = res!(render_page(&Page::new(1, geom, frame)));

		assert!(svg.contains("class=\"tsel\""), "a selectable text layer is drawn, found: {}", svg);
		assert!(svg.contains(".tsel { fill: transparent; }"), "the layer is transparent");
		// The word is recoverable letter by letter, as `window.find`/copy-paste would see it: each
		// character sits in its own positioned `<tspan>`, in order.
		for ch in "Oxegen".chars() {
			assert!(svg.contains(&fmt!(">{}</tspan>", ch)), "'{}' is drawn as a selectable tspan, found: {}", ch, svg);
		}
		// The visible outlines are unaffected: still one filled black path per inked glyph, nothing new
		// added to that part of the document.
		assert!(svg.contains("fill=\"#000000\""), "the outline glyphs still draw in black");
		Ok(())
	}

	/// Places every text leaf of a broken paragraph into a frame and renders it to SVG, so a test can see
	/// the fill the line breaker set without standing up the whole driver.
	fn render_text_leaves(geom: PageGeometry, nodes: &[Node]) -> Outcome<String> {
		let mut frame	= Frame::new();
		let mut x		= Sp::from_pt(60.0);
		let y			= Sp::from_pt(80.0);
		for node in nodes {
			place_text_leaves(node, &mut frame, &mut x, y);
		}
		render_page(&Page::new(1, geom, frame))
	}

	// Walks a node tree, placing each text leaf at the running pen so its glyphs reach the SVG.
	fn place_text_leaves(node: &Node, frame: &mut Frame, x: &mut Sp, y: Sp) {
		match node {
			Node::Leaf(leaf) => {
				if let LeafKind::Text(shaped) = &leaf.kind {
					frame.push(Placed::new(*x, y, leaf.dims, PlacedKind::Text(shaped.clone())));
					*x = Sp(x.raw() + leaf.dims.width.raw());
				}
			},
			Node::HBox(b) | Node::VBox(b)	=> for child in &b.list { place_text_leaves(child, frame, x, y); },
			_								=> {},
		}
	}
}
