//! The door, driven natively: every call the wasm surface offers goes through [`door`], so what runs here is
//! what the browser runs. External facts check each result: the sites and warnings of `typst` 0.15.1
//! (`--diagnostic-format short`, whose line is 1-based and whose column is 0-based, in characters), the
//! rows of `typst query`, the families `typst fonts` lists, and `pdfinfo` and `pdftotext` on the PDF.
//!
//! The door's columns are 1-based UTF-16 code units, so a site's column here is Typst's character column
//! plus one, plus one for each character outside the Basic Multilingual Plane before it on its line.

#[allow(dead_code)]
#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::json::{
	self,
	J,
};
use harness::pdf::{
	self,
	tool,
	TYPST,
};

use oxedyne_fe2o3_austenite::compile::DiagnosticKind;
use oxedyne_fe2o3_austenite::door::{
	self,
	Failure,
	Instance,
	Made,
	Project,
};
use oxedyne_fe2o3_austenite::emit::sinks::Chunks;
use oxedyne_fe2o3_austenite::eval::package;
use oxedyne_fe2o3_austenite::fonts::FontBook;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pdf_font::collection_of;

use std::collections::BTreeSet;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

// The cases the node smoke prints one line for, and the lines it must equal.
const SKELETON: &str = "#set page(width: 200pt, height: 100pt)\n= Skeleton\nA short paragraph.\n";

fn work_dir(name: &str) -> Outcome<PathBuf> { pdf::work_dir(&fmt!("eval-door-{}", name)) }

/// A project of `files`, `/main.typ` among them, strict.
fn strict(files: &[(&str, &str)]) -> Project {
	let mut p = project(files);
	p.strict = true;
	p
}

fn project(files: &[(&str, &str)]) -> Project {
	let mut p = Project::default();
	for (path, text) in files {
		p.sources.push((path.to_string(), text.to_string()));
	}
	p
}

fn pdf_of(made: &Made<Chunks>) -> Vec<u8> { made.product.to_vec() }

fn must_pdf(r: Result<Made<Chunks>, Failure>) -> Outcome<Made<Chunks>> {
	match r {
		Ok(m)	=> Ok(m),
		Err(f)	=> Err(err!("The door refused: {} {:?}", f.head, f.head.kind; Test)),
	}
}

fn must_fail(r: Result<Made<Chunks>, Failure>) -> Outcome<Failure> {
	match r {
		Ok(_)	=> Err(err!("The door compiled what it should have refused."; Test)),
		Err(f)	=> Ok(f),
	}
}

/// A failure's `<file>:<line>:<col>`, as the smoke prints it.
fn site(f: &Failure) -> String { fmt!("{}:{}:{}", f.head.file, f.head.line, f.head.col) }

/// Writes `files` under `dir` and returns the path of `main.typ`.
fn write_project(dir: &Path, files: &[(&str, &str)]) -> Outcome<PathBuf> {
	res!(std::fs::create_dir_all(dir));
	for (path, text) in files {
		let at = dir.join(path.trim_start_matches('/'));
		if let Some(parent) = at.parent() {
			res!(std::fs::create_dir_all(parent));
		}
		res!(std::fs::write(&at, text));
	}
	Ok(dir.join("main.typ"))
}

/// Typst's run with the cap every oracle run is given: its standard error, whatever the exit.
fn typst(args: &[&str]) -> Outcome<(bool, String, String)> {
	let out = res!(Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", TYPST])
		.args(args)
		.output());
	Ok((
		out.status.success(),
		String::from_utf8_lossy(&out.stdout).to_string(),
		String::from_utf8_lossy(&out.stderr).to_string(),
	))
}

/// Typst's first `severity` site in its short format, as (line, column), the column 0-based.
fn typst_site(main: &Path, severity: &str) -> Outcome<(usize, usize)> {
	let out = main.with_extension("pdf");
	let (_, _, stderr) = res!(typst(&["compile", "--diagnostic-format", "short",
		&main.display().to_string(), &out.display().to_string()]));
	let marker = fmt!(": {}:", severity);
	for l in stderr.lines() {
		if let Some(at) = l.find(&marker) {
			let mut parts = l[..at].rsplitn(3, ':');
			let col = parts.next().and_then(|c| c.parse::<usize>().ok());
			let line = parts.next().and_then(|c| c.parse::<usize>().ok());
			if let (Some(line), Some(col)) = (line, col) {
				return Ok((line, col));
			}
		}
	}
	Err(err!("Typst reported no {} site for {}: {}", severity, main.display(), stderr; Test))
}

/// The column the door gives for Typst's 0-based character column `col` on `line` of `text`: 1-based, in
/// UTF-16 code units.
fn door_col(text: &str, line: usize, col: usize) -> usize {
	let row = text.split('\n').nth(line - 1).unwrap_or("");
	let before: usize = row.chars().take(col).map(|c| c.len_utf16()).sum();
	before + 1
}

