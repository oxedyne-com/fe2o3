//! Registers the reader with the desktop for the current user: `.prl` documents get a type of their own,
//! the reader becomes their default handler, and a launcher entry appears among the applications.
//! Nothing is written outside the user's own profile, so no elevation is asked for.
//!
//! On Linux that is the freedesktop trio: a shared-mime-info package declaring `application/x-pearlite`,
//! a `.desktop` entry whose `Exec` is this very executable, and `xdg-mime default`. On Windows it is the
//! per-user `HKCU\Software\Classes` keys, written through `reg.exe` so that no Win32 call, and so no
//! `unsafe`, is needed. Registration is idempotent, and is rerun after the executable moves.

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

pub const MIME_TYPE:	&str = "application/x-pearlite";
pub const DESKTOP_ID:	&str = "pearlite-reader.desktop";
pub const PROG_ID:		&str = "Pearlite.Document";

/// One action of a registration: a file to write, or a program to run. A plan is a list of these, built
/// without touching the system, so what a registration would do is inspectable before it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
	Write {
		path:	PathBuf,
		body:	String,
	},
	Run {
		prog:		String,
		args:		Vec<String>,
		required:	bool,	// a failure stops the registration, rather than being reported and passed
	},
}

/// The registration a Linux desktop needs, rooted at the XDG data home `data_home` (normally
/// `~/.local/share`), launching `exe`.
pub fn plan_linux(exe: &Path, data_home: &Path) -> Vec<Step> {
	let mime_dir	= data_home.join("mime");
	let apps_dir	= data_home.join("applications");
	let mime_xml	= fmt!(concat!(
		"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
		"<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
		"  <mime-type type=\"{}\">\n",
		"    <comment>Pearlite document</comment>\n",
		"    <glob pattern=\"*.prl\"/>\n",
		"    <icon name=\"x-office-document\"/>\n",
		"  </mime-type>\n",
		"</mime-info>\n"), MIME_TYPE);
	let desktop		= fmt!(concat!(
		"[Desktop Entry]\n",
		"Type=Application\n",
		"Version=1.0\n",
		"Name=Pearlite Reader\n",
		"GenericName=Document Reader\n",
		"Comment=Open and read Pearlite (.prl) documents\n",
		"Exec={} %f\n",
		"Icon=x-office-document\n",
		"Terminal=false\n",
		"Categories=Office;Viewer;\n",
		"MimeType={};\n"), desktop_exec_arg(exe), MIME_TYPE);
	vec![
		Step::Write { path: mime_dir.join("packages").join("pearlite.xml"), body: mime_xml },
		Step::Write { path: apps_dir.join(DESKTOP_ID), body: desktop },
		// The two cache refreshes are what make the type and the entry visible at once; a desktop without
		// the tools still picks both up at its next scan, so neither failure is fatal.
		Step::Run {
			prog:		"update-mime-database".to_string(),
			args:		vec![mime_dir.to_string_lossy().into_owned()],
			required:	false,
		},
		Step::Run {
			prog:		"update-desktop-database".to_string(),
			args:		vec![apps_dir.to_string_lossy().into_owned()],
			required:	false,
		},
		Step::Run {
			prog:		"xdg-mime".to_string(),
			args:		vec!["default".to_string(), DESKTOP_ID.to_string(), MIME_TYPE.to_string()],
			required:	true,
		},
	]
}

/// The per-user registry entries Windows needs, launching `exe`. Each is a `reg add … /f`, so a rerun
/// overwrites rather than fails.
pub fn plan_windows(exe: &Path) -> Vec<Step> {
	let exe		= exe.to_string_lossy().into_owned();
	let classes	= "HKCU\\Software\\Classes";
	let prog	= fmt!("{}\\{}", classes, PROG_ID);
	let values: Vec<(String, Option<&str>, String)> = vec![	// (key, value name or default, data)
		(fmt!("{}\\.prl", classes),				None,					PROG_ID.to_string()),
		(fmt!("{}\\.prl", classes),				Some("Content Type"),	MIME_TYPE.to_string()),
		(prog.clone(),							None,					"Pearlite document".to_string()),
		(fmt!("{}\\DefaultIcon", prog),			None,					fmt!("\"{}\",0", exe)),
		(fmt!("{}\\shell\\open\\command", prog),	None,					fmt!("\"{}\" \"%1\"", exe)),
	];
	values.into_iter().map(|(key, name, data)| {
		let mut args = vec!["add".to_string(), key];
		match name {
			Some(n)	=> { args.push("/v".to_string()); args.push(n.to_string()); },
			None	=> args.push("/ve".to_string()),
		}
		args.extend(["/t".to_string(), "REG_SZ".to_string(), "/d".to_string(), data, "/f".to_string()]);
		Step::Run { prog: "reg".to_string(), args, required: true }
	}).collect()
}

