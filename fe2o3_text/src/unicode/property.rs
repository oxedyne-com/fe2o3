//! Character classes by Unicode property: General_Category, Script, Script_Extensions and the
//! binary properties of UAX #44, as named in a regular expression's `\p{...}`.
//!
//! Name resolution follows UTS #18 and the Rust `regex` crate, which Typst's `regex(...)` uses, so
//! a pattern means the same here as there. A bare name is tried first as a binary property, then
//! as a General_Category, then as a Script -- the Script proper, not Script_Extensions, which is
//! asked for as `scx=`. Names match loosely (UAX44-LM3): case, spaces, underscores, hyphens and a
//! leading `is` are ignored, so `\p{Greek}`, `\p{isGreek}` and `\p{sc=grek}` agree.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::property::CharClass;
//!
//! let greek = CharClass::parse("Greek").expect("a known script");
//! assert!(greek.contains('λ'));
//! assert!(!greek.contains('l'));
//! ```

use crate::unicode::{
	lookup::Partitioned,
	prop::{
		GeneralCategory,
		GraphemeClass,
		Script,
		SentenceClass,
		WordClass,
	},
	tables::cat::{
		BIN_ALPHABETIC,
		BIN_JOIN_CONTROL,
		BIN_LONG,
		BIN_NAMES,
		BIN_OFFS,
		BIN_RANGES,
		BIN_WHITE_SPACE,
		GCB_NAMES,
		GC_NAMES,
		SB_NAMES,
		SCRIPT_NAMES,
		SCX_OFFS,
		SCX_POOL,
		SCX_STARTS,
		SCX_VALS,
		WB_NAMES,
	},
	lookup,
};

use oxedyne_fe2o3_core::prelude::*;


/// The Script_Extensions of one character: the scripts it is used with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Extensions {
	One(Script),				// no explicit extensions: the character's own script
	Many(&'static [Script]),
}

impl Extensions {

	pub fn of(c: char) -> Self {
		let set = lookup::get(&SCX_VALS, lookup::run(&SCX_STARTS, c), 0) as usize;
		if set == 0 {
			return Self::One(Script::of(c));
		}
		let a = lookup::get(&SCX_OFFS, set - 1, 0) as usize;
		let b = lookup::get(&SCX_OFFS, set, 0) as usize;
		match SCX_POOL.get(a..b) {
			Some(s)	=> Self::Many(s),
			None	=> Self::One(Script::of(c)),
		}
	}

	pub fn as_slice(&self) -> &[Script] {
		match self {
			Self::One(s)	=> std::slice::from_ref(s),
			Self::Many(v)	=> v,
		}
	}

	pub fn contains(&self, s: Script) -> bool {
		self.as_slice().contains(&s)
	}
}

/// A binary property of UAX #44, such as Alphabetic or White_Space.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Binary(u8);

impl Binary {

	pub const ALPHABETIC:	Self = Self(BIN_ALPHABETIC);
	pub const JOIN_CONTROL:	Self = Self(BIN_JOIN_CONTROL);
	pub const WHITE_SPACE:	Self = Self(BIN_WHITE_SPACE);

	/// Every binary property the tables carry.
	pub fn all() -> impl Iterator<Item = Self> {
		(0..BIN_LONG.len()).map(|i| Self(i as u8))
	}

	/// Finds a binary property by any of its aliases, loosely matched.
	pub fn find(name: &str) -> Option<Self> {
		match find_alias(&BIN_NAMES, name) {
			Some(i)	=> Some(Self(i)),
			None	=> None,
		}
	}

	/// The long UCD name, `White_Space` for instance.
	pub fn name(self) -> &'static str {
		lookup::get(&BIN_LONG, self.0 as usize, "")
	}

	/// Does `c` have the property?
	pub fn contains(self, c: char) -> bool {
		let a = lookup::get(&BIN_OFFS, self.0 as usize, 0) as usize;
		let b = lookup::get(&BIN_OFFS, self.0 as usize + 1, 0) as usize;
		let pairs = match BIN_RANGES.get(a..b) {
			Some(p)	=> p,
			None	=> return false,
		};
		// Pairs of (low, high): find the last pair whose low is at or below `c`.
		let n = pairs.len() / 2;
		let cp = c as u32;
		let i = partition_pairs(pairs, n, cp);
		if i == 0 {
			return false;
		}
		match (pairs.get(2 * (i - 1)), pairs.get(2 * (i - 1) + 1)) {
			(Some(lo), Some(hi))	=> *lo <= cp && cp <= *hi,
			_						=> false,
		}
	}
}

/// The number of pairs whose low end is at or below `cp`.
fn partition_pairs(pairs: &[u32], n: usize, cp: u32) -> usize {
	let (mut lo, mut hi) = (0usize, n);
	while lo < hi {
		let mid = (lo + hi) / 2;
		if lookup::get(pairs, 2 * mid, u32::MAX) <= cp {
			lo = mid + 1;
		} else {
			hi = mid;
		}
	}
	lo
}

/// A set of characters named by Unicode property, what `\p{...}` stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharClass {
	Any,
	Ascii,
	Assigned,
	Category(u32),			// mask over the `GeneralCategory` variants
	Script(Script),
	Extension(Script),		// Script_Extensions contains this script
	Binary(Binary),
	Grapheme(GraphemeClass),	// Grapheme_Cluster_Break
	WordBreak(WordClass),
	Sentence(SentenceClass),	// Sentence_Break
}

impl CharClass {

