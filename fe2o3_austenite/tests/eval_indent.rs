//! U6a-fix: a justified paragraph's first-line indent, against the `typst` 0.15.1 oracle at level 4.
//!
//! Level 4 is the page count and size and, for each line, its text, its baseline and now the x where
//! it starts (`tests/eval_oracle/layout.rs`). The x is what an indent, a margin and an alignment move
//! without moving a break or a baseline.
//!
//! - The indent corpus (`tests/fixtures/eval/indent/`) sets justified, hanging-indented, ragged and
//!   aligned paragraphs, each in the situation it is met in: after a paragraph, first, after a block.
//!   Each is held to Typst's lines.
//! - The corpora of the line-break and hyphenation fixtures, which set a single paragraph with no
//!   indent, are held to the same x.
//! - The comparison itself is tested: a line set sideways is reported, and a loosely justified line that
//!   pdftotext cut in pieces is read as the one line it is, starting at its first word.
//!
//! Austenite's side lays each paragraph out with `layout_par` in the situation a block flow gives it
//! (Typst's `collect`: first, consecutive after a paragraph, other after a block) and stacks the lines
//! a paragraph spacing apart; the pagination itself is U6b's. A fixture that sets anything else is
//! refused, so no result rests on a layout this file did not do.
//!
//! The one block a fixture may hold is `#block(height: 0pt)` under `#set block(spacing: 0pt)`, which
//! Typst lays out as nothing but a paragraph's `other` situation. Austenite's evaluator has no `block`
//! yet, so this file takes both lines out of the source and keeps the situation; Typst's own lines hold
//! the model to account. A fixture whose first lines say `// needs: u6b` (it sets an `align`, a `box` or
//! an `h`, whose schemas are U6b's) is reported as pending while Austenite cannot evaluate it, and held
//! to Typst once it can.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::corpus::{
	discover,
	Filter,
	Fixture,
};
use harness::layout::{
	attach_baselines,
	attach_starts,
	compare,
	glyph_origins,
	read_bbox,
	run_starts,
	APage,
	ARun,
	OLine,
	OPage,
	START_TOL,
};
use harness::oracle::Oracle;

