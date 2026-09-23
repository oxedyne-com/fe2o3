//! Declarations, includes and fields are read where the reader meets them: a `#show` rule, an `#include` or
//! a front-matter field that a comment holds or a raw block shows is text and does nothing, and a `#show`
//! rule inside a body the engine does not scope it to is refused there rather than applied to the whole
//! document. Driven through the `compile` functions the wasm `DaimondTypst` calls, with the source map
//! installed as it installs one.

use oxedyne_fe2o3_austenite::compile::{
	self,
	DiagnosticKind,
	Rendered,
	Report,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::{
	Arc,
	Mutex,
	MutexGuard,
};

// The source map is a process global, so the tests that install one take turns.
static VFS_TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
	VFS_TURN.lock().unwrap_or_else(|p| p.into_inner())
}

/// Installs `files`, compiles `main` as the wasm surface does, and returns the render and its report,
/// clearing the map either way.
fn compile_of(main: &str, files: &[(&str, &str)]) -> Outcome<(Rendered, Report)> {
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (p, b) in files {
		map.insert(PathBuf::from(p), b.as_bytes().to_vec());
	}
	res!(vfs::install(map));
	let main	= PathBuf::from(main);
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<(Rendered, Report)> {
		let assembled	= res!(compile::assemble(&main, || Ok(fonts.clone())));
		let empty		= assembled.blocks.is_empty();
		let rendered	= res!(compile::author_and_run(assembled));
		let report		= Report::new(rendered.out.pages.len(), &rendered.refusals, empty);
		Ok((rendered, report))
	};
	let out = run();
	res!(vfs::clear());
	out
}

/// Every text run set on every page, with the size it is set at.
fn runs(rendered: &Rendered) -> Vec<(String, f32)> {
	let mut out = Vec::new();
	for page in &rendered.out.pages {
		for placed in &page.frame.placed {
			if let PlacedKind::Text(t) = &placed.kind {
				out.push((t.source().to_string(), t.size()));
			}
		}
	}
	out
}

/// A `#show heading` rule shown in a raw block, written in a comment or placed in a `#styled-box` body sizes
/// no heading of the document; the boxed one is refused where it stands, and nothing else is reported.
#[test]
fn a_shown_commented_or_boxed_show_rule_sizes_no_heading() -> Outcome<()> {
	let _turn = turn();
	let src = "= Visible Heading\n\n```typst\n#show heading: set text(size: 30pt)\n```\n\n\
		/*\n#show heading: set text(size: 31pt)\n*/\n\n#styled-box[\n#show heading: set text(size: 32pt)\n\
		Boxed words.\n]\n\nBody words.\n";
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
	let head = runs(&rendered).into_iter().find(|(t, _)| t.contains("Visible"));
	let size = match head {
		Some((_, size))	=> size,
		None			=> return Err(err!("the heading is set: {:?}", runs(&rendered); Test, Missing)),
	};
	assert!(size < 29.0, "no rule resized the heading: {}", size);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #show heading (inside a body, where it is not applied) (fixed-point)"]);
	assert_eq!(report.diagnostics[0].kind, DiagnosticKind::Unsupported);
	Ok(())
}

/// An `#include` a block comment holds is not followed: the draft chapter sets none of its words, the
/// comment's closer is not set as prose, and the included chapter beside it still is.
#[test]
fn a_commented_include_is_not_followed() -> Outcome<()> {
	let _turn = turn();
	let root = "#include \"ch1.typ\"\n/*\n#include \"draft.typ\"\n*/\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter\n\nCHAPTERWORDS are set.\n"),
		("/proj/draft.typ",	"= Draft\n\nDRAFTWORDS never shown.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text: String = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("CHAPTERWORDS"), "the chapter is set: {}", text);
	assert!(!text.contains("DRAFTWORDS") && !text.contains("Draft"), "the draft is not: {}", text);
	assert!(!text.contains("*/"), "the comment's closer is not prose: {}", text);
	assert!(report.strict_failure(Path::new("/proj/root.typ")).is_none(), "{:?}", report.diagnostics);
	Ok(())
}

/// A block comment nests, as Typst nests one: a `#set document`, a `#show` rule and an `#include` in a comment
/// that holds another are text, the outer comment's closer is not set as prose, and nothing is reported --
/// where the first `*/` once closed the comment, and the title, the rule and the draft all took effect.
#[test]
fn a_nested_comment_holds_its_declarations_and_includes() -> Outcome<()> {
	let _turn = turn();
	let root = "/* outer /* inner */\n#set document(title: \"Commented Out\")\n\
		#show heading: set text(size: 30pt)\n#include \"draft.typ\"\n*/\n#include \"ch1.typ\"\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter\n\nCHAPTERWORDS are set.\n"),
		("/proj/draft.typ",	"= Draft\n\nDRAFTWORDS never shown.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let set		= runs(&rendered);
	let text	= set.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" ");
	assert!(text.contains("CHAPTERWORDS"), "the chapter is set: {}", text);
	assert!(!text.contains("DRAFTWORDS") && !text.contains("Draft"), "the draft is not: {}", text);
	assert!(!text.contains("*/"), "the outer closer is not prose: {}", text);
	assert_eq!(rendered.doc_info.title, None, "the commented title is not written");
	let size = match set.iter().find(|(t, _)| t.contains("Chapter")) {
		Some((_, size))	=> *size,
		None			=> return Err(err!("the heading is set: {}", text; Test, Missing)),
	};
	assert!(size < 29.0, "the commented rule sizes no heading: {}", size);
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
	Ok(())
}

