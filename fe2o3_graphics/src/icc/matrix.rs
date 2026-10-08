//! The matrix path: RGB to XYZ (D50) through a profile's colorants and curves, and RGB to grey
//! through the grey curve's inverse.
//!
//! A matrix profile is three curves, one for each of red, green and blue, that take a device value
//! to a linear intensity, and three XYZ colorants, the columns of the matrix that takes the
//! intensities to XYZ in the connection space. A grey profile is one curve, `kTRC`, from the grey
//! level to Y. Nothing here interpolates a table; a profile whose colour is held in a lattice is
//! the lookup-table path, and is read, not evaluated, by [`crate::icc::read`].
//!
//! # RGB to grey
//!
//! The Y of the RGB colour, found through the source's matrix, is the luminance the destination
//! grey must reproduce, so the grey level is the destination curve's inverse at that Y. This is the
//! conversion LittleCMS makes for an RGB source and a grey profile with an XYZ connection space.
//!
//! # What is refused
//!
//! A grey profile with a Lab connection space is refused by name, since its `kTRC` takes a level to
//! L* rather than to Y and this path makes no conversion between the two; so is an RGB profile with
//! a Lab one, a profile lacking a colorant or a curve, and a curve that falls from its start to its
//! end.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::read::{
	Curve,
	Profile,
	Space,
};

use oxedyne_fe2o3_core::prelude::*;

pub type M3 = [[f64; 3]; 3];

/// The product `a * b`.
pub fn mul(a: &M3, b: &M3) -> M3 {
	let mut m = [[0.0; 3]; 3];
	for i in 0..3 {
		for j in 0..3 {
			m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
		}
	}
	m
}

/// The product `a * v`.
pub fn mul_v(a: &M3, v: [f64; 3]) -> [f64; 3] {
	let mut r = [0.0; 3];
	for i in 0..3 {
		r[i] = a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2];
	}
	r
}

/// The inverse of `a`, by its adjugate; a matrix of determinant near zero is refused.
pub fn inv(a: &M3) -> Outcome<M3> {
	let c = [
		[
			a[1][1] * a[2][2] - a[1][2] * a[2][1],
			a[0][2] * a[2][1] - a[0][1] * a[2][2],
			a[0][1] * a[1][2] - a[0][2] * a[1][1],
		],
		[
			a[1][2] * a[2][0] - a[1][0] * a[2][2],
			a[0][0] * a[2][2] - a[0][2] * a[2][0],
			a[0][2] * a[1][0] - a[0][0] * a[1][2],
		],
		[
			a[1][0] * a[2][1] - a[1][1] * a[2][0],
			a[0][1] * a[2][0] - a[0][0] * a[2][1],
			a[0][0] * a[1][1] - a[0][1] * a[1][0],
		],
	];
	let det = a[0][0] * c[0][0] + a[0][1] * c[1][0] + a[0][2] * c[2][0];
	if det.abs() < 1e-12 {
		return Err(err!("A colour matrix of determinant {} has no inverse.", det;
			Invalid, Input, Numeric));
	}
	let mut m = c;
	for row in m.iter_mut() {
		for v in row.iter_mut() {
			*v /= det;
		}
	}
	Ok(m)
}

// An RGB space: the matrix from linear RGB to XYZ (D50), and the curve of each channel.
#[derive(Clone, Debug)]
pub struct RgbMatrix {
	pub m:		M3,				// row major, the colorants as columns
	pub trc:	[Curve; 3],
}

impl RgbMatrix {
	/// The matrix path of an RGB profile with an XYZ connection space.
	pub fn read(p: &Profile) -> Outcome<Self> {
		if p.head.space != Space::Rgb || p.head.pcs != Space::Xyz {
			return Err(err!(
				"Profile '{}' is a {:?} profile with a {:?} connection space, and the matrix path \
				is of an RGB profile with an XYZ one.", p.name, p.head.space, p.head.pcs;
				Invalid, Input, Mismatch));
		}
		let mut m = [[0.0; 3]; 3];
		let mut trc = [Curve::Identity, Curve::Identity, Curve::Identity];
		for i in 0..3 {
			let (xyz, sig) = (p.cols[i], ["rXYZ", "gXYZ", "bXYZ"][i]);
			let col = res!(xyz.ok_or_else(|| err!(
				"Profile '{}' has no '{}' tag, so it has no matrix path.", p.name, sig;
				Invalid, Input, Missing)));
			for j in 0..3 {
				m[j][i] = col[j];
			}
			let (curve, sig) = (&p.trcs[i], ["rTRC", "gTRC", "bTRC"][i]);
			let curve = res!(curve.as_ref().ok_or_else(|| err!(
				"Profile '{}' has no '{}' tag, so it has no matrix path.", p.name, sig;
				Invalid, Input, Missing)));
			if !curve.rises() {
				return Err(err!(
					"Tag '{}' of profile '{}' falls from its start to its end, and a curve that \
					falls cannot be inverted.", sig, p.name; Invalid, Input, Invalid));
			}
			trc[i] = curve.clone();
		}
		Ok(Self { m, trc })
	}

	/// The linear intensities of a device colour.
	pub fn linear(&self, rgb: [f64; 3]) -> [f64; 3] {
		[self.trc[0].eval(rgb[0]), self.trc[1].eval(rgb[1]), self.trc[2].eval(rgb[2])]
	}

	/// The XYZ (D50) of a device colour.
	pub fn xyz(&self, rgb: [f64; 3]) -> [f64; 3] {
		mul_v(&self.m, self.linear(rgb))
	}

	/// The Y of a device colour, the luminance a grey must match.
	pub fn y(&self, rgb: [f64; 3]) -> f64 {
		let lin = self.linear(rgb);
		self.m[1][0] * lin[0] + self.m[1][1] * lin[1] + self.m[1][2] * lin[2]
	}
}

// A grey space: the curve from the grey level to Y.
#[derive(Clone, Debug)]
pub struct Grey {
	pub trc: Curve,
}

impl Grey {
	/// The grey curve of a grey profile with an XYZ connection space.
	pub fn read(p: &Profile) -> Outcome<Self> {
		if p.head.space != Space::Gray || p.head.pcs != Space::Xyz {
			return Err(err!(
				"Profile '{}' is a {:?} profile with a {:?} connection space, and the grey path is \
				of a grey profile with an XYZ one.", p.name, p.head.space, p.head.pcs;
				Invalid, Input, Mismatch));
		}
		let trc = res!(p.ktrc.as_ref().ok_or_else(|| err!(
			"Profile '{}' has no 'kTRC' tag, so it has no grey curve.", p.name;
			Invalid, Input, Missing)));
		if !trc.rises() {
			return Err(err!(
				"Tag 'kTRC' of profile '{}' falls from its start to its end, and a curve that \
				falls cannot be inverted.", p.name; Invalid, Input, Invalid));
		}
		Ok(Self { trc: trc.clone() })
	}

	/// The Y of a grey level.
	pub fn y(&self, g: f64) -> f64 {
		self.trc.eval(g)
	}

	/// The grey level whose Y is `y`.
	pub fn level(&self, y: f64) -> f64 {
		self.trc.inverse(y)
	}
}

/// The grey level a destination grey space gives the colour `rgb` of a source RGB space.
pub fn rgb_to_grey(src: &RgbMatrix, dst: &Grey, rgb: [f64; 3]) -> f64 {
	dst.level(src.y(rgb))
}
