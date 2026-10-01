//! GNU date as the oracle for the zone and moment tests.
//!
//! The machine's own `date` may not be GNU's (Ubuntu 25.10 ships uutils), so
//! `gnudate`, `gdate` and `date` are tried in turn and the first whose
//! `--version` says GNU coreutils is used. A machine with none skips the
//! oracle tests with a message and still runs the fixed-answer ones.
#![allow(dead_code)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::time::{
	CalClockZone,
	TZifParser,
};

use std::{
	collections::HashSet,
	fs,
	io::Write,
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::{
		atomic::{AtomicUsize, Ordering},
		OnceLock,
	},
};

pub const ZONEINFO: &str = "/usr/share/zoneinfo";

// Whole-year bounds, in UTC seconds.
pub const Y2026: i64 = 1_767_225_600;	// 2026-01-01T00:00:00Z
pub const Y2027: i64 = 1_798_761_600;	// 2027-01-01T00:00:00Z
pub const Y2038: i64 = 2_145_916_800;	// 2038-01-01T00:00:00Z
pub const Y2043: i64 = 2_303_683_200;	// 2043-01-01T00:00:00Z

/// The GNU date to ask, if the machine has one.
pub fn gnu_date() -> Option<&'static str> {
	static GNU: OnceLock<Option<String>> = OnceLock::new();
	GNU.get_or_init(|| {
		for name in ["gnudate", "gdate", "date"] {
			if let Ok(out) = Command::new(name).arg("--version").output() {
				if String::from_utf8_lossy(&out.stdout).contains("GNU coreutils") {
					return Some(name.to_string());
				}
			}
		}
		None
	}).as_deref()
}

/// Says so and returns true when there is no GNU date to ask.
pub fn skip_without_oracle() -> bool {
	if gnu_date().is_none() {
		eprintln!("skipped: no GNU date on this machine");
		return true;
	}
	false
}

fn scratch() -> PathBuf {
	let dir = std::env::temp_dir().join(fmt!("datime-oracle-{}", std::process::id()));
	let _ = fs::create_dir_all(&dir);
	dir
}

/// One answer per line, in order, from GNU date run with `tz` set and `tzdir`
/// as its zoneinfo tree if given. A line it refuses comes back None.
pub fn date_batch(
	tz:		&str,
	tzdir:	Option<&Path>,
	lines:	&[String],
	format:	&str,
)
	-> Outcome<Vec<Option<String>>>
{
	let gnu = res!(gnu_date().ok_or_else(|| err!("No GNU date."; System, Missing)));
	// Tests run in parallel, so each call has a file of its own.
	static CALLS: AtomicUsize = AtomicUsize::new(0);
	let file = scratch().join(fmt!("in-{}.txt", CALLS.fetch_add(1, Ordering::SeqCst)));
	{
		let mut f = res!(fs::File::create(&file).map_err(|e| err!(e, "Could not write {:?}", file; IO, File)));
		for line in lines {
			res!(writeln!(f, "{}", line).map_err(|e| err!(e, "Could not write {:?}", file; IO, File)));
		}
	}
	let mut cmd = Command::new(gnu);
	cmd.env("TZ", tz).env_remove("TZDIR");
	if let Some(dir) = tzdir {
		cmd.env("TZDIR", dir);
	}
	let out = res!(cmd.arg("-f").arg(&file).arg(format)
		.stdin(Stdio::null()).output().map_err(|e| err!(e, "Could not run {}", gnu; IO)));
	let _ = fs::remove_file(&file);
	let stdout = String::from_utf8_lossy(&out.stdout).to_string();
	let mut said = stdout.lines();
	// The lines it refuses are named on stderr, between curly quotes.
	let mut refused: HashSet<String> = HashSet::new();
	for line in String::from_utf8_lossy(&out.stderr).lines() {
		if let (Some(a), Some(b)) = (line.find('\u{2018}'), line.rfind('\u{2019}')) {
			if line.contains("invalid date") && a < b {
				refused.insert(line[a + '\u{2018}'.len_utf8()..b].to_string());
			}
		}
	}
	let mut answers = Vec::with_capacity(lines.len());
	for line in lines {
		if refused.contains(line) {
			answers.push(None);
		} else {
			match said.next() {
				Some(s)	=> answers.push(Some(s.to_string())),
				None	=> return Err(err!(
					"GNU date gave no answer for '{}' under TZ={} TZDIR={:?}: {}",
					line, tz, tzdir, String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("");
					Invalid, Mismatch)),
			}
		}
	}
	if said.next().is_some() {
		return Err(err!("GNU date answered more lines than it was given."; Invalid, Mismatch));
	}
	Ok(answers)
}

/// GNU date's `+%z` text as seconds east of UTC.
pub fn z_secs(z: &str) -> i64 {
	let sign = if z.starts_with('-') { -1 } else { 1 };
	let n: i64 = z[1..3].parse().unwrap_or(0) ;
	let m: i64 = z[3..5].parse().unwrap_or(0);
	sign * (n * 3600 + m * 60)
}

/// A zone as `here()` makes one, from the host's zoneinfo file; None when the
/// file is absent. Slim copies live in `dir`.
pub fn zone_from(dir: &str, name: &str) -> Outcome<Option<CalClockZone>> {
	let path = Path::new(dir).join(name);
	if !path.is_file() {
		eprintln!("skipped {}: no file {:?}", name, path);
		return Ok(None);
	}
	let mut parser = TZifParser::new();
	res!(parser.load_from_file(&path));
	let data = res!(parser.timezone_data().ok_or_else(|| err!("No data in {:?}", path; Invalid, Missing)));
	Ok(Some(res!(CalClockZone::from_tzif_data(name, data.clone()))))
}

/// The wall times of a list of local seconds, as GNU date writes them in UTC.
pub fn walls(locals: &[i64]) -> Outcome<Vec<String>> {
	let lines: Vec<String> = locals.iter().map(|n| fmt!("@{}", n)).collect();
	let out = res!(date_batch("UTC", None, &lines, "+%F %T"));
	let mut walls = Vec::with_capacity(out.len());
	for w in out {
		walls.push(res!(w.ok_or_else(|| err!("GNU date refused an epoch second."; Invalid, Mismatch))));
	}
	Ok(walls)
}
