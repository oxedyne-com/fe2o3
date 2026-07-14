//! Normalisation, following UAX #15.
//!
//! The four normalisation forms differ along two axes: whether the decomposition is canonical or
//! compatibility, and whether the result is left decomposed or recomposed. Each form decomposes
//! the text, puts the combining marks into canonical order, and, for the composed forms, recombines
//! what it can.
//!
//! ```
//! use oxedyne_fe2o3_text::unicode::norm::{
//!     self,
//!     Form,
//! };
//!
//! // The same text, spelled two ways.
//! assert_eq!(norm::nfc("A\u{030A}"), "\u{00C5}");
//! assert_eq!(norm::nfd("\u{00C5}"), "A\u{030A}");
//!
//! // A compatibility form folds away a presentation difference.
//! assert_eq!(norm::normalise("\u{FB01}", Form::Nfkc), "fi");
//! ```

use crate::unicode::{
	lookup,
	tables::norm::{
		CANON_KEYS,
		CANON_OFFS,
		CANON_POOL,
		CCC_STARTS,
		CCC_VALS,
		COMPAT_KEYS,
		COMPAT_OFFS,
		COMPAT_POOL,
		COMPOSE_FIRST,
		COMPOSE_SECOND,
		COMPOSE_VALS,
	},
};

use oxedyne_fe2o3_core::prelude::*;

/// The first Hangul syllable.
const S_BASE: u32	= 0xAC00;
/// The first Hangul leading jamo.
const L_BASE: u32	= 0x1100;
/// The first Hangul vowel jamo.
const V_BASE: u32	= 0x1161;
/// One before the first Hangul trailing jamo, which is why a trailing jamo index of zero means
/// there is none.
const T_BASE: u32	= 0x11A7;
/// The number of Hangul leading jamo.
const L_COUNT: u32	= 19;
/// The number of Hangul vowel jamo.
const V_COUNT: u32	= 21;
/// The number of Hangul trailing jamo, counting the absent one.
const T_COUNT: u32	= 28;
/// The number of Hangul syllables per leading jamo.
const N_COUNT: u32	= V_COUNT * T_COUNT;
/// The number of Hangul syllables.
const S_COUNT: u32	= L_COUNT * N_COUNT;

/// A Unicode normalisation form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
	/// Canonical decomposition followed by canonical composition.
	Nfc,
	/// Canonical decomposition.
	Nfd,
	/// Compatibility decomposition followed by canonical composition.
	Nfkc,
	/// Compatibility decomposition.
	Nfkd,
}

impl Form {

	/// Whether the form decomposes compatibility as well as canonical equivalents.
	pub fn is_compat(&self) -> bool {
		matches!(self, Self::Nfkc | Self::Nfkd)
	}

	/// Whether the form recomposes after decomposing.
	pub fn is_composed(&self) -> bool {
		matches!(self, Self::Nfc | Self::Nfkc)
	}
}

/// Returns the canonical combining class of `c`.
pub fn combining_class(c: char) -> u8 {
	lookup::flags(&CCC_STARTS, &CCC_VALS, c)
}

/// Returns `s` in the given normalisation form.
pub fn normalise(s: &str, form: Form) -> String {

	// Text that is entirely ASCII is already in every form, since no ASCII character decomposes,
	// composes or carries a combining class.
	if s.is_ascii() {
		return s.to_string();
	}

	let mut buf = Vec::with_capacity(s.len());
	for c in s.chars() {
		decompose(c, form.is_compat(), &mut buf);
	}
	order(&mut buf);
	if form.is_composed() {
		buf = compose(&buf);
	}
	buf.into_iter().collect()
}

/// Returns `s` in normalisation form C.
pub fn nfc(s: &str) -> String {
	normalise(s, Form::Nfc)
}

/// Returns `s` in normalisation form D.
pub fn nfd(s: &str) -> String {
	normalise(s, Form::Nfd)
}

/// Returns `s` in normalisation form KC.
pub fn nfkc(s: &str) -> String {
	normalise(s, Form::Nfkc)
}

/// Returns `s` in normalisation form KD.
pub fn nfkd(s: &str) -> String {
	normalise(s, Form::Nfkd)
}

