//! Generator for the committed Unicode tables in `fe2o3_text::unicode`.
//!
//! This binary downloads the Unicode Character Database files for the pinned UCD version, parses
//! them, and writes Rust source into `src/unicode/tables/`, plus the Unicode Consortium
//! conformance test files into `tests/unicode_data/`. Both sets of outputs are committed, so the
//! library builds with no build script, no runtime download and no third party dependency.
//!
//! Downloads go through `curl`, which keeps the crate free of an HTTP dependency; the files are
//! cached under the system temporary directory so that repeated runs do not hit unicode.org.
//!
//! Run it from the crate root:
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_text --bin gen_unicode
//! ```
//!
#![forbid(unsafe_code)]

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::{
		BTreeMap,
		BTreeSet,
	},
	fmt::Write as _,
	fs,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

/// The pinned Unicode Character Database version. Every generated table and vendored test file
/// comes from this release.
const UCD_VERSION: &str = "17.0.0";

/// Root of the Unicode public file server.
const UCD_BASE: &str = "https://www.unicode.org/Public";

/// The highest Unicode code point.
const MAX_CP: u32 = 0x10FFFF;

/// The number of code points, including the surrogate gap.
const NUM_CP: usize = (MAX_CP as usize) + 1;

/// Data files parsed into tables, given as paths below the version directory.
const DATA_FILES: &[&str] = &[
	"ucd/UnicodeData.txt",
	"ucd/DerivedNormalizationProps.txt",
	"ucd/DerivedCoreProperties.txt",
	"ucd/extracted/DerivedBidiClass.txt",
	"ucd/BidiBrackets.txt",
	"ucd/LineBreak.txt",
	"ucd/EastAsianWidth.txt",
	"ucd/auxiliary/GraphemeBreakProperty.txt",
	"ucd/auxiliary/WordBreakProperty.txt",
	"ucd/emoji/emoji-data.txt",
	"ucd/Scripts.txt",
	"ucd/ScriptExtensions.txt",
	"ucd/PropList.txt",
	"ucd/PropertyAliases.txt",
	"ucd/PropertyValueAliases.txt",
	"ucd/auxiliary/SentenceBreakProperty.txt",
];

/// Conformance test files vendored into `tests/unicode_data/`.
const TEST_FILES: &[&str] = &[
	"ucd/NormalizationTest.txt",
	"ucd/BidiTest.txt",
	"ucd/BidiCharacterTest.txt",
	"ucd/auxiliary/GraphemeBreakTest.txt",
	"ucd/auxiliary/WordBreakTest.txt",
	"ucd/auxiliary/LineBreakTest.txt",
];

// The property value lists below fix the enum variant order. A value in a UCD file that is absent
// from these lists is an error, so a new Unicode release cannot silently drop characters into a
// wrong class.

/// Line_Break property values, in the order UAX #14 lists them.
const LB_CLASSES: &[(&str, &str, &str)] = &[
	("XX",	"XX",	"Unknown, treated as AL."),
	("AI",	"AI",	"Ambiguous, treated as AL by default."),
	("AK",	"AK",	"Aksara."),
	("AL",	"AL",	"Alphabetic."),
	("AP",	"AP",	"Aksara pre-base."),
	("AS",	"AS",	"Aksara start."),
	("B2",	"B2",	"Break opportunity before and after."),
	("BA",	"BA",	"Break after."),
	("BB",	"BB",	"Break before."),
	("BK",	"BK",	"Mandatory break."),
	("CB",	"CB",	"Contingent break opportunity."),
	("CJ",	"CJ",	"Conditional Japanese starter, treated as NS by default."),
	("CL",	"CL",	"Close punctuation."),
	("CM",	"CM",	"Combining mark."),
	("CP",	"CP",	"Close parenthesis."),
	("CR",	"CR",	"Carriage return."),
	("EB",	"EB",	"Emoji base."),
	("EM",	"EM",	"Emoji modifier."),
	("EX",	"EX",	"Exclamation or interrogation."),
	("GL",	"GL",	"Non-breaking glue."),
	("H2",	"H2",	"Hangul LV syllable."),
	("H3",	"H3",	"Hangul LVT syllable."),
	("HH",	"HH",	"Unambiguous hyphen."),
	("HL",	"HL",	"Hebrew letter."),
	("HY",	"HY",	"Hyphen."),
	("ID",	"ID",	"Ideographic."),
	("IN",	"IN",	"Inseparable."),
	("IS",	"IS",	"Infix numeric separator."),
	("JL",	"JL",	"Hangul leading jamo."),
	("JT",	"JT",	"Hangul trailing jamo."),
	("JV",	"JV",	"Hangul vowel jamo."),
	("LF",	"LF",	"Line feed."),
	("NL",	"NL",	"Next line."),
	("NS",	"NS",	"Nonstarter."),
	("NU",	"NU",	"Numeric."),
	("OP",	"OP",	"Open punctuation."),
	("PO",	"PO",	"Postfix numeric."),
	("PR",	"PR",	"Prefix numeric."),
	("QU",	"QU",	"Quotation."),
	("RI",	"RI",	"Regional indicator."),
	("SA",	"SA",	"Complex context, South East Asian."),
	("SG",	"SG",	"Surrogate, treated as AL."),
	("SP",	"SP",	"Space."),
	("SY",	"SY",	"Symbols allowing break after."),
	("VF",	"VF",	"Virama final."),
	("VI",	"VI",	"Virama."),
	("WJ",	"WJ",	"Word joiner."),
	("ZW",	"ZW",	"Zero width space."),
	("ZWJ",	"ZWJ",	"Zero width joiner."),
];

/// Grapheme_Cluster_Break property values, as the UCD name, the Rust variant, and its doc.
const GCB_CLASSES: &[(&str, &str, &str)] = &[
	("Other",				"Other",				"Any character not in another class."),
	("CR",					"CR",					"Carriage return."),
	("LF",					"LF",					"Line feed."),
	("Control",				"Control",				"A control, format or line separator character."),
	("Extend",				"Extend",				"A character that extends the preceding one."),
	("ZWJ",					"ZWJ",					"Zero width joiner."),
	("Regional_Indicator",	"RegionalIndicator",	"One half of a flag sequence."),
	("Prepend",				"Prepend",				"A character that prefixes the following one."),
	("SpacingMark",			"SpacingMark",			"A spacing combining mark."),
	("L",					"L",					"Hangul leading jamo."),
	("V",					"V",					"Hangul vowel jamo."),
	("T",					"T",					"Hangul trailing jamo."),
	("LV",					"LV",					"Hangul LV syllable."),
	("LVT",					"LVT",					"Hangul LVT syllable."),
];

/// Word_Break property values, as the UCD name, the Rust variant, and its doc.
const WB_CLASSES: &[(&str, &str, &str)] = &[
	("Other",				"Other",				"Any character not in another class."),
	("CR",					"CR",					"Carriage return."),
	("LF",					"LF",					"Line feed."),
	("Newline",				"Newline",				"A newline character other than CR or LF."),
	("Extend",				"Extend",				"A character that extends the preceding one."),
	("Format",				"Format",				"A format character."),
	("Katakana",			"Katakana",				"A katakana character."),
	("ALetter",				"ALetter",				"A letter that takes part in words."),
	("MidLetter",			"MidLetter",			"A character found inside a word, such as a colon."),
	("MidNum",				"MidNum",				"A character found inside a number, such as a comma."),
	("MidNumLet",			"MidNumLet",			"A character found inside either a word or a number."),
	("Numeric",				"Numeric",				"A decimal digit."),
	("ExtendNumLet",		"ExtendNumLet",			"A character that joins words and numbers, such as an underscore."),
	("Regional_Indicator",	"RegionalIndicator",	"One half of a flag sequence."),
	("Hebrew_Letter",		"HebrewLetter",			"A Hebrew letter."),
	("Double_Quote",		"DoubleQuote",			"A double quotation mark."),
	("Single_Quote",		"SingleQuote",			"A single quotation mark."),
	("ZWJ",					"ZWJ",					"Zero width joiner."),
	("WSegSpace",			"WSegSpace",			"A space that separates words."),
];

