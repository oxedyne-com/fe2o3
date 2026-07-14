//! Line breaking, following UAX #14.
//!
//! The algorithm answers one question: where in this text may a renderer end a line, and where
//! must it? It does not decide where a line actually ends, because that depends on the width of
//! the glyphs and of the column, which are no business of a text library. A layout engine walks the
//! opportunities this module yields, measures the run up to each one, and breaks at the last that
//! fits.
//!
//! The rules are those of the default, untailored algorithm, which is what the Unicode Consortium's
//! `LineBreakTest.txt` tests: class AI, SG and XX resolve to AL, class CJ resolves to NS, and class
//! SA resolves to CM if the character is a nonspacing or spacing mark and to AL otherwise.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::linebreak::{
//!     self,
//!     Break,
//! };
//!
//! let s = "one two\nthree";
//! let opps = linebreak::line_breaks(s);
//!
//! // After the space, after the newline, and at the end of the text.
//! assert_eq!(opps[0].offset, 4);
//! assert_eq!(opps[0].kind, Break::Optional);
//! assert_eq!(opps[1].offset, 8);
//! assert_eq!(opps[1].kind, Break::Mandatory);
//! ```

use crate::unicode::{
	lookup::{
		self,
		Partitioned,
	},
	prop::LineBreakClass as L,
	tables::lb::{
		LB_FLAG_STARTS,
		LB_FLAG_VALS,
	},
};

/// Bit marking a character of East Asian width F, W or H.
const FLAG_EAST_ASIAN: u8	= 1 << 0;
/// Bit marking an initial quotation mark, general category Pi.
const FLAG_PI: u8			= 1 << 1;
/// Bit marking a final quotation mark, general category Pf.
const FLAG_PF: u8			= 1 << 2;
/// Bit marking an unassigned code point that is Extended_Pictographic.
const FLAG_EXT_PICT_UNASSIGNED: u8 = 1 << 3;
/// Bit marking general category Mn or Mc, which decides how class SA resolves.
const FLAG_MARK: u8			= 1 << 4;

/// The dotted circle, which the Brahmic rules LB28a treat as an aksara.
const DOTTED_CIRCLE: char = '\u{25CC}';

/// Whether a break at an offset is allowed or required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Break {
	/// The line may end here.
	Optional,
	/// The line must end here, because the text says so or because it has run out.
	Mandatory,
}

/// A place at which a line may or must end, as a byte offset into the string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opportunity {
	/// The byte offset of the first character of the next line.
	pub offset:	usize,
	/// Whether the break is allowed or required.
	pub kind:	Break,
}

/// Returns every place a line may or must end in `s`, in order, ending with the length of the
/// string. The end of the text is always a mandatory break, since a line cannot run past it.
pub fn line_breaks(s: &str) -> Vec<Opportunity> {

	let mut out = Vec::new();
	if s.is_empty() {
		return out;
	}

	let chs	= scan(s);
	let cls	= clusters(&chs);

	for i in 1..chs.len() {
		match rule(&chs, &cls, i) {
			Some(kind) => out.push(Opportunity { offset: chs[i].byte, kind }),
			None => (),
		}
	}

	out.push(Opportunity { offset: s.len(), kind: Break::Mandatory });
	out
}

/// Returns the byte offsets at which `s` may or must break, without saying which is which.
pub fn break_offsets(s: &str) -> Vec<usize> {
	line_breaks(s).into_iter().map(|o| o.offset).collect()
}

/// A character, with the line breaking class it resolves to.
struct Ch {
	/// The byte offset of the character in the string.
	byte:	usize,
	/// The resolved line breaking class.
	cls:	L,
	/// The line breaking flags.
	flags:	u8,
	/// Whether the character is the dotted circle.
	dot:	bool,
}

/// A run of a character and the combining marks that rule LB9 folds into it. The rules from LB11
/// onwards read clusters, not characters.
struct Cl {
	/// The index of the character the cluster begins with.
	first:	usize,
	/// The class of that character, with a lone combining mark resolved to AL by rule LB10.
	cls:	L,
	/// The flags of that character.
	flags:	u8,
	/// Whether that character is the dotted circle.
	dot:	bool,
}

