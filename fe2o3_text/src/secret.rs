//! Credential shapes in text, so that a machine can refuse one before it is written down.
//!
//! The shapes, the placeholder excuses, the skipped paths and the `allowlist secret` marker are
//! those of the global git `pre-commit` hook at `~/usr/code/bash/githooks/pre-commit`, written on
//! 2026-07-10 after a live API key was put in an example as a fallback default, pushed to a public
//! repository, and used by a stranger nine days later. Three people read that file inside those
//! nine days and each scrubbed a different copy of it. What was recorded there is that a
//! credential has to be stopped by a machine, and what this module adds is that git is not the
//! only thing a person writes history with: one marker and one set of shapes have to serve every
//! tool, or a fixture marked for one is refused by the other.
//!
//! # Bytes, not text
//!
//! A file is scanned as the bytes it holds and never decoded. A source file carrying one invalid
//! UTF-8 byte in a comment is exactly where an unnoticed key would sit, and a scanner that decoded
//! first would pass over the file entirely.
//!
//! # Key material with no text shape
//!
//! A private key written as raw DER is a credential that no run of characters describes: it has no
//! armour, no vendor prefix and no field name beside it, and it holds NULs, so the binary skip
//! below was passing over precisely the thing this module exists to stop. It is caught instead by
//! the fixed bytes the encoding itself puts at the front of one -- an algorithm's object
//! identifier, which is as literal as the PEM header above it and is a heuristic in no sense at
//! all. Written on 2026-08-23, after a live DKIM signing key spent four months at mode 644 in a
//! replicated folder and nothing here could have seen it.
//!
//! The bytes were read off keys generated for the purpose and are stated in [`DER_ALGOS`]; none of
//! them came from a file in anybody's tree, and nothing in this module was tuned against one. That
//! matters to the next reader, who will otherwise assume the opposite and be right to distrust the
//! result.
//!
//! # Why the shapes are matched by hand
//!
//! [`crate::regex`] would say these patterns in one line each, and is not used for two reasons: it
//! matches over `str` where this works over bytes, and every shape here is a literal opening
//! followed by a run of one character class, which one pass along the line decides. The
//! [`interesting`] prefilter is what makes that pass cheap -- a byte that opens no shape is
//! rejected on a handful of comparisons -- and [`leads_are_covered`] is the test that keeps the
//! prefilter honest as shapes are added.

// Fewest bytes an assigned literal must hold before it is worth suspecting, how far into a file
// the scan looks for a NUL before calling it a binary, and the marker that excuses a line, spelled
// as a caller should tell a person to spell it.
pub const MIN_LITERAL: usize = 20;
const BINARY_HEAD: usize = 8000;
pub const MARKER: &str = "allowlist secret";

// Lockfiles, which carry long hashes that read like keys.
const LOCKFILES: &[&str] = &[
	"Cargo.lock",
	"package-lock.json",
	"yarn.lock",
	"pnpm-lock.yaml",
	"go.sum",
];

// Directories holding somebody else's code, or a build's output, and the one directory name that
// says a person wrote whatever is under it. A name on the vendored list skips only while no `src`
// stands above it: every convention that put a name there -- a bundler's `dist`, a package
// manager's `node_modules`, cargo's `target` -- writes its directory beside a source tree and never
// inside one, so a `dist` below a `src` is hand written by construction. Matching the bare name at
// any depth read fourteen hand-written Rust files under one `src/dist/` as build output and
// exempted them from this guard and from the git hook, which is a hole rather than a saving. It was
// found on 2026-08-21, when the tree holding them was put under a version control system that
// cannot forget what it captures.
const VENDORED: &[&str] = &[
	"node_modules",
	"target",
	"vendor",
	".venv",
	"dist",
	"build",
];
const SOURCE: &str = "src";

// The two halves of a PEM private key header, which names its algorithm in the middle. Held apart
// so that this file does not itself carry the header a scanner looks for, its own included.
const PEM_ALGOS: &[&str] = &["", "RSA ", "EC ", "DSA ", "OPENSSH ", "PGP "];
const PEM_KEY: &str = "PRIVATE KEY";

