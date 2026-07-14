//! Colours and alpha compositing.

use oxedyne_fe2o3_core::prelude::*;

/// An 8-bit-per-channel colour with straight, non-premultiplied alpha.
///
/// Straight alpha is stored rather than premultiplied because it is what a PNG carries and what a
/// caller names a colour with. The premultiplication happens inside [`Rgba::over`], where it
/// belongs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
	/// Red.
	pub r:	u8,
	/// Green.
	pub g:	u8,
	/// Blue.
	pub b:	u8,
	/// Alpha, from 0 transparent to 255 opaque.
	pub a:	u8,
}

impl Rgba {

	/// Fully transparent.
	pub const TRANSPARENT:	Self = Self { r: 0,	g: 0,	b: 0,	a: 0	};
	/// Opaque black.
	pub const BLACK:	Self = Self { r: 0,	g: 0,	b: 0,	a: 255	};
	/// Opaque white.
	pub const WHITE:	Self = Self { r: 255,	g: 255,	b: 255,	a: 255	};

	/// Creates a colour from its four channels.
	pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
		Self { r, g, b, a }
	}

	/// Creates an opaque colour from its three colour channels.
	pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
		Self { r, g, b, a: 255 }
	}

	/// Parses a colour from a hexadecimal string, with or without a leading `#`, in either `rgb`,
	/// `rrggbb` or `rrggbbaa` form.
	pub fn from_hex(s: &str) -> Outcome<Self> {
		let h = s.strip_prefix('#').unwrap_or(s);
		let nyb = |c: u8| -> Outcome<u8> {
			match c {
				b'0'..=b'9'	=> Ok(c - b'0'),
				b'a'..=b'f'	=> Ok(c - b'a' + 10),
				b'A'..=b'F'	=> Ok(c - b'A' + 10),
				_ => Err(err!(
					"'{}' is not a hexadecimal digit, in the colour \"{}\".", c as char, s;
				Invalid, Input)),
			}
		};
		let b = h.as_bytes();
		match b.len() {
			3 => Ok(Self::opaque(
				res!(nyb(b[0])) * 17,
				res!(nyb(b[1])) * 17,
				res!(nyb(b[2])) * 17,
			)),
			6 => Ok(Self::opaque(
				(res!(nyb(b[0])) << 4) | res!(nyb(b[1])),
				(res!(nyb(b[2])) << 4) | res!(nyb(b[3])),
				(res!(nyb(b[4])) << 4) | res!(nyb(b[5])),
			)),
			8 => Ok(Self::new(
				(res!(nyb(b[0])) << 4) | res!(nyb(b[1])),
				(res!(nyb(b[2])) << 4) | res!(nyb(b[3])),
				(res!(nyb(b[4])) << 4) | res!(nyb(b[5])),
				(res!(nyb(b[6])) << 4) | res!(nyb(b[7])),
			)),
			n => Err(err!(
				"A hexadecimal colour has 3, 6 or 8 digits, but \"{}\" has {}.", s, n;
			Invalid, Input)),
		}
	}

	/// Whether this colour is fully transparent, and so paints nothing.
	pub fn is_transparent(&self) -> bool {
		self.a == 0
	}

	/// Whether this colour is fully opaque, and so needs no compositing.
	pub fn is_opaque(&self) -> bool {
		self.a == 255
	}

	/// Returns this colour with its alpha scaled by a coverage from 0 to 1.
	///
	/// This is how the rasteriser's anti-aliasing reaches the pixel: a pixel the shape half covers
	/// is painted with a colour of half the alpha.
	pub fn with_coverage(&self, cov: f32) -> Self {
		let c = cov.clamp(0.0, 1.0);
		Self {
			a: ((self.a as f32) * c + 0.5) as u8,
			..*self
		}
	}

	/// Composites this colour, as the source, over `dst`, the destination: Porter-Duff source-over.
	///
	/// Both operands carry straight alpha, so each is premultiplied, combined, and un-premultiplied
	/// on the way out.
	pub fn over(&self, dst: Self) -> Self {
		if self.is_opaque() || dst.is_transparent() {
			return *self;
		}
		if self.is_transparent() {
			return dst;
		}
		let sa = (self.a as f32) / 255.0;
		let da = (dst.a as f32) / 255.0;
		let oa = sa + da * (1.0 - sa); // Output alpha, never zero here.
		let chan = |s: u8, d: u8| -> u8 {
			let sc = (s as f32) / 255.0;
			let dc = (d as f32) / 255.0;
			let oc = (sc * sa + dc * da * (1.0 - sa)) / oa;
			(oc * 255.0 + 0.5).clamp(0.0, 255.0) as u8
		};
		Self {
			r: chan(self.r, dst.r),
			g: chan(self.g, dst.g),
			b: chan(self.b, dst.b),
			a: (oa * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_opaque_source_replaces_destination_00() {
		let src = Rgba::new(10, 20, 30, 255);
		assert_eq!(src.over(Rgba::WHITE), src);
	}

	#[test]
	fn test_transparent_source_leaves_destination_01() {
		assert_eq!(Rgba::TRANSPARENT.over(Rgba::WHITE), Rgba::WHITE);
	}

	#[test]
	fn test_half_alpha_black_over_white_is_grey_02() {
		let src = Rgba::new(0, 0, 0, 128);
		let out = src.over(Rgba::WHITE);
		assert_eq!(out.a, 255);
		// 128/255 of the way from white to black.
		assert!(out.r >= 126 && out.r <= 128, "expected mid grey, found {}", out.r);
	}

	#[test]
	fn test_coverage_scales_alpha_03() {
		let c = Rgba::new(1, 2, 3, 200).with_coverage(0.5);
		assert_eq!(c.a, 100);
		assert_eq!((c.r, c.g, c.b), (1, 2, 3));
	}

	#[test]
	fn test_hex_forms_04() -> Outcome<()> {
		assert_eq!(res!(Rgba::from_hex("#fff")), Rgba::WHITE);
		assert_eq!(res!(Rgba::from_hex("000000")), Rgba::BLACK);
		assert_eq!(res!(Rgba::from_hex("#0a141e80")), Rgba::new(10, 20, 30, 128));
		assert!(Rgba::from_hex("#xyz").is_err());
		assert!(Rgba::from_hex("#ffff").is_err());
		Ok(())
	}
}
