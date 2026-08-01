//! Turning a decoded picture into something that can be looked at.
//!
//! A coded picture is three planes of brightness and colour difference, the colour ones at half the
//! width and half the height, and every number in them is in a *studio* range rather than a full
//! one: brightness runs from 16 to 235 and colour from 16 to 240, with the room at each end left
//! for signals that overshoot. Getting that range wrong is the commonest fault in a hand-written
//! conversion, and it looks like a photograph with no real black in it.
//!
//! The matrix is the other half. Two are in wide use -- the older one from standard-definition
//! television and the one from high definition -- and a photograph out of a phone is coded against
//! the second. They differ by a few per cent in the green channel, which is enough to shift a face.

use crate::{
	hevc::decode::Picture,
	pixmap::Pixmap,
};

use oxedyne_fe2o3_core::prelude::*;

/// Which set of weights turns colour difference back into red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
	/// Rec. 601, from standard-definition television.
	Sd,
	/// Rec. 709, which is what a photograph out of a phone is coded against.
	Hd,
}

impl Matrix {

	/// The two weights the conversion is built from: how much of the luminance is red, and how much
	/// is blue. Green is what is left.
	fn weights(self) -> (f32, f32) {
		match self {
			Self::Sd	=> (0.299, 0.114),
			Self::Hd	=> (0.2126, 0.0722),
		}
	}
}

/// Turns a decoded picture into eight-bit RGBA.
///
/// The colour planes are half size, so each of their samples covers four of the brightness plane's;
/// they are stretched by **bilinear interpolation between sample centres**, which for 4:2:0 means
/// the chroma sample sits a quarter of a pixel up and to the left of the luma one it is named for.
/// Repeating each sample four times instead is visibly blockier along a hard colour edge -- a red
/// jumper against a white wall is where it shows.
pub fn rgb(pic: &Picture, matrix: Matrix, full_range: bool) -> Outcome<Pixmap> {
	let (w, h) = (pic.y.w, pic.y.h);
	if w == 0 || h == 0 {
		return Err(err!("A picture of {} by {} has nothing in it.", w, h; Invalid, Input));
	}
	let (kr, kb) = matrix.weights();
	let kg = 1.0 - kr - kb;
	// From colour difference back to the two channels that carry it, and thence to green.
	let (vr, ub) = (2.0 * (1.0 - kr), 2.0 * (1.0 - kb));
	let top = ((1u32 << pic.depth) - 1) as f32;
	// The studio range, scaled to whatever depth the picture is coded at.
	let (y_low, y_span, c_span) = if full_range {
		(0.0, top, top)
	} else {
		let one = (1u32 << (pic.depth - 8)) as f32;
		(16.0 * one, 219.0 * one, 224.0 * one)
	};
	let c_mid = ((1u32 << (pic.depth - 1)) as f32).floor();

	let mut out = vec![0u8; w * h * 4];
	for y in 0..h {
		for x in 0..w {
			let luma = match pic.y.at(x, y) {
				Some(v) => v as f32,
				None => continue,
			};
			let (cb, cr) = chroma_at(pic, x, y);
			let l = ((luma - y_low) / y_span).clamp(0.0, 1.0);
			let u = (cb - c_mid) / c_span;
			let v = (cr - c_mid) / c_span;
			let r = l + vr * v;
			let b = l + ub * u;
			let g = (l - kr * r - kb * b) / kg;
			let at = (y * w + x) * 4;
			out[at] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
			out[at + 1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
			out[at + 2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
			out[at + 3] = 255;
		}
	}
	Pixmap::from_data(w, h, out)
}

/// The colour difference at one brightness sample, interpolated between the four around it.
///
/// In 4:2:0 a colour sample sits between brightness samples both ways, which is why the offsets are
/// halves rather than whole steps.
fn chroma_at(pic: &Picture, x: usize, y: usize) -> (f32, f32) {
	let fx = (x as f32 - 0.5) / 2.0;
	let fy = (y as f32 - 0.5) / 2.0;
	let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
	let (dx, dy) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
	let (x1, y1) = ((x0 + 1).min(pic.cb.w.saturating_sub(1)), (y0 + 1).min(pic.cb.h.saturating_sub(1)));
	let take = |p: &crate::hevc::decode::Plane| {
		let a = p.at(x0, y0).unwrap_or(0) as f32;
		let b = p.at(x1, y0).unwrap_or(a as u16) as f32;
		let c = p.at(x0, y1).unwrap_or(a as u16) as f32;
		let d = p.at(x1, y1).unwrap_or(a as u16) as f32;
		(a * (1.0 - dx) + b * dx) * (1.0 - dy) + (c * (1.0 - dx) + d * dx) * dy
	};
	(take(&pic.cb), take(&pic.cr))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::hevc::decode::Plane;

	/// A picture of one colour throughout.
	fn flat(w: usize, h: usize, y: u16, cb: u16, cr: u16) -> Picture {
		let plane = |w: usize, h: usize, v: u16| Plane { w, h, px: vec![v; w * h] };
		Picture {
			y:	plane(w, h, y),
			cb:	plane(w / 2, h / 2, cb),
			cr:	plane(w / 2, h / 2, cr),
			depth:	8,
		}
	}

	#[test]
	fn test_the_studio_range_reaches_black_and_white_00() -> Outcome<()> {
		// The fault a hand-written conversion nearly always has: a picture whose darkest sample is
		// 16 must come out as nought and not as 16, or nothing in the library has a real black in
		// it. The same at the top, where 235 is white.
		let black = res!(rgb(&flat(4, 4, 16, 128, 128), Matrix::Hd, false));
		req!(black.data()[0], 0u8, "the darkest studio level is not black");
		let white = res!(rgb(&flat(4, 4, 235, 128, 128), Matrix::Hd, false));
		req!(white.data()[0], 255u8, "the brightest studio level is not white");
		// And with a full-range picture the same numbers mean what they say.
		let full = res!(rgb(&flat(4, 4, 16, 128, 128), Matrix::Hd, true));
		req!(full.data()[0], 16u8, "a full-range picture was stretched anyway");
		Ok(())
	}

	#[test]
	fn test_no_colour_difference_is_a_grey_01() -> Outcome<()> {
		// With both colour planes at their middle, the three channels must agree exactly, whatever
		// the matrix. A conversion with a sign or a weight wrong shows here as a green or magenta
		// cast over the whole library.
		for matrix in [Matrix::Sd, Matrix::Hd] {
			for level in [16u16, 60, 128, 200, 235] {
				let grey = res!(rgb(&flat(4, 4, level, 128, 128), matrix, false));
				let px = grey.data();
				req!(px[0], px[1], "{:?} at {} is not grey", matrix, level);
				req!(px[1], px[2], "{:?} at {} is not grey", matrix, level);
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_colour_difference_axes_point_the_right_way_02() -> Outcome<()> {
		// Cr carries red and Cb carries blue, and swapping the two is a mistake that survives every
		// test written in greys. A picture with Cr well above the middle must come out red.
		let red = res!(rgb(&flat(4, 4, 128, 128, 240), Matrix::Hd, false));
		let px = red.data();
		let reddest = px[0] > px[1] && px[0] > px[2];
		req!(reddest, true, "high Cr gave {:?} rather than a red", &px[..3]);
		let blue = res!(rgb(&flat(4, 4, 128, 240, 128), Matrix::Hd, false));
		let px = blue.data();
		let bluest = px[2] > px[0] && px[2] > px[1];
		req!(bluest, true, "high Cb gave {:?} rather than a blue", &px[..3]);
		Ok(())
	}
}
