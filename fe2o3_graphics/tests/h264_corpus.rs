//! Hold the H.264 reader to a library of real films, and to FFmpeg.
//!
//! Nothing here uses a fixture. The films are somebody's actual photograph library, written by a
//! dozen different phones and cameras over twenty years, and FFmpeg -- which has no idea this crate
//! exists -- is the oracle for every number.
//!
//! Point `H264_FILMS` at a directory of films to run it:
//!
//! ```text
//! H264_FILMS=/srv/nfs4/Gallery cargo test --release -p oxedyne_fe2o3_graphics --test h264_corpus \
//!     -- --nocapture
//! ```
//!
//! Absent, each test says so rather than passing quietly: a check that skipped in silence would be
//! a check nobody ran.
//!
//! `H264_LIMIT` caps how many films are read, for a quick pass; `H264_ONE` names a single file.

use oxedyne_fe2o3_graphics::{
	h264,
	mp4::{
		Film,
		Kind,
	},
};

use std::io::{
	Read,
	Seek,
};

use oxedyne_fe2o3_core::prelude::*;

/// Unwraps an outcome, naming the film it came from.
///
/// `res!` carries error tags and not a message, and over a corpus of thousands of files the one
/// thing a failure must say is *which file*, so this says it.
macro_rules! at {
	($r:expr, $p:expr) => {
		match $r {
			Ok(v) => v,
			Err(e) => return Err(err!("{:?}: {}", $p, e; Test, Invalid)),
		}
	};
}

/// Every film under the corpus directory, in a stable order.
fn films() -> Outcome<Option<Vec<std::path::PathBuf>>> {
	let root = match std::env::var("H264_FILMS") {
		Ok(p) => p,
		Err(_) => {
			println!("  skipped: set H264_FILMS to a directory of films");
			return Ok(None);
		},
	};
	if let Ok(one) = std::env::var("H264_ONE") {
		return Ok(Some(vec![std::path::PathBuf::from(one)]));
	}
	let mut out = Vec::new();
	let mut stack = vec![std::path::PathBuf::from(&root)];
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
	if let Ok(n) = std::env::var("H264_LIMIT") {
		if let Ok(n) = n.parse::<usize>() {
			out.truncate(n);
		}
	}
	Ok(Some(out))
}


