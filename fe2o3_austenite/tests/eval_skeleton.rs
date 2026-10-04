//! The walking skeleton, from source to PDF through the evaluator's own pipeline: `compile::assemble_eval`
//! with the streaming `PdfSink`, then the file Austenite wrote is judged against what `typst` 0.15.1
//! writes for the same source (streaming addendum, section 5):
//!
//! - K1: the file is a PDF of one page whose MediaBox is Typst's.
//! - K2: `pdftotext` of Austenite's PDF equals Typst's after whitespace normalisation, "Section: Skeleton"
//!   included, which shows the show rule ran.
//! - K3: level 4b, on Austenite's own PDF: each line's text, and where its box and its first word sit on
//!   the page, lie within a point of Typst's. Both PDFs are read by the same `pdftotext -bbox-layout`, so
//!   the face-dependent offset of a box from its baseline is common to both.
//! - K4: the strict compile has no diagnostic and no refusal.
//!
//! K5, the heap and size, is measured by `tools/heap_probe.mjs` and gated at M4.

#[allow(dead_code)]
#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::pdf::{
	self,
	k3_differences,
	lines_of,
	page_size,
	text_of,
	typst_pdf,
};

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

fn skeleton() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/skeleton/skeleton.typ")
}

fn work_dir() -> Outcome<PathBuf> { pdf::work_dir("eval-skeleton") }

#[test]
fn the_skeleton_is_a_pdf_of_one_a4_page_with_typsts_text_and_lines_and_no_diagnostic() -> Outcome<()> {
	let src		= skeleton();
	let dir		= res!(work_dir());
	let made	= res!(pdf::austenite_pdf(&src));
	assert_eq!(made.pages, 1, "the skeleton is one page");
	let (bytes, report) = (made.bytes, made.report);
	let ours	= dir.join("austenite.pdf");
	let theirs	= dir.join("typst.pdf");
	res!(std::fs::write(&ours, &bytes));
	res!(typst_pdf(&src, &theirs));

	// K1
	assert!(bytes.starts_with(b"%PDF-"), "the output is a PDF");
	let text = String::from_utf8_lossy(&bytes);
	assert!(text.contains("/Count 1"), "the page tree counts one page");
	let (mine, want) = (res!(page_size(&ours)), res!(page_size(&theirs)));
	assert!((mine.0 - want.0).abs() < 0.01 && (mine.1 - want.1).abs() < 0.01, "MediaBox {:?} is not typst's {:?}", mine, want);
	assert!((mine.0 - 595.276).abs() < 0.01 && (mine.1 - 841.89).abs() < 0.01, "the page is A4, found {:?}", mine);
	assert!(text.contains("/MediaBox [0 0 595.276 841.89]"), "austenite writes the A4 MediaBox, as typst does");

	// K2
	let (a, t) = (res!(text_of(&ours)), res!(text_of(&theirs)));
	assert!(t.contains("Section: Skeleton"), "typst's own text carries the show rule's output: {}", t);
	assert_eq!(a, t, "pdftotext of austenite's PDF equals typst's");

	// K3
	let diffs = k3_differences(&res!(lines_of(&theirs)), &res!(lines_of(&ours)));
	assert!(diffs.is_empty(), "level 4b differences on austenite's own PDF: {:#?}", diffs);

	// K4
	assert!(report.diagnostics.is_empty(), "the skeleton raises a diagnostic: {:?}", report.diagnostics);
	assert!(report.strict_failure(&src).is_none(), "the strict compile refuses the skeleton");
	Ok(())
}

/// Compiles `source`, written to a file of its own, through the same pipeline.
fn compile_text(name: &str, source: &str) -> Outcome<(Vec<u8>, compile::Report, PathBuf)> {
	let dir = res!(work_dir()).join(name);
	res!(std::fs::create_dir_all(&dir));
	let src = dir.join("main.typ");
	res!(std::fs::write(&src, source));
	let mut sink = res!(PdfSink::new());
	let done = res!(compile::assemble_eval(&src, &dir, FontStore::default(), &mut sink));
	res!(done.laid.as_ref().map_err(|e| err!("{} did not lay out: {}", name, e.plain(); Test)));
	let report = done.report();
	let out = res!(sink.output().ok_or_else(|| err!("The fixpoint finished with no PDF."; Test)));
	Ok((out.to_vec(), report, src))
}

#[test]
fn a_warning_of_kind_unsupported_refuses_a_strict_compile_and_a_typst_warning_does_not() -> Outcome<()> {
	// A gradient is drawn in one flat colour: a construct set otherwise than Typst sets it.
	let (_, report, src) = res!(compile_text("strict-unsupported",
		"#rect(width: 40pt, height: 10pt, fill: gradient.linear(red, blue))\n"));
	assert!(report.diagnostics.iter().any(|d| d.kind == compile::DiagnosticKind::Unsupported),
		"the gradient raises an unsupported warning, found {:?}", report.diagnostics);
	let refusal = res!(report.strict_failure(&src).ok_or_else(|| err!("strict accepted an unsupported warning"; Test)));
	assert!(refusal.message.starts_with("strict:"), "the refusal is the strict one: {}", refusal.message);
	assert!(report.skipped.as_deref().map_or(false, |l| l.starts_with("skipped: ")), "the terse line names it: {:?}", report.skipped);

	// Typst's own warning stands beside the PDF: strict does not refuse it.
	let dir = res!(work_dir()).join("strict-typst-warning");
	res!(std::fs::create_dir_all(&dir));
	res!(std::fs::write(dir.join("t.typ"), "#let doc = 1\n"));
	let (_, report, src) = res!(compile_text("strict-typst-warning",
		"#import \"t.typ\": doc as doc\nBody text.\n"));
	assert!(report.diagnostics.iter().any(|d| d.message.contains("unnecessary import rename")),
		"Typst's own warning is reported, found {:?}", report.diagnostics);
	assert!(report.diagnostics.iter().filter(|d| d.message.contains("unnecessary import rename"))
		.all(|d| d.kind == compile::DiagnosticKind::Lint && d.kind.as_str() == "lint"), "Typst's own warning is a lint");
	assert!(report.diagnostics.iter().all(|d| !d.kind.refuses_strict()), "none of Typst's own warnings refuses strict");
	assert!(report.strict_failure(&src).is_none(), "a Typst warning stands beside the PDF");
	Ok(())
}
