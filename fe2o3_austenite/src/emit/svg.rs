//! The SVG page writer.
//!
//! The geometry and paint are handed to `fe2o3_graphics`: [`Path::rect`] builds each box, a glyph's
//! outline arrives from `fe2o3_font` as a [`Path`] and is placed with [`Path::transform`], and the
//! crate's own [`write_path_data`] and [`presentation`] render the `d` attribute and the fill or
//! stroke. This module writes only the element tree around them -- the `<svg>`, `<rect>` and
//! `<path>` -- which `fe2o3_graphics::svg` deliberately leaves to the caller, because the document
//! shape above a `<path>` is the caller's format, not that crate's.
//!
//! Glyph reuse (SZ1, 12 Oct 2026): each distinct glyph outline is written once per page, as a `<path id>`
//! in the page's `<defs>`, and every occurrence is a `<use>` of it. A text page fell from 3.27 MB, every
//! outline written in full at every occurrence, to a fraction of that. The id is a hash of the outline's
//! own path data, so one id always names one shape: Daimond inlines several page SVGs into one DOM, where
//! a repeated id must still resolve to the same outline whichever page's copy the browser finds. The scope
//! is the page, not the document, because Daimond's delta caches and drops pages independently.

use crate::font::ShapedText;
use crate::ir::{
	DrawOp,
	Graphic,
	Sp,
};
use crate::memo::Memo;
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
		write_path_data_into,
	},
	transform::Transform,
};
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;
use oxedyne_fe2o3_text::base64;
use oxedyne_fe2o3_text::xml::write::escape as xml_escape;

use std::collections::BTreeMap;
use std::fmt::Write;

/// The glyph outlines a page draws, each once, keyed by [`glyph_id`]. A placed glyph is a `<use>` of its
/// entry; [`assemble`] writes the entries as the page's `<defs>`, in id order, so two renders of the same
/// content write the same bytes however the page was split for the emit memo.
#[derive(Clone, Debug, Default)]
pub struct GlyphDefs {
	defs:		BTreeMap<u64, String>,	// id's hash -> outline flipped to y down, as path data
	scratch:	String,					// the outline being placed, font frame, for its id
}

impl GlyphDefs {
	pub fn new() -> Self {
		Self::default()
	}

	/// Writes a `<use>` of `outline` -- font frame, y up, already at its point size -- with its origin at
	/// the page point (`x`, `y`), defining the outline first if this page has not yet drawn it.
	pub fn place(&mut self, out: &mut String, outline: &Path, x: f32, y: f32, colour: Rgba) -> Outcome<()> {
		// The outline is written into a buffer kept for the purpose, so placing a glyph allocates nothing
		// once the page has drawn it. The map keys on the id's hash, which orders the defs as the id would.
		self.scratch.clear();
		write_path_data_into(&mut self.scratch, outline);
		let key = glyph_key(&self.scratch);
		if !self.defs.contains_key(&key) {
			let flipped = res!(outline.transform(&Transform::scale(1.0, -1.0)));
			self.defs.insert(key, write_path_data(&flipped));
		}
		let _ = write!(out, "  <use href=\"#ag{:016x}\" x=\"{}\" y=\"{}\" ", key, x, y);
		out.push_str(&presentation(Some(colour), None));
		out.push_str("/>\n");
		Ok(())
	}

	/// Takes in another slice's outlines, as the furniture's join the cached body's.
	pub fn merge(&mut self, other: &GlyphDefs) {
		for (id, d) in &other.defs {
			if !self.defs.contains_key(id) {
				self.defs.insert(*id, d.clone());
			}
		}
	}

	pub fn len(&self) -> usize {
		self.defs.len()
	}

	pub fn is_empty(&self) -> bool {
		self.defs.is_empty()
	}

	fn write(&self, out: &mut String) {
		if self.defs.is_empty() {
			return;
		}
		out.push_str("<defs>\n");
		for (id, d) in &self.defs {
			let _ = write!(out, "  <path id=\"ag{:016x}\" d=\"", id);
			out.push_str(d);
			out.push_str("\"/>\n");
		}
		out.push_str("</defs>\n");
	}
}

