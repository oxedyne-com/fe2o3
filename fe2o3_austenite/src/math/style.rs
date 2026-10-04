// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (the `codex` crate's `styling` module, version 0.3, as Typst 0.15.1 uses it).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Maths alphabets: which Mathematical Alphanumeric Symbol a letter becomes under `bold`, `italic` and
//! the variant functions (`bb`, `cal`, `frak`, ...), ported from the `codex` crate's `styling` module
//! (0.3, Apache-2.0) that Typst 0.15 uses. Only the styles `select` can return are carried.

/// How big a maths expression is set: display and text at full size, scripts smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MathSize {
	ScriptScript,
	Script,
	Text,
	Display,
}

impl MathSize {
	pub fn name(self) -> &'static str {
		match self {
			MathSize::ScriptScript	=> "script-script",
			MathSize::Script		=> "script",
			MathSize::Text			=> "text",
			MathSize::Display		=> "display",
		}
	}

	pub fn from_name(s: &str) -> Option<Self> {
		Some(match s {
			"script-script"	=> MathSize::ScriptScript,
			"script"		=> MathSize::Script,
			"text"			=> MathSize::Text,
			"display"		=> MathSize::Display,
			_				=> return None,
		})
	}
}

/// The alphabet a variant function selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathVariant {
	Plain,
	Fraktur,
	SansSerif,
	Monospace,
	DoubleStruck,
	Chancery,
	Roundhand,
}

