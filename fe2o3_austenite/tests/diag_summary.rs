//! `austenite --eval --diag-summary`: one stderr line for each severity, kind and construct, counts only.
//!
//! The report names what a compile passed over and never the document it passed over in, so it can be read
//! where the document may not be. Each case compiles a synthetic file with the built binary and holds the
//! lines to their exact text.

use oxedyne_fe2o3_austenite::diag::{
	error_lines,
	summary_lines,
	Diagnostic,
	DiagnosticKind,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Source,
	Span,
};

use std::path::PathBuf;
use std::process::Command;

// One file's compile: the exit status's success, the report's lines and the error lines.
struct Run {
	ok:		bool,
	lines:	Vec<String>,
	errors:	Vec<String>,
	stderr:	String,
}

fn run(name: &str, src: &str, flags: &[&str]) -> Run {
	run_with(name, src, &[], flags)
}

// As `run`, with the extra files (path below the case's directory, text) written beside `main.typ`.
fn run_with(name: &str, src: &str, files: &[(&str, &str)], flags: &[&str]) -> Run {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diag_summary").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	let main = dir.join("main.typ");
	std::fs::write(&main, src).expect("the case's source");
	for (path, text) in files {
		let at = dir.join(path);
		std::fs::create_dir_all(at.parent().expect("a parent")).expect("the file's directory");
		std::fs::write(&at, text).expect("the case's file");
	}
	let out = Command::new(env!("CARGO_BIN_EXE_austenite"))
		.args(flags)
		.arg(&main)
		.arg(dir.join("out"))
		.output()
		.expect("the built austenite binary");
	let stderr = String::from_utf8_lossy(&out.stderr).to_string();
	let lines = stderr.lines()
		.filter(|l| l.starts_with("diag-summary "))
		.map(|l| l.to_string())
		.collect();
	let errors = stderr.lines()
		.filter(|l| l.starts_with("diag-error "))
		.map(|l| l.to_string())
		.collect();
	Run { ok: out.status.success(), lines, errors, stderr }
}

// A page bleed is passed over and a gradient drawn flat, both of kind `unsupported` (the second message
// has a colon, so its head is not the whole message); a label that no element carries is Typst's own
// remark, of kind `lint`.
const TWO_UNSUPPORTED_ONE_LINT: &str = "\
<nothing>
#set page(bleed: 3pt)
#rect(fill: gradient.linear(red, blue))
";

#[test]
fn two_unsupported_constructs_and_a_lint_are_counted_and_nothing_else_is_printed() {
	let r = run("counts", TWO_UNSUPPORTED_ONE_LINT, &["--eval", "--diag-summary"]);
	assert!(r.ok, "the compile succeeds:\n{}", r.stderr);
	assert_eq!(r.lines, vec![
		"diag-summary warning lint - 1",
		"diag-summary warning unsupported gradients_are_drawn_in_one_flat_colour 1",
		"diag-summary warning unsupported page_bleed_is_not_drawn 1",
	], "the summary of:\n{}", r.stderr);
	// Nothing a message, a path or a position would add: the summary lines and the terse skip line only.
	for l in r.stderr.lines() {
		assert!(l.starts_with("diag-summary ") || l.starts_with("[austenite] skipped: "),
			"a line that is neither the summary nor the skip line: {:?}", l);
	}
	assert!(r.errors.is_empty(), "warnings are not named as errors:\n{}", r.stderr);
}

#[test]
fn no_report_is_printed_unless_the_flag_is_given() {
	let r = run("off", TWO_UNSUPPORTED_ONE_LINT, &["--eval"]);
	assert!(r.ok, "the compile succeeds:\n{}", r.stderr);
	assert!(r.lines.is_empty(), "no report is printed unasked:\n{}", r.stderr);
	let r = run("off_error", "#text(size: \"a\")[x]\n", &["--eval"]);
	assert!(!r.ok, "the compile fails");
	assert!(r.errors.is_empty(), "no error is named unasked:\n{}", r.stderr);
}

#[test]
fn a_compile_that_fails_prints_the_report_before_the_error() {
	// A show rule's error is the final pass's own, so the compile fails after the layout that raised the
	// bleed's warning.
	let src = "\
#set page(bleed: 3pt)
#show heading: it => it.nonesuch
= Title
";
	let r = run("fails", src, &["--eval", "--diag-summary"]);
	assert!(!r.ok, "the compile fails:\n{}", r.stderr);
	let at = |needle: &str| r.stderr.lines().position(|l| l.contains(needle));
	assert_eq!(r.lines, vec![
		"diag-summary error type - 1",
		"diag-summary warning unsupported page_bleed_is_not_drawn 1",
	], "the summary of:\n{}", r.stderr);
	let summary	= at("diag-summary ").expect("a summary line");
	let error	= at(": error: ").expect("the rendered error");
	assert!(summary < error, "the summary comes before the error:\n{}", r.stderr);
}