use oxedyne_fe2o3_austenite::eval::content::ElemKind;
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	Pair,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::value::Value;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::inline::ParSituation;
use oxedyne_fe2o3_austenite::flow::par::layout_par;
use oxedyne_fe2o3_austenite::flow::Region;
use oxedyne_fe2o3_austenite::ir::{
	LeafKind,
	Node,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ AUSTENITE'S PAGE                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// A fixture's `#set page(width: .., height: .., margin: ..)`, which the evaluator reads only once the
/// page schema exists.
struct Setup {
	width:	f64,
	height:	Option<f64>,	// `None` for `auto`
	margin:	f64,
}

/// The number of points a `key: <n>pt` argument gives, `None` for `auto` or an absent key.
fn points(args: &str, key: &str) -> Option<f64> {
	let rest = args.split(&fmt!("{}:", key)).nth(1).map(|r| r.trim());
	let word = rest.and_then(|r| r.split(|c: char| c == ',' || c == ')').next()).map(|w| w.trim());
	match word {
		Some("auto") | None	=> None,
		Some(w)				=> w.trim_end_matches("pt").trim().parse::<f64>().ok(),
	}
}

/// The page a fixture sets, its source with the lines Austenite does not evaluate taken out, and which
/// paragraphs (counted from 0) stand right after a `#block(height: 0pt)`.
fn split_source(text: &str) -> std::result::Result<(Setup, String, Vec<usize>), String> {
	let mut setup = None;
	let mut kept: Vec<&str> = Vec::new();
	let (mut after_block, mut blank, mut paragraphs) = (false, true, 0usize);
	let mut others = Vec::new();
	for line in text.lines() {
		if let Some(args) = line.strip_prefix("#set page(") {
			let width = match points(args, "width") {
				Some(w)	=> w,
				None	=> return Err(fmt!("no page width in {:?}", line)),
			};
			setup = Some(Setup { width, height: points(args, "height"), margin: points(args, "margin").unwrap_or(0.0) });
		} else if line == "#set block(spacing: 0pt)" {
			// Typst's own, which Austenite has no block to set it on.
		} else if line == "#block(height: 0pt)" {
			after_block = true;
		} else {
			if !line.starts_with("//") && !line.starts_with("#set ") {
				if line.trim().is_empty() {
					blank = true;
				} else if blank {
					// The first line of a paragraph.
					if after_block {
						others.push(paragraphs);
						after_block = false;
					}
					paragraphs += 1;
					blank = false;
				}
			}
			kept.push(line);
		}
	}
	match setup {
		Some(s)	=> Ok((s, kept.join("\n"), others)),
		None	=> Err("the fixture sets no page".to_string()),
	}
}

fn last_msg(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}

/// The text runs of a line's box with the x each starts at.
fn line_runs(list: &[Node], x0: f64, base: f64, out: &mut Vec<ARun>) {
	let mut x = x0;
	for n in list {
		match n {
			Node::Glue(g)	=> x += g.natural.to_pt(),
			Node::Leaf(l)	=> {
				let w = l.dims.width.to_pt();
				if let LeafKind::Text(t) = &l.kind {
					out.push(ARun { text: t.source().to_string(), x0: x, x1: x + w, base: base + l.shift.to_pt() });
				}
				x += w;
			}
			Node::HBox(b) | Node::VBox(b)	=> x += b.dims.width.to_pt(),
			_	=> (),
		}
	}
}

/// What a paragraph's `spacing` comes to.
fn spacing_of(pair: &Pair) -> std::result::Result<f64, String> {
	let size = pair.styles.font_size();
	let id = match ElemKind::Par.field_id("spacing") {
		Some(id)	=> id,
		None		=> return Err("no par spacing field".to_string()),
	};
	match pair.styles.resolve(&pair.content, id) {
		Ok(Some(Value::Length(l)))	=> Ok(l.resolve(size)),
		Ok(_)						=> Ok(1.2 * size),
		Err(e)						=> Err(fmt!("par spacing: {}", last_msg(&e))),
	}
}

/// Austenite's one page for a fixture: its paragraphs laid out in the situation each is met in.
fn austenite_page(fx: &Fixture) -> std::result::Result<APage, String> {
	let (setup, source, others) = match split_source(&fx.text) {
		Ok(t)	=> t,
		Err(e)	=> return Err(e),
	};
	let mut world = World::new(fx.root.clone());
	let id = match world.add_source(fx.path.clone(), source) {
		Ok(id)	=> id,
		Err(e)	=> return Err(fmt!("source: {}", last_msg(&e))),
	};
	let mut engine = Engine::new(world);
	let module = match eval_source(&mut engine, id) {
		Ok(m)	=> m,
		Err(e)	=> return Err(fmt!("eval: {}", last_msg(&e))),
	};
	let pairs = match realise(&mut engine, &module.content, &StyleChain::root(), RealiseMode::Document) {
		Ok(p)	=> p,
		Err(e)	=> return Err(fmt!("realise: {}", last_msg(&e))),
	};
	let measure = setup.width - 2.0 * setup.margin;
	let mut y = setup.margin;
	let mut index = 0usize;		// the paragraph in hand, counted from 0
	let mut gap: Option<f64> = None;	// the spacing a paragraph leaves after it, until the next one asks
	let mut runs: Vec<ARun> = Vec::new();
	for pair in &pairs {
		if pair.is_tag() {
			continue;
		}
		if pair.content.is(ElemKind::Par) {
			let spacing = match spacing_of(pair) {
				Ok(g)	=> g,
				Err(e)	=> return Err(e),
			};
			if let Some(g) = gap.take() {
				y += g.max(spacing);
			}
			let region = Region::new(Sp::from_pt(measure), Sp::from_pt(setup.height.unwrap_or(10_000.0)));
			// Typst's `collect`: first at the top, consecutive after a paragraph, other after a block.
			let situation = if others.contains(&index) {
				ParSituation::Other
			} else if index == 0 {
				ParSituation::First
			} else {
				ParSituation::Consecutive
			};
			index += 1;
			let nodes = match layout_par(&mut engine, &pair.content, &pair.styles, region, situation) {
				Ok(n)	=> n,
				Err(e)	=> return Err(fmt!("layout_par: {}", last_msg(&e))),
			};
			for n in &nodes {
				match n {
					Node::HBox(b)		=> {
						line_runs(&b.list, setup.margin, y + b.dims.height.to_pt(), &mut runs);
						y += b.dims.vextent().to_pt();
					}
					Node::Glue(g)		=> y += g.natural.to_pt(),
					Node::Penalty(_)	=> (),
					other				=> return Err(fmt!("a paragraph laid out to {:?}", other)),
				}
			}
			gap = Some(spacing);
		} else {
			return Err(fmt!("the fixture sets {:?}, which this file does not lay out", pair.content.kind()));
		}
	}
	Ok(APage {
		width:	setup.width,
		height:	setup.height.unwrap_or(y + setup.margin),
		runs,
	})
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE COMPARISON                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Typst's pages for a fixture, or the reason there are none.
fn typst_pages(o: &Oracle, fx: &Fixture) -> std::result::Result<Vec<OPage>, String> {
	match o.layout(fx) {
		Ok(Ok(p))	=> Ok(p),
		Ok(Err(e))	=> Err(fmt!("typst: {}", e)),
		Err(e)		=> Err(fmt!("oracle: {}", last_msg(&e))),
	}
}

/// What a fixture's `// indents: <n> <x>` header asks of Typst's own lines: at least `n` of them start
/// at `x` points, so a fixture that says it sets indented lines cannot pass without any.
fn indents_asked(text: &str) -> Option<(usize, f64)> {
	for line in text.lines().take_while(|l| l.starts_with("//")) {
		if let Some(rest) = line.strip_prefix("// indents:") {
			let words: Vec<&str> = rest.split_whitespace().collect();
			return match (words.first().and_then(|n| n.parse::<usize>().ok()), words.get(1).and_then(|x| x.parse::<f64>().ok())) {
				(Some(n), Some(x))	=> Some((n, x)),
				_					=> None,
			};
		}
	}
	None
}

/// Does the fixture say it needs a unit that may not be merged yet?
fn needs_unit(text: &str) -> bool {
	text.lines().take_while(|l| l.starts_with("//")).any(|l| l.starts_with("// needs:"))
}

/// The fault a fixture records as known: Austenite is expected to differ from Typst on it, and the test
/// fails the day it does not, so the header cannot outlive the fault.
fn known_fault(text: &str) -> Option<String> {
	text.lines().take_while(|l| l.starts_with("//"))
		.find_map(|l| l.strip_prefix("// known:").map(|r| r.trim().to_string()))
}

/// What comparing a directory of fixtures against Typst found.
struct Compared {
	fixtures:	usize,
	lines:		usize,
	indented:	usize,
	pending:	Vec<String>,
	known:		Vec<String>,	// each with the differences it was held to
	failures:	Vec<String>,
}

fn compare_area(o: &Oracle, area: &str, only: Option<&str>) -> Outcome<Compared> {
	let filter = Filter { areas: Some(vec![area.to_string()]), name: only.map(|s| s.to_string()) };
	let fxs = res!(discover(&filter));
	let mut c = Compared { fixtures: fxs.len(), lines: 0, indented: 0, pending: Vec::new(), known: Vec::new(), failures: Vec::new() };
	for fx in &fxs {
		let want = match typst_pages(o, fx) {
			Ok(w)	=> w,
			Err(e)	=> {
				c.failures.push(fmt!("{}: {}", fx.id(), e));
				continue;
			}
		};
		c.lines += want.iter().map(|p| p.lines.len()).sum::<usize>();
		if let Some((n, x)) = indents_asked(&fx.text) {
			let got = want.iter().flat_map(|p| p.lines.iter()).filter(|l| (l.x0 - x).abs() < 0.2).count();
			c.indented += got;
			if got < n {
				c.failures.push(fmt!("{}: asks for at least {} typst lines starting at x {} but typst has {}", fx.id(), n, x, got));
				continue;
			}
		}
		let page = match austenite_page(fx) {
			Ok(p)	=> p,
			Err(e)	=> {
				if needs_unit(&fx.text) {
					c.pending.push(fx.id());
				} else {
					c.failures.push(fmt!("{}: austenite: {}", fx.id(), e));
				}
				continue;
			}
		};
		let mut diffs = Vec::new();
		compare(&want, &[page.clone()], &mut diffs);
		if !diffs.is_empty() && std::env::var("INDENT_DUMP").is_ok() {
			eprintln!("-- {}: austenite runs", fx.id());
			for r in &page.runs {
				eprintln!("   x {:8.2}..{:8.2} base {:8.2} {:?}", r.x0, r.x1, r.base, r.text);
			}
			eprintln!("-- {}: typst lines", fx.id());
			for l in want.iter().flat_map(|p| p.lines.iter()) {
				eprintln!("   x {:8.2}..{:8.2} base {:8.2} {:?}", l.x0, l.x1, l.base, l.text);
			}
		}
		match (known_fault(&fx.text), diffs.is_empty()) {
			(Some(why), true)	=> c.failures.push(fmt!("{} no longer differs from typst; drop its `// known:` header ({})", fx.id(), why)),
			(Some(why), false)	=> c.known.push(fmt!("{}: {} differences, known: {}", fx.id(), diffs.len(), why)),
			(None, false)		=> c.failures.push(fmt!("{} ({} differences):\n  {}", fx.id(), diffs.len(), diffs.join("\n  "))),
			(None, true)		=> (),
		}
	}
	Ok(c)
}

#[test]
fn indented_paragraphs_match_typst_lines_and_starts_at_level_4() -> Outcome<()> {
	let o = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let only = std::env::var("INDENT_FIXTURE").ok();
	let c = res!(compare_area(&o, "indent", only.as_deref()));
	if only.is_none() {
		assert_eq!(c.fixtures, 27, "the indent corpus is twenty-seven fixtures");
		assert!(c.indented >= 40, "typst starts only {} lines at an indent, too few to test indents with", c.indented);
		assert!(c.pending.len() <= 5, "{} fixtures are pending, only the five that need U6b's schema may be: {:?}", c.pending.len(), c.pending);
		assert_eq!(c.known.len(), 1, "one fixture records a known fault: {:?}", c.known);
	}
	if !c.pending.is_empty() {
		eprintln!("pending until their unit is merged: {:?}", c.pending);
	}
	for k in &c.known {
		eprintln!("known: {}", k);
	}
	assert!(c.failures.is_empty(), "{} of {} fixtures ({} typst lines) differ at level 4:\n{}",
		c.failures.len(), c.fixtures, c.lines, c.failures.join("\n"));
	Ok(())
}

#[test]
fn the_line_break_and_hyphenation_corpora_start_their_lines_where_typst_does() -> Outcome<()> {
	let o = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let (mut fixtures, mut lines) = (0usize, 0usize);
	let (mut failures, mut pending) = (Vec::new(), Vec::new());
	for area in ["linebreak", "hyphenation"] {
		let c = res!(compare_area(&o, area, None));
		fixtures += c.fixtures;
		lines += c.lines;
		failures.extend(c.failures);
		pending.extend(c.pending);
	}
	assert_eq!(fixtures, 84 + 27, "the line-break corpus is 84 fixtures and the hyphenation corpus 27");
	// A shorthand such as `--` is text inside its paragraph, as in Typst, so nothing here waits on a unit.
	assert!(pending.is_empty(), "no fixture of these corpora may be pending: {:?}", pending);
	assert!(failures.is_empty(), "{} of {} fixtures ({} typst lines) differ at level 4:\n{}",
		failures.len(), fixtures, lines, failures.join("\n"));
	Ok(())
}

fn oline(text: &str, x0: f64, x1: f64, base: f64) -> OLine {
	OLine { text: text.to_string(), x0, x1, start: x0, y0: base - 8.0, y1: base + 3.0, base, measured: true }
}

fn arun(text: &str, x0: f64, x1: f64, base: f64) -> ARun {
	ARun { text: text.to_string(), x0, x1, base }
}

/// The comparison holds a line to the x it starts at: a line set sideways, which no break and no
/// baseline shows, is reported, and so is a line Typst starts later than Austenite does.
#[test]
fn level_4_reports_a_line_that_starts_in_the_wrong_place() {
	let want = vec![OPage { width: 200.0, height: 100.0, lines: vec![oline("Lorem ipsum dolor", 12.0, 150.0, 20.0)] }];
	let page = |x0: f64| vec![APage { width: 200.0, height: 100.0, runs: vec![arun("Lorem ipsum dolor", x0, x0 + 138.0, 20.0)] }];
	let mut said = Vec::new();
	compare(&want, &page(12.0), &mut said);
	assert!(said.is_empty(), "an exact start was reported: {:?}", said);
	compare(&want, &page(12.0 + START_TOL * 0.5), &mut said);
	assert!(said.is_empty(), "a start within the tolerance was reported: {:?}", said);
	for sideways in [0.0, 6.0, 24.0] {
		let mut said = Vec::new();
		compare(&want, &page(sideways), &mut said);
		assert!(said.iter().any(|m| m.contains("starts at x 12.00 in typst")),
			"a line starting at {} instead of 12 was not reported: {:?}", sideways, said);
	}
}

/// pdftotext cuts a loosely justified line into pieces at its widest gaps, and gives a line the left
/// edge of its second word. Read back, the pieces on one baseline are one line, starting at its first
/// word; a line in another block on the same baseline (the next column) is not joined to it.
#[test]
fn a_line_pdftotext_cut_in_pieces_is_one_line_starting_at_its_first_word() {
	let xml = r#"<doc>
		<page width="200.000000" height="100.000000">
			<flow>
				<block xMin="0" yMin="0" xMax="200" yMax="40">
					<line xMin="60.0" yMin="0.0" xMax="90.0" yMax="12.0">
						<word xMin="12.0" yMin="0.0" xMax="50.0" yMax="12.0">Lorem</word>
						<word xMin="60.0" yMin="0.0" xMax="90.0" yMax="12.0">ipsum</word>
					</line>
					<line xMin="150.0" yMin="0.0" xMax="190.0" yMax="12.0">
						<word xMin="140.0" yMin="0.0" xMax="148.0" yMax="12.0">dolor</word>
						<word xMin="150.0" yMin="0.0" xMax="190.0" yMax="12.0">sit</word>
					</line>
					<line xMin="30.0" yMin="14.0" xMax="60.0" yMax="26.0">
						<word xMin="20.0" yMin="14.0" xMax="28.0" yMax="26.0">amet</word>
						<word xMin="30.0" yMin="14.0" xMax="60.0" yMax="26.0">elit</word>
					</line>
				</block>
				<block xMin="0" yMin="0" xMax="200" yMax="40">
					<line xMin="5.0" yMin="0.0" xMax="9.0" yMax="12.0">
						<word xMin="2.0" yMin="0.0" xMax="4.0" yMax="12.0">other</word>
						<word xMin="5.0" yMin="0.0" xMax="9.0" yMax="12.0">block</word>
					</line>
				</block>
			</flow>
		</page>
	</doc>"#;
	let pages = read_bbox(xml);
	assert_eq!(pages.len(), 1);
	let lines = &pages[0].lines;
	assert_eq!(lines.len(), 3, "two lines in the first block, one in the second: {:?}", lines);
	assert_eq!(lines[0].text, "Lorem ipsum dolor sit", "the pieces of one baseline join in order");
	assert_eq!((lines[0].x0, lines[0].x1), (12.0, 190.0), "the line spans its words, not the second word's left edge");
	assert_eq!(lines[1].text, "amet elit");
	assert_eq!(lines[1].x0, 20.0);
	assert_eq!(lines[2].text, "other block", "a line in another block stays its own, though it shares a baseline");
}

/// pdftotext's words leave out a space, so a line that opens with one (after an inline box, say) is
/// read to start at its first word. Typst's SVG draws the run from its start, the space it leaves out
/// included, and Austenite's run opens with the space, so a line starts where the run holding its first
/// word starts: here 152, not 159.8.
#[test]
fn a_line_that_opens_with_a_space_starts_where_its_text_run_does() {
	let svg = r##"<svg><defs><g id="g1"><path d="M0 0"/></g></defs>
		<g transform="matrix(1 0 0 -1 152 20)"><use xlink:href="#g1" x="7.8" y="0"/><use xlink:href="#g1" x="13" y="0"/></g>
		<g transform="matrix(1 0 0 -1 12 40)"><use xlink:href="#g1" x="0" y="0"/><use xlink:href="#g1" x="6" y="0"/></g>
		<g transform="matrix(1 0 0 -1 100 60)"><use xlink:href="#g1" x="0" y="0"/></g></svg>"##;
	let line = |text: &str, x0: f64, y: f64| OLine {
		text: text.to_string(), x0, x1: x0 + 30.0, start: x0, y0: y - 10.0, y1: y + 2.0, base: 0.0, measured: false,
	};
	let mut page = OPage {
		width:	200.0,
		height:	100.0,
		lines:	vec![line("Supercal", 159.8, 20.0), line("Alpha", 12.0, 40.0), line("Far", 140.0, 60.0)],
	};
	attach_baselines(&mut page, &glyph_origins(svg));
	attach_starts(&mut page, &run_starts(svg));
	assert_eq!(page.lines[0].start, 152.0, "the line opens with a space");
	assert_eq!(page.lines[1].start, 12.0, "a line opening with its first word starts there");
	assert_eq!(page.lines[2].start, 140.0, "a run more than a space's reach before the word is another line's");
	assert_eq!(page.lines[0].x0, 159.8, "the first word is kept");
}
