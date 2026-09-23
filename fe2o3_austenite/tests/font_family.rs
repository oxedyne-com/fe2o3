//! Do the families a document names reach the page, and does a family nobody supplied stop the compile?
//!
//! Every case drives the real [`compile::assemble`]/[`compile::author_and_run`] path with the document's
//! fonts injected into the source map, as the wasm surface injects a project's `fonts`. The fixture family
//! is Noto Sans, borrowed from `fe2o3_font`'s embedded set, and it is injected under file names that say
//! nothing about it (`/assets/fonts/pack/f1.ttf` ...), so a pass proves the family was matched by the name
//! its own name table declares -- Typst's rule -- and not by a filename convention.
//!
//! Five assertions map to one fix each, and reverting that fix reds it (checked 2026-09-23):
//!
//!   * [`a_body_family_sets_the_body_bold_and_italic`]: the root reading-set swap in `compile::assemble`.
//!   * [`a_missing_family_is_a_hard_error`]: the precheck in `FaceResolver::require`.
//!   * [`a_heading_rule_sets_the_heading_face`]: the heading `set text(font:)` redirect in `rules.rs`.
//!   * [`a_scoped_family_sets_only_its_scope`]: the scope swap in `doc::Authoring::walk`.
//!   * [`smallcaps_asks_the_font_for_small_capitals`]: the `smcp` feature threaded through line breaking.
//!
//! The sixth, [`a_document_naming_no_family_is_byte_unchanged`], is the guard on all of them: fonts supplied
//! but never named change not one byte of the page.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::svg;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
	Arc,
	Mutex,
};

// The source map is a process-wide global, so the cases take turns.
static VFS: Mutex<()> = Mutex::new(());

/// One placed text run: its source string, the family/weight/slant of the face that set it, and its glyphs.
#[derive(Debug)]
struct Run {
	src:	String,
	family:	String,
	weight:	u16,
	italic:	bool,
	glyphs:	Vec<u32>,
}

/// The Noto Sans fixture faces, under deliberately uninformative file names.
fn noto() -> Vec<(String, Vec<u8>)> {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts");
	let mut out = Vec::new();
	for (i, f) in ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf", "NotoSans-Italic.ttf", "NotoSans-BoldItalic.ttf"]
		.iter().enumerate()
	{
		let bytes = std::fs::read(dir.join(f)).expect("the fe2o3_font Noto Sans fixture must be present");
		out.push((fmt!("/assets/fonts/pack/f{}.ttf", i + 1), bytes));
	}
	out
}

/// Compiles `src` as `/doc/main.typ` with `fonts` installed, returning every placed text run and the SVG of
/// each page, or the assembly/compile error.
fn compile(src: &str, fonts: &[(String, Vec<u8>)]) -> Outcome<(Vec<Run>, Vec<String>)> {
	let _turn = match VFS.lock() {
		Ok(g)	=> g,
		Err(p)	=> p.into_inner(),	// a failed case poisons the lock; the map is reinstalled regardless
	};
	let main = PathBuf::from("/doc/main.typ");
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	files.insert(main.clone(), src.as_bytes().to_vec());
	for (path, bytes) in fonts {
		files.insert(PathBuf::from(path), bytes.clone());
	}
	res!(vfs::install(files));
	let set		= Arc::new(res!(fonts::libertinus()));
	let result	= compile::assemble(&main, || Ok(set.clone()))
		.and_then(|(a, _, _)| compile::author_and_run(a));
	let _ = vfs::clear();
	let rendered = res!(result);

	let mut runs = Vec::new();
	let mut svgs = Vec::new();
	for page in &rendered.out.pages {
		svgs.push(res!(svg::render_page(page)));
		for placed in &page.frame.placed {
			if let PlacedKind::Text(shaped) = &placed.kind {
				let info = res!(shaped.face_info());
				runs.push(Run {
					src:	shaped.source().to_string(),
					family:	info.family,
					weight:	info.weight,
					italic:	info.italic,
					glyphs:	shaped.run().glyphs.iter().map(|g| g.id).collect(),
				});
			}
		}
	}
	Ok((runs, svgs))
}

/// The first run whose source contains `word`, failing the test when the word never reached the page.
fn run_of<'a>(runs: &'a [Run], word: &str) -> Outcome<&'a Run> {
	match runs.iter().find(|r| r.src.contains(word)) {
		Some(r)	=> Ok(r),
		None	=> Err(err!("No placed run carries {:?}; runs were {:?}.", word, runs; Test, Missing)),
	}
}

/// A root `#set text(font: "Noto Sans")` sets the body in Noto Sans, and its strong and emphasised runs in
/// the family's own bold and italic, each matched by the weight and slant the file declares.
#[test]
fn a_body_family_sets_the_body_bold_and_italic() -> Outcome<()> {
	let src = "#set text(font: \"Noto Sans\")\n\nPlain words, *heavy* words and _leaning_ words.\n";
	let (runs, _) = res!(compile(src, &noto()));
	let plain = res!(run_of(&runs, "Plain"));
	assert_eq!((plain.family.as_str(), plain.weight, plain.italic), ("Noto Sans", 400, false), "{:?}", plain);
	let heavy = res!(run_of(&runs, "heavy"));
	assert_eq!((heavy.family.as_str(), heavy.weight, heavy.italic), ("Noto Sans", 700, false), "{:?}", heavy);
	let lean = res!(run_of(&runs, "leaning"));
	assert_eq!((lean.family.as_str(), lean.italic), ("Noto Sans", true), "{:?}", lean);

	// A fall-back list led by the supplied family sets in it too.
	let listed = "#set text(font: (\"Noto Sans\", \"Libertinus Serif\"))\n\nListed words.\n";
	let (runs, _) = res!(compile(listed, &noto()));
	assert_eq!(res!(run_of(&runs, "Listed")).family, "Noto Sans");
	Ok(())
}