/// Bidi_Class property values, by their UCD long names.
const BIDI_CLASSES: &[(&str, &str, &str)] = &[
	("Left_To_Right",			"L",	"Strong left to right."),
	("Right_To_Left",			"R",	"Strong right to left."),
	("Arabic_Letter",			"AL",	"Strong right to left Arabic."),
	("European_Number",			"EN",	"European number."),
	("European_Separator",		"ES",	"European number separator."),
	("European_Terminator",		"ET",	"European number terminator."),
	("Arabic_Number",			"AN",	"Arabic number."),
	("Common_Separator",		"CS",	"Common number separator."),
	("Nonspacing_Mark",			"NSM",	"Nonspacing mark."),
	("Boundary_Neutral",		"BN",	"Boundary neutral."),
	("Paragraph_Separator",		"B",	"Paragraph separator."),
	("Segment_Separator",		"S",	"Segment separator."),
	("White_Space",				"WS",	"Whitespace."),
	("Other_Neutral",			"ON",	"Other neutral."),
	("Left_To_Right_Embedding",	"LRE",	"Left to right embedding."),
	("Left_To_Right_Override",	"LRO",	"Left to right override."),
	("Right_To_Left_Embedding",	"RLE",	"Right to left embedding."),
	("Right_To_Left_Override",	"RLO",	"Right to left override."),
	("Pop_Directional_Format",	"PDF",	"Pop directional format."),
	("Left_To_Right_Isolate",	"LRI",	"Left to right isolate."),
	("Right_To_Left_Isolate",	"RLI",	"Right to left isolate."),
	("First_Strong_Isolate",	"FSI",	"First strong isolate."),
	("Pop_Directional_Isolate",	"PDI",	"Pop directional isolate."),
];

/// Indic_Conjunct_Break property values.
const INCB_CLASSES: &[(&str, &str, &str)] = &[
	("None",	"None",	"Not part of a conjunct."),
	("Consonant",	"Consonant",	"A consonant that can begin or end a conjunct."),
	("Extend",	"Extend",	"A mark that does not interrupt a conjunct."),
	("Linker",	"Linker",	"A virama that joins two consonants."),
];

/// Bit in the line breaking flags table marking a character of East Asian width F, W or H.
const LB_FLAG_EAST_ASIAN: u8	= 1 << 0;
/// Bit marking an initial quotation mark, general category Pi.
const LB_FLAG_PI: u8			= 1 << 1;
/// Bit marking a final quotation mark, general category Pf.
const LB_FLAG_PF: u8			= 1 << 2;
/// Bit marking an unassigned code point with Extended_Pictographic.
const LB_FLAG_EXT_PICT_UNASSIGNED: u8 = 1 << 3;
/// Bit marking general category Mn or Mc, which decides how class SA resolves.
const LB_FLAG_MARK: u8			= 1 << 4;

/// Bit in the segmentation flags table marking Extended_Pictographic.
const SEG_FLAG_EXT_PICT: u8		= 1 << 0;
/// Shift of the two bit Indic_Conjunct_Break field in the segmentation flags table.
const SEG_INCB_SHIFT: u8		= 1;

fn main() {
	match run() {
		Ok(()) => println!("Done."),
		Err(e) => {
			eprintln!("gen_unicode failed: {}", e);
			std::process::exit(1);
		},
	}
}

/// Downloads, parses and emits everything.
fn run() -> Outcome<()> {

	let root	= PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let cache	= std::env::temp_dir().join("fe2o3_ucd").join(UCD_VERSION);
	res!(mkdir(&cache));

	let mut src = BTreeMap::new();
	for path in DATA_FILES {
		let text = res!(fetch(&cache, path));
		src.insert(*path, text);
	}

	let ucd = res!(Ucd::parse(&src));

	let tables = root.join("src").join("unicode").join("tables");
	res!(mkdir(&tables));

	res!(write_file(&tables.join("mod.rs"),		&emit_mod()));
	let cats = res!(Cats::parse(&src, &ucd));

	res!(write_file(&tables.join("prop.rs"),	&emit_prop(&cats)));
	res!(write_file(&tables.join("cat.rs"),		&res!(emit_cat(&cats))));
	res!(write_file(&tables.join("norm.rs"),	&res!(emit_norm(&ucd))));
	res!(write_file(&tables.join("lb.rs"),		&emit_lb(&ucd)));
	res!(write_file(&tables.join("seg.rs"),		&emit_seg(&ucd)));
	res!(write_file(&tables.join("bidi.rs"),	&emit_bidi(&ucd)));

	// Vendor the conformance data.
	let data = root.join("tests").join("unicode_data");
	res!(mkdir(&data));
	for path in TEST_FILES {
		let text	= res!(fetch(&cache, path));
		let name	= res!(base_name(path));
		let stripped = strip_comments(&text);
		let out		= fmt!(
			"# {}\n\
			 # Vendored from {}/{}/{} by fe2o3_text/src/bin/gen_unicode.rs.\n\
			 # Comment and blank lines have been stripped; the data lines are unchanged.\n\
			 {}",
			name, UCD_BASE, UCD_VERSION, path, stripped,
		);
		res!(write_file(&data.join(&name), &out));
	}

	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Fetching                                                                                  │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Creates a directory and any missing parents.
fn mkdir(path: &Path) -> Outcome<()> {
	match fs::create_dir_all(path) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e, "While creating {:?}.", path; IO, Path)),
	}
}

/// Returns the file name at the end of a UCD path.
fn base_name(path: &str) -> Outcome<String> {
	match path.rsplit('/').next() {
		Some(name) => Ok(name.to_string()),
		None => Err(err!("The path {} has no file name.", path; Invalid, Input)),
	}
}

/// Reads a UCD file from the cache, downloading it with `curl` if it is not there.
fn fetch(cache: &Path, path: &str) -> Outcome<String> {

	let name = res!(base_name(path));
	let dest = cache.join(&name);

	if !dest.exists() {
		let url = fmt!("{}/{}/{}", UCD_BASE, UCD_VERSION, path);
		println!("Downloading {}", url);
		let out = match Command::new("curl")
			.arg("-sS")
			.arg("--fail")
			.arg("-o")
			.arg(&dest)
			.arg(&url)
			.output()
		{
			Ok(out) => out,
			Err(e) => return Err(err!(e,
				"While running curl for {}. The generator needs curl on the path.", url;
				IO, Network)),
		};
		if !out.status.success() {
			return Err(err!(
				"curl could not fetch {}: {}", url, String::from_utf8_lossy(&out.stderr);
				IO, Network));
		}
	}

	match fs::read_to_string(&dest) {
		Ok(text) => Ok(text),
		Err(e) => Err(err!(e, "While reading {:?}.", dest; IO, File)),
	}
}

/// Writes a generated file.
fn write_file(path: &Path, text: &str) -> Outcome<()> {
	match fs::write(path, text) {
		Ok(()) => {
			println!("Wrote {:?} ({} bytes)", path, text.len());
			Ok(())
		},
		Err(e) => Err(err!(e, "While writing {:?}.", path; IO, File)),
	}
}

/// Removes comment and blank lines, and any trailing comment on a data line.
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

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Parsing                                                                                   │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Everything parsed out of the UCD, indexed by code point where it is dense.
struct Ucd {
	/// General category, as the two letter abbreviation. Unassigned code points are `Cn`.
	gc:			Vec<[u8; 2]>,
	/// Canonical combining class.
	ccc:		Vec<u8>,
	/// Canonical decomposition mappings.
	canon:		BTreeMap<u32, Vec<u32>>,
	/// Compatibility decomposition mappings, excluding the canonical ones.
	compat:		BTreeMap<u32, Vec<u32>>,
	/// Code points with Full_Composition_Exclusion.
	excl:		BTreeSet<u32>,
	/// Line_Break class index into [`LB_CLASSES`].
	lb:			Vec<u8>,
	/// Line breaking flags, a bitset of the `LB_FLAG_*` constants.
	lb_flags:	Vec<u8>,
	/// Grapheme_Cluster_Break class index into [`GCB_CLASSES`].
	gcb:		Vec<u8>,
	/// Word_Break class index into [`WB_CLASSES`].
	wb:			Vec<u8>,
	/// Segmentation flags: Extended_Pictographic and Indic_Conjunct_Break.
	seg_flags:	Vec<u8>,
	/// Bidi_Class index into [`BIDI_CLASSES`].
	bidi:		Vec<u8>,
	/// Bidi paired brackets, mapping a bracket to its pair and its kind, 0 open and 1 close.
	brackets:	BTreeMap<u32, (u32, u8)>,
}

