//! Windows bitmaps and icons.
//!
//! A BMP is a header, an optional palette and rows of pixels, and the variety is all in the header:
//! five generations of it (the 12-byte OS/2 core, the 40-byte Windows 3 info, and the 52, 56, 108
//! and 124-byte versions that added channel masks, alpha and colour spaces), depths from one to
//! thirty-two bits, rows stored bottom-up unless the height is negative, and three compressions --
//! run-length coding of 8-bit and 4-bit indices, and channel masks, which are not compression at
//! all. A BMP may also carry a whole PNG or JPEG as its pixels, which is handed to that decoder.
//!
//! An ICO is a directory of the same pixel blocks without the file header, each followed by a
//! one-bit transparency mask, or of whole PNGs. The largest picture in it is the one decoded.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	jpeg,
	pixmap::Pixmap,
	png,
};

use oxedyne_fe2o3_core::prelude::*;

// Compression methods.
const BI_RGB:		u32 = 0;
const BI_RLE8:		u32 = 1;
const BI_RLE4:		u32 = 2;
const BI_BITFIELDS:	u32 = 3;
const BI_JPEG:		u32 = 4;
const BI_PNG:		u32 = 5;
const BI_ALPHABITFIELDS:	u32 = 6;

fn u16le(b: &[u8], at: usize) -> Outcome<u16> {
	match b.get(at..at + 2) {
		Some(s) => Ok(u16::from_le_bytes([s[0], s[1]])),
		None => Err(err!(
			"A bitmap needs two bytes at offset {}, but holds {}.", at, b.len();
		Invalid, Input, Decode)),
	}
}

fn u32le(b: &[u8], at: usize) -> Outcome<u32> {
	match b.get(at..at + 4) {
		Some(s) => Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]])),
		None => Err(err!(
			"A bitmap needs four bytes at offset {}, but holds {}.", at, b.len();
		Invalid, Input, Decode)),
	}
}

/// One channel's mask, as the shift and width that read it out of a pixel.
#[derive(Clone, Copy, Debug)]
struct Mask {
	shift:	u32,
	bits:	u32,
}

impl Mask {

	fn new(m: u32) -> Self {
		if m == 0 {
			return Self { shift: 0, bits: 0 };
		}
		let shift = m.trailing_zeros();
		let bits = (m >> shift).trailing_ones();
		Self { shift, bits }
	}

	/// The channel scaled to eight bits, or `none` where the mask is empty.
	fn read(&self, px: u32, none: u8) -> u8 {
		if self.bits == 0 {
			return none;
		}
		let max = if self.bits >= 32 { u32::MAX } else { (1u32 << self.bits) - 1 };
		let v = (px >> self.shift) & max;
		if self.bits == 8 {
			return v as u8;
		}
		((v as u64 * 255 + max as u64 / 2) / max as u64) as u8
	}
}

/// What the header says, gathered from whichever generation of it the file carries.
struct Header {
	w:		usize,
	h:		usize,
	top_down:	bool,
	bpp:		u16,
	comp:		u32,
	masks:		Option<[u32; 4]>,	// red, green, blue, alpha
	palette:	Vec<[u8; 4]>,		// RGBA, alpha always opaque
	end:		usize,			// the first byte after the header, masks and palette
}