/// An `#include` in a callout's body is the body's: the file is the lone file it is, with no contents page,
/// the callout is read whole and set, its include refused where it stands, and nothing after it is refused
/// as never closing -- where the include was once followed as the file's own, the callout cut at it, and its
/// `]` set as prose. A binding the unfollowed file defines is not in scope.
#[test]
fn an_include_in_a_body_is_refused_where_it_stands() -> Outcome<()> {
	let _turn = turn();
	let src = "= Lone\n\n#styled-box[\nBoxed words.\n#include \"ch1.typ\"\n]\n\nAfter words.\n\n#chapword\n";
	let files = [
		("/proj/main.typ",	src),
		("/proj/ch1.typ",	"#let chapword = [CHAPTERWORDS bound.]\n= Chapter\n\nCHAPTERWORDS are set.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/main.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert_eq!(rendered.out.pages.len(), 1, "one page, no contents: {}", text);
	assert!(text.contains("Boxed") && text.contains("After"), "the callout and what follows are set: {}", text);
	assert!(!text.contains("CHAPTERWORDS") && !text.contains(']'), "{}", text);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #include (inside a body, where it is not followed) (unsupported)", "skipped #chapword (unsupported)"]);
	Ok(())
}

/// An `#include` in a binding's body is the binding's: a binding never used sets nothing from the file, as
/// Typst sets nothing, its `]` is not prose, and the binding is not refused as never closing.
#[test]
fn an_include_in_an_unused_binding_is_not_followed() -> Outcome<()> {
	let _turn = turn();
	let root = "#let later = [\n#include \"draft.typ\"\n]\n#include \"ch1.typ\"\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter\n\nCHAPTERWORDS are set.\n"),
		("/proj/draft.typ",	"= Draft\n\nDRAFTWORDS never shown.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("CHAPTERWORDS"), "the chapter is set: {}", text);
	assert!(!text.contains("DRAFTWORDS") && !text.contains("Draft") && !text.contains(']'), "{}", text);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #let (fixed-point)"]);
	Ok(())
}

/// An `#include` directly in an `#if` include guard's branch is the file's own: a root whose only includes
/// stand in a guard is a book, and the taken branch's chapter is set while the other is not.
#[test]
fn an_include_in_a_guard_branch_is_followed() -> Outcome<()> {
	let _turn = turn();
	let root = "#let media = \"ebook\"\n\n= Sources\n\n#if media == \"ebook\" [\n  #include \"ebook.typ\"\n] else [\n\
		  #include \"print.typ\"\n]\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ebook.typ",	"EBOOKWORDS are set.\n"),
		("/proj/print.typ",	"PRINTWORDS are not.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("EBOOKWORDS") && !text.contains("PRINTWORDS"), "{}", text);
	assert!(report.diagnostics.iter().all(|d| !d.message.contains("#include") && !d.message.contains("#if")),
		"{:?}", report.diagnostics);
	Ok(())
}

/// A lone file that shows an `#include` in a raw block is compiled as the lone file it is: no chapter is
/// looked for, so the absent one is no error, and no contents page is set.
#[test]
fn a_shown_include_makes_no_book() -> Outcome<()> {
	let _turn = turn();
	let src = "= Lone\n\nHow to include a chapter:\n\n```typst\n#include \"chapter.typ\"\n```\n";
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
	assert_eq!(rendered.out.pages.len(), 1, "one page, no contents");
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
	Ok(())
}

/// A title-page logo field a comment holds is not read: no logo is looked for, so none is missing.
#[test]
fn a_commented_logo_field_asks_for_no_logo() -> Outcome<()> {
	let _turn = turn();
	let root = "#show: doc.with(\n  title: [Doc],\n  // title-top-logo-path: \"assets/old-top.png\",\n)\n\
		#include \"ch1.typ\"\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter\n\nWords.\n"),
	];
	let (_, report) = res!(compile_of("/proj/root.typ", &files));
	assert!(report.diagnostics.iter().all(|d| d.kind != DiagnosticKind::MissingFile),
		"no logo is missing: {:?}", report.diagnostics);
	Ok(())
}
