//! How many of a library's H.264 films can the decoder draw the first frame of?
//!
//! Two films in every five in the survey corpus are coded with the variable-length entropy coder and
//! three in five with the arithmetic one, and until both were written only the first kind could be
//! drawn at all. This asks the question of the real corpus and **reports** the answer rather than
//! asserting one, because "how many films can be drawn today" is a measurement and not a promise.
//! What it does assert is that nothing is refused silently: every refusal is counted and named, and
//! a picture of no size is a refusal rather than a success.
//!
//! There is no oracle here. Agreement with FFmpeg, sample for sample, is `h264_corpus.rs`'s job;
//! this one says how far the decoder reaches.
//!
//! ```text
//! H264_FILMS=/srv/nfs4/Gallery cargo test --release -p oxedyne_fe2o3_graphics --test h264_films \
//!     -- --nocapture
//! ```
//!
//! Absent, it says so rather than passing quietly. `H264_FILMS_MAX` caps how many films are read.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_graphics::{
	h264,
	mp4::{
		Film,
		Kind,
	},
};

use std::{
	collections::BTreeMap,
	env,
	fs,
	io::{
		Read,
		Seek,
	},
	path::PathBuf,
};

use oxedyne_fe2o3_core::prelude::*;

/// Every film under a directory, in a stable order.
fn films(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return,
	};
	let mut here = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			films(&path, out);
		} else {
			let name = path.to_string_lossy().to_lowercase();
			if name.ends_with(".mp4") || name.ends_with(".mov") || name.ends_with(".m4v") {
				here.push(path);
			}
		}
	}
	here.sort();
	out.extend(here);
}

/// Reads a film's index, and one sample of it, without holding the film.
///
/// The corpus runs to 91 gigabytes and one film in it is four gigabytes on its own, so nothing here
/// reads a whole file: the top-level boxes are walked by seeking over each one's header, the `moov`
/// box alone is read, and then one sample is read from the span the index names.
fn open(path: &std::path::Path) -> Outcome<Option<(Film, Vec<u8>)>> {
	let mut f = res!(fs::File::open(path));
	let len = res!(f.metadata()).len();
	let mut at = 0u64;
	let mut moov: Option<Vec<u8>> = None;
	while at + 8 <= len {
		res!(f.seek(std::io::SeekFrom::Start(at)));
		let mut head = [0u8; 16];
		if res!(f.read(&mut head[..8])) < 8 {
			break;
		}
		let short = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
		let (size, hlen) = match short {
			1 => {
				if res!(f.read(&mut head[8..16])) < 8 {
					break;
				}
				let mut wide = [0u8; 8];
				wide.copy_from_slice(&head[8..16]);
				(u64::from_be_bytes(wide), 16u64)
			},
			0 => (len - at, 8u64),
			n => (n as u64, 8u64),
		};
		if size < hlen || at + size > len {
			break;
		}
		if &head[4..8] == b"moov" {
			let mut body = vec![0u8; (size - hlen) as usize];
			res!(f.seek(std::io::SeekFrom::Start(at + hlen)));
			res!(f.read_exact(&mut body));
			moov = Some(body);
			break;
		}
		at += size;
	}
	let moov = match moov {
		Some(m) => m,
		None => return Ok(None),
	};
	let film = res!(Film::from_moov(&moov));
	let i = res!(film.first_sync());
	let (off, size) = res!(film.span(i));
	let mut sample = vec![0u8; size as usize];
	res!(f.seek(std::io::SeekFrom::Start(off)));
	res!(f.read_exact(&mut sample));
	Ok(Some((film, sample)))
}

/// The first sentence of a refusal, which is what distinguishes one from another.
///
/// The tail names the file, and keeping it would make every refusal look distinct and the count
/// useless.
fn why(e: &Error<ErrTag>) -> String {
	let text = fmt!("{}", e.plain());
	let head = text.split(". ").next().unwrap_or(&text).trim().to_string();
	// A message that runs over several lines in the source arrives with its indentation in it.
	head.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn how_many_films_the_decoder_can_draw() -> Outcome<()> {
	let dir = match env::var("H264_FILMS") {
		Ok(d) if !d.is_empty() => PathBuf::from(d),
		_ => {
			println!("skipped: set H264_FILMS to a directory of films");
			return Ok(());
		},
	};
	let cap = match env::var("H264_FILMS_MAX") {
		Ok(n) => res!(n.parse::<usize>()),
		Err(_) => usize::MAX,
	};

	let mut all = Vec::new();
	films(&dir, &mut all);
	println!("{} films under {}", all.len(), dir.display());

	let mut avc = 0usize;
	let mut drawn = 0usize;
	let mut cabac = 0usize;
	let mut cabac_drawn = 0usize;
	let mut refused: BTreeMap<String, usize> = BTreeMap::new();
	let mut looked = 0usize;

	for path in &all {
		if looked >= cap {
			break;
		}
		let (film, sample) = match open(path) {
			Ok(Some(f)) => f,
			Ok(None) => continue,
			Err(e) => {
				*refused.entry(why(&e)).or_insert(0) += 1;
				continue;
			},
		};
		if film.kind() != Kind::Avc {
			continue;
		}
		avc += 1;
		looked += 1;
		// Which entropy coder the film uses, which is the split this decoder was widened for. A
		// film whose parameter sets will not read at all is counted as a refusal below rather than
		// here.
		let arithmetic = match h264::config(film.config()) {
			Ok(cfg) => {
				let mut sets = Vec::new();
				for u in &cfg.sps {
					if let Ok(s) = h264::sps(&u.body) {
						sets.push(s);
					}
				}
				cfg.pps.iter()
					.filter_map(|u| h264::pps(&u.body, &sets).ok())
					.any(|p| p.cabac)
			},
			Err(_) => false,
		};
		if arithmetic {
			cabac += 1;
		}
		match h264::decode::picture(film.config(), &sample) {
			Ok(pic) => {
				// A picture, and one of the size the parameter sets promised: a decoder that
				// answered an empty frame would otherwise count as a success here.
				if pic.y.w == 0 || pic.y.h == 0 {
					*refused.entry("an empty picture".to_string()).or_insert(0) += 1;
				} else {
					drawn += 1;
					if arithmetic {
						cabac_drawn += 1;
					}
				}
			},
			Err(e) => {
				*refused.entry(why(&e)).or_insert(0) += 1;
			},
		}
	}

	println!("{} H.264 films looked at, {} first frames drawn", avc, drawn);
	if avc > 0 {
		println!("that is {:.1}% of them", 100.0 * drawn as f64 / avc as f64);
	}
	println!("{} of them are coded with the arithmetic entropy coder, {} drawn", cabac, cabac_drawn);
	println!("{} with the variable-length one, {} drawn", avc - cabac, drawn - cabac_drawn);
	// Every refusal, by name and by count. Nothing is folded into an "other": a refusal nobody can
	// read is a refusal nobody will fix.
	let mut sorted: Vec<(String, usize)> = refused.into_iter().collect();
	sorted.sort_by(|a, b| b.1.cmp(&a.1));
	println!("refused:");
	if sorted.is_empty() {
		println!("      -  nothing");
	}
	let mut total = 0usize;
	for (name, n) in &sorted {
		println!("  {:>5}  {}", n, name);
		total += n;
	}
	// The one thing asserted: every film is accounted for, so a film that vanished between the walk
	// and the count would show up here rather than as a quietly smaller total.
	req!(drawn + total, avc,
		"{} films drawn and {} refused, out of {} looked at", drawn, total, avc);
	Ok(())
}
