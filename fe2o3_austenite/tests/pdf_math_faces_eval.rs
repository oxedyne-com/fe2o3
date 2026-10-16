//! The maths faces of a PDF written through the evaluator are the faces Typst writes: an equation is set in
//! New Computer Modern Math Book, and text that asks for the family in bold is set in its Bold face. The
//! expectation is Typst's own PDF of the same source, its font names read by the same `pdffonts`.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::oracle::Oracle;
use harness::pdf::{
	austenite_pdf,
	tool,
	typst_pdf,
	work_dir,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;
use std::path::Path;

// An equation, and the family asked for in bold.
const SOURCE: &str = "#set page(width: 200pt, height: 80pt, margin: 10pt)\n\
	An equation $x^2 + y$ and #text(font: \"New Computer Modern Math\", weight: \"bold\")[bold].\n";

// The base names of the fonts `pdffonts` lists, without the subset tag and without `-Identity-H`.
fn names(pdf: &Path) -> Outcome<BTreeSet<String>> {
	let listing = res!(tool("pdffonts", &[], pdf, &[]));
	let mut out = BTreeSet::new();
	for line in listing.lines().skip(2) {
		if let Some(name) = line.split_whitespace().next() {
			let name = name.split_once('+').map(|(_, n)| n).unwrap_or(name);
			out.insert(name.strip_suffix("-Identity-H").unwrap_or(name).to_string());
		}
	}
	Ok(out)
}

#[test]
fn an_equation_is_set_in_the_book_face_and_bold_text_in_the_bold_face_as_typst_sets_them() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let dir = res!(work_dir("pdf-math-faces-eval"));
	let src = dir.join("main.typ");
	res!(std::fs::write(&src, SOURCE));
	let theirs = dir.join("typst.pdf");
	res!(typst_pdf(&src, &theirs));
	let made = res!(austenite_pdf(&src));
	let ours = dir.join("austenite.pdf");
	res!(std::fs::write(&ours, &made.bytes));
	let (want, got) = (res!(names(&theirs)), res!(names(&ours)));
	// Fixed by construction, so a comparison of two sets that are both missing the faces cannot pass.
	assert!(want.contains("NewCMMath-Book") && want.contains("NewCMMath-Bold"), "Typst's fonts: {:?}", want);
	assert!(!want.contains("NewCMMath-Regular"), "Typst's fonts: {:?}", want);
	assert_eq!(got, want, "the fonts differ from Typst's");
	Ok(())
}
