//! Maths set in a face the host supplies. An equation's face is found through the compilation's font store, the
//! way the text around it is, so a family given by `--font-path` or `add_bytes` is found by
//! `show math.equation: set text(font: ..)`, and the embedded New Computer Modern Math is the fallback. Each
//! synthetic document is compiled through the evaluator to its own PDF and by Typst 0.15.1 with the same
//! `--font-path`; the lines of the two files (text without its spaces, box bottom, first word) and the fonts
//! they embed are compared. The faces are two the font crate carries for its own tests: DejaVu Sans, which has
//! a MATH table, and Noto Sans, which has none.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::oracle::Oracle;
use harness::pdf::{
	pdf_compare,
	work_dir,
};

use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::fonts::FaceVariant;
use oxedyne_fe2o3_austenite::math::font;
use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

const DOCUMENTS: [(&str, &str); 5] = [
	// The equation's own show-set names the family.
	("show_set", r#"
#show math.equation: set text(font: "DejaVu Sans")
Euler: $e^(i pi) + 1 = 0$, and a block:
$ sum_(k=1)^n k = (n (n + 1)) / 2 $
$ sqrt(x^2 + y^2) quad lr(( a / b )) quad lim_(x -> 0) $
$ integral_0^1 f(x) dif x = lim_(n -> oo) sum_(i=1)^n f(i / n) 1 / n $
"#),
	// A list whose first family is nowhere: the second is found.
	("family_list", r#"
#show math.equation: set text(font: ("No Such Math", "DejaVu Sans"))
$ x^2 + y^2 = z^2 $
$ mat(1, 2; 3, 4) vec(a, b) $
"#),
	// Only block equations take the host face; an inline one stays in the embedded face.
	("block_only", r#"
#show math.equation.where(block: true): set text(font: "DejaVu Sans")
An inline $a^2 + b^2$ in the default face.
$ a^2 + b^2 = c^2 $
"#),
	// A face that is nowhere: the embedded maths face, the head of Typst's maths fallback list.
	("absent_family", r#"
#show math.equation: set text(font: "No Such Math")
$ a^2 + sqrt(b) $
$ c^2 + d / e $
"#),
	// A face switched inside one equation.
	("inner_text", r#"
$ a + text(font: "DejaVu Sans", b^2) + c $
"#),
];

// The font crate's own faces: DejaVu Sans carries a (small) MATH table, Noto Sans none.
fn crate_font(name: &str) -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts").join(name)
}

// Does the face name the host maths face, DejaVu Sans, not the mono sibling the crate embeds?
fn is_host(name: &str) -> bool {
	name.starts_with("DejaVuSans") && !name.contains("Mono")
}

// A directory holding just `file`, so `--font-path` offers Typst exactly the face the store is given.
fn only(dir: &Path, name: &str, file: &Path) -> Outcome<PathBuf> {
	let d = dir.join(name);
	res!(std::fs::create_dir_all(&d));
	let target = d.join(file.file_name().unwrap_or_default());
	res!(std::fs::copy(file, &target));
	Ok(d)
}

fn written(dir: &Path, name: &str, src: &str) -> Outcome<PathBuf> {
	let p = dir.join(fmt!("{}.typ", name));
	res!(std::fs::write(&p, src));
	Ok(p)
}

#[test]
fn a_family_the_host_supplies_sets_the_equation() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let root = res!(work_dir("eval-math-fonts"));
	let fonts = res!(only(&root, "host_math", &crate_font("DejaVuSans.ttf")));
	let mut failures = Vec::new();
	for (name, src) in DOCUMENTS {
		let typ = res!(written(&root, name, src));
		let found = res!(pdf_compare(&typ, &root.join(name), Some(&fonts), true));
		let both = |names: &[String]| names.iter().any(|n| is_host(n));
		// Only the documents that name the host face set it; the others fall back to the embedded one.
		let host = !matches!(name, "absent_family");
		if !found.differences.is_empty() {
			failures.push(fmt!("{}: {}", name, found.differences.join("; ")));
		}
		if host && !(both(&found.theirs) && both(&found.ours)) {
			failures.push(fmt!("{}: the host face is not in both files: typst {:?}, austenite {:?}", name, found.theirs, found.ours));
		}
		if !host && both(&found.ours) {
			failures.push(fmt!("{}: austenite embeds the host face for a document that does not name it: {:?}", name, found.ours));
		}
		println!("{:14} typst {:?} / austenite {:?}", name, found.theirs, found.ours);
	}
	for f in &failures {
		println!("FAIL    {}", f);
	}
	assert!(failures.is_empty(), "{} document(s) differ from typst", failures.len());
	Ok(())
}

#[test]
fn a_text_face_with_no_maths_table_sets_the_equation_with_a_warning() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let root = res!(work_dir("eval-math-fonts"));
	let fonts = res!(only(&root, "host_text", &crate_font("NotoSans-Regular.ttf")));
	let typ = res!(written(&root, "no_math_table", r#"
#show math.equation: set text(font: "Noto Sans")
$ x^2 + 1 / (y + 1) = sqrt(z) $
"#));
	let found = res!(pdf_compare(&typ, &root.join("no_math_table"), Some(&fonts), true));
	println!("typst {:?} / austenite {:?}", found.theirs, found.ours);
	assert!(found.differences.is_empty(), "differs from typst: {}", found.differences.join("; "));
	let warned = found.report.diagnostics.iter().any(|d| d.message.contains("not designed for math")
		&& d.kind == oxedyne_fe2o3_austenite::diag::DiagnosticKind::Lint);
	assert!(warned, "no `current font is not designed for math` lint: {:?}", found.report.diagnostics);
	Ok(())
}

#[test]
fn the_store_supplies_the_maths_face_and_the_embedded_one_is_the_fallback() -> Outcome<()> {
	let file = crate_font("DejaVuSans.ttf");
	let bytes = res!(std::fs::read(&file));
	let family = res!(oxedyne_fe2o3_austenite::fonts::declared_family(&bytes));
	let key = family.to_lowercase();
	let want = FaceVariant::default();
	let names = vec![key.clone()];

	// Without the file the store holds no such family, so the embedded face answers.
	let mut bare = FontStore::default();
	let book = res!(bare.book());
	let got = res!(font::resolve(&book, &names, true, want));
	assert_eq!(got.family, "New Computer Modern Math", "the embedded face is the fallback");
	assert!(res!(font::select(&book, &key, want)).is_none(), "{} is not in a bare store", family);

	// With the file added as bytes, the family is found, and its own MATH table drives the layout.
	let mut host = FontStore::default();
	host.add_bytes(bytes);
	let book = res!(host.book());
	let got = res!(font::resolve(&book, &names, true, want));
	assert_eq!(got.family, family, "the host's family is found through the store");
	assert!(got.has_math, "{} carries a MATH table", family);
	assert_eq!(res!(font::chain(&book, &names, true, want)).len() > 1, true, "the fallback faces follow the host's");
	let again = res!(font::select(&book, &key, want));
	assert!(again.map(|a| std::sync::Arc::ptr_eq(&a, &got)).unwrap_or(false), "a face is parsed once and shared");

	// A family the store lacks falls through the list to the one it has.
	let list = vec!["no such math".to_string(), key];
	assert_eq!(res!(font::resolve(&book, &list, true, want)).family, family);
	Ok(())
}
