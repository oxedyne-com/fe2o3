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
//
// A line is held to its text, its baseline and the x where it starts. The start is where a first-line
// or hanging indent, a margin and an alignment all show, none of which moves a break or a baseline, so
// a line set sideways passes the first two checks and fails the third.

use crate::harness::markup::{
	tokens,
	Tok,
};

use oxedyne_fe2o3_core::prelude::*;

pub const BASELINE_TOL: f64 = 1.0;	// points, the design's level-4 tolerance
pub const START_TOL: f64 = 0.05;	// points, where a line starts: its indent or its alignment offset
pub const SPACE_REACH: f64 = 40.0;	// points, the most a space opening a line can be wide

#[derive(Clone, Debug)]
pub struct OLine {
	pub text:		String,
	pub x0:			f64,	// where the first word starts
	pub x1:			f64,
	pub start:		f64,	// where the first glyph drawn starts, a leading space included: its text run's start
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
///
/// A line's horizontal extent is its words', not its own `xMin` and `xMax`: pdftotext (poppler 26.01)
/// gives a line the left edge of its second word, so a line's start read from the line is wrong. And
/// a loosely justified line comes back from it as several, cut at its widest gaps, so the pieces of one
/// block that stand on one baseline are one line again.
pub fn read_bbox(xml: &str) -> Vec<OPage> {
	let mut pages: Vec<OPage> = Vec::new();
	let mut block_from = 0usize;	// where the current block's lines begin on the page
	let mut line: Option<OLine> = None;
	let mut words_seen = false;
	let mut in_word = false;
	for t in tokens(xml) {
		match t {
			Tok::Open { name, attrs, .. } => match name.as_str() {
				"page"	=> {
					block_from = 0;
					pages.push(OPage { width: num(&attrs, "width"), height: num(&attrs, "height"), lines: Vec::new() });
				}
				"block"	=> block_from = pages.last().map_or(0, |p| p.lines.len()),
				"line"	=> {
					words_seen = false;
					line = Some(OLine {
						text:		String::new(),
						x0:			num(&attrs, "xmin"),
						x1:			num(&attrs, "xmax"),
						start:		num(&attrs, "xmin"),
						y0:			num(&attrs, "ymin"),
						y1:			num(&attrs, "ymax"),
						base:		0.0,
						measured:	false,
					});
				}
				"word"	=> {
					in_word = true;
					if let Some(l) = line.as_mut() {
						let (w0, w1) = (num(&attrs, "xmin"), num(&attrs, "xmax"));
						if words_seen {
							l.x0 = l.x0.min(w0);
							l.x1 = l.x1.max(w1);
						} else {
							l.x0 = w0;
							l.x1 = w1;
							words_seen = true;
						}
						l.start = l.x0;
					}
				}
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
						let same = |o: &OLine| (o.y0 - l.y0).abs() < 0.5 && (o.y1 - l.y1).abs() < 0.5;
						match p.lines.iter_mut().skip(block_from).find(|o| same(o)) {
							Some(o) => {
								o.text = fmt!("{} {}", o.text, l.text);
								o.x0 = o.x0.min(l.x0);
								o.x1 = o.x1.max(l.x1);
								o.start = o.x0;
							}
							None => p.lines.push(l),
						}
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

/// The page-space origin of every glyph a Typst SVG page draws, and the origin of each text run: the
/// group that holds its glyphs, which Typst draws at the run's start, a space it leaves out included.
fn walk_glyphs(svg: &str) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
	// Per open element: its transform, whether it is inside defs, and its origin while no glyph has been
	// drawn in it.
	let mut stack: Vec<(M, bool, Option<(f64, f64)>)> = vec![(M::ID, false, None)];
	let (mut out, mut starts) = (Vec::new(), Vec::new());
	for t in tokens(svg) {
		match t {
			Tok::Open { name, attrs, closed } => {
				let (top, in_defs, _) = stack.last().copied().unwrap_or((M::ID, false, None));
				let own = Tok::attr(&attrs, "transform").map(parse_transform).unwrap_or(M::ID);
				let cur = top.then(own);
				let defs = in_defs || name == "defs" || name == "symbol";
				if name == "use" && !defs {
					let href = Tok::attr(&attrs, "xlink:href").or_else(|| Tok::attr(&attrs, "href")).unwrap_or("");
					if href.starts_with("#g") {
						out.push(cur.apply(num(&attrs, "x"), num(&attrs, "y")));
						if let Some(last) = stack.last_mut() {
							if let Some(origin) = last.2.take() {
								starts.push(origin);
							}
						}
					}
				}
				if !closed {
					stack.push((cur, defs, if name == "g" { Some(cur.apply(0.0, 0.0)) } else { None }));
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
	(out, starts)
}

/// The page-space origin of every glyph a Typst SVG page draws.
pub fn glyph_origins(svg: &str) -> Vec<(f64, f64)> {
	walk_glyphs(svg).0
}

/// The page-space origin of every text run a Typst SVG page draws.
pub fn run_starts(svg: &str) -> Vec<(f64, f64)> {
	walk_glyphs(svg).1
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

/// Gives each line the start of the text run its first word is drawn in: the nearest run start on the
/// line's baseline at or before the word, within a space's reach. That run opens with a space when
/// Typst's line begins after an inline box or a space (pdftotext's words leave the space out), where
/// Austenite's run opens as well; a line whose run is not found starts where its first word does.
/// Call after [`attach_baselines`].
pub fn attach_starts(page: &mut OPage, starts: &[(f64, f64)]) {
	for l in &mut page.lines {
		let found = starts.iter()
			.filter(|(x, y)| (*y - l.base).abs() < 0.02 && *x <= l.x0 + 0.02 && *x >= l.x0 - SPACE_REACH)
			.map(|(x, _)| *x)
			.fold(f64::NEG_INFINITY, f64::max);
		if found.is_finite() {
			l.start = found;
		}
	}
}

fn squash(s: &str) -> String {
	s.chars().filter(|c| !c.is_whitespace() && *c != '\u{ad}').collect()
}

/// Level-4 differences: page count exact, page size within half a point, every Typst line matched by
/// Austenite text on the same page with the same characters, a baseline within [`BASELINE_TOL`] and a
/// start within [`START_TOL`] of where Typst's line starts.
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
			// Where the line starts: its first run's left edge, to the left edge of Typst's first run.
			if let Some(first) = mine.first() {
				if (first.x0 - l.start).abs() > START_TOL {
					out.push(fmt!("page {}: line {:?}: starts at x {:.2} in typst, {:.2} in austenite ({:+.2})",
						pno, l.text, l.start, first.x0, first.x0 - l.start));
				}
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
