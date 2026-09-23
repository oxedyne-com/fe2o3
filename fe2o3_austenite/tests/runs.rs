//! A caption, a footnote and a heading set a reference, a citation, a footnote and an index marker as a
//! paragraph sets them, since every place a run is set answers for what the run asks for: nothing is
//! dropped, and a citation is never set as its raw key when the bibliography holds it. Driven through the
//! `compile` functions the wasm `DaimondTypst` calls, with the source map installed as it installs one.

use oxedyne_fe2o3_austenite::compile::{
	self,
	DiagnosticKind,
	Rendered,
	Report,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::ir::DrawOp;
use oxedyne_fe2o3_austenite::ledger::AnchorKind;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;

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

const MAIN: &str = "/proj/main.typ";
const BIB: &str = "@article{smith2020, author = {Smith, John}, title = {A Study}, journal = {J}, year = {2020}}\n";

/// Installs `files`, compiles `MAIN` as the wasm surface does, and returns the render and its report,
/// clearing the map either way.
fn compile_of(files: &[(&str, &str)]) -> Outcome<(Rendered, Report)> {
	let bytes: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_bytes())).collect();
	compile_bytes(&bytes)
}

/// As [`compile_of`], for files given as bytes.
fn compile_bytes(files: &[(&str, &[u8])]) -> Outcome<(Rendered, Report)> {
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (p, b) in files {
		map.insert(PathBuf::from(p), b.to_vec());
	}
	res!(vfs::install(map));
	let main	= PathBuf::from(MAIN);
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

/// Every word set on every page, in placing order, one space between words.
fn words(rendered: &Rendered) -> String {
	let mut out = Vec::new();
	for page in &rendered.out.pages {
		for placed in &page.frame.placed {
			if let PlacedKind::Text(t) = &placed.kind {
				out.push(t.source().to_string());
			}
		}
	}
	out.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The body, a caption, a footnote and a heading each set a reference as the words its label resolves to
/// and a citation as its author-year text; a footnote in a caption and one in a footnote are set at the
/// page foot, numbered in order; an index marker in a caption is recorded; and nothing is reported.
#[test]
fn captions_footnotes_and_headings_set_what_a_paragraph_sets() -> Outcome<()> {
	let _turn = turn();
	let src = "= Intro <intro>\n\nBody sees @intro and #cite(<smith2020>).\n\n\
		== Sub about @intro #cite(<smith2020>)#footnote[Headnote words.]\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Caption sees @intro and #cite(<smith2020>)#footnote[Capnote words.]#index[Captioned].])\n\n\
		Para.#footnote[Footnote sees @intro and #cite(<smith2020>)#footnote[Nested words.].]\n";
	let (rendered, report) = res!(compile_of(&[(MAIN, src), ("/proj/refs.bib", BIB)]));
	let text = words(&rendered);
	assert!(report.diagnostics.is_empty(), "nothing is reported: {:?}", report.diagnostics);
	assert!(!text.contains("smith2020"), "no citation is set as its key: {}", text);
	for want in ["Caption sees Chapter 1 and (Smith 2020)", "Footnote sees Chapter 1 and (Smith 2020)",
		"about Chapter 1 (Smith 2020)", "Headnote words.", "Capnote words.", "Nested words."]
	{
		assert!(text.contains(want), "{:?} is set: {}", want, text);
	}
	let indexed = rendered.out.ledger.anchors()
		.any(|a| a.id.kind == AnchorKind::IndexEntry && a.id.key.contains("Captioned"));
	assert!(indexed, "the caption's index marker is recorded");
	Ok(())
}

/// A citation in a caption or a heading whose key the bibliography lacks is set as its keys and reported
/// where it is written, as one in a paragraph is; a reference in a footnote to a label nothing carries is
/// reported too. Before, both were set as raw keys or dropped with nothing said.
#[test]
fn what_a_caption_footnote_or_heading_cannot_resolve_is_reported() -> Outcome<()> {
	let _turn = turn();
	let src = "= Intro\n\n== Sub #cite(<nobody>)\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Cites #cite(<nokey>).])\n\n\
		Para.#footnote[See @nolabel.]\n";
	let (_, report) = res!(compile_of(&[(MAIN, src), ("/proj/refs.bib", BIB)]));
	let lines: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
	assert_eq!(lines.len(), 3, "{:?}", lines);
	assert!(lines[0].0 == 3 && lines[0].1.starts_with("#cite(<nobody>) names a key the bibliography does not hold"), "{:?}", lines);
	assert!(lines[1].0 == 5 && lines[1].1.starts_with("#cite(<nokey>) names a key"), "{:?}", lines);
	assert!(lines[2].0 == 7 && lines[2].1.starts_with("@nolabel names no label"), "{:?}", lines);
	Ok(())
}

/// A callout's body is set through the whole block walk: a heading, a figure with its image, a display
/// equation, a plain image and its prose are all set, and an image the project lacks is reported where it
/// stands, so strict refuses the document -- where the callout once set its prose alone and dropped the rest
/// with nothing said.
#[test]
fn a_callout_body_sets_every_block_it_holds() -> Outcome<()> {
	let _turn = turn();
	let png = res!(res!(Pixmap::filled(8, 8, Rgba::opaque(200, 0, 0))).to_png());
	let src = "= Top\n\n#styled-box[\n= Boxed Heading\n\n#figure(image(\"pic.png\", width: 2cm), caption: [Boxed figure.])\n\n\
		$ x^2 + y^2 $\n\n#image(\"gone-in-box.png\")\n\nBoxed words.\n]\n\nAfter.\n";
	let (rendered, report) = res!(compile_bytes(&[(MAIN, src.as_bytes()), ("/proj/pic.png", &png)]));
	let text = words(&rendered);
	for want in ["Boxed Heading", "Figure 1: Boxed figure.", "Boxed words.", "After."] {
		assert!(text.contains(want), "{:?} is set: {}", want, text);
	}
	assert!(text.contains('\u{1d465}') || text.contains(" x "), "the equation is set: {}", text);
	let drawn = rendered.out.pages.iter().flat_map(|p| p.frame.placed.iter()).any(|pl| match &pl.kind {
		PlacedKind::Graphic(g)	=> g.ops.iter().any(|op| matches!(op, DrawOp::Image { .. })),
		_						=> false,
	});
	assert!(drawn, "the boxed figure's image is drawn");
	let missing: Vec<_> = report.diagnostics.iter().filter(|d| d.message.contains("gone-in-box.png")).collect();
	assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
	assert_eq!(missing[0].kind, DiagnosticKind::MissingFile, "{}", missing[0]);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses the missing image");
	Ok(())
}

/// A floating `#place` inside a callout, which a callout laid out whole has no band to float, is refused
/// where the reader builds the callout, and the rest of the body is set.
#[test]
fn a_float_inside_a_callout_is_refused() -> Outcome<()> {
	let _turn = turn();
	let src = "= Top\n\n#styled-box[\nBefore.\n\n#place(top, float: true)[Floated words.]\n\nAfter.\n]\n";
	let (rendered, report) = res!(compile_of(&[(MAIN, src)]));
	let text = words(&rendered);
	assert!(text.contains("Before.") && text.contains("After."), "{}", text);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["skipped #place (inside a container, which cannot float one) (unsupported)"]);
	Ok(())
}
