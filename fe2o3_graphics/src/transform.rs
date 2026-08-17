//! Affine transforms in two dimensions.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::path::Pt;

/// A 2D affine transform.
///
/// The six coefficients are those of the matrix
///
/// ```text
/// | a  c  e |
/// | b  d  f |
/// | 0  0  1 |
/// ```
///
/// which maps a point `(x, y)` to `(a·x + c·y + e, b·x + d·y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
	pub a:	f32,	// horizontal scale
	pub b:	f32,	// vertical shear
	pub c:	f32,	// horizontal shear
	pub d:	f32,	// vertical scale
	pub e:	f32,	// horizontal translation
	pub f:	f32,	// vertical translation
}

impl Default for Transform {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Transform {

	pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

	pub const fn translate(tx: f32, ty: f32) -> Self {
		Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
	}

	/// Scales about the origin.
	pub const fn scale(sx: f32, sy: f32) -> Self {
		Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
	}

	/// Rotates about the origin, anticlockwise in a y-up frame, by an angle in radians.
	pub fn rotate(radians: f32) -> Self {
		let (s, c) = radians.sin_cos();
		Self { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
	}

	/// Applies `self` first and then `next`.  That is the order a caller means when they say "scale
	/// it, then move it", which is the reverse of the order the matrices multiply in.
	pub fn then(&self, next: &Self) -> Self {
		Self {
			a: next.a * self.a + next.c * self.b,
			b: next.b * self.a + next.d * self.b,
			c: next.a * self.c + next.c * self.d,
			d: next.b * self.c + next.d * self.d,
			e: next.a * self.e + next.c * self.f + next.e,
			f: next.b * self.e + next.d * self.f + next.f,
		}
	}

	pub fn apply(&self, p: Pt) -> Pt {
		Pt {
			x: self.a * p.x + self.c * p.y + self.e,
			y: self.b * p.x + self.d * p.y + self.f,
		}
	}

	/// The square root of the absolute determinant, which is the factor by which lengths stretch.
	///
	/// A curve is flattened in the space it is defined in, but the tolerance that matters is the
	/// one measured in pixels, so the tolerance is divided by this before flattening.
	pub fn scale_factor(&self) -> f32 {
		(self.a * self.d - self.b * self.c).abs().sqrt()
	}

	/// Is this the identity, and so skippable?
	pub fn is_identity(&self) -> bool {
		*self == Self::IDENTITY
	}

	/// A transform with a zero determinant has collapsed the plane onto a line or a point and
	/// cannot be undone, since everything on that line came from somewhere different. A caller
	/// carrying a pixel back into the coordinates a shape was defined in -- to read a gradient, a
	/// pattern or a texture there -- is what this is for.
	pub fn invert(&self) -> Option<Self> {
		let det = self.a * self.d - self.b * self.c;
		if det == 0.0 || !det.is_finite() {
			return None;
		}
		let k = 1.0 / det;
		Some(Self {
			a: self.d * k,
			b: -self.b * k,
			c: -self.c * k,
			d: self.a * k,
			e: (self.c * self.f - self.d * self.e) * k,
			f: (self.b * self.e - self.a * self.f) * k,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_identity_leaves_a_point_00() {
		let p = Pt::new(3.0, 4.0);
		assert_eq!(Transform::IDENTITY.apply(p), p);
	}

	#[test]
	fn test_scale_then_translate_01() {
		// Scale by two, then move right by ten: the point (1, 1) lands at (12, 2).
		let t = Transform::scale(2.0, 2.0).then(&Transform::translate(10.0, 0.0));
		assert_eq!(t.apply(Pt::new(1.0, 1.0)), Pt::new(12.0, 2.0));
	}

	#[test]
	fn test_translate_then_scale_differs_02() {
		// The other order: move right by ten, then scale by two, landing at (22, 2).
		let t = Transform::translate(10.0, 0.0).then(&Transform::scale(2.0, 2.0));
		assert_eq!(t.apply(Pt::new(1.0, 1.0)), Pt::new(22.0, 2.0));
	}

	#[test]
	fn test_scale_factor_03() {
		assert_eq!(Transform::scale(3.0, 3.0).scale_factor(), 3.0);
		assert_eq!(Transform::IDENTITY.scale_factor(), 1.0);
	}
	#[test]
	fn test_a_transform_and_its_inverse_return_a_point_04() {
		let t = Transform::scale(3.0, -2.0)
			.then(&Transform::rotate(0.7))
			.then(&Transform::translate(11.0, -4.0));
		let inv = match t.invert() {
			Some(inv) => inv,
			None => panic!("a transform of non-zero determinant must invert"),
		};
		for p in [Pt::new(0.0, 0.0), Pt::new(1.0, 0.0), Pt::new(-13.5, 7.25)] {
			let back = inv.apply(t.apply(p));
			assert!((back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3,
				"({}, {}) came back as ({}, {})", p.x, p.y, back.x, back.y);
		}
	}

	#[test]
	fn test_a_collapsed_transform_has_no_inverse_05() {
		// A scale of zero on one axis folds the plane onto a line, and everything on that line
		// came from somewhere different.
		assert!(Transform::scale(1.0, 0.0).invert().is_none());
		assert!(Transform::scale(0.0, 0.0).invert().is_none());
		assert!(Transform::IDENTITY.invert().is_some());
	}

}