impl MathVariant {
	pub fn name(self) -> &'static str {
		match self {
			MathVariant::Plain			=> "plain",
			MathVariant::Fraktur		=> "fraktur",
			MathVariant::SansSerif		=> "sans-serif",
			MathVariant::Monospace		=> "monospace",
			MathVariant::DoubleStruck	=> "double-struck",
			MathVariant::Chancery		=> "chancery",
			MathVariant::Roundhand		=> "roundhand",
		}
	}

	pub fn from_name(s: &str) -> Option<Self> {
		Some(match s {
			"plain"			=> MathVariant::Plain,
			"fraktur"		=> MathVariant::Fraktur,
			"sans-serif"	=> MathVariant::SansSerif,
			"monospace"		=> MathVariant::Monospace,
			"double-struck"	=> MathVariant::DoubleStruck,
			"chancery"		=> MathVariant::Chancery,
			"roundhand"		=> MathVariant::Roundhand,
			_				=> return None,
		})
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathStyle {
	Plain,
	Bold,
	Italic,
	BoldItalic,
	Fraktur,
	BoldFraktur,
	SansSerif,
	SansSerifBold,
	SansSerifItalic,
	SansSerifBoldItalic,
	Monospace,
	DoubleStruck,
	DoubleStruckItalic,
	Chancery,
	BoldChancery,
	Roundhand,
	BoldRoundhand,
	Hebrew,
}

impl MathStyle {
	/// The style a character takes under a variant, boldness and italic setting (`None`: automatic,
	/// italic for Latin and lower-case Greek letters).
	pub fn select(c: char, variant: Option<MathVariant>, bold: bool, italic: Option<bool>) -> MathStyle {
		use MathVariant as Mv;
		match (variant.unwrap_or(Mv::Plain), bold, italic) {
			(Mv::SansSerif, false, Some(false)) if is_latin(c)	=> MathStyle::SansSerif,
			(Mv::SansSerif, false, _) if is_latin(c)			=> MathStyle::SansSerifItalic,
			(Mv::SansSerif, true, Some(false)) if is_latin(c)	=> MathStyle::SansSerifBold,
			(Mv::SansSerif, true, _) if is_latin(c)				=> MathStyle::SansSerifBoldItalic,
			(Mv::SansSerif, false, _) if is_digit(c)			=> MathStyle::SansSerif,
			(Mv::SansSerif, true, _) if is_digit(c)				=> MathStyle::SansSerifBold,
			(Mv::SansSerif, _, Some(false)) if is_greek(c)		=> MathStyle::SansSerifBold,
			(Mv::SansSerif, _, Some(true)) if is_greek(c)		=> MathStyle::SansSerifBoldItalic,
			(Mv::SansSerif, _, None) if is_upper_greek(c)		=> MathStyle::SansSerifBold,
			(Mv::SansSerif, _, None) if is_lower_greek(c)		=> MathStyle::SansSerifBoldItalic,
			(Mv::Fraktur, false, _) if is_latin(c)				=> MathStyle::Fraktur,
			(Mv::Fraktur, true, _) if is_latin(c)				=> MathStyle::BoldFraktur,
			(Mv::Monospace, _, _) if is_digit(c) || is_latin(c)	=> MathStyle::Monospace,
			(Mv::DoubleStruck, _, Some(true)) if matches!(c, 'D' | 'd' | 'e' | 'i' | 'j')
																=> MathStyle::DoubleStruckItalic,
			(Mv::DoubleStruck, _, _) if is_digit(c) || is_latin(c)
				|| matches!(c, '\u{2211}' | '\u{393}' | '\u{3a0}' | '\u{3b3}' | '\u{3c0}')
																=> MathStyle::DoubleStruck,
			(Mv::Chancery, false, _) if is_latin(c)				=> MathStyle::Chancery,
			(Mv::Chancery, true, _) if is_latin(c)				=> MathStyle::BoldChancery,
			(Mv::Roundhand, false, _) if is_latin(c)			=> MathStyle::Roundhand,
			(Mv::Roundhand, true, _) if is_latin(c)				=> MathStyle::BoldRoundhand,
			(_, false, Some(true)) if is_latin(c) || is_greek(c)	=> MathStyle::Italic,
			(_, false, None) if is_latin(c) || is_lower_greek(c)	=> MathStyle::Italic,
			(_, true, Some(false)) if is_latin(c) || is_greek(c)	=> MathStyle::Bold,
			(_, true, Some(true)) if is_latin(c) || is_greek(c)	=> MathStyle::BoldItalic,
			(_, true, None) if is_latin(c) || is_lower_greek(c)	=> MathStyle::BoldItalic,
			(_, true, None) if is_upper_greek(c)				=> MathStyle::Bold,
			(_, true, _) if is_digit(c) || matches!(c, '\u{3dc}' | '\u{3dd}')	=> MathStyle::Bold,
			(_, _, Some(true) | None) if matches!(c, '\u{131}' | '\u{237}' | '\u{127}')	=> MathStyle::Italic,
			(_, _, Some(true) | None) if is_hebrew(c)			=> MathStyle::Hebrew,
			_													=> MathStyle::Plain,
		}
	}
}

/// Appends the styled form of `c` to `out`: one character, or two where a variation selector picks
/// the chancery or roundhand script.
pub fn push_styled(out: &mut String, c: char, style: MathStyle) {
	let (a, b) = match style {
		MathStyle::Plain				=> (c, None),
		MathStyle::Bold					=> (to_bold(c), None),
		MathStyle::Italic				=> (to_italic(c), None),
		MathStyle::BoldItalic			=> (to_bold_italic(c), None),
		MathStyle::Fraktur				=> (to_fraktur(c), None),
		MathStyle::BoldFraktur			=> (to_bold_fraktur(c), None),
		MathStyle::SansSerif			=> (to_sans_serif(c), None),
		MathStyle::SansSerifBold		=> (to_sans_serif_bold(c), None),
		MathStyle::SansSerifItalic		=> (to_sans_serif_italic(c), None),
		MathStyle::SansSerifBoldItalic	=> (to_sans_serif_bold_italic(c), None),
		MathStyle::Monospace			=> (to_monospace(c), None),
		MathStyle::DoubleStruck			=> (to_double_struck(c), None),
		MathStyle::DoubleStruckItalic	=> (to_double_struck_italic(c), None),
		MathStyle::Chancery				=> (to_script(c), is_latin(c).then_some(VS1)),
		MathStyle::BoldChancery			=> (to_bold_script(c), is_latin(c).then_some(VS1)),
		MathStyle::Roundhand			=> (to_script(c), is_latin(c).then_some(VS2)),
		MathStyle::BoldRoundhand		=> (to_bold_script(c), is_latin(c).then_some(VS2)),
		MathStyle::Hebrew				=> (to_hebrew(c), None),
	};
	out.push(a);
	if let Some(b) = b {
		out.push(b);
	}
}

/// A string restyled character by character, as maths text and symbols are.
pub fn styled(text: &str, variant: Option<MathVariant>, bold: bool, italic: Option<bool>) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		push_styled(&mut out, c, MathStyle::select(c, variant, bold, italic));
	}
	out
}

const VS1: char = '\u{fe00}';
const VS2: char = '\u{fe01}';

fn is_digit(c: char) -> bool { c.is_ascii_digit() }
fn is_latin(c: char) -> bool { c.is_ascii_alphabetic() }
fn is_greek(c: char) -> bool { is_upper_greek(c) || is_lower_greek(c) }
fn is_upper_greek(c: char) -> bool { matches!(c, '\u{391}'..='\u{3a9}' | '\u{2207}' | '\u{3f4}') }
fn is_hebrew(c: char) -> bool { matches!(c, '\u{5d0}'..='\u{5d3}') }

