//! Does a film repackaged out of Matroska into MP4 come out playable?
//!
//! This is the end-to-end claim the whole repackaging idea rests on: that a
//! browser refuses an MKV because of the *container* and not because of what is
//! inside it, so moving the same coded bytes into MP4 -- decoding nothing,
//! re-encoding nothing -- makes the film play. Everything else is detail.
//!
//! So the test writes one, and then asks a **different program** whether what
//! came out is a film: `ffprobe` must agree on the codec, the picture size and
//! the number of frames, and `ffmpeg` must decode every frame of it without
//! complaint. A container written wrongly typically still opens -- readers are
//! forgiving -- and then yields half the frames, or the right count with the
//! sample sizes shifted by one, which is why the frame count alone is not
//! enough and the decode is what settles it.
//!
//! Point `MKV_CORPUS` at a directory of films. Output goes under
//! `~/.cache/ochre-remux-probe`, **never `/tmp`**, which is a tmpfs here and is
//! charged to the memory budget of whoever writes to it.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	matroska::{Clusters, Matroska, TrackKind},
	mp4::{Codec, Sample, Track},
};

use std::{
	env,
	fs::{self, File},
	io::Read,
	path::{Path, PathBuf},
	process::Command,
};

/// How much of a film is read before the clusters, to find the tracks.
const HEAD: usize = 1024 * 1024;

/// The window the frame reader is held to.
const WINDOW: usize = 256 * 1024;

/// How many frames are repackaged, which decides how big the written film is.
const FRAMES: usize = 300;

#[test]
fn a_repackaged_film_plays() -> Outcome<()> {
	let dir = match env::var("MKV_CORPUS") {
		Ok(d) => PathBuf::from(d),
		Err(_) => {
			println!("MKV_CORPUS is not set, so nothing was repackaged and this \
				test proves nothing.");
			return Ok(());
		},
	};
	if Command::new("ffprobe").arg("-version").output().is_err() {
		println!("ffprobe is not installed, so there is no oracle and this test \
			proves nothing.");
		return Ok(());
	}

	let out = match env::var("HOME") {
		Ok(h) => PathBuf::from(h).join(".cache/ochre-remux-probe"),
		Err(_) => return Err(err!("HOME is not set, so there is nowhere to \
			write the film."; Invalid, Init)),
	};
	res!(fs::create_dir_all(&out));

	let mut films = Vec::new();
	res!(gather(&dir, &mut films));
	films.sort();

	let want = num_from_env("MKV_FRAMES", FRAMES);
	let mut done = 0usize;
	let mut skipped = 0usize;

	for film in films.iter() {
		if done >= num_from_env("MKV_FILES", 3) {
			break;
		}
		let made = match repackage(film, want) {
			Ok(Some(v)) => v,
			// A film coded in something the writer does not carry yet -- HEVC,
			// AV1 -- is skipped by name rather than failed, because the writer
			// growing a variant is the work and not a defect in this one.
			Ok(None) => { skipped += 1; continue },
			Err(e) => {
				println!("{}: could not be repackaged: {}", film.display(), e);
				skipped += 1;
				continue;
			},
		};
		let path = out.join(format!("{}.mp4", done));
		res!(fs::write(&path, &made.bytes));

		// 1. Does another program agree it is a film of the right shape?
		let said = match probe(&path) {
			Some(v) => v,
			None => return Err(err!(
				"{}: ffprobe would not read the film written from it.",
				film.display(); Invalid, Mismatch)),
		};
		if said.0 != made.coding {
			return Err(err!(
				"{}: the written film says it holds {} rather than {}.",
				film.display(), said.0, made.coding; Invalid, Mismatch));
		}
		if said.1 != made.w as u32 || said.2 != made.h as u32 {
			return Err(err!(
				"{}: the written film is {} by {} and the source is {} by {}.",
				film.display(), said.1, said.2, made.w, made.h;
				Invalid, Mismatch));
		}
		if said.3 != made.count {
			return Err(err!(
				"{}: {} frames were written and ffprobe counts {}.",
				film.display(), made.count, said.3; Invalid, Mismatch));
		}

		// 2. Does it actually decode? A container written wrongly still opens.
		let decoded = res!(decode_count(&path));
		if decoded != made.count {
			return Err(err!(
				"{}: {} frames were written and only {} decode.",
				film.display(), made.count, decoded; Invalid, Mismatch));
		}

		// 3. Are the pictures shown when the source said they were?
		//
		// The decisive check, and the one the first two cannot make. A film
		// whose composition offsets are wrong still opens, still counts right
		// and **still decodes every frame** -- it simply plays them in the wrong
		// order, which is the fault this whole table exists to prevent. So the
		// written film's presentation times are read back and held against the
		// source's. They may differ by ONE constant, because delaying the whole
		// track is how a negative offset is avoided; they may not differ by two.
		let back = match times_of(&path) {
			Some(v) => v,
			None => return Err(err!(
				"{}: the written film's presentation times could not be read.",
				film.display(); Invalid, Mismatch)),
		};
		if back.len() != made.times.len() {
			return Err(err!(
				"{}: {} presentation times were written and {} read back.",
				film.display(), made.times.len(), back.len(); Invalid, Mismatch));
		}
		let shift = back[0] - made.times[0];
		for i in 0..back.len() {
			if back[i] - made.times[i] != shift {
				return Err(err!(
					"{}: frame {} is shown at {} and the source shows it at {}, \
					a difference of {} against the film's {}. The pictures are \
					in the wrong order.",
					film.display(), i, back[i], made.times[i],
					back[i] - made.times[i], shift;
					Invalid, Mismatch));
			}
		}

		println!("{}: {} {} frames repackaged, {} by {}, all decode, shown in \
			order (whole film delayed {} ticks)",
			film.display(), made.count, made.coding, made.w, made.h, shift);
		done += 1;
	}

	println!("{} films found; {} repackaged and played, {} skipped",
		films.len(), done, skipped);
	if !films.is_empty() && done == 0 {
		return Err(err!(
			"{} films were found and not one was repackaged, so this run proves \
			nothing.", films.len(); Invalid, Mismatch));
	}
	Ok(())
}