fn pdf_pages(pdf: &[u8], dir: &Path, name: &str) -> Outcome<usize> {
	let path = dir.join(fmt!("{}.pdf", name));
	res!(std::fs::write(&path, pdf));
	let info = res!(tool("pdfinfo", &[], &path, &[]));
	let line = res!(info.lines().find(|l| l.starts_with("Pages:")).ok_or_else(|| err!("pdfinfo gave no page count"; Test)));
	Ok(res!(line.trim_start_matches("Pages:").trim().parse::<usize>().map_err(|e| err!("{}", e; Test))))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ G9: NO FALSE GREEN                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn g9_a_missing_font_is_refused_under_strict_at_typsts_site() -> Outcome<()> {
	let dir		= res!(work_dir("g9-font"));
	let text	= "#set text(font: \"Nope Sans\")\nHello.\n";
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let (line, col) = res!(typst_site(&main, "warning"));
	let mut inst = Instance::new();
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFont, "the kind: {:?}", f.head);
	assert_eq!((f.head.line, f.head.col), (line, door_col(text, line, col)), "at typst's site {}:{}: {:?}", line, col, f.head);
	assert_eq!(f.head.file, "/main.typ");
	assert!(f.head.message.starts_with("strict:"), "a strict refusal: {}", f.head.message);
	assert_eq!(f.head.hint.as_deref(), Some("missing_font \u{d7}1"), "the hint names what refused");
	// Not strict, the same document is a PDF with the warning beside it.
	let made = res!(must_pdf(inst.compile_pdf(&project(&[("/main.typ", text)]))));
	assert!(made.report.diagnostics.iter().any(|d| d.kind == DiagnosticKind::MissingFont), "{:?}", made.report.diagnostics);
	Ok(())
}

#[test]
fn g9_a_missing_import_is_refused_at_typsts_site() -> Outcome<()> {
	let dir		= res!(work_dir("g9-import"));
	let text	= "Intro.\n#import \"gone.typ\": thing\n";
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let (line, col) = res!(typst_site(&main, "error"));
	let mut inst = Instance::new();
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFile, "the kind: {:?}", f.head);
	assert_eq!((f.head.line, f.head.col), (line, door_col(text, line, col)), "at typst's site {}:{}: {:?}", line, col, f.head);
	assert_eq!(f.head.file, "/main.typ");
	Ok(())
}

#[test]
fn g9_a_missing_image_is_refused_at_typsts_site() -> Outcome<()> {
	let dir		= res!(work_dir("g9-image"));
	let text	= "Text.\n\n#image(\"gone.png\")\n";
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let (line, col) = res!(typst_site(&main, "error"));
	let mut inst = Instance::new();
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFile, "the kind: {:?}", f.head);
	assert_eq!((f.head.line, f.head.col), (line, door_col(text, line, col)), "at typst's site {}:{}: {:?}", line, col, f.head);
	assert_eq!(f.head.file, "/main.typ");
	Ok(())
}

#[test]
fn g9_a_main_that_sets_no_content_is_refused_at_main_one_one_as_internal_though_typst_raises_nothing() -> Outcome<()> {
	let dir = res!(work_dir("g9-empty"));
	let mut inst = Instance::new();
	for (name, text) in [("empty", ""), ("spaces", "  \n\n  "), ("set-only", "#set text(size: 11pt)\n"), ("let-only", "#let x = 1\n")] {
		// Typst compiles each to one blank page and raises nothing at all.
		let case = dir.join(name);
		let main = res!(write_project(&case, &[("main.typ", text)]));
		let (ok, _, stderr) = res!(typst(&["compile", "--diagnostic-format", "short",
			&main.display().to_string(), &main.with_extension("pdf").display().to_string()]));
		assert!(ok && stderr.trim().is_empty(), "typst compiles {} clean: {}", name, stderr);

		let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
		assert_eq!(site(&f), "/main.typ:1:1", "{}: {:?}", name, f.head);
		assert_eq!(f.head.kind, DiagnosticKind::Internal, "{}: {:?}", name, f.head);
		assert!(f.head.message.contains("sets no content"), "{}: {}", name, f.head.message);
		// Not strict, the blank page is made.
		let made = res!(must_pdf(inst.compile_pdf(&project(&[("/main.typ", text)]))));
		assert_eq!(made.report.pages, 1, "{}", name);
	}
	// A main that sets something is not blank.
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", "Words.\n")]))));
	assert_eq!(made.report.pages, 1);
	Ok(())
}

