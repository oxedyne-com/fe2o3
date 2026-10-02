//! The evaluator's structure oracle (U12): Austenite against the host's `typst` 0.15.x on a synthetic
//! fixture corpus, at four levels -- values, positions, realised structure, layout. The driver is
//! `tests/eval_oracle/mod.rs`; this file states what is asserted.
//!
//! Two kinds of test live here. The harness's own tests prove each reader and comparator against the
//! oracle on fixtures whose answer is fixed by construction, and prove each comparator goes red on a
//! difference -- a harness that cannot fail measures nothing. The conformance test runs every
//! fixture and prints the scoreboard; it fails on any Austenite panic or unbounded run, on any
//! fixture Typst rejects (or accepts, when it is marked `oracle: rejects`), and, in the areas `EVAL_ORACLE_STRICT` names, on any shortfall.
//!
//!     cargo test -p oxedyne_fe2o3_austenite --test eval_oracle -- --nocapture
//!     EVAL_ORACLE_AREA=general EVAL_ORACLE_STRICT=general cargo test ... --test eval_oracle

// `#[path]` because this test crate's root is itself `eval_oracle.rs`; a plain `mod eval_oracle;`
// would find the module at two paths.
#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::austenite::{
	self,
	RunEnd,
};
use harness::corpus::{
	self,
	Expect,
	Filter,
};
use harness::json::{
	self,
	J,
};
use harness::layout::{
	self,
	APage,
	ARun,
};
use harness::oracle::Oracle;
use harness::structure::{
	self,
	Sk,
};
use harness::{
	Board,
	PosRow,
};

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	Elem,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::fixpoint::Laid;
use oxedyne_fe2o3_austenite::eval::intro::Builder;
use oxedyne_fe2o3_austenite::eval::locate::Location;
use oxedyne_fe2o3_austenite::eval::value::{
	Dict,
	Label,
	Value,
};
use oxedyne_fe2o3_austenite::font::ShapedText;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::ir::Sp;
use oxedyne_fe2o3_austenite::ledger::Position;
use oxedyne_fe2o3_austenite::page::{
	Frame,
	Page,
	PageGeometry,
	Placed,
	PlacedKind,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Source,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::face::Role;
use oxedyne_fe2o3_font::shape::Dir;

use std::path::PathBuf;
use std::sync::Arc;

/// The oracle, or `None` after saying loudly that it was skipped on request.
fn oracle() -> Outcome<Option<Oracle>> {
	let o = res!(Oracle::find());
	if o.is_none() {
		println!("[eval-oracle] SKIPPED: EVAL_ORACLE_SKIP=1 and no typst 0.15.x -- nothing was compared");
	}
	Ok(o)
}

fn harness_fixture(name: &str) -> Outcome<corpus::Fixture> {
	let all = res!(corpus::discover(&Filter::only("harness")));
	match all.into_iter().find(|f| f.name == name) {
		Some(f)	=> Ok(f),
		None	=> Err(err!("The harness fixture {} is missing from tests/fixtures/eval/harness", name; Missing)),
	}
}

#[test]
fn oracle_is_present_and_pinned() -> Outcome<()> {
	if let Some(o) = res!(oracle()) {
		assert!(o.version.starts_with(harness::oracle::PINNED), "oracle version {}", o.version);
		println!("[eval-oracle] {} (cache {})", o.version, o.work.display());
	}
	Ok(())
}

#[test]
fn fixture_corpus_is_synthetic_and_accepted_by_typst() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let all = res!(corpus::discover(&Filter::from_env()));
	assert!(!all.is_empty(), "no fixtures under {}", corpus::fixtures_dir().display());
	let mut bad = Vec::new();
	for fx in &all {
		if let Some(f) = corpus::provenance_fault(fx) {
			bad.push(f);
			continue;
		}
		match (res!(o.compiles(fx)), fx.expect) {
			(Err(e), Expect::Accepts)	=> bad.push(fmt!("{}: typst rejects it: {}", fx.id(), e)),
			(Ok(()), Expect::Rejects)	=> bad.push(fmt!("{}: marked `oracle: rejects`, but typst accepts it", fx.id())),
			_							=> (),
		}
	}
	assert!(bad.is_empty(), "corpus faults:\n{}", bad.join("\n"));
	Ok(())
}

