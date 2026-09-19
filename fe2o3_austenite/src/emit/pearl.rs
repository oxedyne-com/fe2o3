//! The Pearl page writer: a content-addressed, self-describing document file (`.prl`).
//!
//! Where the SVG arm renders a frame straight to markup, Pearl serialises the same placed frame to a
//! neutral, queryable file: the glyph outlines stored once and referenced, each page a view over a
//! content-addressed block, and the resolved ledger shipping inside. The point is that the file, not
//! the engine, is enough to render or query the document -- `pearl_render` reads a `.prl` back to the
//! very SVG the SVG arm would have written.
//!
//! A leaf is the placed geometry itself: a text run is its box plus, per glyph, the key of a stored
//! outline and that glyph's offset within the run; a figure op is its placement plus a path and its
//! paint. This is the "outlines, not programs" shape -- the outline is stored in the glyph store in
//! the font frame, once, and a placement carries only where it went, not a baked copy.
//!
//! v0 scope: one document round-tripping to the SVG arm's own output. Every leaf kind the SVG arm
//! draws is carried -- text, rule, reservation, and a figure's fills, strokes and rasters. A glyph is
//! keyed by the content of its outline rather than by `face:id:size`, which is font-source-safe: two
//! runs drawn from different font chains cannot collide on a shared `(face, id)`.

use crate::ir::{
	DrawOp,
	LinkTarget,
	Sp,
};
use crate::ledger::{
	AnchorId,
	Ledger,
};
use crate::page::{
	Page,
	PageGeometry,
	PlacedKind,
};
use crate::vfs;

use std::collections::BTreeMap;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
	pixmap::Pixmap,
	stroke::Stroke,
	svg::{
		path_data,
		presentation,
		write_path_data,
	},
	transform::Transform,
};
use oxedyne_fe2o3_text::base64;

/// The Pearl format version this writer emits and the reader accepts.
pub const PEARL_VERSION: &str = "0";

/// A 64-bit FNV-1a over bytes, as sixteen lower-case hex digits: the content address of a block, a
/// glyph outline or a raster. A short hex key is enough for v0 -- a collision costs a wrong lookup, not
/// a silent corruption, and the space is far too large to meet one here.
fn address(bytes: &[u8]) -> String {
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for b in bytes {
		h ^= *b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	fmt!("{:016x}", h)
}

/// A colour as a four-element `[r, g, b, a]` list, the form a leaf stores its paint in.
fn rgba_to_dat(c: Rgba) -> Dat {
	listdat![c.r, c.g, c.b, c.a]
}

/// Reads a colour back from a `[r, g, b, a]` list.
pub fn rgba_from_dat(dat: &Dat) -> Outcome<Rgba> {
	let v = try_extract_dat!(dat.clone(), List);
	if v.len() != 4 {
		return Err(err!(
			"A Pearl colour is a list of four bytes, found {}.", v.len(); Input, Invalid, Mismatch));
	}
	Ok(Rgba::new(
		try_extract_dat!(v[0].clone(), U8),
		try_extract_dat!(v[1].clone(), U8),
		try_extract_dat!(v[2].clone(), U8),
		try_extract_dat!(v[3].clone(), U8),
	))
}

/// A link target as it is stored in a `link` leaf: `["uri", <string>]` for an external address, or
/// `["anchor", <AnchorId map>]` for an internal cross-reference. The anchor rides its own [`ToDat`] form
/// so the reader rebuilds it with [`AnchorId::from_dat`], no private tag table exposed.
fn link_target_to_dat(target: &LinkTarget) -> Outcome<Dat> {
	Ok(match target {
		LinkTarget::Uri(uri)	=> listdat![dat!("uri"), dat!(uri.clone())],
		LinkTarget::Anchor(id)	=> listdat![dat!("anchor"), res!(id.to_dat())],
	})
}

/// Reads a link target back from its stored `["uri", ...]` or `["anchor", ...]` form.
fn link_target_from_dat(dat: &Dat) -> Outcome<LinkTarget> {
	let items	= try_extract_dat!(dat.clone(), List);
	let tag		= try_extract_dat!(res!(items.first().ok_or_else(|| err!(
		"A link target carries no kind tag."; Input, Invalid))).clone(), Str);
	match tag.as_str() {
		"uri" => {
			let uri = try_extract_dat!(res!(items.get(1).ok_or_else(|| err!(
				"A uri link target is missing its address."; Input, Invalid))).clone(), Str);
			Ok(LinkTarget::Uri(uri))
		},
		"anchor" => {
			let id = res!(AnchorId::from_dat(res!(items.get(1).ok_or_else(|| err!(
				"An anchor link target is missing its anchor identity."; Input, Invalid))).clone()));
			Ok(LinkTarget::Anchor(id))
		},
		other => Err(err!(
			"'{}' is not a Pearl v0 link-target kind.", other; Input, Invalid)),
	}
}

/// A link read back from a `.prl`: the rectangle it covers on its page, and where it points. Where a
/// [`Resolved`](LinkResolution) internal target is wanted, [`PearlDoc::resolve_link`] turns the anchor
/// into a block address through the shipped ledger and index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PearlLink {
	pub x:		Sp,
	pub y:		Sp,
	pub w:		Sp,
	pub h:		Sp,
	pub target:	LinkTarget,
}

/// A link's destination once resolved: an external address stands as-is; an internal anchor becomes the
/// content-addressed block it landed in, on the page the ledger fixed it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkResolution {
	Uri(String),
	Block { block: String, page: u32 },
}

