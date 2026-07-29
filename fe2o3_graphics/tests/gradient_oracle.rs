//! Shade a shape the way a browser shades it, and check the colours land in the same places.
//!
//! The unit tests for [`Gradient`] assert what its own sampler returns, which is a fair check of the
//! interpolation and no check at all of the conventions around it: whether a position is measured
//! along the axis or across it, whether the ends pad or repeat, whether a radial gradient's position
//! is a distance or a squared one, and whether the colour is interpolated straight or premultiplied.
//! Every one of those can be wrong while the sampler agrees with itself, and the tests written by the
//! same hand as the sampler would still pass.
//!
//! The fixtures in `tests/gradient/` close that gap. Each `.grad` file describes one gradient and the
//! rectangle it fills, in four line kinds both sides read, and the `.png` beside it is Chromium's
//! rendering of exactly that description. Nothing in the expected output originates here.
//!
//! The tolerance is two levels a channel and the worst divergence observed is one, over a mean of a
//! tenth. Chromium dithers a gradient to hide the banding a long, shallow ramp would otherwise show,
//! so its output is deliberately not the exact ramp and two renderers are not obliged to agree to
//! the bit. The test prints the worst divergence and the mean it saw, so a drift towards the
//! tolerance is visible before it crosses it.
//!
//! To regenerate the PNGs: see `tests/gradient/gen.sh`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::{
		Gradient,
		Rgba,
		Stop,
	},
	path::{
		Bounds,
		Path,
	},
	pixmap::Pixmap,
	png,
	raster::FillRule,
	transform::Transform,
};

use std::{
	fs,
	path::PathBuf,
};

/// The side of every fixture, in pixels. `gen.sh` renders at this size.
const SIZE: usize = 256;

/// The furthest a channel may differ from the browser's.
const TOL: i32 = 2;

const CASES: &[&str] = &["linear_v", "linear_diag", "linear_pad", "radial", "linear_alpha"];

/// A colour's three channels multiplied by its alpha, which is what is seen.
///
/// The comparison is made here rather than on the straight channels because the browser's PNG has
/// been through a premultiplied buffer, so a colour under a very low alpha has been quantised to
/// nothing on the way and cannot be recovered: at an alpha of one two-hundred-and-fifty-fifth, a
/// saturated magenta and a black are the same picture, and demanding that two renderers agree about
/// which of them it was is demanding agreement about something invisible.
fn premul(c: Rgba) -> (u8, u8, u8) {
	let f = |v: u8| -> u8 { (((v as u32) * (c.a as u32) + 127) / 255) as u8 };
	(f(c.r), f(c.g), f(c.b))
}

/// A fixture, read from its `.grad` file.
struct Case {
	/// The gradient it describes.
	grad:	Gradient,
	/// The rectangle it fills.
	rect:	Bounds,
}

/// Reads a `.grad` fixture. The format is described in `gen.sh`, and both read it.
fn read_case(text: &str) -> Outcome<Case> {
	let mut axis: Option<(f32, f32, f32, f32)> = None;
	let mut centre: Option<(f32, f32, f32)> = None;
	let mut stops = Vec::new();
	let mut rect = None;
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		let f: Vec<&str> = line.split_whitespace().collect();
		let num = |s: &str| -> Outcome<f32> {
			Ok(res!(s.parse::<f32>(), Invalid, Input, Decode))
		};
		match f[0] {
			"linear" => axis = Some((
				res!(num(f[1])), res!(num(f[2])), res!(num(f[3])), res!(num(f[4])),
			)),
			"radial" => centre = Some((res!(num(f[1])), res!(num(f[2])), res!(num(f[3])))),
			"stop" => {
				let hex = f[2];
				let ch = |i: usize| -> Outcome<u8> {
					Ok(res!(u8::from_str_radix(&hex[i..i + 2], 16), Invalid, Input, Decode))
				};
				stops.push(Stop::new(
					res!(num(f[1])),
					Rgba::new(res!(ch(0)), res!(ch(2)), res!(ch(4)), res!(ch(6))),
				));
			},
			"rect" => {
				let (x, y, w, h) = (res!(num(f[1])), res!(num(f[2])), res!(num(f[3])), res!(num(f[4])));
				rect = Some(Bounds::new(x, y, x + w, y + h));
			},
			other => return Err(err!("A fixture line begins {:?}, which names nothing.", other;
			Invalid, Input)),
		}
	}
	let grad = match (axis, centre) {
		(Some((x0, y0, x1, y1)), None) => Gradient::Linear {
			from: (x0, y0),
			to: (x1, y1),
			stops,
		},
		(None, Some((cx, cy, r))) => Gradient::Radial {
			centre: (cx, cy),
			radius: r,
			stops,
		},
		_ => return Err(err!("A fixture must name exactly one of a linear or a radial gradient.";
		Invalid, Input)),
	};
	Ok(Case {
		grad,
		rect: match rect {
			Some(r) => r,
			None => return Err(err!("A fixture must name the rectangle it fills.";
			Invalid, Input, Missing)),
		},
	})
}

#[test]
fn test_a_gradient_shades_where_a_browser_shades_00() -> Outcome<()> {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/gradient");
	let mut worst = 0i32;
	let mut worst_case = String::new();
	let mut total = 0u64;
	let mut count = 0u64;

	for name in CASES {
		let text = res!(fs::read_to_string(dir.join(format!("{}.grad", name))), IO, File, Read);
		let case = res!(read_case(&text));

		let mut pm = res!(Pixmap::new(SIZE, SIZE));
		let path = res!(Path::rect(case.rect));
		res!(pm.fill_gradient(&path, &Transform::IDENTITY, &case.grad, None, FillRule::NonZero));

		let want = res!(png::decode(&res!(fs::read(dir.join(format!("{}.png", name))), IO, File, Read)));
		req!(want.width(), SIZE);
		req!(want.height(), SIZE);

		for y in 0..SIZE {
			for x in 0..SIZE {
				let got = match pm.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!("Our pixel ({}, {}) is off the pixmap.", x, y;
					Invalid, Input, Range)),
				};
				let exp = match want.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!("The browser's pixel ({}, {}) is off the image.", x, y;
					Invalid, Input, Range)),
				};
				let (gp, ep) = (premul(got), premul(exp));
				let pairs = [(gp.0, ep.0), (gp.1, ep.1), (gp.2, ep.2), (got.a, exp.a)];
				for i in 0..4 {
					let d = ((pairs[i].0 as i32) - (pairs[i].1 as i32)).abs();
					total += d as u64;
					count += 1;
					if d > worst {
						worst = d;
						worst_case = format!(
							"{} at ({}, {}): ours {:?}, the browser's {:?}", name, x, y, got, exp);
					}
				}
			}
		}
	}
	println!(
		"The worst channel differs by {} and the mean by {:.4}. Worst: {}",
		worst, (total as f64) / (count as f64), worst_case,
	);
	if worst > TOL {
		return Err(err!(
			"A gradient differs from the browser's by {} levels, over the tolerance of {}. {}",
			worst, TOL, worst_case;
		Invalid, Input, Mismatch));
	}
	Ok(())
}
