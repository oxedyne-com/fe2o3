//! The bidirectional algorithm, following UAX #9.
//!
//! Text that mixes a right to left script with a left to right one is stored in the order it is
//! read, not the order it is drawn. The algorithm resolves an embedding level for every character,
//! and from those levels a renderer can put the characters in the order they appear on the line.
//!
//! The whole string is taken as one paragraph. A caller with several paragraphs should split them
//! first, which is rule P1, and resolve each on its own.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::bidi::{
//!     self,
//!     Direction,
//! };
//!
//! // A Hebrew word between two English ones.
//! let info = bidi::resolve("a \u{05D0}\u{05D1} b", Direction::Auto);
//! assert_eq!(info.para_level, 0);
//! assert_eq!(info.levels[2], 1); // The Hebrew runs right to left.
//! ```

use crate::unicode::{
	lookup::{
		self,
		Partitioned,
	},
	prop::{
		BidiClass as B,
		BracketKind,
	},
	tables::{
		bidi::{
			BRACKET_KEYS,
			BRACKET_KINDS,
			BRACKET_PAIRS,
		},
		norm::{
			CANON_KEYS,
			CANON_OFFS,
			CANON_POOL,
		},
	},
};

/// The deepest embedding the algorithm allows, from BD2.
const MAX_DEPTH: u8 = 125;

/// The most bracket pairs BD16 will track before it gives up on the rest of the sequence.
const MAX_PAIRS: usize = 63;

/// The direction a paragraph is laid out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
	/// Left to right, whatever the text says.
	Ltr,
	/// Right to left, whatever the text says.
	Rtl,
	/// Taken from the first strong character, by rules P2 and P3.
	Auto,
}

/// The resolved levels of a paragraph.
#[derive(Clone, Debug)]
pub struct BidiInfo {
	/// The embedding level of the paragraph, even for left to right and odd for right to left.
	pub para_level:	u8,
	/// The embedding level of each character.
	pub levels:		Vec<u8>,
	/// The Bidi_Class of each character, as it was before the algorithm resolved anything.
	pub classes:	Vec<B>,
	/// Whether rule X9 removed each character, which is to say whether it is an embedding, an
	/// override, a pop or a boundary neutral. A renderer draws nothing for these, and their level
	/// means nothing.
	pub removed:	Vec<bool>,
	/// The byte offset of each character in the string.
	pub offsets:	Vec<usize>,
}

impl BidiInfo {

	/// Returns the indices of the characters in the order they are drawn, left to right, leaving
	/// out the characters that rule X9 removed. This is rule L2.
	pub fn visual_order(&self) -> Vec<usize> {

		let keep: Vec<usize> = (0..self.levels.len())
			.filter(|i| !lookup::get(&self.removed, *i, true))
			.collect();

		let lv: Vec<u8> = keep.iter()
			.map(|i| lookup::get(&self.levels, *i, self.para_level))
			.collect();

		let mut order = keep;
		let hi = lv.iter().copied().max().unwrap_or(self.para_level);
		let lo = lv.iter()
			.copied()
			.filter(|l| l % 2 == 1)
			.min()
			.unwrap_or(hi.saturating_add(1));

		// Reverse every run at the deepest level, then at each level above the lowest odd one.
		let mut level = hi;
		while level >= lo && level > 0 {
			let mut i = 0;
			while i < lv.len() {
				if lookup::get(&lv, i, 0) >= level {
					let mut j = i;
					while j < lv.len() && lookup::get(&lv, j, 0) >= level {
						j += 1;
					}
					order[i..j].reverse();
					i = j;
				} else {
					i += 1;
				}
			}
			level -= 1;
		}

		order
	}

	/// Whether any character needs the algorithm at all, which is to say whether the text is not
	/// simply left to right.
	pub fn has_rtl(&self) -> bool {
		self.para_level % 2 == 1 || self.levels.iter().any(|l| l % 2 == 1)
	}
}

/// Resolves the embedding levels of `s`, taken as one paragraph.
pub fn resolve(s: &str, dir: Direction) -> BidiInfo {

	let chars:		Vec<char>	= s.chars().collect();
	let offsets:	Vec<usize>	= s.char_indices().map(|(i, _)| i).collect();
	let classes:	Vec<B>		= chars.iter().map(|c| B::of(*c)).collect();

	let para_level = match dir {
		Direction::Ltr	=> 0,
		Direction::Rtl	=> 1,
		Direction::Auto	=> first_strong(&classes, 0, classes.len()),
	};

	resolve_levels(&chars, &classes, para_level, offsets)
}