/// A family no supplied font declares stops the compile with an error naming it and the families that
/// were on offer -- never a silent fall-back to Libertinus. A fall-back list is checked whole.
#[test]
fn a_missing_family_is_a_hard_error() -> Outcome<()> {
	for src in [
		"#set text(font: \"Nonesuch Serif\")\n\nWords.\n",
		"#set text(font: (\"Nonesuch Serif\", \"Noto Sans\"))\n\nWords.\n",
		"#show heading: set text(font: \"Nonesuch Serif\")\n\n= A heading\n\nWords.\n",
	] {
		match compile(src, &noto()) {
			Ok((runs, _))	=> return Err(err!(
				"{:?} named a family no font supplies, yet compiled; runs were {:?}.", src, runs; Test, Mismatch)),
			Err(e)			=> {
				let msg = fmt!("{}", e);
				assert!(msg.contains("Nonesuch Serif"), "the error must name the family: {}", msg);
				assert!(msg.contains("Noto Sans"), "the error must name the families on offer: {}", msg);
			},
		}
	}
	Ok(())
}

/// A document that names no family renders byte-identically whether or not fonts were supplied: an unnamed
/// font is never read, let alone used.
#[test]
fn a_document_naming_no_family_is_byte_unchanged() -> Outcome<()> {
	let src = "= A heading\n\nPlain words, *heavy* words and _leaning_ words.\n";
	let (runs_with, svg_with)	= res!(compile(src, &noto()));
	let (_, svg_without)		= res!(compile(src, &[]));
	assert!(!svg_with.is_empty());
	assert_eq!(svg_with, svg_without, "supplying an unnamed family must not change one byte");
	assert!(runs_with.iter().all(|r| r.family == "Libertinus Serif"), "{:?}", runs_with);
	Ok(())
}

/// `#show heading: set text(font: ...)` sets every heading in that family while the body keeps its own.
#[test]
fn a_heading_rule_sets_the_heading_face() -> Outcome<()> {
	let src = "#show heading: set text(font: \"Noto Sans\")\n\n= Display\n\nBody words.\n\n== Second\n\nMore words.\n";
	let (runs, _) = res!(compile(src, &noto()));
	assert_eq!(res!(run_of(&runs, "Display")).family, "Noto Sans");
	assert_eq!(res!(run_of(&runs, "Second")).family, "Noto Sans");
	assert_eq!(res!(run_of(&runs, "Body")).family, "Libertinus Serif");
	Ok(())
}

/// A family set by a scoped rule (`#show par: set text(font: ...)`) sets the paragraphs it scopes and
/// nothing else: the heading outside the scope stays in the document's own family.
#[test]
fn a_scoped_family_sets_only_its_scope() -> Outcome<()> {
	let src = "#show par: set text(font: \"Noto Sans\")\n\n= Outside\n\nDog run.\n";
	let (runs, _) = res!(compile(src, &noto()));
	assert_eq!(res!(run_of(&runs, "Dog")).family, "Noto Sans");
	assert_eq!(res!(run_of(&runs, "Outside")).family, "Libertinus Serif");
	Ok(())
}

/// `#smallcaps[...]` shapes with the font's `smcp` feature: the same word draws different glyphs from its
/// plain setting, in the same face.
#[test]
fn smallcaps_asks_the_font_for_small_capitals() -> Outcome<()> {
	let src = "A #smallcaps[capped] treaty.\n\nA plain capped treaty.\n";
	let (runs, _) = res!(compile(src, &[]));
	let capped: Vec<&Run> = runs.iter().filter(|r| r.src == "capped").collect();
	assert_eq!(capped.len(), 2, "one small-capped and one plain setting of the word: {:?}", capped);
	assert_eq!(capped[0].family, capped[1].family, "both in the body face");
	assert_ne!(capped[0].glyphs, capped[1].glyphs,
		"smcp must swap the lower-case letters for small capitals: {:?}", capped);
	Ok(())
}

/// An equation sets in New Computer Modern Math -- Typst's own default maths face, embedded -- when the
/// document names no maths font, while the prose around it keeps the body family.
#[test]
fn maths_sets_in_new_computer_modern_math_by_default() -> Outcome<()> {
	let (runs, _) = res!(compile("Let $x + y$ be given.\n", &[]));
	let maths: Vec<&Run> = runs.iter().filter(|r| r.family.to_lowercase().contains("math")).collect();
	assert!(!maths.is_empty(), "the equation must reach the page: {:?}", runs);
	for m in &maths {
		assert!(fonts::same_family(&m.family, "New Computer Modern Math"), "{:?}", m);
	}
	assert_eq!(res!(run_of(&runs, "given")).family, "Libertinus Serif");
	Ok(())
}

/// An embedded family needs no file: a heading rule naming Libertinus Mono resolves with nothing supplied,
/// and the family accessors agree with what the files declare.
#[test]
fn embedded_families_need_no_file() -> Outcome<()> {
	let (runs, _) = res!(compile("#show heading: set text(font: \"Libertinus Mono\")\n\n= Typed\n\nBody.\n", &[]));
	assert_eq!(res!(run_of(&runs, "Typed")).family, "Libertinus Mono");
	let listed = fonts::embedded_families();
	assert!(listed.iter().any(|f| f == "New Computer Modern Math"), "{:?}", listed);
	let noto = noto();
	assert_eq!(res!(fonts::declared_family(&noto[0].1)), "Noto Sans");
	assert!(fonts::same_family("NewComputerModern Math", "new computer modern math"));
	assert!(!fonts::same_family("Noto Sans", "Noto Serif"));
	Ok(())
}
