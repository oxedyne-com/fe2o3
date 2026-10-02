//! U7: equations laid out by `flow::math` against the `typst` 0.15 oracle. Each fixture under
//! `tests/fixtures/eval/math/` holds maths alone; it is evaluated, realised and laid out here, and
//! compiled by `typst` on a page that fits the equation exactly, at New Computer Modern Math's regular
//! weight (the face this crate embeds; Typst's default is its book weight, whose glyphs sit at the same
//! positions). Every glyph's ink box and every rule's box in Typst's SVG must be matched by one in
//! Austenite's layout, and the box sizes must agree.
//!
//! A fixture's first line may be `// needs: <field>` naming a schema field of another unit (for
//! instance `text.fill`); the fixture is reported pending until that field exists. A missing `typst`
//! fails the suite unless `EVAL_ORACLE_SKIP=1` is set.

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::eval_source as eval_module;
use oxedyne_fe2o3_austenite::eval::intro::Builder;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::math::layout_equation;
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::ir::{
	DrawOp,
	LeafKind,
	Node,
	Sp,
};
use oxedyne_fe2o3_austenite::ledger::Position;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::transform::Transform;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

const TOL: f64 = 0.02;	// points

type BoxF = [f64; 4];	// x0, y0, x1, y1, points, y down

fn fixture_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/math")
}

fn work_dir() -> Outcome<PathBuf> {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_math");
	res!(std::fs::create_dir_all(&d).map_err(|e| err!(e, "work dir"; IO)));
	Ok(d)
}

