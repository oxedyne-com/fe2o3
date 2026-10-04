//! U6c: grid and table layout against the `typst` oracle. Each case is written once, as data, and rendered
//! both as Typst source (for the installed `typst` 0.15.1) and as the evaluator's own argument values, so
//! the two engines lay out the same grid. Every cell holds a fixed-size block carrying a `metadata` marker;
//! Typst's positions for the markers, and the fills and lines in its SVG, are the expected values.
//!
//! The block flow is not this unit's, so cells are measured and set by a stand-in that reads the size from
//! the cell body and adds the resolved inset, which is exactly what Typst's padded fixed-size block does.
//! Everything under test -- placement, track sizing, groups, fills, lines -- is the grid's own.
//!
//! The rendered sources are kept under `tests/fixtures/eval/grid/`; a case whose source drifts from its
//! fixture fails, and `GRID_FIXTURES_WRITE=1` rewrites them.

use oxedyne_fe2o3_austenite::eval::args::Args;
use oxedyne_fe2o3_austenite::eval::content::{
	construct,
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::lib::grid::fid;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::{
	Alignment,
	Color,
	ColorSpace,
	Dict,
	Fraction,
	HAlign,
	Length,
	Paint,
	Ratio,
	Stroke,
	VAlign,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::grid::{
	plan,
	resolve,
	CellLayout,
	GridPlan,
	Part,
};
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::ir::{
	Dims,
	Node,
	Sp,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

// Case vocabulary

#[derive(Clone, Debug)]
enum V {
	Auto,
	Nil,
	Pt(f64),
	Fr(f64),
	Pct(f64),
	Int(i64),
	Bool(bool),
	Col(&'static str),
	Stroke(f64, &'static str),
	Arr(Vec<V>),
	Dict(Vec<(&'static str, V)>),
	Align(&'static str),
}

type Named = Vec<(&'static str, V)>;

#[derive(Clone, Debug)]
enum Ch {
	C(u32, f64, f64),					// a plain child: id, width, height
	Cell(u32, f64, f64, Named),			// an explicit cell
	HLine(Named),
	VLine(Named),
	Head(Named, Vec<Ch>),
	Foot(Named, Vec<Ch>),
}

struct Case {
	name:		&'static str,
	table:		bool,
	args:		Named,
	children:	Vec<Ch>,
}

fn hex(name: &str) -> &'static str {
	match name {
		"red"		=> "#ff4136",
		"blue"		=> "#0074d9",
		"green"		=> "#2ecc40",
		"yellow"	=> "#ffdc00",
		"aqua"		=> "#7fdbff",
		"olive"		=> "#3d9970",
		_			=> "#000000",
	}
}

fn colour(name: &str) -> Color {
	if name == "black" {
		return Color { space: ColorSpace::Luma, c: [0.0; 4], alpha: 1.0 };
	}
	let h = hex(name);
	let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
	Color { space: ColorSpace::Rgb, c: [byte(1), byte(3), byte(5), 0.0], alpha: 1.0 }
}

fn colour_hex(c: &Color) -> String {
	let b = |v: f32| (v * 255.0).round() as u8;
	match c.space {
		ColorSpace::Luma	=> format!("#{:02x}{:02x}{:02x}", b(c.c[0]), b(c.c[0]), b(c.c[0])),
		_					=> format!("#{:02x}{:02x}{:02x}", b(c.c[0]), b(c.c[1]), b(c.c[2])),
	}
}

impl V {
	fn typst(&self) -> String {
		match self {
			V::Auto				=> "auto".into(),
			V::Nil				=> "none".into(),
			V::Pt(p)			=> format!("{}pt", p),
			V::Fr(f)			=> format!("{}fr", f),
			V::Pct(p)			=> format!("{}%", p),
			V::Int(i)			=> format!("{}", i),
			V::Bool(b)			=> format!("{}", b),
			V::Col(c)			=> c.to_string(),
			V::Align(a)			=> a.to_string(),
			V::Stroke(t, c)		=> format!("{}pt + {}", t, c),
			V::Arr(a)			=> format!("({},)", a.iter().map(|v| v.typst()).collect::<Vec<_>>().join(", ")),
			V::Dict(d)			=> format!("({})", d.iter().map(|(k, v)| format!("{}: {}", k, v.typst()))
				.collect::<Vec<_>>().join(", ")),
		}
	}

	fn value(&self) -> Value {
		match self {
			V::Auto			=> Value::Auto,
			V::Nil			=> Value::None,
			V::Pt(p)		=> Value::Length(Length::pt(*p)),
			V::Fr(f)		=> Value::Fraction(Fraction(*f)),
			V::Pct(p)		=> Value::Ratio(Ratio(p / 100.0)),
			V::Int(i)		=> Value::Int(*i),
			V::Bool(b)		=> Value::Bool(*b),
			V::Col(c)		=> Value::Color(colour(c)),
			V::Align(a)		=> Value::Alignment(match *a {
				"end"		=> Alignment { x: Some(HAlign::End), y: None },
				"bottom"	=> Alignment { x: None, y: Some(VAlign::Bottom) },
				_			=> Alignment::default(),
			}),
			V::Stroke(t, c)	=> Value::Stroke(Arc::new(Stroke {
				paint:		Some(Paint::Color(colour(c))),
				thickness:	Some(Length::pt(*t)),
				..Stroke::default()
			})),
			V::Arr(a)		=> Value::array(a.iter().map(|v| v.value()).collect()),
			V::Dict(d)		=> {
				let mut out = Dict::new();
				for (k, v) in d {
					out.insert(k, v.value());
				}
				Value::dict(out)
			}
		}
	}
}

fn named_typst(n: &Named) -> String {
	n.iter().map(|(k, v)| format!("{}: {}, ", k, v.typst())).collect()
}

impl Ch {
	fn typst(&self, pre: &str) -> String {
		match self {
			Ch::C(id, w, h)				=> format!("c({}, {}, {})", id, w, h),
			Ch::Cell(id, w, h, n)		=> format!("{}.cell({}c({}, {}, {}))", pre, named_typst(n), id, w, h),
			Ch::HLine(n)				=> format!("{}.hline({})", pre, named_typst(n)),
			Ch::VLine(n)				=> format!("{}.vline({})", pre, named_typst(n)),
			Ch::Head(n, cs)				=> format!("{}.header({}{})", pre, named_typst(n),
				cs.iter().map(|c| c.typst(pre)).collect::<Vec<_>>().join(", ")),
			Ch::Foot(n, cs)				=> format!("{}.footer({}{})", pre, named_typst(n),
				cs.iter().map(|c| c.typst(pre)).collect::<Vec<_>>().join(", ")),
		}
	}

	fn value(&self, engine: &mut Engine, table: bool) -> Outcome<Value> {
		let k = |g: ElemKind, t: ElemKind| if table { t } else { g };
		let body = |id: &u32, w: &f64, h: &f64| Value::Content(Content::text(&format!("{}:{}:{}", id, w, h)));
		let build = |engine: &mut Engine, kind: ElemKind, pos: Vec<Value>, n: &Named| -> Outcome<Value> {
			let mut args = Args::new(Span::detached());
			for v in pos {
				args.push(Span::detached(), v);
			}
			for (name, v) in n {
				args.push_named(Span::detached(), *name, v.value());
			}
			Ok(Value::Content(res!(construct(engine, kind, &mut args))))
		};
		match self {
			Ch::C(id, w, h)			=> Ok(body(id, w, h)),
			Ch::Cell(id, w, h, n)	=> build(engine, k(ElemKind::GridCell, ElemKind::TableCell), vec![body(id, w, h)], n),
			Ch::HLine(n)			=> build(engine, k(ElemKind::GridHLine, ElemKind::TableHLine), vec![], n),
			Ch::VLine(n)			=> build(engine, k(ElemKind::GridVLine, ElemKind::TableVLine), vec![], n),
			Ch::Head(n, cs) | Ch::Foot(n, cs) => {
				let mut pos = Vec::new();
				for c in cs {
					pos.push(res!(c.value(engine, table)));
				}
				let kind = match self {
					Ch::Head(..)	=> k(ElemKind::GridHeader, ElemKind::TableHeader),
					_				=> k(ElemKind::GridFooter, ElemKind::TableFooter),
				};
				build(engine, kind, pos, n)
			}
		}
	}
}

impl Case {
	fn typst(&self) -> String {
		let pre = if self.table { "table" } else { "grid" };
		format!(
			"#set page(width: 200pt, height: 400pt, margin: 0pt, fill: none)\n\
			#let c(id, w, h) = block(width: w * 1pt, height: h * 1pt)[#metadata(id)<m>]\n\
			#{}({}{})\n",
			pre, named_typst(&self.args),
			self.children.iter().map(|c| c.typst(pre)).collect::<Vec<_>>().join(", "))
	}

	fn element(&self, engine: &mut Engine) -> Outcome<Content> {
		let mut args = Args::new(Span::detached());
		for (name, v) in &self.args {
			args.push_named(Span::detached(), *name, v.value());
		}
		for c in &self.children {
			let v = res!(c.value(engine, self.table));
			args.push(Span::detached(), v);
		}
		let kind = if self.table { ElemKind::Table } else { ElemKind::Grid };
		construct(engine, kind, &mut args)
	}
}

// The stand-in cell layout

/// Reads `id:w:h` from the cell body and pads it by the resolved inset, as Typst's padded block measures.
struct Blocks;

fn inset_pt(cell: &Content, side: &str) -> f64 {
	match cell.get(fid::CELL_INSET) {
		Some(Value::Dict(d)) => match d.get(side) {
			Some(Value::Length(l))		=> l.abs,
			Some(Value::Relative(r))	=> r.abs.abs,
			_							=> 0.0,
		},
		_ => 0.0,
	}
}

fn body_of(cell: &Content) -> (u32, f64, f64) {
	let text = match cell.get(fid::BODY) {
		Some(Value::Content(c))	=> c.plain_text(),
		_						=> String::new(),
	};
	let parts: Vec<&str> = text.split(':').collect();
	let num = |i: usize| parts.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
	(num(0) as u32, num(1), num(2))
}

impl CellLayout for Blocks {
	fn measure(&mut self, _: &mut Engine, cell: &Content, _: &StyleChain, _: Region) -> Outcome<Dims> {
		let (_, w, h) = body_of(cell);
		Ok(Dims::new(
			Sp::from_pt(w + inset_pt(cell, "left") + inset_pt(cell, "right")),
			Sp::from_pt(h + inset_pt(cell, "top") + inset_pt(cell, "bottom")),
			Sp::ZERO))
	}

	fn layout(&mut self, _: &mut Engine, _: &Content, _: &StyleChain, _: Region) -> Outcome<Vec<Node>> {
		Ok(Vec::new())
	}
}

// The oracle

fn fixtures() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/grid")
}

fn scratch() -> PathBuf {
	PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_grid")
}

/// Writes or checks the case's fixture, then runs `typst` over it. A case Typst must reject is marked
/// so for the structure oracle's corpus.
fn oracle_source(case: &Case, rejects: bool) -> Outcome<PathBuf> {
	let src	= if rejects { format!("// oracle: rejects\n{}", case.typst()) } else { case.typst() };
	let dir	= fixtures();
	let path = dir.join(format!("{}.typ", case.name));
	if std::env::var("GRID_FIXTURES_WRITE").is_ok() {
		res!(std::fs::create_dir_all(&dir).map_err(|e| err!("{}", e; IO)));
		res!(std::fs::write(&path, &src).map_err(|e| err!("{}", e; IO)));
	}
	let kept = res!(std::fs::read_to_string(&path).map_err(|e| err!(
		"fixture {} unreadable ({}); run with GRID_FIXTURES_WRITE=1", path.display(), e; IO)));
	if kept != src {
		return Err(err!("fixture {} is stale; run with GRID_FIXTURES_WRITE=1", path.display(); Invalid));
	}
	Ok(path)
}

fn typst(args: &[&str]) -> Outcome<(bool, String, String)> {
	let out = res!(Command::new("typst").args(args).output().map_err(|e| err!(
		"the typst oracle could not run: {}", e; IO)));
	Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(),
		String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// `[[id, x, y, page], ...]` from `typst eval`, as numbers.
fn numbers(s: &str) -> Vec<Vec<f64>> {
	s.trim().trim_start_matches('[').trim_end_matches(']').split("],[")
		.filter(|r| !r.trim().is_empty())
		.map(|r| r.trim_matches(|c| c == '[' || c == ']').split(',')
			.filter_map(|n| n.trim().parse::<f64>().ok()).collect())
		.collect()
}

struct Drawn {
	fills:	Vec<(String, f64, f64, f64, f64)>,		// colour, x, y, w, h
	lines:	Vec<(bool, String, f64, f64, f64, f64)>,	// vertical, colour, width, at, from, to
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
	let key = format!(" {}=\"", name);
	let i = tag.find(&key)? + key.len();
	let j = tag[i..].find('"')? + i;
	Some(&tag[i..j])
}

/// The fills and lines of Typst's SVG: every path is `translate(x y)` then `M 0 0` and axis moves.
fn svg_drawn(svg: &str) -> Drawn {
	let mut d = Drawn { fills: Vec::new(), lines: Vec::new() };
	for tag in svg.split('<').filter(|t| t.starts_with("path ")) {
		let (tx, ty) = match attr(tag, "transform") {
			Some(t) => {
				let n: Vec<f64> = t.trim_start_matches("translate(").trim_end_matches(')')
					.split(' ').filter_map(|v| v.parse().ok()).collect();
				(n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0))
			}
			None => (0.0, 0.0),
		};
		let path = attr(tag, "d").unwrap_or("");
		let steps: Vec<(char, f64)> = {
			let mut out = Vec::new();
			let body = path.trim_start_matches("M 0 0");
			let mut cur: Option<char> = None;
			let mut num = String::new();
			for ch in body.chars() {
				if ch == 'v' || ch == 'h' || ch == 'Z' {
					if let (Some(c), Ok(n)) = (cur, num.trim().parse::<f64>()) {
						out.push((c, n));
					}
					num.clear();
					cur = if ch == 'Z' { None } else { Some(ch) };
				} else {
					num.push(ch);
				}
			}
			if let (Some(c), Ok(n)) = (cur, num.trim().parse::<f64>()) {
				out.push((c, n));
			}
			out
		};
		match (attr(tag, "stroke"), attr(tag, "fill")) {
			(Some(col), _) => {
				let w: f64 = attr(tag, "stroke-width").and_then(|w| w.parse().ok()).unwrap_or(1.0);
				if let Some((axis, len)) = steps.first() {
					match axis {
						'v'	=> d.lines.push((true, col.to_string(), w, tx, ty, ty + len)),
						_	=> d.lines.push((false, col.to_string(), w, ty, tx, tx + len)),
					}
				}
			}
			(None, Some(col)) if steps.len() >= 2 => {
				let h = steps[0].1;
				let w = steps[1].1;
				d.fills.push((col.to_string(), tx, ty, w, h));
			}
			_ => (),
		}
	}
	d
}

/// Merges collinear, overlapping or touching segments of one pen into maximal ones, and sorts them, so
/// Typst's merged runs and the grid's per-group runs compare as the same ink.
fn normalise(mut v: Vec<(bool, String, f64, f64, f64, f64)>) -> Vec<(bool, String, f64, f64, f64, f64)> {
	v.sort_by(|a, b| (a.0, &a.1, a.3 as i64 * 1000, a.4 as i64 * 1000).cmp(&(b.0, &b.1, b.3 as i64 * 1000, b.4 as i64 * 1000))
		.then(a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal))
		.then(a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal)));
	let mut out: Vec<(bool, String, f64, f64, f64, f64)> = Vec::new();
	for s in v {
		if let Some(l) = out.last_mut() {
			if l.0 == s.0 && l.1 == s.1 && (l.2 - s.2).abs() < 0.01 && (l.3 - s.3).abs() < 0.01 && s.4 <= l.5 + 0.01 {
				if s.5 > l.5 {
					l.5 = s.5;
				}
				continue;
			}
		}
		out.push(s);
	}
	out
}

fn close(a: f64, b: f64) -> bool { (a - b).abs() < 0.02 }

fn ours_drawn(plan: &GridPlan) -> Drawn {
	let mut d = Drawn { fills: Vec::new(), lines: Vec::new() };
	for g in &plan.groups {
		for f in &g.fills {
			if let Paint::Color(c) = &f.paint {
				d.fills.push((colour_hex(c), f.x.to_pt(), f.y.to_pt(), f.w.to_pt(), f.h.to_pt()));
			}
		}
		for s in &g.lines {
			if let Paint::Color(c) = &s.pen.paint {
				d.lines.push((s.vertical, colour_hex(c), s.pen.thickness.to_pt(), s.at.to_pt(), s.from.to_pt(), s.to.to_pt()));
			}
		}
	}
	d
}

fn engine() -> Engine { Engine::new(World::new(PathBuf::from("."))) }

fn region() -> Region { Region::new(Sp::from_pt(200.0), Sp::from_pt(400.0)) }

/// Lays the case out both ways and compares positions, fills and lines. Returns the plan for extra checks.
fn check(case: &Case) -> Outcome<GridPlan> {
	let src = res!(oracle_source(case, false));
	let src_s = src.to_string_lossy().into_owned();
	let (ok, out, err) = res!(typst(&["eval",
		"query(<m>).map(m => (m.value, m.location().position().x.pt(), m.location().position().y.pt(), m.location().page()))",
		"--in", &src_s]));
	if !ok {
		return Err(err!("typst failed on {}: {}", case.name, err; Invalid));
	}
	let expected = numbers(&out);
	res!(std::fs::create_dir_all(scratch()).map_err(|e| err!("{}", e; IO)));
	let svg_path = scratch().join(format!("{}.svg", case.name));
	let (ok, _, err) = res!(typst(&["compile", &src_s, &svg_path.to_string_lossy()]));
	if !ok {
		return Err(err!("typst compile failed on {}: {}", case.name, err; Invalid));
	}
	let svg = res!(std::fs::read_to_string(&svg_path).map_err(|e| err!("{}", e; IO)));

	let mut engine = engine();
	let elem	= res!(case.element(&mut engine));
	let styles	= StyleChain::root();
	let grid	= res!(resolve(&mut engine, &elem, &styles));
	let plan	= res!(plan(&mut engine, &grid, &styles, region(), &mut Blocks));

	// Positions: each marker sits at its cell's origin plus the inset.
	let mut ours = Vec::new();
	for g in &plan.groups {
		for pc in &g.cells {
			let cell = &grid.cells[pc.index];
			if body_of(&cell.elem) == (0, 0.0, 0.0) {
				continue;	// an empty cell filling a free slot carries no marker
			}
			let (id, _, _) = body_of(&cell.elem);
			ours.push((id, pc.x.to_pt() + inset_pt(&cell.elem, "left"), pc.y.to_pt() + inset_pt(&cell.elem, "top")));
		}
	}
	assert_eq!(ours.len(), expected.len(), "{}: cell count, ours {:?} typst {:?}", case.name, ours, expected);
	for e in &expected {
		let id = e[0] as u32;
		assert!(close(e[3], 1.0), "{}: marker {} on page {}", case.name, id, e[3]);
		match ours.iter().find(|o| o.0 == id) {
			Some(o) => assert!(close(o.1, e[1]) && close(o.2, e[2]),
				"{}: cell {} at ({}, {}), typst ({}, {})", case.name, id, o.1, o.2, e[1], e[2]),
			None => panic!("{}: cell {} missing", case.name, id),
		}
	}

	let theirs	= svg_drawn(&svg);
	let mine	= ours_drawn(&plan);
	let mut tf = theirs.fills.clone();
	let mut of = mine.fills.clone();
	let key = |f: &(String, f64, f64, f64, f64)| (f.0.clone(), (f.1 * 100.0) as i64, (f.2 * 100.0) as i64);
	tf.sort_by_key(key);
	of.sort_by_key(key);
	assert_eq!(tf.len(), of.len(), "{}: fills, ours {:?} typst {:?}", case.name, of, tf);
	for (a, b) in tf.iter().zip(&of) {
		assert!(a.0 == b.0 && close(a.1, b.1) && close(a.2, b.2) && close(a.3, b.3) && close(a.4, b.4),
			"{}: fill ours {:?} typst {:?}", case.name, b, a);
	}
	let tl = normalise(theirs.lines);
	let ol = normalise(mine.lines);
	assert_eq!(tl.len(), ol.len(), "{}: lines,\n ours  {:?}\n typst {:?}", case.name, ol, tl);
	for (a, b) in tl.iter().zip(&ol) {
		assert!(a.0 == b.0 && a.1 == b.1 && close(a.2, b.2) && close(a.3, b.3) && close(a.4, b.4) && close(a.5, b.5),
			"{}: line ours {:?} typst {:?}", case.name, b, a);
	}
	Ok(plan)
}

/// A case Typst refuses: the evaluator must refuse it with Typst's message.
fn check_error(case: &Case) -> Outcome<()> {
	let src = res!(oracle_source(case, true));
	let (ok, _, err) = res!(typst(&["compile", &src.to_string_lossy(), &scratch().join("err.pdf").to_string_lossy()]));
	assert!(!ok, "{}: typst accepted it", case.name);
	let theirs = err.lines().next().unwrap_or("").trim_start_matches("error: ").to_string();
	let mut engine = engine();
	let elem = res!(case.element(&mut engine));
	let got = resolve(&mut engine, &elem, &StyleChain::root());
	assert!(got.is_err(), "{}: evaluator accepted it", case.name);
	let ours = engine.diags.last().map(|d| d.message.clone()).unwrap_or_default();
	assert_eq!(ours, theirs, "{}: message", case.name);
	Ok(())
}

fn n(v: Vec<(&'static str, V)>) -> Named { v }

// Cases

#[test]
fn tracks_auto_fixed_fraction_with_gutters() -> Outcome<()> {
	res!(check(&Case { name: "tracks", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Auto, V::Pt(30.0), V::Fr(1.0)])),
			("column-gutter", V::Pt(3.0)), ("row-gutter", V::Pt(2.0))]),
		children: vec![Ch::C(0, 20.0, 10.0), Ch::C(1, 10.0, 15.0), Ch::C(2, 5.0, 5.0),
			Ch::Cell(3, 70.0, 4.0, n(vec![("colspan", V::Int(2))])), Ch::C(4, 1.0, 1.0), Ch::C(5, 1.0, 1.0)],
	}));
	// Fractions share by weight after relative (percent) and auto columns.
	res!(check(&Case { name: "fractions", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Fr(1.0), V::Pct(25.0), V::Fr(2.0), V::Auto])), ("gutter", V::Pt(4.0))]),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::C(1, 5.0, 5.0), Ch::C(2, 5.0, 5.0), Ch::C(3, 12.0, 7.0),
			Ch::C(4, 5.0, 9.0)],
	}));
	// A colspan over two autos puts its excess on the later one; a line after it sits past its span.
	res!(check(&Case { name: "colspan_autos", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Auto, V::Auto, V::Auto]))]),
		children: vec![Ch::C(0, 10.0, 5.0), Ch::C(1, 20.0, 5.0), Ch::C(2, 5.0, 5.0),
			Ch::Cell(3, 60.0, 5.0, n(vec![("colspan", V::Int(2))])),
			Ch::VLine(n(vec![("stroke", V::Col("red"))])), Ch::C(4, 5.0, 5.0)],
	}));
	// Integer columns, and auto columns wider than the region shrink fairly.
	res!(check(&Case { name: "shrink", table: false,
		args: n(vec![("columns", V::Int(3))]),
		children: vec![Ch::C(0, 150.0, 5.0), Ch::C(1, 20.0, 5.0), Ch::C(2, 120.0, 5.0)],
	}));
	Ok(())
}

