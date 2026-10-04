//! Resolving, and launching, an external editor.
//!
//! `fe2o3_syntax` knows the shape every caller shares -- an explicit flag beats an app-specific
//! environment variable beats a config value beats `$VISUAL` beats `$EDITOR` beats a PATH probe,
//! the child runs under `sh -c` so a value like `code --wait` or `emacsclient -t` needs no
//! quoting, and a bare GUI editor name wants a flag added to make it block -- but it knows
//! nothing about any one caller's flag name or config file. Those are supplied by the caller;
//! this module only reads `$VISUAL`, `$EDITOR`, `$TERM`, `$DISPLAY`/`$WAYLAND_DISPLAY` and
//! `$PATH`, which are the same for everyone.

use oxedyne_fe2o3_core::prelude::*;

use std::{
	ffi::OsStr,
	path::{Path, PathBuf},
	process::{Command, ExitStatus},
	time::{Duration, Instant},
};

/// Which precedence rung supplied the resolved editor command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
	Explicit,			// --editor, or the caller's equivalent
	EnvVar(String),		// the app-specific env var name the caller asked about, e.g. "ORE_EDITOR"
	Config,				// the caller's own config value
	Visual,				// $VISUAL
	Editor,				// $EDITOR
	Probe,				// found on $PATH from the platform's fallback list
}

/// The winning editor command, exactly as its source wrote it (a probe result is the bare
/// executable, plus any wait flag this module adds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
	pub command: String,
	pub source:  Source,
}

/// The parts of the standard environment that resolution and the platform probe need. A
/// caller's own flag, env var and config value are not here: they have no standard shape for
/// this module to read on the caller's behalf.
#[derive(Clone, Debug)]
pub struct StandardEnv {
	pub visual:    Option<String>,
	pub editor:    Option<String>,
	pub tty:       bool,			// stdin is a terminal
	pub term_dumb: bool,			// $TERM == "dumb"
	pub display:   bool,			// $DISPLAY or $WAYLAND_DISPLAY is set
	pub path_dirs: Vec<PathBuf>,
}

impl StandardEnv {

	/// Reads `$VISUAL`, `$EDITOR`, `$TERM`, `$DISPLAY`, `$WAYLAND_DISPLAY`, `$PATH` and stdin's
	/// terminal-ness from the real process environment.
	pub fn from_process() -> Self {
		use std::io::IsTerminal;
		let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
		Self {
			visual:    var("VISUAL"),
			editor:    var("EDITOR"),
			tty:       std::io::stdin().is_terminal(),
			term_dumb: var("TERM").as_deref() == Some("dumb"),
			display:   var("DISPLAY").is_some() || var("WAYLAND_DISPLAY").is_some(),
			path_dirs: match std::env::var_os("PATH") {
				Some(p) => std::env::split_paths(&p).collect(),
				None    => Vec::new(),
			},
		}
	}
}

/// Resolves the editor to run, in order: `explicit`, then `app_env` (the caller's own
/// environment variable, named and read by the caller), then `config` (the caller's own config
/// value), then `$VISUAL` (only when the terminal can use it: a tty and `$TERM` is not
/// `"dumb"`), then `$EDITOR`, then the platform's PATH probe. A bare known GUI editor name
/// gains its wait flag (`code` -> `code --wait`, and so on) whichever rung supplied it; a value
/// that already carries its own arguments is left exactly as written.
pub fn resolve(
	explicit: Option<&str>,
	app_env:  Option<(&str, &str)>,	// (env var name, its value), when the caller found one set
	config:   Option<&str>,
	env:      &StandardEnv,
) -> Outcome<Resolved> {
	if let Some(cmd) = non_empty(explicit) {
		return Ok(finish(cmd, Source::Explicit));
	}
	if let Some((name, val)) = app_env {
		if let Some(cmd) = non_empty(Some(val)) {
			return Ok(finish(cmd, Source::EnvVar(name.to_string())));
		}
	}
	if let Some(cmd) = non_empty(config) {
		return Ok(finish(cmd, Source::Config));
	}
	if env.tty && !env.term_dumb {
		if let Some(cmd) = non_empty(env.visual.as_deref()) {
			return Ok(finish(cmd, Source::Visual));
		}
	}
	if let Some(cmd) = non_empty(env.editor.as_deref()) {
		return Ok(finish(cmd, Source::Editor));
	}
	for probe in probe_list(env.tty, env.display) {
		let exe = match probe.first() {
			Some(exe) => *exe,
			None => continue, // no executable named for this platform; nothing to look up
		};
		if find_on_path(exe, &env.path_dirs).is_some() {
			return Ok(Resolved { command: probe.join(" "), source: Source::Probe });
		}
	}
	Err(err!(
		"No editor found: pass one explicitly, set {}, $VISUAL or $EDITOR, or install one of \
		the usual editors on PATH.", app_env.map(|(n, _)| n).unwrap_or("$EDITOR");
		Missing, Configuration))
}

