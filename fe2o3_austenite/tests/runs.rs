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

/// A floating `#place` inside a callout, which a callout laid out whole has no band to float, has its float
/// refused where the reader builds the callout, and its body is set where it stands with the rest.
#[test]
fn a_float_inside_a_callout_is_refused() -> Outcome<()> {
	let _turn = turn();
	let src = "= Top\n\n#styled-box[\nBefore.\n\n#place(top, float: true)[Floated words.]\n\nAfter.\n]\n";
	let (rendered, report) = res!(compile_of(&[(MAIN, src)]));
	let text = words(&rendered);
	assert!(text.contains("Before. Floated words. After."), "{}", text);
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, ["#place float inside a container is refused, so its body is set where it stands"]);
	Ok(())
}

const IN_PLACE: &str = "#figure placement inside a container is refused, so the figure is set where it stands";

/// The position of each of `words` in `text`, in order, failing unless each follows the one before.
fn in_order(text: &str, words: &[&str]) -> Outcome<()> {
	let mut from = 0usize;
	for w in words {
		match text[from..].find(w) {
			Some(at)	=> from += at + w.len(),
			None		=> return Err(err!("{:?} does not follow {:?} in: {}", w, &text[..from], text; Test, Missing)),
		}
	}
	Ok(())
}

/// A floating figure or table in a callout, and a floating figure in a floating `#place`, are set where they
/// stand, each placement refused at its figure, where the compile once ended with no PDF. Strict refuses them.
#[test]
fn a_floating_figure_inside_a_container_is_set_where_it_stands() -> Outcome<()> {
	let _turn = turn();
	let png = res!(res!(Pixmap::filled(8, 8, Rgba::opaque(200, 0, 0))).to_png());
	let src = "= Top\n\n#styled-box[\nBefore.\n\n\
		#figure(image(\"pic.png\", width: 2cm), caption: [Boxed float.], placement: top)\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Boxed table.], placement: auto)\n\nAfter.\n]\n\n\
		#place(top, float: true)[\nPlaced.\n\n#figure(table(columns: 1, [Inner]), caption: [Placed table.], placement: bottom)\n]\n\n\
		Body.\n";
	let (rendered, report) = res!(compile_bytes(&[(MAIN, src.as_bytes()), ("/proj/pic.png", &png)]));
	let text = words(&rendered);
	res!(in_order(&text, &["Before.", "Boxed float.", "Boxed table.", "After."]));
	res!(in_order(&text, &["Placed.", "Inner", "Placed table."]));
	let drawn = rendered.out.pages.iter().flat_map(|p| p.frame.placed.iter()).any(|pl| match &pl.kind {
		PlacedKind::Graphic(g)	=> g.ops.iter().any(|op| matches!(op, DrawOp::Image { .. })),
		_						=> false,
	});
	assert!(drawn, "the boxed figure's image is drawn");
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, [IN_PLACE, IN_PLACE, IN_PLACE]);
	assert!(report.diagnostics.iter().all(|d| d.kind == DiagnosticKind::Unsupported), "{:?}", report.diagnostics);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses a placement not honoured");
	Ok(())
}

/// A floating callout in a callout is set where it stands, its float refused, and a column change in a
/// `#columns` body inside a callout is refused, its body set in the callout's one column: neither ends the
/// compile.
#[test]
fn a_floating_callout_or_a_column_change_inside_a_callout_is_set_in_place() -> Outcome<()> {
	let _turn = turn();
	let src = "#let aside-box(title: none, float: true, body) = {\n\
		\tlet inner = box(width: 100%, inset: 1em, fill: luma(240), [#text(size: 0.85em)[#body]])\n\
		\tif float { figure(placement: auto, inner) } else { inner }\n}\n\n\
		= Top\n\n#styled-box[\nBefore.\n\n#aside-box[Aside words.]\n\n\
		#columns(2)[\n#set columns(gutter: 8pt)\nColumned words.\n]\n\nAfter.\n]\n";
	let (rendered, report) = res!(compile_of(&[(MAIN, src)]));
	let text = words(&rendered);
	res!(in_order(&text, &["Before.", "Aside words.", "Columned words.", "After."]));
	let names: Vec<&str> = report.diagnostics.iter().map(|d| d.message.as_str()).collect();
	assert_eq!(names, [
		"skipped #let (fixed-point)",
		"skipped #columns (unsupported)",
		"callout float inside a container is refused, so the callout is set where it stands",
		"column change inside a container is refused, so its body is set in the container's one column",
	]);
	Ok(())
}

