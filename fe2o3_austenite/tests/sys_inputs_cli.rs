//! `--input key=value` on the command line reaches the document as `sys.inputs`, as `typst compile --input`
//! does (D-20261018-03): on a plain compile, under `--explain`, and on `austenite build`. Each source asserts
//! what it should see, so a run that succeeds is a run whose document saw it; a failed assertion fails the
//! compile and the run.

use std::path::{
	Path,
	PathBuf,
};
use std::process::{
	Command,
	Output,
};

// A fresh case directory holding `main.typ` with `src`.
fn case(name: &str, src: &str) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sys_inputs_cli").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	std::fs::write(dir.join("main.typ"), src).expect("the case's source");
	dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_austenite"))
		.current_dir(dir)
		.args(args)
		.output()
		.expect("the built austenite binary")
}

fn said(out: &Output) -> String {
	format!("stdout: {}\nstderr: {}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

const ENGINE: &str = "#assert.eq(sys.inputs.at(\"engine\", default: none), \"austenite\")\n\
	#assert.eq(sys.inputs, (engine: \"austenite\", note: \"a=b\"))\nCreated using Austenite.\n";

const NONE: &str = "#assert.eq(sys.inputs, (:))\n#assert.eq(sys.inputs.at(\"engine\", default: none), none)\nPlain.\n";

#[test]
fn a_plain_compile_passes_each_input_to_the_document() {
	let d = case("plain", ENGINE);
	let out = run(&d, &["--input", "engine=austenite", "--input", "note=a=b", "main.typ", "out"]);
	assert!(out.status.success(), "the document saw its inputs\n{}", said(&out));
	assert!(d.join("out/document.pdf").is_file());
}

#[test]
fn explain_passes_each_input_to_the_document() {
	let d = case("explain", ENGINE);
	let out = run(&d, &["--explain", "main.typ", "out", "--input", "engine=austenite", "--input", "note=a=b"]);
	assert!(out.status.success(), "the document saw its inputs under --explain\n{}", said(&out));
}

#[test]
fn build_passes_each_input_to_the_document() {
	let d = case("build", ENGINE);
	let out = run(&d, &["build", "main.typ", "--set", "view.open=false", "--input", "engine=austenite", "--input", "note=a=b"]);
	assert!(out.status.success(), "the document saw its inputs under build\n{}", said(&out));
}

#[test]
fn no_input_gives_the_document_an_empty_dictionary() {
	let d = case("none", NONE);
	let out = run(&d, &["main.typ", "out"]);
	assert!(out.status.success(), "sys.inputs is empty with no --input\n{}", said(&out));
}

#[test]
fn a_wrong_expectation_fails_the_run() {
	// The control: the assertions above are what make a success mean the document saw its inputs.
	let d = case("control", ENGINE);
	let out = run(&d, &["--input", "engine=typst", "--input", "note=a=b", "main.typ", "out"]);
	assert!(!out.status.success(), "a document that sees another engine fails its assertion\n{}", said(&out));
}

#[test]
fn a_malformed_input_is_refused_by_name() {
	let d = case("malformed", NONE);
	for (arg, why) in [("=x", "key"), (" =x", "key"), ("noeq", "=")] {
		let out = run(&d, &["--input", arg, "main.typ", "out"]);
		let s = said(&out);
		assert!(!out.status.success(), "--input {:?} is refused\n{}", arg, s);
		assert!(s.contains("--input") && s.contains(arg.trim()) && s.contains(why), "the refusal names {:?}\n{}", arg, s);
	}
	let out = run(&d, &["main.typ", "out", "--input"]);
	let s = said(&out);
	assert!(!out.status.success() && s.contains("--input needs key=value"), "a missing value is refused\n{}", s);
	let out = run(&d, &["build", "main.typ", "--input", "=x"]);
	let s = said(&out);
	assert!(!out.status.success() && s.contains("--input"), "build refuses it too\n{}", s);
}

#[test]
fn version_names_the_engine_and_its_commit() {
	let d = case("version", NONE);
	let out = run(&d, &["--version"]);
	let o = String::from_utf8_lossy(&out.stdout).to_string();
	assert!(out.status.success() && o.starts_with("austenite ") && o.contains('('), "{}", said(&out));
}