fn typst_available() -> bool {
	Command::new("typst").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

const PRELUDE: &str = "#set page(width: auto, height: auto, margin: 0pt)\n#show math.equation: set text(weight: 400)\n";

// A construct another unit has still to implement stops a layout with an `Unimplemented` error: the
// fixture waits for it, and `EVAL_ORACLE_STRICT=math` turns the wait into a failure.
fn waiting_on(e: &Error<ErrTag>) -> Option<String> {
	if e.tags().contains(&ErrTag::Unimplemented) {
		Some(e.msgs().last().cloned().unwrap_or_default())
	} else {
		None
	}
}

fn strict() -> bool {
	std::env::var("EVAL_ORACLE_STRICT").map(|v| v.split(',').any(|a| a.trim() == "math")).unwrap_or(false)
}

// Austenite

fn austenite(name: &str, src: &str) -> Outcome<(Vec<BoxF>, (f64, f64))> { austenite_nth(name, src, 0) }

// The `nth` equation of the document, laid out after a second pass, so a number counts the equations
// before it as the previous pass saw them.
fn austenite_nth(name: &str, src: &str, nth: usize) -> Outcome<(Vec<BoxF>, (f64, f64))> {
	let dir = res!(work_dir());
	let mut e = Engine::new(World::new(dir.clone()));
	let id = res!(e.world.add_source(dir.join(fmt!("{}.typ", name)), src.to_string()));
	let module = match eval_module(&mut e, id) {
		Ok(m)	=> m,
		Err(err)	=> return Err(err!("eval {}: {} {:?}", name, err, e.diags; Invalid)),
	};
	// An inline equation alone realises as a paragraph's content; until paragraphs have their schema,
	// such a fixture is realised in inline mode instead. A leading directive (`#show`, `#set`) or comment
	// does not decide it.
	let first = src.lines().map(|l| l.trim_start()).find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"));
	let mode = if first.map(|l| l.starts_with("$ ")).unwrap_or(false) { RealiseMode::Flow } else { RealiseMode::Inline };
	let mut pairs = match realise(&mut e, &module.content, &StyleChain::root(), mode) {
		Ok(p)	=> p,
		Err(err)	=> return Err(err!("realise {}: {} {:?}", name, err, e.diags; Invalid)),
	};
	// A number reads the equation counter at its own location, which only the previous pass's introspector
	// holds, so a numbered equation needs the second pass the fixpoint would make.
	if nth > 0 || src.contains("numbering") {
		// The second pass sees the first's equations, as the fixpoint's second pass does.
		let elems: Vec<Content> = pairs.iter().filter(|p| p.content.is(ElemKind::Equation)).map(|p| p.content.clone()).collect();
		let mut builder = Builder::new();
		for (i, c) in elems.iter().enumerate() {
			builder.record(c, Position::new(1, Sp::ZERO, Sp(i as i32)), None);
		}
		e.intro = Arc::new(builder.finish());
		e.locator.reset();
		pairs = match realise(&mut e, &module.content, &StyleChain::root(), mode) {
			Ok(p)	=> p,
			Err(err)	=> return Err(err!("second pass {}: {} {:?}", name, err, e.diags; Invalid)),
		};
	}
	let (eq, styles) = match find_equation(&pairs, nth) {
		Some(x)	=> x,
		None	=> return Err(err!("{}: no equation {} realised", name, nth + 1; Missing)),
	};
	let region = Region::new(Sp(i32::MAX), Sp(i32::MAX));
	let node = match layout_equation(&mut e, &eq, &styles, region) {
		Ok(n)	=> n,
		Err(err)	=> return Err(err!(err, "layout {}: {:?}", name, e.diags; Invalid)),
	};
	let (w, h, d) = match &node {
		Node::HBox(b) | Node::VBox(b)	=> (b.dims.width.to_pt(), b.dims.height.to_pt(), b.dims.depth.to_pt()),
		_								=> (0.0, 0.0, 0.0),
	};
	let mut boxes = Vec::new();
	res!(walk(&node, 0.0, 0.0, &mut boxes));
	Ok((boxes, (w, h + d)))
}

fn find_equation(pairs: &[oxedyne_fe2o3_austenite::eval::realise::Pair], nth: usize) -> Option<(Content, StyleChain)> {
	pairs.iter().filter(|p| p.content.is(ElemKind::Equation)).nth(nth).map(|p| (p.content.clone(), p.styles.clone()))
}

// Walks a node with its top-left at (x, y), collecting ink boxes.
fn walk(node: &Node, x: f64, y: f64, out: &mut Vec<BoxF>) -> Outcome<()> {
	match node {
		Node::HBox(b) => {
			let base = y + b.dims.height.to_pt();
			let mut cx = x;
			for c in &b.list {
				match c {
					Node::Glue(g)	=> cx += g.natural.to_pt(),
					Node::Leaf(l)	=> {
						let top = base - l.dims.height.to_pt() + l.shift.to_pt();
						res!(leaf(l, cx, top, out));
						cx += l.dims.width.to_pt();
					}
					Node::HBox(i) | Node::VBox(i) => {
						res!(walk(c, cx, base - i.dims.height.to_pt(), out));
						cx += i.dims.width.to_pt();
					}
					Node::Frame(f) => {
						res!(walk(c, cx, base - f.dims.height.to_pt(), out));
						cx += f.dims.width.to_pt();
					}
					_ => (),
				}
			}
		}
		Node::VBox(b) => {
			let mut cy = y;
			for c in &b.list {
				match c {
					Node::Glue(g)	=> cy += g.natural.to_pt(),
					Node::Leaf(l)	=> {
						res!(leaf(l, x, cy + l.shift.to_pt(), out));
						cy += l.dims.vextent().to_pt();
					}
					Node::HBox(i) | Node::VBox(i) => {
						res!(walk(c, x, cy, out));
						cy += i.dims.vextent().to_pt();
					}
					Node::Frame(f) => {
						res!(walk(c, x, cy, out));
						cy += f.dims.vextent().to_pt();
					}
					_ => (),
				}
			}
		}
		Node::Leaf(l) => res!(leaf(l, x, y + l.shift.to_pt(), out)),
		// A number is laid as a block, which the block flow hands back as a frame of positioned children.
		Node::Frame(f) => for (ox, oy, c) in &f.items {
			res!(walk(c, x + ox.to_pt(), y + oy.to_pt(), out));
		},
		_ => (),
	}
	Ok(())
}

fn leaf(l: &oxedyne_fe2o3_austenite::ir::Leaf, x: f64, top: f64, out: &mut Vec<BoxF>) -> Outcome<()> {
	match &l.kind {
		LeafKind::Text(shaped) => {
			let base = top + l.dims.height.to_pt();
			for g in &shaped.run().glyphs {
				let path = res!(shaped.outline(g));
				if let Some(b) = path.bounds(&Transform::IDENTITY) {
					let ox = x + g.x as f64;
					let oy = base - g.y as f64;
					out.push([ox + b.x0 as f64, oy - b.y1 as f64, ox + b.x1 as f64, oy - b.y0 as f64]);
				}
			}
		}
		LeafKind::Graphic(gr) => for op in &gr.ops {
			if let DrawOp::Fill { path, .. } = op {
				if let Some(b) = path.bounds(&Transform::IDENTITY) {
					out.push([x + b.x0 as f64, top + b.y0 as f64, x + b.x1 as f64, top + b.y1 as f64]);
				}
			}
		},
		LeafKind::Rule => {
			out.push([x, top, x + l.dims.width.to_pt(), top + l.dims.vextent().to_pt()]);
		}
		_ => (),
	}
	Ok(())
}

// The oracle's SVG

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
	let key = fmt!(" {}=\"", name);
	let i = tag.find(&key)? + key.len();
	let j = tag[i..].find('"')? + i;
	Some(&tag[i..j])
}

