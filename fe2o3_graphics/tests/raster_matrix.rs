//! Decode BMP, GIF and TIFF fixtures written by ImageMagick and Pillow, and check the pixels against
//! ImageMagick's own reading of each file.
//!
//! `tests/raster/gen.sh` writes every fixture and, beside it, `<name>.png`: ImageMagick's decode of
//! that fixture as eight-bit RGBA (`<name>.p<n>.png` for page `n` of a multipage TIFF). That PNG is
//! read here with this crate's PNG decoder, which `png_matrix.rs` holds to the same independent
//! standard, so nothing in a comparison originates in the codec under test.
//!
//! Where both readers agree a pixel is fully transparent, its colour is not compared: neither format
//! defines it and the two readers fill it differently.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	convert,
	pixmap::Pixmap,
	png,
};
use oxedyne_fe2o3_stds::media::Media;

use std::{
	fs,
	path::PathBuf,
};

// A fixture: its file name, its page count, and the largest difference a channel may show.
type Case = (&'static str, usize, u8);

const CASES: &[Case] = &[
	// BMP
	("bmp1_text.bmp",		1, 0),
	("bmp1_text_pil.bmp",		1, 0),
	("bmp4.bmp",			1, 0),
	("bmp4_rle.bmp",		1, 0),
	("bmp8.bmp",			1, 0),
	("bmp8_rle.bmp",		1, 0),
	("bmp16_555.bmp",		1, 1),	// five bits scaled to eight, rounded differently
	("bmp16_565.bmp",		1, 1),
	("bmp24.bmp",			1, 0),
	("bmp24_os2.bmp",		1, 0),
	("bmp24_topdown.bmp",		1, 0),
	("bmp32_v5.bmp",		1, 0),
	// GIF
	("gif_plain.gif",		1, 0),
	("gif_interlaced.gif",		1, 0),
	("gif_transparent.gif",		1, 0),
	("gif_text.gif",		1, 0),
	// TIFF, one bit
	("tif_g1_none.tif",		1, 0),
	("tif_g1_packbits.tif",		1, 0),
	("tif_g1_lzw.tif",		1, 0),
	("tif_fax_mh.tif",		1, 0),
	("tif_fax_g3_1d.tif",		1, 0),
	("tif_fax_g3_2d.tif",		1, 0),
	("tif_fax_g3_2d_fill.tif",	1, 0),
	("tif_fax_g3_wide.tif",		1, 0),
	("tif_fax_g4.tif",		1, 0),
	("tif_fax_g4_wide.tif",		1, 0),
	("tif_fax_g4_lsb.tif",		1, 0),
	("tif_fax_g4_strips.tif",	1, 0),
	// TIFF, grey and palette
	("tif_grey2.tif",		1, 0),
	("tif_grey4.tif",		1, 0),
	("tif_grey8_lzw.tif",		1, 0),
	("tif_grey8_miniswhite.tif",	1, 0),
	("tif_grey16_deflate.tif",	1, 1),	// the two reductions to eight bits differ by one
	("tif_grey16_be.tif",		1, 1),
	("tif_palette4.tif",		1, 0),
	("tif_palette_lzw.tif",		1, 1),	// Pillow writes v * 256 and ImageMagick truncates v / 257
	// TIFF, colour
	("tif_rgb_none.tif",		1, 0),
	("tif_rgb_packbits.tif",	1, 0),
	("tif_rgb_lzw_pred.tif",	1, 0),
	("tif_rgb_deflate.tif",		1, 0),
	("tif_rgb_jpeg.tif",		1, 8),	// two JPEG decoders' transforms and upsampling
	("tif_rgb_tiled.tif",		1, 0),
	("tif_rgb_planar.tif",		1, 0),
	("tif_rgba_lzw.tif",		1, 0),
	("tif_cmyk.tif",		1, 1),	// the naive CMYK to RGB, rounded differently
	("tif_multipage.tif",		3, 0),
];

fn dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("raster")
}

