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

use oxedyne_fe2o3_core::prelude::*;

/// One drawn shape: a path and how it is painted. Fill and stroke are the two the typesetter needs --
/// glyphs and rules fill, a held-open reservation strokes.
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
}

impl Draw {

	/// The paint colour, whichever kind of draw this is.
	fn colour(&self) -> Rgba {
		match self {
			Draw::Fill { colour, .. }	=> *colour,
			Draw::Stroke { colour, .. }	=> *colour,
		}
	}
}

/// One page: its size in points, and the shapes drawn on it, back to front.
///
/// The coordinates in the paths are the engine's page frame -- top-left origin, y increasing
/// downwards -- exactly as the SVG writer receives them. [`PdfWriter`] applies the flip to PDF's
/// y-up frame itself, so a caller hands the same paths to either writer.
#[derive(Clone, Debug)]
pub struct PdfPage {
	pub width:	f64,	// media box width, in points
	pub height:	f64,	// media box height, in points
	pub draws:	Vec<Draw>,
}

impl PdfPage {

	pub fn new(width: f64, height: f64) -> Self {
		Self { width, height, draws: Vec::new() }
	}

	pub fn fill(&mut self, path: Path, colour: Rgba) {
		self.draws.push(Draw::Fill { path, colour });
	}

	pub fn stroke(&mut self, path: Path, colour: Rgba, width: f64) {
		self.draws.push(Draw::Stroke { path, colour, width });
	}
}

/// Accumulates pages and writes them out as one PDF file.
#[derive(Clone, Debug, Default)]
pub struct PdfWriter {
	pages:		Vec<PdfPage>,
	compress:	bool,
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

	/// Renders the whole document to PDF bytes.
	///
	/// The body is built into one buffer while the byte offset of every object is recorded, so the
	/// cross-reference table can name each object's exact position. The trailer's `/ID` is a hash of
	/// that body, so two runs over the same pages agree to the byte.
	pub fn to_bytes(&self) -> Outcome<Vec<u8>> {
		let n = self.pages.len();
		// Object 1 is the catalogue, 2 the page tree, then a page object and a content stream for each
		// page: page i takes 3 + 2i and 4 + 2i.
		let obj_count = 2 + 2 * n;
		let mut buf: Vec<u8> = Vec::new();
		let mut offsets: Vec<usize> = vec![0; obj_count + 1]; // one-based; [0] is the free object

		buf.extend_from_slice(b"%PDF-1.7\n");
		// A comment of high bytes tells a naive tool the file is binary, so it is not mangled in
		// transit. Four bytes above 127, as the specification suggests.
		buf.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

		// The content streams are built first, since a stream's `/Length` must be known before its
		// object is written.
		let mut streams: Vec<Vec<u8>> = Vec::with_capacity(n);
		for page in &self.pages {
			let raw = content_stream(page).into_bytes();
			let bytes = if self.compress {
				res!(deflate(&raw))
			} else {
				raw
			};
			streams.push(bytes);
		}

		// The catalogue.
		offsets[1] = buf.len();
		buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

		// The page tree.
		offsets[2] = buf.len();
		let mut kids = String::new();
		for i in 0..n {
			if i > 0 {
				kids.push(' ');
			}
			kids.push_str(&fmt!("{} 0 R", 3 + 2 * i));
		}
		buf.extend_from_slice(fmt!(
			"2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n", kids, n).as_bytes());

		// Each page, then its content stream.
		for i in 0..n {
			let page	= &self.pages[i];
			let page_obj	= 3 + 2 * i;
			let content_obj	= 4 + 2 * i;

			offsets[page_obj] = buf.len();
			buf.extend_from_slice(fmt!(
				"{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] {} \
					/Contents {} 0 R >>\nendobj\n",
				page_obj, numf(page.width), numf(page.height), resources(page), content_obj,
			).as_bytes());

			offsets[content_obj] = buf.len();
			let filter = if self.compress { " /Filter /FlateDecode" } else { "" };
			buf.extend_from_slice(fmt!(
				"{} 0 obj\n<< /Length {}{} >>\nstream\n",
				content_obj, streams[i].len(), filter).as_bytes());
			buf.extend_from_slice(&streams[i]);
			buf.extend_from_slice(b"\nendstream\nendobj\n");
		}

		// The identifier is derived from the body already written, never from the clock.
		let id = doc_id(&buf);

		// The cross-reference table. Every entry is exactly twenty bytes: a ten-digit offset, a
		// five-digit generation, the type, and a two-byte end.
		let xref_off = buf.len();
		buf.extend_from_slice(fmt!("xref\n0 {}\n", obj_count + 1).as_bytes());
		buf.extend_from_slice(b"0000000000 65535 f\r\n");
		for k in 1..=obj_count {
			buf.extend_from_slice(fmt!("{:010} 00000 n\r\n", offsets[k]).as_bytes());
		}

		buf.extend_from_slice(fmt!(
			"trailer\n<< /Size {} /Root 1 0 R /ID [<{}> <{}>] >>\nstartxref\n{}\n%%EOF\n",
			obj_count + 1, id, id, xref_off).as_bytes());

		Ok(buf)
	}
}