fn nums(s: &str) -> Vec<f64> {
	s.split(|c: char| c == ' ' || c == ',' || c == '(' || c == ')')
		.filter_map(|t| t.parse::<f64>().ok())
		.collect()
}

// An SVG transform attribute as an affine map (a, b, c, d, e, f).
fn transform(s: Option<&str>) -> [f64; 6] {
	let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
	let s = match s {
		Some(s)	=> s,
		None	=> return m,
	};
	for part in s.split(')').filter(|p| !p.trim().is_empty()) {
		let n = nums(part);
		let t = if part.contains("matrix") && n.len() == 6 {
			[n[0], n[1], n[2], n[3], n[4], n[5]]
		} else if part.contains("translate") {
			[1.0, 0.0, 0.0, 1.0, n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0)]
		} else if part.contains("scale") {
			let sx = n.first().copied().unwrap_or(1.0);
			[sx, 0.0, 0.0, n.get(1).copied().unwrap_or(sx), 0.0, 0.0]
		} else {
			continue;
		};
		m = compose(m, t);
	}
	m
}

// `outer` applied after `inner`.
fn compose(outer: [f64; 6], inner: [f64; 6]) -> [f64; 6] {
	let [a, b, c, d, e, f] = outer;
	let [a2, b2, c2, d2, e2, f2] = inner;
	[a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f]
}

