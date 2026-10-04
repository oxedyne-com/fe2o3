//! Maths on the page, from source to Austenite's own PDF (level 4b): every fixture of the `math` area is
//! compiled through the streaming `PdfSink`, and the file is read back by the `pdftotext -bbox-layout` that
//! reads Typst's, so each line's text and where its box and first word sit are compared as the reader of the
//! PDF sees them. The board's level 4 reads Austenite's placed runs from memory; this reads the file.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::corpus::{
	self,
	Filter,
};
use harness::oracle::Oracle;
use harness::pdf::{
	pdf_differences,
	work_dir,
};

use oxedyne_fe2o3_core::prelude::*;

// Fixtures whose lines differ from Typst's on the file for a reason not yet traced to a root: Typst pushes the
// items of an accent, a matrix's delimiters, a limit's operands, a root's index and a brace's label in an
// order Austenite's frames do not repeat, and a text run's glyphs and spacing are written as one `TJ` by
// Typst and glyph by glyph here, which poppler reads as a word break. Each is a glyph-exact match in
// `eval_math`; the file differs only in the order and grouping of its text. The test requires exactly these
// to differ, so a fix shrinks the list and a new fault is not hidden in it.
const KNOWN: [&str; 13] = [
	"accent_hat", "accent_opts", "call_accents", "call_nested", "call_ops", "call_spread", "call_styles",
	"matrix", "matrix_aug", "root_syntax", "spreaders", "tall_parens", "variants",
];

#[test]
fn maths_reaches_austenites_own_pdf_as_it_reaches_typsts() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let only = std::env::var("EVAL_MATH_PDF_FIXTURE").ok();
	let (mut compared, mut failures) = (0usize, Vec::new());
	let root = res!(work_dir("eval-math-pdf"));
	let mut fixtures = res!(corpus::discover(&Filter::only("math")));
	// A directory of variants of the fixtures to take instead, by name.
	if let Ok(dir) = std::env::var("EVAL_MATH_PDF_DIR") {
		for fx in fixtures.iter_mut() {
			fx.path = std::path::Path::new(&dir).join(format!("{}.typ", fx.name));
		}
	}
	for fx in fixtures {
		if let Some(o) = &only {
			if !fx.name.contains(o.as_str()) {
				continue;
			}
		}
		let diffs = match pdf_differences(&fx.path, &root.join(&fx.name)) {
			Ok(d)	=> d,
			Err(e)	=> vec![fmt!("{}", e)],
		};
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
	println!("{} maths fixtures agree on austenite's own PDF; {} differ", compared, failures.len());
	if only.is_none() {
		let differing: Vec<String> = failures.iter()
			.map(|f| f.split(':').next().unwrap_or("").trim_start_matches("math/").to_string()).collect();
		let unexpected: Vec<&String> = differing.iter().filter(|n| !KNOWN.contains(&n.as_str())).collect();
		let mended: Vec<&&str> = KNOWN.iter().filter(|k| !differing.iter().any(|n| n == **k)).collect();
		assert!(unexpected.is_empty(), "fixtures differ from typst on the PDF that are not known to: {:?}", unexpected);
		assert!(mended.is_empty(), "fixtures in KNOWN now agree, so take them off the list: {:?}", mended);
		assert!(compared >= 88, "only {} maths fixtures agree on the PDF, 88 expected", compared);
	}
	Ok(())
}
