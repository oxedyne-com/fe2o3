//! Every place page building sets a stand-in for what a construct asked for -- an image that will not load,
//! a logo, a cover, a reference to a label nothing carries, a citation nothing resolves, a bibliography or a
//! terms file that is not there -- is reported as a warning at the site that asked, so a strict compile
//! refuses it. Driven through the same `compile` functions the wasm `DaimondTypst` calls, with the source
//! map installed as it installs one.

use oxedyne_fe2o3_austenite::compile::{
	self,
	Diagnostic,
	DiagnosticKind,
	Report,
	Severity,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::memo::Memo;
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

const MAIN: &str = "/proj/main.typ";

/// Installs `files`, compiles `main` as the wasm surface does -- through the memo when one is given -- and
/// returns the report, clearing the map either way.
fn report_of(main: &str, files: &[(&str, &[u8])], memo: Option<&mut Memo>) -> Outcome<Report> {
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (p, b) in files {
		map.insert(PathBuf::from(p), b.to_vec());
	}
	res!(vfs::install(map));
	let main	= PathBuf::from(main);
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<Report> {
		let assembled	= res!(compile::assemble(&main, || Ok(fonts.clone())));
		let empty		= assembled.blocks.is_empty();
		let rendered	= res!(compile::author_and_run_memo(assembled, memo));
		Ok(Report::new(rendered.out.pages.len(), &rendered.refusals, empty))
	};
	let out = run();
	res!(vfs::clear());
	out
}

/// The warnings whose message holds `needle`.
fn sites<'a>(report: &'a Report, needle: &str) -> Vec<&'a Diagnostic> {
	report.diagnostics.iter().filter(|d| d.message.contains(needle)).collect()
}

/// The one warning whose message holds `needle`.
fn site<'a>(report: &'a Report, needle: &str) -> Outcome<&'a Diagnostic> {
	let found = sites(report, needle);
	match found.as_slice() {
		[d]	=> Ok(d),
		_	=> Err(err!("Expected one site naming {:?}, found {:?}.", needle, report.diagnostics; Test, Mismatch)),
	}
}

/// A file with a PNG's signature and nothing a decoder can read after it.
const BAD_PNG: &[u8] = b"\x89PNG\r\n\x1a\nthis is no image data";

/// A figure image, a plain `#image` and a section banner's logo that are there but will not load are each
/// set with a stand-in and reported at their own line as `unsupported`, not `missing_file`, so a strict
/// compile refuses the document; a GIF and a WebP, types this build does not read, and an SVG that is not
/// well-formed are reported the same.
#[test]
fn an_image_that_will_not_load_is_reported_where_it_is_named() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\n#figure(image(\"bad.png\"), caption: [Corrupt.])\n\n#image(\"anim.gif\")\n\n\
		#section-banner(\"broken.png\")\n\n#image(\"pic.webp\")\n\n#image(\"bad.svg\")\n\nBody.\n";
	let files: [(&str, &[u8]); 6] = [
		(MAIN, &src[..]),
		("/proj/bad.png", BAD_PNG),
		("/proj/anim.gif", b"GIF89a\x01\x00\x01\x00\x00\x00\x00;"),
		("/proj/broken.png", BAD_PNG),
		("/proj/pic.webp", b"RIFF\x0c\x00\x00\x00WEBPVP8 "),
		("/proj/bad.svg", b"<svg width=\"10\""),
	];
	let report = res!(report_of(MAIN, &files, None));
	for (needle, line, stand_in) in [
		("\"bad.png\"", 3, "a placeholder is set"),
		("\"anim.gif\"", 5, "a placeholder is set"),
		("\"broken.png\"", 7, "the banner is drawn without it"),
		("\"pic.webp\"", 9, "a placeholder is set"),
		("\"bad.svg\"", 11, "a placeholder is set"),
	] {
		let d = res!(site(&report, needle));
		assert_eq!((d.severity, d.kind, d.file.as_str(), d.line), (Severity::Warning, DiagnosticKind::Unsupported, MAIN, line), "{}", d);
		assert!(d.message.contains("will not load") && d.message.contains(stand_in), "{}", d);
	}
	assert_eq!(report.skipped, None, "a stand-in is not a construct skipped: {:?}", report.skipped);
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse an image that will not load"; Test)),
	};
	assert_eq!((head.kind, head.line), (DiagnosticKind::Unsupported, 3), "{}", head);
	assert!(head.hint.as_deref().map_or(false, |h| h.starts_with("substituted: ")), "{:?}", head.hint);
	Ok(())
}

