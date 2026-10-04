//! `unicode::property::is_invisible`: the characters a reader cannot see and a program reads.
//!
//! Written for the Daimond steering lint (QA A-F3, 2026-10-04), which let a soft hyphen, a
//! variation selector and a run of tag characters through a hand-made list of ranges. The cases
//! are named by code point so a failure says which character got in.

use oxedyne_fe2o3_text::unicode::{
	norm,
	property::{
		is_invisible,
		CharClass,
	},
};

use oxedyne_fe2o3_core::prelude::*;


// Every code point, surrogates left out because a char cannot hold one.
fn every_char() -> impl Iterator<Item = char> {
	(0u32..=0x10FFFF).filter_map(char::from_u32)
}

fn class(query: &str) -> Outcome<CharClass> {
	CharClass::parse(query)
}

#[test]
fn test_a_character_that_draws_nothing_is_invisible() {
	for (cp, name) in [
		// Controls, C0 and C1.
		(0x0000u32,	"NUL"),
		(0x0007,	"BEL"),
		(0x0009,	"TAB"),
		(0x000A,	"LF"),
		(0x000D,	"CR"),
		(0x001B,	"ESC"),
		(0x007F,	"DEL"),
		(0x0080,	"PAD"),
		(0x0085,	"NEL"),
		(0x009F,	"APC"),
		// The probes of the QA run.
		(0x00AD,	"SOFT HYPHEN"),
		(0x034F,	"COMBINING GRAPHEME JOINER"),
		(0x061C,	"ARABIC LETTER MARK"),
		(0x180E,	"MONGOLIAN VOWEL SEPARATOR"),
		(0xFE0F,	"VARIATION SELECTOR-16"),
		(0xE0020,	"TAG SPACE"),
		// Zero width and direction.
		(0x200B,	"ZERO WIDTH SPACE"),
		(0x200C,	"ZERO WIDTH NON-JOINER"),
		(0x200D,	"ZERO WIDTH JOINER"),
		(0x200E,	"LEFT-TO-RIGHT MARK"),
		(0x200F,	"RIGHT-TO-LEFT MARK"),
		(0x202A,	"LEFT-TO-RIGHT EMBEDDING"),
		(0x202E,	"RIGHT-TO-LEFT OVERRIDE"),
		(0x2066,	"LEFT-TO-RIGHT ISOLATE"),
		(0x2069,	"POP DIRECTIONAL ISOLATE"),
		(0x2060,	"WORD JOINER"),
		(0x2061,	"FUNCTION APPLICATION"),
		(0x2064,	"INVISIBLE PLUS"),
		(0xFEFF,	"BYTE ORDER MARK"),
		// Line and paragraph separators.
		(0x2028,	"LINE SEPARATOR"),
		(0x2029,	"PARAGRAPH SEPARATOR"),
		// Variation selectors, both blocks, and the Mongolian free ones.
		(0xFE00,	"VARIATION SELECTOR-1"),
		(0xE0100,	"VARIATION SELECTOR-17"),
		(0xE01EF,	"VARIATION SELECTOR-256"),
		(0x180B,	"MONGOLIAN FREE VARIATION SELECTOR ONE"),
		// The tag block, whole.
		(0xE0000,	"TAG block start, unassigned"),
		(0xE0001,	"LANGUAGE TAG"),
		(0xE0041,	"TAG LATIN CAPITAL LETTER A"),
		(0xE007E,	"TAG TILDE"),
		(0xE007F,	"CANCEL TAG"),
		// Fillers that stand for a letter and draw none.
		(0x115F,	"HANGUL CHOSEONG FILLER"),
		(0x1160,	"HANGUL JUNGSEONG FILLER"),
		(0x17B4,	"KHMER VOWEL INHERENT AQ"),
		(0x3164,	"HANGUL FILLER"),
		(0xFFA0,	"HALFWIDTH HANGUL FILLER"),
		// Other format characters.
		(0x0600,	"ARABIC NUMBER SIGN"),
		(0xFFF9,	"INTERLINEAR ANNOTATION ANCHOR"),
		(0x1BCA0,	"SHORTHAND FORMAT LETTER OVERLAP"),
		(0x1D173,	"MUSICAL SYMBOL BEGIN BEAM"),
		// Private use, three planes.
		(0xE000,	"PRIVATE USE, BMP"),
		(0xF8FF,	"PRIVATE USE, last of BMP"),
		(0xF0000,	"PRIVATE USE, plane 15"),
		(0x10FFFD,	"PRIVATE USE, plane 16"),
		// Unassigned and not characters.
		(0x0378,	"UNASSIGNED, Greek block"),
		(0xE0080,	"UNASSIGNED, past the tag block"),
		(0xFFFE,	"NONCHARACTER"),
		(0xFFFF,	"NONCHARACTER"),
		(0x2FFFF,	"NONCHARACTER"),
	] {
		match char::from_u32(cp) {
			Some(c)	=> assert!(is_invisible(c), "U+{:04X} {} is let through", cp, name),
			None	=> panic!("U+{:04X} {} is not a char", cp, name),
		}
	}
}

