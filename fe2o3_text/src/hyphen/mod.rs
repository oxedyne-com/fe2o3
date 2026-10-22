// SPDX-License-Identifier: MIT OR Apache-2.0
// Original work copyright (c) the hypher authors, Laurenz Mädje (hypher 0.1.6).
// Modified for Hematite: ported to its error handling, returning split offsets.
//! Liang hyphenation for 35 languages, over pattern automata compiled from the TeX patterns.
//!
//! A port of `hypher` 0.1.6 (Laurenz Mädje, MIT or Apache-2.0), the crate Typst hyphenates with, and of
//! its compiled automata (`tries/*.bin`), so a word breaks where Typst breaks it. The patterns behind
//! the automata come from the TeX `hyph-utf8` collection under their own permissive licences (LPPL,
//! MPL, MIT, BSD-3); `tries/NOTICE.txt` names the sources. The module is compiled only with the
//! `hyphenation` feature, since the automata add about 1.1 MiB.

/// A language hyphenation has patterns for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
	Afrikaans,
	Albanian,
	Belarusian,
	Bulgarian,
	Catalan,
	Croatian,
	Czech,
	Danish,
	Dutch,
	English,
	Estonian,
	Finnish,
	French,
	Georgian,
	German,
	Greek,
	Hungarian,
	Icelandic,
	Italian,
	Kurmanji,
	Latin,
	Lithuanian,
	Mongolian,
	Norwegian,
	Polish,
	Portuguese,
	Russian,
	Serbian,
	Slovak,
	Slovenian,
	Spanish,
	Swedish,
	Turkish,
	Turkmen,
	Ukrainian,
}

impl Lang {

	/// The language an ISO 639-1 code names, where there are patterns for it.
	pub fn from_iso(code: &str) -> Option<Self> {
		match code {
			"af"	=> Some(Self::Afrikaans),
			"sq"	=> Some(Self::Albanian),
			"be"	=> Some(Self::Belarusian),
			"bg"	=> Some(Self::Bulgarian),
			"ca"	=> Some(Self::Catalan),
			"hr"	=> Some(Self::Croatian),
			"cs"	=> Some(Self::Czech),
			"da"	=> Some(Self::Danish),
			"nl"	=> Some(Self::Dutch),
			"en"	=> Some(Self::English),
			"et"	=> Some(Self::Estonian),
			"fi"	=> Some(Self::Finnish),
			"fr"	=> Some(Self::French),
			"ka"	=> Some(Self::Georgian),
			"de"	=> Some(Self::German),
			"el"	=> Some(Self::Greek),
			"hu"	=> Some(Self::Hungarian),
			"is"	=> Some(Self::Icelandic),
			"it"	=> Some(Self::Italian),
			"ku"	=> Some(Self::Kurmanji),
			"la"	=> Some(Self::Latin),
			"lt"	=> Some(Self::Lithuanian),
			"mn"	=> Some(Self::Mongolian),
			"no" | "nb" | "nn"	=> Some(Self::Norwegian),
			"pl"	=> Some(Self::Polish),
			"pt"	=> Some(Self::Portuguese),
			"ru"	=> Some(Self::Russian),
			"sr"	=> Some(Self::Serbian),
			"sk"	=> Some(Self::Slovak),
			"sl"	=> Some(Self::Slovenian),
			"es"	=> Some(Self::Spanish),
			"sv"	=> Some(Self::Swedish),
			"tr"	=> Some(Self::Turkish),
			"tk"	=> Some(Self::Turkmen),
			"uk"	=> Some(Self::Ukrainian),
			_	=> None,
		}
	}

	/// The fewest characters the patterns leave before the first break and after the last.
	pub fn bounds(self) -> (usize, usize) {
		match self {
			Self::Afrikaans	=> (1, 2),
			Self::Albanian	=> (2, 2),
			Self::Belarusian	=> (2, 2),
			Self::Bulgarian	=> (2, 2),
			Self::Catalan	=> (2, 2),
			Self::Croatian	=> (2, 2),
			Self::Czech	=> (2, 2),
			Self::Danish	=> (2, 2),
			Self::Dutch	=> (2, 2),
			Self::English	=> (2, 3),
			Self::Estonian	=> (2, 3),
			Self::Finnish	=> (2, 2),
			Self::French	=> (2, 2),
			Self::Georgian	=> (1, 2),
			Self::German	=> (2, 2),
			Self::Greek	=> (1, 1),
			Self::Hungarian	=> (2, 2),
			Self::Icelandic	=> (2, 2),
			Self::Italian	=> (2, 2),
			Self::Kurmanji	=> (2, 2),
			Self::Latin	=> (2, 2),
			Self::Lithuanian	=> (2, 2),
			Self::Mongolian	=> (2, 2),
			Self::Norwegian	=> (2, 2),
			Self::Polish	=> (2, 2),
			Self::Portuguese	=> (2, 3),
			Self::Russian	=> (2, 2),
			Self::Serbian	=> (2, 2),
			Self::Slovak	=> (2, 3),
			Self::Slovenian	=> (2, 2),
			Self::Spanish	=> (2, 2),
			Self::Swedish	=> (2, 2),
			Self::Turkish	=> (2, 2),
			Self::Turkmen	=> (2, 2),
			Self::Ukrainian	=> (2, 2),
		}
	}