/// Whether `s` is already in the given form.
pub fn is_normalised(s: &str, form: Form) -> bool {
	normalise(s, form) == s
}

/// Whether `a` and `b` are canonically equivalent, that is, whether they are the same text spelled
/// differently.
pub fn eq_canonical(a: &str, b: &str) -> bool {
	nfd(a) == nfd(b)
}

/// Whether `a` and `b` are compatibility equivalent.
pub fn eq_compat(a: &str, b: &str) -> bool {
	nfkd(a) == nfkd(b)
}

/// Appends the full decomposition of `c` to `out`, recursing until nothing decomposes further.
fn decompose(c: char, compat: bool, out: &mut Vec<char>) {

	let cp = c as u32;

	// A Hangul syllable decomposes arithmetically rather than by table.
	if cp >= S_BASE && cp < S_BASE + S_COUNT {
		let i	= cp - S_BASE;
		let l	= L_BASE + i / N_COUNT;
		let v	= V_BASE + (i % N_COUNT) / T_COUNT;
		let t	= T_BASE + i % T_COUNT;
		push(out, l);
		push(out, v);
		if i % T_COUNT != 0 {
			push(out, t);
		}
		return;
	}

	if compat {
		if let Some(seq) = mapping(&COMPAT_KEYS, &COMPAT_OFFS, &COMPAT_POOL, c) {
			for d in seq {
				decompose(*d, compat, out);
			}
			return;
		}
	}

	if let Some(seq) = mapping(&CANON_KEYS, &CANON_OFFS, &CANON_POOL, c) {
		for d in seq {
			decompose(*d, compat, out);
		}
		return;
	}

	out.push(c);
}

/// Returns the decomposition of `c` in one of the mapping tables.
fn mapping<'a>(
	keys:	&[u32],
	offs:	&[u32],
	pool:	&'a [char],
	c:		char,
)
	-> Option<&'a [char]>
{
	let i = match lookup::find(keys, c) {
		Some(i) => i,
		None => return None,
	};
	let a = lookup::get(offs, i, 0) as usize;
	let b = lookup::get(offs, i + 1, 0) as usize;
	Some(lookup::pool(pool, a, b))
}

/// Puts the combining marks of `buf` into canonical order, which is a stable sort of each run of
/// non-starters by combining class.
fn order(buf: &mut [char]) {
	let n = buf.len();
	if n < 2 {
		return;
	}
	// An insertion sort, which is stable, and which touches nothing outside a run of marks because
	// a starter has class zero and so never moves.
	for i in 1..n {
		let c	= buf[i];
		let cc	= combining_class(c);
		if cc == 0 {
			continue;
		}
		let mut j = i;
		while j > 0 {
			let prev = combining_class(buf[j - 1]);
			if prev <= cc {
				break;
			}
			buf[j] = buf[j - 1];
			j -= 1;
		}
		buf[j] = c;
	}
}

/// Recombines a decomposed, canonically ordered sequence.
fn compose(buf: &[char]) -> Vec<char> {

	let mut out: Vec<char>	= Vec::with_capacity(buf.len());
	let mut starter			= None;	// Index in `out` of the last starter
	let mut prev_ccc		= 0u8;	// Class of the character last appended

	for c in buf {
		let cc = combining_class(*c);
		if let Some(li) = starter {
			// The character is blocked from the starter if anything between them has a class that
			// is zero, or is not lower than its own. The sequence is canonically ordered, so the
			// character immediately before it carries the highest class of the run.
			let adjacent	= out.len() == li + 1;
			let blocked		= !adjacent && prev_ccc >= cc;
			if !blocked {
				if let Some(comp) = primary_composite(lookup::get(&out, li, *c), *c) {
					if let Some(slot) = out.get_mut(li) {
						*slot = comp;
						continue;
					}
				}
			}
		}
		if cc == 0 {
			starter = Some(out.len());
		}
		prev_ccc = cc;
		out.push(*c);
	}

	out
}

