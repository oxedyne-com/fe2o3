//! The native half of the wasm compile surface's reporting: page counts, positioned diagnostics, strict
//! refusals, font families and engine identity, driven through the same `compile` functions the wasm
//! `DaimondTypst` calls with the source map installed as it installs one. External facts check each: the
//! PDF's page count from `pdfinfo`, the embedded family names from `fc-scan`, the commit from `git`.

use oxedyne_fe2o3_austenite::compile::{
	self,
	Diagnostic,
	Report,
};
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

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
	let sources: Vec<PathBuf> = map.keys().cloned().collect();
	res!(vfs::install(map));
	let main	= PathBuf::from(MAIN);
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<(Report, Vec<u8>)> {
		let (assembled, refusals, skip)	= res!(compile::assemble(&main, || Ok(fonts.clone())));
		let empty						= assembled.blocks.is_empty();
		let mut rendered				= res!(compile::author_and_run(assembled));
		let report	= Report::new(rendered.out.pages.len(), &refusals, skip, empty);
		let pdf		= res!(compile::emit_pdf(&mut rendered.out, &rendered.heads));
		Ok((report, pdf))
	};
	let out = match run() {
		Ok((r, pdf))	=> Compiled::Done(r, pdf),
		Err(e)			=> Compiled::Failed(compile::locate_error(&e, &main, &sources)),
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

#[test]
fn an_error_citing_no_source_path_reports_zero_zero() -> Outcome<()> {
	let e = err!("Something failed with no path."; Test);
	let d = compile::locate_error(&e, Path::new(MAIN), &[]);
	assert_eq!(fmt!("{}", d), "/proj/main.typ:0:0: Something failed with no path.");
	Ok(())
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
	for f in ["LibertinusSerif-Regular.otf", "LibertinusMono-Regular.otf", "latinmodern-math.otf"] {
		read.push(res!(fc_family(&dir.join(f))));
	}
	let mut listed: Vec<String> = compile::EMBEDDED_FAMILIES.iter().map(|s| s.to_string()).collect();
	read.sort();
	listed.sort();
	assert_eq!(listed, read, "the embedded list must be the files' own family names");
	Ok(())
}

#[test]
fn injected_families_are_listed_only_when_the_resolver_loads_them() -> Outcome<()> {
	let _turn = turn();
	let face = res!(std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf")));
	let main = PathBuf::from(MAIN);
	let given = [
		PathBuf::from("/fonts/Testface-Regular.otf"),	// a usable face under an arbitrary path
		PathBuf::from("Otherface-Bold.ttf"),			// a bare basename, bold only
		PathBuf::from("/fonts/Broken-Regular.ttf"),		// will not parse
		PathBuf::from("/fonts/nameless.otf"),			// no <Family>-<Variant> name
	];
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(main.clone(), b"= H\n".to_vec());
	for g in &given {
		let bytes = if g.to_string_lossy().contains("Broken") { b"not a font".to_vec() } else { face.clone() };
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
	want.push("Otherface".to_string());
	want.push("Testface".to_string());
	want.sort();
	assert_eq!(families, want);
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