/// Parses a hexadecimal code point.
fn hex(s: &str) -> Outcome<u32> {
	match u32::from_str_radix(s.trim(), 16) {
		Ok(v) if v <= MAX_CP => Ok(v),
		Ok(v) => Err(err!("The code point {:X} is out of range.", v; Invalid, Input, Range)),
		Err(e) => Err(err!(e, "The string {:?} is not a code point.", s; Invalid, Input)),
	}
}

/// Parses a `start..end` or `cp` range from the first field of a property file line.
fn range(s: &str) -> Outcome<(u32, u32)> {
	let s = s.trim();
	match s.split_once("..") {
		Some((a, b)) => Ok((res!(hex(a)), res!(hex(b)))),
		None => {
			let cp = res!(hex(s));
			Ok((cp, cp))
		},
	}
}

/// Splits a property file line into its fields, dropping any comment.
fn fields(line: &str) -> Option<Vec<&str>> {
	let data = match line.find('#') {
		Some(i) => &line[..i],
		None => line,
	};
	if data.trim().is_empty() {
		return None;
	}
	Some(data.split(';').map(|f| f.trim()).collect())
}

/// Returns the index of `name` in a property value list, or an error naming the file.
fn class_index(list: &[(&str, &str, &str)], name: &str, file: &str) -> Outcome<u8> {
	for (i, (ucd, rust, _)) in list.iter().enumerate() {
		if *ucd == name || *rust == name {
			return Ok(i as u8);
		}
	}
	Err(err!(
		"The property value {:?} in {} is not known to the generator. \
		 A new Unicode release may have added it.", name, file;
		Invalid, Input, Mismatch))
}

impl Ucd {

	/// Parses every downloaded data file.
	fn parse(src: &BTreeMap<&str, String>) -> Outcome<Self> {

		let get = |path: &str| -> Outcome<&String> {
			match src.get(path) {
				Some(text) => Ok(text),
				None => Err(err!("The file {} was not downloaded.", path; Missing, Input)),
			}
		};

		let mut ucd = Self {
			gc:			vec![*b"Cn"; NUM_CP],
			ccc:		vec![0; NUM_CP],
			canon:		BTreeMap::new(),
			compat:		BTreeMap::new(),
			excl:		BTreeSet::new(),
			lb:			vec![0; NUM_CP], // XX
			lb_flags:	vec![0; NUM_CP],
			gcb:		vec![0; NUM_CP], // Other
			wb:			vec![0; NUM_CP], // Other
			seg_flags:	vec![0; NUM_CP],
			bidi:		vec![0; NUM_CP], // L
			brackets:	BTreeMap::new(),
		};

		res!(ucd.parse_unicode_data(res!(get("ucd/UnicodeData.txt"))));
		res!(ucd.parse_norm_props(res!(get("ucd/DerivedNormalizationProps.txt"))));
		res!(ucd.parse_bidi_class(res!(get("ucd/extracted/DerivedBidiClass.txt"))));
		res!(ucd.parse_brackets(res!(get("ucd/BidiBrackets.txt"))));
		res!(ucd.parse_line_break(res!(get("ucd/LineBreak.txt"))));

		let eaw	= res!(ucd.parse_east_asian(res!(get("ucd/EastAsianWidth.txt"))));
		let ext	= res!(parse_ext_pict(res!(get("ucd/emoji/emoji-data.txt"))));
		res!(ucd.parse_gcb(res!(get("ucd/auxiliary/GraphemeBreakProperty.txt"))));
		res!(ucd.parse_wb(res!(get("ucd/auxiliary/WordBreakProperty.txt"))));
		let incb = res!(parse_incb(res!(get("ucd/DerivedCoreProperties.txt"))));

		res!(ucd.derive_flags(&eaw, &ext, &incb));

		Ok(ucd)
	}

	/// Parses UnicodeData.txt for general category, combining class and decompositions.
	fn parse_unicode_data(&mut self, text: &str) -> Outcome<()> {

		let mut first: Option<u32> = None;

		for line in text.lines() {
			if line.trim().is_empty() {
				continue;
			}
			let f: Vec<&str> = line.split(';').collect();
			if f.len() < 6 {
				return Err(err!(
					"UnicodeData.txt line {:?} has {} fields, expected at least 6.",
					line, f.len(); Invalid, Input));
			}
			let cp		= res!(hex(f[0]));
			let name	= f[1];
			let gc		= f[2].as_bytes();
			if gc.len() != 2 {
				return Err(err!(
					"The general category {:?} at U+{:04X} is not two letters.", f[2], cp;
					Invalid, Input));
			}
			let ccc = match f[3].trim().parse::<u16>() {
				Ok(v) if v <= 255 => v as u8,
				_ => return Err(err!(
					"The combining class {:?} at U+{:04X} is not a byte.", f[3], cp;
					Invalid, Input)),
			};

			// A range is written as a pair of lines ending in <..., First> and <..., Last>.
			let (lo, hi) = if name.ends_with(", First>") {
				first = Some(cp);
				continue;
			} else if name.ends_with(", Last>") {
				let lo = match first.take() {
					Some(lo) => lo,
					None => return Err(err!(
						"The range ending at U+{:04X} has no First line.", cp; Invalid, Input)),
				};
				(lo, cp)
			} else {
				(cp, cp)
			};

			for c in lo..=hi {
				let i = c as usize;
				self.gc[i] = [gc[0], gc[1]];
				self.ccc[i] = ccc;
			}

			// Field 5 is the decomposition mapping, prefixed by a tag if it is a compatibility
			// mapping.
			let dec = f[5].trim();
			if !dec.is_empty() && lo == hi {
				let (compat, body) = if dec.starts_with('<') {
					match dec.split_once('>') {
						Some((_, rest)) => (true, rest.trim()),
						None => return Err(err!(
							"The decomposition {:?} at U+{:04X} has an unclosed tag.", dec, cp;
							Invalid, Input)),
					}
				} else {
					(false, dec)
				};
				let mut seq = Vec::new();
				for part in body.split_whitespace() {
					seq.push(res!(hex(part)));
				}
				if seq.is_empty() {
					return Err(err!(
						"The decomposition {:?} at U+{:04X} is empty.", dec, cp; Invalid, Input));
				}
				if compat {
					self.compat.insert(cp, seq);
				} else {
					self.canon.insert(cp, seq);
				}
			}
		}

		Ok(())
	}

