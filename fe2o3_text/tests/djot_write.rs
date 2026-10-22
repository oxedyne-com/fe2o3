//! The Djot writer's invariant, `parse(&render(&d)) == d`, over generated trees and the escaping
//! cases one by one.

use oxedyne_fe2o3_text::doc::{
	Align,
	Attrs,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
	Term,
	djot,
};

use oxedyne_fe2o3_core::prelude::*;

/// A small deterministic generator, so a failure names its seed and reproduces.
struct Gen(u64);

impl Gen {
	fn next(&mut self) -> u64 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0
	}
	fn below(&mut self, n: usize) -> usize {
		(self.next() % n as u64) as usize
	}
	fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
		xs[self.below(xs.len())]
	}
	fn chance(&mut self, n: usize) -> bool {
		self.below(n) == 0
	}
}

// Every Djot metacharacter, a leading block opener of each kind, and plain words.
const WORDS: &[&str] = &[
	"word", "café", "naïve", "x", "*", "_", "[", "]", "{", "}", "\\", "<", ">", "#", "-", "+",
	"1.", "2)", "12.", "|", "^", "~", "!", ":::", ":", "`", "``", "a_b", "*star*", "{_", "_}",
	"(", ")", "&", "$", "%", "\"", "'", "=", "---", "***", ">x", "#x", "^ cap", "!",
];
const NAMES: &[&str] = &["note", "warn", "a1", "x-y", "big_one", "c"];
const VALUES: &[&str] = &["v", "two words", "1/2", "a}b", "", "x-y.z", "{q}"];
const DESTS: &[&str] = &[
	"https://example.com/a", "page.html", "a b.png", "", "x(1).png", "q\\r", "w[1]", "t`t", "p|q",
	"<x>", "a)b",
];
const CODES: &[&str] = &["x", "a|b", "`", "a``b", " lead", "trail ", " both ", "  ", "*_[{"];
const LINES: &[&str] = &["fn main() {}", "", "  indented", "```", "~~~ x", "a | b", "\t tab", "> q"];

/// One text run, never empty, and trimmed where `trim` says the run stands at an edge.
fn text(g: &mut Gen) -> String {
	let n = 1 + g.below(3);
	let mut s = String::new();
	for i in 0..n {
		if i > 0 && !g.chance(3) {
			s.push(' ');
		}
		s.push_str(g.pick(WORDS));
	}
	s
}

fn attrs(g: &mut Gen) -> Attrs {
	let mut a = Attrs::default();
	if g.chance(2) {
		a.id = Some(g.pick(NAMES).to_string());
	}
	for _ in 0..g.below(3) {
		a.classes.push(g.pick(NAMES).to_string());
	}
	for _ in 0..g.below(3) {
		a.pairs.push((g.pick(NAMES).to_string(), g.pick(VALUES).to_string()));
	}
	a
}

/// A run of inlines: `brk` allows hard breaks, which a cell and a caption cannot hold.
fn inlines(g: &mut Gen, depth: usize, brk: bool) -> Vec<Inline> {
	let n = 1 + g.below(4);
	let mut out: Vec<Inline> = Vec::new();
	for _ in 0..n {
		let k = if depth == 0 { 0 } else { g.below(11) };
		let item = match k {
			1	=> Inline::Emph { strong: g.chance(2), content: inlines(g, depth - 1, false) },
			2	=> Inline::Link { to: g.pick(DESTS).to_string(), content: inlines(g, depth - 1, false) },
			3	=> Inline::Image {
				src:	g.pick(DESTS).to_string(),
				alt:	if g.chance(4) { String::new() } else { text(g) },
				attrs:	if g.chance(2) { Attrs::default() } else { attrs(g) },
			},
			4	=> Inline::Code(g.pick(CODES).to_string()),
			5	=> Inline::Span { attrs: attrs(g), content: inlines(g, depth - 1, false) },
			6 if brk => Inline::Break,
			7	=> Inline::Sup(inlines(g, depth - 1, false)),
			8	=> Inline::Sub(inlines(g, depth - 1, false)),
			9	=> Inline::Emph { strong: true, content: inlines(g, depth - 1, false) },
			_	=> Inline::Text(text(g)),
		};
		// Two code spans side by side are one run of backticks in Djot, so no reader makes them.
		let item = match (out.last(), item) {
			(Some(Inline::Code(_)), Inline::Code(_))			=> Inline::Text("x".into()),
			(_, item)						=> item,
		};
		// Text merges with text beside it, as the reader would merge it.
		match (out.last_mut(), &item) {
			(Some(Inline::Text(a)), Inline::Text(b))	=> a.push_str(b),
			_						=> out.push(item),
		}
	}
	out
}