/// Builds the content stream for one page: the flip, then every shape's colour, path and paint.
///
/// The first operator flips the frame. PDF has its origin at the bottom left with y increasing
/// upwards; the engine places from the top left with y increasing downwards. The matrix `1 0 0 -1 0
/// H` maps `(x, y)` to `(x, H - y)`, so a point at the top of the page (`y = 0`) lands at PDF's `H`
/// and one at the foot (`y = H`) lands at `0`. Applied once as the current transform, it carries the
/// whole page across, and the paths need no per-point flip.
fn content_stream(page: &PdfPage) -> String {
	let mut s = String::new();
	s.push_str(&fmt!("1 0 0 -1 0 {} cm\n", numf(page.height)));

	// A translucent shape needs its alpha set through a graphics state; an all-opaque page needs none,
	// and sets nothing.
	let translucent = page.draws.iter().any(|d| d.colour().a != 255);
	let mut cur_alpha: Option<u8> = None;

	for d in &page.draws {
		match d {
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
		}
	}
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

/// The page's `/Resources`, holding an `/ExtGState` for each distinct alpha only when the page has a
/// translucent shape. An all-opaque page carries an empty resource dictionary.
fn resources(page: &PdfPage) -> String {
	let translucent = page.draws.iter().any(|d| d.colour().a != 255);
	if !translucent {
		return fmt!("/Resources << >>");
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
	fmt!("/Resources << /ExtGState << {}>> >>", gs)
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

/// A length or coordinate in its shortest exact decimal. Rust's own float formatting already gives
/// the shortest form that reads back to the same value.
fn numf(v: f64) -> String {
	fmt!("{}", v)
}

fn numf32(v: f32) -> String {
	fmt!("{}", v)
}

/// A deterministic identifier for the file, thirty-two hex digits from two FNV-1a hashes of the body.
///
/// A `/ID` conventionally identifies a file, and a fresh file's two elements are equal. Deriving it
/// from the content rather than a clock or a random source keeps the whole file byte-deterministic.
fn doc_id(bytes: &[u8]) -> String {
	let a = fnv1a_64(bytes, 0xcbf2_9ce4_8422_2325);
	let b = fnv1a_64(bytes, 0x8422_2325_cbf2_9ce4);
	fmt!("{:016x}{:016x}", a, b)
}

/// FNV-1a over the bytes, from a given basis.
fn fnv1a_64(bytes: &[u8], basis: u64) -> u64 {
	let mut h = basis;
	for &b in bytes {
		h ^= b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	h
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
	fn test_the_xref_offsets_land_on_their_objects_03() -> Outcome<()> {
		// The heart of a valid PDF: every offset in the cross-reference table must point at the first
		// byte of the object it names. This reads each twenty-byte entry's offset back and confirms the
		// object at that offset opens with "N 0 obj", which catches an off-by-one in the byte
		// accounting. One page gives four objects: catalogue, page tree, page, content.
		let mut w = PdfWriter::new();
		let mut page = PdfPage::new(50.0, 50.0);
		page.fill(res!(Path::rect(Bounds::new(1.0, 1.0, 9.0, 9.0))), Rgba::BLACK);
		w.add_page(page);
		let bytes = res!(w.to_bytes());
		let obj_count = 4;

		// The entries begin after "xref\n" and the "0 M\n" subsection header. The free object is entry
		// zero; objects 1..=obj_count follow, twenty bytes each.
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