fn apply(m: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
	(m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn bbox_of(points: &[(f64, f64)], m: &[f64; 6]) -> Option<BoxF> {
	let mut out: Option<BoxF> = None;
	for &(x, y) in points {
		let (px, py) = apply(m, x, y);
		out = Some(match out {
			None	=> [px, py, px, py],
			Some(b)	=> [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)],
		});
	}
	out
}

// The points of an SVG path, control points included, as the font outline's control box has them.
fn path_points(d: &str) -> Vec<(f64, f64)> {
	let mut pts = Vec::new();
	let mut cur = (0.0f64, 0.0f64);
	let mut start = cur;
	let mut cmd = 'M';
	let mut pending: Option<(f64, f64)> = None;	// a move counts only once something is drawn from it
	let mut toks: Vec<String> = Vec::new();
	let mut num = String::new();
	for ch in d.chars() {
		if ch.is_ascii_alphabetic() && ch != 'e' {
			if !num.is_empty() {
				toks.push(std::mem::take(&mut num));
			}
			toks.push(ch.to_string());
		} else if ch == ' ' || ch == ',' {
			if !num.is_empty() {
				toks.push(std::mem::take(&mut num));
			}
		} else if ch == '-' && !num.is_empty() && !num.ends_with('e') {
			toks.push(std::mem::take(&mut num));
			num.push(ch);
		} else {
			num.push(ch);
		}
	}
	if !num.is_empty() {
		toks.push(num);
	}
	let mut i = 0;
	let mut args: Vec<f64> = Vec::new();
	while i <= toks.len() {
		let t = toks.get(i);
		let is_cmd = t.map(|t| t.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(true);
		if is_cmd {
			// Flush the previous command's arguments.
			let rel = cmd.is_ascii_lowercase();
			let n = match cmd.to_ascii_uppercase() { 'M' | 'L' | 'T' => 2, 'H' | 'V' => 1, 'C' => 6, 'S' | 'Q' => 4, _ => 0 };
			let mut k = 0;
			while n > 0 && k + n <= args.len() {
				let a = &args[k..k + n];
				let (ox, oy) = if rel { cur } else { (0.0, 0.0) };
				let up = cmd.to_ascii_uppercase();
				if up != 'M' || k > 0 {
					if let Some(p) = pending.take() {
						pts.push(p);
					}
				}
				match up {
					'M' if k == 0 => {
						cur = (ox + a[0], oy + a[1]);
						start = cur;
						pending = Some(cur);
					}
					'M' | 'L' | 'T' => {
						cur = (ox + a[0], oy + a[1]);
						pts.push(cur);
					}
					'H' => { cur = (if rel { cur.0 + a[0] } else { a[0] }, cur.1); pts.push(cur); }
					'V' => { cur = (cur.0, if rel { cur.1 + a[0] } else { a[0] }); pts.push(cur); }
					'C' => {
						pts.push((ox + a[0], oy + a[1]));
						pts.push((ox + a[2], oy + a[3]));
						cur = (ox + a[4], oy + a[5]);
						pts.push(cur);
					}
					'S' | 'Q' => {
						pts.push((ox + a[0], oy + a[1]));
						cur = (ox + a[2], oy + a[3]);
						pts.push(cur);
					}
					_ => (),
				}
				k += n;
			}
			args.clear();
			match t {
				Some(t) => {
					cmd = t.chars().next().unwrap_or('M');
					if cmd == 'Z' || cmd == 'z' {
						cur = start;
					}
				}
				None => break,
			}
		} else if let Some(v) = t.and_then(|t| t.parse::<f64>().ok()) {
			args.push(v);
		}
		i += 1;
	}
	pts
}

fn oracle(name: &str, src: &str) -> Outcome<(Vec<BoxF>, (f64, f64))> {
	let dir = res!(work_dir());
	let typ = dir.join(fmt!("{}.typ", name));
	let svg = dir.join(fmt!("{}.svg", name));
	res!(std::fs::write(&typ, fmt!("{}{}", PRELUDE, src)).map_err(|e| err!(e, "write fixture"; IO)));
	let out = res!(Command::new("typst").arg("compile").arg(&typ).arg(&svg).output()
		.map_err(|e| err!(e, "run typst"; IO)));
	if !out.status.success() {
		return Err(err!("typst failed on {}: {}", name, String::from_utf8_lossy(&out.stderr); Input, Invalid));
	}
	let text = res!(std::fs::read_to_string(&svg).map_err(|e| err!(e, "read svg"; IO)));
	Ok(read_svg(&text))
}

fn read_svg(text: &str) -> (Vec<BoxF>, (f64, f64)) {
	// Symbols first: each glyph's outline points by id.
	let mut symbols: Vec<(String, Vec<(f64, f64)>)> = Vec::new();
	let mut rest = text;
	while let Some(i) = rest.find("<symbol ") {
		let tag_end = rest[i..].find('>').map(|j| i + j).unwrap_or(rest.len());
		let tag = &rest[i..tag_end];
		let id = attr(tag, "id").unwrap_or("").to_string();
		let close = rest[i..].find("</symbol>").map(|j| i + j).unwrap_or(rest.len());
		let body = &rest[tag_end..close];
		let mut pts = Vec::new();
		let mut b = body;
		while let Some(p) = b.find("<path ") {
			let pe = b[p..].find('>').map(|j| p + j).unwrap_or(b.len());
			if let Some(d) = attr(&b[p..pe], "d") {
				pts.extend(path_points(d));
			}
			b = &b[pe..];
		}
		symbols.push((id, pts));
		rest = &rest[close..];
	}
	let size = match text.find("<svg ") {
		Some(i) => {
			let tag = &text[i..text[i..].find('>').map(|j| i + j).unwrap_or(text.len())];
			let v = nums(attr(tag, "viewBox").unwrap_or(""));
			(v.get(2).copied().unwrap_or(0.0), v.get(3).copied().unwrap_or(0.0))
		}
		None => (0.0, 0.0),
	};
	// Then the body, outside `<defs>`, with a stack of group transforms.
	let body = match text.find("<defs") {
		Some(i)	=> &text[..i],
		None	=> text,
	};
	let mut boxes = Vec::new();
	let mut stack: Vec<[f64; 6]> = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
	let mut first_path = true;
	let mut s = body;
	while let Some(i) = s.find('<') {
		let end = s[i..].find('>').map(|j| i + j).unwrap_or(s.len());
		let tag = &s[i..=end.min(s.len() - 1)];
		let top = *stack.last().unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
		if tag.starts_with("<g") {
			let m = compose(top, transform(attr(tag, "transform")));
			if tag.ends_with("/>") {
				// An empty group.
			} else {
				stack.push(m);
			}
		} else if tag.starts_with("</g") {
			if stack.len() > 1 {
				stack.pop();
			}
		} else if tag.starts_with("<use") {
			let id = attr(tag, "xlink:href").or_else(|| attr(tag, "href")).unwrap_or("").trim_start_matches('#');
			let ux = attr(tag, "x").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
			let uy = attr(tag, "y").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
			let m = compose(compose(top, transform(attr(tag, "transform"))), [1.0, 0.0, 0.0, 1.0, ux, uy]);
			if let Some((_, pts)) = symbols.iter().find(|(sid, _)| sid == id) {
				if let Some(b) = bbox_of(pts, &m) {
					boxes.push(b);
				}
			}
		} else if tag.starts_with("<path") {
			if first_path {
				first_path = false;	// the page background
			} else {
				let m = compose(top, transform(attr(tag, "transform")));
				let pts = path_points(attr(tag, "d").unwrap_or(""));
				match attr(tag, "stroke-width").and_then(|v| v.parse::<f64>().ok()) {
					Some(t) if attr(tag, "fill") == Some("none") => {
						let cap = attr(tag, "stroke-linecap").unwrap_or("butt");
						for w in pts.windows(2) {
							if let Some(b) = stroke_box(w[0], w[1], t, cap, &m) {
								boxes.push(b);
							}
						}
					}
					_ => if let Some(b) = bbox_of(&pts, &m) {
						boxes.push(b);
					},
				}
			}
		}
		s = &s[end.min(s.len() - 1) + 1..];
	}
	(boxes, size)
}

fn stroke_box(p0: (f64, f64), p1: (f64, f64), t: f64, cap: &str, m: &[f64; 6]) -> Option<BoxF> {
	let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
	let len = dx.hypot(dy);
	if len == 0.0 {
		return None;
	}
	let (ux, uy) = (dx / len, dy / len);
	let (nx, ny) = (-uy * t / 2.0, ux * t / 2.0);
	let ext = if cap == "butt" { 0.0 } else { t / 2.0 };
	let a = (p0.0 - ux * ext, p0.1 - uy * ext);
	let b = (p1.0 + ux * ext, p1.1 + uy * ext);
	bbox_of(&[(a.0 + nx, a.1 + ny), (a.0 - nx, a.1 - ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny)], m)
}

// Matching

fn close(a: &BoxF, b: &BoxF) -> bool {
	(0..4).all(|i| (a[i] - b[i]).abs() <= TOL)
}

fn compare(name: &str, ours: &(Vec<BoxF>, (f64, f64)), theirs: &(Vec<BoxF>, (f64, f64))) -> Vec<String> {
	let mut errs = Vec::new();
	if (ours.1.0 - theirs.1.0).abs() > TOL || (ours.1.1 - theirs.1.1).abs() > TOL {
		errs.push(fmt!("{}: size {:.4} x {:.4}, oracle {:.4} x {:.4}", name, ours.1.0, ours.1.1, theirs.1.0, theirs.1.1));
	}
	let mut left: Vec<BoxF> = ours.0.clone();
	let mut missing = Vec::new();
	for t in &theirs.0 {
		match left.iter().position(|o| close(o, t)) {
			Some(i)	=> { left.remove(i); }
			None	=> missing.push(*t),
		}
	}
	let f = |b: &BoxF| fmt!("[{:.3} {:.3} {:.3} {:.3}]", b[0], b[1], b[2], b[3]);
	for m in &missing {
		errs.push(fmt!("{}: oracle ink {} unmatched", name, f(m)));
	}
	for l in &left {
		errs.push(fmt!("{}: austenite ink {} extra", name, f(l)));
	}
	errs
}

// A fixture's dependency, `// needs: elem.field`, absent from the schemas yet.
fn pending(src: &str) -> Option<String> {
	let first = src.lines().next()?;
	let need = first.strip_prefix("// needs:")?.trim();
	let (kind, field) = need.split_once('.')?;
	let k = ElemKind::ALL.iter().find(|k| k.path() == kind)?;
	if k.field_id(field).is_some() { None } else { Some(need.to_string()) }
}

#[test]
fn maths_fixtures_match_the_typst_oracle() -> Outcome<()> {
	if !typst_available() {
		if std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") {
			return Ok(());
		}
		return Err(err!("typst is not on PATH; set EVAL_ORACLE_SKIP=1 to skip the oracle deliberately"; Missing));
	}
	let only = std::env::var("MATH_FIXTURE").ok();
	let mut names: Vec<String> = Vec::new();
	for entry in res!(std::fs::read_dir(fixture_dir()).map_err(|e| err!(e, "fixture dir"; IO))) {
		let p = res!(entry.map_err(|e| err!(e, "fixture entry"; IO))).path();
		if p.extension().and_then(|x| x.to_str()) == Some("typ") {
			if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
				if only.as_deref().map(|o| o == stem).unwrap_or(true) {
					names.push(stem.to_string());
				}
			}
		}
	}
	names.sort();
	let mut errs = Vec::new();
	let mut passed = 0;
	let mut waiting = Vec::new();
	for name in &names {
		let src = res!(std::fs::read_to_string(fixture_dir().join(fmt!("{}.typ", name))).map_err(|e| err!(e, "read"; IO)));
		if let Some(need) = pending(&src) {
			waiting.push(fmt!("{} ({})", name, need));
			continue;
		}
		let theirs = match oracle(name, &src) {
			Ok(t)	=> t,
			Err(e)	=> {
				errs.push(fmt!("{}: {}", name, e));
				continue;
			}
		};
		if theirs.0.is_empty() {
			errs.push(fmt!("{}: the oracle drew nothing", name));
			continue;
		}
		match austenite(name, &src) {
			Ok(ours) => {
				let e = compare(name, &ours, &theirs);
				if e.is_empty() {
					passed += 1;
				}
				errs.extend(e);
			}
			Err(e) => match waiting_on(&e) {
				Some(why)	=> waiting.push(fmt!("{} ({})", name, why)),
				None		=> errs.push(fmt!("{}: {}", name, e)),
			},
		}
	}
	println!("maths fixtures: {} of {} match the oracle, {} pending: {:?}", passed, names.len() - waiting.len(),
		waiting.len(), waiting);
	if strict() && !waiting.is_empty() {
		return Err(err!("{} maths fixtures are still waiting on another unit", waiting.len(); Missing));
	}
	if !errs.is_empty() {
		for e in &errs {
			println!("{}", e);
		}
		return Err(err!("{} maths layout differences from the oracle", errs.len(); Mismatch));
	}
	Ok(())
}

