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

	/// Renders this colour as a hexadecimal string with a leading `#`.
	///
	/// The inverse of [`Rgba::from_hex`]: an opaque colour comes back as `#rrggbb`, and one with
	/// alpha as `#rrggbbaa`, so a colour written out and read back in is the colour it began as. The
	/// three-digit short form is never emitted, since most colours do not fit it and a writer that
	/// only sometimes shortened would be the harder thing to reason about.
	pub fn to_hex(&self) -> String {
		if self.is_opaque() {
			fmt!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
		} else {
			fmt!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
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

	/// The WCAG relative luminance of this colour, from 0 for black to 1 for white.
	///
	/// Each channel is taken back from the display-encoded sRGB the colour is stored in to the
	/// linear-light value the eye weighs, by the sRGB transfer function, and the three are then
	/// combined with the luminance weights the standard gives. This is the quantity two colours'
	/// contrast is measured from, so it is worked in `f64`: the contrast of near-black text against
	/// black turns on small differences the wider type keeps.
	///
	/// Alpha is ignored. Luminance is a property of a colour once it is on the screen, and a colour
	/// with alpha is not yet on the screen -- a caller wanting the luminance of a translucent colour
	/// over a background should composite it with [`Rgba::over`] first.
	pub fn relative_luminance(&self) -> f64 {
		// The sRGB transfer function, taking one channel from encoded 0..255 to linear 0..1.
		let lin = |c: u8| -> f64 {
			let c = (c as f64) / 255.0;
			if c <= 0.03928 {
				c / 12.92
			} else {
				((c + 0.055) / 1.055).powf(2.4)
			}
		};
		0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
	}

	/// The WCAG contrast ratio between this colour and another, from 1 for two equal colours to 21
	/// for black against white.
	///
	/// The ratio is `(L1 + 0.05) / (L2 + 0.05)`, where `L1` is the lighter of the two relative
	/// luminances and `L2` the darker, and the `0.05` is the flare the standard adds for the light a
	/// real screen reflects even where it shows black. The result does not depend on which colour is
	/// named first: the brighter is always taken as `L1`.
	///
	/// WCAG asks 4.5 of body text and 3 of large text for its AA level, and 7 and 4.5 for AAA.
	pub fn contrast_ratio(&self, other: &Self) -> f64 {
		let a = self.relative_luminance();
		let b = other.relative_luminance();
		let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
		(hi + 0.05) / (lo + 0.05)
	}

	/// This colour as it would look to an eye with a colour-vision deficiency.
	///
	/// The simulation is a single linear map of the sRGB channels, the standard matrix for each of
	/// the three dichromacies (see [`ColourVision`]). It is the approximation used across the web
	/// tooling that checks a palette for the roughly one man in twelve who cannot tell red from
	/// green: not a model of the retina, but enough to show whether two colours a design leans on
	/// collapse into one for a viewer who lacks a cone. Alpha is carried through unchanged.
	pub fn simulate(&self, cvd: ColourVision) -> Self {
		let m = cvd.matrix();
		// The channels stay in sRGB: these matrices are fitted to the encoded values, not to
		// linear light, so no transfer function is applied on the way through.
		let (r, g, b) = (self.r as f32, self.g as f32, self.b as f32);
		let ch = |row: [f32; 3]| -> u8 {
			(row[0] * r + row[1] * g + row[2] * b + 0.5).clamp(0.0, 255.0) as u8
		};
		Self {
			r: ch(m[0]),
			g: ch(m[1]),
			b: ch(m[2]),
			a: self.a,
		}
	}
}

/// A form of colour blindness, for [`Rgba::simulate`] to show a colour through.
///
/// The three dichromacies, each the loss of one of the eye's three cones. Protanopia and
/// deuteranopia are the two red-green kinds and together much the most common; tritanopia, the
/// blue-yellow kind, is rare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColourVision {
	/// Red-blind: the long-wavelength cone is missing.
	Protanopia,
	/// Green-blind: the medium-wavelength cone is missing.
	Deuteranopia,
	/// Blue-blind: the short-wavelength cone is missing.
	Tritanopia,
}

impl ColourVision {