/// The page-local id of a glyph outline, from the outline's own path data (font frame, y up): `ag` and
/// sixteen hex digits. Equal outlines share an id on every page and in every font, and a different outline
/// would need a 64-bit collision to borrow one.
pub fn glyph_id(font_d: &str) -> String {
	fmt!("ag{:016x}", glyph_key(font_d))
}

/// The hash a [`glyph_id`] writes in hex.
fn glyph_key(font_d: &str) -> u64 {
	let mut h = Fingerprinter::new();
	h.write(b"glyph");
	h.write_str(font_d);
	h.finish().fold()
}

/// Renders one page as a self-contained SVG document. This is the whole-frame path, unchanged in its
/// bytes: it renders every placed item -- body and furniture alike -- as one slice.
pub fn render_page(page: &Page) -> Outcome<String> {
	let mut ink		= String::new();
	let mut tsel	= TselLayer::new();
	let mut defs	= GlyphDefs::new();
	res!(render_slice(&page.frame.placed, &mut ink, &mut tsel, &mut defs));
	Ok(assemble(page, &defs, &ink, &tsel.finish()))
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
	let (body_ink, body_tsel, body_defs) = match memo.page_lookup(key) {
		Some(e)	=> (e.body_ink, e.body_tsel, e.body_defs),
		None	=> {
			let mut ink		= String::new();
			let mut tsel	= TselLayer::new();
			let mut defs	= GlyphDefs::new();
			res!(render_slice(body, &mut ink, &mut tsel, &mut defs));
			memo.page_store(key, ink.clone(), tsel.clone(), defs.clone());
			(ink, tsel, defs)
		},
	};

	// The furniture, drawn fresh, its selectable tspans continuing the body's interword spacing and its
	// outlines joining the body's, so a glyph both use is defined once.
	let mut furn_ink	= String::new();
	let mut tsel		= body_tsel;
	let mut defs		= body_defs;
	res!(render_slice(furn, &mut furn_ink, &mut tsel, &mut defs));

	let ink = fmt!("{}{}", body_ink, furn_ink);
	Ok(assemble(page, &defs, &ink, &tsel.finish()))
}

/// The stable identity of a whole page for the changed-only delta ([`crate::delta`]): the content hash of
/// its ENTIRE frame -- body and furniture (running head, folio) alike -- so two compiles yield the same id
/// for a page exactly when it renders to the same SVG bytes. Deliberately a superset of [`page_key`], which
/// hashes the body alone for the emit memo (whose furniture is always redrawn fresh); the delta needs the
/// folio in the key, since the consumer caches and reuses the whole page SVG by this id, and a page whose
/// only change is its printed folio renders differently and must be resent.
pub(crate) fn page_id(page: &Page) -> u64 {
	// Folded to 64 bits: the id crosses the wasm boundary as a `u64` (the consumer's cache key).
	page_key(page, &page.frame.placed).fold()
}

/// The content key of a page's body frame: its geometry and every body-placed item's position, size and
/// ink. Furniture is excluded (it hashes nothing here); the verso mirror shift is already baked into the
/// placed positions, so a page that changes parity hashes differently and misses, which is correct -- its
/// body sits at different coordinates. Theme is not in the key because it is baked into the shaped runs
/// (a run's glyph ids and colour already reflect it) and into the caller's global fingerprint besides.
fn page_key(page: &Page, body: &[Placed]) -> Fingerprint {
	let mut h = Fingerprinter::new();
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
			PlacedKind::Group(g)	=> {
				h.write_u8(4);
				for v in [g.transform.a, g.transform.b, g.transform.c, g.transform.d, g.transform.e, g.transform.f] {
					h.write(&v.to_le_bytes());
				}
				if let Some(c) = &g.clip {
					h.write_str(&write_path_data(c));
				}
			},
			PlacedKind::GroupEnd	=> h.write_u8(5),
		}
	}
	h.finish()
}