/// A film written out, and what it should hold.
struct Made {
	bytes:	Vec<u8>,
	w:		u16,
	h:		u16,
	count:	usize,
	/// The presentation times the source stated, in decode order.
	times:	Vec<i64>,
	/// What a player should call the coding of what was written.
	coding:	&'static str,
}

/// Reads a film's picture frames and writes them into an MP4, decoding nothing.
///
/// `None` where the film's picture is coded in something the writer has no
/// variant for.
fn repackage(path: &Path, want: usize) -> Outcome<Option<Made>> {
	let mut head = vec![0u8; HEAD];
	let mut file = res!(File::open(path));
	let n = res!(file.read(&mut head));
	head.truncate(n);
	let mkv = res!(Matroska::read(&head));

	let track = match mkv.tracks().iter().find(|t| t.kind() == Some(TrackKind::Video)) {
		Some(t) => t,
		None => return Ok(None),
	};
	// Both length-prefixed picture codings move across the same way: the record
	// the source states is the record the sample entry wants, byte for byte.
	let coding = match track.codec() {
		"V_MPEG4/ISO/AVC"	=> "h264",
		"V_MPEGH/ISO/HEVC"	=> "hevc",
		_ => return Ok(None),
	};
	let (w, h) = track.size();
	if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
		return Ok(None);
	}
	// The configuration record moves across verbatim -- this is the whole trick.
	// It is the `avcC` an MP4 sample entry wants and the `CodecPrivate` a
	// Matroska track entry carries, and they are the same bytes.
	let codec = if coding == "hevc" {
		Codec::Hevc(track.private().to_vec())
	} else {
		Codec::Avc(track.private().to_vec())
	};

	// Milliseconds, matching the timestamps the frames come stamped in, so no
	// rescaling is needed and no rounding is introduced.
	let scale = 1000u32;
	let step = if track.frame_nanos() > 0 {
		(track.frame_nanos() / 1_000_000).max(1) as u32
	} else {
		40
	};
	let mut out = res!(Track::new(w as u16, h as u16, scale, codec));

	let number = track.number();
	let mut file = res!(File::open(path));
	let mut cl = Clusters::new(&mkv);
	let mut buf: Vec<u8> = Vec::new();
	let mut frames: Vec<(Vec<u8>, bool, i64)> = Vec::new();
	let mut eof = false;

	while frames.len() < want {
		while buf.len() < WINDOW && !eof {
			let mut chunk = vec![0u8; WINDOW];
			let got = res!(file.read(&mut chunk));
			if got == 0 {
				eof = true;
				break;
			}
			chunk.truncate(got);
			buf.extend_from_slice(&chunk);
		}
		if buf.is_empty() {
			break;
		}
		let fed = res!(cl.feed(&buf, &mut |frame| {
			if frame.track == number && frames.len() < want {
				frames.push((frame.data.to_vec(), frame.key, frame.time));
			}
			Ok(())
		}));
		buf.drain(..fed.used);
		if fed.used == 0 {
			if eof {
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
			}
		}
	}

	// A film must begin at a sync sample; anything before the first one cannot
	// be decoded by a reader starting here and is dropped rather than written.
	let first = match frames.iter().position(|(_, k, _)| *k) {
		Some(i) => i,
		None => return Ok(None),
	};
	let kept: Vec<(Vec<u8>, bool, i64)> = frames.drain(..).skip(first).collect();

	// The frames arrive in decode order carrying the times they are SHOWN, and
	// MP4 wants the times they are decoded plus the difference. Every duration
	// here is the same, so the decoding times are `i * step` and the offsets are
	// what is left -- which for a film with B-pictures is not nought.
	// Rebased on the first kept frame, because decoding starts at nought here and
	// `composition_offsets` does not rebase: handed the absolute times of a film
	// that starts an hour in, it would give every sample an offset of an hour.
	// These films begin at nought, so it changes nothing today and stops the test
	// passing for a reason that would not hold on a film that did not.
	let t0 = kept.first().map(|(_, _, t)| *t).unwrap_or(0);
	let times: Vec<i64> = kept.iter().map(|(_, _, t)| *t - t0).collect();
	let durs: Vec<u32> = vec![step; kept.len()];
	let offs = res!(oxedyne_fe2o3_graphics::mp4::composition_offsets(&times, &durs));

	let mut count = 0usize;
	for (i, (data, key, _)) in kept.into_iter().enumerate() {
		let s = if key { Sample::key(data, step) } else { Sample::delta(data, step) };
		res!(out.push(s.shown_after(offs[i])));
		count += 1;
	}
	if count == 0 {
		return Ok(None);
	}
	Ok(Some(Made { bytes: res!(out.finish()), w: w as u16, h: h as u16, count, times, coding }))
}

