//! Does the Matroska header reader agree with a player about real films?
//!
//! The unit tests beside the reader build their own files, so they prove the
//! reader agrees with the author's idea of the format and nothing more. This
//! asks a different program about films nobody here wrote: point
//! `MKV_CORPUS` at a directory and every `.mkv` under it is read twice, once by
//! this crate and once by `ffprobe`, and the two answers are compared.
//!
//! Three things are compared, not one. The size and the running time are what
//! [`avi_corpus`](../avi_corpus.rs) checks, and they would pass on a reader that
//! found the picture and stopped. **The codec of every stream is checked too**,
//! because the question a film library actually asks of this reader is whether a
//! browser will play the file, and that is answered by the sound as often as by
//! the picture.
//!
//! Only the front of each file is given to the reader, and how much of a front
//! is needed is itself measured: a file that put its cover art before its
//! tracks needs more than a scanner's usual sniffing buffer, and the number of
//! files that do is the thing a scanner's buffer size should be chosen from.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::matroska::{Matroska, TrackKind};

use std::{
	collections::BTreeMap,
	env,
	fs::{self, File},
	io::Read,
	path::{Path, PathBuf},
	process::Command,
};

const HEAD: usize = 64 * 1024;	// the head a scanner would hold, and all the reader is first given

// The larger head tried where the first found no tracks. A file may carry a
// poster, a font for its subtitles, or a long seek index before the track list.
// Reading a megabyte of a four-gigabyte film is still cheap; the point of
// measuring is to find out how often it is needed.
const DEEP: usize = 1024 * 1024;

const SLACK_MS: i64 = 100;	// how far a running time may differ from the player's and still agree

/// What a Matroska codec identifier is called by the player.
///
/// The identifiers are fixed by the specification and the names are ffprobe's,
/// so this table is the translation between two vocabularies and belongs with
/// the oracle rather than in the library. Anything absent is not compared --
/// the file is still checked for its size and running time.
fn as_player_says(id: &str) -> Option<&'static str> {
	let name = match id {
		"V_MPEG4/ISO/AVC"	=> "h264",
		"V_MPEGH/ISO/HEVC"	=> "hevc",
		"V_AV1"				=> "av1",
		"V_VP8"				=> "vp8",
		"V_VP9"				=> "vp9",
		"V_MPEG4/ISO/ASP"	=> "mpeg4",
		"V_MPEG4/ISO/SP"	=> "mpeg4",
		"V_MPEG4/ISO/AP"	=> "mpeg4",
		"V_MPEG2"			=> "mpeg2video",
		"V_MPEG1"			=> "mpeg1video",
		"V_THEORA"			=> "theora",
		"A_AAC"				=> "aac",
		"A_AC3"				=> "ac3",
		"A_EAC3"			=> "eac3",
		"A_TRUEHD"			=> "truehd",
		"A_DTS"				=> "dts",
		"A_MPEG/L3"			=> "mp3",
		"A_MPEG/L2"			=> "mp2",
		"A_FLAC"			=> "flac",
		"A_OPUS"			=> "opus",
		"A_VORBIS"			=> "vorbis",
		"S_TEXT/UTF8"		=> "subrip",
		"S_TEXT/ASS"		=> "ass",
		"S_TEXT/SSA"		=> "ssa",
		"S_TEXT/WEBVTT"		=> "webvtt",
		"S_VOBSUB"			=> "dvd_subtitle",
		"S_HDMV/PGS"		=> "hdmv_pgs_subtitle",
		// `V_MS/VFW/FOURCC` names its codec in a Windows bitmap header inside
		// the private data, which is a second format and not this reader's job.
		// `A_AAC/...` profiles and anything later fall through here too.
		_ => return None,
	};
	Some(name)
}

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
		} else if path.to_string_lossy().to_lowercase().ends_with(".mkv") {
			here.push(path);
		}
	}
	here.sort();
	out.extend(here);
}

fn head_of(path: &Path, n: usize) -> Option<Vec<u8>> {
	let mut f = match File::open(path) {
		Ok(f) => f,
		Err(_) => return None,
	};
	let mut buf = vec![0u8; n];
	let mut got = 0usize;
	while got < n {
		match f.read(&mut buf[got..]) {
			Ok(0) => break,
			Ok(k) => got += k,
			Err(_) => return None,
		}
	}
	buf.truncate(got);
	Some(buf)
}

