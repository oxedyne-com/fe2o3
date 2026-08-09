//! Can the still-picture decoder draw a film's first frame?
//!
//! Three quarters of the films in the survey corpus are HEVC, and this crate
//! already carries a complete HEVC intra decoder written for HEIC photographs.
//! A film's first frame is an IDR coded with intra prediction only, which is the
//! same thing a photograph is -- so the question is whether the parameter sets a
//! camera writes for a film stay inside what the still decoder reads.
//!
//! This test asks that question of the real corpus and reports the answer rather
//! than asserting one, because "how many films can be drawn today" is a
//! measurement and not a promise. Point `HEVC_FILMS` at a directory of films.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	hevc,
	mp4,
};

use std::{
	collections::BTreeMap,
	env,
	fs,
	path::PathBuf,
};

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
			if name.ends_with(".mp4") || name.ends_with(".mov") {
				here.push(path);
			}
		}
	}
	here.sort();
	out.extend(here);
}

#[test]
fn how_many_films_the_still_decoder_can_already_draw() -> Outcome<()> {
	let dir = match env::var("HEVC_FILMS") {
		Ok(d) if !d.is_empty() => PathBuf::from(d),
		_ => {
			println!("skipped: set HEVC_FILMS to a directory of films");
			return Ok(());
		},
	};
	let cap = match env::var("HEVC_FILMS_MAX") {
		Ok(n) => res!(n.parse::<usize>()),
		Err(_) => 400,
	};

	let mut all = Vec::new();
	films(&dir, &mut all);
	println!("{} films under {}", all.len(), dir.display());

	let mut hevc_films = 0usize;
	let mut drawn = 0usize;
	let mut refused: BTreeMap<String, usize> = BTreeMap::new();
	let mut looked = 0usize;

	for path in &all {
		if looked >= cap {
			break;
		}
		let bytes = match fs::read(path) {
			Ok(b) => b,
			Err(_) => continue,
		};
		let film = match mp4::Film::read(&bytes) {
			Ok(f) => f,
			Err(_) => continue,
		};
		if film.kind() != mp4::Kind::Hevc {
			continue;
		}
		hevc_films += 1;
		looked += 1;
		if film.samples() == 0 {
			*refused.entry("the track holds no sample".to_string()).or_insert(0) += 1;
			continue;
		}
		let sample = match film.sample(&bytes, 0) {
			Ok(s) => s,
			Err(e) => {
				*refused.entry(fmt!("{}", e.plain())).or_insert(0) += 1;
				continue;
			},
		};
		match hevc::picture(film.config(), sample) {
			Ok(pic) => {
				// A picture, and one the right size: a decoder that answered an
				// empty frame would otherwise count as a success here.
				let (w, h) = (pic.y.w, pic.y.h);
				if w == 0 || h == 0 {
					*refused.entry("an empty picture".to_string()).or_insert(0) += 1;
				} else {
					drawn += 1;
				}
			},
			Err(e) => {
				// The first line only: the tail names the file and would make
				// every refusal look distinct.
				let why = e.plain();
				let head = why.split(". ").next().unwrap_or(&why).to_string();
				*refused.entry(head).or_insert(0) += 1;
			},
		}
	}

	println!("{} HEVC films looked at, {} first frames drawn", hevc_films, drawn);
	if hevc_films > 0 {
		println!("that is {:.1}% of them", 100.0 * drawn as f64 / hevc_films as f64);
	}
	println!("refused:");
	let mut sorted = refused.into_iter().collect::<Vec<_>>();
	sorted.sort_by(|a, b| b.1.cmp(&a.1));
	for (why, n) in sorted.iter().take(12) {
		println!("  {:>5}  {}", n, why);
	}
	Ok(())
}