/// Resolves the embedding levels of a sequence whose classes are already known, which is what the
/// Unicode conformance test gives.
pub fn resolve_classes(chars: &[char], classes: &[B], dir: Direction) -> BidiInfo {

	let para_level = match dir {
		Direction::Ltr	=> 0,
		Direction::Rtl	=> 1,
		Direction::Auto	=> first_strong(classes, 0, classes.len()),
	};

	let offsets = (0..chars.len()).collect();
	resolve_levels(chars, classes, para_level, offsets)
}

/// Returns the level the first strong character in `cls[from..to]` calls for, skipping anything
/// inside an isolate. This is rules P2 and P3.
fn first_strong(cls: &[B], from: usize, to: usize) -> u8 {
	let mut depth = 0usize;
	for i in from..to {
		match lookup::get(cls, i, B::ON) {
			B::LRI | B::RLI | B::FSI	=> depth += 1,
			B::PDI						=> depth = depth.saturating_sub(1),
			B::L if depth == 0			=> return 0,
			B::R | B::AL if depth == 0	=> return 1,
			_							=> (),
		}
	}
	0
}

/// Returns the index of the PDI that closes the isolate initiator at `i`, or the length of the
/// text if there is none. This is rule BD9.
fn matching_pdi(cls: &[B], i: usize) -> usize {
	let mut depth = 1usize;
	for j in (i + 1)..cls.len() {
		match lookup::get(cls, j, B::ON) {
			B::LRI | B::RLI | B::FSI => depth += 1,
			B::PDI => {
				depth -= 1;
				if depth == 0 {
					return j;
				}
			},
			_ => (),
		}
	}
	cls.len()
}

/// An entry on the directional status stack of rules X1 to X8.
#[derive(Clone, Copy)]
struct Status {
	/// The embedding level the entry establishes.
	level:	u8,
	/// The direction the entry forces on the characters it covers, if it forces one.
	over:	Option<B>,
	/// Whether the entry was pushed by an isolate initiator.
	iso:	bool,
}

