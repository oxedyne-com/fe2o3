//! The number of cells a character occupies on a terminal screen.
//!
//! A terminal grid is addressed in cells, not in characters, so before a character can be placed
//! the model must know how many cells it will take. Three answers are possible: none, for a
//! combining mark or a format control that hangs off the character before it; one, for the great
//! majority; and two, for the East Asian wide and fullwidth forms and for most emoji.
//!
//! The ranges below are a hand maintained condensation of the Unicode `EastAsianWidth.txt`
//! property values `W` and `F`, together with the common zero width blocks. They are deliberately
//! kept small and self contained. The generated Unicode tables in `oxedyne_fe2o3_text::unicode`
//! are the better long term home for this data, since its generator already downloads
//! `EastAsianWidth.txt` in order to build the line breaking table; see the note in the module
//! documentation of [`super`].

/// How many cells a character occupies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharWidth {
	/// A combining mark or format control, which attaches to the preceding cell.
	Zero,
	/// One cell, the common case.
	Narrow,
	/// Two cells, for East Asian wide and fullwidth forms and most emoji.
	Wide,
}

impl CharWidth {
	/// The width as a cell count.
	pub fn cells(&self) -> usize {
		match self {
			Self::Zero	=> 0,
			Self::Narrow	=> 1,
			Self::Wide	=> 2,
		}
	}
}

/// Returns the number of cells `c` occupies.
///
/// Control characters answer [`CharWidth::Zero`]; they never reach the grid, since the parser
/// intercepts them, but a caller measuring an arbitrary string should not be told they are wide.
pub fn char_width(c: char) -> CharWidth {
	let u = c as u32;
	if u < 0x20 || (0x7F..0xA0).contains(&u) {
		return CharWidth::Zero;
	}
	if in_ranges(u, ZERO_WIDTH) {
		return CharWidth::Zero;
	}
	if in_ranges(u, WIDE) {
		return CharWidth::Wide;
	}
	CharWidth::Narrow
}

/// Returns the total cell width of a string.
pub fn str_width(s: &str) -> usize {
	let mut n = 0;
	for c in s.chars() {
		n += char_width(c).cells();
	}
	n
}

/// Binary searches a sorted, non overlapping range table.
fn in_ranges(u: u32, table: &[(u32, u32)]) -> bool {
	let mut lo = 0;
	let mut hi = table.len();
	while lo < hi {
		let mid = (lo + hi) / 2;
		let (a, b) = table[mid];
		if u < a {
			hi = mid;
		} else if u > b {
			lo = mid + 1;
		} else {
			return true;
		}
	}
	false
}

/// Combining marks, variation selectors and the zero width format controls.
static ZERO_WIDTH: &[(u32, u32)] = &[
	(0x0300, 0x036F),	// Combining diacritical marks
	(0x0483, 0x0489),	// Cyrillic
	(0x0591, 0x05BD),	// Hebrew points
	(0x05BF, 0x05BF),
	(0x05C1, 0x05C2),
	(0x05C4, 0x05C5),
	(0x05C7, 0x05C7),
	(0x0610, 0x061A),	// Arabic
	(0x064B, 0x065F),
	(0x0670, 0x0670),
	(0x06D6, 0x06DC),
	(0x06DF, 0x06E4),
	(0x06E7, 0x06E8),
	(0x06EA, 0x06ED),
	(0x0711, 0x0711),	// Syriac
	(0x0730, 0x074A),
	(0x07A6, 0x07B0),	// Thaana
	(0x07EB, 0x07F3),
	(0x0816, 0x0819),	// Samaritan
	(0x081B, 0x0823),
	(0x0825, 0x0827),
	(0x0829, 0x082D),
	(0x0859, 0x085B),	// Mandaic
	(0x08D3, 0x08E1),	// Arabic extended
	(0x08E3, 0x0902),
	(0x093A, 0x093A),	// Devanagari
	(0x093C, 0x093C),
	(0x0941, 0x0948),
	(0x094D, 0x094D),
	(0x0951, 0x0957),
	(0x0962, 0x0963),
	(0x0E31, 0x0E31),	// Thai
	(0x0E34, 0x0E3A),
	(0x0E47, 0x0E4E),
	(0x1AB0, 0x1AFF),	// Combining diacritical marks extended
	(0x1DC0, 0x1DFF),	// Combining diacritical marks supplement
	(0x200B, 0x200F),	// Zero width space, joiners, directional marks
	(0x202A, 0x202E),	// Directional embedding and override
	(0x2060, 0x2064),	// Word joiner and invisible operators
	(0x206A, 0x206F),
	(0x20D0, 0x20F0),	// Combining marks for symbols
	(0xFE00, 0xFE0F),	// Variation selectors
	(0xFE20, 0xFE2F),	// Combining half marks
	(0xFEFF, 0xFEFF),	// Zero width no break space
	(0xFFF9, 0xFFFB),	// Interlinear annotation
	(0xE0100, 0xE01EF),	// Variation selectors supplement
];

