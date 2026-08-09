//! Does the AVI header reader agree with a player about real films?
//!
//! The unit tests beside the reader build their own files, so they prove the
//! reader agrees with the author's idea of the format and nothing more. This
//! asks a different program about films nobody here wrote: point `AVI_CORPUS` at
//! a directory and every `.avi` under it is read twice, once by this crate and
//! once by `ffprobe`, and the two answers are compared.
//!
//! Only the front of each file is given to the reader -- the same sniffing
//! buffer a scanner would hold -- because answering from a head is the property
//! that makes this usable during a walk.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::avi::Avi;

use std::{
	env,
	fs::{self, File},
	io::Read,
	path::{Path, PathBuf},
	process::Command,
};

/// The head a scanner holds, and all this reader is given.
const HEAD: usize = 64 * 1024;

/// How far a running time may differ from the player's and still agree.
///
/// A tenth of a second, or one part in a hundred, whichever is larger. The
/// container states a frame count and a rate; a player may also consult the
/// index and the last frame's timestamp, so the two need not agree to the
/// millisecond and a disagreement that matters is far larger than this.
const SLACK_MS: i64 = 100;

fn films(dir: &Path, out: &mut Vec<PathBuf>) {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return,
	};
	let mut here = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			films(&path, out);
		} else if path.to_string_lossy().to_lowercase().ends_with(".avi") {
			here.push(path);
		}
	}
	here.sort();
	out.extend(here);
}

/// The head of a file, which is all the reader is entitled to.
fn head_of(path: &Path) -> Option<Vec<u8>> {
	let mut f = match File::open(path) {
		Ok(f) => f,
		Err(_) => return None,
	};
	let mut buf = vec![0u8; HEAD];
	let mut got = 0usize;
	while got < HEAD {
		match f.read(&mut buf[got..]) {
			Ok(0) => break,
			Ok(n) => got += n,
			Err(_) => return None,
		}
	}
	buf.truncate(got);
	Some(buf)
}

/// What ffprobe says: width, height and duration in milliseconds.
fn oracle(path: &Path) -> Option<(u32, u32, i64)> {
	let out = Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", "v:0",
			"-show_entries", "stream=width,height:format=duration",
			"-of", "default=noprint_wrappers=1:nokey=1",
		])
		.arg(path)
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let mut lines = text.lines();
	let w: u32 = lines.next()?.trim().parse().ok()?;
	let h: u32 = lines.next()?.trim().parse().ok()?;
	let secs: f64 = lines.next()?.trim().parse().ok()?;
	Some((w, h, (secs * 1000.0).round() as i64))
}

#[test]
fn the_header_agrees_with_ffprobe() -> Outcome<()> {
	let dir = match env::var("AVI_CORPUS") {
		Ok(d) if !d.is_empty() => PathBuf::from(d),
		_ => {
			println!("skipped: set AVI_CORPUS to a directory of films");
			return Ok(());
		},
	};
	let cap: usize = env::var("AVI_CORPUS_MAX").ok()
		.and_then(|n| n.parse().ok())
		.unwrap_or(600);

	let mut all = Vec::new();
	films(&dir, &mut all);
	println!("{} AVI files under {}", all.len(), dir.display());

	let (mut compared, mut agreed, mut refused, mut no_oracle) = (0usize, 0usize, 0usize, 0usize);
	let mut worst_ms = 0i64;
	let mut sizes_wrong = Vec::new();
	let mut times_wrong = Vec::new();

	for path in all.iter().take(cap) {
		let head = match head_of(path) {
			Some(h) => h,
			None => continue,
		};
		let mine = match Avi::read(&head) {
			Ok(a) => a,
			Err(_) => { refused += 1; continue },
		};
		let (w, h, ms) = match oracle(path) {
			Some(o) => o,
			None => { no_oracle += 1; continue },
		};
		compared += 1;
		let (mw, mh) = mine.size();
		if (mw, mh) != (w, h) {
			if sizes_wrong.len() < 5 {
				sizes_wrong.push(fmt!("{}: {}x{} against {}x{}",
					path.display(), mw, mh, w, h));
			}
			continue;
		}
		match mine.millis() {
			Some(got) => {
				let off = (got as i64 - ms).abs();
				let slack = SLACK_MS.max(ms / 100);
				if off > slack {
					if times_wrong.len() < 5 {
						times_wrong.push(fmt!("{}: {} ms against {} ms",
							path.display(), got, ms));
					}
					continue;
				}
				worst_ms = worst_ms.max(off);
			},
			None => {
				if times_wrong.len() < 5 {
					times_wrong.push(fmt!("{}: no running time, player says {} ms",
						path.display(), ms));
				}
				continue;
			},
		}
		agreed += 1;
	}

	println!("{} compared, {} agreed on size and running time", compared, agreed);
	println!("worst running-time difference: {} ms", worst_ms);
	println!("{} refused by the reader, {} ffprobe would not read", refused, no_oracle);
	for line in sizes_wrong.iter().chain(times_wrong.iter()) {
		println!("  {}", line);
	}

	// Reporting is not the point here: a header reader that disagrees with a
	// player about the size of the picture is wrong, and the catalogue would
	// show the wrong shape for every one of these films.
	if compared > 0 {
		req!(agreed, compared,
			"The header reader and ffprobe disagree about {} of {} films.",
			compared - agreed, compared);
	}
	Ok(())
}
