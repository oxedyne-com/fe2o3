//! Shaped text: the glyphs a string becomes, and where each one sits.

/// The direction a run of text is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
	/// Left to right, as English is.
	#[default]
	Ltr,
	/// Right to left, as Arabic and Hebrew are.
	Rtl,
}

impl Dir {

	/// The direction named by a style's `dir` property.
	pub fn from_str(s: &str) -> Option<Self> {
		match s {
			"ltr"	=> Some(Self::Ltr),
			"rtl"	=> Some(Self::Rtl),
			_	=> None,
		}
	}
}

/// One glyph, placed.
///
/// The position is in pixels, relative to the start of the run and its baseline, with y increasing
/// upwards as the font does. Painting flips it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
	/// The glyph's index in the font, which is not a character: shaping may merge several
	/// characters into one glyph, or split one into several.
	pub id:		u32,
	/// Which face of the font's chain drew it.
	///
	/// A glyph index means nothing without the face it indexes: face 0's glyph 42 and face 1's glyph
	/// 42 are different letters. A run may hold glyphs from more than one face -- an arrow the reading
	/// face lacks is drawn by the face behind it -- so the pairing travels with the glyph rather than
	/// being remembered per run. See [`crate::font::Font`].
	pub face:	u8,
	/// Horizontal offset from the start of the run, in pixels.
	pub x:		f32,
	/// Vertical offset from the baseline, in pixels, upwards.
	pub y:		f32,
	/// How far this glyph moves the pen, in pixels.
	///
	/// This is not the same as the gap to the next glyph's `x`, because a glyph may be offset off
	/// the pen's path — a mark placed over the letter it belongs to has an advance of nothing and an
	/// `x` well to the left of it. A caret's position is a sum of advances, so the advance is kept
	/// rather than inferred, and inferring it from the neighbouring positions is exactly the error
	/// that puts a caret through the middle of an accented letter.
	pub adv:	f32,
	/// The byte offset, in the original string, of the text this glyph came from.
	///
	/// Several glyphs may share a cluster, and one glyph may span several characters, which is why
	/// a caret moves by cluster and not by glyph.
	pub cluster:	usize,
}

/// A run of shaped text: one string, one font, one size, one direction.
#[derive(Clone, Debug, Default)]
pub struct Run {
	/// The glyphs, in visual order.
	pub glyphs:	Vec<Glyph>,
	/// How far the pen travels over the whole run, in pixels.
	pub advance:	f32,
	/// The size the run was shaped at, in pixels per em.
	pub size:	f32,
}

impl Run {

	/// Whether the run puts no glyphs on the page.
	pub fn is_empty(&self) -> bool {
		self.glyphs.is_empty()
	}
}
