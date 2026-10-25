//! TIFF, page by page.
//!
//! A TIFF is a chain of directories, one a page, each a list of tagged values saying how the page's
//! pixels are cut into strips or tiles, how each is compressed and what its samples mean. What is
//! read here is what scanners, fax software and photograph tools write: uncompressed, PackBits,
//! LZW (with or without the horizontal predictor), Deflate, JPEG, and the three CCITT fax codings;
//! one to sixteen bits a sample; bilevel, greyscale, palette, RGB, CMYK, and JPEG's YCbCr; an alpha
//! channel associated or not; strips or tiles; chunky or planar samples; either fill order.
//!
//! A directory marked as a reduced-resolution copy of another -- the thumbnail a scanner puts
//! beside the page -- is not a page and is passed over.
//!
//! BigTIFF, floating-point samples, the old (pre-6.0) JPEG coding and the old LZW bit order are
//! refused by name.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	ccitt::{
		self,
		Coding,
	},
	jpeg,
	lzw::{
		self,
		BitOrder,
	},
	pixmap::Pixmap,
};

use oxedyne_fe2o3_core::prelude::*;

use std::io::Read;

use flate2::read::ZlibDecoder;

// The most pages read from one file, a ceiling against a directory chain that loops or an attack.
pub const MAX_PAGES: usize = 1000;

// Tags.
const NEW_SUBFILE:	u16 = 254;
const WIDTH:		u16 = 256;
const HEIGHT:		u16 = 257;
const BITS:		u16 = 258;
const COMPRESSION:	u16 = 259;
const PHOTOMETRIC:	u16 = 262;
const FILL_ORDER:	u16 = 266;
const STRIP_OFFSETS:	u16 = 273;
const SAMPLES:		u16 = 277;
const ROWS_PER_STRIP:	u16 = 278;
const STRIP_COUNTS:	u16 = 279;
const PLANAR:		u16 = 284;
const T4_OPTIONS:	u16 = 292;
const PREDICTOR:	u16 = 317;
const COLOUR_MAP:	u16 = 320;
const TILE_WIDTH:	u16 = 322;
const TILE_LENGTH:	u16 = 323;
const TILE_OFFSETS:	u16 = 324;
const TILE_COUNTS:	u16 = 325;
const INK_SET:		u16 = 332;
const EXTRA:		u16 = 338;
const SAMPLE_FORMAT:	u16 = 339;
const JPEG_TABLES:	u16 = 347;

/// The byte order a file declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
	Le,
	Be,
}

struct File<'a> {
	b:	&'a [u8],
	order:	Order,
}

impl<'a> File<'a> {

	fn u16(&self, at: usize) -> Outcome<u16> {
		match self.b.get(at..at + 2) {
			Some(s) => Ok(match self.order {
				Order::Le => u16::from_le_bytes([s[0], s[1]]),
				Order::Be => u16::from_be_bytes([s[0], s[1]]),
			}),
			None => Err(err!(
				"A TIFF needs two bytes at offset {}, but holds {}.", at, self.b.len();
			Invalid, Input, Decode)),
		}
	}

	fn u32(&self, at: usize) -> Outcome<u32> {
		match self.b.get(at..at + 4) {
			Some(s) => Ok(match self.order {
				Order::Le => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
				Order::Be => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
			}),
			None => Err(err!(
				"A TIFF needs four bytes at offset {}, but holds {}.", at, self.b.len();
			Invalid, Input, Decode)),
		}
	}
}

/// One directory's tags, each as the integers or the bytes it holds.
struct Ifd {
	tags:	Vec<(u16, Vec<u32>)>,
	tables:	Option<Vec<u8>>,	// JPEGTables, kept as bytes
	next:	usize,
}

impl Ifd {

	fn get(&self, tag: u16) -> Option<&[u32]> {
		self.tags.iter().find(|t| t.0 == tag).map(|t| &t.1[..])
	}

	fn one(&self, tag: u16, default: u32) -> u32 {
		match self.get(tag) {
			Some(v) if !v.is_empty() => v[0],
			_ => default,
		}
	}

	fn need(&self, tag: u16, name: &str) -> Outcome<&[u32]> {
		match self.get(tag) {
			Some(v) if !v.is_empty() => Ok(v),
			_ => Err(err!("A TIFF page has no {} tag ({}).", name, tag; Invalid, Input, Decode)),
		}
	}
}

