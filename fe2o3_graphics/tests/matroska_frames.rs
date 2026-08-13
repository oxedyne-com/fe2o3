//! Does the cluster reader hand back the same frames a player demuxes?
//!
//! [`matroska_corpus`](../matroska_corpus.rs) asks whether the *description* of
//! a film is right. This asks the harder question: whether the frames lifted out
//! of the clusters are the ones that are actually there. It matters more,
//! because the point of reading clusters at all is repackaging, and a
//! repackager that drops a frame, mistimes one, or glues a laced pair together
//! produces a file that plays -- badly, or for a while, or with the sound adrift
//! -- rather than one that fails.
//!
//! **The oracle is `ffprobe -show_packets`**, which demuxes the same file with a
//! wholly separate implementation and reports every packet's size, presentation
//! time and whether decoding may begin at it. Four things are compared per
//! frame, in order: the count, the size in bytes, the time, and the keyframe
//! flag. A reader that found the clusters but mis-split the blocks agrees on
//! none of them.
//!
//! Point `MKV_CORPUS` at a directory of films. `MKV_FRAMES` caps how many frames
//! of each are compared, because a two-hour film holds two hundred thousand and
//! the disease this catches shows itself in the first thousand.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::matroska::{Clusters, Matroska, TrackKind};

use std::{
	env,
	fs::{self, File},
	io::Read,
	path::{Path, PathBuf},
	process::Command,
};

/// How much of a film is given to the header reader before the clusters.
const HEAD: usize = 1024 * 1024;

/// The window the streaming reader is held to, in bytes.
///
/// Deliberately small -- far smaller than a cluster, which is megabytes -- so
/// that the test fails if the reader ever needs a whole cluster in hand. A
/// window this size passing is the evidence that a four-gigabyte film can be
/// repackaged without being held.
const WINDOW: usize = 256 * 1024;

/// How many frames of each film are compared unless `MKV_FRAMES` says otherwise.
const FRAMES: usize = 4000;

/// How many films are read unless `MKV_FILES` says otherwise.
const FILES: usize = 8;

/// One frame, as either implementation describes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Packet {
	size:	usize,
	time:	i64,
	key:	bool,
}

#[test]
fn frames_agree_with_the_player() -> Outcome<()> {
	let dir = match env::var("MKV_CORPUS") {
		Ok(d) => PathBuf::from(d),
		Err(_) => {
			println!("MKV_CORPUS is not set, so no film was read. This test \
				compares real films against ffprobe and proves nothing without \
				them.");
			return Ok(());
		},
	};
	if Command::new("ffprobe").arg("-version").output().is_err() {
		println!("ffprobe is not installed, so there is no oracle to compare \
			against and this test proves nothing.");
		return Ok(());
	}

	let want = num_from_env("MKV_FRAMES", FRAMES);
	let most = num_from_env("MKV_FILES", FILES);

	let mut films = Vec::new();
	res!(gather(&dir, &mut films));
	films.sort();

	let mut compared	= 0usize;
	let mut refused		= 0usize;
	let mut no_oracle	= 0usize;
	let mut frames		= 0usize;

	for film in films.iter() {
		if compared >= most {
			break;
		}
		// Both streams, and the sound is not an afterthought: **lacing lives in
		// audio**. A film's picture is one frame a block and exercises none of
		// the three lacing schemes, so a reader checked on the picture alone has
		// its lacing arithmetic wholly unproven against anything real.
		let mut both = 0usize;
		for (kind, pick) in [(TrackKind::Video, "v:0"), (TrackKind::Audio, "a:0")] {
			let ours = match read_ours(film, want, kind) {
				Ok(Some(v)) => v,
				// A film with no such track, or one this reader will not open,
				// is counted rather than passed over in silence.
				Ok(None) => continue,
				Err(e) => {
					println!("{}: {} refused by the reader: {}", film.display(), pick, e);
					refused += 1;
					continue;
				},
			};
			let theirs = match read_theirs(film, want, pick) {
				Some(v) => v,
				None => { no_oracle += 1; continue },
			};
			if theirs.is_empty() {
				no_oracle += 1;
				continue;
			}

			if ours.len() != theirs.len() {
				return Err(err!(
					"{}: on {} the reader found {} frames and ffprobe found {}.",
					film.display(), pick, ours.len(), theirs.len();
					Invalid, Mismatch));
			}
			for i in 0..ours.len() {
				if ours[i] != theirs[i] {
					return Err(err!(
						"{}: on {}, frame {} of {} differs. The reader says {:?} \
						and ffprobe says {:?}.",
						film.display(), pick, i, ours.len(), ours[i], theirs[i];
						Invalid, Mismatch));
				}
			}
			frames += ours.len();
			both += 1;
			println!("{}: {} {} frames agree", film.display(), ours.len(), pick);
		}
		if both == 0 {
			refused += 1;
			continue;
		}
		compared += 1;
	}

	// Every film is accounted for, so that a run which quietly compared nothing
	// cannot read as a pass.
	println!(
		"{} films found; {} compared ({} frames), {} refused, {} without an oracle",
		films.len(), compared, frames, refused, no_oracle);
	if !films.is_empty() && compared == 0 {
		return Err(err!(
			"{} films were found and not one was compared, so this run proves \
			nothing.", films.len();
			Invalid, Mismatch));
	}
	Ok(())
}

