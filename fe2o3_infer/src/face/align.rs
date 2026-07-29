//! The similarity transform that puts five facial landmarks on a fixed template,
//! and the bilinear warp that follows it.

use crate::face::Image;

use oxedyne_fe2o3_core::prelude::*;

/// The five-point template an aligned crop is warped onto: right eye, left eye,
/// nose tip, right corner of the mouth, left corner of the mouth, in the
/// hundred and twelve pixel square the embedder consumes.
pub const TEMPLATE: [(f32, f32); 5] = [
	(38.2946, 51.6963),
	(73.5318, 51.5014),
	(56.0252, 71.7366),
	(41.5493, 92.3655),
	(70.7299, 92.2041),
];

/// The template's own centroid, carried as a constant so that the transform
/// matches the reference implementation digit for digit.
const TEMPLATE_MEAN: (f32, f32) = (56.0262, 71.9008);

/// Side of the aligned crop, in pixels.
pub const CROP: usize = 112;

/// An affine map, `[[a, b, tx], [c, d, ty]]`, taking a source point to a
/// destination point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
	/// Row-major coefficients.
	pub m:	[[f64; 3]; 2],
}

impl Affine {
	/// Applies the map to a point.
	pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
		(
			self.m[0][0] * x + self.m[0][1] * y + self.m[0][2],
			self.m[1][0] * x + self.m[1][1] * y + self.m[1][2],
		)
	}

	/// Inverts the map, which a warp needs because it walks the destination.
	pub fn invert(&self) -> Outcome<Self> {
		let det = self.m[0][0] * self.m[1][1] - self.m[0][1] * self.m[1][0];
		if det.abs() < 1e-12 {
			return Err(err!(
				"An affine map with determinant {} cannot be inverted, which means the five \
				landmarks are collinear.", det;
			Invalid, Input, Range));
		}
		let (a, b, c, d) = (self.m[0][0], self.m[0][1], self.m[1][0], self.m[1][1]);
		let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
		let tx = -(ia * self.m[0][2] + ib * self.m[1][2]);
		let ty = -(ic * self.m[0][2] + id * self.m[1][2]);
		Ok(Self { m: [[ia, ib, tx], [ic, id, ty]] })
	}
}

/// Singular value decomposition of a two by two matrix, `a = u · diag(s) · vt`.
///
/// Two by two is small enough to solve in closed form, which avoids an
/// iterative routine and keeps the result reproducible.
fn svd2(a: [[f64; 2]; 2]) -> ([[f64; 2]; 2], [f64; 2], [[f64; 2]; 2]) {
	let e = (a[0][0] + a[1][1]) / 2.0;
	let f = (a[0][0] - a[1][1]) / 2.0;
	let g = (a[1][0] + a[0][1]) / 2.0;
	let h = (a[1][0] - a[0][1]) / 2.0;
	let q = (e * e + h * h).sqrt();
	let r = (f * f + g * g).sqrt();
	let mut s0 = q + r;
	let mut s1 = q - r;
	let a1 = g.atan2(f);
	let a2 = h.atan2(e);
	let theta = (a2 - a1) / 2.0;
	let phi = (a2 + a1) / 2.0;
	let (cp, sp) = (phi.cos(), phi.sin());
	let (ct, st) = (theta.cos(), theta.sin());
	// With these two angles the decomposition reads `a = rot(phi) · s · rot(theta)`,
	// so the right factor is already the one a product wants on the right.
	let mut u = [[cp, -sp], [sp, cp]];
	let mut vt = [[ct, -st], [st, ct]];
	if s1 < 0.0 {
		s1 = -s1;
		vt[1][0] = -vt[1][0];
		vt[1][1] = -vt[1][1];
	}
	if s1 > s0 {
		core::mem::swap(&mut s0, &mut s1);
		u = [[u[0][1], u[0][0]], [u[1][1], u[1][0]]];
		vt = [[vt[1][0], vt[1][1]], [vt[0][0], vt[0][1]]];
	}
	(u, [s0, s1], vt)
}

