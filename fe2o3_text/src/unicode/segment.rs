//! Grapheme cluster and word segmentation, following UAX #29.
//!
//! An extended grapheme cluster is what a reader calls a character, and so it is what a cursor
//! should step over and what a selection should snap to. A word boundary is coarser, and is what a
//! double click should select.
//!
//! Both functions return byte offsets into the string, including zero and its length, so that
//! `s[b[i]..b[i + 1]]` is always a valid slice.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::segment;
//!
//! // A base and its mark are one cluster, and a flag is one cluster.
//! let s = "e\u{0301}\u{1F1E6}\u{1F1FA}";
//! assert_eq!(segment::graphemes(s), vec!["e\u{0301}", "\u{1F1E6}\u{1F1FA}"]);
//! ```

use crate::unicode::{
	lookup::{
		self,
		Partitioned,
	},
	prop::{
		ConjunctBreak,
		GraphemeClass as G,
		WordClass as W,
	},
	tables::seg::{
		SEG_FLAG_STARTS,
		SEG_FLAG_VALS,
	},
};

/// Bit in the segmentation flags marking Extended_Pictographic.
const FLAG_EXT_PICT: u8 = 1 << 0;
/// Shift of the two bit Indic_Conjunct_Break field in the segmentation flags.
const INCB_SHIFT: u8 = 1;

/// Whether `c` has the Extended_Pictographic property, which is to say whether it is an emoji or
/// could become one.
pub fn is_extended_pictographic(c: char) -> bool {
	flags(c) & FLAG_EXT_PICT != 0
}

/// Returns the Indic_Conjunct_Break property of `c`.
pub fn conjunct_break(c: char) -> ConjunctBreak {
	match (flags(c) >> INCB_SHIFT) & 0b11 {
		1 => ConjunctBreak::Consonant,
		2 => ConjunctBreak::Extend,
		3 => ConjunctBreak::Linker,
		_ => ConjunctBreak::None,
	}
}

/// Returns the segmentation flags of `c`.
fn flags(c: char) -> u8 {
	lookup::flags(&SEG_FLAG_STARTS, &SEG_FLAG_VALS, c)
}

/// A character with everything the segmentation rules ask of it.
struct Ch {
	/// The byte offset of the character in the string.
	byte:	usize,
	/// The Grapheme_Cluster_Break class.
	gcb:	G,
	/// The Word_Break class.
	wb:		W,
	/// The Indic_Conjunct_Break class.
	incb:	ConjunctBreak,
	/// Whether the character is Extended_Pictographic.
	pict:	bool,
}

/// Reads a string into the per character state the rules work over.
fn scan(s: &str) -> Vec<Ch> {
	let mut chs = Vec::with_capacity(s.len());
	for (byte, c) in s.char_indices() {
		chs.push(Ch {
			byte,
			gcb:	G::of(c),
			wb:		W::of(c),
			incb:	conjunct_break(c),
			pict:	is_extended_pictographic(c),
		});
	}
	chs
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Grapheme clusters                                                                         │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Returns the byte offsets of the extended grapheme cluster boundaries of `s`, beginning with
/// zero and ending with its length.
pub fn grapheme_boundaries(s: &str) -> Vec<usize> {

	let mut out = vec![0];
	if s.is_empty() {
		return out;
	}
	let chs = scan(s);

	for i in 1..chs.len() {
		if grapheme_break(&chs, i) {
			out.push(chs[i].byte);
		}
	}
	out.push(s.len());
	out
}

/// Returns the extended grapheme clusters of `s`.
pub fn graphemes(s: &str) -> Vec<&str> {
	let bounds = grapheme_boundaries(s);
	let mut out = Vec::with_capacity(bounds.len().saturating_sub(1));
	for w in bounds.windows(2) {
		if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
			if let Some(part) = s.get(*a..*b) {
				out.push(part);
			}
		}
	}
	out
}

