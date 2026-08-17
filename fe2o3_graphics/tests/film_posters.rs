//! How many of a real library's films can this crate draw a poster frame for?
//!
//! A film's first frame is an IDR coded with intra prediction only, which is the same thing a
//! photograph is, and this crate carries an intra decoder for each of the two codecs a household's
//! films are written in. So a poster costs the container's metadata, one sample and one decode --
//! and the question worth asking of a real library is how many of its films that actually works for.
//!
//! Nothing here holds a film. The corpus runs to hundreds of gigabytes and single films in it are
//! larger than any sensible buffer, so `mp4::moov_of` lifts the index out by walking the top-level
//! box headers and `Film::read_sample` fetches the one sample that holds the first frame.
//!
//! Point `FILM_POSTERS` at a directory of films to run these:
//!
//! ```text
//! FILM_POSTERS=/srv/nfs4/Gallery cargo test --release -p oxedyne_fe2o3_graphics \
//!     --test film_posters -- --nocapture
//! ```
//!
//! `FILM_POSTERS_MAX` caps how many films are looked at. `FILM_POSTERS_ORACLE` says how many of
//! them are held to FFmpeg, sample for sample, which costs an FFmpeg process each and is therefore
//! a sample of the corpus rather than the whole of it.
//!
//! The first test **reports rather than asserts**: how many films can be drawn today is a
//! measurement and not a promise, and a number that is asserted stops being measured. The tests
//! after it assert, because agreeing with another decoder is not a matter of degree.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_graphics::{
	h264,
	hevc,
	jpeg,
	mp4::{
		self,
		Film,
		Kind,
	},
	pixmap::Pixmap,
	yuv,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::BTreeMap,
	env,
	fs::File,
	path::{
		Path,
		PathBuf,
	},
};

/// Every film under a directory, in a stable order.
fn films() -> Outcome<Option<Vec<PathBuf>>> {
	let root = match env::var("FILM_POSTERS") {
		Ok(d) if !d.is_empty() => PathBuf::from(d),
		_ => {
			println!("  skipped: set FILM_POSTERS to a directory of films");
			return Ok(None);
		},
	};
	let mut out = Vec::new();
	let mut stack = vec![root];
	while let Some(dir) = stack.pop() {
		let entries = match std::fs::read_dir(&dir) {
			Ok(e) => e,
			Err(_) => continue,
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else {
				let ext = path.extension()
					.map(|e| e.to_string_lossy().to_lowercase())
					.unwrap_or_default();
				if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
					out.push(path);
				}
			}
		}
	}
	out.sort();
	if let Ok(n) = env::var("FILM_POSTERS_MAX") {
		if let Ok(n) = n.parse::<usize>() {
			out.truncate(n);
		}
	}
	Ok(Some(out))
}

/// A film's index and the bytes of its first frame, holding none of the film itself.
fn first_frame(path: &Path) -> Outcome<(Film, Vec<u8>)> {
	let mut f = res!(File::open(path));
	let film = res!(Film::of(&mut f));
	let sample = res!(film.read_first_sync(&mut f));
	Ok((film, sample))
}

/// The picture a film's first frame holds, as its codec asks for it.
///
/// Both decoders answer the same planar picture -- `yuv::Frame` -- and each hands back the size the
/// stream says is to be shown rather than the size it was coded at.
fn decode(film: &Film, sample: &[u8]) -> Outcome<yuv::Frame> {
	let pic = match film.kind() {
		Kind::Hevc => res!(hevc::picture_shown(film.config(), sample)),
		Kind::Avc => (&res!(h264::decode::picture(film.config(), sample))).into(),
		Kind::Mjpeg => {
			// A Motion JPEG sample is a whole JPEG, which is a different decoder and a picture
			// that arrives already in red, green and blue. It has no place in a planar
			// comparison, so this says so rather than pretending.
			let _ = res!(jpeg::decode(sample));
			return Err(err!("A Motion JPEG frame is not a planar picture."; Unimplemented));
		},
		Kind::Other(code) => return Err(err!(
			"A film coded as {}, which there is no decoder for.", String::from_utf8_lossy(&code);
		Unimplemented)),
	};
	// And then the window the container says is the picture, where it says so.
	Ok(match film.aperture() {
		Some((x, y, w, h)) => pic.window(x as usize, y as usize, w as usize, h as usize),
		None => pic,
	})
}

