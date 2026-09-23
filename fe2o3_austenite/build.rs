//! Captures the git commit the engine is built from, for `compile::engine_git_hash` and the wasm
//! `engineInfo()`. No git, or no repository, yields `unknown` rather than failing the build.

use std::path::Path;
use std::process::Command;

fn main() {
	let hash = match git(&["rev-parse", "--short=12", "HEAD"]) {
		Some(h) if !h.is_empty() => {
			// Dirty only for this crate's own tree, so an unrelated edit elsewhere in the workspace does
			// not mark the engine.
			match git(&["status", "--porcelain", "--", "."]) {
				Some(s) if !s.is_empty()	=> format!("{}-dirty", h),
				_							=> h,
			}
		},
		_ => "unknown".to_string(),
	};
	println!("cargo:rustc-env=AUSTENITE_GIT_HASH={}", hash);

	// Naming any rerun trigger turns off Cargo's default of rerunning on every package file, so the
	// crate's sources are named alongside the git state a commit or checkout moves.
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=Cargo.toml");
	println!("cargo:rerun-if-changed=src");
	if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
		println!("cargo:rerun-if-changed={}", Path::new(&dir).join("HEAD").display());
		println!("cargo:rerun-if-changed={}", Path::new(&dir).join("index").display());
	}
	if let Some(dir) = git(&["rev-parse", "--git-common-dir"]) {
		println!("cargo:rerun-if-changed={}", Path::new(&dir).join("packed-refs").display());
		if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
			println!("cargo:rerun-if-changed={}", Path::new(&dir).join(head_ref).display());
		}
	}
}

/// The trimmed standard output of a successful git command, or `None`.
fn git(args: &[&str]) -> Option<String> {
	match Command::new("git").args(args).output() {
		Ok(out) if out.status.success() => match String::from_utf8(out.stdout) {
			Ok(s)	=> Some(s.trim().to_string()),
			Err(_)	=> None,
		},
		_ => None,
	}
}