// The values of `harness/values.typ`, built by hand from Austenite's own value types.
fn hand_values() -> Vec<Value> {
	let mut inner = Dict::new();
	inner.insert("c", Value::str("x"));
	let mut d = Dict::new();
	d.insert("b", Value::Int(1));
	d.insert("a", Value::dict(inner));
	vec![
		Value::Int(1),
		Value::Int(-7),
		Value::Float(1.5),
		Value::Float(1.0),
		Value::str("quoted \"text\", tab\tand ünïcode →"),
		Value::Bool(true),
		Value::None,
		Value::array(vec![Value::Int(1), Value::array(vec![Value::Int(2), Value::Int(3)]), Value::array(vec![])]),
		Value::dict(d),
		Value::Float(f64::INFINITY),
		Value::Int(9_007_199_254_740_993),
	]
}

#[test]
fn value_serialisation_agrees_with_the_oracle_and_goes_red_on_a_difference() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fx = res!(harness_fixture("values"));
	let want = match res!(o.probes(&fx)) {
		Ok(v)	=> J::Arr(v),
		Err(e)	=> return Err(err!("the oracle could not read harness/values: {}", e; Invalid)),
	};
	let vals = hand_values();
	let got = J::Arr(vals.iter().map(austenite::to_json).collect());
	let mut d = Vec::new();
	json::diff("probes", &want, &got, &mut d);
	assert!(d.is_empty(), "hand-built values differ from typst:\n{}", d.join("\n"));

	// Each perturbation is a distinction Typst makes and the comparison must keep.
	let perturb: [(&str, fn(&mut Vec<Value>)); 7] = [
		("int for float",	|v| v[3] = Value::Int(1)),
		("float for int",	|v| v[0] = Value::Float(1.0)),
		("changed string",	|v| v[4] = Value::str("quoted")),
		("dictionary order",	|v| {
			let mut d = Dict::new();
			d.insert("a", Value::Int(0));
			d.insert("b", Value::Int(1));
			v[8] = Value::dict(d);
		}),
		("missing item",	|v| { v.pop(); }),
		("none for false",	|v| v[5] = Value::None),
		("near integer",	|v| v[10] = Value::Int(9_007_199_254_740_992)),
	];
	for (what, f) in perturb {
		let mut vals = hand_values();
		f(&mut vals);
		let got = J::Arr(vals.iter().map(austenite::to_json).collect());
		let mut d = Vec::new();
		json::diff("probes", &want, &got, &mut d);
		assert!(!d.is_empty(), "the comparison missed a {}", what);
	}
	Ok(())
}