	fn data(self) -> &'static [u8] {
		match self {
			Self::Afrikaans	=> include_bytes!("tries/af.bin"),
			Self::Albanian	=> include_bytes!("tries/sq.bin"),
			Self::Belarusian	=> include_bytes!("tries/be.bin"),
			Self::Bulgarian	=> include_bytes!("tries/bg.bin"),
			Self::Catalan	=> include_bytes!("tries/ca.bin"),
			Self::Croatian	=> include_bytes!("tries/hr.bin"),
			Self::Czech	=> include_bytes!("tries/cs.bin"),
			Self::Danish	=> include_bytes!("tries/da.bin"),
			Self::Dutch	=> include_bytes!("tries/nl.bin"),
			Self::English	=> include_bytes!("tries/en.bin"),
			Self::Estonian	=> include_bytes!("tries/et.bin"),
			Self::Finnish	=> include_bytes!("tries/fi.bin"),
			Self::French	=> include_bytes!("tries/fr.bin"),
			Self::Georgian	=> include_bytes!("tries/ka.bin"),
			Self::German	=> include_bytes!("tries/de.bin"),
			Self::Greek	=> include_bytes!("tries/el.bin"),
			Self::Hungarian	=> include_bytes!("tries/hu.bin"),
			Self::Icelandic	=> include_bytes!("tries/is.bin"),
			Self::Italian	=> include_bytes!("tries/it.bin"),
			Self::Kurmanji	=> include_bytes!("tries/ku.bin"),
			Self::Latin	=> include_bytes!("tries/la.bin"),
			Self::Lithuanian	=> include_bytes!("tries/lt.bin"),
			Self::Mongolian	=> include_bytes!("tries/mn.bin"),
			Self::Norwegian	=> include_bytes!("tries/no.bin"),
			Self::Polish	=> include_bytes!("tries/pl.bin"),
			Self::Portuguese	=> include_bytes!("tries/pt.bin"),
			Self::Russian	=> include_bytes!("tries/ru.bin"),
			Self::Serbian	=> include_bytes!("tries/sr.bin"),
			Self::Slovak	=> include_bytes!("tries/sk.bin"),
			Self::Slovenian	=> include_bytes!("tries/sl.bin"),
			Self::Spanish	=> include_bytes!("tries/es.bin"),
			Self::Swedish	=> include_bytes!("tries/sv.bin"),
			Self::Turkish	=> include_bytes!("tries/tr.bin"),
			Self::Turkmen	=> include_bytes!("tries/tk.bin"),
			Self::Ukrainian	=> include_bytes!("tries/uk.bin"),
		}
	}
}

/// The byte offsets inside `word` at which it may be hyphenated in `lang`, in order, the language's own
/// minimum run of characters kept either side.
pub fn hyphenate(word: &str, lang: Lang) -> Vec<usize> {
	let (l, r) = lang.bounds();
	hyphenate_bounded(word, lang, l, r)
}

/// As [`hyphenate`], with `left_min` and `right_min` characters kept either side.
pub fn hyphenate_bounded(word: &str, lang: Lang, left_min: usize, right_min: usize) -> Vec<usize> {
	let data = lang.data();
	let root = match State::root(data) {
		Some(s)	=> s,
		None	=> return Vec::new(),
	};
	let dotted = lowercase_and_dot(word);
	let (min_idx, max_idx) = byte_bounds(word, left_min, right_min);
	let mut levels = vec![0u8; word.len().saturating_sub(1)];
	for start in 0..dotted.len() {
		if !is_char_boundary(dotted[start]) {
			continue;
		}
		let mut state = root;
		for &b in &dotted[start..] {
			match state.transition(b) {
				Some(next) => {
					state = next;
					for (offset, level) in state.levels() {
						let split = start + offset;
						if split >= min_idx && split <= max_idx {
							if let Some(slot) = split.checked_sub(2).and_then(|i| levels.get_mut(i)) {
								*slot = (*slot).max(level);
							}
						}
					}
				}
				None => break,
			}
		}
	}
	levels.iter().enumerate().filter(|(_, l)| *l % 2 == 1).map(|(i, _)| i + 1).collect()
}

/// The word lower-cased where that keeps each character's length, between two dots.
fn lowercase_and_dot(word: &str) -> Vec<u8> {
	let mut out = Vec::with_capacity(word.len() + 2);
	out.push(b'.');
	let mut buf = [0u8; 4];
	for c in word.chars() {
		let mut lower = c.to_lowercase();
		let c = match (lower.next(), lower.next()) {
			(Some(l), None) if l.len_utf8() == c.len_utf8()	=> l,
			_												=> c,
		};
		out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
	}
	out.push(b'.');
	out
}