/// A run fit to stand as a whole block: no whitespace at either end, and no hard break last, nor a
/// break followed by whitespace.
fn edge(mut v: Vec<Inline>) -> Vec<Inline> {
	let mut out: Vec<Inline> = Vec::new();
	for (i, item) in v.drain(..).enumerate() {
		match item {
			Inline::Text(t)	=> {
				let after_break = matches!(out.last(), Some(Inline::Break));
				let t = if i == 0 || after_break { t.trim_start().to_string() } else { t };
				if !t.is_empty() {
					out.push(Inline::Text(t));
				}
			}
			other		=> out.push(other),
		}
	}
	while matches!(out.last(), Some(Inline::Break)) {
		out.pop();
	}
	if let Some(Inline::Text(t)) = out.last_mut() {
		let n = t.trim_end().len();
		t.truncate(n);
		if t.is_empty() {
			out.pop();
		}
	}
	if out.is_empty() {
		out.push(Inline::Text("w".to_string()));
	}
	out
}

/// A row of cells, each a header where `header` says, or now and then the other way.
fn cells(g: &mut Gen, n: usize, depth: usize, header: bool) -> Row {
	Row((0..n).map(|_| {
		let content = match g.chance(5) {
			true	=> Vec::new(),
			false	=> edge(inlines(g, depth, false)),
		};
		Cell { content, header: header != g.chance(6) }
	}).collect())
}

fn blocks(g: &mut Gen, depth: usize) -> Vec<Block> {
	(0..1 + g.below(3)).map(|_| block(g, depth)).collect()
}

fn block(g: &mut Gen, depth: usize) -> Block {
	let k = if depth == 0 { g.below(4) } else { g.below(11) };
	let d = depth.saturating_sub(1);
	match k {
		0	=> Block::Para(edge(inlines(g, depth.min(3), true))),
		1	=> Block::Heading { level: 1 + g.below(6) as u8, content: edge(inlines(g, depth.min(3), true)) },
		2	=> Block::Rule,
		3	=> {
			let mut text = String::new();
			for _ in 0..g.below(4) {
				text.push_str(g.pick(LINES));
				text.push('\n');
			}
			Block::Code { lang: if g.chance(2) { Some("rust".to_string()) } else { None }, text }
		}
		4	=> Block::List {
			ordered:	g.chance(2),
			items:		(0..1 + g.below(3)).map(|_| match g.chance(6) {
				true	=> Vec::new(),
				false	=> blocks(g, d),
			}).collect(),
		},
		5	=> Block::Quote(blocks(g, d)),
		6	=> {
			let n = 1 + g.below(3);
			let cols = (0..n).map(|_| [Align::None, Align::Start, Align::Centre, Align::End][g.below(4)]).collect();
			let head = if g.chance(3) { None } else { Some(cells(g, n, d.min(2), true)) };
			let mut rows: Vec<Row> = (0..g.below(4)).map(|_| cells(g, n, d.min(2), false)).collect();
			// The page case: the first cell of every body row heads its row.
			if g.chance(4) {
				for r in rows.iter_mut() {
					for (i, c) in r.0.iter_mut().enumerate() {
						c.header = i == 0;
					}
				}
			}
			Block::Table {
				head,
				rows,
				cols,
				caption:	if g.chance(2) { Vec::new() } else { edge(inlines(g, d.min(2), false)) },
			}
		}
		9	=> Block::Figure {
			attrs:		if g.chance(2) { Attrs::default() } else { attrs(g) },
			content:	if g.chance(6) { Vec::new() } else { blocks(g, d) },
			caption:	if g.chance(3) { Vec::new() } else { edge(inlines(g, d.min(2), true)) },
		},
		10	=> Block::Terms {
			attrs:	if g.chance(2) { Attrs::default() } else { attrs(g) },
			items:	(0..1 + g.below(3)).map(|_| Term {
				term:	if g.chance(6) { Vec::new() } else { edge(inlines(g, d.min(2), true)) },
				defs:	(0..g.below(3)).map(|_| match g.chance(5) {
					true	=> Vec::new(),
					false	=> blocks(g, d),
				}).collect(),
			}).collect(),
		},
		_	=> Block::Div {
			attrs:		if g.chance(4) { Attrs::default() } else { attrs(g) },
			content:	if g.chance(6) { Vec::new() } else { blocks(g, d) },
		},
	}
}

