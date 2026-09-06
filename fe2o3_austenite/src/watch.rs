//! Poll-based recompile for the `austenite --watch` mode.
//!
//! There is no inotify on this fleet, so the watch samples a caller-supplied set of files on a fixed
//! interval and rebuilds whenever any of them changes. The file set is recomputed each tick from a
//! closure, so a chapter added to a book's `#include` list (or an asset dropped into its tree) is
//! picked up without restarting the watch. A file appearing or disappearing counts as a change, since
//! the snapshot keys on the paths that currently exist.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{
	Duration,
	SystemTime,
};

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
