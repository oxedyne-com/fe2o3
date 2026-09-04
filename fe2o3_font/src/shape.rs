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
