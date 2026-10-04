//! Page furniture against the `typst` 0.15.1 oracle at level 4 (INT-1): page count, page sizes, and every
//! line's text, baseline and start, for documents whose pages carry a number, a running header, a footer,
//! a background or a foreground.
//!
//! Austenite's side runs the whole pipeline's fixpoint, so a header that reads the introspector (a `context`
//! query, the page counter) settles over its passes as it does in a compile; the final pass's pages are read.
//! The fixtures are `tests/fixtures/eval/furniture/*.typ`: numbered pages, a pattern with a total, a numbering
//! function, a running header from a query, roman then arabic numbering after a counter reset, an odd/even
//! header on two-sided margins, the `none` and `auto` slots, a background and a foreground, the order the
//! furniture's located elements take, and a parity blank page.
//!
//! The test fails when a fixture differs from typst, or when fewer than `MIN_FIXTURES` were compared.

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
use harness::pdf::{
	pdf_differences,
	work_dir,
};

use oxedyne_fe2o3_austenite::eval::fixpoint;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};

use oxedyne_fe2o3_core::prelude::*;

const MIN_FIXTURES: usize = 16;

#[test]
fn page_furniture_matches_the_typst_oracle() -> Outcome<()> {
	let oracle = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let only = std::env::var("EVAL_FURNITURE_FIXTURE").ok();
	let (mut compared, mut failures) = (0usize, Vec::new());
	for fx in res!(corpus::discover(&Filter::only("furniture"))) {
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
		if diffs.is_empty() {
			compared += 1;
			println!("ok      {} ({} pages)", fx.id(), want.len());
		} else {
			failures.push(fmt!("{}:\n    {}", fx.id(), diffs.join("\n    ")));
			if std::env::var("EVAL_FURNITURE_DUMP").is_ok() {
				for (i, p) in want.iter().enumerate() {
					for l in &p.lines {
						println!("typst     p{} x {:7.2} base {:7.2} {:?}", i + 1, l.x0, l.base, l.text);
					}
				}
				for (i, p) in got.iter().enumerate() {
					for r in &p.runs {
						println!("austenite p{} x {:7.2} base {:7.2} {:?}", i + 1, r.x0, r.base, r.text);
					}
				}
			}
		}
	}
	for f in &failures {
		println!("FAIL    {}", f);
	}
	println!("{} furniture fixtures compared at level 4; {} failing", compared, failures.len());
	assert!(failures.is_empty(), "{} fixture(s) differ from typst", failures.len());
	if only.is_none() {
		assert!(compared >= MIN_FIXTURES, "only {} furniture fixtures were compared, {} expected", compared, MIN_FIXTURES);
	}
	Ok(())
}

/// The file Austenite writes carries the furniture too: each fixture is compiled through the streaming
/// `PdfSink`, and the PDF's text and its lines' boxes are read back by the `pdftotext` that reads Typst's.
#[test]
fn page_furniture_reaches_austenites_own_pdf() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let (mut compared, mut failures) = (0usize, Vec::new());
	let root = res!(work_dir("eval-furniture"));
	for fx in res!(corpus::discover(&Filter::only("furniture"))) {
		let diffs = res!(pdf_differences(&fx.path, &root.join(&fx.name)));
		if diffs.is_empty() {
			compared += 1;
			println!("ok      {} (pdf)", fx.id());
		} else {
			failures.push(fmt!("{}:\n    {}", fx.id(), diffs.join("\n    ")));
		}
	}
	for f in &failures {
		println!("FAIL    {}", f);
	}
	println!("{} furniture fixtures compared on austenite's own PDF; {} failing", compared, failures.len());
	assert!(failures.is_empty(), "{} fixture(s) differ from typst on the PDF", failures.len());
	assert!(compared >= MIN_FIXTURES, "only {} PDFs were compared, {} expected", compared, MIN_FIXTURES);
	Ok(())
}
