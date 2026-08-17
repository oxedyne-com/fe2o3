//! Decode a PNG of every legal bit depth, colour type and interlace method, and check the pixels
//! against the implementation that wrote the file.
//!
//! Each fixture in `tests/png/` was written by ImageMagick from a source this crate never saw, and
//! the `.pam` beside it is ImageMagick reading its own PNG back out. Nothing in the comparison
//! originates here, so agreement is agreement with an independent implementation rather than with
//! ourselves. `tests/png/gen.sh` regenerates the lot.
//!
//! Each case also declares the bit depth, colour type and interlace method its name claims, and the
//! test reads the fixture's IHDR to check that the file really carries them. ImageMagick silently
//! ignores a `png:bit-depth` it cannot honour, so without that check a matrix could quietly become
//! twenty copies of the eight-bit case and still pass.
//!
//! # The tolerance
//!
//! Fixtures of eight bits and below are compared exactly: there is one right answer and both
//! implementations must reach it.
//!
//! Sixteen-bit fixtures are compared to within one level a channel, because the two reductions to
//! eight bits differ and neither is wrong. This codec keeps the high byte, which is `v >> 8`.
//! ImageMagick's is `v * 255 / 65535` truncated. The two agree exactly whenever the sample is an
//! eight-bit value written twice, which is what a 16-bit file converted up from an 8-bit one holds
//! throughout, and differ by at most one otherwise: `v/256 - v/257` is under one for every `v` a
//! `u16` can hold. The fixtures deliberately carry samples that are *not* multiples of 257, so this
//! tolerance is exercised rather than merely allowed for.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::png;

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
};

/// A fixture: its name, its size, the bit depth and colour type its IHDR must carry, and whether it
/// must be interlaced.
type Case = (&'static str, usize, usize, u8, u8, bool);

// Every fixture gen.sh writes. The names say what they hold: g greyscale, p palette, pt palette
// with a tRNS chunk, ga greyscale with alpha, rgb truecolour, rgba truecolour with alpha, followed
// by the bit depth, the size, and i for interlaced or n for not.
const CASES: &[Case] = &[
	// One bit of greyscale, at every size, which is the narrowest sample the format has and the
	// one whose row padding is widest.
	("g1_1x1_i",		1, 1, 1, 0, true),
	("g1_1x1_n",		1, 1, 1, 0, false),
	("g1_1x5_i",		1, 5, 1, 0, true),
	("g1_1x5_n",		1, 5, 1, 0, false),
	("g1_3x2_i",		3, 2, 1, 0, true),
	("g1_3x2_n",		3, 2, 1, 0, false),
	("g1_5x1_i",		5, 1, 1, 0, true),
	("g1_5x1_n",		5, 1, 1, 0, false),
	("g1_9x9_i",		9, 9, 1, 0, true),
	("g1_9x9_n",		9, 9, 1, 0, false),
	("g1_17x13_i",		17, 13, 1, 0, true),
	("g1_17x13_n",		17, 13, 1, 0, false),
	// The rest of the greyscale depths.
	("g2_17x13_i",		17, 13, 2, 0, true),
	("g2_17x13_n",		17, 13, 2, 0, false),
	("g4_17x13_i",		17, 13, 4, 0, true),
	("g4_17x13_n",		17, 13, 4, 0, false),
	("g8_17x13_i",		17, 13, 8, 0, true),
	("g8_17x13_n",		17, 13, 8, 0, false),
	("g16_17x13_i",		17, 13, 16, 0, true),
	("g16_17x13_n",		17, 13, 16, 0, false),
	// Palette, at every depth the specification allows it.
	("p1_17x13_i",		17, 13, 1, 3, true),
	("p1_17x13_n",		17, 13, 1, 3, false),
	("p2_17x13_i",		17, 13, 2, 3, true),
	("p2_17x13_n",		17, 13, 2, 3, false),
	("p4_17x13_i",		17, 13, 4, 3, true),
	("p4_17x13_n",		17, 13, 4, 3, false),
	("p8_17x13_i",		17, 13, 8, 3, true),
	("p8_17x13_n",		17, 13, 8, 3, false),
	// The same, carrying a tRNS chunk, so that the alpha comes from outside the image data.
	("pt1_17x13_i",		17, 13, 1, 3, true),
	("pt1_17x13_n",		17, 13, 1, 3, false),
	("pt2_17x13_i",		17, 13, 2, 3, true),
	("pt2_17x13_n",		17, 13, 2, 3, false),
	("pt4_17x13_i",		17, 13, 4, 3, true),
	("pt4_17x13_n",		17, 13, 4, 3, false),
	("pt8_17x13_i",		17, 13, 8, 3, true),
	("pt8_17x13_n",		17, 13, 8, 3, false),
	// Truecolour and greyscale with alpha, at both depths each allows.
	("rgb8_17x13_i",	17, 13, 8, 2, true),
	("rgb8_17x13_n",	17, 13, 8, 2, false),
	("rgb16_17x13_i",	17, 13, 16, 2, true),
	("rgb16_17x13_n",	17, 13, 16, 2, false),
	("ga8_17x13_i",		17, 13, 8, 4, true),
	("ga8_17x13_n",		17, 13, 8, 4, false),
	("ga16_17x13_i",	17, 13, 16, 4, true),
	("ga16_17x13_n",	17, 13, 16, 4, false),
	// Truecolour with alpha at eight and at sixteen bits, at every size: the widest pixel the
	// format has, against the same Adam7 geometry the narrowest was put through.
	("rgba8_1x1_i",		1, 1, 8, 6, true),
	("rgba8_1x1_n",		1, 1, 8, 6, false),
	("rgba8_1x5_i",		1, 5, 8, 6, true),
	("rgba8_1x5_n",		1, 5, 8, 6, false),
	("rgba8_3x2_i",		3, 2, 8, 6, true),
	("rgba8_3x2_n",		3, 2, 8, 6, false),
	("rgba8_5x1_i",		5, 1, 8, 6, true),
	("rgba8_5x1_n",		5, 1, 8, 6, false),
	("rgba8_9x9_i",		9, 9, 8, 6, true),
	("rgba8_9x9_n",		9, 9, 8, 6, false),
	("rgba8_17x13_i",	17, 13, 8, 6, true),
	("rgba8_17x13_n",	17, 13, 8, 6, false),
	("rgba16_1x1_i",	1, 1, 16, 6, true),
	("rgba16_1x1_n",	1, 1, 16, 6, false),
	("rgba16_1x5_i",	1, 5, 16, 6, true),
	("rgba16_1x5_n",	1, 5, 16, 6, false),
	("rgba16_3x2_i",	3, 2, 16, 6, true),
	("rgba16_3x2_n",	3, 2, 16, 6, false),
	("rgba16_5x1_i",	5, 1, 16, 6, true),
	("rgba16_5x1_n",	5, 1, 16, 6, false),
	("rgba16_9x9_i",	9, 9, 16, 6, true),
	("rgba16_9x9_n",	9, 9, 16, 6, false),
	("rgba16_17x13_i",	17, 13, 16, 6, true),
	("rgba16_17x13_n",	17, 13, 16, 6, false),
];

fn dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("png")
}