fn read_ifd(f: &File, at: usize) -> Outcome<Ifd> {
	let n = res!(f.u16(at)) as usize;
	let mut tags = Vec::with_capacity(n);
	let mut tables = None;
	for i in 0..n {
		let e = at + 2 + i * 12;
		let tag = res!(f.u16(e));
		let kind = res!(f.u16(e + 2));
		let count = res!(f.u32(e + 4)) as usize;
		let size = match kind {
			1 | 2 | 6 | 7	=> 1,
			3 | 8		=> 2,
			4 | 9 | 11 | 13	=> 4,
			5 | 10 | 12	=> 8,
			_ => continue, // a type this does not know is a tag this does not need
		};
		let bytes = match count.checked_mul(size) {
			Some(b) if b <= f.b.len() => b,
			_ => return Err(err!(
				"A TIFF tag {} claims {} values, more than the file holds.", tag, count;
			Invalid, Input, Decode)),
		};
		let base = if bytes <= 4 { e + 8 } else { res!(f.u32(e + 8)) as usize };
		if tag == JPEG_TABLES {
			match f.b.get(base..base + bytes) {
				Some(s) => tables = Some(s.to_vec()),
				None => return Err(err!(
					"A TIFF's JPEG tables at offset {} pass the end of the file.", base;
				Invalid, Input, Decode)),
			}
			continue;
		}
		let mut v = Vec::with_capacity(count);
		for k in 0..count {
			let p = base + k * size;
			let x = match kind {
				1 | 2 | 6 | 7 => match f.b.get(p) {
					Some(b) => *b as u32,
					None => return Err(err!(
						"A TIFF tag {}'s values pass the end of the file.", tag;
					Invalid, Input, Decode)),
				},
				3 | 8 => res!(f.u16(p)) as u32,
				4 | 9 | 13 => res!(f.u32(p)),
				_ => res!(f.u32(p)), // a rational's numerator, or a float's bits; unused
			};
			v.push(x);
		}
		tags.push((tag, v));
	}
	let next = res!(f.u32(at + 2 + n * 12)) as usize;
	Ok(Ifd { tags, tables, next })
}

fn header(buf: &[u8]) -> Outcome<(File<'_>, usize)> {
	let order = match buf.get(..4) {
		Some(b"II*\0") => Order::Le,
		Some(b"MM\0*") => Order::Be,
		Some(b"II+\0") | Some(b"MM\0+") => return Err(err!(
			"The file is a BigTIFF, which this does not read."; Invalid, Input, Decode, Unimplemented)),
		_ => return Err(err!("The bytes do not begin with a TIFF header."; Invalid, Input, Decode)),
	};
	let f = File { b: buf, order };
	let first = res!(f.u32(4)) as usize;
	Ok((f, first))
}

/// Every page's directory, thumbnails passed over.
fn pages_of<'a>(buf: &'a [u8]) -> Outcome<(File<'a>, Vec<Ifd>)> {
	let (f, mut at) = res!(header(buf));
	let mut seen = Vec::new();
	let mut out = Vec::new();
	while at != 0 {
		if seen.contains(&at) || seen.len() >= MAX_PAGES {
			break;
		}
		seen.push(at);
		let ifd = res!(read_ifd(&f, at));
		at = ifd.next;
		if ifd.one(NEW_SUBFILE, 0) & 1 == 0 {
			out.push(ifd);
		}
	}
	if out.is_empty() {
		return Err(err!("A TIFF holds no page."; Invalid, Input, Decode));
	}
	Ok((f, out))
}

/// How many pages a TIFF holds, thumbnails not counted.
pub fn page_count(buf: &[u8]) -> Outcome<usize> {
	Ok(res!(pages_of(buf)).1.len())
}

/// The first page's width and height.
pub fn dimensions(buf: &[u8]) -> Outcome<(usize, usize)> {
	let (_, pages) = res!(pages_of(buf));
	let p = &pages[0];
	Ok((res!(p.need(WIDTH, "ImageWidth"))[0] as usize, res!(p.need(HEIGHT, "ImageLength"))[0] as usize))
}

/// Decodes the first page.
pub fn decode(buf: &[u8]) -> Outcome<Pixmap> {
	let (f, pages) = res!(pages_of(buf));
	page(&f, &pages[0])
}

