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
//! The decoder reads every combination of bit depth and colour type the specification allows -- 1,
//! 2, 4, 8 and 16 bits per channel across greyscale, truecolour, palette, greyscale with alpha and
//! truecolour with alpha -- with or without Adam7 interlacing. The two are independent, so an
//! interlaced 1-bit palette image and a non-interlaced 16-bit truecolour one are both read.
//!
//! Samples narrower than eight bits are widened so that the widest value the depth can hold becomes
//! 255: a 1-bit sample is 0 or 255, a 2-bit one a multiple of 85, a 4-bit one a multiple of 17.
//! Sixteen-bit samples are reduced to their high byte, which is a deliberate loss: a [`Pixmap`] is
//! eight bits a channel, and a lossless path for 16 would be a second pixel type rather than a
//! change here. Palette indices are never widened, since they are indices and not intensities.
//!
//! The `tRNS` chunk is read. It is nominally ancillary, but it is the one ancillary chunk that
//! carries pixel data: for the three colour types without an alpha channel of their own it is where
//! the alpha channel is written, so a decoder that skips it does not drop decoration, it reports the
//! wrong image. Its samples are compared against the file's own samples at the file's own depth,
//! before any widening, so the match is the one the specification describes.
//!
//! # What is refused, by name
//!
//! A colour type outside 0, 2, 3, 4 and 6; a bit depth outside 1, 2, 4, 8 and 16; a depth the
//! declared colour type does not allow; a compression method other than DEFLATE; a filter method
//! other than the adaptive one; an interlace method other than none or Adam7; a `tRNS` sample wider
//! than the declared depth; a `tRNS` chunk under a colour type that already carries alpha, or out
//! of order; a palette index beyond the palette; a chunk whose CRC does not match; and image data
//! that decompresses to anything other than the exact size the header implies.
//!
//! The encoder writes one form only: eight-bit truecolour with alpha, not interlaced.

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

	/// The number of samples each pixel carries.
	fn channels(&self) -> usize {
		match self {
			Self::Grey		=> 1,
			Self::Rgb		=> 3,
			Self::Palette		=> 1,
			Self::GreyAlpha		=> 2,
			Self::Rgba		=> 4,
		}
	}

	/// The bit depths the specification allows this colour type.
	///
	/// Only the two one-channel types may go below eight bits, and only the types whose samples are
	/// intensities rather than palette indices may go above it.
	fn depths(&self) -> &'static [u8] {
		match self {
			Self::Grey		=> &[1, 2, 4, 8, 16],
			Self::Rgb		=> &[8, 16],
			Self::Palette		=> &[1, 2, 4, 8],
			Self::GreyAlpha		=> &[8, 16],
			Self::Rgba		=> &[8, 16],
		}
	}
}

/// What a `tRNS` chunk says, which is a different thing for each colour type that may carry one.
///
/// The greyscale and truecolour forms hold the sample as the file writes it, at the file's own bit
/// depth, because that is what they are compared against.
#[derive(Clone, Debug)]
enum Trns {
	/// One alpha byte per palette entry, in palette order. It may be shorter than the palette, and
	/// every entry beyond its end is opaque.
	Palette(Vec<u8>),
	/// The one luminance that is fully transparent. Every other luminance is opaque.
	Grey(u16),
	/// The one colour that is fully transparent. Every other colour is opaque.
	Rgb(u16, u16, u16),
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
	/// Bits per sample: 1, 2, 4, 8 or 16.
	depth:	u8,
	/// Whether the image data is Adam7 interlaced.
	laced:	bool,
}

/// One pass of image data: where its pixels begin, how far apart they sit, and how many there are.
///
/// A non-interlaced image is one pass with a step of one in each direction, so the interlaced and
/// non-interlaced cases share every line of the decoding below.
#[derive(Clone, Copy, Debug)]
struct Pass {
	/// Column of the pass's first pixel.
	x0:	usize,
	/// Row of the pass's first pixel.
	y0:	usize,
	/// Columns between one of the pass's pixels and the next.
	dx:	usize,
	/// Rows between one of the pass's scanlines and the next.
	dy:	usize,
	/// Pixels across the pass.
	w:	usize,
	/// Scanlines down the pass.
	h:	usize,
}

