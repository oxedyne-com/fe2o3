//! Affine transforms in two dimensions.

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
	/// Horizontal scale.
	pub a:	f32,
	/// Vertical shear.
	pub b:	f32,
	/// Horizontal shear.
	pub c:	f32,
	/// Vertical scale.
	pub d:	f32,
	/// Horizontal translation.
	pub e:	f32,
	/// Vertical translation.
	pub f:	f32,
}

impl Default for Transform {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Transform {

	/// The transform that changes nothing.
	pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

	/// A translation by `(tx, ty)`.
	pub const fn translate(tx: f32, ty: f32) -> Self {
		Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
	}

	/// A scaling by `sx` horizontally and `sy` vertically, about the origin.
	pub const fn scale(sx: f32, sy: f32) -> Self {
		Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
	}

	/// A rotation about the origin, anticlockwise in a y-up frame, by an angle in radians.
	pub fn rotate(radians: f32) -> Self {
		let (s, c) = radians.sin_cos();
		Self { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
	}

	/// Returns the transform that applies `self` first and then `next`.
	///
	/// The order is the one a caller means when they say "scale it, then move it", which is the
	/// reverse of the order the matrices multiply in.
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

	/// Applies this transform to a point.
	pub fn apply(&self, p: Pt) -> Pt {
		Pt {
			x: self.a * p.x + self.c * p.y + self.e,
			y: self.b * p.x + self.d * p.y + self.f,
		}
	}

	/// The factor by which this transform stretches lengths, taken as the square root of the
	/// absolute determinant.
	///
	/// A curve is flattened in the space it is defined in, but the tolerance that matters is the
	/// one measured in pixels, so the tolerance is divided by this before flattening.
	pub fn scale_factor(&self) -> f32 {
		(self.a * self.d - self.b * self.c).abs().sqrt()
	}

	/// Whether this transform is the identity, and so may be skipped.
	pub fn is_identity(&self) -> bool {
		*self == Self::IDENTITY
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
}
