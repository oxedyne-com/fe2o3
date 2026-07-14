//! Conformance tests for `fe2o3_text::unicode`, run against the Unicode Consortium's own test
//! files.
//!
//! The files live in `tests/unicode_data/`, and come from the same UCD release as the tables, so
//! the data can never drift from what it is testing.
//!
//! Four of the six are kept in the repository. The two bidi suites are 15 MB between them, which is
//! not worth carrying in a repository's history forever, so they are fetched on demand at the
//! pinned UCD version the first time a test needs them. A test that has no data fetches it; a test
//! that cannot fetch it fails. Nothing is ever skipped.

use oxedyne_fe2o3_text::unicode::{
	bidi::{
		self,
		BidiInfo,
		Direction,
	},
	linebreak,
	lookup::Partitioned,
	norm::{
		self,
		Form,
	},
	prop::BidiClass as B,
	segment,
	UCD_VERSION,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

/// How many failing cases a report lists before it stops.
const SHOWN: usize = 8;

/// The tally of a conformance run.
struct Tally {
	/// The name of the test file.
	name:	String,
	/// The number of cases run.
	total:	usize,
	/// The cases that failed, as a line number and a description.
	failed:	Vec<(usize, String)>,
}

impl Tally {

	/// Begins a tally.
	fn new(name: &str) -> Self {
		Self {
			name:	name.to_string(),
			total:	0,
			failed:	Vec::new(),
		}
	}

	/// Records a case.
	fn case(&mut self, line: usize, ok: bool, why: String) {
		self.total += 1;
		if !ok {
			self.failed.push((line, why));
		}
	}

	/// Reports the pass rate, and fails the test if any case failed.
	fn finish(self) -> Outcome<()> {
		let passed = self.total - self.failed.len();
		let rate = if self.total == 0 {
			0.0
		} else {
			100.0 * (passed as f64) / (self.total as f64)
		};
		msg!("{}: {}/{} passed ({:.4}%), UCD {}.", self.name, passed, self.total, rate,
			UCD_VERSION);
		if self.failed.is_empty() {
			return Ok(());
		}
		for (line, why) in self.failed.iter().take(SHOWN) {
			msg!("  line {}: {}", line, why);
		}
		if self.failed.len() > SHOWN {
			msg!("  ... and {} more.", self.failed.len() - SHOWN);
		}
		Err(err!(
			"{}: {} of {} conformance cases failed.", self.name, self.failed.len(), self.total;
			Test, Mismatch))
	}
}

/// Where the Unicode Consortium publishes its data.
const UCD_BASE: &str = "https://www.unicode.org/Public";

/// Each conformance file, and its path below the UCD version directory.
///
/// The four small suites are kept in the repository. The two bidi suites are 15 MB between them,
/// which is not worth carrying in a repository's history forever, so they are absent and fetched on
/// demand by [`fetch`].
const TEST_FILES: [(&str, &str); 6] = [
	("NormalizationTest.txt",	"ucd/NormalizationTest.txt"),
	("BidiTest.txt",		"ucd/BidiTest.txt"),
	("BidiCharacterTest.txt",	"ucd/BidiCharacterTest.txt"),
	("GraphemeBreakTest.txt",	"ucd/auxiliary/GraphemeBreakTest.txt"),
	("WordBreakTest.txt",		"ucd/auxiliary/WordBreakTest.txt"),
	("LineBreakTest.txt",		"ucd/auxiliary/LineBreakTest.txt"),
];

/// Reads a conformance file, fetching it first if the repository does not carry it.
///
/// A test is never skipped for want of its data: either the file is found, or it is fetched, or the
/// test fails saying why.
fn data(name: &str) -> Outcome<String> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("unicode_data")
		.join(name);
	if !path.exists() {
		res!(fetch(name, &path));
	}
	match fs::read_to_string(&path) {
		Ok(text) => Ok(text),
		Err(e) => Err(err!(e,
			"The conformance file {:?} could not be read. Run \
			 `cargo run -p oxedyne_fe2o3_text --bin gen_unicode` to rebuild the tables and \
			 refetch it.", path;
			IO, File, Missing)),
	}
}

