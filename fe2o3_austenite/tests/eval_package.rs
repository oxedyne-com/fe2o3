//! Package specifications (`@namespace/name:1.2.3`) parsed by `package::parse_spec` against the
//! `typst` 0.15 oracle: every malformed form must fail with Typst's own message, and a well-formed one
//! must yield its parts. `EVAL_ORACLE_SKIP=1` skips explicitly.

use oxedyne_fe2o3_austenite::diag::message_of;
use oxedyne_fe2o3_austenite::eval::package::parse_spec;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::process::Command;

// Malformed specifications, each failing a different check, and well-formed ones Typst accepts as far
// as looking for the package (which is absent).
const SPECS: &[&str] = &[
	"@preview/x:1.2",
	"@preview/x:1.2.3.4",
	"@/x:1.2.3",
	"@preview/:1.2.3",
	"@preview/x",
	"@preview/x:",
	"@preview/x:a.2.3",
	"@preview/x:1..3",
	"@pre view/x:1.2.3",
	"@preview/x y:1.2.3",
	"@preview/x:1.2.b",
	"@pre-view/x:1.2.3",
	"@preview/x:01.2.3",
	"@local/tidy_thing:10.20.30",
];

fn skip() -> bool { std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false) }

// The first error typst gives for `#import "<spec>"`.
fn oracle(spec: &str, n: usize) -> Outcome<String> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_package");
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!("Cannot create {}: {}", dir.display(), e; IO, File, Write)));
	let src = dir.join(fmt!("p{}.typ", n));
	res!(std::fs::write(&src, fmt!("#import \"{}\"\n", spec)).map_err(|e| err!(
		"Cannot write {}: {}", src.display(), e; IO, File, Write)));
	let out = res!(Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", "typst", "compile",
			"-f", "pdf"])
		.arg(&src).arg(dir.join(fmt!("p{}.pdf", n))).output()
		.map_err(|e| err!("The typst oracle could not run ({}); set EVAL_ORACLE_SKIP=1 to skip.", e; IO, Missing)));
	let stderr = String::from_utf8_lossy(&out.stderr).to_string();
	match stderr.lines().find_map(|l| l.strip_prefix("error: ")) {
		Some(m)	=> Ok(m.to_string()),
		None	=> Err(err!("typst accepted `{}`, which no package here satisfies", spec; Invalid)),
	}
}

#[test]
fn package_specifications_match_the_typst_oracle() -> Outcome<()> {
	if skip() {
		println!("EVAL_ORACLE_SKIP=1: oracle comparison skipped");
		return Ok(());
	}
	let mut failures = Vec::new();
	let mut parsed = 0;
	for (n, spec) in SPECS.iter().enumerate() {
		let want = res!(oracle(spec, n));
		match parse_spec(spec) {
			// Typst got past the specification and looked for the package.
			Ok(p) => {
				parsed += 1;
				let shown = fmt!("@{}/{}:{}.{}.{}", p.namespace, p.name, p.version.0, p.version.1, p.version.2);
				if want != fmt!("package not found (searched for {})", shown) {
					failures.push(fmt!("{}: parsed as {}, typst: {}", spec, shown, want));
				}
			}
			Err(e) => {
				let got = message_of(&e);
				if got != want {
					failures.push(fmt!("{}: austenite `{}`, typst `{}`", spec, got, want));
				}
			}
		}
	}
	assert!(parsed >= 2 && parsed < SPECS.len(), "{} of {} parsed: the corpus must hold both kinds", parsed, SPECS.len());
	assert!(failures.is_empty(), "{} of {} specifications differ:\n{}", failures.len(), SPECS.len(), failures.join("\n"));
	Ok(())
}
