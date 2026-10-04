//! Shaped text: the glyphs a string becomes, and where each one sits.

/// The direction a run of text is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
	#[default]
	Ltr,	// left to right, as English is
	Rtl,	// right to left, as Arabic and Hebrew are
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

/// An OpenType feature applied across a whole run: `smcp` for small capitals, `onum` for old-style
/// figures. The value is the feature's setting -- 0 switches it off, 1 on, and a higher value picks an
/// alternate where the feature offers several.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
	pub tag:	[u8; 4],
	pub value:	u32,
}

impl Feature {

	/// The feature switched on.
	pub const fn on(tag: &[u8; 4]) -> Self {
		Self { tag: *tag, value: 1 }
	}

	/// Small capitals from lower case: what Typst's `smallcaps` asks the font for.
	pub const SMALL_CAPS: Self = Self::on(b"smcp");

	/// The feature at a given setting; 0 switches off a feature a shaper applies by default.
	pub const fn set(tag: &[u8; 4], value: u32) -> Self {
		Self { tag: *tag, value }
	}
}

/// Everything a shaping call may be told beyond the string, the size and the direction. The default is
/// what [`Face::shape_with`](crate::face::Face::shape_with) has always done: no features, the language
/// and script guessed from the text, and default-ignorable characters shaped as the font has them.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShapeSpec<'a> {
	pub features:			&'a [Feature],
	pub language:			Option<&'a str>,	// BCP 47, as `en` or `de-CH`; selects `locl` forms
	pub script:				Option<[u8; 4]>,	// ISO 15924, as `Latn`; else guessed from the text
	pub remove_ignorables:	bool,				// drop default ignorables (a soft hyphen, a joiner) from the run
}

/// One glyph, placed in pixels relative to the run's start and baseline, y up as the font has it.
/// Painting flips it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
	pub id:		u32,	// index in the font, not a character: shaping may merge or split
	pub face:	u8,	// which face of the chain drew it; a glyph index means nothing without it
	pub x:		f32,	// horizontal offset from the run's start, pixels
	pub y:		f32,	// vertical offset from the baseline, pixels, upwards
	pub adv:	f32,	// pen movement, kept not inferred: a mark's advance is nought, its x is not
	pub cluster:	usize,	// byte offset into the ORIGINAL string, so a caret moves by cluster
}

/// A run of shaped text: one string, one font, one size, one direction.
#[derive(Clone, Debug, Default)]
pub struct Run {
	pub glyphs:	Vec<Glyph>,	// in visual order
	pub advance:	f32,	// how far the pen travels over the whole run, pixels
	pub size:	f32,	// the size it was shaped at, pixels per em
}

impl Run {

	pub fn is_empty(&self) -> bool {
		self.glyphs.is_empty()
	}
}
