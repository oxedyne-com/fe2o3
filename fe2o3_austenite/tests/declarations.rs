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
	compile_map(main, map)
}

/// As [`compile_of`], for a source map already built.
fn compile_map(main: &str, map: HashMap<PathBuf, Vec<u8>>) -> Outcome<(Rendered, Report)> {
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

/// A `#set document`'s site names the fields it left unapplied, read from the same arguments the Info
/// dictionary is read from, comments as trivia: a field commented out asks for nothing, a field whose value
/// the reader cannot evaluate is named rather than dropped, and a field named twice, which Typst refuses,
/// applies nothing and says so.
#[test]
fn a_set_documents_site_names_the_fields_it_left_unapplied() -> Outcome<()> {
	let _turn = turn();
	let cases = [
		("#set document(title: \"T\" /* c */, date: auto)\n", Some("T"),
			vec!["#set document left unapplied: date"]),
		("#set document(title: 1 + 1)\n", None, vec!["#set document left unapplied: title"]),
		("#set document(title: \"A\", title: \"B\")\n", None,
			vec!["#set document names title twice, so none of it is applied"]),
		("#set document(\n  // author: \"Old\", date: auto,\n  title: \"T\",\n)\n", Some("T"), vec![]),
	];
	for (set, title, want) in cases {
		let src = fmt!("{}= H\n\nBody.\n", set);
		let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", &src)]));
		let sites: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
		let want: Vec<(usize, &str)> = want.into_iter().map(|m| (1, m)).collect();
		assert_eq!(sites, want, "{:?}", set);
		assert_eq!(rendered.doc_info.title.as_deref(), title, "{:?}", set);
		assert_eq!(report.strict_failure(Path::new("/proj/main.typ")).is_some(), !want.is_empty(), "{:?}", set);
	}
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

/// A `[` in prose is text, as Typst reads one: in a paragraph, a heading or a chapter, it hides none of the
/// declarations, includes, guards or part pages after it, and nothing is refused for it.
#[test]
fn a_bracket_in_prose_hides_nothing_after_it() -> Outcome<()> {
	let _turn = turn();
	let root = "#let media = \"ebook\"\n\n= Intervals [0, 1)\n\nThe interval [0, 1) is BRACKETWORDS.\n\n\
		#set document(title: \"After Bracket\")\n\n#include \"ch1.typ\"\n\n#if media == \"ebook\" [\n  #include \"ebook.typ\"\n\
		] else [\n  #include \"print.typ\"\n]\n\n#part-page[Part Two]\n\n#include \"ch2.typ\"\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter One\n\nThe set [a, b) is open.\n\n#include \"sub.typ\"\n"),
		("/proj/sub.typ",	"SUBWORDS are set.\n"),
		("/proj/ebook.typ",	"EBOOKWORDS are set.\n"),
		("/proj/print.typ",	"PRINTWORDS are not.\n"),
		("/proj/ch2.typ",	"= Chapter Two\n\nCHAPTWOWORDS are set.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	for want in ["BRACKETWORDS", "SUBWORDS", "EBOOKWORDS", "Part Two", "CHAPTWOWORDS"] {
		assert!(text.contains(want), "{} is set: {}", want, text);
	}
	assert!(!text.contains("PRINTWORDS") && !text.contains("#include"), "{}", text);
	assert_eq!(rendered.doc_info.title.as_deref(), Some("After Bracket"));
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #let (fixed-point)"]);
	Ok(())
}

/// A comment is where Typst's lexer opens one: a link is one token, so the `/*` and `//` in it open none and
/// the declaration, the include and the prose after it are read; a quotation mark in prose is a character,
/// so a `/*` in a quoted phrase opens a comment that holds the declaration after it.
#[test]
fn a_comment_opens_where_typst_opens_one() -> Outcome<()> {
	let _turn = turn();
	let root = "See https://example.com/x/*y ZQX and https://a.io//b too.\n\n\
		#set document(title: \"After Url\")\n\n#include \"ch1.typ\"\n\nLASTWORDS set.\n";
	let files = [
		("/proj/root.typ",	root),
		("/proj/ch1.typ",	"= Chapter\n\nCHAPTERWORDS are set.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("ZQX") && text.contains("CHAPTERWORDS") && text.contains("LASTWORDS"), "{}", text);
	assert_eq!(rendered.doc_info.title.as_deref(), Some("After Url"));
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);

	let src = "He said \"use /* here\" today.\n\n#set document(title: \"After Quote\")\n\nHIDDENWORDS */ TAILWORDS.\n";
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("TAILWORDS") && !text.contains("HIDDENWORDS") && !text.contains("today"), "{}", text);
	assert_eq!(rendered.doc_info.title, None, "the commented title is not written");
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
	Ok(())
}

/// A conditional the reader meets sets its taken branch where it stands and nothing else of it -- no branch
/// it does not take, no `else`, no bracket and no site -- as Typst sets it: in a lone file mid-paragraph,
/// line-leading, mid-sentence on one line and in a callout, and in a callout of a book root.
#[test]
fn a_guard_the_reader_meets_sets_its_taken_branch_alone() -> Outcome<()> {
	let _turn = turn();
	let guard = "#if media == \"print\" [\nPRINTWORDS\n] else if media == \"ebook\" [\nEBOOKWORDS set.\n] else [\n\
		OTHERWORDS\n]";
	let lone = fmt!("#let media = \"ebook\"\n\nText before.\n{g}\n\n{g}\n\n\
		Words #if media == \"ebook\" [INLINEWORDS] else [PRINTWORDS] after.\n\n#styled-box[\nBoxed.\n{g}\n]\n\n\
		TAILWORDS.\n", g = guard);
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", &lone)]));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert_eq!(text.matches("EBOOKWORDS").count(), 3, "{}", text);
	for want in ["Text before.", "Words", "INLINEWORDS", "after.", "Boxed.", "TAILWORDS"] {
		assert!(text.contains(want), "{} is set: {}", want, text);
	}
	for leak in ["PRINTWORDS", "OTHERWORDS", "#if", "else", "[", "]"] {
		assert!(!text.contains(leak), "{} is not set: {}", leak, text);
	}
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #let (fixed-point)"]);

	let root = fmt!("#let media = \"ebook\"\n\n= Root\n\n#styled-box[\nBoxed.\n{g}\n]\n\n#include \"ch1.typ\"\n", g = guard);
	let files = [
		("/proj/root.typ",	root.as_str()),
		("/proj/ch1.typ",	"= Chapter One\n\nCHAPTERWORDS are set.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/root.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	assert!(text.contains("Boxed.") && text.contains("EBOOKWORDS") && text.contains("CHAPTERWORDS"), "{}", text);
	for leak in ["PRINTWORDS", "OTHERWORDS", "#if", "else", "]"] {
		assert!(!text.contains(leak), "{} is not set: {}", leak, text);
	}
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #let (fixed-point)"]);
	Ok(())
}

/// A loop, and a conditional the reader does not evaluate -- one on a name nothing binds, one taking a code
/// block, one whose taken branch holds an `#include` -- are refused whole, each at its own line: nothing of
/// them is set, neither branch, no `else` and no bracket, and the prose around them is.
#[test]
fn a_loop_or_an_unread_conditional_is_refused_whole_at_its_line() -> Outcome<()> {
	let _turn = turn();
	let src = "#let media = \"ebook\"\n\n= Top\n\n#for i in range(2) [\nFORWORDS\n]\n\n\
		Words #while false [WHILEWORDS] after.\n\n#if unbound == \"x\" [\nIFWORDS\n] else [\nELSEWORDS\n]\n\n\
		#if media == \"ebook\" {\n[CODEWORDS]\n}\n\n#styled-box[\nBoxed.\n#if media == \"ebook\" [\n\
		#include \"ch1.typ\"\n]\n]\n\nTAILWORDS.\n";
	let files = [
		("/proj/main.typ",	src),
		("/proj/ch1.typ",	"CHAPTERWORDS.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/main.typ", &files));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	for want in ["Top", "Words", "after.", "Boxed.", "TAILWORDS"] {
		assert!(text.contains(want), "{} is set: {}", want, text);
	}
	for leak in ["FORWORDS", "WHILEWORDS", "IFWORDS", "ELSEWORDS", "CODEWORDS", "CHAPTERWORDS", "#", "else", "]", "}"] {
		assert!(!text.contains(leak), "{} is not set: {}", leak, text);
	}
	let line_of = |needle: &str| src.lines().position(|l| l.contains(needle)).map_or(0, |i| i + 1);
	let sites: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
	let want: Vec<(usize, &str)> = vec![
		(1,							"skipped #let (fixed-point)"),
		(line_of("#for"),			"#for is a loop the reader does not run, so its body is not set"),
		(line_of("#while"),			"#while is a loop the reader does not run, so its body is not set"),
		(line_of("#if unbound"),	"#if has a condition the reader does not evaluate, so no branch of it is set"),
		(line_of("{"),				"#if takes a code block the reader does not run, so no branch of it is set"),
	];
	assert_eq!(&sites[..want.len()], &want[..], "{:?}", sites);
	// The callout's body is read apart, so its site's line is counted from the body (gate note, F1).
	assert_eq!(sites.len(), want.len() + 1, "{:?}", sites);
	assert_eq!(sites[want.len()].1,
		"#if takes a branch holding #include, which the reader does not read there, so no branch of it is set");
	Ok(())
}

/// A conditional in a chapter resolves in the one scope the include walk resolves its guards in: the book's
/// `config.typ`, then the chapter's own bindings. A guard on the config's `media` in a chapter's callout and
/// one mid-paragraph set their taken branch alone, as the chapter's include guard does, with no site; a name
/// only another chapter binds is not in scope, so the guard testing it is refused at its line.
#[test]
fn a_chapter_guard_resolves_in_the_book_config_as_the_include_walk_does() -> Outcome<()> {
	let _turn = turn();
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts");
	for f in ["LibertinusSerif-Regular.otf", "LibertinusSerif-Bold.otf", "LibertinusSerif-Italic.otf",
		"LibertinusSerif-BoldItalic.otf", "LibertinusMono-Regular.otf"]
	{
		map.insert(PathBuf::from(fmt!("/b/assets/fonts/libertinus/{}", f)), res!(std::fs::read(fonts.join(f))));
	}
	let root	= "#show: book.with(\n  title: [A Book],\n)\n\n#include \"ch1.typ\"\n#include \"ch2.typ\"\n";
	let config	= "#let format = \"a5\"\n#let media = \"ebook\"\n";
	let ch1		= "#import \"config.typ\": media\n#let side = \"a\"\n\n= One\n\n#styled-box[\nBoxed.\n\
		#if media == \"ebook\" [\nEBOOKWORDS set.\n] else [\nPRINTWORDS\n]\n]\n\n\
		Words #if media == \"print\" [PRINTWORDS] else [INLINEWORDS] after.\n\n\
		#if media == \"ebook\" [\n  #include \"sub.typ\"\n]\n\n#if side == \"a\" [SIDEWORDS]\n\n\
		#if media == \"print\" [\nPRINTWORDS\n] else if media == \"ebook\" [\nCHAINWORDS\n] else [\nOTHERWORDS\n]\n";
	let ch2		= "= Two\n\nTWOWORDS #if side == \"a\" [LEAKWORDS] here.\n";
	for (p, b) in [("/b/book/main.typ", root), ("/b/book/config.typ", config), ("/b/book/ch1.typ", ch1),
		("/b/book/ch2.typ", ch2), ("/b/book/sub.typ", "SUBWORDS set.\n")]
	{
		map.insert(PathBuf::from(p), b.as_bytes().to_vec());
	}
	let (rendered, report) = res!(compile_map("/b/book/main.typ", map));
	let text = runs(&rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ");
	for want in ["Boxed.", "EBOOKWORDS", "Words", "INLINEWORDS", "after.", "SUBWORDS", "SIDEWORDS", "CHAINWORDS", "TWOWORDS",
		"here."]
	{
		assert!(text.contains(want), "{} is set: {}", want, text);
	}
	for leak in ["PRINTWORDS", "OTHERWORDS", "LEAKWORDS", "#if", "else", "]"] {
		assert!(!text.contains(leak), "{} is not set: {}", leak, text);
	}
	let ifs: Vec<(&str, usize, &str)> = report.diagnostics.iter()
		.filter(|d| d.message.contains("#if"))
		.map(|d| (d.file.as_str(), d.line, d.message.as_str()))
		.collect();
	assert_eq!(ifs, [("/b/book/ch2.typ", 3, "#if has a condition the reader does not evaluate, so no branch of it is set")]);
	Ok(())
}

/// Every text run set, joined by spaces.
fn words(rendered: &Rendered) -> String {
	runs(rendered).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join(" ")
}

/// The size of the first text run holding `needle`.
fn size_of(rendered: &Rendered, needle: &str) -> Outcome<f32> {
	match runs(rendered).into_iter().find(|(t, _)| t.contains(needle)) {
		Some((_, size))	=> Ok(size),
		None			=> Err(err!("no run holds {:?}: {:?}", needle, runs(rendered); Test, Missing)),
	}
}

/// A `#!` line opening the file is a comment to its line's end, as Typst's lexer reads it: the `/*` in it
/// opens no comment, so the heading and the `#include` after it stand, and the shebang is not set as prose.
#[test]
fn a_shebang_line_is_trivia_and_the_include_after_it_is_followed() -> Outcome<()> {
	let _turn = turn();
	let root = "#!/usr/bin/env typst /*\n= Root\n\n#include \"ch1.typ\"\n";
	let files = [
		("/proj/main.typ",	root),
		("/proj/ch1.typ",	"= Chapter One\n\nINCLUDEDWORDS are set.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/main.typ", &files));
	let text = words(&rendered);
	assert!(text.contains("Root") && text.contains("INCLUDEDWORDS"), "the heading and the chapter are set: {}", text);
	assert!(!text.contains("#!") && !text.contains("usr/bin"), "the shebang is not prose: {}", text);
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
	Ok(())
}

/// A rule in a list item, strong or emphasis ends with it, as Typst's does: it is not lowered for the whole
/// document and not passed over, but refused where it stands, so the text after the item keeps its size and
/// the heading its own.
#[test]
fn a_rule_in_an_item_strong_or_emphasis_ends_with_it() -> Outcome<()> {
	let _turn = turn();
	let cases = [
		("item",	"= Root\n\n- item\n  #set text(size: 20pt)\n\n= Next\n\nTAILWORDS.\n",					4),
		("strong",	"= Root\n\n*bold\n#set text(size: 20pt)\nstill*\n\n= Next\n\nTAILWORDS.\n",				4),
		("emph",	"= Root\n\n_emph\n#set text(size: 20pt)\nstill_\n\n= Next\n\nTAILWORDS.\n",				4),
		("show",	"= Root\n\n- item\n  #show heading: set text(size: 40pt)\n\n= Next\n\nTAILWORDS.\n",	4),
	];
	for (name, src, line) in cases {
		let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
		assert!(res!(size_of(&rendered, "TAILWORDS")) < 15.0, "{}: the tail keeps its size: {:?}", name, runs(&rendered));
		assert!(res!(size_of(&rendered, "Next")) < 30.0, "{}: the heading keeps its size: {:?}", name, runs(&rendered));
		let sites: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
		assert_eq!(sites.len(), 1, "{}: one site: {:?}", name, sites);
		assert_eq!(sites[0].0, line, "{}: at the rule: {:?}", name, sites);
		assert!(sites[0].1.contains("inside a list item, strong or emphasis"), "{}: {:?}", name, sites);
	}
	Ok(())
}

/// An `#include` in a list item or in emphasis is that body's, not a chapter: gathered with it and refused
/// where it stands, so the file makes no book, sets no contents page and follows nothing.
#[test]
fn an_include_in_an_item_or_emphasis_is_not_a_chapter() -> Outcome<()> {
	let _turn = turn();
	let cases = [
		("item",	"= Root\n\n- item\n  #include \"ch1.typ\"\n\nBody.\n",		4),
		("emph",	"= Root\n\n_emph\n#include \"ch1.typ\"\nstill_\n\nTail.\n",	4),
	];
	for (name, src, line) in cases {
		let files = [
			("/proj/main.typ",	src),
			("/proj/ch1.typ",	"= Chapter One\n\nCHAPTERWORDS are set.\n"),
		];
		let (rendered, report) = res!(compile_of("/proj/main.typ", &files));
		let text = words(&rendered);
		assert!(!text.contains("CHAPTERWORDS") && !text.contains("Chapter One"), "{}: not followed: {}", name, text);
		assert_eq!(rendered.out.pages.len(), 1, "{}: no contents page, no new page: {}", name, text);
		let sites: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
		assert_eq!(sites, [(line, "skipped #include (inside a body, where it is not followed) (unsupported)")], "{}", name);
	}
	Ok(())
}

/// A line in a guard's branch is read as the file's own only with no list item, heading, strong or emphasis
/// open around it there: an `#include` indented in an item in the branch is the item's, refused where it
/// stands, while the file's own include still makes the book.
#[test]
fn an_include_in_an_item_in_a_guard_branch_is_not_a_chapter() -> Outcome<()> {
	let _turn = turn();
	let root = "#let media = \"ebook\"\n\n= Root\n\n#include \"ch0.typ\"\n\n#if media == \"ebook\" [\n- item\n  \
		#include \"ch1.typ\"\n]\n\nTail.\n";
	let files = [
		("/proj/main.typ",	root),
		("/proj/ch0.typ",	"= Chapter Zero\n\nZEROWORDS are set.\n"),
		("/proj/ch1.typ",	"= Chapter One\n\nONEWORDS are not.\n"),
	];
	let (rendered, report) = res!(compile_of("/proj/main.typ", &files));
	let text = words(&rendered);
	assert!(text.contains("ZEROWORDS") && text.contains("Tail."), "the file's own include is followed: {}", text);
	assert!(!text.contains("ONEWORDS"), "the item's is not: {}", text);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert!(names.contains(&"skipped #include (inside a body, where it is not followed) (unsupported)"), "{:?}", names);
	Ok(())
}

/// A bare content block `#[ ... ]` is set where it stands, joined into the markup around it: its brackets are
/// not prose, and a `#set` in it governs the block alone, as Typst's does.
#[test]
fn a_bare_content_block_is_set_where_it_stands_and_scopes_its_rule() -> Outcome<()> {
	let _turn = turn();
	let src = "= Root\n\n#[\n#set text(size: 20pt)\nBIGWORDS in block.\n]\n\nThen tail words.\n";
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
	let text = words(&rendered);
	assert!(!text.contains("#[") && !text.contains(']'), "the block's brackets are not prose: {}", text);
	assert!(res!(size_of(&rendered, "BIGWORDS")) > 18.0, "the rule governs the block: {:?}", runs(&rendered));
	assert!(res!(size_of(&rendered, "tail")) < 15.0, "and ends with it: {:?}", runs(&rendered));
	assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
	Ok(())
}

/// A bare content block that opens inside a paragraph joins it in Typst; this reader sets it as the text it is
/// written as, brackets and all, and reports it at its line rather than leaving the brackets unexplained.
#[test]
fn a_bare_content_block_inside_a_paragraph_is_reported() -> Outcome<()> {
	let _turn = turn();
	let src = "= Root\n\nPara words\n#[\nin para\n]\nmore words\n";
	let (rendered, report) = res!(compile_of("/proj/main.typ", &[("/proj/main.typ", src)]));
	assert!(words(&rendered).contains("in para"), "{}", words(&rendered));
	let sites: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
	assert_eq!(sites, [(4, "#[ stands inside a paragraph, so it is set as text, brackets and all")]);
	Ok(())
}
