//! Daimond's content scrubber, in Rust: every credential in a string replaced by a marker.
//!
//! `www/js/debugshare.js` carries a scrubber that searches each string of a debug feed for the
//! shapes a credential wears and writes `[redacted <shape> #<hash>/<length>]` in the place of
//! each, so that a reader still sees that a key was there and can tell two sightings apart. It is
//! one block of that file, copied into `dev/lens.mjs` and `fe2o3_net/lens/js/redact.js`. This
//! module is the same block in Rust, so that a peer's feed is guarded as a browser's is, and
//! `fe2o3_net/tests/data/lens_scrub.tsv` holds the two together: a table of inputs, each stored
//! as a recipe so that no credential is, and what the JavaScript made of every one.
//!
//! It is a table of its own, beside the one [`super::scan`] reads and not an extension of it.
//! `scan` reads files a person wrote and refuses a commit, so a shape added to it is a shape that
//! stops work, and it excuses a placeholder. This answers for text bound for a log, where all a
//! false hit costs is a word, and it excuses nothing.
//!
//! # Why UTF-16, and why no regular expressions
//!
//! The JavaScript counts and hashes in UTF-16 code units, and a marker's hash and length are how
//! a reader tells one sighting from another, so a string is walked here as `u16`s and a minimum
//! length means what it means there: an astral character counts as two. For the same reason no
//! `regex` stands in this file. JavaScript's `\b` and `/i` are ASCII-only, its `\s` holds U+FEFF
//! and not U+0085, which is the reverse of `char::is_whitespace`, and the crate's own engine
//! follows the `regex` crate on every one of the three.
//!
//! # No run is measured twice
//!
//! Almost every rule is a literal opening and a run of one class of character. A string with an
//! opening every few bytes and one long run behind them would cost a rule that re-measured the run
//! at each opening the square of its length, and a feed's author may be anyone. `Text::run`
//! therefore remembers the last run it measured of each class, and the two rules that look again
//! after a miss step over what they have already decided. The one place the JavaScript backtracks
//! in earnest, an AWS secret beside its key id, is solved outright: its greedy run and its lazy
//! gap together come to "the first separator within 200 units after the run".

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	f64::consts::LN_2,
};


pub const MIN_RUN:	usize	= 32;	// shortest unbroken run the entropy catch weighs
pub const MIN_BITS:	f64		= 4.0;	// bits a character that run must exceed

// Fields whose strings are correlation ids, which a reader joins rows by and a provider may mint
// in an alphabet no different from a key's. They are spared the entropy catch and nothing else.
pub const ID_KEYS: &[&str] = &[
	"id", "mid", "callId", "call_id", "tool_call_id", "toolCallId", "turn", "chat", "chatId",
	"device", "deviceId", "build", "d", "b", "n", "w",
];

const NO_MAX: usize = usize::MAX;

// The characters of a pem header's words, which are held apart so that this file does not itself
// carry a header a scanner looks for.
const BEGIN:	&str = "-----BEGIN ";
const END:		&str = "-----END ";
const KEY:		&str = "PRIVATE KEY";
const DASHES:	&str = "-----";


// What a run of characters after an opening may hold.
#[derive(Clone, Copy)]
enum Class {
	Alnum,		// [A-Za-z0-9]
	Word,		// [A-Za-z0-9_]
	Token,		// [A-Za-z0-9_-]
	Dash,		// [A-Za-z0-9-]
	Upper,		// [0-9A-Z]
	Hex,		// [0-9a-f]
	Std,		// [A-Za-z0-9/+=]
	Bearer,		// [A-Za-z0-9._~+/=-]
	Named,		// [A-Za-z0-9_./+=-]
	Url,		// anything but & " ' # < > [ ] and white space
	Caps,		// [A-Z0-9 ]
	Quote,		// a quote mark or white space
	Space,		// white space
}

const CLASSES: usize = 13;

impl Class {

