//! The connection-space arithmetic of a transform: XYZ and Lab, the encodings a lookup table holds
//! Lab in, and the black points a transform compensates between.
//!
//! The conversions use the D50 white `(0.9642, 1.0, 0.8249)` that LittleCMS takes for the
//! connection space, not the profile header's s15Fixed16 illuminant, since the black points and
//! the swatches that pin them are LittleCMS's. A black point is an XYZ colour; the one for a
//! lookup-table CMYK profile is found by the methods Adobe published and LittleCMS follows: the
//! darker colorant of the table, the round trip of Lab black through the output table, or, for a
//! destination, a quadratic fitted to the shadow end of the round trip of a ramp of lightness.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::{
	matrix::{
		inv,
		mul_v,
	},
	read::{
		Class,
		Lut,
		Profile,
		Space,
	},
	transform::Intent,
};

pub const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

// The black point a version 4 profile has for the perceptual and saturation intents.
const V4_BLACK: [f64; 3] = [0.00336, 0.0034731, 0.00287];

const EDGE: f64 = 24.0 / 116.0;

fn f(t: f64) -> f64 {
	if t <= EDGE * EDGE * EDGE { 841.0 / 108.0 * t + 16.0 / 116.0 } else { t.cbrt() }
}

fn f_inv(t: f64) -> f64 {
	if t <= EDGE { 108.0 / 841.0 * (t - 16.0 / 116.0) } else { t * t * t }
}

/// The Lab of an XYZ colour, both against the D50 white.
pub fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
	let (x, y, z) = (f(xyz[0] / D50[0]), f(xyz[1] / D50[1]), f(xyz[2] / D50[2]));
	[116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// The XYZ of a Lab colour, both against the D50 white.
pub fn lab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
	let y = (lab[0] + 16.0) / 116.0;
	let (x, z) = (y + lab[1] / 500.0, y - lab[2] / 200.0);
	[f_inv(x) * D50[0], f_inv(y) * D50[1], f_inv(z) * D50[2]]
}

/// A Lab colour as the fractions of full scale a table of `bits` bits takes it in.
///
/// A lut16 holds the legacy encoding, where L* of 100 is 0xFF00 and a* or b* of zero is 0x8000.
/// A lut8 holds L* of 100 as 255 and a* or b* of zero as 128.
pub fn lab_enc(bits: u8, lab: [f64; 3]) -> [f64; 3] {
	let k = if bits == 8 { 1.0 } else { 65280.0 / 65535.0 };
	[lab[0] / 100.0 * k, (lab[1] + 128.0) / 255.0 * k, (lab[2] + 128.0) / 255.0 * k]
}

/// The Lab colour a table of `bits` bits holds as the fractions `v`; the inverse of [`lab_enc`].
pub fn lab_dec(bits: u8, v: [f64; 3]) -> [f64; 3] {
	let k = if bits == 8 { 1.0 } else { 65535.0 / 65280.0 };
	[v[0] * 100.0 * k, v[1] * 255.0 * k - 128.0, v[2] * 255.0 * k - 128.0]
}

/// The table for an intent, or the first when the profile holds none for it.
pub fn pick(t: &[Option<Lut>; 3], i: usize) -> Option<&Lut> {
	t[i].as_ref().or(t[0].as_ref())
}

// Lab to the CMYK of the output table `tag` and back through the relative colorimetric input table.
fn round(p: &Profile, tag: usize, lab: [f64; 3]) -> Option<[f64; 3]> {
	let b = p.b2a[tag].as_ref()?;
	let a = pick(&p.a2b, 1)?;
	let e = lab_enc(b.bits, lab);
	let cmyk = b.eval([e[0], e[1], e[2], 0.0]);
	let g = a.eval(cmyk);
	Some(lab_dec(a.bits, [g[0], g[1], g[2]]))
}

// A neutral of the lightness of `lab`, no lighter than L* 50, as XYZ.
fn neutral(lab: [f64; 3]) -> [f64; 3] {
	lab_to_xyz([lab[0].min(50.0), 0.0, 0.0])
}

/// The XYZ of a device black that is the darkest colorant of a matrix or grey profile.
pub fn dark_colorant(xyz: [f64; 3]) -> [f64; 3] {
	neutral(xyz_to_lab(xyz))
}

// The darkest colorant of the input table for `tag`: all four inks at full strength.
fn darker(p: &Profile, tag: usize) -> [f64; 3] {
	match p.a2b[tag].as_ref() {
		Some(a)	=> {
			let g = a.eval([1.0; 4]);
			dark_colorant(lab_to_xyz(lab_dec(a.bits, [g[0], g[1], g[2]])))
		},
		None	=> [0.0; 3],
	}
}

// Lab black through the perceptual output table and back: the black the ink limits allow.
fn perceptual_black(p: &Profile) -> [f64; 3] {
	if p.a2b[0].is_none() {
		return [0.0; 3];
	}
	match round(p, 0, [0.0; 3]) {
		Some(l)	=> neutral(l),
		None	=> [0.0; 3],
	}
}

fn linked(p: &Profile) -> bool {
	matches!(p.head.class, Class::Link | Class::Abstract | Class::Named)
}