/// Reads a string into the per character state the rules work over, resolving the classes that the
/// default algorithm does not use.
fn scan(s: &str) -> Vec<Ch> {

	let mut chs = Vec::with_capacity(s.len());
	for (byte, c) in s.char_indices() {
		let flags	= lookup::flags(&LB_FLAG_STARTS, &LB_FLAG_VALS, c);
		let cls		= match L::of(c) {
			L::AI | L::SG | L::XX	=> L::AL,
			L::CJ					=> L::NS,
			L::SA					=> if flags & FLAG_MARK != 0 { L::CM } else { L::AL },
			other					=> other,
		};
		chs.push(Ch {
			byte,
			cls,
			flags,
			dot: c == DOTTED_CIRCLE,
		});
	}
	chs
}

/// Whether the character at `i` is a combining mark that rule LB9 folds into the character before
/// it.
fn folds(chs: &[Ch], i: usize) -> bool {
	let (a, b) = match (chs.get(i.wrapping_sub(1)), chs.get(i)) {
		(Some(a), Some(b)) => (a, b),
		_ => return false,
	};
	if !matches!(b.cls, L::CM | L::ZWJ) {
		return false;
	}
	!matches!(a.cls, L::BK | L::CR | L::LF | L::NL | L::SP | L::ZW)
}

/// Groups the characters into the clusters that rule LB9 leaves behind.
fn clusters(chs: &[Ch]) -> Vec<Cl> {
	let mut out = Vec::with_capacity(chs.len());
	for (i, ch) in chs.iter().enumerate() {
		if i > 0 && folds(chs, i) {
			continue;
		}
		// LB10. A combining mark that folds into nothing stands for an alphabetic character.
		let cls = match ch.cls {
			L::CM | L::ZWJ	=> L::AL,
			other			=> other,
		};
		out.push(Cl {
			first:	i,
			cls,
			flags:	ch.flags,
			dot:	ch.dot,
		});
	}
	out
}

/// Returns the index of the cluster the character at `i` belongs to, given that `i` begins one.
fn cluster_at(cls: &[Cl], i: usize) -> Option<usize> {
	match cls.binary_search_by(|c| c.first.cmp(&i)) {
		Ok(q) => Some(q),
		Err(_) => None,
	}
}

/// The class of cluster `q`, or `None` past either end of the text.
fn cl(cls: &[Cl], q: Option<usize>) -> Option<L> {
	q.and_then(|q| cls.get(q)).map(|c| c.cls)
}

/// Whether cluster `q` is East Asian. A cluster past the end of the text is not.
fn ea(cls: &[Cl], q: Option<usize>) -> bool {
	match q.and_then(|q| cls.get(q)) {
		Some(c) => c.flags & FLAG_EAST_ASIAN != 0,
		None => false,
	}
}

/// Whether cluster `q` carries a flag.
fn has(cls: &[Cl], q: Option<usize>, flag: u8) -> bool {
	match q.and_then(|q| cls.get(q)) {
		Some(c) => c.flags & flag != 0,
		None => false,
	}
}

/// Whether cluster `q` is an aksara or the dotted circle, which rules LB28a group together.
fn aksara(cls: &[Cl], q: Option<usize>) -> bool {
	match q.and_then(|q| cls.get(q)) {
		Some(c) => matches!(c.cls, L::AK | L::AS) || c.dot,
		None => false,
	}
}

/// Steps back over the clusters of the classes in `over`, returning the first cluster that is not
/// one of them.
fn skip_back(cls: &[Cl], from: usize, over: &[L]) -> Option<usize> {
	let mut q = Some(from);
	while let Some(i) = q {
		match cls.get(i) {
			Some(c) if over.contains(&c.cls) => q = i.checked_sub(1),
			_ => break,
		}
	}
	q
}