fn non_empty(v: Option<&str>) -> Option<&str> {
	v.map(str::trim).filter(|v| !v.is_empty())
}

fn finish(cmd: &str, source: Source) -> Resolved {
	Resolved { command: apply_wait_flag(cmd), source }
}

// GUI editors that, run bare, open and return at once without blocking, and the flag that
// makes each wait for the file to close. Verify a flag against a real binary before trusting
// it; xed and pluma are not in this table because they were not checked.
const WAIT_FLAGS: &[(&str, &str)] = &[
	("code",              "--wait"),
	("codium",            "--wait"),
	("cursor",            "--wait"),
	("windsurf",          "--wait"),
	("zed",               "--wait"),
	("subl",              "-w"),
	("mate",              "-w"),
	("gvim",              "-f"),
	("mvim",              "-f"),
	("kate",              "-b"),
	("gedit",             "--wait"),
	("gnome-text-editor", "--standalone"),
];

/// Adds a known GUI editor's wait flag when `command` is a bare name with no arguments of its
/// own; otherwise returns it unchanged.
pub fn apply_wait_flag(command: &str) -> String {
	let trimmed = command.trim();
	if trimmed.split_whitespace().count() == 1 {
		for (name, flag) in WAIT_FLAGS {
			if trimmed == *name {
				return format!("{} {}", trimmed, flag);
			}
		}
	}
	trimmed.to_string()
}

/// Editor names the probe already knows how to run from a terminal. A caller refusing a
/// terminal editor with no tty (§B2 of the CLI plan) checks its resolved command against this
/// before opening anything.
pub const KNOWN_TERMINAL_EDITORS: &[&str] = &["editor", "nano", "nvim", "vim", "vi"];

/// Does `command`'s first word name one of [`KNOWN_TERMINAL_EDITORS`]?
pub fn is_known_terminal_editor(command: &str) -> bool {
	match command.trim().split_whitespace().next() {
		Some(word) => {
			let base = Path::new(word).file_name().and_then(OsStr::to_str).unwrap_or(word);
			KNOWN_TERMINAL_EDITORS.contains(&base)
		},
		None => false,
	}
}

/// The platform's fallback probe order, first found on PATH wins. `tty` picks the terminal vs.
/// GUI branch; `display` gates the Linux/BSD no-tty branch on `$DISPLAY`/`$WAYLAND_DISPLAY`
/// being set, since a headless box with no display has nothing in that list to find.
pub fn probe_list(tty: bool, display: bool) -> Vec<Vec<&'static str>> {
	if cfg!(target_os = "windows") {
		vec![vec!["code", "--wait"], vec!["notepad"]]
	} else if cfg!(target_os = "macos") {
		if tty {
			vec![vec!["nano"], vec!["vim"], vec!["vi"]]
		} else {
			vec![vec!["open", "-W", "-n", "-t"]]
		}
	} else if tty {
		vec![vec!["editor"], vec!["nano"], vec!["nvim"], vec!["vim"], vec!["vi"]]
	} else if display {
		vec![
			vec!["gnome-text-editor", "--standalone"],
			vec!["gedit", "--wait"],
			vec!["kate", "--block"],
			vec!["code", "--wait"],
			vec!["gvim", "-f"],
		]
	} else {
		Vec::new()
	}
}

/// The first directory in `dirs` holding an executable file named `name` (`name.exe` too, on
/// Windows), or `None`.
pub fn find_on_path(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
	for dir in dirs {
		let candidate = dir.join(name);
		if is_executable_file(&candidate) {
			return Some(candidate);
		}
		if cfg!(target_os = "windows") {
			let exe = dir.join(format!("{}.exe", name));
			if exe.is_file() {
				return Some(exe);
			}
		}
	}
	None
}

// A real `#[cfg(unix)]` split, not a runtime `if cfg!(unix)`: the unix branch names
// `std::os::unix::fs::PermissionsExt`, which does not exist to name on a non-Unix target.
#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
	use std::os::unix::fs::PermissionsExt;
	match std::fs::metadata(path) {
		Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
		Err(_) => false,
	}
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
	path.is_file()
}