/// The black point of a CMYK lookup-table profile as a source.
pub fn cmyk_black_src(p: &Profile, intent: Intent) -> [f64; 3] {
	if linked(p) || intent == Intent::Absolute {
		return [0.0; 3];
	}
	if p.head.major >= 4 && matches!(intent, Intent::Perceptual | Intent::Saturation) {
		return V4_BLACK;
	}
	if intent == Intent::Relative && p.head.class == Class::Output && p.head.space == Space::Cmyk {
		return perceptual_black(p);
	}
	darker(p, intent.tag())
}

// The least squares quadratic through (x, y), and the root of it that the black point is.
fn quad_root(x: &[f64], y: &[f64]) -> f64 {
	let n = x.len();
	if n < 4 {
		return 0.0;
	}
	let (mut s1, mut s2, mut s3, mut s4) = (0.0, 0.0, 0.0, 0.0);
	let (mut t0, mut t1, mut t2) = (0.0, 0.0, 0.0);
	for i in 0..n {
		let (a, b) = (x[i], y[i]);
		s1 += a;
		s2 += a * a;
		s3 += a * a * a;
		s4 += a * a * a * a;
		t0 += b;
		t1 += b * a;
		t2 += b * a * a;
	}
	let m = [[s4, s3, s2], [s3, s2, s1], [s2, s1, n as f64]];
	let r = match inv(&m) {
		Ok(i)	=> mul_v(&i, [t2, t1, t0]),
		Err(_)	=> return 0.0,
	};
	let (a, b, c) = (r[0], r[1], r[2]);
	if a.abs() < 1e-10 {
		return 0.0f64.min(50.0f64.max(-c / b));
	}
	let d = b * b - 4.0 * a * c;
	if d <= 0.0 {
		return 0.0;
	}
	((-b + d.sqrt()) / (2.0 * a)).min(50.0).max(0.0)
}

/// The black point of a CMYK lookup-table profile as a destination.
pub fn cmyk_black_dst(p: &Profile, intent: Intent) -> [f64; 3] {
	if linked(p) || intent == Intent::Absolute {
		return [0.0; 3];
	}
	if p.head.major >= 4 && matches!(intent, Intent::Perceptual | Intent::Saturation) {
		return V4_BLACK;
	}
	let tag = intent.tag();
	if p.b2a[tag].is_none() {
		return cmyk_black_src(p, intent);
	}
	let rel = intent == Intent::Relative;
	let ini = if rel { xyz_to_lab(cmyk_black_src(p, intent)) } else { [0.0; 3] };
	let (mut ins, mut outs) = ([0.0f64; 256], [0.0f64; 256]);
	for l in 0..256 {
		let lab = [l as f64 * 100.0 / 255.0, ini[1].clamp(-50.0, 50.0), ini[2].clamp(-50.0, 50.0)];
		ins[l] = lab[0];
		outs[l] = match round(p, tag, lab) {
			Some(o)	=> o[0],
			None	=> return [0.0; 3],
		};
	}
	for l in (1..255).rev() {
		outs[l] = outs[l].min(outs[l + 1]);
	}
	if !(outs[0] < outs[255]) {
		return [0.0; 3];
	}
	let (lo, hi) = (outs[0], outs[255]);
	if rel && (0..256).all(|l| ins[l] <= lo + 0.2 * (hi - lo) || (ins[l] - outs[l]).abs() < 4.0) {
		return lab_to_xyz(ini);
	}
	let (from, to) = if rel { (0.1, 0.5) } else { (0.03, 0.25) };
	let (mut px, mut py) = (Vec::new(), Vec::new());
	for l in 0..256 {
		let y = (outs[l] - lo) / (hi - lo);
		if y >= from && y < to {
			px.push(ins[l]);
			py.push(y);
		}
	}
	if px.len() < 3 {
		return [0.0; 3];
	}
	let l = quad_root(&px, &py).max(0.0);
	lab_to_xyz([l, ini[1], ini[2]])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lab_and_xyz_are_inverses_and_d50_is_white() {
		let w = xyz_to_lab(D50);
		assert!((w[0] - 100.0).abs() < 1e-9 && w[1].abs() < 1e-9 && w[2].abs() < 1e-9);
		for xyz in [[0.2, 0.3, 0.1], [0.001, 0.0005, 0.002], [0.9, 0.95, 0.7]] {
			let b = lab_to_xyz(xyz_to_lab(xyz));
			for i in 0..3 {
				assert!((b[i] - xyz[i]).abs() < 1e-12, "{:?} came back as {:?}", xyz, b);
			}
		}
	}

	#[test]
	fn the_encodings_round_trip_and_place_white_and_neutral() {
		let w16 = lab_enc(16, [100.0, 0.0, 0.0]);
		assert!((w16[0] * 65535.0 - 65280.0).abs() < 1e-9 && (w16[1] * 65535.0 - 32768.0).abs() < 1e-9);
		let w8 = lab_enc(8, [100.0, 0.0, 0.0]);
		assert!((w8[0] - 1.0).abs() < 1e-12 && (w8[1] * 255.0 - 128.0).abs() < 1e-9);
		for bits in [8u8, 16] {
			let l = [47.0, -33.0, 21.5];
			let b = lab_dec(bits, lab_enc(bits, l));
			for i in 0..3 {
				assert!((b[i] - l[i]).abs() < 1e-9);
			}
		}
	}
}
