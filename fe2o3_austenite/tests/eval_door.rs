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
use oxedyne_fe2o3_austenite::delta::{
	Changed,
	PageDelta,
};
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

/// Typst's own run of `text` as one project: whether it compiled clean, and its pages.
fn typst_clean_pages(dir: &Path, name: &str, text: &str) -> Outcome<(bool, usize)> {
	let case	= dir.join(name);
	let main	= res!(write_project(&case, &[("main.typ", text)]));
	let pdf		= main.with_extension("pdf");
	let (ok, _, stderr) = res!(typst(&["compile", "--diagnostic-format", "short",
		&main.display().to_string(), &pdf.display().to_string()]));
	let pages = if ok { res!(pdf_pages(&res!(std::fs::read(&pdf)), &case, "typst")) } else { 0 };
	Ok((ok && stderr.trim().is_empty(), pages))
}

// A main sets content when its body, once its contexts are resolved and its show rules applied, holds an
// element other than a space, a paragraph break, a page break or a column break. What it realises to is
// judged, never what it evaluated to: a `context` that gives nothing is an element until it is resolved, and
// the furniture of a `set page` is laid out beside the body and is no part of it.
const BLANK: [(&str, &str); 14] = [
	("context-none",		"#context none\n"),
	("context-empty",		"#context []\n"),
	("context-of-context",	"#context context none\n"),
	("context-space",		"#context [ ]\n"),
	("header-only",			"#set page(header: [x])\n"),
	("footer-and-folio",	"#set page(footer: [y], numbering: \"1\")\n"),
	("background-and-fill",	"#set page(background: [bg], fill: red)\n"),
	("pagebreak",			"#pagebreak()\n"),
	("metadata",			"#metadata(1) <m>\n"),
	("show-all",			"#show: it => it\n"),
	("styled-nothing",		"#align(center)[]\n"),
	("gone-by-the-last-pass", "#context if query(<a>).len() == 0 [first pass only]\n#metadata(1) <a>\n"),
	("set-and-let",			"#set text(size: 11pt)\n#let x = 1\n"),
	("spaces",				"  \n\n  "),
];

const CONTENT: [(&str, &str); 9] = [
	("vertical-space",		"#v(1cm)\n"),
	("horizontal-space",	"#h(1cm)\n"),
	("filled-box",			"#box(fill: red, width: 1cm, height: 1cm)\n"),
	("empty-box",			"#box()\n"),
	("context-space",		"#context v(1cm)\n"),
	("context-text",		"#context [hello]\n"),
	("furniture-and-body",	"#set page(header: [x])\nBody.\n"),
	("styled-text",			"#text(red)[x]\n"),
	("there-by-the-last-pass", "#context if query(<a>).len() > 0 [there]\n#metadata(1) <a>\n"),
];

#[test]
fn a_main_whose_body_realises_to_nothing_is_refused_on_every_door_though_typst_raises_nothing() -> Outcome<()> {
	let dir = res!(work_dir("blank-body"));
	let mut inst = Instance::new();
	for (name, text) in BLANK {
		// Typst lays each out as one blank page, or more for a page break, and says nothing.
		let (clean, pages) = res!(typst_clean_pages(&dir, name, text));
		assert!(clean && pages >= 1, "typst compiles {} clean", name);
		let p = strict(&[("/main.typ", text)]);
		let pdf		= res!(must_fail(inst.compile_pdf(&p)));
		let svg		= match inst.compile_svg(&p) {
			Ok(_)	=> return Err(err!("The vector door compiled {}, which sets no content.", name; Test)),
			Err(f)	=> f,
		};
		let delta	= res!(delta_fail(&mut inst, &p));
		for (door, f) in [("pdf", &pdf), ("svg", &svg), ("delta", &delta)] {
			assert_eq!(site(f), "/main.typ:1:1", "{} on the {} door: {:?}", name, door, f.head);
			assert_eq!(f.head.kind, DiagnosticKind::Internal, "{} on the {} door: {:?}", name, door, f.head);
			assert!(f.head.message.contains("sets no content"), "{} on the {} door: {}", name, door, f.head.message);
		}
		// Not strict, the page is made.
		let made = res!(must_pdf(inst.compile_pdf(&project(&[("/main.typ", text)]))));
		assert!(made.report.pages >= 1, "{}", name);
	}
	Ok(())
}