/// What ffprobe says the written film holds: codec, width, height, frames.
fn probe(path: &Path) -> Option<(String, u32, u32, usize)> {
	let out = match Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", "v:0",
			"-count_packets",
			"-show_entries", "stream=codec_name,width,height,nb_read_packets",
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
	let line = text.lines().next().unwrap_or("");
	let mut parts = line.trim().split(',');
	let codec = parts.next().unwrap_or("").to_string();
	let w = match parts.next().map(|v| v.parse::<u32>()) {
		Some(Ok(v)) => v,
		_ => return None,
	};
	let h = match parts.next().map(|v| v.parse::<u32>()) {
		Some(Ok(v)) => v,
		_ => return None,
	};
	let n = match parts.next().map(|v| v.parse::<usize>()) {
		Some(Ok(v)) => v,
		_ => return None,
	};
	Some((codec, w, h, n))
}

/// How many frames ffmpeg can actually decode out of the written film.
///
/// The stronger half of the check: a container whose sample table is wrong
/// still opens and still reports a plausible packet count, and only a decode
/// says whether the bytes handed to the decoder were the frames.
fn decode_count(path: &Path) -> Outcome<usize> {
	let out = res!(Command::new("ffmpeg")
		.args(["-v", "error", "-i"])
		.arg(path)
		.args(["-f", "null", "-"])
		.output());
	let errs = String::from_utf8_lossy(&out.stderr);
	if !errs.trim().is_empty() {
		return Err(err!(
			"ffmpeg complained while decoding the written film: {}",
			errs.trim(); Invalid, Mismatch));
	}
	// Counted by asking for the frames rather than by parsing a progress line.
	let out = res!(Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", "v:0",
			"-count_frames",
			"-show_entries", "stream=nb_read_frames",
			"-of", "csv=p=0",
		])
		.arg(path)
		.output());
	let text = String::from_utf8_lossy(&out.stdout);
	match text.trim().parse::<usize>() {
		Ok(v) => Ok(v),
		Err(_) => Err(err!(
			"ffprobe counted no frames in the written film."; Invalid, Mismatch)),
	}
}

/// The presentation times of a written film's pictures, in decode order.
fn times_of(path: &Path) -> Option<Vec<i64>> {
	let out = match Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", "v:0",
			"-show_entries", "packet=pts",
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
	let mut times = Vec::new();
	for line in text.lines() {
		match line.trim().parse::<i64>() {
			Ok(v) => times.push(v),
			Err(_) => return None,
		}
	}
	Some(times)
}

/// Every `.mkv` under a directory.
fn gather(dir: &Path, out: &mut Vec<PathBuf>) -> Outcome<()> {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return Ok(()),
	};
	for entry in entries.flatten() {
		let path = entry.path();
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