// Widest a file can be and still be taken for a bare DER key, and narrowest. The upper bound is
// what keeps this off the compiled artefacts: a 22 MB binary is rejected on a length comparison
// before a byte of it is looked at, and no private key comes near 8000 bytes -- an RSA-8192 key in
// PKCS#8 is about 4.7 kB, and everything else on this list is under 2.4 kB. The lower bound is
// below the smallest of them, an ed25519 key at 48 bytes.
const DER_MAX: usize = 8000;
const DER_MIN: usize = 32;

/// The `AlgorithmIdentifier` that stands after the version in a PKCS#8 private key, one per
/// algorithm, each an object identifier the encoding fixes and nobody chooses.
///
/// Every sequence here was read off the front of a key generated for the purpose -- `openssl
/// genpkey -outform DER` piped through `openssl pkcs8 -topk8`, on 2026-08-23 -- and off no file in
/// any tree. There is nothing to tune and nothing that was tuned: a file opening with one of these
/// is a private key of that algorithm, and the question has no second answer.
pub const DER_ALGOS: &[&[u8]] = &[
	&[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70],							// ed25519
	&[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e],							// X25519
	&[0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01,
		0x01, 0x01, 0x05, 0x00],											// RSA
	&[0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
		0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],		// ECDSA, P-256
	&[0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
		0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22],							// ECDSA, P-384
	&[0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
		0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x23],							// ECDSA, P-521
];

// Widths of the private scalar of the curves whose keys `openssl ecparam -genkey` writes in the
// older SEC1 form, which names no algorithm and is what this machine's openssl produces by
// default. Read off one key per curve, the same day and the same way. P-256, P-384, P-521.
const DER_SCALARS: &[u8] = &[0x20, 0x30, 0x42];

// Field names that say outright what the value beside them is.
const FIELDS: &[&str] = &[
	"api_key",
	"api-key",
	"apikey",
	"secret",
	"passwd",
	"password",
	"auth_token",
	"auth-token",
	"authtoken",
	"access_token",
	"access-token",
	"accesstoken",
];

// Openings of a value nobody has filled in yet. Matched at the start of the literal, with anything
// after them, so `your-key-here` and `example_token_1` are both excused.
const PLACEHOLDERS: &[&str] = &[
	"your", "my", "the", "some", "a", "an", "test", "dummy", "fake", "example", "sample",
	"placeholder", "changeme", "redacted", "insert", "replace", "todo", "fixme", "none", "null",
	"empty", "abc", "foo", "bar", "baz", "secret", "password", "token", "key",
];


/// What was found, which is what a refusal names.
///
/// Every variant bar [`Kind::Assigned`] is a shape that is a credential and essentially nothing
/// else; `Assigned` is a named field holding a long literal, which is noisier and is why
/// placeholders are excused from it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
	Fireworks,	// fw_
	Anthropic,	// sk-ant-
	OpenAi,		// sk-proj-, sk-or-v1-
	OpenAiOld,	// sk- and a long run, the shape before the prefixes
	Aws,		// AKIA
	GitHub,		// ghp_, gho_, ghu_, ghs_, ghr_
	GitHubPat,	// github_pat_
	Slack,		// xoxb-, xoxa-, xoxp-, xoxr-, xoxs-
	Stripe,		// sk_live_, rk_live_
	Google,		// AIza
	PrivateKey,	// a PEM private key block
	DerKey,		// a private key written as DER, with nothing around it
	Assigned,	// a named secret field holding a long literal
}

impl Kind {
	/// What to call it in a message to a person.
	pub fn label(&self) -> &'static str {
		match self {
			Self::Fireworks		=> "Fireworks key",
			Self::Anthropic		=> "Anthropic key",
			Self::OpenAi		=> "OpenAI or OpenRouter key",
			Self::OpenAiOld		=> "OpenAI key, older shape",
			Self::Aws			=> "AWS access key",
			Self::GitHub		=> "GitHub token",
			Self::GitHubPat		=> "GitHub personal access token",
			Self::Slack			=> "Slack token",
			Self::Stripe		=> "Stripe live secret key",
			Self::Google		=> "Google API key",
			Self::PrivateKey	=> "private key block",
			Self::DerKey		=> "private key in DER form",
			Self::Assigned		=> "assigned secret literal",
		}
	}
}