/// The dotted-word offsets between which a break may fall.
fn byte_bounds(word: &str, left_min: usize, right_min: usize) -> (usize, usize) {
	let left_min = left_min.max(1);
	let right_min = right_min.max(1);
	let min_idx = 1 + word.chars().take(left_min).map(char::len_utf8).sum::<usize>();
	let right: usize = word.chars().rev().take(right_min).map(char::len_utf8).sum();
	let max_idx = (1 + word.len()).saturating_sub(right);
	(min_idx, max_idx)
}

fn is_char_boundary(b: u8) -> bool {
	(b as i8) >= -0x40
}

/// A state of the automaton: its levels, and its transitions by byte to targets `stride` bytes wide.
#[derive(Clone, Copy)]
struct State<'a> {
	data:		&'a [u8],
	addr:		usize,
	stride:		usize,
	levels:		&'a [u8],
	trans:		&'a [u8],
	targets:	&'a [u8],
}

impl<'a> State<'a> {

	fn root(data: &'a [u8]) -> Option<Self> {
		let b = data.get(..4)?;
		let addr = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
		Self::at(data, addr)
	}

	fn at(data: &'a [u8], addr: usize) -> Option<Self> {
		let node = data.get(addr..)?;
		let head = *node.first()?;
		let mut pos = 1;
		let has_levels = head >> 7 != 0;
		let stride = usize::from((head >> 5) & 3);
		let mut count = usize::from(head & 31);
		if count == 31 {
			count = usize::from(*node.get(pos)?);
			pos += 1;
		}
		let mut levels: &[u8] = &[];
		if has_levels {
			let hi = usize::from(*node.get(pos)?) << 4;
			let second = *node.get(pos + 1)?;
			let offset = hi | (usize::from(second) >> 4);
			let len = usize::from(second & 15);
			levels = data.get(offset..offset + len)?;
			pos += 2;
		}
		let trans = node.get(pos..pos + count)?;
		pos += count;
		let targets = node.get(pos..pos + stride * count)?;
		Some(Self { data, addr, stride, levels, trans, targets })
	}

	fn transition(self, b: u8) -> Option<Self> {
		let idx = self.trans.iter().position(|&x| x == b)?;
		let off = self.stride * idx;
		let delta = from_be_bytes(self.targets.get(off..off + self.stride)?)?;
		let next = (self.addr as isize + delta) as usize;
		Self::at(self.data, next)
	}

	fn levels(self) -> impl Iterator<Item = (usize, u8)> + 'a {
		let mut offset = 0;
		self.levels.iter().map(move |&packed| {
			offset += usize::from(packed / 10);
			(offset, packed % 10)
		})
	}
}

/// A signed big-endian offset of one, two or three bytes (three biased by 2^23).
fn from_be_bytes(buf: &[u8]) -> Option<isize> {
	match buf {
		[a]			=> Some(i8::from_be_bytes([*a]) as isize),
		[a, b]		=> Some(i16::from_be_bytes([*a, *b]) as isize),
		[a, b, c]	=> {
			let u = (usize::from(*a) << 16) | (usize::from(*b) << 8) | usize::from(*c);
			Some(u as isize - (1 << 23))
		}
		_			=> None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn joined(word: &str, lang: Lang) -> String {
		let mut out = String::new();
		let mut last = 0;
		for at in hyphenate(word, lang) {
			out.push_str(&word[last..at]);
			out.push('-');
			last = at;
		}
		out.push_str(&word[last..]);
		out
	}

	#[test]
	fn test_english_breaks_as_hypher_does_00() {
		for w in ["wel-come", "walk-ing", "cap-tiVe", "pur-sue", "wHaT-eVeR", "bro-ken", "ex-ten-sive",
			"Prob-a-bil-ity", "rec-og-nize", "hello", "hi", ""] {
			assert_eq!(joined(&w.replace('-', ""), Lang::English), w);
		}
	}

	#[test]
	fn test_german_breaks_as_hypher_does_01() {
		for w in ["Baum", "ge-hen", "Ap-fel", "To-ma-te", "Ein-ga-be-auf-for-de-rung", "Fort-pflan-zungs-lem-ma",
			"stra-te-gie-er-hal-ten-den", "hübsch", "häss-lich", "über-zeu-gen-der"] {
			assert_eq!(joined(&w.replace('-', ""), Lang::German), w);
		}
	}

	#[test]
	fn test_other_scripts_02() {
		assert_eq!(joined("διαμερίσματα", Lang::Greek), "δια-με-ρί-σμα-τα");
		assert_eq!(joined("კარტოფილი", Lang::Georgian), "კარ-ტო-ფი-ლი");
		assert_eq!(joined("wykształciuchy", Lang::Polish), "wy-kształ-ciu-chy");
		assert_eq!(joined("brněnský", Lang::Czech), "br-něn-ský");
	}
}
