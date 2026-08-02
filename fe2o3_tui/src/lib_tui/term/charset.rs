//! The character sets a designation escape puts in front of the printable ASCII range.
//!
//! An application that draws a box does not send `┌`. It sends `ESC ( 0`, then `l`, and expects the
//! terminal to understand that `l` now means the top left corner. This is the DEC special graphics
//! set, and it is how every curses programme has drawn a line since the VT100; `ncurses` emits it
//! for `ACS_ULCORNER` whatever the locale, so a terminal that ignores the designation shows
//! `lqqqqk` where the top of a dialogue box belongs.
//!
//! Four slots, G0 to G3, each hold a designated set, and one of them at a time is mapped over the
//! printable ASCII range. `SI` maps G0 and `SO` maps G1, which is the pair a curses programme
//! actually uses. The designation escapes are `ESC (` for G0, `ESC )` for G1, `ESC *` for G2 and
//! `ESC +` for G3, each followed by a byte naming the set: `0` for the special graphics and `B` for
//! ASCII.
//!
//! ## Where the table came from
//!
//! Every mapping below was read out of tmux 3.6. The sequence `ESC ( 0` followed by every byte from
//! 0x20 to 0x7E was fed to a tmux pane whose output went to a pseudoterminal, and the UTF-8 tmux
//! wrote to that pseudoterminal was decoded character by character. Thirty six of the ninety five
//! came back changed; those thirty six are the table, and the other fifty nine are why [`Charset::map`]
//! returns its argument unchanged by default.
//!
//! Two of tmux's answers are worth stating because a reader may expect otherwise. `_` is *not*
//! mapped: the VT100 manual calls position 5/15 a blank and xterm draws a space there, but tmux
//! leaves the underscore alone, and the oracle is what is followed here. And `ESC ( A`, the United
//! Kingdom set in which `#` becomes `£`, is not implemented by tmux at all, so it is treated here as
//! ASCII rather than guessed at.

/// One of the character sets an escape sequence can designate into G0 to G3.
///
/// The set is deliberately narrow. A designation naming anything else is accepted and treated as
/// ASCII, which is what leaves an unrecognised set harmless: the bytes are printed as they arrived
/// rather than being dropped or substituted for something invented here.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Charset {
	/// Plain ASCII, designated by `B` and by every byte this model does not know.
	#[default]
	Ascii,
	/// The DEC special graphics and line drawing set, designated by `0`.
	DecSpecial,
}

impl Charset {

	/// The set a designation byte names.
	///
	/// The byte is the one after `ESC (`, `ESC )`, `ESC *` or `ESC +`.
	pub fn from_designator(b: u8) -> Self {
		match b {
			b'0'	=> Self::DecSpecial,
			_	=> Self::Ascii,
		}
	}

	/// Whether this set changes anything, which lets a caller skip the mapping altogether.
	pub fn is_ascii(&self) -> bool {
		matches!(self, Self::Ascii)
	}

	/// The character `c` stands for in this set.
	///
	/// Only the printable ASCII range is mapped. A character that arrived as UTF-8 is outside the
	/// range a designation covers and passes through untouched, which is what tmux does and what
	/// keeps a programme that mixes the two from losing its accented letters.
	pub fn map(&self, c: char) -> char {
		match self {
			Self::Ascii		=> c,
			Self::DecSpecial	=> dec_special(c),
		}
	}
}