/// The number of a numbered block equation is its counter's value, read through the model's lookup
/// seam: the third equation of a document reads (3), as it does in Typst with a counter update ahead of
/// one equation. A first equation reads (1), so the two are told apart.
#[test]
fn a_numbered_equation_counts_the_equations_before_it() -> Outcome<()> {
	if !typst_available() {
		if std::env::var("EVAL_ORACLE_SKIP").as_deref() == Ok("1") {
			return Ok(());
		}
		return Err(err!("typst is not on PATH; set EVAL_ORACLE_SKIP=1 to skip the oracle deliberately"; Missing));
	}
	let set = "#set math.equation(numbering: \"(1)\")\n";
	let third = match austenite_nth("count_third", &fmt!("{}$ a $\n$ b $\n$ c + d = 1 $", set), 2) {
		Ok(t)	=> t,
		Err(e)	=> match waiting_on(&e) {
			Some(why) if !strict()	=> {
				println!("count_third waits on another unit: {}", why);
				return Ok(());
			}
			_						=> return Err(e),
		},
	};
	let counted = res!(oracle("count_third_typst", &fmt!("{}#counter(math.equation).update(2)\n$ c + d = 1 $", set)));
	let first = res!(oracle("count_first_typst", &fmt!("{}$ c + d = 1 $", set)));
	let errs = compare("count_third", &third, &counted);
	for e in &errs {
		println!("{}", e);
	}
	assert!(errs.is_empty(), "{} differences from the oracle's third equation", errs.len());
	assert!(!compare("count_third", &third, &first).is_empty(), "(3) must not be laid out as (1)");
	Ok(())
}