/// What an annotation is. A kind the reader does not know is a limit it declares, not a gap it hides, so
/// decoding an unknown kind is an error rather than a silent default -- the same stance the ledger takes
/// on an anchor kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationKind {
	Highlight,	// a marked span, no text of its own beyond the region
	Note,		// a comment attached to the anchor, its words in the payload
}

impl AnnotationKind {
	pub fn as_str(&self) -> &'static str {
		match self {
			AnnotationKind::Highlight	=> "highlight",
			AnnotationKind::Note		=> "note",
		}
	}

	pub fn from_str(s: &str) -> Outcome<Self> {
		match s {
			"highlight"	=> Ok(AnnotationKind::Highlight),
			"note"		=> Ok(AnnotationKind::Note),
			other		=> Err(err!(
				"'{}' is not a Pearl v0 annotation kind.", other; Input, Invalid)),
		}
	}
}

/// A saved annotation. The anchor is the content address of the block it attaches to -- not a page or an
/// offset -- so it survives repagination: the block keeps its identity when it moves to another page, and
/// the annotation follows it. `rect` is a region within that block, in the block's own coordinates, or the
/// whole block when absent. `created` is an author-supplied timestamp, carried verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
	pub anchor:		String,					// content address of the anchored block
	pub rect:		Option<(Sp, Sp, Sp, Sp)>,	// x, y, w, h within the block, or the whole block when None
	pub kind:		AnnotationKind,
	pub payload:	String,					// the note's text, or a highlight's optional label
	pub author:		String,
	pub created:	String,					// author-supplied timestamp, carried as written
}

impl Annotation {
	pub fn new<S: Into<String>>(
		anchor:		S,
		kind:		AnnotationKind,
		payload:	S,
		author:		S,
		created:	S,
	)
		-> Self
	{
		Self {
			anchor:		anchor.into(),
			rect:		None,
			kind,
			payload:	payload.into(),
			author:		author.into(),
			created:	created.into(),
		}
	}

	/// Confines the annotation to a rectangle within its anchored block, rather than the whole block.
	pub fn with_rect(mut self, x: Sp, y: Sp, w: Sp, h: Sp) -> Self {
		self.rect = Some((x, y, w, h));
		self
	}
}

impl ToDat for Annotation {
	fn to_dat(&self) -> Outcome<Dat> {
		let mut d = omapdat!{
			"anchor"	=> dat!(self.anchor.clone()),
			"kind"		=> dat!(self.kind.as_str()),
			"payload"	=> dat!(self.payload.clone()),
			"author"	=> dat!(self.author.clone()),
			"created"	=> dat!(self.created.clone()),
		};
		if let Some((x, y, w, h)) = self.rect {
			res!(d.map_put(dat!("rect"), listdat![
				res!(x.to_dat()),
				res!(y.to_dat()),
				res!(w.to_dat()),
				res!(h.to_dat()),
			]));
		}
		Ok(d)
	}
}

impl FromDat for Annotation {
	fn from_dat(mut dat: Dat) -> Outcome<Self> {
		let anchor	= try_extract_dat!(res!(dat.map_remove_must(&dat!("anchor"))), Str);
		let kind	= res!(AnnotationKind::from_str(
			&try_extract_dat!(res!(dat.map_remove_must(&dat!("kind"))), Str)));
		let payload	= try_extract_dat!(res!(dat.map_remove_must(&dat!("payload"))), Str);
		let author	= try_extract_dat!(res!(dat.map_remove_must(&dat!("author"))), Str);
		let created	= try_extract_dat!(res!(dat.map_remove_must(&dat!("created"))), Str);
		let rect	= match res!(dat.map_remove(&dat!("rect"))) {
			Some(d) => {
				let v = try_extract_dat!(d, List);
				if v.len() != 4 {
					return Err(err!(
						"An annotation rect is four scaled lengths, found {}.", v.len();
						Input, Invalid, Mismatch));
				}
				Some((
					res!(Sp::from_dat(v[0].clone())),
					res!(Sp::from_dat(v[1].clone())),
					res!(Sp::from_dat(v[2].clone())),
					res!(Sp::from_dat(v[3].clone())),
				))
			},
			None => None,
		};
		Ok(Self { anchor, rect, kind, payload, author, created })
	}
}

/// Accumulates pages into one Pearl document: the glyph, image and block stores deduplicated by content
/// address, the page index in order, and the ledger shipped inside. Fed one page at a time from the
/// emit loop, before each page's frame is dropped, so it streams exactly as the SVG and PDF arms do.
pub struct PearlBuilder {
	glyphs:	BTreeMap<String, Dat>,	// outline address -> { "d", "adv" }
	images:	BTreeMap<String, Dat>,	// raster address  -> { "w", "h", "png" (base64) }
	blocks:	BTreeMap<String, Dat>,	// block address   -> page block
	index:	Vec<Dat>,				// [{ "page", "block" }, ...], in page order
	geom:	Dat,					// the document geometry, [w, h, inside, outside, top, bottom]
	ledger:	Ledger,					// shipped inside, and used to resolve internal link targets
}

impl PearlBuilder {
	pub fn new(ledger: &Ledger, geom: PageGeometry) -> Outcome<Self> {
		Ok(Self {
			glyphs:	BTreeMap::new(),
			images:	BTreeMap::new(),
			blocks:	BTreeMap::new(),
			index:	Vec::new(),
			geom:	geometry_to_dat(&geom),
			ledger:	ledger.clone(),
		})
	}