#[test]
fn rows_relative_auto_fraction_and_rowspan_excess() -> Outcome<()> {
	res!(check(&Case { name: "rows", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Pt(30.0), V::Pt(30.0)])),
			("rows", V::Arr(vec![V::Pt(12.0), V::Auto, V::Auto, V::Fr(1.0), V::Pt(8.0)]))]),
		children: vec![Ch::C(0, 5.0, 20.0), Ch::C(1, 5.0, 5.0),
			Ch::Cell(2, 5.0, 50.0, n(vec![("rowspan", V::Int(2))])), Ch::C(3, 5.0, 7.0), Ch::C(4, 5.0, 9.0),
			Ch::C(5, 5.0, 5.0), Ch::C(6, 5.0, 5.0), Ch::C(7, 5.0, 5.0), Ch::C(8, 5.0, 5.0)],
	}));
	// The last given row size repeats.
	res!(check(&Case { name: "rows_repeat", table: false,
		args: n(vec![("columns", V::Int(2)), ("rows", V::Arr(vec![V::Pt(9.0), V::Pt(13.0)]))]),
		children: (0..8).map(|i| Ch::C(i, 4.0, 4.0)).collect(),
	}));
	Ok(())
}

#[test]
fn placement_fixed_partial_and_spans() -> Outcome<()> {
	res!(check(&Case { name: "placement", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Pt(10.0), V::Pt(10.0), V::Pt(10.0)])), ("rows", V::Pt(10.0))]),
		children: vec![Ch::Cell(0, 5.0, 5.0, n(vec![("x", V::Int(0)), ("y", V::Int(2))])),
			Ch::Cell(1, 5.0, 5.0, n(vec![("x", V::Int(1))])),
			Ch::Cell(2, 5.0, 5.0, n(vec![("y", V::Int(1))])),
			Ch::C(3, 5.0, 5.0), Ch::C(4, 5.0, 5.0),
			Ch::Cell(5, 5.0, 5.0, n(vec![("x", V::Int(0))])),
			Ch::Cell(6, 5.0, 5.0, n(vec![("colspan", V::Int(2)), ("rowspan", V::Int(2))])),
			Ch::C(7, 5.0, 5.0), Ch::C(8, 5.0, 5.0), Ch::C(9, 5.0, 5.0)],
	}));
	Ok(())
}