/// The largest channel difference between two pixmaps of one size, and where it is.
fn worst(got: &Pixmap, want: &Pixmap) -> (u8, usize, usize) {
	let w = got.width();
	let mut out = (0u8, 0usize, 0usize);
	for (i, (a, b)) in got.data().chunks_exact(4).zip(want.data().chunks_exact(4)).enumerate() {
		if a[3] == 0 && b[3] == 0 {
			continue;
		}
		for c in 0..4 {
			let d = a[c].abs_diff(b[c]);
			if d > out.0 {
				out = (d, i % w, i / w);
			}
		}
	}
	out
}

#[test]
fn test_every_fixture_matches_imagemagick_00() -> Outcome<()> {
	let mut fails = Vec::new();
	for &(name, pages, tol) in CASES {
		let path = dir().join(name);
		let bytes = res!(fs::read(&path));
		assert!(convert::decodable(Media::sniff(&bytes)), "{}: not sniffed as decodable", name);
		let got = match convert::decode_pages(&bytes) {
			Ok(g) => g,
			Err(e) => {
				fails.push(fmt!("{}: {}", name, e));
				continue;
			}
		};
		if got.len() != pages {
			fails.push(fmt!("{}: {} pages, wanted {}", name, got.len(), pages));
			continue;
		}
		let stem = &name[..name.rfind('.').unwrap_or(name.len())];
		for (i, pm) in got.iter().enumerate() {
			let refname = if pages > 1 { fmt!("{}.p{}.png", stem, i) } else { fmt!("{}.png", stem) };
			let want = res!(png::decode(&res!(fs::read(dir().join(&refname)))));
			if (pm.width(), pm.height()) != (want.width(), want.height()) {
				fails.push(fmt!("{} page {}: {}x{}, wanted {}x{}", name, i,
					pm.width(), pm.height(), want.width(), want.height()));
				continue;
			}
			let (d, x, y) = worst(pm, &want);
			if d > tol {
				fails.push(fmt!("{} page {}: channel off by {} at ({}, {}): got {:?}, wanted {:?}",
					name, i, d, x, y, pm.pixel(x, y), want.pixel(x, y)));
			}
		}
	}
	assert!(fails.is_empty(), "{} of {} fixtures differ:\n{}", fails.len(), CASES.len(),
		fails.join("\n"));
	Ok(())
}

#[test]
fn test_every_fixture_converts_to_a_common_format_00() -> Outcome<()> {
	for &(name, pages, _) in CASES {
		let bytes = res!(fs::read(dir().join(name)));
		let out = res!(convert::to_common(&bytes));
		assert_eq!(out.len(), pages, "{}", name);
		for c in &out {
			assert!(matches!(c.media, Media::Png | Media::Jpeg), "{}: {:?}", name, c.media);
			assert_eq!(Media::sniff(&c.bytes), c.media, "{}", name);
		}
	}
	Ok(())
}

/// A read-only check on a real scan, named by `FE2O3_RASTER_SCAN`, which nothing here commits.
/// It prints the size of the PNG the scan becomes, and checks that decoding the PNG gives back the
/// scan's pixels.
#[test]
fn test_a_real_scan_named_in_the_environment_00() -> Outcome<()> {
	let path = match std::env::var("FE2O3_RASTER_SCAN") {
		Ok(p) => p,
		Err(_) => return Ok(()),
	};
	let bytes = res!(fs::read(&path));
	let pm = res!(convert::decode(&bytes));
	let out = res!(convert::to_common(&bytes));
	assert_eq!(out.len(), 1);
	println!("{}: {} bytes, {}x{}, becomes a {} of {} bytes", path, bytes.len(), pm.width(),
		pm.height(), out[0].media.label(), out[0].bytes.len());
	assert_eq!(res!(png::decode(&out[0].bytes)), pm);
	Ok(())
}
