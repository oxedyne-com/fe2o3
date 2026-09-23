//! The native half of the wasm compile surface's reporting: page counts, positioned diagnostics, strict
//! refusals, font families and engine identity, driven through the same `compile` functions the wasm
//! `DaimondTypst` calls with the source map installed as it installs one. External facts check each: the
//! PDF's page count from `pdfinfo`, the embedded family names from `fc-scan`, the commit from `git`.

use oxedyne_fe2o3_austenite::compile::{
	self,
	Diagnostic,
	DiagnosticKind,
	Report,
	Severity,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;
use std::sync::{
	Arc,
	Mutex,
	MutexGuard,
};

// The source map is a process global, so the tests that install one take turns.
static VFS_TURN: Mutex<()> = Mutex::new(());

/// Takes this test's turn at the source map; a turn a failed test poisoned is still a turn.
fn turn() -> MutexGuard<'static, ()> {
	VFS_TURN.lock().unwrap_or_else(|p| p.into_inner())
}

const MAIN: &str = "/proj/main.typ";

/// What a compile of the installed project returned: its report and PDF, or the placed hard error.
enum Compiled {
	Done(Report, Vec<u8>),
	Failed(Diagnostic),
}

/// Installs `files`, compiles `MAIN` to PDF as the wasm `compileProject` does, and clears the map.
fn compile_pdf(files: &[(&str, &[u8])]) -> Outcome<Compiled> {
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (p, b) in files {
		map.insert(PathBuf::from(p), b.to_vec());
	}
	res!(vfs::install(map));
	let main	= PathBuf::from(MAIN);
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<(Report, Vec<u8>)> {
		let (assembled, refusals, skip)	= res!(compile::assemble(&main, || Ok(fonts.clone())));
		let empty						= assembled.blocks.is_empty();
		let mut rendered				= res!(compile::author_and_run(assembled));
		let report	= Report::new(rendered.out.pages.len(), &refusals, skip, empty);
		let pdf		= res!(compile::emit_pdf(&mut rendered.out, &rendered.heads, &rendered.doc_info));
		Ok((report, pdf))
	};
	let out = match run() {
		Ok((r, pdf))	=> Compiled::Done(r, pdf),
		Err(e)			=> Compiled::Failed(Diagnostic::from_error(&e, &main)),
	};
	res!(vfs::clear());
	Ok(out)
}

fn done(c: Compiled) -> Outcome<(Report, Vec<u8>)> {
	match c {
		Compiled::Done(r, pdf)	=> Ok((r, pdf)),
		Compiled::Failed(d)		=> Err(err!("Expected a compile, got the error {}.", d; Test)),
	}
}

fn failed(c: Compiled) -> Outcome<Diagnostic> {
	match c {
		Compiled::Failed(d)	=> Ok(d),
		Compiled::Done(r, _)	=> Err(err!("Expected an error, got {} page(s).", r.pages; Test)),
	}
}

/// The `Pages:` line `pdfinfo` reads from a PDF written to the scratch target directory.
fn pdfinfo_pages(pdf: &[u8], name: &str) -> Outcome<usize> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
	let path = dir.join(fmt!("{}.pdf", name));
	res!(std::fs::write(&path, pdf));
	let out = res!(Command::new("pdfinfo").arg(&path).output());
	let text = res!(String::from_utf8(out.stdout));
	for line in text.lines() {
		if let Some(rest) = line.strip_prefix("Pages:") {
			return Ok(res!(rest.trim().parse::<usize>()));
		}
	}
	Err(err!("pdfinfo printed no Pages line for {:?}: {}", path, text; Test))
}

fn prose(paras: usize) -> String {
	let mut s = String::from("= Report\n\n");
	for i in 0..paras {
		s.push_str(&fmt!("Paragraph {} sets enough ordinary words to fill a few lines of the measure, so \
			that forty of them spill well past a single A4 page of body text.\n\n", i));
	}
	s
}

#[test]
fn reported_pages_match_the_pdf_and_a_clean_compile_passes_strict() -> Outcome<()> {
	let _turn = turn();
	let src = prose(40);
	let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, src.as_bytes())]))));
	let external = res!(pdfinfo_pages(&pdf, "wasm_api_pages"));
	assert!(report.pages > 1, "the fixture must paginate, found {} page(s)", report.pages);
	assert_eq!(report.pages, external, "the reported page count must be the PDF's own");
	assert!(report.diagnostics.is_empty(), "a clean compile has no diagnostics: {:?}", report.diagnostics);
	assert_eq!(report.skipped, None);
	assert_eq!(report.strict_failure(Path::new(MAIN)), None, "a clean compile passes strict");
	Ok(())
}