/// What the player says: the picture's size and the running time, and every
/// stream's codec in the order the file lists them.
struct Said {
	w:		u32,
	h:		u32,
	ms:		i64,
	codecs:	Vec<String>,
}

fn oracle(path: &Path) -> Option<Said> {
	let out = Command::new("ffprobe")
		.args([
			"-v", "error",
			"-show_entries", "stream=codec_type,codec_name,width,height:format=duration",
			"-of", "default=noprint_wrappers=1",
		])
		.arg(path)
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let mut said = Said { w: 0, h: 0, ms: 0, codecs: Vec::new() };
	// ffprobe prints a block per stream and then the format block. The first
	// video stream's size is the one wanted, and a later one must not replace it.
	let mut have_size = false;
	let (mut this_name, mut this_type) = (String::new(), String::new());
	let (mut this_w, mut this_h) = (0u32, 0u32);
	let flush = |name: &mut String,
		kind: &mut String,
		w: &mut u32,
		h: &mut u32,
		said: &mut Said,
		have: &mut bool|
	{
		if name.is_empty() {
			return;
		}
		if kind == "video" && !*have && *w > 0 {
			said.w = *w;
			said.h = *h;
			*have = true;
		}
		said.codecs.push(name.clone());
		name.clear();
		kind.clear();
		*w = 0;
		*h = 0;
	};
	for line in text.lines() {
		let (key, val) = match line.split_once('=') {
			Some(kv) => kv,
			None => continue,
		};
		match key.trim() {
			"codec_name" => {
				flush(&mut this_name, &mut this_type, &mut this_w, &mut this_h,
					&mut said, &mut have_size);
				this_name = val.trim().to_string();
			},
			"codec_type" => this_type = val.trim().to_string(),
			"width" => this_w = val.trim().parse().unwrap_or(0),
			"height" => this_h = val.trim().parse().unwrap_or(0),
			"duration" => {
				flush(&mut this_name, &mut this_type, &mut this_w, &mut this_h,
					&mut said, &mut have_size);
				let secs: f64 = val.trim().parse().ok()?;
				said.ms = (secs * 1000.0).round() as i64;
			},
			_ => {},
		}
	}
	flush(&mut this_name, &mut this_type, &mut this_w, &mut this_h,
		&mut said, &mut have_size);
	Some(said)
}

/// The codecs this reader found, in file order, as the player would name them.
///
/// A stream whose identifier is not in the table is `None`, which the comparison
/// steps over on both sides rather than counting as a disagreement.
fn mine_as_player_says(mkv: &Matroska) -> Vec<Option<&'static str>> {
	mkv.tracks().iter().map(|t| {
		// A track the file gave no type is not a stream a player lists either.
		match t.kind() {
			Some(TrackKind::Other(_)) | None => None,
			_ => as_player_says(t.codec()),
		}
	}).collect()
}

