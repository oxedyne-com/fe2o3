//! One decoded planar picture, and the one way out of it into something anybody can look at.
//!
//! Both video decoders in this crate produce the same thing -- three planes of brightness and
//! colour difference, the colour ones at half the width and half the height -- and for a while they
//! produced it as two unrelated types, with the conversion into red, green and blue written against
//! only one of them. So a caller that had decoded an H.264 frame held a picture with no way to draw
//! it, and a caller that wanted to serve either codec had to know which it had.
//!
//! This module names the shared form. [`Frame`] is the picture, [`Plane`] is one of its components,
//! and [`rgb`] is the conversion; an H.264 picture reaches all three through `From`, which widens
//! its eight-bit samples into the sixteen-bit ones a picture of any depth needs. Nothing here is a
//! second implementation of anything: the type and the conversion are `hevc`'s, and this is where
//! they are named as belonging to neither codec.

pub use crate::hevc::{
	colour::{
		rgb,
		Matrix,
	},
	decode::{
		Picture as Frame,
		Plane,
	},
};

use crate::{
	h264,
	hevc,
};

impl From<&h264::decode::Picture> for hevc::decode::Picture {

	/// Widens an H.264 picture into the shared form.
	///
	/// H.264 as this crate reads it is eight bits a sample and the shared plane holds sixteen, so
	/// the samples are widened and the depth is stated rather than assumed. The numbers do not
	/// change: a sample of 235 is 235 at either width, and the studio range [`rgb`] undoes is the
	/// same range.
	fn from(pic: &h264::decode::Picture) -> Self {
		let plane = |p: &h264::decode::Plane| hevc::decode::Plane {
			w:	p.w,
			h:	p.h,
			px:	p.px.iter().map(|v| *v as u16).collect(),
		};
		Self {
			y:	plane(&pic.y),
			cb:	plane(&pic.cb),
			cr:	plane(&pic.cr),
			depth:	8,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use oxedyne_fe2o3_core::prelude::*;

	/// An H.264 picture of one colour throughout.
	fn flat(w: usize, h: usize, y: u8, cb: u8, cr: u8) -> h264::decode::Picture {
		let plane = |w: usize, h: usize, v: u8| h264::decode::Plane { w, h, px: vec![v; w * h] };
		h264::decode::Picture {
			y:	plane(w, h, y),
			cb:	plane(w / 2, h / 2, cb),
			cr:	plane(w / 2, h / 2, cr),
		}
	}

	#[test]
	fn test_an_h264_picture_reaches_the_shared_conversion_00() -> Outcome<()> {
		// The point of the module: a picture out of the other decoder is drawable at all, and it
		// comes out with the same studio range undone. A conversion that read the widened samples
		// as though they were ten-bit would put this well away from white.
		let white: Frame = (&flat(8, 8, 235, 128, 128)).into();
		req!(white.depth, 8u32);
		req!(white.y.w, 8usize);
		let px = res!(rgb(&white, Matrix::Hd, false));
		req!(px.data()[0], 255u8, "the brightest studio level is not white");
		let black: Frame = (&flat(8, 8, 16, 128, 128)).into();
		let px = res!(rgb(&black, Matrix::Hd, false));
		req!(px.data()[0], 0u8, "the darkest studio level is not black");
		Ok(())
	}

	#[test]
	fn test_widening_changes_no_sample_01() -> Outcome<()> {
		// Every sample the same number at either width, and the colour planes still half size.
		let src = flat(4, 4, 90, 40, 200);
		let wide: Frame = (&src).into();
		for (a, b) in src.y.px.iter().zip(wide.y.px.iter()) {
			req!(*b, *a as u16);
		}
		req!(wide.cb.w, 2usize);
		req!(wide.cr.h, 2usize);
		Ok(())
	}
}