/// The first sentence of a refusal, which is the part that names the reason rather than the file.
fn why(e: &Error<ErrTag>) -> String {
	let text = e.plain();
	match text.split_once(". ") {
		Some((first, _)) => first.to_string(),
		None => text,
	}
}

#[test]
fn test_how_many_films_get_a_poster_00() -> Outcome<()> {
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	println!("  {} films", films.len());
	let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
	let mut drawn_by_kind: BTreeMap<String, usize> = BTreeMap::new();
	let mut refused: BTreeMap<String, usize> = BTreeMap::new();
	let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
	let (mut looked, mut drawn, mut turned) = (0usize, 0usize, 0usize);

	let began = std::time::Instant::now();
	for path in &films {
		looked += 1;
		// A run over thousands of films is minutes long, and a test that says nothing until it
		// ends is a test nobody can tell from a hung one.
		if looked % 250 == 0 {
			println!("    {} of {} looked at, {} drawn, {:.1}/s", looked, films.len(), drawn,
				looked as f64 / began.elapsed().as_secs_f64().max(0.001));
		}
		let (film, sample) = match first_frame(path) {
			Ok(f) => f,
			Err(e) => {
				let head = why(&e);
				*refused.entry(head.clone()).or_insert(0) += 1;
				named.entry(head).or_default().push(name(path));
				continue;
			},
		};
		let kind = match film.kind() {
			Kind::Hevc => "hevc".to_string(),
			Kind::Avc => "avc".to_string(),
			Kind::Mjpeg => "mjpeg".to_string(),
			Kind::Other(code) => fmt!("{}", String::from_utf8_lossy(&code)),
		};
		*by_kind.entry(kind.clone()).or_insert(0) += 1;
		if film.rotation() != 0 {
			turned += 1;
		}
		// Motion JPEG is drawn by the JPEG decoder rather than by either video one, so it counts
		// as a poster here: what is being measured is which films get a picture.
		let made: Outcome<(usize, usize)> = if film.kind() == Kind::Mjpeg {
			match jpeg::decode(&sample) {
				Ok(pm) => Ok((pm.width(), pm.height())),
				Err(e) => Err(e),
			}
		} else {
			match decode(&film, &sample) {
				Ok(pic) => Ok((pic.y.w, pic.y.h)),
				Err(e) => Err(e),
			}
		};
		match made {
			Ok((w, h)) if w > 0 && h > 0 => {
				drawn += 1;
				*drawn_by_kind.entry(kind).or_insert(0) += 1;
			},
			Ok((w, h)) => {
				let head = fmt!("A picture of {} by {} has nothing in it", w, h);
				*refused.entry(head.clone()).or_insert(0) += 1;
				named.entry(head).or_default().push(name(path));
			},
			Err(e) => {
				let head = why(&e);
				*refused.entry(head.clone()).or_insert(0) += 1;
				named.entry(head).or_default().push(name(path));
			},
		}
	}

	println!("  {} films looked at, {} posters drawn", looked, drawn);
	if looked > 0 {
		println!("  that is {:.1}% of them", 100.0 * drawn as f64 / looked as f64);
	}
	println!("  {} of them carry a rotation in the track header", turned);
	println!("  by codec:");
	for (kind, n) in &by_kind {
		println!("    {:>6}  {:>5} films, {:>5} drawn", kind, n,
			drawn_by_kind.get(kind).copied().unwrap_or(0));
	}
	println!("  refused:");
	let mut sorted: Vec<(String, usize)> = refused.into_iter().collect();
	sorted.sort_by(|a, b| b.1.cmp(&a.1));
	for (head, n) in &sorted {
		println!("    {:>5}  {}", n, head);
		// Named, because a count says how many and a name says which -- and the next fault is
		// found by opening one of them.
		if let Some(list) = named.get(head) {
			for one in list.iter().take(4) {
				println!("             {}", one);
			}
			if list.len() > 4 {
				println!("             ... and {} more", list.len() - 4);
			}
		}
	}
	// The one thing asserted: that the chain works at all. A run where nothing was drawn has
	// measured nothing, and a broken reader would otherwise report a tidy zero per cent.
	if looked > 0 {
		let any = drawn > 0;
		req!(any, true, "not one film of {} gave up a frame", looked);
	}
	Ok(())
}

