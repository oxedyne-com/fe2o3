//! U4-fix: a symbol realises as text, as Typst's realisation makes it, so a shorthand stays in its paragraph.
//!
//! A shorthand (`--`, `---`, `...`, `~`, `-?`), an escape and a symbol value evaluate to `symbol`
//! elements. Typst turns each into a `text` element before any show rule or grouping runs outside maths
//! (`typst-realize`, `visit_kind_rules`), so the symbol joins the paragraph and the textual run it stands
//! in. Realisation that left it a symbol split the paragraph in two at every ` -- ` and dropped the dash.
//!
//! - The `realise` corpus (`tests/fixtures/eval/realise/`) holds each kind of symbol inside paragraphs,
//!   headings, tight lists, terms, strong and emphasis, with and without a text show rule that reaches
//!   it. Each fixture is held to the structure of Typst's own realisation, at level 3.
//! - In maths a symbol stays a symbol and a text or regex rule reaches it one element at a time. That
//!   has no HTML to compare, so the equation of `math_rule.typ` is held to the text Typst sets for it.
//!   Until the evaluator has a maths schema, Austenite's side is the equation's content built by hand.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::corpus::{
	discover,
	Expect,
	Filter,
	Fixture,
};
use harness::oracle::Oracle;
use harness::Verdict;

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	ElemKind,
};
use oxedyne_fe2o3_austenite::eval::realise::{
	realise,
	RealiseMode,
};
use oxedyne_fe2o3_austenite::eval::select::Selector;
use oxedyne_fe2o3_austenite::eval::styles::{
	Recipe,
	Style,
	StyleChain,
	Styles,
	Transformation,
};
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::sync::Arc;

const AREA: &str = "realise";

/// The fixtures the corpus must hold; the count can only go up.
const FIXTURES: usize = 11;

fn oracle() -> Outcome<Option<Oracle>> {
	let o = res!(Oracle::find());
	if o.is_none() {
		println!("[eval-realise] SKIPPED: EVAL_ORACLE_SKIP=1 and no typst 0.15.x -- nothing was compared");
	}
	Ok(o)
}

#[test]
fn symbols_in_markup_stay_in_their_paragraph_as_typst_realises_them() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let all = res!(discover(&Filter::only(AREA)));
	assert!(all.len() >= FIXTURES, "{} fixtures under tests/fixtures/eval/{}, {} expected", all.len(), AREA, FIXTURES);
	let limit = harness::timeout();
	let mut bad = Vec::new();
	for fx in &all {
		assert!(fx.wants(3) && !fx.wants(1) && !fx.wants(2) && !fx.wants(4),
			"{} must ask for level 3 alone", fx.id());
		let rep = res!(harness::check(fx, &o, limit));
		if let Some(f) = &rep.fault {
			bad.push(fmt!("{}: {}", fx.id(), f));
			continue;
		}
		match &rep.levels[2] {
			Verdict::Pass		=> (),
			Verdict::Fail(d)	=> bad.push(fmt!("{}: {}", fx.id(), d.join("; "))),
			// A fixture Typst's HTML export cannot compare would pass without being held to anything.
			other				=> bad.push(fmt!("{}: level 3 was not compared ({:?})", fx.id(), other)),
		}
	}
	println!("[eval-realise] {} fixtures compared at level 3, {} differ", all.len(), bad.len());
	assert!(bad.is_empty(), "{} fixture(s) differ from Typst's realisation:\n{}", bad.len(), bad.join("\n"));
	Ok(())
}

/// The characters of a page's text, as the PDF holds them, with all whitespace gone.
fn typst_text(o: &Oracle, fx: &Fixture) -> Outcome<String> {
	let pages = match res!(o.layout(fx)) {
		Ok(p)	=> p,
		Err(e)	=> return Err(err!("typst cannot lay {} out: {}", fx.id(), e; Test)),
	};
	let mut s = String::new();
	for p in &pages {
		for l in &p.lines {
			s.push_str(&l.text);
		}
	}
	Ok(s.chars().filter(|c| !c.is_whitespace()).collect())
}