#[test]
fn headers_and_footers_place_and_group() -> Outcome<()> {
	let plan = res!(check(&Case { name: "sections", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Pt(10.0), V::Pt(10.0), V::Pt(10.0)])), ("rows", V::Pt(10.0)),
			("row-gutter", V::Pt(3.0))]),
		children: vec![Ch::C(0, 5.0, 5.0),
			Ch::Head(n(vec![]), vec![Ch::C(1, 5.0, 5.0), Ch::Cell(2, 5.0, 5.0, n(vec![("x", V::Int(2))]))]),
			Ch::C(3, 5.0, 5.0), Ch::Cell(4, 5.0, 5.0, n(vec![("rowspan", V::Int(2))])), Ch::C(5, 5.0, 5.0),
			Ch::Head(n(vec![("level", V::Int(2))]), vec![Ch::C(6, 5.0, 5.0)]),
			Ch::C(7, 5.0, 5.0),
			Ch::Foot(n(vec![]), vec![Ch::C(8, 5.0, 5.0)])],
	}));
	let parts: Vec<Part> = plan.groups.iter().map(|g| g.part).collect();
	assert_eq!(parts, vec![Part::Body, Part::Header { level: 1, repeat: true }, Part::Body,
		Part::Header { level: 2, repeat: true }, Part::Body, Part::Footer { repeat: true }]);
	// The rowspan welds its two rows (and the gutter between them) into one group.
	assert_eq!((plan.groups[2].start, plan.groups[2].end), (4, 7));
	Ok(())
}