/// Returns the byte offset of the grapheme cluster boundary at or after `from`, which is the
/// string length once there is nothing left. This is where a cursor moving right should land.
pub fn next_grapheme(s: &str, from: usize) -> usize {
	for b in grapheme_boundaries(s) {
		if b > from {
			return b;
		}
	}
	s.len()
}

/// Returns the byte offset of the grapheme cluster boundary before `from`, which is zero once
/// there is nothing left. This is where a cursor moving left should land.
pub fn prev_grapheme(s: &str, from: usize) -> usize {
	let mut prev = 0;
	for b in grapheme_boundaries(s) {
		if b >= from {
			break;
		}
		prev = b;
	}
	prev
}

/// Whether `at` is a grapheme cluster boundary, which is where a cursor is allowed to be. A cursor
/// anywhere else sits inside a character, which is a corruption rather than a position.
pub fn is_grapheme_boundary(s: &str, at: usize) -> bool {
	if at == 0 || at == s.len() {
		return true;
	}
	grapheme_boundaries(s).contains(&at)
}

/// Returns the byte offset of the grapheme cluster boundary at or after `from` that is nearest to
/// it, snapping a cursor onto the character grid it must sit on.
pub fn snap_grapheme(s: &str, at: usize) -> usize {
	let at = at.min(s.len());
	let mut best = 0;
	for b in grapheme_boundaries(s) {
		if b == at {
			return at;
		}
		// The boundaries come in order, so the last one below `at` and the first one above it are
		// the only two candidates, and the nearer of those two wins.
		if b < at {
			best = b;
		} else {
			return if at - best <= b - at { best } else { b };
		}
	}
	best
}

/// Returns the byte offset of the word boundary after `from`, which is the string length once there
/// is nothing left. This is where a cursor moving a word to the right should land.
///
/// A word boundary is UAX #29's, so an apostrophe does not break `don't` and a full stop does not
/// break `3.14`.
pub fn next_word(s: &str, from: usize) -> usize {
	for b in word_boundaries(s) {
		if b > from {
			return b;
		}
	}
	s.len()
}

/// Returns the byte offset of the word boundary before `from`, which is zero once there is nothing
/// left. This is where a cursor moving a word to the left should land.
pub fn prev_word(s: &str, from: usize) -> usize {
	let mut prev = 0;
	for b in word_boundaries(s) {
		if b >= from {
			break;
		}
		prev = b;
	}
	prev
}

/// Whether there is a grapheme cluster boundary before the character at `i`, by the rules of
/// UAX #29, taken in order.
fn grapheme_break(chs: &[Ch], i: usize) -> bool {

	let (a, b) = match (chs.get(i - 1), chs.get(i)) {
		(Some(a), Some(b)) => (a, b),
		_ => return true,
	};

	// GB3, GB4, GB5. A CR and its LF stay together; nothing else joins a control.
	if a.gcb == G::CR && b.gcb == G::LF {
		return false;
	}
	if matches!(a.gcb, G::Control | G::CR | G::LF) {
		return true;
	}
	if matches!(b.gcb, G::Control | G::CR | G::LF) {
		return true;
	}

	// GB6, GB7, GB8. A Hangul syllable holds together.
	if a.gcb == G::L && matches!(b.gcb, G::L | G::V | G::LV | G::LVT) {
		return false;
	}
	if matches!(a.gcb, G::LV | G::V) && matches!(b.gcb, G::V | G::T) {
		return false;
	}
	if matches!(a.gcb, G::LVT | G::T) && b.gcb == G::T {
		return false;
	}

	// GB9, GB9a, GB9b.
	if matches!(b.gcb, G::Extend | G::ZWJ) {
		return false;
	}
	if b.gcb == G::SpacingMark {
		return false;
	}
	if a.gcb == G::Prepend {
		return false;
	}

	// GB9c. An Indic conjunct, that is a consonant joined to a consonant by a virama, is one
	// cluster.
	if b.incb == ConjunctBreak::Consonant && conjunct_before(chs, i) {
		return false;
	}

	// GB11. An emoji joined to an emoji by a zero width joiner is one cluster.
	if a.gcb == G::ZWJ && b.pict && pictographic_before(chs, i - 1) {
		return false;
	}

	// GB12, GB13. Regional indicators pair up into flags, so a break falls between pairs.
	if a.gcb == G::RegionalIndicator && b.gcb == G::RegionalIndicator {
		return regional_run(chs, i - 1) % 2 == 0;
	}

	// GB999.
	true
}