/// Reads a device-independent bitmap header starting at `dib[0]`.
fn header(dib: &[u8]) -> Outcome<Header> {
	let size = res!(u32le(dib, 0)) as usize;
	let (w, h, bpp, comp, mut at, entry, used) = match size {
		12 => {
			let w = res!(u16le(dib, 4)) as i64;
			let h = res!(u16le(dib, 6)) as i64;
			let bpp = res!(u16le(dib, 10));
			(w, h, bpp, BI_RGB, 12usize, 3usize, 0u32)
		}
		16 | 40 | 52 | 56 | 64 | 108 | 124 => {
			let w = res!(u32le(dib, 4)) as i32 as i64;
			let h = res!(u32le(dib, 8)) as i32 as i64;
			let bpp = res!(u16le(dib, 14));
			let comp = if size >= 20 { res!(u32le(dib, 16)) } else { BI_RGB };
			let used = if size >= 36 { res!(u32le(dib, 32)) } else { 0 };
			(w, h, bpp, comp, size, 4usize, used)
		}
		_ => return Err(err!(
			"A bitmap header of {} bytes is not one of the generations this reads \
			(12, 16, 40, 52, 56, 64, 108 or 124).", size;
		Invalid, Input, Decode, Unimplemented)),
	};
	// An OS/2 2.x header numbers its compressions differently past RLE4.
	if (size == 16 || size == 64) && comp > BI_RLE4 {
		return Err(err!(
			"An OS/2 bitmap uses compression {}, which this does not read.", comp;
		Invalid, Input, Decode, Unimplemented));
	}
	if w <= 0 || h == 0 {
		return Err(err!(
			"A bitmap declares a size of {} by {}.", w, h;
		Invalid, Input, Decode));
	}
	let top_down = h < 0;
	let masks = match comp {
		BI_BITFIELDS | BI_ALPHABITFIELDS => {
			let n = if comp == BI_ALPHABITFIELDS { 4 } else { 3 };
			// A 40-byte header puts the masks after itself; later ones hold them inside.
			let base = if size == 40 { at } else { 40 };
			let mut m = [0u32; 4];
			for i in 0..n {
				m[i] = res!(u32le(dib, base + 4 * i));
			}
			if size >= 56 && n == 3 {
				m[3] = res!(u32le(dib, 52));
			}
			if size == 40 {
				at += 4 * n;
			}
			Some(m)
		}
		_ if size >= 56 && (bpp == 16 || bpp == 32) => {
			// A V4 or V5 header without BI_BITFIELDS still names an alpha mask some writers rely on.
			let a = res!(u32le(dib, 52));
			if a != 0 && comp == BI_RGB {
				let def = default_masks(bpp);
				Some([def[0], def[1], def[2], a])
			} else {
				None
			}
		}
		_ => None,
	};
	let mut palette = Vec::new();
	if bpp <= 8 {
		let full = 1usize << bpp;
		let n = if used == 0 || used as usize > full { full } else { used as usize };
		for i in 0..n {
			let p = at + i * entry;
			match dib.get(p..p + 3) {
				Some(c) => palette.push([c[2], c[1], c[0], 255]),
				None => break, // a short palette leaves the rest black
			}
		}
		at += n * entry;
	}
	Ok(Header {
		w: w as usize,
		h: h.unsigned_abs() as usize,
		top_down,
		bpp,
		comp,
		masks,
		palette,
		end: at,
	})
}

fn default_masks(bpp: u16) -> [u32; 4] {
	match bpp {
		16 => [0x7c00, 0x03e0, 0x001f, 0],
		_ => [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0],
	}
}

fn colour(palette: &[[u8; 4]], i: usize) -> [u8; 4] {
	match palette.get(i) {
		Some(c) => *c,
		None => [0, 0, 0, 255],
	}
}