#[test]
fn content_comparison_ignores_field_order_but_not_text() -> Outcome<()> {
	// Content in the shape the oracle writes it (see `markup_content.typ`), and variations of it.
	let want = res!(json::parse(r#"{"func":"strong","body":{"func":"text","text":"b"},"label":"<x>"}"#));
	let same = res!(json::parse(r#"{"label":"<x>","body":{"text":"b","func":"text"},"func":"strong"}"#));
	let other = res!(json::parse(r#"{"func":"strong","body":{"func":"text","text":"c"},"label":"<x>"}"#));
	let unlabelled = res!(json::parse(r#"{"func":"strong","body":{"func":"text","text":"b"}}"#));
	let mut d = Vec::new();
	json::diff("v", &want, &same, &mut d);
	assert!(d.is_empty(), "field order is not significant in content: {:?}", d);
	json::diff("v", &want, &other, &mut d);
	assert!(!d.is_empty(), "changed text must differ");
	d.clear();
	json::diff("v", &want, &unlabelled, &mut d);
	assert!(!d.is_empty(), "a lost label must differ");
	// And a dictionary, unlike content, is ordered.
	let d1 = res!(json::parse(r#"{"a":1,"b":2}"#));
	let d2 = res!(json::parse(r#"{"b":2,"a":1}"#));
	d.clear();
	json::diff("v", &d1, &d2, &mut d);
	assert!(!d.is_empty(), "dictionary order is significant");
	Ok(())
}

#[test]
fn layout_reader_recovers_known_baselines() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fx = res!(harness_fixture("baselines"));
	let pages = match res!(o.layout(&fx)) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("the oracle could not lay out harness/baselines: {}", e; Invalid)),
	};
	assert_eq!(pages.len(), 1);
	let p = &pages[0];
	assert!((p.width - 200.0).abs() < 0.01 && (p.height - 220.0).abs() < 0.01, "page {}x{}", p.width, p.height);
	// Placed with baseline edges at these offsets, so these are the baselines, whatever the face.
	let expect = [("Alpha beta", 40.0), ("Gamma", 100.0), ("Delta", 150.0), ("Epsilon2 zeta", 190.0)];
	assert_eq!(p.lines.len(), expect.len(), "lines: {:?}", p.lines);
	for (l, (text, base)) in p.lines.iter().zip(expect.iter()) {
		assert_eq!(l.text.replace(' ', ""), text.replace(' ', ""), "line text");
		assert!(l.measured, "{:?}: baseline estimated, not measured", l.text);
		assert!((l.base - base).abs() < 0.01, "{:?}: baseline {} not {}", l.text, l.base, base);
		assert!((l.x0 - 10.0).abs() < 0.5, "{:?}: x {}", l.text, l.x0);
	}

	// The comparator: Typst's own lines as Austenite runs pass; a 2pt drop, a lost line, changed
	// text, an extra run and an extra page each fail.
	let as_runs = |shift: f64| -> Vec<APage> {
		vec![APage {
			width:	p.width,
			height:	p.height,
			runs:	p.lines.iter().map(|l| ARun { text: l.text.clone(), x0: l.x0, x1: l.x1, base: l.base + shift }).collect(),
		}]
	};
	let mut d = Vec::new();
	layout::compare(&pages, &as_runs(0.0), &mut d);
	assert!(d.is_empty(), "identity differs: {:?}", d);
	layout::compare(&pages, &as_runs(0.8), &mut d);
	assert!(d.is_empty(), "0.8pt is inside the tolerance: {:?}", d);
	layout::compare(&pages, &as_runs(2.0), &mut d);
	assert!(!d.is_empty(), "a 2pt drop must fail");
	let mut lost = as_runs(0.0);
	lost[0].runs.remove(1);
	d.clear();
	layout::compare(&pages, &lost, &mut d);
	assert!(!d.is_empty(), "a lost line must fail");
	let mut changed = as_runs(0.0);
	changed[0].runs[0].text = "Alpha beat".to_string();
	d.clear();
	layout::compare(&pages, &changed, &mut d);
	assert!(!d.is_empty(), "changed text must fail");
	let mut extra = as_runs(0.0);
	extra[0].runs.push(ARun { text: "stray".to_string(), x0: 100.0, x1: 120.0, base: 70.0 });
	d.clear();
	layout::compare(&pages, &extra, &mut d);
	assert!(!d.is_empty(), "stray text must fail");
	let mut more = as_runs(0.0);
	more.push(APage { width: p.width, height: p.height, runs: Vec::new() });
	d.clear();
	layout::compare(&pages, &more, &mut d);
	assert!(!d.is_empty(), "an extra page must fail");
	Ok(())
}

#[test]
fn structure_reader_matches_the_html_export() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fx = res!(harness_fixture("structure"));
	let html = match res!(o.html(&fx)) {
		Ok(h)	=> h,
		Err(e)	=> return Err(err!("the oracle could not export harness/structure: {}", e; Invalid)),
	};
	assert!(html.ignored.is_empty(), "ignored: {:?}", html.ignored);
	let want = concat!(
		r#"h2["Title words"] p["A paragraph with " strong["strong"] ", " em["emph"] " and " code["code"] "."] "#,
		r#"ul[li["one"] li["two"]] ol[li["first"] li["second"]] dl[dt["Term"] dd["its description"]] "#,
		r#"blockquote["Quoted"] p[a["a link"]]"#,
	);
	assert_eq!(structure::render(&html.skeleton), want);

	let mut d = Vec::new();
	structure::compare(&html.skeleton, &html.skeleton.clone(), &mut d);
	assert!(d.is_empty());
	let t = |s: &str| Sk::Text(s.to_string());
	let tag = |n: &str, k: Vec<Sk>| Sk::Tag(n.to_string(), k);
	// Whitespace and quote glyphs normalise away; a tag change or lost element does not.
	let a = structure::normalise(vec![tag("p", vec![t("  a  “b”\n c ")])]);
	let b = structure::normalise(vec![tag("p", vec![t("a \"b\" c")])]);
	structure::compare(&a, &b, &mut d);
	assert!(d.is_empty(), "{:?}", d);
	let c = structure::normalise(vec![tag("h3", vec![t("a \"b\" c")])]);
	structure::compare(&a, &c, &mut d);
	assert!(!d.is_empty(), "a changed tag must fail");
	d.clear();
	let mut lost = html.skeleton.clone();
	lost.pop();
	structure::compare(&html.skeleton, &lost, &mut d);
	assert!(!d.is_empty(), "a lost element must fail");
	// And an export Typst mutilates is recognised, so it is never compared.
	assert_eq!(structure::ignored_elements("warning: align was ignored during HTML export\nwarning: x"), vec!["align"]);
	Ok(())
}

#[test]
fn position_reader_reads_known_positions() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fx = res!(harness_fixture("positions"));
	let rows = match res!(o.positions(&fx)) {
		Ok(r)	=> r,
		Err(e)	=> return Err(err!("the oracle could not locate harness/positions: {}", e; Invalid)),
	};
	let funcs: Vec<&str> = rows.iter().map(|r| r.func.as_str()).collect();
	assert_eq!(funcs, vec!["heading", "metadata", "heading"]);
	// Both headings sit at the top-left margin corner, one on each page.
	assert_eq!((rows[0].page, rows[2].page), (1, 2));
	for r in [&rows[0], &rows[2]] {
		assert!((r.x - 20.0).abs() < 1e-6 && (r.y - 20.0).abs() < 1e-6, "{:?}", r);
	}
	assert!(harness::compare_positions(&rows, &rows).is_empty());
	let moved: Vec<PosRow> = rows.iter().map(|r| PosRow { y: r.y + 0.6, ..r.clone() }).collect();
	assert!(!harness::compare_positions(&rows, &moved).is_empty(), "0.6pt is outside the tolerance");
	let within: Vec<PosRow> = rows.iter().map(|r| PosRow { x: r.x - 0.4, ..r.clone() }).collect();
	assert!(harness::compare_positions(&rows, &within).is_empty(), "0.4pt is inside the tolerance");
	let paged: Vec<PosRow> = rows.iter().map(|r| PosRow { page: 1, ..r.clone() }).collect();
	assert!(!harness::compare_positions(&rows, &paged).is_empty(), "a page change must fail");
	assert!(!harness::compare_positions(&rows, &rows[..2]).is_empty(), "a lost element must fail");
	Ok(())
}

// A laid-out document built by hand in Austenite's own page and introspector types, placed where
// `harness/baselines.typ` and `harness/positions.typ` put things, so Austenite's side of levels 2 and
// 4 is read through the same code the conformance run uses and checked against the oracle.
fn hand_laid(lines: &[(&str, f64)], located: &[(ElemKind, bool, u32, f64, f64)], shift: f64)
	-> Outcome<(Laid, Vec<Page>)>
{
	let fonts = Arc::new(res!(fonts::libertinus()));
	let mut frame = Frame::new();
	for (text, base) in lines {
		let st = res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, Sp::from_pt(11.0), text));
		let dims = st.dims();
		// A placed box's top-left is its position; its baseline sits `height` below.
		let y = Sp::from_pt(base + shift) - dims.height;
		frame.push(Placed::new(Sp::from_pt(10.0), y, dims, PlacedKind::Text(st)));
	}
	let mut builder = Builder::new();
	for (i, (kind, probe, page, x, y)) in located.iter().enumerate() {
		let loc = Location(0x1000 + i as u64);
		let label = if *probe { Some(Label::new("probe")) } else { None };
		let elem = Content::Elem(Arc::new(Elem {
			kind:		*kind,
			fields:		Vec::new(),
			label,
			location:	Some(loc),
			span:		Span::detached(),
			guards:		Vec::new(),
			prepared:	true,
		}));
		builder.record(&elem, Position::new(*page, Sp::from_pt(*x), Sp::from_pt(*y + shift)), None);
	}
	builder.page(1, &Value::None);
	let geom = PageGeometry::new(Sp::from_pt(200.0), Sp::from_pt(220.0), Sp::ZERO);
	Ok((Laid {
		intro:		Arc::new(builder.finish()),
		pages:		1,
		passes:		1,
		converged:	true,
	}, vec![Page::new(1, geom, frame)]))
}

#[test]
fn austenite_side_readers_agree_with_the_oracle_on_hand_built_layout() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let lines = [("Alpha beta", 40.0), ("Gamma", 100.0), ("Delta", 150.0), ("Epsilon2 zeta", 190.0)];
	let want_pages = match res!(o.layout(&res!(harness_fixture("baselines")))) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("the oracle could not lay out harness/baselines: {}", e; Invalid)),
	};
	let want_pos = match res!(o.positions(&res!(harness_fixture("positions")))) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("the oracle could not locate harness/positions: {}", e; Invalid)),
	};
	let located = [
		(ElemKind::Heading, false, 1, 20.0, 20.0),
		(ElemKind::Metadata, true, 1, 20.0, want_pos[1].y),
		(ElemKind::Heading, false, 2, 20.0, 20.0),
	];
	let (laid, laid_pages) = res!(hand_laid(&lines, &located, 0.0));
	let mut d = Vec::new();
	layout::compare(&want_pages, &austenite::pages(&laid_pages), &mut d);
	assert!(d.is_empty(), "hand-built layout differs from typst: {:?}", d);
	let pos = match austenite::positions(&laid) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("positions: {}", e; Invalid)),
	};
	let d = harness::compare_positions(&want_pos, &pos);
	assert!(d.is_empty(), "hand-built positions differ from typst: {:?}", d);

	// Moved by 1.5pt, both go red.
	let (moved, moved_pages) = res!(hand_laid(&lines, &located, 1.5));
	let mut d = Vec::new();
	layout::compare(&want_pages, &austenite::pages(&moved_pages), &mut d);
	assert!(!d.is_empty(), "a 1.5pt shift of every baseline must fail level 4");
	let pos = match austenite::positions(&moved) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("positions: {}", e; Invalid)),
	};
	assert!(!harness::compare_positions(&want_pos, &pos).is_empty(), "a 1.5pt shift must fail level 2");
	// An unlabelled metadata element is not a probe, so the rows no longer line up.
	let unlabelled = [located[0], (ElemKind::Metadata, false, 1, 20.0, want_pos[1].y), located[2]];
	let pos = match austenite::positions(&res!(hand_laid(&lines, &unlabelled, 0.0)).0) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("positions: {}", e; Invalid)),
	};
	assert!(!harness::compare_positions(&want_pos, &pos).is_empty(), "a lost probe label must fail level 2");
	Ok(())
}

