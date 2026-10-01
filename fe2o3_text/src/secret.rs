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
//! the fixed bytes the encoding itself puts in front of one -- an algorithm's object identifier,
//! which is as literal as the PEM header above it and is a heuristic in no sense at all. Written
//! on 2026-08-23, after a live DKIM signing key spent four months at mode 644 in a replicated
//! folder and nothing here could have seen it.
//!
//! The bytes were read off keys generated for the purpose and are stated in [`DER_ALGOS`]; none of
//! them came from a file in anybody's tree, and nothing in this module was tuned against one. That
//! matters to the next reader, who will otherwise assume the opposite and be right to distrust the
//! result.
//!
//! # Why a key is looked for at every offset, and only in a small file
//!
//! Ring 0.17.8 holds the head of a PKCS#8 key as a `const` template -- the outer `SEQUENCE`, the
//! version, the algorithm's object identifier and the tag that opens the private bytes -- and a
//! compiler puts that template in the read-only data of whatever links it. Those bytes are the
//! structure of a private key because that is what a template of one is, so [`der_key`] says so,
//! and it is right: there is no test that separates a template from a key which is not a guess
//! about the bytes standing after it. A sweep of every file under this tree on 2026-08-23 --
//! 623,722 files, 202 GB, nothing skipped -- found the structure at 1,729 offsets in 303 files,
//! and every one of the 303 was a compiled artefact carrying that template: executables, `.rlib`,
//! `.rmeta`, `.o`, WebAssembly modules, and one capture of a tree holding an executable. Not one
//! was a key.
//!
//! So a scan of every offset in a compiled artefact refuses it, and a guard that refuses an
//! ordinary build output is a guard somebody turns off. [`DER_SPAN`] is what stops that, and it is
//! a size and nothing cleverer: the whole of a file is read at every offset while the file is
//! small enough to be a key and the things a key is bundled with, and above that only its front is
//! read, which is where the rule stood until this was written. The smallest artefact in that sweep
//! was 101,960 bytes, three times the span.
//!
//! # Key material without its wrapper
//!
//! A credential is recognised by the part of it that makes it one, and some credentials put that
//! part last. The body of a PEM key written into a file before its armour is, a password typed up to
//! but not including its closing quote: a tool that records a file as it is saved will record such a
//! file between the two saves, and the save that completes it is refused too late. Written on
//! 2026-10-02 after qa1 found exactly that (D3) with `ore edit`, which saves without being asked.
//!
//! So the body is recognised without its armour. Every run of lines made of base64 and nothing else,
//! and every long base64 token in a line that is not, is decoded with [`crate::base64`] and put to
//! [`der_key`], the rule that already refuses a key written as raw DER, so that a certificate body
//! is not refused and a key body is, armoured or not. An OpenSSH key is not DER; it opens with a
//! fixed string, which is as exact a mark as an object identifier, and is found by that. Armour is
//! still what is reported when it is there, and a marker is still what excuses a key written as
//! text, on the line above the body as on the line above the armour. The finding names the line the
//! key's last byte is on, because a caller that passes over a line already recorded must be asked
//! about the line that completed the key.
//!
//! What stays a fragment is stated rather than found later. A body saved short of its last byte is
//! not a key, as the first characters of a token are not a token. Base64 that is wrapped in
//! anything, such as quotes at the start of a line, a comment marker on every line, or `\n` escapes
//! in a JSON string, is read only where a long enough token stands whole in one line.
//!
//! # Why the shapes are matched by hand
//!
//! [`crate::regex`] would say these patterns in one line each, and is not used for two reasons: it
//! matches over `str` where this works over bytes, and every shape here is a literal opening
//! followed by a run of one character class, which one pass along the line decides. The
//! [`interesting`] prefilter is what makes that pass cheap -- a byte that opens no shape is
//! rejected on a handful of comparisons -- and [`leads_are_covered`] is the test that keeps the
//! prefilter honest as shapes are added.

use crate::base64;


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

// Widest a DER key may declare itself and still be looked at, and narrowest a file can be to hold
// one at all. The ceiling is on the length the SEQUENCE declares rather than on the file, because
// what stands after a key is not the key, and gating on the file was what let a key with one byte
// appended through. No private key comes near 8000 bytes: an RSA-8192 key in PKCS#8 is about
// 4.7 kB and everything else on this list is under 2.4 kB. The floor is below the smallest key, an
// ed25519 at 48 bytes.
const DER_MAX: usize = 8000;
const DER_MIN: usize = 32;