#[test]
fn a_skipped_construct_is_positioned_and_refused_under_strict() -> Outcome<()> {
	let _turn = turn();
	// `#columns` opens line 5.
	let src = "= H\n\nBody.\n\n#columns(2)[a]\n\nMore body.\n";
	let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, src.as_bytes())]))));
	assert!(pdf.starts_with(b"%PDF-"), "a non-strict compile still produces its PDF");
	assert_eq!(report.diagnostics.len(), 1, "one refused site: {:?}", report.diagnostics);
	let d = &report.diagnostics[0];
	assert_eq!((d.file.as_str(), d.line, d.col), (MAIN, 5, 1), "the site's real position: {}", d);
	assert!(d.message.contains("#columns"), "the message names the construct: {}", d);
	assert_eq!(report.skipped.as_deref(), Some("skipped: #columns ×1"));

	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a compile that skipped a construct"; Test)),
	};
	assert_eq!((head.line, head.col), (5, 1), "strict reports at the first site: {}", head);
	assert!(fmt!("{}", head).starts_with("/proj/main.typ:5:1: strict:"), "the error line: {}", head);
	Ok(())
}

#[test]
fn a_failed_import_is_a_diagnostic_and_a_strict_error() -> Outcome<()> {
	let _turn = turn();
	let src = "= H\n\n#import \"template.typ\": *\n\nBody.\n";
	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, src.as_bytes())]))));
	let first = match report.diagnostics.first() {
		Some(d)	=> d.clone(),
		None	=> return Err(err!("an unresolved #import must be reported"; Test)),
	};
	assert_eq!((first.line, first.col), (3, 1), "{}", first);
	assert!(first.message.contains("#import"), "{}", first);
	assert!(report.strict_failure(Path::new(MAIN)).is_some());
	Ok(())
}

#[test]
fn strict_refuses_an_empty_source_and_zero_pages() -> Outcome<()> {
	let _turn = turn();
	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, b"")]))));
	assert!(report.empty, "an empty main reads no content block");
	assert!(report.diagnostics.is_empty(), "and refuses nothing, so only the emptiness catches it");
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a source that sets nothing"; Test)),
	};
	assert_eq!(fmt!("{}", head), "/proj/main.typ:1:1: strict: the source sets no content.");

	let none = Report { pages: 0, diagnostics: Vec::new(), skipped: None, empty: false };
	let head = match none.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse zero pages"; Test)),
	};
	assert!(head.message.contains("no pages"), "{}", head);
	Ok(())
}

#[test]
fn a_missing_include_is_placed_at_the_line_that_cites_it() -> Outcome<()> {
	let _turn = turn();
	// Cited from the root.
	let root = "= H\n\nBody.\n#include \"ch1.typ\"\n";
	let d = res!(failed(res!(compile_pdf(&[(MAIN, root.as_bytes())]))));
	assert_eq!((d.file.as_str(), d.line, d.col), (MAIN, 4, 10), "{}", d);
	assert!(d.message.contains("ch1.typ") && !d.message.contains(".rs:"), "plain words: {}", d);

	// Cited from a chapter one directory down, resolved against the chapter's own directory.
	let root	= "= H\n\n#include \"parts/a.typ\"\n";
	let chap	= "== A\n\nText.\n\n#include \"../gone.typ\"\n";
	let d = res!(failed(res!(compile_pdf(&[(MAIN, root.as_bytes()), ("/proj/parts/a.typ", chap.as_bytes())]))));
	assert_eq!((d.file.as_str(), d.line, d.col), ("/proj/parts/a.typ", 5, 10), "{}", d);
	Ok(())
}

/// The site travels in the error from the read that failed, so a path quoted earlier in an unrelated
/// line cannot draw the report to itself, as a search of the sources for the path's text would.
#[test]
fn a_missing_include_is_placed_at_its_own_line_when_an_earlier_line_quotes_the_same_path() -> Outcome<()> {
	let _turn = turn();
	let root = "= H\n\nSee \"gone.typ\" for background.\n\n#include \"gone.typ\"\n";
	let d = res!(failed(res!(compile_pdf(&[(MAIN, root.as_bytes())]))));
	assert_eq!((d.file.as_str(), d.line, d.col), (MAIN, 5, 10), "{}", d);
	Ok(())
}

