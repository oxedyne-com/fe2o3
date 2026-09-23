//! Every warning a compile of the oracle corpus raises reaches Daimond as plain words: no terminal colour
//! code and no source location of the engine's own, in a diagnostic's message, in the skip line or in a
//! strict refusal's hint. The corpus is compiled through the same `compile` functions the wasm
//! `DaimondTypst` calls, one root after another in this binary's one test, since the image base and the term
//! dictionary those functions install are process globals.

#[allow(dead_code)]
#[path = "oracle/mod.rs"]
mod driver;

use oxedyne_fe2o3_austenite::compile::{
	self,
	Report,
};
use oxedyne_fe2o3_austenite::fonts;

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;
use std::sync::Arc;

/// Why `text` is not plain, or `None` when it is.
fn not_plain(text: &str) -> Option<&'static str> {
	if text.contains('\u{1b}') {
		return Some("a terminal escape");
	}
	if text.contains(".rs:") {
		return Some("an engine source location");
	}
	None
}

#[test]
fn corpus_warnings_are_plain_text() -> Outcome<()> {
	let fonts		= Arc::new(res!(fonts::libertinus()));
	let mut checked	= 0usize;
	let mut sites	= 0usize;
	let mut faults: Vec<String> = Vec::new();
	for root in driver::corpus() {
		let main = Path::new(root.path);
		if !main.is_file() {
			println!("[corpus-warnings] {}: not on this machine, not checked", root.name);
			continue;
		}
		let assembled	= res!(compile::assemble(main, || Ok(fonts.clone())));
		let empty		= assembled.blocks.is_empty();
		let rendered	= res!(compile::author_and_run(assembled));
		let report		= Report::new(rendered.out.pages.len(), &rendered.refusals, empty);
		let mut texts: Vec<(String, String)> = Vec::new();
		for d in &report.diagnostics {
			texts.push(("message".to_string(), d.message.clone()));
		}
		if let Some(s) = &report.skipped {
			texts.push(("skipped".to_string(), s.clone()));
		}
		if let Some(head) = report.strict_failure(main) {
			texts.push(("strict message".to_string(), head.message.clone()));
			if let Some(h) = &head.hint {
				texts.push(("hint".to_string(), h.clone()));
			}
		}
		for (field, text) in &texts {
			if let Some(why) = not_plain(text) {
				faults.push(fmt!("{}: a {} carries {}: {:?}", root.name, field, why, text));
			}
		}
		println!("[corpus-warnings] {}: {} warning(s), plain", root.name, report.diagnostics.len());
		sites	+= report.diagnostics.len();
		checked	+= 1;
	}
	assert!(checked > 0, "no corpus root was on this machine to check");
	if !faults.is_empty() {
		return Err(err!("{} warning text(s) are not plain:\n{}", faults.len(), faults.join("\n"); Test, Mismatch));
	}
	println!("[corpus-warnings] {} root(s), {} warning(s), all plain", checked, sites);
	Ok(())
}