/// Whether the characters before `i` are a linking consonant, then extenders including at least
/// one linker, as grapheme rule GB9c requires.
fn conjunct_before(chs: &[Ch], i: usize) -> bool {
	let mut j		= i;
	let mut linked	= false;
	while j > 0 {
		match chs.get(j - 1) {
			Some(ch) => match ch.incb {
				ConjunctBreak::Linker => {
					linked = true;
					j -= 1;
				},
				ConjunctBreak::Extend => j -= 1,
				ConjunctBreak::Consonant => return linked,
				ConjunctBreak::None => return false,
			},
			None => return false,
		}
	}
	false
}

/// Whether the character at `i` is a zero width joiner preceded by an Extended_Pictographic
/// character and any number of extenders, as grapheme rule GB11 requires.
fn pictographic_before(chs: &[Ch], i: usize) -> bool {
	let mut j = i;
	while j > 0 {
		match chs.get(j - 1) {
			Some(ch) if ch.gcb == G::Extend => j -= 1,
			Some(ch) => return ch.pict,
			None => return false,
		}
	}
	false
}

/// Returns the number of regional indicators running back from `i`, inclusive.
fn regional_run(chs: &[Ch], i: usize) -> usize {
	let mut n = 0;
	let mut j = i + 1;
	while j > 0 {
		match chs.get(j - 1) {
			Some(ch) if ch.gcb == G::RegionalIndicator => {
				n += 1;
				j -= 1;
			},
			_ => break,
		}
	}
	n
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Words                                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Returns the byte offsets of the word boundaries of `s`, beginning with zero and ending with its
/// length. The spans between them include the spaces and punctuation as well as the words.
pub fn word_boundaries(s: &str) -> Vec<usize> {

	let mut out = vec![0];
	if s.is_empty() {
		return out;
	}
	let chs = scan(s);

	// Rule WB4 folds extenders into the character they follow, so the later rules see a sequence of
	// clusters rather than of characters. `base[i]` is the index of the character that begins the
	// cluster holding character `i`, and `next[i]` the index of the cluster after it.
	let (base, next) = word_clusters(&chs);

	for i in 1..chs.len() {
		if word_break(&chs, &base, &next, i) {
			out.push(chs[i].byte);
		}
	}
	out.push(s.len());
	out
}

/// Returns the words and the spans between them, in order.
pub fn words(s: &str) -> Vec<&str> {
	let bounds = word_boundaries(s);
	let mut out = Vec::with_capacity(bounds.len().saturating_sub(1));
	for w in bounds.windows(2) {
		if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
			if let Some(part) = s.get(*a..*b) {
				out.push(part);
			}
		}
	}
	out
}

/// Whether the character at `i` extends the one before it, under word rule WB4.
fn extends(chs: &[Ch], i: usize) -> bool {
	let (a, b) = match (chs.get(i.wrapping_sub(1)), chs.get(i)) {
		(Some(a), Some(b)) => (a, b),
		_ => return false,
	};
	if !matches!(b.wb, W::Extend | W::Format | W::ZWJ) {
		return false;
	}
	!matches!(a.wb, W::CR | W::LF | W::Newline)
}