/// One credential, at the line of the scanned bytes that holds it.
///
/// The value itself is deliberately absent: a caller reports the position and the shape, and
/// whoever reads the report opens the file. Putting the value in a message copies it into a
/// terminal's scrollback, a log and a bug report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Find {
	pub line:	usize,	// 1-based, counting line feeds
	pub kind:	Kind,
}

/// What a run of bytes after a literal opening may hold.
#[derive(Clone, Copy, Debug)]
enum Set {
	Alnum,		// [A-Za-z0-9]
	Token,		// [A-Za-z0-9_-]
	Upper,		// [0-9A-Z]
	Word,		// [A-Za-z0-9_]
	Pem,		// an algorithm name, then the words that matter
}

impl Set {
	fn admits(&self, b: u8) -> bool {
		let alnum = b.is_ascii_alphanumeric();
		match self {
			Self::Alnum		=> alnum,
			Self::Token		=> alnum || b == b'_' || b == b'-',
			Self::Upper		=> b.is_ascii_digit() || b.is_ascii_uppercase(),
			Self::Word		=> alnum || b == b'_',
			Self::Pem		=> false,
		}
	}
}

/// A literal opening and the run that must follow it.
struct Shape {
	kind:	Kind,
	lead:	&'static [u8],	// matched exactly, and case sensitively
	set:	Set,			// what the run after the opening admits
	min:	usize,			// fewest bytes that run must hold
}

impl Shape {
	/// Does the shape stand at the start of these bytes?
	fn at(&self, from: &[u8]) -> bool {
		if !from.starts_with(self.lead) {
			return false;
		}
		let tail = &from[self.lead.len()..];
		if let Set::Pem = self.set {
			return PEM_ALGOS.iter().any(|algo|
				tail.starts_with(algo.as_bytes())
					&& tail[algo.len()..].starts_with(PEM_KEY.as_bytes()));
		}
		let mut n = 0;
		while n < tail.len() && self.set.admits(tail[n]) {
			n += 1;
		}
		n >= self.min
	}
}

// Every shape a hit on which is a refusal, ordered by opening byte so that [`shapes_for`] can
// answer with one contiguous run. The ranges there are what makes the order load bearing.
const SHAPES: &[Shape] = &[
	Shape { kind: Kind::PrivateKey,	lead: b"-----BEGIN ",	set: Set::Pem,		min: 0 },
	Shape { kind: Kind::Aws,		lead: b"AKIA",			set: Set::Upper,	min: 16 },
	Shape { kind: Kind::Google,		lead: b"AIza",			set: Set::Token,	min: 35 },
	Shape { kind: Kind::Fireworks,	lead: b"fw_",			set: Set::Alnum,	min: 20 },
	Shape { kind: Kind::GitHub,		lead: b"ghp_",			set: Set::Alnum,	min: 36 },
	Shape { kind: Kind::GitHub,		lead: b"gho_",			set: Set::Alnum,	min: 36 },
	Shape { kind: Kind::GitHub,		lead: b"ghu_",			set: Set::Alnum,	min: 36 },
	Shape { kind: Kind::GitHub,		lead: b"ghs_",			set: Set::Alnum,	min: 36 },
	Shape { kind: Kind::GitHub,		lead: b"ghr_",			set: Set::Alnum,	min: 36 },
	Shape { kind: Kind::GitHubPat,	lead: b"github_pat_",	set: Set::Word,		min: 40 },
	Shape { kind: Kind::Stripe,		lead: b"rk_live_",		set: Set::Alnum,	min: 20 },
	Shape { kind: Kind::Anthropic,	lead: b"sk-ant-",		set: Set::Token,	min: 20 },
	Shape { kind: Kind::OpenAi,		lead: b"sk-proj-",		set: Set::Token,	min: 20 },
	Shape { kind: Kind::OpenAi,		lead: b"sk-or-v1-",		set: Set::Token,	min: 20 },
	Shape { kind: Kind::OpenAiOld,	lead: b"sk-",			set: Set::Alnum,	min: 32 },
	Shape { kind: Kind::Stripe,		lead: b"sk_live_",		set: Set::Alnum,	min: 20 },
	Shape { kind: Kind::Slack,		lead: b"xoxb-",			set: Set::Token,	min: 10 },
	Shape { kind: Kind::Slack,		lead: b"xoxa-",			set: Set::Token,	min: 10 },
	Shape { kind: Kind::Slack,		lead: b"xoxp-",			set: Set::Token,	min: 10 },
	Shape { kind: Kind::Slack,		lead: b"xoxr-",			set: Set::Token,	min: 10 },
	Shape { kind: Kind::Slack,		lead: b"xoxs-",			set: Set::Token,	min: 10 },
];