/// The comparator must fail on a real difference: a glyph moved by a tenth of a point, a box missing.
#[test]
fn the_comparator_catches_a_moved_glyph() {
	let a = (vec![[0.0, 0.0, 1.0, 1.0], [2.0, 0.0, 3.0, 1.0]], (3.0, 1.0));
	let moved = (vec![[0.0, 0.0, 1.0, 1.0], [2.1, 0.0, 3.1, 1.0]], (3.0, 1.0));
	let fewer = (vec![[0.0, 0.0, 1.0, 1.0]], (3.0, 1.0));
	assert!(compare("same", &a, &a).is_empty());
	assert!(!compare("moved", &moved, &a).is_empty());
	assert!(!compare("fewer", &fewer, &a).is_empty());
}

#[test]
fn svg_paths_read_relative_and_absolute_commands() {
	let pts = path_points("M 1 2l 3 0h 1v -2c 0 1 1 1 1 0Z");
	assert_eq!(pts.first(), Some(&(1.0, 2.0)));
	assert!(pts.contains(&(4.0, 2.0)));
	assert!(pts.contains(&(5.0, 2.0)));
	assert!(pts.contains(&(5.0, 0.0)));
	assert!(pts.contains(&(6.0, 0.0)));
}

#[test]
fn debug_dump() {
	let src = std::env::var("MATH_DUMP").unwrap_or_default();
	if src.is_empty() { return; }
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
	let mut e = Engine::new(World::new(dir.clone()));
	let id = e.world.add_source(dir.join("dump.typ"), src).unwrap_or_else(|x| panic!("{}", x));
	let m = eval_module(&mut e, id).unwrap_or_else(|x| panic!("{} {:?}", x, e.diags));
	let pairs = realise(&mut e, &m.content, &StyleChain::root(), RealiseMode::Flow).unwrap_or_else(|x| panic!("{}", x));
	let (eq, st) = find_equation(&pairs, 0).unwrap_or_else(|| panic!("no eq"));
	let node = layout_equation(&mut e, &eq, &st, Region::new(Sp(i32::MAX), Sp(i32::MAX))).unwrap_or_else(|x| panic!("{}", x));
	fn show(n: &Node, d: usize) {
		match n {
			Node::HBox(b) | Node::VBox(b) => { println!("{:w$}box {:?}", "", b.dims, w = d); for c in &b.list { show(c, d + 2); } }
			Node::Leaf(l) => match &l.kind {
				LeafKind::Text(t) => println!("{:w$}text {:?} shift {:?}", "", t.source(), l.shift, w = d),
				LeafKind::Graphic(g) => println!("{:w$}graphic {} ops", "", g.ops.len(), w = d),
				_ => println!("{:w$}leaf", "", w = d),
			},
			Node::Glue(g) => println!("{:w$}glue {:?}", "", g.natural.to_pt(), w = d),
			_ => (),
		}
	}
	show(&node, 0);
}
