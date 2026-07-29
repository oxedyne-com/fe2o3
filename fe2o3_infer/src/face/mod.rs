//! Face detection and face embedding: the two things this crate exists to do.
//!
//! # Order of operations
//!
//! 1. Fit the photograph into the detector's canvas with [`letterbox`], which
//!    answers the scale needed to put the results back in the original frame.
//! 2. Run [`Detector::detect`], which gives a box, a score and five landmarks
//!    per face.
//! 3. Run [`Embedder::embed`] on the original photograph and one detection's
//!    landmarks, which warps the face onto a fixed template and answers a
//!    hundred and twenty-eight dimensional unit vector.
//! 4. Compare two of those with [`cosine`].
//!
//! # Channel order
//!
//! The two networks disagree, and neither says so. The detector was exported
//! against blue-green-red input and the embedder against red-green-blue. Both
//! entry points here take ordinary red-green-blue pixels and put the channels
//! in the order each network was trained on, so a caller never has to know.

pub mod align;
pub mod detect;
pub mod embed;

pub use align::{
	align_crop,
	similarity,
	Affine,
	CROP,
	TEMPLATE,
};
pub use detect::{
	Detection,
	Detector,
	DetectorOptions,
};
pub use embed::{
	cosine,
	Embedder,
	Embedding,
};

use oxedyne_fe2o3_core::prelude::*;

/// A borrowed, interleaved, eight-bit image.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
	/// Pixels, row-major, `channels` values per pixel.
	pub pixels:		&'a [u8],
	/// Width in pixels.
	pub width:		usize,
	/// Height in pixels.
	pub height:		usize,
	/// Values per pixel, three for red-green-blue.
	pub channels:	usize,
}

impl<'a> Image<'a> {
	/// Wraps a buffer, checking that it holds what the extents claim.
	pub fn new(pixels: &'a [u8], width: usize, height: usize, channels: usize)
		-> Outcome<Self>
	{
		let want = width * height * channels;
		if pixels.len() != want {
			return Err(err!(
				"An image of {} by {} with {} channels wants {} bytes, but {} were given.",
				width, height, channels, want, pixels.len();
			Invalid, Input, Mismatch));
		}
		Ok(Self { pixels, width, height, channels })
	}

	/// Reads one channel of one pixel, answering zero outside the frame, which
	/// is the constant border a warp needs.
	#[inline]
	pub fn sample(&self, x: f64, y: f64, c: usize) -> f64 {
		if x < 0.0 || y < 0.0 || c >= self.channels {
			return 0.0;
		}
		let (xi, yi) = (x as usize, y as usize);
		if xi >= self.width || yi >= self.height {
			return 0.0;
		}
		self.pixels[(yi * self.width + xi) * self.channels + c] as f64
	}
}

/// What a letterbox did, so that a result can be put back in the original frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
	/// Multiplier applied to the original, at most one.
	pub scale:	f64,
	/// Canvas width.
	pub width:	usize,
	/// Canvas height.
	pub height:	usize,
}

impl Letterbox {
	/// Maps a point on the canvas back to the original frame.
	pub fn back(&self, x: f32, y: f32) -> (f32, f32) {
		((x as f64 / self.scale) as f32, (y as f64 / self.scale) as f32)
	}
}

/// Fits an image into a canvas of the given size, keeping the aspect ratio and
/// leaving the unused right and lower margin black.
///
/// Downscaling averages over the source footprint rather than taking one sample
/// from it, because a face sixty pixels across in a four thousand pixel
/// photograph is ten pixels across in a six hundred and forty pixel canvas, and
/// a point sample of it is noise.
pub fn letterbox(img: &Image<'_>, width: usize, height: usize)
	-> Outcome<(Vec<u8>, Letterbox)>
{
	if width == 0 || height == 0 || img.width == 0 || img.height == 0 {
		return Err(err!(
			"A letterbox of {} by {} from {} by {} has no area.",
			width, height, img.width, img.height;
		Invalid, Input, Range));
	}
	let ch = img.channels;
	let scale = (width as f64 / img.width as f64).min(height as f64 / img.height as f64);
	let dw = ((img.width as f64 * scale).round() as usize).clamp(1, width);
	let dh = ((img.height as f64 * scale).round() as usize).clamp(1, height);
	let mut out = vec![0u8; width * height * ch];

	if scale <= 1.0 {
		// Area average: each destination pixel is the mean of the source
		// rectangle that maps onto it.
		for y in 0..dh {
			let y0 = (y as f64 * img.height as f64 / dh as f64).floor() as usize;
			let y1 = (((y + 1) as f64 * img.height as f64 / dh as f64).ceil() as usize)
				.clamp(y0 + 1, img.height);
			for x in 0..dw {
				let x0 = (x as f64 * img.width as f64 / dw as f64).floor() as usize;
				let x1 = (((x + 1) as f64 * img.width as f64 / dw as f64).ceil() as usize)
					.clamp(x0 + 1, img.width);
				let n = ((y1 - y0) * (x1 - x0)) as f64;
				for c in 0..ch {
					let mut s = 0.0f64;
					for sy in y0..y1 {
						for sx in x0..x1 {
							s += img.pixels[(sy * img.width + sx) * ch + c] as f64;
						}
					}
					out[(y * width + x) * ch + c] = (s / n).round().clamp(0.0, 255.0) as u8;
				}
			}
		}
	} else {
		// Bilinear, which is what an enlargement wants.
		for y in 0..dh {
			let sy = (y as f64 + 0.5) / scale - 0.5;
			let yb = sy.floor();
			let fy = sy - yb;
			for x in 0..dw {
				let sx = (x as f64 + 0.5) / scale - 0.5;
				let xb = sx.floor();
				let fx = sx - xb;
				for c in 0..ch {
					let p00 = img.sample(xb.max(0.0), yb.max(0.0), c);
					let p10 = img.sample((xb + 1.0).max(0.0), yb.max(0.0), c);
					let p01 = img.sample(xb.max(0.0), (yb + 1.0).max(0.0), c);
					let p11 = img.sample((xb + 1.0).max(0.0), (yb + 1.0).max(0.0), c);
					let top = p00 + (p10 - p00) * fx;
					let bot = p01 + (p11 - p01) * fx;
					let v = top + (bot - top) * fy;
					out[(y * width + x) * ch + c] = v.round().clamp(0.0, 255.0) as u8;
				}
			}
		}
	}
	Ok((out, Letterbox { scale, width, height }))
}
