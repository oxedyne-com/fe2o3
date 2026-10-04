//! Poll-based recompile for the `austenite --watch` mode.
//!
//! There is no inotify on this fleet, so the watch samples a caller-supplied set of files on a fixed
//! interval and rebuilds whenever any of them changes. The file set is recomputed each tick from a
//! closure, so a chapter added to a book's `#include` list (or an asset dropped into its tree) is
//! picked up without restarting the watch. A file appearing or disappearing counts as a change, since
//! the snapshot keys on the paths that currently exist.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{
	Duration,
	SystemTime,
};

// How far a file's stamp may fall behind the clock it was taken from, the kernel's tick.
const STAMP_LAG: Duration = Duration::from_millis(10);

/// Runs the poll loop: build once, then on every tick recompute the watched set via `files`, sample
/// their modification times, and call `build` whenever the sample differs from the last one -- an
/// mtime moved, or a watched file appeared or vanished. A build that fails returns its error, which is
/// printed here, and the loop carries on, so a transient compile error never stops the watch. Under
/// normal use this never returns; it ends only on an interrupt.
pub fn run<F, G>(mut files: G, mut build: F, interval: Duration) -> Outcome<()>
where
	F: FnMut() -> Outcome<()>,
	G: FnMut() -> Vec<PathBuf>,
{
	// Build once at the start so the watch shows a result immediately rather than on the first edit.
	if let Err(e) = build() {
		eprintln!("[austenite] {}", e);
	}
	let mut prev = snapshot(&files());
	loop {
		std::thread::sleep(interval);
		let now = snapshot(&files());
		if now != prev {
			prev = now;
			if let Err(e) = build() {
				eprintln!("[austenite] {}", e);
			}
		}
	}
}

/// As [`run`], for a build that reports the files it read, which are then the watched set: the loop
/// rebuilds when any of them changes and for no other cause. The set is whatever the last build read, so a
/// build that failed still watches what it got as far as reading, and mending a file it stopped at is seen.
///
/// The snapshot a build is compared against is the one taken before it began, for the files known then, so
/// a file saved while a long build runs is built again. A file the build was the first to read has no such
/// snapshot, so one saved since the build began, by its time, is built again too.
pub fn run_read<F>(mut build: F, interval: Duration) -> Outcome<()>
where
	F: FnMut() -> (Outcome<()>, Vec<PathBuf>),
{
	let mut set:	Vec<PathBuf>					= Vec::new();
	let mut seen:	BTreeMap<PathBuf, SystemTime>	= BTreeMap::new();
	let mut first	= true;
	loop {
		if first || snapshot(&set) != seen {
			first = false;
			let before = snapshot(&set);
			let began = SystemTime::now();
			let (result, files) = build();
			if let Err(e) = result {
				eprintln!("[austenite] {}", e.plain());
			}
			seen = carry(&set, &files, &before, &snapshot(&files), began);
			set = files;
		}
		std::thread::sleep(interval);
	}
}

// The snapshot to compare the next tick with, over the build's files: `before` for each path of the set
// the build began with, and `after` for a path it read for the first time, unless that was saved since the
// build `began`, when it gets the epoch so that the next tick sees a change. A path of the old set that did
// not exist before the build is left out, so that if it exists now the next tick sees a change; one that
// existed and has gone is kept, so that the next tick sees it missing.
fn carry(
	old:	&[PathBuf],
	files:	&[PathBuf],
	before:	&BTreeMap<PathBuf, SystemTime>,
	after:	&BTreeMap<PathBuf, SystemTime>,
	began:	SystemTime,
)
	-> BTreeMap<PathBuf, SystemTime>
{
	// A stamp can lag the clock it was taken from by a tick, so a file saved just before the build is
	// built again once, which costs a rebuild and not a missed save.
	let since = began.checked_sub(STAMP_LAG).unwrap_or(began);
	let known: BTreeSet<&PathBuf> = old.iter().collect();
	let mut out = BTreeMap::new();
	for path in files {
		if known.contains(path) {
			if let Some(t) = before.get(path) {
				out.insert(path.clone(), *t);
			}
		} else if let Some(t) = after.get(path) {
			out.insert(path.clone(), if *t >= since { SystemTime::UNIX_EPOCH } else { *t });
		}
	}
	out
}

/// Maps each path that currently exists and can be stat'd to its last-modified time. A path that
/// cannot be read is simply absent from the map, so its later appearance -- or the disappearance of one
/// present before -- changes the snapshot and triggers a rebuild.
fn snapshot(paths: &[PathBuf]) -> BTreeMap<PathBuf, SystemTime> {
	let mut m = BTreeMap::new();
	for p in paths {
		if let Ok(meta) = std::fs::metadata(p) {
			if let Ok(t) = meta.modified() {
				m.insert(p.clone(), t);
			}
		}
	}
	m
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::time::UNIX_EPOCH;

	fn at(s: u64) -> SystemTime { UNIX_EPOCH + Duration::from_secs(s) }

	fn p(name: &str) -> PathBuf { PathBuf::from(name) }

	fn snap(items: &[(&str, u64)]) -> BTreeMap<PathBuf, SystemTime> {
		items.iter().map(|(n, s)| (p(n), at(*s))).collect()
	}

	#[test]
	fn a_known_file_keeps_the_time_it_had_before_the_build() {
		// `a` was saved again while the build ran: the build may have read it either side of the save, so
		// the next tick must see it as changed.
		let out = carry(&[p("a")], &[p("a")], &snap(&[("a", 1)]), &snap(&[("a", 2)]), at(0));
		assert_eq!(out, snap(&[("a", 1)]));
	}

	#[test]
	fn a_file_first_read_by_the_build_takes_the_time_after_it() {
		let out = carry(&[p("a")], &[p("a"), p("b")], &snap(&[("a", 1)]), &snap(&[("a", 1), ("b", 5)]), at(9));
		assert_eq!(out, snap(&[("a", 1), ("b", 5)]));
	}

	#[test]
	fn a_file_first_read_and_saved_since_the_build_began_reads_as_changed() {
		// `b` carries a time after the start of the build (10 s), so whatever the build read of it is
		// stale: the epoch stands in, which no later sample equals.
		let out = carry(&[], &[p("a"), p("b")], &snap(&[]), &snap(&[("a", 5), ("b", 12)]), at(10));
		assert_eq!(out, snap(&[("a", 5), ("b", 0)]));
	}

	#[test]
	fn a_known_file_that_was_absent_and_has_arrived_reads_as_changed() {
		// Absent before the build, present after it: left out, so the next sample, which has it, differs.
		let out = carry(&[p("a")], &[p("a")], &snap(&[]), &snap(&[("a", 3)]), at(9));
		assert_eq!(out, snap(&[]));
	}

	#[test]
	fn a_known_file_that_has_gone_reads_as_changed() {
		let out = carry(&[p("a")], &[p("a")], &snap(&[("a", 1)]), &snap(&[]), at(9));
		assert_eq!(out, snap(&[("a", 1)]));
	}

	#[test]
	fn a_file_the_build_no_longer_reads_is_dropped() {
		let out = carry(&[p("a"), p("b")], &[p("a")], &snap(&[("a", 1), ("b", 2)]), &snap(&[("a", 1), ("b", 2)]), at(9));
		assert_eq!(out, snap(&[("a", 1)]));
	}
}
