//! A minimal PDF writer: pages of filled and stroked outline paths.
//!
//! The typesetter above this crate turns every glyph into a filled outline [`Path`] -- the Pearl
//! principle, that a font is a set of outlines and not a program -- so a page is a list of filled
//! paths and nothing else. This writer leans on that entirely: it embeds no font, no CMap and no font
//! program, and writes each glyph as ordinary path-construction and fill operators in a content
//! stream. That is the whole simplification, and it is why the file this module produces is small and
//! self-contained.
//!
//! The geometry and colour are the crate's own [`Path`], [`Pt`], [`Seg`] and [`Rgba`]; nothing here
//! defines a parallel type. A quadratic segment is elevated to a cubic on the way out, since PDF has
//! no quadratic operator, and the whole page is flipped in y so the engine's top-left, y-down frame
//! meets PDF's bottom-left, y-up one.
//!
//! The bytes are deterministic: no dates are written, the `/ID` is derived from the file's own
//! content rather than the clock, and no producer string leaks a version or a build. The same page
//! list yields the same bytes on every run, which is what a content-addressed pipeline needs.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::colour::Rgba;
use crate::path::{
	Path,
	Pt,
	Seg,
};
use crate::transform::Transform;

use std::collections::HashMap;
use std::io::Write;

use oxedyne_fe2o3_core::prelude::*;

// The FNV-1a parameters the file's deterministic `/ID` is folded with: two bases give the two hash
// halves, and the one prime multiplies each step. Kept as constants so the streaming writer can fold
// the body as it goes rather than hashing a whole buffer at the end.
const FNV_BASIS_A:	u64 = 0xcbf2_9ce4_8422_2325;
const FNV_BASIS_B:	u64 = 0x8422_2325_cbf2_9ce4;
const FNV_PRIME:	u64 = 0x0000_0100_0000_01b3;

// A glyph outline is stored once, in the font frame, at this many units per em, and shown at any point
// size by the Type-3 `/FontMatrix` and text-showing size. A single canonical size means the same glyph
// drawn at two sizes shares one stored outline.
const GLYPH_UPM:	f32 = 1000.0;

/// A 64-bit FNV-1a over bytes: the content key a glyph outline is deduplicated by. Two glyphs whose
/// path operators serialise identically share one Type-3 `CharProc`, whatever face or size drew them,
/// which is what a content key buys over a `(face, id)` pair that means nothing across font chains.
fn fnv1a(bytes: &[u8]) -> u64 {
	let mut h = FNV_BASIS_A;
	for &b in bytes {
		h ^= b as u64;
		h = h.wrapping_mul(FNV_PRIME);
	}
	h
}

/// One drawn shape: a path and how it is painted, or a raster image placed in a rectangle. Fill and
/// stroke are the two the typesetter needs for vector ink -- glyphs and rules fill, a held-open
/// reservation strokes; `Image` embeds a decoded raster (a figure's photograph or diagram) as an image
/// XObject, its samples straight RGB with an optional grey soft mask for translucency.
#[derive(Clone, Debug)]
pub enum Draw {
	Fill {
		path:	Path,
		colour:	Rgba,
	},
	Stroke {
		path:	Path,
		colour:	Rgba,
		width:	f64,	// pen width, in points
	},
	Image {
		rgb:	Vec<u8>,			// packed RGB, iw*ih*3, row-major, top row first
		alpha:	Option<Vec<u8>>,	// packed grey soft mask, iw*ih, present only when a pixel is translucent
		iw:		usize,				// image width in samples
		ih:		usize,				// image height in samples
		x:		f64,				// placement rectangle, engine frame (top-left, y down), points
		y:		f64,
		w:		f64,
		h:		f64,
	},
	Glyph {
		outline:	Path,		// the outline in the font frame, y up, at GLYPH_UPM units per em
		x:			f32,		// pen x in the engine frame (top-left, y down), points
		y:			f32,		// pen y (the baseline) in the engine frame, points
		size:		f32,		// point size the glyph is shown at
		adv:		f32,		// advance in points at that size, for the font's /Widths
		colour:		Rgba,
		text:		String,		// source scalar(s) this glyph stands for, for /ToUnicode; may be empty
	},
}

impl Draw {

	/// The paint colour of a vector draw, opaque for an image (whose translucency rides its own soft
	/// mask, not the page's alpha graphics state).
	fn colour(&self) -> Rgba {
		match self {
			Draw::Fill { colour, .. }	=> *colour,
			Draw::Stroke { colour, .. }	=> *colour,
			Draw::Glyph { colour, .. }	=> *colour,
			Draw::Image { .. }			=> Rgba::new(0, 0, 0, 255),
		}
	}
}

/// A clickable link over a rectangle of the page: a `/Link` annotation whose action opens a URI. The
/// rectangle is in the engine's page frame -- top-left origin, y down, in points -- exactly as a draw's
/// coordinates are, so a caller hands over the same rectangle it placed the ink in and [`PdfStream`]
/// flips it into PDF's y-up frame when it writes the annotation. The border is drawn away, so a linked
/// image reads as the picture alone.
#[derive(Clone, Debug)]
pub struct LinkAnnot {
	pub x0:		f64,	// left, engine frame (points)
	pub y0:		f64,	// top
	pub x1:		f64,	// right
	pub y1:		f64,	// bottom
	pub uri:	String,	// the destination the link opens
}

/// One page: its size in points, the shapes drawn on it back to front, and any clickable link
/// annotations over it.
///
/// The coordinates in the paths are the engine's page frame -- top-left origin, y increasing
/// downwards -- exactly as the SVG writer receives them. [`PdfWriter`] applies the flip to PDF's
/// y-up frame itself, so a caller hands the same paths to either writer.
#[derive(Clone, Debug)]
pub struct PdfPage {
	pub width:	f64,	// media box width, in points
	pub height:	f64,	// media box height, in points
	pub draws:	Vec<Draw>,
	pub annots:	Vec<LinkAnnot>,	// clickable link rectangles; a page with none writes no /Annots
}

impl PdfPage {

	pub fn new(width: f64, height: f64) -> Self {
		Self { width, height, draws: Vec::new(), annots: Vec::new() }
	}

	/// Adds a clickable link over the rectangle at top-left `(x, y)`, `w` wide and `h` tall in the
	/// engine's y-down point frame -- the same frame [`image`](Self::image) places a raster in -- opening
	/// `uri`. The page gains an `/Annots` entry; a page with no link stays byte-identical to before.
	pub fn link(&mut self, x: f64, y: f64, w: f64, h: f64, uri: String) {
		self.annots.push(LinkAnnot { x0: x, y0: y, x1: x + w, y1: y + h, uri });
	}

	pub fn fill(&mut self, path: Path, colour: Rgba) {
		self.draws.push(Draw::Fill { path, colour });
	}

	pub fn stroke(&mut self, path: Path, colour: Rgba, width: f64) {
		self.draws.push(Draw::Stroke { path, colour, width });
	}

	/// Places a decoded raster in the rectangle at top-left `(x, y)`, `w` wide and `h` tall, in the
	/// engine's y-down point frame. `rgb` is `iw * ih * 3` straight-RGB samples, top row first; `alpha`,
	/// when given, is the matching `iw * ih` grey soft mask that carries any translucency. The image is
	/// scaled to fill the rectangle, so the caller sizes the rectangle to the image's aspect if it wants
	/// no distortion.
	#[allow(clippy::too_many_arguments)]
	pub fn image(
		&mut self,
		rgb:	Vec<u8>,
		alpha:	Option<Vec<u8>>,
		iw:		usize,
		ih:		usize,
		x:		f64,
		y:		f64,
		w:		f64,
		h:		f64,
	) {
		self.draws.push(Draw::Image { rgb, alpha, iw, ih, x, y, w, h });
	}

	/// Adds a glyph placed at pen `(x, y)` -- `x` the left, `y` the baseline, in the engine's top-left,
	/// y-down point frame -- shown at `size` points and painted `colour`. `outline` is the glyph in the
	/// font frame (y up, at [`GLYPH_UPM`] units per em); `adv` is its advance in points for the font's
	/// `/Widths`; `text` is the source scalar(s) it stands for, recorded in the font's `/ToUnicode` so the
	/// glyph is text-extractable, or empty when none is known. The writer stores each distinct outline
	/// once as a Type-3 `CharProc` and references it here, so a glyph drawn a thousand times costs its
	/// outline once.
	pub fn glyph(&mut self, outline: Path, x: f32, y: f32, size: f32, adv: f32, colour: Rgba, text: String) {
		self.draws.push(Draw::Glyph { outline, x, y, size, adv, colour, text });
	}
}