/// Decodes every page, in order.
pub fn decode_pages(buf: &[u8]) -> Outcome<Vec<Pixmap>> {
	let (f, pages) = res!(pages_of(buf));
	let mut out = Vec::with_capacity(pages.len());
	for (i, p) in pages.iter().enumerate() {
		out.push(res!(page(&f, p).map_err(|e| err!(e,
			"Decoding page {} of {} of a TIFF.", i + 1, pages.len(); Decode))));
	}
	Ok(out)
}

/// PackBits: a signed count byte, then either that many plus one literal bytes or one byte
/// repeated one minus that many times.
fn packbits(data: &[u8], limit: usize) -> Outcome<Vec<u8>> {
	let mut out = Vec::with_capacity(limit);
	let mut i = 0;
	while i < data.len() && out.len() < limit {
		let n = data[i] as i8;
		i += 1;
		if n >= 0 {
			let k = n as usize + 1;
			let end = (i + k).min(data.len());
			out.extend_from_slice(&data[i..end]);
			i += k;
		} else if n != -128 {
			let k = 1 - n as isize;
			if let Some(b) = data.get(i) {
				for _ in 0..k {
					out.push(*b);
				}
			}
			i += 1;
		}
	}
	out.truncate(limit);
	Ok(out)
}

/// What one page's samples mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Photo {
	WhiteIsZero,
	BlackIsZero,
	Rgb,
	Palette,
	Cmyk,
	YCbCr,
}

struct Page<'a> {
	w:		usize,
	h:		usize,
	spp:		usize,		// samples a pixel
	bps:		usize,		// bits a sample
	photo:		Photo,
	planar:		bool,
	alpha:		Option<(usize, bool)>,	// which sample, and whether it is premultiplied
	map:		Option<&'a [u32]>,
	be:		bool,		// sixteen-bit samples are big-endian
}