#[test]
fn test_a_character_that_draws_something_is_not_invisible() {
	for (c, name) in [
		('a',			"a"),
		('Z',			"Z"),
		('7',			"7"),
		('#',			"number sign"),
		(' ',			"space"),
		('-',			"hyphen-minus"),
		('\u{00E9}',	"e acute"),
		('\u{00DF}',	"sharp s"),
		('\u{03BB}',	"Greek lambda"),
		('\u{044F}',	"Cyrillic ya"),
		('\u{0301}',	"combining acute, which a reader sees on its letter"),
		('\u{4E2D}',	"CJK ideograph"),
		('\u{3042}',	"hiragana a"),
		('\u{D55C}',	"Hangul han"),
		('\u{3002}',	"ideographic full stop"),
		// A CJK writer's commas and colons are fullwidth forms and must stay writable.
		('\u{FF0C}',	"fullwidth comma"),
		('\u{FF1B}',	"fullwidth semicolon"),
		('\u{FF1A}',	"fullwidth colon"),
		('\u{FF21}',	"fullwidth A, visible and folded by NFKC instead"),
		('\u{FF71}',	"halfwidth katakana a"),
		// Spaces that draw as space.
		('\u{00A0}',	"no-break space"),
		('\u{2003}',	"em space"),
		('\u{3000}',	"ideographic space"),
		// Symbols and emoji, without their selectors.
		('\u{1F44D}',	"thumbs up"),
		('\u{2764}',	"heavy black heart"),
		('\u{2026}',	"horizontal ellipsis"),
		('\u{2122}',	"trade mark"),
		('\u{1D400}',	"mathematical bold A"),
	] {
		assert!(!is_invisible(c), "U+{:04X} {} is refused", c as u32, name);
	}
}

#[test]
fn test_whatever_the_categories_call_hidden_is_invisible() -> Outcome<()> {
	// The four categories of the brief, and the two separators, taken from the tables rather than
	// from a list written here, so a character the list forgot cannot slip through.
	for cat in ["gc=Cc", "gc=Cf", "gc=Co", "gc=Cn", "gc=Zl", "gc=Zp"] {
		let set = res!(class(cat));
		let mut n = 0usize;
		for c in every_char().filter(|c| set.contains(*c)) {
			n += 1;
			assert!(is_invisible(c), "U+{:04X} of {} is let through", c as u32, cat);
		}
		assert!(n > 0, "{} holds no character, so nothing was tested", cat);
	}
	Ok(())
}

#[test]
fn test_every_default_ignorable_and_variation_selector_is_invisible() -> Outcome<()> {
	for prop in ["Default_Ignorable_Code_Point", "Variation_Selector", "Bidi_Control", "Join_Control"] {
		let set = res!(class(prop));
		let mut n = 0usize;
		for c in every_char().filter(|c| set.contains(*c)) {
			n += 1;
			assert!(is_invisible(c), "U+{:04X} of {} is let through", c as u32, prop);
		}
		assert!(n > 0, "{} holds no character, so nothing was tested", prop);
	}
	// The tag block in full, assigned or not.
	for cp in 0xE0000u32..=0xE007F {
		if let Some(c) = char::from_u32(cp) {
			assert!(is_invisible(c), "tag U+{:04X} is let through", cp);
		}
	}
	Ok(())
}

#[test]
fn test_the_invisible_set_is_small_and_leaves_the_writing_alone() -> Outcome<()> {
	// A guard against the predicate growing past the intent: no letter, mark, number, punctuation
	// or symbol is invisible, in any script.
	for cat in ["L", "M", "N", "P", "S"] {
		let set = res!(class(cat));
		for c in every_char().filter(|c| set.contains(*c)) {
			// The fillers are letters that draw nothing, and the variation selectors are marks.
			let dropped = matches!(c as u32,
				0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x3164 | 0xFFA0
				| 0x034F | 0x180B..=0x180D | 0x180F | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF);
			assert_eq!(dropped, is_invisible(c), "U+{:04X} of {}", c as u32, cat);
		}
	}
	Ok(())
}

#[test]
fn test_folding_a_visible_character_never_makes_it_invisible() {
	// The lint judges the text as written and then judges the NFKC fold of it for the word list.
	// That is sound only if the fold cannot bring a hidden character out of a visible one.
	let mut folded = 0usize;
	for c in every_char().filter(|c| !is_invisible(*c)) {
		let s = c.to_string();
		let f = norm::nfkc(&s);
		if f != s {
			folded += 1;
		}
		assert!(!f.chars().any(is_invisible),
			"U+{:04X} folds to a hidden character: {:?}", c as u32, f);
	}
	assert!(folded > 3000, "only {} characters changed under NFKC, so the table did not load", folded);
}