/// The bit depth, colour type and interlace method a PNG's IHDR declares, read straight out of the
/// bytes rather than through the decoder the test is checking.
fn ihdr(buf: &[u8]) -> Outcome<(usize, usize, u8, u8, bool)> {
	if buf.len() < 33 {
		return Err(err!("A PNG needs 33 bytes to hold a signature and an IHDR."; Test, Invalid));
	}
	let w = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]) as usize;
	let h = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]) as usize;
	Ok((w, h, buf[24], buf[25], buf[28] != 0))
}

/// Reads a binary PAM, which is what ImageMagick writes its reference decodings as, and returns it
/// as RGBA.
///
/// ImageMagick writes a greyscale PNG back out as `GRAYSCALE_ALPHA` rather than `RGB_ALPHA`, and
/// the two channels are widened here rather than by asking ImageMagick for a colourspace change,
/// which would put a transform between the file and the comparison.
fn read_pam(path: &Path) -> Outcome<(usize, usize, Vec<u8>)> {
	let buf = res!(fs::read(path), IO, File);
	if buf.len() < 3 || &buf[0..3] != b"P7\n" {
		return Err(err!("{} is not a binary PAM.", path.display(); Test, Invalid, Input));
	}
	let (mut w, mut h, mut chans, mut max) = (0usize, 0usize, 0usize, 0usize);
	let mut tuple = String::new();
	let mut i = 3usize;
	loop {
		let start = i;
		while i < buf.len() && buf[i] != b'\n' {
			i += 1;
		}
		if i >= buf.len() {
			return Err(err!("{} has no ENDHDR.", path.display(); Test, Invalid, Input));
		}
		let line = String::from_utf8_lossy(&buf[start..i]).to_string();
		i += 1;
		let mut it = line.split_whitespace();
		let key = it.next().unwrap_or("");
		let val = it.next().unwrap_or("");
		match key {
			"ENDHDR"	=> break,
			"WIDTH"		=> w = res!(val.parse::<usize>(), Test, Invalid),
			"HEIGHT"	=> h = res!(val.parse::<usize>(), Test, Invalid),
			"DEPTH"		=> chans = res!(val.parse::<usize>(), Test, Invalid),
			"MAXVAL"	=> max = res!(val.parse::<usize>(), Test, Invalid),
			"TUPLTYPE"	=> tuple = val.to_string(),
			_		=> (),
		}
	}
	if max != 255 {
		return Err(err!(
			"{} declares a maximum sample of {}, and this reader wants 255.", path.display(), max;
		Test, Invalid, Input));
	}
	let need = w * h * chans;
	if buf.len() < i + need {
		return Err(err!(
			"{} declares {} by {} pixels of {} channels, needing {} bytes, but carries {}.",
			path.display(), w, h, chans, need, buf.len() - i;
		Test, Invalid, Input));
	}
	let px = &buf[i..i + need];
	let mut out = Vec::with_capacity(w * h * 4);
	for p in px.chunks_exact(chans) {
		match (chans, tuple.as_str()) {
			(4, "RGB_ALPHA")	=> out.extend_from_slice(p),
			(3, "RGB")		=> out.extend_from_slice(&[p[0], p[1], p[2], 255]),
			(2, "GRAYSCALE_ALPHA")	=> out.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
			(1, "GRAYSCALE")	=> out.extend_from_slice(&[p[0], p[0], p[0], 255]),
			_ => return Err(err!(
				"{} declares {} channels of tuple type '{}', which this reader does not know.",
				path.display(), chans, tuple;
			Test, Invalid, Input)),
		}
	}
	Ok((w, h, out))
}