/// Reads a film's frames through the streaming reader, holding only a window.
///
/// `None` where the film carries no picture track to compare.
fn read_ours(path: &Path, want: usize, kind: TrackKind) -> Outcome<Option<Vec<Packet>>> {
	let mut head = vec![0u8; HEAD];
	let mut file = res!(File::open(path));
	let n = res!(file.read(&mut head));
	head.truncate(n);
	let mkv = res!(Matroska::read(&head));
	// The **first** track of the kind, in file order, because that is what
	// ffprobe's `v:0` and `a:0` mean. Deliberately not `Matroska::video()`,
	// which prefers a default-flagged stream and would compare two different
	// streams on a film carrying an alternative take.
	let track = match mkv.tracks().iter().find(|t| t.kind() == Some(kind)) {
		Some(t) => {
			if env::var("MKV_SAY_TRACKS").is_ok() {
				println!("  track {} {:?} {} frame_ns={} rate={}",
					t.number(), t.kind(), t.codec(), t.frame_nanos(), t.rate());
			}
			t.number()
		},
		None => return Ok(None),
	};

	let mut file = res!(File::open(path));
	let mut cl = Clusters::new(&mkv);
	let mut buf: Vec<u8> = Vec::new();
	let mut out = Vec::new();
	let mut done = false;

	loop {
		// Top the window up. Reading stops at the end of the file, which is not
		// an error -- the reader is simply told nothing more is coming.
		while buf.len() < WINDOW && !done {
			let mut chunk = vec![0u8; WINDOW];
			let got = res!(file.read(&mut chunk));
			if got == 0 {
				done = true;
				break;
			}
			chunk.truncate(got);
			buf.extend_from_slice(&chunk);
		}
		if buf.is_empty() {
			break;
		}

		let mut full = false;
		let fed = res!(cl.feed(&buf, &mut |frame| {
			if frame.track == track && out.len() < want {
				out.push(Packet {
					size:	frame.data.len(),
					time:	frame.time,
					key:	frame.key,
				});
				if out.len() >= want {
					full = true;
				}
			}
			Ok(())
		}));
		buf.drain(..fed.used);
		if full {
			break;
		}
		// No progress and nothing more to read means the file ended inside an
		// element; no progress with more to read means the window must grow,
		// which a block larger than the window legitimately requires.
		if fed.used == 0 {
			if done && fed.want > buf.len() {
				break;
			}
			if fed.want > buf.len() {
				let mut chunk = vec![0u8; fed.want - buf.len()];
				let got = res!(file.read(&mut chunk));
				if got == 0 {
					break;
				}
				chunk.truncate(got);
				buf.extend_from_slice(&chunk);
			} else if done {
				break;
			}
		}
	}
	Ok(Some(out))
}

/// Reads the same film's packets with ffprobe.
fn read_theirs(path: &Path, want: usize, pick: &str) -> Option<Vec<Packet>> {
	let out = match Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", pick,
			"-show_entries", "packet=pts,size,flags",
			"-of", "csv=p=0",
		])
		.arg(path)
		.output()
	{
		Ok(o) => o,
		Err(_) => return None,
	};
	if !out.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let mut packets = Vec::new();
	for line in text.lines() {
		if packets.len() >= want {
			break;
		}
		let mut parts = line.trim().split(',');
		// A packet with no presentation time is one this comparison cannot
		// speak about, and it is rare enough that giving up on the file is
		// honest -- a partial list silently compared would be worse.
		let time = match parts.next().map(|v| v.parse::<i64>()) {
			Some(Ok(v)) => v,
			_ => return None,
		};
		let size = match parts.next().map(|v| v.parse::<usize>()) {
			Some(Ok(v)) => v,
			_ => return None,
		};
		let flags = parts.next().unwrap_or("");
		packets.push(Packet { size, time, key: flags.starts_with('K') });
	}
	Some(packets)
}

/// Every `.mkv` under a directory, following no symbolic link twice.
fn gather(dir: &Path, out: &mut Vec<PathBuf>) -> Outcome<()> {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return Ok(()),
	};
	for entry in entries.flatten() {
		let path = entry.path();
		// A dangling link is a fact about the collection, not a fault here.
		let meta = match fs::metadata(&path) {
			Ok(m) => m,
			Err(_) => continue,
		};
		if meta.is_dir() {
			res!(gather(&path, out));
		} else if meta.len() > 0
			&& path.extension().map(|e| e.eq_ignore_ascii_case("mkv")).unwrap_or(false)
		{
			out.push(path);
		}
	}
	Ok(())
}

/// A count from the environment, or the given default.
fn num_from_env(key: &str, or: usize) -> usize {
	match env::var(key) {
		Ok(v) => v.parse::<usize>().unwrap_or(or),
		Err(_) => or,
	}
}