/// Decodes uncompressed or channel-masked rows into the pixmap.
fn rows(hdr: &Header, px: &[u8], pm: &mut Pixmap) -> Outcome<()> {
	let bpp = hdr.bpp as usize;
	if !matches!(bpp, 1 | 2 | 4 | 8 | 16 | 24 | 32) {
		return Err(err!(
			"A bitmap of {} bits a pixel is not one of 1, 2, 4, 8, 16, 24 or 32.", bpp;
		Invalid, Input, Decode, Unimplemented));
	}
	let stride = ((hdr.w * bpp + 31) / 32) * 4;
	let need = match stride.checked_mul(hdr.h) {
		Some(n) => n,
		None => return Err(err!(
			"A bitmap of {} rows of {} bytes overflows.", hdr.h, stride;
		Invalid, Input, Decode, Overflow)),
	};
	// The last row may lack its padding, which some writers leave off.
	let short = stride - (hdr.w * bpp + 7) / 8;
	if px.len() + short < need {
		return Err(err!(
			"A bitmap of {} by {} at {} bits needs {} bytes of pixels, but {} are present.",
			hdr.w, hdr.h, bpp, need, px.len();
		Invalid, Input, Decode));
	}
	let masks = match hdr.masks {
		Some(m) => m,
		None => default_masks(hdr.bpp),
	};
	let mk = [Mask::new(masks[0]), Mask::new(masks[1]), Mask::new(masks[2]), Mask::new(masks[3])];
	let w = hdr.w;
	let data = pm.data_mut();
	for r in 0..hdr.h {
		let y = if hdr.top_down { r } else { hdr.h - 1 - r };
		let row = &px[r * stride..(r * stride + stride).min(px.len())];
		let out = &mut data[y * w * 4..(y + 1) * w * 4];
		for x in 0..w {
			let c = match bpp {
				1 | 2 | 4 | 8 => {
					let bit = x * bpp;
					let byte = row[bit / 8];
					let shift = 8 - bpp - (bit % 8);
					let i = (byte >> shift) as usize & ((1 << bpp) - 1);
					colour(&hdr.palette, i)
				}
				24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
				_ => {
					let v = if bpp == 16 {
						u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32
					} else {
						u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]])
					};
					[mk[0].read(v, 0), mk[1].read(v, 0), mk[2].read(v, 0), mk[3].read(v, 255)]
				}
			};
			out[x * 4..x * 4 + 4].copy_from_slice(&c);
		}
	}
	Ok(())
}

/// Decodes 8-bit or 4-bit run-length coded indices into the pixmap.
///
/// Pixels a delta skips over are left as the palette's first colour, which is what a reader that
/// paints the bitmap onto a cleared surface shows.
fn rle(hdr: &Header, px: &[u8], pm: &mut Pixmap) -> Outcome<()> {
	let four = hdr.comp == BI_RLE4;
	let (w, h) = (hdr.w, hdr.h);
	let back = colour(&hdr.palette, 0);
	{
		let data = pm.data_mut();
		for p in data.chunks_exact_mut(4) {
			p.copy_from_slice(&back);
		}
	}
	let data = pm.data_mut();
	let mut put = |x: usize, y: usize, i: usize| {
		if x < w && y < h {
			let yy = h - 1 - y;
			let c = colour(&hdr.palette, i);
			data[(yy * w + x) * 4..(yy * w + x) * 4 + 4].copy_from_slice(&c);
		}
	};
	let (mut x, mut y) = (0usize, 0usize);
	let mut i = 0;
	while i + 1 < px.len() && y < h {
		let (n, v) = (px[i] as usize, px[i + 1]);
		i += 2;
		if n > 0 {
			for k in 0..n {
				let idx = if four {
					if k % 2 == 0 { v >> 4 } else { v & 0x0f }
				} else {
					v
				};
				put(x, y, idx as usize);
				x += 1;
			}
			continue;
		}
		match v {
			0 => { x = 0; y += 1; }		// end of line
			1 => break,			// end of bitmap
			2 => {				// delta
				if i + 1 >= px.len() {
					break;
				}
				x += px[i] as usize;
				y += px[i + 1] as usize;
				i += 2;
			}
			m => {				// absolute run of m indices
				let m = m as usize;
				let bytes = if four { (m + 1) / 2 } else { m };
				if i + bytes > px.len() {
					return Err(err!(
						"A run-length bitmap's literal run of {} at offset {} passes its end.", m, i;
					Invalid, Input, Decode));
				}
				for k in 0..m {
					let idx = if four {
						let b = px[i + k / 2];
						if k % 2 == 0 { b >> 4 } else { b & 0x0f }
					} else {
						px[i + k]
					};
					put(x, y, idx as usize);
					x += 1;
				}
				// Each literal run is padded to a whole number of 16-bit words.
				i += (bytes + 1) & !1;
			}
		}
	}
	Ok(())
}