	/// Serialises one page into a content block, folding its glyphs and rasters into the shared stores
	/// and appending its index entry. The leaf order and per-glyph order match the SVG arm's walk of
	/// `frame.placed`, which is what lets the reader reproduce that arm's bytes.
	pub fn add_page(&mut self, page: &Page) -> Outcome<()> {
		let mut leaves: Vec<Dat> = Vec::new();

		for placed in &page.frame.placed {
			match &placed.kind {
				PlacedKind::Text(shaped) => {
					let mut glyphs: Vec<Dat> = Vec::new();
					for glyph in &shaped.run().glyphs {
						let path = res!(shaped.outline(glyph));
						// A glyph with no ink -- a space -- carries an advance but nothing to draw, and the
						// SVG arm skips it; skip it here too so the two runs place the same marks.
						if path.is_empty() {
							continue;
						}
						let d	= write_path_data(&path);
						let key	= address(d.as_bytes());
						self.glyphs.entry(key.clone()).or_insert_with(|| omapdat!{
							"d"		=> dat!(d),
							"adv"	=> dat!(glyph.adv),
						});
						glyphs.push(listdat![dat!(key), dat!(glyph.x), dat!(glyph.y)]);
					}
					let mut leaf = vec![
						dat!("text"),
						res!(placed.x.to_dat()),
						res!(placed.y.to_dat()),
						res!(placed.dims.width.to_dat()),
						res!(placed.dims.height.to_dat()),
						res!(placed.dims.depth.to_dat()),
						Dat::List(glyphs),
					];
					// The fill rides as an optional trailing element, written only when the run is not black.
					// A black run adds nothing, so an all-black document's bytes are exactly what they were
					// before text carried a colour; the reader defaults a missing element to black.
					if shaped.colour() != Rgba::BLACK {
						leaf.push(rgba_to_dat(shaped.colour()));
					}
					leaves.push(Dat::List(leaf));
				},
				PlacedKind::Rule => {
					leaves.push(box_leaf("rule", placed.x, placed.y, placed.dims)?);
				},
				PlacedKind::Reserved => {
					leaves.push(box_leaf("reserved", placed.x, placed.y, placed.dims)?);
				},
				PlacedKind::Graphic(g) => {
					let bx = placed.x;
					let by = placed.y;
					// A linked graphic carries a `link` leaf over its placement box, additive beside its ink so a
					// reader that ignores the tag still draws the figure. The box is the graphic's, matching the
					// PDF arm's annotation rectangle: width, and height plus depth.
					if let Some(target) = &g.link {
						leaves.push(listdat![
							dat!("link"),
							res!(bx.to_dat()),
							res!(by.to_dat()),
							res!(g.dims.width.to_dat()),
							res!((g.dims.height + g.dims.depth).to_dat()),
							res!(link_target_to_dat(target)),
						]);
					}
					for op in &g.ops {
						match op {
							DrawOp::Fill { path, colour } => {
								leaves.push(listdat![
									dat!("fill"),
									res!(bx.to_dat()),
									res!(by.to_dat()),
									dat!(write_path_data(path)),
									rgba_to_dat(*colour),
								]);
							},
							DrawOp::Stroke { path, colour, width } => {
								leaves.push(listdat![
									dat!("stroke"),
									res!(bx.to_dat()),
									res!(by.to_dat()),
									dat!(write_path_data(path)),
									rgba_to_dat(*colour),
									dat!(*width),
								]);
							},
							DrawOp::Image { image, x, y, w, h } => {
								// Re-encode the raster exactly as the SVG arm does, and store that PNG's base64
								// so the reader emits a byte-identical data URI.
								let pm	= res!(Pixmap::from_data(image.width, image.height, image.rgba.clone()));
								let png	= res!(pm.to_png());
								let b64	= base64::encode(&png);
								let key	= address(png.as_slice());
								self.images.entry(key.clone()).or_insert_with(|| omapdat!{
									"w"		=> dat!(image.width as u32),
									"h"		=> dat!(image.height as u32),
									"png"	=> dat!(b64.clone()),
								});
								leaves.push(listdat![
									dat!("image"),
									res!(bx.to_dat()),
									res!(by.to_dat()),
									dat!(*x),
									dat!(*y),
									dat!(*w),
									dat!(*h),
									dat!(key),
								]);
							},
						}
					}
				},
			}
		}

		let block = omapdat!{
			"kind"		=> dat!("page"),
			"number"	=> dat!(page.number),
			"geom"		=> geometry_to_dat(&page.geom),
			"leaves"	=> Dat::List(leaves),
		};
		let enc		= res!(encode(&block));
		let key		= address(enc.as_bytes());
		self.index.push(omapdat!{
			"page"	=> dat!(page.number),
			"block"	=> dat!(key.clone()),
		});
		self.blocks.entry(key).or_insert(block);
		Ok(())
	}

	/// The whole document as one jdat map, ready to encode. The `annotations` section opens empty: the
	/// engine authors none, and a reader adds them through [`PearlDoc::add_annotation`].
	pub fn into_dat(self) -> Outcome<Dat> {
		let glyphs	= create_dat_ordmap(self.glyphs.into_iter().map(|(k, v)| (dat!(k), v)).collect());
		let images	= create_dat_ordmap(self.images.into_iter().map(|(k, v)| (dat!(k), v)).collect());
		let blocks	= create_dat_ordmap(self.blocks.into_iter().map(|(k, v)| (dat!(k), v)).collect());
		Ok(omapdat!{
			"pearl"			=> dat!(PEARL_VERSION),
			"index"			=> Dat::List(self.index),
			"glyphs"		=> glyphs,
			"images"		=> images,
			"blocks"		=> blocks,
			"ledger"		=> res!(self.ledger.to_dat()),
			"geom"			=> self.geom,
			"annotations"	=> Dat::List(Vec::new()),
		})
	}

