//! Any picture this crate can read, turned into one every consumer takes.
//!
//! Services and programs that take pictures agree on PNG and JPEG and on little else: a vision
//! model, an OCR service, a typesetter and an `<img>` will each refuse a BMP or a TIFF, while a
//! scanner writes nothing else. This is the step that sits between them, so that a tool handed a
//! scan converts it itself rather than asking a person, or a model, to do it.
//!
//! PNG is the default, because it is lossless and a scan of print is what this mostly sees: flat
//! paper and sharp edges, which PNG holds in less than JPEG does at a quality that keeps the edges.
//! Only where the PNG would be large -- a photograph, whose noise PNG cannot compress -- is a JPEG
//! made as well, and the smaller of the two kept.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	bmp,
	gif,
	heif,
	jpeg,
	pixmap::Pixmap,
	png,
	tiff,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::media::Media;

// A PNG up to this size is kept even where a JPEG would be smaller.
pub const PNG_CEILING: usize = 2 * 1024 * 1024;

// The JPEG quality a large picture is written at: high enough to keep print legible.
pub const JPEG_QUALITY: u8 = 90;

/// One picture in a form every consumer takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Common {
	pub media:	Media,		// Png or Jpeg
	pub bytes:	Vec<u8>,
	pub w:		usize,
	pub h:		usize,
}

impl Common {

	/// The picture as a `data:` URL.
	pub fn data_url(&self) -> String {
		fmt!("data:{};base64,{}", self.media.mime(), oxedyne_fe2o3_text::base64::encode(&self.bytes))
	}
}

/// Can this crate decode a picture in this format?
pub fn decodable(m: Media) -> bool {
	matches!(m, Media::Png | Media::Jpeg | Media::Gif | Media::Bmp | Media::Ico | Media::Tiff
		| Media::Heic)
}

/// Decodes a picture in any format [`decodable`] names: a TIFF's first page, a GIF's first frame,
/// an icon's largest picture.
pub fn decode(bytes: &[u8]) -> Outcome<Pixmap> {
	match Media::sniff(bytes) {
		Media::Png	=> png::decode(bytes),
		Media::Jpeg	=> jpeg::decode(bytes),
		Media::Gif	=> gif::decode(bytes),
		Media::Bmp	=> bmp::decode(bytes),
		Media::Ico	=> bmp::decode_ico(bytes),
		Media::Tiff	=> tiff::decode(bytes),
		Media::Heic	=> heif::decode(bytes),
		m => Err(err!(
			"A picture in {} format is not one this can decode.", m.label();
		Invalid, Input, Decode, Unimplemented)),
	}
}

/// Decodes every page of a picture: each of a TIFF's, and the one of anything else.
pub fn decode_pages(bytes: &[u8]) -> Outcome<Vec<Pixmap>> {
	match Media::sniff(bytes) {
		Media::Tiff => tiff::decode_pages(bytes),
		_ => Ok(vec![res!(decode(bytes))]),
	}
}

/// Encodes a pixmap as PNG, or as JPEG where that is much the smaller of a large picture.
pub fn encode(pm: &Pixmap) -> Outcome<Common> {
	let p = res!(png::encode(pm));
	let (w, h) = (pm.width(), pm.height());
	let opaque = pm.data().chunks_exact(4).all(|c| c[3] == 255);
	if p.len() > PNG_CEILING && opaque {
		let opts = jpeg::Options { quality: JPEG_QUALITY, ..Default::default() };
		let j = res!(jpeg::encode_with(pm, &opts));
		if j.len() < p.len() {
			return Ok(Common { media: Media::Jpeg, bytes: j, w, h });
		}
	}
	Ok(Common { media: Media::Png, bytes: p, w, h })
}

/// The picture as PNG or JPEG, one for each page. A PNG or a JPEG comes back as it was, unread
/// beyond its size.
pub fn to_common(bytes: &[u8]) -> Outcome<Vec<Common>> {
	let m = Media::sniff(bytes);
	match m {
		Media::Png => {
			let (w, h) = res!(png::dimensions(bytes));
			return Ok(vec![Common { media: m, bytes: bytes.to_vec(), w, h }]);
		}
		Media::Jpeg => {
			let (w, h) = res!(jpeg::dimensions(bytes));
			return Ok(vec![Common { media: m, bytes: bytes.to_vec(), w, h }]);
		}
		_ => (),
	}
	let pages = res!(decode_pages(bytes).map_err(|e| err!(e,
		"Converting a {} picture to PNG or JPEG.", m.label(); Decode)));
	let mut out = Vec::with_capacity(pages.len());
	for p in &pages {
		out.push(res!(encode(p)));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::colour::Rgba;

	#[test]
	fn test_a_bmp_comes_back_as_a_png_of_the_same_pixels_00() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(40, 30, Rgba::new(255, 255, 255, 255)));
		for x in 5..35 {
			pm.set_pixel(x, 15, Rgba::new(0, 0, 0, 255));
		}
		let b = res!(bmp::encode(&pm));
		assert_eq!(Media::sniff(&b), Media::Bmp);
		let out = res!(to_common(&b));
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].media, Media::Png);
		assert_eq!((out[0].w, out[0].h), (40, 30));
		assert_eq!(res!(png::decode(&out[0].bytes)), pm);
		assert!(out[0].data_url().starts_with("data:image/png;base64,"));
		Ok(())
	}

	#[test]
	fn test_a_png_passes_through_untouched_00() -> Outcome<()> {
		let pm = res!(Pixmap::filled(3, 2, Rgba::new(1, 2, 3, 255)));
		let p = res!(png::encode(&pm));
		let out = res!(to_common(&p));
		assert_eq!(out[0].bytes, p);
		Ok(())
	}
}
