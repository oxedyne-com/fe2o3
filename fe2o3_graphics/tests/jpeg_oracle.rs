//! Decode JPEGs that another implementation wrote, and check we read the pixels it says they hold.
//!
//! A codec tested by round trip through itself proves only that it is self-consistent, which a codec
//! that misreads the format is too. Every fixture in `tests/jpeg/` was therefore compressed by
//! ImageMagick, and the pixels each is checked against are ImageMagick's own reading of it, decoded
//! back out to a PPM. Nothing in the comparison originates here.
//!
//! The images the fixtures were made from are synthetic -- gradients, deterministic noise, saturated
//! primaries, a single pixel, and dimensions that are a multiple of neither a block nor an MCU --
//! and `tests/jpeg/gen.sh` generates them along with everything else in that directory.
//!
//! The tolerance is two levels a channel. Two decoders of the same file are not obliged to agree
//! exactly: the inverse DCT is specified by its result rather than its arithmetic, and different
//! roundings are legal. In practice this decoder uses the same integer transform, colour transform
//! and chroma filter libjpeg does, so most fixtures agree to the last bit; the test prints the worst
//! divergence it saw so that a drift towards the tolerance is visible before it crosses it.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	jpeg,
	pixmap::Pixmap,
};

use std::{
	fs,
	path::{
		Path,
		PathBuf,
	},
	process::Command,
};

/// How far a channel may differ from the reference before the test fails.
const TOL: i32 = 2;

/// Every fixture: its name, the size it should decode to, and whether its chrominance is carried at
/// full resolution, which decides how closely a block's mean can be expected to survive subsampling.
const CASES: &[(&str, usize, usize, bool)] = &[
	("gradient_q90_444",	64, 48,	true),
	("gradient_q75_420",	64, 48,	false),
	("gradient_q50_422",	64, 48,	false),
	("gradient_q80_prog",	64, 48,	false),
	("noise_q95_444",	33, 17,	true),
	("noise_q60_420",	33, 17,	false),
	("noise_q85_prog",	33, 17,	true),
	("noise_q80_rst",	33, 17,	false),
	("primaries_q92_444",	48, 32,	true),
	("primaries_q70_420",	48, 32,	false),
	("primaries_q70_422",	48, 32,	false),
	("tiny_q90_420",	1, 1,	false),
	("odd_q88_444",		17, 13,	true),
	("odd_q64_420",		17, 13,	false),
	("odd_q88_prog",	17, 13,	false),
	("ramp_q90_grey",	40, 24,	true),
	("ramp_q90_prog",	40, 24,	true),
];

/// The fixture directory.
fn dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("jpeg")
}

/// Reads a binary PPM, which is what ImageMagick writes its reference decodings as.
fn read_ppm(path: &Path) -> Outcome<(usize, usize, Vec<u8>)> {
	let buf = res!(fs::read(path), IO, File);
	// The header is three whitespace-separated fields after the magic, and comments may sit between.
	let mut fields: Vec<usize> = Vec::new();
	let mut i = 2usize;
	if buf.len() < 2 || &buf[0..2] != b"P6" {
		return Err(err!("{} is not a binary PPM.", path.display(); Test, Invalid, Input));
	}
	while fields.len() < 3 {
		while i < buf.len() && (buf[i] as char).is_ascii_whitespace() {
			i += 1;
		}
		if i < buf.len() && buf[i] == b'#' {
			while i < buf.len() && buf[i] != b'\n' {
				i += 1;
			}
			continue;
		}
		let start = i;
		while i < buf.len() && (buf[i] as char).is_ascii_digit() {
			i += 1;
		}
		if i == start {
			return Err(err!("{} has a malformed PPM header.", path.display(); Test, Invalid, Input));
		}
		let s = String::from_utf8_lossy(&buf[start..i]).to_string();
		fields.push(res!(s.parse::<usize>(), Test, Invalid));
	}
	i += 1; // The single whitespace byte that ends the header.
	let (w, h, max) = (fields[0], fields[1], fields[2]);
	if max != 255 {
		return Err(err!(
			"{} declares a maximum sample of {}, and this reader wants 255.", path.display(), max;
		Test, Invalid, Input));
	}
	let need = w * h * 3;
	if buf.len() < i + need {
		return Err(err!(
			"{} declares {} by {} pixels, needing {} bytes, but carries {}.",
			path.display(), w, h, need, buf.len() - i;
		Test, Invalid, Input));
	}
	Ok((w, h, buf[i..i + need].to_vec()))
}

