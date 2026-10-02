//! One typeface: parse, coverage, metrics, shaping and glyph outlines.
//!
//! Where `harfrust` shapes and `skrifa` draws, both turned back into this crate's own types at once.
//! A face is rarely used alone; what a caller draws with is a [`Font`](crate::font::Font), a chain of
//! these.

use crate::shape::{
	Dir,
	Feature,
	Glyph,
	Run,
	ShapeSpec,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::prelude::*;
use oxedyne_fe2o3_graphics::pdf_font::FontProgram;

use harfrust::{
	BufferFlags,
	Feature as ShapeFeature,
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
	attribute::Style,
	raw::TableProvider,
	string::StringId,
	FontRef as OutlineFont,
	GlyphId,
	MetadataProvider,
};

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{
	Arc,
	RwLock,
};

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

/// The metrics a typesetter reads from a face beyond its ascent and descent, in font units, taken as
/// Typst takes them: the typographic ascender and descender of the `OS/2` table where the face has one
/// (whatever its `USE_TYPO_METRICS` bit says), else the `hhea` pair; the cap and x heights only where the
/// table records a positive value; the decoration lines from `OS/2` and `post`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutMetrics {
	pub units_per_em:	f32,
	pub ascender:		f32,
	pub descender:		f32,					// negative below the baseline, as the font records it
	pub cap_height:		Option<f32>,
	pub x_height:		Option<f32>,
	pub strikeout:		Option<(f32, f32)>,		// position above the baseline, thickness
	pub underline:		Option<(f32, f32)>,		// position (negative below), thickness
	pub subscript:		Option<ScriptMetrics>,
	pub superscript:	Option<ScriptMetrics>,
}

/// A face's own advice for synthesising a sub- or superscript, in font units (`OS/2`'s `ySubscript*` and
/// `ySuperscript*` fields). The offsets are as the table records them: positive `y_offset` is upwards for a
/// superscript and downwards for a subscript.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptMetrics {
	pub x_size:		f32,
	pub y_size:		f32,
	pub x_offset:	f32,
	pub y_offset:	f32,
}

/// How a face describes itself for selection by family and variant: its names, its weight, width and
/// slant classes, and the flags a fallback search compares on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceClass {
	pub family:		Option<String>,	// name ID 1, the legacy family
	pub full_name:	Option<String>,	// name ID 4
	pub postscript:	Option<String>,	// name ID 6
	pub weight:		u16,			// OS/2 usWeightClass, 400 when absent
	pub width:		u16,			// OS/2 usWidthClass, 1 to 9, 5 when absent
	pub italic:		bool,			// fsSelection bit 0
	pub oblique:	bool,			// fsSelection bit 9
	pub monospace:	bool,			// post isFixedPitch
	pub serif:		bool,			// the PANOSE family is Latin text with a serif style
	pub variable:	bool,			// the face carries an fvar table
	pub math:		bool,			// the face carries a MATH table
}

/// What a font file says about itself: the family it belongs to and where in that family it sits. This
/// is what a document's `font: "Name"` is matched against, so a face is found by the name its designer
/// gave it rather than by whatever its file happens to be called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceInfo {
	pub family:	String,	// the typographic family (name ID 16), else the legacy family (name ID 1)
	pub weight:	u16,	// OS/2 weight class, 100-900; 400 regular, 700 bold
	pub italic:	bool,	// italic or oblique
	pub index:	u32,	// which face of its file: 0 but in a collection (`.ttc`)
}

impl FaceInfo {

	/// Reads a font file's family, weight and slant without building a shaper, so a directory of fonts
	/// can be indexed cheaply before any of them is needed. A collection answers for its first face.
	pub fn read(bytes: &[u8]) -> Outcome<Self> {
		let of = match OutlineFont::from_index(bytes, 0) {
			Ok(f) => f,
			Err(e) => return Err(err!(
				"The {} bytes given are not a font whose names can be read: {:?}.", bytes.len(), e;
			Invalid, Input)),
		};
		Self::of_face(&of, 0, bytes.len())
	}