/// Fetches a conformance file the repository does not carry, at the pinned UCD version.
///
/// The version fetched is [`UCD_VERSION`], the one the committed tables were generated from, so the
/// data can never drift from the tables it is testing. A newer Unicode release does not silently
/// become the thing under test.
fn fetch(name: &str, dest: &Path) -> Outcome<()> {
	let rel = match TEST_FILES.iter().find(|(n, _)| *n == name) {
		Some((_, p)) => *p,
		None => return Err(err!(
			"There is no Unicode conformance file named {}.", name; Invalid, Input)),
	};
	if let Some(dir) = dest.parent() {
		if let Err(e) = fs::create_dir_all(dir) {
			return Err(err!(e, "While creating {:?}.", dir; IO, Path));
		}
	}
	let url = fmt!("{}/{}/{}", UCD_BASE, UCD_VERSION, rel);
	msg!("{} is not in the repository. Fetching {}.", name, url);
	let out = match Command::new("curl")
		.arg("-sS")
		.arg("--fail")
		.arg(&url)
		.output()
	{
		Ok(out) => out,
		Err(e) => return Err(err!(e,
			"While running curl to fetch {}. The conformance suites this file feeds cannot run \
			 without it, and fetching needs curl on the path and a network.", url;
		IO, Network)),
	};
	if !out.status.success() {
		return Err(err!(
			"curl could not fetch {}: {}", url, String::from_utf8_lossy(&out.stderr);
		IO, Network));
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let body = fmt!(
		"# {}\n\
		 # Fetched from {}/{}/{} by fe2o3_text/tests/unicode.rs.\n\
		 # Comment and blank lines have been stripped; the data lines are unchanged.\n\
		 {}",
		name, UCD_BASE, UCD_VERSION, rel, strip_comments(&text),
	);
	if let Err(e) = fs::write(dest, &body) {
		return Err(err!(e, "While writing {:?}.", dest; IO, File));
	}
	Ok(())
}

/// Drops comment and blank lines, leaving the data lines untouched, as the generator does.
fn strip_comments(text: &str) -> String {
	let mut out = String::new();
	for line in text.lines() {
		if line.starts_with('#') {
			continue;
		}
		let data = match line.find('#') {
			Some(i) => &line[..i],
			None => line,
		};
		let data = data.trim_end();
		if data.trim().is_empty() {
			continue;
		}
		out.push_str(data);
		out.push('\n');
	}
	out
}

/// Parses a hexadecimal code point.
fn hex(s: &str) -> Outcome<char> {
	let cp = match u32::from_str_radix(s.trim(), 16) {
		Ok(cp) => cp,
		Err(e) => return Err(err!(e, "{:?} is not a code point.", s; Invalid, Input)),
	};
	match char::from_u32(cp) {
		Some(c) => Ok(c),
		None => Err(err!("U+{:04X} is not a character.", cp; Invalid, Input)),
	}
}

/// Parses a space separated run of hexadecimal code points.
fn hexes(s: &str) -> Outcome<String> {
	let mut out = String::new();
	for part in s.split_whitespace() {
		out.push(res!(hex(part)));
	}
	Ok(out)
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Tables                                                                                    │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

#[test]
fn tables_are_consistent() -> Outcome<()> {
	res!(norm::check_tables());
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ UAX #15, normalisation                                                                    │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

#[test]
fn normalisation_conformance() -> Outcome<()> {

	let text = res!(data("NormalizationTest.txt"));
	let mut tally = Tally::new("NormalizationTest");

	for (n, line) in text.lines().enumerate() {
		if line.starts_with('#') || line.starts_with('@') || line.trim().is_empty() {
			continue;
		}
		let f: Vec<&str> = line.split(';').collect();
		if f.len() < 5 {
			return Err(err!(
				"Line {} of NormalizationTest.txt has {} fields, expected 5.", n + 1, f.len();
				Invalid, Input));
		}
		let c1 = res!(hexes(f[0]));
		let c2 = res!(hexes(f[1]));
		let c3 = res!(hexes(f[2]));
		let c4 = res!(hexes(f[3]));
		let c5 = res!(hexes(f[4]));

		// The invariants the file itself states.
		let mut bad = Vec::new();
		for (src, want, form, name) in [
			(&c1, &c2, Form::Nfc, "NFC(c1)"),	(&c2, &c2, Form::Nfc, "NFC(c2)"),
			(&c3, &c2, Form::Nfc, "NFC(c3)"),	(&c4, &c4, Form::Nfc, "NFC(c4)"),
			(&c5, &c4, Form::Nfc, "NFC(c5)"),
			(&c1, &c3, Form::Nfd, "NFD(c1)"),	(&c2, &c3, Form::Nfd, "NFD(c2)"),
			(&c3, &c3, Form::Nfd, "NFD(c3)"),	(&c4, &c5, Form::Nfd, "NFD(c4)"),
			(&c5, &c5, Form::Nfd, "NFD(c5)"),
			(&c1, &c4, Form::Nfkc, "NFKC(c1)"),	(&c2, &c4, Form::Nfkc, "NFKC(c2)"),
			(&c3, &c4, Form::Nfkc, "NFKC(c3)"),	(&c4, &c4, Form::Nfkc, "NFKC(c4)"),
			(&c5, &c4, Form::Nfkc, "NFKC(c5)"),
			(&c1, &c5, Form::Nfkd, "NFKD(c1)"),	(&c2, &c5, Form::Nfkd, "NFKD(c2)"),
			(&c3, &c5, Form::Nfkd, "NFKD(c3)"),	(&c4, &c5, Form::Nfkd, "NFKD(c4)"),
			(&c5, &c5, Form::Nfkd, "NFKD(c5)"),
		] {
			let got = norm::normalise(src, form);
			if got != *want {
				bad.push(fmt!("{} gave {:?}, expected {:?}", name, got, want));
			}
		}

		tally.case(n + 1, bad.is_empty(), bad.join("; "));
	}

	tally.finish()
}

#[test]
fn normalisation_leaves_the_rest_alone() -> Outcome<()> {

	// Every character that Part 1 of the test file does not list is its own normalisation, in
	// every form.
	let text = res!(data("NormalizationTest.txt"));
	let mut listed = vec![false; 0x110000];
	let mut part1 = false;
	for line in text.lines() {
		if line.starts_with("@Part1") {
			part1 = true;
			continue;
		}
		if line.starts_with('@') {
			part1 = false;
			continue;
		}
		if !part1 || line.starts_with('#') || line.trim().is_empty() {
			continue;
		}
		if let Some(first) = line.split(';').next() {
			let c = res!(hex(first));
			listed[c as usize] = true;
		}
	}

	let mut tally = Tally::new("NormalizationTest, Part 1 invariant");
	for cp in 0..=0x10FFFFu32 {
		let c = match char::from_u32(cp) {
			Some(c) => c,
			None => continue,
		};
		if listed[cp as usize] {
			continue;
		}
		let s = c.to_string();
		let ok = norm::nfc(&s) == s
			&& norm::nfd(&s) == s
			&& norm::nfkc(&s) == s
			&& norm::nfkd(&s) == s;
		if !ok {
			tally.case(cp as usize, false, fmt!("U+{:04X} is not its own normalisation.", cp));
		} else {
			tally.total += 1;
		}
	}

	tally.finish()
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ UAX #29, segmentation                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Parses a line of a break test into the string and the byte offsets of the breaks in it,
/// including the offset zero if the line begins with a break.
fn break_case(line: &str) -> Outcome<(String, Vec<usize>)> {
	let mut s		= String::new();
	let mut breaks	= Vec::new();
	for tok in line.split_whitespace() {
		match tok {
			"\u{00F7}" => breaks.push(s.len()),
			"\u{00D7}" => (),
			hx => s.push(res!(hex(hx))),
		}
	}
	Ok((s, breaks))
}

#[test]
fn grapheme_conformance() -> Outcome<()> {

	let text = res!(data("GraphemeBreakTest.txt"));
	let mut tally = Tally::new("GraphemeBreakTest");

	for (n, line) in text.lines().enumerate() {
		if line.starts_with('#') || line.trim().is_empty() {
			continue;
		}
		let (s, want)	= res!(break_case(line));
		let got			= segment::grapheme_boundaries(&s);
		tally.case(n + 1, got == want, fmt!("{:?}: got {:?}, expected {:?}", s, got, want));
	}

	tally.finish()
}

#[test]
fn word_conformance() -> Outcome<()> {

	let text = res!(data("WordBreakTest.txt"));
	let mut tally = Tally::new("WordBreakTest");

	for (n, line) in text.lines().enumerate() {
		if line.starts_with('#') || line.trim().is_empty() {
			continue;
		}
		let (s, want)	= res!(break_case(line));
		let got			= segment::word_boundaries(&s);
		tally.case(n + 1, got == want, fmt!("{:?}: got {:?}, expected {:?}", s, got, want));
	}

	tally.finish()
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ UAX #14, line breaking                                                                    │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

#[test]
fn linebreak_conformance() -> Outcome<()> {

	let text = res!(data("LineBreakTest.txt"));
	let mut tally = Tally::new("LineBreakTest");

	for (n, line) in text.lines().enumerate() {
		if line.starts_with('#') || line.trim().is_empty() {
			continue;
		}
		let (s, want)	= res!(break_case(line));
		let got			= linebreak::break_offsets(&s);
		tally.case(n + 1, got == want, fmt!("{:?}: got {:?}, expected {:?}", s, got, want));
	}

	tally.finish()
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ UAX #9, the bidirectional algorithm                                                       │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// A character of each Bidi_Class, for the test file that gives classes rather than characters.
/// None of them is a paired bracket, which is what `BidiTest.txt` assumes.
const REPS: &[(&str, char)] = &[
	("L",		'A'),
	("R",		'\u{05D0}'),
	("AL",		'\u{0627}'),
	("EN",		'0'),
	("ES",		'+'),
	("ET",		'#'),
	("AN",		'\u{0660}'),
	("CS",		','),
	("NSM",		'\u{0300}'),
	("BN",		'\u{00AD}'),
	("B",		'\u{2029}'),
	("S",		'\t'),
	("WS",		' '),
	("ON",		'!'),
	("LRE",		'\u{202A}'),
	("RLE",		'\u{202B}'),
	("PDF",		'\u{202C}'),
	("LRO",		'\u{202D}'),
	("RLO",		'\u{202E}'),
	("LRI",		'\u{2066}'),
	("RLI",		'\u{2067}'),
	("FSI",		'\u{2068}'),
	("PDI",		'\u{2069}'),
];

/// Returns the representative character of a Bidi_Class name.
fn rep(name: &str) -> Outcome<char> {
	for (n, c) in REPS {
		if *n == name {
			return Ok(*c);
		}
	}
	Err(err!("The Bidi_Class {:?} has no representative character.", name; Invalid, Input))
}

/// Checks the resolved levels and the visual order against what a test file expects, where an
/// expected level of `x` means the character is one the algorithm removes.
fn bidi_case(
	info:	&BidiInfo,
	levels:	&str,
	order:	&str,
)
	-> Outcome<Option<String>>
{
	let want: Vec<&str> = levels.split_whitespace().collect();
	if want.len() != info.levels.len() {
		return Ok(Some(fmt!(
			"expected {} levels, the text has {} characters", want.len(), info.levels.len())));
	}
	for (i, w) in want.iter().enumerate() {
		if *w == "x" {
			continue;
		}
		let lv = match w.parse::<u8>() {
			Ok(lv) => lv,
			Err(e) => return Err(err!(e, "{:?} is not a level.", w; Invalid, Input)),
		};
		if info.levels[i] != lv {
			return Ok(Some(fmt!(
				"level {} is {}, expected {}; all levels {:?}", i, info.levels[i], lv,
				info.levels)));
		}
	}

	let mut want_order = Vec::new();
	for part in order.split_whitespace() {
		match part.parse::<usize>() {
			Ok(i) => want_order.push(i),
			Err(e) => return Err(err!(e, "{:?} is not an index.", part; Invalid, Input)),
		}
	}
	let got = info.visual_order();
	if got != want_order {
		return Ok(Some(fmt!("visual order {:?}, expected {:?}", got, want_order)));
	}

	Ok(None)
}

#[test]
fn bidi_conformance() -> Outcome<()> {

	let text = res!(data("BidiTest.txt"));
	let mut tally = Tally::new("BidiTest");

	// The representative characters must really carry the class they stand for.
	for (name, c) in REPS {
		let got = fmt!("{:?}", B::of(*c));
		if got != *name {
			return Err(err!(
				"The representative U+{:04X} of Bidi_Class {} is {}.", *c as u32, name, got;
				Bug, Invalid));
		}
	}

	let mut levels = String::new();
	let mut order = String::new();

	for (n, line) in text.lines().enumerate() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		if let Some(rest) = line.strip_prefix("@Levels:") {
			levels = rest.trim().to_string();
			continue;
		}
		if let Some(rest) = line.strip_prefix("@Reorder:") {
			order = rest.trim().to_string();
			continue;
		}
		if line.starts_with('@') {
			continue;
		}

		let (input, bits) = match line.split_once(';') {
			Some((a, b)) => (a, b),
			None => return Err(err!(
				"Line {} of BidiTest.txt has no semicolon.", n + 1; Invalid, Input)),
		};
		let bits = match u8::from_str_radix(bits.trim(), 16) {
			Ok(bits) => bits,
			Err(e) => return Err(err!(e,
				"{:?} on line {} is not a bitset.", bits, n + 1; Invalid, Input)),
		};

		let mut chars	= Vec::new();
		let mut classes	= Vec::new();
		for name in input.split_whitespace() {
			let c = res!(rep(name));
			chars.push(c);
			classes.push(B::of(c));
		}

		for (bit, dir) in [(1u8, Direction::Auto), (2, Direction::Ltr), (4, Direction::Rtl)] {
			if bits & bit == 0 {
				continue;
			}
			let info = bidi::resolve_classes(&chars, &classes, dir);
			match res!(bidi_case(&info, &levels, &order)) {
				Some(why) => tally.case(n + 1, false,
					fmt!("{} with {:?}: {}", input.trim(), dir, why)),
				None => tally.case(n + 1, true, String::new()),
			}
		}
	}

	tally.finish()
}

#[test]
fn bidi_character_conformance() -> Outcome<()> {

	let text = res!(data("BidiCharacterTest.txt"));
	let mut tally = Tally::new("BidiCharacterTest");

	for (n, line) in text.lines().enumerate() {
		if line.starts_with('#') || line.trim().is_empty() {
			continue;
		}
		let f: Vec<&str> = line.split(';').collect();
		if f.len() < 5 {
			return Err(err!(
				"Line {} of BidiCharacterTest.txt has {} fields, expected 5.", n + 1, f.len();
				Invalid, Input));
		}

		let mut chars = Vec::new();
		for part in f[0].split_whitespace() {
			chars.push(res!(hex(part)));
		}
		let dir = match f[1].trim() {
			"0" => Direction::Ltr,
			"1" => Direction::Rtl,
			"2" => Direction::Auto,
			other => return Err(err!(
				"{:?} on line {} is not a paragraph direction.", other, n + 1; Invalid, Input)),
		};
		let want_para = match f[2].trim().parse::<u8>() {
			Ok(lv) => lv,
			Err(e) => return Err(err!(e,
				"{:?} on line {} is not a level.", f[2], n + 1; Invalid, Input)),
		};

		let classes: Vec<B> = chars.iter().map(|c| B::of(*c)).collect();
		let info = bidi::resolve_classes(&chars, &classes, dir);

		if info.para_level != want_para {
			tally.case(n + 1, false, fmt!(
				"paragraph level {}, expected {}", info.para_level, want_para));
			continue;
		}
		match res!(bidi_case(&info, f[3], f[4])) {
			Some(why) => tally.case(n + 1, false, why),
			None => tally.case(n + 1, true, String::new()),
		}
	}

	tally.finish()
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ The shape of the API                                                                      │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

#[test]
fn normalisation_basics() -> Outcome<()> {

	req!(norm::nfc("e\u{0301}"), "\u{00E9}");
	req!(norm::nfd("\u{00E9}"), "e\u{0301}");
	req!(norm::nfkc("\u{FB01}"), "fi");
	req!(norm::nfkd("\u{2460}"), "1");

	// A Hangul syllable composes and decomposes arithmetically.
	req!(norm::nfd("\u{AC01}"), "\u{1100}\u{1161}\u{11A8}");
	req!(norm::nfc("\u{1100}\u{1161}\u{11A8}"), "\u{AC01}");

	// The marks come out in canonical order whichever order they went in.
	req!(norm::nfd("q\u{0307}\u{0323}"), norm::nfd("q\u{0323}\u{0307}"));
	req!(norm::eq_canonical("\u{00C5}", "A\u{030A}"), true);

	// A singleton does not compose back.
	req!(norm::nfc("\u{2126}"), "\u{03A9}");

	req!(norm::is_normalised("abc", Form::Nfc), true);
	req!(norm::is_normalised("e\u{0301}", Form::Nfc), false);

	Ok(())
}

#[test]
fn segmentation_basics() -> Outcome<()> {

	req!(segment::graphemes("hi"), vec!["h", "i"]);
	req!(segment::graphemes("e\u{0301}"), vec!["e\u{0301}"]);
	req!(segment::graphemes("\u{1F1E6}\u{1F1FA}"), vec!["\u{1F1E6}\u{1F1FA}"]);
	req!(segment::graphemes("\r\n"), vec!["\r\n"]);

	// A cursor steps over a whole cluster.
	let s = "e\u{0301}x";
	req!(segment::next_grapheme(s, 0), 3);
	req!(segment::prev_grapheme(s, 4), 3);

	req!(segment::words("one two"), vec!["one", " ", "two"]);

	Ok(())
}

#[test]
fn linebreak_basics() -> Outcome<()> {

	let opps = linebreak::line_breaks("one two");
	req!(opps.len(), 2);
	req!(opps[0].offset, 4);
	req!(opps[0].kind, linebreak::Break::Optional);
	req!(opps[1].offset, 7);
	req!(opps[1].kind, linebreak::Break::Mandatory);

	// A hard break is mandatory, and no break falls inside a non-breaking space.
	let opps = linebreak::line_breaks("a\nb");
	req!(opps[0].offset, 2);
	req!(opps[0].kind, linebreak::Break::Mandatory);

	req!(linebreak::break_offsets("a\u{00A0}b"), vec![4]);

	Ok(())
}

#[test]
fn bidi_basics() -> Outcome<()> {

	// A left to right paragraph with a right to left word in it.
	let info = bidi::resolve("he said \u{05D0}\u{05D1}\u{05D2} to me", Direction::Auto);
	req!(info.para_level, 0);
	req!(info.has_rtl(), true);
	req!(info.levels[8], 1);

	// A right to left paragraph, taken from its first strong character.
	let info = bidi::resolve("\u{05D0} a", Direction::Auto);
	req!(info.para_level, 1);
	req!(info.visual_order(), vec![2, 1, 0]);

	// Plain text needs nothing done to it.
	let info = bidi::resolve("plain", Direction::Auto);
	req!(info.has_rtl(), false);
	req!(info.visual_order(), vec![0, 1, 2, 3, 4]);

	Ok(())
}
