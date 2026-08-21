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

// Directories holding somebody else's code, or a build's output.
const VENDORED: &[&str] = &[
	"node_modules",
	"target",
	"vendor",
	".venv",
	"dist",
	"build",
];

// The two halves of a PEM private key header, which names its algorithm in the middle. Held apart
// so that this file does not itself carry the header a scanner looks for, its own included.
const PEM_ALGOS: &[&str] = &["", "RSA ", "EC ", "DSA ", "OPENSSH ", "PGP "];
const PEM_KEY: &str = "PRIVATE KEY";

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
/// A lockfile by name, or anything under a vendored or built directory. The path is relative to
/// the root of whatever is being scanned, with `/` between its components.
pub fn skip_path(path: &[u8]) -> bool {
	let mut last: &[u8] = b"";
	let mut dirs = 0;
	for comp in path.split(|b| *b == b'/') {
		if dirs > 0 && VENDORED.iter().any(|v| v.as_bytes() == last) {
			return true;
		}
		last = comp;
		dirs += 1;
	}
	LOCKFILES.iter().any(|f| f.as_bytes() == last)
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
