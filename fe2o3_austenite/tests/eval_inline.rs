//! U6a: inline flow and fonts against the `typst` 0.15.1 oracle, on synthetic fixtures only.
//!
//! - `lorem` gives Typst's words.
//! - Shaped widths: a corpus of words in Libertinus Serif, four sizes and four styles, measured by
//!   Typst's `measure` and by Austenite's shaper, agree to 0.01pt.
//! - Line breaking: every paragraph of `tests/fixtures/eval/linebreak/` breaks into the same lines,
//!   read from Typst's PDF by `pdftotext -bbox-layout`.
//! - Hyphenation: English, German and French paragraphs set so narrow that most lines end in a hyphen
//!   break where Typst breaks them (`tests/fixtures/eval/hyphenation/`).
//! - Overhang: a justified line's last punctuation mark hangs into the margin as far as Typst hangs it,
//!   and never moves a break.
//! - The paragraph rule: `layout_par` keeps nothing, takes nothing but its inputs, tags a located element
//!   where it stands, and shares one face between every line.
//! - The shaped-run cache holds its byte budget and evicts the least recently used.
//! - A family no font declares warns as Typst warns, with kind `missing_font`.
//! - The language and region a chain sets, which the document's language is read from.
//!
//! Every `typst` run is capped by `systemd-run` and ignores system fonts, so both sides shape from the
//! same embedded faces. A missing `typst` fails the suite unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::diag::DiagnosticKind;
use oxedyne_fe2o3_austenite::eval::content::ElemKind;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	Pair,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use oxedyne_fe2o3_austenite::eval::value::{
	Length,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::inline::{
	LineBox,
	Linebreaks,
	ParSituation,
};
use oxedyne_fe2o3_austenite::flow::par::{
	layout_par,
	layout_with,
	ConfigBase,
};
use oxedyne_fe2o3_austenite::flow::text::locale;
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::fonts::{
	FaceVariant,
	FontBook,
};
use oxedyne_fe2o3_austenite::ir::{
	LeafKind,
	Node,
	Sp,
};
use oxedyne_fe2o3_austenite::syntax::Span;
use oxedyne_fe2o3_font::shape::{
	Dir as ShapeDir,
	ShapeSpec,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;
use std::sync::Arc;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ORACLE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// The typst binary to run, or `None` when absent and explicitly skipped.
fn oracle() -> Outcome<Option<String>> {
	let bin = std::env::var("TYPST").unwrap_or_else(|_| "typst".to_string());
	match Command::new(&bin).arg("--version").output() {
		Ok(o) if o.status.success() => {
			let v = String::from_utf8_lossy(&o.stdout).to_string();
			if !v.starts_with("typst 0.15.") {
				return Err(err!("The oracle is {}, not typst 0.15.x.", v.trim(); Test, Invalid));
			}
			Ok(Some(bin))
		}
		_ => {
			if std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") {
				Ok(None)
			} else {
				Err(err!("typst 0.15.1 is not on the PATH; set EVAL_ORACLE_SKIP=1 to skip on purpose."; Test, Missing))
			}
		}
	}
}

/// Runs typst under the memory cap, in `cwd`; its standard output, or an error with its standard error.
fn typst(bin: &str, args: &[&str], cwd: &Path) -> Outcome<String> {
	let capped = Command::new("systemd-run").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
	let mut cmd = if capped {
		let mut c = Command::new("systemd-run");
		c.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", bin]);
		c
	} else {
		Command::new(bin)
	};
	cmd.args(args).current_dir(cwd);
	let out = res!(cmd.output().map_err(|e| err!(e, "Could not run typst."; IO)));
	if !out.status.success() {
		return Err(err!("typst {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr); Test, Invalid));
	}
	Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// A scratch directory under the test target, never `/tmp`.
fn work(name: &str) -> Outcome<PathBuf> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("u6a").join(name);
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!(e, "Could not make {:?}.", dir; IO)));
	Ok(dir)
}

fn fixtures(area: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("eval").join(area)
}

/// The strings of a JSON array of strings, as `typst eval` prints one.
fn json_strings(s: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut cur: Option<String> = None;
	let mut chars = s.chars();
	while let Some(c) = chars.next() {
		match (&mut cur, c) {
			(None, '"')				=> cur = Some(String::new()),
			(Some(buf), '"')		=> {
				out.push(std::mem::take(buf));
				cur = None;
			}
			(Some(buf), '\\')		=> match chars.next() {
				Some('n')	=> buf.push('\n'),
				Some('u')	=> {
					let hex: String = chars.by_ref().take(4).collect();
					if let Ok(n) = u32::from_str_radix(&hex, 16) {
						if let Some(ch) = char::from_u32(n) {
							buf.push(ch);
						}
					}
				}
				Some(x)		=> buf.push(x),
				None		=> (),
			},
			(Some(buf), x)			=> buf.push(x),
			(None, _)				=> (),
		}
	}
	out
}

/// The numbers of a JSON array of numbers.
fn json_numbers(s: &str) -> Vec<f64> {
	s.trim().trim_start_matches('[').trim_end_matches(']').split(',')
		.filter_map(|x| x.trim().parse::<f64>().ok()).collect()
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOREM                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn lorem_gives_typsts_words() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let counts = [0usize, 1, 2, 3, 5, 8, 13, 30, 57, 100, 250, 600];
	let expr = fmt!("({})", counts.iter().map(|n| fmt!("lorem({})", n)).collect::<Vec<_>>().join(", "));
	let dir = res!(work("lorem"));
	let got = json_strings(&res!(typst(&bin, &["eval", &expr], &dir)));
	assert_eq!(got.len(), counts.len(), "typst gave {} strings", got.len());
	for (n, want) in counts.iter().zip(got.iter()) {
		let ours = oxedyne_fe2o3_austenite::eval::lib::text::lorem(*n);
		assert_eq!(&ours, want, "lorem({}) differs", n);
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WIDTHS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Words for the width corpus: ligatures, kerning pairs, figures, punctuation and ordinary prose. Every
/// character is one Libertinus Serif draws: a character it lacks falls back, in Typst, to New Computer
/// Modern, which Austenite does not embed (the precomposed `ﬁ` in italic, say).
fn width_words() -> Vec<String> {
	let mut w: Vec<String> = [
		"Hyphenation", "office", "affine", "fluffy", "shuffle", "efficient", "flight", "finish", "waffle",
		"AVAST", "Tomorrow", "Water", "Yawning", "LTA", "P.", "T,", "Vo", "WA", "Ty", "Te", "Yo", "fjord",
		"1234567890", "3.75", "(parenthesis)", "[bracket]", "quote\u{2019}s", "\u{201C}open\u{201D}",
		"em\u{2014}dash", "en\u{2013}dash", "naïve", "façade", "Übermäßig", "Æsop", "œuvre", "Q.E.D.",
		"http://example.org", "e.g.,", "i.e.;", "etc.:", "!?", "#42", "50%", "&amp", "x*y", "a/b",
	].iter().map(|s| s.to_string()).collect();
	let prose = oxedyne_fe2o3_austenite::eval::lib::text::lorem(160);
	for p in prose.split_whitespace() {
		if w.len() >= 125 {
			break;
		}
		if !w.iter().any(|x| x == p) {
			w.push(p.to_string());
		}
	}
	w
}

fn typst_string(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"'		=> out.push_str("\\\""),
			'\\'	=> out.push_str("\\\\"),
			c		=> out.push(c),
		}
	}
	out.push('"');
	out
}

