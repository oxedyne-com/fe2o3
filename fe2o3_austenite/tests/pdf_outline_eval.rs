//! The bookmarks of a PDF written through the evaluator are Typst's: a numbered heading is titled with the
//! number it shows, a space and its body, as Typst titles it, and an unnumbered one with its body alone. The
//! expectation is Typst's own PDF of the same source, read back with the same reader.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::oracle::Oracle;
use harness::pdf::{
	austenite_pdf,
	typst_pdf,
	work_dir,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;
use std::process::Command;

// Every number pattern and function a heading can show: a pattern whose last piece repeats, a numbering of
// no text, an empty one, one that is only a space, a heading left out of the outline, one left out of the
// bookmarks only, and a counter that is moved.
const SOURCE: &str = r#"#set page(width: 200pt, height: 80pt, margin: 10pt)
#set heading(numbering: "1.1")
= One
== Under *bold* one
#heading(numbering: none)[Plain]
#heading(level: 2, numbering: "A)")[Lettered]
#set heading(numbering: "I.")
= Roman
== Sub
#set heading(numbering: (..n) => [Part #n.pos().map(str).join("-")])
= Func
== Func sub
#set heading(numbering: "1.")
#counter(heading).update(10)
= Ten
#heading(outlined: false)[Hidden]
#heading(bookmarked: false)[NoMark]
#heading(numbering: " 1 ")[Spaced]
#heading(numbering: "(1)", supplement: [Sec])[Paren]
#heading(numbering: (..n) => none)[NoneFn]
#heading(numbering: (..n) => [])[EmptyFn]
#heading(numbering: (..n) => [#h(1em)x])[SpaceFn]
"#;

// The outline as a JSON list of [level, title], read by pikepdf; None where the tool is not here.
fn titles(pdf: &Path) -> Option<String> {
	let script = "import json, sys, pikepdf\n\
		pdf = pikepdf.open(sys.argv[1])\n\
		out = []\n\
		def walk(items, level):\n\
		\tfor i in items:\n\
		\t\tout.append([level, str(i.title)])\n\
		\t\twalk(i.children, level + 1)\n\
		walk(pdf.open_outline().root, 1)\n\
		print(json.dumps(out))\n";
	match Command::new("python3").arg("-c").arg(script).arg(pdf).output() {
		Ok(o) if o.status.success()	=> Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
		_							=> None,
	}
}

#[test]
fn bookmarks_are_titled_with_the_number_a_heading_shows_as_typst_titles_them() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let dir = res!(work_dir("pdf-outline-eval"));
	let src = dir.join("main.typ");
	res!(std::fs::write(&src, SOURCE));
	let typst_out = dir.join("typst.pdf");
	res!(typst_pdf(&src, &typst_out));
	let made = res!(austenite_pdf(&src));
	let ours = dir.join("austenite.pdf");
	res!(std::fs::write(&ours, &made.bytes));
	let (want, got) = match (titles(&typst_out), titles(&ours)) {
		(Some(w), Some(g))	=> (w, g),
		_					=> {
			println!("[pdf-outline] SKIPPED: python3 with pikepdf is not here");
			return Ok(());
		}
	};
	// Fixed by construction, so a comparison of two empty outlines cannot pass.
	assert!(want.starts_with(r#"[[1, "1 One"], [2, "1.1 Under bold one"], [1, "Plain"]"#), "Typst's outline: {}", want);
	assert!(want.contains(r#"[1, "11. Ten"]"#) && want.contains(r#"[1, " NoneFn"]"#), "Typst's outline: {}", want);
	assert!(!want.contains("Hidden") && !want.contains("NoMark"), "Typst's outline: {}", want);
	assert_eq!(got, want, "the bookmarks differ from Typst's");
	Ok(())
}