/// A block the authoring memo serves from cache reports the stand-in it set when it was authored, so the
/// delta path's second compile is no quieter than its first.
#[test]
fn a_memo_hit_reports_its_stand_in_again() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\n#figure(image(\"bad.png\"), caption: [Corrupt.])\n\nBody.\n";
	let files: [(&str, &[u8]); 2] = [(MAIN, &src[..]), ("/proj/bad.png", BAD_PNG)];
	let mut memo = Memo::new();
	let first	= res!(report_of(MAIN, &files, Some(&mut memo)));
	memo.sweep();
	let second	= res!(report_of(MAIN, &files, Some(&mut memo)));
	assert_eq!(sites(&first, "\"bad.png\"").len(), 1, "{:?}", first.diagnostics);
	assert_eq!(sites(&second, "\"bad.png\"").len(), 1, "the hit reports it again: {:?}", second.diagnostics);
	Ok(())
}

/// A documentation tree's title-page and footer logos that are not in the project are set without, and
/// each is reported as `missing_file` at the field of the template call that names it.
#[test]
fn a_documents_logos_are_reported_at_the_fields_that_name_them() -> Outcome<()> {
	let _turn = turn();
	let root = b"#show: doc.with(\n  title: [Logos],\n  title-top-logo-path: \"/assets/top.svg\",\n  \
		title-bottom-logo-path: \"/assets/bottom.svg\",\n  footer-left-logo-path: \"/assets/foot.svg\",\n)\n\n\
		= First\n\nBody.\n\n#include \"a.typ\"\n";
	let files: [(&str, &[u8]); 2] = [(MAIN, &root[..]), ("/proj/a.typ", b"== A\n\nText.\n")];
	let report = res!(report_of(MAIN, &files, None));
	for (needle, line, stand_in) in [
		("\"/assets/top.svg\"", 3, "the title page is set without it"),
		("\"/assets/bottom.svg\"", 4, "the title page is set without it"),
		("\"/assets/foot.svg\"", 5, "the footers are set without it"),
	] {
		let d = res!(site(&report, needle));
		assert_eq!((d.severity, d.kind, d.file.as_str(), d.line), (Severity::Warning, DiagnosticKind::MissingFile, MAIN, line), "{}", d);
		assert!(d.message.contains("is not in the project") && d.message.contains(stand_in), "{}", d);
	}
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses a missing logo");
	Ok(())
}

/// The five Libertinus faces a book's own font tree ships, read from the crate's copies.
fn libertinus_tree(dir: &str) -> Outcome<Vec<(String, Vec<u8>)>> {
	let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts");
	let mut out = Vec::new();
	for f in ["LibertinusSerif-Regular.otf", "LibertinusSerif-Bold.otf", "LibertinusSerif-Italic.otf",
		"LibertinusSerif-BoldItalic.otf", "LibertinusMono-Regular.otf"]
	{
		out.push((fmt!("{}/{}", dir, f), res!(std::fs::read(fonts.join(f)))));
	}
	Ok(out)
}

/// A book's cover, named in its config, is reported at the config's line when it is not there; its title
/// logo, there but not a raster this path reads, at the root's field; and a bibliography the root names but
/// the project lacks at its field, with every citation then reported where it is written.
#[test]
fn a_books_cover_logo_and_bibliography_are_reported_at_their_fields() -> Outcome<()> {
	let _turn = turn();
	let main	= "/b/book/main.typ";
	let root	= b"#show: book.with(\n  title: [A Book],\n  title-logo-path: \"/assets/logo.png\",\n  \
		meta-data: (\n    bibliography: \"/missing.bib\",\n  ),\n)\n\n#include \"ch1.typ\"\n";
	let config	= b"#let format = \"a5\"\n#let mode = \"dev\"\n#let cover-image-path = if format == \"a5\" { \"/assets/cover.png\" }\n";
	let chapter	= b"= One\n\nAs argued #cite(<smith>).\n";
	let tree	= res!(libertinus_tree("/b/assets/fonts/libertinus"));
	let mut files: Vec<(&str, &[u8])> = vec![
		(main, &root[..]),
		("/b/book/config.typ", &config[..]),
		("/b/book/ch1.typ", &chapter[..]),
		("/b/assets/logo.png", BAD_PNG),
	];
	for (p, b) in &tree {
		files.push((p.as_str(), b.as_slice()));
	}
	let report = res!(report_of(main, &files, None));

	let d = res!(site(&report, "\"/assets/cover.png\""));
	assert_eq!((d.kind, d.file.as_str(), d.line), (DiagnosticKind::MissingFile, "/b/book/config.typ", 3), "{}", d);
	assert!(d.message.contains("no cover page is set"), "{}", d);
	let d = res!(site(&report, "\"/assets/logo.png\""));
	assert_eq!((d.kind, d.file.as_str(), d.line), (DiagnosticKind::Unsupported, main, 3), "{}", d);
	assert!(d.message.contains("the title page is set without it"), "{}", d);
	let d = res!(site(&report, "\"/missing.bib\""));
	assert_eq!((d.kind, d.file.as_str(), d.line), (DiagnosticKind::MissingFile, main, 5), "{}", d);
	assert!(d.message.contains("no reference list is set"), "{}", d);
	let d = res!(site(&report, "#cite(<smith>)"));
	assert_eq!((d.kind, d.file.as_str(), d.line), (DiagnosticKind::Unsupported, "/b/book/ch1.typ", 3), "{}", d);
	assert!(d.message.contains("no bibliography"), "{}", d);
	Ok(())
}