const STYLES: &[(&str, &str)] = &[
	("normal", "regular"),
	("italic", "regular"),
	("normal", "bold"),
	("italic", "bold"),
];

const WIDTH_SIZES: &[f64] = &[9.0, 10.0, 11.0, 12.0];

fn text_chain(size: f64, style: &str, weight: &str) -> StyleChain {
	let mut s = Styles::new();
	let mut put = |name: &str, v: Value| {
		if let Some(id) = ElemKind::Text.field_id(name) {
			s.push(Style::Property(Property::new(ElemKind::Text, id, v, Span::detached())));
		}
	};
	put("size", Value::Length(Length::pt(size)));
	put("style", Value::str(style));
	put("weight", Value::str(weight));
	StyleChain::root().chain(&s)
}

#[test]
fn shaped_widths_match_typsts_measure() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let words = width_words();
	let dir = res!(work("widths"));
	let list = words.iter().map(|w| typst_string(w)).collect::<Vec<_>>().join(", ");
	let src = fmt!(
		"#let words = ({},)\n#context [#metadata({{\n  let out = ()\n  for size in (9pt, 10pt, 11pt, 12pt) {{\n    \
		for (style, weight) in ((\"normal\", \"regular\"), (\"italic\", \"regular\"), (\"normal\", \"bold\"), (\"italic\", \"bold\")) {{\n      \
		for w in words {{ out.push(measure(text(size: size, style: style, weight: weight, w)).width.pt()) }}\n    }}\n  }}\n  out\n}}) <m>]\n",
		list,
	);
	let path = dir.join("widths.typ");
	res!(std::fs::write(&path, src).map_err(|e| err!(e, "write"; IO)));
	let want = json_numbers(&res!(typst(&bin, &["query", "--ignore-system-fonts", "--field", "value", "--one", "widths.typ", "<m>"], &dir)));
	let expect = WIDTH_SIZES.len() * STYLES.len() * words.len();
	assert_eq!(want.len(), expect, "typst measured {} words, expected {}", want.len(), expect);

	let mut engine = Engine::new(World::new(dir.clone()));
	let mut i = 0;
	let mut bad: Vec<String> = Vec::new();
	for size in WIDTH_SIZES {
		for (style, weight) in STYLES {
			let chain = text_chain(*size, style, weight);
			for w in &words {
				let shaped = res!(oxedyne_fe2o3_austenite::flow::text::shape(&mut engine, w, &chain));
				let ours = shaped.dims().width.to_pt();
				if (ours - want[i]).abs() > 0.01 {
					bad.push(fmt!("{}pt {} {} {:?}: typst {:.4} austenite {:.4}", size, style, weight, w, want[i], ours));
				}
				i += 1;
			}
		}
	}
	assert!(bad.is_empty(), "{} of {} widths differ by more than 0.01pt:\n{}", bad.len(), expect, bad.join("\n"));
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LINE BREAKING                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The lines of a `pdftotext -bbox-layout` page: its words grouped by their vertical middles (a loose
/// justified line can come back from pdftotext as several), each line's words in order across.
fn bbox_lines(html: &str) -> Vec<String> {
	let attr = |tag: &str, key: &str| -> f64 {
		tag.split(&fmt!("{}=\"", key)).nth(1).and_then(|r| r.split('"').next())
			.and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
	};
	let mut words: Vec<(f64, f64, f64, String)> = Vec::new();	// (middle, height, x, text)
	let mut rest = html;
	while let Some(i) = rest.find("<word") {
		let tail = &rest[i..];
		let (gt, end) = match (tail.find('>'), tail.find("</word>")) {
			(Some(g), Some(e))	=> (g, e),
			_					=> break,
		};
		let tag = &tail[..gt];
		let (y0, y1) = (attr(tag, "yMin"), attr(tag, "yMax"));
		words.push(((y0 + y1) / 2.0, y1 - y0, attr(tag, "xMin"), unescape(&tail[gt + 1..end])));
		rest = &tail[end..];
	}
	words.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
	let mut lines: Vec<(f64, f64, Vec<(f64, String)>)> = Vec::new();
	for (mid, h, x, t) in words {
		match lines.last_mut() {
			Some((lm, lh, ws)) if (mid - *lm).abs() <= 0.35 * lh.max(h) => ws.push((x, t)),
			_ => lines.push((mid, h, vec![(x, t)])),
		}
	}
	lines.into_iter().map(|(_, _, mut ws)| {
		ws.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
		ws.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join(" ")
	}).collect()
}

fn unescape(s: &str) -> String {
	s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

/// A line's text for comparison: soft hyphens as hyphens, white space collapsed.
fn norm(s: &str) -> String {
	s.replace('\u{00AD}', "-").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A fixture's page width and `par` settings, read from its `#set page` and `#set par` lines, and its
/// source with those lines removed while the page and paragraph schemas are not yet built.
struct Fixture {
	width:	f64,
	base:	ConfigBase,
	source:	String,
}

fn read_fixture(text: &str) -> Outcome<Fixture> {
	let mut width = None;
	let mut base = ConfigBase { justify: false, linebreaks: None, first_line_indent: (0.0, false), hanging_indent: 0.0 };
	let par_schema = ElemKind::Par.field_id("justify").is_some();
	let mut kept = Vec::new();
	for line in text.lines() {
		if let Some(args) = line.strip_prefix("#set page(") {
			if let Some(w) = args.split("width:").nth(1).and_then(|r| r.trim().split("pt").next()) {
				width = w.trim().parse::<f64>().ok();
			}
			// The width goes to the line setter directly; a page rule would be refused in an inline body.
		} else if let Some(args) = line.strip_prefix("#set par(") {
			base.justify = args.contains("justify: true");
			if args.contains("\"simple\"") {
				base.linebreaks = Some(Linebreaks::Simple);
			} else if args.contains("\"optimized\"") {
				base.linebreaks = Some(Linebreaks::Optimized);
			}
			if par_schema {
				kept.push(line);
			}
		} else {
			kept.push(line);
		}
	}
	let width = match width {
		Some(w)	=> w,
		None	=> return Err(err!("A line-break fixture names no page width."; Test, Missing)),
	};
	Ok(Fixture { width, base, source: kept.join("\n") })
}

/// Austenite's lines for a fixture: evaluated, realised, and set as one paragraph at the page width.
fn austenite_lines(path: &Path, fx: &Fixture) -> Outcome<Vec<String>> {
	Ok(res!(austenite_boxes(path, fx)).into_iter().map(|l| l.text).collect())
}

/// As [`austenite_lines`], the committed line boxes themselves.
fn austenite_boxes(path: &Path, fx: &Fixture) -> Outcome<Vec<LineBox>> {
	let root = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
	let mut world = World::new(root);
	let id = res!(world.add_source(path.to_path_buf(), fx.source.clone()));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	// One paragraph's worth of content, realised as a paragraph's body is.
	let pairs = res!(realise(&mut engine, &module.content, &StyleChain::root(), RealiseMode::Inline));
	let trunk = StyleChain::trunk(pairs.iter().filter(|p| !p.is_tag()).map(|p| &p.styles));
	let region = Region::new(Sp::from_pt(fx.width), Sp::from_pt(10_000.0));
	layout_with(&mut engine, &pairs, &trunk, region, true, Some(ParSituation::First), &fx.base)
}

/// Typst's lines for a fixture, cached by its bytes under the test target.
fn typst_lines(bin: &str, path: &Path) -> Outcome<Vec<String>> {
	let bytes = res!(std::fs::read(path).map_err(|e| err!(e, "read {:?}", path; IO)));
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for b in &bytes {
		h ^= *b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	let dir = res!(work("linebreak"));
	let cached = dir.join(fmt!("{:016x}.lines", h));
	if let Ok(s) = std::fs::read_to_string(&cached) {
		return Ok(s.lines().map(|l| l.to_string()).collect());
	}
	let typ = dir.join(fmt!("{:016x}.typ", h));
	res!(std::fs::write(&typ, &bytes).map_err(|e| err!(e, "write"; IO)));
	let pdf = dir.join(fmt!("{:016x}.pdf", h));
	let (t, p) = (typ.to_string_lossy().to_string(), pdf.to_string_lossy().to_string());
	res!(typst(bin, &["compile", "--ignore-system-fonts", &t, &p], &dir));
	let out = res!(Command::new("pdftotext").args(["-bbox-layout", &p, "-"]).output()
		.map_err(|e| err!(e, "Could not run pdftotext."; IO)));
	let lines: Vec<String> = bbox_lines(&String::from_utf8_lossy(&out.stdout)).into_iter()
		.map(|l| norm(&l)).filter(|l| !l.is_empty()).collect();
	res!(std::fs::write(&cached, lines.join("\n")).map_err(|e| err!(e, "write"; IO)));
	Ok(lines)
}

/// What comparing a directory of fixtures against Typst found.
struct Compared {
	fixtures:	usize,
	lines:		usize,
	hyphens:	BTreeMap<String, usize>,	// lines of Typst's that end in a hyphen, by the fixture's language
	failures:	Vec<String>,
}

/// Every fixture of `dir` (those whose path contains `filter`, when given) set by both engines.
fn compare_fixtures(bin: &str, dir: &Path, filter: Option<&str>) -> Outcome<Compared> {
	let mut paths: Vec<PathBuf> = res!(std::fs::read_dir(dir).map_err(|e| err!(e, "read {:?}", dir; IO)))
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().map_or(false, |x| x == "typ"))
		.filter(|p| filter.map_or(true, |f| p.to_string_lossy().contains(f)))
		.collect();
	paths.sort();
	assert!(!paths.is_empty(), "no line-break fixtures found under {:?}", dir);
	let mut out = Compared { fixtures: paths.len(), lines: 0, hyphens: BTreeMap::new(), failures: Vec::new() };
	for path in &paths {
		let text = res!(std::fs::read_to_string(path).map_err(|e| err!(e, "read"; IO)));
		let fx = res!(read_fixture(&text));
		let want = res!(typst_lines(bin, path));
		let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
		let got: Vec<String> = match austenite_lines(path, &fx) {
			Ok(l)	=> l.iter().map(|x| norm(x)).collect(),
			Err(e)	=> {
				out.failures.push(fmt!("{}: austenite failed: {}", name, e));
				continue;
			}
		};
		out.lines += want.len();
		// The language is the letters after the fixture's first hyphen: `hy-fr2-90.typ` is `fr`.
		let lang: String = name.split('-').nth(1).unwrap_or("").chars().take_while(|c| c.is_alphabetic()).collect();
		*out.hyphens.entry(lang).or_insert(0) += want.iter().filter(|l| l.ends_with('-')).count();
		let n = want.len().max(got.len());
		let mut diffs = Vec::new();
		for i in 0..n {
			let (w, g) = (want.get(i).map(|s| s.as_str()).unwrap_or("<none>"), got.get(i).map(|s| s.as_str()).unwrap_or("<none>"));
			if w != g {
				diffs.push(fmt!("  line {}:\n    typst:     {}\n    austenite: {}", i + 1, w, g));
			}
		}
		if !diffs.is_empty() {
			out.failures.push(fmt!("{} ({} lines, {} differ):\n{}", name, want.len(), diffs.len(), diffs.join("\n")));
		}
	}
	Ok(out)
}

#[test]
fn paragraphs_break_into_typsts_lines() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let filter = std::env::var("U6A_FIXTURE").ok();
	let c = res!(compare_fixtures(&bin, &fixtures("linebreak"), filter.as_deref()));
	assert!(c.failures.is_empty(), "{} of {} fixtures ({} typst lines) break differently:\n{}",
		c.failures.len(), c.fixtures, c.lines, c.failures.join("\n"));
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HYPHENATION                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// English, German and French paragraphs of long words in narrow columns, set under `justify: true` so
/// that `hyphenate` is on, break at the same hyphenation points as Typst's. The count of hyphenated lines
/// is asserted so the test cannot pass on paragraphs that never needed a hyphen.
#[test]
fn hyphenation_in_english_german_and_french_matches_typst() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let c = res!(compare_fixtures(&bin, &fixtures("hyphenation"), None));
	assert_eq!(c.fixtures, 27, "the hyphenation corpus is nine fixtures in each of three languages");
	for lang in ["en", "de", "fr"] {
		let n = c.hyphens.get(lang).copied().unwrap_or(0);
		assert!(n >= 20, "typst hyphenates only {} {} lines, too few to test hyphenation with", n, lang);
	}
	assert!(c.failures.is_empty(), "{} of {} fixtures ({} typst lines) hyphenate differently:\n{}",
		c.failures.len(), c.fixtures, c.lines, c.failures.join("\n"));
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ OVERHANG                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Typst's lines of a source, each with the right edge of its last word, from `pdftotext -bbox-layout`.
fn typst_line_edges(bin: &str, name: &str, src: &str) -> Outcome<Vec<(String, f64)>> {
	let dir = res!(work("overhang"));
	let typ = dir.join(fmt!("{}.typ", name));
	let pdf = dir.join(fmt!("{}.pdf", name));
	res!(std::fs::write(&typ, src).map_err(|e| err!(e, "write"; IO)));
	let (t, p) = (typ.to_string_lossy().to_string(), pdf.to_string_lossy().to_string());
	res!(typst(bin, &["compile", "--ignore-system-fonts", &t, &p], &dir));
	let out = res!(Command::new("pdftotext").args(["-bbox-layout", &p, "-"]).output()
		.map_err(|e| err!(e, "Could not run pdftotext."; IO)));
	let html = String::from_utf8_lossy(&out.stdout).to_string();
	let attr = |tag: &str, key: &str| -> f64 {
		tag.split(&fmt!("{}=\"", key)).nth(1).and_then(|r| r.split('"').next())
			.and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
	};
	let mut words: Vec<(f64, f64, f64, f64, String)> = Vec::new();	// (middle, height, x0, x1, text)
	let mut rest = html.as_str();
	while let Some(i) = rest.find("<word") {
		let tail = &rest[i..];
		let (gt, end) = match (tail.find('>'), tail.find("</word>")) {
			(Some(g), Some(e))	=> (g, e),
			_					=> break,
		};
		let tag = &tail[..gt];
		let (y0, y1) = (attr(tag, "yMin"), attr(tag, "yMax"));
		words.push(((y0 + y1) / 2.0, y1 - y0, attr(tag, "xMin"), attr(tag, "xMax"), unescape(&tail[gt + 1..end])));
		rest = &tail[end..];
	}
	words.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
	let mut lines: Vec<(f64, f64, Vec<(f64, f64, String)>)> = Vec::new();
	for (mid, h, x0, x1, t) in words {
		match lines.last_mut() {
			Some((lm, lh, ws)) if (mid - *lm).abs() <= 0.35 * lh.max(h) => ws.push((x0, x1, t)),
			_ => lines.push((mid, h, vec![(x0, x1, t)])),
		}
	}
	Ok(lines.into_iter().map(|(_, _, mut ws)| {
		ws.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
		let edge = ws.last().map_or(0.0, |w| w.1);
		(norm(&ws.into_iter().map(|(_, _, t)| t).collect::<Vec<_>>().join(" ")), edge)
	}).collect())
}

/// How wide a committed line's items run, in points: the box's width plus whatever hangs past it.
fn run_width(node: &Node) -> f64 {
	match node {
		Node::HBox(b)	=> b.list.iter().map(|n| match n {
			Node::Leaf(l)	=> l.dims.width.to_pt(),
			Node::Glue(g)	=> g.natural.to_pt(),
			Node::HBox(x) | Node::VBox(x)	=> x.dims.width.to_pt(),
			_				=> 0.0,
		}).sum(),
		_				=> 0.0,
	}
}

/// Overhang moves text only when a line is committed: the last punctuation mark of a justified line hangs
/// into the end margin by a fixed fraction of its advance, the line is justified into that much more
/// room, and the breaker, which measures lines without it, breaks where it breaks with overhang off.
/// Typst's lines and the right edge of each are the oracle.
#[test]
fn overhang_hangs_punctuation_at_commit_and_never_moves_a_break() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	// Every line of every run ends in a comma, full stop, hyphen, colon or dash often enough that some hang.
	let body = "Rain, wind, fog; hail: sleet. Night, dusk, dawn; noon: haze. Salt, sand, kelp, stone, tide. \
		Boats, nets, ropes, oars, sails, masts, decks. Gulls cry, waves roll, bells ring, lamps burn low. \
		Ink, paper, thread, glue, wax, board, cloth, leather, gold. Slow carts creak; tired horses plod; dust rises.";
	let mut hung = 0usize;
	for (width, on) in [(150.0f64, true), (150.0, false), (190.0, true), (190.0, false), (230.0, true), (230.0, false)] {
		let flag = if on { "true" } else { "false" };
		let src = fmt!(
			"#set page(width: {}pt, height: auto, margin: 0pt)\n#set par(justify: true)\n\
			#set text(size: 10pt, lang: \"en\", hyphenate: false, overhang: {})\n{}\n", width, flag, body);
		let name = fmt!("w{}-{}", width as u32, flag);
		let want = res!(typst_line_edges(&bin, &name, &src));
		assert!(want.len() >= 5, "{}: typst set only {} lines", name, want.len());

		let path = res!(work("overhang")).join(fmt!("{}.typ", name));
		let fx = res!(read_fixture(&src));
		let boxes = res!(austenite_boxes(&path, &fx));
		let got_lines: Vec<String> = boxes.iter().map(|l| norm(&l.text)).collect();
		let want_lines: Vec<String> = want.iter().map(|l| l.0.clone()).collect();
		assert_eq!(got_lines, want_lines, "{}: the lines differ", name);
		for (i, (line, w)) in boxes.iter().zip(want.iter()).enumerate() {
			let ours = run_width(&line.node) - width;
			let theirs = w.1 - width;
			assert!((ours - theirs).abs() < 0.06,
				"{} line {} {:?}: the last mark hangs {:.3}pt in typst and {:.3}pt in austenite", name, i + 1, w.0, theirs, ours);
			if on && theirs > 0.5 {
				hung += 1;
			}
		}
	}
	assert!(hung >= 6, "only {} lines hang a mark in typst, too few to test overhang with", hung);
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE PARAGRAPH RULE                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// The paragraphs a source realises into as a document, as a block flow meets them, with the engine.
fn paragraphs(src: &str) -> Outcome<(Engine, Vec<Pair>)> {
	let root = res!(work("pars"));
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("p.typ"), src.to_string()));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	let pairs = res!(realise(&mut engine, &module.content, &StyleChain::root(), RealiseMode::Document));
	Ok((engine, pairs.into_iter().filter(|p| p.content.is(ElemKind::Par)).collect()))
}

fn region(width: f64) -> Region {
	Region::new(Sp::from_pt(width), Sp::from_pt(10_000.0))
}

/// Everything layout decides about nodes, to the bit: the kinds, the extents, every glyph's id and place,
/// the text and, when `addr`, the font's address. Two equal signatures are two identical layouts.
fn signature(nodes: &[Node], out: &mut String, addr: bool) {
	for n in nodes {
		match n {
			Node::HBox(b)		=> {
				out.push_str(&fmt!("[h {} {} {}|", b.dims.width.raw(), b.dims.height.raw(), b.dims.depth.raw()));
				signature(&b.list, out, addr);
				out.push(']');
			}
			Node::VBox(b)		=> {
				out.push_str(&fmt!("[v {} {} {}|", b.dims.width.raw(), b.dims.height.raw(), b.dims.depth.raw()));
				signature(&b.list, out, addr);
				out.push(']');
			}
			Node::Leaf(l)		=> match &l.kind {
				LeafKind::Text(t) => {
					out.push_str(&fmt!("(t {:?} {} {}", t.source(), l.dims.width.raw(), l.shift.raw()));
					if addr {
						out.push_str(&fmt!(" @{:p}", t.font()));
					}
					for g in &t.run().glyphs {
						out.push_str(&fmt!(" {}:{}:{}:{}", g.id, g.x.to_bits(), g.y.to_bits(), g.adv.to_bits()));
					}
					out.push(')');
				}
				other => out.push_str(&fmt!("(leaf {:?} {})", std::mem::discriminant(other), l.dims.width.raw())),
			},
			Node::Glue(g)		=> out.push_str(&fmt!("(g {} {} {})", g.natural.raw(), g.stretch.raw(), g.shrink.raw())),
			Node::Penalty(p)	=> out.push_str(&fmt!("(p {:?})", p)),
			Node::Anchor(a)		=> out.push_str(&fmt!("(a {:?})", a)),
			other				=> out.push_str(&fmt!("({:?})", std::mem::discriminant(other))),
		}
	}
}

/// The signature of `nodes` with the fonts' addresses, for layouts made from one book.
fn sig(nodes: &[Node]) -> String {
	let mut s = String::new();
	signature(nodes, &mut s, true);
	s
}

/// The signature without addresses, for layouts made from two books.
fn shape_sig(nodes: &[Node]) -> String {
	let mut s = String::new();
	signature(nodes, &mut s, false);
	s
}

fn lines_of(nodes: &[Node]) -> usize {
	nodes.iter().filter(|n| matches!(n, Node::HBox(_))).count()
}

/// `layout_par` keeps nothing and answers from its inputs alone: the same paragraph gives the same lines
/// before and after another is set, and no locator ordinal, diagnostic or read is left behind.
#[test]
fn layout_par_keeps_nothing_and_depends_only_on_its_inputs() -> Outcome<()> {
	let src = "A first paragraph with #strong[some strong words] and #emph[slanted ones], long enough to wrap onto \
		several lines at a narrow measure, with #lorem(30)\n\nA second paragraph, quite different from the first, \
		set between the two settings of the first, and #lorem(45)\n";
	let (mut engine, pars) = res!(paragraphs(src));
	assert_eq!(pars.len(), 2, "the source realises into two paragraphs");
	let (a, b) = (&pars[0], &pars[1]);

	let locator_before = fmt!("{:?}", engine.locator);
	let (diags_before, reads_before) = (engine.diags.len(), fmt!("{:?}", engine.reads));
	let one = res!(layout_par(&mut engine, &a.content, &a.styles, region(150.0), ParSituation::First));
	assert!(lines_of(&one) >= 4, "the first paragraph sets in {} lines at 150pt, too few to test with", lines_of(&one));
	assert_eq!(fmt!("{:?}", engine.locator), locator_before, "layout_par consumed locator ordinals");
	assert_eq!(engine.diags.len(), diags_before, "layout_par left diagnostics behind");
	assert_eq!(fmt!("{:?}", engine.reads), reads_before, "layout_par recorded reads it had no cause to");

	let other = res!(layout_par(&mut engine, &b.content, &b.styles, region(200.0), ParSituation::Consecutive));
	assert!(lines_of(&other) >= 3);
	let again = res!(layout_par(&mut engine, &a.content, &a.styles, region(150.0), ParSituation::First));
	assert!(sig(&one) == sig(&again), "a paragraph set again, after another, came out differently");

	// A fresh engine, which has set nothing, sets it identically: nothing it needs was in the first.
	let (mut fresh, fresh_pars) = res!(paragraphs(src));
	let from_fresh = res!(layout_par(&mut fresh, &fresh_pars[0].content, &fresh_pars[0].styles, region(150.0), ParSituation::First));
	assert!(shape_sig(&one) == shape_sig(&from_fresh), "a fresh engine set the paragraph differently");

	// And the inputs do matter: the width and the situation each change the answer.
	let wide = res!(layout_par(&mut engine, &a.content, &a.styles, region(400.0), ParSituation::First));
	assert!(lines_of(&wide) < lines_of(&one), "a wider measure should set fewer lines");
	Ok(())
}

/// A located element inside a paragraph is met where it stands in the text: `layout_par` puts its start
/// in the line as an anchor, before the text it covers and after the text before it.
#[test]
fn a_located_element_is_tagged_where_it_stands_in_its_paragraph() -> Outcome<()> {
	let (mut engine, pars) = res!(paragraphs("Alpha #strong[beta] gamma #emph[delta] epsilon\n"));
	assert_eq!(pars.len(), 1);
	let nodes = res!(layout_par(&mut engine, &pars[0].content, &pars[0].styles, region(400.0), ParSituation::First));
	assert_eq!(lines_of(&nodes), 1, "the paragraph fits one line");
	let mut flat: Vec<String> = Vec::new();
	fn walk(n: &Node, out: &mut Vec<String>) {
		match n {
			Node::HBox(b) | Node::VBox(b)	=> for c in &b.list { walk(c, out); },
			Node::Leaf(l)					=> if let LeafKind::Text(t) = &l.kind { out.push(fmt!("text:{}", t.source())); },
			Node::Anchor(_)					=> out.push("anchor".to_string()),
			_								=> (),
		}
	}
	for n in &nodes {
		walk(n, &mut flat);
	}
	let joined = flat.join("|");
	let anchors = flat.iter().filter(|f| f.as_str() == "anchor").count();
	assert_eq!(anchors, 2, "strong and emph each start an anchor: {}", joined);
	let at = |needle: &str| flat.iter().position(|f| f.contains(needle));
	let (a1, a2) = {
		let mut it = flat.iter().enumerate().filter(|(_, f)| f.as_str() == "anchor").map(|(i, _)| i);
		(it.next().unwrap_or(usize::MAX), it.next().unwrap_or(usize::MAX))
	};
	assert!(at("Alpha").map_or(false, |i| i < a1), "the first anchor precedes `Alpha`: {}", joined);
	assert!(at("beta").map_or(false, |i| i > a1 && i < a2), "`beta` stands between the anchors: {}", joined);
	assert!(at("delta").map_or(false, |i| i > a2), "`delta` follows the second anchor: {}", joined);
	Ok(())
}

/// Lines share the face that drew them: every text leaf of a paragraph holds the book's one font, and
/// the same face, shaped twice, holds the one glyph list.
#[test]
fn lines_share_one_face_and_shaping_shares_one_glyph_list() -> Outcome<()> {
	let (mut engine, pars) = res!(paragraphs("A paragraph of plain prose long enough to set over several lines in one face, #lorem(40)\n"));
	let nodes = res!(layout_par(&mut engine, &pars[0].content, &pars[0].styles, region(120.0), ParSituation::First));
	assert!(lines_of(&nodes) >= 4);
	let book = res!(engine.fonts.book());
	let face = res!(book.select("libertinus serif", FaceVariant::default()).ok_or_else(|| err!("no Libertinus"; Test, Missing)));
	let want: *const _ = &*res!(book.face(face).ok_or_else(|| err!("no face"; Test, Missing))).font;
	let mut leaves = 0usize;
	fn walk(n: &Node, want: *const oxedyne_fe2o3_font::font::Font, leaves: &mut usize, bad: &mut usize) {
		match n {
			Node::HBox(b) | Node::VBox(b)	=> for c in &b.list { walk(c, want, leaves, bad); },
			Node::Leaf(l)					=> if let LeafKind::Text(t) = &l.kind {
				*leaves += 1;
				if !std::ptr::eq(t.font(), want) {
					*bad += 1;
				}
			},
			_								=> (),
		}
	}
	let mut bad = 0usize;
	for n in &nodes {
		walk(n, want, &mut leaves, &mut bad);
	}
	assert!(leaves >= 8, "only {} text leaves", leaves);
	assert_eq!(bad, 0, "{} of {} text leaves hold a copy of the font, not the book's", bad, leaves);

	let spec = ShapeSpec { remove_ignorables: true, ..ShapeSpec::default() };
	let x = res!(book.shape(face, "shared", ShapeDir::Ltr, &spec));
	let y = res!(book.shape(face, "shared", ShapeDir::Ltr, &spec));
	assert!(Arc::ptr_eq(&x, &y), "the same call returned two glyph lists, not one");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SHAPED-RUN CACHE                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// The cache never holds more than its budget, evicts the least recently used entry first, and keeps
/// none that is larger than the budget alone.
#[test]
fn the_shape_cache_holds_its_budget_and_evicts_the_least_recently_used() -> Outcome<()> {
	let book = res!(FontBook::embedded());
	let face = res!(book.select("libertinus serif", FaceVariant::default()).ok_or_else(|| err!("no Libertinus"; Test, Missing)));
	let spec = ShapeSpec { remove_ignorables: true, ..ShapeSpec::default() };
	// Six letters and no ligature or kern pair between them, so every word costs the same.
	let words = ["abcdef", "ghijkl", "mnopqr", "stuvwx", "yzabcd", "efghij"];

	// What one entry costs: the cache's own count after one call.
	res!(book.set_shape_budget(1 << 20));
	res!(book.shape(face, words[0], ShapeDir::Ltr, &spec));
	let one = res!(book.shape_stats()).bytes;
	assert!(one > 0);

	// A budget for exactly three.
	let book = res!(FontBook::embedded());
	res!(book.set_shape_budget(3 * one));
	let check = |label: &str| -> Outcome<oxedyne_fe2o3_austenite::fonts::ShapeStats> {
		let s = res!(book.shape_stats());
		assert!(s.bytes <= s.budget, "{}: the cache holds {} bytes over a budget of {}", label, s.bytes, s.budget);
		Ok(s)
	};
	for w in &words[..3] {
		res!(book.shape(face, w, ShapeDir::Ltr, &spec));
		res!(check(w));
	}
	let s = res!(check("three words"));
	assert_eq!((s.entries, s.hits, s.misses, s.evictions), (3, 0, 3, 0));

	// A is used again, so B is now the least recently used.
	res!(book.shape(face, words[0], ShapeDir::Ltr, &spec));
	assert_eq!(res!(check("A used again")).hits, 1);
	// D arrives and B goes.
	res!(book.shape(face, words[3], ShapeDir::Ltr, &spec));
	let s = res!(check("D arrives"));
	assert_eq!((s.entries, s.evictions), (3, 1), "one entry should have gone");
	let (hits, misses) = (s.hits, s.misses);
	res!(book.shape(face, words[0], ShapeDir::Ltr, &spec));
	res!(book.shape(face, words[2], ShapeDir::Ltr, &spec));
	let s = res!(check("A and C asked for"));
	assert_eq!((s.hits, s.misses), (hits + 2, misses), "A and C should both have been kept");
	res!(book.shape(face, words[1], ShapeDir::Ltr, &spec));
	let s = res!(check("B asked for"));
	assert_eq!(s.misses, misses + 1, "B should have been evicted, being the least recently used");

	// A long run of words never takes the cache past its budget.
	for i in 0..200 {
		res!(book.shape(face, &fmt!("word{:03}", i), ShapeDir::Ltr, &spec));
		res!(check("a long run"));
	}
	assert!(res!(book.shape_stats()).evictions >= 190);

	// A result bigger than the whole budget is not kept, and the budget holds.
	let tiny = res!(FontBook::embedded());
	res!(tiny.set_shape_budget(16));
	let glyphs = res!(tiny.shape(face, "far larger than sixteen bytes", ShapeDir::Ltr, &spec));
	assert!(!glyphs.is_empty());
	let s = res!(tiny.shape_stats());
	assert_eq!((s.entries, s.bytes), (0, 0), "an entry over the budget was kept");

	// Lowering the budget evicts down to it at once.
	res!(book.set_shape_budget(one));
	let s = res!(book.shape_stats());
	assert!(s.bytes <= one && s.entries <= 1, "{} entries, {} bytes after the budget fell to {}", s.entries, s.bytes, one);
	Ok(())
}

/// The store's budget reaches the book it builds, and a paragraph set again is shaped from the cache.
#[test]
fn a_paragraph_set_again_is_shaped_from_the_cache() -> Outcome<()> {
	let (mut engine, pars) = res!(paragraphs("Words that shape once and are then served again from the cache, #lorem(30)\n"));
	res!(engine.fonts.set_shape_budget(1 << 20));
	let p = &pars[0];
	res!(layout_par(&mut engine, &p.content, &p.styles, region(200.0), ParSituation::First));
	let book = res!(engine.fonts.book());
	let first = res!(book.shape_stats());
	assert_eq!(first.budget, 1 << 20, "the store's budget did not reach the book");
	assert!(first.entries > 0 && first.misses > 0);
	res!(layout_par(&mut engine, &p.content, &p.styles, region(200.0), ParSituation::First));
	let second = res!(book.shape_stats());
	assert_eq!(second.misses, first.misses, "the same paragraph, set again, was shaped again");
	assert!(second.hits > first.hits, "the second setting asked the cache for nothing");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ A MISSING FAMILY                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Typst's warnings as `(message, line, column)`, in the order it prints them. Typst prints its columns
/// from zero, and Austenite's diagnostics from one, as the gate's do; the column here is from one.
fn typst_warnings(bin: &str, name: &str, src: &str) -> Outcome<Vec<(String, usize, usize)>> {
	let dir = res!(work("fonts"));
	let typ = dir.join(fmt!("{}.typ", name));
	let pdf = dir.join(fmt!("{}.pdf", name));
	res!(std::fs::write(&typ, src).map_err(|e| err!(e, "write"; IO)));
	let capped = Command::new("systemd-run").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
	let mut cmd = if capped {
		let mut c = Command::new("systemd-run");
		c.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", bin]);
		c
	} else {
		Command::new(bin)
	};
	cmd.args(["compile", "--ignore-system-fonts"]).arg(&typ).arg(&pdf).current_dir(&dir);
	let out = res!(cmd.output().map_err(|e| err!(e, "Could not run typst."; IO)));
	let stderr = String::from_utf8_lossy(&out.stderr).to_string();
	let mut found = Vec::new();
	let mut message: Option<String> = None;
	for line in stderr.lines() {
		if let Some(m) = line.strip_prefix("warning: ") {
			message = Some(m.to_string());
		} else if let (Some(m), Some(at)) = (&message, line.split("┌─ ").nth(1)) {
			let mut parts = at.rsplit(':');
			let col = parts.next().and_then(|c| c.trim().parse::<usize>().ok());
			let ln = parts.next().and_then(|c| c.trim().parse::<usize>().ok());
			if let (Some(l), Some(c)) = (ln, col) {
				found.push((m.clone(), l, c + 1));
				message = None;
			}
		}
	}
	Ok(found)
}

/// Austenite's `missing_font` warnings of a source as `(message, line, column)`.
fn missing_fonts(src: &str) -> Outcome<(Vec<(String, usize, usize)>, Engine)> {
	let root = res!(work("fonts"));
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("m.typ"), src.to_string()));
	let mut engine = Engine::new(world);
	res!(eval_source(&mut engine, id));
	let mut out = Vec::new();
	for d in &engine.diags {
		if d.kind != DiagnosticKind::MissingFont {
			continue;
		}
		assert!(!d.is_error(), "a missing family is a warning, not an error: {:?}", d);
		let (l, c) = match engine.world.source(d.span.file) {
			Some(s)	=> s.line_col(d.span.start),
			None	=> return Err(err!("A diagnostic names a source the world does not hold."; Test, Missing)),
		};
		out.push((d.message.clone(), l as usize, c as usize));
	}
	Ok((out, engine))
}

/// A family no font declares is Typst's warning, `unknown font family: <name>`, at the `font` argument,
/// once for each family of a list, from a `set` rule and from a `text(..)` call, and once however often
/// the rule is met. Its kind refuses a strict compile.
#[test]
fn a_missing_family_warns_as_typst_warns_with_kind_missing_font() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let cases: &[(&str, &str, usize)] = &[
		("single", "#set text(font: \"Nonexistent Family\")\nHello world.\n", 1),
		("list", "#text(font: (\"Libertinus Serif\", \"Fooo\"))[Hi]\n\n#set text(font: (\"Bar\", \"Baz\"))\nTwo.\n", 3),
		("dict", "#set text(font: (name: \"Quux\", covers: \"latin-in-cjk\"))\nThree.\n", 1),
		("known", "#set text(font: (\"libertinus serif\", \"DejaVu Sans Mono\", \"New Computer Modern Math\"))\nFine.\n", 0),
		("again", "#for i in range(3) [#text(font: \"Nothing Like It\")[x]]\n", 1),
	];
	let mut total = 0usize;
	for (name, src, count) in cases {
		let want = res!(typst_warnings(&bin, name, src));
		let (got, engine) = res!(missing_fonts(src));
		assert_eq!(want.len(), *count, "{}: typst gave {} warnings, expected {}: {:?}", name, want.len(), count, want);
		assert_eq!(got, want, "{}: austenite's warnings differ from typst's", name);
		for d in engine.diags.iter().filter(|d| d.kind == DiagnosticKind::MissingFont) {
			assert!(d.kind.refuses_strict(), "{}: a missing family should refuse a strict compile", name);
		}
		total += got.len();
	}
	assert_eq!(total, 6);
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE LANGUAGE A CHAIN SETS                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// The language and region a document's text is set in, which the document's own language is read from,
/// follow `set text(lang:, region:)` as Typst reads them in a `context`, a later rule overriding an earlier.
#[test]
fn set_text_lang_reaches_the_language_a_chain_gives() -> Outcome<()> {
	let bin = match res!(oracle()) {
		Some(b)	=> b,
		None	=> return Ok(()),
	};
	let cases: &[(&str, &str)] = &[
		("none", ""),
		("lang", "#set text(lang: \"de\")\n"),
		("both", "#set text(lang: \"de\", region: \"CH\")\n"),
		("later", "#set text(lang: \"fr\")\n#set text(lang: \"pt\", region: \"BR\")\n"),
		("region", "#set text(region: \"TW\", lang: \"zh\")\n"),
	];
	for (name, rules) in cases {
		let probe = fmt!("{}#context [#metadata((text.lang, text.region)) <m>]\n", rules);
		let dir = res!(work("lang"));
		let path = dir.join(fmt!("{}.typ", name));
		res!(std::fs::write(&path, &probe).map_err(|e| err!(e, "write"; IO)));
		let p = path.to_string_lossy().to_string();
		let json = res!(typst(&bin, &["query", "--ignore-system-fonts", "--field", "value", "--one", &p, "<m>"], &dir));
		let mut want = json_strings(&json);
		let region_null = json.contains("null");
		let (lang, region) = (want.remove(0), if region_null { None } else { want.pop() });

		let (_, pars) = res!(paragraphs(&fmt!("{}Some text.\n", rules)));
		assert_eq!(pars.len(), 1, "{}: one paragraph", name);
		let (l, r) = res!(locale(&pars[0].styles));
		assert_eq!((l, r), (lang, region), "{}: the chain's language and region differ from typst's ({})", name, json.trim());
	}
	Ok(())
}