/// A path as a `.desktop` `Exec` argument: always quoted, with the four characters the specification
/// reserves inside quotes escaped, so a path with spaces or a dollar sign survives the launcher.
pub fn desktop_exec_arg(exe: &Path) -> String {
	let mut out = String::from("\"");
	for c in exe.to_string_lossy().chars() {
		if matches!(c, '"' | '`' | '$' | '\\') {
			out.push('\\');
		}
		out.push(c);
	}
	out.push('"');
	out
}

/// The executable a registration should point the desktop at. On Windows that is the console-free
/// `pearlite-reader.exe` beside this binary when it is there, so a double-clicked document opens a
/// window and no console; otherwise it is this binary itself.
pub fn handler_exe() -> Outcome<PathBuf> {
	let me = res!(std::env::current_exe(), IO, File);
	if cfg!(windows) {
		if let Some(dir) = me.parent() {
			let gui = dir.join("pearlite-reader.exe");
			if gui.is_file() {
				return Ok(gui);
			}
		}
	}
	Ok(me)
}

/// The XDG data home: `$XDG_DATA_HOME` when set and absolute, else `$HOME/.local/share`.
fn xdg_data_home() -> Outcome<PathBuf> {
	if let Ok(d) = std::env::var("XDG_DATA_HOME") {
		let p = PathBuf::from(d);
		if p.is_absolute() {
			return Ok(p);
		}
	}
	let home = res!(std::env::var("HOME").map_err(|e| err!(e,
		"Neither XDG_DATA_HOME nor HOME is set, so there is nowhere to register the reader."; Missing)));
	Ok(PathBuf::from(home).join(".local").join("share"))
}

/// The plan for the platform this binary runs on.
pub fn plan() -> Outcome<Vec<Step>> {
	let exe = res!(handler_exe());
	if cfg!(windows) {
		Ok(plan_windows(&exe))
	} else if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") || cfg!(target_os = "openbsd") {
		Ok(plan_linux(&exe, &res!(xdg_data_home())))
	} else {
		Err(err!("Registering the reader is not implemented on {}.", std::env::consts::OS;
			Unimplemented))
	}
}

/// Carries out a plan, returning one line per step saying what happened. A required step that fails
/// stops the run with an error naming it; an optional one is reported and passed.
pub fn execute(steps: &[Step]) -> Outcome<Vec<String>> {
	let mut report = Vec::with_capacity(steps.len());
	for step in steps {
		match step {
			Step::Write { path, body } => {
				if let Some(dir) = path.parent() {
					res!(std::fs::create_dir_all(dir), IO, File, Write);
				}
				res!(std::fs::write(path, body), IO, File, Write);
				report.push(fmt!("wrote {}", path.display()));
			},
			Step::Run { prog, args, required } => {
				let line = fmt!("{} {}", prog, args.join(" "));
				let failure = match Command::new(prog).args(args).output() {
					Ok(o) if o.status.success()	=> None,
					Ok(o)						=> Some(fmt!("exited with {}: {}", o.status,
						String::from_utf8_lossy(&o.stderr).trim())),
					Err(e)						=> Some(fmt!("could not be started: {}", e)),
				};
				match (failure, required) {
					(None, _)			=> report.push(fmt!("ran {}", line)),
					(Some(f), true)		=> return Err(err!(
						"Registration step `{}` {}.", line, f; IO, System)),
					(Some(f), false)	=> report.push(fmt!("skipped {} ({})", line, f)),
				}
			},
		}
	}
	Ok(report)
}