/// The shapes that can open with a byte.
///
/// Every position of every line asks this, and most of them are answered with nothing, which is
/// what keeps a scan to about a comparison a byte. [`leads_are_covered`] is what stops a shape
/// added to the table above from falling outside the ranges and reading as live while matching
/// nothing.
fn shapes_for(b: u8) -> &'static [Shape] {
	match b {
		b'-'	=> &SHAPES[0..1],
		b'A'	=> &SHAPES[1..3],
		b'f'	=> &SHAPES[3..4],
		b'g'	=> &SHAPES[4..10],
		b'r'	=> &SHAPES[10..11],
		b's'	=> &SHAPES[11..16],
		b'x'	=> &SHAPES[16..21],
		_		=> &[],
	}
}

/// Could a byte open a named field, in either case?
fn field_lead(b: u8) -> bool {
	matches!(b, b'a' | b'A' | b's' | b'S' | b'p' | b'P')
}


/// Every credential in these bytes, in the order the lines hold them.
///
/// Bytes holding a NUL near their start are taken for a binary and scanned no further: a
/// compiled artefact matches these shapes by chance often enough to make a scanner nobody
/// believes, and a credential compiled into a binary was in a source file first.
pub fn scan(data: &[u8]) -> Vec<Find> {
	let mut out = Vec::new();
	// Asked before the binary skip, because a key in DER form is exactly what that skip passes
	// over: NULs at the front, no text anywhere, and nothing the line walk below can see. It is a
	// property of the whole input rather than of a line, so it answers on its own and stops here.
	if der_key(data) {
		out.push(Find { line: 1, kind: Kind::DerKey });
		return out;
	}
	let head = data.len().min(BINARY_HEAD);
	if data[..head].contains(&0) {
		return out;
	}
	let mut kinds = Vec::new();
	let mut prev: &[u8] = b"";
	for (i, line) in data.split(|b| *b == b'\n').enumerate() {
		// The line above excuses this one, so that a marker can sit in a comment over the line it
		// speaks for rather than trailing off the end of it.
		if !excused(line) && !excused(prev) {
			kinds.clear();
			kinds_at(line, &mut kinds);
			for kind in &kinds {
				out.push(Find { line: i + 1, kind: *kind });
			}
		}
		prev = line;
	}
	out
}

/// Is the path one whose long hashes read like keys, and which is therefore not scanned?
///
/// A lockfile by name, or anything under a vendored or built directory that no `src` stands above.
/// The path is relative to the root of whatever is being scanned, with `/` between its components.
pub fn skip_path(path: &[u8]) -> bool {
	let mut last: &[u8] = b"";
	let mut dirs = 0;
	let mut sourced = false;
	for comp in path.split(|b| *b == b'/') {
		// Something follows `last`, so `last` is a directory rather than the file at the end.
		if dirs > 0 {
			if last == SOURCE.as_bytes() {
				sourced = true;
			}
			// A source tree inside a vendored one is still somebody else's, so the first of the two
			// names to appear is the one that decides.
			if !sourced && VENDORED.iter().any(|v| v.as_bytes() == last) {
				return true;
			}
		}
		last = comp;
		dirs += 1;
	}
	LOCKFILES.iter().any(|f| f.as_bytes() == last)
}