/// The seven Adam7 passes, as the offset and step at which each lays its pixels into the image.
const ADAM7: [(usize, usize, usize, usize); 7] = [
	(0, 0, 8, 8),
	(4, 0, 8, 8),
	(0, 4, 4, 8),
	(2, 0, 4, 4),
	(0, 2, 2, 4),
	(1, 0, 2, 2),
	(0, 1, 1, 2),
];

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

/// The passes the image data is written in: one for a plain image, up to seven for an interlaced
/// one.
///
/// An Adam7 pass whose grid falls entirely outside a small image holds no pixels and no scanlines,
/// and contributes nothing at all to the stream -- not even a filter byte. Such passes are dropped
/// here, so that everything downstream, the size arithmetic included, sees only passes that exist.
fn passes_of(hdr: &Header) -> Vec<Pass> {
	if !hdr.laced {
		return vec![Pass { x0: 0, y0: 0, dx: 1, dy: 1, w: hdr.w, h: hdr.h }];
	}
	let mut out = Vec::with_capacity(ADAM7.len());
	for (x0, y0, dx, dy) in ADAM7 {
		// The pixels of a pass are those at x0, x0 + dx, x0 + 2dx and so on that fall inside the
		// image, which is a count of zero once x0 reaches the width.
		let w = if hdr.w > x0 { (hdr.w - x0 + dx - 1) / dx } else { 0 };
		let h = if hdr.h > y0 { (hdr.h - y0 + dy - 1) / dy } else { 0 };
		if w > 0 && h > 0 {
			out.push(Pass { x0, y0, dx, dy, w, h });
		}
	}
	out
}

/// The number of bytes one scanline of `w` pixels occupies, rounded up to a whole byte.
fn row_bytes(ct: ColourType, depth: u8, w: usize) -> Outcome<usize> {
	let bits = match w.checked_mul(ct.channels()).and_then(|n| n.checked_mul(depth as usize)) {
		Some(n) => n,
		None => return Err(err!(
			"A PNG scanline of {} pixels at {} channels of {} bits overflows a count of bits.",
			w, ct.channels(), depth;
		Invalid, Input, Decode, Overflow)),
	};
	Ok((bits + 7) / 8)
}

/// The number of bytes a pixel occupies in a filtered scanline, which is what the filters step by.
///
/// The specification rounds this up to one, so that samples narrower than a byte filter against the
/// byte beside them rather than against a fraction of one.
fn filter_bpp(ct: ColourType, depth: u8) -> usize {
	let bits = ct.channels() * depth as usize;
	std::cmp::max(1, bits / 8)
}

