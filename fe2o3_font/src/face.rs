//! One typeface: parse, coverage, metrics, shaping and glyph outlines.
//!
//! This is where `harfrust` shapes and `skrifa` draws, and where both are turned back into this
//! crate's own types at once. A face is rarely used alone; what a caller draws with is a
//! [`Font`](crate::font::Font), a chain of these.

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

use std::collections::HashSet;

/// The part a font plays. A document names a role; the reader's font set decides what it looks
/// like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
	/// Running text.
	#[default]
	Body,
	/// Running text, emphasised strongly.
	Bold,
	/// Running text, emphasised.
	Italic,
	/// Running text, emphasised, and strongly.
	BoldItalic,
	/// Preserved source, where the columns must line up.
	Mono,
}

/// The vertical metrics of a font at a size, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
	/// How far the tallest letters rise above the baseline.
	pub ascent:	f32,
	/// How far the deepest letters fall below the baseline, as a positive number.
	pub descent:	f32,
	/// The gap the designer asks for between one line's descent and the next line's ascent.
	pub leading:	f32,
}

impl Metrics {

	/// The distance from one baseline to the next.
	pub fn line_height(&self) -> f32 {
		self.ascent + self.descent + self.leading
	}
}

/// One typeface, at any size: a single font file.
///
/// The font's bytes are owned, and both third-party parsers are handed a borrow of them when they
/// are needed. The shaper's tables, which are what costs anything to build, are cached.
///
/// A face is rarely used alone. What the engine draws with is a [`Font`](crate::font::Font), which
/// is a chain of these.
pub struct Face {
	/// The font file.
	bytes:	Vec<u8>,
	/// The shaper's cached view of the font, built once.
	shaper:	ShaperData,
	/// Font units per em, the number every measurement in the file is in terms of.
	upem:	f32,
	/// Every character the face can draw.
	///
	/// Read once, when the face is read, because the question "can you draw this?" is asked of every
	/// character of every string shaped, and the alternative is re-parsing the file to ask it.
	covers:	HashSet<u32>,
}

impl Face {

	/// Reads a face from the bytes of a font file.
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
		})
	}

	/// Whether the face can draw a character.
	///
	/// A face that cannot is passed over, and the next in the chain asked. A face that can is asked
	/// to, even where a later one would draw it better: the chain is an order of preference, and the
	/// first face is the preferred one by construction.
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
	///
	/// This is the deep part, and it is HarfBuzz's. What comes back is the same answer the rest of
	/// the world's software would give for the same text and the same font, which for Arabic
	/// joining or Indic reordering is not something worth being original about.
	///
	/// `face` is which face in the chain this is, which each glyph carries so that painting knows
	/// what to ask for its outline. `at` is where the string sits in the one it was cut from, which
	/// is added to every cluster: a cluster is a byte offset into the ORIGINAL text, and a face that
	/// shaped only the middle of a paragraph would otherwise report offsets into its own fragment and
	/// put every caret in the wrong place.
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

	/// The outline of one glyph, at a size, as a path.
	///
	/// The path is in the font's frame: the origin is the glyph's own, and y increases upwards.
	/// Painting flips it onto the page.
	pub fn outline(&self, id: u32, size: f32) -> Outcome<Path> {
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
		pen.finish()
	}
}

/// Turns the outline reader's calls into one of our paths.
struct Pen {
	pb: PathBuilder,
}

impl Pen {

	/// A pen with nothing drawn.
	fn new() -> Self {
		Self {
			pb: PathBuilder::new(),
		}
	}

	/// The path drawn, or the first fault met drawing it.
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