/// One entry in the document outline (the viewer's bookmark side panel): a title, the zero-based page
/// it jumps to, and its nesting depth. Depth zero is a top-level entry; a deeper entry nests under the
/// nearest preceding entry of a shallower depth, so a flat list in reading order builds the tree. The
/// destination is the top of the page fitted to the window, which every entry shares -- the outline
/// names pages, not positions within them.
#[derive(Clone, Debug)]
pub struct OutlineItem {
	pub title:	String,
	pub page:	usize,	// zero-based page index the entry jumps to
	pub level:	u8,		// nesting depth, zero at the top
}

/// The parent, sibling and child links one outline item needs, resolved from the flat level list.
/// Indices are into the item slice; `count` is the number of descendants, always shown open.
struct OutlineLinks {
	parent:	Option<usize>,
	prev:	Option<usize>,
	next:	Option<usize>,
	first:	Option<usize>,
	last:	Option<usize>,
	count:	usize,
}

/// Resolves the flat, reading-order outline list into a tree: each item's parent, siblings and
/// children, and the roots. A stack of open ancestors gives the nearest shallower item as parent, so
/// a level that skips a depth still nests sensibly.
fn build_outline_links(items: &[OutlineItem]) -> (Vec<OutlineLinks>, Vec<usize>) {
	let n = items.len();
	let mut children:	Vec<Vec<usize>>	= vec![Vec::new(); n];
	let mut parent:		Vec<Option<usize>> = vec![None; n];
	let mut roots:		Vec<usize>		= Vec::new();
	let mut stack:		Vec<usize>		= Vec::new();
	for i in 0..n {
		while let Some(&t) = stack.last() {
			if items[t].level >= items[i].level {
				stack.pop();
			} else {
				break;
			}
		}
		match stack.last() {
			Some(&p) => {
				parent[i] = Some(p);
				children[p].push(i);
			},
			None => roots.push(i),
		}
		stack.push(i);
	}

	// The descendant count of a pre-order list is the run of following items whose level stays deeper.
	let mut links = Vec::with_capacity(n);
	for i in 0..n {
		let siblings = match parent[i] {
			Some(p)	=> &children[p],
			None	=> &roots,
		};
		let at		= siblings.iter().position(|&j| j == i).unwrap_or(0);
		let prev	= if at > 0 { Some(siblings[at - 1]) } else { None };
		let next	= siblings.get(at + 1).copied();
		let first	= children[i].first().copied();
		let last	= children[i].last().copied();
		let mut count = 0usize;
		let mut j = i + 1;
		while j < n && items[j].level > items[i].level {
			count += 1;
			j += 1;
		}
		links.push(OutlineLinks { parent: parent[i], prev, next, first, last, count });
	}
	(links, roots)
}

/// A PDF text string for a title: a parenthesised literal with `(`, `)` and `\` escaped when the text
/// is printable ASCII, else a UTF-16BE hex string with a byte-order mark so any Unicode renders. Both
/// forms are deterministic, which the content-addressed file needs.
fn pdf_text_string(s: &str) -> String {
	if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
		let mut out = String::from("(");
		for c in s.chars() {
			match c {
				'('		=> out.push_str("\\("),
				')'		=> out.push_str("\\)"),
				'\\'	=> out.push_str("\\\\"),
				_		=> out.push(c),
			}
		}
		out.push(')');
		out
	} else {
		let mut out = String::from("<FEFF");
		for u in s.encode_utf16() {
			out.push_str(&fmt!("{:04X}", u));
		}
		out.push('>');
		out
	}
}

/// Accumulates pages and writes them out as one PDF file.
#[derive(Clone, Debug, Default)]
pub struct PdfWriter {
	pages:		Vec<PdfPage>,
	compress:	bool,
	outline:	Vec<OutlineItem>,
}

impl PdfWriter {

	pub fn new() -> Self {
		Self::default()
	}

	/// Compress each content stream with zlib and mark it `/FlateDecode`. Off by default: an
	/// uncompressed stream is trivially deterministic and easy to read while the writer is young.
	pub fn with_compression(mut self, on: bool) -> Self {
		self.compress = on;
		self
	}

	pub fn add_page(&mut self, page: PdfPage) {
		self.pages.push(page);
	}

	/// Sets the document outline (the viewer's bookmark side panel), replacing any earlier one. An empty
	/// list leaves the file with no outline, byte for byte as before the feature existed.
	pub fn set_outline(&mut self, outline: Vec<OutlineItem>) {
		self.outline = outline;
	}

	/// Renders the whole document to PDF bytes.
	///
	/// A convenience over [`PdfStream`]: it streams the accumulated pages into an in-memory buffer and
	/// returns it. The bytes are exactly those [`PdfStream`] writes page by page, so a caller that
	/// cannot hold the whole document keeps the identical file by streaming to a file handle instead.
	pub fn to_bytes(&self) -> Outcome<Vec<u8>> {
		let mut stream = res!(PdfStream::new_with_outline(
			Vec::new(), self.pages.len(), self.compress, self.outline.clone()));
		for page in &self.pages {
			res!(stream.page(page));
		}
		Ok(res!(stream.finish()))
	}
}

/// A PDF writer that emits one page at a time to any [`Write`] sink, holding no more than the page in
/// hand. Where [`PdfWriter`] accumulates every page's outline paths and then serialises them into one
/// buffer -- three live copies of the whole document at the peak -- this writes each page's objects
/// the moment it is handed over and lets the caller drop the page, so a book of any length costs one
/// page of memory. The bytes are identical to [`PdfWriter::to_bytes`]: the same object numbering, the
/// same body order, the same content-derived `/ID`.
///
/// The page count is fixed at construction because the page-tree object -- written first, before any
/// page -- names every page object and their count. The deterministic `/ID` is a hash of the body,
/// folded here as each byte is written rather than over a finished buffer, so no buffer is needed.
///
/// Text is written as Type-3 fonts: each distinct glyph outline is stored once as a `CharProc` and
/// shown per occurrence by a one-byte code, rather than its outline written inline every time. The
/// glyph store and the fonts are filled as pages are written -- in page order, on the writer's thread,
/// so a code is assigned once and deterministically -- and the font objects themselves are written in
/// [`finish`](Self::finish) after the last page, on numbers reserved as each font is first needed.
pub struct PdfStream<W: Write> {
	out:		W,
	compress:	bool,
	n:			usize,			// the fixed page count, named by the page tree
	offsets:	Vec<usize>,		// one-based object byte offsets; [0] is the free object
	pos:		usize,			// bytes of body written so far, the next object's offset
	added:		usize,			// pages handed over so far
	next_extra:	usize,			// next free object number past the fixed block and the outline, for image XObjects
	hash_a:		u64,			// running FNV-1a of the body, first `/ID` half
	hash_b:		u64,			// running FNV-1a of the body, second half
	outline:	Vec<OutlineItem>,	// document outline entries, empty for none
	outline_root:	usize,		// object number of the /Outlines dict, zero when there is no outline
	glyph_slots:	HashMap<u64, (usize, u8)>,	// outline content key -> (font index, code)
	fonts:		Vec<Type3Font>,	// one Type-3 font per 256 distinct glyphs, in assignment order
}

/// One Type-3 font: up to 256 distinct glyphs, and the object number reserved for its font dictionary
/// when the font was first needed. The `CharProc`, `/ToUnicode` and dictionary objects are written in
/// [`PdfStream::finish`].
struct Type3Font {
	obj:		usize,			// reserved object number of the font dictionary
	glyphs:		Vec<GlyphEntry>,	// one per code, indexed by code (0..len)
}

/// One stored glyph: its `CharProc` path operators, its advance and bounding box in the glyph frame
/// (at [`GLYPH_UPM`] units per em), and the source scalar(s) it stands for.
struct GlyphEntry {
	ops:		String,			// path-construction operators of the outline
	wx:			i64,			// advance width, glyph units
	bbox:		(f32, f32, f32, f32),	// llx, lly, urx, ury, glyph units
	text:		String,			// source scalar(s), for /ToUnicode; empty when unknown
}

impl<W: Write> PdfStream<W> {
	/// Opens a stream for a document of exactly `n` pages, writing the header, catalogue and page tree
	/// at once. `compress` zlib-compresses each content stream, matching [`PdfWriter::with_compression`].
	pub fn new(out: W, n: usize, compress: bool) -> Outcome<Self> {
		Self::new_with_outline(out, n, compress, Vec::new())
	}