/// Groups the characters into the clusters that word rule WB4 leaves behind, returning the first
/// character of the cluster holding each character, and the first character of the cluster after
/// it.
fn word_clusters(chs: &[Ch]) -> (Vec<usize>, Vec<usize>) {

	let n			= chs.len();
	let mut base	= vec![0usize; n];
	let mut next	= vec![n; n];

	let mut start = 0;
	for i in 0..n {
		if i > 0 && !extends(chs, i) {
			start = i;
		}
		base[i] = start;
	}
	for i in 0..n {
		let mut j = base[i] + 1;
		while j < n && base[j] != j {
			j += 1;
		}
		next[i] = j;
	}

	(base, next)
}

/// The Word_Break class of the cluster beginning at `i`, or `None` past the end of the text.
fn wcls(chs: &[Ch], i: usize) -> Option<W> {
	chs.get(i).map(|ch| ch.wb)
}

/// Whether `w` is a letter that takes part in words, the AHLetter of UAX #29.
fn is_ah(w: Option<W>) -> bool {
	matches!(w, Some(W::ALetter) | Some(W::HebrewLetter))
}

/// Whether `w` may appear inside a word or a number, the MidNumLetQ of UAX #29.
fn is_midnumlet(w: Option<W>) -> bool {
	matches!(w, Some(W::MidNumLet) | Some(W::SingleQuote))
}

/// Whether there is a word boundary before the character at `i`, by the rules of UAX #29, taken in
/// order.
fn word_break(chs: &[Ch], base: &[usize], next: &[usize], i: usize) -> bool {

	let (a, b) = match (chs.get(i - 1), chs.get(i)) {
		(Some(a), Some(b)) => (a, b),
		_ => return true,
	};

	// WB3, WB3a, WB3b. A CR and its LF stay together; nothing else joins a newline.
	if a.wb == W::CR && b.wb == W::LF {
		return false;
	}
	if matches!(a.wb, W::Newline | W::CR | W::LF) {
		return true;
	}
	if matches!(b.wb, W::Newline | W::CR | W::LF) {
		return true;
	}

	// WB3c. A zero width joiner holds an emoji to what follows it.
	if a.wb == W::ZWJ && b.pict {
		return false;
	}

	// WB3d. Spaces that segment words stay with each other.
	if a.wb == W::WSegSpace && b.wb == W::WSegSpace {
		return false;
	}

	// WB4. An extender or format character joins the cluster before it.
	if extends(chs, i) {
		return false;
	}

	// The remaining rules read clusters rather than characters: `p` begins the cluster before the
	// boundary, `q` begins the one after, `o` the one before `p`, and `r` the one after `q`.
	let q = i;
	let p = base[i - 1];
	let o = if p == 0 { None } else { Some(base[p - 1]) };
	let r = lookup::get(next, q, chs.len());

	let ca = wcls(chs, p);
	let cb = wcls(chs, q);
	let co = o.and_then(|o| wcls(chs, o));
	let cr = wcls(chs, r);

	// WB5, WB6, WB7. Letters hold together, across at most one character that lives inside a word.
	if is_ah(ca) && is_ah(cb) {
		return false;
	}
	if is_ah(ca) && (cb == Some(W::MidLetter) || is_midnumlet(cb)) && is_ah(cr) {
		return false;
	}
	if is_ah(co) && (ca == Some(W::MidLetter) || is_midnumlet(ca)) && is_ah(cb) {
		return false;
	}

	// WB7a, WB7b, WB7c. Hebrew keeps its quotation marks.
	if ca == Some(W::HebrewLetter) && cb == Some(W::SingleQuote) {
		return false;
	}
	if ca == Some(W::HebrewLetter) && cb == Some(W::DoubleQuote)
		&& cr == Some(W::HebrewLetter)
	{
		return false;
	}
	if co == Some(W::HebrewLetter) && ca == Some(W::DoubleQuote)
		&& cb == Some(W::HebrewLetter)
	{
		return false;
	}

	// WB8, WB9, WB10, WB11, WB12. Numbers hold together, and hold on to the letters beside them.
	if ca == Some(W::Numeric) && cb == Some(W::Numeric) {
		return false;
	}
	if is_ah(ca) && cb == Some(W::Numeric) {
		return false;
	}
	if ca == Some(W::Numeric) && is_ah(cb) {
		return false;
	}
	if co == Some(W::Numeric) && (ca == Some(W::MidNum) || is_midnumlet(ca))
		&& cb == Some(W::Numeric)
	{
		return false;
	}
	if ca == Some(W::Numeric) && (cb == Some(W::MidNum) || is_midnumlet(cb))
		&& cr == Some(W::Numeric)
	{
		return false;
	}

	// WB13, WB13a, WB13b. Katakana holds together, and an underscore or the like joins what it
	// sits between.
	if ca == Some(W::Katakana) && cb == Some(W::Katakana) {
		return false;
	}
	if (is_ah(ca) || matches!(ca, Some(W::Numeric) | Some(W::Katakana) | Some(W::ExtendNumLet)))
		&& cb == Some(W::ExtendNumLet)
	{
		return false;
	}
	if ca == Some(W::ExtendNumLet)
		&& (is_ah(cb) || matches!(cb, Some(W::Numeric) | Some(W::Katakana)))
	{
		return false;
	}

	// WB15, WB16. Regional indicators pair up into flags.
	if ca == Some(W::RegionalIndicator) && cb == Some(W::RegionalIndicator) {
		return word_regional_run(chs, base, p) % 2 == 0;
	}

	// WB999.
	true
}