/// Whether the clusters running back from `q` are a number followed by any number of symbols and
/// separators, the `NU (SY | IS)*` of rule LB25.
fn numeric_run(cls: &[Cl], q: usize) -> bool {
	match skip_back(cls, q, &[L::SY, L::IS]) {
		Some(i) => cl(cls, Some(i)) == Some(L::NU),
		None => false,
	}
}

/// Returns the number of regional indicator clusters running back from `q`, inclusive.
fn regional_run(cls: &[Cl], q: usize) -> usize {
	let mut n	= 0;
	let mut i	= Some(q);
	while let Some(j) = i {
		match cls.get(j) {
			Some(c) if c.cls == L::RI => {
				n += 1;
				i = j.checked_sub(1);
			},
			_ => break,
		}
	}
	n
}

/// Whether a line may or must end before the character at `i`, by the rules of UAX #14, taken in
/// order. `None` means it may not.
fn rule(chs: &[Ch], cls: &[Cl], i: usize) -> Option<Break> {

	let (a, b) = match (chs.get(i - 1), chs.get(i)) {
		(Some(a), Some(b)) => (a, b),
		_ => return Some(Break::Mandatory),
	};

	// LB4, LB5. A mandatory break follows a hard line break, and a carriage return keeps its line
	// feed.
	if a.cls == L::BK {
		return Some(Break::Mandatory);
	}
	if a.cls == L::CR && b.cls == L::LF {
		return None;
	}
	if matches!(a.cls, L::CR | L::LF | L::NL) {
		return Some(Break::Mandatory);
	}

	// LB6, LB7. Nothing breaks before a hard line break, a space or a zero width space.
	if matches!(b.cls, L::BK | L::CR | L::LF | L::NL) {
		return None;
	}
	if matches!(b.cls, L::SP | L::ZW) {
		return None;
	}

	// LB8. A zero width space breaks after, even across the spaces that follow it.
	let mut j = i - 1;
	while j > 0 && chs[j].cls == L::SP {
		j -= 1;
	}
	if chs[j].cls == L::ZW {
		return Some(Break::Optional);
	}

	// LB8a. A zero width joiner holds on to what follows it.
	if a.cls == L::ZWJ {
		return None;
	}

	// LB9. A combining mark is part of the character it follows.
	if folds(chs, i) {
		return None;
	}

	// The remaining rules read clusters. `q` is the cluster beginning at `i`, `p` the one before
	// it, `o` the one before that, and `r` and `t` the ones after `q`.
	let q = match cluster_at(cls, i) {
		Some(q) => q,
		None => return None,
	};
	let p = match q.checked_sub(1) {
		Some(p) => p,
		None => return None,
	};
	let o = p.checked_sub(1);
	let r = Some(q + 1);
	let t = Some(q + 2);

	let (ca, cb)	= (cl(cls, Some(p)), cl(cls, Some(q)));
	let (co, cr)	= (cl(cls, o), cl(cls, r));
	let ct			= cl(cls, t);

	let is = |c: Option<L>, set: &[L]| -> bool {
		match c {
			Some(c) => set.contains(&c),
			None => false,
		}
	};

	// LB11. A word joiner binds on both sides.
	if cb == Some(L::WJ) || ca == Some(L::WJ) {
		return None;
	}

	// LB12, LB12a. Non-breaking glue binds on both sides, except after a space or a hyphen.
	if ca == Some(L::GL) {
		return None;
	}
	if !is(ca, &[L::SP, L::BA, L::HY, L::HH]) && cb == Some(L::GL) {
		return None;
	}

	// LB13. Nothing breaks before a closing bracket or the punctuation that clings to a word.
	if is(cb, &[L::EX, L::CL, L::CP, L::SY]) {
		return None;
	}

	// LB14. An opening bracket holds on to what follows it, across any spaces.
	if cl(cls, skip_back(cls, p, &[L::SP])) == Some(L::OP) {
		return None;
	}

	// LB15a, LB15b. An opening quotation mark holds on to what follows it, and a closing one to
	// what precedes it.
	if let Some(k) = skip_back(cls, p, &[L::SP]) {
		if cl(cls, Some(k)) == Some(L::QU) && has(cls, Some(k), FLAG_PI) {
			let before = k.checked_sub(1);
			if before.is_none()
				|| is(cl(cls, before), &[L::BK, L::CR, L::LF, L::NL, L::OP, L::QU, L::GL, L::SP,
					L::ZW])
			{
				return None;
			}
		}
	}
	if cb == Some(L::QU) && has(cls, Some(q), FLAG_PF) {
		if cr.is_none()
			|| is(cr, &[L::SP, L::GL, L::WJ, L::CL, L::QU, L::CP, L::EX, L::IS, L::SY, L::BK,
				L::CR, L::LF, L::NL, L::ZW])
		{
			return None;
		}
	}

	// LB15c, LB15d. A separator that begins a number breaks after a space, but otherwise binds.
	if ca == Some(L::SP) && cb == Some(L::IS) && cr == Some(L::NU) {
		return Some(Break::Optional);
	}
	if cb == Some(L::IS) {
		return None;
	}

	// LB16, LB17. A nonstarter clings to the bracket before it, and an em dash to an em dash.
	if cb == Some(L::NS) && is(cl(cls, skip_back(cls, p, &[L::SP])), &[L::CL, L::CP]) {
		return None;
	}
	if cb == Some(L::B2) && cl(cls, skip_back(cls, p, &[L::SP])) == Some(L::B2) {
		return None;
	}

	// LB18. A space breaks after.
	if ca == Some(L::SP) {
		return Some(Break::Optional);
	}

	// LB19, LB19a. Quotation marks bind, except where an East Asian character makes the side
	// unambiguous.
	if cb == Some(L::QU) && !has(cls, Some(q), FLAG_PI) {
		return None;
	}
	if ca == Some(L::QU) && !has(cls, Some(p), FLAG_PF) {
		return None;
	}
	if !ea(cls, Some(p)) && cb == Some(L::QU) {
		return None;
	}
	if cb == Some(L::QU) && (cr.is_none() || !ea(cls, r)) {
		return None;
	}
	if ca == Some(L::QU) && !ea(cls, Some(q)) {
		return None;
	}
	if ca == Some(L::QU) && (o.is_none() || !ea(cls, o)) {
		return None;
	}

	// LB20. A contingent break breaks on both sides.
	if cb == Some(L::CB) || ca == Some(L::CB) {
		return Some(Break::Optional);
	}

	// LB20a. A hyphen that begins a word holds on to it.
	if is(ca, &[L::HY, L::HH]) && is(cb, &[L::AL, L::HL])
		&& (o.is_none()
			|| is(cl(cls, o), &[L::BK, L::CR, L::LF, L::NL, L::SP, L::ZW, L::CB, L::GL]))
	{
		return None;
	}

	// LB21, LB21a, LB21b. A break clings to the character before a hyphen, and to a Hebrew letter.
	if is(cb, &[L::BA, L::HH, L::HY, L::NS]) || ca == Some(L::BB) {
		return None;
	}
	if is(ca, &[L::HY, L::HH]) && co == Some(L::HL) && cb != Some(L::HL) {
		return None;
	}
	if ca == Some(L::SY) && cb == Some(L::HL) {
		return None;
	}

	// LB22. Nothing breaks before an inseparable character, such as an ellipsis.
	if cb == Some(L::IN) {
		return None;
	}

	// LB23, LB23a. Letters and numbers bind to each other, as do numeric prefixes and ideographs.
	if is(ca, &[L::AL, L::HL]) && cb == Some(L::NU) {
		return None;
	}
	if ca == Some(L::NU) && is(cb, &[L::AL, L::HL]) {
		return None;
	}
	if ca == Some(L::PR) && is(cb, &[L::ID, L::EB, L::EM]) {
		return None;
	}
	if is(ca, &[L::ID, L::EB, L::EM]) && cb == Some(L::PO) {
		return None;
	}

	// LB24. A numeric prefix or postfix binds to a letter.
	if is(ca, &[L::PR, L::PO]) && is(cb, &[L::AL, L::HL]) {
		return None;
	}
	if is(ca, &[L::AL, L::HL]) && is(cb, &[L::PR, L::PO]) {
		return None;
	}

	// LB25. A number does not break apart, and holds on to the currency and percent signs around
	// it.
	if is(ca, &[L::CL, L::CP]) && is(cb, &[L::PO, L::PR]) {
		if let Some(k) = p.checked_sub(1) {
			if numeric_run(cls, k) {
				return None;
			}
		}
	}
	if is(cb, &[L::PO, L::PR]) && numeric_run(cls, p) {
		return None;
	}
	if is(ca, &[L::PO, L::PR]) && cb == Some(L::OP) && cr == Some(L::NU) {
		return None;
	}
	if is(ca, &[L::PO, L::PR]) && cb == Some(L::OP) && cr == Some(L::IS) && ct == Some(L::NU) {
		return None;
	}
	if is(ca, &[L::PO, L::PR, L::HY, L::IS]) && cb == Some(L::NU) {
		return None;
	}
	if cb == Some(L::NU) && numeric_run(cls, p) {
		return None;
	}

	// LB26, LB27. A Hangul syllable does not break apart, and binds to the numeric affixes.
	if ca == Some(L::JL) && is(cb, &[L::JL, L::JV, L::H2, L::H3]) {
		return None;
	}
	if is(ca, &[L::JV, L::H2]) && is(cb, &[L::JV, L::JT]) {
		return None;
	}
	if is(ca, &[L::JT, L::H3]) && cb == Some(L::JT) {
		return None;
	}
	if is(ca, &[L::JL, L::JV, L::JT, L::H2, L::H3]) && cb == Some(L::PO) {
		return None;
	}
	if ca == Some(L::PR) && is(cb, &[L::JL, L::JV, L::JT, L::H2, L::H3]) {
		return None;
	}

	// LB28. Letters bind to letters.
	if is(ca, &[L::AL, L::HL]) && is(cb, &[L::AL, L::HL]) {
		return None;
	}

	// LB28a. A Brahmic orthographic syllable does not break apart.
	if ca == Some(L::AP) && aksara(cls, Some(q)) {
		return None;
	}
	if aksara(cls, Some(p)) && is(cb, &[L::VF, L::VI]) {
		return None;
	}
	if ca == Some(L::VI) && aksara(cls, o) && (cb == Some(L::AK) || has_dot(cls, Some(q))) {
		return None;
	}
	if aksara(cls, Some(p)) && aksara(cls, Some(q)) && cr == Some(L::VF) {
		return None;
	}

	// LB29. A numeric separator binds to the letter after it.
	if ca == Some(L::IS) && is(cb, &[L::AL, L::HL]) {
		return None;
	}

	// LB30. A letter or number binds to the narrow bracket beside it.
	if is(ca, &[L::AL, L::HL, L::NU]) && cb == Some(L::OP) && !ea(cls, Some(q)) {
		return None;
	}
	if ca == Some(L::CP) && !ea(cls, Some(p)) && is(cb, &[L::AL, L::HL, L::NU]) {
		return None;
	}

	// LB30a. Regional indicators pair up into flags, so a break falls between pairs.
	if ca == Some(L::RI) && cb == Some(L::RI) {
		if regional_run(cls, p) % 2 == 1 {
			return None;
		}
		return Some(Break::Optional);
	}

	// LB30b. An emoji keeps its modifier.
	if ca == Some(L::EB) && cb == Some(L::EM) {
		return None;
	}
	if has(cls, Some(p), FLAG_EXT_PICT_UNASSIGNED) && cb == Some(L::EM) {
		return None;
	}

	// LB31.
	Some(Break::Optional)
}

/// Whether cluster `q` is the dotted circle.
fn has_dot(cls: &[Cl], q: Option<usize>) -> bool {
	match q.and_then(|q| cls.get(q)) {
		Some(c) => c.dot,
		None => false,
	}
}