	/// As [`new`](Self::new), but the file also carries a document outline (the viewer's bookmark side
	/// panel). Each entry's page must be one of the `n` promised, since its destination names that page
	/// object. An empty outline yields a file byte-identical to [`new`]'s.
	pub fn new_with_outline(out: W, n: usize, compress: bool, outline: Vec<OutlineItem>) -> Outcome<Self> {
		for it in &outline {
			if it.page >= n {
				return Err(err!(
					"An outline entry points at page {} (zero-based), but the document has only {} \
					page(s); its destination could not be named.", it.page, n; Input, Invalid, Range));
			}
		}
		let obj_count		= 2 + 2 * n;
		let has_outline		= !outline.is_empty();
		// The outline dict and one object per entry sit directly after the fixed page/content block; any
		// image XObject is numbered past them. With no outline the numbering is exactly the original.
		let outline_root	= if has_outline { obj_count + 1 } else { 0 };
		let next_extra		= if has_outline {
			outline_root + 1 + outline.len()
		} else {
			obj_count + 1
		};
		let mut s = Self {
			out,
			compress,
			n,
			offsets:	vec![0; obj_count + 1],
			pos:		0,
			added:		0,
			next_extra,
			hash_a:		FNV_BASIS_A,
			hash_b:		FNV_BASIS_B,
			outline,
			outline_root,
			glyph_slots:	HashMap::new(),
			fonts:		Vec::new(),
		};

		res!(s.body(b"%PDF-1.7\n"));
		// A comment of high bytes tells a naive tool the file is binary, so it is not mangled in
		// transit. Four bytes above 127, as the specification suggests.
		res!(s.body(b"%\xE2\xE3\xCF\xD3\n"));

		// The catalogue. When the file carries an outline the catalogue names it and asks the viewer to
		// open the bookmark panel; with none it is byte for byte the original.
		s.offsets[1] = s.pos;
		if has_outline {
			let cat = fmt!(
				"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines {} 0 R /PageMode /UseOutlines >>\nendobj\n",
				outline_root);
			res!(s.body(cat.as_bytes()));
		} else {
			res!(s.body(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"));
		}

		// The page tree, naming every page object up front, which is why `n` is fixed here.
		s.offsets[2] = s.pos;
		let mut kids = String::new();
		for i in 0..n {
			if i > 0 {
				kids.push(' ');
			}
			kids.push_str(&fmt!("{} 0 R", 3 + 2 * i));
		}
		let tree = fmt!("2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n", kids, n);
		res!(s.body(tree.as_bytes()));

		Ok(s)
	}

	/// Writes the next page -- its page object and its content stream -- then advances. The page must
	/// be the next of the `n` promised at construction; an extra page is a mismatch the file could not
	/// name, so it is refused rather than written past the page tree.
	///
	/// The content stream is built here, on the writer's thread and in page order, so a glyph's Type-3
	/// code and its font's object number are assigned once and deterministically -- two runs of the same
	/// document yield the same bytes. The glyph outlines the content references are stored in the writer's
	/// font tables and written in [`finish`](Self::finish).
	pub fn page(&mut self, page: &PdfPage) -> Outcome<()> {
		if self.added >= self.n {
			return Err(err!(
				"A PdfStream opened for {} page(s) was handed a further page; the page tree cannot \
				name it.", self.n; Input, Invalid, Excessive));
		}
		let i			= self.added;
		let page_obj	= 3 + 2 * i;
		let content_obj	= 4 + 2 * i;

		// Assign object numbers for every image on the page -- one for the image itself, and one more for
		// its soft mask when it carries translucency -- so the page's resource dictionary can name them
		// before the streams are written. The numbers run past the fixed page/content block, growing the
		// object count a no-image document never touches.
		let mut img_objs: Vec<(usize, Option<usize>)> = Vec::new();
		for d in &page.draws {
			if let Draw::Image { alpha, .. } = d {
				let image_obj = self.next_extra;
				self.next_extra += 1;
				let smask_obj = if alpha.is_some() {
					let m = self.next_extra;
					self.next_extra += 1;
					Some(m)
				} else {
					None
				};
				img_objs.push((image_obj, smask_obj));
			}
		}

		// The link annotations, if any, take one further object -- the `/Annots` array -- numbered after the
		// images. A page with no link claims no number and writes no `/Annots`, so its bytes are unchanged.
		let annots_obj = if page.annots.is_empty() {
			None
		} else {
			let a = self.next_extra;
			self.next_extra += 1;
			Some(a)
		};

		// Build the content stream, which assigns each new glyph a Type-3 code and reserves a font-dictionary
		// object number the first time a font is needed. The used fonts come back so the page's resources can
		// name them by forward reference, exactly as the page tree forward-references its pages.
		let (raw, page_font_idxs) = res!(self.build_content(page));
		let page_fonts: Vec<(usize, usize)> = page_font_idxs.iter()
			.map(|&k| (k, self.fonts[k].obj))
			.collect();

		// The content stream is now serialised, so its `/Length` is known before the object that wraps it;
		// only the optional compression is left to do here.
		let bytes = if self.compress {
			res!(deflate(&raw))
		} else {
			raw
		};

		self.offsets[page_obj] = self.pos;
		let annots = match annots_obj {
			Some(a)	=> fmt!(" /Annots {} 0 R", a),
			None	=> String::new(),
		};
		let head = fmt!(
			"{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] {} \
				/Contents {} 0 R{} >>\nendobj\n",
			page_obj, numf(page.width), numf(page.height),
			resources(page, &img_objs, &page_fonts), content_obj, annots);
		res!(self.body(head.as_bytes()));

		self.offsets[content_obj] = self.pos;
		let filter = if self.compress { " /Filter /FlateDecode" } else { "" };
		let open = fmt!(
			"{} 0 obj\n<< /Length {}{} >>\nstream\n", content_obj, bytes.len(), filter);
		res!(self.body(open.as_bytes()));
		res!(self.body(&bytes));
		res!(self.body(b"\nendstream\nendobj\n"));

		// The image XObjects, in the order their numbers were assigned. The soft mask, when present, is
		// written straight after the image object that references it.
		let mut idx = 0;
		for d in &page.draws {
			if let Draw::Image { rgb, alpha, iw, ih, .. } = d {
				let (image_obj, smask_obj) = img_objs[idx];
				idx += 1;
				res!(self.write_image(image_obj, rgb, *iw, *ih, smask_obj));
				if let (Some(m), Some(a)) = (smask_obj, alpha) {
					res!(self.write_smask(m, a, *iw, *ih));
				}
			}
		}

		// The `/Annots` array, on the number reserved above: one `/Link` annotation per link, each with its
		// rectangle flipped into PDF's y-up frame and a URI action. Written after the images so the extra
		// offsets are recorded in increasing number order.
		if let Some(a) = annots_obj {
			res!(self.write_annots(a, page));
		}

		self.added += 1;
		Ok(())
	}

	/// Builds one page's content stream: the flip, then every shape's paint. A vector fill or stroke is
	/// written inline as before; a glyph is shown by a Type-3 text operator against a code assigned here.
	/// Returns the serialised bytes and the distinct font indices the page used, so the caller can name
	/// them in the page's `/Resources`.
	///
	/// Glyph codes are assigned in draw order, on this (the writer's) thread and in page order across the
	/// document, so the assignment is a deterministic function of the page sequence. Consecutive glyphs of
	/// one font, colour and size share a single `BT ... ET` text object; every glyph carries its own text
	/// matrix, so the font's own advances never move the pen and a per-glyph offset is exact.
	fn build_content(&mut self, page: &PdfPage) -> Outcome<(Vec<u8>, Vec<usize>)> {
		let mut s = String::new();
		s.push_str(&fmt!("1 0 0 -1 0 {} cm\n", numf(page.height)));

		let translucent = page.draws.iter().any(|d| d.colour().a != 255);
		let mut cur_alpha: Option<u8> = None;
		let mut img_k = 0;	// the image index, naming each `/Im{k}` XObject in draw order
		let mut used_fonts: Vec<usize> = Vec::new();

		// The open text object, if any: the font index, colour and size its `BT` set. A glyph reuses it when
		// all three match, else the block is closed and a fresh one opened.
		let mut open: Option<(usize, Rgba, f32)> = None;

		for d in &page.draws {
			// A non-glyph draw ends any run of glyphs first, so the text object is well formed.
			if !matches!(d, Draw::Glyph { .. }) && open.is_some() {
				s.push_str("ET\n");
				open = None;
			}
			match d {
				Draw::Image { x, y, w, h, .. } => {
					// The page CTM already flips y into the engine's top-left frame. An image's sample space
					// paints the unit square with its top row at the square's top, so mapping it into the
					// rectangle at top-left (x, y) needs `w 0 0 -h x (y+h)`: the negative height and the raised
					// origin put the first row at y and the last at y+h. Bracketed in q/Q so it disturbs nothing
					// after it.
					s.push_str("q\n");
					s.push_str(&fmt!("{} 0 0 {} {} {} cm\n", numf(*w), numf(-*h), numf(*x), numf(*y + *h)));
					s.push_str(&fmt!("/Im{} Do\n", img_k));
					s.push_str("Q\n");
					img_k += 1;
				},
				Draw::Fill { path, colour } => {
					if translucent {
						set_alpha(&mut s, &mut cur_alpha, colour.a);
					}
					s.push_str(&fmt!("{} {} {} rg\n",
						chan(colour.r), chan(colour.g), chan(colour.b)));
					path_ops(&mut s, path);
					// Non-zero winding, to match the SVG writer, whose fill-rule defaults to nonzero.
					s.push_str("f\n");
				},
				Draw::Stroke { path, colour, width } => {
					if translucent {
						set_alpha(&mut s, &mut cur_alpha, colour.a);
					}
					s.push_str(&fmt!("{} {} {} RG\n",
						chan(colour.r), chan(colour.g), chan(colour.b)));
					s.push_str(&fmt!("{} w\n", numf(*width)));
					path_ops(&mut s, path);
					s.push_str("S\n");
				},
				Draw::Glyph { outline, x, y, size, adv, colour, text } => {
					let (font_idx, code) = self.assign_glyph(outline, *adv, *size, text);
					if !used_fonts.contains(&font_idx) {
						used_fonts.push(font_idx);
					}
					// Open a fresh text object when the font, colour or size changes.
					if open != Some((font_idx, *colour, *size)) {
						if open.is_some() {
							s.push_str("ET\n");
						}
						if translucent {
							set_alpha(&mut s, &mut cur_alpha, colour.a);
						}
						// Under `d1` a Type-3 glyph paints with the text state's fill colour, set here once.
						s.push_str(&fmt!("{} {} {} rg\n",
							chan(colour.r), chan(colour.g), chan(colour.b)));
						s.push_str(&fmt!("BT\n/F{} {} Tf\n", font_idx, numf32(*size)));
						open = Some((font_idx, *colour, *size));
					}
					// The text matrix places the glyph and flips it back to y up: the page CTM flips the whole
					// page in y, and this `[1 0 0 -1 x y]` flips the text within it, so the glyph reads upright.
					// A per-glyph matrix means the font's advance never moves the pen -- the offset is exact.
					s.push_str(&fmt!("1 0 0 -1 {} {} Tm\n", numf32(*x), numf32(*y)));
					s.push_str(&fmt!("<{:02x}> Tj\n", code));
				},
			}
		}
		if open.is_some() {
			s.push_str("ET\n");
		}
		Ok((s.into_bytes(), used_fonts))
	}

	/// Assigns a glyph its Type-3 font index and code, storing the outline once. The key is the content of
	/// the outline's path operators, so the same shape drawn by any face or size shares one `CharProc`. A
	/// new glyph takes the next code; each font holds 256 codes, and crossing that boundary reserves a
	/// fresh font-dictionary object number from the extra-object counter.
	fn assign_glyph(&mut self, outline: &Path, adv: f32, size: f32, text: &str) -> (usize, u8) {
		let mut ops = String::new();
		path_ops(&mut ops, outline);
		let key = fnv1a(ops.as_bytes());
		if let Some(&slot) = self.glyph_slots.get(&key) {
			return slot;
		}
		let seq			= self.glyph_slots.len();
		let font_idx	= seq / 256;
		let code		= (seq % 256) as u8;
		if code == 0 {
			// A new font begins: reserve its dictionary object now so a page written before `finish` can name
			// it, and write the dictionary itself later.
			let obj = self.next_extra;
			self.next_extra += 1;
			self.fonts.push(Type3Font { obj, glyphs: Vec::new() });
		}
		let bbox = outline.bounds(&Transform::IDENTITY)
			.map(|b| (b.x0, b.y0, b.x1, b.y1))
			.unwrap_or((0.0, 0.0, 0.0, 0.0));
		// The advance in glyph units: points at the shown size scaled to the em. Positioning never uses it,
		// but the /Widths array and the `d1` operator declare it.
		let wx = if size != 0.0 { (adv / size * GLYPH_UPM).round() as i64 } else { 0 };
		self.fonts[font_idx].glyphs.push(GlyphEntry { ops, wx, bbox, text: text.to_string() });
		self.glyph_slots.insert(key, (font_idx, code));
		(font_idx, code)
	}

	/// Writes the Type-3 fonts: for each, its `CharProc` glyph streams, a `/ToUnicode` CMap, and the font
	/// dictionary naming both. Called from [`finish`](Self::finish) after the last page, on the object
	/// numbers reserved as each font and glyph was first met, so the cross-reference table stays exact.
	fn write_fonts(&mut self) -> Outcome<()> {
		let fonts = std::mem::take(&mut self.fonts);
		for font in &fonts {
			// Each glyph's outline is one CharProc stream. The `d1` operator declares the glyph a shape only,
			// so it paints with the text state's fill colour; the advance and bounding box are its operands.
			let mut cp_objs = Vec::with_capacity(font.glyphs.len());
			for g in &font.glyphs {
				let obj = self.next_extra;
				self.next_extra += 1;
				cp_objs.push(obj);
				let body = fmt!("{} 0 {} {} {} {} d1\n{}f\n",
					g.wx, numf32(g.bbox.0), numf32(g.bbox.1), numf32(g.bbox.2), numf32(g.bbox.3), g.ops);
				res!(self.write_stream(obj, body.as_bytes()));
			}

			// The /ToUnicode CMap, so a viewer extracts the source text rather than the Type-3 codes.
			let tu_obj = self.next_extra;
			self.next_extra += 1;
			let cmap = build_tounicode(&font.glyphs);
			res!(self.write_stream(tu_obj, cmap.as_bytes()));

			// The font dictionary, on the number reserved when the font began.
			self.set_extra_offset(font.obj);
			let last = font.glyphs.len().saturating_sub(1);
			// The font bounding box is the union of the glyph boxes. Start from the extremes so a glyph whose
			// box is wholly positive or wholly negative is not clipped to the origin.
			let mut bbox = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
			for g in &font.glyphs {
				bbox.0 = bbox.0.min(g.bbox.0);
				bbox.1 = bbox.1.min(g.bbox.1);
				bbox.2 = bbox.2.max(g.bbox.2);
				bbox.3 = bbox.3.max(g.bbox.3);
			}
			let mut char_procs = String::new();
			let mut diffs = String::from("0");
			let mut widths = String::new();
			for (code, obj) in cp_objs.iter().enumerate() {
				char_procs.push_str(&fmt!(" /g{} {} 0 R", code, obj));
				diffs.push_str(&fmt!(" /g{}", code));
				if code > 0 {
					widths.push(' ');
				}
				widths.push_str(&fmt!("{}", font.glyphs[code].wx));
			}
			let dict = fmt!(
				"{} 0 obj\n<< /Type /Font /Subtype /Type3 \
					/FontBBox [{} {} {} {}] /FontMatrix [0.001 0 0 0.001 0 0] \
					/CharProcs <<{} >> /Encoding << /Type /Encoding /Differences [{}] >> \
					/FirstChar 0 /LastChar {} /Widths [{}] /ToUnicode {} 0 R /Resources << >> >>\nendobj\n",
				font.obj,
				numf32(bbox.0), numf32(bbox.1), numf32(bbox.2), numf32(bbox.3),
				char_procs, diffs, last, widths, tu_obj);
			res!(self.body(dict.as_bytes()));
		}
		Ok(())
	}

	/// Writes one body stream object, compressing it and marking `/FlateDecode` when compression is on, and
	/// records its offset. The bytes are folded into the deterministic `/ID` like all body bytes.
	fn write_stream(&mut self, obj: usize, raw: &[u8]) -> Outcome<()> {
		let bytes = if self.compress {
			res!(deflate(raw))
		} else {
			raw.to_vec()
		};
		self.set_extra_offset(obj);
		let filter = if self.compress { " /Filter /FlateDecode" } else { "" };
		let head = fmt!("{} 0 obj\n<< /Length {}{} >>\nstream\n", obj, bytes.len(), filter);
		res!(self.body(head.as_bytes()));
		res!(self.body(&bytes));
		res!(self.body(b"\nendstream\nendobj\n"));
		Ok(())
	}

	/// Writes a page's `/Annots` array: one `/Link` annotation per [`LinkAnnot`], its rectangle flipped from
	/// the engine's top-left, y-down frame into PDF's bottom-left, y-up one, its border drawn away, and its
	/// action opening the URI. The dictionaries are inlined in the array object, so a page's links cost one
	/// object however many they are. Only called when the page carries at least one link.
	fn write_annots(&mut self, obj: usize, page: &PdfPage) -> Outcome<()> {
		self.set_extra_offset(obj);
		let mut s = fmt!("{} 0 obj\n[ ", obj);
		for a in &page.annots {
			// Flip y: a point at engine y lands at PDF `height - y`, so the top edge (smaller engine y)
			// becomes the upper PDF coordinate and the bottom edge the lower one.
			let lly = page.height - a.y1;
			let ury = page.height - a.y0;
			s.push_str(&fmt!(
				"<< /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [{} {} {} {}] \
					/A << /S /URI /URI {} >> >> ",
				numf(a.x0), numf(lly), numf(a.x1), numf(ury), pdf_text_string(&a.uri)));
		}
		s.push_str("]\nendobj\n");
		res!(self.body(s.as_bytes()));
		Ok(())
	}

	/// Writes an image XObject: a straight-RGB, eight-bit `/DeviceRGB` sample stream, always
	/// zlib-compressed so a photograph does not bloat the file, and pointing at its soft mask when one
	/// was assigned. The samples are folded into the deterministic `/ID` like all body bytes.
	fn write_image(
		&mut self,
		obj:	usize,
		rgb:	&[u8],
		iw:		usize,
		ih:		usize,
		smask:	Option<usize>,
	)
		-> Outcome<()>
	{
		let data = res!(deflate(rgb));
		self.set_extra_offset(obj);
		let mask = match smask {
			Some(m)	=> fmt!(" /SMask {} 0 R", m),
			None	=> String::new(),
		};
		let head = fmt!(
			"{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
				/BitsPerComponent 8{} /Filter /FlateDecode /Length {} >>\nstream\n",
			obj, iw, ih, mask, data.len());
		res!(self.body(head.as_bytes()));
		res!(self.body(&data));
		res!(self.body(b"\nendstream\nendobj\n"));
		Ok(())
	}

	/// Writes a soft-mask XObject: a single-channel `/DeviceGray` image the same size as its owner, its
	/// samples the straight alpha, zlib-compressed and folded into the `/ID` like any body bytes.
	fn write_smask(&mut self, obj: usize, alpha: &[u8], iw: usize, ih: usize) -> Outcome<()> {
		let data = res!(deflate(alpha));
		self.set_extra_offset(obj);
		let head = fmt!(
			"{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray \
				/BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
			obj, iw, ih, data.len());
		res!(self.body(head.as_bytes()));
		res!(self.body(&data));
		res!(self.body(b"\nendstream\nendobj\n"));
		Ok(())
	}

	/// Writes the document outline: the `/Outlines` dictionary, then one object per entry, each a title,
	/// its tree links, and a destination fitting the top of the page it names. The tree is built from the
	/// flat, reading-order entry list by nesting each entry under the nearest preceding shallower one; the
	/// counts are shown open, so a viewer opens the whole tree. The objects take the numbers reserved at
	/// construction, directly after the fixed page/content block.
	fn write_outline(&mut self) -> Outcome<()> {
		let root		= self.outline_root;
		let item_base	= root + 1;	// the first entry's object number
		// Taken out so the entry loop may borrow it while `self` is mutated for each object written.
		let outline		= std::mem::take(&mut self.outline);
		let (links, roots) = build_outline_links(&outline);

		// The `/Outlines` dict: its first and last top-level entries, and the total number of entries,
		// all open.
		self.set_extra_offset(root);
		let first_root	= roots.first().map(|&i| item_base + i).unwrap_or(0);
		let last_root	= roots.last().map(|&i| item_base + i).unwrap_or(0);
		let head = fmt!(
			"{} 0 obj\n<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>\nendobj\n",
			root, first_root, last_root, outline.len());
		res!(self.body(head.as_bytes()));

		// One object per entry, in reading order so its reserved number matches its slice index.
		for (i, item) in outline.iter().enumerate() {
			let obj		= item_base + i;
			let link	= &links[i];
			let parent	= match link.parent {
				Some(p)	=> item_base + p,
				None	=> root,
			};
			let page_obj = 3 + 2 * item.page;

			let mut dict = fmt!("{} 0 obj\n<< /Title {} /Parent {} 0 R",
				obj, pdf_text_string(&item.title), parent);
			if let Some(p) = link.prev {
				dict.push_str(&fmt!(" /Prev {} 0 R", item_base + p));
			}
			if let Some(nx) = link.next {
				dict.push_str(&fmt!(" /Next {} 0 R", item_base + nx));
			}
			if let (Some(f), Some(l)) = (link.first, link.last) {
				dict.push_str(&fmt!(" /First {} 0 R /Last {} 0 R /Count {}",
					item_base + f, item_base + l, link.count));
			}
			dict.push_str(&fmt!(" /Dest [{} 0 R /Fit] >>\nendobj\n", page_obj));
			self.set_extra_offset(obj);
			res!(self.body(dict.as_bytes()));
		}
		Ok(())
	}

	/// Records the byte offset of an extra object -- an image or soft mask numbered past the fixed
	/// page/content block -- growing the offset table to reach it. The extras are assigned and written in
	/// increasing number order, so the table grows one slot at a time and stays indexed by object number.
	fn set_extra_offset(&mut self, obj: usize) {
		while self.offsets.len() <= obj {
			self.offsets.push(0);
		}
		self.offsets[obj] = self.pos;
	}

	/// Closes the file: writes the cross-reference table and the trailer, flushes, and returns the sink.
	/// The `/ID` is the body hash folded as the body was written, so the file matches
	/// [`PdfWriter::to_bytes`] to the byte.
	pub fn finish(mut self) -> Outcome<W> {
		// The outline objects -- the `/Outlines` dict and one object per entry -- are written after the
		// pages, on the numbers reserved for them at construction. A document with no outline writes none.
		if !self.outline.is_empty() {
			res!(self.write_outline());
		}

		// The Type-3 fonts -- each glyph's CharProc, a /ToUnicode CMap, and the font dictionary -- are
		// written last, taking further object numbers past everything the pages reserved. A document with no
		// text writes none, so its numbering is exactly the original.
		if !self.fonts.is_empty() {
			res!(self.write_fonts());
		}

		// The fixed page/content block is `2 + 2n` objects; the outline, every image and soft mask, and the
		// Type-3 fonts took further numbers past it, so the highest object written is one below the next free
		// number. A document with no outline, image or text leaves `next_extra` at `2 + 2n + 1`, the original.
		let obj_count = self.next_extra - 1;

		// The identifier is derived from the body already written, never from the clock. The two halves
		// were folded byte by byte as the body streamed out.
		let id = fmt!("{:016x}{:016x}", self.hash_a, self.hash_b);

		// The cross-reference table and trailer sit after the body and are not part of the hash, so they
		// are written straight to the sink without folding. Every entry is exactly twenty bytes: a
		// ten-digit offset, a five-digit generation, the type, and a two-byte end.
		let xref_off = self.pos;
		let mut tail = String::new();
		tail.push_str(&fmt!("xref\n0 {}\n", obj_count + 1));
		tail.push_str("0000000000 65535 f\r\n");
		for k in 1..=obj_count {
			tail.push_str(&fmt!("{:010} 00000 n\r\n", self.offsets[k]));
		}
		tail.push_str(&fmt!(
			"trailer\n<< /Size {} /Root 1 0 R /ID [<{}> <{}>] >>\nstartxref\n{}\n%%EOF\n",
			obj_count + 1, id, id, xref_off));
		res!(self.out.write_all(tail.as_bytes()));
		res!(self.out.flush());
		Ok(self.out)
	}

	/// Writes a run of body bytes: out to the sink, on to the running offset, and folded into both
	/// halves of the deterministic `/ID`. Only the body passes through here; the xref and trailer, which
	/// the hash excludes, are written directly.
	fn body(&mut self, bytes: &[u8]) -> Outcome<()> {
		res!(self.out.write_all(bytes));
		self.pos += bytes.len();
		for &b in bytes {
			self.hash_a ^= b as u64;
			self.hash_a = self.hash_a.wrapping_mul(FNV_PRIME);
			self.hash_b ^= b as u64;
			self.hash_b = self.hash_b.wrapping_mul(FNV_PRIME);
		}
		Ok(())
	}
}

/// Builds the `/ToUnicode` CMap for a Type-3 font: one `bfchar` mapping per glyph whose source text is
/// known, the code as a one-byte hex string and the destination as UTF-16BE, so a viewer extracts the
/// real words rather than the font's private codes. A glyph with no known source (a decoration, or the
/// tail of a decomposed cluster) is left out. The `bfchar` entries are batched under a hundred, the CMap
/// operator's limit.
fn build_tounicode(glyphs: &[GlyphEntry]) -> String {
	let mut maps: Vec<(usize, String)> = Vec::new();
	for (code, g) in glyphs.iter().enumerate() {
		if g.text.is_empty() {
			continue;
		}
		let mut hex = String::new();
		for u in g.text.encode_utf16() {
			hex.push_str(&fmt!("{:04X}", u));
		}
		maps.push((code, hex));
	}

	let mut s = String::new();
	s.push_str("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n");
	s.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
	s.push_str("/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n");
	s.push_str("1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");
	for chunk in maps.chunks(100) {
		s.push_str(&fmt!("{} beginbfchar\n", chunk.len()));
		for (code, hex) in chunk {
			s.push_str(&fmt!("<{:02X}> <{}>\n", code, hex));
		}
		s.push_str("endbfchar\n");
	}
	s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
	s
}

/// Emits the path-construction operators for one path.
///
/// A move is `m`, a line `l`, a cubic `c`, a close `h`. A quadratic has no operator of its own and is
/// elevated to a cubic exactly: the cubic through the same ends whose two controls sit two-thirds of
/// the way from each end towards the quadratic's single control traces the identical curve. The
/// current point is tracked because the elevation needs the segment's start, and a close returns it
/// to where the contour began.
fn path_ops(s: &mut String, path: &Path) {
	let mut cur = Pt::default();
	let mut start = Pt::default();
	for seg in path.segs() {
		match *seg {
			Seg::MoveTo(p) => {
				s.push_str(&fmt!("{} {} m\n", numf32(p.x), numf32(p.y)));
				cur = p;
				start = p;
			},
			Seg::LineTo(p) => {
				s.push_str(&fmt!("{} {} l\n", numf32(p.x), numf32(p.y)));
				cur = p;
			},
			Seg::QuadTo(c, p) => {
				let two_thirds = 2.0 / 3.0;
				let c0 = Pt::new(
					cur.x + two_thirds * (c.x - cur.x),
					cur.y + two_thirds * (c.y - cur.y));
				let c1 = Pt::new(
					p.x + two_thirds * (c.x - p.x),
					p.y + two_thirds * (c.y - p.y));
				s.push_str(&fmt!("{} {} {} {} {} {} c\n",
					numf32(c0.x), numf32(c0.y), numf32(c1.x), numf32(c1.y),
					numf32(p.x), numf32(p.y)));
				cur = p;
			},
			Seg::CubicTo(c0, c1, p) => {
				s.push_str(&fmt!("{} {} {} {} {} {} c\n",
					numf32(c0.x), numf32(c0.y), numf32(c1.x), numf32(c1.y),
					numf32(p.x), numf32(p.y)));
				cur = p;
			},
			Seg::Close => {
				s.push_str("h\n");
				cur = start;
			},
		}
	}
}

/// The page's `/Resources`: an `/ExtGState` for each distinct alpha when a shape is translucent, an
/// `/XObject` dict naming each image `/Im{k}` by the object number assigned in [`PdfStream::page`], and a
/// `/Font` dict naming each Type-3 font `/F{k}` this page shows text from. An all-opaque page with no
/// image and no text carries an empty resource dictionary -- byte for byte the original.
fn resources(page: &PdfPage, img_objs: &[(usize, Option<usize>)], page_fonts: &[(usize, usize)]) -> String {
	let translucent = page.draws.iter().any(|d| d.colour().a != 255);

	// The image resource dict, `/Im{k}` in draw order to match the content stream's `Do` names.
	let xobjects = if img_objs.is_empty() {
		String::new()
	} else {
		let mut x = String::from(" /XObject << ");
		for (k, (obj, _)) in img_objs.iter().enumerate() {
			x.push_str(&fmt!("/Im{} {} 0 R ", k, obj));
		}
		x.push_str(">>");
		x
	};

	// The font resource dict, `/F{idx}` by the global font index the content stream names.
	let fonts = if page_fonts.is_empty() {
		String::new()
	} else {
		let mut f = String::from(" /Font << ");
		for (idx, obj) in page_fonts {
			f.push_str(&fmt!("/F{} {} 0 R ", idx, obj));
		}
		f.push_str(">>");
		f
	};

	if !translucent {
		if xobjects.is_empty() && fonts.is_empty() {
			return fmt!("/Resources << >>");
		}
		return fmt!("/Resources <<{}{} >>", xobjects, fonts);
	}

	let mut alphas: Vec<u8> = Vec::new();
	for d in &page.draws {
		let a = d.colour().a;
		if !alphas.contains(&a) {
			alphas.push(a);
		}
	}
	// The opaque state is always present, so a shape after a translucent one can return to full
	// opacity.
	if !alphas.contains(&255) {
		alphas.push(255);
	}
	alphas.sort_unstable();
	let mut gs = String::new();
	for a in &alphas {
		let v = chan(*a);
		gs.push_str(&fmt!("/GS{} << /ca {} /CA {} >> ", a, v, v));
	}
	fmt!("/Resources << /ExtGState << {}>>{}{} >>", gs, xobjects, fonts)
}

/// Sets the alpha graphics state, but only when it changes, naming each state `/GSn` by its alpha
/// byte to match [`resources`].
fn set_alpha(s: &mut String, cur: &mut Option<u8>, a: u8) {
	if *cur != Some(a) {
		s.push_str(&fmt!("/GS{} gs\n", a));
		*cur = Some(a);
	}
}

/// One 8-bit channel as a PDF colour component from 0 to 1.
fn chan(c: u8) -> String {
	dec6((c as f64) / 255.0)
}

/// A number to at most six decimal places, trailing zeros trimmed, for a colour or an alpha.
fn dec6(v: f64) -> String {
	let s = fmt!("{:.6}", v);
	let t = s.trim_end_matches('0').trim_end_matches('.');
	if t.is_empty() { "0".to_string() } else { t.to_string() }
}

/// A length or coordinate to three decimal places, trailing zeros and the point trimmed. Three places
/// is a thousandth of a point -- far below any renderer's resolution -- so nothing visible is lost,
/// while a coordinate like `841.8897705078125` shrinks to `841.89`: coordinate digits are about a
/// third of a content stream's bytes, and full f32 precision spends them on noise. The rounding is
/// deterministic (round-half-to-even), which the content-addressed file needs.
fn numf(v: f64) -> String {
	trim_dec(fmt!("{:.3}", v))
}

fn numf32(v: f32) -> String {
	trim_dec(fmt!("{:.3}", v))
}

/// Trims the trailing zeros and any bare point from a fixed-precision decimal, and folds a rounded
/// `-0` back to `0` so the bytes stay canonical.
fn trim_dec(s: String) -> String {
	if !s.contains('.') {
		return s;
	}
	let t = s.trim_end_matches('0').trim_end_matches('.');
	if t.is_empty() || t == "-0" { "0".to_string() } else { t.to_string() }
}

/// Zlib-compresses a content stream, for `/FlateDecode`.
///
/// The level is fixed so the output is byte-deterministic for a given input and a given `flate2`
/// version.
fn deflate(raw: &[u8]) -> Outcome<Vec<u8>> {
	use flate2::write::ZlibEncoder;
	use flate2::Compression;
	use std::io::Write;
	let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
	res!(enc.write_all(raw));
	Ok(res!(enc.finish()))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::path::{
		Bounds,
		PathBuilder,
	};

	#[test]
	fn test_a_quadratic_elevates_to_the_matching_cubic_00() -> Outcome<()> {
		// A quadratic with start (0,0), control (0,10), end (10,10) elevates to a cubic whose controls
		// sit two-thirds of the way from each end towards (0,10): (0, 6.6667) and (3.3333, 10).
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(0.0, 0.0));
		pb.quad_to(Pt::new(0.0, 10.0), Pt::new(10.0, 10.0));
		let p = res!(pb.finish());
		let mut s = String::new();
		path_ops(&mut s, &p);
		// The move, then one cubic ending at the quadratic's endpoint.
		assert!(s.contains("0 0 m"), "the move, found: {}", s);
		assert!(s.contains(" c\n"), "a cubic operator, found: {}", s);
		assert!(s.contains("10 10 c"), "the cubic ends where the quadratic did, found: {}", s);
		Ok(())
	}

	#[test]
	fn test_the_file_has_a_header_xref_and_trailer_01() -> Outcome<()> {
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(100.0, 200.0);
		page.fill(res!(Path::rect(Bounds::new(10.0, 10.0, 90.0, 90.0))), Rgba::BLACK);
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);
		assert!(text.starts_with("%PDF-1.7"), "the header");
		assert!(text.contains("/Type /Catalog"), "the catalogue");
		assert!(text.contains("/MediaBox [0 0 100 200]"), "the media box, found in: {}", text);
		assert!(text.contains("1 0 0 -1 0 200 cm"), "the y-flip for a 200pt page");
		assert!(text.contains("xref"), "the cross-reference table");
		assert!(text.contains("startxref"), "the startxref");
		assert!(text.trim_end().ends_with("%%EOF"), "the end-of-file marker");
		Ok(())
	}