/// Reads a film's index, and one sample of it, without holding the film.
///
/// The corpus runs to 91 gigabytes and one film in it is four gigabytes on its own, so nothing here
/// reads a whole file. The top-level boxes are walked by seeking over each one's header, the `moov`
/// box alone is read, and then one sample is read from the span the index names. That is also the
/// shape a photograph library wants: a thumbnail should cost the metadata and one frame.
fn open(path: &std::path::Path) -> Outcome<Option<(Film, Vec<u8>)>> {
	let mut f = res!(std::fs::File::open(path));
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

/// What FFmpeg's own header tracer says about a film's parameter sets.
///
/// `trace_headers` prints every syntax element of the sequence and picture parameter sets by name,
/// which is a wholly separate parse of the same bytes by a wholly separate implementation. That is
/// the point: a parser checked against itself proves nothing.
fn ffmpeg_headers(path: &std::path::Path) -> Outcome<std::collections::HashMap<String, i64>> {
	let out = res!(std::process::Command::new("ffmpeg")
		.args(["-v", "trace", "-i"])
		.arg(path)
		.args(["-map", "0:v:0", "-c:v", "copy", "-bsf:v", "trace_headers", "-frames:v", "1",
			"-f", "null", "-"])
		.output());
	let text = String::from_utf8_lossy(&out.stderr);
	let mut got = std::collections::HashMap::new();
	for line in text.lines() {
		// Each traced field is `<bit position> <name> <bits> = <value>`, and the first occurrence
		// of a name is the one in the first parameter set of its kind.
		// A traced field reads `[trace_headers @ 0x..] <bit> <name> <bits> = <value>`, so the
		// name is the token before the bit pattern on the left of the equals sign.
		let (left, value) = match line.split_once(" = ") {
			Some((l, v)) => (l, v.trim()),
			None => continue,
		};
		let mut tail = left.split_whitespace().rev();
		let _bits = tail.next();
		let name = match tail.next() {
			Some(n) => n,
			None => continue,
		};
		if let Ok(v) = value.parse::<i64>() {
			got.entry(name.to_string()).or_insert(v);
		}
	}
	Ok(got)
}

#[test]
fn test_every_parameter_set_reads_as_ffmpeg_reads_it_01() -> Outcome<()> {
	// The whole header layer, held to a separate implementation over the whole corpus. A field read
	// one bit out of place still yields a plausible number, and the only way to catch that is to
	// ask somebody else what the number is.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let mut avc = 0usize;
	let mut other = 0usize;
	let mut checked = 0usize;
	for path in &films {
		let (film, sample) = match at!(open(path), path) {
			Some(f) => f,
			None => continue,
		};
		let _ = &sample;
		if film.kind() != Kind::Avc {
			other += 1;
			continue;
		}
		avc += 1;
		let cfg = at!(h264::config(film.config()), path);
		let mut sets = Vec::new();
		for u in &cfg.sps {
			sets.push(at!(h264::sps(&u.body), path));
		}
		let mut pics = Vec::new();
		for u in &cfg.pps {
			pics.push(at!(h264::pps(&u.body, &sets), path));
		}
		let (s, p) = match (sets.first(), pics.first()) {
			(Some(s), Some(p)) => (s, p),
			_ => return Err(err!(
				"{:?} carries {} sequence and {} picture parameter sets, and a film needs one of \
				each.", path, sets.len(), pics.len(); Test, Invalid)),
		};
		let ff = res!(ffmpeg_headers(path));
		if ff.is_empty() {
			return Err(err!(
				"FFmpeg traced no headers for {:?}, so there is nothing to check against.", path;
			Test, Missing));
		}
		// Every field both implementations name, what each of ours holds, and -- for the fields a
		// parameter set may leave out -- what §7.4.2.1.1 says is inferred when it does. A field
		// FFmpeg does not trace is a field the set did not carry, and this decoder must then be
		// holding the inferred value rather than a value of its own.
		let mine: Vec<(&str, i64, Option<i64>)> = vec![
			("profile_idc",			s.profile as i64,			None),
			("level_idc",			s.level as i64,				None),
			("chroma_format_idc",		s.chroma as i64,			Some(1)),
			("bit_depth_luma_minus8",	s.luma_bits as i64 - 8,			Some(0)),
			("bit_depth_chroma_minus8",	s.chroma_bits as i64 - 8,		Some(0)),
			("log2_max_frame_num_minus4",	s.frame_num_bits as i64 - 4,		None),
			("pic_order_cnt_type",		s.poc_type as i64,			None),
			("pic_width_in_mbs_minus1",	s.mbs_w as i64 - 1,			None),
			("pic_height_in_map_units_minus1", s.map_units_h as i64 - 1,		None),
			("frame_mbs_only_flag",		s.frame_mbs_only as i64,		None),
			("entropy_coding_mode_flag",	p.cabac as i64,				None),
			("num_slice_groups_minus1",	p.slice_groups as i64 - 1,		None),
			("pic_init_qp_minus26",		p.init_qp as i64 - 26,			None),
			("chroma_qp_index_offset",	p.cb_qp_offset as i64,			None),
			("constrained_intra_pred_flag",	p.constrained_intra as i64,		None),
			("deblocking_filter_control_present_flag", p.deblocking_control as i64,	None),
			("transform_8x8_mode_flag",	p.transform_8x8 as i64,			Some(0)),
			("second_chroma_qp_index_offset", p.cr_qp_offset as i64,	Some(p.cb_qp_offset as i64)),
		];
		for (name, held, inferred) in mine {
			let theirs = match (ff.get(name), inferred) {
				(Some(v), _) => *v,
				(None, Some(d)) if held == d => continue,
				(None, Some(d)) => return Err(err!(
					"{:?}: {} is absent from the set, so it is inferred as {}, and this decoder \
					holds {}.", path, name, d, held; Test, Mismatch)),
				(None, None) => return Err(err!(
					"{:?}: this decoder reads {} as {} and FFmpeg does not name it at all.",
					path, name, held; Test, Mismatch)),
			};
			if theirs != held {
				return Err(err!(
					"{:?}: this decoder reads {} as {} and FFmpeg reads it as {}.",
					path, name, held, theirs; Test, Mismatch));
			}
			checked += 1;
		}
	}
	println!("  {} H.264 films, {} fields agreed with FFmpeg; {} films of other codecs",
		avc, checked, other);
	if avc == 0 {
		return Err(err!("Not one H.264 film was found, so nothing was checked."; Test, Missing));
	}
	Ok(())
}

#[test]
fn test_every_film_gives_up_its_first_sample_02() -> Outcome<()> {
	// The container half. A sample table read wrongly hands the decoder somebody else's bytes, and
	// the check that catches it is that the first sample begins with a NAL unit of the type the
	// stream says it should.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let mut counts = std::collections::BTreeMap::new();
	let mut idrs = 0usize;
	for path in &films {
		let (film, sample) = match at!(open(path), path) {
			Some(f) => f,
			None => {
				*counts.entry(fmt!("no video track")).or_insert(0usize) += 1;
				continue;
			},
		};
		*counts.entry(fmt!("{:?}", film.kind())).or_insert(0usize) += 1;
		if film.kind() != Kind::Avc {
			continue;
		}
		let cfg = at!(h264::config(film.config()), path);
		let units = at!(h264::split_lengthed(&sample, cfg.length_size), path);
		let slices: Vec<&h264::Unit> = units.iter()
			.filter(|u| matches!(u.kind, h264::nal::SLICE | h264::nal::IDR))
			.collect();
		if slices.is_empty() {
			return Err(err!(
				"{:?}: the first sample holds no coded slice, only NAL units {:?}.",
				path, units.iter().map(|u| u.kind).collect::<Vec<_>>(); Test, Invalid));
		}
		if slices.iter().all(|u| u.kind == h264::nal::IDR) {
			idrs += 1;
		}
	}
	println!("  {:?}; {} films whose first sample is an IDR", counts, idrs);
	Ok(())
}

#[test]
fn test_every_first_slice_header_reads_03() -> Outcome<()> {
	// The slice header is where a misread costs most: it ends at the first bit of the entropy-coded
	// data, so a field read one bit out puts the whole picture's decode one bit out. There is no
	// oracle for its length in FFmpeg's trace, so what is asserted here is what can be: that every
	// header in the corpus reads to completion, that every slice is intra, and that the slices of a
	// picture between them cover every macroblock in it exactly once.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let mut headers = 0usize;
	let mut multi = 0usize;
	for path in &films {
		let (film, sample) = match at!(open(path), path) {
			Some(f) => f,
			None => continue,
		};
		if film.kind() != Kind::Avc {
			continue;
		}
		let cfg = at!(h264::config(film.config()), path);
		let mut sets = Vec::new();
		for u in &cfg.sps {
			sets.push(at!(h264::sps(&u.body), path));
		}
		let mut pics = Vec::new();
		for u in &cfg.pps {
			pics.push(at!(h264::pps(&u.body, &sets), path));
		}
		let units = at!(h264::split_lengthed(&sample, cfg.length_size), path);
		let mut firsts = Vec::new();
		for u in &units {
			// A sample may carry its own parameter sets, which override the record's.
			match u.kind {
				h264::nal::SPS => {
					let s = at!(h264::sps(&u.body), path);
					sets.retain(|o| o.id != s.id);
					sets.push(s);
				},
				h264::nal::PPS => {
					let p = at!(h264::pps(&u.body, &sets), path);
					pics.retain(|o| o.id != p.id);
					pics.push(p);
				},
				h264::nal::SLICE | h264::nal::IDR => {
					let sh = at!(h264::slice(u, &sets, &pics), path);
					let intra = sh.kind.is_intra();
					req!(intra, true, "{:?} opens with a {:?} slice", path, sh.kind);
					firsts.push(sh.first_mb);
					headers += 1;
				},
				_ => {},
			}
		}
		if firsts.len() > 1 {
			multi += 1;
		}
		// The slices must tile the picture: the first begins at macroblock nought, and each one
		// after it begins after the one before. A header read one field out of place shows up here
		// as a first macroblock that is enormous or out of order.
		let sps = match sets.first() {
			Some(s) => s,
			None => continue,
		};
		let total = sps.mbs_w * sps.map_units_h;
		let mut sorted = firsts.clone();
		sorted.sort_unstable();
		req!(sorted, firsts, "{:?}: the slices arrive out of order", path);
		req!(firsts.first().copied().unwrap_or(u32::MAX), 0u32,
			"{:?}: the first slice does not begin at macroblock nought", path);
		for f in &firsts {
			let inside = *f < total;
			req!(inside, true,
				"{:?}: a slice begins at macroblock {} of a picture that holds {}",
				path, f, total);
		}
	}
	println!("  {} slice headers read; {} pictures of more than one slice", headers, multi);
	Ok(())
}

/// FFmpeg's own decode of a film's first frame, as raw 4:2:0 planes.
///
/// `skip_filter` asks FFmpeg to leave the deblocking filter out, which is how a fault in prediction
/// or in the residual is told apart from a fault in the filter.
fn ffmpeg_frame(path: &std::path::Path, skip_filter: bool) -> Outcome<Vec<u8>> {
	let mut cmd = std::process::Command::new("ffmpeg");
	// **`-noautorotate` is load-bearing.** A phone writes the rotation it was held at into the
	// track header, and FFmpeg turns the picture on the way out. A decoder produces the picture as
	// it was coded, and rotation is the container's business, so without this the two are compared
	// after one of them has been turned -- and at ninety degrees the byte count is unchanged, so
	// nothing but the samples says so.
	cmd.args(["-v", "error", "-noautorotate"]);
	if skip_filter {
		cmd.args(["-skip_loop_filter", "all"]);
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

/// Compares one decoded plane against the matching run of FFmpeg's output.
///
/// Returns the number of samples that differ and the worst difference, plus where the first one is.
fn compare(mine: &[u8], theirs: &[u8], w: usize) -> (usize, i32, Option<(usize, usize)>) {
	let mut wrong = 0usize;
	let mut worst = 0i32;
	let mut first = None;
	for (i, (a, b)) in mine.iter().zip(theirs.iter()).enumerate() {
		let d = (*a as i32 - *b as i32).abs();
		if d != 0 {
			wrong += 1;
			worst = worst.max(d);
			if first.is_none() {
				first = Some((i % w, i / w));
			}
		}
	}
	(wrong, worst, first)
}

#[test]
fn test_the_first_frame_matches_ffmpeg_04() -> Outcome<()> {
	// The only check that matters. Every table, every prediction mode, every shift in the inverse
	// transform and every threshold in the deblocking filter has to be right at once, or some
	// sample somewhere differs -- and a decoder that is nearly right produces a picture that looks
	// decoded, so nothing short of sample-for-sample agreement is evidence of anything.
	//
	// `H264_UNFILTERED` compares against a decode with the deblocking filter left out of both,
	// which says whether a mismatch is in the prediction and residual or in the filter.
	let films = match res!(films()) {
		Some(f) => f,
		None => return Ok(()),
	};
	let unfiltered = std::env::var("H264_UNFILTERED").is_ok();
	let mut exact = 0usize;
	let mut differ: Vec<String> = Vec::new();
	let mut refused: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
	let mut tried = 0usize;
	for path in &films {
		let (film, sample) = match at!(open(path), path) {
			Some(f) => f,
			None => continue,
		};
		if film.kind() != Kind::Avc {
			continue;
		}
		tried += 1;
		let mine = if unfiltered {
			h264::decode::picture_undeblocked(film.config(), &sample)
		} else {
			h264::decode::picture(film.config(), &sample)
		};
		let mine = match mine {
			Ok(p) => p,
			Err(e) => {
				// A refusal is an outcome, not a failure: what matters is that it is refused by
				// name rather than decoded into a wrong picture.
				let text = fmt!("{}", e);
				let line = text.lines().last().unwrap_or("?").to_string();
				*refused.entry(line).or_insert(0) += 1;
				continue;
			},
		};
		let theirs = res!(ffmpeg_frame(path, unfiltered));
		let (w, h) = (mine.y.w, mine.y.h);
		let luma = w * h;
		let chroma = (w / 2) * (h / 2);
		if theirs.len() != luma + 2 * chroma {
			return Err(err!(
				"{:?}: this decoder made a picture of {} by {} and FFmpeg gave {} bytes, which is \
				not {} plus two of {}.", path, w, h, theirs.len(), luma, chroma; Test, Mismatch));
		}
		let (wy, dy, fy) = compare(&mine.y.px, &theirs[..luma], w);
		let (wu, du, _) = compare(&mine.cb.px, &theirs[luma..luma + chroma], w / 2);
		let (wv, dv, _) = compare(&mine.cr.px, &theirs[luma + chroma..], w / 2);
		if wy == 0 && wu == 0 && wv == 0 {
			exact += 1;
		} else {
			differ.push(fmt!(
				"{:?} {}x{}: luma {} wrong (worst {}) first at {:?}, Cb {} (worst {}), \
				Cr {} (worst {})",
				path.file_name().unwrap_or_default(), w, h, wy, dy, fy, wu, du, wv, dv));
		}
	}
	println!("  {} H.264 films tried, {} matched FFmpeg exactly", tried, exact);
	if !refused.is_empty() {
		println!("  refused:");
		for (why, n) in &refused {
			println!("    {} x {}", n, why.trim());
		}
	}
	for line in &differ {
		println!("  DIFFERS {}", line);
	}
	if !differ.is_empty() {
		return Err(err!(
			"{} of {} films decoded to something other than what FFmpeg decoded. A picture that \
			is nearly right is a wrong picture.", differ.len(), tried; Test, Mismatch));
	}
	if exact == 0 && tried > 0 {
		return Err(err!(
			"Not one of {} films matched FFmpeg, so nothing was proved.", tried; Test, Mismatch));
	}
	Ok(())
}

#[test]
fn test_zz_a_window_onto_one_film_05() -> Outcome<()> {
	// Not a check but a window, and it earned its place: every fault found in this decoder was
	// found by looking through it. Point `H264_DUMP` at one film and it prints that film's
	// parameter sets, its NAL units, its slice headers, and an eight-by-eight patch of luma beside
	// FFmpeg's -- `H264_X0` and `H264_Y0` move the patch, `H264_UNFILTERED` takes the deblocking
	// filter out of both sides.
	//
	// The point of the patch is that a whole-frame difference count says nothing about *what* went
	// wrong, while eight rows of samples beside eight rows of somebody else's say a great deal: a
	// prediction using the wrong direction, a coefficient at the wrong frequency and a filter run
	// one sample too deep all look different from each other here, and identical in a count.
	let one = match std::env::var("H264_DUMP") {
		Ok(p) => std::path::PathBuf::from(p),
		Err(_) => return Ok(()),
	};
	let (film, sample) = match at!(open(&one), &one) {
		Some(f) => f,
		None => return Err(err!("no video track"; Test, Missing)),
	};
	let cfg = at!(h264::config(film.config()), &one);
	let mut sets = Vec::new();
	for u in &cfg.sps {
		sets.push(at!(h264::sps(&u.body), &one));
	}
	let mut pics = Vec::new();
	for u in &cfg.pps {
		pics.push(at!(h264::pps(&u.body, &sets), &one));
	}
	println!("  sps: {:?}", sets.first());
	println!("  pps: {:?}", pics.first());
	let units = at!(h264::split_lengthed(&sample, cfg.length_size), &one);
	for u in &units {
		println!("  nal {} ref {} len {}", u.kind, u.ref_idc, u.body.len());
		if matches!(u.kind, h264::nal::SLICE | h264::nal::IDR) {
			println!("    slice: {:?}", at!(h264::slice(u, &sets, &pics), &one));
		}
	}
	let unfiltered = std::env::var("H264_UNFILTERED").is_ok();
	let mine = if unfiltered {
		at!(h264::decode::picture_undeblocked(film.config(), &sample), &one)
	} else {
		at!(h264::decode::picture(film.config(), &sample), &one)
	};
	let theirs = res!(ffmpeg_frame(&one, unfiltered));
	let w = mine.y.w;
	let x0: usize = std::env::var("H264_X0").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
	let y0: usize = std::env::var("H264_Y0").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
	for y in y0..y0 + 8 {
		let a: Vec<u8> = (x0..x0 + 8).map(|x| mine.y.px[y * w + x]).collect();
		let b: Vec<u8> = (x0..x0 + 8).map(|x| theirs[y * w + x]).collect();
		println!("  row {}: mine {:?}", y, a);
		println!("          ffmg {:?}", b);
	}
	Ok(())
}