/// Widest a file can be and still be read at every offset rather than only at its front.
///
/// Four times the widest a key may declare itself, so that a key and the certificate chain it is
/// bundled with are inside it several times over, and far below any compiled artefact: see this
/// module's header for the sweep that says so and for why a size is what stands here.
pub const DER_SPAN: usize = 4 * DER_MAX;

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

// Fewest base64 characters that can hold a private key, which is DER_MIN bytes at six bits a
// character, and most that are kept of a run to be decoded: DER_SPAN bytes' worth and a little over,
// so that a run just past the span is told from one at it.
const B64_MIN: usize = (DER_MIN * 4 + 2) / 3;
const B64_KEEP: usize = (DER_SPAN * 4 + 2) / 3 + 8;

// The front of an OpenSSH private key, which `ssh-keygen` writes by default and which is no DER: it
// opens with this string and a NUL, and holds a private key whether or not a passphrase wraps it.
const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";

// The same fifteen bytes as base64 at the start of a line, which is where they stand in a body.
const OPENSSH_B64: &[u8] = b"b3BlbnNzaC1rZXktdjEA";

// Which bytes are in the base64 alphabet.
const B64: [bool; 256] = b64_table();

const fn b64_table() -> [bool; 256] {
	let mut table = [false; 256];
	let mut i = 0;
	while i < 256 {
		let b = i as u8;
		table[i] = b.is_ascii_alphanumeric() || b == b'+' || b == b'/';
		i += 1;
	}
	table
}

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
	PrivateKey,	// a PEM private key block, or the body of one without its armour
	DerKey,		// a private key written as DER, at any offset in a small file
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
	// property of the bytes rather than of a line, so it answers on its own and stops here,
	// whatever else stands around the key.
	if let Some((at, _)) = der_key(data) {
		out.push(Find { line: line_at(data, at), kind: Kind::DerKey });
		return out;
	}
	let head = data.len().min(BINARY_HEAD);
	if data[..head].contains(&0) {
		return out;
	}
	let mut kinds = Vec::new();
	let mut run = Run::new();
	let mut prev_ex = false;		// the line above carries the marker
	let mut prev_armour = false;	// the line above opens a private key block
	let mut behind = false;			// a run put a finding on a line above the ones since
	for (i, line) in data.split(|b| *b == b'\n').enumerate() {
		// The line above excuses this one, so that a marker can sit in a comment over the line it
		// speaks for rather than trailing off the end of it.
		let marked = excused(line);
		let ex = marked || prev_ex;
		kinds.clear();
		kinds_at(line, &mut kinds);
		let armour = kinds.contains(&Kind::PrivateKey);
		if !ex {
			for kind in &kinds {
				out.push(Find { line: i + 1, kind: *kind });
			}
		}
		match b64_line(line) {
			Some((chars, padded)) => {
				if !run.on {
					// A body under its armour is spoken for by the armour, found or excused.
					run.start(ex || prev_armour);
				}
				run.push(i + 1, chars);
				if padded {
					behind |= run.end(&mut out);
				}
			},
			None => {
				behind |= run.end(&mut out);
				if !ex && !armour {
					tokens_at(line, i + 1, &mut out);
				}
			},
		}
		prev_ex = marked;
		prev_armour = armour;
	}
	behind |= run.end(&mut out);
	if behind {
		out.sort_by_key(|f| f.line);
	}
	out
}

/// Paths that are a secret by name, as ignore rules in git's glob syntax, one per line.
///
/// The other half of this module. [`scan`] reads bytes and refuses a credential it can recognise;
/// this names the files a credential conventionally lives in, whatever their bytes say, so that a
/// tool writing a history can keep them out before it has read a byte of them. A `.env` holding
/// `DB_PASSWORD=hunter2` has no shape [`scan`] answers to, and a PEM certificate is not a secret at
/// all but stands beside the key that is, so a rule by name is what stops both.
///
/// The syntax is a `.gitignore`'s, so that a tool which already compiles one can compile this by
/// prepending it: a repository's own rules then come last and win, and a `!` line in them
/// re-includes anything here by name. The one re-inclusion this list makes itself is the example
/// file every `.env` convention ships beside the real one, which holds placeholders by definition
/// and is the file a reader needs most.
///
/// This is not compiled here, because the glob machinery lives downstream of this crate; the test
/// that every line is a rule the matcher accepts, and that it decides what this comment says it
/// does, is beside that machinery.
pub const SECRET_PATHS: &[&str] = &[
	// Environment files, which hold credentials by convention and nothing by shape.
	".env",
	".env.*",
	"!.env.example",
	"!.env.sample",
	"!.env.template",
	// Key material by extension, and the certificate that conventionally stands beside it.
	"*.pem",
	"*.key",
	"*.p12",
	"*.pfx",
	"*.jks",
	"*.keystore",
	// The names every SSH client writes a private key under.
	"id_rsa",
	"id_dsa",
	"id_ecdsa",
	"id_ed25519",
	// Machine credentials for other services, kept in the home directory by convention and
	// copied into a project by mistake.
	".netrc",
	".pgpass",
	".htpasswd",
	// Directories whose name says what they hold.
	"keys/",
	"tls/",
];

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