/// The first 1-based page whose runs, joined, hold `word`, or `None` when no page does. A word may be set
/// as several runs, parted at a kerning pair or a ligature, so the runs are joined with nothing between.
fn page_of(rendered: &Rendered, word: &str) -> Option<usize> {
	rendered.out.pages.iter().position(|page| {
		let joined: String = page.frame.placed.iter().filter_map(|placed| match &placed.kind {
			PlacedKind::Text(t)	=> Some(t.source()),
			_					=> None,
		}).collect();
		joined.contains(word)
	}).map(|i| i + 1)
}

const ASIDE: &str = "#let aside-box(title: none, float: true, body) = {\n\
	\tlet inner = box(width: 100%, inset: 1em, fill: luma(240), [#text(size: 0.85em)[#body]])\n\
	\tif float { figure(placement: auto, inner) } else { inner }\n}\n\n";

/// A footnote in a float -- a floating figure's caption, a floating `#place`'s body, a floating callout's
/// body -- is set at the foot of the page the float lands on, where it was answered "set" and laid nowhere;
/// a float deferred to a later page takes its note there, in one column or two.
#[test]
fn a_float_sets_its_footnotes_on_the_page_it_lands_on() -> Outcome<()> {
	let _turn = turn();
	let filler: String = (1..=40).map(|n| fmt!("Filler paragraph {} of the page{}.\n\n", n,
		if n == 40 { " Fortieth" } else { "" })).collect();
	// A float too tall for any page that already carries a few paragraphs, so it waits for the next.
	let rows: String = (1..=34).map(|n| fmt!("[Row {}], ", n)).collect();
	let one = fmt!("{}= Top\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Alphafloat.#footnote[Alphanote.]], placement: top)\n\n\
		#place(bottom, float: true)[Betaplace.#footnote[Betanote.]]\n\n\
		#aside-box[Gammaaside.#footnote[Gammanote.]]\n\n\
		{}#figure(table(columns: 1, {}), caption: [Deltadeferred.#footnote[Deltanote.]], placement: top)\n\n\
		{}", ASIDE, filler, rows, filler);
	let two = fmt!("#set page(columns: 2)\n\n= Top\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Epsilonparent.#footnote[Epsilonnote.]], placement: top, scope: \"parent\")\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Thetaparent.#footnote[Thetanote.]], placement: bottom, scope: \"parent\")\n\n\
		#figure(table(columns: 1, [Cell]), caption: [Zetacolumn.#footnote[Zetanote.]], placement: bottom)\n\n\
		{}#figure(table(columns: 1, {}), caption: [Etadeferred.#footnote[Etanote.]], placement: top)\n\n\
		{}{}", filler, rows, filler, filler);
	for (src, pairs, deferred) in [
		(one.as_str(), vec![("Alphafloat", "Alphanote"), ("Betaplace", "Betanote"), ("Gammaaside", "Gammanote"),
			("Deltadeferred", "Deltanote")], "Deltadeferred"),
		(two.as_str(), vec![("Epsilonparent", "Epsilonnote"), ("Thetaparent", "Thetanote"), ("Zetacolumn", "Zetanote"),
			("Etadeferred", "Etanote")],
			"Etadeferred"),
	] {
		let (rendered, report) = res!(compile_of(&[(MAIN, src)]));
		for (float, note) in pairs {
			let at = page_of(&rendered, float);
			assert!(at.is_some(), "{:?} is set", float);
			assert_eq!(page_of(&rendered, note), at, "{:?} is set on the page {:?} lands on", note, float);
		}
		assert!(page_of(&rendered, deferred) > page_of(&rendered, "Fortieth"), "{:?} is deferred past the filler: {:?} {:?}",
			deferred, page_of(&rendered, deferred), page_of(&rendered, "Fortieth"));
		assert!(report.diagnostics.iter().all(|d| d.message.starts_with("skipped #let")), "{:?}", report.diagnostics);
	}
	Ok(())
}

/// What a list item asks for is answered at that item's own line, and a nested item's at its own, as the
/// reader places an unknown call in the same item -- where every item's once stood at the list's first line.
#[test]
fn a_list_item_answers_at_its_own_line() -> Outcome<()> {
	let _turn = turn();
	let src = "= Top\n\n- first item\n- second item\n- third #foo(1) item and @gone\n  - nested @alsogone\n";
	let (_, report) = res!(compile_of(&[(MAIN, src)]));
	let got: Vec<(usize, &str)> = report.diagnostics.iter().map(|d| (d.line, d.message.as_str())).collect();
	assert_eq!(got.len(), 3, "{:?}", got);
	assert!(got.iter().any(|(l, m)| *l == 5 && m.contains("#foo")), "{:?}", got);
	assert!(got.iter().any(|(l, m)| *l == 5 && m.starts_with("@gone")), "{:?}", got);
	assert!(got.iter().any(|(l, m)| *l == 6 && m.starts_with("@alsogone")), "{:?}", got);
	Ok(())
}