	/// Parses the Full_Composition_Exclusion property.
	fn parse_norm_props(&mut self, text: &str) -> Outcome<()> {
		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 || f[1] != "Full_Composition_Exclusion" {
				continue;
			}
			let (lo, hi) = res!(range(f[0]));
			for c in lo..=hi {
				self.excl.insert(c);
			}
		}
		Ok(())
	}

	/// Parses the Bidi_Class property, honouring the `@missing` defaults for unassigned code
	/// points.
	fn parse_bidi_class(&mut self, text: &str) -> Outcome<()> {

		let index = |name: &str| -> Outcome<u8> {
			class_index(BIDI_CLASSES, name, "DerivedBidiClass.txt")
		};

		// The @missing lines give the class of the unassigned code points in a range, so they must
		// be applied before the explicit assignments.
		for line in text.lines() {
			let body = match line.strip_prefix("# @missing:") {
				Some(body) => body,
				None => continue,
			};
			let f: Vec<&str> = body.split(';').map(|s| s.trim()).collect();
			if f.len() < 2 {
				return Err(err!(
					"The @missing line {:?} has too few fields.", line; Invalid, Input));
			}
			let (lo, hi)	= res!(range(f[0]));
			let cls			= res!(index(f[1]));
			for c in lo..=hi {
				self.bidi[c as usize] = cls;
			}
		}

		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi)	= res!(range(f[0]));
			let cls			= res!(index(f[1]));
			for c in lo..=hi {
				self.bidi[c as usize] = cls;
			}
		}

		Ok(())
	}

	/// Parses BidiBrackets.txt.
	fn parse_brackets(&mut self, text: &str) -> Outcome<()> {
		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 3 {
				continue;
			}
			let cp		= res!(hex(f[0]));
			let pair	= res!(hex(f[1]));
			let kind	= match f[2] {
				"o" => 0u8,
				"c" => 1u8,
				other => return Err(err!(
					"The bracket kind {:?} at U+{:04X} is neither o nor c.", other, cp;
					Invalid, Input)),
			};
			self.brackets.insert(cp, (pair, kind));
		}
		Ok(())
	}

	/// Parses LineBreak.txt.
	///
	/// The file lists the reserved code points that take a value other than the default, so the
	/// defaults its header describes in prose need no code here: a code point the file does not
	/// list takes XX, which the algorithm resolves to AL. The Extended_Pictographic code points
	/// that are still unassigned are the ones this matters for, and they are deliberately left at
	/// XX.
	fn parse_line_break(&mut self, text: &str) -> Outcome<()> {

		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi)	= res!(range(f[0]));
			let cls			= res!(class_index(LB_CLASSES, f[1], "LineBreak.txt"));
			for c in lo..=hi {
				self.lb[c as usize] = cls;
			}
		}

		Ok(())
	}

	/// Parses EastAsianWidth.txt into a per code point width letter. As with LineBreak.txt, the
	/// file lists the reserved ranges that take a value other than the default of N.
	fn parse_east_asian(&self, text: &str) -> Outcome<Vec<u8>> {

		let mut eaw = vec![b'N'; NUM_CP];

		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi) = res!(range(f[0]));
			let w = match f[1] {
				"A"		=> b'A',
				"F"		=> b'F',
				"H"		=> b'H',
				"N"		=> b'N',
				"Na"	=> b'n',
				"W"		=> b'W',
				other => return Err(err!(
					"The East_Asian_Width value {:?} is not known.", other; Invalid, Input)),
			};
			for c in lo..=hi {
				eaw[c as usize] = w;
			}
		}

		Ok(eaw)
	}

	/// Parses GraphemeBreakProperty.txt.
	fn parse_gcb(&mut self, text: &str) -> Outcome<()> {
		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi)	= res!(range(f[0]));
			let cls			= res!(class_index(GCB_CLASSES, f[1], "GraphemeBreakProperty.txt"));
			for c in lo..=hi {
				self.gcb[c as usize] = cls;
			}
		}
		Ok(())
	}

	/// Parses WordBreakProperty.txt.
	fn parse_wb(&mut self, text: &str) -> Outcome<()> {
		for line in text.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None => continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi)	= res!(range(f[0]));
			let cls			= res!(class_index(WB_CLASSES, f[1], "WordBreakProperty.txt"));
			for c in lo..=hi {
				self.wb[c as usize] = cls;
			}
		}
		Ok(())
	}

	/// Combines the auxiliary properties into the two flag tables.
	fn derive_flags(
		&mut self,
		eaw:	&[u8],
		ext:	&[bool],
		incb:	&[u8],
	)
		-> Outcome<()>
	{
		for c in 0..NUM_CP {
			let gc = self.gc[c];

			let mut lb = 0u8;
			if matches!(eaw[c], b'F' | b'W' | b'H') {
				lb |= LB_FLAG_EAST_ASIAN;
			}
			if gc == *b"Pi" {
				lb |= LB_FLAG_PI;
			}
			if gc == *b"Pf" {
				lb |= LB_FLAG_PF;
			}
			if ext[c] && gc == *b"Cn" {
				lb |= LB_FLAG_EXT_PICT_UNASSIGNED;
			}
			if gc == *b"Mn" || gc == *b"Mc" {
				lb |= LB_FLAG_MARK;
			}
			self.lb_flags[c] = lb;

			let mut seg = 0u8;
			if ext[c] {
				seg |= SEG_FLAG_EXT_PICT;
			}
			seg |= incb[c] << SEG_INCB_SHIFT;
			self.seg_flags[c] = seg;
		}
		Ok(())
	}
}

/// Parses the Extended_Pictographic property from emoji-data.txt.
fn parse_ext_pict(text: &str) -> Outcome<Vec<bool>> {
	let mut ext = vec![false; NUM_CP];
	for line in text.lines() {
		let f = match fields(line) {
			Some(f) => f,
			None => continue,
		};
		if f.len() < 2 || f[1] != "Extended_Pictographic" {
			continue;
		}
		let (lo, hi) = res!(range(f[0]));
		for c in lo..=hi {
			ext[c as usize] = true;
		}
	}
	Ok(ext)
}