#[test]
fn table_strokes_fills_and_lines() -> Outcome<()> {
	// Default table stroke and inset, fill cycling by column, a cell's own stroke and fill.
	res!(check(&Case { name: "table_basic", table: true,
		args: n(vec![("columns", V::Arr(vec![V::Auto, V::Pt(30.0), V::Auto])),
			("fill", V::Arr(vec![V::Col("yellow"), V::Nil, V::Col("aqua")]))]),
		children: vec![Ch::C(0, 10.0, 5.0), Ch::C(1, 5.0, 5.0), Ch::C(2, 7.0, 5.0),
			Ch::Cell(3, 5.0, 5.0, n(vec![("stroke", V::Stroke(2.0, "blue")), ("fill", V::Col("red"))])),
			Ch::Cell(4, 5.0, 15.0, n(vec![("colspan", V::Int(2)), ("inset", V::Dict(vec![("left", V::Pt(1.0)), ("y", V::Pt(2.0))]))])),
			Ch::Cell(5, 5.0, 5.0, n(vec![("rowspan", V::Int(2))])), Ch::C(6, 5.0, 5.0), Ch::C(7, 5.0, 5.0)],
	}));
	// Explicit lines, auto positioned and ranged, over a stroke dictionary; `none` removes a stretch.
	res!(check(&Case { name: "table_lines", table: true,
		args: n(vec![("columns", V::Arr(vec![V::Pt(20.0), V::Pt(20.0), V::Pt(20.0)])), ("inset", V::Pt(2.0)),
			("stroke", V::Dict(vec![("x", V::Stroke(0.5, "black")), ("bottom", V::Stroke(1.5, "olive"))]))]),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::HLine(n(vec![("stroke", V::Col("red"))])), Ch::C(1, 5.0, 5.0),
			Ch::VLine(n(vec![("stroke", V::Stroke(1.0, "blue"))])), Ch::C(2, 5.0, 5.0), Ch::C(3, 5.0, 5.0),
			Ch::C(9, 5.0, 5.0), Ch::C(10, 5.0, 5.0),
			Ch::HLine(n(vec![("start", V::Int(1)), ("end", V::Int(2)), ("stroke", V::Stroke(2.0, "green"))])),
			Ch::VLine(n(vec![("x", V::Int(1)), ("stroke", V::Nil)])),
			Ch::Cell(4, 5.0, 5.0, n(vec![("colspan", V::Int(3))])), Ch::C(5, 5.0, 5.0)],
	}));
	// Two grid-level strokes meeting at an edge: the later cell's wins the tie.
	res!(check(&Case { name: "table_tie", table: true,
		args: n(vec![("columns", V::Arr(vec![V::Pt(15.0), V::Pt(15.0)])), ("inset", V::Pt(1.0)),
			("stroke", V::Dict(vec![("top", V::Stroke(2.0, "red")), ("bottom", V::Stroke(1.5, "olive")),
				("left", V::Col("blue")), ("right", V::Stroke(3.0, "green"))]))]),
		children: (0..4).map(|i| Ch::C(i, 5.0, 5.0)).collect(),
	}));
	// Gutters frame every cell separately; a `position: end` line sits before the gutter.
	res!(check(&Case { name: "table_gutter", table: true,
		args: n(vec![("columns", V::Arr(vec![V::Pt(10.0), V::Pt(10.0)])), ("rows", V::Pt(10.0)), ("inset", V::Pt(0.0)),
			("gutter", V::Pt(4.0)), ("fill", V::Col("blue"))]),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::C(1, 5.0, 5.0),
			Ch::Cell(2, 5.0, 5.0, n(vec![("colspan", V::Int(2))])),
			Ch::HLine(n(vec![("stroke", V::Col("red"))])),
			Ch::VLine(n(vec![("x", V::Int(1)), ("stroke", V::Col("green"))])),
			Ch::VLine(n(vec![("x", V::Int(1)), ("position", V::Align("end")), ("stroke", V::Col("olive"))]))],
	}));
	Ok(())
}

