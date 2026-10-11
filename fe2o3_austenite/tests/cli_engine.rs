//! FIRE-2 (2026-10-11): every `.typ` source compiles through the evaluator -- plain `austenite`, `--explain`
//! and `--eval` alike -- a book opts back to the curated reader with `"engine": "curated"` in its
//! `austenite.jdat`, and a flag the chosen engine would not read is refused by name.
//!
//! The two engines are told apart by what they leave: the evaluator writes `document.pdf` alone and reports
//! its bytes; the curated reader writes an SVG per page beside it and reports its ledger's anchors.

use std::path::{
	Path,
	PathBuf,
};
use std::process::{
	Command,
	Output,
};

// One unsupported construct (the gradient fill) and one lint, so `--explain` has sites to report.
const SRC: &str = "= Title\n\nSome text.\n\n#rect(fill: gradient.linear(red, blue))\n";

// A fresh case directory holding `main.typ`, and an `austenite.jdat` beside it when `jdat` is given.
fn case(name: &str, jdat: Option<&str>) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli_engine").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	std::fs::write(dir.join("main.typ"), SRC).expect("the case's source");
	if let Some(j) = jdat {
		std::fs::write(dir.join("austenite.jdat"), j).expect("the case's settings");
	}
	dir
}

// Runs the built binary in `dir` with `args`, then `main.typ out`.
fn run(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_austenite"))
		.current_dir(dir)
		.args(args)
		.args(["main.typ", "out"])
		.output()
		.expect("the built austenite binary")
}

fn text(out: &Output) -> (String, String) {
	(String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

// Did the evaluator compile it: a PDF, its byte count reported, and no per-page SVG?
fn by_evaluator(dir: &Path, out: &Output) -> bool {
	let (o, _) = text(out);
	out.status.success() && dir.join("out/document.pdf").is_file()
		&& o.contains("byte(s)") && !dir.join("out/page-001.svg").exists()
}

// Did the curated reader compile it: a PDF, an SVG per page, and the ledger's anchors reported?
fn by_curated(dir: &Path, out: &Output) -> bool {
	let (o, _) = text(out);
	out.status.success() && dir.join("out/document.pdf").is_file()
		&& o.contains("anchor(s) in the ledger") && dir.join("out/page-001.svg").is_file()
}

#[test]
fn plain_austenite_compiles_through_the_evaluator() {
	let dir = case("plain", None);
	let out = run(&dir, &[]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
}

#[test]
fn eval_is_accepted_and_changes_nothing() {
	let dir = case("eval_alias", None);
	let out = run(&dir, &["--eval"]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
}

#[test]
fn explain_reports_the_evaluators_sites_with_line_and_column() {
	let dir = case("explain", None);
	let out = run(&dir, &["--explain"]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
	let (o, e) = text(&out);
	let head = o.lines().find(|l| l.contains("main.typ:5:2: warning: unsupported: gradients"));
	assert!(head.is_some(), "the gradient fill is reported at line 5:\n{}\n{}", o, e);
	assert!(o.contains("#rect(fill: gradient.linear(red, blue))\n"), "the source line is shown:\n{}", o);
	assert!(o.lines().any(|l| l.trim_start() == "^"), "a caret marks the column:\n{}", o);
	assert!(!e.contains("skipped:"), "the detailed report stands in for the terse line:\n{}", e);
}

#[test]
fn a_book_opted_back_reaches_the_curated_reader() {
	let dir = case("opted_back", Some("{\"engine\": \"curated\"}"));
	let out = run(&dir, &[]);
	assert!(by_curated(&dir, &out), "{:?}", text(&out));
}

#[test]
fn a_book_opted_back_still_reaches_the_evaluator_by_name() {
	let dir = case("opted_back_eval", Some("{\"engine\": \"curated\"}"));
	let out = run(&dir, &["--engine", "evaluator"]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
	let dir = case("opted_back_eval_flag", Some("{\"engine\": \"curated\"}"));
	let out = run(&dir, &["--eval"]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
}

#[test]
fn a_jdat_that_does_not_opt_back_leaves_the_evaluator() {
	let dir = case("jdat_evaluator", Some("{\"engine\": \"evaluator\", \"strict\": true}"));
	let out = run(&dir, &[]);
	assert!(by_evaluator(&dir, &out), "{:?}", text(&out));
}

#[test]
fn an_engine_austenite_does_not_have_is_refused_by_the_key() {
	let dir = case("jdat_bad", Some("{\"engine\": \"typst\"}"));
	let out = run(&dir, &[]);
	let (_, e) = text(&out);
	assert!(!out.status.success() && e.contains("'engine'"), "{}", e);
}

#[test]
fn the_curated_readers_flags_are_refused_on_the_evaluator() {
	for args in [&["--pearl"][..], &["--ledger-out", "l.json"][..]] {
		let dir = case("refuse_eval", None);
		let out = run(&dir, args);
		let (_, e) = text(&out);
		let want = fmt(args[0], "the evaluator path");
		assert!(!out.status.success() && e.contains(&want), "{:?}: {}", args, e);
		assert!(!dir.join("out/document.pdf").exists(), "nothing is compiled: {:?}", args);
	}
}

#[test]
fn the_evaluators_flags_are_refused_on_the_curated_reader() {
	for args in [
		&["--strict"][..],
		&["--diag-summary"][..],
		&["--timings", "t.json"][..],
		&["--root", "."][..],
		&["--font-path", "."][..],
	] {
		let dir = case("refuse_curated", Some("{\"engine\": \"curated\"}"));
		let out = run(&dir, args);
		let (_, e) = text(&out);
		let want = fmt(args[0], "the curated reader path");
		assert!(!out.status.success() && e.contains(&want), "{:?}: {}", args, e);
		assert!(e.contains("austenite.jdat"), "the file that chose the engine is named: {}", e);
	}
}

#[test]
fn explain_with_watch_is_refused_on_both_engines() {
	let dir = case("explain_watch", None);
	let out = run(&dir, &["--explain", "--watch"]);
	let (_, e) = text(&out);
	assert!(!out.status.success() && e.contains(&fmt("--explain", "the evaluator path")), "{}", e);
	let dir = case("explain_watch_curated", Some("{\"engine\": \"curated\"}"));
	let out = run(&dir, &["--explain", "--watch"]);
	let (_, e) = text(&out);
	assert!(!out.status.success() && e.contains(&fmt("--explain", "the curated reader path")), "{}", e);
}

#[test]
fn eval_and_engine_curated_together_are_refused() {
	let dir = case("two_engines", None);
	let out = run(&dir, &["--eval", "--engine", "curated"]);
	let (_, e) = text(&out);
	assert!(!out.status.success() && e.contains("two engines"), "{}", e);
}

#[test]
fn build_refuses_a_book_opted_back_to_the_curated_reader() {
	let dir = case("build_curated", Some("{\"engine\": \"curated\"}"));
	let out = Command::new(env!("CARGO_BIN_EXE_austenite"))
		.current_dir(&dir)
		.args(["build", "main.typ"])
		.output()
		.expect("the built austenite binary");
	let (_, e) = text(&out);
	assert!(!out.status.success() && e.contains("'engine'") && e.contains("curated"), "{}", e);
}

fn fmt(flag: &str, path: &str) -> String {
	format!("{} is not read on {}", flag, path)
}