/// Parses the Indic_Conjunct_Break property from DerivedCoreProperties.txt.
fn parse_incb(text: &str) -> Outcome<Vec<u8>> {
	let mut incb = vec![0u8; NUM_CP];
	for line in text.lines() {
		let f = match fields(line) {
			Some(f) => f,
			None => continue,
		};
		if f.len() < 3 || f[1] != "InCB" {
			continue;
		}
		let (lo, hi)	= res!(range(f[0]));
		let cls			= res!(class_index(INCB_CLASSES, f[2], "DerivedCoreProperties.txt"));
		for c in lo..=hi {
			incb[c as usize] = cls;
		}
	}
	Ok(incb)
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Emission                                                                                  │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Returns the header that every generated file carries.
fn header(what: &str) -> String {
	fmt!(
		"// GENERATED FILE. DO NOT EDIT.\n\
		 //\n\
		 // {}\n\
		 //\n\
		 // Generated by fe2o3_text/src/bin/gen_unicode.rs from the Unicode Character Database,\n\
		 // version {}, at {}/{}/.\n\
		 // Run `cargo run -p oxedyne_fe2o3_text --bin gen_unicode` to rebuild.\n\n",
		what, UCD_VERSION, UCD_BASE, UCD_VERSION,
	)
}

/// Compresses a dense per code point value array into a partition: the code point at which each
/// run starts, and the value of that run. A lookup binary searches the starts.
fn partition(vals: &[u8]) -> (Vec<u32>, Vec<u8>) {
	let mut starts	= Vec::new();
	let mut runs	= Vec::new();
	let mut prev	= None;
	for (c, v) in vals.iter().enumerate() {
		if prev != Some(*v) {
			starts.push(c as u32);
			runs.push(*v);
			prev = Some(*v);
		}
	}
	(starts, runs)
}

/// Emits a `u32` array, eight values to the line.
fn emit_u32(out: &mut String, name: &str, doc: &str, vals: &[u32]) {
	let _ = write!(out, "/// {}\npub static {}: [u32; {}] = [\n", doc, name, vals.len());
	for chunk in vals.chunks(8) {
		let mut line = String::from("\t");
		for v in chunk {
			let _ = write!(line, "0x{:X}, ", v);
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
}

/// Emits a `u8` array, sixteen values to the line.
fn emit_u8(out: &mut String, name: &str, doc: &str, vals: &[u8]) {
	let _ = write!(out, "/// {}\npub static {}: [u8; {}] = [\n", doc, name, vals.len());
	for chunk in vals.chunks(16) {
		let mut line = String::from("\t");
		for v in chunk {
			let _ = write!(line, "{}, ", v);
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
}

/// Emits an array of enum variants, eight values to the line.
fn emit_enum_vals(
	out:	&mut String,
	name:	&str,
	doc:	&str,
	typ:	&str,
	list:	&[(&str, &str, &str)],
	vals:	&[u8],
)
	-> Outcome<()>
{
	let _ = write!(out, "/// {}\npub static {}: [{}; {}] = [\n", doc, name, typ, vals.len());
	for chunk in vals.chunks(8) {
		let mut line = String::from("\t");
		for v in chunk {
			let variant = match list.get(*v as usize) {
				Some((_, rust, _)) => *rust,
				None => return Err(err!(
					"The class index {} is out of range for {}.", v, typ; Bug, Index)),
			};
			let _ = write!(line, "{}::{}, ", typ, variant);
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
	Ok(())
}

/// Emits an array of `char` literals, eight values to the line.
fn emit_chars(out: &mut String, name: &str, doc: &str, vals: &[u32]) {
	let _ = write!(out, "/// {}\npub static {}: [char; {}] = [\n", doc, name, vals.len());
	for chunk in vals.chunks(8) {
		let mut line = String::from("\t");
		for v in chunk {
			let _ = write!(line, "'\\u{{{:X}}}', ", v);
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
}

/// Emits the table module root.
fn emit_mod() -> String {
	let mut out = header("The generated Unicode tables.");
	out.push_str(
"//! The tables are partitions of the code point space: a sorted array of run start code points,
//! and a parallel array of the value each run takes. A lookup is a binary search of the starts,
//! about eleven steps for the largest table. This representation was chosen over a staged trie
//! because the tables are committed source: a partition of the Line_Break property is a few
//! thousand entries where a two level trie is tens of thousands, and the difference in lookup cost
//! is invisible next to the rest of a segmentation pass.

pub mod bidi;
pub mod cat;
pub mod lb;
pub mod norm;
pub mod prop;
pub mod seg;

/// The Unicode Character Database version every table in this module was generated from.
pub const UCD_VERSION: &str = \"");
	out.push_str(UCD_VERSION);
	out.push_str("\";\n");
	out
}

/// Emits one property enum and its `Partitioned` implementation.
fn emit_enum(
	out:	&mut String,
	doc:	&str,
	typ:	&str,
	list:	&[(&str, &str, &str)],
	dflt:	&str,
	table:	Option<(&str, &str)>,
) {
	let _ = write!(out, "/// {}\n", doc);
	out.push_str("#[repr(u8)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n");
	let _ = write!(out, "pub enum {} {{\n", typ);
	for (ucd, rust, vdoc) in list {
		if ucd == rust {
			let _ = write!(out, "\t/// {}\n\t{},\n", vdoc, rust);
		} else {
			let _ = write!(out, "\t/// {} The UCD name is `{}`.\n\t{},\n", vdoc, ucd, rust);
		}
	}
	out.push_str("}\n\n");

	if let Some((module, prefix)) = table {
		let _ = write!(out,
"impl Partitioned for {typ} {{

	const DEFAULT: Self = Self::{dflt};

	fn table() -> (&'static [u32], &'static [Self]) {{
		(&super::{module}::{prefix}_STARTS, &super::{module}::{prefix}_VALS)
	}}
}}

");
	}
}

/// Emits the property enums.
fn emit_prop(cats: &Cats) -> String {

	let mut out = header("The Unicode character property enums.");
	out.push_str("\nuse crate::unicode::lookup::Partitioned;\n\n");

	emit_enum(&mut out,
		"The Line_Break property of UAX #14, as the UCD gives it, before any tailoring.",
		"LineBreakClass", LB_CLASSES, "XX", Some(("lb", "LB")));

	emit_enum(&mut out,
		"The Grapheme_Cluster_Break property of UAX #29.",
		"GraphemeClass", GCB_CLASSES, "Other", Some(("seg", "GCB")));

	emit_enum(&mut out,
		"The Word_Break property of UAX #29.",
		"WordClass", WB_CLASSES, "Other", Some(("seg", "WB")));

	emit_enum(&mut out,
		"The Bidi_Class property of UAX #9.",
		"BidiClass", BIDI_CLASSES, "L", Some(("bidi", "BIDI")));

	emit_enum(&mut out,
		"The Indic_Conjunct_Break property of UAX #44, which grapheme rule GB9c consults.",
		"ConjunctBreak", INCB_CLASSES, "None", None);

	out.push_str(
"/// The Bidi_Paired_Bracket_Type property of UAX #9.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BracketKind {
	/// Not a paired bracket.
	None,
	/// An opening bracket.
	Open,
	/// A closing bracket.
	Close,
}

");
	emit_cat_enums(&mut out, cats);
	out
}

/// Emits the normalisation tables.
fn emit_norm(ucd: &Ucd) -> Outcome<String> {

	let mut out = header("Tables for the normalisation of UAX #15.");

	let (starts, vals) = partition(&ucd.ccc);
	emit_u32(&mut out, "CCC_STARTS", "Start code points of the canonical combining class runs.", &starts);
	emit_u8(&mut out, "CCC_VALS", "The canonical combining class of each run.", &vals);

	// Canonical decompositions, flattened into a pool of characters.
	let mut keys	= Vec::new();
	let mut offs	= Vec::new();
	let mut pool	= Vec::new();
	for (cp, seq) in &ucd.canon {
		keys.push(*cp);
		offs.push(pool.len() as u16);
		pool.extend_from_slice(seq);
	}
	offs.push(pool.len() as u16);
	emit_u32(&mut out, "CANON_KEYS",
		"Code points with a canonical decomposition, sorted.", &keys);
	let offs32: Vec<u32> = offs.iter().map(|o| *o as u32).collect();
	emit_u32(&mut out, "CANON_OFFS",
		"Offsets into `CANON_POOL`, one per key plus a final bound.", &offs32);
	emit_chars(&mut out, "CANON_POOL",
		"The canonical decompositions, concatenated.", &pool);

	// Compatibility decompositions. Only the code points whose mapping is tagged appear here; a
	// caller wanting the full compatibility decomposition falls back on the canonical table.
	let mut keys	= Vec::new();
	let mut offs	= Vec::new();
	let mut pool	= Vec::new();
	for (cp, seq) in &ucd.compat {
		keys.push(*cp);
		offs.push(pool.len() as u16);
		pool.extend_from_slice(seq);
	}
	offs.push(pool.len() as u16);
	emit_u32(&mut out, "COMPAT_KEYS",
		"Code points with a compatibility decomposition, sorted.", &keys);
	let offs32: Vec<u32> = offs.iter().map(|o| *o as u32).collect();
	emit_u32(&mut out, "COMPAT_OFFS",
		"Offsets into `COMPAT_POOL`, one per key plus a final bound.", &offs32);
	emit_chars(&mut out, "COMPAT_POOL",
		"The compatibility decompositions, concatenated.", &pool);

	// Primary composites: a canonical decomposition of exactly two characters whose code point is
	// not excluded from composition.
	let mut comp: Vec<(u64, u32)> = Vec::new();
	for (cp, seq) in &ucd.canon {
		if seq.len() != 2 || ucd.excl.contains(cp) {
			continue;
		}
		let key = ((seq[0] as u64) << 32) | (seq[1] as u64);
		comp.push((key, *cp));
	}
	comp.sort();
	let mut hi = Vec::new();
	let mut lo = Vec::new();
	let mut val = Vec::new();
	for (key, cp) in &comp {
		hi.push((*key >> 32) as u32);
		lo.push((*key & 0xFFFF_FFFF) as u32);
		val.push(*cp);
	}
	emit_u32(&mut out, "COMPOSE_FIRST",
		"The first character of each primary composite pair, sorted with `COMPOSE_SECOND`.", &hi);
	emit_u32(&mut out, "COMPOSE_SECOND",
		"The second character of each primary composite pair.", &lo);
	emit_chars(&mut out, "COMPOSE_VALS",
		"The composite each pair yields.", &val);

	Ok(out)
}

/// Emits the line breaking tables.
fn emit_lb(ucd: &Ucd) -> String {

	let mut out = header("Tables for the line breaking of UAX #14.");
	out.push_str("\nuse crate::unicode::prop::LineBreakClass;\n\n");

	let (starts, vals) = partition(&ucd.lb);
	emit_u32(&mut out, "LB_STARTS", "Start code points of the Line_Break class runs.", &starts);
	let _ = emit_enum_vals(&mut out, "LB_VALS", "The Line_Break class of each run.",
		"LineBreakClass", LB_CLASSES, &vals);

	let (starts, vals) = partition(&ucd.lb_flags);
	emit_u32(&mut out, "LB_FLAG_STARTS", "Start code points of the line breaking flag runs.", &starts);
	emit_u8(&mut out, "LB_FLAG_VALS",
		"The line breaking flags of each run. See the `flag` constants in `unicode::linebreak`.",
		&vals);

	out
}

/// Emits the segmentation tables.
fn emit_seg(ucd: &Ucd) -> String {

	let mut out = header("Tables for the segmentation of UAX #29.");
	out.push_str("\nuse crate::unicode::prop::{\n\tGraphemeClass,\n\tWordClass,\n};\n\n");

	let (starts, vals) = partition(&ucd.gcb);
	emit_u32(&mut out, "GCB_STARTS",
		"Start code points of the Grapheme_Cluster_Break class runs.", &starts);
	let _ = emit_enum_vals(&mut out, "GCB_VALS", "The Grapheme_Cluster_Break class of each run.",
		"GraphemeClass", GCB_CLASSES, &vals);

	let (starts, vals) = partition(&ucd.wb);
	emit_u32(&mut out, "WB_STARTS", "Start code points of the Word_Break class runs.", &starts);
	let _ = emit_enum_vals(&mut out, "WB_VALS", "The Word_Break class of each run.",
		"WordClass", WB_CLASSES, &vals);

	let (starts, vals) = partition(&ucd.seg_flags);
	emit_u32(&mut out, "SEG_FLAG_STARTS",
		"Start code points of the segmentation flag runs.", &starts);
	emit_u8(&mut out, "SEG_FLAG_VALS",
		"The segmentation flags of each run: bit 0 is Extended_Pictographic, bits 1 and 2 are the \
		 Indic_Conjunct_Break value.",
		&vals);

	out
}

/// Emits the bidirectional tables.
fn emit_bidi(ucd: &Ucd) -> String {

	let mut out = header("Tables for the bidirectional algorithm of UAX #9.");
	out.push_str("\nuse crate::unicode::prop::BidiClass;\n\n");

	let (starts, vals) = partition(&ucd.bidi);
	emit_u32(&mut out, "BIDI_STARTS", "Start code points of the Bidi_Class runs.", &starts);
	let _ = emit_enum_vals(&mut out, "BIDI_VALS", "The Bidi_Class of each run.",
		"BidiClass", BIDI_CLASSES, &vals);

	let keys:	Vec<u32> = ucd.brackets.keys().copied().collect();
	let pairs:	Vec<u32> = ucd.brackets.values().map(|(p, _)| *p).collect();
	let kinds:	Vec<u8> = ucd.brackets.values().map(|(_, k)| *k).collect();
	emit_u32(&mut out, "BRACKET_KEYS", "The paired bracket code points, sorted.", &keys);
	emit_chars(&mut out, "BRACKET_PAIRS", "The bracket each key pairs with.", &pairs);
	emit_u8(&mut out, "BRACKET_KINDS", "The kind of each bracket: 0 opening, 1 closing.", &kinds);

	out
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Character classes: General_Category, Script, Script_Extensions, binary properties         │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// The General_Category values in the order the generated enum takes them. The groups (`L`, `LC`,
/// ...) are not variants; they become bit masks over this order.
const GC_CODES: &[&str] = &[
	"Lu", "Ll", "Lt", "Lm", "Lo",
	"Mn", "Mc", "Me",
	"Nd", "Nl", "No",
	"Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po",
	"Sm", "Sc", "Sk", "So",
	"Zs", "Zl", "Zp",
	"Cc", "Cf", "Cs", "Co", "Cn",
];

/// Normalises a property name or value alias by UAX44-LM3, less the `is` prefix, which the
/// run time lookup tries both with and without: ASCII lower case, and no spaces, underscores
/// or hyphens.
fn loose(name: &str) -> String {
	name.chars()
		.filter(|c| *c != ' ' && *c != '_' && *c != '-')
		.map(|c| c.to_ascii_lowercase())
		.collect()
}

/// Sorts and merges a list of inclusive code point ranges, joining those that touch.
fn merge(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
	ranges.sort();
	let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
	for (lo, hi) in ranges {
		match out.last_mut() {
			Some(last) if lo <= last.1.saturating_add(1) => {
				if hi > last.1 {
					last.1 = hi;
				}
			},
			_ => out.push((lo, hi)),
		}
	}
	out
}

/// Inserts an alias, refusing one that already names something else.
fn alias<T: Copy + PartialEq + std::fmt::Debug>(
	map:	&mut BTreeMap<String, T>,
	name:	&str,
	val:	T,
	what:	&str,
)
	-> Outcome<()>
{
	let key = loose(name);
	match map.get(&key) {
		Some(old) if *old != val => Err(err!(
			"The {} alias {:?} names both {:?} and {:?}.", what, name, old, val;
			Invalid, Input, Mismatch)),
		_ => {
			map.insert(key, val);
			Ok(())
		},
	}
}

/// The character class properties, parsed.
struct Cats {
	gc:			Vec<u8>,						// GC_CODES index per code point
	gc_long:	Vec<String>,					// long name per GC_CODES entry
	gc_alias:	BTreeMap<String, u32>,			// loose alias to category mask
	scripts:	Vec<(String, String)>,			// (short, long), enum order
	sc_alias:	BTreeMap<String, u8>,			// loose alias to script index
	sc:			Vec<u8>,						// script index per code point
	scx:		Vec<u16>,						// 0, or 1 + index into scx_sets
	scx_sets:	Vec<Vec<u8>>,
	bins:		BTreeMap<String, Vec<(u32, u32)>>,	// long name to merged ranges
	bin_alias:	BTreeMap<String, u8>,			// loose alias to index in `bins`
	gcb_alias:	BTreeMap<String, u8>,			// loose alias to GCB_CLASSES index
	wb_alias:	BTreeMap<String, u8>,			// loose alias to WB_CLASSES index
	sbs:		Vec<(String, String)>,			// Sentence_Break (short, long), enum order
	sb_alias:	BTreeMap<String, u8>,
	sb:			Vec<u8>,						// Sentence_Break index per code point
}

impl Cats {

	fn parse(src: &BTreeMap<&str, String>, ucd: &Ucd) -> Outcome<Self> {

		let get = |path: &str| -> Outcome<&String> {
			match src.get(path) {
				Some(text) => Ok(text),
				None => Err(err!("The file {} was not downloaded.", path; Missing, Input)),
			}
		};
		let pva = res!(get("ucd/PropertyValueAliases.txt"));

		// General_Category names and groups.
		let mut gc_long		= vec![String::new(); GC_CODES.len()];
		let mut gc_alias	= BTreeMap::new();
		for line in pva.lines() {
			let (data, note) = match line.split_once('#') {
				Some((d, n))	=> (d, n),
				None			=> (line, ""),
			};
			let f: Vec<&str> = data.split(';').map(|x| x.trim()).collect();
			if f.len() < 3 || f[0] != "gc" {
				continue;
			}
			let mask = if note.contains('|') {
				let mut m = 0u32;
				for part in note.split('|') {
					let code = part.trim();
					match GC_CODES.iter().position(|c| *c == code) {
						Some(i)	=> m |= 1 << i,
						None	=> return Err(err!(
							"The category group {} names an unknown member {:?}.", f[1], code;
							Invalid, Input)),
					}
				}
				m
			} else {
				match GC_CODES.iter().position(|c| *c == f[1]) {
					Some(i) => {
						gc_long[i] = f[2].to_string();
						1 << i
					},
					None => return Err(err!(
						"The general category {:?} is not known to the generator.", f[1];
						Invalid, Input, Mismatch)),
				}
			};
			for name in &f[1..] {
				res!(alias(&mut gc_alias, name, mask, "General_Category"));
			}
		}
		for (i, l) in gc_long.iter().enumerate() {
			if l.is_empty() {
				return Err(err!(
					"PropertyValueAliases.txt gives no long name for {}.", GC_CODES[i];
					Missing, Input));
			}
		}

		let mut gc = vec![0u8; NUM_CP];
		for (c, code) in ucd.gc.iter().enumerate() {
			let code = String::from_utf8_lossy(code);
			match GC_CODES.iter().position(|x| *x == code) {
				Some(i)	=> gc[c] = i as u8,
				None	=> return Err(err!(
					"The general category {} at U+{:04X} is not known.", code, c; Invalid, Input)),
			}
		}

		// Script names.
		let mut scripts		= Vec::new();
		let mut sc_alias	= BTreeMap::new();
		for line in pva.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 3 || f[0] != "sc" {
				continue;
			}
			let i = scripts.len();
			if i > 255 {
				return Err(err!("There are more scripts than a byte can index."; Excessive));
			}
			scripts.push((f[1].to_string(), f[2].to_string()));
			for name in &f[1..] {
				res!(alias(&mut sc_alias, name, i as u8, "Script"));
			}
		}
		let unknown = match scripts.iter().position(|(s, _)| s == "Zzzz") {
			Some(i)	=> i as u8,
			None	=> return Err(err!("No script Zzzz (Unknown) was listed."; Missing, Input)),
		};
		let by_long = |name: &str| -> Outcome<u8> {
			match scripts.iter().position(|(_, l)| l == name) {
				Some(i)	=> Ok(i as u8),
				None	=> Err(err!("The script {:?} is not in PropertyValueAliases.txt.", name;
					Invalid, Input, Mismatch)),
			}
		};
		let by_short = |name: &str| -> Outcome<u8> {
			match scripts.iter().position(|(s, _)| s == name) {
				Some(i)	=> Ok(i as u8),
				None	=> Err(err!("The script {:?} is not in PropertyValueAliases.txt.", name;
					Invalid, Input, Mismatch)),
			}
		};

		let mut sc = vec![unknown; NUM_CP];
		for line in res!(get("ucd/Scripts.txt")).lines() {
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi)	= res!(range(f[0]));
			let v			= res!(by_long(f[1]));
			for c in lo..=hi {
				sc[c as usize] = v;
			}
		}

		let mut scx			= vec![0u16; NUM_CP];
		let mut scx_sets: Vec<Vec<u8>> = Vec::new();
		for line in res!(get("ucd/ScriptExtensions.txt")).lines() {
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi) = res!(range(f[0]));
			let mut set = Vec::new();
			for s in f[1].split_whitespace() {
				set.push(res!(by_short(s)));
			}
			set.sort();
			set.dedup();
			let id = match scx_sets.iter().position(|x| *x == set) {
				Some(i)	=> i,
				None	=> {
					scx_sets.push(set);
					scx_sets.len() - 1
				},
			};
			for c in lo..=hi {
				scx[c as usize] = (id + 1) as u16;
			}
		}

		// Binary properties.  The contributory `Other_*` properties are left out: UAX #44 says
		// they exist to derive others and are not for use in their own right.
		let mut raw: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
		for path in [
			"ucd/PropList.txt",
			"ucd/DerivedCoreProperties.txt",
			"ucd/emoji/emoji-data.txt",
		] {
			for line in res!(get(path)).lines() {
				let f = match fields(line) {
					Some(f) => f,
					None	=> continue,
				};
				if f.len() != 2 || f[1].starts_with("Other_") {
					continue;
				}
				let r = res!(range(f[0]));
				raw.entry(f[1].to_string()).or_default().push(r);
			}
		}
		let bins: BTreeMap<String, Vec<(u32, u32)>> =
			raw.into_iter().map(|(k, v)| (k, merge(v))).collect();

		let mut bin_alias = BTreeMap::new();
		for (i, name) in bins.keys().enumerate() {
			res!(alias(&mut bin_alias, name, i as u8, "binary property"));
		}
		let mut in_binary = false;
		for line in res!(get("ucd/PropertyAliases.txt")).lines() {
			if line.starts_with('#') {
				if line.contains("Binary Properties") {
					in_binary = true;
				} else if line.contains("Properties") {
					in_binary = false;
				}
				continue;
			}
			if !in_binary {
				continue;
			}
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 2 {
				continue;
			}
			if let Some(i) = bins.keys().position(|k| k == f[1]) {
				for name in &f {
					res!(alias(&mut bin_alias, name, i as u8, "binary property"));
				}
			}
		}

		// The segmentation break properties, by the aliases a `\p{gcb=..}` may use.  A value
		// no character takes (the retired emoji Word_Break values) is left out.
		let mut gcb_alias	= BTreeMap::new();
		let mut wb_alias	= BTreeMap::new();
		let mut sbs			= Vec::new();
		let mut sb_alias	= BTreeMap::new();
		for line in pva.lines() {
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 3 {
				continue;
			}
			let (list, map) = match f[0] {
				"GCB"	=> (GCB_CLASSES, &mut gcb_alias),
				"WB"	=> (WB_CLASSES, &mut wb_alias),
				"SB"	=> {
					let i = sbs.len() as u8;
					sbs.push((f[1].to_string(), f[2].to_string()));
					for name in &f[1..] {
						res!(alias(&mut sb_alias, name, i, "Sentence_Break"));
					}
					continue;
				},
				_		=> continue,
			};
			if let Some(i) = list.iter().position(|(u, _, _)| *u == f[2] || *u == f[1]) {
				for name in &f[1..] {
					res!(alias(map, name, i as u8, f[0]));
				}
			}
		}
		let other = match sbs.iter().position(|(_, l)| l == "Other") {
			Some(i)	=> i as u8,
			None	=> return Err(err!("No Sentence_Break value Other was listed."; Missing, Input)),
		};
		let mut sb = vec![other; NUM_CP];
		for line in res!(get("ucd/auxiliary/SentenceBreakProperty.txt")).lines() {
			let f = match fields(line) {
				Some(f) => f,
				None	=> continue,
			};
			if f.len() < 2 {
				continue;
			}
			let (lo, hi) = res!(range(f[0]));
			let v = match sbs.iter().position(|(_, l)| l == f[1]) {
				Some(i)	=> i as u8,
				None	=> return Err(err!("The Sentence_Break value {:?} is not known.", f[1];
					Invalid, Input, Mismatch)),
			};
			for c in lo..=hi {
				sb[c as usize] = v;
			}
		}

		Ok(Self {
			gc, gc_long, gc_alias, scripts, sc_alias, sc, scx, scx_sets, bins, bin_alias,
			gcb_alias, wb_alias, sbs, sb_alias, sb,
		})
	}

	/// The Rust variant name of a script: its long name without underscores.
	fn variant(&self, i: usize) -> String {
		match self.scripts.get(i) {
			Some((_, long))	=> long.replace('_', ""),
			None			=> String::from("Unknown"),
		}
	}
}

/// Emits a `u16` array, twelve values to the line.
fn emit_u16(out: &mut String, name: &str, doc: &str, vals: &[u16]) {
	let _ = write!(out, "/// {}\npub static {}: [u16; {}] = [\n", doc, name, vals.len());
	for chunk in vals.chunks(12) {
		let mut line = String::from("\t");
		for v in chunk {
			let _ = write!(line, "{}, ", v);
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
}

/// Emits an array of variants of an enum whose variant names are given, eight to the line.
fn emit_variants(
	out:	&mut String,
	name:	&str,
	doc:	&str,
	typ:	&str,
	short:	&str,
	names:	&[String],
	vals:	&[u8],
)
	-> Outcome<()>
{
	let _ = write!(out, "/// {}\npub static {}: [{}; {}] = [\n", doc, name, typ, vals.len());
	for chunk in vals.chunks(8) {
		let mut line = String::from("\t");
		for v in chunk {
			match names.get(*v as usize) {
				Some(n)	=> { let _ = write!(line, "{}::{}, ", short, n); },
				None	=> return Err(err!(
					"The index {} is out of range for {}.", v, typ; Bug, Index)),
			}
		}
		let _ = write!(out, "{}\n", line.trim_end());
	}
	out.push_str("];\n\n");
	Ok(())
}

/// Emits the General_Category and Script enums, into the property enum file.
fn emit_cat_enums(out: &mut String, cats: &Cats) {

	out.push_str(
"/// The General_Category property of UAX #44, by its two letter abbreviation.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeneralCategory {
");
	for code in GC_CODES {
		let _ = write!(out, "\t{},\n", code);
	}
	out.push_str("}\n\n");
	out.push_str(
"impl Partitioned for GeneralCategory {

	const DEFAULT: Self = Self::Cn;

	fn table() -> (&'static [u32], &'static [Self]) {
		(&super::cat::GC_STARTS, &super::cat::GC_VALS)
	}
}

impl GeneralCategory {

	/// The long UCD name, `Uppercase_Letter` for `Lu`.
	pub fn name(self) -> &'static str {
		match self {
");
	for (i, code) in GC_CODES.iter().enumerate() {
		let long = match cats.gc_long.get(i) {
			Some(l)	=> l.as_str(),
			None	=> "",
		};
		let _ = write!(out, "\t\t\tSelf::{}\t=> \"{}\",\n", code, long);
	}
	out.push_str("\t\t}\n\t}\n\n\t/// The two letter abbreviation.\n\tpub fn abbr(self) -> &'static str {\n\t\tmatch self {\n");
	for code in GC_CODES {
		let _ = write!(out, "\t\t\tSelf::{}\t=> \"{}\",\n", code, code);
	}
	out.push_str("\t\t}\n\t}\n}\n\n");

	out.push_str(
"/// The Script property of UAX #24. A character's Script_Extensions, the wider set of scripts it
/// is used with, are in `unicode::property`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
");
	for i in 0..cats.scripts.len() {
		let _ = write!(out, "\t{},\n", cats.variant(i));
	}
	out.push_str("}\n\n");
	out.push_str(
"impl Partitioned for Script {

	const DEFAULT: Self = Self::Unknown;

	fn table() -> (&'static [u32], &'static [Self]) {
		(&super::cat::SC_STARTS, &super::cat::SC_VALS)
	}
}

impl Script {

	/// The long UCD name, `Old_Italic` for instance.
	pub fn name(self) -> &'static str {
		match self {
");
	for (i, (_, long)) in cats.scripts.iter().enumerate() {
		let _ = write!(out, "\t\t\tSelf::{}\t=> \"{}\",\n", cats.variant(i), long);
	}
	out.push_str("\t\t}\n\t}\n\n\t/// The four letter ISO 15924 code, `Ital` for Old_Italic.\n\tpub fn code(self) -> &'static str {\n\t\tmatch self {\n");
	for (i, (short, _)) in cats.scripts.iter().enumerate() {
		let _ = write!(out, "\t\t\tSelf::{}\t=> \"{}\",\n", cats.variant(i), short);
	}
	out.push_str("\t\t}\n\t}\n}\n\n");

	out.push_str(
"/// The Sentence_Break property of UAX #29.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SentenceClass {
");
	for (_, long) in &cats.sbs {
		let _ = write!(out, "\t{},\n", long.replace('_', ""));
	}
	out.push_str(
"}

impl Partitioned for SentenceClass {

	const DEFAULT: Self = Self::Other;

	fn table() -> (&'static [u32], &'static [Self]) {
		(&super::cat::SB_STARTS, &super::cat::SB_VALS)
	}
}
");
}

/// Emits the character class tables.
fn emit_cat(cats: &Cats) -> Outcome<String> {

	let mut out = header(
		"Tables for the character classes of UAX #44 and UAX #24: General_Category, Script, \
		 Script_Extensions and the binary properties.");
	// Short aliases keep the long value arrays readable and the file a third smaller.
	out.push_str("\nuse crate::unicode::prop::{\n\tGeneralCategory,\n\tGeneralCategory as G,\n\tGraphemeClass,\n\tScript,\n\tScript as S,\n\tSentenceClass,\n\tWordClass,\n};\n\n");

	let gc_names: Vec<String> = GC_CODES.iter().map(|c| c.to_string()).collect();
	let (starts, vals) = partition(&cats.gc);
	emit_u32(&mut out, "GC_STARTS", "Start code points of the General_Category runs.", &starts);
	res!(emit_variants(&mut out, "GC_VALS", "The General_Category of each run.",
		"GeneralCategory", "G", &gc_names, &vals));

	let _ = write!(out,
		"/// Every General_Category alias, loosely normalised and sorted, with the categories it \
		 names as a mask over the `GeneralCategory` variants.\npub static GC_NAMES: [(&str, u32); {}] = [\n",
		cats.gc_alias.len());
	for (k, v) in &cats.gc_alias {
		let _ = write!(out, "\t(\"{}\", 0x{:08X}),\n", k, v);
	}
	out.push_str("];\n\n");

	let sc_names: Vec<String> = (0..cats.scripts.len()).map(|i| cats.variant(i)).collect();
	let (starts, vals) = partition(&cats.sc);
	emit_u32(&mut out, "SC_STARTS", "Start code points of the Script runs.", &starts);
	res!(emit_variants(&mut out, "SC_VALS", "The Script of each run.", "Script", "S", &sc_names, &vals));

	let _ = write!(out,
		"/// Every Script alias, loosely normalised and sorted.\npub static SCRIPT_NAMES: [(&str, Script); {}] = [\n",
		cats.sc_alias.len());
	for (k, v) in &cats.sc_alias {
		let _ = write!(out, "\t(\"{}\", S::{}),\n", k, cats.variant(*v as usize));
	}
	out.push_str("];\n\n");

	// Script_Extensions as a partition of set numbers, zero where a character's extensions are
	// just its script.
	let mut starts	= Vec::new();
	let mut runs	= Vec::new();
	let mut prev	= None;
	for (c, v) in cats.scx.iter().enumerate() {
		if prev != Some(*v) {
			starts.push(c as u32);
			runs.push(*v);
			prev = Some(*v);
		}
	}
	emit_u32(&mut out, "SCX_STARTS", "Start code points of the Script_Extensions runs.", &starts);
	emit_u16(&mut out, "SCX_VALS",
		"The Script_Extensions set of each run: zero for the character's own script alone, \
		 otherwise one more than the set's index in `SCX_OFFS`.", &runs);
	let mut offs: Vec<u16> = vec![0];
	let mut pool: Vec<u8> = Vec::new();
	for set in &cats.scx_sets {
		pool.extend_from_slice(set);
		offs.push(pool.len() as u16);
	}
	emit_u16(&mut out, "SCX_OFFS",
		"Where each Script_Extensions set begins in `SCX_POOL`, with the end as the last entry.",
		&offs);
	res!(emit_variants(&mut out, "SCX_POOL", "The scripts of every Script_Extensions set, in turn.",
		"Script", "S", &sc_names, &pool));

	// Binary properties as inclusive range pairs.
	let mut flat: Vec<u32>	= Vec::new();
	let mut boffs: Vec<u32>	= vec![0];
	let mut longs			= String::new();
	for (name, ranges) in &cats.bins {
		for (lo, hi) in ranges {
			flat.push(*lo);
			flat.push(*hi);
		}
		boffs.push(flat.len() as u32);
		let _ = write!(longs, "\t\"{}\",\n", name);
	}
	emit_u32(&mut out, "BIN_RANGES",
		"The binary properties as inclusive code point ranges, low then high, one property after \
		 another.", &flat);
	emit_u32(&mut out, "BIN_OFFS",
		"Where each binary property's ranges begin in `BIN_RANGES`, with the end as the last entry.",
		&boffs);
	let _ = write!(out, "/// The long name of each binary property.\npub static BIN_LONG: [&str; {}] = [\n{}];\n\n",
		cats.bins.len(), longs);
	for (i, name) in cats.bins.keys().enumerate() {
		let _ = write!(out, "pub const BIN_{}: u8 = {};\n", name.to_ascii_uppercase(), i);
	}
	out.push('\n');
	let _ = write!(out,
		"/// Every binary property alias, loosely normalised and sorted, with the property's index.\npub static BIN_NAMES: [(&str, u8); {}] = [\n",
		cats.bin_alias.len());
	for (k, v) in &cats.bin_alias {
		let _ = write!(out, "\t(\"{}\", {}),\n", k, v);
	}
	out.push_str("];\n\n");

	// The segmentation break properties' names; their values are in `seg`.
	for (name, typ, list, map) in [
		("GCB_NAMES", "GraphemeClass", GCB_CLASSES, &cats.gcb_alias),
		("WB_NAMES", "WordClass", WB_CLASSES, &cats.wb_alias),
	] {
		let _ = write!(out,
			"/// Every {} alias, loosely normalised and sorted.\npub static {}: [(&str, {}); {}] = [\n",
			typ, name, typ, map.len());
		for (k, v) in map {
			let variant = match list.get(*v as usize) {
				Some((_, rust, _))	=> *rust,
				None				=> return Err(err!("Index {} is out of range for {}.", v, typ;
					Bug, Index)),
			};
			let _ = write!(out, "\t(\"{}\", {}::{}),\n", k, typ, variant);
		}
		out.push_str("];\n\n");
	}

	let sb_names: Vec<String> = cats.sbs.iter().map(|(_, l)| l.replace('_', "")).collect();
	let (starts, vals) = partition(&cats.sb);
	emit_u32(&mut out, "SB_STARTS", "Start code points of the Sentence_Break runs.", &starts);
	res!(emit_variants(&mut out, "SB_VALS", "The Sentence_Break of each run.", "SentenceClass",
		"SentenceClass", &sb_names, &vals));
	let _ = write!(out,
		"/// Every Sentence_Break alias, loosely normalised and sorted.\npub static SB_NAMES: [(&str, SentenceClass); {}] = [\n",
		cats.sb_alias.len());
	for (k, v) in &cats.sb_alias {
		let n = sb_names.get(*v as usize).cloned().unwrap_or_default();
		let _ = write!(out, "\t(\"{}\", SentenceClass::{}),\n", k, n);
	}
	out.push_str("];\n");

	Ok(out)
}