/// A site in a book root is placed at its own line, past an `#include` as before one.
#[test]
fn a_site_after_an_include_is_placed_at_its_own_line() -> Outcome<()> {
	let _turn = turn();
	let root = "= H\n\nIntro.\n\n#include \"a.typ\"\n\nAfter.\n\n#columns(2)[x]\n";
	let chap = "== A\n\nText.\n";
	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, root.as_bytes()), ("/proj/a.typ", chap.as_bytes())]))));
	assert_eq!(report.diagnostics.len(), 1, "{:?}", report.diagnostics);
	let d = &report.diagnostics[0];
	assert_eq!((d.file.as_str(), d.line, d.col), (MAIN, 9, 1), "{}", d);
	Ok(())
}

#[test]
fn an_error_carrying_no_site_reports_zero_zero() -> Outcome<()> {
	let e = err!("Something failed with no path."; Test);
	let d = Diagnostic::from_error(&e, Path::new(MAIN));
	assert_eq!(fmt!("{}", d), "/proj/main.typ:0:0: Something failed with no path.");
	assert_eq!(d.kind, DiagnosticKind::Internal);
	Ok(())
}

/// Each kind is fixed where its fault is raised: a site not set as written by the reader's refusal class,
/// a hard error by the tags it was raised with, however many frames it then crossed, and a strict refusal
/// by its cause. A site is a warning and a hard error an error.
#[test]
fn each_diagnostic_kind_is_fixed_where_its_fault_is_raised() -> Outcome<()> {
	let _turn = turn();

	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, b"= H\n\n#columns(2)[a]\n")]))));
	let d = &report.diagnostics[0];
	assert_eq!((d.severity, d.kind, d.hint.as_deref()), (Severity::Warning, DiagnosticKind::Unsupported, None), "{}", d);
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a skipped construct"; Test)),
	};
	assert_eq!((head.severity, head.kind), (Severity::Error, DiagnosticKind::Unsupported), "{}", head);
	assert_eq!(head.hint.as_deref(), Some("skipped: #columns ×1"), "{}", head);

	// Passed over, like any other construct the reader does not set.
	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, b"= H\n\n#import \"t.typ\": *\n\nBody.\n")]))));
	assert_eq!(report.diagnostics[0].kind, DiagnosticKind::Unsupported, "{}", report.diagnostics[0]);

	// A missing image is set as a placeholder with a warning, at its own line, which strict refuses.
	let src = b"= H\n\nBody.\n\n#figure(image(\"gone.png\"), caption: [A figure.])\n";
	let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, src)]))));
	assert!(pdf.starts_with(b"%PDF-"), "the placeholder still sets");
	let d = &report.diagnostics[0];
	assert_eq!((d.severity, d.kind, d.file.as_str(), d.line), (Severity::Warning, DiagnosticKind::MissingFile, MAIN, 5), "{}", d);
	assert!(d.message.contains("gone.png"), "{}", d);
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a missing image"; Test)),
	};
	assert_eq!((head.kind, head.line), (DiagnosticKind::MissingFile, 5), "{}", head);

	let d = res!(failed(res!(compile_pdf(&[(MAIN, b"= H\n\n#include \"ch1.typ\"\n")]))));
	assert_eq!((d.severity, d.kind), (Severity::Error, DiagnosticKind::MissingFile), "{}", d);

	let src = b"#set text(font: \"Nonesuch Sans\")\n\n= H\n\nBody.\n";
	let d = res!(failed(res!(compile_pdf(&[(MAIN, src)]))));
	assert_eq!((d.severity, d.kind), (Severity::Error, DiagnosticKind::MissingFont), "{}", d);
	assert!(d.message.contains("Nonesuch Sans"), "{}", d);

	// A bound reached, as layout that will not settle is raised, crossing frames on its way out.
	fn settle() -> Outcome<()> {
		Err(err!("Composition did not converge."; Data, Excessive, LimitReached))
	}
	fn compose() -> Outcome<()> {
		res!(settle());
		Ok(())
	}
	let e = match compose() {
		Ok(())	=> return Err(err!("The error was supposed to propagate."; Bug)),
		Err(e)	=> e,
	};
	assert_eq!(Diagnostic::from_error(&e, Path::new(MAIN)).kind, DiagnosticKind::Limit);

	let (report, _) = res!(done(res!(compile_pdf(&[(MAIN, b"")]))));
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a source that sets nothing"; Test)),
	};
	assert_eq!((head.severity, head.kind), (Severity::Error, DiagnosticKind::Internal), "{}", head);
	Ok(())
}