/// The whole of the algorithm, once the paragraph level is settled.
fn resolve_levels(
	chars:		&[char],
	orig:		&[B],
	para_level:	u8,
	offsets:	Vec<usize>,
)
	-> BidiInfo
{
	let n = orig.len();

	// X1 to X8. The explicit embeddings, overrides and isolates give every character a level, and
	// take the formatting characters themselves out of the text.
	let mut levels	= vec![para_level; n];
	let mut cls		= orig.to_vec();
	let mut removed	= vec![false; n];

	let mut stack = vec![Status { level: para_level, over: None, iso: false }];
	let mut overflow_iso	= 0usize;
	let mut overflow_emb	= 0usize;
	let mut valid_iso		= 0usize;

	for i in 0..n {
		let last = match stack.last() {
			Some(s) => *s,
			None => Status { level: para_level, over: None, iso: false },
		};
		match lookup::get(orig, i, B::ON) {

			// X2 to X5. An embedding or an override raises the level.
			c @ (B::RLE | B::LRE | B::RLO | B::LRO) => {
				levels[i]	= last.level;
				removed[i]	= true;
				let rtl		= matches!(c, B::RLE | B::RLO);
				let next	= next_level(last.level, rtl);
				let over	= match c {
					B::RLO => Some(B::R),
					B::LRO => Some(B::L),
					_ => None,
				};
				if next <= MAX_DEPTH && overflow_iso == 0 && overflow_emb == 0 {
					stack.push(Status { level: next, over, iso: false });
				} else if overflow_iso == 0 {
					overflow_emb += 1;
				}
			},

			// X5a, X5b, X5c. An isolate raises the level, but unlike an embedding it stays in the
			// text and hides its contents from the rules outside it.
			c @ (B::RLI | B::LRI | B::FSI) => {
				let rtl = match c {
					B::RLI => true,
					B::LRI => false,
					// X5c. A first strong isolate takes its direction from what it holds.
					_ => first_strong(orig, i + 1, matching_pdi(orig, i)) == 1,
				};
				levels[i] = last.level;
				if let Some(o) = last.over {
					cls[i] = o;
				}
				let next = next_level(last.level, rtl);
				if next <= MAX_DEPTH && overflow_iso == 0 && overflow_emb == 0 {
					valid_iso += 1;
					stack.push(Status { level: next, over: None, iso: true });
				} else {
					overflow_iso += 1;
				}
			},

			// X6a. A pop directional isolate closes the nearest isolate that is still open.
			B::PDI => {
				if overflow_iso > 0 {
					overflow_iso -= 1;
				} else if valid_iso > 0 {
					overflow_emb = 0;
					while stack.last().map(|s| !s.iso).unwrap_or(false) {
						stack.pop();
					}
					stack.pop();
					valid_iso -= 1;
				}
				let now = match stack.last() {
					Some(s) => *s,
					None => Status { level: para_level, over: None, iso: false },
				};
				levels[i] = now.level;
				if let Some(o) = now.over {
					cls[i] = o;
				}
			},

			// X7. A pop directional format closes the nearest embedding or override.
			B::PDF => {
				levels[i]	= last.level;
				removed[i]	= true;
				if overflow_iso > 0 {
					// An isolate is still open, so this pop belongs to nothing.
				} else if overflow_emb > 0 {
					overflow_emb -= 1;
				} else if !last.iso && stack.len() >= 2 {
					stack.pop();
				}
			},

			// X8. A paragraph separator sits at the paragraph level.
			B::B => {
				levels[i] = para_level;
			},

			// X9. A boundary neutral leaves the text.
			B::BN => {
				levels[i]	= last.level;
				removed[i]	= true;
			},

			// X6. Everything else takes the level, and the direction, of the entry it sits under.
			_ => {
				levels[i] = last.level;
				if let Some(o) = last.over {
					cls[i] = o;
				}
			},
		}
	}

	// BD13. The characters that survive X9 fall into level runs, and the runs join up across the
	// isolates that link them into isolating run sequences.
	let seqs = sequences(orig, &levels, &removed);

	// X10. Each sequence needs to know what lies beyond each of its ends.
	let mut ends = Vec::with_capacity(seqs.len());
	for seq in &seqs {
		ends.push(surrounding(orig, &levels, &removed, seq, para_level));
	}

	// W, N and I, one sequence at a time. The levels only change at the end, so that the sos and
	// eos above were all read from the explicit levels.
	let mut resolved: Vec<(usize, u8)> = Vec::new();
	for (seq, (sos, eos)) in seqs.iter().zip(ends.iter()) {
		let level = seq.first()
			.and_then(|i| levels.get(*i))
			.copied()
			.unwrap_or(para_level);
		let mut sc: Vec<B> = seq.iter()
			.map(|i| lookup::get(&cls, *i, B::ON))
			.collect();

		// Rule N0 asks what a character was before W1 changed it, so the classes are kept.
		let entering = sc.clone();
		weak(&mut sc, *sos);
		neutral(chars, seq, &mut sc, &entering, level, *sos, *eos);

		// I1, I2. A run of the opposite direction, or a number, sits one or two levels deeper.
		for (k, c) in sc.iter().enumerate() {
			let mut lv = level;
			if level % 2 == 0 {
				match c {
					B::R			=> lv = level.saturating_add(1),
					B::AN | B::EN	=> lv = level.saturating_add(2),
					_				=> (),
				}
			} else if matches!(c, B::L | B::EN | B::AN) {
				lv = level.saturating_add(1);
			}
			if let Some(i) = seq.get(k) {
				resolved.push((*i, lv));
			}
		}
	}
	for (i, lv) in resolved {
		if let Some(slot) = levels.get_mut(i) {
			*slot = lv;
		}
	}

	// L1. Separators, and the whitespace that runs up to them or to the end of the paragraph, go
	// back to the paragraph level.
	for i in 0..n {
		if matches!(lookup::get(orig, i, B::ON), B::S | B::B) {
			levels[i] = para_level;
			reset_before(orig, &mut levels, &removed, i, para_level);
		}
	}
	reset_before(orig, &mut levels, &removed, n, para_level);

	BidiInfo {
		para_level,
		levels,
		classes: orig.to_vec(),
		removed,
		offsets,
	}
}

/// Returns the next level above `level` in the given direction, which rules X2 to X5b call the
/// least odd or least even level greater than it.
fn next_level(level: u8, rtl: bool) -> u8 {
	if rtl {
		(level + 1) | 1
	} else {
		(level + 2) & !1
	}
}