fn is_lower_greek(c: char) -> bool {
	matches!(c, '\u{3b1}'..='\u{3c9}' | '\u{2202}' | '\u{3f5}' | '\u{3d1}' | '\u{3f0}' | '\u{3d5}' | '\u{3f1}' | '\u{3d6}')
}

// Every delta lands inside the Unicode scalar range by construction; an impossible sum keeps the letter.
fn shift(c: char, delta: u32) -> char {
	char::from_u32(c as u32 + delta).unwrap_or(c)
}

fn to_bold(c: char) -> char {
	let d = match c {
		'A'..='Z'				=> 0x1D3BF,
		'a'..='z'				=> 0x1D3B9,
		'\u{391}'..='\u{3a1}'	=> 0x1D317,
		'\u{3f4}'				=> 0x1D2C5,
		'\u{3a3}'..='\u{3a9}'	=> 0x1D317,
		'\u{2207}'				=> 0x1B4BA,
		'\u{3b1}'..='\u{3c9}'	=> 0x1D311,
		'\u{2202}'				=> 0x1B4D9,
		'\u{3f5}'				=> 0x1D2E7,
		'\u{3d1}'				=> 0x1D30C,
		'\u{3f0}'				=> 0x1D2EE,
		'\u{3d5}'				=> 0x1D30A,
		'\u{3f1}'				=> 0x1D2EF,
		'\u{3d6}'				=> 0x1D30B,
		'\u{3dc}'..='\u{3dd}'	=> 0x1D3EE,
		'0'..='9'				=> 0x1D79E,
		_						=> return c,
	};
	shift(c, d)
}

fn to_italic(c: char) -> char {
	let d = match c {
		'h'						=> 0x20A6,
		'\u{127}'				=> 0x1FE8,
		'A'..='Z'				=> 0x1D3F3,
		'a'..='z'				=> 0x1D3ED,
		'\u{131}'				=> 0x1D573,
		'\u{237}'				=> 0x1D46E,
		'\u{391}'..='\u{3a1}'	=> 0x1D351,
		'\u{3f4}'				=> 0x1D2FF,
		'\u{3a3}'..='\u{3a9}'	=> 0x1D351,
		'\u{2207}'				=> 0x1B4F4,
		'\u{3b1}'..='\u{3c9}'	=> 0x1D34B,
		'\u{2202}'				=> 0x1B513,
		'\u{3f5}'				=> 0x1D321,
		'\u{3d1}'				=> 0x1D346,
		'\u{3f0}'				=> 0x1D328,
		'\u{3d5}'				=> 0x1D344,
		'\u{3f1}'				=> 0x1D329,
		'\u{3d6}'				=> 0x1D345,
		_						=> return c,
	};
	shift(c, d)
}

fn to_bold_italic(c: char) -> char {
	let d = match c {
		'A'..='Z'				=> 0x1D427,
		'a'..='z'				=> 0x1D421,
		'\u{391}'..='\u{3a1}'	=> 0x1D38B,
		'\u{3f4}'				=> 0x1D339,
		'\u{3a3}'..='\u{3a9}'	=> 0x1D38B,
		'\u{2207}'				=> 0x1B52E,
		'\u{3b1}'..='\u{3c9}'	=> 0x1D385,
		'\u{2202}'				=> 0x1B54D,
		'\u{3f5}'				=> 0x1D35B,
		'\u{3d1}'				=> 0x1D380,
		'\u{3f0}'				=> 0x1D362,
		'\u{3d5}'				=> 0x1D37E,
		'\u{3f1}'				=> 0x1D363,
		'\u{3d6}'				=> 0x1D37F,
		_						=> return c,
	};
	shift(c, d)
}

fn to_script(c: char) -> char {
	let d = match c {
		'g'			=> 0x20A3,
		'H'			=> 0x20C3,
		'I'			=> 0x20C7,
		'L'			=> 0x20C6,
		'R'			=> 0x20C9,
		'B'			=> 0x20EA,
		'e'			=> 0x20CA,
		'E'..='F'	=> 0x20EB,
		'M'			=> 0x20E6,
		'o'			=> 0x20C5,
		'A'..='Z'	=> 0x1D45B,
		'a'..='z'	=> 0x1D455,
		_			=> return c,
	};
	shift(c, d)
}

fn to_bold_script(c: char) -> char {
	let d = match c {
		'A'..='Z'	=> 0x1D48F,
		'a'..='z'	=> 0x1D489,
		_			=> return c,
	};
	shift(c, d)
}

