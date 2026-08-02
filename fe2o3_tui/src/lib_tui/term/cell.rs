//! The contents of one screen cell: a character, the pen it was drawn with, and how many cells it
//! claims.
//!
//! A [`Cell`] is small on purpose. A screen of 200 columns with ten thousand lines of scrollback is
//! two million cells, so every byte in the struct is multiplied by two million. The pen is
//! therefore packed into a copyable value rather than being reference counted or boxed, and the
//! attribute set is a bit field rather than a collection.

use crate::lib_tui::style::Colour;

use oxedyne_fe2o3_core::prelude::*;


/// One of the sixteen colours an ANSI terminal names.
///
/// The first eight are the original ANSI colours and the second eight their bright variants, which
/// SGR reaches either as `90`--`97` or, historically, by pairing bold with a normal colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamedColour {
	Black,
	Red,
	Green,
	Yellow,
	Blue,
	Magenta,
	Cyan,
	White,
	BrightBlack,
	BrightRed,
	BrightGreen,
	BrightYellow,
	BrightBlue,
	BrightMagenta,
	BrightCyan,
	BrightWhite,
}

impl NamedColour {
	/// The colour's index in the first sixteen entries of the 256 colour palette.
	pub fn index(&self) -> u8 {
		match self {
			Self::Black		=> 0,
			Self::Red		=> 1,
			Self::Green		=> 2,
			Self::Yellow		=> 3,
			Self::Blue		=> 4,
			Self::Magenta		=> 5,
			Self::Cyan		=> 6,
			Self::White		=> 7,
			Self::BrightBlack	=> 8,
			Self::BrightRed		=> 9,
			Self::BrightGreen	=> 10,
			Self::BrightYellow	=> 11,
			Self::BrightBlue	=> 12,
			Self::BrightMagenta	=> 13,
			Self::BrightCyan	=> 14,
			Self::BrightWhite	=> 15,
		}
	}

	/// The named colour for a palette index below sixteen, or `None` above it.
	pub fn from_index(i: u8) -> Option<Self> {
		match i {
			0	=> Some(Self::Black),
			1	=> Some(Self::Red),
			2	=> Some(Self::Green),
			3	=> Some(Self::Yellow),
			4	=> Some(Self::Blue),
			5	=> Some(Self::Magenta),
			6	=> Some(Self::Cyan),
			7	=> Some(Self::White),
			8	=> Some(Self::BrightBlack),
			9	=> Some(Self::BrightRed),
			10	=> Some(Self::BrightGreen),
			11	=> Some(Self::BrightYellow),
			12	=> Some(Self::BrightBlue),
			13	=> Some(Self::BrightMagenta),
			14	=> Some(Self::BrightCyan),
			15	=> Some(Self::BrightWhite),
			_	=> None,
		}
	}

	/// The bright variant of a colour, or the colour itself if it is already bright.
	pub fn brighten(&self) -> Self {
		match Self::from_index(self.index() | 0x08) {
			Some(c)	=> c,
			None	=> *self,
		}
	}
}

/// A cell colour, in any of the three forms a terminal can express.
///
/// `Default` means the colour the renderer uses when nothing has been selected, which is not the
/// same as any particular named colour; a foreground default and a background default differ, and
/// only the renderer knows what they are.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TermColour {
	/// The renderer's own foreground or background colour.
	Default,
	/// One of the sixteen named colours.
	Named(NamedColour),
	/// An index into the 256 colour palette.
	Indexed(u8),
	/// A direct 24 bit colour.
	Rgb(u8, u8, u8),
}

impl Default for TermColour {
	fn default() -> Self {
		Self::Default
	}
}

impl From<TermColour> for Colour {
	fn from(c: TermColour) -> Self {
		match c {
			TermColour::Default	=> Colour::Reset,
			TermColour::Named(n)	=> match n {
				NamedColour::Black		=> Colour::Black,
				NamedColour::Red		=> Colour::Red,
				NamedColour::Green		=> Colour::Green,
				NamedColour::Yellow		=> Colour::Yellow,
				NamedColour::Blue		=> Colour::Blue,
				NamedColour::Magenta		=> Colour::Magenta,
				NamedColour::Cyan		=> Colour::Cyan,
				NamedColour::White		=> Colour::Gray,
				NamedColour::BrightBlack	=> Colour::DarkGray,
				NamedColour::BrightRed		=> Colour::LightRed,
				NamedColour::BrightGreen	=> Colour::LightGreen,
				NamedColour::BrightYellow	=> Colour::LightYellow,
				NamedColour::BrightBlue		=> Colour::LightBlue,
				NamedColour::BrightMagenta	=> Colour::LightMagenta,
				NamedColour::BrightCyan		=> Colour::LightCyan,
				NamedColour::BrightWhite	=> Colour::White,
			},
			TermColour::Indexed(i)	=> Colour::Indexed(i),
			TermColour::Rgb(r, g, b)	=> Colour::Rgb(r, g, b),
		}
	}
}

/// The graphic attribute bits, packed into one word.
///
/// The set is deliberately narrow: these are the attributes a renderer can be expected to honour.
/// Sequences selecting anything outside it are parsed and discarded rather than stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Attrs(u16);

/// Increased intensity.
pub const ATTR_BOLD: u16	= 1 << 0;
/// Decreased intensity.
pub const ATTR_DIM: u16		= 1 << 1;
/// Italic.
pub const ATTR_ITALIC: u16	= 1 << 2;
/// Single underline.
pub const ATTR_UNDERLINE: u16	= 1 << 3;
/// Blink.
pub const ATTR_BLINK: u16	= 1 << 4;
/// Foreground and background exchanged.
pub const ATTR_REVERSE: u16	= 1 << 5;
/// Not drawn at all.
pub const ATTR_HIDDEN: u16	= 1 << 6;
/// Struck through.
pub const ATTR_STRIKE: u16	= 1 << 7;