#[test]
fn test_imagemagicks_pngs_decode_to_the_pixels_imagemagick_reads_from_them() -> Outcome<()> {
	let dir = dir();
	let mut worst_overall = 0i32;
	for (name, w, h, depth, ct, laced) in CASES {
		let buf = res!(fs::read(dir.join(fmt!("{}.png", name))), IO, File);

		// The fixture must be what its name says, or the matrix has a hole in it.
		let (fw, fh, fd, fct, fl) = res!(ihdr(&buf));
		req!(fw, *w, "The IHDR width of {}.", name);
		req!(fh, *h, "The IHDR height of {}.", name);
		req!(fd, *depth, "The IHDR bit depth of {}.", name);
		req!(fct, *ct, "The IHDR colour type of {}.", name);
		req!(fl, *laced, "The IHDR interlace method of {}.", name);

		let pm = res!(png::decode(&buf), Decode, Input);
		req!(pm.width(), *w, "Decoded width of {}.", name);
		req!(pm.height(), *h, "Decoded height of {}.", name);

		let (rw, rh, want) = res!(read_pam(&dir.join(fmt!("{}.pam", name))));
		req!(rw, *w, "Reference width of {}.", name);
		req!(rh, *h, "Reference height of {}.", name);

		// Eight bits and below have one right answer; sixteen leaves a level of latitude in the
		// reduction, as this file's header explains.
		let tol = if *depth == 16 { 1i32 } else { 0i32 };
		for y in 0..*h {
			for x in 0..*w {
				let c = match pm.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!(
						"The decoding of {} has no pixel at ({}, {}).", name, x, y; Test, Missing)),
				};
				let at = (y * w + x) * 4;
				let got = [c.r, c.g, c.b, c.a];
				for k in 0..4 {
					let d = (got[k] as i32) - (want[at + k] as i32);
					if d.abs() > tol {
						return Err(err!(
							"Decoding {} diverges from ImageMagick's own reading of it at pixel \
							({}, {}): it reads ({}, {}, {}, {}) and we read ({}, {}, {}, {}).",
							name, x, y,
							want[at], want[at + 1], want[at + 2], want[at + 3],
							got[0], got[1], got[2], got[3];
						Test, Mismatch));
					}
					worst_overall = worst_overall.max(d.abs());
				}
			}
		}
	}
	println!(
		"The worst divergence across {} fixtures was {}.", CASES.len(), worst_overall);
	Ok(())
}

#[test]
fn test_the_fixtures_are_not_all_the_same_picture() -> Outcome<()> {
	// A matrix of fixtures that all decoded to a flat grey would pass the comparison above without
	// exercising anything. Every 17 by 13 fixture must therefore hold at least two distinct
	// colours, the tRNS ones must hold at least one fully transparent pixel, and the 1-bit ones
	// must hold both extremes rather than one of them.
	let dir = dir();
	for (name, w, h, depth, _, _) in CASES {
		if *w != 17 {
			continue;
		}
		let buf = res!(fs::read(dir.join(fmt!("{}.png", name))), IO, File);
		let pm = res!(png::decode(&buf), Decode, Input);
		let mut seen: Vec<(u8, u8, u8, u8)> = Vec::new();
		let mut clear = 0usize;
		for y in 0..*h {
			for x in 0..*w {
				let c = match pm.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!(
						"The decoding of {} has no pixel at ({}, {}).", name, x, y; Test, Missing)),
				};
				let t = (c.r, c.g, c.b, c.a);
				if !seen.contains(&t) {
					seen.push(t);
				}
				if c.a == 0 {
					clear += 1;
				}
			}
		}
		if seen.len() < 2 {
			return Err(err!(
				"The fixture {} decodes to a single colour, so it tests nothing.", name;
			Test, Invalid));
		}
		if name.starts_with("pt") && clear == 0 {
			return Err(err!(
				"The fixture {} carries a tRNS chunk but decodes fully opaque.", name;
			Test, Invalid));
		}
		if *depth == 1 && !name.starts_with("p") {
			let lo = seen.iter().any(|c| c.0 == 0);
			let hi = seen.iter().any(|c| c.0 == 255);
			if !(lo && hi) {
				return Err(err!(
					"The 1-bit fixture {} does not widen to both 0 and 255.", name; Test, Invalid));
			}
		}
	}
	Ok(())
}