#[test]
fn a_main_that_is_not_utf8_is_an_encoding_error_at_the_main() -> Outcome<()> {
	let mut inst = Instance::new();
	let mut p = Project::default();
	p.assets.push(("/main.typ".to_string(), vec![b'a', 0xff, 0xfe, b'b']));
	let f = res!(must_fail(inst.compile_pdf(&p)));
	assert_eq!(f.head.kind, DiagnosticKind::Encoding, "{:?}", f.head);
	assert_eq!(site(&f), "/main.typ:0:0");
	Ok(())
}

#[test]
fn a_project_with_no_main_source_is_a_missing_file_error() -> Outcome<()> {
	let mut inst = Instance::new();
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/other.typ", "x")]))));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFile, "{:?}", f.head);
	assert_eq!(site(&f), "/main.typ:0:0");
	Ok(())
}

#[test]
fn the_project_is_sealed_so_nothing_resolves_from_the_hosts_filesystem() -> Outcome<()> {
	// The host does hold the file the project names, so only the seal keeps it out.
	assert!(Path::new("/etc/passwd").is_file(), "the host holds /etc/passwd");
	let mut inst = Instance::new();
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", "#read(\"/etc/passwd\")\n")]))));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFile, "{:?}", f.head);
	assert!(f.head.message.contains("file not found"), "{}", f.head.message);
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STRICT PASS, BESIDE THE PDF, AND COLUMNS                                   │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn a_strict_skeleton_is_one_page_with_no_diagnostic_and_an_import_rename_stays_a_lint_beside_the_pdf() -> Outcome<()> {
	let dir = res!(work_dir("strict-pass"));
	let mut inst = Instance::new();
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", SKELETON)]))));
	assert_eq!(made.report.pages, 1);
	assert!(made.report.diagnostics.is_empty(), "{:?}", made.report.diagnostics);
	assert!(made.needs.is_empty());
	let bytes = pdf_of(&made);
	assert!(bytes.starts_with(b"%PDF-"));
	assert_eq!(res!(pdf_pages(&bytes, &dir, "skeleton")), 1, "the report's count is the PDF's");

	// Typst's own warning stands beside the PDF; strict does not refuse it.
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[
		("/main.typ", "#import \"t.typ\": doc as doc\nBody text.\n"),
		("/t.typ", "#let doc = 1\n"),
	]))));
	let lints: Vec<_> = made.report.diagnostics.iter().filter(|d| d.message.contains("unnecessary import rename")).collect();
	assert_eq!(lints.len(), 1, "{:?}", made.report.diagnostics);
	assert_eq!(lints[0].kind, DiagnosticKind::Lint);
	assert_eq!(lints[0].severity.as_str(), "warning");
	Ok(())
}

#[test]
fn a_document_that_does_not_settle_in_five_passes_stands_beside_the_pdf_under_strict() -> Outcome<()> {
	// A state that feeds itself never settles; Typst stops after five passes with a warning.
	let text = "#let s = state(\"s\", 0)\n\
		#context s.update(s.final() + 1)\n\
		#context [#metadata(s.final()) <probe>]\n";
	let dir		= res!(work_dir("unsettled"));
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let (_, _, stderr) = res!(typst(&["compile", "--diagnostic-format", "short",
		&main.display().to_string(), &main.with_extension("pdf").display().to_string()]));
	let typst_unsettled = stderr.contains("did not converge");
	let mut inst = Instance::new();
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	let ours = made.report.diagnostics.iter().any(|d| d.kind == DiagnosticKind::Limit);
	assert_eq!(ours, typst_unsettled, "typst {}, the door {}: {:?}", typst_unsettled, ours, made.report.diagnostics);
	assert!(typst_unsettled, "the probe document must not settle in typst either: {}", stderr);
	Ok(())
}

#[test]
fn a_character_outside_the_bmp_before_a_site_adds_one_utf16_unit_to_typsts_character_column() -> Outcome<()> {
	let dir = res!(work_dir("utf16"));
	let mut inst = Instance::new();
	// The ideographic space is one UTF-16 unit, the emoji two.
	for (name, lead) in [("plain", ""), ("bmp", "\u{3000}"), ("astral", "\u{1F600}"), ("astral2", "\u{1F600}\u{1F600}x")] {
		let text = fmt!("{}#include \"gone.typ\"\n", lead);
		let main = res!(write_project(&dir.join(name), &[("main.typ", &text)]));
		let (line, col) = res!(typst_site(&main, "error"));
		let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", &text)]))));
		let want = door_col(&text, line, col);
		let astral = lead.chars().filter(|c| c.len_utf16() == 2).count();
		assert_eq!(f.head.col, col + 1 + astral, "{}: typst column {} plus one plus {} astral", name, col, astral);
		assert_eq!((f.head.line, f.head.col), (line, want), "{}: {:?}", name, f.head);
	}
	Ok(())
}

