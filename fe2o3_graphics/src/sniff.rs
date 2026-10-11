//! Telling an image's format from its leading bytes.
//!
//! A file's name is a claim and its bytes are the fact: Zoapedia's prototype held three `.jpg`
//! files that were PNGs, and a decoder chosen by the extension refused them. This reads the
//! signature only, so it is cheap enough to run on every upload before anything is decoded.

use crate::heif::Heif;
use crate::jpeg;
use crate::png;

use oxedyne_fe2o3_core::prelude::*;

/// A still-image format this crate decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageKind {
	Jpeg,
	Png,
	Heic,	// HEIF holding an HEVC picture
}

impl ImageKind {

	pub fn mime(self) -> &'static str {
		match self {
			Self::Jpeg	=> "image/jpeg",
			Self::Png	=> "image/png",
			Self::Heic	=> "image/heic",
		}
	}

	/// The usual file extension, without the dot.
	pub fn ext(self) -> &'static str {
		match self {
			Self::Jpeg	=> "jpg",
			Self::Png	=> "png",
			Self::Heic	=> "heic",
		}
	}

	/// The kind a MIME type names, the inverse of [`Self::mime`].
	pub fn from_mime(s: &str) -> Option<Self> {
		match s {
			"image/jpeg"	=> Some(Self::Jpeg),
			"image/png"	=> Some(Self::Png),
			"image/heic"	=> Some(Self::Heic),
			_		=> None,
		}
	}

	/// The width and height in pixels as coded, read from the header without decoding the
	/// picture. Neither an Exif orientation nor a HEIF `irot` is applied, so the three formats
	/// answer alike.
	pub fn dimensions(self, buf: &[u8]) -> Outcome<(u32, u32)> {
		let (w, h) = match self {
			Self::Jpeg	=> res!(jpeg::dimensions(buf)),
			Self::Png	=> res!(png::dimensions(buf)),
			Self::Heic	=> {
				let (w, h) = res!(res!(Heif::head(buf)).extent());
				(w as usize, h as usize)
			}
		};
		let w = res!(u32::try_from(w));
		let h = res!(u32::try_from(h));
		Ok((w, h))
	}
}

const PNG_SIG:		[u8; 8] = *b"\x89PNG\r\n\x1a\n";
const HEIC_BRANDS:	[&[u8; 4]; 4] = [b"heic", b"heix", b"heim", b"heis"];

/// The format the leading bytes announce, or `None` for anything else. Only the signature is
/// read, so a `Some` says what to decode with, not that the decode will succeed.
pub fn sniff(buf: &[u8]) -> Option<ImageKind> {
	if buf.starts_with(&PNG_SIG) {
		return Some(ImageKind::Png);
	}
	if buf.starts_with(&[0xFF, 0xD8, 0xFF]) {
		return Some(ImageKind::Jpeg);
	}
	// An ISO-BMFF `ftyp` box: size, "ftyp", major brand, minor version, compatible brands. A `mif1`
	// file names its codec among the compatible brands, so every brand is looked at.
	if buf.len() >= 16 && &buf[4..8] == b"ftyp" {
		let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
		let end = size.min(buf.len());
		if size < 16 {
			return None;
		}
		let major = &buf[8..12];
		let compat = buf[16..end].chunks_exact(4);
		let is_heic = |b: &[u8]| HEIC_BRANDS.iter().any(|h| &h[..] == b);
		if is_heic(major) || compat.into_iter().any(is_heic) {
			return Some(ImageKind::Heic);
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn signatures_decide() {
		assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"), Some(ImageKind::Png));
		assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16]), Some(ImageKind::Jpeg));
		assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"), Some(ImageKind::Heic));
		assert_eq!(sniff(b"\0\0\0\x18ftypmif1\0\0\0\0mif1heic"), Some(ImageKind::Heic));
	}

	#[test]
	fn others_refused() {
		assert_eq!(sniff(b""), None);
		assert_eq!(sniff(b"\x89PN"), None);
		assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
		assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), None);
		assert_eq!(sniff(b"\0\0\0\x18ftypavif\0\0\0\0mif1avif"), None, "AV1 is not decoded here");
		assert_eq!(sniff(b"\0\0\0\x18ftypisom\0\0\0\0isomavc1"), None, "a video is not an image");
	}

	#[test]
	fn mime_round_trips() {
		for k in [ImageKind::Jpeg, ImageKind::Png, ImageKind::Heic] {
			assert_eq!(ImageKind::from_mime(k.mime()), Some(k));
		}
	}
}