fn page(f: &File, ifd: &Ifd) -> Outcome<Pixmap> {
	let w = res!(ifd.need(WIDTH, "ImageWidth"))[0] as usize;
	let h = res!(ifd.need(HEIGHT, "ImageLength"))[0] as usize;
	let spp = ifd.one(SAMPLES, 1) as usize;
	let bps = ifd.one(BITS, 1) as usize;
	if let Some(v) = ifd.get(BITS) {
		if v.iter().any(|b| *b as usize != bps) {
			return Err(err!(
				"A TIFF page mixes sample depths {:?}, which this does not read.", v;
			Invalid, Input, Decode, Unimplemented));
		}
	}
	if !(1..=16).contains(&bps) || spp == 0 || spp > 8 {
		return Err(err!(
			"A TIFF page of {} samples of {} bits is not one this reads.", spp, bps;
		Invalid, Input, Decode, Unimplemented));
	}
	if ifd.one(SAMPLE_FORMAT, 1) == 3 {
		return Err(err!(
			"A TIFF page of floating-point samples is not one this reads.";
		Invalid, Input, Decode, Unimplemented));
	}
	let comp = ifd.one(COMPRESSION, 1);
	let photo = match ifd.get(PHOTOMETRIC).and_then(|v| v.first().copied()) {
		Some(0) => Photo::WhiteIsZero,
		Some(1) => Photo::BlackIsZero,
		Some(2) => Photo::Rgb,
		Some(3) => Photo::Palette,
		Some(5) => {
			if ifd.one(INK_SET, 1) != 1 || spp < 4 {
				return Err(err!(
					"A separated TIFF page that is not CMYK is not one this reads.";
				Invalid, Input, Decode, Unimplemented));
			}
			Photo::Cmyk
		}
		Some(6) => Photo::YCbCr,
		Some(p) => return Err(err!(
			"A TIFF page's photometric interpretation {} is not one this reads.", p;
		Invalid, Input, Decode, Unimplemented)),
		// Missing, which the specification forbids; guess as most readers do.
		None => if matches!(comp, 2 | 3 | 4) { Photo::WhiteIsZero }
			else if spp >= 3 { Photo::Rgb } else { Photo::BlackIsZero },
	};
	if photo == Photo::YCbCr && comp != 7 {
		return Err(err!(
			"A TIFF page of YCbCr samples outside JPEG coding is not one this reads.";
		Invalid, Input, Decode, Unimplemented));
	}
	let base = match photo {
		Photo::Rgb | Photo::YCbCr	=> 3,
		Photo::Cmyk			=> 4,
		_				=> 1,
	};
	if spp < base {
		return Err(err!(
			"A TIFF page of {} samples a pixel is too few for its colour interpretation.", spp;
		Invalid, Input, Decode));
	}
	let alpha = match ifd.get(EXTRA) {
		Some(v) if spp > base => match v.first() {
			Some(1) => Some((base, true)),
			Some(2) => Some((base, false)),
			_ => None,
		},
		_ => None,
	};
	let map = match photo {
		Photo::Palette => {
			let m = res!(ifd.need(COLOUR_MAP, "ColorMap"));
			if m.len() < 3 << bps {
				return Err(err!(
					"A TIFF palette of {} values is short of the {} its depth needs.",
					m.len(), 3 << bps;
				Invalid, Input, Decode));
			}
			Some(m)
		}
		_ => None,
	};
	let pg = Page {
		w, h, spp, bps, photo,
		planar: ifd.one(PLANAR, 1) == 2 && spp > 1,
		alpha, map,
		be: f.order == Order::Be,
	};
	let mut pm = res!(Pixmap::new(w, h));

	// The chunks: strips are tiles as wide as the page.
	let tiled = ifd.get(TILE_OFFSETS).is_some();
	let (cw, ch, offs, counts) = if tiled {
		let tw = res!(ifd.need(TILE_WIDTH, "TileWidth"))[0] as usize;
		let tl = res!(ifd.need(TILE_LENGTH, "TileLength"))[0] as usize;
		(tw, tl, res!(ifd.need(TILE_OFFSETS, "TileOffsets")), ifd.get(TILE_COUNTS))
	} else {
		let rps = (ifd.one(ROWS_PER_STRIP, h as u32) as usize).clamp(1, h.max(1));
		(w, rps, res!(ifd.need(STRIP_OFFSETS, "StripOffsets")), ifd.get(STRIP_COUNTS))
	};
	if cw == 0 || ch == 0 {
		return Err(err!("A TIFF page's tiles are {} by {}.", cw, ch; Invalid, Input, Decode));
	}
	let across = (w + cw - 1) / cw;
	let down = (h + ch - 1) / ch;
	let planes = if pg.planar { spp } else { 1 };
	let per = across * down;
	if offs.len() < per * planes {
		return Err(err!(
			"A TIFF page of {} chunks lists only {} offsets.", per * planes, offs.len();
		Invalid, Input, Decode));
	}
	let fill_reversed = ifd.one(FILL_ORDER, 1) == 2;
	let predictor = ifd.one(PREDICTOR, 1);
	if predictor == 3 {
		return Err(err!(
			"A TIFF page uses the floating-point predictor, which this does not read.";
		Invalid, Input, Decode, Unimplemented));
	}
	let t4 = ifd.one(T4_OPTIONS, 0);
	// The planar case gathers every plane of a chunk before converting it.
	let chunk_spp = if pg.planar { 1 } else { spp };
	for cy in 0..down {
		for cx in 0..across {
			let x0 = cx * cw;
			let y0 = cy * ch;
			// A strip's last chunk is short; a tile is always whole.
			let rows = if tiled { ch } else { ch.min(h - y0) };
			let stride = (cw * chunk_spp * bps + 7) / 8;
			let size = stride * rows;
			let mut planes_px: Vec<Vec<u8>> = Vec::with_capacity(planes);
			for p in 0..planes {
				let k = p * per + cy * across + cx;
				let off = offs[k] as usize;
				let len = match counts {
					Some(c) if k < c.len() => c[k] as usize,
					_ => f.b.len().saturating_sub(off),
				};
				let end = off.saturating_add(len).min(f.b.len());
				let raw = match f.b.get(off..end) {
					Some(s) => s,
					None => return Err(err!(
						"A TIFF chunk at offset {} passes the end of the file at {}.", off, f.b.len();
					Invalid, Input, Decode)),
				};
				let owned;
				let raw = if fill_reversed {
					owned = raw.iter().map(|b| b.reverse_bits()).collect::<Vec<u8>>();
					&owned[..]
				} else {
					raw
				};
				if comp == 7 {
					// A JPEG chunk decodes straight to colour.
					let jp = res!(jpeg_chunk(raw, ifd.tables.as_deref()));
					blit(&mut pm, &jp, x0, y0);
					continue;
				}
				let mut px = match comp {
					1 => raw.to_vec(),
					32773 => res!(packbits(raw, size)),
					5 => {
						if raw.len() >= 2 && raw[0] == 0 && raw[1] & 1 == 1 {
							return Err(err!(
								"A TIFF chunk is in the pre-6.0 LZW bit order, which this does \
								not read."; Invalid, Input, Decode, Unimplemented));
						}
						res!(lzw::decode(raw, 8, BitOrder::Msb, true, size))
					}
					8 | 32946 => {
						let mut v = Vec::with_capacity(size);
						let mut z = ZlibDecoder::new(raw).take(size as u64);
						res!(z.read_to_end(&mut v).map_err(|e| err!(e,
							"A TIFF chunk's Deflate stream at offset {} is damaged.", off;
						Invalid, Input, Decode)));
						v
					}
					2 | 3 | 4 => {
						if bps != 1 || chunk_spp != 1 {
							return Err(err!(
								"A fax-coded TIFF page must be one bit of one sample, not {} of {}.",
								bps, chunk_spp; Invalid, Input, Decode));
						}
						let coding = match comp {
							2 => Coding::Mh,
							3 => Coding::T4 { two_d: t4 & 1 == 1, aligned: t4 & 4 == 4 },
							_ => Coding::T6,
						};
						res!(ccitt::decode(raw, cw, rows, coding).map_err(|e| err!(e,
							"Decoding the fax-coded TIFF chunk at offset {}.", off; Decode)))
					}
					6 => return Err(err!(
						"A TIFF page uses the pre-6.0 JPEG coding, which this does not read.";
					Invalid, Input, Decode, Unimplemented)),
					c => return Err(err!(
						"A TIFF page uses compression {}, which this does not read.", c;
					Invalid, Input, Decode, Unimplemented)),
				};
				px.resize(size, 0); // a short chunk reads as zeros rather than out of bounds
				if predictor == 2 {
					undo_predictor(&mut px, cw, rows, stride, chunk_spp, bps, pg.be);
				}
				planes_px.push(px);
			}
			if comp == 7 {
				continue;
			}
			res!(convert(&pg, &planes_px, stride, x0, y0, cw, rows, &mut pm));
		}
	}
	Ok(pm)
}