/// Folds a placed graphic into a page key: each op by its kind, its paint, and its geometry. A path is
/// hashed by the very `d` string the writer emits, so two paths hash alike exactly when they draw alike.
fn hash_graphic(g: &Graphic, h: &mut Fingerprinter) {
	for op in &g.ops {
		match op {
			DrawOp::Fill { path, colour } => {
				h.write_u8(0);
				h.write_str(&write_path_data(path));
				h.write(&colour.key());
			},
			DrawOp::Stroke { path, colour, width } => {
				h.write_u8(1);
				h.write_str(&write_path_data(path));
				h.write(&colour.key());
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

fn assemble(page: &Page, defs: &GlyphDefs, ink: &str, tspans: &str) -> String {
	let size = page.geom.media_box();
	assemble_sized(size.x.as_usize(), size.y.as_usize(), defs, ink, tspans)
}

/// Wraps the glyph outlines, the visible ink and the selectable tspans of a page `w` by `h` points in the
/// SVG document shell -- the `<svg>` viewport, the white backing rectangle, the `.tsel` style, the
/// `<defs>`, and the one page-wide `<text>` layer -- so every render path, memo, Pearl or neither, writes
/// the same bytes for the same content.
pub(crate) fn assemble_sized(w: usize, h: usize, defs: &GlyphDefs, ink: &str, tspans: &str) -> String {
	// Sized once, so a large page is not grown by doubling and left holding the slack.
	let mut out = String::with_capacity(ink.len() + tspans.len() + 64 * defs.len() + 512);
	out.push_str(&fmt!(
		"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
		w, h, w, h));
	out.push_str(&fmt!(
		"<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n", w, h));
	// `.tsel` (Typst.ts's own name for the same idea) is the selectable text layer's class: transparent,
	// so it draws nothing over the glyph outlines below, and pointer-events left at the SVG default
	// (not `none`) so a mouse drag still hits real text nodes rather than only outline paths.
	out.push_str("<style>.tsel { fill: transparent; }</style>\n");
	defs.write(&mut out);
	out.push_str(ink);
	if !tspans.is_empty() {
		out.push_str(&fmt!("  <text class=\"tsel\">{}</text>\n", tspans));
	}
	out.push_str("</svg>\n");
	out
}

/// Renders one slice of a page's placed items into two buffers: `ink` gets the visible glyph outlines,
/// rules and graphics; `tsel` gets the invisible selectable text layer, and its state threads across
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
	tsel:		&mut TselLayer,
	defs:		&mut GlyphDefs,
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
			res!(draw_text(ink, defs, p.x, p.y, p.dims.height, shaped));
			continue;
		}
		// A group opens an SVG group under its matrix and clip; its end closes it.
		if let PlacedKind::Group(g) = &p.kind {
			res!(open_group(ink, p, g));
			continue;
		}
		if let PlacedKind::GroupEnd = &p.kind {
			ink.push_str("  </g>\n");
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
			PlacedKind::Group(_) | PlacedKind::GroupEnd	=> continue,	// handled above
		};
		ink.push_str(&fmt!("  <path d=\"{}\" {}/>\n", d, attrs));
	}

	// Text under a transform is left out of the selectable layer, whose spans sit at untransformed baseline
	// positions; it stays visible as ink.
	let mut bent: Vec<bool> = Vec::new();
	for p in placed {
		match &p.kind {
			PlacedKind::Group(g) => {
				let outer = bent.last().copied().unwrap_or(false);
				bent.push(outer || g.transform != crate::ir::Transform::identity());
			},
			PlacedKind::GroupEnd => {
				bent.pop();
			},
			PlacedKind::Text(shaped) if !bent.last().copied().unwrap_or(false) => {
				run_text_layer(tsel, p.x, p.y, p.dims.height, shaped);
			},
			_ => (),
		}
	}
	Ok(())
}

/// Opens a group: `<g>` under the group's page matrix, clipped when the group clips. The clip path is given
/// in page coordinates before the matrix, which is the user space `clipPathUnits="userSpaceOnUse"` names
/// for an element carrying a `transform`; its id is a hash of what it draws, so equal clips share a name.
fn open_group(out: &mut String, p: &Placed, g: &crate::page::Group) -> Outcome<()> {
	let m = g.page_transform(p.x, p.y);
	let mut attrs = String::new();
	if m != crate::ir::Transform::identity() {
		attrs.push_str(&fmt!(" transform=\"matrix({} {} {} {} {} {})\"",
			num(m.a), num(m.b), num(m.c), num(m.d), num(m.e), num(m.f)));
	}
	if let Some(clip) = &g.clip {
		let t	= Transform::translate(p.x.to_pt() as f32, p.y.to_pt() as f32);
		let d	= write_path_data(&res!(clip.transform(&t)));
		let mut h = Fingerprinter::new();
		h.write_str(&d);
		let id = fmt!("clip{:016x}", h.finish().fold());
		out.push_str(&fmt!("  <clipPath id=\"{}\"><path d=\"{}\"/></clipPath>\n", id, d));
		attrs.push_str(&fmt!(" clip-path=\"url(#{})\"", id));
	}
	out.push_str(&fmt!("  <g{}>\n", attrs));
	Ok(())
}

/// A matrix entry as SVG writes it: fixed to four decimals, trailing zeros trimmed.
fn num(v: f64) -> String {
	let s = fmt!("{:.4}", v);
	let s = s.trim_end_matches('0').trim_end_matches('.');
	if s == "-0" { "0".to_string() } else { s.to_string() }
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
				out.push_str("  <path d=\"");
				write_path_data_into(out, &p);
				out.push_str("\" ");
				out.push_str(&presentation(Some(colour.to_rgba()), None));
				out.push_str("/>\n");
			},
			DrawOp::Stroke { path, colour, width } => {
				let pen	= res!(Stroke::new(*width));
				let p	= res!(path.transform(&t));
				out.push_str("  <path d=\"");
				write_path_data_into(out, &p);
				out.push_str("\" ");
				out.push_str(&presentation(None, Some((colour.to_rgba(), &pen))));
				out.push_str("/>\n");
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

/// Draws a placed run as glyph outlines, each a `<use>` of its entry in `defs`. `height` is the line's own HBox height -- the face
/// ascent for an ordinary line, or the cap height for a first line raised under the block-edge model
/// (see `linebreak::set_lines`), whose glyphs then carry a compensating negative shift -- so
/// `by + height` is the baseline either way. `bx`/`by` are the box's top-left.
fn draw_text(
	out:	&mut String,
	defs:	&mut GlyphDefs,
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
		// The outline is font-frame, y up; the page is y down. The definition is flipped in y and the use
		// moves it onto the baseline at the glyph's own offset. The run is shaped in points, so no scale
		// beyond the flip.
		res!(defs.place(out, &path, base_x + glyph.x, base_y - glyph.y, shaped.colour().to_rgba()));
	}
	Ok(())
}

/// Appends a placed run's invisible, selectable characters to the page's one `.tsel` layer, each
/// carrying the source text [`ShapedText::glyph_text`] maps its glyph to -- the very mapping the PDF
/// writer's `/ToUnicode` CMap uses, so the two extraction paths never disagree -- and sitting on that
/// glyph's own baseline position, `(base_x + glyph.x, base_y - glyph.y)`, where [`draw_text`] paints the
/// outline.
fn run_text_layer(tsel: &mut TselLayer, bx: Sp, by: Sp, height: Sp, shaped: &ShapedText) {
	let base_x	= bx.to_pt() as f32;
	let base_y	= (by + height).to_pt() as f32;
	let texts	= shaped.glyph_text();
	let glyphs	= shaped.run().glyphs.iter().zip(texts.iter()).map(|(g, t)| (g.x, g.y, t.as_str()));
	tsel.run(base_x, base_y, shaped.size(), glyphs);
}

/// A page's selectable text layer as it is written: the markup so far, whether a run has yet written
/// text, and the line its last span left open. Both the SVG arm and Pearl's `SvgSink` write through it,
/// so their pages agree byte for byte, and the emit memo keeps the body's state, so the furniture carries
/// on from it exactly as a single-pass render does.
///
/// A line -- spans on one baseline at one size -- is one `<tspan y font-size>`, and within it each
/// stretch of characters between spaces is one `<tspan x>` listing a position for every UTF-16 unit, on
/// its glyph's own offset, so selection and the browser's find still land on the outline. The layer has
/// no font of its own, so the browser's advances say nothing about where a glyph sits and every position
/// is written, rounded to 0.01 pt. A space is left bare between the spans: `<text>` collapses a run of
/// spaces, and a collapsed character takes no position from an `x` list, so a space inside a span would
/// push every later position one character along. Bare, it collapses exactly as the old
/// one-span-per-glyph layer's did, and the text a copy or a search reads is the same. The later
/// characters of a ligature are bare too, flowing on from its first as they did there.
#[derive(Clone, Debug, Default)]
pub struct TselLayer {
	buf:	String,
	seen:	bool,				// a run has written text, so the next one takes a leading space
	line:	Option<(f32, f32)>,	// the open line's baseline and size
}

impl TselLayer {
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends one run, `glyphs` being each glyph's offset from the run's baseline origin and the text it
	/// stands for. Every run but the first takes a leading space, standing in for the interword gap the
	/// line breaker never gives a glyph of its own; a run with no text writes nothing.
	pub fn run<'a, I>(&mut self, base_x: f32, base_y: f32, size: f32, glyphs: I)
		where I: Iterator<Item = (f32, f32, &'a str)>
	{
		let mut out		= String::new();
		let mut xs		= String::new();	// the open span's positions
		let mut txt		= String::new();	// the open span's characters
		let mut line	= self.line;
		let mut any		= false;
		let close = |out: &mut String, xs: &mut String, txt: &mut String| {
			if !xs.is_empty() {
				out.push_str(&fmt!("<tspan x=\"{}\">{}</tspan>", xs, xml_escape(txt)));
				xs.clear();
				txt.clear();
			}
		};
		for (gx, gy, text) in glyphs {
			// A later part of a ligature or decomposed mark has no text of its own; the earlier glyph at
			// the same cluster already carries the character.
			if text.is_empty() {
				continue;
			}
			any = true;
			let at	= (base_y - gy, size);
			let x	= num2(base_x + gx);
			for (k, ch) in text.chars().enumerate() {
				if matches!(ch, ' ' | '\t' | '\n' | '\r') || k > 0 {
					close(&mut out, &mut xs, &mut txt);
					out.push_str(&xml_escape(&ch.to_string()));
					continue;
				}
				if line != Some(at) {
					close(&mut out, &mut xs, &mut txt);
					if line.is_some() {
						out.push_str("</tspan>");
					}
					out.push_str(&fmt!("<tspan y=\"{}\" font-size=\"{}\">", num2(at.0), num2(at.1)));
					line = Some(at);
				}
				for _ in 0..ch.len_utf16() {
					if !xs.is_empty() {
						xs.push(' ');
					}
					xs.push_str(&x);
				}
				txt.push(ch);
			}
			// A browser that counts an astral character once, not as its two UTF-16 units, would read
			// every later position one along; a fresh span after it keeps that inside this glyph.
			if text.chars().any(|c| c.len_utf16() > 1) {
				close(&mut out, &mut xs, &mut txt);
			}
		}
		close(&mut out, &mut xs, &mut txt);
		if !any {
			return;
		}
		if self.seen {
			self.buf.push(' ');
		}
		self.buf.push_str(&out);
		self.seen = true;
		self.line = line;
	}

	/// The layer's markup, its open line closed.
	pub fn finish(&self) -> String {
		match self.line {
			Some(_)	=> fmt!("{}</tspan>", self.buf),
			None	=> self.buf.clone(),
		}
	}
}

/// A selectable-layer coordinate: two decimals, trailing zeros trimmed.
fn num2(v: f32) -> String {
	let s = fmt!("{:.2}", v);
	let s = s.trim_end_matches('0').trim_end_matches('.');
	if s == "-0" { "0".to_string() } else { s.to_string() }
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
		// The word is recoverable as `window.find`/copy-paste would see it: one positioned span, one `x`
		// per character.
		assert_eq!(tsel_text(&svg), "Oxegen", "the layer reads as the word, found: {}", svg);
		assert_eq!(svg.matches("<tspan x=").count(), 1, "a word is one span, found: {}", svg);
		let xs = res!(svg.split("<tspan x=\"").nth(1).and_then(|t| t.split('"').next())
			.ok_or_else(|| err!("no tspan x in {}", svg; Missing)));
		assert_eq!(xs.split(' ').count(), 6, "every character has its own position, found: {}", xs);
		// The visible outlines are unaffected: still one filled black path per inked glyph, nothing new
		// added to that part of the document.
		assert!(svg.contains("fill=\"#000000\""), "the outline glyphs still draw in black");
		Ok(())
	}

	/// The selectable layer's text is what the old one-span-per-glyph layer held, character for
	/// character, so a copy or a search reads the same: spaces and a ligature's later characters stand
	/// bare between spans, each span's `x` has one position per UTF-16 unit, an astral character closes its
	/// span, and a line is one `<tspan y font-size>` the next run carries on in.
	#[test]
	fn the_selectable_layer_keeps_its_text_and_one_position_per_character() -> Outcome<()> {
		let glyphs = [
			(0.0f32,	0.0f32,	"fi"),
			(5.0,		0.0,	""),
			(9.0,		0.0,	"x"),
			(12.0,		0.0,	" "),
			(15.0,		0.0,	"<a>"),
			(20.0,		0.0,	"\u{1D400}"),
			(30.0,		0.0,	"z"),
			(34.0,		2.0,	"q"),
		];
		let mut tsel = TselLayer::new();
		tsel.run(10.0, 100.0, 11.0, glyphs.iter().map(|(x, y, t)| (*x, *y, *t)));
		// A run of nothing but ligature tails carries no text and writes nothing, not even its space.
		tsel.run(0.0, 0.0, 11.0, [(0.0f32, 0.0f32, "")].into_iter());
		tsel.run(50.0, 98.0, 11.0, [(0.0f32, 0.0f32, "w")].into_iter());
		let buf = tsel.finish();
		let svg = fmt!("<text class=\"tsel\">{}</text>", buf);
		assert_eq!(tsel_text(&svg), "fix <a>\u{1D400}zq w", "the text is unchanged, found: {}", buf);
		assert_eq!(buf, [
			"<tspan y=\"100\" font-size=\"11\"><tspan x=\"10\">f</tspan>i<tspan x=\"19\">x</tspan> ",
			"<tspan x=\"25\">&lt;</tspan>a&gt;<tspan x=\"30 30\">\u{1D400}</tspan><tspan x=\"40\">z</tspan>",
			"</tspan><tspan y=\"98\" font-size=\"11\"><tspan x=\"44\">q</tspan> <tspan x=\"50\">w</tspan></tspan>",
		].concat(), "the layout");
		Ok(())
	}

	// The `.tsel` layer's text content as a browser reads it before collapsing spaces: tags dropped,
	// entities resolved.
	fn tsel_text(svg: &str) -> String {
		let body = svg.split("<text class=\"tsel\">").nth(1).unwrap_or("").split("</text>").next().unwrap_or("");
		let mut out = String::new();
		let mut tag = false;
		for c in body.chars() {
			match c {
				'<'			=> tag = true,
				'>'			=> tag = false,
				_ if !tag	=> out.push(c),
				_			=> (),
			}
		}
		out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
	}

	/// A glyph drawn three times on a page is defined once, in the page's `<defs>`, and used at each
	/// occurrence; no outline is written in place, and every use names the defined id.
	#[test]
	fn a_repeated_glyph_is_defined_once_and_used_at_each_occurrence() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let shaped	= res!(crate::font::ShapedText::new(fonts, Role::Body, Dir::Ltr, Sp::from_pt(11.0), "eee"));
		let dims	= shaped.dims();
		let mut frame = Frame::new();
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0), dims, PlacedKind::Text(shaped)));
		let svg = res!(render_page(&Page::new(1, geom, frame)));

		assert_eq!(svg.matches("<defs>").count(), 1, "one defs block, found: {}", svg);
		assert_eq!(svg.matches("<path id=\"ag").count(), 1, "the 'e' outline is defined once, found: {}", svg);
		assert_eq!(svg.matches("<use href=\"#ag").count(), 3, "each 'e' is a use, found: {}", svg);
		assert!(!svg.contains("<path d="), "no outline is written in place, found: {}", svg);
		let start	= res!(svg.find("<path id=\"").ok_or_else(|| err!("No glyph definition."; Test, Missing)));
		let id		= &svg[start + 10..start + 28];
		assert!(svg.contains(&fmt!("href=\"#{}\"", id)), "the uses name the defined id {}", id);
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
