//! `austenite --eval --diag-summary`: one stderr line for each severity, kind and construct, counts only.
//!
//! The report names what a compile passed over and never the document it passed over in, so it can be read
//! where the document may not be. Each case compiles a synthetic file with the built binary and holds the
//! lines to their exact text.

use oxedyne_fe2o3_austenite::diag::{
	summary_lines,
	Diagnostic,
	DiagnosticKind,
};
use oxedyne_fe2o3_austenite::syntax::Span;

use std::path::PathBuf;
use std::process::Command;

// One file's compile: the exit status's success, and the report's lines.
struct Run {
	ok:		bool,
	lines:	Vec<String>,
	stderr:	String,
}

fn run(name: &str, src: &str, flags: &[&str]) -> Run {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diag_summary").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	let main = dir.join("main.typ");
	std::fs::write(&main, src).expect("the case's source");
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
	Run { ok: out.status.success(), lines, stderr }
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
}

#[test]
fn no_report_is_printed_unless_the_flag_is_given() {
	let r = run("off", TWO_UNSUPPORTED_ONE_LINT, &["--eval"]);
	assert!(r.ok, "the compile succeeds:\n{}", r.stderr);
	assert!(r.lines.is_empty(), "no report is printed unasked:\n{}", r.stderr);
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