/// East Asian wide and fullwidth forms, and the emoji that render two cells wide.
static WIDE: &[(u32, u32)] = &[
	(0x1100, 0x115F),	// Hangul jamo initial
	(0x231A, 0x231B),
	(0x2329, 0x232A),
	(0x23E9, 0x23EC),
	(0x23F0, 0x23F0),
	(0x23F3, 0x23F3),
	(0x25FD, 0x25FE),
	(0x2614, 0x2615),
	(0x2648, 0x2653),
	(0x267F, 0x267F),
	(0x2693, 0x2693),
	(0x26A1, 0x26A1),
	(0x26AA, 0x26AB),
	(0x26BD, 0x26BE),
	(0x26C4, 0x26C5),
	(0x26CE, 0x26CE),
	(0x26D4, 0x26D4),
	(0x26EA, 0x26EA),
	(0x26F2, 0x26F3),
	(0x26F5, 0x26F5),
	(0x26FA, 0x26FA),
	(0x26FD, 0x26FD),
	(0x2705, 0x2705),
	(0x270A, 0x270B),
	(0x2728, 0x2728),
	(0x274C, 0x274C),
	(0x274E, 0x274E),
	(0x2753, 0x2755),
	(0x2757, 0x2757),
	(0x2795, 0x2797),
	(0x27B0, 0x27B0),
	(0x27BF, 0x27BF),
	(0x2B1B, 0x2B1C),
	(0x2B50, 0x2B50),
	(0x2B55, 0x2B55),
	(0x2E80, 0x2E99),	// CJK radicals supplement
	(0x2E9B, 0x2EF3),
	(0x2F00, 0x2FD5),	// Kangxi radicals
	(0x2FF0, 0x2FFB),	// Ideographic description
	(0x3000, 0x303E),	// CJK symbols and punctuation
	(0x3041, 0x3096),	// Hiragana
	(0x3099, 0x30FF),	// Katakana
	(0x3105, 0x312F),	// Bopomofo
	(0x3131, 0x318E),	// Hangul compatibility jamo
	(0x3190, 0x31E3),
	(0x31F0, 0x321E),
	(0x3220, 0x3247),
	(0x3250, 0x4DBF),
	(0x4E00, 0xA48C),	// CJK unified ideographs
	(0xA490, 0xA4C6),
	(0xA960, 0xA97C),	// Hangul jamo extended A
	(0xAC00, 0xD7A3),	// Hangul syllables
	(0xF900, 0xFAFF),	// CJK compatibility ideographs
	(0xFE10, 0xFE19),	// Vertical forms
	(0xFE30, 0xFE52),	// CJK compatibility forms
	(0xFE54, 0xFE66),
	(0xFE68, 0xFE6B),
	(0xFF01, 0xFF60),	// Fullwidth forms
	(0xFFE0, 0xFFE6),
	(0x16FE0, 0x16FE4),
	(0x16FF0, 0x16FF1),
	(0x17000, 0x187F7),	// Tangut
	(0x18800, 0x18CD5),
	(0x18D00, 0x18D08),
	(0x1B000, 0x1B152),	// Kana supplement
	(0x1B164, 0x1B167),
	(0x1B170, 0x1B2FB),	// Nushu
	(0x1F004, 0x1F004),
	(0x1F0CF, 0x1F0CF),
	(0x1F18E, 0x1F18E),
	(0x1F191, 0x1F19A),
	(0x1F200, 0x1F320),
	(0x1F32D, 0x1F335),
	(0x1F337, 0x1F37C),
	(0x1F37E, 0x1F393),
	(0x1F3A0, 0x1F3CA),
	(0x1F3CF, 0x1F3D3),
	(0x1F3E0, 0x1F3F0),
	(0x1F3F4, 0x1F3F4),
	(0x1F3F8, 0x1F43E),
	(0x1F440, 0x1F440),
	(0x1F442, 0x1F4FC),
	(0x1F4FF, 0x1F53D),
	(0x1F54B, 0x1F54E),
	(0x1F550, 0x1F567),
	(0x1F57A, 0x1F57A),
	(0x1F595, 0x1F596),
	(0x1F5A4, 0x1F5A4),
	(0x1F5FB, 0x1F64F),
	(0x1F680, 0x1F6C5),
	(0x1F6CC, 0x1F6CC),
	(0x1F6D0, 0x1F6D2),
	(0x1F6D5, 0x1F6D7),
	(0x1F6EB, 0x1F6EC),
	(0x1F6F4, 0x1F6FC),
	(0x1F7E0, 0x1F7EB),
	(0x1F90C, 0x1F93A),
	(0x1F93C, 0x1F945),
	(0x1F947, 0x1F978),
	(0x1F97A, 0x1F9CB),
	(0x1F9CD, 0x1F9FF),
	(0x1FA70, 0x1FA74),
	(0x1FA78, 0x1FA7A),
	(0x1FA80, 0x1FA86),
	(0x1FA90, 0x1FAA8),
	(0x1FAB0, 0x1FAB6),
	(0x1FAC0, 0x1FAC2),
	(0x1FAD0, 0x1FAD6),
	(0x20000, 0x2FFFD),	// CJK extension B onwards
	(0x30000, 0x3FFFD),
];