#[test]
fn a_line_break_other_than_the_line_feed_counts_a_line_as_typst_counts_it() -> Outcome<()> {
	let dir = res!(work_dir("breaks"));
	let mut inst = Instance::new();
	for (name, brk) in [("cr", "\r"), ("crlf", "\r\n"), ("ls", "\u{2028}"), ("ff", "\u{c}")] {
		let text = fmt!("a{}#import \"gone.typ\"\n", brk);
		let main = res!(write_project(&dir.join(name), &[("main.typ", &text)]));
		let (line, col) = res!(typst_site(&main, "error"));
		let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", &text)]))));
		assert_eq!(line, 2, "{}: typst counts the break as one", name);
		assert_eq!((f.head.line, f.head.col), (line, col + 1), "{}: {:?}", name, f.head);
	}
	Ok(())
}

// The cases of `tools/bench/door_smoke.mjs`, in its order, and the lines both print: the browser's result
// for each must be this door's, so each is held to `tests/fixtures/door_smoke_lines.txt`.
const SMOKE: [(&str, &str); 5] = [
	("skeleton",		SKELETON),
	("missing_font",	"#set text(font: \"Nope Sans\")\nHello.\n"),
	("missing_import",	"Intro.\n#import \"gone.typ\": thing\n"),
	("missing_image",	"Text.\n\n#image(\"gone.png\")\n"),
	("empty_main",		""),
];