#[test]
fn refusals_match_typst() -> Outcome<()> {
	let cols = || n(vec![("columns", V::Arr(vec![V::Pt(10.0), V::Pt(10.0), V::Pt(10.0)]))]);
	res!(check_error(&Case { name: "err_colspan", table: false, args: cols(),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::Cell(1, 5.0, 5.0, n(vec![("colspan", V::Int(3))]))] }));
	res!(check_error(&Case { name: "err_second", table: false, args: cols(),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::Cell(1, 5.0, 5.0, n(vec![("x", V::Int(0)), ("y", V::Int(0))]))] }));
	res!(check_error(&Case { name: "err_row_full", table: false, args: cols(),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::C(1, 5.0, 5.0), Ch::C(2, 5.0, 5.0), Ch::Cell(3, 5.0, 5.0, n(vec![("y", V::Int(0))]))] }));
	res!(check_error(&Case { name: "err_footer", table: false, args: cols(),
		children: vec![Ch::Foot(n(vec![]), vec![Ch::C(0, 5.0, 5.0)]), Ch::C(1, 5.0, 5.0)] }));
	res!(check_error(&Case { name: "err_span", table: false, args: cols(),
		children: vec![Ch::C(0, 5.0, 5.0), Ch::Cell(1, 5.0, 5.0, n(vec![("x", V::Int(2)), ("y", V::Int(1))])),
			Ch::Cell(2, 5.0, 5.0, n(vec![("colspan", V::Int(2)), ("rowspan", V::Int(2))])),
			Ch::Cell(3, 5.0, 5.0, n(vec![("colspan", V::Int(2)), ("rowspan", V::Int(2))]))] }));
	res!(check_error(&Case { name: "err_column", table: false, args: cols(),
		children: vec![Ch::Cell(0, 5.0, 5.0, n(vec![("x", V::Int(3))]))] }));
	Ok(())
}