	/// The whole document encoded as text jdat, the same encoding the ledger uses.
	pub fn to_string(self) -> Outcome<String> {
		encode(&res!(self.into_dat()))
	}

	/// Writes the document to `path` as text jdat.
	pub fn to_file<P: AsRef<std::path::Path>>(self, path: P) -> Outcome<()> {
		res!(vfs::write(path.as_ref(), res!(self.to_string()).as_bytes()));
		Ok(())
	}
}

/// A rule or reservation leaf: the box's position and its three dimensions, from which the reader
/// rebuilds the same rectangle the SVG arm draws.
fn box_leaf(tag: &str, x: Sp, y: Sp, dims: crate::ir::Dims) -> Outcome<Dat> {
	Ok(listdat![
		dat!(tag),
		res!(x.to_dat()),
		res!(y.to_dat()),
		res!(dims.width.to_dat()),
		res!(dims.height.to_dat()),
		res!(dims.depth.to_dat()),
	])
}

/// A page geometry as `[width, height, inside, outside, top, bottom]` scaled points.
fn geometry_to_dat(g: &PageGeometry) -> Dat {
	listdat![
		g.width.raw(),
		g.height.raw(),
		g.inside.raw(),
		g.outside.raw(),
		g.top.raw(),
		g.bottom.raw(),
	]
}

/// Encodes a `Dat` to text jdat with the default config, as [`Ledger::to_file`] does.
fn encode(dat: &Dat) -> Outcome<String> {
	let cfg = oxedyne_fe2o3_jdat::string::enc::EncoderConfig::<(), ()>::default();
	Ok(res!(dat.encode_string_with_config(&cfg)))
}

// ---------------------------------------------------------------------------------------------------
// Reading a `.prl` back, and rendering it to the SVG the SVG arm would have written.
// ---------------------------------------------------------------------------------------------------

/// A decoded Pearl document, enough to render or query without the engine. The rendering below walks
/// each page's block and its stored outlines through the very `write_path_data` and `presentation` the
/// SVG arm uses, so a rendered page is that arm's output byte for byte.
pub struct PearlDoc {
	top: Dat,
}

impl PearlDoc {
	/// Reads a `.prl` file, decoding its text jdat.
	pub fn read_file<P: AsRef<std::path::Path>>(path: P) -> Outcome<Self> {
		Self::from_string(res!(vfs::read_to_string(path.as_ref())))
	}

	/// Decodes a Pearl document from its text-jdat form, checking the version.
	pub fn from_string(s: String) -> Outcome<Self> {
		let top	= res!(Dat::decode_string(s));
		let ver	= res!(top.map_get_string(&dat!("pearl")));
		if ver != PEARL_VERSION {
			return Err(err!(
				"This reader speaks Pearl v{}, but the file is v{}.", PEARL_VERSION, ver;
				Input, Invalid, Mismatch));
		}
		Ok(Self { top })
	}

	/// The number of pages in the document's index.
	pub fn page_count(&self) -> Outcome<usize> {
		Ok(res!(self.top.map_get_list(&dat!("index"))).len())
	}