/// An image the project does not hold is set as a placeholder, and a section banner without its logo, and
/// each is reported where it is named as a missing file, which strict refuses. An image the project holds
/// is reported nowhere.
#[test]
fn a_missing_image_is_reported_where_it_is_named_and_a_present_one_is_not() -> Outcome<()> {
	let _turn = turn();
	let png = res!(res!(Pixmap::filled(4, 4, Rgba::BLACK)).to_png());
	let src = b"= H\n\n#figure(image(\"here.png\"), caption: [Present.])\n\n#image(\"gone.png\")\n\n\
		#section-banner(\"logo.svg\")\n\nBody.\n";
	let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, &src[..]), ("/proj/here.png", png.as_slice())]))));
	assert!(pdf.starts_with(b"%PDF-"), "a non-strict compile still sets the page");
	let sites: Vec<(Severity, DiagnosticKind, &str, usize, usize)> = report.diagnostics.iter()
		.map(|d| (d.severity, d.kind, d.file.as_str(), d.line, d.col))
		.collect();
	assert_eq!(sites, [
		(Severity::Warning, DiagnosticKind::MissingFile, MAIN, 5, 1),
		(Severity::Warning, DiagnosticKind::MissingFile, MAIN, 7, 1),
	], "{:?}", report.diagnostics);
	assert!(report.diagnostics[0].message.contains("gone.png"), "{}", report.diagnostics[0]);
	assert!(report.diagnostics[1].message.contains("logo.svg"), "{}", report.diagnostics[1]);
	let head = match report.strict_failure(Path::new(MAIN)) {
		Some(h)	=> h,
		None	=> return Err(err!("strict must refuse a missing image"; Test)),
	};
	assert_eq!((head.severity, head.kind, head.line), (Severity::Error, DiagnosticKind::MissingFile, 5), "{}", head);
	Ok(())
}

/// A `#set document` inside a container is not applied -- Typst refuses one there -- so it is reported at its
/// site as a construct not set as written, which strict refuses, and the Info dictionary takes nothing from it.
#[test]
fn a_set_document_in_a_container_is_refused() -> Outcome<()> {
	let _turn = turn();
	let src = b"= H\n\n#styled-box[\n#set document(title: \"Boxed\")\nInside.\n]\n\nAfter.\n";
	let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, &src[..])]))));
	let d = match report.diagnostics.iter().find(|d| d.message.contains("#set document")) {
		Some(d)	=> d.clone(),
		None	=> return Err(err!("a #set document in a box must be reported: {:?}", report.diagnostics; Test)),
	};
	assert_eq!((d.severity, d.kind, d.file.as_str()), (Severity::Warning, DiagnosticKind::Unsupported, MAIN), "{}", d);
	assert!(report.strict_failure(Path::new(MAIN)).is_some(), "strict refuses it");
	assert!(!pdf.windows(5).any(|w| w == b"Boxed"), "no Info entry is written from inside the box");
	Ok(())
}

/// Strict mode decides by severity and kind: an error always refuses, a warning only where its kind says
/// the document was not set as written, and any other warning stands beside the PDF.
#[test]
fn strict_refuses_by_severity_and_kind() {
	let site = |severity: Severity, kind: DiagnosticKind| Diagnostic {
		file:		MAIN.to_string(),
		line:		3,
		col:		1,
		message:	"a site".to_string(),
		severity,
		kind,
		hint:		None,
	};
	let report = |d: Diagnostic| Report { pages: 1, diagnostics: vec![d], skipped: None, empty: false };
	for kind in [DiagnosticKind::Unsupported, DiagnosticKind::MissingFile, DiagnosticKind::Package,
		DiagnosticKind::MissingFont]
	{
		assert!(report(site(Severity::Warning, kind)).strict_failure(Path::new(MAIN)).is_some(), "{}", kind);
	}
	for kind in [DiagnosticKind::Limit, DiagnosticKind::Syntax, DiagnosticKind::Type,
		DiagnosticKind::UnknownVariable, DiagnosticKind::Internal]
	{
		assert_eq!(report(site(Severity::Warning, kind)).strict_failure(Path::new(MAIN)), None, "{}", kind);
		assert!(report(site(Severity::Error, kind)).strict_failure(Path::new(MAIN)).is_some(), "{}", kind);
	}
}