fn show_text(sel: &str, out: &str) -> Style {
	Style::Recipe(Arc::new(Recipe {
		selector:	Some(Selector::Text(sel.into())),
		transform:	Transformation::Content(Content::text(out)),
		span:		Span::detached(),
		outside:	false,
	}))
}

/// The equation of `math_rule.typ`, `$ x + x -> x $`, as maths evaluates it: a character is a symbol and
/// the arrow shorthand is one, a space is a space. The evaluator has no maths schema on this branch, so
/// the content is built as the evaluator builds it, and realised in maths under the fixture's two rules.
fn austenite_math_text() -> Outcome<String> {
	let sp = || Content::marker(ElemKind::Space, Span::detached());
	let body = Content::sequence(vec![
		Content::symbol("x"), sp(), Content::symbol("+"), sp(), Content::symbol("x"), sp(),
		Content::symbol("\u{2192}"), sp(), Content::symbol("x"),
	]);
	let mut chain = StyleChain::root();
	for st in [show_text("x", "Z"), show_text("\u{2192}", "R")] {
		chain = chain.chain(&Styles::from_style(st));
	}
	let mut e = Engine::new(World::new(PathBuf::from("/")));
	let mut s = String::new();
	for q in res!(realise(&mut e, &body, &chain, RealiseMode::Math)) {
		if q.tag.is_none() {
			s.push_str(&q.content.plain_text());
		}
	}
	Ok(s.chars().filter(|c| !c.is_whitespace()).collect())
}

#[test]
fn a_text_rule_reaches_the_symbols_of_an_equation() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let path = harness::corpus::fixtures_dir().join(AREA).join("math_rule.typ");
	let text = res!(std::fs::read_to_string(&path).map_err(|e| err!("Cannot read {}: {}", path.display(), e; IO, File, Read)));
	let root: PathBuf = harness::corpus::fixtures_dir().join(AREA);
	let fx = Fixture {
		area:	AREA.to_string(),
		name:	"math_rule".to_string(),
		path,
		root,
		text,
		levels:	[false, false, false, true],
		expect:	Expect::Accepts,
	};
	let want = res!(typst_text(&o, &fx));
	// Typst sets the replaced `x` and the arrow as `Z` and `R`; nothing of the source is left.
	assert_eq!(want, "Z+ZRZ", "the oracle's text for the equation");
	let got = res!(austenite_math_text());
	assert_eq!(got, want, "Austenite's realised equation against Typst's text");
	Ok(())
}

#[test]
fn a_text_rule_slices_a_symbol_longer_than_a_character() -> Outcome<()> {
	// Typst's `slice_textual` cuts a symbol as it cuts text, though a symbol is usually one character:
	// an emoji sequence or a decomposed letter is several, and a rule can match a part of it. No oracle
	// can be asked, since markup cannot make such a symbol outside an equation; the expectation is
	// `typst-realize`'s `visit_regex_match`, which keeps the unmatched parts as symbols.
	let body = Content::symbol("a\u{2192}b");
	let chain = StyleChain::root().chain(&Styles::from_style(show_text("\u{2192}", "R")));
	let mut e = Engine::new(World::new(PathBuf::from("/")));
	let pairs = res!(realise(&mut e, &body, &chain, RealiseMode::Math));
	let parts: Vec<String> = pairs.iter().filter(|p| p.tag.is_none()).map(|p| p.content.plain_text()).collect();
	assert_eq!(parts, vec!["a".to_string(), "R".to_string(), "b".to_string()], "the symbol cut around the match");
	assert!(pairs.iter().filter(|p| p.tag.is_none()).next().map(|p| p.content.is(ElemKind::Symbol)).unwrap_or(false),
		"the part before the match stays a symbol");
	Ok(())
}