impl Attrs {
	/// An empty attribute set.
	pub fn none() -> Self {
		Self(0)
	}

	/// Whether every bit in `bits` is set.
	pub fn has(&self, bits: u16) -> bool {
		self.0 & bits == bits
	}

	/// Sets the given bits.
	pub fn set(&mut self, bits: u16) {
		self.0 |= bits;
	}

	/// Clears the given bits.
	pub fn clear(&mut self, bits: u16) {
		self.0 &= !bits;
	}

	/// The raw bit field, for a renderer that wants to compare two pens cheaply.
	pub fn bits(&self) -> u16 {
		self.0
	}

	/// Whether no attribute at all is set.
	pub fn is_empty(&self) -> bool {
		self.0 == 0
	}

	/// Bold.
	pub fn bold(&self) -> bool {
		self.has(ATTR_BOLD)
	}

	/// Dim.
	pub fn dim(&self) -> bool {
		self.has(ATTR_DIM)
	}

	/// Italic.
	pub fn italic(&self) -> bool {
		self.has(ATTR_ITALIC)
	}

	/// Underlined.
	pub fn underline(&self) -> bool {
		self.has(ATTR_UNDERLINE)
	}

	/// Blinking.
	pub fn blink(&self) -> bool {
		self.has(ATTR_BLINK)
	}

	/// Reversed.
	pub fn reverse(&self) -> bool {
		self.has(ATTR_REVERSE)
	}

	/// Hidden.
	pub fn hidden(&self) -> bool {
		self.has(ATTR_HIDDEN)
	}

	/// Struck through.
	pub fn strike(&self) -> bool {
		self.has(ATTR_STRIKE)
	}
}

/// The colours and attributes a character is drawn with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pen {
	/// Foreground colour.
	pub fore: TermColour,
	/// Background colour.
	pub back: TermColour,
	/// Graphic attributes.
	pub attrs: Attrs,
}

impl Pen {
	/// The pen a reset leaves behind: default colours, no attributes.
	pub fn plain() -> Self {
		Self::default()
	}

	/// Whether this is the pen a reset leaves behind.
	pub fn is_plain(&self) -> bool {
		*self == Self::default()
	}

	/// The foreground and background as the renderer should draw them, with reverse video already
	/// applied so that a caller need not think about it.
	pub fn resolved(&self) -> (TermColour, TermColour) {
		if self.attrs.reverse() {
			(self.back, self.fore)
		} else {
			(self.fore, self.back)
		}
	}
}

/// How a cell relates to a character that is wider than one cell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Wide {
	/// An ordinary single width cell.
	#[default]
	No,
	/// The left half of a double width character; the character is in this cell.
	Lead,
	/// The right half of a double width character; the cell holds no character of its own.
	Trail,
}

/// One cell of the grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
	/// The character drawn in this cell. A [`Wide::Trail`] cell holds a space.
	pub chr: char,
	/// The colours and attributes.
	pub pen: Pen,
	/// Whether the cell is half of a double width character.
	pub wide: Wide,
}

impl Default for Cell {
	fn default() -> Self {
		Self {
			chr:	' ',
			pen:	Pen::default(),
			wide:	Wide::No,
		}
	}
}

impl Cell {
	/// A blank cell drawn with the given pen.
	///
	/// Erasure uses the current pen rather than the default one, because that is what a terminal
	/// does: after selecting a background colour, an erase paints that colour.
	pub fn blank(pen: Pen) -> Self {
		Self {
			chr:	' ',
			pen,
			wide:	Wide::No,
		}
	}

	/// Whether the cell holds nothing but a space in the default pen.
	pub fn is_blank(&self) -> bool {
		self.chr == ' ' && self.pen.is_plain() && self.wide == Wide::No
	}
}

/// A run of adjacent cells sharing one pen, which is the unit a renderer most cheaply emits.
#[derive(Clone, Debug)]
pub struct Run {
	/// Column at which the run starts.
	pub col: usize,
	/// The shared pen.
	pub pen: Pen,
	/// The text of the run, with the right hand halves of wide characters omitted.
	pub text: String,
	/// The number of cells the run covers, which exceeds the character count when it holds wide
	/// characters.
	pub cells: usize,
}

/// Splits a row of cells into runs of constant pen.
///
/// Trailing blank cells in the default pen are dropped, since a renderer that has cleared its
/// background has nothing to draw for them.
pub fn runs(row: &[Cell]) -> Outcome<Vec<Run>> {
	let mut end = row.len();
	while end > 0 && row[end - 1].is_blank() {
		end -= 1;
	}
	let mut out: Vec<Run> = Vec::new();
	let mut col = 0;
	while col < end {
		let cell = row[col];
		if cell.wide == Wide::Trail {
			// A trailing half with no lead before it, which can only arise from a resize that
			// cut the character in two. Treat it as a space.
			match out.last_mut() {
				Some(run) if run.pen == cell.pen => {
					run.text.push(' ');
					run.cells += 1;
				}
				_ => out.push(Run {
					col,
					pen:	cell.pen,
					text:	fmt!(" "),
					cells:	1,
				}),
			}
			col += 1;
			continue;
		}
		let step = if cell.wide == Wide::Lead { 2 } else { 1 };
		match out.last_mut() {
			Some(run) if run.pen == cell.pen => {
				run.text.push(cell.chr);
				run.cells += step;
			}
			_ => out.push(Run {
				col,
				pen:	cell.pen,
				text:	cell.chr.to_string(),
				cells:	step,
			}),
		}
		col += step;
	}
	Ok(out)
}