/// Multiplies two two by two matrices.
fn mul2(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
	[
		[a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
		[a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
	]
}

/// Determinant of a two by two matrix.
fn det2(a: [[f64; 2]; 2]) -> f64 {
	a[0][0] * a[1][1] - a[0][1] * a[1][0]
}

/// Builds the similarity transform taking five detected landmarks onto the
/// template, by the Umeyama construction.
///
/// This is the least-squares rotation, uniform scale and translation, and it is
/// what the reference implementation of the embedder's preprocessing computes.
pub fn similarity(src: &[(f32, f32); 5]) -> Outcome<Affine> {
	let mut src_mean = (0.0f64, 0.0f64);
	for p in src {
		src_mean.0 += p.0 as f64;
		src_mean.1 += p.1 as f64;
	}
	src_mean.0 /= 5.0;
	src_mean.1 /= 5.0;
	let dst_mean = (TEMPLATE_MEAN.0 as f64, TEMPLATE_MEAN.1 as f64);

	let mut sd = [[0.0f64; 2]; 5];
	let mut dd = [[0.0f64; 2]; 5];
	for i in 0..5 {
		sd[i][0] = src[i].0 as f64 - src_mean.0;
		sd[i][1] = src[i].1 as f64 - src_mean.1;
		dd[i][0] = TEMPLATE[i].0 as f64 - dst_mean.0;
		dd[i][1] = TEMPLATE[i].1 as f64 - dst_mean.1;
	}

	let mut a = [[0.0f64; 2]; 2];
	for i in 0..5 {
		a[0][0] += dd[i][0] * sd[i][0];
		a[0][1] += dd[i][0] * sd[i][1];
		a[1][0] += dd[i][1] * sd[i][0];
		a[1][1] += dd[i][1] * sd[i][1];
	}
	for r in a.iter_mut() {
		for v in r.iter_mut() {
			*v /= 5.0;
		}
	}

	let mut d = [1.0f64, 1.0];
	if det2(a) < 0.0 {
		d[1] = -1.0;
	}
	let (u, s, vt) = svd2(a);
	let smax = s[0].max(s[1]);
	let tol = smax * 2.0 * (f32::MIN_POSITIVE as f64);
	let rank = (s[0] > tol) as usize + (s[1] > tol) as usize;

	let rot = if rank == 1 {
		if det2(u) * det2(vt) > 0.0 {
			mul2(u, vt)
		} else {
			let dm = [[d[0], 0.0], [0.0, -1.0]];
			mul2(u, mul2(dm, vt))
		}
	} else {
		let dm = [[d[0], 0.0], [0.0, d[1]]];
		mul2(u, mul2(dm, vt))
	};

	let mut var = 0.0f64;
	for i in 0..5 {
		var += sd[i][0] * sd[i][0];
	}
	for i in 0..5 {
		var += sd[i][1] * sd[i][1];
	}
	var /= 5.0;
	if var <= 0.0 {
		return Err(err!(
			"Five landmarks that all coincide give no scale to align by."; Invalid, Input, Range));
	}
	let scale = (s[0] * d[0] + s[1] * d[1]) / var;

	let tsx = rot[0][0] * src_mean.0 + rot[0][1] * src_mean.1;
	let tsy = rot[1][0] * src_mean.0 + rot[1][1] * src_mean.1;
	Ok(Affine { m: [
		[rot[0][0] * scale, rot[0][1] * scale, dst_mean.0 - scale * tsx],
		[rot[1][0] * scale, rot[1][1] * scale, dst_mean.1 - scale * tsy],
	] })
}

/// Warps an image through an affine map into a square crop, sampling bilinearly
/// and reading zero outside the source.
pub fn warp(img: &Image<'_>, t: &Affine, side: usize) -> Outcome<Vec<u8>> {
	let inv = res!(t.invert());
	let ch = img.channels;
	let mut out = vec![0u8; side * side * ch];
	for y in 0..side {
		for x in 0..side {
			let (sx, sy) = inv.apply(x as f64 + 0.0, y as f64 + 0.0);
			let x0 = sx.floor();
			let y0 = sy.floor();
			let fx = sx - x0;
			let fy = sy - y0;
			let dst = (y * side + x) * ch;
			for c in 0..ch {
				let p00 = img.sample(x0, y0, c);
				let p10 = img.sample(x0 + 1.0, y0, c);
				let p01 = img.sample(x0, y0 + 1.0, c);
				let p11 = img.sample(x0 + 1.0, y0 + 1.0, c);
				let top = p00 + (p10 - p00) * fx;
				let bot = p01 + (p11 - p01) * fx;
				let v = top + (bot - top) * fy;
				out[dst + c] = v.round().clamp(0.0, 255.0) as u8;
			}
		}
	}
	Ok(out)
}

/// Warps a face out of an image onto the template, giving the crop the embedder
/// consumes.
pub fn align_crop(img: &Image<'_>, landmarks: &[(f32, f32); 5]) -> Outcome<Vec<u8>> {
	let t = res!(similarity(landmarks));
	warp(img, &t, CROP)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_template_maps_to_itself() -> Outcome<()> {
		let t = res!(similarity(&TEMPLATE));
		for p in TEMPLATE.iter() {
			let (x, y) = t.apply(p.0 as f64, p.1 as f64);
			req!(((x - p.0 as f64).abs() < 1e-4), true);
			req!(((y - p.1 as f64).abs() < 1e-4), true);
		}
		Ok(())
	}

	#[test]
	fn a_scaled_and_turned_face_comes_back_to_the_template() -> Outcome<()> {
		// Place the template in a larger frame, rotated by a fifth of a radian
		// and scaled by three, and check the transform undoes exactly that.
		let (c, s) = (0.2f64.cos(), 0.2f64.sin());
		let mut src = [(0.0f32, 0.0f32); 5];
		for i in 0..5 {
			let (x, y) = (TEMPLATE[i].0 as f64, TEMPLATE[i].1 as f64);
			src[i] = (
				(3.0 * (c * x - s * y) + 100.0) as f32,
				(3.0 * (s * x + c * y) + 40.0) as f32,
			);
		}
		let t = res!(similarity(&src));
		for i in 0..5 {
			let (x, y) = t.apply(src[i].0 as f64, src[i].1 as f64);
			req!(((x - TEMPLATE[i].0 as f64).abs() < 1e-3), true);
			req!(((y - TEMPLATE[i].1 as f64).abs() < 1e-3), true);
		}
		Ok(())
	}

	#[test]
	fn a_decomposition_reproduces_its_matrix() {
		for a in [
			[[3.0f64, 1.0], [0.5, 2.0]],
			[[-1.0f64, 2.0], [3.0, 0.25]],
			[[0.0f64, 1.0], [1.0, 0.0]],
		] {
			let (u, s, vt) = svd2(a);
			let m = mul2(u, mul2([[s[0], 0.0], [0.0, s[1]]], vt));
			for r in 0..2 {
				for c in 0..2 {
					assert!((m[r][c] - a[r][c]).abs() < 1e-9, "{:?} against {:?}", m, a);
				}
			}
			assert!(s[0] >= s[1] && s[1] >= 0.0, "singular values {:?}", s);
		}
	}
}