#[test]
fn the_smoke_cases_print_the_lines_the_node_smoke_is_held_to() -> Outcome<()> {
	let mut inst = Instance::new();
	let mut got: Vec<String> = Vec::new();
	for (name, text) in SMOKE {
		match inst.compile_pdf(&strict(&[("/main.typ", text)])) {
			Ok(_)	=> got.push(fmt!("{} ok - -:0:0", name)),
			Err(f)	=> got.push(fmt!("{} error {} {}", name, f.head.kind.as_str(), site(&f))),
		}
	}
	let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/door_smoke_lines.txt");
	let want: Vec<String> = res!(std::fs::read_to_string(&file)).lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect();
	assert_eq!(got, want, "the door's lines are the ones the node smoke is held to");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ISOLATION, PACKAGES AND FONTS                                              │
// └───────────────────────────────────────────────────────────────────────────┘

fn noto() -> Outcome<Vec<u8>> {
	Ok(res!(std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts").join("NotoSans-Regular.ttf"))))
}

fn font_file(name: &str) -> Outcome<Vec<u8>> {
	Ok(res!(std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts").join(name))))
}

#[test]
fn two_projects_in_turn_on_one_instance_share_no_file_and_no_font() -> Outcome<()> {
	let mut inst = Instance::new();
	// The first holds a file and a font the second does not.
	let mut first = strict(&[
		("/main.typ", "#import \"/shared.typ\": greeting\n#set text(font: \"Noto Sans\")\n#greeting\n"),
		("/shared.typ", "#let greeting = [Hello.]\n"),
	]);
	first.fonts.push(("noto.ttf".to_string(), res!(noto())));
	let made = res!(must_pdf(inst.compile_pdf(&first)));
	assert!(made.report.diagnostics.is_empty(), "{:?}", made.report.diagnostics);
	assert!(inst.font_families(Some(&first)).iter().any(|f| f == "Noto Sans"));

	// The second names both and holds neither.
	let second = strict(&[("/main.typ", "#import \"/shared.typ\": greeting\n#greeting\n")]);
	let f = res!(must_fail(inst.compile_pdf(&second)));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFile, "the first project's file does not resolve: {:?}", f.head);
	let third = strict(&[("/main.typ", "#set text(font: \"Noto Sans\")\nHello.\n")]);
	let f = res!(must_fail(inst.compile_pdf(&third)));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFont, "the first project's font does not resolve: {:?}", f.head);
	assert!(!inst.font_families(None).iter().any(|f| f == "Noto Sans"), "nor is it listed");
	assert!(!inst.font_families(Some(&second)).iter().any(|f| f == "Noto Sans"));
	Ok(())
}

const PKG: &str = "@local/dpk:0.1.0";

fn pkg_files() -> Vec<(String, Vec<u8>)> {
	vec![
		("typst.toml".to_string(), b"[package]\nname = \"dpk\"\nversion = \"0.1.0\"\nentrypoint = \"lib.typ\"\n".to_vec()),
		("lib.typ".to_string(), b"#let greet(n) = [Hello, #n.]\n".to_vec()),
	]
}

#[test]
fn a_supplied_package_is_imported_and_once_withdrawn_the_compile_fails_with_kind_package_and_names_it_in_needs() -> Outcome<()> {
	let dir		= res!(work_dir("package"));
	let text	= "#import \"@local/dpk:0.1.0\": greet\n#greet(\"door\")\n";
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let mut inst = Instance::new();

	assert_eq!(res!(door::supply_package(PKG, pkg_files())), 2);
	assert!(res!(door::packages()).iter().any(|s| s == PKG), "{:?}", res!(door::packages()));
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert!(made.needs.is_empty(), "{:?}", made.needs);
	let path = dir.join("dpk.pdf");
	res!(std::fs::write(&path, pdf_of(&made)));
	assert!(res!(pdf::text_of(&path)).contains("Hello, door."), "the package's function ran");

	assert!(res!(door::withdraw_package(PKG)), "it was held");
	assert!(!res!(door::withdraw_package(PKG)), "and is not held twice");
	assert!(!res!(door::packages()).iter().any(|s| s == PKG));
	let (line, col) = res!(typst_site(&main, "error"));
	let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert_eq!(f.head.kind, DiagnosticKind::Package, "{:?}", f.head);
	assert_eq!((f.head.line, f.head.col), (line, col + 1), "at typst's site: {:?}", f.head);
	assert_eq!(f.needs, vec![PKG.to_string()], "the specification, taken at the import");
	Ok(())
}

#[test]
fn needs_lists_each_unsupplied_package_once_in_order_from_the_diagnostic_not_the_message() -> Outcome<()> {
	let mut inst = Instance::new();
	// The first unsupplied import stops the compile, so one spec is named; a repeat of it names it once.
	let f = res!(must_fail(inst.compile_pdf(&project(&[
		("/main.typ", "#import \"@preview/zzz-absent:9.9.9\": x\n#import \"@preview/zzz-absent:9.9.9\": y\n"),
	]))));
	assert_eq!(f.needs, vec!["@preview/zzz-absent:9.9.9".to_string()]);
	Ok(())
}

#[test]
fn font_families_are_what_the_font_book_resolves_and_what_typst_adds_for_the_same_files() -> Outcome<()> {
	let dir		= res!(work_dir("families"));
	let noto	= res!(font_file("NotoSans-Regular.ttf"));
	let nmono	= res!(font_file("NotoSansMono-Regular.ttf"));
	let wide	= res!(font_file("DejaVuSans.ttf"));
	let ttc		= res!(collection_of(&[&noto, &nmono]));
	// A collection of two families, and a file whose name is another family's.
	let fonts	= dir.join("fonts");
	res!(std::fs::create_dir_all(&fonts));
	res!(std::fs::write(fonts.join("pair.ttc"), &ttc));
	res!(std::fs::write(fonts.join("Otherface-Bold.ttf"), &wide));
	let mut p = strict(&[("/main.typ", "Hello.\n")]);
	p.fonts.push(("pair.ttc".to_string(), ttc.clone()));
	p.fonts.push(("Otherface-Bold.ttf".to_string(), wide.clone()));

	let inst = Instance::new();
	let none	= inst.font_families(None);
	let listed	= inst.font_families(Some(&p));
	for want in ["Noto Sans", "Noto Sans Mono", "DejaVu Sans"] {
		assert!(listed.iter().any(|f| f == want), "{} is listed: {:?}", want, listed);
	}
	assert!(!listed.iter().any(|f| f == "Otherface"), "never guessed from a file name: {:?}", listed);
	let mut sorted = listed.clone();
	sorted.sort_by_key(|f| f.to_lowercase());
	assert_eq!(listed, sorted, "sorted");
	for embedded in ["Libertinus Serif", "New Computer Modern Math", "DejaVu Sans Mono"] {
		assert!(none.iter().any(|f| f == embedded), "{} is embedded: {:?}", embedded, none);
	}

	// Typst lists the same families for the same files, apart from the faces each carries of its own.
	let (_, out, err) = res!(typst(&["fonts", "--ignore-system-fonts", "--font-path", &fonts.display().to_string()]));
	let theirs: BTreeSet<String> = out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
	assert!(!theirs.is_empty(), "typst lists fonts: {}", err);
	let embedded_typst	= ["DejaVu Sans Mono", "Libertinus Serif", "New Computer Modern", "New Computer Modern Math"];
	let added_typst: BTreeSet<&String> = theirs.iter().filter(|f| !embedded_typst.contains(&f.as_str())).collect();
	let added_ours: BTreeSet<&String> = listed.iter().filter(|f| !none.contains(f)).collect();
	assert_eq!(added_ours, added_typst, "the families the project's files add");

	// Every listed family is one a compile sets, and one that is not listed is refused.
	for family in &listed {
		let mut q = strict(&[("/main.typ", &fmt!("#set text(font: \"{}\")\nHello.\n", family))]);
		q.fonts = p.fonts.clone();
		let mut inst = Instance::new();
		let made = res!(must_pdf(inst.compile_pdf(&q)));
		assert!(made.report.diagnostics.is_empty(), "{} is listed, so it sets: {:?}", family, made.report.diagnostics);
	}
	let mut inst = Instance::new();
	let mut q = strict(&[("/main.typ", "#set text(font: \"Otherface\")\nHello.\n")]);
	q.fonts = p.fonts.clone();
	let f = res!(must_fail(inst.compile_pdf(&q)));
	assert_eq!(f.head.kind, DiagnosticKind::MissingFont);
	Ok(())
}

#[test]
fn the_font_book_adds_every_face_of_a_collection_and_a_fork_leaves_its_base_alone() -> Outcome<()> {
	let noto	= res!(font_file("NotoSans-Regular.ttf"));
	let nmono	= res!(font_file("NotoSansMono-Regular.ttf"));
	let ttc		= res!(collection_of(&[&noto, &nmono]));
	let base	= res!(FontBook::embedded());
	let held	= base.len();
	let mut fork = base.fork();
	assert_eq!(fork.len(), held, "a fork starts as its base");
	let first = res!(fork.add(ttc));
	assert_eq!(fork.len(), held + 2, "both faces of the collection");
	assert_eq!(first, held, "the id of the first face is returned");
	assert!(fork.has_family("noto sans") && fork.has_family("noto sans mono"));
	assert_eq!(base.len(), held, "the base is unchanged");
	assert!(!base.has_family("noto sans"), "and does not hold what its fork added");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INFO, THE VECTOR DOOR AND THE TYPST VERSION                                │
// └───────────────────────────────────────────────────────────────────────────┘

fn info_field(pdf: &Path, key: &str) -> Outcome<String> {
	let info = res!(tool("pdfinfo", &[], pdf, &[]));
	Ok(info.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string()).unwrap_or_default())
}

#[test]
fn a_tilde_in_the_title_and_author_gives_the_pdfinfo_text_typst_gives() -> Outcome<()> {
	let dir		= res!(work_dir("info"));
	let text	= "#set document(title: [Hello~world: it's ~ here], author: \"A~B\")\nBody.\n";
	let main	= res!(write_project(&dir, &[("main.typ", text)]));
	let theirs	= dir.join("typst.pdf");
	res!(pdf::typst_pdf(&main, &theirs));
	let mut inst = Instance::new();
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	let ours = dir.join("austenite.pdf");
	res!(std::fs::write(&ours, pdf_of(&made)));
	for key in ["Title:", "Author:"] {
		let (a, t) = (res!(info_field(&ours, key)), res!(info_field(&theirs, key)));
		assert!(!t.is_empty(), "typst writes {}", key);
		assert_eq!(a, t, "{} as pdfinfo reads it", key);
	}
	assert!(res!(info_field(&ours, "Title:")).contains('\u{a0}'), "the tilde is a non-breaking space");
	Ok(())
}

#[test]
fn the_vector_door_gives_one_svg_per_page_and_refuses_as_the_pdf_door_does() -> Outcome<()> {
	let dir		= res!(work_dir("vector"));
	// The final page count is read before it is known, so the document takes a second pass, whose pages are
	// the ones kept: a pass that was discarded must leave none behind.
	let text	= "#set page(height: 60pt, margin: 5pt)\nOne.\n#pagebreak()\nTwo.\n#pagebreak()\n\
		Three of #context counter(page).final().first().\n";
	let mut inst = Instance::new();
	let pdf = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	let svg = match inst.compile_svg(&strict(&[("/main.typ", text)])) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The vector door refused: {}", f.head; Test)),
	};
	assert_eq!(svg.product.len(), 3);
	assert_eq!(svg.report.pages, 3);
	assert_eq!(res!(pdf_pages(&pdf_of(&pdf), &dir, "vector")), 3, "the PDF door's count");
	for (i, s) in svg.product.iter().enumerate() {
		assert!(s.starts_with("<svg") || s.contains("<svg"), "page {} is an SVG document", i + 1);
	}
	let refused = match inst.compile_svg(&strict(&[("/main.typ", "")])) {
		Ok(_)	=> return Err(err!("The vector door compiled a main that sets no content."; Test)),
		Err(f)	=> f,
	};
	assert_eq!(site(&refused), "/main.typ:1:1");
	Ok(())
}

#[test]
fn engine_info_names_the_typst_version_the_evaluator_follows() -> Outcome<()> {
	let (ok, out, err) = res!(typst(&["--version"]));
	assert!(ok, "{}", err);
	let want = out.trim().trim_start_matches("typst ").split_whitespace().next().unwrap_or("").to_string();
	assert_eq!(package::version_text(package::TYPST_VERSION), want, "the version engineInfo().typst carries");
	Ok(())
}

#[test]
fn the_delta_door_keeps_the_curated_readers_changed_only_contract_until_it_moves() -> Outcome<()> {
	let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma";
	let mut text = String::new();
	for i in 0..70 {
		text.push_str(&fmt!("Paragraph {} {} {} {}.\n\n", i, words, words, words));
	}
	let mut inst = Instance::new();
	let mut p = project(&[("/main.typ", &text)]);
	let first = match inst.compile_delta(&p) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The delta door refused: {}", f.head; Test)),
	};
	let d = &first.product;
	assert!(d.reset && d.version == 1, "a first compile is a reset at version 1");
	assert!(d.order.len() >= 2 && first.report.pages == d.order.len(), "pages {} order {}", first.report.pages, d.order.len());
	assert!(!d.changed.is_empty() && d.changed.iter().all(|(_, svg)| svg.contains("<svg")));
	// The consumer holds every page, so the same document sends none.
	p.known = d.order.clone();
	let second = match inst.compile_delta(&p) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The delta door refused: {}", f.head; Test)),
	};
	assert!(!second.product.reset && second.product.changed.is_empty() && second.product.version == 2);
	assert_eq!(second.product.order, first.product.order);
	// A refusal (the reader fails a family no font declares outright) does not step the version.
	let mut bad = strict(&[("/main.typ", "#set text(font: \"Nope Sans\")\nx\n")]);
	bad.known = p.known.clone();
	let refused = match inst.compile_delta(&bad) {
		Ok(_)	=> return Err(err!("The delta door compiled what strict refuses."; Test)),
		Err(f)	=> f,
	};
	assert_eq!(refused.head.kind, DiagnosticKind::MissingFont, "{:?}", refused.head);
	let third = match inst.compile_delta(&p) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The delta door refused: {}", f.head; Test)),
	};
	assert_eq!(third.product.version, 3, "the refusal left the version where it was");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ queryProject                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