	/// Resolves the text between the braces of `\p{...}`: a bare name such as `L`, `Greek` or
	/// `Alphabetic`, or `property=value` (also `property:value`) where the property is
	/// General_Category (`gc`), Script (`sc`), Script_Extensions (`scx`),
	/// Grapheme_Cluster_Break (`gcb`), Word_Break (`wb`) or Sentence_Break (`sb`). Negation,
	/// `!=`, is the caller's business.
	pub fn parse(query: &str) -> Outcome<Self> {
		let split = match query.find(|c| c == '=' || c == ':') {
			Some(i)	=> Some((&query[..i], &query[i + 1..])),
			None	=> None,
		};
		let found = match split {
			Some((prop, val)) => {
				let p = loose(prop);
				match strip_is(&p).as_str() {
					"gc" | "generalcategory"	=> Self::category(val),
					"sc" | "script"				=> Self::script(val).map(Self::Script),
					"scx" | "scriptextensions"	=> Self::script(val).map(Self::Extension),
					"gcb" | "graphemeclusterbreak"	=>
						find_alias(&GCB_NAMES, val).map(Self::Grapheme),
					"wb" | "wordbreak"			=> find_alias(&WB_NAMES, val).map(Self::WordBreak),
					"sb" | "sentencebreak"		=> find_alias(&SB_NAMES, val).map(Self::Sentence),
					_ => return Err(err!(
						"Unicode property '{}' in '\\p{{{}}}' is not supported: the properties \
						that take a value are gc, sc, scx, gcb, wb and sb.", prop, query;
						Unimplemented, Input)),
				}
			},
			None => {
				let n = loose(query);
				let binary = if n != "cf" && n != "sc" && n != "lc" {
					Binary::find(query).map(Self::Binary)
				} else {
					None
				};
				binary
					.or_else(|| Self::category(query))
					.or_else(|| Self::script(query).map(Self::Script))
			},
		};
		match found {
			Some(c)	=> Ok(c),
			None	=> Err(err!(
				"Unicode property '\\p{{{}}}' is not known: it is neither a binary property, a \
				General_Category nor a Script.", query; Invalid, Input, Missing)),
		}
	}

	fn category(val: &str) -> Option<Self> {
		match loose(val).as_str() {
			"any"		=> return Some(Self::Any),
			"ascii"		=> return Some(Self::Ascii),
			"assigned"	=> return Some(Self::Assigned),
			_			=> {},
		}
		find_alias(&GC_NAMES, val).map(Self::Category)
	}

	fn script(val: &str) -> Option<Script> {
		find_alias(&SCRIPT_NAMES, val)
	}

	pub fn contains(&self, c: char) -> bool {
		match self {
			Self::Any			=> true,
			Self::Ascii			=> c.is_ascii(),
			Self::Assigned		=> GeneralCategory::of(c) != GeneralCategory::Cn,
			Self::Category(m)	=> m & (1u32 << (GeneralCategory::of(c) as u32)) != 0,
			Self::Script(s)		=> Script::of(c) == *s,
			Self::Extension(s)	=> Extensions::of(c).contains(*s),
			Self::Binary(b)		=> b.contains(c),
			Self::Grapheme(g)	=> GraphemeClass::of(c) == *g,
			Self::WordBreak(w)	=> WordClass::of(c) == *w,
			Self::Sentence(v)	=> SentenceClass::of(c) == *v,
		}
	}
}

/// Is `c` a word character in the Unicode sense of UTS #18 and `\w`: Alphabetic, a mark, a
/// decimal digit, connector punctuation or a join control?
pub fn is_word(c: char) -> bool {
	if c.is_ascii() {
		return c.is_ascii_alphanumeric() || c == '_';
	}
	match GeneralCategory::of(c) {
		GeneralCategory::Mn
		| GeneralCategory::Mc
		| GeneralCategory::Me
		| GeneralCategory::Nd
		| GeneralCategory::Pc	=> true,
		_ => Binary::ALPHABETIC.contains(c) || Binary::JOIN_CONTROL.contains(c),
	}
}

/// Is `c` white space, the White_Space property that `\s` stands for?
pub fn is_space(c: char) -> bool {
	if c.is_ascii() {
		return matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r');
	}
	Binary::WHITE_SPACE.contains(c)
}

/// Is `c` a decimal digit, General_Category Nd?
pub fn is_digit(c: char) -> bool {
	if c.is_ascii() {
		return c.is_ascii_digit();
	}
	GeneralCategory::of(c) == GeneralCategory::Nd
}

/// UAX44-LM3 without the `is` prefix rule: ASCII lower case, no spaces, underscores or hyphens,
/// and nothing that is not ASCII.
fn loose(name: &str) -> String {
	name.chars()
		.filter(|c| c.is_ascii() && *c != ' ' && *c != '_' && *c != '-')
		.map(|c| c.to_ascii_lowercase())
		.collect()
}

/// The name with a leading `is` removed, which UAX44-LM3 ignores, except that `isc` is the
/// ISO_Comment alias rather than `is` and `c`.
fn strip_is(n: &str) -> String {
	match n.strip_prefix("is") {
		Some(rest) if n != "isc" && !rest.is_empty() => rest.to_string(),
		_ => n.to_string(),
	}
}

/// Looks a name up in a sorted alias table, as written and then without an `is` prefix.
fn find_alias<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
	let n = loose(name);
	if let Ok(i) = table.binary_search_by(|(k, _)| (*k).cmp(n.as_str())) {
		return table.get(i).map(|(_, v)| *v);
	}
	let s = strip_is(&n);
	if s != n {
		if let Ok(i) = table.binary_search_by(|(k, _)| (*k).cmp(s.as_str())) {
			return table.get(i).map(|(_, v)| *v);
		}
	}
	None
}
