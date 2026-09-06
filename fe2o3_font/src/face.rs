//! One typeface: parse, coverage, metrics, shaping and glyph outlines.
//!
//! Where `harfrust` shapes and `skrifa` draws, both turned back into this crate's own types at once.
//! A face is rarely used alone; what a caller draws with is a [`Font`](crate::font::Font), a chain of
//! these.

use crate::shape::{
	Dir,
	Glyph,
	Run,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::prelude::*;

use harfrust::{
	FontRef as ShapeFont,
	ShapeOptions,
	ShaperData,
	UnicodeBuffer,
};

use skrifa::{
	instance::{
		LocationRef,
		Size,
	},
	outline::{
		DrawSettings,
		OutlinePen,
	},
	FontRef as OutlineFont,
	GlyphId,
	MetadataProvider,
};

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::RwLock;

/// The part a font plays. A document names a role; the reader's font set decides what it looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
	#[default]
	Body,		// running text
	Bold,		// running text, emphasised strongly
	Italic,		// running text, emphasised
	BoldItalic,	// running text, emphasised, and strongly
	Mono,		// preserved source, where the columns must line up
}

/// The vertical metrics of a font at a size, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
	pub ascent:	f32,	// how far the tallest letters rise above the baseline
	pub descent:	f32,	// how far the deepest fall below it, as a positive number
	pub leading:	f32,	// the gap the designer asks between one line's descent and the next's ascent
}

impl Metrics {

	/// The distance from one baseline to the next.
	pub fn line_height(&self) -> f32 {
		self.ascent + self.descent + self.leading
	}
}

/// One typeface, at any size: a single font file. Its bytes are owned and lent to both third-party
/// parsers when needed; the shaper's tables, the costly part to build, are cached.
pub struct Face {
	bytes:		Vec<u8>,		// the font file
	shaper:		ShaperData,		// the shaper's cached view, built once
	upem:		f32,			// font units per em, what every measurement in the file is in terms of
	covers:		HashSet<u32>,	// every character the face can draw, read once (asked per character)
	// Drawn glyph outlines, memoised by (glyph id, size in its raw bits). A book draws the same few
	// hundred glyphs at the same few sizes hundreds of thousands of times; re-reading the font and
	// redrawing each outline every time was the whole cost of emit. The outline is a pure function of
	// its key, so the cache changes nothing in the bytes drawn -- only how many times they are computed.
	outlines:	RwLock<HashMap<(u32, u32), Path>>,
}

impl Face {

	pub fn new(bytes: Vec<u8>) -> Outcome<Self> {
		let sf = match ShapeFont::new(&bytes) {
			Ok(f) => f,
			Err(e) => return Err(err!(
				"The {} bytes given are not a font a shaper can read: {:?}.", bytes.len(), e;
			Invalid, Input)),
		};
		let shaper = ShaperData::new(&sf);
		let of = match OutlineFont::new(&bytes) {
			Ok(f) => f,
			Err(e) => return Err(err!(
				"The {} bytes given are not a font an outline reader can read: {:?}.",
				bytes.len(), e;
			Invalid, Input)),
		};
		let upem = of.metrics(Size::unscaled(), LocationRef::default()).units_per_em as f32;
		if upem <= 0.0 {
			return Err(err!(
				"The font declares {} units per em, which cannot be scaled by.", upem;
			Invalid, Input));
		}
		let covers: HashSet<u32> = of.charmap().mappings().map(|(c, _)| c).collect();
		drop(of);
		Ok(Self {
			bytes,
			shaper,
			upem,
			covers,
			outlines:	RwLock::new(HashMap::new()),
		})
	}

	/// Can the face draw this character?
	pub fn covers(&self, ch: char) -> bool {
		self.covers.contains(&(ch as u32))
	}