/// Returns the number of regional indicator clusters running back from the one beginning at `p`,
/// inclusive.
fn word_regional_run(chs: &[Ch], base: &[usize], p: usize) -> usize {
	let mut n = 0;
	let mut j = Some(p);
	while let Some(k) = j {
		match chs.get(k) {
			Some(ch) if ch.wb == W::RegionalIndicator => {
				n += 1;
				j = if k == 0 { None } else { Some(lookup::get(base, k - 1, 0)) };
			},
			_ => break,
		}
	}
	n
}

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_core::prelude::*;

	/// A cursor moves by character, and a character is a grapheme cluster: the acute and the letter
	/// it sits on are one press of an arrow key, not two.
	#[test]
	fn test_a_cursor_steps_over_a_combining_mark_00() -> Outcome<()> {
		let s = "e\u{301}f";	// e + combining acute, then f.
		assert_eq!(next_grapheme(s, 0), 3);
		assert_eq!(prev_grapheme(s, 3), 0);
		Ok(())
	}

	/// Word movement lands on UAX #29's boundaries, so an apostrophe does not split a word.
	#[test]
	fn test_word_movement_keeps_a_contraction_whole_01() -> Outcome<()> {
		let s = "don't stop";
		// The word runs to its end rather than breaking at the apostrophe.
		assert_eq!(next_word(s, 0), 5);
		assert_eq!(prev_word(s, 10), 6);
		Ok(())
	}

	/// A cursor offset that fell inside a character is snapped back onto the character grid.
	#[test]
	fn test_an_offset_inside_a_character_snaps_to_a_boundary_02() -> Outcome<()> {
		let s = "e\u{301}f";
		assert!(!is_grapheme_boundary(s, 1));	// Inside the cluster.
		assert!(is_grapheme_boundary(s, 0));
		assert!(is_grapheme_boundary(s, 3));
		assert_eq!(snap_grapheme(s, 1), 0);	// Nearer the start of the cluster.
		assert_eq!(snap_grapheme(s, 2), 3);	// Nearer its end.
		Ok(())
	}

	/// The ends of the string are boundaries, and movement stops at them rather than running off.
	#[test]
	fn test_movement_stops_at_the_ends_03() -> Outcome<()> {
		let s = "ab";
		assert_eq!(next_grapheme(s, 2), 2);
		assert_eq!(prev_grapheme(s, 0), 0);
		assert_eq!(next_word(s, 2), 2);
		assert_eq!(prev_word(s, 0), 0);
		Ok(())
	}
}
