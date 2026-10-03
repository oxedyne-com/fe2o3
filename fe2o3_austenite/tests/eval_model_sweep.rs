//! Every model element set once, against the `typst` 0.15.1 oracle (U6b-S2): no element may be set otherwise
//! than Typst sets it without a warning saying so.
//!
//! Each fixture under `tests/fixtures/eval/sweep/` is a minimal document that sets one element of Typst's
//! model (a heading, a list, a figure, a table, a quote, a reference, a footnote, an outline...) with its default
//! show, and, where the element has settable parts, a set rule on them. Austenite lays each out through the
//! whole pipeline and the page count, every line's text, baseline and start are compared with Typst's. A fixture
//! passes when the two agree, or when they differ and Austenite said so with a warning of kind `unsupported`;
//! one that differs in silence fails the suite. So a construct can never again be set wrongly without a warning:
//! when a layout is made right its warning goes, and the fixture is then held to the comparison alone.
//!
//! The suite also fails when a model element has no fixture, or fewer fixtures than `MIN_FIXTURES` were found.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::austenite::{
	pages,
	plain,
	Keep,
};
use harness::corpus::{
	self,
	Filter,
};
use harness::layout::compare;
use harness::oracle::Oracle;

use oxedyne_fe2o3_austenite::diag::{
	DiagnosticKind,
	Severity,
};
use oxedyne_fe2o3_austenite::eval::content::{
	ElemKind,
	Family,
};
use oxedyne_fe2o3_austenite::eval::fixpoint;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};

use oxedyne_fe2o3_core::prelude::*;

const MIN_FIXTURES: usize = 19;

/// The model elements a fixture of their own sets: its name is the fixture's. The others are held by one of
/// these (an item by its list, a caption by its figure) or are what realisation makes.
const ELEMENTS: &[&str] = &[
	"heading", "par", "list", "enum", "terms", "figure", "caption", "table", "quote", "quote_inline", "raw",
	"link", "ref", "cite", "footnote", "outline", "title", "bibliography", "strong", "emph",
];

/// How one fixture came out.
#[derive(Debug)]
enum Verdict {
	Match,
	Warned(String),		// differs, with an `unsupported` warning: the head of its message
	Silent(String),		// differs with no such warning: the fault
}

#[test]
fn every_model_element_matches_typst_or_warns_unsupported() -> Outcome<()> {
	let oracle = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let only = std::env::var("EVAL_SWEEP_FIXTURE").ok();
	let (mut matched, mut warned, mut failures) = (0usize, 0usize, Vec::new());
	let mut found = Vec::new();
	for fx in res!(corpus::discover(&Filter::only("sweep"))) {
		found.push(fx.name.clone());
		if let Some(o) = &only {
			if !fx.name.contains(o.as_str()) {
				continue;
			}
		}
		let want = match res!(oracle.layout(&fx)) {
			Ok(w)	=> w,
			Err(e)	=> {
				failures.push(fmt!("{}: typst: {}", fx.id(), e));
				continue;
			},
		};
		let mut world = World::new(fx.root.clone());
		let id = match world.load(&fx.path) {
			Ok(id)	=> id,
			Err(e)	=> {
				failures.push(fmt!("{}: load: {}", fx.id(), plain(&e)));
				continue;
			},
		};
		let mut engine = Engine::new(world);
		let module = match eval_source(&mut engine, id) {
			Ok(m)	=> m,
			Err(e)	=> {
				failures.push(fmt!("{}: eval: {}", fx.id(), plain(&e)));
				continue;
			},
		};
		let mut kept = Keep::default();
		if let Err(e) = fixpoint::run(&mut engine, &module, &mut kept) {
			failures.push(fmt!("{}: layout: {}", fx.id(), plain(&e)));
			continue;
		}
		let got = pages(&kept.pages);
		let mut diffs = Vec::new();
		compare(&want, &got, &mut diffs);
		let warning = engine.diags.iter()
			.find(|d| d.severity == Severity::Warning && d.kind == DiagnosticKind::Unsupported)
			.map(|d| d.head().to_string());
		let verdict = match (diffs.is_empty(), warning) {
			(true, _)			=> Verdict::Match,
			(false, Some(w))	=> Verdict::Warned(w),
			(false, None)		=> Verdict::Silent(diffs.iter().take(3).cloned().collect::<Vec<_>>().join(" | ")),
		};
		match &verdict {
			Verdict::Match		=> {
				matched += 1;
				println!("match     {}", fx.id());
			},
			Verdict::Warned(w)	=> {
				warned += 1;
				println!("warned    {} ({})", fx.id(), w);
			},
			Verdict::Silent(d)	=> {
				failures.push(fmt!("{} differs from typst and carries no `unsupported` warning: {}", fx.id(), d));
			},
		}
	}
	if only.is_none() {
		for e in ELEMENTS {
			if !found.iter().any(|f| f == e) {
				failures.push(fmt!("the model element `{}` has no fixture under sweep/", e));
			}
		}
		// Every model element of the registry is one of those above or part of one.
		for k in ElemKind::ALL.iter().filter(|k| k.family() == Family::Model) {
			let head = k.path().split('.').next().unwrap_or("");
			if !ELEMENTS.contains(&head) && !PARTS.contains(&head) {
				failures.push(fmt!("the model element `{}` is in neither ELEMENTS nor PARTS", k.path()));
			}
		}
	}
	println!("[sweep] {} fixture(s) match typst, {} warn `unsupported`, {} failure(s)", matched, warned, failures.len());
	for f in &failures {
		println!("FAIL    {}", f);
	}
	assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
	if only.is_none() {
		assert!(matched + warned >= MIN_FIXTURES, "only {} sweep fixture(s) compared; {} are expected",
			matched + warned, MIN_FIXTURES);
	}
	Ok(())
}

/// Model elements that are parts of the ones above, or what the model itself builds from them.
const PARTS: &[&str] = &["document", "parbreak", "divider", "bibliography_entry", "outline_entry", "footnote_entry", "cite-group"];