// `harness/rejects.typ` fails at a place fixed by construction: line 2, column 4 -- the `p` of `panic`,
// since an embedded expression's span leaves out its `#`, and counted in characters, so the two-byte
// `é` before it is one column.
#[test]
fn first_error_reader_and_comparator_agree_with_the_oracle_and_go_red_on_a_difference() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fx = res!(harness_fixture("rejects"));
	assert_eq!(fx.expect, Expect::Rejects, "harness/rejects.typ must carry `oracle: rejects`");
	let said = match res!(o.compiles(&fx)) {
		Ok(())	=> return Err(err!("typst accepts harness/rejects.typ"; Invalid)),
		Err(e)	=> e,
	};
	let (msg, pos) = harness::split_first_error(&said);
	assert!(msg.contains("rejected on purpose"), "message read as `{}`", msg);
	assert_eq!(pos, Some((2, 4)), "position read from `{}`", said);
	let same = (msg.clone(), pos);
	assert!(harness::compare_first_error(&msg, pos, Some(&same)).is_pass());
	let moved = (msg.clone(), Some((2, 5)));
	assert!(!harness::compare_first_error(&msg, pos, Some(&moved)).is_pass(), "a moved error must fail");
	let reworded = ("rejected".to_string(), pos);
	assert!(!harness::compare_first_error(&msg, pos, Some(&reworded)).is_pass(), "a reworded error must fail");
	assert!(!harness::compare_first_error(&msg, pos, None).is_pass(), "accepting it must fail");
	Ok(())
}

