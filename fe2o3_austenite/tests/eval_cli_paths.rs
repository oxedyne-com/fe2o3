//! `austenite --eval` resolves the source and the root as `typst compile` does: both are made canonical, so
//! a source named relative to the working directory under an absolute `--root`, or the reverse, still lies
//! within the root and the files it reaches for are found.
//!
//! Each case compiles a synthetic project with the built binary run from inside it.

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

// A project: `main.typ` includes and imports files below it. Written fresh for each case.
fn project(name: &str) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_cli_paths").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("sub")).expect("the project's directory");
	std::fs::write(dir.join("main.typ"), "#import \"sub/lib.typ\": word\n#include \"sub/part.typ\"\n#word\n").expect("main");
	std::fs::write(dir.join("sub/lib.typ"), "#let word = [Imported.]\n").expect("lib");
	std::fs::write(dir.join("sub/part.typ"), "Included.\n").expect("part");
	dir
}

// Compiles `src` with `--root root`, run from `cwd`; the exit status's success and the standard error.
fn compile(cwd: &Path, root: &str, src: &str) -> (bool, String) {
	let out_dir = cwd.join("out");
	let out = Command::new(env!("CARGO_BIN_EXE_austenite"))
		.current_dir(cwd)
		.args(["--eval", "--diag-summary", "--root", root, src])
		.arg(&out_dir)
		.output()
		.expect("the built austenite binary");
	(out.status.success() && out_dir.join("document.pdf").is_file(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn a_source_named_relative_to_the_working_directory_under_an_absolute_root_is_within_it() {
	let dir = project("relative_source");
	let (ok, err) = compile(&dir, dir.to_str().expect("a UTF-8 path"), "main.typ");
	assert!(ok, "the relative source compiles under the absolute root:\n{}", err);
	assert!(!err.contains("diag-error"), "no error is named:\n{}", err);
}

#[test]
fn a_root_named_relative_to_the_working_directory_holds_an_absolute_source() {
	let dir = project("relative_root");
	let main = dir.join("main.typ");
	let (ok, err) = compile(&dir, ".", main.to_str().expect("a UTF-8 path"));
	assert!(ok, "the absolute source compiles under the relative root:\n{}", err);
}

#[test]
fn a_root_and_a_source_that_climb_with_dotdot_agree_where_the_source_is() {
	let dir = project("dotdot");
	let (ok, err) = compile(&dir.join("sub"), "..", "../main.typ");
	assert!(ok, "the source reached through `..` compiles under the root reached through `..`:\n{}", err);
}