fn to_fraktur(c: char) -> char {
	let d = match c {
		'H'			=> 0x20C4,
		'I'			=> 0x20C8,
		'R'			=> 0x20CA,
		'Z'			=> 0x20CE,
		'C'			=> 0x20EA,
		'A'..='Z'	=> 0x1D4C3,
		'a'..='z'	=> 0x1D4BD,
		_			=> return c,
	};
	shift(c, d)
}

fn to_bold_fraktur(c: char) -> char {
	let d = match c {
		'A'..='Z'	=> 0x1D52B,
		'a'..='z'	=> 0x1D525,
		_			=> return c,
	};
	shift(c, d)
}

fn to_sans_serif(c: char) -> char {
	let d = match c {
		'A'..='Z'	=> 0x1D55F,
		'a'..='z'	=> 0x1D559,
		'0'..='9'	=> 0x1D7B2,
		_			=> return c,
	};
	shift(c, d)
}

fn to_sans_serif_bold(c: char) -> char {
	let d = match c {
		'A'..='Z'				=> 0x1D593,
		'a'..='z'				=> 0x1D58D,
		'\u{391}'..='\u{3a1}'	=> 0x1D3C5,
		'\u{3f4}'				=> 0x1D373,
		'\u{3a3}'..='\u{3a9}'	=> 0x1D3C5,
		'\u{2207}'				=> 0x1B568,
		'\u{3b1}'..='\u{3c9}'	=> 0x1D3BF,
		'\u{2202}'				=> 0x1B587,
		'\u{3f5}'				=> 0x1D395,
		'\u{3d1}'				=> 0x1D3BA,
		'\u{3f0}'				=> 0x1D39C,
		'\u{3d5}'				=> 0x1D3B8,
		'\u{3f1}'				=> 0x1D39D,
		'\u{3d6}'				=> 0x1D3B9,
		'0'..='9'				=> 0x1D7BC,
		_						=> return c,
	};
	shift(c, d)
}

fn to_sans_serif_italic(c: char) -> char {
	let d = match c {
		'A'..='Z'	=> 0x1D5C7,
		'a'..='z'	=> 0x1D5C1,
		_			=> return c,
	};
	shift(c, d)
}

fn to_sans_serif_bold_italic(c: char) -> char {
	let d = match c {
		'A'..='Z'				=> 0x1D5FB,
		'a'..='z'				=> 0x1D5F5,
		'\u{391}'..='\u{3a1}'	=> 0x1D3FF,
		'\u{3f4}'				=> 0x1D3AD,
		'\u{3a3}'..='\u{3a9}'	=> 0x1D3FF,
		'\u{2207}'				=> 0x1B5A2,
		'\u{3b1}'..='\u{3c9}'	=> 0x1D3F9,
		'\u{2202}'				=> 0x1B5C1,
		'\u{3f5}'				=> 0x1D3CF,
		'\u{3d1}'				=> 0x1D3F4,
		'\u{3f0}'				=> 0x1D3D6,
		'\u{3d5}'				=> 0x1D3F2,
		'\u{3f1}'				=> 0x1D3D7,
		'\u{3d6}'				=> 0x1D3F3,
		_						=> return c,
	};
	shift(c, d)
}

fn to_monospace(c: char) -> char {
	let d = match c {
		'A'..='Z'	=> 0x1D62F,
		'a'..='z'	=> 0x1D629,
		'0'..='9'	=> 0x1D7C6,
		_			=> return c,
	};
	shift(c, d)
}

fn to_double_struck(c: char) -> char {
	let d = match c {
		'C'						=> 0x20BF,
		'H'						=> 0x20C5,
		'N'						=> 0x20C7,
		'P'..='Q'				=> 0x20C9,
		'R'						=> 0x20CB,
		'Z'						=> 0x20CA,
		'\u{3c0}'				=> 0x1D7C,
		'\u{3b3}'				=> 0x1D8A,
		'\u{393}'				=> 0x1DAB,
		'\u{3a0}'				=> 0x1D9F,
		'\u{2211}'				=> return '\u{2140}',
		'A'..='Z'				=> 0x1D4F7,
		'a'..='z'				=> 0x1D4F1,
		'0'..='9'				=> 0x1D7A8,
		_						=> return c,
	};
	shift(c, d)
}

fn to_double_struck_italic(c: char) -> char {
	let d = match c {
		'D'			=> 0x2101,
		'd'..='e'	=> 0x20E2,
		'i'..='j'	=> 0x20DF,
		_			=> return c,
	};
	shift(c, d)
}

fn to_hebrew(c: char) -> char {
	match c {
		'\u{5d0}'..='\u{5d3}'	=> shift(c, 0x1B65),
		_						=> c,
	}
}