#[test]
fn construction_normalises_as_typst_stores() -> Outcome<()> {
	// `typst eval 'grid(columns: 3, gutter: 2pt, [a]).fields()'` gives three autos, both gutters as
	// `0% + 2pt`, and the child wrapped in a cell.
	let mut engine = engine();
	let mut args = Args::new(Span::detached());
	args.push_named(Span::detached(), "columns", Value::Int(3));
	args.push_named(Span::detached(), "gutter", Value::Length(Length::pt(2.0)));
	args.push(Span::detached(), Value::Content(Content::text("a")));
	let g = res!(construct(&mut engine, ElemKind::Grid, &mut args));
	match g.field("columns") {
		Some(Value::Array(a)) => assert!(a.len() == 3 && a.iter().all(|v| v.is_auto())),
		other => panic!("columns: {:?}", other),
	}
	for name in ["column-gutter", "row-gutter"] {
		match g.field(name) {
			Some(Value::Array(a)) => assert!(matches!(a.first(),
				Some(Value::Relative(r)) if r.rel.0 == 0.0 && r.abs.abs == 2.0)),
			other => panic!("{}: {:?}", name, other),
		}
	}
	match g.field("children") {
		Some(Value::Array(a)) => match a.first() {
			Some(Value::Content(c)) => assert!(c.is(ElemKind::GridCell) && c.plain_text() == "a"),
			other => panic!("child: {:?}", other),
		},
		other => panic!("children: {:?}", other),
	}
	// A grid cell inside a table is refused with Typst's wording.
	let mut args = Args::new(Span::detached());
	args.push(Span::detached(), Value::Content(Content::text("a")));
	let cell = res!(construct(&mut engine, ElemKind::GridCell, &mut args));
	let mut args = Args::new(Span::detached());
	args.push(Span::detached(), Value::Content(cell));
	assert!(construct(&mut engine, ElemKind::Table, &mut args).is_err());
	assert_eq!(engine.diags.last().map(|d| d.message.clone()).unwrap_or_default(),
		"cannot use `grid.cell` as a table cell");
	Ok(())
}

