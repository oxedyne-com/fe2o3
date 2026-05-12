//! Annealer -- the Oxedyne code formatter.
//!
//! CLI binary for formatting source files using the Wadler/Lindig
//! layout algebra engine in `fe2o3_text::fmt`.
//!
#![forbid(unsafe_code)]

use oxedyne_fe2o3_text::fmt::{
	self,
	spec::FormatSpec,
};

use std::io::Read;
use std::path::Path;
use std::process;


/// Supported language identifiers.
const LANGS: &[&str] = &["rust", "c", "cpp", "csharp", "go", "java", "javascript", "python"];

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();

	if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
		print_usage();
		process::exit(0);
	}

	let mut write   = false;
	let mut check   = false;
	let mut stdin   = false;
	let mut lang: Option<String> = None;
	let mut config: Option<String> = None;
	let mut paths:  Vec<String> = Vec::new();

	let mut i = 0;
	while i < args.len() {
		match args[i].as_str() {
			"--write" | "-w" => write = true,
			"--check" | "-c" => check = true,
			"--stdin"        => stdin = true,
			"--lang" => {
				i += 1;
				if i >= args.len() {
					eprintln!("error: --lang requires a value");
					process::exit(2);
				}
				lang = Some(args[i].to_lowercase());
			}
			"--config" => {
				i += 1;
				if i >= args.len() {
					eprintln!("error: --config requires a path");
					process::exit(2);
				}
				config = Some(args[i].clone());
			}
			other if other.starts_with('-') => {
				eprintln!("error: unknown flag '{}'", other);
				process::exit(2);
			}
			_ => paths.push(args[i].clone()),
		}
		i += 1;
	}

	if write && check {
		eprintln!("error: --write and --check are mutually exclusive");
		process::exit(2);
	}

	// Load format specification.
	let spec = match config {
		Some(ref path) => {
			let content = match std::fs::read_to_string(path) {
				Ok(s)  => s,
				Err(e) => {
					eprintln!("error: cannot read config '{}': {}", path, e);
					process::exit(1);
				}
			};
			match FormatSpec::from_config_str(&content) {
				Ok(s)  => s,
				Err(e) => {
					eprintln!("error: invalid config '{}': {}", path, e);
					process::exit(1);
				}
			}
		}
		None => FormatSpec::fe2o3(),
	};

	// Stdin mode.
	if stdin {
		let mut src = String::new();
		if let Err(e) = std::io::stdin().read_to_string(&mut src) {
			eprintln!("error: failed to read stdin: {}", e);
			process::exit(1);
		}
		let detected = lang.as_deref().unwrap_or("rust");
		if detected != "rust" {
			eprintln!(
				"warning: only Rust has full structural formatting; \
				'{}' will be lexed but not restructured",
				detected,
			);
		}
		match format_source(&src, detected, &spec) {
			Ok(formatted) => print!("{}", formatted),
			Err(e) => {
				eprintln!("error: {}", e);
				process::exit(1);
			}
		}
		return;
	}

	if paths.is_empty() {
		eprintln!("error: no input files");
		process::exit(2);
	}

	// Expand directories recursively.
	let files = match collect_files(&paths) {
		Ok(f) => f,
		Err(e) => {
			eprintln!("error: {}", e);
			process::exit(1);
		}
	};

	if files.is_empty() {
		eprintln!("error: no source files found");
		process::exit(2);
	}

	let mut failures: usize = 0;
	let mut would_change: Vec<String> = Vec::new();

	for path in &files {
		let detected = lang.as_deref().unwrap_or_else(
			|| detect_language_from_ext(path).unwrap_or("rust")
		);

		let src = match std::fs::read_to_string(path) {
			Ok(s)  => s,
			Err(e) => {
				eprintln!("error: {}: {}", path, e);
				failures += 1;
				continue;
			}
		};

		let formatted = match format_source(&src, detected, &spec) {
			Ok(f)  => f,
			Err(e) => {
				eprintln!("error: {}: {}", path, e);
				failures += 1;
				continue;
			}
		};

		if check {
			if formatted != src {
				would_change.push(path.clone());
			}
		} else if write {
			if formatted != src {
				if let Err(e) = std::fs::write(path, &formatted) {
					eprintln!("error: {}: {}", path, e);
					failures += 1;
				}
			}
		} else {
			print!("{}", formatted);
		}
	}

	if check {
		if !would_change.is_empty() {
			for p in &would_change {
				println!("{}", p);
			}
			process::exit(1);
		}
	}

	if failures > 0 {
		process::exit(1);
	}
}

/// Format source code for the given language.
fn format_source(src: &str, lang: &str, spec: &FormatSpec) -> Result<String, String> {
	match lang {
		"rust" => fmt::format_rust(src, spec).map_err(|e| format!("{}", e)),
		_ => Err(format!(
			"language '{}' is not yet supported for structural formatting \
			(supported: rust)",
			lang,
		)),
	}
}

/// Detect language from file extension.
fn detect_language_from_ext(path: &str) -> Option<&'static str> {
	let ext = Path::new(path).extension()?.to_str()?;
	match ext {
		"rs"                        => Some("rust"),
		"c" | "h"                   => Some("c"),
		"cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
		"cs"                        => Some("csharp"),
		"go"                        => Some("go"),
		"java"                      => Some("java"),
		"js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => Some("javascript"),
		"py" | "pyi"                => Some("python"),
		_                           => None,
	}
}

/// Recursively collect source files from paths (files or directories).
fn collect_files(paths: &[String]) -> Result<Vec<String>, String> {
	let mut result = Vec::new();
	for path in paths {
		let meta = std::fs::metadata(path)
			.map_err(|e| format!("{}: {}", path, e))?;
		if meta.is_file() {
			result.push(path.clone());
		} else if meta.is_dir() {
			walk_dir(Path::new(path), &mut result)?;
		}
	}
	result.sort();
	Ok(result)
}

/// Walk a directory tree collecting files with recognised extensions.
fn walk_dir(dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
	let entries = std::fs::read_dir(dir)
		.map_err(|e| format!("{}: {}", dir.display(), e))?;
	for entry in entries {
		let entry = entry.map_err(|e| format!("{}: {}", dir.display(), e))?;
		let path = entry.path();
		if path.is_dir() {
			// Skip hidden directories and target/.
			let name = path.file_name()
				.and_then(|n| n.to_str())
				.unwrap_or("");
			if name.starts_with('.') || name == "target" {
				continue;
			}
			walk_dir(&path, out)?;
		} else if path.is_file() {
			let p = path.to_string_lossy().to_string();
			if detect_language_from_ext(&p).is_some() {
				out.push(p);
			}
		}
	}
	Ok(())
}

/// Print usage information.
fn print_usage() {
	eprintln!("annealer -- the Oxedyne code formatter");
	eprintln!();
	eprintln!("USAGE:");
	eprintln!("    annealer [OPTIONS] [FILES/DIRS...]");
	eprintln!();
	eprintln!("OPTIONS:");
	eprintln!("    -w, --write      Format files in place");
	eprintln!("    -c, --check      Check formatting (exit 1 if changes needed)");
	eprintln!("    --stdin          Read from stdin");
	eprintln!("    --lang <LANG>    Override language detection ({})",
		LANGS.join(", "));
	eprintln!("    --config <FILE>  Load format specification from file");
	eprintln!("    -h, --help       Show this help");
	eprintln!();
	eprintln!("Without --write or --check, formatted output goes to stdout.");
	eprintln!("Directories are searched recursively for source files.");
}