/// The exact number of bytes the image data must decompress to.
///
/// Each scanline of each pass carries one filter byte ahead of its samples, so an interlaced image
/// carries as many filter bytes as all seven passes have scanlines between them, which is more than
/// the image has rows. Getting this wrong in either direction either refuses a sound file or lets a
/// larger stream through the ceiling, so it is summed over the passes rather than estimated.
fn expected_size(hdr: &Header, passes: &[Pass]) -> Outcome<usize> {
	let mut total = 0usize;
	for p in passes {
		let stride = res!(row_bytes(hdr.ct, hdr.depth, p.w));
		let pass = match (stride + 1).checked_mul(p.h) {
			Some(n) => n,
			None => return Err(err!(
				"A PNG pass of {} by {} pixels overflows a count of bytes.", p.w, p.h;
			Invalid, Input, Decode, Overflow)),
		};
		total = match total.checked_add(pass) {
			Some(n) => n,
			None => return Err(err!(
				"A PNG of {} by {} pixels overflows a count of image bytes.", hdr.w, hdr.h;
			Invalid, Input, Decode, Overflow)),
		};
	}
	Ok(total)
}

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
				trns = Some(res!(decode_transparency(data, h.ct, h.depth, &palette)));
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

	// Inflate, refusing anything larger than the header says it should be. The buffer starts small
	// whatever the header claims, so that a handful of bytes declaring a large image cannot make us
	// reserve a large allocation before a single byte of it has been inflated.
	let passes = passes_of(&hdr);
	let expect = res!(expected_size(&hdr, &passes));
	let mut raw = Vec::with_capacity(std::cmp::min(expect, 1 << 20));
	let mut z = ZlibDecoder::new(&idat[..]).take((expect as u64) + 1);
	res!(z.read_to_end(&mut raw));
	if raw.len() != expect {
		return Err(err!(
			"The PNG's image data decompresses to {} bytes, but its header of {} by {} pixels at {} \
			bits a channel{} implies {}.",
			raw.len(), hdr.w, hdr.h, hdr.depth,
			if hdr.laced { ", interlaced," } else { "," }, expect;
		Invalid, Input, Decode, Mismatch));
	}

	// Unfilter each pass, then expand into RGBA. A pass's scanlines filter against each other and
	// not against the image's rows, so `prev` restarts at each pass.
	let bpp = filter_bpp(hdr.ct, hdr.depth);
	let mut pm = res!(Pixmap::new(hdr.w, hdr.h));
	let mut at = 0;
	for p in &passes {
		let stride = res!(row_bytes(hdr.ct, hdr.depth, p.w));
		let mut prev = vec![0u8; stride];
		let mut line = vec![0u8; stride];
		for j in 0..p.h {
			let ftype = raw[at];
			line.copy_from_slice(&raw[at + 1..at + 1 + stride]);
			at += stride + 1;
			res!(unfilter_scanline(ftype, &mut line, &prev, bpp, j));
			let y = p.y0 + j * p.dy;
			for i in 0..p.w {
				let c = res!(pixel_of(&hdr, &line, i, &palette, trns.as_ref()));
				pm.set_pixel(p.x0 + i * p.dx, y, c);
			}
			prev.copy_from_slice(&line);
		}
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
	let comp = data[10];
	let filt = data[11];
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
	if !matches!(depth, 1 | 2 | 4 | 8 | 16) {
		return Err(err!(
			"The PNG declares {} bits per channel, which is not one of 1, 2, 4, 8 or 16.", depth;
		Invalid, Input, Decode));
	}
	if !ct.depths().contains(&depth) {
		return Err(err!(
			"The PNG declares {} bits per channel under colour type {:?}, which the specification \
			allows only at {:?} bits.", depth, ct, ct.depths();
		Invalid, Input, Decode));
	}
	if comp != 0 {
		return Err(err!(
			"The PNG declares compression method {}. DEFLATE, method 0, is the only one the \
			specification defines.", comp;
		Invalid, Input, Decode));
	}
	if filt != 0 {
		return Err(err!(
			"The PNG declares filter method {}. The adaptive filtering of method 0 is the only one \
			the specification defines.", filt;
		Invalid, Input, Decode));
	}
	let laced = match interlace {
		0	=> false,
		1	=> true,
		_	=> return Err(err!(
			"The PNG declares interlace method {}, which is neither 0, no interlacing, nor 1, \
			Adam7.", interlace;
		Invalid, Input, Decode)),
	};
	Ok(Header { w, h, ct, depth, laced })
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
fn decode_transparency(data: &[u8], ct: ColourType, depth: u8, palette: &[Rgba]) -> Outcome<Trns> {
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
			Ok(Trns::Grey(res!(trns_sample(&data[0..2], "luminance", depth))))
		},
		ColourType::Rgb => {
			if data.len() != 6 {
				return Err(err!(
					"A truecolour PNG's tRNS chunk is three 2-byte samples, but this one is {} \
					bytes.", data.len();
				Invalid, Input, Decode));
			}
			Ok(Trns::Rgb(
				res!(trns_sample(&data[0..2], "red", depth)),
				res!(trns_sample(&data[2..4], "green", depth)),
				res!(trns_sample(&data[4..6], "blue", depth)),
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
/// The value must fit the declared depth, since it is compared against samples of that width. A
/// larger one names a sample no pixel in the file can hold, and is refused rather than truncated
/// into a match that was never there.
fn trns_sample(be: &[u8], name: &str, depth: u8) -> Outcome<u16> {
	let v = u16::from_be_bytes([be[0], be[1]]);
	let max = if depth >= 16 { u16::MAX } else { (1u16 << depth) - 1 };
	if v > max {
		return Err(err!(
			"The PNG's tRNS chunk names a transparent {} of {}, but at {} bits a channel its \
			samples run from 0 to {}.", name, v, depth, max;
		Invalid, Input, Decode, Range));
	}
	Ok(v)
}

/// Reads the `i`th sample of an unfiltered scanline, at the bit depth the header declares.
///
/// Samples narrower than a byte are packed most significant first, and the row is padded out to a
/// whole byte. The padding is masked away rather than merely left unread, so bits a hostile file
/// sets there cannot reach a pixel.
fn sample_at(line: &[u8], i: usize, depth: u8) -> Outcome<u16> {
	match depth {
		1 | 2 | 4 => {
			let per = 8 / depth as usize; // Samples to the byte.
			let byte = match line.get(i / per) {
				Some(b) => *b,
				None => return Err(err!(
					"Sample {} at {} bits lies beyond a scanline of {} bytes.", i, depth, line.len();
				Invalid, Input, Decode, Range)),
			};
			let shift = 8 - depth as usize * (i % per + 1);
			Ok(((byte >> shift) as u16) & ((1u16 << depth) - 1))
		},
		8 => match line.get(i) {
			Some(b) => Ok(*b as u16),
			None => Err(err!(
				"Sample {} at 8 bits lies beyond a scanline of {} bytes.", i, line.len();
			Invalid, Input, Decode, Range)),
		},
		16 => match (line.get(2 * i), line.get(2 * i + 1)) {
			(Some(hi), Some(lo)) => Ok(u16::from_be_bytes([*hi, *lo])),
			_ => Err(err!(
				"Sample {} at 16 bits lies beyond a scanline of {} bytes.", i, line.len();
			Invalid, Input, Decode, Range)),
		},
		_ => Err(err!(
			"A scanline was read at {} bits a sample, which the header should have refused.", depth;
		Bug, Unreachable)),
	}
}

/// Widens a sample of the declared bit depth to the eight bits a pixmap holds.
///
/// The narrow depths are scaled so that the widest value the depth can hold becomes 255, which is
/// what the specification's sample-depth scaling amounts to and what makes a 1-bit image black and
/// white rather than black and very-nearly-black.
///
/// Sixteen bits are reduced by keeping the high byte, the same reduction `libpng` performs for
/// `png_set_strip_16`. It is a truncation and not a rounding: a sample that is an eight-bit value
/// written twice, which is what almost every 16-bit file in practice holds, survives it exactly,
/// and anything else loses at most one part in 256. A pixmap is eight bits a channel, so some
/// reduction has to happen here; a lossless path would be a wider pixel type, not a change to this
/// function.
fn widen(v: u16, depth: u8) -> u8 {
	match depth {
		1	=> if v == 0 { 0 } else { 255 },
		2	=> (v as u8) * 85,
		4	=> (v as u8) * 17,
		8	=> v as u8,
		_	=> (v >> 8) as u8,
	}
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

/// Reads one pixel out of an unfiltered scanline, whatever the colour type and bit depth.
///
/// The three colour types that carry no alpha channel take theirs from `tRNS`, if the file gave one.
/// The comparison against `tRNS` happens on the raw sample, before widening, because that is the
/// sample the chunk names; comparing widened values would make every 1-bit black pixel match a
/// `tRNS` of 0 whether or not the file said so.
fn pixel_of(
	hdr:	&Header,
	line:	&[u8],
	x:	usize,
	palette: &[Rgba],
	trns:	Option<&Trns>,
)
	-> Outcome<Rgba>
{
	let d = hdr.depth;
	let i = x * hdr.ct.channels(); // Index of the pixel's first sample.
	match hdr.ct {
		ColourType::Grey => {
			let s = res!(sample_at(line, i, d));
			let a = match trns {
				Some(Trns::Grey(t)) if s == *t	=> 0,
				_				=> 255,
			};
			let g = widen(s, d);
			Ok(Rgba::new(g, g, g, a))
		},
		ColourType::Rgb => {
			let r = res!(sample_at(line, i, d));
			let g = res!(sample_at(line, i + 1, d));
			let b = res!(sample_at(line, i + 2, d));
			let a = match trns {
				Some(Trns::Rgb(tr, tg, tb)) if r == *tr && g == *tg && b == *tb	=> 0,
				_								=> 255,
			};
			Ok(Rgba::new(widen(r, d), widen(g, d), widen(b, d), a))
		},
		ColourType::GreyAlpha => {
			let g = widen(res!(sample_at(line, i, d)), d);
			let a = widen(res!(sample_at(line, i + 1, d)), d);
			Ok(Rgba::new(g, g, g, a))
		},
		ColourType::Rgba		=> Ok(Rgba::new(
							widen(res!(sample_at(line, i, d)), d),
							widen(res!(sample_at(line, i + 1, d)), d),
							widen(res!(sample_at(line, i + 2, d)), d),
							widen(res!(sample_at(line, i + 3, d)), d),
						)),
		ColourType::Palette => {
			// A palette sample is an index and not an intensity, so it is never widened.
			let idx = res!(sample_at(line, i, d)) as usize;
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
		ihdr_at(w, h, 8, ct, 0)
	}

	/// An image header at a given bit depth and interlace method.
	fn ihdr_at(w: u32, h: u32, depth: u8, ct: u8, laced: u8) -> Vec<u8> {
		let mut v = Vec::with_capacity(13);
		v.extend_from_slice(&w.to_be_bytes());
		v.extend_from_slice(&h.to_be_bytes());
		v.extend_from_slice(&[depth, ct, 0, 0, laced]);
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

	// ┌───────────────────────────────────────────────────────────────────────┐
	// │ ADAM7, BIT DEPTH, AND THE SIZES THEY IMPLY                             │
	// └───────────────────────────────────────────────────────────────────────┘

	/// A header for the pass and size arithmetic below, which reads nothing else from it.
	fn hdr_of(w: usize, h: usize, ct: ColourType, depth: u8, laced: bool) -> Header {
		Header { w, h, ct, depth, laced }
	}

	#[test]
	fn test_the_adam7_passes_partition_the_image_12() -> Outcome<()> {
		// Whatever the size, the seven passes between them name every pixel exactly once. Summing
		// their areas is therefore a check on all seven width and height formulae at once, and it
		// catches the off-by-one that a size smaller than a pass's grid invites.
		for w in 1..=20usize {
			for h in 1..=20usize {
				let hdr = hdr_of(w, h, ColourType::Grey, 8, true);
				let passes = passes_of(&hdr);
				let area: usize = passes.iter().map(|p| p.w * p.h).sum();
				assert_eq!(area, w * h, "the passes of a {} by {} image cover it once", w, h);

				// And no pass may lay a pixel outside the image.
				for p in &passes {
					assert!(p.x0 + (p.w - 1) * p.dx < w, "a pass of {} by {} runs off the right", w, h);
					assert!(p.y0 + (p.h - 1) * p.dy < h, "a pass of {} by {} runs off the bottom", w, h);
				}
			}
		}

		// The passes a small image leaves empty are dropped, not carried as zero-sized ones: an
		// empty pass contributes no scanline and no filter byte to the stream at all.
		assert_eq!(passes_of(&hdr_of(1, 1, ColourType::Grey, 8, true)).len(), 1);
		assert_eq!(passes_of(&hdr_of(3, 2, ColourType::Grey, 8, true)).len(), 4);
		assert_eq!(passes_of(&hdr_of(8, 8, ColourType::Grey, 8, true)).len(), 7);
		assert_eq!(passes_of(&hdr_of(9, 9, ColourType::Grey, 8, false)).len(), 1);
		Ok(())
	}

	#[test]
	fn test_the_expected_size_sums_the_passes_13() -> Outcome<()> {
		// A 64 by 64 greyscale image carries 64 filter bytes when it is not interlaced, and 112 --
		// the scanlines of all seven passes -- when it is. A ceiling computed from the
		// non-interlaced figure would refuse a sound interlaced file as too large.
		let plain = hdr_of(64, 64, ColourType::Grey, 8, false);
		let laced = hdr_of(64, 64, ColourType::Grey, 8, true);
		req!(res!(expected_size(&plain, &passes_of(&plain))), 64 * 65);
		req!(res!(expected_size(&laced, &passes_of(&laced))), 4216);

		// A sub-byte depth rounds each scanline up to a whole byte, so a 17-pixel row of 1-bit
		// greyscale is three bytes and not two and an eighth.
		let bits = hdr_of(17, 2, ColourType::Grey, 1, false);
		req!(res!(expected_size(&bits, &passes_of(&bits))), 2 * 4);

		// Sixteen bits double every scanline.
		let wide = hdr_of(5, 3, ColourType::Rgba, 16, false);
		req!(res!(expected_size(&wide, &passes_of(&wide))), 3 * (5 * 8 + 1));

		// And the filter's stride is the pixel in bytes, rounded up to one.
		req!(filter_bpp(ColourType::Grey, 1), 1);
		req!(filter_bpp(ColourType::Grey, 16), 2);
		req!(filter_bpp(ColourType::Rgb, 16), 6);
		req!(filter_bpp(ColourType::Rgba, 16), 8);
		req!(filter_bpp(ColourType::Palette, 4), 1);
		Ok(())
	}

	#[test]
	fn test_an_interlaced_stream_of_the_wrong_length_is_refused_14() -> Outcome<()> {
		// The image data must decompress to the sum over the passes, exactly. A stream cut short,
		// a stream run long, and a stream of exactly the size the image would have taken without
		// interlacing are all wrong, and the last is the one a decoder that got the arithmetic
		// wrong would accept.
		let full = 4216usize;
		for n in [full - 1, full + 1, 64 * 65] {
			let buf = assemble(&[
				(b"IHDR", ihdr_at(64, 64, 8, 0, 1)),
				(b"IDAT", res!(idat_of(&vec![0u8; n]))),
			]);
			assert!(decode(&buf).is_err(), "an interlaced stream of {} bytes must be refused", n);
		}

		// The right length decodes.
		let buf = assemble(&[
			(b"IHDR", ihdr_at(64, 64, 8, 0, 1)),
			(b"IDAT", res!(idat_of(&vec![0u8; full]))),
		]);
		let pm = res!(decode(&buf));
		req!(pm.width(), 64usize);
		req!(pm.height(), 64usize);
		Ok(())
	}

	#[test]
	fn test_a_bomb_cannot_grow_past_the_interlaced_ceiling_15() -> Outcome<()> {
		// Half a megabyte of zeroes deflates to a few hundred bytes. The header says the image is
		// 64 by 64, so the decoder must stop well short of inflating it, and refuse.
		let buf = assemble(&[
			(b"IHDR", ihdr_at(64, 64, 8, 0, 1)),
			(b"IDAT", res!(idat_of(&vec![0u8; 512 * 1024]))),
		]);
		assert!(decode(&buf).is_err(), "a stream far larger than the header allows must be refused");
		Ok(())
	}

	// ┌───────────────────────────────────────────────────────────────────────┐
	// │ SUB-BYTE DEPTHS                                                        │
	// └───────────────────────────────────────────────────────────────────────┘
	//
	// The two files below are written out byte by byte, and the pixels they are checked against
	// come from ImageMagick reading these exact bytes, not from this decoder.
	//
	// Pillow, which wrote the eight-bit fixtures further up, reads the second of them wrongly: it
	// reports every pixel opaque, because it compares the tRNS sample against the widened value
	// rather than the raw one. ImageMagick and the specification agree with the reading below.

	/// Colour type 0 at 1 bit, 3 by 1. The row's five padding bits are all set, and none of them
	/// may reach a pixel.
	const GREY1_PAD: [u8; 67] = [
		0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
		0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01,
		0x01, 0x00, 0x00, 0x00, 0x00, 0x33, 0x9B, 0x29, 0x19, 0x00, 0x00, 0x00,
		0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xD8, 0x0F, 0x00, 0x00,
		0xC1, 0x00, 0xC0, 0xD5, 0xE9, 0xCD, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x49,
		0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
	];

	/// Colour type 0 at 4 bits, 5 by 2, with a tRNS chunk naming the raw sample 5. Both rows carry
	/// a set padding nibble, and the two pixels of sample 5 widen to 85 and are transparent.
	const GREY4_TRNS: [u8; 87] = [
		0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
		0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x02,
		0x04, 0x00, 0x00, 0x00, 0x00, 0x70, 0xF1, 0xA4, 0x80, 0x00, 0x00, 0x00,
		0x02, 0x74, 0x52, 0x4E, 0x53, 0x00, 0x05, 0x06, 0xF9, 0x39, 0xB7, 0x00,
		0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0xFD,
		0x11, 0xCF, 0xF0, 0x21, 0xF8, 0x38, 0x00, 0x0C, 0x13, 0x03, 0x67, 0x9B,
		0x2A, 0x44, 0xD0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
		0x42, 0x60, 0x82,
	];

	#[test]
	fn test_the_padding_of_a_sub_byte_row_stays_out_of_the_pixels_16() -> Outcome<()> {
		let pm = res!(decode(&GREY1_PAD));
		req!(pm.width(), 3usize);
		req!(pm.height(), 1usize);
		let want = [Rgba::WHITE, Rgba::new(0, 0, 0, 255), Rgba::WHITE];
		for (x, exp) in want.iter().enumerate() {
			let got = match pm.pixel(x, 0) {
				Some(c) => c,
				None => return Err(err!("Pixel {},0 lies outside the pixmap.", x; Invalid, Input)),
			};
			assert_eq!(got, *exp, "pixel {},0 of a 1-bit row whose padding is all ones", x);
		}
		Ok(())
	}

	#[test]
	fn test_trns_compares_the_raw_sample_at_a_sub_byte_depth_17() -> Outcome<()> {
		let pm = res!(decode(&GREY4_TRNS));
		req!(pm.width(), 5usize);
		req!(pm.height(), 2usize);
		let want: [[(u8, u8); 5]; 2] = [
			// Sample, then the alpha ImageMagick reads. The sample 5 is the transparent one.
			[(0, 255),	(85, 0),	(255, 255),	(136, 255),	(85, 0)],
			[(255, 255),	(0, 255),	(85, 0),	(51, 255),	(204, 255)],
		];
		for (y, row) in want.iter().enumerate() {
			for (x, (g, a)) in row.iter().enumerate() {
				let got = match pm.pixel(x, y) {
					Some(c) => c,
					None => return Err(err!(
						"Pixel {},{} lies outside the pixmap.", x, y; Invalid, Input)),
				};
				assert_eq!(got, Rgba::new(*g, *g, *g, *a), "pixel {},{} of the 4-bit tRNS file", x, y);
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_narrow_sample_widens_to_the_full_range_18() {
		// The widest value a depth can hold must become 255, or a 1-bit image comes out black and
		// very-nearly-black rather than black and white.
		assert_eq!(widen(0, 1), 0);
		assert_eq!(widen(1, 1), 255);
		assert_eq!((0..4).map(|v| widen(v, 2)).collect::<Vec<_>>(), vec![0, 85, 170, 255]);
		assert_eq!(widen(0, 4), 0);
		assert_eq!(widen(15, 4), 255);
		assert_eq!(widen(200, 8), 200);
		// Sixteen bits keep the high byte, so an eight-bit value written twice survives exactly.
		for v in 0..=255u16 {
			assert_eq!(widen(v * 257, 16), v as u8, "the sample {} repeated", v);
		}
		assert_eq!(widen(0xFFFE, 16), 255);
	}

	#[test]
	fn test_a_sample_is_read_from_the_right_bits_19() -> Outcome<()> {
		// Sub-byte samples are packed most significant first.
		let line = [0b1101_0010u8, 0b0011_1000];
		req!(res!(sample_at(&line, 0, 1)), 1u16);
		req!(res!(sample_at(&line, 1, 1)), 1u16);
		req!(res!(sample_at(&line, 2, 1)), 0u16);
		req!(res!(sample_at(&line, 8, 1)), 0u16);
		req!(res!(sample_at(&line, 0, 2)), 0b11u16);
		req!(res!(sample_at(&line, 3, 2)), 0b10u16);
		req!(res!(sample_at(&line, 0, 4)), 0b1101u16);
		req!(res!(sample_at(&line, 1, 4)), 0b0010u16);
		req!(res!(sample_at(&line, 0, 8)), 0b1101_0010u16);
		req!(res!(sample_at(&line, 0, 16)), 0xD238u16);
		// And a sample past the end of the row is an error rather than a panic or a zero.
		assert!(sample_at(&line, 16, 1).is_err());
		assert!(sample_at(&line, 2, 8).is_err());
		assert!(sample_at(&line, 1, 16).is_err());
		Ok(())
	}

	#[test]
	fn test_the_header_refuses_what_it_cannot_read_20() -> Outcome<()> {
		// A bit depth outside the five the format defines.
		for depth in [0u8, 3, 5, 7, 9, 12, 32] {
			let buf = assemble(&[
				(b"IHDR", ihdr_at(1, 1, depth, 0, 0)),
				(b"IDAT", res!(idat_of(&[0, 0]))),
			]);
			assert!(decode(&buf).is_err(), "a bit depth of {} must be refused", depth);
		}

		// A depth the declared colour type does not allow: truecolour and the two alpha types
		// start at eight bits, and a palette index cannot be sixteen.
		for (ct, depth) in [(2u8, 4u8), (2, 1), (4, 2), (6, 4), (3, 16)] {
			let buf = assemble(&[
				(b"IHDR", ihdr_at(1, 1, depth, ct, 0)),
				(b"PLTE", vec![1, 2, 3]),
				(b"IDAT", res!(idat_of(&[0, 0, 0, 0, 0, 0, 0, 0, 0]))),
			]);
			assert!(decode(&buf).is_err(),
				"colour type {} at {} bits must be refused", ct, depth);
		}

		// A compression, filter or interlace method the format does not define. The header is
		// built by hand here because these three bytes are the ones `ihdr_at` fixes.
		for (at, v) in [(10usize, 1u8), (11, 1), (12, 2), (12, 255)] {
			let mut h = ihdr(1, 1, 0);
			h[at] = v;
			let buf = assemble(&[(b"IHDR", h), (b"IDAT", res!(idat_of(&[0, 0])))]);
			assert!(decode(&buf).is_err(),
				"byte {} of the header set to {} must be refused", at, v);
		}

		// The same header, untouched, decodes: it is the byte and not the file that is refused.
		let buf = assemble(&[(b"IHDR", ihdr(1, 1, 0)), (b"IDAT", res!(idat_of(&[0, 0])))]);
		assert!(decode(&buf).is_ok(), "a sound 1 by 1 greyscale file decodes");
		Ok(())
	}

	#[test]
	fn test_a_palette_index_beyond_the_palette_is_refused_at_every_depth_21() -> Outcome<()> {
		// A four-bit index of 3 against a palette of two entries names a colour that is not there.
		// Widening it into range, or reading past the palette, would paint something the file does
		// not hold.
		let buf = assemble(&[
			(b"IHDR", ihdr_at(2, 1, 4, 3, 0)),
			(b"PLTE", vec![255, 0, 0, 0, 255, 0]),
			(b"IDAT", res!(idat_of(&[0, 0x03]))),
		]);
		assert!(decode(&buf).is_err(), "a palette index of 3 against two entries must be refused");

		// The same row with both indices in range decodes.
		let buf = assemble(&[
			(b"IHDR", ihdr_at(2, 1, 4, 3, 0)),
			(b"PLTE", vec![255, 0, 0, 0, 255, 0]),
			(b"IDAT", res!(idat_of(&[0, 0x01]))),
		]);
		let pm = res!(decode(&buf));
		req!(pm.pixel(0, 0), Some(Rgba::opaque(255, 0, 0)));
		req!(pm.pixel(1, 0), Some(Rgba::opaque(0, 255, 0)));
		Ok(())
	}
}