	fn admits(self, c: u16) -> bool {
		match self {
			Self::Url		=> !matches!(c, 0x26 | 0x22 | 0x27 | 0x23 | 0x3c | 0x3e | 0x5b | 0x5d)
								&& !js_space(c),
			Self::Quote		=> c == 0x22 || c == 0x27 || js_space(c),
			Self::Space		=> js_space(c),
			_ if c > 127	=> false,
			_				=> {
				let b = c as u8;
				let alnum = b.is_ascii_alphanumeric();
				match self {
					Self::Alnum		=> alnum,
					Self::Word		=> alnum || b == b'_',
					Self::Token		=> alnum || b == b'_' || b == b'-',
					Self::Dash		=> alnum || b == b'-',
					Self::Upper		=> b.is_ascii_digit() || b.is_ascii_uppercase(),
					Self::Hex		=> b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
					Self::Std		=> alnum || matches!(b, b'/' | b'+' | b'='),
					Self::Bearer	=> alnum || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-'),
					Self::Named		=> alnum || matches!(b, b'_' | b'-' | b'.' | b'/' | b'+' | b'='),
					Self::Caps		=> b.is_ascii_digit() || b.is_ascii_uppercase() || b == b' ',
					_				=> false,
				}
			},
		}
	}
}

// JavaScript's `\s`.
fn js_space(c: u16) -> bool {
	matches!(c,
		0x09..=0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f
		| 0x3000 | 0xfeff)
}

// JavaScript's `\w`, which is ASCII alone.
fn is_word(c: u16) -> bool {
	c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u16)
}

fn is_digit(c: u16) -> bool {
	(0x30..=0x39).contains(&c)
}

fn is_letter(c: u16) -> bool {
	c < 128 && (c as u8).is_ascii_alphabetic()
}

fn is_hex(c: u16) -> bool {
	c < 128 && (c as u8).is_ascii_hexdigit()
}


// The string being scanned, as UTF-16 units, with the last run measured of each class.
struct Text<'a> {
	s:		&'a [u16],
	runs:	[(usize, usize); CLASSES],	// from and to, every unit between being of the class
}

impl<'a> Text<'a> {

	fn new(s: &'a [u16]) -> Self {
		Self { s, runs: [(0, 0); CLASSES] }
	}

	fn at(&self, i: usize) -> Option<u16> {
		self.s.get(i).copied()
	}

	fn is(&self, i: usize, b: u8) -> bool {
		self.at(i) == Some(b as u16)
	}

	// Does this ASCII text stand here, exactly?
	fn lit(&self, i: usize, lit: &str) -> bool {
		lit.bytes().enumerate().all(|(k, b)| self.is(i + k, b))
	}

	// Does this ASCII text stand here, in either case? JavaScript's `/i` folds ASCII letters only.
	fn lit_ci(&self, i: usize, lit: &str) -> bool {
		lit.bytes().enumerate().all(|(k, b)| match self.at(i + k) {
			Some(c)	=> c < 128 && (c as u8).eq_ignore_ascii_case(&b),
			None	=> false,
		})
	}

	// Where a name ends if it stands here, in either case. A `~` in it is `[_-]?`.
	fn name(&self, i: usize, pat: &str) -> Option<usize> {
		let mut p = i;
		for b in pat.bytes() {
			let c = self.at(p);
			if b == b'~' {
				if c == Some(b'_' as u16) || c == Some(b'-' as u16) {
					p += 1;
				}
			} else {
				match c {
					Some(c) if c < 128 && (c as u8).eq_ignore_ascii_case(&b)	=> p += 1,
					_															=> return None,
				}
			}
		}
		Some(p)
	}

	// Is there a `\b` before a word character at this position?
	fn boundary(&self, i: usize) -> bool {
		i == 0 || !is_word(self.s[i - 1])
	}

	// How many units from here are of the class.
	fn run(&mut self, i: usize, class: Class) -> usize {
		let k = class as usize;
		let (from, to) = self.runs[k];
		if from <= i && i < to {
			return to - i;
		}
		let mut e = i;
		while e < self.s.len() && class.admits(self.s[e]) {
			e += 1;
		}
		if e > i {
			self.runs[k] = (i, e);
		}
		e - i
	}

	// As `run`, counting no further than `cap`.
	fn count(&self, i: usize, class: Class, cap: usize) -> usize {
		let mut e = i;
		while e < self.s.len() && e - i < cap && class.admits(self.s[e]) {
			e += 1;
		}
		e - i
	}
}