/// Lists side by side merge unless the second takes the other marker, and a list with a sibling
/// list of the other kind is its own; both are what the writer must keep apart.
fn round(d: &Doc) -> Outcome<()> {
	let text = djot::render(d);
	let back = res!(djot::parse(&text));
	if back != *d {
		return Err(err!("Round trip changed the tree.\n--- djot ---\n{}\n--- want ---\n{:?}\n--- got ---\n{:?}",
			text, d, back; Invalid, Output));
	}
	Ok(())
}

#[test]
fn test_round_trip_over_generated_trees() -> Outcome<()> {
	let mut seen = std::collections::BTreeSet::new();
	for seed in 1..=5000u64 {
		let mut g = Gen(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
		let doc = Doc { blocks: blocks(&mut g, 4) };
		let dbg = fmt!("{:?}", doc);
		for name in ["Heading", "Para", "List", "Code", "Quote", "Table", "Rule", "Div", "Text", "Emph",
			"Link", "Image", "Span", "Break", "Sup", "Sub", "caption: [", "head: None", "Centre",
			"Figure", "Terms", "defs: []", "term: []", "header: true", "header: false"]
		{
			if dbg.contains(name) {
				seen.insert(name);
			}
		}
		if round(&doc).is_err() {
			let small = shrink(doc.blocks.clone());
			let e = round(&Doc { blocks: small }).err().map(|e| fmt!("{}", e)).unwrap_or_default();
			return Err(err!("Seed {}: {}", seed, e; Invalid, Output));
		}
	}
	assert_eq!(seen.len(), 25, "variants seen: {:?}", seen);
	Ok(())
}

/// The smallest run of blocks within a failing one that still fails, for a readable report.
fn shrink(bs: Vec<Block>) -> Vec<Block> {
	let fails = |v: &Vec<Block>| round(&Doc { blocks: v.clone() }).is_err();
	// One block alone, or any pair of neighbours, that still fails.
	for i in 0..bs.len() {
		for j in i + 1..=bs.len().min(i + 2) {
			let v = bs[i..j].to_vec();
			if v.len() < bs.len() && fails(&v) {
				return shrink(v);
			}
		}
	}
	// Then the children of each block.
	for b in &bs {
		let kids: Vec<Vec<Block>> = match b {
			Block::Div { content, .. }	=> vec![content.clone()],
			Block::Quote(c)			=> vec![c.clone()],
			Block::List { items, .. }	=> items.clone(),
			Block::Figure { content, .. }	=> vec![content.clone()],
			Block::Terms { items, .. }	=> items.iter().flat_map(|t| t.defs.clone()).collect(),
			_				=> Vec::new(),
		};
		for k in kids {
			if fails(&k) {
				return shrink(k);
			}
		}
	}
	bs
}

fn para(v: Vec<Inline>) -> Doc {
	Doc { blocks: vec![Block::Para(v)] }
}

fn t(s: &str) -> Inline {
	Inline::Text(s.to_string())
}

#[test]
fn test_escaping_case_by_case() -> Outcome<()> {
	// A paragraph that starts like an ordered list, and its parenthesis form.
	res!(round(&para(vec![t("1. not a list")])));
	res!(round(&para(vec![t("1) nor this")])));
	// Each mark alone.
	for s in ["a literal _ here", "a * star", "an open [ bracket", "a { brace", "a } brace",
		"x^2 and H~2", "<https://not.a/link>", "trailing!", "a ` tick"]
	{
		res!(round(&para(vec![t(s)])));
	}
	// A backslash at the end of a line, before a break and at the very end.
	res!(round(&para(vec![t("ends \\"), Inline::Break, t("next")])));
	res!(round(&para(vec![t("ends \\")])));
	// Every block opener at the start of a line, and again after a break.
	for s in ["# no heading", "> no quote", "- no item", "+ no item", "::: no div", "| no table",
		"^ no caption", "*** no rule", "``` no fence", "~~~ no fence", "{.no attrs}", "[no]: ref"]
	{
		res!(round(&para(vec![t(s)])));
		res!(round(&para(vec![t("lead"), Inline::Break, t(s)])));
	}
	// A `!` before a link is not an image.
	res!(round(&para(vec![t("bang!"), Inline::Link { to: "x".into(), content: vec![t("y")] }])));
	Ok(())
}

#[test]
fn test_reader_learns_sup_sub_and_captions() -> Outcome<()> {
	let d = res!(djot::parse("H~2~O and x^2^.\n"));
	assert_eq!(d.blocks, vec![Block::Para(vec![
		t("H"), Inline::Sub(vec![t("2")]), t("O and x"), Inline::Sup(vec![t("2")]), t("."),
	])]);
	let d = res!(djot::parse("| a |\n|---|\n| b |\n\n^ The caption.\n"));
	match &d.blocks[0] {
		Block::Table { caption, .. }	=> assert_eq!(caption, &vec![t("The caption.")]),
		other				=> return Err(err!("Not a table: {:?}", other; Invalid, Output)),
	}
	// Forced emphasis around a space.
	let d = res!(djot::parse("{_ spaced _}\n"));
	assert_eq!(d.blocks, vec![Block::Para(vec![Inline::Emph { strong: false, content: vec![t(" spaced ")] }])]);
	Ok(())
}

#[test]
fn test_reader_learns_the_page_constructs() -> Outcome<()> {
	// Two terms, the second with two definitions kept apart, and attrs above the list.
	let d = res!(djot::parse("{.terms}\n: Bark\n\n  A tree's skin.\n\n  ::\n\n  A dog's call.\n\n: Jaw\n"));
	assert_eq!(d.blocks, vec![Block::Terms {
		attrs:	Attrs { id: None, classes: vec!["terms".into()], pairs: Vec::new() },
		items:	vec![
			Term { term: vec![t("Bark")], defs: vec![
				vec![Block::Para(vec![t("A tree's skin.")])],
				vec![Block::Para(vec![t("A dog's call.")])],
			] },
			Term { term: vec![t("Jaw")], defs: Vec::new() },
		],
	}]);
	// A figure and its caption; a division classed `figure` stays a division.
	let d = res!(djot::parse("::: figure {#f}\n![x](a.png)\n\n^ A caption.\n:::\n\n::: {.figure}\nP\n:::\n"));
	assert_eq!(d.blocks[0], Block::Figure {
		attrs:		Attrs { id: Some("f".into()), classes: Vec::new(), pairs: Vec::new() },
		content:	vec![Block::Para(vec![Inline::Image { src: "a.png".into(), alt: "x".into(), attrs: Attrs::default() }])],
		caption:	vec![t("A caption.")],
	});
	assert!(matches!(&d.blocks[1], Block::Div { .. }), "got {:?}", d.blocks[1]);
	// Row headers in the short form and the long, and a heads value that fits nothing stays a pair.
	let d = res!(djot::parse("{heads=\"rows\"}\n| --- | --- |\n| a | 1 |\n"));
	match &d.blocks[0] {
		Block::Table { rows, .. }	=> assert_eq!((rows[0].0[0].header, rows[0].0[1].header), (true, false)),
		other				=> return Err(err!("Not a table: {:?}", other; Invalid, Output)),
	}
	let d = res!(djot::parse("{heads=\"01 10\"}\n| h | i |\n|---|---|\n| a | 1 |\n"));
	match &d.blocks[0] {
		Block::Table { head: Some(h), rows, .. } => {
			assert_eq!((h.0[0].header, h.0[1].header), (false, true));
			assert_eq!((rows[0].0[0].header, rows[0].0[1].header), (true, false));
		}
		other				=> return Err(err!("Not a headed table: {:?}", other; Invalid, Output)),
	}
	let d = res!(djot::parse("{heads=\"x\"}\n| a |\n|---|\n"));
	assert!(matches!(&d.blocks[0], Block::Div { .. }), "got {:?}", d.blocks[0]);
	// A line of prose that opens with a colon does not break a paragraph.
	let d = res!(djot::parse("Note\n: not a term\n"));
	assert_eq!(d.blocks, vec![Block::Para(vec![t("Note : not a term")])]);
	Ok(())
}