/// Where a private key, written as DER and left unarmoured, stands in these bytes, and how many
/// bytes it takes, if one does.
///
/// The front of the input is asked whatever its size, and every offset in it as well while it is
/// no wider than [`DER_SPAN`]. Until 2026-08-23 only the front was asked, and `cat cert.der
/// key.der` -- a bundle nobody has to tamper with to produce, and one `openssl pkey -inform DER`
/// reads the private key straight out of and signs with -- went free because the certificate stood
/// first.
///
/// There is no marker that excuses a finding here, and there cannot be: this reads the key's own
/// structure and nothing around it, so there is nowhere to write one that it would look at. A test
/// that needs a key should generate one, which is what this crate's own suite does.
fn der_key(data: &[u8]) -> Option<(usize, usize)> {
	if let Some(whole) = der_key_at(data, 0) {
		return Some((0, whole));
	}
	if data.len() > DER_SPAN {
		return None;
	}
	// The version INTEGER is the one thing every form below has in common, and it stands at a fixed
	// distance into the SEQUENCE, whose header is one, two or three bytes wide. So a candidate
	// opening is at one of three known distances back from a version, and a walk looking for the
	// version rather than for the SEQUENCE tag asks the full test 210 times less often: three bytes
	// of a compiled artefact answer where one does not. What the two find is the same set.
	let last = data.len().saturating_sub(2);
	for v in 0..last {
		if data[v] != 0x02 || data[v + 1] != 0x01 || (data[v + 2] != 0x00 && data[v + 2] != 0x01) {
			continue;
		}
		for hdr in 1..=3 {
			if v >= 1 + hdr {
				if let Some(whole) = der_key_at(data, v - 1 - hdr) {
					return Some((v - 1 - hdr, whole));
				}
			}
		}
	}
	None
}

/// How many bytes the key takes, if one stands at this offset.
///
/// The three questions are the encoding's own, and each of them has one answer. The outer
/// `SEQUENCE` declares its own length and that is where the key ends: everything asked below is
/// asked of those bytes, and whatever stands after them is no part of the key and is not looked
/// at. The version `INTEGER` is 0 for PKCS#8 and PKCS#1 and 1 for the `OneAsymmetricKey` form that
/// carries the public key too, which is the 83-byte shape `ring` writes and the shape the DKIM key
/// was in. What follows the version is then an algorithm's object identifier from [`DER_ALGOS`],
/// or the modulus of a PKCS#1 RSA key, or the private scalar of a SEC1 elliptic curve key.
fn der_key_at(data: &[u8], at: usize) -> Option<usize> {
	if data.len() - at < DER_MIN || data[at] != 0x30 {
		return None;
	}
	let (len, hdr) = match der_len(&data[at + 1..]) {
		Some(v)	=> v,
		None	=> return None,
	};
	let whole = 1 + hdr + len;
	// The SEQUENCE has to be all there, and the ceiling is asked of the length it declares rather
	// than of what is left of the file.
	if whole > data.len() - at || whole > DER_MAX {
		return None;
	}
	// Bounded by the declared length, so that a SEQUENCE too short to hold one of the shapes below
	// cannot borrow the bytes standing after it to finish the match.
	let body = &data[at + 1 + hdr..at + whole];
	let after = if body.starts_with(&[0x02, 0x01, 0x00]) {
		&body[3..]
	} else if body.starts_with(&[0x02, 0x01, 0x01]) {
		// The version says the public key follows the private one, so a SEC1 scalar can stand here
		// as well as a PKCS#8 algorithm.
		let after = &body[3..];
		if DER_SCALARS.iter().any(|w| after.starts_with(&[0x04, *w])) {
			return Some(whole);
		}
		after
	} else {
		return None;
	};
	if DER_ALGOS.iter().any(|a| after.starts_with(a)) {
		return Some(whole);
	}
	// PKCS#1, which names no algorithm: what follows the version is the modulus, an INTEGER whose
	// length is written long form because no key worth having has one under 128 bytes.
	if after.starts_with(&[0x02, 0x81]) || after.starts_with(&[0x02, 0x82]) {
		Some(whole)
	} else {
		None
	}
}