	/// Every face of a file, one for a lone font and one per face of a collection. A face naming no family
	/// is left out rather than hiding its siblings; only a file with no readable face is an error.
	///
	/// A collection's header states its own face count, and that is not believed: a face is read once for
	/// each table directory the file holds, however often the header lists it, and no more faces are read
	/// than the file has bytes for directories, so a crafted count costs no more than the file's length.
	pub fn read_all(bytes: &[u8]) -> Outcome<Vec<Self>> {
		let indices = match collection_faces(bytes) {
			Some(ix)	=> ix,
			None		=> vec![0],
		};
		let mut out = Vec::new();
		let mut last_err: Option<Error<ErrTag>> = None;
		for i in indices {
			let of = match OutlineFont::from_index(bytes, i) {
				Ok(f)	=> f,
				Err(_)	=> continue,	// a listed face the file does not hold, or no font at all
			};
			match Self::of_face(&of, i, bytes.len()) {
				Ok(info)	=> out.push(info),
				Err(e)		=> last_err = Some(e),
			}
		}
		if out.is_empty() {
			return Err(match last_err {
				Some(e)	=> e,
				None	=> err!(
					"The {} bytes given are not a font whose names can be read.", bytes.len();
				Invalid, Input),
			});
		}
		Ok(out)
	}

	fn of_face(of: &OutlineFont, index: u32, len: usize) -> Outcome<Self> {
		// The typographic family groups every weight and width under one name ("Noto Sans"), where the
		// legacy family splits them four to a family ("Noto Sans SemiBold"); prefer it where present.
		let family = of.localized_strings(StringId::TYPOGRAPHIC_FAMILY_NAME).english_or_first()
			.or_else(|| of.localized_strings(StringId::FAMILY_NAME).english_or_first())
			.map(|s| s.to_string());
		let family = match family {
			Some(f) if !f.trim().is_empty() => f.trim().to_string(),
			_ => return Err(err!(
				"Face {} of the {} byte font names no family in its name table.", index, len;
			Invalid, Input, Missing)),
		};
		let attrs = of.attributes();
		Ok(Self {
			family,
			weight:	attrs.weight.value().round().clamp(1.0, 1000.0) as u16,
			italic:	!matches!(attrs.style, Style::Normal),
			index,
		})
	}
}

/// The smallest table directory a face can have, its 12-byte header alone.
const MIN_DIRECTORY: usize = 12;

/// The faces of a collection worth reading, by index: each one whose table directory no earlier index
/// shares, and no more than the file has room for directories. `None` for a file that is not a collection.
fn collection_faces(bytes: &[u8]) -> Option<Vec<u32>> {
	if bytes.get(0..4) != Some(&b"ttcf"[..]) {
		return None;
	}
	let be32 = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
	// The offsets follow the 12-byte header, four bytes each; a count the file cannot hold is cut to what
	// it can.
	let count	= be32(8).unwrap_or(0) as usize;
	let listed	= count.min(bytes.len().saturating_sub(12) / 4);
	let room	= bytes.len() / MIN_DIRECTORY;
	let mut seen:	HashSet<u32>	= HashSet::new();
	let mut out:	Vec<u32>		= Vec::new();
	for i in 0..listed {
		if out.len() >= room {
			break;
		}
		if let Some(offset) = be32(12 + 4 * i) {
			if seen.insert(offset) {
				out.push(i as u32);
			}
		}
	}
	Some(out)
}

