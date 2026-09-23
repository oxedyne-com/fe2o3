// Level 4: pages, line text and baselines.
//
// The oracle side comes from two of Typst's outputs, because neither alone has both halves.
// `pdftotext -bbox-layout` on Typst's PDF gives each line's text and box, but a box, not a
// baseline: poppler sizes it from the font descriptor's ascent and descent, which differ per face.
// Typst's SVG pages give every glyph's origin exactly -- a text run is a group translated to its
// baseline -- but no text. So each pdftotext line takes its baseline from the glyph origins that
// fall inside its box, the most common y among them.
//
// Austenite's side is its placed text runs, read straight from the laid-out pages: a run's top-left
// is `(x, y)` and its baseline `y + height`.

use crate::harness::markup::{
	tokens,
	Tok,
};

use oxedyne_fe2o3_core::prelude::*;

pub const BASELINE_TOL: f64 = 1.0;	// points, the design's level-4 tolerance

#[derive(Clone, Debug)]
pub struct OLine {
	pub text:		String,
	pub x0:			f64,
	pub x1:			f64,
	pub y0:			f64,
	pub y1:			f64,
	pub base:		f64,
	pub measured:	bool,	// the baseline came from glyph origins, not the box
}

#[derive(Clone, Debug, Default)]
pub struct OPage {
	pub width:	f64,
	pub height:	f64,
	pub lines:	Vec<OLine>,
}

#[derive(Clone, Debug)]
pub struct ARun {
	pub text:	String,
	pub x0:		f64,
	pub x1:		f64,
	pub base:	f64,
}

#[derive(Clone, Debug, Default)]
pub struct APage {
	pub width:	f64,
	pub height:	f64,
	pub runs:	Vec<ARun>,
}

fn num(attrs: &[(String, String)], key: &str) -> f64 {
	Tok::attr(attrs, key).and_then(|v| v.trim().trim_end_matches("pt").parse::<f64>().ok()).unwrap_or(0.0)
}

/// The pages and lines of `pdftotext -bbox-layout` output, baselines not yet attached.
pub fn read_bbox(xml: &str) -> Vec<OPage> {
	let mut pages: Vec<OPage> = Vec::new();
	let mut line: Option<OLine> = None;
	let mut in_word = false;
	for t in tokens(xml) {
		match t {
			Tok::Open { name, attrs, .. } => match name.as_str() {
				"page"	=> pages.push(OPage { width: num(&attrs, "width"), height: num(&attrs, "height"), lines: Vec::new() }),
				"line"	=> line = Some(OLine {
					text:		String::new(),
					x0:			num(&attrs, "xmin"),
					x1:			num(&attrs, "xmax"),
					y0:			num(&attrs, "ymin"),
					y1:			num(&attrs, "ymax"),
					base:		0.0,
					measured:	false,
				}),
				"word"	=> in_word = true,
				_		=> (),
			},
			Tok::Close(name) => match name.as_str() {
				"word"	=> {
					in_word = false;
					if let Some(l) = line.as_mut() {
						l.text.push(' ');
					}
				}
				"line"	=> if let Some(mut l) = line.take() {
					l.text = l.text.trim().to_string();
					if let Some(p) = pages.last_mut() {
						p.lines.push(l);
					}
				},
				_		=> (),
			},
			Tok::Text(s) => if in_word {
				if let Some(l) = line.as_mut() {
					l.text.push_str(&s);
				}
			},
		}
	}
	pages
}

// An affine map (x, y) -> (a x + c y + e, b x + d y + f).
#[derive(Clone, Copy, Debug)]
struct M([f64; 6]);