#[test]
fn the_flag_without_eval_is_refused() {
	let r = run("no_eval", "Hello.\n", &["--diag-summary"]);
	assert!(!r.ok, "the flag alone is refused");
	assert!(r.lines.is_empty(), "no report is printed:\n{}", r.stderr);
}

#[test]
fn a_construct_is_the_message_head_with_its_spaces_written_as_underscores() {
	let at = Span::detached();
	let w = |kind, msg: &str| Diagnostic::warning(kind, at, msg);
	let diags = vec![
		w(DiagnosticKind::Unsupported,	"unknown paper size: a5x"),
		w(DiagnosticKind::Unsupported,	"unknown paper size: b9"),
		w(DiagnosticKind::Unsupported,	"page bleed is not drawn"),
		w(DiagnosticKind::Lint,			"label `<a>` is not attached to anything"),
		w(DiagnosticKind::Lint,			"label `<b>` is not attached to anything"),
		w(DiagnosticKind::MissingFont,	"unknown font family: Nonesuch"),
		Diagnostic::error(DiagnosticKind::Unsupported, at, "plugins are not supported: here"),
		Diagnostic::error(DiagnosticKind::Syntax, at, "unclosed delimiter"),
	];
	assert_eq!(summary_lines(&diags), vec![
		"diag-summary error syntax - 1",
		"diag-summary error unsupported plugins_are_not_supported 1",
		"diag-summary warning lint - 2",
		"diag-summary warning missing_font - 1",
		"diag-summary warning unsupported page_bleed_is_not_drawn 1",
		"diag-summary warning unsupported unknown_paper_size 2",
	]);
	assert!(summary_lines(&[]).is_empty(), "a clean compile has no lines");
}

// Each error line has the shape the probe lets through, `diag-error <kind> callee:<name> expected:<type>
// found:<type> file:<class>`, with each token in its closed vocabulary.
fn shape_ok(line: &str) -> bool {
	let t: Vec<&str> = line.split(' ').collect();
	if t.len() != 6 || t[0] != "diag-error" {
		return false;
	}
	let word = |s: &str, extra: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || extra.contains(c));
	word(t[1], "_")
		&& t[2].strip_prefix("callee:").map_or(false, |v| word(v, "0123456789.-"))
		&& t[3].strip_prefix("expected:").map_or(false, |v| word(v, "-"))
		&& t[4].strip_prefix("found:").map_or(false, |v| word(v, "-"))
		&& matches!(t[5], "file:main" | "file:sibling" | "file:other")
}

// One failing source, the error line it gives, and that the line has the probe's shape.
fn named(name: &str, src: &str, files: &[(&str, &str)]) -> Vec<String> {
	let r = run_with(name, src, files, &["--eval", "--diag-summary"]);
	assert!(!r.ok, "the compile fails:\n{}", r.stderr);
	for l in &r.errors {
		assert!(shape_ok(l), "an error line outside the probe's shape: {:?}", l);
	}
	r.errors
}

#[test]
fn a_standard_call_with_a_type_pair_is_named_with_both_types() {
	// `text(size: "a")`: Typst's "expected length, found string", at the argument inside the call.
	assert_eq!(named("pair", "#text(size: \"a\")[x]\n", &[]),
		vec!["diag-error type callee:text expected:length found:str file:main"]);
	// A type spelled differently in the message and in `repr` is one registry type: "relative length".
	assert_eq!(named("pair_rel", "#rect(width: \"a\")\n", &[]),
		vec!["diag-error type callee:rect expected:relative found:str file:main"]);
}

#[test]
fn a_dotted_standard_path_is_named_and_a_message_offering_several_types_has_no_pair() {
	assert_eq!(named("dotted", "#calc.abs(\"a\")\n", &[]),
		vec!["diag-error type callee:calc.abs expected:- found:- file:main"]);
	// A standard name outside the probe's lower-case alphabet (`sym.Alpha`) is `-`, as is any other.
	assert_eq!(named("upper", "#sym.Alpha(1)\n", &[]),
		vec!["diag-error type callee:- expected:function found:symbol file:main"]);
}