/// Is the whole of these bytes one private key, written as DER and left unarmoured?
///
/// The three questions are the encoding's own, and each of them has one answer. The outer
/// `SEQUENCE` has to account for the input exactly, so what is refused is a file that is a key and
/// nothing else -- a byte of anything else in it and this is not what it claims to be. The version
/// `INTEGER` is 0 for PKCS#8 and PKCS#1 and 1 for the `OneAsymmetricKey` form that carries the
/// public key too, which is the 83-byte shape `ring` writes and the shape the DKIM key was in.
/// What follows the version is then an algorithm's object identifier from [`DER_ALGOS`], or the
/// modulus of a PKCS#1 RSA key, or the private scalar of a SEC1 elliptic curve key.
///
/// There is no marker that excuses a finding here, and there cannot be: a file which is a key and
/// nothing else has nowhere to put one, and anything appended to make room stops the length
/// accounting above from agreeing, at which point there is no finding to excuse. A test that needs
/// a key should generate one, which is what this crate's own suite does.
fn der_key(data: &[u8]) -> bool {
	if data.len() < DER_MIN || data.len() > DER_MAX || data.first() != Some(&0x30) {
		return false;
	}
	let (len, hdr) = match der_len(&data[1..]) {
		Some(v)	=> v,
		None	=> return false,
	};
	let whole = 1 + hdr + len;
	// An editor or a shell redirect will put a newline after the last byte, and that is the only
	// thing allowed to stand outside the SEQUENCE.
	if whole > data.len() || !data[whole..].iter().all(|b| *b == b'\n' || *b == b'\r') {
		return false;
	}
	let body = &data[1 + hdr..];
	let after = if body.starts_with(&[0x02, 0x01, 0x00]) {
		&body[3..]
	} else if body.starts_with(&[0x02, 0x01, 0x01]) {
		// The version says the public key follows the private one, so a SEC1 scalar can stand here
		// as well as a PKCS#8 algorithm.
		let after = &body[3..];
		if DER_SCALARS.iter().any(|w| after.starts_with(&[0x04, *w])) {
			return true;
		}
		after
	} else {
		return false;
	};
	if DER_ALGOS.iter().any(|a| after.starts_with(a)) {
		return true;
	}
	// PKCS#1, which names no algorithm: what follows the version is the modulus, an INTEGER whose
	// length is written long form because no key worth having has one under 128 bytes.
	after.starts_with(&[0x02, 0x81]) || after.starts_with(&[0x02, 0x82])
}

/// The length a DER header declares, and the bytes that header took, or nothing where the form is
/// one no private key is written in.
fn der_len(from: &[u8]) -> Option<(usize, usize)> {
	match from.first() {
		Some(n) if *n < 0x80	=> Some((*n as usize, 1)),
		Some(&0x81)				=> from.get(1).map(|n| (*n as usize, 2)),
		Some(&0x82)				=> match (from.get(1), from.get(2)) {
			(Some(hi), Some(lo))	=> Some((((*hi as usize) << 8) | *lo as usize, 3)),
			_						=> None,
		},
		_						=> None,
	}
}

/// Does the line carry the marker that excuses it?
///
/// Two spellings are taken, `allowlist secret` in any case and with a space, an underscore or a
/// hyphen between the words, and `pragma: allowlist` as the detect-secrets convention spells it.
pub fn excused(line: &[u8]) -> bool {
	for at in 0..line.len() {
		let from = &line[at..];
		if starts_ci(from, b"allowlist") {
			let rest = &from["allowlist".len()..];
			match rest.first() {
				Some(b' ') | Some(b'_') | Some(b'-')
					if starts_ci(&rest[1..], b"secret") => return true,
				_ => (),
			}
		}
		if starts_ci(from, b"pragma:") && starts_ci(blank(&from["pragma:".len()..]), b"allowlist") {
			return true;
		}
	}
	false
}