const BOOK: &str = "#set page(height: 140pt, margin: 10pt)\n\
	= First <a>\n\
	#context [#metadata(here().page()) <pp>]\n\
	#metadata((k: 1, s: \"x\", l: (1, 2.5, none, true))) <m1>\n\
	#figure(table(columns: 2)[a][b], caption: [Tbl]) <t1>\n\
	#pagebreak()\n\
	== Second <b>\n\
	#context [#metadata(here().page()) <pp>]\n\
	#figure(table[c], caption: [Tbl2]) <t2>\n\
	#pagebreak()\n\
	= Third\n\
	#context [#metadata(here().page()) <pp>]\n";

fn typst_query(main: &Path, selector: &str, field: Option<&str>) -> Outcome<J> {
	let mut args = vec!["query".to_string(), "--format".to_string(), "json".to_string()];
	if let Some(f) = field {
		args.push("--field".to_string());
		args.push(f.to_string());
	}
	args.push(main.display().to_string());
	args.push(selector.to_string());
	let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
	let (ok, out, err) = res!(typst(&refs));
	if !ok {
		return Err(err!("typst query {} failed: {}", selector, err; Test));
	}
	json::parse(&out)
}

fn arr(j: &J) -> Outcome<&Vec<J>> {
	match j {
		J::Arr(a)	=> Ok(a),
		other		=> Err(err!("Expected a JSON array, found {}", other.render(); Test)),
	}
}