	/// Renders the page at `idx` (zero-based) to a self-contained SVG document, reconstructing the SVG
	/// arm's output from the stored geometry, glyph outlines and paint.
	pub fn render_page(&self, idx: usize) -> Outcome<String> {
		let index	= res!(self.top.map_get_list(&dat!("index")));
		let entry	= res!(index.get(idx).ok_or_else(|| err!(
			"Page index {} is past the {} pages the document holds.", idx, index.len(); Input, Range)));
		let block_key	= res!(entry.map_get_string(&dat!("block")));
		let blocks		= res!(self.top.map_get_must(&dat!("blocks")));
		let block		= res!(blocks.map_get_must(&dat!(block_key)));
		let glyphs		= res!(self.top.map_get_must(&dat!("glyphs")));
		let images		= res!(self.top.map_get_must(&dat!("images")));

		// The viewport is the media box: the geometry's width and height rounded to whole points, exactly
		// as `PageGeometry::media_box` does.
		let geom	= res!(block.map_get_list(&dat!("geom")));
		let w		= sp_at(geom, 0)?.to_pt().round() as usize;
		let h		= sp_at(geom, 1)?.to_pt().round() as usize;

		let mut out = String::new();
		out.push_str(&fmt!(
			"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
			w, h, w, h));
		out.push_str(&fmt!(
			"<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n", w, h));

		// A half-point grey pen for a reservation, matching the SVG arm's `pen`/`grey`.
		let pen		= res!(Stroke::new(0.5));
		let grey	= Rgba::new(176, 176, 176, 255);

		let leaves = res!(block.map_get_list(&dat!("leaves")));
		for leaf in leaves {
			let items	= try_extract_dat!(leaf.clone(), List);
			let tag		= try_extract_dat!(res!(items.first().ok_or_else(|| err!(
				"An empty leaf carries no kind tag."; Input, Invalid))).clone(), Str);
			match tag.as_str() {
				"text" => {
					let x		= sp_at(&items, 1)?;
					let y		= sp_at(&items, 2)?;
					let height	= sp_at(&items, 4)?;
					let base_x	= x.to_pt() as f32;
					let base_y	= (y + height).to_pt() as f32;
					// The fill is an optional trailing element; a leaf without one is black, the form every
					// pre-colour text leaf took, so an all-black document reads back byte-identical.
					let colour	= match items.get(7) {
						Some(d)	=> res!(rgba_from_dat(d)),
						None	=> Rgba::BLACK,
					};
					let run		= try_extract_dat!(res!(items.get(6).ok_or_else(|| err!(
						"A text leaf is missing its glyph list."; Input, Invalid))).clone(), List);
					for g in &run {
						let gl	= try_extract_dat!(g.clone(), List);
						let key	= try_extract_dat!(res!(gl.first().ok_or_else(|| err!(
							"A glyph placement carries no outline key."; Input, Invalid))).clone(), Str);
						let gx	= f32_at(&gl, 1)?;
						let gy	= f32_at(&gl, 2)?;
						let entry	= res!(glyphs.map_get_must(&dat!(key)));
						let d		= res!(entry.map_get_string(&dat!("d")));
						let path	= res!(path_data(&d));
						if path.is_empty() {
							continue;
						}
						// The stored outline is font-frame, y up; flip in y and move onto the baseline at the
						// glyph's offset, exactly as `draw_text` does.
						let t = Transform::scale(1.0, -1.0)
							.then(&Transform::translate(base_x + gx, base_y - gy));
						let placed = res!(path.transform(&t));
						out.push_str(&fmt!(
							"  <path d=\"{}\" {}/>\n",
							write_path_data(&placed), presentation(Some(colour), None)));
					}
				},
				"rule" | "reserved" => {
					let x		= sp_at(&items, 1)?;
					let y		= sp_at(&items, 2)?;
					let width	= sp_at(&items, 3)?;
					let height	= sp_at(&items, 4)?;
					let depth	= sp_at(&items, 5)?;
					let x0 = x.to_pt() as f32;
					let y0 = y.to_pt() as f32;
					let x1 = (x + width).to_pt() as f32;
					let y1 = (y + height + depth).to_pt() as f32;
					// A zero-area box has nothing to draw, and `Path::rect` would reject it -- the SVG arm
					// skips it too, so skipping here keeps the two outputs identical.
					if x1 <= x0 || y1 <= y0 {
						continue;
					}
					let path	= res!(Path::rect(Bounds::new(x0, y0, x1, y1)));
					let d		= write_path_data(&path);
					let attrs	= if tag == "rule" {
						presentation(Some(Rgba::BLACK), None)
					} else {
						presentation(None, Some((grey, &pen)))
					};
					out.push_str(&fmt!("  <path d=\"{}\" {}/>\n", d, attrs));
				},
				"fill" => {
					let bx		= sp_at(&items, 1)?;
					let by		= sp_at(&items, 2)?;
					let d		= try_extract_dat!(res!(items.get(3).ok_or_else(|| err!(
						"A fill leaf is missing its path."; Input, Invalid))).clone(), Str);
					let colour	= res!(rgba_from_dat(res!(items.get(4).ok_or_else(|| err!(
						"A fill leaf is missing its colour."; Input, Invalid)))));
					let t = Transform::translate(bx.to_pt() as f32, by.to_pt() as f32);
					let p = res!(res!(path_data(&d)).transform(&t));
					out.push_str(&fmt!(
						"  <path d=\"{}\" {}/>\n", write_path_data(&p), presentation(Some(colour), None)));
				},
				"stroke" => {
					let bx		= sp_at(&items, 1)?;
					let by		= sp_at(&items, 2)?;
					let d		= try_extract_dat!(res!(items.get(3).ok_or_else(|| err!(
						"A stroke leaf is missing its path."; Input, Invalid))).clone(), Str);
					let colour	= res!(rgba_from_dat(res!(items.get(4).ok_or_else(|| err!(
						"A stroke leaf is missing its colour."; Input, Invalid)))));
					let width	= f32_at(&items, 5)?;
					let stroke	= res!(Stroke::new(width));
					let t = Transform::translate(bx.to_pt() as f32, by.to_pt() as f32);
					let p = res!(res!(path_data(&d)).transform(&t));
					out.push_str(&fmt!(
						"  <path d=\"{}\" {}/>\n",
						write_path_data(&p), presentation(None, Some((colour, &stroke)))));
				},
				"image" => {
					let bx	= sp_at(&items, 1)?;
					let by	= sp_at(&items, 2)?;
					let x	= f32_at(&items, 3)?;
					let y	= f32_at(&items, 4)?;
					let iw	= f32_at(&items, 5)?;
					let ih	= f32_at(&items, 6)?;
					let key	= try_extract_dat!(res!(items.get(7).ok_or_else(|| err!(
						"An image leaf is missing its raster key."; Input, Invalid))).clone(), Str);
					let entry	= res!(images.map_get_must(&dat!(key)));
					let b64		= res!(entry.map_get_string(&dat!("png")));
					let ox = bx.to_pt() as f32;
					let oy = by.to_pt() as f32;
					out.push_str(&fmt!(
						"  <image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" \
							href=\"data:image/png;base64,{}\"/>\n",
						ox + x, oy + y, iw, ih, b64));
				},
				// A link leaf places no ink: the SVG arm draws no clickable annotation, so rendering skips it and
				// the page stays byte-identical to that arm's output. A reader that wants the links reads them
				// with `PearlDoc::links_on_page`.
				"link" => {},
				other => return Err(err!(
					"'{}' is not a Pearl v0 leaf kind.", other; Input, Invalid)),
			}
		}
		out.push_str("</svg>\n");
		Ok(out)
	}