/// Builds the child command for `resolved`, following the file(s) as the last argument(s).
/// On Unix this is `sh -c '<resolved> "$@"' <argv0> <paths...>`, so a value the person wrote
/// with its own arguments (`code --wait`, `emacsclient -t`) needs no quoting or splitting by
/// this crate, and a path holding spaces reaches the editor as one argument because `sh` never
/// re-parses the positional parameters. On Windows, `resolved` is split on whitespace since cmd
/// has no `sh -c` equivalent worth relying on. `argv0` is the caller's own program name, used
/// only as `$0` inside the shell and otherwise invisible to the editor.
pub fn build_command<P: AsRef<OsStr>>(resolved: &str, argv0: &str, paths: &[P]) -> Command {
	if cfg!(target_os = "windows") {
		let mut parts = resolved.split_whitespace();
		let mut cmd = Command::new(parts.next().unwrap_or_default());
		cmd.args(parts);
		cmd.args(paths);
		cmd
	} else {
		let mut cmd = Command::new("sh");
		cmd.arg("-c");
		cmd.arg(format!("{} \"$@\"", resolved));
		cmd.arg(argv0);
		cmd.args(paths);
		cmd
	}
}

/// Whether a finished editor was actually waited on, or handed the file to a window that was
/// already open and returned at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Waited, Detached }

/// The outcome of running an editor to completion.
pub struct Run {
	pub status:  ExitStatus,
	pub elapsed: Duration,
	pub verdict: Verdict,
}