fn name(path: &Path) -> String {
	path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// What FFmpeg makes of a film's first frame, in the planar form the decoders answer.
///
/// **`-noautorotate` is load-bearing.** A phone writes the angle it was held at into the track
/// header, and FFmpeg turns the picture on the way out. A decoder answers the picture as it was
/// *coded*, and the turn is the container's business, so without this the two are compared after
/// one of them has been turned -- and at ninety degrees the byte count is unchanged, so nothing but
/// the samples says so.
fn ffmpeg_planes(path: &Path, rotate: bool) -> Outcome<Vec<u8>> {
	let mut cmd = std::process::Command::new("ffmpeg");
	cmd.args(["-v", "error"]);
	if !rotate {
		cmd.arg("-noautorotate");
	}
	cmd.arg("-i").arg(path)
		.args(["-map", "0:v:0", "-frames:v", "1", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-"]);
	let out = res!(cmd.output());
	if out.stdout.is_empty() {
		return Err(err!(
			"FFmpeg decoded no frame of {:?}: {}", path, String::from_utf8_lossy(&out.stderr);
		Test, Missing));
	}
	Ok(out.stdout)
}

/// How many samples differ and by how much at worst.
fn compare(mine: &[u16], theirs: &[u8]) -> (usize, i32) {
	let mut wrong = 0usize;
	let mut worst = 0i32;
	for (a, b) in mine.iter().zip(theirs.iter()) {
		let d = (*a as i32 - *b as i32).abs();
		if d != 0 {
			wrong += 1;
			worst = worst.max(d);
		}
	}
	(wrong, worst)
}

#[test]
fn test_a_sample_of_posters_matches_ffmpeg_01() -> Outcome<()> {
	// The check that matters. Every table, every prediction mode, every shift in an inverse
	// transform and every threshold in a loop filter has to be right at once or some sample
	// somewhere differs -- and a decoder that is nearly right produces a picture that looks
	// decoded, so nothing short of sample-for-sample agreement is evidence of anything.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let want: usize = match env::var("FILM_POSTERS_ORACLE") {
		Ok(n) => res!(n.parse()),
		Err(_) => 40,
	};
	if films.is_empty() || want == 0 {
		return Ok(());
	}
	// Spread across the corpus rather than taken from its front: the front of this tree is one
	// import from one phone, and a sample of that says nothing about the twenty years behind it.
	let stride = (films.len() / want).max(1);
	let mut exact = 0usize;
	let mut compared = 0usize;
	let mut differ: Vec<String> = Vec::new();
	let mut refused: BTreeMap<String, usize> = BTreeMap::new();
	let mut worst_seen = 0i32;
	for path in films.iter().step_by(stride).take(want) {
		let (film, sample) = match first_frame(path) {
			Ok(f) => f,
			Err(e) => {
				*refused.entry(why(&e)).or_insert(0) += 1;
				continue;
			},
		};
		if matches!(film.kind(), Kind::Mjpeg | Kind::Other(_)) {
			continue;
		}
		let mine = match decode(&film, &sample) {
			Ok(p) => p,
			Err(e) => {
				*refused.entry(why(&e)).or_insert(0) += 1;
				continue;
			},
		};
		let theirs = res!(ffmpeg_planes(path, false));
		let (w, h) = (mine.y.w, mine.y.h);
		let luma = w * h;
		let chroma = w.div_ceil(2) * h.div_ceil(2);
		if theirs.len() != luma + 2 * chroma {
			differ.push(fmt!(
				"{}: this decoder made {} by {} and FFmpeg gave {} bytes, which is not {} plus \
				two of {}", name(path), w, h, theirs.len(), luma, chroma));
			continue;
		}
		compared += 1;
		let (wy, dy) = compare(&mine.y.px, &theirs[..luma]);
		let (wu, du) = compare(&mine.cb.px, &theirs[luma..luma + chroma]);
		let (wv, dv) = compare(&mine.cr.px, &theirs[luma + chroma..]);
		worst_seen = worst_seen.max(dy).max(du).max(dv);
		if wy == 0 && wu == 0 && wv == 0 {
			exact += 1;
		} else {
			differ.push(fmt!(
				"{} {}x{} {:?}: luma {} wrong (worst {}), Cb {} (worst {}), Cr {} (worst {})",
				name(path), w, h, film.kind(), wy, dy, wu, du, wv, dv));
		}
	}
	println!("  {} posters held to FFmpeg, {} matched sample for sample", compared, exact);
	println!("  worst difference anywhere in the sample: {}", worst_seen);
	if !refused.is_empty() {
		println!("  refused before any comparison:");
		for (head, n) in &refused {
			println!("    {:>5}  {}", n, head);
		}
	}
	for line in &differ {
		println!("  DIFFERS {}", line);
	}
	if !differ.is_empty() {
		return Err(err!(
			"{} of {} posters decoded to something other than what FFmpeg decoded. A picture that \
			is nearly right is a wrong picture.", differ.len(), compared; Test, Mismatch));
	}
	if compared == 0 {
		println!("  nothing was comparable, so nothing was proved");
	}
	Ok(())
}

#[test]
fn test_a_turned_film_says_so_and_ffmpeg_agrees_02() -> Outcome<()> {
	// A rotation applied nowhere is invisible in a test of a film with a unity matrix, and at
	// ninety degrees the turned picture has exactly as many samples as the untuned one -- so this
	// finds a film the container says is turned and asks FFmpeg, twice, whether it agrees: once
	// with the turn suppressed, where the frame must come out as it was coded, and once with the
	// turn applied, where its two dimensions must be exchanged.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let mut found = None;
	let mut turns: BTreeMap<u16, usize> = BTreeMap::new();
	for path in &films {
		let (film, sample) = match first_frame(path) {
			Ok(f) => f,
			Err(_) => continue,
		};
		*turns.entry(film.rotation()).or_insert(0) += 1;
		if film.rotation() == 90 && found.is_none() && !matches!(film.kind(), Kind::Other(_)) {
			if let Ok(pic) = decode(&film, &sample) {
				found = Some((path.clone(), film, pic));
			}
		}
	}
	println!("  rotations across the corpus: {:?}", turns);
	let (path, film, pic) = match found {
		Some(f) => f,
		None => {
			println!("  no film in this corpus is turned a quarter, so nothing was proved");
			return Ok(());
		},
	};
	println!("  {} is turned {} degrees, coded {} by {}",
		name(&path), film.rotation(), pic.y.w, pic.y.h);

	// Coded: the decoder's picture and FFmpeg's untuned one are the same shape and the same
	// samples.
	let coded = res!(ffmpeg_planes(&path, false));
	let luma = pic.y.w * pic.y.h;
	let chroma = pic.y.w.div_ceil(2) * pic.y.h.div_ceil(2);
	req!(coded.len(), luma + 2 * chroma,
		"FFmpeg's untuned frame is not the shape this decoder's is");
	let (wrong, worst) = compare(&pic.y.px, &coded[..luma]);
	req!(wrong, 0usize, "the coded frame differs from FFmpeg's by up to {}", worst);

	// Shown: FFmpeg's turned frame has the two dimensions exchanged, which is what the turn a
	// viewer applies has to do.
	let shown = res!(ffmpeg_planes(&path, true));
	let turned_luma = pic.y.h * pic.y.w;
	let turned_chroma = pic.y.h.div_ceil(2) * pic.y.w.div_ceil(2);
	req!(shown.len(), turned_luma + 2 * turned_chroma,
		"FFmpeg's turned frame is not the size a quarter turn makes");
	// And the turn is a transposition and not a copy: the sample at (x, y) of the coded frame is
	// the one at (h - 1 - y, x) of the turned frame, which is what a quarter turn clockwise means.
	let (w, h) = (pic.y.w, pic.y.h);
	let mut checked = 0usize;
	for y in (0..h).step_by((h / 16).max(1)) {
		for x in (0..w).step_by((w / 16).max(1)) {
			let mine = pic.y.px[y * w + x];
			let theirs = shown[x * h + (h - 1 - y)];
			req!(mine, theirs as u16,
				"at ({}, {}) the turned frame is not the coded one transposed", x, y);
			checked += 1;
		}
	}
	println!("  {} sampled positions agree after the quarter turn", checked);
	Ok(())
}

#[test]
fn test_a_poster_can_be_drawn_in_colour_03() -> Outcome<()> {
	// The last step of the chain, which the planar comparison above does not reach: the picture
	// becomes red, green and blue, at the size the film says it is shown at.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	for path in films.iter().take(200) {
		let (film, sample) = match first_frame(path) {
			Ok(f) => f,
			Err(_) => continue,
		};
		let pic = match decode(&film, &sample) {
			Ok(p) => p,
			Err(_) => continue,
		};
		let px: Pixmap = res!(yuv::rgb(&pic, yuv::Matrix::Hd, false));
		req!(px.width(), pic.y.w);
		req!(px.height(), pic.y.h);
		// A picture, not a flat field: a decoder answering one colour everywhere would satisfy
		// every dimension check there is.
		let first = &px.data()[..4];
		let differs = px.data().chunks(4).any(|p| p != first);
		req!(differs, true, "{} came out one flat colour", name(path));
		println!("  {} drew {} by {} in colour", name(path), px.width(), px.height());
		return Ok(());
	}
	println!("  no film in this corpus gave up a frame, so nothing was proved");
	Ok(())
}

#[test]
fn test_zz_a_window_onto_one_film_05() -> Outcome<()> {
	// Not a check but a window. `FILM_POSTERS_DUMP` names one film and this writes its first
	// frame's brightness plane beside FFmpeg's, so that a disagreement can be looked at rather
	// than counted: a picture decoded from the wrong sample, one shifted by a few rows and one
	// quantised against the wrong weights are indistinguishable in a difference count and obvious
	// side by side.
	let one = match env::var("FILM_POSTERS_DUMP") {
		Ok(p) => PathBuf::from(p),
		Err(_) => return Ok(()),
	};
	let (film, sample) = res!(first_frame(&one));
	// The parameter sets and the slice header as this crate reads them, to be held beside what
	// FFmpeg's `trace_headers` prints for the same bytes.
	if film.kind() == Kind::Hevc {
		let cfg = res!(hevc::config(film.config()));
		for unit in &cfg.sets {
			match unit.kind {
				hevc::nal::SPS => println!("  sps: {:?}", res!(hevc::sps(&unit.body))),
				hevc::nal::PPS => println!("  pps: {:?}", res!(hevc::pps(&unit.body))),
				_ => {},
			}
		}
		let units = res!(hevc::split_lengthed(&sample, cfg.length_size));
		let mut seqs = Vec::new();
		let mut pics = Vec::new();
		for unit in &cfg.sets {
			match unit.kind {
				hevc::nal::SPS => seqs.push(res!(hevc::sps(&unit.body))),
				hevc::nal::PPS => pics.push(res!(hevc::pps(&unit.body))),
				_ => {},
			}
		}
		for unit in &units {
			println!("  nal {} of {} bytes", unit.kind, unit.body.len());
			if matches!(unit.kind, 19 | 20 | 21) {
				let want = res!(hevc::slice_pps_id(&unit.body));
				if let Some(pps) = pics.iter().find(|p| p.id == want) {
					if let Some(sps) = seqs.iter().find(|s| s.id == pps.sps_id) {
						println!("    slice: {:?}", res!(hevc::slice_of(unit.kind, &unit.body, sps, pps)));
					}
				}
			}
		}
	}
	let pic = res!(decode(&film, &sample));
	println!("  {} decoded {} by {}", name(&one), pic.y.w, pic.y.h);
	let mine: Vec<u8> = pic.y.px.iter().map(|v| *v as u8).collect();
	let out = std::env::temp_dir().join("film_poster_luma.gray");
	res!(std::fs::write(&out, &mine));
	println!("  wrote {} bytes of luma to {:?}", mine.len(), out);
	Ok(())
}

#[test]
fn test_the_index_is_read_without_holding_the_film_04() -> Outcome<()> {
	// The property the whole pass depends on: a film larger than any buffer a scan will allocate
	// still gives up its metadata and one sample. Held to the file's own size, so a reader that
	// quietly read the whole thing would have to have read more than this asserts.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let mut biggest: Option<(PathBuf, u64)> = None;
	for path in &films {
		if let Ok(meta) = std::fs::metadata(path) {
			let len = meta.len();
			if biggest.as_ref().map(|(_, b)| len > *b).unwrap_or(true) {
				biggest = Some((path.clone(), len));
			}
		}
	}
	let (path, len) = match biggest {
		Some(b) => b,
		None => return Ok(()),
	};
	println!("  the largest film here is {} at {} bytes", name(&path), len);
	let mut f = res!(File::open(&path));
	let moov = match res!(mp4::moov_of(&mut f)) {
		Some(m) => m,
		None => {
			println!("  it carries no movie box, so nothing was proved");
			return Ok(());
		},
	};
	let film = res!(Film::from_moov(&moov));
	let i = res!(film.first_sync());
	let (_off, size) = res!(film.span(i));
	let read = moov.len() as u64 + size as u64;
	println!("  its index is {} bytes and its first sample {}, which is {:.2}% of the file",
		moov.len(), size, 100.0 * read as f64 / len.max(1) as f64);
	let part = read < len;
	req!(part, true, "reading the index and one sample read the whole film");
	Ok(())
}
