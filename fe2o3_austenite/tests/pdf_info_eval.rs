//! The Info dictionary of a PDF written through the evaluator carries the document's metadata, as Typst
//! applies it: from `set document` at the top level and from a template the document is shown through
//! (`show: doc.with(..)`), the engine named as creator and producer.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

fn pdf_of(name: &str, files: &[(&str, &str)]) -> Outcome<String> {
	let home	= res!(std::env::var("HOME"));
	let dir		= Path::new(&home).join(".cache").join("austenite-qc").join("pdf-info-eval").join(name);
	res!(std::fs::create_dir_all(&dir));
	for (f, text) in files {
		res!(std::fs::write(dir.join(f), text));
	}
	let main: PathBuf = dir.join("main.typ");
	let mut sink = res!(PdfSink::new());
	let done = res!(compile::assemble_eval(&main, &dir, FontStore::default(), &mut sink));
	res!(done.laid.as_ref().map_err(|e| err!("{} did not lay out: {}", name, e.plain(); Test)));
	let out = res!(sink.output().ok_or_else(|| err!("no PDF"; Test)));
	Ok(String::from_utf8_lossy(&out.to_vec()).to_string())
}

#[test]
fn a_top_level_set_document_reaches_the_info_dictionary() -> Outcome<()> {
	let pdf = res!(pdf_of("top", &[("main.typ",
		"#set document(title: [A Title], author: (\"Ann\", \"Bob\"), description: [About], keywords: (\"k1\", \"k2\"))\nBody.\n")]));
	assert!(pdf.contains("/Title (A Title)"), "the title: {}", pdf);
	assert!(pdf.contains("/Author (Ann, Bob)"), "the authors, joined");
	assert!(pdf.contains("/Subject (About)"), "the description is the subject");
	assert!(pdf.contains("/Keywords (k1, k2)"), "the keywords, joined");
	assert!(pdf.contains("/Creator (Austenite)") && pdf.contains("/Producer (Austenite)"), "the engine named, never a version");
	Ok(())
}

#[test]
fn a_set_document_inside_a_template_function_reaches_the_info_dictionary() -> Outcome<()> {
	let pdf = res!(pdf_of("template", &[
		("t.typ", "#let doc(title: none, body) = {\n  set document(title: title, author: \"Templated\")\n  body\n}\n"),
		("main.typ", "#import \"t.typ\": doc\n#show: doc.with(title: [From Template])\nBody.\n"),
	]));
	assert!(pdf.contains("/Title (From Template)"), "the title set through doc.with: {}", pdf);
	assert!(pdf.contains("/Author (Templated)"), "the author the template sets");
	Ok(())
}

#[test]
fn a_document_with_no_metadata_names_only_the_engine() -> Outcome<()> {
	let pdf = res!(pdf_of("none", &[("main.typ", "Body.\n")]));
	assert!(!pdf.contains("/Title") && !pdf.contains("/Author"), "no field is invented");
	assert!(pdf.contains("/Creator (Austenite)"), "the creator is the engine");
	Ok(())
}