/// Runs `resolved` on `paths` and waits for it to exit, classifying a success that returned
/// within `detach_after` as [`Verdict::Detached`] -- handed off to a window that was already
/// running, rather than genuinely finished. A non-zero exit is reported as [`Verdict::Waited`]
/// regardless of how quickly it happened, since a fast failure is not a detach; the caller
/// decides what a failing editor means.
pub fn run_and_classify<P: AsRef<OsStr>>(
	resolved:      &str,
	argv0:         &str,
	paths:         &[P],
	detach_after:  Duration,
) -> Outcome<Run> {
	let started = Instant::now();
	let mut child = res!(build_command(resolved, argv0, paths).spawn(), IO, System, Init);
	let status = res!(child.wait(), IO, System);
	let elapsed = started.elapsed();
	let verdict = if status.success() && elapsed < detach_after {
		Verdict::Detached
	} else {
		Verdict::Waited
	};
	Ok(Run { status, elapsed, verdict })
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::fs;

	fn stub_env(tty: bool, display: bool, path_dirs: Vec<PathBuf>) -> StandardEnv {
		StandardEnv {
			visual: None,
			editor: None,
			tty,
			term_dumb: false,
			display,
			path_dirs,
		}
	}

	#[cfg(unix)]
	fn make_executable(p: &Path) {
		use std::os::unix::fs::PermissionsExt;
		fs::set_permissions(p, fs::Permissions::from_mode(0o755))
			.expect("marking the fake executable +x");
	}

	#[cfg(not(unix))]
	fn make_executable(_p: &Path) {}

	// A directory of fake executables, one per name, each just `#!/bin/sh\ntrue`.
	fn fake_path(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
		for name in names {
			let p = dir.join(name);
			fs::write(&p, "#!/bin/sh\ntrue\n").expect("writing a fake executable");
			make_executable(&p);
		}
		vec![dir.to_path_buf()]
	}

	#[test]
	fn precedence_explicit_beats_everything_00() -> Outcome<()> {
		let env = StandardEnv {
			visual: Some(fmt!("visual-editor")),
			editor: Some(fmt!("editor-editor")),
			..stub_env(true, false, Vec::new())
		};
		let r = res!(resolve(
			Some("explicit-editor"),
			Some(("ORE_EDITOR", "env-editor")),
			Some("config-editor"),
			&env,
		));
		req!(r.command, fmt!("explicit-editor"));
		req!(r.source, Source::Explicit);
		Ok(())
	}

	#[test]
	fn precedence_app_env_beats_config_and_visual_01() -> Outcome<()> {
		let env = StandardEnv {
			visual: Some(fmt!("visual-editor")),
			..stub_env(true, false, Vec::new())
		};
		let r = res!(resolve(None, Some(("ORE_EDITOR", "env-editor")), Some("config-editor"), &env));
		req!(r.command, fmt!("env-editor"));
		req!(r.source, Source::EnvVar(fmt!("ORE_EDITOR")));
		Ok(())
	}

	#[test]
	fn precedence_config_beats_visual_and_editor_02() -> Outcome<()> {
		let env = StandardEnv {
			visual: Some(fmt!("visual-editor")),
			editor: Some(fmt!("editor-editor")),
			..stub_env(true, false, Vec::new())
		};
		let r = res!(resolve(None, None, Some("config-editor"), &env));
		req!(r.command, fmt!("config-editor"));
		req!(r.source, Source::Config);
		Ok(())
	}

	#[test]
	fn visual_beats_editor_when_terminal_capable_03() -> Outcome<()> {
		let env = StandardEnv {
			visual: Some(fmt!("visual-editor")),
			editor: Some(fmt!("editor-editor")),
			..stub_env(true, false, Vec::new())
		};
		let r = res!(resolve(None, None, None, &env));
		req!(r.command, fmt!("visual-editor"));
		req!(r.source, Source::Visual);
		Ok(())
	}

	#[test]
	fn visual_is_skipped_when_term_is_dumb_04() -> Outcome<()> {
		let env = StandardEnv {
			visual:    Some(fmt!("visual-editor")),
			editor:    Some(fmt!("editor-editor")),
			term_dumb: true,
			..stub_env(true, false, Vec::new())
		};
		let r = res!(resolve(None, None, None, &env));
		req!(r.command, fmt!("editor-editor"));
		req!(r.source, Source::Editor);
		Ok(())
	}

	#[test]
	fn visual_is_skipped_without_a_tty_05() -> Outcome<()> {
		let env = StandardEnv {
			visual: Some(fmt!("visual-editor")),
			editor: Some(fmt!("editor-editor")),
			..stub_env(false, false, Vec::new())
		};
		let r = res!(resolve(None, None, None, &env));
		req!(r.command, fmt!("editor-editor"));
		req!(r.source, Source::Editor);
		Ok(())
	}

	#[test]
	fn probe_finds_first_available_on_a_scratch_path_06() -> Outcome<()> {
		let dir = res!(oxedyne_fe2o3_test::scratch::scratch_dir("fe2o3_syntax_editor_probe_06"));
		// nvim and vim exist, but nano does not -- nano still comes first in the probe list, so
		// its absence must not stop nvim from being found.
		let path_dirs = fake_path(&dir, &["nvim", "vim"]);
		let env = stub_env(true, false, path_dirs);
		let r = res!(resolve(None, None, None, &env));
		req!(r.command, fmt!("nvim"));
		req!(r.source, Source::Probe);
		Ok(())
	}

	#[test]
	fn probe_finds_nothing_returns_an_error_07() -> Outcome<()> {
		let env = stub_env(true, false, Vec::new());
		match resolve(None, None, None, &env) {
			Err(_) => Ok(()),
			Ok(r) => Err(err!("expected no editor to be found, got {:?}", r; Test)),
		}
	}

	#[test]
	fn wait_flag_added_to_a_bare_known_gui_name_08() -> Outcome<()> {
		let env = stub_env(true, false, Vec::new());
		let r = res!(resolve(Some("code"), None, None, &env));
		req!(r.command, fmt!("code --wait"));
		Ok(())
	}

	#[test]
	fn wait_flag_not_added_when_the_value_has_its_own_arguments_09() -> Outcome<()> {
		let env = stub_env(true, false, Vec::new());
		let r = res!(resolve(Some("code --new-window"), None, None, &env));
		req!(r.command, fmt!("code --new-window"));
		Ok(())
	}

	#[test]
	fn is_known_terminal_editor_matches_by_basename_10() -> Outcome<()> {
		req!(is_known_terminal_editor("/usr/bin/vim"), true);
		req!(is_known_terminal_editor("vi"), true);
		req!(is_known_terminal_editor("code --wait"), false);
		Ok(())
	}

	#[test]
	fn build_command_carries_a_path_with_spaces_intact_11() -> Outcome<()> {
		let dir = res!(oxedyne_fe2o3_test::scratch::scratch_dir("fe2o3_syntax_editor_spaces_11"));
		let out = dir.join("argv.txt");
		let script = dir.join("record-argv.sh");
		res!(fs::write(&script, format!(
			"#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {}; done\n",
			out.display(),
		)), IO, File, Write);
		make_executable(&script);
		let weird_path = dir.join("a file with spaces.txt");
		let paths = vec![weird_path.as_os_str().to_owned()];
		let mut cmd = build_command(&script.display().to_string(), "test-argv0", &paths);
		let status = res!(cmd.status(), IO, System);
		req!(status.success(), true);
		let recorded = res!(fs::read_to_string(&out), IO, File, Read);
		req!(recorded.trim_end(), weird_path.display().to_string());
		Ok(())
	}

	#[test]
	fn run_and_classify_detects_a_child_that_exits_at_once_12() -> Outcome<()> {
		let paths: Vec<&OsStr> = Vec::new();
		let run = res!(run_and_classify("true", "test-argv0", &paths, Duration::from_secs(2)));
		req!(run.status.success(), true);
		req!(run.verdict, Verdict::Detached);
		Ok(())
	}

	#[test]
	fn run_and_classify_treats_a_slow_success_as_waited_13() -> Outcome<()> {
		let paths: Vec<&OsStr> = Vec::new();
		let run = res!(run_and_classify(
			"sleep 0.3", "test-argv0", &paths, Duration::from_millis(50),
		));
		req!(run.status.success(), true);
		req!(run.verdict, Verdict::Waited);
		Ok(())
	}
}