/// The words a caller switches on. A change here breaks every consumer's mapping.
#[test]
fn the_kind_and_severity_words_are_the_wire_contract() {
	let words: Vec<&str> = [
		DiagnosticKind::MissingFile,
		DiagnosticKind::MissingFont,
		DiagnosticKind::Syntax,
		DiagnosticKind::Type,
		DiagnosticKind::UnknownVariable,
		DiagnosticKind::Package,
		DiagnosticKind::Limit,
		DiagnosticKind::Unsupported,
		DiagnosticKind::Internal,
	].iter().map(|k| k.as_str()).collect();
	assert_eq!(words, ["missing_file", "missing_font", "syntax", "type", "unknown_variable", "package",
		"limit", "unsupported", "internal"]);
	assert_eq!([Severity::Error.as_str(), Severity::Warning.as_str()], ["error", "warning"]);
}

/// The family name `fc-scan` reads from a font file's own name table.
fn fc_family(path: &Path) -> Outcome<String> {
	let out = res!(Command::new("fc-scan").arg("--format").arg("%{family[0]}").arg(path).output());
	Ok(res!(String::from_utf8(out.stdout)).trim().to_string())
}

#[test]
fn embedded_families_are_the_names_in_the_font_files() -> Outcome<()> {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts");
	let mut read = Vec::new();
	for f in ["LibertinusSerif-Regular.otf", "LibertinusMono-Regular.otf", "NewCMMath-Regular.otf"] {
		read.push(res!(fc_family(&dir.join(f))));
	}
	let mut listed: Vec<String> = compile::EMBEDDED_FAMILIES.iter().map(|s| s.to_string()).collect();
	read.sort();
	listed.sort();
	// Same-family rather than a literal match: the New Computer Modern files declare `NewComputerModern
	// Math` in their own name table, where the embedded list and every document write it `New Computer
	// Modern Math` (see `fonts::same_family`'s own doc comment for why).
	assert_eq!(listed.len(), read.len(), "the embedded list must name exactly the files' own families");
	for (l, r) in listed.iter().zip(read.iter()) {
		assert!(fonts::same_family(l, r), "{:?} is not the family {:?} declares", l, r);
	}
	Ok(())
}