/// Joins a JPEG chunk to the page's shared tables and decodes it.
fn jpeg_chunk(raw: &[u8], tables: Option<&[u8]>) -> Outcome<Pixmap> {
	match tables {
		Some(t) if t.len() > 4 && raw.len() > 2 => {
			// The tables are a stream of their own, SOI to EOI; the chunk starts with its own SOI.
			let mut j = Vec::with_capacity(t.len() + raw.len());
			j.extend_from_slice(&t[..t.len() - 2]);
			j.extend_from_slice(&raw[2..]);
			jpeg::decode(&j)
		}
		_ => jpeg::decode(raw),
	}
}

fn blit(pm: &mut Pixmap, src: &Pixmap, x0: usize, y0: usize) {
	let (w, h) = (pm.width(), pm.height());
	let sw = src.width();
	let sd = src.data();
	let dd = pm.data_mut();
	for y in 0..src.height() {
		if y0 + y >= h {
			break;
		}
		for x in 0..sw {
			if x0 + x >= w {
				break;
			}
			let s = (y * sw + x) * 4;
			let d = ((y0 + y) * w + x0 + x) * 4;
			dd[d..d + 4].copy_from_slice(&sd[s..s + 4]);
		}
	}
}

/// Undoes horizontal differencing, sample by sample along each row.
fn undo_predictor(px: &mut [u8], w: usize, rows: usize, stride: usize, spp: usize, bps: usize, be: bool) {
	for r in 0..rows {
		let row = &mut px[r * stride..(r + 1) * stride];
		match bps {
			8 => {
				for i in spp..w * spp {
					row[i] = row[i].wrapping_add(row[i - spp]);
				}
			}
			16 => {
				let get = |row: &[u8], i: usize| -> u16 {
					if be { u16::from_be_bytes([row[2 * i], row[2 * i + 1]]) }
					else { u16::from_le_bytes([row[2 * i], row[2 * i + 1]]) }
				};
				for i in spp..w * spp {
					let v = get(row, i).wrapping_add(get(row, i - spp));
					let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
					row[2 * i] = b[0];
					row[2 * i + 1] = b[1];
				}
			}
			_ => (), // the predictor is defined for whole bytes only
		}
	}
}

