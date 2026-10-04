//! Captures the git commit the engine is built from, for `compile::engine_git_hash` and the wasm
//! `engineInfo()`. No git, or no repository, yields `unknown` rather than failing the build.
//!
//! Sets the wasm module's stack. The layout depth guard ends a nest of containers where Typst does, at about
//! seventy levels, and the stack has to hold the deepest nest the guard lets through with room to spare: an
//! overflow is a trap, which ends the module for the page's life. It is set here, as a link argument of the
//! `cdylib` for the `wasm32` target alone, so that every build of the wasm module has it, whichever directory
//! it is started in and whatever `RUSTFLAGS` says.

use std::path::Path;
use std::process::Command;

// The wasm stack, in bytes: 8 MiB. The deepest nest the guard lets through, seventy boxes, uses 535 KiB of it,
// and the default stack is 1 MiB; the rest is margin for a nest of mixed containers and for a nest of equations,
// which the guard does not count and which uses about 2.8 KiB a level.
const WASM_STACK: usize = 8 << 20;

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

	if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
		println!("cargo:rustc-link-arg-cdylib=-zstack-size={}", WASM_STACK);
	}

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