#[test]
fn evaluator_conformance_against_the_oracle() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let fixtures = res!(corpus::discover(&Filter::from_env()));
	let limit = harness::timeout();
	let mut board = Board { reports: Vec::new() };
	for fx in &fixtures {
		board.reports.push(res!(harness::check(fx, &o, limit)));
	}
	board.print();
	let report = o.work.join("report.json");
	res!(std::fs::write(&report, board.to_json().render()).map_err(|e| err!(
		"Cannot write {}: {}", report.display(), e; IO, File, Write)));
	println!("[eval-oracle] scoreboard written to {}", report.display());
	let failures = board.failures();
	assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
	Ok(())
}

// The parse-and-evaluate corpus: every `.typ` of five popular packages. There is no oracle for a
// library file, so the claims are Austenite's own: the tree is lossless, and evaluation neither
// panics nor runs away. Error nodes are counted; under `EVAL_ORACLE_STRICT` naming `packages` (or
// `all`) any error node or lossy tree fails.
const PACKAGES: [&str; 5] = ["cetz", "cetz-plot", "fletcher", "tablex", "oxifmt"];

fn package_root(p: &std::path::Path) -> PathBuf {
	let mut d = p.parent();
	while let Some(dir) = d {
		if dir.join("typst.toml").exists() {
			return dir.to_path_buf();
		}
		d = dir.parent();
	}
	p.parent().map(|x| x.to_path_buf()).unwrap_or_default()
}