/// Walks back from `i` over the whitespace and isolate formatting characters, and the characters
/// X9 removed, putting them back at the paragraph level. This is the second half of rule L1.
fn reset_before(
	orig:		&[B],
	levels:		&mut [u8],
	removed:	&[bool],
	i:			usize,
	para_level:	u8,
) {
	let mut j = i;
	while j > 0 {
		j -= 1;
		if lookup::get(removed, j, false) {
			levels[j] = para_level;
			continue;
		}
		match lookup::get(orig, j, B::ON) {
			B::WS | B::LRI | B::RLI | B::FSI | B::PDI => levels[j] = para_level,
			_ => break,
		}
	}
}

/// Builds the isolating run sequences of rule BD13.
fn sequences(orig: &[B], levels: &[u8], removed: &[bool]) -> Vec<Vec<usize>> {

	// The characters X9 did not remove, in order.
	let keep: Vec<usize> = (0..orig.len())
		.filter(|i| !lookup::get(removed, *i, true))
		.collect();

	// The level runs: maximal stretches of the surviving characters at one level.
	let mut runs: Vec<Vec<usize>> = Vec::new();
	for i in keep {
		let lv = lookup::get(levels, i, 0);
		let same = runs.last()
			.and_then(|r| r.last())
			.map(|last| lookup::get(levels, *last, 0) == lv)
			.unwrap_or(false);
		if same {
			if let Some(r) = runs.last_mut() {
				r.push(i);
			}
		} else {
			runs.push(vec![i]);
		}
	}

	// A run beginning with a PDI that closes an isolate belongs to the sequence of that isolate,
	// so it does not begin one of its own.
	let closes: Vec<bool> = runs.iter()
		.map(|r| match r.first() {
			Some(i) => lookup::get(orig, *i, B::ON) == B::PDI && has_initiator(orig, *i),
			None => false,
		})
		.collect();

	let mut seqs = Vec::new();
	for (r, run) in runs.iter().enumerate() {
		if lookup::get(&closes, r, false) {
			continue;
		}
		let mut seq = run.clone();
		let mut k = r;
		// While the sequence ends on an isolate initiator that is closed somewhere, the run that
		// begins with its PDI carries on the same sequence.
		loop {
			let last = match seq.last() {
				Some(i) => *i,
				None => break,
			};
			if !matches!(lookup::get(orig, last, B::ON), B::LRI | B::RLI | B::FSI) {
				break;
			}
			let pdi = matching_pdi(orig, last);
			if pdi >= orig.len() {
				break;
			}
			// Find the run that begins with that PDI.
			let mut found = None;
			for (j, run) in runs.iter().enumerate().skip(k + 1) {
				if run.first() == Some(&pdi) {
					found = Some(j);
					break;
				}
			}
			match found {
				Some(j) => {
					if let Some(run) = runs.get(j) {
						seq.extend_from_slice(run);
					}
					k = j;
				},
				None => break,
			}
		}
		seqs.push(seq);
	}

	seqs
}

/// Whether the PDI at `i` closes an isolate initiator.
fn has_initiator(orig: &[B], i: usize) -> bool {
	let mut depth = 0usize;
	let mut j = i;
	while j > 0 {
		j -= 1;
		match lookup::get(orig, j, B::ON) {
			B::PDI => depth += 1,
			B::LRI | B::RLI | B::FSI => {
				if depth == 0 {
					return true;
				}
				depth -= 1;
			},
			_ => (),
		}
	}
	false
}

/// Returns the directions that stand at either end of an isolating run sequence, the sos and eos
/// of rule X10.
fn surrounding(
	orig:		&[B],
	levels:		&[u8],
	removed:	&[bool],
	seq:		&[usize],
	para_level:	u8,
)
	-> (B, B)
{
	let first	= seq.first().copied().unwrap_or(0);
	let last	= seq.last().copied().unwrap_or(0);
	let level	= lookup::get(levels, first, para_level);

	// Before the sequence: the nearest surviving character, or the paragraph itself.
	let mut before = para_level;
	let mut i = first;
	while i > 0 {
		i -= 1;
		if !lookup::get(removed, i, true) {
			before = lookup::get(levels, i, para_level);
			break;
		}
	}

	// After it: the same, unless the sequence ends on an isolate that nothing closes, in which
	// case the paragraph stands beyond it.
	let mut after = para_level;
	let open_isolate = matches!(lookup::get(orig, last, B::ON), B::LRI | B::RLI | B::FSI)
		&& matching_pdi(orig, last) >= orig.len();
	if !open_isolate {
		for i in (last + 1)..orig.len() {
			if !lookup::get(removed, i, true) {
				after = lookup::get(levels, i, para_level);
				break;
			}
		}
	}

	(dir_of(level.max(before)), dir_of(level.max(after)))
}