/// A reference to a label nothing in the document carries, and a citation with no bibliography to resolve
/// against, are each set with a stand-in and reported at their paragraph; a reference to a label that is
/// there is reported nowhere.
#[test]
fn an_unplaced_label_and_an_unresolved_citation_are_reported() -> Outcome<()> {
	let _turn = turn();
	let src = b"= Heading <sec>\n\nSee @sec and @nolabel.\n\nAs argued #cite(<nobib>).\n";
	let report = res!(report_of(MAIN, &[(MAIN, &src[..])], None));
	let d = res!(site(&report, "@nolabel"));
	assert_eq!((d.severity, d.kind, d.line), (Severity::Warning, DiagnosticKind::Unsupported, 3), "{}", d);
	assert!(d.message.contains("names no label"), "{}", d);
	assert!(sites(&report, "@sec").is_empty(), "a placed label is not reported: {:?}", report.diagnostics);
	let d = res!(site(&report, "#cite(<nobib>)"));
	assert_eq!((d.kind, d.line), (DiagnosticKind::Unsupported, 5), "{}", d);
	assert!(d.message.contains("no bibliography"), "{}", d);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses both");
	Ok(())
}

/// With a bibliography found beside the file, a citation of a key it holds resolves and is reported
/// nowhere, and one of a key it lacks is reported at its paragraph.
#[test]
fn a_citation_key_the_bibliography_lacks_is_reported() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\nKnown #cite(<smith2020>).\n\nUnknown #cite(<jones1999>).\n";
	let bib = b"@article{smith2020, author = {Smith, John}, title = {A Title}, year = {2020}, journal = {J}}\n";
	let report = res!(report_of(MAIN, &[(MAIN, &src[..]), ("/proj/refs.bib", &bib[..])], None));
	assert!(sites(&report, "smith2020").is_empty(), "a held key is not reported: {:?}", report.diagnostics);
	let d = res!(site(&report, "#cite(<jones1999>)"));
	assert_eq!((d.kind, d.line), (DiagnosticKind::Unsupported, 5), "{}", d);
	assert!(d.message.contains("does not hold"), "{}", d);
	Ok(())
}

/// A show rule whose transform sets more than the element -- here its body and a page number, where
/// Austenite would set the heading unchanged -- is refused where it is written, not passed through.
#[test]
fn a_show_rule_this_reader_does_not_run_is_refused() -> Outcome<()> {
	let _turn = turn();
	let src = b"#show heading: it => [#it.body #here().page()]\n\n= Heading\n\n== Sub\n\nBody.\n";
	let report = res!(report_of(MAIN, &[(MAIN, &src[..])], None));
	let d = res!(site(&report, "#show heading"));
	assert_eq!((d.severity, d.kind, d.file.as_str(), d.line), (Severity::Warning, DiagnosticKind::Unsupported, MAIN, 1), "{}", d);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses it");
	Ok(())
}

/// A `terms.typ` that is there but is not UTF-8 text is reported against the file, as `encoding`, where it
/// once became an empty dictionary without a word.
#[test]
fn a_terms_file_that_will_not_read_is_reported() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\nThe #t(\"org\") view.\n";
	let terms: &[u8] = b"#let term-dict = (\"org\": \"Oxedyne \xff\xfe\")\n";
	let report = res!(report_of(MAIN, &[(MAIN, &src[..]), ("/proj/terms.typ", terms)], None));
	let d = res!(site(&report, "terms.typ"));
	assert_eq!((d.severity, d.kind, d.file.as_str()), (Severity::Warning, DiagnosticKind::Encoding, "/proj/terms.typ"), "{}", d);
	assert!(d.message.contains("is not valid UTF-8"), "{}", d);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses it");
	Ok(())
}

/// A Fletcher edge written before any node, which the diagram has no node to draw from, is refused where
/// the figure stands, so strict refuses a figure not drawn as written; the nodes are still drawn.
#[test]
fn an_edge_a_diagram_cannot_place_is_refused() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\n#figure(diagram(edge((0,0), (0,1), \"->\"), node((0,0), [A]), node((0,1), [B])), caption: [Flow.])\n";
	let report = res!(report_of(MAIN, &[(MAIN, &src[..])], None));
	let d = res!(site(&report, "#figure (diagram)"));
	assert_eq!((d.severity, d.kind, d.line), (Severity::Warning, DiagnosticKind::Unsupported, 3), "{}", d);
	assert!(d.message.contains("is drawn without an edge written before any node"), "{}", d);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses it");
	Ok(())
}
