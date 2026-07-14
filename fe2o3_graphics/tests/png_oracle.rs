//! Decode PNGs that another implementation wrote, and check we read the pixels it says they hold.
//!
//! The unit tests in `png.rs` build their PNGs by hand from the specification, which is a stronger
//! guard than a round trip but still one written by the same hand as the decoder: a
//! misunderstanding of the format would be encoded into the fixture and the reader alike, and the
//! test would pass.  Our encoder only ever writes RGBA, so it emits no `tRNS` chunk at all, which
//! is precisely why a round-trip suite could not construct an input that would fail -- and why the
//! decoder ignored `tRNS` for as long as it did.
//!
//! The fixtures in `tests/png/` were therefore written by Pillow, and the expected pixels below are
//! Pillow's own reading of them, taken from the files on disk.  Nothing here originates with us, so
//! agreement means agreement with an independent implementation.  Against the decoder as it stood
//! before `tRNS` was read, the three transparent fixtures each decode fully opaque, and this test
//! fails.
//!
//! To regenerate: see `tests/png/gen.py`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::png;

use std::{
	fs,
	path::PathBuf,
};

/// Fixture name, dimensions, and the RGBA pixels Pillow reads from it, in row-major order.
type Case = (&'static str, usize, usize, &'static [(u8, u8, u8, u8)]);

const CASES: &[Case] = &[
	// Greyscale, with a `tRNS` chunk naming one luminance as the transparent one.
	(
		"grey_trns",
		4, 4,
		&[(0,0,0,255), (64,64,64,255), (128,128,128,0), (255,255,255,255), (0,0,0,255), (64,64,64,255), (128,128,128,0), (255,255,255,255), (0,0,0,255), (64,64,64,255), (128,128,128,0), (255,255,255,255), (0,0,0,255), (64,64,64,255), (128,128,128,0), (255,255,255,255)],
	),
	// Palette, with a `tRNS` chunk giving one alpha byte per palette entry: transparent, half, opaque.
	(
		"palette_trns",
		4, 4,
		&[(0,0,0,0), (7,13,29,128), (14,26,58,255), (21,39,87,255), (0,0,0,0), (7,13,29,128), (14,26,58,255), (21,39,87,255), (0,0,0,0), (7,13,29,128), (14,26,58,255), (21,39,87,255), (0,0,0,0), (7,13,29,128), (14,26,58,255), (21,39,87,255)],
	),
	// Truecolour, with a `tRNS` chunk naming one RGB triple as the transparent one.
	(
		"rgb_trns",
		4, 4,
		&[(255,0,0,255), (0,255,0,0), (0,0,255,255), (10,20,30,255), (255,0,0,255), (0,255,0,0), (0,0,255,255), (10,20,30,255), (255,0,0,255), (0,255,0,0), (0,0,255,255), (10,20,30,255), (255,0,0,255), (0,255,0,0), (0,0,255,255), (10,20,30,255)],
	),
	// A control: no `tRNS`, so every pixel is opaque, and reading `tRNS` must not change that.
	(
		"rgb_plain",
		4, 4,
		&[(1,2,3,255), (4,5,6,255), (7,8,9,255), (10,11,12,255), (1,2,3,255), (4,5,6,255), (7,8,9,255), (10,11,12,255), (1,2,3,255), (4,5,6,255), (7,8,9,255), (10,11,12,255), (1,2,3,255), (4,5,6,255), (7,8,9,255), (10,11,12,255)],
	),
	// A control: alpha carried in the image data, where it always worked, and must still.
	(
		"rgba_plain",
		4, 4,
		&[(1,2,3,0), (4,5,6,85), (7,8,9,170), (10,11,12,255), (1,2,3,0), (4,5,6,85), (7,8,9,170), (10,11,12,255), (1,2,3,0), (4,5,6,85), (7,8,9,170), (10,11,12,255), (1,2,3,0), (4,5,6,85), (7,8,9,170), (10,11,12,255)],
	),
];

#[test]
fn test_pillows_pngs_decode_to_the_pixels_pillow_reads_from_them() -> Outcome<()> {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("png");
	for (name, w, h, expected) in CASES {
		let path = dir.join(fmt!("{}.png", name));
		let buf = res!(fs::read(&path), IO, File);
		let pm = res!(png::decode(&buf), Decode, Input);

		req!(pm.width(), *w, "Width of {}.", name);
		req!(pm.height(), *h, "Height of {}.", name);

		for (i, exp) in expected.iter().enumerate() {
			let (x, y) = (i % w, i / w);
			let c = match pm.pixel(x, y) {
				Some(c) => c,
				None => return Err(err!(
					"The fixture {} has no pixel at ({}, {}).", name, x, y;
				Test, Missing)),
			};
			let got = (c.r, c.g, c.b, c.a);
			req!(got, *exp,
				"Pixel ({}, {}) of {}: an independent decoder reads {:?}, we read {:?}.",
				x, y, name, exp, got);
		}
	}
	Ok(())
}