	/// The simulation matrix for this deficiency: three rows, each the weights that make one output
	/// channel from the three input channels.
	///
	/// These are the widely used dichromat matrices that operate directly on sRGB. Each row sums to
	/// one, so a grey is left where it was and only the hues that the missing cone distinguished are
	/// folded together.
	fn matrix(&self) -> [[f32; 3]; 3] {
		match self {
			Self::Protanopia => [
				[0.567, 0.433, 0.000],
				[0.558, 0.442, 0.000],
				[0.000, 0.242, 0.758],
			],
			Self::Deuteranopia => [
				[0.625, 0.375, 0.000],
				[0.700, 0.300, 0.000],
				[0.000, 0.300, 0.700],
			],
			Self::Tritanopia => [
				[0.950, 0.050, 0.000],
				[0.000, 0.433, 0.567],
				[0.000, 0.475, 0.525],
			],
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

	#[test]
	fn test_hex_round_trips_through_the_reader_05() -> Outcome<()> {
		// A colour written out and read back is the colour it began as: to_hex is the inverse of
		// from_hex, opaque in six digits and translucent in eight.
		for c in [
			Rgba::BLACK,
			Rgba::WHITE,
			Rgba::new(10, 20, 30, 255),
			Rgba::new(10, 20, 30, 128),
			Rgba::new(1, 2, 3, 0),
		] {
			assert_eq!(res!(Rgba::from_hex(&c.to_hex())), c, "{} did not round trip", c.to_hex());
		}
		assert_eq!(Rgba::WHITE.to_hex(), "#ffffff");
		assert_eq!(Rgba::new(10, 20, 30, 128).to_hex(), "#0a141e80");
		Ok(())
	}

	#[test]
	fn test_black_on_white_is_the_maximum_contrast_06() {
		// The published anchor: black on white is exactly 21:1, and white on itself is 1:1. These
		// are the two ends of the WCAG scale and fix both the luminances and the ratio formula.
		let bw = Rgba::BLACK.contrast_ratio(&Rgba::WHITE);
		assert!((bw - 21.0).abs() < 1e-6, "black on white should be 21:1, found {}", bw);
		let ww = Rgba::WHITE.contrast_ratio(&Rgba::WHITE);
		assert!((ww - 1.0).abs() < 1e-6, "white on white should be 1:1, found {}", ww);
	}

	#[test]
	fn test_the_contrast_ratio_does_not_depend_on_order_07() {
		// The lighter colour is always taken as L1, so naming the pair either way gives one answer.
		let a = Rgba::new(0x76, 0x76, 0x76, 255);
		assert_eq!(a.contrast_ratio(&Rgba::WHITE), Rgba::WHITE.contrast_ratio(&a));
	}

	#[test]
	fn test_the_aa_reference_grey_meets_the_threshold_08() {
		// #767676 on white is the grey WCAG's own reference gives as ~4.54:1 -- the darkest grey
		// that clears the 4.5:1 AA bar for body text. This checks the sRGB linearisation against a
		// published value, not against the formula restated.
		let grey = Rgba::new(0x76, 0x76, 0x76, 255);
		let r = grey.contrast_ratio(&Rgba::WHITE);
		assert!((r - 4.54).abs() < 0.02, "#767676 on white should be ~4.54:1, found {}", r);
	}

	#[test]
	fn test_a_grey_is_unmoved_by_a_deficiency_09() {
		// Every simulation matrix has rows that sum to one, so an achromatic colour, which loses no
		// hue because it has none, comes back where it was.
		let grey = Rgba::new(128, 128, 128, 200);
		for cvd in [ColourVision::Protanopia, ColourVision::Deuteranopia, ColourVision::Tritanopia] {
			let out = grey.simulate(cvd);
			assert!((out.r as i32 - 128).abs() <= 1, "{:?} moved a grey to {}", cvd, out.r);
			assert!((out.g as i32 - 128).abs() <= 1, "{:?} moved a grey to {}", cvd, out.g);
			assert!((out.b as i32 - 128).abs() <= 1, "{:?} moved a grey to {}", cvd, out.b);
			assert_eq!(out.a, 200, "alpha must be carried through");
		}
	}

	#[test]
	fn test_red_and_green_collapse_under_protanopia_10() {
		// The point of the simulation: a red and a green a design might rely on to differ become
		// nearly the same colour to a red-green-blind eye. Measured as a distance through the RGB
		// cube, which turns on hue and not just brightness, the gap between them falls to a fraction
		// of what it was once both are seen through protanopia.
		let red = Rgba::new(230, 40, 40, 255);
		let green = Rgba::new(40, 180, 40, 255);
		let dist = |a: Rgba, b: Rgba| -> f32 {
			let dr = a.r as f32 - b.r as f32;
			let dg = a.g as f32 - b.g as f32;
			let db = a.b as f32 - b.b as f32;
			(dr * dr + dg * dg + db * db).sqrt()
		};
		let normal = dist(red, green);
		let seen = dist(
			red.simulate(ColourVision::Protanopia),
			green.simulate(ColourVision::Protanopia),
		);
		assert!(
			seen < 0.4 * normal,
			"red and green stood {} apart but should collapse under protanopia, found {}",
			normal, seen,
		);
	}
}