	/// The links on the page at `idx` (zero-based): each `link` leaf's rectangle and target, in the order
	/// they were emitted. A page with no links returns an empty vector.
	pub fn links_on_page(&self, idx: usize) -> Outcome<Vec<PearlLink>> {
		let index	= res!(self.top.map_get_list(&dat!("index")));
		let entry	= res!(index.get(idx).ok_or_else(|| err!(
			"Page index {} is past the {} pages the document holds.", idx, index.len(); Input, Range)));
		let block_key	= res!(entry.map_get_string(&dat!("block")));
		let blocks		= res!(self.top.map_get_must(&dat!("blocks")));
		let block		= res!(blocks.map_get_must(&dat!(block_key)));
		let leaves		= res!(block.map_get_list(&dat!("leaves")));
		let mut out = Vec::new();
		for leaf in leaves {
			let items	= try_extract_dat!(leaf.clone(), List);
			let tag		= try_extract_dat!(res!(items.first().ok_or_else(|| err!(
				"An empty leaf carries no kind tag."; Input, Invalid))).clone(), Str);
			if tag != "link" {
				continue;
			}
			let target = res!(link_target_from_dat(res!(items.get(5).ok_or_else(|| err!(
				"A link leaf is missing its target."; Input, Invalid)))));
			out.push(PearlLink {
				x:		sp_at(&items, 1)?,
				y:		sp_at(&items, 2)?,
				w:		sp_at(&items, 3)?,
				h:		sp_at(&items, 4)?,
				target,
			});
		}
		Ok(out)
	}

	/// Resolves a link's destination: an external target stands as its address; an internal anchor is
	/// resolved through the shipped ledger to the page it landed on, then through the index to that page's
	/// content-addressed block. `None` when the ledger has not fixed the anchor, or no page in the index
	/// carries it -- a dangling cross-reference the caller reports rather than follows.
	pub fn resolve_link(&self, target: &LinkTarget) -> Outcome<Option<LinkResolution>> {
		match target {
			LinkTarget::Uri(uri) => Ok(Some(LinkResolution::Uri(uri.clone()))),
			LinkTarget::Anchor(id) => {
				let ledger	= res!(Ledger::from_dat(res!(self.top.map_get_must(&dat!("ledger"))).clone()));
				let page	= match ledger.page_of(id) {
					Some(p)	=> p,
					None	=> return Ok(None),	// the ledger has not fixed this anchor
				};
				let index = res!(self.top.map_get_list(&dat!("index")));
				for entry in index {
					if try_extract_dat!(res!(entry.map_get_must(&dat!("page"))).clone(), U32) == page {
						let block = res!(entry.map_get_string(&dat!("block")));
						return Ok(Some(LinkResolution::Block { block, page }));
					}
				}
				Ok(None)	// the anchor's page is not one the index holds
			},
		}
	}

	/// The content addresses of the document's page blocks, in page order, read from the index. These are
	/// the stable identities an annotation anchors to.
	pub fn block_hashes(&self) -> Outcome<Vec<String>> {
		let index = res!(self.top.map_get_list(&dat!("index")));
		let mut out = Vec::with_capacity(index.len());
		for entry in index {
			out.push(res!(entry.map_get_string(&dat!("block"))));
		}
		Ok(out)
	}

	/// Is `hash` the address of a block the document holds? An annotation whose anchor answers false is
	/// orphaned -- its content is gone from the file.
	pub fn has_block(&self, hash: &str) -> Outcome<bool> {
		let blocks = res!(self.top.map_get_must(&dat!("blocks")));
		Ok(res!(blocks.map_get(&dat!(hash))).is_some())
	}

	/// The annotations carried in the document, in the order they were added. A file written before the
	/// annotations section existed, or one with an empty section, returns an empty vector.
	pub fn annotations(&self) -> Outcome<Vec<Annotation>> {
		let list = match self.top.map_get(&dat!("annotations")) {
			Ok(Some(d))	=> try_extract_dat!(d.clone(), List),
			_			=> return Ok(Vec::new()),
		};
		let mut out = Vec::with_capacity(list.len());
		for d in list {
			out.push(res!(Annotation::from_dat(d)));
		}
		Ok(out)
	}

	/// Attaches an annotation, appending it to the `annotations` section. The anchor must address a block
	/// the document holds, so an annotation cannot be attached to content that is not here; the rectangle,
	/// if any, is a region within that block. The change lives in memory until [`to_string`](Self::to_string)
	/// or [`write_file`](Self::write_file) writes the document back.
	pub fn add_annotation(&mut self, ann: Annotation) -> Outcome<()> {
		if !res!(self.has_block(&ann.anchor)) {
			return Err(err!(
				"Annotation anchor block {} is not in the document, so nothing to attach it to.",
				ann.anchor; Input, Invalid, Missing));
		}
		let mut list = match res!(self.top.map_get(&dat!("annotations"))) {
			Some(d)	=> try_extract_dat!(d.clone(), List),
			None	=> Vec::new(),
		};
		list.push(res!(ann.to_dat()));
		res!(self.top.map_put(dat!("annotations"), Dat::List(list)));
		Ok(())
	}

	/// The document re-encoded as text jdat, carrying every later change -- added annotations included.
	pub fn to_string(&self) -> Outcome<String> {
		encode(&self.top)
	}

	/// Writes the document back to `path` as text jdat, carrying every later change.
	pub fn write_file<P: AsRef<std::path::Path>>(&self, path: P) -> Outcome<()> {
		res!(vfs::write(path.as_ref(), res!(self.to_string()).as_bytes()));
		Ok(())
	}
}