/// Decodes a device-independent bitmap whose pixels start at `pixels`, an offset into `dib`.
fn dib(dib: &[u8], pixels: Option<usize>, icon: bool) -> Outcome<Pixmap> {
	let mut hdr = res!(header(dib));
	if icon {
		// An icon's height counts the transparency mask below the colour rows as well.
		hdr.h /= 2;
		if hdr.h == 0 {
			return Err(err!("An icon entry is less than one row tall."; Invalid, Input, Decode));
		}
	}
	let start = pixels.unwrap_or(hdr.end);
	let px = match dib.get(start..) {
		Some(p) => p,
		None => return Err(err!(
			"A bitmap's pixels are said to start at offset {}, past its end at {}.",
			start, dib.len();
		Invalid, Input, Decode)),
	};
	match hdr.comp {
		BI_JPEG => return jpeg::decode(px),
		BI_PNG => return png::decode(px),
		_ => (),
	}
	let mut pm = res!(Pixmap::new(hdr.w, hdr.h));
	match hdr.comp {
		BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS => res!(rows(&hdr, px, &mut pm)),
		BI_RLE8 | BI_RLE4 => {
			if hdr.top_down {
				return Err(err!(
					"A run-length bitmap cannot be stored top-down, but this one says it is.";
				Invalid, Input, Decode));
			}
			res!(rle(&hdr, px, &mut pm));
		}
		c => return Err(err!(
			"A bitmap uses compression {}, which this does not read.", c;
		Invalid, Input, Decode, Unimplemented)),
	}
	if icon {
		let stride = ((hdr.w * hdr.bpp as usize + 31) / 32) * 4;
		let and_at = stride * hdr.h;
		let and_stride = ((hdr.w + 31) / 32) * 4;
		let w = hdr.w;
		let h = hdr.h;
		// A 32-bit icon carries its own alpha; the mask is used only where that alpha is empty.
		let use_mask = hdr.bpp < 32 || pm.data().chunks_exact(4).all(|p| p[3] == 0);
		if hdr.bpp == 32 && use_mask {
			for p in pm.data_mut().chunks_exact_mut(4) {
				p[3] = 255;
			}
		}
		if use_mask {
			let data = pm.data_mut();
			for r in 0..h {
				let y = h - 1 - r;
				for x in 0..w {
					let at = and_at + r * and_stride + x / 8;
					if let Some(b) = px.get(at) {
						if (b >> (7 - x % 8)) & 1 == 1 {
							data[(y * w + x) * 4 + 3] = 0;
						}
					}
				}
			}
		}
	}
	Ok(pm)
}

/// The width and height a bitmap declares, read from its header alone.
pub fn dimensions(buf: &[u8]) -> Outcome<(usize, usize)> {
	if buf.len() < 2 || &buf[..2] != b"BM" {
		return Err(err!("The bytes do not begin with a bitmap's 'BM'."; Invalid, Input, Decode));
	}
	let hdr = res!(header(&buf[14.min(buf.len())..]));
	Ok((hdr.w, hdr.h))
}

/// Decodes a Windows or OS/2 bitmap file.
pub fn decode(buf: &[u8]) -> Outcome<Pixmap> {
	if buf.len() < 14 || &buf[..2] != b"BM" {
		return Err(err!("The bytes do not begin with a bitmap's 'BM'."; Invalid, Input, Decode));
	}
	let off = res!(u32le(buf, 10)) as usize;
	// The pixel offset counts from the start of the file; the header from byte 14.
	let pixels = if off >= 14 { Some(off - 14) } else { None };
	dib(&buf[14..], pixels, false)
}