	/// The font as the shaper reads it.
	fn shape_font(&self) -> Outcome<ShapeFont<'_>> {
		match ShapeFont::new(&self.bytes) {
			Ok(f) => Ok(f),
			Err(e) => Err(err!("The font could not be re-read for shaping: {:?}.", e; Bug)),
		}
	}

	/// The font as the outline reader reads it.
	fn outline_font(&self) -> Outcome<OutlineFont<'_>> {
		match OutlineFont::new(&self.bytes) {
			Ok(f) => Ok(f),
			Err(e) => Err(err!("The font could not be re-read for outlines: {:?}.", e; Bug)),
		}
	}

	/// The vertical metrics at a size, in pixels.
	pub fn metrics(&self, size: f32) -> Outcome<Metrics> {
		let of = res!(self.outline_font());
		let m = of.metrics(Size::new(size), LocationRef::default());
		Ok(Metrics {
			ascent:		m.ascent,
			descent:	m.descent.abs(),
			leading:	m.leading.max(0.0),
		})
	}

	/// Shapes a string this face can draw the whole of: the glyphs it becomes, and where each sits.
	/// `face` is which face in the chain this is, carried on every glyph so painting knows whose
	/// outline to ask for; `at` is the string's byte offset in the one it was cut from, added to each
	/// cluster so a caret reads offsets into the original text rather than into this fragment.
	pub fn shape(&self, text: &str, size: f32, dir: Dir, face: u8, at: usize) -> Outcome<Run> {
		if text.is_empty() {
			return Ok(Run {
				glyphs:		Vec::new(),
				advance:	0.0,
				size,
			});
		}
		let sf = res!(self.shape_font());
		let shaper = self.shaper.shaper(&sf).build();

		let mut buf = UnicodeBuffer::new();
		buf.push_str(text);
		buf.set_direction(match dir {
			Dir::Ltr	=> harfrust::Direction::LeftToRight,
			Dir::Rtl	=> harfrust::Direction::RightToLeft,
		});
		buf.guess_segment_properties();

		let out = shaper.shape(buf, ShapeOptions::new());
		let infos = out.glyph_infos();
		let posns = out.glyph_positions();

		// Font units become pixels here, and nowhere else.
		let scale = size / self.upem;
		let mut glyphs = Vec::with_capacity(infos.len());
		let mut pen = 0.0f32;
		for (i, p) in infos.iter().zip(posns.iter()) {
			let adv = (p.x_advance as f32) * scale;
			glyphs.push(Glyph {
				id:		i.glyph_id,
				face,
				x:		pen + (p.x_offset as f32) * scale,
				y:		(p.y_offset as f32) * scale,
				adv,
				cluster:	(i.cluster as usize) + at,
			});
			pen += adv;
		}
		Ok(Run {
			glyphs,
			advance: pen,
			size,
		})
	}

	/// The outline of one glyph at a size, in the font's frame: origin the glyph's own, y up. Painting
	/// flips it onto the page.
	pub fn outline(&self, id: u32, size: f32) -> Outcome<Path> {
		// The same glyph at the same size is drawn again and again across a book; memoise it. The key is
		// the size's raw bits, so two calls at the identical `f32` share an entry and a re-shaped run at a
		// new size (a heading, a footnote) gets its own -- no float is compared for near-equality.
		let key = (id, size.to_bits());
		{
			let cache = lock_read!(self.outlines);
			if let Some(path) = cache.get(&key) {
				return Ok(path.clone());
			}
		}

		let of = res!(self.outline_font());
		let glyphs = of.outline_glyphs();
		let glyph = match glyphs.get(GlyphId::new(id)) {
			Some(g) => g,
			None => return Err(err!(
				"The font holds no glyph {}, which shaping asked for.", id; Invalid, Input)),
		};
		let mut pen = Pen::new();
		let settings = DrawSettings::unhinted(Size::new(size), LocationRef::default());
		if let Err(e) = glyph.draw(settings, &mut pen) {
			return Err(err!("The outline of glyph {} could not be drawn: {:?}.", id, e; Invalid));
		}
		let path = res!(pen.finish());
		let mut cache = lock_write!(self.outlines);
		cache.insert(key, path.clone());
		Ok(path)
	}
}

/// Turns the outline reader's calls into one of our paths.
struct Pen {
	pb: PathBuilder,
}

impl Pen {

	fn new() -> Self {
		Self {
			pb: PathBuilder::new(),
		}
	}

	fn finish(self) -> Outcome<Path> {
		self.pb.finish()
	}
}

impl OutlinePen for Pen {

	fn move_to(&mut self, x: f32, y: f32) {
		self.pb.move_to(Pt::new(x, y));
	}

	fn line_to(&mut self, x: f32, y: f32) {
		self.pb.line_to(Pt::new(x, y));
	}

	fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
		self.pb.quad_to(Pt::new(cx, cy), Pt::new(x, y));
	}

	fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
		self.pb.cubic_to(Pt::new(cx0, cy0), Pt::new(cx1, cy1), Pt::new(x, y));
	}

	fn close(&mut self) {
		self.pb.close();
	}
}