/// `fontFamilies()` (`compile::font_families`) must report the family a font file *declares in its own
/// name table*, never one guessed from the file's own name (M3): a file named `<Family>-<Variant>` that
/// declares something else lists the declared name, and a file with no such name (`felipa.ttf`'s own
/// shape) is no longer excluded for lacking one.
#[test]
fn injected_families_are_read_from_content_not_the_file_name() -> Outcome<()> {
	let _turn = turn();
	let libertinus = res!(std::fs::read(
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf")));
	let noto = res!(std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("..").join("fe2o3_font").join("fonts").join("NotoSans-Regular.ttf")));
	let main = PathBuf::from(MAIN);
	let given = [
		// Named as though it were "Otherface", but its bytes declare Noto Sans -- proves the list is not
		// read from the file name.
		PathBuf::from("/fonts/Otherface-Bold.ttf"),
		// No `<Family>-<Variant>` shape at all -- felipa.ttf's own shape -- yet declares Libertinus Serif.
		PathBuf::from("/fonts/nameless.otf"),
		// Will not parse as a font at all, so it contributes nothing either way.
		PathBuf::from("/fonts/Broken-Regular.ttf"),
	];
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(main.clone(), b"= H\n".to_vec());
	for g in &given {
		let name = g.to_string_lossy();
		let bytes = if name.contains("Otherface") {
			noto.clone()
		} else if name.contains("Broken") {
			b"not a font".to_vec()
		} else {
			libertinus.clone()
		};
		// Installed as the wasm surface installs a font: at the given path and at the resolver's path.
		if let Some(routed) = oxedyne_fe2o3_austenite::book::project_font_path(&main, g) {
			map.insert(routed, bytes.clone());
		}
		map.insert(g.clone(), bytes);
	}
	let bare = compile::font_families(&main, &[]);
	res!(vfs::install(map));
	let families = compile::font_families(&main, &given);
	res!(vfs::clear());

	let mut want: Vec<String> = compile::EMBEDDED_FAMILIES.iter().map(|s| s.to_string()).collect();
	want.sort();
	assert_eq!(bare, want, "with nothing injected only the embedded families are listed");

	// Noto Sans is new; Libertinus Serif was already embedded, so the nameless file's declared family
	// changes nothing observable in the list besides no longer being silently dropped from the read.
	let mut want_injected = want.clone();
	want_injected.push("Noto Sans".to_string());
	want_injected.sort();
	want_injected.dedup();
	assert_eq!(families, want_injected, "families are read from content, never guessed from a file name");
	assert!(!families.iter().any(|f| f == "Otherface"), "the misleading file name must not appear: {:?}", families);
	Ok(())
}

/// The PostScript names `pdffonts` reads from a PDF's embedded fonts, subset tags removed.
fn pdffonts_names(pdf: &[u8], name: &str) -> Outcome<Vec<String>> {
	let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(fmt!("{}.pdf", name));
	res!(std::fs::write(&path, pdf));
	let out = res!(Command::new("pdffonts").arg(&path).output());
	let text = res!(String::from_utf8(out.stdout));
	Ok(text.lines().skip(2)
		.filter_map(|l| l.split_whitespace().next())
		.map(|n| n.split_once('+').map_or(n, |(_, rest)| rest).to_string())
		.collect())
}

/// Every family the list names is one a compile sets, the faces of a collection included: the list and
/// the resolver read the same name tables.
#[test]
fn every_listed_family_is_set_by_a_compile_including_a_collections_faces() -> Outcome<()> {
	let _turn = turn();
	let dir		= PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts");
	let noto	= res!(std::fs::read(dir.join("NotoSans-Regular.ttf")));
	let mono	= res!(std::fs::read(dir.join("DejaVuSansMono.ttf")));
	let ttc		= res!(oxedyne_fe2o3_graphics::pdf_font::collection_of(&[&noto, &mono]));
	let main	= PathBuf::from(MAIN);
	let given	= PathBuf::from("/fonts/pair.ttc");
	let routed	= match oxedyne_fe2o3_austenite::book::project_font_path(&main, &given) {
		Some(p)	=> p,
		None	=> return Err(err!("A font path with a file name must route somewhere."; Test)),
	};

	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(main.clone(), b"= H\n".to_vec());
	map.insert(routed.clone(), ttc.clone());
	res!(vfs::install(map));
	let families = compile::font_families(&main, &[given]);
	res!(vfs::clear());
	let injected: Vec<&String> = families.iter()
		.filter(|f| !compile::EMBEDDED_FAMILIES.contains(&f.as_str()))
		.collect();
	assert_eq!(injected, ["DejaVu Sans Mono", "Noto Sans"], "one entry per face: {:?}", families);

	let routed = routed.to_string_lossy().to_string();
	for (family, postscript) in [("Noto Sans", "NotoSans-Regular"), ("DejaVu Sans Mono", "DejaVuSansMono")] {
		let src = fmt!("#set text(font: \"{}\")\n\nHamburgefonts.\n", family);
		let (report, pdf) = res!(done(res!(compile_pdf(&[(MAIN, src.as_bytes()), (routed.as_str(), ttc.as_slice())]))));
		assert!(report.diagnostics.is_empty(), "{}: {:?}", family, report.diagnostics);
		let fonts = res!(pdffonts_names(&pdf, &fmt!("wasm_api_ttc_{}", postscript)));
		assert!(fonts.iter().any(|f| f == postscript), "{} is embedded from the collection: {:?}", family, fonts);
	}
	Ok(())
}

#[test]
fn engine_identity_is_the_crate_version_and_the_built_commit() -> Outcome<()> {
	let manifest = res!(std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")));
	let version = manifest.lines()
		.find_map(|l| l.strip_prefix("version = \"").and_then(|r| r.strip_suffix('"')));
	assert_eq!(Some(compile::engine_version()), version);

	let out = res!(Command::new("git").args(["rev-parse", "--short=12", "HEAD"])
		.current_dir(env!("CARGO_MANIFEST_DIR")).output());
	let head = res!(String::from_utf8(out.stdout)).trim().to_string();
	let hash = compile::engine_git_hash();
	assert_eq!(head.len(), 12, "git must name the commit: {:?}", head);
	assert!(hash == head || hash == fmt!("{}-dirty", head), "built from {:?}, HEAD is {:?}", hash, head);
	Ok(())
}