/// Returns the primary composite of `a` and `b`, if the pair has one.
fn primary_composite(a: char, b: char) -> Option<char> {

	let (x, y) = (a as u32, b as u32);

	// Hangul composes arithmetically: a leading and a vowel jamo make an LV syllable, and an LV
	// syllable and a trailing jamo make an LVT syllable.
	if x >= L_BASE && x < L_BASE + L_COUNT && y >= V_BASE && y < V_BASE + V_COUNT {
		let li = x - L_BASE;
		let vi = y - V_BASE;
		return char::from_u32(S_BASE + (li * V_COUNT + vi) * T_COUNT);
	}
	if x >= S_BASE && x < S_BASE + S_COUNT && (x - S_BASE) % T_COUNT == 0
		&& y > T_BASE && y < T_BASE + T_COUNT
	{
		return char::from_u32(x + (y - T_BASE));
	}

	// The remaining composites are a sorted table of pairs.
	let mut lo = 0usize;
	let mut hi = COMPOSE_FIRST.len();
	while lo < hi {
		let mid	= lo + (hi - lo) / 2;
		let fa	= lookup::get(&COMPOSE_FIRST, mid, 0);
		let fb	= lookup::get(&COMPOSE_SECOND, mid, 0);
		if (fa, fb) < (x, y) {
			lo = mid + 1;
		} else if (fa, fb) > (x, y) {
			hi = mid;
		} else {
			return COMPOSE_VALS.get(mid).copied();
		}
	}

	None
}

/// Pushes a code point that a Hangul index has produced, which is always a valid character.
fn push(out: &mut Vec<char>, cp: u32) {
	if let Some(c) = char::from_u32(cp) {
		out.push(c);
	}
}

/// Returns an error if a generated normalisation table has lost an invariant the lookups rely on.
/// The `tables_are_consistent` test calls it.
pub fn check_tables() -> Outcome<()> {

	if CANON_OFFS.len() != CANON_KEYS.len() + 1 {
		return Err(err!(
			"CANON_OFFS has {} entries, expected {} for {} keys.",
			CANON_OFFS.len(), CANON_KEYS.len() + 1, CANON_KEYS.len(); Bug, Mismatch, Size));
	}
	if COMPAT_OFFS.len() != COMPAT_KEYS.len() + 1 {
		return Err(err!(
			"COMPAT_OFFS has {} entries, expected {} for {} keys.",
			COMPAT_OFFS.len(), COMPAT_KEYS.len() + 1, COMPAT_KEYS.len(); Bug, Mismatch, Size));
	}
	if lookup::get(&CANON_OFFS, CANON_KEYS.len(), 0) as usize != CANON_POOL.len() {
		return Err(err!("The last CANON_OFFS entry does not end the pool."; Bug, Mismatch));
	}
	if lookup::get(&COMPAT_OFFS, COMPAT_KEYS.len(), 0) as usize != COMPAT_POOL.len() {
		return Err(err!("The last COMPAT_OFFS entry does not end the pool."; Bug, Mismatch));
	}
	if COMPOSE_FIRST.len() != COMPOSE_SECOND.len() || COMPOSE_FIRST.len() != COMPOSE_VALS.len() {
		return Err(err!(
			"The composition tables are {}, {} and {} long, and must agree.",
			COMPOSE_FIRST.len(), COMPOSE_SECOND.len(), COMPOSE_VALS.len(); Bug, Mismatch, Size));
	}
	for i in 1..COMPOSE_FIRST.len() {
		let a = (lookup::get(&COMPOSE_FIRST, i - 1, 0), lookup::get(&COMPOSE_SECOND, i - 1, 0));
		let b = (lookup::get(&COMPOSE_FIRST, i, 0), lookup::get(&COMPOSE_SECOND, i, 0));
		if a >= b {
			return Err(err!(
				"The composition table is not sorted at entry {}.", i; Bug, Order));
		}
	}
	for i in 1..CCC_STARTS.len() {
		if lookup::get(&CCC_STARTS, i - 1, 0) >= lookup::get(&CCC_STARTS, i, 0) {
			return Err(err!(
				"The combining class table is not sorted at entry {}.", i; Bug, Order));
		}
	}

	// Every partition table must begin at U+0000, or a lookup below its first run start would fall
	// off the front.
	if lookup::get(&CCC_STARTS, 0, 1) != 0 {
		return Err(err!("The combining class table does not begin at U+0000."; Bug, Invalid));
	}

	Ok(())
}