	#[test]
	fn test_the_bytes_are_deterministic_02() -> Outcome<()> {
		// The same pages twice give the same bytes: no clock, no random source anywhere in the file.
		let build = || -> Outcome<Vec<u8>> {
			let mut w = PdfWriter::new();
			let mut page = PdfPage::new(100.0, 100.0);
			page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::new(10, 20, 30, 255));
			w.add_page(page);
			w.to_bytes()
		};
		assert_eq!(res!(build()), res!(build()));
		Ok(())
	}

	#[test]
	fn test_the_outline_nests_by_level_04() -> Outcome<()> {
		// Two top-level entries, the second with a child and a grandchild, then a third top-level entry.
		let items = vec![
			OutlineItem { title: "Title".into(),	page: 0, level: 0 },
			OutlineItem { title: "One".into(),		page: 1, level: 0 },
			OutlineItem { title: "One.a".into(),	page: 2, level: 1 },
			OutlineItem { title: "One.a.i".into(),	page: 3, level: 2 },
			OutlineItem { title: "Two".into(),		page: 4, level: 0 },
		];
		let (links, roots) = build_outline_links(&items);
		assert_eq!(roots, vec![0, 1, 4], "the three top-level entries are the roots");
		// The first entry has no parent, no previous sibling, and "One" as its next.
		assert_eq!(links[0].parent, None);
		assert_eq!(links[0].prev, None);
		assert_eq!(links[0].next, Some(1));
		// "One" parents "One.a", and its descendant count includes the grandchild.
		assert_eq!(links[1].first, Some(2));
		assert_eq!(links[1].last, Some(2));
		assert_eq!(links[1].count, 2, "child and grandchild are both descendants");
		assert_eq!(links[1].next, Some(4), "Two is the next top-level sibling");
		// "One.a" nests under "One" and parents the grandchild.
		assert_eq!(links[2].parent, Some(1));
		assert_eq!(links[2].first, Some(3));
		assert_eq!(links[3].parent, Some(2));
		assert_eq!(links[3].count, 0, "the leaf has no descendants");
		Ok(())
	}

	#[test]
	fn test_the_outline_reaches_the_file_and_dest_pages_05() -> Outcome<()> {
		// A three-page document with a two-entry outline: the catalogue names the outline and the entries
		// carry destinations to their page objects (3 + 2*page).
		let mut w = PdfWriter::new();
		for _ in 0..3 {
			let mut page = PdfPage::new(50.0, 50.0);
			page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::BLACK);
			w.add_page(page);
		}
		w.set_outline(vec![
			OutlineItem { title: "Title".into(),	page: 0, level: 0 },
			OutlineItem { title: "Body".into(),		page: 2, level: 0 },
		]);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);
		assert!(text.contains("/Outlines"), "the catalogue names an outline");
		assert!(text.contains("/Type /Outlines"), "the outline root dict is present");
		assert!(text.contains("/Title (Title)"), "the first entry's title");
		assert!(text.contains("/Title (Body)"), "the second entry's title");
		// Page 0 is object 3, page 2 is object 7.
		assert!(text.contains("/Dest [3 0 R /Fit]"), "the first entry jumps to page object 3");
		assert!(text.contains("/Dest [7 0 R /Fit]"), "the second entry jumps to page object 7");
		Ok(())
	}

	#[test]
	fn test_no_outline_leaves_the_catalogue_untouched_06() -> Outcome<()> {
		// A document with no outline set carries the bare catalogue, byte for byte as before the feature.
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(50.0, 50.0);
		page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::BLACK);
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);
		assert!(text.contains("<< /Type /Catalog /Pages 2 0 R >>"), "the bare catalogue");
		assert!(!text.contains("/Outlines"), "no outline object when none was set");
		Ok(())
	}

	#[test]
	fn test_a_non_ascii_title_encodes_as_utf16_07() -> Outcome<()> {
		// A title with an em dash cannot be a printable-ASCII literal, so it is a UTF-16BE hex string with
		// a byte-order mark.
		let s = pdf_text_string("A — B");
		assert!(s.starts_with("<FEFF"), "a UTF-16BE hex string, found: {}", s);
		assert!(s.ends_with('>'), "closed hex string");
		// A plain title stays a readable literal.
		assert_eq!(pdf_text_string("Contents"), "(Contents)");
		// Parentheses and backslashes in a literal are escaped.
		assert_eq!(pdf_text_string("a (b) \\ c"), "(a \\(b\\) \\\\ c)");
		Ok(())
	}

	#[test]
	fn test_a_link_annotation_is_emitted_over_a_rect_08() -> Outcome<()> {
		// A page 100 wide by 200 tall with a link over the rectangle at top-left (10, 20), 30 wide, 40 tall.
		// The page dict names an /Annots array, and the annotation is a /Link with a URI action whose /Rect
		// is the rectangle flipped into PDF's y-up frame: [10, 200-60, 40, 200-20] = [10 140 40 180].
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(100.0, 200.0);
		page.fill(res!(Path::rect(Bounds::new(10.0, 10.0, 90.0, 90.0))), Rgba::BLACK);
		page.link(10.0, 20.0, 30.0, 40.0, "https://need2know.ai/with-ai/doc".to_string());
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);
		assert!(text.contains("/Annots"), "the page dict names an annotation array, found: {}", text);
		assert!(text.contains("/Subtype /Link"), "a link annotation is written");
		assert!(text.contains("/S /URI /URI (https://need2know.ai/with-ai/doc)"), "the URI action, found: {}", text);
		assert!(text.contains("/Rect [10 140 40 180]"), "the rectangle flipped into PDF space, found: {}", text);
		assert!(text.contains("/Border [0 0 0]"), "the border is drawn away");
		Ok(())
	}

	#[test]
	fn test_no_link_leaves_the_page_bytes_identical_09() -> Outcome<()> {
		// A page with no link writes no /Annots and no annotation object -- byte for byte as before the
		// feature. The two builds of the same annot-free page also agree, so the field adds no nondeterminism.
		let build = || -> Outcome<Vec<u8>> {
			let mut w = PdfWriter::new();
			let mut page = PdfPage::new(100.0, 100.0);
			page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::new(10, 20, 30, 255));
			w.add_page(page);
			w.to_bytes()
		};
		let bytes = res!(build());
		let text = String::from_utf8_lossy(&bytes);
		assert!(!text.contains("/Annots"), "no annotation array when the page carries no link");
		assert!(!text.contains("/Subtype /Link"), "no link annotation is written");
		assert_eq!(res!(build()), bytes, "an annot-free page is deterministic");
		Ok(())
	}

	#[test]
	fn test_a_coordinate_rounds_to_three_places_10() -> Outcome<()> {
		// Full f32 precision is spent on noise below a thousandth of a point; three places is far finer
		// than any renderer resolves. A rounded -0 folds back to 0 so the bytes stay canonical.
		assert_eq!(numf32(841.8897705078125), "841.89");
		assert_eq!(numf(3.14159), "3.142");
		assert_eq!(numf(0.0004), "0");
		assert_eq!(numf(-0.0004), "0");
		assert_eq!(numf(12.5), "12.5");
		assert_eq!(numf(100.0), "100");
		Ok(())
	}

	#[test]
	fn test_compression_shrinks_and_flate_decodes_11() -> Outcome<()> {
		use flate2::read::ZlibDecoder;
		use std::io::Read;

		// The same page compressed and uncompressed: the compressed file names /FlateDecode on its content
		// stream, is smaller, and its stream inflates back to the operators the uncompressed file writes in
		// the clear.
		let build = |compress: bool| -> Outcome<Vec<u8>> {
			let mut w = PdfWriter::new().with_compression(compress);
			let mut page = PdfPage::new(200.0, 200.0);
			for i in 0..200 {
				let o = i as f32 * 0.1;
				page.fill(res!(Path::rect(Bounds::new(1.0 + o, 1.0 + o, 9.0 + o, 9.0 + o))), Rgba::BLACK);
			}
			w.add_page(page);
			w.to_bytes()
		};
		let plain	= res!(build(false));
		let zipped	= res!(build(true));
		assert!(zipped.len() < plain.len(),
			"compression shrank the file: {} < {}", zipped.len(), plain.len());
		let ztext = String::from_utf8_lossy(&zipped);
		assert!(ztext.contains("/Filter /FlateDecode"), "the content stream is flate-filtered");

		// The content object is object 4 (catalogue, page tree, page, content); pull its stream bytes and
		// inflate them.
		let obj		= b"4 0 obj";
		let at		= match zipped.windows(obj.len()).position(|w| w == obj) {
			Some(i)	=> i,
			None	=> return Err(err!("no content object in the compressed file"; Test)),
		};
		let tail	= &zipped[at..];
		let sopen	= b"stream\n";
		let sp		= match tail.windows(sopen.len()).position(|w| w == sopen) {
			Some(i)	=> i + sopen.len(),
			None	=> return Err(err!("the content object opens no stream"; Test)),
		};
		let sclose	= b"\nendstream";
		let ep		= match tail[sp..].windows(sclose.len()).position(|w| w == sclose) {
			Some(i)	=> sp + i,
			None	=> return Err(err!("the content stream is unterminated"; Test)),
		};
		let mut dec	= ZlibDecoder::new(&tail[sp..ep]);
		let mut raw	= Vec::new();
		res!(dec.read_to_end(&mut raw));
		let ctext	= String::from_utf8_lossy(&raw);
		assert!(ctext.contains("1 0 0 -1 0 200 cm"), "the inflated stream holds the page flip");
		assert!(ctext.contains("\nf\n"), "the inflated stream holds fill operators");
		Ok(())
	}

	/// A small closed outline for glyph tests, offset by `dx` so two calls make two distinct outlines.
	fn tri(dx: f32) -> Outcome<Path> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(dx, 0.0));
		pb.line_to(Pt::new(dx + 100.0, 0.0));
		pb.line_to(Pt::new(dx + 50.0, 200.0));
		pb.close();
		pb.finish()
	}

	#[test]
	fn test_a_repeated_glyph_makes_one_charproc_12() -> Outcome<()> {
		// A glyph drawn twice on a page is stored once as a Type-3 CharProc and shown twice by one code; a
		// second, different outline adds a second CharProc. Text is shown with text operators, not inline
		// path fills. Built uncompressed so the structure is readable in the bytes.
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(200.0, 200.0);
		let a = res!(tri(0.0));
		let b = res!(tri(100.0));
		page.glyph(a.clone(), 10.0, 50.0, 12.0, 8.0, Rgba::BLACK, "A".into());
		page.glyph(a, 30.0, 50.0, 12.0, 8.0, Rgba::BLACK, "A".into());
		page.glyph(b, 50.0, 50.0, 12.0, 8.0, Rgba::BLACK, "B".into());
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);

		assert!(text.contains("/Subtype /Type3"), "a Type-3 font is emitted");
		assert!(text.contains("BT\n"), "a text object opens");
		assert!(text.contains(" Tj\n"), "glyphs are shown with Tj, not inline fills");
		assert!(text.contains("/ToUnicode"), "a ToUnicode CMap is referenced");
		// Two distinct glyphs give /g0 and /g1 and no /g2 -- the repeat added no CharProc.
		assert!(text.contains("/g0 "), "the first glyph's CharProc");
		assert!(text.contains("/g1 "), "the second glyph's CharProc");
		assert!(!text.contains("/g2"), "the repeated glyph added no third CharProc");
		// The repeated glyph is code 0, shown twice; the other is code 1, shown once.
		assert_eq!(text.matches("<00> Tj").count(), 2, "the repeat shows one code twice");
		assert_eq!(text.matches("<01> Tj").count(), 1, "the other glyph shows once");
		Ok(())
	}

	#[test]
	fn test_glyph_bytes_are_deterministic_13() -> Outcome<()> {
		// The same glyphs twice give the same bytes: code assignment is by draw order, and the font tables
		// serialise in a fixed order, so nothing depends on hash-map iteration.
		let build = || -> Outcome<Vec<u8>> {
			let mut w = PdfWriter::new().with_compression(true);
			let mut page = PdfPage::new(200.0, 200.0);
			let a = res!(tri(0.0));
			let b = res!(tri(100.0));
			page.glyph(a.clone(), 10.0, 50.0, 12.0, 8.0, Rgba::BLACK, "A".into());
			page.glyph(b, 30.0, 50.0, 12.0, 8.0, Rgba::BLACK, "B".into());
			page.glyph(a, 50.0, 50.0, 12.0, 8.0, Rgba::BLACK, "A".into());
			w.add_page(page);
			w.to_bytes()
		};
		assert_eq!(res!(build()), res!(build()));
		Ok(())
	}

	#[test]
	fn test_tounicode_maps_codes_to_source_text_14() -> Outcome<()> {
		// The /ToUnicode CMap maps each code to the UTF-16BE of its source text, so a viewer extracts the
		// real characters rather than the font's private codes. Built uncompressed so the CMap is readable.
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(200.0, 200.0);
		page.glyph(res!(tri(0.0)), 10.0, 50.0, 12.0, 8.0, Rgba::BLACK, "O".into());
		page.glyph(res!(tri(100.0)), 30.0, 50.0, 12.0, 8.0, Rgba::BLACK, "x".into());
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let text = String::from_utf8_lossy(&bytes);
		assert!(text.contains("beginbfchar"), "the CMap carries character mappings");
		assert!(text.contains("<00> <004F>"), "code 0 maps to 'O' (U+004F)");
		assert!(text.contains("<01> <0078>"), "code 1 maps to 'x' (U+0078)");
		Ok(())
	}

	#[test]
	fn test_the_xref_offsets_land_on_their_objects_03() -> Outcome<()> {
		// The heart of a valid PDF: every offset in the cross-reference table must point at the first
		// byte of the object it names. This reads each twenty-byte entry's offset back and confirms the
		// object at that offset opens with "N 0 obj", which catches an off-by-one in the byte accounting.
		// The page carries glyphs as well as a fill, so the CharProc, ToUnicode and font-dictionary objects
		// appended at finish() are covered too -- the object count is read from the xref header, not assumed.
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(50.0, 50.0);
		page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::BLACK);
		page.glyph(res!(tri(0.0)), 10.0, 20.0, 12.0, 8.0, Rgba::BLACK, "O".into());
		page.glyph(res!(tri(100.0)), 20.0, 20.0, 12.0, 8.0, Rgba::BLACK, "x".into());
		w.add_page(page);
		let bytes = res!(w.to_bytes());

		// The entries begin after "xref\n" and the "0 M\n" subsection header, where M is the object count
		// plus the free entry. The free object is entry zero; objects 1..=obj_count follow, twenty bytes
		// each.
		// Search the raw bytes, not a lossy string: the header's binary-marker comment holds non-UTF-8
		// bytes, so a String index would not line up with the byte offsets the entries are read at.
		let needle = b"xref\n0 ";
		let marker = match bytes.windows(needle.len()).position(|w| w == needle) {
			Some(i) => i,
			None => return Err(err!("no xref section in the file"; Test)),
		};
		let nl1 = match bytes[marker..].iter().position(|&b| b == b'\n') {
			Some(i) => marker + i,
			None => return Err(err!("the xref header is malformed"; Test)),
		};
		let nl2 = match bytes[nl1 + 1..].iter().position(|&b| b == b'\n') {
			Some(i) => nl1 + 1 + i,
			None => return Err(err!("the xref header is malformed"; Test)),
		};
		// The subsection header reads "0 {count}", the count being every object plus the free entry, so the
		// highest numbered object is one less. Its digits run from just past the needle (the newline nl1 sits
		// inside "xref\n", before them) to nl2. Reading it here covers the font objects appended at finish().
		let count_str	= res!(std::str::from_utf8(&bytes[marker + needle.len()..nl2]));
		let obj_count	= res!(count_str.trim().parse::<usize>()) - 1;
		assert!(obj_count > 4, "the glyphs added font objects past the four base objects: {}", obj_count);
		let entries = &bytes[nl2 + 1..];
		for obj in 1..=obj_count {
			let field = res!(std::str::from_utf8(&entries[obj * 20..obj * 20 + 10]));
			let off: usize = res!(field.parse::<usize>());
			let want = fmt!("{} 0 obj", obj);
			assert!(bytes[off..].starts_with(want.as_bytes()),
				"object {} offset {} does not open with '{}'", obj, off, want);
		}
		Ok(())
	}
}