/// The largest per-channel difference between a decoded pixmap and a reference, and where it was.
fn worst(pm: &Pixmap, w: usize, h: usize, want: &[u8]) -> Outcome<(i32, usize, usize)> {
	let mut worst = (0i32, 0usize, 0usize);
	for y in 0..h {
		for x in 0..w {
			let c = match pm.pixel(x, y) {
				Some(c) => c,
				None => return Err(err!(
					"The decoded pixmap has no pixel at ({}, {}).", x, y; Test, Missing)),
			};
			let at = (y * w + x) * 3;
			let d = [
				((c.r as i32) - (want[at] as i32)).abs(),
				((c.g as i32) - (want[at + 1] as i32)).abs(),
				((c.b as i32) - (want[at + 2] as i32)).abs(),
			];
			for v in d {
				if v > worst.0 {
					worst = (v, x, y);
				}
			}
		}
	}
	Ok(worst)
}

#[test]
fn test_imagemagicks_jpegs_decode_to_the_pixels_imagemagick_reads_from_them() -> Outcome<()> {
	let dir = dir();
	let mut overall = 0i32;
	for (name, w, h, _) in CASES {
		let jpg = dir.join(fmt!("{}.jpg", name));
		let ppm = dir.join(fmt!("{}.ppm", name));
		let buf = res!(fs::read(&jpg), IO, File);
		let pm = res!(jpeg::decode(&buf), Decode, Input);
		let (rw, rh, want) = res!(read_ppm(&ppm));

		req!(pm.width(), *w, "Width of {}.", name);
		req!(pm.height(), *h, "Height of {}.", name);
		req!(rw, *w, "Width of the reference decoding of {}.", name);
		req!(rh, *h, "Height of the reference decoding of {}.", name);

		let (d, x, y) = res!(worst(&pm, *w, *h, &want));
		if d > TOL {
			let c = match pm.pixel(x, y) {
				Some(c) => c,
				None => return Err(err!("No pixel at ({}, {}).", x, y; Test, Missing)),
			};
			let at = (y * w + x) * 3;
			return Err(err!(
				"Decoding {} diverges from ImageMagick's own reading of it by {} at pixel ({}, {}): \
				it reads ({}, {}, {}) and we read ({}, {}, {}).",
				name, d, x, y, want[at], want[at + 1], want[at + 2], c.r, c.g, c.b;
			Test, Mismatch));
		}
		overall = overall.max(d);
	}
	println!("The worst divergence across {} fixtures was {}.", CASES.len(), overall);
	Ok(())
}

#[test]
fn test_the_size_probe_agrees_with_a_full_decode() -> Outcome<()> {
	let dir = dir();
	for (name, w, h, _) in CASES {
		let buf = res!(fs::read(dir.join(fmt!("{}.jpg", name))), IO, File);
		let (pw, ph) = res!(jpeg::dimensions(&buf), Decode, Input);
		req!(pw, *w, "The probe's width for {}.", name);
		req!(ph, *h, "The probe's height for {}.", name);
	}
	Ok(())
}