/// The DEC special graphics character `c` stands for, or `c` itself where the set agrees with ASCII.
///
/// Read out of tmux 3.6; see the module documentation for the method.
fn dec_special(c: char) -> char {
	match c {
		'+'	=> '\u{2192}',	// →  rightwards arrow
		','	=> '\u{2190}',	// ←  leftwards arrow
		'-'	=> '\u{2191}',	// ↑  upwards arrow
		'.'	=> '\u{2193}',	// ↓  downwards arrow
		'0'	=> '\u{25AE}',	// ▮  black vertical rectangle
		'`'	=> '\u{25C6}',	// ◆  black diamond
		'a'	=> '\u{2592}',	// ▒  medium shade
		'b'	=> '\u{2409}',	// ␉  symbol for horizontal tabulation
		'c'	=> '\u{240C}',	// ␌  symbol for form feed
		'd'	=> '\u{240D}',	// ␍  symbol for carriage return
		'e'	=> '\u{240A}',	// ␊  symbol for line feed
		'f'	=> '\u{00B0}',	// °  degree sign
		'g'	=> '\u{00B1}',	// ±  plus minus sign
		'h'	=> '\u{2424}',	// ␤  symbol for newline
		'i'	=> '\u{240B}',	// ␋  symbol for vertical tabulation
		'j'	=> '\u{2518}',	// ┘  box drawings light up and left
		'k'	=> '\u{2510}',	// ┐  box drawings light down and left
		'l'	=> '\u{250C}',	// ┌  box drawings light down and right
		'm'	=> '\u{2514}',	// └  box drawings light up and right
		'n'	=> '\u{253C}',	// ┼  box drawings light vertical and horizontal
		'o'	=> '\u{23BA}',	// ⎺  horizontal scan line 1
		'p'	=> '\u{23BB}',	// ⎻  horizontal scan line 3
		'q'	=> '\u{2500}',	// ─  box drawings light horizontal
		'r'	=> '\u{23BC}',	// ⎼  horizontal scan line 7
		's'	=> '\u{23BD}',	// ⎽  horizontal scan line 9
		't'	=> '\u{251C}',	// ├  box drawings light vertical and right
		'u'	=> '\u{2524}',	// ┤  box drawings light vertical and left
		'v'	=> '\u{2534}',	// ┴  box drawings light up and horizontal
		'w'	=> '\u{252C}',	// ┬  box drawings light down and horizontal
		'x'	=> '\u{2502}',	// │  box drawings light vertical
		'y'	=> '\u{2264}',	// ≤  less than or equal to
		'z'	=> '\u{2265}',	// ≥  greater than or equal to
		'{'	=> '\u{03C0}',	// π  greek small letter pi
		'|'	=> '\u{2260}',	// ≠  not equal to
		'}'	=> '\u{00A3}',	// £  pound sign
		'~'	=> '\u{00B7}',	// ·  middle dot
		other	=> other,
	}
}

/// The four designated sets and which of them is mapped over the printable ASCII range.
///
/// A terminal holds one of these. `SI` and `SO` move [`Charsets::shift`]; the designation escapes
/// replace one of the four sets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Charsets {
	/// G0 to G3.
	sets:	[Charset; 4],
	/// Which of the four is mapped over the printable range: zero after `SI`, one after `SO`.
	shift:	usize,
}

impl Charsets {

	/// The state a reset leaves behind: ASCII throughout, with G0 in front.
	pub fn new() -> Self {
		Self::default()
	}

	/// Puts `set` into slot `g`, which must be zero to three.
	pub fn designate(&mut self, g: usize, set: Charset) {
		if let Some(slot) = self.sets.get_mut(g) {
			*slot = set;
		}
	}

	/// The set in slot `g`, or ASCII if `g` is not a slot.
	pub fn designated(&self, g: usize) -> Charset {
		self.sets.get(g).copied().unwrap_or(Charset::Ascii)
	}

	/// Maps slot `g` over the printable ASCII range, which is what `SI` and `SO` do.
	pub fn shift_to(&mut self, g: usize) {
		if g < self.sets.len() {
			self.shift = g;
		}
	}

	/// Which slot is mapped over the printable ASCII range.
	pub fn shift(&self) -> usize {
		self.shift
	}

	/// The set currently in front.
	pub fn active(&self) -> Charset {
		self.designated(self.shift)
	}

	/// The character `c` stands for under the set currently in front.
	pub fn map(&self, c: char) -> char {
		self.active().map(c)
	}

	/// Returns every slot to ASCII and puts G0 in front.
	pub fn reset(&mut self) {
		*self = Self::default();
	}
}