/// The line an offset falls on, counting line feeds, so that a key written into a text file is
/// reported where a person will find it. A key at the front of a file is line 1, which is where
/// every one of them was reported before offsets were looked at.
fn line_at(data: &[u8], at: usize) -> usize {
	1 + data[..at].iter().filter(|b| **b == b'\n').count()
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

/// A run of lines made of base64 and nothing else, gathered to be decoded as one body.
///
/// Held once and reused, because most files are full of one-word lines that are base64 by the
/// alphabet and nothing more, and each of them would otherwise cost an allocation.
struct Run {
	on:		bool,
	skip:	bool,					// excused, or under armour that speaks for it
	chars:	Vec<u8>,				// the first B64_KEEP characters
	total:	usize,					// every character, kept or not
	ends:	Vec<(usize, usize)>,	// characters held, and the line, at the end of each kept line
	last:	usize,					// the line the run has reached
}

impl Run {
	fn new() -> Self {
		Self { on: false, skip: false, chars: Vec::new(), total: 0, ends: Vec::new(), last: 0 }
	}

	fn start(&mut self, skip: bool) {
		self.on = true;
		self.skip = skip;
		self.chars.clear();
		self.ends.clear();
		self.total = 0;
	}

	fn push(&mut self, line: usize, chars: &[u8]) {
		self.last = line;
		if self.skip {
			return;
		}
		self.total += chars.len();
		let room = B64_KEEP.saturating_sub(self.chars.len());
		if room > 0 {
			self.chars.extend_from_slice(&chars[..chars.len().min(room)]);
			self.ends.push((self.chars.len(), line));
		}
	}

	/// Closes the run, putting a finding into `out` if it was the body of a key. True if it did.
	fn end(&mut self, out: &mut Vec<Find>) -> bool {
		if !self.on {
			return false;
		}
		self.on = false;
		if self.skip {
			return false;
		}
		let found = match body_key(&self.chars, self.total) {
			Some(at)	=> Some(at),
			None		=> self.later(),
		};
		match found {
			Some(at) => {
				out.push(Find { line: self.line_of(at), kind: Kind::PrivateKey });
				true
			},
			None => false,
		}
	}

	/// Tries each line after the first as the start of a body of its own.
	///
	/// A word or a label on a line of its own in front of a body puts every character after it
	/// out of step with the run's start, and the run is then decoded as noise. A key's first
	/// byte is a `SEQUENCE` tag, so its first character is fixed, and a line is only decoded
	/// where it could open one.
	fn later(&self) -> Option<usize> {
		for j in 1..self.ends.len() {
			let from = self.ends[j - 1].0;
			if let Some(at) = front_key(&self.chars[from..]) {
				return Some(if at == usize::MAX { at } else { from + at });
			}
		}
		None
	}

	/// The line a character was on. Past the characters kept, which is where an OpenSSH key is
	/// said to end, it is the last line the run reached.
	fn line_of(&self, at: usize) -> usize {
		for (held, line) in &self.ends {
			if at < *held {
				return *line;
			}
		}
		self.last
	}
}

/// The base64 a line is made of when it is made of nothing else, and whether padding ends it.
///
/// Blanks at either end are not part of it, so a body indented in a YAML block or ended with a
/// carriage return is the same body.
fn b64_line(line: &[u8]) -> Option<(&[u8], bool)> {
	let mut end = line.len();
	while end > 0 && matches!(line[end - 1], b' ' | b'\t' | b'\r') {
		end -= 1;
	}
	let mut pad = 0;
	while pad < 2 && end > 0 && line[end - 1] == b'=' {
		end -= 1;
		pad += 1;
	}
	let body = blank(&line[..end]);
	if body.is_empty() || !body.iter().all(|b| B64[*b as usize]) {
		return None;
	}
	Some((body, pad > 0))
}

/// Puts a finding at this line into `out` if a long base64 token in it decodes to a private key.
///
/// For a line that is not made of base64 alone, so a key on one line with other text beside it:
/// `KEY=`, a quote, a tag. Where the text is glued to the token with no character between that
/// base64 does not admit, the token starts out of step with the base64 it carries and is not read.
fn tokens_at(line: &[u8], no: usize, out: &mut Vec<Find>) {
	let mut at = 0;
	while at < line.len() {
		if !B64[line[at] as usize] {
			at += 1;
			continue;
		}
		let from = at;
		while at < line.len() && B64[line[at] as usize] {
			at += 1;
		}
		let token = &line[from..at];
		if token.len() >= B64_MIN
			&& body_key(&token[..token.len().min(B64_KEEP)], token.len()).is_some()
		{
			out.push(Find { line: no, kind: Kind::PrivateKey });
			return;
		}
	}
}

/// Where a private key's body ends among these base64 characters, if the characters are one.
///
/// The characters decoded are put to the rule a raw DER key is, on the same terms: every offset
/// while the bytes are no wider than [`DER_SPAN`], and only the front above that. `total` is the
/// run's whole length where `chars` holds only the front of it. The answer is the index of the
/// last character the key takes, or the largest `usize` for an OpenSSH key, which runs to the end
/// of its body.
fn body_key(chars: &[u8], total: usize) -> Option<usize> {
	if total < B64_MIN {
		return None;
	}
	let bytes = unbase64(chars);
	if total * 3 / 4 > DER_SPAN {
		return match der_key_at(&bytes, 0) {
			Some(whole)	=> Some(last_char(whole)),
			None		=> if bytes.starts_with(OPENSSH_MAGIC) { Some(usize::MAX) } else { None },
		};
	}
	if let Some((at, whole)) = der_key(&bytes) {
		return Some(last_char(at + whole));
	}
	if bytes.windows(OPENSSH_MAGIC.len()).any(|w| w == OPENSSH_MAGIC) {
		return Some(usize::MAX);
	}
	None
}

/// Where a private key's body ends if one begins at the first of these base64 characters.
///
/// Asked of the front only, and cheaply, since it is asked of every line a run holds: a `SEQUENCE`
/// opens with `M` and a second character from `A` to `I` whatever its length, and only then is
/// the length read and that much decoded.
fn front_key(chars: &[u8]) -> Option<usize> {
	if chars.starts_with(OPENSSH_B64) {
		return Some(usize::MAX);
	}
	if chars.len() < B64_MIN || chars[0] != b'M' || !(b'A'..=b'I').contains(&chars[1]) {
		return None;
	}
	let head = unbase64(&chars[..8]);
	let need = match head.get(1..).and_then(der_len) {
		Some((len, hdr))	=> 1 + hdr + len,
		None				=> return None,
	};
	if need > DER_MAX {
		return None;
	}
	let upto = ((need * 4 + 2) / 3 + 4).min(chars.len());
	der_key_at(&unbase64(&chars[..upto]), 0).map(last_char)
}

/// What base64 characters decode to, taking the last quantum as it stands.
///
/// Padded out with zero sextets to whole quanta and cut back to the bytes the characters carry, so
/// that a body whose padding was left off, or whose last character a careless encoder left bits
/// in, reads as the key it is rather than as nothing. The characters are all in the alphabet, which
/// is what the strict decoder asks, so it has nothing to refuse.
fn unbase64(chars: &[u8]) -> Vec<u8> {
	let mut text = Vec::with_capacity(chars.len() + 3);
	text.extend_from_slice(chars);
	while text.len() % 4 != 0 {
		text.push(b'A');
	}
	let text = match String::from_utf8(text) {
		Ok(t)	=> t,
		Err(_)	=> return Vec::new(),
	};
	match base64::decode(&text) {
		Ok(mut bytes)	=> {
			bytes.truncate(chars.len() * 3 / 4);
			bytes
		},
		Err(_)	=> Vec::new(),
	}
}

/// The index of the last base64 character that carries a bit of the byte before `end`.
fn last_char(end: usize) -> usize {
	let b = end - 1;
	(b / 3) * 4 + b % 3 + 1
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
	// The run has to end where the quote does, or at the end of the line. A quote not yet typed is
	// the state a password is saved in between the keystrokes, and a scanner that waits for the
	// quote records it whole. Qa1's D3, 2026-09-23. What follows the run on the same line, when it
	// is neither, says it is not one literal.
	match rest.get(n) {
		Some(b'"') | Some(b'\'')	=> (),
		_ if rest[n..].iter().all(|b| matches!(b, b' ' | b'\t' | b'\r'))	=> (),
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
