//! Liang's pattern hyphenation, the algorithm behind TeX's `\patterns`.
//!
//! A word is padded with boundary dots (`.word.`), every substring is looked up against the pattern
//! set, and the odd/even inter-letter values combine by maximum. An odd value at a gap that clears
//! the left and right minima is a legal hyphenation point. The patterns are Liang's US-English set,
//! embedded as a crate asset (see `patterns/`); the breaker turns each returned point into a flagged
//! discretionary.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

const PATTERNS:	&str = include_str!("../patterns/hyph-en-us.txt");

/// A hyphenator: the pattern set plus the two minima that keep stubs off the line ends.
pub struct Hyphenator {
	left_min:	usize,	// letters that must precede the first break (TeX lefthyphenmin)
	right_min:	usize,	// letters that must follow the last break (TeX righthyphenmin)
	patterns:	HashMap<String, Vec<u8>>,	// letters-and-dots key -> inter-letter values
}

impl Hyphenator {
	/// Builds a hyphenator from pattern text: one pattern per line, `%` comments and blanks ignored.
	pub fn from_patterns(text: &str, left_min: usize, right_min: usize) -> Self {
		let mut patterns = HashMap::new();
		for line in text.lines() {
			let line = line.trim();
			if line.is_empty() || line.starts_with('%') {
				continue;
			}
			let (key, vals) = parse_pattern(line);
			patterns.insert(key, vals);
		}
		Self { left_min, right_min, patterns }
	}

	/// The US-English hyphenator over the embedded Liang patterns, with the usual 2/3 minima.
	pub fn en_us() -> Self {
		Self::from_patterns(PATTERNS, 2, 3)
	}

	/// The char offsets within `word` at which a discretionary hyphen is legal -- each the count of
	/// characters to the left of the break, honouring the minima. A run that is not all ASCII letters,
	/// or too short to break, yields none.
	pub fn hyphenate(&self, word: &str) -> Vec<usize> {
		let lower: Vec<char> = word.chars().flat_map(|c| c.to_lowercase()).collect();
		if lower.len() < self.left_min + self.right_min || lower.len() != word.chars().count() {
			// Case folding that changes the char count (e.g. the German eszett) breaks the offset
			// mapping back to the original word; refuse rather than mis-split.
			return Vec::new();
		}
		if !lower.iter().all(|c| c.is_ascii_lowercase()) {
			return Vec::new();
		}

		// Pad with boundary dots and score every gap by the maximum over all matching substrings.
		let mut s: Vec<char> = Vec::with_capacity(lower.len() + 2);
		s.push('.');
		s.extend_from_slice(&lower);
		s.push('.');
		let n			= s.len();
		let mut val		= vec![0u8; n + 1];
		for i in 0..n {
			let mut key = String::new();
			for j in (i + 1)..=n {
				key.push(s[j - 1]);
				if let Some(vs) = self.patterns.get(&key) {
					for (g, &v) in vs.iter().enumerate() {
						if val[i + g] < v {
							val[i + g] = v;
						}
					}
				}
			}
		}

		// A break after original char `c` (zero-based) reads the gap at `val[c + 2]`: one for the
		// leading dot, one because the gap sits after the char.
		let no			= lower.len();
		let mut pts		= Vec::new();
		for c in 0..no {
			let before	= c + 1;
			let after	= no - before;
			if val[c + 2] % 2 == 1 && before >= self.left_min && after >= self.right_min {
				pts.push(before);
			}
		}
		pts
	}
}

/// Splits a pattern into its letters-and-dots key and the value at each gap. A digit sets the value
/// of the gap it precedes; a missing digit leaves it zero. The value vector is one longer than the
/// key: a score before the first letter through to after the last.
fn parse_pattern(p: &str) -> (String, Vec<u8>) {
	let mut key		= String::new();
	let mut vals	= vec![0u8];
	let mut gap		= 0usize;	// the gap being scored == key.len()
	for c in p.chars() {
		match c.to_digit(10) {
			Some(d)	=> vals[gap] = d as u8,
			None	=> {
				key.push(c);
				gap += 1;
				vals.push(0);
			},
		}
	}
	(key, vals)
}