// How a shape is recognised.
enum How {
	Pem,
	Jwt,
	Lead {
		leads:	&'static [&'static str],	// the literal openings, any one
		class:	Class,						// what follows
		min:	usize,						// fewest of them
		max:	usize,						// and most, which `{n}` makes the same
	},
}

struct Shape {
	kind:	&'static str,
	how:	How,
}

// Daimond's `SCRUB_SHAPES`, in its order. Each is matched whole and replaced whole, and each
// but the first opens at a word boundary.
const SHAPES: &[Shape] = &[
	Shape { kind: "pem", how: How::Pem },
	Shape { kind: "gh", how: How::Lead {
		leads: &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"], class: Class::Alnum, min: 16, max: NO_MAX } },
	Shape { kind: "ghpat", how: How::Lead {
		leads: &["github_pat_"], class: Class::Word, min: 20, max: NO_MAX } },
	Shape { kind: "stripe", how: How::Lead {
		leads: &["sk_live_", "sk_test_", "rk_live_", "rk_test_"], class: Class::Alnum, min: 10, max: NO_MAX } },
	Shape { kind: "whsec", how: How::Lead {
		leads: &["whsec_"], class: Class::Alnum, min: 16, max: NO_MAX } },
	// `sk-`, `sk-or-v1-`, `sk-ant-api03-`: every provider that took the prefix kept the alphabet.
	Shape { kind: "sk", how: How::Lead {
		leads: &["sk-"], class: Class::Token, min: 16, max: NO_MAX } },
	Shape { kind: "aws", how: How::Lead {
		leads: &["AKIA", "ASIA", "ABIA", "ACCA", "AGPA", "AIDA", "AIPA", "ANPA", "ANVA", "AROA", "APKA"],
		class: Class::Upper, min: 12, max: NO_MAX } },
	Shape { kind: "gcp", how: How::Lead {
		leads: &["AIza"], class: Class::Token, min: 30, max: NO_MAX } },
	Shape { kind: "slack", how: How::Lead {
		leads: &["xoxa-", "xoxb-", "xoxe-", "xoxp-", "xoxr-", "xoxs-"], class: Class::Dash, min: 10, max: NO_MAX } },
	Shape { kind: "jwt", how: How::Jwt },
	// The tune relay mints one a run and a page carries it as its provider key. Exactly 32.
	Shape { kind: "tune", how: How::Lead {
		leads: &["tune-"], class: Class::Hex, min: 32, max: 32 } },
];

// Where a shape stands at `i`, as the span it replaces.
fn shape_at(shape: &Shape, t: &mut Text, i: usize) -> Option<(usize, usize)> {
	match &shape.how {
		How::Pem	=> pem(t, i),
		How::Jwt	=> jwt(t, i),
		How::Lead { leads, class, min, max } => {
			if !t.boundary(i) {
				return None;
			}
			for lead in leads.iter() {
				if t.lit(i, lead) {
					let from = i + lead.len();
					let n = t.run(from, *class);
					if n >= *min {
						return Some((i, from + n.min(*max)));
					}
				}
			}
			None
		},
	}
}

// A header's opening, a run of capitals, digits and spaces, the two words and the dashes. The
// run ends where the first dash stands, so the words are looked for just before it.
fn pem_head(t: &mut Text, i: usize, open: &str) -> Option<usize> {
	if !t.lit(i, open) {
		return None;
	}
	let from = i + open.len();
	let n = t.run(from, Class::Caps);
	let end = from + n;
	if n >= KEY.len() && t.lit(end - KEY.len(), KEY) && t.lit(end, DASHES) {
		return Some(end + DASHES.len());
	}
	None
}

// A private key, whole or clipped by elision: to its footer, or else to the end of the string.
fn pem(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	let mut q = match pem_head(t, i, BEGIN) {
		Some(q)	=> q,
		None	=> return None,
	};
	let n = t.s.len();
	while q < n {
		if t.is(q, b'-') {
			if let Some(e) = pem_head(t, q, END) {
				return Some((i, e));
			}
		}
		q += 1;
	}
	Some((i, n))
}

// Three runs of the url-safe alphabet joined by dots: the first opens `eyJ` and holds six more,
// the second six, the third four.
fn jwt(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	if !t.boundary(i) || !t.lit(i, "eyJ") {
		return None;
	}
	let mut p = i + 3;
	for (k, min) in [6usize, 6, 4].iter().enumerate() {
		let n = t.run(p, Class::Token);
		if n < *min {
			return None;
		}
		p += n;
		if k < 2 {
			if !t.is(p, b'.') {
				return None;
			}
			p += 1;
		}
	}
	Some((i, p))
}


// Daimond's `SCRUB_PAIRS`, where the name survives and only the value goes. Each answers with
// the span of the value alone.

// An `Authorization` header, however it was spelled into the text.
fn bearer(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	if !t.boundary(i) {
		return None;
	}
	for word in ["Bearer", "Basic", "Token"] {
		if t.lit(i, word) {
			let at = i + word.len();
			let sp = t.run(at, Class::Space);
			if sp == 0 {
				continue;
			}
			let v = at + sp;
			let n = t.run(v, Class::Bearer);
			if n >= 8 {
				return Some((v, v + n));
			}
		}
	}
	None
}

const URLARG: &[&str] = &[
	"access_token", "refresh_token", "id_token", "token", "api~key", "apikey", "key", "auth",
	"secret", "password", "passwd", "sig", "signature",
];

// A credential in a URL's query or fragment. A value never holds `[` or `]`, so that a marker
// already in its place is not taken for a value and marked a second time.
fn urlarg(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	if !(t.is(i, b'?') || t.is(i, b'&') || t.is(i, b'#')) {
		return None;
	}
	for name in URLARG {
		if let Some(e) = t.name(i + 1, name) {
			if t.is(e, b'=') {
				let n = t.run(e + 1, Class::Url);
				if n >= 4 {
					return Some((e + 1, e + 1 + n));
				}
			}
		}
	}
	None
}

const NAMED: &[&str] = &[
	"api~key", "apikey", "auth~token", "access~token", "token", "secret", "passphrase", "password",
	"master~key", "private~key", "mnemonic", "seed~phrase", "salt", "sealed", "wrapped",
];

// What follows a name: up to three quote marks or spaces, a colon or an equals sign, and up to
// three more. Where more than three stand, the JavaScript's greedy count takes three and meets
// another, and giving some back leaves a quote where its value should start, so it is a miss.
fn after_name(t: &mut Text, i: usize) -> Option<usize> {
	let q = t.run(i, Class::Quote);
	if q > 3 {
		return None;
	}
	let c = i + q;
	if !(t.is(c, b':') || t.is(c, b'=')) {
		return None;
	}
	let q2 = t.run(c + 1, Class::Quote);
	if q2 > 3 {
		return None;
	}
	Some(c + 1 + q2)
}

// A secret's name sitting against a value, which is what a bare `looksSecret` used to answer yes
// or no to, with the value cut out so that the message stays readable.
fn named(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	for name in NAMED {
		let e = match t.name(i, name) {
			Some(e)	=> e,
			None	=> continue,
		};
		if let Some(v) = after_name(t, e) {
			let n = t.run(v, Class::Named);
			if n >= 8 {
				return Some((v, v + n));
			}
		}
	}
	None
}

fn separator(c: u16) -> bool {
	matches!(c, 0x22 | 0x27 | 0x3a | 0x3d | 0x2c) || js_space(c)
}

// An AWS secret is forty characters of base64 wearing no prefix, so it is recognisable only beside
// the key id, or the name, it belongs to. This one is beside a key id: the id, twelve or more
// capitals and digits, any 200 units or fewer, a separator, and the forty. The JavaScript is
// `[0-9A-Z]{12,}[\s\S]{0,200}?["'\s:=,]`, and the run before the gap can give units back, but a
// unit it gives is a capital or a digit and so no separator, so the first separator at or after
// the run's end is the only one the pattern can use. Every start within one run meets that same
// separator, which is why a miss is all of them and `skip` steps past the run.
fn aws_id(t: &mut Text, i: usize, skip: &mut usize) -> Option<(usize, usize)> {
	if i < *skip || !(t.lit(i, "AKIA") || t.lit(i, "ASIA")) {
		return None;
	}
	let n = t.run(i + 4, Class::Upper);
	let end = i + 4 + n;
	*skip = end;
	if n < 12 {
		return None;
	}
	for p in end..=end + 200 {
		match t.at(p) {
			Some(c) if separator(c)	=> if t.count(p + 1, Class::Std, 41) == 40 {
				return Some((p + 1, p + 41));
			},
			Some(_)					=> (),
			None					=> break,
		}
	}
	None
}

// The same secret beside its name: `aws_secret_access_key`, with or without the first word and
// each underscore, in either case, then the usual separators and forty characters exactly.
fn aws_name(t: &mut Text, i: usize) -> Option<(usize, usize)> {
	let mut p = i;
	if t.lit_ci(p, "aws") {
		p += 3;
	}
	for word in ["secret", "access", "key"] {
		if t.is(p, b'_') {
			p += 1;
		}
		if !t.lit_ci(p, word) {
			return None;
		}
		p += word.len();
	}
	let v = match after_name(t, p) {
		Some(v)	=> v,
		None	=> return None,
	};
	if t.count(v, Class::Std, 41) == 40 {
		return Some((v, v + 40));
	}
	None
}


// One rule over the whole string, left to right, replacing each span it finds with a marker. A
// span is where the rule says its value stands, and what lies between the last replacement and
// it is kept, which is how a pair keeps its name. Answers whether it found anything.
fn pass<F>(cur: &mut Vec<u16>, kind: &str, mut rule: F) -> bool
where
	F: FnMut(&mut Text, usize) -> Option<(usize, usize)>,
{
	let n = cur.len();
	let mut text = Text::new(cur);
	let mut out: Option<Vec<u16>> = None;
	let mut done = 0;
	let mut i = 0;
	while i < n {
		match rule(&mut text, i) {
			Some((v, e)) => {
				let o = out.get_or_insert_with(|| Vec::with_capacity(n + 32));
				o.extend_from_slice(&cur[done..v]);
				o.extend(mark_units(kind, &cur[v..e]));
				done = e;
				i = e;
			},
			None => i += 1,
		}
	}
	match out {
		Some(mut o) => {
			o.extend_from_slice(&cur[done..]);
			*cur = o;
			true
		},
		None => false,
	}
}

fn units(s: &str) -> Vec<u16> {
	s.encode_utf16().collect()
}

fn mark_units(kind: &str, v: &[u16]) -> Vec<u16> {
	// djb2 over the UTF-16 units, as the JavaScript's `scrubMark` and `fingerprint` do.
	let mut h: u32 = 5381;
	for &c in v {
		h = (h << 5).wrapping_add(h).wrapping_add(c as u32);
	}
	units(&fmt!("[redacted {} #{:x}/{}]", kind, h, v.len()))
}

/// The marker for one scrubbed value: its shape, a stable hash and its length in UTF-16 units.
pub fn mark(kind: &str, v: &str) -> String {
	String::from_utf16_lossy(&mark_units(kind, &units(v)))
}

/// Shannon entropy of a string in bits a unit. Hex tops out at 4.0 by construction and
/// base64url at 6.0, which is where the threshold sits: a digest stops and a key begins.
///
/// The terms are summed in the order JavaScript's `Object.keys` gives the counts, the digits
/// ascending and then the rest as they were first met. `f64::ln` is the platform's and V8's is
/// its own, so the two can differ in the last bit or two. That matters only for a run whose
/// entropy lies within 2e-15 of the threshold, and on 2026-10-02 a differential run over 10,000
/// runs, most of them built to sit on it, found no decision that differed.
pub fn entropy(s: &str) -> f64 {
	entropy_units(&units(s))
}

fn entropy_units(s: &[u16]) -> f64 {
	let mut at: HashMap<u16, usize> = HashMap::new();
	let mut seen: Vec<(u16, usize)> = Vec::new();
	for &c in s {
		match at.get(&c) {
			Some(&k)	=> seen[k].1 += 1,
			None		=> {
				at.insert(c, seen.len());
				seen.push((c, 1));
			},
		}
	}
	let mut order: Vec<(u16, usize)> = seen.iter().filter(|(c, _)| is_digit(*c)).copied().collect();
	order.sort_by_key(|(c, _)| *c);
	order.extend(seen.iter().filter(|(c, _)| !is_digit(*c)).copied());
	let len = s.len() as f64;
	let mut h = 0.0;
	for (_, n) in order {
		let p = n as f64 / len;
		h -= p * (p.ln() / LN_2);
	}
	h
}

/// Is this unbroken run one the feed is supposed to carry? A digest, a build id and a device id
/// are hex, a correlation id is hex joined by `-` or `_`, a count is decimal, a stretch of prose
/// is letters, and an identifier a person wrote is every segment a word or a number. Named
/// rather than left to the entropy threshold, so that the feed stays legible by rule.
pub fn safe_run(s: &str) -> bool {
	safe_units(&units(s))
}

fn safe_units(s: &[u16]) -> bool {
	let sep = |c: u16| c == b'-' as u16 || c == b'_' as u16;
	let bare: Vec<u16> = s.iter().copied().filter(|c| !sep(*c)).collect();
	if !bare.is_empty() {
		if bare.iter().all(|c| is_hex(*c)) || bare.iter().all(|c| is_digit(*c))
			|| bare.iter().all(|c| is_letter(*c))
		{
			return true;
		}
	}
	let parts: Vec<&[u16]> = s.split(|c| sep(*c)).collect();
	if parts.len() > 1 {
		return parts.iter().all(|p|
			(p.len() >= 2 && p.iter().all(|c| is_letter(*c)))
			|| (!p.is_empty() && p.iter().all(|c| is_digit(*c))));
	}
	false
}

// The scrubbed units, or nothing where no rule found anything.
fn go(s: &str, is_id: bool) -> Option<Vec<u16>> {
	let mut cur = units(s);
	// Nothing in the list above is shorter than a dozen.
	if cur.len() < 8 {
		return None;
	}
	let mut hit = false;
	for shape in SHAPES {
		hit |= pass(&mut cur, shape.kind, |t, i| shape_at(shape, t, i));
	}
	hit |= pass(&mut cur, "bearer", bearer);
	hit |= pass(&mut cur, "urlarg", urlarg);
	hit |= pass(&mut cur, "named", named);
	let mut skip = 0;
	hit |= pass(&mut cur, "awssec", |t, i| aws_id(t, i, &mut skip));
	hit |= pass(&mut cur, "awssec", aws_name);
	// The catch-all runs last, so a run already turned into a marker is not weighed again. It is
	// a shape nobody has listed: a long unbroken run that is neither one of the shapes a feed
	// carries nor low in entropy.
	if !is_id {
		let mut skip = 0;
		hit |= pass(&mut cur, "hi", |t, i| {
			if i < skip {
				return None;
			}
			let n = t.run(i, Class::Token);
			if n == 0 {
				return None;
			}
			// Every start in this run sees the same run, shorter, so it is decided once.
			skip = i + n;
			if n < MIN_RUN {
				return None;
			}
			let s = t.s;
			let run = &s[i..i + n];
			if safe_units(run) || entropy_units(run) <= MIN_BITS {
				return None;
			}
			Some((i, i + n))
		});
	}
	if hit { Some(cur) } else { None }
}

/// The string with every credential in it replaced by `[redacted <shape> #<hash>/<length>]`,
/// exactly as the JavaScript scrubber writes it, and the same string where there is none.
///
/// `is_id` is for a string found in a field that names a correlation id ([`ID_KEYS`]): the
/// entropy catch is not applied to it, and every shape and pair still is.
pub fn text(s: &str, is_id: bool) -> String {
	match go(s, is_id) {
		Some(u)	=> String::from_utf16_lossy(&u),
		None	=> s.to_string(),
	}
}

/// Does `text` find anything in this string?
pub fn hit(s: &str, is_id: bool) -> bool {
	go(s, is_id).is_some()
}