#[test]
fn a_main_with_any_element_in_its_body_passes_strict_though_it_sets_an_empty_looking_page() -> Outcome<()> {
	let dir = res!(work_dir("content-body"));
	let mut inst = Instance::new();
	for (name, text) in CONTENT {
		let (clean, pages) = res!(typst_clean_pages(&dir, name, text));
		assert!(clean && pages == 1, "typst compiles {} clean to one page", name);
		let p = strict(&[("/main.typ", text)]);
		let pdf = res!(must_pdf(inst.compile_pdf(&p)));
		assert_eq!(pdf.report.pages, 1, "{}", name);
		match inst.compile_svg(&p) {
			Ok(m)	=> assert_eq!(m.product.len(), 1, "{}", name),
			Err(f)	=> return Err(err!("The vector door refused {}: {}", name, f.head; Test)),
		}
		let delta = res!(delta_made(&mut inst, &p));
		assert_eq!(delta.product.order.len(), 1, "{}", name);
	}
	// What the body is in the pass that stood decides it: the late content is there, as typst's page says.
	let late = CONTENT[CONTENT.len() - 1].1;
	let case = dir.join("late-text");
	let main = res!(write_project(&case, &[("main.typ", late)]));
	let pdf = main.with_extension("pdf");
	let (ok, _, stderr) = res!(typst(&["compile", &main.display().to_string(), &pdf.display().to_string()]));
	assert!(ok, "{}", stderr);
	assert_eq!(res!(pdf::text_of(&pdf)), "there");
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

	// The evaluator's own walking skeleton, a show rule and all, passes strict as one page.
	let fixture = res!(std::fs::read_to_string(
		Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/skeleton/skeleton.typ")));
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", &fixture)]))));
	assert!(made.report.diagnostics.is_empty() && made.report.pages == 1, "{:?}", made.report.diagnostics);
	assert_eq!(res!(pdf_pages(&pdf_of(&made), &dir, "fixture")), 1);

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
	let text = "Words.\n\
		#let s = state(\"s\", 0)\n\
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
		assert!(s.contains("<svg"), "page {} is an SVG document", i + 1);
		assert!(s.contains("class=\"tsel\""), "page {} carries its selectable text layer", i + 1);
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DELTA DOOR                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

const NAMES: [&str; 6] = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];

/// Six pages of one short paragraph each; `edit` sets one letter of the word in that page's paragraph.
fn six_pages(edit: Option<usize>) -> String {
	let mut t = String::from("#set page(width: 220pt, height: 100pt, margin: 12pt)\n");
	for (k, name) in NAMES.iter().enumerate() {
		if k > 0 {
			t.push_str("#pagebreak()\n");
		}
		let word = if edit == Some(k) { "quarts" } else { "quartz" };
		t.push_str(&fmt!("Page {} holds the {} of paragraph {} alone.\n", name, word, k));
	}
	t
}

fn delta_made(inst: &mut Instance, p: &Project) -> Outcome<Made<PageDelta>> {
	match inst.compile_delta(p) {
		Ok(m)	=> Ok(m),
		Err(f)	=> Err(err!("The delta door refused: {} {:?}", f.head, f.head.kind; Test)),
	}
}

fn delta_fail(inst: &mut Instance, p: &Project) -> Outcome<Failure> {
	match inst.compile_delta(p) {
		Ok(_)	=> Err(err!("The delta door produced a delta where it should have refused."; Test)),
		Err(f)	=> Ok(f),
	}
}

/// The text a page's SVG offers for selection, with its whitespace removed: the characters of its `.tsel` layer.
fn text_layer(svg: &str) -> String {
	let mut out = String::new();
	let mut rest = match svg.find("<text class=\"tsel\">") {
		Some(i)	=> &svg[i..],
		None	=> return out,
	};
	while let Some(at) = rest.find("<tspan") {
		let span	= &rest[at..];
		let open	= match span.find('>') { Some(i) => i + 1, None => break };
		let close	= match span.find("</tspan>") { Some(i) => i, None => break };
		out.push_str(&span[open..close]);
		rest = &span[close + "</tspan>".len()..];
	}
	out.split_whitespace().collect()
}

/// Typst's PDF of `text` and each page's text, whitespace removed: the external fact the delta is held to.
fn typst_page_texts(dir: &Path, name: &str, text: &str) -> Outcome<Vec<String>> {
	let case	= dir.join(name);
	let main	= res!(write_project(&case, &[("main.typ", text)]));
	let pdf		= main.with_extension("pdf");
	let (ok, _, stderr) = res!(typst(&["compile", &main.display().to_string(), &pdf.display().to_string()]));
	assert!(ok, "typst compiles {}: {}", name, stderr);
	let pages = res!(pdf_pages(&res!(std::fs::read(&pdf)), &case, "pages"));
	let mut out = Vec::with_capacity(pages);
	for n in 1..=pages {
		let page = n.to_string();
		let t = res!(tool("pdftotext", &["-f", &page, "-l", &page], &pdf, &["-"]));
		out.push(t.split_whitespace().collect::<String>());
	}
	Ok(out)
}

#[test]
fn the_delta_door_sends_every_page_once_then_nothing_while_the_consumer_holds_them() -> Outcome<()> {
	let mut inst = Instance::new();
	let mut p = strict(&[("/main.typ", &six_pages(None))]);
	let first = res!(delta_made(&mut inst, &p));
	let d = &first.product;
	assert!(d.reset && d.version == 1, "a first compile is a reset at version 1");
	assert_eq!(first.report.pages, 6);
	assert_eq!(d.order.len(), first.report.pages, "order names every page");
	assert_eq!(d.changed.len(), 6, "a reset carries every page");
	let ids: BTreeSet<u64> = d.order.iter().copied().collect();
	assert_eq!(ids.len(), 6, "six different pages, six different ids");
	for (id, svg) in &d.changed {
		assert!(ids.contains(id) && svg.contains("<svg") && svg.contains("class=\"tsel\""), "{}", id);
	}
	// The consumer holds every page, so the same document sends none, and the version moves on.
	p.known = d.order.clone();
	let second = res!(delta_made(&mut inst, &p));
	assert!(!second.product.reset && second.product.changed.is_empty() && second.product.version == 2);
	assert_eq!(second.product.order, first.product.order);
	// A cleared cache is a reset: a resend over a blank.
	p.known = Vec::new();
	let third = res!(delta_made(&mut inst, &p));
	assert!(third.product.reset && third.product.changed.len() == 6 && third.product.version == 3);
	Ok(())
}

#[test]
fn a_one_letter_edit_sends_exactly_the_pages_whose_text_typst_says_changed() -> Outcome<()> {
	let dir = res!(work_dir("delta-edit"));
	let was = res!(typst_page_texts(&dir, "was", &six_pages(None)));
	assert_eq!(was.len(), 6);
	let mut inst = Instance::new();
	let mut p = strict(&[("/main.typ", &six_pages(None))]);
	let first = res!(delta_made(&mut inst, &p));
	for (n, k) in [0usize, 2, 5].iter().enumerate() {
		let text = six_pages(Some(*k));
		let now = res!(typst_page_texts(&dir, &fmt!("now{}", n), &text));
		let differ: Vec<usize> = (0..6).filter(|i| was[*i] != now[*i]).collect();
		assert_eq!(differ, vec![*k], "typst changes the text of page {} alone", k + 1);

		p.known = first.product.order.clone();
		p.sources = vec![("/main.typ".to_string(), text)];
		let after = res!(delta_made(&mut inst, &p));
		let d = &after.product;
		assert!(!d.reset, "the consumer holds pages");
		assert_eq!(d.order.len(), 6);
		let want: BTreeSet<u64> = differ.iter().map(|i| d.order[*i]).collect();
		let got: BTreeSet<u64> = d.changed.iter().map(|(id, _)| *id).collect();
		assert_eq!(got, want, "changed holds the pages typst's text says changed (edit in page {})", k + 1);
		for i in 0..6 {
			if differ.contains(&i) {
				assert_ne!(d.order[i], first.product.order[i], "page {} has a new id", i + 1);
			} else {
				assert_eq!(d.order[i], first.product.order[i], "page {} keeps its id", i + 1);
			}
		}
		// What is sent is the edited page, not an old copy of it.
		match d.changed.first() {
			Some((_, svg))	=> assert!(text_layer(svg).contains("quarts"), "the page sent carries the edit"),
			None			=> return Err(err!("The edit sent no page."; Test)),
		}
	}
	Ok(())
}

#[test]
fn a_page_of_a_discarded_pass_never_reaches_the_delta() -> Outcome<()> {
	// The final page count is read before it is known, so the document takes a second pass and the pages of
	// the first are discarded: page three reads its total, and says another thing the first time.
	let text = "#set page(width: 220pt, height: 60pt, margin: 8pt)\nOne.\n#pagebreak()\nTwo.\n#pagebreak()\n\
		Three of #context counter(page).final().first().\n";
	let dir = res!(work_dir("delta-passes"));
	let theirs = res!(typst_page_texts(&dir, "typst", text));
	assert_eq!(theirs, vec!["One.", "Two.", "Threeof3."], "typst's pages");
	let mut inst = Instance::new();
	let p = strict(&[("/main.typ", text)]);
	let made = res!(delta_made(&mut inst, &p));
	let d = &made.product;
	assert_eq!(made.report.pages, 3);
	assert_eq!(d.order.len(), 3, "the order is the pages of the pass that stood");
	let ids: BTreeSet<u64> = d.order.iter().copied().collect();
	assert_eq!(d.changed.len(), ids.len(), "each id once");
	let vector = match inst.compile_svg(&p) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The vector door refused: {}", f.head; Test)),
	};
	for (i, id) in d.order.iter().enumerate() {
		let sent = d.changed.iter().find(|(c, _)| c == id).map(|(_, svg)| svg);
		match sent {
			Some(svg)	=> {
				assert_eq!(svg, &vector.product[i], "page {} is the page the final pass drew", i + 1);
				assert_eq!(text_layer(svg), theirs[i], "and carries the text typst's page does");
			},
			None		=> return Err(err!("Page {} of the order was not sent on a reset.", i + 1; Test)),
		}
	}
	Ok(())
}

// A document that settles in two passes: page three reads the page total, which the first pass has not seen.
const TWO_PASSES: &str = "#set page(width: 220pt, height: 60pt, margin: 8pt)\nOne.\n#pagebreak()\nTwo.\n#pagebreak()\n\
	Three of #context counter(page).final().first().\n";
const ONE_PASS: &str = "#set page(width: 220pt, height: 60pt, margin: 8pt)\nOne.\n#pagebreak()\nTwo.\n#pagebreak()\n\
	Three of 3.\n";

#[test]
fn an_open_draws_the_pages_of_the_pass_that_stands_and_no_others() -> Outcome<()> {
	// The first pass of TWO_PASSES is discarded, and its three pages are not drawn: the sink counts each SVG it
	// draws, and the count is the pages sent, not the pages of both passes.
	let mut inst = Instance::new();
	let two = res!(delta_made(&mut inst, &strict(&[("/main.typ", TWO_PASSES)])));
	assert_eq!(two.report.pages, 3);
	assert_eq!(two.product.changed.len(), 3);
	assert_eq!(two.product.rendered, 3, "three pages drawn, none of the discarded pass's");
	// A document that settles in one pass draws the same.
	let one = res!(delta_made(&mut inst, &strict(&[("/main.typ", ONE_PASS)])));
	assert_eq!(one.product.rendered, 3);
	// An edit to page one, the consumer holding the rest: one page is drawn, though the edited document also
	// takes two passes and its first pass holds a page three that the consumer does not.
	let mut p = strict(&[("/main.typ", TWO_PASSES)]);
	p.known = two.product.order.clone();
	let same = res!(delta_made(&mut inst, &p));
	assert_eq!((same.product.changed.len(), same.product.rendered), (0, 0), "an unchanged recompile draws nothing");
	let edited = TWO_PASSES.replacen("One.", "Uno.", 1);
	let mut q = strict(&[("/main.typ", &edited)]);
	q.known = two.product.order.clone();
	let made = res!(delta_made(&mut inst, &q));
	assert_eq!(made.product.order.len(), 3);
	assert_eq!((made.product.changed.len(), made.product.rendered), (1, 1), "only the edited page is drawn");
	Ok(())
}

/// A consumer of changed pages that keeps the ids in the order they arrive and the bytes it was handed.
#[derive(Default)]
struct Tape {
	ids:	Vec<u64>,
	bytes:	usize,
}

impl Changed for Tape {
	fn take(&mut self, id: u64, svg: String) -> Outcome<()> {
		assert!(svg.contains("class=\"tsel\""), "page {} carries its text layer", id);
		self.ids.push(id);
		self.bytes += svg.len();
		Ok(())
	}
}

#[test]
fn each_changed_page_goes_to_the_consumer_in_reading_order_as_it_is_drawn() -> Outcome<()> {
	let mut inst = Instance::new();
	let p = strict(&[("/main.typ", &six_pages(None))]);
	let whole = res!(delta_made(&mut inst, &p));
	let sent = match inst.compile_delta_into(&p, Tape::default()) {
		Ok(m)	=> m,
		Err(f)	=> return Err(err!("The delta door refused: {}", f.head; Test)),
	};
	let (head, tape) = sent.product;
	assert_eq!(head.order, whole.product.order, "the order is the same through either entry");
	assert_eq!(tape.ids, head.order, "six different pages arrive in reading order");
	assert_eq!(head.rendered as usize, tape.ids.len());
	let held: usize = whole.product.changed.iter().map(|(_, svg)| svg.len()).sum();
	assert_eq!(tape.bytes, held, "the bytes handed over are the bytes the whole delta holds");
	assert_eq!(head.version, whole.product.version + 1, "each compile steps the tick, by either entry");
	Ok(())
}

#[test]
fn a_refusal_leaves_the_delta_version_where_it_was() -> Outcome<()> {
	let mut inst = Instance::new();
	let mut p = strict(&[("/main.typ", &six_pages(None))]);
	let one = res!(delta_made(&mut inst, &p));
	p.known = one.product.order.clone();
	assert_eq!(one.product.version, 1);
	assert_eq!(res!(delta_made(&mut inst, &p)).product.version, 2);
	let mut bad = Vec::new();
	// A strict refusal of a warning, one of a body that sets nothing, and a hard error.
	bad.push(strict(&[("/main.typ", "#set text(font: \"Nope Sans\")\nx\n")]));
	bad.push(strict(&[("/main.typ", "#context none\n")]));
	bad.push(strict(&[("/main.typ", "#nonesuch()\n")]));
	let kinds = [DiagnosticKind::MissingFont, DiagnosticKind::Internal, DiagnosticKind::UnknownVariable];
	for (b, kind) in bad.iter_mut().zip(kinds) {
		b.known = p.known.clone();
		let f = res!(delta_fail(&mut inst, b));
		assert_eq!(f.head.kind, kind, "{:?}", f.head);
	}
	assert_eq!(res!(delta_made(&mut inst, &p)).product.version, 3, "the three refusals left the version where it was");
	// Not strict, the same missing font is a delta, and it steps the tick.
	let mut soft = project(&[("/main.typ", "#set text(font: \"Nope Sans\")\nx\n")]);
	soft.known = Vec::new();
	let made = res!(delta_made(&mut inst, &soft));
	assert_eq!(made.product.version, 4);
	assert!(made.report.diagnostics.iter().any(|d| d.kind == DiagnosticKind::MissingFont), "{:?}", made.report.diagnostics);
	Ok(())
}

#[test]
fn the_delta_result_carries_pages_diagnostics_skipped_and_needs_as_the_other_doors_do() -> Outcome<()> {
	let mut inst = Instance::new();
	let gradient = "#rect(width: 2cm, height: 1cm, fill: gradient.linear(red, blue))\nBody.\n";
	// Not strict, a construct passed over stands beside the delta, and the line names it.
	let made = res!(delta_made(&mut inst, &project(&[("/main.typ", gradient)])));
	assert_eq!(made.report.pages, 1);
	assert!(made.report.skipped.as_deref().unwrap_or("").contains("gradients"), "{:?}", made.report.skipped);
	assert!(made.report.diagnostics.iter().any(|d| d.kind == DiagnosticKind::Unsupported), "{:?}", made.report.diagnostics);
	assert!(made.needs.is_empty());
	// Strict, the same document is refused as the PDF door refuses it, its sites and skip line with it.
	let f = res!(delta_fail(&mut inst, &strict(&[("/main.typ", gradient)])));
	let g = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", gradient)]))));
	assert_eq!(f.head, g.head);
	assert_eq!(f.rest, g.rest);
	assert_eq!(f.skipped, g.skipped);
	assert_eq!(f.pages, Some(1));
	// An unsupplied package is named in `needs`.
	let f = res!(delta_fail(&mut inst, &project(&[("/main.typ", "#import \"@preview/zzz-absent:9.9.9\": x\n")])));
	assert_eq!(f.head.kind, DiagnosticKind::Package, "{:?}", f.head);
	assert_eq!(f.needs, vec!["@preview/zzz-absent:9.9.9".to_string()]);
	Ok(())
}

#[test]
fn a_page_that_differs_only_in_its_face_has_another_id() -> Outcome<()> {
	// Monospaced faces share their advances and, in a family, their glyph ids: the same word in the regular, the
	// bold and the oblique face is placed alike, and the page is a different one. Typst says so too.
	let dir = res!(work_dir("delta-faces"));
	let regular = "#set page(width: 220pt, height: 60pt, margin: 8pt)\n#raw(\"hello world\")\n";
	let bold	= "#set page(width: 220pt, height: 60pt, margin: 8pt)\n#strong(raw(\"hello world\"))\n";
	let oblique	= "#set page(width: 220pt, height: 60pt, margin: 8pt)\n#emph(raw(\"hello world\"))\n";
	let mut fonts: Vec<String> = Vec::new();
	for (name, text) in [("regular", regular), ("bold", bold), ("oblique", oblique)] {
		let case	= dir.join(name);
		let main	= res!(write_project(&case, &[("main.typ", text)]));
		let pdf		= main.with_extension("pdf");
		let (ok, _, stderr) = res!(typst(&["compile", &main.display().to_string(), &pdf.display().to_string()]));
		assert!(ok, "{}", stderr);
		fonts.push(res!(tool("pdffonts", &[], &pdf, &[])));
	}
	assert!(fonts[0].contains("DejaVuSansMono") && !fonts[0].contains("Bold") && !fonts[0].contains("Oblique"), "{}", fonts[0]);
	assert!(fonts[1].contains("DejaVuSansMono-Bold"), "typst sets the bold face: {}", fonts[1]);
	assert!(fonts[2].contains("Oblique"), "typst sets the oblique face: {}", fonts[2]);

	let mut inst = Instance::new();
	let mut p = strict(&[("/main.typ", regular)]);
	let was = res!(delta_made(&mut inst, &p));
	p.known = was.product.order.clone();
	for (name, text) in [("bold", bold), ("oblique", oblique)] {
		p.sources = vec![("/main.typ".to_string(), text.to_string())];
		let now = res!(delta_made(&mut inst, &p));
		assert_ne!(now.product.order, was.product.order, "the {} page is not the regular one", name);
		assert_eq!(now.product.changed.len(), 1, "and is sent: {}", name);
		assert_ne!(now.product.changed[0].1, was.product.changed[0].1, "and is drawn otherwise: {}", name);
	}
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

/// Runs `body` on a thread whose stack holds the deepest nest the door allows. A debug build's frames are many
/// times a release build's, and a test thread has two mebibytes.
fn deep<F: FnOnce() -> Outcome<()> + Send + 'static>(body: F) -> Outcome<()> {
	let t = res!(std::thread::Builder::new().stack_size(128 << 20).spawn(body));
	match t.join() {
		Ok(r)	=> r,
		Err(_)	=> Err(err!("The nest test panicked on its own stack."; Test)),
	}
}

// A nest of containers: `kind` n deep around one word, or a list n deep.
fn nest(kind: &str, n: usize) -> String {
	let (open, close) = match kind {
		"box"		=> ("#box[", "]"),
		"block"		=> ("#block[", "]"),
		"stack"		=> ("#stack[", "]"),
		_			=> return (0..n).map(|i| fmt!("{}- item\n", "  ".repeat(i))).collect(),
	};
	fmt!("{}x{}\n", open.repeat(n), close.repeat(n))
}

// As [`nest`], built by a loop at evaluation, so the parser's own depth limit does not end it first.
fn looped(kind: &str, n: usize) -> String {
	match kind {
		"list"	=> nest(kind, n),
		_		=> fmt!("#let x = [a]\n#for i in range({}) {{ x = {}(x) }}\n#x\n", n, kind),
	}
}

/// Typst's first error message and its short-format site for `text`, or `None` where typst compiles it.
fn typst_refusal(dir: &Path, name: &str, text: &str) -> Outcome<Option<(String, usize, usize)>> {
	let case	= dir.join(name);
	let main	= res!(write_project(&case, &[("main.typ", text)]));
	let pdf		= main.with_extension("pdf");
	let (ok, _, stderr) = res!(typst(&["compile", "--diagnostic-format", "short",
		&main.display().to_string(), &pdf.display().to_string()]));
	if ok {
		return Ok(None);
	}
	let (line, col) = res!(typst_site(&main, "error"));
	let message = stderr.lines()
		.find_map(|l| l.split_once(": error: ").map(|(_, m)| m.trim().to_string()))
		.unwrap_or_default();
	Ok(Some((message, line, col)))
}

// Typst refuses a container nested past its depth, `maximum layout depth exceeded` for boxes and blocks,
// `maximum show rule depth exceeded` for a stack and a list, and each kind costs its own number of levels. The
// door sets the nest typst sets one level under and refuses the one a level over, at typst's site, as a `limit`.
#[test]
fn a_nest_of_containers_is_set_one_level_under_typsts_depth_and_refused_one_level_over() -> Outcome<()> {
	deep(|| {
	let dir = res!(work_dir("depth-kinds"));
	let mut inst = Instance::new();
	for (kind, under) in [("box", 70usize), ("block", 35), ("stack", 32), ("list", 31)] {
		let set = nest(kind, under);
		assert!(res!(typst_refusal(&dir, &fmt!("{}-under", kind), &set)).is_none(), "typst must set {} {} deep", kind, under);
		res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", set.as_str())]))));
		let over = nest(kind, under + 1);
		let (message, line, col) = match res!(typst_refusal(&dir, &fmt!("{}-over", kind), &over)) {
			Some(r)	=> r,
			None	=> return Err(err!("Typst must refuse {} {} deep.", kind, under + 1; Test)),
		};
		let f = res!(must_fail(inst.compile_pdf(&strict(&[("/main.typ", over.as_str())]))));
		assert_eq!(f.head.message, message, "{} {} deep: the message", kind, under + 1);
		assert_eq!(f.head.kind, DiagnosticKind::Limit, "{} {} deep: the kind", kind, under + 1);
		assert_eq!((f.head.line, f.head.col), (line, door_col(&over, line, col)), "{} {} deep: the site", kind, under + 1);
	}
	Ok(())
	})
}

// Without the guard these nests overflow the stack: a trap in the browser, which ends the module for the page's
// life, and an abort here. With it each is a `limit` error, and the instance and a new one compile on.
#[test]
fn a_nest_two_thousand_deep_is_a_limit_error_and_the_next_compile_succeeds_on_the_same_instance_and_a_new_one() -> Outcome<()> {
	deep(|| {
	let mut inst = Instance::new();
	for kind in ["box", "block", "stack", "table", "list"] {
		let n = if kind == "list" { 100 } else { 2_000 };
		let text = looped(kind, n);
		let f = match inst.compile_pdf(&strict(&[("/main.typ", text.as_str())])) {
			Ok(_)	=> return Err(err!("The door compiled {} {} deep; it must refuse it as a limit.", kind, n; Test)),
			Err(f)	=> f,
		};
		assert_eq!(f.head.kind, DiagnosticKind::Limit, "{} {} deep: {}", kind, n, f.head);
		res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", SKELETON)]))));
		res!(must_pdf(Instance::new().compile_pdf(&strict(&[("/main.typ", SKELETON)]))));
	}
	Ok(())
	})
}

// The contract places an error with no site at 0:0 in the main, and a warning is no different: its file is
// never empty.
#[test]
fn a_warning_with_no_site_is_reported_at_zero_zero_in_the_main() -> Outcome<()> {
	let text = "#context [#counter(\"c\").update(counter(\"c\").final().first() + 1)]\nx\n";
	let mut inst = Instance::new();
	let made = res!(must_pdf(inst.compile_pdf(&strict(&[("/main.typ", text)]))));
	let detached: Vec<_> = made.report.diagnostics.iter().filter(|d| d.line == 0).collect();
	assert!(!detached.is_empty(), "the probe must raise a warning with no site: {:?}", made.report.diagnostics);
	for d in &made.report.diagnostics {
		assert_eq!(d.file, "/main.typ", "a diagnostic's file is never empty: {:?}", d);
	}
	for d in detached {
		assert_eq!((d.line, d.col), (0, 0), "{:?}", d);
	}
	Ok(())
}