/// Sample `i` of a row, scaled to eight bits.
fn sample(row: &[u8], i: usize, bps: usize, be: bool) -> u8 {
	match bps {
		8 => row[i],
		16 => if be { row[2 * i] } else { row[2 * i + 1] },
		_ => {
			let max = (1u32 << bps) - 1;
			let mut v = 0u32;
			let bit = i * bps;
			for k in 0..bps {
				let p = bit + k;
				v = (v << 1) | ((row[p >> 3] >> (7 - (p & 7))) & 1) as u32;
			}
			if bps < 8 {
				((v * 255 + max / 2) / max) as u8
			} else {
				(v >> (bps - 8)) as u8
			}
		}
	}
}

/// The palette index of sample `i`, unscaled.
fn index(row: &[u8], i: usize, bps: usize, be: bool) -> usize {
	match bps {
		8 => row[i] as usize,
		16 => (if be { u16::from_be_bytes([row[2 * i], row[2 * i + 1]]) }
			else { u16::from_le_bytes([row[2 * i], row[2 * i + 1]]) }) as usize,
		_ => {
			let mut v = 0usize;
			let bit = i * bps;
			for k in 0..bps {
				let p = bit + k;
				v = (v << 1) | ((row[p >> 3] >> (7 - (p & 7))) & 1) as usize;
			}
			v
		}
	}
}

/// Turns a chunk's samples into the page's pixels.
fn convert(pg: &Page, planes: &[Vec<u8>], stride: usize, x0: usize, y0: usize, cw: usize,
	rows: usize, pm: &mut Pixmap) -> Outcome<()>
{
	let w = pg.w;
	let n = 1usize << pg.bps;
	let data = pm.data_mut();
	let mut s = [0u8; 8];
	for r in 0..rows {
		let y = y0 + r;
		if y >= pg.h {
			break;
		}
		for x in 0..cw {
			if x0 + x >= w {
				break;
			}
			// The pixel's samples, from one chunk or one from each plane.
			let mut pal = 0usize;
			for k in 0..pg.spp {
				let (row, i) = if pg.planar {
					(&planes[k][r * stride..(r + 1) * stride], x)
				} else {
					(&planes[0][r * stride..(r + 1) * stride], x * pg.spp + k)
				};
				if k == 0 && pg.photo == Photo::Palette {
					pal = index(row, i, pg.bps, pg.be);
				}
				s[k] = sample(row, i, pg.bps, pg.be);
			}
			let mut c = match pg.photo {
				Photo::WhiteIsZero	=> [255 - s[0], 255 - s[0], 255 - s[0], 255],
				Photo::BlackIsZero	=> [s[0], s[0], s[0], 255],
				Photo::Rgb | Photo::YCbCr	=> [s[0], s[1], s[2], 255],
				Photo::Palette => match pg.map {
					Some(m) if pal < n => [
						(m[pal] >> 8) as u8,
						(m[n + pal] >> 8) as u8,
						(m[2 * n + pal] >> 8) as u8,
						255,
					],
					_ => [0, 0, 0, 255],
				},
				Photo::Cmyk => {
					let k = 255 - s[3] as u32;
					let ink = |v: u8| ((255 - v as u32) * k / 255) as u8;
					[ink(s[0]), ink(s[1]), ink(s[2]), 255]
				}
			};
			if let Some((a, pre)) = pg.alpha {
				let al = s[a];
				c[3] = al;
				if pre && al > 0 && al < 255 {
					for v in c.iter_mut().take(3) {
						*v = ((*v as u32 * 255 + al as u32 / 2) / al as u32).min(255) as u8;
					}
				}
			}
			let d = (y * w + x0 + x) * 4;
			data[d..d + 4].copy_from_slice(&c);
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_packbits_literal_and_repeat_runs_00() -> Outcome<()> {
		// The example in the TIFF 6.0 specification.
		let packed = [
			0xfe, 0xaa, 0x02, 0x80, 0x00, 0x2a, 0xfd, 0xaa, 0x03, 0x80, 0x00, 0x2a, 0x22, 0xf7, 0xaa,
		];
		let want = [
			0xaa, 0xaa, 0xaa, 0x80, 0x00, 0x2a, 0xaa, 0xaa, 0xaa, 0xaa, 0x80, 0x00, 0x2a, 0x22,
			0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
		];
		assert_eq!(res!(packbits(&packed, 64)), want.to_vec());
		Ok(())
	}

	#[test]
	fn test_a_bigtiff_is_refused_by_name_00() {
		let got = decode(b"II+\0\x08\0\0\0");
		assert!(matches!(&got, Err(e) if fmt!("{}", e).contains("BigTIFF")));
	}
}
