// The one way a test runs `typst`, shared by every harness that does: the structure oracle
// (`tests/eval_oracle/`), its PDF comparators and the corpus oracle (`tests/oracle/`). It lives in a
// directory of its own because a bare `tests/*.rs` file would be built as a test target, and each
// harness includes it by `#[path]`.

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;
use std::process::Command;

/// The one way a test runs `typst`: under `systemd-run`'s memory cap when `cap` is given, and with the
/// account's own fonts hidden. Typst looks for them under `$XDG_DATA_HOME/fonts`; a face installed there (a
/// second Libertinus Serif of another version, say) outranks Typst's own and would move a glyph on one side
/// only. Typst sees the faces the system and Typst carry, whoever runs the test, which is what the oracle's cache key, `fonts=system`, says.
/// `work` is where the directory that does not exist is named, so that it never does.
pub fn typst_command(bin: &str, cap: Option<&str>, work: &Path) -> Command {
	let mut c = match cap {
		Some(cap) => {
			let mut c = Command::new("systemd-run");
			c.args(["--user", "--scope", "--quiet", "-p"]);
			c.arg(fmt!("MemoryMax={}", cap));
			c.arg("--slice=claude-rc.slice");
			c.arg(bin);
			c
		}
		None => Command::new(bin),
	};
	c.env("XDG_DATA_HOME", work.join("no-user-fonts"));
	c
}
