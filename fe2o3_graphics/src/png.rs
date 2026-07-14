//! A PNG codec.
//!
//! PNG is a short list of length-prefixed chunks wrapped around a DEFLATE stream, so the only piece
//! worth borrowing is the DEFLATE, which `flate2` supplies. The chunk framing, the CRC-32 each
//! chunk carries, and the scanline filters are small enough to own.
//!
//! Owning the decoder is also a security position. An image decoder is the classic place a viewer
//! is attacked from, and this one is written in a crate that forbids `unsafe`, checks every length
//! it is told, and refuses a decompressed stream larger than the header says it should be.
//!
//! # What is supported
//!
//! Eight bits per channel, all five colour types (greyscale, truecolour, palette, greyscale with
//! alpha, truecolour with alpha), and no interlacing. Sixteen-bit channels and Adam7 interlacing
//! are refused by name rather than misread.

use crate::{
	colour::Rgba,
	pixmap::{
		Pixmap,
		MAX_PIXELS,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::io::{
	Read,
	Write,
};

use flate2::{
	read::ZlibDecoder,
	write::ZlibEncoder,
	Compression,
};

/// The eight bytes that begin every PNG.
const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// How a PNG says what each pixel carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColourType {
	/// One channel: luminance.
	Grey,
	/// Three channels: red, green, blue.
	Rgb,
	/// One channel: an index into the palette.
	Palette,
	/// Two channels: luminance and alpha.
	GreyAlpha,
	/// Four channels: red, green, blue, alpha.
	Rgba,
}

impl ColourType {

	/// The colour type for a PNG's header byte.
	fn from_code(code: u8) -> Outcome<Self> {
		match code {
			0	=> Ok(Self::Grey),
			2	=> Ok(Self::Rgb),
			3	=> Ok(Self::Palette),
			4	=> Ok(Self::GreyAlpha),
			6	=> Ok(Self::Rgba),
			_	=> Err(err!(
				"The PNG header declares colour type {}, which is not one of 0, 2, 3, 4 or 6.",
				code;
			Invalid, Input, Decode)),
		}
	}

	/// The number of bytes each pixel occupies in the filtered scanlines, at eight bits a channel.
	fn bytes_per_pixel(&self) -> usize {
		match self {
			Self::Grey		=> 1,
			Self::Rgb		=> 3,
			Self::Palette		=> 1,
			Self::GreyAlpha		=> 2,
			Self::Rgba		=> 4,
		}
	}
}

/// A PNG's image header, once believed.
#[derive(Clone, Copy, Debug)]
struct Header {
	/// Width in pixels.
	w:	usize,
	/// Height in pixels.
	h:	usize,
	/// What each pixel carries.
	ct:	ColourType,
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CRC-32                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// The CRC-32 of some bytes, as PNG defines it: the ISO 3309 polynomial, reflected.
fn crc32(bytes: &[u8]) -> u32 {
	let mut crc = 0xFFFF_FFFFu32;
	for b in bytes {
		let mut c = (crc ^ (*b as u32)) & 0xFF;
		for _ in 0..8 {
			c = if c & 1 != 0 {
				0xEDB8_8320 ^ (c >> 1)
			} else {
				c >> 1
			};
		}
		crc = c ^ (crc >> 8);
	}
	crc ^ 0xFFFF_FFFF
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENCODING                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Encodes a pixmap as a PNG: eight-bit truecolour with alpha, no interlacing.
pub fn encode(pm: &Pixmap) -> Outcome<Vec<u8>> {
	let (w, h) = (pm.width(), pm.height());
	let mut out = Vec::with_capacity(w * h + 1024);
	out.extend_from_slice(&SIG);

	// The image header.
	let mut ihdr = Vec::with_capacity(13);
	ihdr.extend_from_slice(&(w as u32).to_be_bytes());
	ihdr.extend_from_slice(&(h as u32).to_be_bytes());
	ihdr.push(8); // Bit depth.
	ihdr.push(6); // Colour type: truecolour with alpha.
	ihdr.push(0); // Compression method: DEFLATE, the only one there is.
	ihdr.push(0); // Filter method: the only one there is.
	ihdr.push(0); // Interlace method: none.
	write_chunk(&mut out, b"IHDR", &ihdr);

	// The image data: each scanline filtered, then the lot deflated.
	let stride = w * 4;
	let mut raw = Vec::with_capacity(h * (stride + 1));
	let mut prev = vec![0u8; stride];
	for y in 0..h {
		let line = &pm.data()[y * stride..(y + 1) * stride];
		filter_scanline(line, &prev, 4, &mut raw);
		prev.copy_from_slice(line);
	}
	let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
	res!(z.write_all(&raw));
	let idat = res!(z.finish());
	write_chunk(&mut out, b"IDAT", &idat);

	write_chunk(&mut out, b"IEND", &[]);
	Ok(out)
}

/// Appends a chunk: its length, its type, its data, and the CRC over type and data.
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
	out.extend_from_slice(&(data.len() as u32).to_be_bytes());
	let start = out.len();
	out.extend_from_slice(kind);
	out.extend_from_slice(data);
	let crc = crc32(&out[start..]);
	out.extend_from_slice(&crc.to_be_bytes());
}

/// Filters one scanline, choosing whichever of the five filters leaves the smallest residue.
///
/// The heuristic is the one the PNG specification suggests: sum the absolute values of the filtered
/// bytes, taken as signed, and keep the smallest. A filter that leaves the bytes closest to zero is
/// the one DEFLATE will do most with.
fn filter_scanline(line: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
	let n = line.len();
	let mut best: Option<(u32, u8, Vec<u8>)> = None;
	for ftype in 0u8..5 {
		let mut buf = Vec::with_capacity(n);
		for i in 0..n {
			let a = if i >= bpp { line[i - bpp] } else { 0 }; // Left.
			let b = prev[i]; // Above.
			let c = if i >= bpp { prev[i - bpp] } else { 0 }; // Above left.
			let x = line[i];
			let v = match ftype {
				0 => x,
				1 => x.wrapping_sub(a),
				2 => x.wrapping_sub(b),
				3 => x.wrapping_sub(((a as u16 + b as u16) / 2) as u8),
				_ => x.wrapping_sub(paeth(a, b, c)),
			};
			buf.push(v);
		}
		let score: u32 = buf.iter().map(|v| (*v as i8).unsigned_abs() as u32).sum();
		let better = match &best {
			None => true,
			Some((s, _, _)) => score < *s,
		};
		if better {
			best = Some((score, ftype, buf));
		}
	}
	if let Some((_, ftype, buf)) = best {
		out.push(ftype);
		out.extend_from_slice(&buf);
	}
}

/// The Paeth predictor: whichever of the left, above and above-left neighbours is closest to their
/// linear estimate.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
	let p = (a as i16) + (b as i16) - (c as i16);
	let pa = (p - a as i16).abs();
	let pb = (p - b as i16).abs();
	let pc = (p - c as i16).abs();
	if pa <= pb && pa <= pc {
		a
	} else if pb <= pc {
		b
	} else {
		c
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DECODING                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Decodes a PNG into a pixmap.
///
/// Every length is checked against the bytes actually present, every chunk's CRC is verified, and
/// the decompressed stream is refused the moment it exceeds the size the header implies, so a small
/// file cannot expand into a large allocation.
pub fn decode(buf: &[u8]) -> Outcome<Pixmap> {
	if buf.len() < SIG.len() || buf[..SIG.len()] != SIG {
		return Err(err!(
			"The bytes do not begin with the PNG signature."; Invalid, Input, Decode));
	}
	let mut pos = SIG.len();
	let mut hdr: Option<Header> = None;
	let mut palette: Vec<Rgba> = Vec::new();
	let mut idat: Vec<u8> = Vec::new();
	let mut ended = false;

	while pos < buf.len() {
		// Length, type, data, CRC.
		if pos + 8 > buf.len() {
			return Err(err!(
				"A PNG chunk header needs 8 bytes at offset {}, but only {} remain.",
				pos, buf.len() - pos;
			Invalid, Input, Decode));
		}
		let len = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
		let kind = [buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]];
		let data_start = pos + 8;
		let data_end = match data_start.checked_add(len) {
			Some(e) => e,
			None => return Err(err!(
				"A PNG chunk at offset {} declares a length of {}, which overflows.", pos, len;
			Invalid, Input, Decode, Overflow)),
		};
		if data_end + 4 > buf.len() {
			return Err(err!(
				"The PNG chunk '{}' at offset {} declares {} bytes, but only {} remain.",
				String::from_utf8_lossy(&kind), pos, len, buf.len().saturating_sub(data_start);
			Invalid, Input, Decode));
		}
		let data = &buf[data_start..data_end];
		let want = u32::from_be_bytes([
			buf[data_end],
			buf[data_end + 1],
			buf[data_end + 2],
			buf[data_end + 3],
		]);
		let got = crc32(&buf[pos + 4..data_end]);
		if got != want {
			return Err(err!(
				"The PNG chunk '{}' at offset {} carries the CRC {:#010X}, but its bytes hash to \
				{:#010X}.", String::from_utf8_lossy(&kind), pos, want, got;
			Invalid, Input, Decode, Checksum));
		}
		pos = data_end + 4;

		match &kind {
			b"IHDR" => hdr = Some(res!(decode_header(data))),
			b"PLTE" => palette = res!(decode_palette(data)),
			b"IDAT" => idat.extend_from_slice(data),
			b"IEND" => {
				ended = true;
				break;
			},
			_ => (), // Ancillary chunks are not our business.
		}
	}

	if !ended {
		return Err(err!("The PNG has no IEND chunk."; Invalid, Input, Decode, Missing));
	}
	let hdr = match hdr {
		Some(h) => h,
		None => return Err(err!("The PNG has no IHDR chunk."; Invalid, Input, Decode, Missing)),
	};
	if idat.is_empty() {
		return Err(err!("The PNG has no image data."; Invalid, Input, Decode, Missing));
	}
	if hdr.ct == ColourType::Palette && palette.is_empty() {
		return Err(err!(
			"The PNG declares a palette colour type but carries no PLTE chunk.";
		Invalid, Input, Decode, Missing));
	}

	// Inflate, refusing anything larger than the header says it should be.
	let bpp = hdr.ct.bytes_per_pixel();
	let stride = hdr.w * bpp;
	let expect = hdr.h * (stride + 1); // One filter byte per scanline.
	let mut raw = Vec::with_capacity(expect);
	let mut z = ZlibDecoder::new(&idat[..]).take((expect as u64) + 1);
	res!(z.read_to_end(&mut raw));
	if raw.len() != expect {
		return Err(err!(
			"The PNG's image data decompresses to {} bytes, but its header of {} by {} pixels \
			implies {}.", raw.len(), hdr.w, hdr.h, expect;
		Invalid, Input, Decode, Mismatch));
	}

	// Unfilter, then expand into RGBA.
	let mut pm = res!(Pixmap::new(hdr.w, hdr.h));
	let mut prev = vec![0u8; stride];
	let mut line = vec![0u8; stride];
	for y in 0..hdr.h {
		let at = y * (stride + 1);
		let ftype = raw[at];
		line.copy_from_slice(&raw[at + 1..at + 1 + stride]);
		res!(unfilter_scanline(ftype, &mut line, &prev, bpp, y));
		for x in 0..hdr.w {
			let c = res!(pixel_of(hdr.ct, &line, x, bpp, &palette));
			pm.set_pixel(x, y, c);
		}
		prev.copy_from_slice(&line);
	}
	Ok(pm)
}

/// Reads the image header, and refuses what this codec does not implement, by name.
fn decode_header(data: &[u8]) -> Outcome<Header> {
	if data.len() != 13 {
		return Err(err!(
			"A PNG image header is 13 bytes, but this one is {}.", data.len();
		Invalid, Input, Decode));
	}
	let w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
	let h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
	let depth = data[8];
	let ct = res!(ColourType::from_code(data[9]));
	let interlace = data[12];

	if w == 0 || h == 0 {
		return Err(err!(
			"The PNG header declares a size of {} by {} pixels.", w, h; Invalid, Input, Decode));
	}
	let n = match w.checked_mul(h) {
		Some(n) => n,
		None => return Err(err!(
			"The PNG header declares {} by {} pixels, which overflows.", w, h;
		Invalid, Input, Decode, Overflow)),
	};
	if n > MAX_PIXELS {
		return Err(err!(
			"The PNG header declares {} by {} pixels, over the ceiling of {}.", w, h, MAX_PIXELS;
		Invalid, Input, Decode, Excessive));
	}
	if depth != 8 {
		return Err(err!(
			"The PNG declares {} bits per channel. This codec implements 8.", depth;
		Invalid, Input, Decode, NoImpl));
	}
	if interlace != 0 {
		return Err(err!(
			"The PNG is Adam7 interlaced. This codec implements the non-interlaced form only.";
		Invalid, Input, Decode, NoImpl));
	}
	Ok(Header { w, h, ct })
}

/// Reads a palette: three bytes a colour, opaque.
fn decode_palette(data: &[u8]) -> Outcome<Vec<Rgba>> {
	if data.len() % 3 != 0 {
		return Err(err!(
			"A PNG palette holds 3 bytes per entry, but this one is {} bytes.", data.len();
		Invalid, Input, Decode));
	}
	Ok(data.chunks_exact(3).map(|c| Rgba::opaque(c[0], c[1], c[2])).collect())
}

/// Reverses one scanline's filter, in place.
fn unfilter_scanline(
	ftype:	u8,
	line:	&mut [u8],
	prev:	&[u8],
	bpp:	usize,
	y:	usize,
)
	-> Outcome<()>
{
	let n = line.len();
	for i in 0..n {
		let a = if i >= bpp { line[i - bpp] } else { 0 }; // Left, already unfiltered.
		let b = prev[i]; // Above.
		let c = if i >= bpp { prev[i - bpp] } else { 0 }; // Above left.
		let x = line[i];
		line[i] = match ftype {
			0 => x,
			1 => x.wrapping_add(a),
			2 => x.wrapping_add(b),
			3 => x.wrapping_add(((a as u16 + b as u16) / 2) as u8),
			4 => x.wrapping_add(paeth(a, b, c)),
			_ => return Err(err!(
				"Scanline {} declares filter type {}, which is not one of 0 to 4.", y, ftype;
			Invalid, Input, Decode)),
		};
	}
	Ok(())
}

/// Reads one pixel out of an unfiltered scanline, whatever the colour type.
fn pixel_of(
	ct:	ColourType,
	line:	&[u8],
	x:	usize,
	bpp:	usize,
	palette: &[Rgba],
)
	-> Outcome<Rgba>
{
	let i = x * bpp;
	match ct {
		ColourType::Grey		=> Ok(Rgba::opaque(line[i], line[i], line[i])),
		ColourType::Rgb			=> Ok(Rgba::opaque(line[i], line[i + 1], line[i + 2])),
		ColourType::GreyAlpha		=> Ok(Rgba::new(line[i], line[i], line[i], line[i + 1])),
		ColourType::Rgba		=> Ok(Rgba::new(
							line[i],
							line[i + 1],
							line[i + 2],
							line[i + 3],
						)),
		ColourType::Palette => {
			let idx = line[i] as usize;
			match palette.get(idx) {
				Some(c) => Ok(*c),
				None => Err(err!(
					"A palette PNG names colour {} at pixel {}, but its palette holds {}.",
					idx, x, palette.len();
				Invalid, Input, Decode, Range)),
			}
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::path::Bounds;

	#[test]
	fn test_a_pixmap_survives_a_round_trip_00() -> Outcome<()> {
		let mut pm = res!(Pixmap::filled(17, 9, Rgba::new(10, 20, 30, 255)));
		res!(pm.fill_bounds(Bounds::new(2.0, 2.0, 8.0, 6.0), Rgba::new(200, 100, 50, 128), None));
		let buf = res!(encode(&pm));
		let back = res!(decode(&buf));
		assert_eq!(back.width(), 17);
		assert_eq!(back.height(), 9);
		assert_eq!(back, pm, "the decoded pixmap must equal the one encoded");
		Ok(())
	}

	#[test]
	fn test_the_signature_is_checked_01() {
		assert!(decode(&[0u8; 8]).is_err());
		assert!(decode(&[]).is_err());
	}

	#[test]
	fn test_a_corrupted_crc_is_caught_02() -> Outcome<()> {
		let pm = res!(Pixmap::filled(4, 4, Rgba::WHITE));
		let mut buf = res!(encode(&pm));
		// Flip a byte of the image data, leaving its chunk's CRC declaring the old bytes.
		let n = buf.len();
		buf[n - 20] ^= 0xFF;
		assert!(decode(&buf).is_err(), "a corrupted chunk must not decode");
		Ok(())
	}

	#[test]
	fn test_a_truncated_file_is_caught_03() -> Outcome<()> {
		let pm = res!(Pixmap::filled(4, 4, Rgba::WHITE));
		let buf = res!(encode(&pm));
		for cut in [10, 20, buf.len() - 1] {
			assert!(decode(&buf[..cut]).is_err(), "a file cut at {} must not decode", cut);
		}
		Ok(())
	}

	#[test]
	fn test_an_absurd_header_is_refused_04() -> Outcome<()> {
		// A header claiming 60000 by 60000 pixels: 3.6 billion, over the ceiling.
		let pm = res!(Pixmap::filled(2, 2, Rgba::WHITE));
		let mut buf = res!(encode(&pm));
		buf[16..20].copy_from_slice(&60000u32.to_be_bytes());
		buf[20..24].copy_from_slice(&60000u32.to_be_bytes());
		// Repair the CRC, so that the size and not the checksum is what refuses it. The CRC covers
		// the chunk's type and data: 4 + 13 bytes from offset 12.
		let crc = crc32(&buf[12..29]);
		buf[29..33].copy_from_slice(&crc.to_be_bytes());
		assert!(decode(&buf).is_err(), "a header over the pixel ceiling must be refused");
		Ok(())
	}

	#[test]
	fn test_crc32_matches_the_known_value_05() {
		// The CRC-32 of "123456789" is a standard check value.
		assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
	}

	#[test]
	fn test_paeth_prefers_the_nearest_neighbour_06() {
		assert_eq!(paeth(10, 20, 10), 20); // The estimate lands on b.
		assert_eq!(paeth(200, 5, 5), 200); // The estimate lands on a.
	}
}