#[test]
fn the_header_agrees_with_ffprobe() -> Outcome<()> {
	let dir = match env::var("MKV_CORPUS") {
		Ok(d) if !d.is_empty() => PathBuf::from(d),
		_ => {
			println!("skipped: set MKV_CORPUS to a directory of films");
			return Ok(());
		},
	};
	let cap: usize = env::var("MKV_CORPUS_MAX").ok()
		.and_then(|n| n.parse().ok())
		.unwrap_or(2000);

	let mut all = Vec::new();
	films(&dir, &mut all);
	println!("{} Matroska files under {}", all.len(), dir.display());

	let (mut compared, mut agreed, mut refused, mut no_oracle) = (0usize, 0usize, 0usize, 0usize);
	let (mut needed_deep, mut no_tracks, mut unreadable) = (0usize, 0usize, 0usize);
	let mut worst_ms = 0i64;
	let mut wrong = Vec::new();
	let mut untranslated: BTreeMap<String, usize> = BTreeMap::new();
	let mut seen: BTreeMap<String, usize> = BTreeMap::new();

	for path in all.iter().take(cap) {
		// A file that will not open is counted rather than stepped over. A
		// corpus check that quietly drops what it could not read reports having
		// covered everything it was given, which is the one thing it must not do.
		let head = match head_of(path, HEAD) {
			Some(h) => h,
			None => { unreadable += 1; continue },
		};
		let mut mine = match Matroska::read(&head) {
			Ok(m) => m,
			Err(_) => { refused += 1; continue },
		};
		// The measurement the sniffing buffer should be chosen from.
		if mine.tracks().is_empty() {
			needed_deep += 1;
			match head_of(path, DEEP) {
				Some(deep) => match Matroska::read(&deep) {
					Ok(m) => mine = m,
					Err(_) => { refused += 1; continue },
				},
				None => { unreadable += 1; continue },
			}
			if mine.tracks().is_empty() {
				no_tracks += 1;
				continue;
			}
		}
		let said = match oracle(path) {
			Some(o) => o,
			None => { no_oracle += 1; continue },
		};
		compared += 1;

		for t in mine.tracks() {
			*seen.entry(t.codec().to_string()).or_insert(0) += 1;
			if as_player_says(t.codec()).is_none() {
				*untranslated.entry(t.codec().to_string()).or_insert(0) += 1;
			}
		}

		let (mw, mh) = mine.size();
		if (mw, mh) != (said.w, said.h) {
			if wrong.len() < 8 {
				wrong.push(fmt!("{}: {}x{} against {}x{}",
					path.display(), mw, mh, said.w, said.h));
			}
			continue;
		}

		match mine.millis() {
			Some(got) => {
				let off = (got as i64 - said.ms).abs();
				let slack = SLACK_MS.max(said.ms / 100);
				if off > slack {
					if wrong.len() < 8 {
						wrong.push(fmt!("{}: {} ms against {} ms",
							path.display(), got, said.ms));
					}
					continue;
				}
				worst_ms = worst_ms.max(off);
			},
			None => {
				if wrong.len() < 8 {
					wrong.push(fmt!("{}: no running time, player says {} ms",
						path.display(), said.ms));
				}
				continue;
			},
		}

		// The streams, in order, where both sides name the codec. A stream this
		// reader translates but the player never listed is a disagreement; so is
		// one whose name differs.
		let names = mine_as_player_says(&mine);
		let mut bad = None;
		let mut at = 0usize;
		for (i, name) in names.iter().enumerate() {
			let name = match name {
				Some(n) => *n,
				None => { at += 1; continue },
			};
			match said.codecs.get(at) {
				Some(theirs) if theirs == name => {},
				Some(theirs) => {
					bad = Some(fmt!("stream {} is {} and the player says {}",
						i, name, theirs));
					break;
				},
				None => {
					bad = Some(fmt!("stream {} is {} and the player listed no such stream",
						i, name));
					break;
				},
			}
			at += 1;
		}
		if let Some(why) = bad {
			if wrong.len() < 8 {
				wrong.push(fmt!("{}: {}", path.display(), why));
			}
			continue;
		}

		agreed += 1;
	}

	println!("{} compared, {} agreed on size, running time and every stream's codec",
		compared, agreed);
	println!("worst running-time difference: {} ms", worst_ms);
	println!("{} needed more than {} KB of head, {} had no track list at all",
		needed_deep, HEAD / 1024, no_tracks);
	println!("{} refused by the reader, {} ffprobe would not read, {} would not open",
		refused, no_oracle, unreadable);
	req!(compared + refused + no_oracle + unreadable + no_tracks, all.len().min(cap),
		"{} files were neither compared nor accounted for.",
		all.len().min(cap) - (compared + refused + no_oracle + unreadable + no_tracks));
	println!("codec identifiers found:");
	for (id, n) in &seen {
		println!("  {:>5}  {}{}", n, id,
			if as_player_says(id).is_none() { "  (not translated)" } else { "" });
	}
	for line in &wrong {
		println!("  {}", line);
	}

	if compared > 0 {
		req!(agreed, compared,
			"The header reader and ffprobe disagree about {} of {} films.",
			compared - agreed, compared);
	}
	// A reader that found no track list in a megabyte of every file would pass
	// every comparison above by never making one.
	req!(no_tracks, 0,
		"{} files gave up no track list even from {} KB of head.", no_tracks, DEEP / 1024);
	Ok(())
}
