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
//!
//! The `tRNS` chunk is read. It is nominally ancillary, but it is the one ancillary chunk that
//! carries pixel data: for the three colour types without an alpha channel of their own it is where
//! the alpha channel is written, so a decoder that skips it does not drop decoration, it reports the
//! wrong image.

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

/// What a `tRNS` chunk says, which is a different thing for each colour type that may carry one.
#[derive(Clone, Debug)]
enum Trns {
	/// One alpha byte per palette entry, in palette order. It may be shorter than the palette, and
	/// every entry beyond its end is opaque.
	Palette(Vec<u8>),
	/// The one luminance that is fully transparent. Every other luminance is opaque.
	Grey(u8),
	/// The one colour that is fully transparent. Every other colour is opaque.
	Rgb(u8, u8, u8),
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
	let mut trns: Option<Trns> = None;
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
			b"tRNS" => {
				// tRNS precedes the image data, and for a palette image follows the palette.
				if !idat.is_empty() {
					return Err(err!(
						"The PNG carries a tRNS chunk after its image data, but tRNS precedes IDAT.";
					Invalid, Input, Decode));
				}
				let h = match hdr {
					Some(h) => h,
					None => return Err(err!(
						"The PNG carries a tRNS chunk before its IHDR chunk.";
					Invalid, Input, Decode, Missing)),
				};
				trns = Some(res!(decode_transparency(data, h.ct, &palette)));
			},
			b"IDAT" => idat.extend_from_slice(data),
			b"IEND" => {
				ended = true;
				break;
			},
			_ => (), // The remaining ancillary chunks are decoration, and not our business.
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
			let c = res!(pixel_of(hdr.ct, &line, x, bpp, &palette, trns.as_ref()));
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

/// Reads a transparency chunk, whose shape the colour type it accompanies decides.
///
/// The specification forbids `tRNS` to the two colour types that already carry an alpha channel, so
/// its presence there is a malformed file rather than a chunk to ignore.
fn decode_transparency(data: &[u8], ct: ColourType, palette: &[Rgba]) -> Outcome<Trns> {
	match ct {
		ColourType::Palette => {
			if palette.is_empty() {
				return Err(err!(
					"The PNG carries a tRNS chunk before its PLTE chunk. In a palette image the \
					palette comes first, because tRNS gives one alpha byte per palette entry.";
				Invalid, Input, Decode, Order));
			}
			if data.len() > palette.len() {
				return Err(err!(
					"The PNG's tRNS chunk holds {} alpha bytes, but its palette holds only {} \
					entries.", data.len(), palette.len();
				Invalid, Input, Decode, Mismatch));
			}
			Ok(Trns::Palette(data.to_vec()))
		},
		ColourType::Grey => {
			if data.len() != 2 {
				return Err(err!(
					"A greyscale PNG's tRNS chunk is a single 2-byte sample, but this one is {} \
					bytes.", data.len();
				Invalid, Input, Decode));
			}
			Ok(Trns::Grey(res!(trns_sample(&data[0..2], "luminance"))))
		},
		ColourType::Rgb => {
			if data.len() != 6 {
				return Err(err!(
					"A truecolour PNG's tRNS chunk is three 2-byte samples, but this one is {} \
					bytes.", data.len();
				Invalid, Input, Decode));
			}
			Ok(Trns::Rgb(
				res!(trns_sample(&data[0..2], "red")),
				res!(trns_sample(&data[2..4], "green")),
				res!(trns_sample(&data[4..6], "blue")),
			))
		},
		ColourType::GreyAlpha | ColourType::Rgba => Err(err!(
			"The PNG carries a tRNS chunk under colour type {:?}, which already has an alpha \
			channel. The specification forbids the combination.", ct;
		Invalid, Input, Decode)),
	}
}

/// Reads one of `tRNS`'s samples, which the specification writes as 16 bits big-endian whatever the
/// bit depth.
///
/// At eight bits a channel the value must fall in 0 to 255, since it is compared against a pixel of
/// that width. A larger one names a sample no pixel here can hold, and is refused rather than
/// truncated into a match that was never in the file.
fn trns_sample(be: &[u8], name: &str) -> Outcome<u8> {
	let v = u16::from_be_bytes([be[0], be[1]]);
	if v > 255 {
		return Err(err!(
			"The PNG's tRNS chunk names a transparent {} of {}, but at 8 bits a channel its samples \
			run from 0 to 255.", name, v;
		Invalid, Input, Decode, Range));
	}
	Ok(v as u8)
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
///
/// The three colour types that carry no alpha channel take theirs from `tRNS`, if the file gave one.
fn pixel_of(
	ct:	ColourType,
	line:	&[u8],
	x:	usize,
	bpp:	usize,
	palette: &[Rgba],
	trns:	Option<&Trns>,
)
	-> Outcome<Rgba>
{
	let i = x * bpp;
	match ct {
		ColourType::Grey => {
			let g = line[i];
			let a = match trns {
				Some(Trns::Grey(t)) if g == *t	=> 0,
				_				=> 255,
			};
			Ok(Rgba::new(g, g, g, a))
		},
		ColourType::Rgb => {
			let (r, g, b) = (line[i], line[i + 1], line[i + 2]);
			let a = match trns {
				Some(Trns::Rgb(tr, tg, tb)) if r == *tr && g == *tg && b == *tb	=> 0,
				_								=> 255,
			};
			Ok(Rgba::new(r, g, b, a))
		},
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
				Some(c) => {
					let mut c = *c;
					// tRNS may stop short of the palette's end, leaving the rest opaque.
					if let Some(Trns::Palette(alpha)) = trns {
						if let Some(a) = alpha.get(idx) {
							c.a = *a;
						}
					}
					Ok(c)
				},
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

	// ┌───────────────────────────────────────────────────────────────────────┐
	// │ tRNS                                                                   │
	// └───────────────────────────────────────────────────────────────────────┘
	//
	// The encoder above writes colour type 6 and nothing else, so it can never produce a file with a
	// tRNS chunk in it, and a round trip through our own encoder cannot say whether tRNS is read
	// correctly or read at all. The three files below are therefore written out byte by byte, and
	// the alpha each pixel is expected to carry was taken from an independent decoder (Python's
	// PIL, reading these exact bytes) rather than from this one.

	/// Colour type 3, 4 by 2. Four palette entries: red, green, blue, white. The tRNS chunk is two
	/// bytes long against a palette of four, so entries 2 and 3 fall beyond it and stay opaque.
	const PAL_TRNS: [u8; 113] = [
		0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
		0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02,
		0x08, 0x03, 0x00, 0x00, 0x00, 0x48, 0x76, 0x8D, 0x51, 0x00, 0x00, 0x00,
		0x0C, 0x50, 0x4C, 0x54, 0x45, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00,
		0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFB, 0x00, 0x60, 0xF6, 0x00, 0x00, 0x00,
		0x02, 0x74, 0x52, 0x4E, 0x53, 0x00, 0x80, 0x9B, 0x2B, 0x4E, 0x18, 0x00,
		0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x60, 0x60,
		0x64, 0x62, 0x66, 0x60, 0x66, 0x62, 0x64, 0x00, 0x00, 0x00, 0x46, 0x00,
		0x0D, 0xA4, 0x00, 0x59, 0x7B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
		0x44, 0xAE, 0x42, 0x60, 0x82,
	];

	/// Colour type 0, 4 by 2. The tRNS chunk names the single transparent luminance, 128.
	const GREY_TRNS: [u8; 89] = [
		0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
		0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02,
		0x08, 0x00, 0x00, 0x00, 0x00, 0x5A, 0xC3, 0x22, 0xBF, 0x00, 0x00, 0x00,
		0x02, 0x74, 0x52, 0x4E, 0x53, 0x00, 0x80, 0x9B, 0x2B, 0x4E, 0x18, 0x00,
		0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x60, 0x68,
		0xF8, 0xDF, 0xC0, 0xD0, 0xC0, 0xD5, 0x70, 0x02, 0x00, 0x11, 0xE9, 0x03,
		0xD2, 0xF6, 0xE5, 0x55, 0x6C, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
		0x44, 0xAE, 0x42, 0x60, 0x82,
	];

	/// Colour type 2, 4 by 2. The tRNS chunk names the single transparent colour, pure red. This is
	/// the shape of PngSuite's `tbrn2c08`, where an independent decoder finds 453 of the 1024 pixels
	/// fully transparent and this codec, before tRNS was read, found none.
	const RGB_TRNS: [u8; 102] = [
		0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
		0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02,
		0x08, 0x02, 0x00, 0x00, 0x00, 0xF0, 0xCA, 0xEA, 0x34, 0x00, 0x00, 0x00,
		0x06, 0x74, 0x52, 0x4E, 0x53, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xA4,
		0xC2, 0xC0, 0x1D, 0x00, 0x00, 0x00, 0x1B, 0x49, 0x44, 0x41, 0x54, 0x78,
		0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0xC0, 0xF0, 0x1F, 0x08, 0x19, 0x18, 0x99,
		0x98, 0x41, 0xD4, 0x7F, 0x06, 0x46, 0xB0, 0x08, 0x03, 0x00, 0x59, 0x20,
		0x06, 0x02, 0x5D, 0xD3, 0x95, 0xA8, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
		0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
	];

	/// Checks a decoded pixmap against every pixel an independent decoder found in it.
	fn expect_pixels(buf: &[u8], want: &[[(u8, u8, u8, u8); 4]; 2]) -> Outcome<()> {
		let pm = res!(decode(buf));
		for y in 0..2 {
			for x in 0..4 {
				let (r, g, b, a) = want[y][x];
				let got = match pm.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!(
						"Pixel {},{} lies outside the decoded pixmap.", x, y; Invalid, Input)),
				};
				assert_eq!(
					got, Rgba::new(r, g, b, a),
					"pixel {},{} should be {:?}", x, y, Rgba::new(r, g, b, a),
				);
			}
		}
		Ok(())
	}

	#[test]
	fn test_trns_gives_a_palette_image_its_alpha_07() -> Outcome<()> {
		res!(expect_pixels(&PAL_TRNS, &[
			[(255, 0, 0, 0),		(0, 255, 0, 128),	(0, 0, 255, 255),	(255, 255, 255, 255)],
			[(255, 255, 255, 255),	(0, 0, 255, 255),	(0, 255, 0, 128),	(255, 0, 0, 0)],
		]));
		Ok(())
	}

	#[test]
	fn test_trns_gives_a_greyscale_image_its_alpha_08() -> Outcome<()> {
		res!(expect_pixels(&GREY_TRNS, &[
			[(0, 0, 0, 255),	(128, 128, 128, 0),	(255, 255, 255, 255),	(128, 128, 128, 0)],
			[(128, 128, 128, 0),	(10, 10, 10, 255),	(128, 128, 128, 0),	(200, 200, 200, 255)],
		]));
		Ok(())
	}

	#[test]
	fn test_trns_gives_a_truecolour_image_its_alpha_09() -> Outcome<()> {
		// The second pixel of the second row is 255,0,1: one off the transparent colour, and so
		// opaque. An implementation that compared loosely would report it transparent.
		res!(expect_pixels(&RGB_TRNS, &[
			[(255, 0, 0, 0),	(0, 255, 0, 255),	(255, 0, 0, 0),		(1, 2, 3, 255)],
			[(255, 0, 0, 0),	(255, 0, 1, 255),	(0, 0, 0, 255),		(255, 0, 0, 0)],
		]));
		Ok(())
	}

	/// Assembles a PNG from the chunks given, in the order given, between a signature and an IEND.
	fn assemble(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(&SIG);
		for (kind, data) in chunks {
			write_chunk(&mut out, kind, data);
		}
		write_chunk(&mut out, b"IEND", &[]);
		out
	}

	/// An image header, eight bits a channel and not interlaced.
	fn ihdr(w: u32, h: u32, ct: u8) -> Vec<u8> {
		let mut v = Vec::with_capacity(13);
		v.extend_from_slice(&w.to_be_bytes());
		v.extend_from_slice(&h.to_be_bytes());
		v.extend_from_slice(&[8, ct, 0, 0, 0]);
		v
	}

	/// Deflates raw scanlines, each already carrying its filter byte, into an IDAT payload.
	fn idat_of(raw: &[u8]) -> Outcome<Vec<u8>> {
		let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
		res!(z.write_all(raw));
		Ok(res!(z.finish()))
	}

	#[test]
	fn test_trns_is_refused_where_the_specification_forbids_it_10() -> Outcome<()> {
		// Colour type 6 already carries an alpha channel, so tRNS has nothing to say and the
		// specification forbids it. A file carrying both is malformed, not merely odd.
		let idat = res!(idat_of(&[0, 1, 2, 3, 4]));
		let buf = assemble(&[
			(b"IHDR", ihdr(1, 1, 6)),
			(b"tRNS", vec![0x00, 0x80]),
			(b"IDAT", idat),
		]);
		assert!(decode(&buf).is_err(), "tRNS under colour type 6 must be refused");
		Ok(())
	}

	#[test]
	fn test_a_malformed_trns_is_refused_11() -> Outcome<()> {
		let plte = vec![255, 0, 0, 0, 255, 0]; // Two entries: red, green.
		let pal_idat = res!(idat_of(&[0, 0])); // Filter 0, then palette index 0.
		let grey_idat = res!(idat_of(&[0, 128])); // Filter 0, then the luminance 128.

		// A sample of 256 cannot apply to an 8-bit pixel, and must be refused, not truncated to 0.
		let over = assemble(&[
			(b"IHDR", ihdr(1, 1, 0)),
			(b"tRNS", vec![0x01, 0x00]),
			(b"IDAT", res!(idat_of(&[0, 0]))),
		]);
		assert!(decode(&over).is_err(), "a tRNS sample above 255 must be refused at 8 bits");

		// A greyscale tRNS is exactly two bytes.
		let short = assemble(&[
			(b"IHDR", ihdr(1, 1, 0)),
			(b"tRNS", vec![0x80]),
			(b"IDAT", grey_idat.clone()),
		]);
		assert!(decode(&short).is_err(), "a one-byte greyscale tRNS must be refused");

		// More alpha bytes than the palette has entries.
		let long = assemble(&[
			(b"IHDR", ihdr(1, 1, 3)),
			(b"PLTE", plte.clone()),
			(b"tRNS", vec![0, 0, 0]),
			(b"IDAT", pal_idat.clone()),
		]);
		assert!(decode(&long).is_err(), "a tRNS longer than the palette must be refused");

		// tRNS gives one alpha byte per palette entry, so it cannot precede the palette.
		let early = assemble(&[
			(b"IHDR", ihdr(1, 1, 3)),
			(b"tRNS", vec![0]),
			(b"PLTE", plte.clone()),
			(b"IDAT", pal_idat.clone()),
		]);
		assert!(decode(&early).is_err(), "a tRNS before the PLTE must be refused");

		// tRNS carries pixel data, so it cannot arrive after the pixels it applies to.
		let late = assemble(&[
			(b"IHDR", ihdr(1, 1, 0)),
			(b"IDAT", grey_idat),
			(b"tRNS", vec![0x00, 0x80]),
		]);
		assert!(decode(&late).is_err(), "a tRNS after the IDAT must be refused");

		// The same file, with the tRNS where it belongs, decodes: it is the order that is refused
		// above and not the chunk.
		let good = assemble(&[
			(b"IHDR", ihdr(1, 1, 3)),
			(b"PLTE", plte),
			(b"tRNS", vec![0]),
			(b"IDAT", pal_idat),
		]);
		let pm = res!(decode(&good));
		let got = match pm.pixel(0, 0) {
			Some(c) => c,
			None => return Err(err!("A 1 by 1 pixmap has a pixel."; Invalid, Input)),
		};
		assert_eq!(got, Rgba::new(255, 0, 0, 0), "the palette's first entry is transparent");
		Ok(())
	}
}