#[test]
fn lowering_welds_headers_and_footers_and_breaks_between_groups() -> Outcome<()> {
	use oxedyne_fe2o3_austenite::flow::grid::lower;
	use oxedyne_fe2o3_austenite::ir::Penalty;
	// Positions are the oracle's (the `sections` case); this checks the vertical material built from them.
	let case = Case { name: "lowering", table: false,
		args: n(vec![("columns", V::Arr(vec![V::Pt(10.0), V::Pt(10.0)])), ("rows", V::Pt(10.0)),
			("row-gutter", V::Pt(2.0))]),
		children: vec![Ch::Head(n(vec![]), vec![Ch::C(1, 5.0, 5.0), Ch::C(2, 5.0, 5.0)]),
			Ch::C(3, 5.0, 5.0), Ch::Cell(4, 5.0, 5.0, n(vec![("rowspan", V::Int(2))])), Ch::C(5, 5.0, 5.0),
			Ch::Foot(n(vec![]), vec![Ch::C(6, 5.0, 5.0), Ch::C(7, 5.0, 5.0)])] };
	let mut engine = engine();
	let elem	= res!(case.element(&mut engine));
	let styles	= StyleChain::root();
	let grid	= res!(resolve(&mut engine, &elem, &styles));
	let plan	= res!(plan(&mut engine, &grid, &styles, region(), &mut Blocks));
	let nodes	= res!(lower(&mut engine, &grid, &plan));
	let shape: Vec<String> = nodes.iter().map(|nd| match nd {
		Node::VBox(b)			=> format!("box {}x{}", b.dims.width.to_pt(), b.dims.height.to_pt()),
		Node::RepeatHead(Some(b))	=> format!("repeat {}", b.dims.height.to_pt()),
		Node::RepeatHead(None)	=> "disarm".to_string(),
		Node::Penalty(p) if p.cost >= Penalty::INFINITY	=> "weld".to_string(),
		Node::Glue(g)			=> format!("glue {}", g.natural.to_pt()),
		other					=> format!("{:?}", other),
	}).collect();
	assert_eq!(shape, vec![
		"box 20x10", "repeat 10", "weld", "glue 2",	// header, armed, welded to the first row
		"box 20x22",								// two rows and their gutter, welded by the rowspan
		"weld", "glue 2", "box 20x10",				// the footer, welded to the last row
		"disarm",
	]);
	Ok(())
}