/// Decodes the largest picture in a Windows icon or cursor file.
pub fn decode_ico(buf: &[u8]) -> Outcome<Pixmap> {
	let kind = res!(u16le(buf, 2));
	if res!(u16le(buf, 0)) != 0 || (kind != 1 && kind != 2) {
		return Err(err!("The bytes are not an icon or cursor directory."; Invalid, Input, Decode));
	}
	let n = res!(u16le(buf, 4)) as usize;
	let mut best: Option<(usize, usize, usize, usize)> = None; // area, depth, offset, size
	for i in 0..n {
		let e = 6 + i * 16;
		let w = match buf.get(e) { Some(0) => 256, Some(v) => *v as usize, None => break };
		let h = match buf.get(e + 1) { Some(0) => 256, Some(v) => *v as usize, None => break };
		let bpp = res!(u16le(buf, e + 6)) as usize;
		let size = res!(u32le(buf, e + 8)) as usize;
		let off = res!(u32le(buf, e + 12)) as usize;
		let cand = (w * h, bpp, off, size);
		best = match best {
			Some(b) if (b.0, b.1) >= (cand.0, cand.1) => Some(b),
			_ => Some(cand),
		};
	}
	let (_, _, off, size) = match best {
		Some(b) => b,
		None => return Err(err!("An icon file lists no pictures."; Invalid, Input, Decode)),
	};
	let end = off.saturating_add(size).min(buf.len());
	let img = match buf.get(off..end) {
		Some(s) => s,
		None => return Err(err!(
			"An icon's picture is said to start at offset {}, past its end at {}.", off, buf.len();
		Invalid, Input, Decode)),
	};
	if img.starts_with(&[0x89, b'P', b'N', b'G']) {
		return png::decode(img);
	}
	dib(img, None, true)
}