/// Could a byte open any shape, or any named field?
///
/// The prefilter the line walk turns on, derived from the table rather than written out beside
/// it, so that the two cannot drift apart.
pub fn interesting(b: u8) -> bool {
	!shapes_for(b).is_empty() || field_lead(b)
}

/// Does the prefilter admit the opening byte of every shape and every field name?
///
/// A shape added with an opening the prefilter rejects would be dead code that reads as live, and
/// nothing else in the module would notice. Exposed so that a caller's test suite can hold the
/// same line as this crate's.
pub fn leads_are_covered() -> bool {
	for shape in SHAPES {
		let lead = match shape.lead.first() {
			Some(b) => *b,
			None => return false,
		};
		if !shapes_for(lead).iter().any(|s| s.lead == shape.lead && s.kind == shape.kind) {
			return false;
		}
	}
	for name in FIELDS {
		let lead = match name.as_bytes().first() {
			Some(b) => *b,
			None => return false,
		};
		if !field_lead(lead) || !field_lead(lead.to_ascii_uppercase()) {
			return false;
		}
	}
	true
}

/// Puts every kind standing anywhere in the line into `out`, once each.
fn kinds_at(line: &[u8], out: &mut Vec<Kind>) {
	for at in 0..line.len() {
		let from = &line[at..];
		for shape in shapes_for(line[at]) {
			if shape.at(from) && !out.contains(&shape.kind) {
				out.push(shape.kind);
			}
		}
		// Only a field name's own opening is worth the walk along the twelve of them.
		if field_lead(line[at]) && !out.contains(&Kind::Assigned) && assigned(from) {
			out.push(Kind::Assigned);
		}
	}
}

/// Does a named secret field stand here, holding a quoted literal that is long and is not a
/// placeholder?
fn assigned(from: &[u8]) -> bool {
	for name in FIELDS {
		if starts_ci(from, name.as_bytes()) && literal(&from[name.len()..]) {
			return true;
		}
	}
	false
}

/// Does what follows a field name amount to it being given a long literal?
fn literal(after: &[u8]) -> bool {
	let rest = blank(after);
	match rest.first() {
		Some(b':') | Some(b'=')	=> (),
		_						=> return false,
	}
	let rest = blank(&rest[1..]);
	match rest.first() {
		Some(b'"') | Some(b'\'')	=> (),
		_							=> return false,
	}
	let rest = &rest[1..];
	let mut n = 0;
	while n < rest.len() && Set::Token.admits(rest[n]) {
		n += 1;
	}
	if n < MIN_LITERAL {
		return false;
	}
	// The run has to end where the quote does, or what stands there is not one literal.
	match rest.get(n) {
		Some(b'"') | Some(b'\'')	=> (),
		_							=> return false,
	}
	!placeholder(&rest[..n])
}

/// Is the literal one nobody has filled in?
fn placeholder(value: &[u8]) -> bool {
	if value.len() >= 4 && value[..4].iter().all(|b| b.eq_ignore_ascii_case(&b'x')) {
		return true;
	}
	// `a` is one of the words, so any literal opening with an `a` is excused. That is the git
	// hook's behaviour and is kept deliberately: one convention across the two tools is worth
	// more than a marginally tighter rule on the noisier of the two classes, and a real key of
	// any issued shape is caught above regardless of what it opens with.
	PLACEHOLDERS.iter().any(|w| starts_ci(value, w.as_bytes()))
}

/// Does the haystack open with the needle, ignoring ASCII case?
fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
	hay.len() >= needle.len()
		&& hay[..needle.len()].eq_ignore_ascii_case(needle)
}

/// Drops leading spaces and tabs.
fn blank(from: &[u8]) -> &[u8] {
	let mut n = 0;
	while n < from.len() && (from[n] == b' ' || from[n] == b'\t') {
		n += 1;
	}
	&from[n..]
}