/// Returns the direction an embedding level stands for.
fn dir_of(level: u8) -> B {
	if level % 2 == 1 {
		B::R
	} else {
		B::L
	}
}

/// Rules W1 to W7, which resolve the weak types of an isolating run sequence in place.
fn weak(sc: &mut [B], sos: B) {

	let n = sc.len();

	// W1. A nonspacing mark takes the type of what it marks, which for a run of marks is the type
	// the mark before it has just taken.
	let mut prev = sos;
	for i in 0..n {
		if lookup::get(sc, i, B::ON) == B::NSM {
			sc[i] = match prev {
				B::LRI | B::RLI | B::FSI | B::PDI => B::ON,
				other => other,
			};
		}
		prev = lookup::get(sc, i, B::ON);
	}

	// W2. A European number after an Arabic letter is an Arabic number.
	let mut strong = sos;
	for i in 0..n {
		match lookup::get(sc, i, B::ON) {
			B::L | B::R | B::AL	=> strong = lookup::get(sc, i, B::ON),
			B::EN if strong == B::AL => sc[i] = B::AN,
			_ => (),
		}
	}

	// W3. An Arabic letter is simply right to left from here on.
	for i in 0..n {
		if lookup::get(sc, i, B::ON) == B::AL {
			sc[i] = B::R;
		}
	}

	// W4. A separator between two numbers of a kind joins them.
	for i in 1..n.saturating_sub(1) {
		let (a, b, c) = (
			lookup::get(sc, i - 1, B::ON),
			lookup::get(sc, i, B::ON),
			lookup::get(sc, i + 1, B::ON),
		);
		if b == B::ES && a == B::EN && c == B::EN {
			sc[i] = B::EN;
		} else if b == B::CS && a == c && matches!(a, B::EN | B::AN) {
			sc[i] = a;
		}
	}

	// W5. A run of terminators beside a European number joins it.
	let mut i = 0;
	while i < n {
		if lookup::get(sc, i, B::ON) != B::ET {
			i += 1;
			continue;
		}
		let mut j = i;
		while j < n && lookup::get(sc, j, B::ON) == B::ET {
			j += 1;
		}
		let before	= i > 0 && lookup::get(sc, i - 1, B::ON) == B::EN;
		let after	= j < n && lookup::get(sc, j, B::ON) == B::EN;
		if before || after {
			for k in i..j {
				sc[k] = B::EN;
			}
		}
		i = j;
	}

	// W6. The separators and terminators that are left are neutral.
	for i in 0..n {
		if matches!(lookup::get(sc, i, B::ON), B::ET | B::ES | B::CS) {
			sc[i] = B::ON;
		}
	}

	// W7. A European number after a left to right character is left to right.
	let mut strong = sos;
	for i in 0..n {
		match lookup::get(sc, i, B::ON) {
			B::L | B::R => strong = lookup::get(sc, i, B::ON),
			B::EN if strong == B::L => sc[i] = B::L,
			_ => (),
		}
	}
}

/// Whether a class is a neutral or an isolate formatting character, the NI of UAX #9.
fn is_ni(c: B) -> bool {
	matches!(c, B::B | B::S | B::WS | B::ON | B::FSI | B::LRI | B::RLI | B::PDI)
}

/// The direction a class stands for when the neutral rules look for a strong one, where a number
/// counts as right to left.
fn strength(c: B) -> Option<B> {
	match c {
		B::L				=> Some(B::L),
		B::R | B::EN | B::AN	=> Some(B::R),
		_					=> None,
	}
}