impl M {
	const ID: M = M([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

	fn then(self, n: M) -> M {
		let (m, n) = (self.0, n.0);
		M([
			m[0] * n[0] + m[2] * n[1],
			m[1] * n[0] + m[3] * n[1],
			m[0] * n[2] + m[2] * n[3],
			m[1] * n[2] + m[3] * n[3],
			m[0] * n[4] + m[2] * n[5] + m[4],
			m[1] * n[4] + m[3] * n[5] + m[5],
		])
	}

	fn apply(self, x: f64, y: f64) -> (f64, f64) {
		let m = self.0;
		(m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
	}
}

fn parse_transform(s: &str) -> M {
	let mut m = M::ID;
	let mut rest = s;
	while let Some(open) = rest.find('(') {
		let name = rest[..open].trim().trim_start_matches(',').trim();
		let close = match rest[open..].find(')') {
			Some(c)	=> open + c,
			None	=> break,
		};
		let args: Vec<f64> = rest[open + 1..close]
			.split(|c: char| c == ',' || c.is_whitespace())
			.filter(|t| !t.is_empty())
			.filter_map(|t| t.parse::<f64>().ok())
			.collect();
		let a = |i: usize| args.get(i).copied().unwrap_or(0.0);
		let own = match name {
			"matrix" if args.len() == 6	=> M([a(0), a(1), a(2), a(3), a(4), a(5)]),
			"translate"					=> M([1.0, 0.0, 0.0, 1.0, a(0), a(1)]),
			"scale"						=> {
				let sy = if args.len() > 1 { a(1) } else { a(0) };
				M([a(0), 0.0, 0.0, sy, 0.0, 0.0])
			}
			"rotate"					=> {
				let r = a(0).to_radians();
				let rot = M([r.cos(), r.sin(), -r.sin(), r.cos(), 0.0, 0.0]);
				if args.len() == 3 {
					M([1.0, 0.0, 0.0, 1.0, a(1), a(2)]).then(rot).then(M([1.0, 0.0, 0.0, 1.0, -a(1), -a(2)]))
				} else {
					rot
				}
			}
			"skewX"						=> M([1.0, 0.0, a(0).to_radians().tan(), 1.0, 0.0, 0.0]),
			"skewY"						=> M([1.0, a(0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
			_							=> M::ID,
		};
		m = m.then(own);
		rest = &rest[close + 1..];
	}
	m
}

/// The page-space origin of every glyph a Typst SVG page draws.
pub fn glyph_origins(svg: &str) -> Vec<(f64, f64)> {
	let mut stack: Vec<(M, bool)> = vec![(M::ID, false)];	// (transform, inside defs)
	let mut out = Vec::new();
	for t in tokens(svg) {
		match t {
			Tok::Open { name, attrs, closed } => {
				let (top, in_defs) = stack.last().copied().unwrap_or((M::ID, false));
				let own = Tok::attr(&attrs, "transform").map(parse_transform).unwrap_or(M::ID);
				let cur = top.then(own);
				let defs = in_defs || name == "defs" || name == "symbol";
				if name == "use" && !defs {
					let href = Tok::attr(&attrs, "xlink:href").or_else(|| Tok::attr(&attrs, "href")).unwrap_or("");
					if href.starts_with("#g") {
						out.push(cur.apply(num(&attrs, "x"), num(&attrs, "y")));
					}
				}
				if !closed {
					stack.push((cur, defs));
				}
			}
			Tok::Close(_) => {
				if stack.len() > 1 {
					stack.pop();
				}
			}
			Tok::Text(_) => (),
		}
	}
	out
}

/// Gives each line the baseline its glyphs sit on; a line with no glyph origin inside its box keeps
/// an estimate from the box (Libertinus Serif's descent) and is marked unmeasured.
pub fn attach_baselines(page: &mut OPage, origins: &[(f64, f64)]) {
	for l in &mut page.lines {
		let mut ys: Vec<i64> = origins.iter()
			.filter(|(x, y)| *x >= l.x0 - 1.0 && *x <= l.x1 + 1.0 && *y >= l.y0 - 0.5 && *y <= l.y1 + 0.5)
			.map(|(_, y)| (y * 100.0).round() as i64)
			.collect();
		if ys.is_empty() {
			l.base = l.y1 - 0.2157 * (l.y1 - l.y0);
			l.measured = false;
			continue;
		}
		ys.sort();
		let mut best = (ys[0], 0usize);
		let mut i = 0;
		while i < ys.len() {
			let mut j = i;
			while j < ys.len() && ys[j] == ys[i] {
				j += 1;
			}
			if j - i > best.1 {
				best = (ys[i], j - i);
			}
			i = j;
		}
		l.base = best.0 as f64 / 100.0;
		l.measured = true;
	}
}

fn squash(s: &str) -> String {
	s.chars().filter(|c| !c.is_whitespace() && *c != '\u{ad}').collect()
}

/// Level-4 differences: page count exact, page size within half a point, every Typst line matched by
/// Austenite text on the same page with the same characters and a baseline within [`BASELINE_TOL`].
pub fn compare(want: &[OPage], got: &[APage], out: &mut Vec<String>) {
	if want.len() != got.len() {
		out.push(fmt!("page count: typst {}, austenite {}", want.len(), got.len()));
	}
	for (pi, (w, g)) in want.iter().zip(got.iter()).enumerate() {
		let pno = pi + 1;
		if (w.width - g.width).abs() > 0.5 || (w.height - g.height).abs() > 0.5 {
			out.push(fmt!("page {}: size typst {:.2}x{:.2}pt, austenite {:.2}x{:.2}pt",
				pno, w.width, w.height, g.width, g.height));
		}
		// Each Austenite run joins the Typst line whose box holds its baseline and overlaps it
		// horizontally, the nearest baseline winning.
		let mut owner: Vec<Option<usize>> = vec![None; g.runs.len()];
		for (ri, r) in g.runs.iter().enumerate() {
			let mut best: Option<(usize, f64)> = None;
			for (li, l) in w.lines.iter().enumerate() {
				let inside = r.base >= l.y0 - BASELINE_TOL && r.base <= l.y1 + BASELINE_TOL;
				let overlap = r.x1 >= l.x0 - 2.0 && r.x0 <= l.x1 + 2.0;
				if inside && overlap {
					let d = (r.base - l.base).abs();
					if best.map(|(_, bd)| d < bd).unwrap_or(true) {
						best = Some((li, d));
					}
				}
			}
			owner[ri] = best.map(|(li, _)| li);
		}
		for (li, l) in w.lines.iter().enumerate() {
			let mut mine: Vec<&ARun> = g.runs.iter().enumerate()
				.filter(|(ri, _)| owner[*ri] == Some(li))
				.map(|(_, r)| r)
				.collect();
			if mine.is_empty() {
				let near = g.runs.iter().min_by(|a, b| {
					let da = (a.base - l.base).abs();
					let db = (b.base - l.base).abs();
					da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
				});
				out.push(match near {
					Some(n)	=> fmt!("page {}: line {:?} at baseline {:.2} has no austenite text (nearest {:?} at {:.2})",
						pno, l.text, l.base, n.text, n.base),
					None	=> fmt!("page {}: line {:?} at baseline {:.2} has no austenite text (the page is empty)",
						pno, l.text, l.base),
				});
				continue;
			}
			mine.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
			let text: String = mine.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ");
			if squash(&text) != squash(&l.text) {
				out.push(fmt!("page {}: line at baseline {:.2}: typst {:?}, austenite {:?}", pno, l.base, l.text, text));
			}
			// The line's baseline is the one most of its characters sit on.
			let main = mine.iter().max_by_key(|r| r.text.chars().count()).map(|r| r.base).unwrap_or(l.base);
			if (main - l.base).abs() > BASELINE_TOL {
				out.push(fmt!("page {}: line {:?}: baseline typst {:.2}, austenite {:.2}", pno, l.text, l.base, main));
			}
		}
		for (ri, r) in g.runs.iter().enumerate() {
			if owner[ri].is_none() && !r.text.trim().is_empty() {
				out.push(fmt!("page {}: austenite text {:?} at baseline {:.2} lies on no typst line", pno, r.text, r.base));
			}
		}
	}
	out.truncate(24);
}