/// The scaled length at `i` in a list, decoded through the same [`Sp::from_dat`] the emit used.
fn sp_at(list: &[Dat], i: usize) -> Outcome<Sp> {
	let d = res!(list.get(i).ok_or_else(|| err!(
		"A leaf is missing its scaled length at position {}.", i; Input, Invalid)));
	Sp::from_dat(d.clone())
}

/// The `f32` at `i` in a list.
fn f32_at(list: &[Dat], i: usize) -> Outcome<f32> {
	let d = res!(list.get(i).ok_or_else(|| err!(
		"A leaf is missing its float at position {}.", i; Input, Invalid)));
	Ok(try_extract_dat!(d.clone(), F32).0)
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::emit::svg;
	use crate::font::ShapedText;
	use crate::ir::{
		Dims,
		DrawOp,
		Graphic,
		LinkTarget,
	};
	use crate::ledger::{
		Anchor,
		AnchorId,
		AnchorKind,
		Ledger,
		Position,
	};
	use crate::page::{
		Frame,
		Page,
		PageGeometry,
		Placed,
		PlacedKind,
	};

	use std::sync::Arc;

	use oxedyne_fe2o3_font::{
		face::Role,
		shape::Dir,
	};
	use oxedyne_fe2o3_graphics::{
		colour::Rgba,
		path::{
			Bounds,
			Path,
		},
	};

	// One page carrying every leaf kind the SVG arm draws bar a raster -- a shaped text run, a rule, and a
	// figure of one fill and one stroke -- rendered both by the SVG arm and by a Pearl round trip, must
	// come out byte for byte the same. This is the keystone claim in miniature, without the CLI or files.
	#[test]
	fn test_pearl_round_trips_to_the_svg_arm_00() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();

		let mut frame = Frame::new();

		// A shaped run of real text, placed like a line of body copy.
		let shaped	= res!(ShapedText::new(fonts, Role::Body, Dir::Ltr, Sp::from_pt(11.0), "Pearl round trip."));
		let tdims	= shaped.dims();
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0), tdims, PlacedKind::Text(shaped)));

		// A rule: a thin filled rectangle.
		let rule_dims = Dims::new(Sp::from_pt(120.0), Sp::from_pt(0.6), Sp::ZERO);
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(100.0), rule_dims, PlacedKind::Rule));

		// A figure of one filled and one stroked path, in the graphic's own frame.
		let fill	= res!(Path::rect(Bounds::new(0.0, 0.0, 40.0, 30.0)));
		let stroke	= res!(Path::rect(Bounds::new(5.0, 5.0, 35.0, 25.0)));
		let ops		= vec![
			DrawOp::Fill { path: fill, colour: Rgba::new(233, 236, 239, 255) },
			DrawOp::Stroke { path: stroke, colour: Rgba::BLACK, width: 1.0 },
		];
		let graphic	= Graphic::new(ops, Dims::new(Sp::from_pt(40.0), Sp::from_pt(30.0), Sp::ZERO));
		frame.push(Placed::new(
			Sp::from_pt(200.0), Sp::from_pt(120.0), graphic.dims, PlacedKind::Graphic(Arc::new(graphic))));

		let page = Page::new(1, geom, frame);

		// The reference: what the SVG arm writes for this page.
		let want = res!(svg::render_page(&page));

		// The round trip: emit to Pearl, encode, decode, render back.
		let mut builder	= res!(PearlBuilder::new(&Ledger::new(), geom));
		res!(builder.add_page(&page));
		let encoded		= res!(builder.to_string());
		let doc			= res!(PearlDoc::from_string(encoded));
		assert_eq!(res!(doc.page_count()), 1);
		let got			= res!(doc.render_page(0));

		assert_eq!(want, got, "the Pearl round trip did not reproduce the SVG arm's page byte for byte");
		Ok(())
	}

	// A colour survives the list encoding it is stored in.
	#[test]
	fn test_a_colour_round_trips_through_its_leaf_form_01() -> Outcome<()> {
		let c = Rgba::new(128, 0, 200, 64);
		assert_eq!(c, res!(rgba_from_dat(&rgba_to_dat(c))));
		Ok(())
	}

	/// A one-op figure, enough to place as a linked graphic without a font.
	fn dot_graphic(link: Option<LinkTarget>) -> Outcome<Graphic> {
		let fill = res!(Path::rect(Bounds::new(0.0, 0.0, 20.0, 20.0)));
		let mut g = Graphic::new(
			vec![DrawOp::Fill { path: fill, colour: Rgba::BLACK }],
			Dims::new(Sp::from_pt(20.0), Sp::from_pt(20.0), Sp::ZERO));
		g.link = link;
		Ok(g)
	}

	// A document with an external `#link` and an internal `@ref` carries both into the `.prl` as `link`
	// leaves, and the reader reads them back: the external one stands as its URI, and the internal one
	// resolves -- through the shipped ledger and index -- to the content-addressed block of the page its
	// anchor landed on, not to a raw page number.
	#[test]
	fn test_links_round_trip_internal_and_external_02() -> Outcome<()> {
		let geom	= PageGeometry::a4();
		let anchor	= AnchorId::new(AnchorKind::Label, "sec:intro");

		// Page 1 carries the two links; page 2 is where the internal anchor resolves.
		let mut frame1 = Frame::new();
		let ext = res!(dot_graphic(Some(LinkTarget::Uri("https://oxedyne.com".to_string()))));
		frame1.push(Placed::new(
			Sp::from_pt(50.0), Sp::from_pt(50.0), ext.dims, PlacedKind::Graphic(Arc::new(ext))));
		let int = res!(dot_graphic(Some(LinkTarget::Anchor(anchor.clone()))));
		frame1.push(Placed::new(
			Sp::from_pt(50.0), Sp::from_pt(120.0), int.dims, PlacedKind::Graphic(Arc::new(int))));
		let page1 = Page::new(1, geom, frame1);

		let mut frame2 = Frame::new();
		let target = res!(dot_graphic(None));
		frame2.push(Placed::new(
			Sp::from_pt(60.0), Sp::from_pt(60.0), target.dims, PlacedKind::Graphic(Arc::new(target))));
		let page2 = Page::new(2, geom, frame2);

		// The ledger fixes the anchor on page 2, as a composition pass would.
		let mut ledger = Ledger::new();
		ledger.record(Anchor::new(anchor.clone(), Position::new(2, Sp::ZERO, Sp::ZERO)));

		let mut builder = res!(PearlBuilder::new(&ledger, geom));
		res!(builder.add_page(&page1));
		res!(builder.add_page(&page2));
		let doc = res!(PearlDoc::from_string(res!(builder.to_string())));

		let links = res!(doc.links_on_page(0));
		assert_eq!(links.len(), 2, "both links are carried onto page 1");
		assert!(res!(doc.links_on_page(1)).is_empty(), "page 2 carries no links");

		// The external link stands as its address.
		let ext_link = res!(links.iter().find(|l| matches!(l.target, LinkTarget::Uri(_)))
			.ok_or_else(|| err!("the external link is missing"; Test)));
		assert_eq!(
			res!(doc.resolve_link(&ext_link.target)),
			Some(LinkResolution::Uri("https://oxedyne.com".to_string())));

		// The internal link resolves to page 2's block, not to the number 2.
		let int_link = res!(links.iter().find(|l| matches!(l.target, LinkTarget::Anchor(_)))
			.ok_or_else(|| err!("the internal link is missing"; Test)));
		let want_block = res!(doc.block_hashes())[1].clone();
		assert_eq!(
			res!(doc.resolve_link(&int_link.target)),
			Some(LinkResolution::Block { block: want_block, page: 2 }));
		Ok(())
	}

	// Annotations anchor to content-addressed block hashes, read back with their fields intact, and survive
	// a re-emit of the document -- the point being that the anchor is the block's identity, so an annotation
	// stays attached across a rewrite the way it would across a repagination.
	#[test]
	fn test_annotations_anchor_to_blocks_and_survive_re_emit_03() -> Outcome<()> {
		let geom = PageGeometry::a4();

		// A two-page document, so there are two real block hashes to anchor to.
		let mut b = res!(PearlBuilder::new(&Ledger::new(), geom));
		for n in 1..=2u32 {
			let mut frame = Frame::new();
			let g = res!(dot_graphic(None));
			frame.push(Placed::new(
				Sp::from_pt(40.0), Sp::from_pt(40.0), g.dims, PlacedKind::Graphic(Arc::new(g))));
			res!(b.add_page(&Page::new(n, geom, frame)));
		}
		let mut doc = res!(PearlDoc::from_string(res!(b.to_string())));

		// Emit is empty by default.
		assert!(res!(doc.annotations()).is_empty(), "the engine authors no annotations");

		let hashes = res!(doc.block_hashes());
		assert_eq!(hashes.len(), 2);
		res!(doc.add_annotation(Annotation::new(
			hashes[0].as_str(), AnnotationKind::Highlight, "the opening claim", "jason",
			"2026-09-17T10:00:00Z").with_rect(
			Sp::from_pt(40.0), Sp::from_pt(40.0), Sp::from_pt(120.0), Sp::from_pt(12.0))));
		res!(doc.add_annotation(Annotation::new(
			hashes[1].as_str(), AnnotationKind::Note, "check this figure", "jason",
			"2026-09-17T11:00:00Z")));

		// An annotation cannot attach to a block the document does not hold.
		assert!(doc.add_annotation(Annotation::new(
			"0000000000000000", AnnotationKind::Note, "orphan", "jason", "2026-09-17T12:00:00Z")).is_err(),
			"attaching to an absent block is refused");

		// Re-emit and re-read: the annotations survive, resolve to the right blocks, and keep their fields.
		let doc2 = res!(PearlDoc::from_string(res!(doc.to_string())));
		let anns = res!(doc2.annotations());
		assert_eq!(anns.len(), 2, "both annotations survive the re-emit");
		assert_eq!(anns[0].anchor, hashes[0], "the first annotation still names page 1's block");
		assert!(res!(doc2.has_block(&anns[0].anchor)), "and that block is still in the document");
		assert_eq!(anns[0].kind, AnnotationKind::Highlight);
		assert_eq!(anns[0].rect, Some((
			Sp::from_pt(40.0), Sp::from_pt(40.0), Sp::from_pt(120.0), Sp::from_pt(12.0))));
		assert_eq!(anns[1].anchor, hashes[1], "the second annotation still names page 2's block");
		assert_eq!(anns[1].kind, AnnotationKind::Note);
		assert_eq!(anns[1].payload, "check this figure");
		assert_eq!(anns[1].rect, None, "a whole-block annotation carries no rect");

		// A second re-emit keeps them still, so the section is stable under repeated rewrites.
		let doc3 = res!(PearlDoc::from_string(res!(doc2.to_string())));
		assert_eq!(res!(doc3.annotations()).len(), 2, "annotations persist across a second re-emit");
		Ok(())
	}
}
