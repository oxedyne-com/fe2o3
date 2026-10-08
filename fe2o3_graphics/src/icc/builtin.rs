//! The two spaces built into the reader, computed from their definitions rather than held as
//! profile bytes: sRGB and sGray.
//!
//! sRGB is IEC 61966-2-1: the Rec. 709 primaries, the D65 white, and the piecewise curve with an
//! exponent of 2.4. The colorants are adapted from D65 to the D50 of the connection space by the
//! Bradford transform, as a profile of sRGB carries them. sGray is the sRGB curve on Y, with the D50
//! white that is a grey profile's.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::{
	matrix::{
		Grey,
		M3,
		RgbMatrix,
		inv,
		mul,
		mul_v,
	},
	read::Curve,
};

use oxedyne_fe2o3_core::prelude::*;

// The connection space illuminant, as an s15Fixed16 profile holds it.
const D50:		[f64; 3] = [0.964_202_880_859_375, 1.0, 0.824_905_395_507_812_5];
const D65_XY:	[f64; 2] = [0.3127, 0.3290];
const PRIM_XY:	[[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];

// The Bradford cone response matrix.
const BRADFORD:	M3 = [
	[ 0.8951,  0.2664, -0.1614],
	[-0.7502,  1.7135,  0.0367],
	[ 0.0389, -0.0685,  1.0296],
];

fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
	[xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

// The matrix that takes linear RGB of the given primaries and white to XYZ.
fn primaries(prim: [[f64; 2]; 3], white: [f64; 3]) -> Outcome<M3> {
	let p: Vec<[f64; 3]> = prim.iter().map(|xy| xyz_of(*xy)).collect();
	let cols = [
		[p[0][0], p[1][0], p[2][0]],
		[p[0][1], p[1][1], p[2][1]],
		[p[0][2], p[1][2], p[2][2]],
	];
	let s = mul_v(&res!(inv(&cols)), white);
	let mut m = cols;
	for row in m.iter_mut() {
		for (j, v) in row.iter_mut().enumerate() {
			*v *= s[j];
		}
	}
	Ok(m)
}

// The Bradford adaptation matrix from one white to another.
fn adapt(from: [f64; 3], to: [f64; 3]) -> Outcome<M3> {
	let (a, b) = (mul_v(&BRADFORD, from), mul_v(&BRADFORD, to));
	let mut scale = [[0.0; 3]; 3];
	for i in 0..3 {
		scale[i][i] = b[i] / a[i];
	}
	Ok(mul(&res!(inv(&BRADFORD)), &mul(&scale, &BRADFORD)))
}

// The sRGB transfer curve as a version 4 parametric curve of function type 3.
fn srgb_curve() -> Curve {
	Curve::Para {
		func:	3,
		p:		[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0],
	}
}

/// The sRGB space, its colorants adapted to D50.
pub fn srgb() -> Outcome<RgbMatrix> {
	let white = xyz_of(D65_XY);
	let m = res!(primaries(PRIM_XY, white));
	let a = res!(adapt(white, D50));
	Ok(RgbMatrix { m: mul(&a, &m), trc: [srgb_curve(), srgb_curve(), srgb_curve()] })
}

/// The sGray space, the sRGB curve on Y.
pub fn sgray() -> Grey {
	Grey { trc: srgb_curve() }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn srgb_colorants_are_the_d50_profile_values() -> Outcome<()> {
		let s = res!(srgb());
		// The D50 sRGB colorants every v2 profile of sRGB carries, to four places.
		let want = [[0.4360, 0.3851, 0.1431], [0.2225, 0.7169, 0.0606], [0.0139, 0.0971, 0.7141]];
		for i in 0..3 {
			for j in 0..3 {
				assert!((s.m[i][j] - want[i][j]).abs() < 6e-4, "m[{}][{}] = {}", i, j, s.m[i][j]);
			}
		}
		// White maps to the D50 illuminant.
		let w = s.xyz([1.0, 1.0, 1.0]);
		for i in 0..3 {
			assert!((w[i] - D50[i]).abs() < 1e-6, "white[{}] = {}", i, w[i]);
		}
		Ok(())
	}

	#[test]
	fn srgb_curve_has_the_known_values() {
		let c = srgb_curve();
		assert!((c.eval(0.5) - 0.214_041).abs() < 1e-5);
		assert!((c.eval(0.04) - 0.04 / 12.92).abs() < 1e-12);
		assert!((c.inverse(c.eval(0.73)) - 0.73).abs() < 1e-9);
	}
}