#[test]
fn query_project_fields_equal_typst_query_and_pages_equal_a_here_page_probe() -> Outcome<()> {
	let dir		= res!(work_dir("query"));
	let main	= res!(write_project(&dir, &[("main.typ", BOOK)]));
	let mut inst = Instance::new();
	res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", BOOK)]))));

	// Every page a probe saw, in document order, one for each heading.
	let probes: Vec<i64> = res!(arr(&res!(typst_query(&main, "<pp>", Some("value"))))).iter().filter_map(|j| j.as_i64()).collect();
	assert_eq!(probes, [1, 2, 3], "the probe's own pages");

	for selector in ["heading", "<a>", "figure.where(kind: table)", "metadata"] {
		let theirs = res!(typst_query(&main, selector, None));
		let theirs = res!(arr(&theirs));
		let ours = match inst.query(selector, "") {
			Some(r)	=> r,
			None	=> return Err(err!("The door answered null for {}.", selector; Test)),
		};
		assert_eq!(ours.len(), theirs.len(), "{}: the same count", selector);
		for (o, t) in ours.iter().zip(theirs.iter()) {
			assert_eq!(Some(o.kind.as_str()), t.get("func").and_then(|f| f.as_str()), "{}: kind", selector);
			let label = t.get("label").and_then(|l| l.as_str()).map(|l| l.trim_start_matches('<').trim_end_matches('>').to_string());
			assert_eq!(o.label, label, "{}: label", selector);
			if o.kind == "heading" {
				assert_eq!(o.level, t.get("level").and_then(|l| l.as_i64()), "{}: level", selector);
				let body = t.get("body").map(|b| b.render()).unwrap_or_default();
				let title = o.title.clone().unwrap_or_default();
				assert!(body.contains(&fmt!("\"{}\"", title)), "{}: title {:?} is in typst's body {}", selector, title, body);
			}
		}
	}

	// `metadata` with `--field value`: the payloads.
	let want = res!(typst_query(&main, "<m1>", Some("value")));
	let want = res!(arr(&want));
	let got = match inst.query("<m1>", "") {
		Some(r)	=> r,
		None	=> return Err(err!("null for <m1>"; Test)),
	};
	assert_eq!(got.len(), 1);
	let text = got[0].value.clone().unwrap_or_default();
	assert_eq!(res!(json::parse(&text)).render(), want[0].render(), "the metadata value, as typst serialises it");

	// A heading's page is the page its probe stood on.
	let heads = match inst.query("heading", "") {
		Some(r)	=> r,
		None	=> return Err(err!("null for heading"; Test)),
	};
	let pages: Vec<i64> = heads.iter().filter_map(|h| h.page.map(|p| p as i64)).collect();
	assert_eq!(pages, probes, "the introspector's page equals typst's here().page()");

	// `field` selects one field.
	let levels = match inst.query("heading", "level") {
		Some(r)	=> r,
		None	=> return Err(err!("null for heading level"; Test)),
	};
	let levels: Vec<String> = levels.iter().map(|r| r.value.clone().unwrap_or_default()).collect();
	assert_eq!(levels, ["1", "2", "1"]);
	Ok(())
}