/// Rules N0 to N2, which resolve the neutral types of an isolating run sequence in place.
fn neutral(
	chars:		&[char],
	seq:		&[usize],
	sc:			&mut [B],
	entering:	&[B],
	level:		u8,
	sos:		B,
	eos:		B,
) {
	let n	= sc.len();
	let e	= dir_of(level);
	let o	= if e == B::L { B::R } else { B::L };

	// N0. A bracket pair takes the direction of what it holds, or of what stands before it.
	for (open, close) in bracket_pairs(chars, seq, sc) {

		let mut inside_e = false;
		let mut inside_o = false;
		for k in (open + 1)..close {
			match strength(lookup::get(sc, k, B::ON)) {
				Some(s) if s == e => {
					inside_e = true;
					break;
				},
				Some(_) => inside_o = true,
				None => (),
			}
		}

		let set = if inside_e {
			Some(e)
		} else if inside_o {
			// The direction before the pair decides, and where it is the opposite direction the
			// brackets follow it rather than the embedding.
			let mut ctx = sos;
			let mut k = open;
			while k > 0 {
				k -= 1;
				if let Some(s) = strength(lookup::get(sc, k, B::ON)) {
					ctx = s;
					break;
				}
			}
			if ctx == o {
				Some(o)
			} else {
				Some(e)
			}
		} else {
			None
		};

		if let Some(d) = set {
			sc[open]	= d;
			sc[close]	= d;
			// A mark that followed a bracket follows it here too.
			for k in [open, close] {
				for m in (k + 1)..n {
					if lookup::get(entering, m, B::ON) != B::NSM {
						break;
					}
					sc[m] = d;
				}
			}
		}
	}

	// N1. A run of neutrals between two characters of the same direction takes that direction.
	// N2. Any other run of neutrals takes the direction of the embedding.
	let mut i = 0;
	while i < n {
		if !is_ni(lookup::get(sc, i, B::ON)) {
			i += 1;
			continue;
		}
		let mut j = i;
		while j < n && is_ni(lookup::get(sc, j, B::ON)) {
			j += 1;
		}
		let before = if i == 0 {
			sos
		} else {
			strength(lookup::get(sc, i - 1, B::ON)).unwrap_or(e)
		};
		let after = if j == n {
			eos
		} else {
			strength(lookup::get(sc, j, B::ON)).unwrap_or(e)
		};
		let d = if before == after { before } else { e };
		for k in i..j {
			sc[k] = d;
		}
		i = j;
	}
}

/// Finds the bracket pairs of an isolating run sequence, in the order they open. This is rule
/// BD16.
fn bracket_pairs(chars: &[char], seq: &[usize], sc: &[B]) -> Vec<(usize, usize)> {

	let mut stack: Vec<(char, usize)> = Vec::new();
	let mut pairs: Vec<(usize, usize)> = Vec::new();

	for (k, i) in seq.iter().enumerate() {
		if lookup::get(sc, k, B::ON) != B::ON {
			continue;
		}
		let c = match chars.get(*i) {
			Some(c) => *c,
			None => continue,
		};
		match bracket_kind(c) {
			BracketKind::Open => {
				if stack.len() >= MAX_PAIRS {
					// BD16 gives up on the rest of the sequence rather than grow the stack.
					break;
				}
				stack.push((canonical(paired(c)), k));
			},
			BracketKind::Close => {
				let want = canonical(c);
				for idx in (0..stack.len()).rev() {
					if let Some((expect, open)) = stack.get(idx) {
						if *expect == want {
							pairs.push((*open, k));
							stack.truncate(idx);
							break;
						}
					}
				}
			},
			BracketKind::None => (),
		}
	}

	pairs.sort();
	pairs
}

/// Returns the kind of bracket `c` is.
fn bracket_kind(c: char) -> BracketKind {
	match lookup::find(&BRACKET_KEYS, c) {
		Some(i) => match lookup::get(&BRACKET_KINDS, i, 2) {
			0 => BracketKind::Open,
			1 => BracketKind::Close,
			_ => BracketKind::None,
		},
		None => BracketKind::None,
	}
}

/// Returns the bracket `c` pairs with, or `c` itself if it is not a bracket.
fn paired(c: char) -> char {
	match lookup::find(&BRACKET_KEYS, c) {
		Some(i) => lookup::get(&BRACKET_PAIRS, i, c),
		None => c,
	}
}

/// Returns the character a bracket is canonically equivalent to, so that the two spellings of the
/// angle brackets pair with each other, as BD16 requires.
fn canonical(c: char) -> char {
	match lookup::find(&CANON_KEYS, c) {
		Some(i) => {
			let a = lookup::get(&CANON_OFFS, i, 0) as usize;
			let b = lookup::get(&CANON_OFFS, i + 1, 0) as usize;
			let seq = lookup::pool(&CANON_POOL, a, b);
			match seq {
				[one] => *one,
				_ => c,
			}
		},
		None => c,
	}
}