fn typ_files(dir: &std::path::Path, out: &mut Vec<PathBuf>, depth: usize) {
	if depth > 12 {
		return;
	}
	if let Ok(rd) = std::fs::read_dir(dir) {
		let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
		v.sort();
		for p in v {
			if p.is_dir() {
				typ_files(&p, out, depth + 1);
			} else if p.extension().map(|e| e == "typ").unwrap_or(false) {
				out.push(p);
			}
		}
	}
}

#[test]
fn package_corpus_parses_losslessly_and_evaluates_without_panic() -> Outcome<()> {
	let home = std::env::var("HOME").unwrap_or_default();
	let base = PathBuf::from(home).join(".cache/typst/packages/preview");
	let mut files = Vec::new();
	for p in PACKAGES {
		typ_files(&base.join(p), &mut files, 0);
	}
	if files.is_empty() {
		let skip = std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false);
		assert!(skip, "no package corpus under {} (set EVAL_ORACLE_SKIP=1 to skip explicitly)", base.display());
		println!("[eval-oracle] SKIPPED: no package corpus under {}", base.display());
		return Ok(());
	}
	let strict = harness::strict("packages");
	let limit = harness::timeout();
	let mut faults = Vec::new();
	let mut shortfalls = Vec::new();
	let (mut errors, mut lossy, mut evaluated) = (0usize, 0usize, 0usize);
	for p in &files {
		let text = match std::fs::read_to_string(p) {
			Ok(t)	=> t,
			Err(_)	=> continue,
		};
		let rel = p.strip_prefix(&base).map(|r| r.display().to_string()).unwrap_or_default();
		let src = std::panic::catch_unwind(|| Source::new(FileId(0), p.clone(), text.clone()));
		let src = match src {
			Ok(s)	=> s,
			Err(_)	=> {
				faults.push(fmt!("{}: the parser panicked", rel));
				continue;
			}
		};
		let errs = src.root.errors().len();
		if errs > 0 {
			errors += 1;
			shortfalls.push(fmt!("{}: {} error node(s)", rel, errs));
		}
		if src.root.full_text() != text {
			lossy += 1;
			shortfalls.push(fmt!("{}: the tree does not reproduce the source", rel));
		}
		match austenite::eval_only(p, &package_root(p), limit) {
			RunEnd::Done(out)	=> if out.eval.is_none() {
				evaluated += 1;
			},
			RunEnd::Panic(m)	=> faults.push(fmt!("{}: evaluation panicked: {}", rel, m)),
			RunEnd::Timeout(s)	=> faults.push(fmt!("{}: evaluation ran past {}s", rel, s)),
		}
	}
	println!("[eval-oracle] packages: {} file(s); {} with error nodes, {} lossy, {} evaluated without error",
		files.len(), errors, lossy, evaluated);
	for s in shortfalls.iter().take(10) {
		println!("[eval-oracle]     {}", s);
	}
	assert!(faults.is_empty(), "{} fault(s):\n{}", faults.len(), faults.join("\n"));
	if strict {
		assert!(shortfalls.is_empty(), "{} shortfall(s) in strict mode:\n{}", shortfalls.len(), shortfalls.join("\n"));
	}
	Ok(())
}
