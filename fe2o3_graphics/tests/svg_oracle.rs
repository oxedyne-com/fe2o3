//! Rasterise path data a browser has already drawn, and check the ink lands in the same places.
//!
//! The unit tests in `svg.rs` assert on the segments the parser emitted. That is a fair check of the
//! grammar and almost no check of the geometry: an arc conversion that is subtly wrong -- a centre a
//! little off, a sweep taken the long way round, a rotation applied in the wrong direction --
//! produces perfectly well-formed cubics, and every one of those tests still passes. The parser and
//! the tests share a hand, so they share any misreading of what the numbers mean.
//!
//! The fixtures in `tests/svg/` close that gap. Each `.path` file holds a viewBox and path data, and
//! the `.png` beside it is Chromium's rendering of exactly those bytes. Four of the fixtures are
//! lifted verbatim from a real drawing program's output rather than composed here, so they carry the
//! forms a person writing fixtures would not think to write: exponents (`-1.22e-4`), numbers run
//! together with their signs (`-45.975-1.22e-4`), arc flags with no separator, and two-subpath
//! donuts that only come out as rings under the non-zero rule.
//!
//! Nothing in the expected output originates here, so agreement means agreement with an independent
//! implementation of the same specification.
//!
//! The page is drawn with a black fill on a transparent background, so the alpha channel of the PNG
//! *is* the coverage Chromium computed, and the comparison is coverage against coverage with no
//! conversion in between.
//!
//! To regenerate the PNGs: see `tests/svg/gen.sh`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	path::TOLERANCE,
	png,
	raster::Raster,
	svg,
	transform::Transform,
};

use std::{
	fs,
	path::PathBuf,
};

/// The side of every fixture, in pixels. `gen.sh` renders at this size.
const SIZE: usize = 256;

/// How far a pixel's coverage may sit from Chromium's before it is counted as disagreeing.
///
/// Two rasterisers will never agree to the bit along an edge: they weigh a partly covered pixel
/// differently, and a half-covered pixel is genuinely ambiguous. A quarter of full coverage is well
/// inside that noise and far outside anything a geometry error could hide in -- a wrong centre or a
/// reversed sweep moves whole regions from 0 to 1, not by a fifth.
const NEAR: f32 = 0.25;

/// How much of the frame may disagree by more than [`NEAR`].
///
/// Disagreement is confined to the anti-aliased band along an edge, which for these shapes runs to
/// a few hundred pixels of the 65536. One percent leaves room for a longer edge without leaving
/// room for a shape in the wrong place.
const MAX_OFF: f32 = 0.01;

/// The mean absolute difference across the whole frame.
///
/// This is the guard that a small, systematic shift cannot pass: an edge band contributes almost
/// nothing to a mean over the whole frame, but a shape displaced by a pixel contributes everywhere
/// along its perimeter.
const MAX_MEAN: f32 = 0.004;

/// Every fixture in `tests/svg/`.
const CASES: &[&str] = &[
	// The four paths of one drawing program's output, verbatim.
	"mark_cross",
	"mark_lens",
	"mark_ring",
	"mark_outer",
	// Aimed at the arc conversion, which is the only part with real arithmetic in it.
	"arc_circle",
	"arc_flags",
	"arc_rotated",
	"arc_grown",
	// The curve commands that carry state from the command before them.
	"smooth_cubic",
	"smooth_quad",
	// The grammar written as tightly as it is allowed to be.
	"compact",
];

/// Where the fixtures live.
fn dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("svg")
}