#[test]
fn test_the_eighth_scale_decode_holds_each_blocks_mean() -> Outcome<()> {
	// A block's DC coefficient is eight times its mean, so a decode that reads that coefficient and
	// nothing else must reproduce the mean of each block of the full decode. Only the blocks that lie
	// wholly inside the image are checked: an edge block's coefficients describe the padding the
	// encoder replicated into it as well, which the full-size image then crops away.
	//
	// Only the fixtures whose chrominance was not subsampled can be checked this way. Where it was,
	// a chrominance block covers sixteen pixels rather than eight, so the reduced image carries its
	// colour at half the resolution of its luminance -- which is what the file holds, and what every
	// other reduced-scale decoder produces, but it is not the mean of the block.
	let dir = dir();
	for (name, w, h, full_chroma) in CASES {
		if !*full_chroma {
			continue;
		}
		let buf = res!(fs::read(dir.join(fmt!("{}.jpg", name))), IO, File);
		let full = res!(jpeg::decode(&buf), Decode, Input);
		let small = res!(jpeg::decode_eighth(&buf), Decode, Input);
		req!(small.width(), (w + 7) / 8, "The eighth-scale width of {}.", name);
		req!(small.height(), (h + 7) / 8, "The eighth-scale height of {}.", name);

		let tol = 4i64;
		for by in 0..(h / 8) {
			for bx in 0..(w / 8) {
				let mut sum = [0i64; 3];
				for y in 0..8 {
					for x in 0..8 {
						let c = match full.pixel(bx * 8 + x, by * 8 + y) {
							Some(c) => c,
							None => return Err(err!(
								"No pixel at ({}, {}) of {}.", bx * 8 + x, by * 8 + y, name;
							Test, Missing)),
						};
						sum[0] += c.r as i64;
						sum[1] += c.g as i64;
						sum[2] += c.b as i64;
					}
				}
				let got = match small.pixel(bx, by) {
					Some(c) => c,
					None => return Err(err!(
						"No pixel at ({}, {}) of the eighth-scale {}.", bx, by, name; Test, Missing)),
				};
				for (i, v) in [got.r, got.g, got.b].iter().enumerate() {
					let mean = sum[i] / 64;
					if (mean - (*v as i64)).abs() > tol {
						return Err(err!(
							"Block ({}, {}) of {} has a mean of {} in channel {}, but the \
							eighth-scale decode gives {}.", bx, by, name, mean, i, v;
						Test, Mismatch));
					}
				}
			}
		}
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE ENCODER                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Whether ImageMagick is on the path, since the encoder's oracle is its decoder.
fn have_convert() -> bool {
	Command::new("convert")
		.arg("-version")
		.output()
		.map(|o| o.status.success())
		.unwrap_or(false)
}

/// A synthetic pixmap: a two-axis gradient with a hard-edged patch of saturated colour in it.
///
/// The grey form drops the patch and the colour, so that a greyscale encoding can be measured
/// against a source it could in principle reproduce.
fn source(w: usize, h: usize, grey: bool) -> Outcome<Pixmap> {
	let mut pm = res!(Pixmap::new(w, h));
	let d = pm.data_mut();
	for y in 0..h {
		for x in 0..w {
			let at = (y * w + x) * 4;
			let hard = x * 3 > w && x * 3 < w * 2 && y * 3 > h && y * 3 < h * 2;
			let (r, g, b) = if grey {
				let v = (((x * 160) / w.max(1)) + ((y * 90) / h.max(1))) as u8;
				(v, v, v)
			} else if hard {
				(230u8, 20u8, 40u8)
			} else {
				(((x * 255) / w.max(1)) as u8, ((y * 255) / h.max(1)) as u8, 128u8)
			};
			d[at] = r;
			d[at + 1] = g;
			d[at + 2] = b;
			d[at + 3] = 255;
		}
	}
	Ok(pm)
}

/// The root-mean-square difference between a pixmap and a PPM of the same size.
fn rmse(pm: &Pixmap, want: &[u8]) -> f64 {
	let mut sum = 0f64;
	let d = pm.data();
	let n = pm.width() * pm.height();
	for i in 0..n {
		for c in 0..3 {
			let e = (d[i * 4 + c] as f64) - (want[i * 3 + c] as f64);
			sum += e * e;
		}
	}
	(sum / ((n * 3) as f64)).sqrt()
}

#[test]
fn test_imagemagick_reads_back_what_this_encoder_writes() -> Outcome<()> {
	if !have_convert() {
		println!("ImageMagick is not installed, so the encoder's oracle test is skipped.");
		return Ok(());
	}
	let tmp = std::env::temp_dir().join(fmt!("fe2o3_jpeg_enc_{}", std::process::id()));
	res!(fs::create_dir_all(&tmp), IO, File);

	let cases: &[(usize, usize, u8, jpeg::Chroma, bool, f64)] = &[
		(64, 48, 95, jpeg::Chroma::Full, false, 3.0),
		(64, 48, 85, jpeg::Chroma::Half, false, 9.0),
		(64, 48, 85, jpeg::Chroma::Quarter, false, 11.0),
		(64, 48, 30, jpeg::Chroma::Quarter, false, 20.0),
		(17, 13, 90, jpeg::Chroma::Quarter, false, 20.0),
		(1, 1, 90, jpeg::Chroma::Quarter, false, 6.0),
		(40, 24, 90, jpeg::Chroma::Full, true, 3.0),
		(40, 24, 60, jpeg::Chroma::Full, true, 6.0),
	];

	for (i, (w, h, q, chroma, grey, limit)) in cases.iter().enumerate() {
		let pm = res!(source(*w, *h, *grey));
		let opts = jpeg::Options { quality: *q, chroma: *chroma, grey: *grey };
		let buf = res!(jpeg::encode_with(&pm, &opts), Encode);

		let jpg = tmp.join(fmt!("case{}.jpg", i));
		let ppm = tmp.join(fmt!("case{}.ppm", i));
		res!(fs::write(&jpg, &buf), IO, File);
		let out = res!(Command::new("convert")
			.arg(&jpg)
			.arg(&ppm)
			.output(), IO);
		if !out.status.success() {
			return Err(err!(
				"ImageMagick refused a JPEG this encoder wrote at {} by {}, quality {}: {}",
				w, h, q, String::from_utf8_lossy(&out.stderr);
			Test, Invalid, Encode));
		}
		let (rw, rh, want) = res!(read_ppm(&ppm));
		req!(rw, *w, "The width ImageMagick reads back at quality {}.", q);
		req!(rh, *h, "The height ImageMagick reads back at quality {}.", q);

		let e = rmse(&pm, &want);
		if e > *limit {
			return Err(err!(
				"A {} by {} image encoded at quality {} comes back from ImageMagick with an RMSE of \
				{:.2}, over the limit of {:.2}.", w, h, q, e, limit;
			Test, Excessive));
		}
		println!(
			"{} by {}, quality {}, {:?}{}: RMSE {:.2}.",
			w, h, q, chroma, if *grey { ", greyscale" } else { "" }, e,
		);
	}
	res!(fs::remove_dir_all(&tmp), IO, File);
	Ok(())
}

#[test]
fn test_this_codecs_own_round_trip_holds_its_colours() -> Outcome<()> {
	// Weaker than the oracle above, but it exercises the decoder against a bitstream this crate's
	// encoder wrote, which the ImageMagick fixtures never are.
	let pm = res!(source(37, 23, false));
	let opts = jpeg::Options {
		quality: 95,
		chroma: jpeg::Chroma::Full,
		grey: false,
	};
	let buf = res!(jpeg::encode_with(&pm, &opts), Encode);
	let back = res!(jpeg::decode(&buf), Decode);
	req!(back.width(), 37usize);
	req!(back.height(), 23usize);
	let mut worst = 0i32;
	for y in 0..23 {
		for x in 0..37 {
			let (a, b) = match (pm.pixel(x, y), back.pixel(x, y)) {
				(Some(a), Some(b)) => (a, b),
				_ => return Err(err!("No pixel at ({}, {}).", x, y; Test, Missing)),
			};
			for (p, q) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
				worst = worst.max(((p as i32) - (q as i32)).abs());
			}
		}
	}
	if worst > 24 {
		return Err(err!(
			"A quality 95 round trip through this codec moves a channel by {}.", worst;
		Test, Excessive));
	}
	Ok(())
}