#[test]
fn a_query_answers_null_rather_than_a_wrong_row() -> Outcome<()> {
	let mut inst = Instance::new();
	assert!(inst.query("heading", "").is_none(), "nothing has compiled");
	res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", BOOK)]))));
	assert!(inst.query("nonesuch(", "").is_none(), "a selector that does not parse");
	assert!(inst.query("nonesuch", "").is_none(), "a selector that does not evaluate");
	assert!(inst.query("1 + 1", "").is_none(), "a value that is no selector");
	// A field typst writes as content, a length or a colour is no row at all.
	assert!(inst.query("figure", "caption").is_none(), "content is not written as a plain value");
	assert!(inst.query("heading", "body").is_none(), "nor is a heading's body");
	Ok(())
}

#[test]
fn a_failed_compile_clears_the_kept_introspector() -> Outcome<()> {
	let mut inst = Instance::new();
	res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", BOOK)]))));
	assert_eq!(inst.query("heading", "").map(|r| r.len()), Some(3));
	// A hard error.
	res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", "= Gone\n#import \"nowhere.typ\": x\n")]))));
	assert!(inst.query("heading", "").is_none(), "the rows of before a failed compile are gone");
	res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", BOOK)]))));
	assert_eq!(inst.query("heading", "").map(|r| r.len()), Some(3));
	// A strict refusal.
	res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", "= Gone\n#set text(font: \"Nope Sans\")\nx\n")]))));
	assert!(inst.query("heading", "").is_none(), "a refused compile answers no query either");
	Ok(())
}

#[test]
fn heading_rows_keep_title_a_plain_string_and_page_a_one_based_number_across_pages() -> Outcome<()> {
	let text = "#set page(height: 80pt, margin: 8pt)\n\
		= Intro\nWords here.\n#pagebreak()\n\
		== Methods and *bold* parts\nMore words.\n#pagebreak()\n\
		=== Deep\nStill more.\n#pagebreak()\n\
		= Close\nEnd.\n";
	let mut inst = Instance::new();
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	assert_eq!(made.report.pages, 4);
	let rows = match inst.query("heading", "") {
		Some(r)	=> r,
		None	=> return Err(err!("The rail's query answered null."; Test)),
	};
	let got: Vec<(String, u32, i64, String)> = rows.iter()
		.map(|r| (r.title.clone().unwrap_or_default(), r.page.unwrap_or(0), r.level.unwrap_or(0), r.kind.clone()))
		.collect();
	let want = [
		("Intro".to_string(),						1u32, 1i64, "heading".to_string()),
		("Methods and bold parts".to_string(),		2, 2, "heading".to_string()),
		("Deep".to_string(),						3, 3, "heading".to_string()),
		("Close".to_string(),						4, 1, "heading".to_string()),
	];
	assert_eq!(got, want, "the rail's rows: a plain title, a 1-based page");
	assert!(rows.iter().all(|r| r.page.map_or(false, |p| p >= 1)), "no page is 0 or missing");
	Ok(())
}