/// Reads a fixture and rasterises it through our own parser, returning per-pixel coverage.
fn ours(name: &str) -> Outcome<Vec<f32>> {
	let txt = res!(fs::read_to_string(dir().join(format!("{}.path", name))));
	let mut lines = txt.lines();
	// The first line is the viewBox, as `# minx miny width height`.
	let head = match lines.next() {
		Some(h) => h,
		None => return Err(err!("Fixture '{}' is empty.", name; Test, Input)),
	};
	let mut vb = Vec::new();
	for tok in head.trim_start_matches('#').split_whitespace() {
		match tok.parse::<f32>() {
			Ok(v) => vb.push(v),
			Err(e) => return Err(err!(e,
				"Fixture '{}' has '{}' in its viewBox line.", name, tok; Test, Input)),
		}
	}
	if vb.len() != 4 {
		return Err(err!(
			"Fixture '{}' names {} viewBox numbers, wanted 4.", name, vb.len(); Test, Input));
	}
	let d = lines.collect::<Vec<_>>().join(" ");
	let p = res!(svg::path_data(&d));
	// The viewBox onto the frame, which is what the browser does with the same two numbers.
	let t = Transform::translate(-vb[0], -vb[1])
		.then(&Transform::scale(SIZE as f32 / vb[2], SIZE as f32 / vb[3]));
	let mut r = Raster::new(SIZE, SIZE);
	for c in p.flatten(&t, TOLERANCE) {
		r.add_contour(&c);
	}
	Ok(r.coverage())
}

/// Reads Chromium's rendering of a fixture, returning per-pixel coverage from its alpha.
fn theirs(name: &str) -> Outcome<Vec<f32>> {
	let buf = res!(fs::read(dir().join(format!("{}.png", name))));
	let pm = res!(png::decode(&buf));
	if pm.width() != SIZE || pm.height() != SIZE {
		return Err(err!("Fixture '{}' is {}x{}, wanted {}x{}.",
			name, pm.width(), pm.height(), SIZE, SIZE; Test, Input));
	}
	let mut out = Vec::with_capacity(SIZE * SIZE);
	for y in 0..SIZE {
		for x in 0..SIZE {
			match pm.pixel(x, y) {
				Some(px) => out.push(px.a as f32 / 255.0),
				None => return Err(err!(
					"Fixture '{}' has no pixel at ({}, {}).", name, x, y; Test, Input)),
			}
		}
	}
	Ok(out)
}

#[test]
fn test_our_ink_lands_where_chromiums_does_00() -> Outcome<()> {
	let mut worst = String::new();
	let mut worst_mean = 0.0f32;
	for name in CASES {
		let a = res!(ours(name));
		let b = res!(theirs(name));
		let mut off = 0usize;
		let mut sum = 0.0f64;
		for i in 0..a.len() {
			let d = (a[i] - b[i]).abs();
			sum += d as f64;
			if d > NEAR {
				off += 1;
			}
		}
		let frac = off as f32 / a.len() as f32;
		let mean = (sum / a.len() as f64) as f32;
		if mean > worst_mean {
			worst_mean = mean;
			worst = (*name).to_string();
		}
		assert!(frac <= MAX_OFF,
			"'{}': {:.3}% of the frame differs from Chromium by more than {}, allowed {:.3}%",
			name, frac * 100.0, NEAR, MAX_OFF * 100.0);
		assert!(mean <= MAX_MEAN,
			"'{}': mean coverage difference from Chromium is {:.5}, allowed {:.5}",
			name, mean, MAX_MEAN);
	}
	// Not an assertion: worth seeing which fixture sits closest to the line.
	println!("closest to the limit: {} at mean {:.5} of {:.5}", worst, worst_mean, MAX_MEAN);
	Ok(())
}

#[test]
fn test_the_fixtures_are_not_blank_01() -> Outcome<()> {
	// A guard on the oracle rather than the code. If `gen.sh` silently rendered nothing -- a browser
	// that failed to load the file, a viewBox that framed empty space -- every comparison above
	// would agree perfectly on a pair of empty frames and the suite would pass having tested
	// nothing.
	for name in CASES {
		let b = res!(theirs(name));
		let ink = b.iter().filter(|v| **v > 0.5).count() as f32 / b.len() as f32;
		assert!(ink > 0.02,
			"Chromium's '{}' is {:.2}% covered: the fixture rendered blank", name, ink * 100.0);
		assert!(ink < 0.98,
			"Chromium's '{}' is {:.2}% covered: the fixture rendered solid", name, ink * 100.0);
	}
	Ok(())
}