/// One typeface, at any size: a single font file. Its bytes are owned and lent to both third-party
/// parsers when needed; the shaper's tables, the costly part to build, are cached.
pub struct Face {
	bytes:		Arc<Vec<u8>>,	// the font file, shared with its embeddable program
	program:	Option<Arc<FontProgram>>,	// the file as a PDF embeds it; `None` when it cannot be
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
		drop(sf);
		let bytes = Arc::new(bytes);
		// A file the embedding reader cannot follow is still a face to shape and outline; it is drawn as
		// outlines in a PDF rather than embedded, so the failure is not the caller's.
		let program = FontProgram::parse(bytes.clone()).ok().flatten().map(Arc::new);
		Ok(Self {
			bytes,
			program,
			shaper,
			upem,
			covers,
			outlines:	RwLock::new(HashMap::new()),
		})
	}

	/// The face's file as a PDF embeds it, or `None` for a face that cannot be embedded -- a variable
	/// `CFF2` face, or one whose licence forbids it -- and must be drawn as outlines.
	pub fn program(&self) -> Option<&Arc<FontProgram>> {
		self.program.as_ref()
	}

	/// The family, weight and slant the file declares.
	pub fn info(&self) -> Outcome<FaceInfo> {
		FaceInfo::read(&self.bytes)
	}

	/// Can the face draw this character?
	pub fn covers(&self, ch: char) -> bool {
		self.covers.contains(&(ch as u32))
	}

	/// The font as the shaper reads it.
	fn shape_font(&self) -> Outcome<ShapeFont<'_>> {
		match ShapeFont::new(&self.bytes[..]) {
			Ok(f) => Ok(f),
			Err(e) => Err(err!("The font could not be re-read for shaping: {:?}.", e; Bug)),
		}
	}

	/// The font as the outline reader reads it.
	fn outline_font(&self) -> Outcome<OutlineFont<'_>> {
		match OutlineFont::new(&self.bytes[..]) {
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

	/// The face's units per em, what every measurement in the file is in terms of.
	pub fn units_per_em(&self) -> f32 {
		self.upem
	}

	/// The layout metrics in font units; see [`LayoutMetrics`].
	pub fn layout_metrics(&self) -> Outcome<LayoutMetrics> {
		let of = res!(self.outline_font());
		let os2 = of.os2().ok();
		let hhea = of.hhea().ok();
		let post = of.post().ok();
		let (ascender, descender) = match (&os2, &hhea) {
			(Some(t), _)		=> (t.s_typo_ascender() as f32, t.s_typo_descender() as f32),
			(None, Some(h))		=> (h.ascender().to_i16() as f32, h.descender().to_i16() as f32),
			(None, None)		=> (self.upem * 0.8, -self.upem * 0.2),
		};
		let positive = |v: Option<i16>| v.filter(|h| *h > 0).map(|h| h as f32);
		Ok(LayoutMetrics {
			units_per_em:	self.upem,
			ascender,
			descender,
			cap_height:		os2.as_ref().and_then(|t| positive(t.s_cap_height())),
			x_height:		os2.as_ref().and_then(|t| positive(t.sx_height())),
			strikeout:		os2.as_ref().map(|t| (t.y_strikeout_position() as f32, t.y_strikeout_size() as f32)),
			underline:		post.as_ref().map(|t| (
				t.underline_position().to_i16() as f32,
				t.underline_thickness().to_i16() as f32,
			)),
			subscript:		os2.as_ref().map(|t| ScriptMetrics {
				x_size:		t.y_subscript_x_size() as f32,
				y_size:		t.y_subscript_y_size() as f32,
				x_offset:	t.y_subscript_x_offset() as f32,
				y_offset:	t.y_subscript_y_offset() as f32,
			}),
			superscript:	os2.as_ref().map(|t| ScriptMetrics {
				x_size:		t.y_superscript_x_size() as f32,
				y_size:		t.y_superscript_y_size() as f32,
				x_offset:	t.y_superscript_x_offset() as f32,
				y_offset:	t.y_superscript_y_offset() as f32,
			}),
		})
	}

	/// The names, classes and flags the face is selected by; see [`FaceClass`].
	pub fn class(&self) -> Outcome<FaceClass> {
		let of = res!(self.outline_font());
		let name = |id: StringId| of.localized_strings(id).english_or_first()
			.map(|s| s.to_string())
			.filter(|s| !s.trim().is_empty());
		let os2 = of.os2().ok();
		let sel = os2.as_ref().map(|t| t.fs_selection().bits()).unwrap_or(0);
		// PANOSE is ten bytes at offset 32 of OS/2: family kind 2 is Latin text, and a serif style from 2
		// to 10 is one of the serifed forms (11 and up are the sans styles).
		let serif = of.table_data(harfrust::Tag::new(b"OS/2"))
			.and_then(|d| d.as_bytes().get(32..34).map(|p| p[0] == 2 && (2..=10).contains(&p[1])))
			.unwrap_or(false);
		Ok(FaceClass {
			family:		name(StringId::FAMILY_NAME),
			full_name:	name(StringId::FULL_NAME),
			postscript:	name(StringId::POSTSCRIPT_NAME),
			weight:		os2.as_ref().map(|t| t.us_weight_class()).unwrap_or(400),
			width:		os2.as_ref().map(|t| t.us_width_class()).filter(|w| (1..=9).contains(w)).unwrap_or(5),
			italic:		sel & 0x0001 != 0,
			oblique:	sel & 0x0200 != 0,
			monospace:	of.post().map(|t| t.is_fixed_pitch() != 0).unwrap_or(false),
			serif,
			variable:	of.table_data(harfrust::Tag::new(b"fvar")).is_some(),
			math:		of.table_data(harfrust::Tag::new(b"MATH")).is_some(),
		})
	}

	/// Every character the face maps, in no set order.
	pub fn coverage(&self) -> impl Iterator<Item = u32> + '_ {
		self.covers.iter().copied()
	}

	/// The glyph the face maps a character to, if it maps it.
	pub fn glyph_index(&self, ch: char) -> Option<u32> {
		let of = self.outline_font().ok()?;
		of.charmap().map(ch).map(|g| g.to_u32())
	}

	/// A glyph's advance width in font units, from `hmtx`.
	pub fn advance_units(&self, id: u32) -> Option<f32> {
		let of = self.outline_font().ok()?;
		of.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(GlyphId::new(id))
	}

	/// Shapes a string this face can draw the whole of: the glyphs it becomes, and where each sits.
	/// `face` is which face in the chain this is, carried on every glyph so painting knows whose
	/// outline to ask for; `at` is the string's byte offset in the one it was cut from, added to each
	/// cluster so a caret reads offsets into the original text rather than into this fragment.
	pub fn shape(&self, text: &str, size: f32, dir: Dir, face: u8, at: usize) -> Outcome<Run> {
		self.shape_with(text, size, dir, face, at, &[])
	}

	/// As [`Face::shape`], with OpenType features applied across the whole string.
	pub fn shape_with(
		&self,
		text:		&str,
		size:		f32,
		dir:		Dir,
		face:		u8,
		at:			usize,
		features:	&[Feature],
	)
		-> Outcome<Run>
	{
		self.shape_spec(text, size, dir, face, at, &ShapeSpec { features, ..ShapeSpec::default() })
	}

	/// As [`Face::shape`], told the features, language and script to shape with, and whether to drop
	/// default-ignorable characters; see [`ShapeSpec`]. A language or script the shaper does not know is
	/// left to be guessed, as an unset one is.
	pub fn shape_spec(
		&self,
		text:	&str,
		size:	f32,
		dir:	Dir,
		face:	u8,
		at:		usize,
		spec:	&ShapeSpec,
	)
		-> Outcome<Run>
	{
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
		if let Some(lang) = spec.language {
			if let Ok(l) = lang.parse::<harfrust::Language>() {
				buf.set_language(l);
			}
		}
		if let Some(tag) = spec.script {
			if let Some(sc) = harfrust::Script::from_iso15924_tag(harfrust::Tag::new(&tag)) {
				buf.set_script(sc);
			}
		}
		buf.set_direction(match dir {
			Dir::Ltr	=> harfrust::Direction::LeftToRight,
			Dir::Rtl	=> harfrust::Direction::RightToLeft,
		});
		buf.guess_segment_properties();
		if spec.remove_ignorables {
			buf.set_flags(BufferFlags::REMOVE_DEFAULT_IGNORABLES);
		}

		let feats: Vec<ShapeFeature> = spec.features.iter()
			.map(|f| ShapeFeature::new(harfrust::Tag::new(&f.tag), f.value, ..))
			.collect();
		let out = shaper.shape(buf, ShapeOptions::new().features(&feats));
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

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_graphics::pdf_font::{
		collection_face,
		collection_of,
	};

	const NOTO_SANS:	&[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");
	const DEJAVU_MONO:	&[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

	#[test]
	fn read_reads_the_family_weight_and_slant_from_the_name_table() -> Outcome<()> {
		let info = res!(FaceInfo::read(NOTO_SANS));
		assert_eq!(info, FaceInfo { family: "Noto Sans".to_string(), weight: 400, italic: false, index: 0 });
		assert_eq!(res!(FaceInfo::read_all(NOTO_SANS)), vec![info], "a lone font is one face");
		Ok(())
	}

	/// Every face a collection lists can be cut out and built into a face that shapes, so a family read
	/// from a `.ttc` is one a document can be set in.
	#[test]
	fn every_face_of_a_collection_is_read_and_can_be_built() -> Outcome<()> {
		let ttc = res!(collection_of(&[NOTO_SANS, DEJAVU_MONO]));
		let all = res!(FaceInfo::read_all(&ttc));
		let names: Vec<(&str, u32)> = all.iter().map(|i| (i.family.as_str(), i.index)).collect();
		assert_eq!(names, vec![("Noto Sans", 0), ("DejaVu Sans Mono", 1)]);
		assert_eq!(res!(FaceInfo::read(&ttc)), all[0], "a collection answers `read` for its first face");
		assert!(Face::new(ttc.clone()).is_err(), "a collection is not itself one face");
		for info in &all {
			let face = res!(Face::new(res!(collection_face(&ttc, info.index as usize))));
			let got = res!(face.info());
			assert_eq!((got.family.as_str(), got.index), (info.family.as_str(), 0));
			assert!(res!(face.shape("Hamburgefonts", 12.0, Dir::Ltr, 0, 0)).glyphs.len() > 0);
			assert!(face.program().is_some(), "face {} embeds in a PDF", info.index);
		}
		Ok(())
	}

	/// A collection's header count is not believed: a crafted file listing one table directory a hundred
	/// thousand times holds one face, and it is read once.
	#[test]
	fn a_collection_listing_one_directory_many_times_is_one_face() -> Outcome<()> {
		let n		= 100_000usize;
		let head	= 12 + 4 * n;
		// The lone font, moved to sit after the header, its table offsets moved with it.
		let mut font = NOTO_SANS.to_vec();
		let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
		for t in 0..tables {
			let at	= 12 + 16 * t + 8;
			let off	= u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) + head as u32;
			font[at..at + 4].copy_from_slice(&off.to_be_bytes());
		}
		let mut ttc: Vec<u8> = Vec::with_capacity(head + font.len());
		ttc.extend_from_slice(b"ttcf");
		ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
		ttc.extend_from_slice(&(n as u32).to_be_bytes());
		for _ in 0..n {
			ttc.extend_from_slice(&(head as u32).to_be_bytes());
		}
		ttc.extend_from_slice(&font);
		let all = res!(FaceInfo::read_all(&ttc));
		let names: Vec<(&str, u32)> = all.iter().map(|i| (i.family.as_str(), i.index)).collect();
		assert_eq!(names, vec![("Noto Sans", 0)], "one directory is one face");
		Ok(())
	}

	/// A real collection, where the machine has one: Debian's `fonts-noto-cjk`, too large to check in.
	#[test]
	fn a_system_collection_lists_every_face() -> Outcome<()> {
		let path = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc";
		let bytes = match std::fs::read(path) {
			Ok(b)	=> b,
			Err(_)	=> {
				eprintln!("SKIP: {} is not installed, so the .ttc case cannot run.", path);
				return Ok(());
			},
		};
		let all = res!(FaceInfo::read_all(&bytes));
		assert!(all.len() > 1, "several regional faces: {:?}", all);
		for (i, info) in all.iter().enumerate() {
			assert_eq!(info.index as usize, i);
			assert!(!info.family.trim().is_empty(), "every face names a family: {:?}", all);
		}
		Ok(())
	}
}