/// Encodes a pixmap as a bitmap: 24 bits a pixel where every pixel is opaque, otherwise 32 bits
/// with an alpha mask in a version 4 header, which is the form every current reader takes alpha
/// from.
pub fn encode(pm: &Pixmap) -> Outcome<Vec<u8>> {
	let (w, h) = (pm.width(), pm.height());
	let opaque = pm.data().chunks_exact(4).all(|p| p[3] == 255);
	let (bpp, hsize) = if opaque { (24usize, 40usize) } else { (32, 108) };
	let stride = ((w * bpp + 31) / 32) * 4;
	let off = 14 + hsize;
	let total = off + stride * h;
	if total > u32::MAX as usize {
		return Err(err!(
			"A bitmap of {} bytes is too large for the format's 32-bit size.", total;
		Invalid, Input, Excessive));
	}
	let mut out = Vec::with_capacity(total);
	out.extend_from_slice(b"BM");
	out.extend_from_slice(&(total as u32).to_le_bytes());
	out.extend_from_slice(&[0, 0, 0, 0]);
	out.extend_from_slice(&(off as u32).to_le_bytes());
	out.extend_from_slice(&(hsize as u32).to_le_bytes());
	out.extend_from_slice(&(w as i32).to_le_bytes());
	out.extend_from_slice(&(h as i32).to_le_bytes());
	out.extend_from_slice(&1u16.to_le_bytes());
	out.extend_from_slice(&(bpp as u16).to_le_bytes());
	let comp = if opaque { BI_RGB } else { BI_BITFIELDS };
	out.extend_from_slice(&comp.to_le_bytes());
	out.extend_from_slice(&((stride * h) as u32).to_le_bytes());
	out.extend_from_slice(&2835u32.to_le_bytes()); // 72 dots an inch, in dots a metre
	out.extend_from_slice(&2835u32.to_le_bytes());
	out.extend_from_slice(&[0; 8]);
	if !opaque {
		for m in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
			out.extend_from_slice(&m.to_le_bytes());
		}
		out.extend_from_slice(b"BGRs"); // LCS_sRGB, stored little-endian
		out.extend_from_slice(&[0; 48]);
	}
	let data = pm.data();
	for y in (0..h).rev() {
		let start = out.len();
		for x in 0..w {
			let p = &data[(y * w + x) * 4..(y * w + x) * 4 + 4];
			out.extend_from_slice(&[p[2], p[1], p[0]]);
			if !opaque {
				out.push(p[3]);
			}
		}
		out.resize(start + stride, 0);
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::colour::Rgba;

	fn sample(w: usize, h: usize, alpha: bool) -> Outcome<Pixmap> {
		let mut pm = res!(Pixmap::new(w, h));
		for y in 0..h {
			for x in 0..w {
				let a = if alpha { ((x * 37 + y * 11) % 256) as u8 } else { 255 };
				pm.set_pixel(x, y, Rgba::new((x * 9) as u8, (y * 13) as u8, ((x + y) * 5) as u8, a));
			}
		}
		Ok(pm)
	}

	#[test]
	fn test_encode_then_decode_returns_the_pixels_00() -> Outcome<()> {
		for &(w, h, a) in &[(1, 1, false), (17, 13, false), (5, 3, true), (31, 2, true)] {
			let pm = res!(sample(w, h, a));
			let back = res!(decode(&res!(encode(&pm))));
			assert_eq!(back, pm, "{}x{} alpha {}", w, h, a);
		}
		Ok(())
	}

	#[test]
	fn test_rle8_with_a_delta_and_an_absolute_run_00() -> Outcome<()> {
		// 4 by 2, palette of three, bottom row first.
		let mut f = Vec::new();
		let pal: [[u8; 4]; 3] = [[0, 0, 0, 0], [0, 0, 255, 0], [0, 255, 0, 0]];
		let px: Vec<u8> = vec![
			2, 1,			// two of index 1
			0, 2, 1, 0,		// delta: right one, down none
			1, 2,			// one of index 2
			0, 0,			// end of line
			0, 3, 2, 1, 2, 0,	// absolute run of three, padded to four bytes
			0, 1,			// end of bitmap
		];
		f.extend_from_slice(b"BM");
		let off = 14 + 40 + 12;
		f.extend_from_slice(&((off + px.len()) as u32).to_le_bytes());
		f.extend_from_slice(&[0; 4]);
		f.extend_from_slice(&(off as u32).to_le_bytes());
		f.extend_from_slice(&40u32.to_le_bytes());
		f.extend_from_slice(&4i32.to_le_bytes());
		f.extend_from_slice(&2i32.to_le_bytes());
		f.extend_from_slice(&1u16.to_le_bytes());
		f.extend_from_slice(&8u16.to_le_bytes());
		f.extend_from_slice(&BI_RLE8.to_le_bytes());
		f.extend_from_slice(&[0; 12]);
		f.extend_from_slice(&3u32.to_le_bytes());
		f.extend_from_slice(&[0; 4]);
		for p in &pal {
			f.extend_from_slice(p);
		}
		f.extend_from_slice(&px);
		let pm = res!(decode(&f));
		let red = Rgba::new(255, 0, 0, 255);
		let green = Rgba::new(0, 255, 0, 255);
		let black = Rgba::new(0, 0, 0, 255);
		// Bottom row: red, red, delta (1, 0) over x = 2, then green at x = 3.
		assert_eq!(pm.pixel(0, 1), Some(red));
		assert_eq!(pm.pixel(1, 1), Some(red));
		assert_eq!(pm.pixel(2, 1), Some(black));
		assert_eq!(pm.pixel(3, 1), Some(green));
		// Top row: green, red, green, then nothing written.
		assert_eq!(pm.pixel(0, 0), Some(green));
		assert_eq!(pm.pixel(1, 0), Some(red));
		assert_eq!(pm.pixel(2, 0), Some(green));
		assert_eq!(pm.pixel(3, 0), Some(black));
		Ok(())
	}
}