#[test]
fn a_document_s_own_function_or_name_is_never_named() {
	// A user function called with an argument it does not take: the error is at its call, and `f` is not
	// the standard library's.
	assert_eq!(named("user_fn", "#let f(x) = x\n#f(1, 2)\n", &[]),
		vec!["diag-error type callee:- expected:- found:- file:main"]);
	// A name bound nowhere is no standard name either.
	assert_eq!(named("unbound", "#nonesuch(1)\n", &[]),
		vec!["diag-error unknown_variable callee:- expected:- found:- file:main"]);
	// A path whose first segment is standard and whose last is not is hidden: `nonesuch` is not in `calc`.
	let lines = named("user_member", "#calc.nonesuch(1)\n", &[]);
	assert_eq!(lines.len(), 1, "one error: {:?}", lines);
	assert!(lines[0].contains(" callee:- "), "a member the module lacks is not named: {:?}", lines);
}

#[test]
fn the_file_of_an_error_is_main_a_sibling_or_other() {
	let bad = "#text(size: \"a\")[x]\n";
	assert_eq!(named("f_main", bad, &[]),
		vec!["diag-error type callee:text expected:length found:str file:main"]);
	assert_eq!(named("f_sibling", "#include \"sib.typ\"\n", &[("sib.typ", bad)]),
		vec!["diag-error type callee:text expected:length found:str file:sibling"]);
	assert_eq!(named("f_other", "#include \"sub/deep.typ\"\n", &[("sub/deep.typ", bad)]),
		vec!["diag-error type callee:text expected:length found:str file:other"]);
}

#[test]
fn the_error_lines_are_built_from_the_registry_and_the_standard_scope_alone() {
	let src = |id: u16, path: &str, text: &str| Source::new(FileId(id), PathBuf::from(path), text.to_string());
	let sources = vec![
		src(0, "/p/main.typ",		"#text(size: val)[a]\n"),
		src(1, "/p/sib.typ",		"#pad(secret_name)[a]\n"),
		src(2, "/p/sub/deep.typ",	"#secretfn(1)\n"),
	];
	let at = |id: u16, key: &str| {
		let start = sources[id as usize].text.find(key).expect("the text");
		Span::new(FileId(id), start as u32, (start + key.len()) as u32)
	};
	let err = |span, msg: &str| Diagnostic::error(DiagnosticKind::Type, span, msg);
	let diags = vec![
		// The argument of a standard call: the call's name, the types of a registry pair.
		err(at(0, "val"),			"expected length, found string"),
		// A user name inside a standard call is not named but the call is.
		err(at(1, "secret_name"),	"expected integer, found content"),
		// A user function's own call is hidden, and `foo` is no type.
		err(at(2, "secretfn"),		"expected foo, found bar"),
		// A span in no file at all, and a message offering several types.
		err(Span::detached(),		"expected length or auto, found string"),
		// A warning is not an error.
		Diagnostic::warning(DiagnosticKind::Lint, at(0, "val"), "expected length, found string"),
	];
	assert_eq!(error_lines(&diags, &sources), vec![
		"diag-error type callee:text expected:length found:str file:main",
		"diag-error type callee:pad expected:int found:content file:sibling",
		"diag-error type callee:- expected:- found:- file:other",
		"diag-error type callee:- expected:- found:- file:other",
	]);
	assert!(error_lines(&[], &sources).is_empty(), "no error, no line");
	// A message of the shape whose sides are not both types has no pair, whatever it says.
	for m in ["expected length, found", "expected , found string", "expected length, found string, x",
		"expected Length, found string", "expected length found string", "expecting length, found string"] {
		let l = error_lines(&[err(Span::detached(), m)], &[]);
		assert_eq!(l, vec!["diag-error type callee:- expected:- found:- file:other"], "the message {:?}", m);
	}
}

#[test]
fn a_pdf_attachment_is_passed_over_by_name_and_an_artifact_is_not() {
	// Austenite embeds no file, so the attachment is named as passed over; an artifact only tags a PDF.
	let r = run("pdf", "#pdf.attach(\"a.txt\", bytes(\"hi\"))\n#pdf.artifact(kind: \"header\")[x]\n", &["--eval", "--diag-summary"]);
	assert!(r.ok, "the compile succeeds:\n{}", r.stderr);
	assert_eq!(r.lines, vec!["diag-summary warning unsupported pdf_attachments_are_not_embedded 1"],
		"the summary of:\n{}", r.stderr);
}
