//! A JPEG codec.
//!
//! JPEG is a stream of marker segments -- tables, a frame header, then one or more scans of
//! entropy-coded data. Nothing in it is a general-purpose compressor that could sensibly be
//! borrowed, so the whole of it is owned here: the Huffman decoder, the inverse DCT, the chroma
//! upsampler and the colour transform alike.
//!
//! Owning the decoder is also a security position, for the reasons `crate::png` gives. This one
//! runs in a crate that forbids `unsafe`, bounds-checks every table index it is handed, and refuses
//! a frame header whose dimensions exceed [`MAX_PIXELS`] before it allocates anything.
//!
//! # What is supported
//!
//! Decoding: baseline sequential (SOF0), extended sequential with Huffman coding (SOF1), and
//! progressive (SOF2), at eight bits a sample. Greyscale, YCbCr and RGB three-component images, and
//! four-component CMYK and YCCK. Any sampling factors, with 4:4:4, 4:2:2 and 4:2:0 taking the same
//! triangle-filter upsampling libjpeg uses by default. Restart intervals, and scans that are
//! interleaved or not.
//!
//! Encoding: baseline sequential, at a quality that maps onto the Annex K quantisation tables the
//! same way libjpeg's does, with 4:4:4, 4:2:2 or 4:2:0 chroma and a greyscale mode.
//!
//! Arithmetic coding, lossless and hierarchical modes, and twelve-bit samples are refused by name
//! rather than misread.
//!
//! A progressive file whose later scans never arrived takes the Annex K.8 block smoothing libjpeg
//! applies by default, which estimates the lowest few AC coefficients of each block from the mean of
//! its neighbours rather than showing the flat squares of a half-loaded photograph.
//!
//! # Damaged files
//!
//! A photograph library holds files that were truncated by a failed copy or a full disk, and a
//! decoder that refuses them shows nothing where it could have shown most of the picture. Where the
//! entropy-coded data runs out, the rest of the image is left flat mid-grey and what did arrive is
//! returned. A malformed *header* is still an error, because there is then no picture to show.
//!
//! # Agreement with other decoders
//!
//! The inverse DCT is the integer one from the specification's informative annex, in the arrangement
//! libjpeg calls `islow`, at the same fixed-point precision and with the same rounding. The colour
//! transform and the chroma upsampler are likewise the fixed-point forms libjpeg uses. Two decoders
//! of the same file are not obliged to agree to the last bit, but these choices mean this one does.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::pixmap::{
	Pixmap,
	MAX_PIXELS,
};

use oxedyne_fe2o3_core::prelude::*;

use std::num::Wrapping;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MARKERS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

const SOI:	u8 = 0xD8;			// start of image
const EOI:	u8 = 0xD9;			// end of image
const SOS:	u8 = 0xDA;			// start of scan
const DQT:	u8 = 0xDB;			// define quantisation tables
const DNL:	u8 = 0xDC;			// define number of lines
const DRI:	u8 = 0xDD;			// define restart interval
const DHT:	u8 = 0xC4;			// define Huffman tables
const DAC:	u8 = 0xCC;			// define arithmetic coding conditioning
const RST0:	u8 = 0xD0;			// the first restart marker; there are eight, consecutive
const RST7:	u8 = 0xD7;			// the last
const APP0:	u8 = 0xE0;			// the first application segment; there are sixteen
const APP15:	u8 = 0xEF;		// the last
const ICC_APP2:	u8 = 0xE2;		// where an ICC colour profile travels
const ADOBE_APP14: u8 = 0xEE;	// where Adobe declares a colour transform
const COM:	u8 = 0xFE;			// a comment
const TEM:	u8 = 0x01;			// arithmetic coding only, and carries no length

const DCTSIZE: usize = 8;	// the side of a DCT block, in samples
const DCTSIZE2: usize = 64;	// and its coefficient count

// The natural (row-major) position each zigzag position maps to.
const NATURAL: [usize; DCTSIZE2] = [
	 0,  1,  8, 16,  9,  2,  3, 10,
	17, 24, 32, 25, 18, 11,  4,  5,
	12, 19, 26, 33, 40, 48, 41, 34,
	27, 20, 13,  6,  7, 14, 21, 28,
	35, 42, 49, 56, 57, 50, 43, 36,
	29, 22, 15, 23, 30, 37, 44, 51,
	58, 59, 52, 45, 38, 31, 39, 46,
	53, 60, 61, 54, 47, 55, 62, 63,
];

const LOOKAHEAD: usize = 8; // bits of a Huffman code the lookahead table resolves in one step

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HUFFMAN TABLES                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// A Huffman table, derived from the counts and values a DHT segment carries.
#[derive(Clone, Debug)]
struct Huff {
	maxcode:	[i32; 18],		// largest code of each length, -1 where there are none
	mincode:	[i32; 17],		// and the smallest
	valptr:		[usize; 17],	// where each length's values begin in vals
	vals:		Vec<u8>,		// the symbols, in canonical code order
	look:		Vec<(u8, u8)>,	// by the next LOOKAHEAD bits: code length and symbol, 0 if none
}

impl Huff {

	/// Derives a table from a count of codes at each length 1 to 16, and the symbols they name.
	fn new(counts: &[u8; 17], vals: Vec<u8>) -> Outcome<Self> {
		// The code length of each symbol, in order.
		let mut sizes: Vec<u8> = Vec::with_capacity(vals.len());
		for l in 1..=16usize {
			for _ in 0..counts[l] {
				sizes.push(l as u8);
			}
		}
		if sizes.len() != vals.len() {
			return Err(err!(
				"A Huffman table declares {} codes across its sixteen lengths but carries {} \
				symbols.", sizes.len(), vals.len();
			Invalid, Input, Decode, Mismatch));
		}
		if sizes.is_empty() {
			return Err(err!("A Huffman table carries no codes."; Invalid, Input, Decode, Missing));
		}

		// The canonical codes.
		let mut codes: Vec<u32> = vec![0; sizes.len()];
		let mut code = 0u32;
		let mut si = sizes[0];
		let mut k = 0usize;
		while k < sizes.len() {
			while k < sizes.len() && sizes[k] == si {
				codes[k] = code;
				code = code.wrapping_add(1);
				k += 1;
			}
			// A binary tree of depth `si` holds two to the `si` leaves, and a table that names more
			// than that has assigned a code that is the prefix of another.
			if code > (1u32 << si) {
				return Err(err!(
					"A Huffman table declares more codes of length {} than that length holds, so it \
					is not a prefix code.", si;
				Invalid, Input, Decode));
			}
			code <<= 1;
			si += 1;
			if si > 16 {
				break;
			}
		}

		let mut maxcode = [-1i32; 18];
		let mut mincode = [0i32; 17];
		let mut valptr = [0usize; 17];
		let mut p = 0usize;
		for l in 1..=16usize {
			if counts[l] > 0 {
				valptr[l] = p;
				mincode[l] = codes[p] as i32;
				p += counts[l] as usize;
				maxcode[l] = codes[p - 1] as i32;
			} else {
				maxcode[l] = -1;
			}
		}
		maxcode[17] = 0x000F_FFFF; // A sentinel, so the slow path always terminates.

		// The lookahead, filled for every code no longer than LOOKAHEAD bits.
		let mut look = vec![(0u8, 0u8); 1 << LOOKAHEAD];
		for (i, sz) in sizes.iter().enumerate() {
			let l = *sz as usize;
			if l > LOOKAHEAD {
				break;
			}
			let lo = (codes[i] as usize) << (LOOKAHEAD - l);
			let hi = lo + (1usize << (LOOKAHEAD - l));
			for e in look.iter_mut().take(hi.min(1 << LOOKAHEAD)).skip(lo) {
				*e = (l as u8, vals[i]);
			}
		}

		Ok(Self { maxcode, mincode, valptr, vals, look })
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE ENTROPY-CODED BIT STREAM                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// A reader of the bits in an entropy-coded segment.
///
/// A 0xFF byte inside entropy-coded data is written as 0xFF 0x00, so the reader unstuffs as it goes.
/// A 0xFF followed by anything else is a marker, which ends the segment: the reader then pads with
/// zero bits rather than reading past it, which is what a decoder must do with a truncated file if
/// it is to show the part that did arrive.
struct Bits<'a> {
	buf:	&'a [u8],	// the whole file
	pos:	usize,		// the next byte to read
	acc:	u32,		// the bit buffer; its lowest cnt bits are unconsumed
	cnt:	u32,		// how many bits of acc are valid
	hit:	bool,		// a marker or the end was met, so the reader now pads
	pad:	u32,		// how many of the cnt bits are padding rather than data
}

impl<'a> Bits<'a> {

	fn new(buf: &'a [u8], pos: usize) -> Self {
		Self { buf, pos, acc: 0, cnt: 0, hit: false, pad: 0 }
	}

	/// The next byte of entropy-coded data, unstuffed, or `None` once a marker has been met.
	fn byte(&mut self) -> Option<u8> {
		if self.hit {
			return None;
		}
		if self.pos >= self.buf.len() {
			self.hit = true;
			return None;
		}
		let b = self.buf[self.pos];
		if b != 0xFF {
			self.pos += 1;
			return Some(b);
		}
		// A 0xFF is a stuffed literal, padding before a marker, or the marker itself.
		let mut k = self.pos + 1;
		while k < self.buf.len() && self.buf[k] == 0xFF {
			k += 1;
		}
		if k < self.buf.len() && self.buf[k] == 0x00 {
			self.pos = k + 1;
			return Some(0xFF);
		}
		self.hit = true;
		None
	}

	/// Tops the bit buffer up to at least 25 bits, padding with zeros past the end of the data.
	fn fill(&mut self) {
		while self.cnt <= 24 {
			match self.byte() {
				Some(b) => self.acc = (self.acc << 8) | (b as u32),
				None => {
					self.acc <<= 8;
					self.pad += 8;
				},
			}
			self.cnt += 8;
		}
	}

	/// Whether every bit left is padding, so the entropy-coded data has genuinely run out.
	///
	/// This is not the same as having met the marker that ends the segment: the buffer reads ahead,
	/// so the marker is normally in hand while several real bits are still to be spent.
	fn starved(&mut self) -> bool {
		self.fill();
		self.hit && self.pad >= self.cnt
	}

	fn bit(&mut self) -> u32 {
		if self.cnt == 0 {
			self.fill();
		}
		self.cnt -= 1;
		self.pad = self.pad.min(self.cnt);
		(self.acc >> self.cnt) & 1
	}

	/// The next `n` bits, as an unsigned integer, where `n` is at most 16.
	fn receive(&mut self, n: u32) -> u32 {
		if n == 0 {
			return 0;
		}
		if self.cnt < n {
			self.fill();
		}
		self.cnt -= n;
		self.pad = self.pad.min(self.cnt);
		(self.acc >> self.cnt) & ((1u32 << n) - 1)
	}

	fn huff(&mut self, t: &Huff) -> Outcome<u8> {
		self.fill();
		if self.cnt >= LOOKAHEAD as u32 {
			let peek = ((self.acc >> (self.cnt - LOOKAHEAD as u32)) & 0xFF) as usize;
			let (l, v) = t.look[peek];
			if l != 0 {
				self.cnt -= l as u32;
				self.pad = self.pad.min(self.cnt);
				return Ok(v);
			}
		}
		let mut code = 0i32;
		for l in 1..=16usize {
			code = (code << 1) | (self.bit() as i32);
			if code <= t.maxcode[l] {
				let idx = t.valptr[l] + ((code - t.mincode[l]) as usize);
				match t.vals.get(idx) {
					Some(v) => return Ok(*v),
					None => break,
				}
			}
		}
		if self.hit {
			// The data ran out; libjpeg's answer here is a zero, and so is ours, so that a truncated
			// file still shows the part of the image that arrived.
			return Ok(0);
		}
		Err(err!(
			"No Huffman code of any length from 1 to 16 matches the bits at offset {} of the \
			entropy-coded data.", self.pos;
		Invalid, Input, Decode))
	}

	/// Steps over the restart marker that ends a restart interval.
	///
	/// Whatever bits are left in the interval are discarded, and any padding an encoder left before
	/// the marker is walked over rather than read as data.
	fn restart(&mut self) {
		self.acc = 0;
		self.cnt = 0;
		self.pad = 0;
		let n = self.buf.len();
		let mut k = self.pos;
		while k + 1 < n {
			if self.buf[k] == 0xFF && self.buf[k + 1] != 0x00 && self.buf[k + 1] != 0xFF {
				break;
			}
			k += 1;
		}
		if k + 1 < n {
			let m = self.buf[k + 1];
			if (RST0..=RST7).contains(&m) {
				self.pos = k + 2;
				self.hit = false;
				return;
			}
		}
		// Whatever is there is not a restart marker, so the scan's data ends at it.
		self.pos = k.min(n);
		self.hit = true;
	}
}

/// Sign-extends a value of `n` bits read out of the stream, as the specification's EXTEND does.
fn extend(v: u32, n: u32) -> i32 {
	if n == 0 {
		return 0;
	}
	let v = v as i32;
	if v < (1 << (n - 1)) {
		v - (1 << n) + 1
	} else {
		v
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE INVERSE DCT                                                            │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The integer inverse DCT of the specification's informative annex, in the arrangement libjpeg calls
// `islow`: a row-column decomposition of the AAN even-odd factorisation, at thirteen fractional
// bits, carrying two extra bits between the passes. The arithmetic wraps rather than panicking,
// because a file whose coefficients are large enough to overflow is a file to be read the way every
// other decoder reads it, not one to abort on.

/// An integer that wraps rather than trapping, so a hostile coefficient cannot panic the decoder.
type W = Wrapping<i32>;

const fn w(v: i32) -> W {
	Wrapping(v)
}

const CONST_BITS: usize = 13;	// fractional bits the constants below carry
const PASS1_BITS: usize = 2;	// extra fractional bits carried between the two passes

//// The rotation constants of the even and odd parts, named for the value each stands for and
//// held at CONST_BITS fractional bits. The first three belong to the even part.
const FIX_0_541196100: W = w(4433);
const FIX_0_765366865: W = w(6270);
const FIX_1_847759065: W = w(15137);
const FIX_0_298631336: W = w(2446);
const FIX_2_053119869: W = w(16819);
const FIX_3_072711026: W = w(25172);
const FIX_1_501321110: W = w(12299);
const FIX_0_899976223: W = w(7373);
const FIX_2_562915447: W = w(20995);
const FIX_1_961570560: W = w(16069);
const FIX_0_390180644: W = w(3196);
const FIX_1_175875602: W = w(9633);

/// Rounds a fixed-point value down by `n` bits, to nearest.
fn descale(x: W, n: usize) -> W {
	(x + w(1 << (n - 1))) >> n
}

fn clamp8(v: i32) -> u8 {
	if v < 0 {
		0
	} else if v > 255 {
		255
	} else {
		v as u8
	}
}

/// The odd part of one pass, shared by both: four coefficients in, four butterfly terms out.
fn odd_part(t0: W, t1: W, t2: W, t3: W) -> (W, W, W, W) {
	let z1 = t0 + t3;
	let z2 = t1 + t2;
	let z3 = t0 + t2;
	let z4 = t1 + t3;
	let z5 = (z3 + z4) * FIX_1_175875602;

	let t0 = t0 * FIX_0_298631336;
	let t1 = t1 * FIX_2_053119869;
	let t2 = t2 * FIX_3_072711026;
	let t3 = t3 * FIX_1_501321110;
	let z1 = z1 * -FIX_0_899976223;
	let z2 = z2 * -FIX_2_562915447;
	let z3 = z3 * -FIX_1_961570560 + z5;
	let z4 = z4 * -FIX_0_390180644 + z5;

	(t0 + z1 + z3, t1 + z2 + z4, t2 + z2 + z3, t3 + z1 + z4)
}

/// The even part of one pass: four coefficients in, four butterfly terms out.
fn even_part(c0: W, c2: W, c4: W, c6: W) -> (W, W, W, W) {
	let z1 = (c2 + c6) * FIX_0_541196100;
	let t2 = z1 + c6 * -FIX_1_847759065;
	let t3 = z1 + c2 * FIX_0_765366865;
	let t0 = (c0 + c4) << CONST_BITS;
	let t1 = (c0 - c4) << CONST_BITS;
	(t0 + t3, t1 + t2, t1 - t2, t0 - t3)
}

/// Inverts the DCT of one block, dequantising on the way in, and writes eight rows of eight samples.
///
/// The coefficients are in natural order, as is the quantisation table, and the samples land at
/// `out[at + y * stride + x]`.
fn idct(coef: &[i16], q: &[u16; DCTSIZE2], out: &mut [u8], at: usize, stride: usize) {
	let mut ws = [w(0); DCTSIZE2];

	// Pass one, over the columns.
	for c in 0..DCTSIZE {
		let ac_zero = (1..DCTSIZE).all(|r| coef[r * DCTSIZE + c] == 0);
		if ac_zero {
			let dc = w((coef[c] as i32) * (q[c] as i32)) << PASS1_BITS;
			for r in 0..DCTSIZE {
				ws[r * DCTSIZE + c] = dc;
			}
			continue;
		}
		let d = |r: usize| w((coef[r * DCTSIZE + c] as i32) * (q[r * DCTSIZE + c] as i32));
		let (t10, t11, t12, t13) = even_part(d(0), d(2), d(4), d(6));
		let (o0, o1, o2, o3) = odd_part(d(7), d(5), d(3), d(1));
		let s = CONST_BITS - PASS1_BITS;
		ws[c] = descale(t10 + o3, s);
		ws[7 * DCTSIZE + c] = descale(t10 - o3, s);
		ws[DCTSIZE + c] = descale(t11 + o2, s);
		ws[6 * DCTSIZE + c] = descale(t11 - o2, s);
		ws[2 * DCTSIZE + c] = descale(t12 + o1, s);
		ws[5 * DCTSIZE + c] = descale(t12 - o1, s);
		ws[3 * DCTSIZE + c] = descale(t13 + o0, s);
		ws[4 * DCTSIZE + c] = descale(t13 - o0, s);
	}

	// Pass two, over the rows, with the level shift folded into the clamp.
	let s = CONST_BITS + PASS1_BITS + 3;
	for r in 0..DCTSIZE {
		let v = &ws[r * DCTSIZE..r * DCTSIZE + DCTSIZE];
		let (t10, t11, t12, t13) = even_part(v[0], v[2], v[4], v[6]);
		let (o0, o1, o2, o3) = odd_part(v[7], v[5], v[3], v[1]);
		let row = at + r * stride;
		let put = |out: &mut [u8], i: usize, x: W| {
			out[row + i] = clamp8(descale(x, s).0 + 128);
		};
		put(out, 0, t10 + o3);
		put(out, 7, t10 - o3);
		put(out, 1, t11 + o2);
		put(out, 6, t11 - o2);
		put(out, 2, t12 + o1);
		put(out, 5, t12 - o1);
		put(out, 3, t13 + o0);
		put(out, 4, t13 - o0);
	}
}

/// The one sample a block reduces to when only its DC coefficient is read.
fn idct_dc(coef: &[i16], q: &[u16; DCTSIZE2]) -> u8 {
	let dc = w((coef[0] as i32) * (q[0] as i32));
	clamp8(descale(dc, 3).0 + 128)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COLOUR                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// The fixed-point tables the YCbCr to RGB transform uses, at sixteen fractional bits.
struct YccTab {
	cr_r:	[i32; 256],		// the red contribution of Cr, already descaled
	cb_b:	[i32; 256],		// the blue contribution of Cb, already descaled
	cr_g:	[i32; 256],		// the green contribution of Cr, still scaled
	cb_g:	[i32; 256],		// the same for Cb, carrying the rounding term
}

impl YccTab {

	fn new() -> Self {
		let half = 1i32 << 15;
		let mut t = Self {
			cr_r: [0; 256],
			cb_b: [0; 256],
			cr_g: [0; 256],
			cb_g: [0; 256],
		};
		for i in 0..256 {
			let x = (i as i32) - 128;
			t.cr_r[i] = (91881 * x + half) >> 16; // 1.40200
			t.cb_b[i] = (116130 * x + half) >> 16; // 1.77200
			t.cr_g[i] = -46802 * x; // -0.71414
			t.cb_g[i] = -22554 * x + half; // -0.34414
		}
		t
	}

	/// One pixel, from luminance and the two chrominances.
	fn rgb(&self, y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
		let (y, cb, cr) = (y as i32, cb as usize, cr as usize);
		(
			clamp8(y + self.cr_r[cr]),
			clamp8(y + ((self.cb_g[cb] + self.cr_g[cr]) >> 16)),
			clamp8(y + self.cb_b[cb]),
		)
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE FRAME                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// How the samples of a frame's components are to be read as colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
	Grey,	// one component: luminance
	Ycc,	// three: luminance and two chrominances
	Rgb,	// three, already red, green and blue
	Cmyk,	// four: cyan, magenta, yellow, black, inverted where Adobe says
	Ycck,	// four: a YCbCr encoding of inverted CMY, then black
}

/// One component of a frame.
#[derive(Clone, Debug)]
struct Comp {
	id:	u8,				// the identifier a scan header names it by
	h:	usize,			// horizontal sampling factor
	v:	usize,			// vertical sampling factor
	tq:	usize,			// which of the four quantisation table slots it uses
	dw:	usize,			// width in samples, before upsampling
	dh:	usize,			// and height
	bw:	usize,			// width in blocks, of the samples that carry image
	bh:	usize,			// and height
	bwp:	usize,		// width in blocks of the allocation, which the MCU grid rounds up
	bhp:	usize,		// and height
	coef:	Vec<i16>,	// natural order within a block, row-major across blocks
}

impl Comp {

	/// Where a block's coefficients begin.
	fn at(&self, bx: usize, by: usize) -> usize {
		(by * self.bwp + bx) * DCTSIZE2
	}
}

/// A frame, and everything the scans within it have filled in.
struct Frame {
	prog:	bool,					// do the coefficients arrive over several scans, each refining?
	w:	usize,						// width in pixels
	h:	usize,						// height in pixels
	hmax:	usize,					// the largest horizontal sampling factor across the components
	vmax:	usize,					// and vertical
	mcux:	usize,					// MCUs across
	mcuy:	usize,					// and down
	comps:	Vec<Comp>,				// in the order the frame header gives them
	// Per component, the successive-approximation bit at which each coefficient was last
	// received, or -1 for a coefficient no scan ever carried.
	seen:	Vec<[i8; DCTSIZE2]>,
}

fn ceil_div(a: usize, b: usize) -> usize {
	if b == 0 {
		0
	} else {
		a.div_ceil(b)
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PARSING                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The tables and frame a file has declared so far.
struct Reader {
	quant:	[Option<[u16; DCTSIZE2]>; 4],	// the four slots, in natural order
	dc:	[Option<Huff>; 4],					// the four DC Huffman table slots
	ac:	[Option<Huff>; 4],					// and the four AC ones
	ri:	usize,								// the restart interval in MCUs, or zero for none
	jfif:	bool,							// a JFIF APP0 alone makes a three-component frame YCbCr
	adobe:	Option<u8>,						// the colour transform an Adobe APP14 declared
	frame:	Option<Frame>,					// the frame, once its header has been read
}

/// Removes the metadata a JPEG carries, without touching the image itself.
///
/// A photograph off a phone arrives with an Exif block naming the camera, the moment and, very
/// often, the coordinates of whoever pressed the shutter. Publishing the file publishes all of it.
/// This walks the marker segments and drops the ones that describe the picture rather than encode
/// it, leaving the entropy-coded scan untouched: the result decodes to exactly the same pixels, so
/// nothing is re-compressed and no quality is lost.
///
/// Removed: every application segment except those below, and every comment segment. That takes
/// Exif and XMP (APP1), Photoshop and IPTC (APP13), maker notes, and any JUMBF or C2PA assertion
/// (APP11) with them -- so a caller relying on embedded provenance must read it before calling.
///
/// Kept, because dropping them changes how the file decodes or renders: JFIF (APP0), an ICC colour
/// profile (APP2), and the Adobe colour transform (APP14).
pub fn strip_metadata(buf: &[u8]) -> Outcome<Vec<u8>> {
	if buf.len() < 2 || buf[0] != 0xFF || buf[1] != SOI {
		return Err(err!("The data does not begin with a JPEG start-of-image marker."; 
			Invalid, Input, Decode));
	}
	let mut out = Vec::with_capacity(buf.len());
	out.extend_from_slice(&buf[..2]);

	let mut pos = 2;
	loop {
		let (marker, after) = res!(next_marker(buf, pos));
		// The scan and everything after it is image data; copy the remainder untouched.
		if marker == SOS {
			out.extend_from_slice(&buf[pos..]);
			return Ok(out);
		}
		// Markers that stand alone carry no length.
		if marker == EOI || marker == TEM || (RST0..=RST7).contains(&marker) {
			out.extend_from_slice(&buf[pos..after]);
			if marker == EOI {
				return Ok(out);
			}
			pos = after;
			continue;
		}
		let (_start, end) = res!(segment(buf, after, marker));
		let drop = match marker {
			APP0 | ADOBE_APP14	=> false,	// JFIF and the Adobe colour transform.
			ICC_APP2			=> false,	// A colour profile is how the pixels are read.
			COM					=> true,
			m if (APP0..=APP15).contains(&m) => true,
			_					=> false,
		};
		if !drop {
			out.extend_from_slice(&buf[pos..end]);
		}
		pos = end;
	}
}

/// The two-byte length a marker segment begins with, and the bounds of its payload.
fn segment(buf: &[u8], pos: usize, marker: u8) -> Outcome<(usize, usize)> {
	if pos + 2 > buf.len() {
		return Err(err!(
			"The marker {:#04X} at offset {} has no length, only {} bytes remain.",
			marker, pos, buf.len() - pos;
		Invalid, Input, Decode));
	}
	let len = ((buf[pos] as usize) << 8) | (buf[pos + 1] as usize);
	if len < 2 {
		return Err(err!(
			"The marker {:#04X} at offset {} declares a segment length of {}, and a length counts \
			its own two bytes.", marker, pos, len;
		Invalid, Input, Decode));
	}
	let end = pos + len;
	if end > buf.len() {
		return Err(err!(
			"The marker {:#04X} at offset {} declares {} bytes, but only {} remain.",
			marker, pos, len, buf.len() - pos;
		Invalid, Input, Decode));
	}
	Ok((pos + 2, end))
}

/// Finds the next marker at or after an offset, stepping over any padding.
fn next_marker(buf: &[u8], pos: usize) -> Outcome<(u8, usize)> {
	let mut k = pos;
	while k < buf.len() && buf[k] != 0xFF {
		k += 1;
	}
	while k < buf.len() && buf[k] == 0xFF {
		k += 1;
	}
	if k >= buf.len() {
		return Err(err!(
			"The file ends at offset {} without a further marker.", buf.len();
		Invalid, Input, Decode, Missing));
	}
	Ok((buf[k], k + 1))
}

/// Reads a frame header, and refuses what this codec does not implement, by name.
///
/// The coefficient buffers are allocated only when `alloc` is set, so that a caller after the size
/// alone pays for nothing else.
fn read_frame(data: &[u8], marker: u8, at: usize, alloc: bool) -> Outcome<Frame> {
	let kind = match marker {
		0xC0	=> "baseline sequential",
		0xC1	=> "extended sequential",
		0xC2	=> "progressive",
		0xC3	=> return Err(err!(
			"The frame at offset {} is lossless (SOF3). This codec implements the DCT modes.", at;
		Invalid, Input, Decode, NoImpl)),
		0xC5..=0xC7 => return Err(err!(
			"The frame at offset {} is differential (SOF{}), part of a hierarchical image. This \
			codec implements the non-hierarchical modes.", at, marker - 0xC0;
		Invalid, Input, Decode, NoImpl)),
		0xC9..=0xCB => return Err(err!(
			"The frame at offset {} is arithmetic coded (SOF{}). This codec implements Huffman \
			coding.", at, marker - 0xC0;
		Invalid, Input, Decode, NoImpl)),
		0xCD..=0xCF => return Err(err!(
			"The frame at offset {} is differential and arithmetic coded (SOF{}). This codec \
			implements neither.", at, marker - 0xC0;
		Invalid, Input, Decode, NoImpl)),
		_ => return Err(err!(
			"The marker {:#04X} at offset {} is not a frame header this codec knows.", marker, at;
		Invalid, Input, Decode, NoImpl)),
	};
	if data.len() < 6 {
		return Err(err!(
			"A {} frame header at offset {} is at least 6 bytes, but this one is {}.",
			kind, at, data.len();
		Invalid, Input, Decode));
	}
	let prec = data[0];
	if prec != 8 {
		return Err(err!(
			"The frame at offset {} declares {} bits a sample. This codec implements 8.", at, prec;
		Invalid, Input, Decode, NoImpl));
	}
	let h = ((data[1] as usize) << 8) | (data[2] as usize);
	let w = ((data[3] as usize) << 8) | (data[4] as usize);
	if h == 0 {
		return Err(err!(
			"The frame at offset {} declares no height, so its number of lines arrives in a DNL \
			segment. This codec implements the height a frame header carries.", at;
		Invalid, Input, Decode, NoImpl));
	}
	if w == 0 {
		return Err(err!(
			"The frame at offset {} declares a width of zero.", at; Invalid, Input, Decode));
	}
	let n = match w.checked_mul(h) {
		Some(n) => n,
		None => return Err(err!(
			"The frame at offset {} declares {} by {} pixels, which overflows.", at, w, h;
		Invalid, Input, Decode, Overflow)),
	};
	if n > MAX_PIXELS {
		return Err(err!(
			"The frame at offset {} declares {} by {} pixels, over the ceiling of {}.",
			at, w, h, MAX_PIXELS;
		Invalid, Input, Decode, Excessive));
	}

	let nc = data[5] as usize;
	if nc == 0 || nc > 4 {
		return Err(err!(
			"The frame at offset {} declares {} components. This codec implements 1 to 4.", at, nc;
		Invalid, Input, Decode, NoImpl));
	}
	if data.len() < 6 + nc * 3 {
		return Err(err!(
			"The frame at offset {} declares {} components, needing {} bytes, but its header is {}.",
			at, nc, 6 + nc * 3, data.len();
		Invalid, Input, Decode));
	}

	let mut comps = Vec::with_capacity(nc);
	let (mut hmax, mut vmax) = (1usize, 1usize);
	for i in 0..nc {
		let b = 6 + i * 3;
		let id = data[b];
		let h_i = (data[b + 1] >> 4) as usize;
		let v_i = (data[b + 1] & 15) as usize;
		let tq = data[b + 2] as usize;
		if h_i == 0 || h_i > 4 || v_i == 0 || v_i > 4 {
			return Err(err!(
				"Component {} of the frame at offset {} declares sampling factors {} by {}, and the \
				specification allows 1 to 4.", id, at, h_i, v_i;
			Invalid, Input, Decode, Range));
		}
		if tq > 3 {
			return Err(err!(
				"Component {} of the frame at offset {} names quantisation table {}, and there are \
				four slots, 0 to 3.", id, at, tq;
			Invalid, Input, Decode, Range));
		}
		hmax = hmax.max(h_i);
		vmax = vmax.max(v_i);
		comps.push(Comp {
			id,
			h: h_i,
			v: v_i,
			tq,
			dw: 0,
			dh: 0,
			bw: 0,
			bh: 0,
			bwp: 0,
			bhp: 0,
			coef: Vec::new(),
		});
	}

	let mcux = ceil_div(w, DCTSIZE * hmax);
	let mcuy = ceil_div(h, DCTSIZE * vmax);
	for c in comps.iter_mut() {
		c.dw = ceil_div(w * c.h, hmax);
		c.dh = ceil_div(h * c.v, vmax);
		c.bw = ceil_div(c.dw, DCTSIZE);
		c.bh = ceil_div(c.dh, DCTSIZE);
		c.bwp = mcux * c.h;
		c.bhp = mcuy * c.v;
		let cells = match c.bwp.checked_mul(c.bhp).and_then(|n| n.checked_mul(DCTSIZE2)) {
			Some(n) => n,
			None => return Err(err!(
				"Component {} of the frame at offset {} needs a coefficient buffer that overflows.",
				c.id, at;
			Invalid, Input, Decode, Overflow)),
		};
		if cells > MAX_PIXELS * 2 {
			return Err(err!(
				"Component {} of the frame at offset {} needs {} coefficients, which is beyond what \
				this codec will allocate.", c.id, at, cells;
			Invalid, Input, Decode, Excessive));
		}
		if alloc {
			c.coef = vec![0i16; cells];
		}
	}

	Ok(Frame {
		seen: vec![[-1i8; DCTSIZE2]; comps.len()],
		prog: marker == 0xC2,
		w,
		h,
		hmax,
		vmax,
		mcux,
		mcuy,
		comps,
	})
}

/// Reads one or more quantisation tables out of a DQT segment.
fn read_quant(data: &[u8], slots: &mut [Option<[u16; DCTSIZE2]>; 4], at: usize) -> Outcome<()> {
	let mut p = 0usize;
	while p < data.len() {
		let pq = (data[p] >> 4) as usize;
		let tq = (data[p] & 15) as usize;
		p += 1;
		if tq > 3 {
			return Err(err!(
				"A DQT segment at offset {} names table slot {}, and there are four, 0 to 3.", at, tq;
			Invalid, Input, Decode, Range));
		}
		if pq > 1 {
			return Err(err!(
				"A DQT segment at offset {} declares precision {} for table {}, and the \
				specification allows 0 for eight bits and 1 for sixteen.", at, pq, tq;
			Invalid, Input, Decode, Range));
		}
		let need = if pq == 0 { DCTSIZE2 } else { DCTSIZE2 * 2 };
		if p + need > data.len() {
			return Err(err!(
				"A DQT segment at offset {} declares table {} but carries only {} of the {} bytes it \
				needs.", at, tq, data.len() - p, need;
			Invalid, Input, Decode));
		}
		let mut t = [0u16; DCTSIZE2];
		for k in 0..DCTSIZE2 {
			let v = if pq == 0 {
				data[p + k] as u16
			} else {
				((data[p + k * 2] as u16) << 8) | (data[p + k * 2 + 1] as u16)
			};
			if v == 0 {
				return Err(err!(
					"A DQT segment at offset {} gives table {} a zero divisor at zigzag position {}.",
					at, tq, k;
				Invalid, Input, Decode, ZeroDenominator));
			}
			t[NATURAL[k]] = v;
		}
		p += need;
		slots[tq] = Some(t);
	}
	Ok(())
}

/// Reads one or more Huffman tables out of a DHT segment.
fn read_huff(
	data:	&[u8],
	dc:	&mut [Option<Huff>; 4],
	ac:	&mut [Option<Huff>; 4],
	at:	usize,
)
	-> Outcome<()>
{
	let mut p = 0usize;
	while p < data.len() {
		let tc = (data[p] >> 4) as usize;
		let th = (data[p] & 15) as usize;
		p += 1;
		if tc > 1 {
			return Err(err!(
				"A DHT segment at offset {} declares table class {}, and there are two: 0 for DC and \
				1 for AC.", at, tc;
			Invalid, Input, Decode, Range));
		}
		if th > 3 {
			return Err(err!(
				"A DHT segment at offset {} names table slot {}, and there are four, 0 to 3.", at, th;
			Invalid, Input, Decode, Range));
		}
		if p + 16 > data.len() {
			return Err(err!(
				"A DHT segment at offset {} ends before the sixteen code counts of table {}.", at, th;
			Invalid, Input, Decode));
		}
		let mut counts = [0u8; 17];
		let mut total = 0usize;
		for l in 1..=16usize {
			counts[l] = data[p + l - 1];
			total += counts[l] as usize;
		}
		p += 16;
		if total > 256 {
			return Err(err!(
				"A DHT segment at offset {} gives table {} {} symbols, and a byte names at most 256.",
				at, th, total;
			Invalid, Input, Decode, Excessive));
		}
		if p + total > data.len() {
			return Err(err!(
				"A DHT segment at offset {} declares {} symbols for table {} but carries only {}.",
				at, total, th, data.len() - p;
			Invalid, Input, Decode));
		}
		let vals = data[p..p + total].to_vec();
		p += total;
		let t = res!(Huff::new(&counts, vals));
		if tc == 0 {
			dc[th] = Some(t);
		} else {
			ac[th] = Some(t);
		}
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SCANS                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// One component of a scan: which component of the frame, and which two table slots it reads with.
#[derive(Clone, Copy, Debug)]
struct ScanComp {
	ci:	usize,	// the index of the component within the frame
	td:	usize,	// the DC table slot
	ta:	usize,	// and the AC one
}

#[derive(Clone, Debug)]
struct Scan {
	comps:	Vec<ScanComp>,	// in the order the scan gives them
	ss:	usize,				// first coefficient of the spectral band, zigzag order
	se:	usize,				// and the last
	ah:	u32,				// bit position the previous scan of this band reached
	al:	u32,				// and the one this scan reaches
}

fn read_scan(data: &[u8], frame: &Frame, at: usize) -> Outcome<Scan> {
	if data.is_empty() {
		return Err(err!("The scan header at offset {} is empty.", at; Invalid, Input, Decode));
	}
	let ns = data[0] as usize;
	if ns == 0 || ns > 4 {
		return Err(err!(
			"The scan at offset {} declares {} components, and the specification allows 1 to 4.",
			at, ns;
		Invalid, Input, Decode, Range));
	}
	if data.len() < 1 + ns * 2 + 3 {
		return Err(err!(
			"The scan header at offset {} declares {} components, needing {} bytes, but it is {}.",
			at, ns, 1 + ns * 2 + 3, data.len();
		Invalid, Input, Decode));
	}
	let mut comps = Vec::with_capacity(ns);
	for i in 0..ns {
		let b = 1 + i * 2;
		let id = data[b];
		let ci = match frame.comps.iter().position(|c| c.id == id) {
			Some(ci) => ci,
			None => return Err(err!(
				"The scan at offset {} names component {}, which its frame does not declare.", at, id;
			Invalid, Input, Decode, NotFound)),
		};
		comps.push(ScanComp {
			ci,
			td: (data[b + 1] >> 4) as usize,
			ta: (data[b + 1] & 15) as usize,
		});
	}
	let b = 1 + ns * 2;
	let ss = data[b] as usize;
	let se = data[b + 1] as usize;
	let ah = (data[b + 2] >> 4) as u32;
	let al = (data[b + 2] & 15) as u32;

	if frame.prog {
		if ss > 63 || se > 63 || ss > se {
			return Err(err!(
				"The progressive scan at offset {} declares the spectral band {} to {}, which is not \
				a band within 0 to 63.", at, ss, se;
			Invalid, Input, Decode, Range));
		}
		if ss == 0 && se != 0 {
			return Err(err!(
				"The progressive scan at offset {} mixes the DC coefficient with AC coefficients, \
				running from 0 to {}. A DC scan carries coefficient 0 alone.", at, se;
			Invalid, Input, Decode));
		}
		if ss != 0 && ns != 1 {
			return Err(err!(
				"The progressive scan at offset {} carries {} components over the AC band {} to {}, \
				and an AC scan carries one component.", at, ns, ss, se;
			Invalid, Input, Decode));
		}
		if al > 13 || ah > 13 {
			return Err(err!(
				"The progressive scan at offset {} declares successive approximation {} to {}, and \
				the specification allows 0 to 13.", at, ah, al;
			Invalid, Input, Decode, Range));
		}
		if ah != 0 && ah != al + 1 {
			return Err(err!(
				"The progressive scan at offset {} refines from bit {} to bit {}, and a refinement \
				moves one bit at a time.", at, ah, al;
			Invalid, Input, Decode));
		}
	}
	Ok(Scan { comps, ss, se, ah, al })
}

/// A borrowed pair of Huffman tables, one for each class.
struct Tables<'a> {
	dc:	Option<&'a Huff>,	// absent for a scan that reads no DC coefficients
	ac:	Option<&'a Huff>,	// absent for a scan that reads no AC coefficients
}

/// The state a scan carries from one block to the next.
struct ScanState {
	pred:	Vec<i32>,	// the DC predictor of each scan component
	eobrun:	u32,		// how many end-of-band runs remain
}

/// Decodes the entropy-coded data of one scan; the offset it ended at comes back.
fn decode_scan(
	buf:	&[u8],
	start:	usize,
	frame:	&mut Frame,
	scan:	&Scan,
	tabs:	&[Tables],
	ri:	usize,
)
	-> Outcome<usize>
{
	let mut bits = Bits::new(buf, start);
	let mut st = ScanState {
		pred: vec![0i32; scan.comps.len()],
		eobrun: 0,
	};

	// A scan of one component walks that component's own block grid; a scan of several walks the
	// MCU grid, taking each component's sampling factors' worth of blocks in turn.
	let single = scan.comps.len() == 1;
	let (nx, ny) = if single {
		let c = &frame.comps[scan.comps[0].ci];
		(c.bw, c.bh)
	} else {
		(frame.mcux, frame.mcuy)
	};
	let total = nx * ny;
	let mut todo = ri;

	for i in 0..total {
		if ri > 0 && todo == 0 {
			bits.restart();
			for p in st.pred.iter_mut() {
				*p = 0;
			}
			st.eobrun = 0;
			todo = ri;
		}
		if bits.starved() {
			// The entropy-coded data has run out before the scan did. The coefficients left behind
			// are zero, which the inverse DCT turns into a flat mid-grey, and that is what every
			// other decoder shows for the tail of a truncated file. Carrying on with whatever the
			// padding decodes to would fill it with noise instead.
			if ri == 0 {
				break;
			}
			todo -= 1;
			continue;
		}
		let (ux, uy) = (i % nx, i / nx);
		if single {
			let sc = scan.comps[0];
			res!(decode_block(&mut bits, frame, scan, &tabs[0], &mut st, 0, sc.ci, ux, uy));
		} else {
			for (k, sc) in scan.comps.iter().enumerate() {
				let (ch, cv) = {
					let c = &frame.comps[sc.ci];
					(c.h, c.v)
				};
				for by in 0..cv {
					for bx in 0..ch {
						res!(decode_block(
							&mut bits,
							frame,
							scan,
							&tabs[k],
							&mut st,
							k,
							sc.ci,
							ux * ch + bx,
							uy * cv + by,
						));
					}
				}
			}
		}
		if ri > 0 {
			todo -= 1;
		}
	}
	Ok(bits.pos)
}

/// Decodes one block, by whichever of the five block codings the scan calls for.
fn decode_block(
	bits:	&mut Bits,
	frame:	&mut Frame,
	scan:	&Scan,
	tabs:	&Tables,
	st:	&mut ScanState,
	k:	usize,
	ci:	usize,
	bx:	usize,
	by:	usize,
)
	-> Outcome<()>
{
	let c = &mut frame.comps[ci];
	if bx >= c.bwp || by >= c.bhp {
		return Ok(()); // Padding beyond the allocation, which no image sample depends on.
	}
	let at = c.at(bx, by);
	let blk = &mut c.coef[at..at + DCTSIZE2];
	if !frame.prog {
		return block_sequential(bits, tabs, &mut st.pred[k], blk);
	}
	match (scan.ss, scan.ah) {
		(0, 0)	=> block_dc_first(bits, tabs, &mut st.pred[k], blk, scan.al),
		(0, _)	=> {
			if bits.bit() != 0 {
				blk[0] |= (1i32 << scan.al) as i16;
			}
			Ok(())
		},
		(_, 0)	=> block_ac_first(bits, tabs, st, blk, scan),
		(_, _)	=> block_ac_refine(bits, tabs, st, blk, scan),
	}
}

/// The AC table a scan needs, or an error naming the slot that was never declared.
fn need_ac<'a>(tabs: &'a Tables) -> Outcome<&'a Huff> {
	match tabs.ac {
		Some(t) => Ok(t),
		None => Err(err!(
			"A scan reads AC coefficients with a Huffman table its file never declared.";
		Invalid, Input, Decode, Missing)),
	}
}

fn need_dc<'a>(tabs: &'a Tables) -> Outcome<&'a Huff> {
	match tabs.dc {
		Some(t) => Ok(t),
		None => Err(err!(
			"A scan reads DC coefficients with a Huffman table its file never declared.";
		Invalid, Input, Decode, Missing)),
	}
}

/// Decodes a whole block, DC and AC together, as a sequential scan carries it.
fn block_sequential(bits: &mut Bits, tabs: &Tables, pred: &mut i32, blk: &mut [i16]) -> Outcome<()> {
	let dct = res!(need_dc(tabs));
	let act = res!(need_ac(tabs));
	let t = res!(bits.huff(dct)) as u32;
	if t > 15 {
		return Err(err!(
			"A DC coefficient declares magnitude category {}, and the categories run to 15.", t;
		Invalid, Input, Decode, Range));
	}
	let diff = extend(bits.receive(t), t);
	*pred = pred.wrapping_add(diff);
	blk[0] = *pred as i16;

	let mut k = 1usize;
	while k < DCTSIZE2 {
		let rs = res!(bits.huff(act));
		let s = (rs & 15) as u32;
		let r = (rs >> 4) as usize;
		if s == 0 {
			if r != 15 {
				break; // End of block.
			}
			k += 16; // A run of sixteen zeros.
		} else {
			k += r;
			if k >= DCTSIZE2 {
				return Err(err!(
					"An AC run of {} carries coefficient {} past the 63rd of its block.", r, k;
				Invalid, Input, Decode, Range));
			}
			blk[NATURAL[k]] = extend(bits.receive(s), s) as i16;
			k += 1;
		}
	}
	Ok(())
}

/// Decodes the DC coefficient of a block in a progressive scan's first pass over it.
fn block_dc_first(
	bits:	&mut Bits,
	tabs:	&Tables,
	pred:	&mut i32,
	blk:	&mut [i16],
	al:	u32,
)
	-> Outcome<()>
{
	let dct = res!(need_dc(tabs));
	let t = res!(bits.huff(dct)) as u32;
	if t > 15 {
		return Err(err!(
			"A DC coefficient declares magnitude category {}, and the categories run to 15.", t;
		Invalid, Input, Decode, Range));
	}
	let diff = extend(bits.receive(t), t);
	*pred = pred.wrapping_add(diff);
	blk[0] = pred.wrapping_shl(al) as i16;
	Ok(())
}

/// Decodes a band of AC coefficients in a progressive scan's first pass over them.
fn block_ac_first(
	bits:	&mut Bits,
	tabs:	&Tables,
	st:	&mut ScanState,
	blk:	&mut [i16],
	scan:	&Scan,
)
	-> Outcome<()>
{
	if st.eobrun > 0 {
		st.eobrun -= 1;
		return Ok(());
	}
	let act = res!(need_ac(tabs));
	let mut k = scan.ss;
	while k <= scan.se {
		let rs = res!(bits.huff(act));
		let s = (rs & 15) as u32;
		let r = (rs >> 4) as u32;
		if s == 0 {
			if r != 15 {
				st.eobrun = (1u32 << r) - 1;
				if r > 0 {
					st.eobrun += bits.receive(r);
				}
				break;
			}
			k += 15;
		} else {
			k += r as usize;
			if k > scan.se {
				return Err(err!(
					"An AC run of {} carries coefficient {} past the {}th, where the scan's band \
					ends.", r, k, scan.se;
				Invalid, Input, Decode, Range));
			}
			blk[NATURAL[k]] = (extend(bits.receive(s), s) << scan.al) as i16;
		}
		k += 1;
	}
	Ok(())
}

/// Appends a bit to a band of AC coefficients a previous scan already placed.
///
/// This is the one block coding with no simple shape: a symbol names a run of coefficients that were
/// zero before this scan, and the bits of the coefficients that were not zero are interleaved
/// between them, one correction bit each, in the order they occur.
fn block_ac_refine(
	bits:	&mut Bits,
	tabs:	&Tables,
	st:	&mut ScanState,
	blk:	&mut [i16],
	scan:	&Scan,
)
	-> Outcome<()>
{
	let act = res!(need_ac(tabs));
	let p1 = 1i16 << scan.al; // The bit a newly nonzero positive coefficient takes.
	let m1 = (-1i16) << scan.al; // The same, negative.
	let mut k = scan.ss;

	if st.eobrun == 0 {
		while k <= scan.se {
			let rs = res!(bits.huff(act));
			let s = rs & 15;
			let mut r = (rs >> 4) as i32;
			let mut place = 0i16;
			if s != 0 {
				if s != 1 {
					return Err(err!(
						"A refining AC scan declares a new coefficient of magnitude category {}, and \
						a refinement can only make a coefficient newly nonzero, category 1.", s;
					Invalid, Input, Decode));
				}
				place = if bits.bit() != 0 { p1 } else { m1 };
			} else if r != 15 {
				st.eobrun = 1u32 << r;
				if r > 0 {
					st.eobrun += bits.receive(r as u32);
				}
				break;
			}
			// Walk over the coefficients already nonzero, correcting each, and over `r` of those
			// still zero, to reach the one the symbol names.
			loop {
				if k > scan.se {
					break;
				}
				let pos = NATURAL[k];
				if blk[pos] != 0 {
					if bits.bit() != 0 && (blk[pos] & p1) == 0 {
						blk[pos] = if blk[pos] >= 0 {
							blk[pos].wrapping_add(p1)
						} else {
							blk[pos].wrapping_add(m1)
						};
					}
				} else {
					r -= 1;
					if r < 0 {
						break;
					}
				}
				k += 1;
			}
			if place != 0 && k <= scan.se {
				blk[NATURAL[k]] = place;
			}
			k += 1;
		}
	}

	if st.eobrun > 0 {
		// The rest of the band lies inside an end-of-band run, so it carries correction bits for
		// the coefficients already nonzero and nothing else.
		while k <= scan.se {
			let pos = NATURAL[k];
			if blk[pos] != 0 && bits.bit() != 0 && (blk[pos] & p1) == 0 {
				blk[pos] = if blk[pos] >= 0 {
					blk[pos].wrapping_add(p1)
				} else {
					blk[pos].wrapping_add(m1)
				};
			}
			k += 1;
		}
		st.eobrun -= 1;
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ UPSAMPLING AND OUTPUT                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// One component's samples, after the inverse DCT and before upsampling.
struct Plane {
	data:	Vec<u8>,	// the samples, row-major
	stride:	usize,		// row distance, which the MCU grid may round up beyond dw
	dw:	usize,			// the width that carries image; the rest of a row is padding
	dh:	usize,			// and the height
}

impl Plane {

	/// A sample, with the coordinates clamped into the part that carries image.
	fn at(&self, x: usize, y: usize) -> i32 {
		let x = x.min(self.dw - 1);
		let y = y.min(self.dh - 1);
		self.data[y * self.stride + x] as i32
	}
}

/// Expands a plane to the full image size.
///
/// Doubling in either direction takes the triangle filter libjpeg applies by default, which weights
/// the nearer source sample three to one against its neighbour; every other ratio replicates. The
/// filter matters: against a sharp chroma edge, replication and the triangle filter disagree by far
/// more than the rounding of an inverse DCT does.
fn upsample(p: &Plane, xf: usize, yf: usize, ow: usize, oh: usize) -> Vec<u8> {
	let mut out = vec![0u8; ow * oh];
	if xf == 1 && yf == 1 {
		for y in 0..oh {
			for x in 0..ow {
				out[y * ow + x] = p.at(x, y) as u8;
			}
		}
		return out;
	}
	if xf == 1 && yf == 2 {
		// Subsampled down the vertical only: the filter runs in that direction alone.
		for oy in 0..oh {
			let iy = oy / 2;
			let far = if oy % 2 == 0 {
				iy.saturating_sub(1)
			} else {
				(iy + 1).min(p.dh - 1)
			};
			// The two output rows take different rounding terms, as the horizontal filter's do.
			let r = if oy % 2 == 0 { 1 } else { 2 };
			for ox in 0..ow {
				out[oy * ow + ox] = clamp8((3 * p.at(ox, iy) + p.at(ox, far) + r) >> 2);
			}
		}
		return out;
	}
	// A plane only two samples wide has no interior for the filter to work over, and libjpeg drops
	// to replication there rather than filtering across the whole width.
	if xf == 2 && (yf == 1 || yf == 2) && p.dw > 2 {
		for oy in 0..oh {
			let (near, far) = if yf == 1 {
				(oy, oy)
			} else {
				let iy = oy / 2;
				let far = if oy % 2 == 0 {
					iy.saturating_sub(1)
				} else {
					(iy + 1).min(p.dh - 1)
				};
				(iy, far)
			};
			// The column sums the filter runs along, weighted three to one vertically.
			let col = |ix: usize| -> i32 {
				if yf == 1 {
					p.at(ix, near)
				} else {
					3 * p.at(ix, near) + p.at(ix, far)
				}
			};
			// The rounding differs between the two, and between the two outputs of each: these are
			// the terms libjpeg's `h2v1_fancy_upsample` and `h2v2_fancy_upsample` add.
			let (shift, r_even, r_odd) = if yf == 1 {
				(2, 1, 2)
			} else {
				(4, 8, 7)
			};
			for ox in 0..ow {
				let ix = ox / 2;
				let this = col(ix);
				let v = if ox % 2 == 0 {
					if ix == 0 {
						(this * 4 + r_even) >> shift
					} else {
						(this * 3 + col(ix - 1) + r_even) >> shift
					}
				} else if ix + 1 >= p.dw {
					(this * 4 + r_odd) >> shift
				} else {
					(this * 3 + col(ix + 1) + r_odd) >> shift
				};
				out[oy * ow + ox] = clamp8(v);
			}
		}
		return out;
	}
	// Any other ratio: replicate.
	for oy in 0..oh {
		let sy = oy / yf;
		for ox in 0..ow {
			out[oy * ow + ox] = p.at(ox / xf, sy) as u8;
		}
	}
	out
}

/// Scales a plane to an arbitrary size by nearest neighbour, for the reduced-scale decode.
fn rescale(p: &Plane, ow: usize, oh: usize) -> Vec<u8> {
	let mut out = vec![0u8; ow * oh];
	for oy in 0..oh {
		let sy = (oy * p.dh) / oh;
		for ox in 0..ow {
			out[oy * ow + ox] = p.at((ox * p.dw) / ow, sy) as u8;
		}
	}
	out
}

/// Which colour the components of a frame carry, given their count and what the file's application
/// segments said.
///
/// The order of the tests is libjpeg's, and it matters: a JFIF segment settles the question by
/// itself, because JFIF is defined as YCbCr. Files exist that carry a JFIF segment and an Adobe one
/// declaring no transform, and reading the Adobe segment first turns them into false colour.
fn space_of(comps: &[Comp], jfif: bool, adobe: Option<u8>) -> Outcome<Space> {
	match comps.len() {
		1 => Ok(Space::Grey),
		3 => {
			if jfif {
				return Ok(Space::Ycc);
			}
			if let Some(t) = adobe {
				return Ok(if t == 0 { Space::Rgb } else { Space::Ycc });
			}
			// With neither segment, component identifiers spelling RGB are the only sign that a
			// three-component frame is not YCbCr.
			if comps[0].id == b'R' && comps[1].id == b'G' && comps[2].id == b'B' {
				Ok(Space::Rgb)
			} else {
				Ok(Space::Ycc)
			}
		},
		4 => Ok(match adobe {
			Some(2)	=> Space::Ycck,
			_	=> Space::Cmyk,
		}),
		n => Err(err!(
			"A frame of {} components has no colour interpretation this codec knows.", n;
		Invalid, Input, Decode, NoImpl)),
	}
}

/// Turns the upsampled component channels into a pixmap.
///
/// The four-component spaces are Adobe's, where the stored samples are the complement of the ink,
/// so a channel of 255 is no ink at all. Where a file carries no Adobe segment its four components
/// are taken as ink directly.
fn colourise(
	ch:	&[Vec<u8>],
	w:	usize,
	h:	usize,
	space:	Space,
	inverted: bool,
)
	-> Outcome<Pixmap>
{
	let mut pm = res!(Pixmap::new(w, h));
	let tab = YccTab::new();
	let out = pm.data_mut();
	for i in 0..(w * h) {
		let (r, g, b) = match space {
			Space::Grey	=> {
				let y = ch[0][i];
				(y, y, y)
			},
			Space::Rgb	=> (ch[0][i], ch[1][i], ch[2][i]),
			Space::Ycc	=> tab.rgb(ch[0][i], ch[1][i], ch[2][i]),
			Space::Cmyk | Space::Ycck => {
				let (c, m, y) = if space == Space::Ycck {
					let (r, g, b) = tab.rgb(ch[0][i], ch[1][i], ch[2][i]);
					(255 - r, 255 - g, 255 - b)
				} else {
					(ch[0][i], ch[1][i], ch[2][i])
				};
				let k = ch[3][i];
				let (c, m, y, k) = if inverted {
					(c as u32, m as u32, y as u32, k as u32)
				} else {
					// No Adobe segment: the samples are ink, so complement them into the same form.
					(255 - c as u32, 255 - m as u32, 255 - y as u32, 255 - k as u32)
				};
				(
					((c * k + 127) / 255) as u8,
					((m * k + 127) / 255) as u8,
					((y * k + 127) / 255) as u8,
				)
			},
		};
		let at = i * 4;
		out[at] = r;
		out[at + 1] = g;
		out[at + 2] = b;
		out[at + 3] = 255;
	}
	Ok(pm)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BLOCK SMOOTHING                                                            │
// └───────────────────────────────────────────────────────────────────────────┘
//
// A progressive file whose later scans never arrived, or whose encoder stopped short of the last
// approximation bit, holds blocks whose low-frequency detail is missing. Rendered as they stand
// they show the flat squares of a half-loaded photograph. The specification's Annex K.8 estimates
// the five lowest AC coefficients from the DC values of the eight neighbouring blocks, which is a
// smooth surface fitted through the block means, and libjpeg applies it by default. A file whose
// scans all completed is untouched, because there is then nothing to estimate.

// The five coefficients an estimate may fill in: their zigzag position, their natural position,
// and the multiplier and neighbour combination Annex K.8 gives each.
const SMOOTH: [(usize, usize, i64); 5] = [
	(1, 1, 36),	// One cycle across.
	(2, 8, 36),	// One cycle down.
	(3, 16, 9),	// Two cycles down.
	(4, 9, 5),	// One cycle each way.
	(5, 2, 9),	// Two cycles across.
];

/// Is this a frame block smoothing has anything to say about?
///
/// Every component's DC must have arrived, since the estimates are built from it, and at least one
/// of the five estimated coefficients must be inexact somewhere.
fn smoothing_helps(frame: &Frame) -> bool {
	if !frame.prog {
		return false;
	}
	let mut useful = false;
	for seen in &frame.seen {
		if seen[0] < 0 {
			return false;
		}
		for (zz, _, _) in SMOOTH {
			if seen[zz] != 0 {
				useful = true;
			}
		}
	}
	useful
}

/// Fills in the five lowest AC coefficients of one block from its neighbours' DC values.
///
/// A coefficient is estimated only where it is still zero and no scan has pinned it down exactly.
fn smooth(
	ws:	&mut [i16; DCTSIZE2],
	coef:	&[i16],
	c:	&Comp,
	bx:	usize,
	by:	usize,
	q:	&[u16; DCTSIZE2],
	seen:	&[i8; DCTSIZE2],
) {
	// The DC values of the three by three neighbourhood, with the edges replicating.
	let dc = |dx: i32, dy: i32| -> i64 {
		let x = (bx as i32 + dx).clamp(0, c.bw as i32 - 1) as usize;
		let y = (by as i32 + dy).clamp(0, c.bh as i32 - 1) as usize;
		coef[(y * c.bwp + x) * DCTSIZE2] as i64
	};
	let (d1, d2, d3) = (dc(-1, -1), dc(0, -1), dc(1, -1));
	let (d4, d5, d6) = (dc(-1, 0), dc(0, 0), dc(1, 0));
	let (d7, d8, d9) = (dc(-1, 1), dc(0, 1), dc(1, 1));
	let q00 = q[0] as i64;

	for (zz, nat, mul) in SMOOTH {
		let al = seen[zz];
		if al == 0 || ws[nat] != 0 {
			continue;
		}
		let comb = match zz {
			1	=> d4 - d6,
			2	=> d2 - d8,
			3	=> d2 + d8 - 2 * d5,
			4	=> d1 - d3 - d7 + d9,
			_	=> d4 + d6 - 2 * d5,
		};
		let num = mul * q00 * comb;
		let qn = q[nat] as i64;
		let mut pred = ((qn << 7) + num.abs()) / (qn << 8);
		// An estimate may not claim more precision than the scans that did arrive left room for.
		if al > 0 && pred >= (1i64 << al) {
			pred = (1i64 << al) - 1;
		}
		if num < 0 {
			pred = -pred;
		}
		ws[nat] = pred as i16;
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DECODER                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Walks the marker segments of a file, decoding every scan it finds.
fn parse(buf: &[u8]) -> Outcome<Reader> {
	if buf.len() < 2 || buf[0] != 0xFF || buf[1] != SOI {
		return Err(err!(
			"The bytes do not begin with a JPEG start-of-image marker."; Invalid, Input, Decode));
	}
	let mut r = Reader {
		quant: [None, None, None, None],
		dc: [None, None, None, None],
		ac: [None, None, None, None],
		ri: 0,
		jfif: false,
		adobe: None,
		frame: None,
	};
	let mut pos = 2usize;
	let mut scans = 0usize;

	loop {
		let (marker, next) = match next_marker(buf, pos) {
			Ok(m) => m,
			Err(_) => break, // A file that stops after its last scan is one we have already read.
		};
		pos = next;
		match marker {
			SOI => (),
			EOI => break,
			TEM => (),
			RST0..=RST7 => (),
			0xFF => (),
			DQT => {
				let (a, b) = res!(segment(buf, pos, marker));
				res!(read_quant(&buf[a..b], &mut r.quant, pos));
				pos = b;
			},
			DHT => {
				let (a, b) = res!(segment(buf, pos, marker));
				res!(read_huff(&buf[a..b], &mut r.dc, &mut r.ac, pos));
				pos = b;
			},
			DRI => {
				let (a, b) = res!(segment(buf, pos, marker));
				if b - a < 2 {
					return Err(err!(
						"A DRI segment at offset {} carries {} bytes, and a restart interval is two.",
						pos, b - a;
					Invalid, Input, Decode));
				}
				r.ri = ((buf[a] as usize) << 8) | (buf[a + 1] as usize);
				pos = b;
			},
			DAC => return Err(err!(
				"The file carries an arithmetic coding conditioning segment at offset {}. This codec \
				implements Huffman coding.", pos;
			Invalid, Input, Decode, NoImpl)),
			APP0..=APP15 => {
				let (a, b) = res!(segment(buf, pos, marker));
				if marker == APP0 && b - a >= 5 && &buf[a..a + 5] == b"JFIF\0" {
					r.jfif = true;
				}
				if marker == 0xEE && b - a >= 12 && &buf[a..a + 5] == b"Adobe" {
					r.adobe = Some(buf[b - 1]);
				}
				pos = b;
			},
			COM | DNL => {
				let (_, b) = res!(segment(buf, pos, marker));
				pos = b;
			},
			SOS => {
				let (a, b) = res!(segment(buf, pos, marker));
				let frame = match r.frame.as_mut() {
					Some(f) => f,
					None => return Err(err!(
						"The scan at offset {} arrives before any frame header.", pos;
					Invalid, Input, Decode, Order)),
				};
				let scan = res!(read_scan(&buf[a..b], frame, pos));
				// Borrow the tables the scan names, refusing a slot the file never filled.
				let mut tabs = Vec::with_capacity(scan.comps.len());
				for sc in &scan.comps {
					let want_dc = !frame.prog || (scan.ss == 0);
					let want_ac = !frame.prog || (scan.ss != 0);
					if sc.td > 3 || sc.ta > 3 {
						return Err(err!(
							"The scan at offset {} names Huffman slots {} and {}, and there are four \
							of each, 0 to 3.", pos, sc.td, sc.ta;
						Invalid, Input, Decode, Range));
					}
					if want_dc && r.dc[sc.td].is_none() && scan.ah == 0 {
						return Err(err!(
							"The scan at offset {} reads component {} with DC Huffman table {}, which \
							the file has not declared.", pos, frame.comps[sc.ci].id, sc.td;
						Invalid, Input, Decode, Missing));
					}
					if want_ac && r.ac[sc.ta].is_none() {
						return Err(err!(
							"The scan at offset {} reads component {} with AC Huffman table {}, which \
							the file has not declared.", pos, frame.comps[sc.ci].id, sc.ta;
						Invalid, Input, Decode, Missing));
					}
					tabs.push(Tables {
						dc: r.dc[sc.td].as_ref(),
						ac: r.ac[sc.ta].as_ref(),
					});
				}
				for sc in &scan.comps {
					for k in scan.ss..=scan.se.min(DCTSIZE2 - 1) {
						frame.seen[sc.ci][k] = scan.al as i8;
					}
				}
				pos = res!(decode_scan(buf, b, frame, &scan, &tabs, r.ri));
				scans += 1;
			},
			0xC0..=0xCF => {
				let (a, b) = res!(segment(buf, pos, marker));
				if r.frame.is_some() {
					return Err(err!(
						"The file carries a second frame header at offset {}. This codec implements \
						the single-frame modes.", pos;
					Invalid, Input, Decode, NoImpl));
				}
				r.frame = Some(res!(read_frame(&buf[a..b], marker, pos, true)));
				pos = b;
			},
			_ => {
				let (_, b) = res!(segment(buf, pos, marker));
				pos = b;
			},
		}
	}

	if r.frame.is_none() {
		return Err(err!("The file carries no frame header."; Invalid, Input, Decode, Missing));
	}
	if scans == 0 {
		return Err(err!("The file carries no scan."; Invalid, Input, Decode, Missing));
	}
	Ok(r)
}

/// The quantisation table a component names, or an error naming the slot the file left empty.
fn quant_of(r: &Reader, c: &Comp) -> Outcome<[u16; DCTSIZE2]> {
	match r.quant[c.tq] {
		Some(t) => Ok(t),
		None => Err(err!(
			"Component {} reads quantisation table {}, which the file has not declared.", c.id, c.tq;
		Invalid, Input, Decode, Missing)),
	}
}

/// The pixels come out opaque: JPEG carries no alpha channel.
pub fn decode(buf: &[u8]) -> Outcome<Pixmap> {
	let mut r = res!(parse(buf));
	let mut frame = match r.frame.take() {
		Some(f) => f,
		None => return Err(err!("The file carries no frame header."; Invalid, Input, Decode, Missing)),
	};
	let space = res!(space_of(&frame.comps, r.jfif, r.adobe));
	let (w, h) = (frame.w, frame.h);
	let (hmax, vmax) = (frame.hmax, frame.vmax);
	let smoothing = smoothing_helps(&frame);
	let seen = frame.seen.clone();

	let mut chans: Vec<Vec<u8>> = Vec::with_capacity(frame.comps.len());
	for (ci, c) in frame.comps.iter_mut().enumerate() {
		let q = res!(quant_of(&r, c));
		let stride = c.bwp * DCTSIZE;
		let mut plane = Plane {
			data: vec![0u8; stride * c.bhp * DCTSIZE],
			stride,
			dw: c.dw,
			dh: c.dh,
		};
		let coef = std::mem::take(&mut c.coef);
		// Only the blocks that carry image are transformed: the upsampler reads no further, and the
		// MCU grid's padding blocks would only be cropped away.
		let mut ws = [0i16; DCTSIZE2];
		for by in 0..c.bh {
			for bx in 0..c.bw {
				let at = (by * c.bwp + bx) * DCTSIZE2;
				let src = if smoothing {
					ws.copy_from_slice(&coef[at..at + DCTSIZE2]);
					smooth(&mut ws, &coef, c, bx, by, &q, &seen[ci]);
					&ws[..]
				} else {
					&coef[at..at + DCTSIZE2]
				};
				idct(
					src,
					&q,
					&mut plane.data,
					by * DCTSIZE * stride + bx * DCTSIZE,
					stride,
				);
			}
		}
		drop(coef);
		let xf = hmax / c.h;
		let yf = vmax / c.v;
		let exact = hmax % c.h == 0 && vmax % c.v == 0;
		chans.push(if exact {
			upsample(&plane, xf, yf, w, h)
		} else {
			rescale(&plane, w, h)
		});
	}
	colourise(&chans, w, h, space, r.adobe.is_some())
}

/// Decodes a JPEG at an eighth of its size, from the DC coefficient of each block alone.
///
/// One coefficient a block is the block's mean, so this costs the entropy decoding and nothing else:
/// no inverse DCT runs, and the image never exists at full size. It is what a thumbnail wants.
pub fn decode_eighth(buf: &[u8]) -> Outcome<Pixmap> {
	let mut r = res!(parse(buf));
	let mut frame = match r.frame.take() {
		Some(f) => f,
		None => return Err(err!("The file carries no frame header."; Invalid, Input, Decode, Missing)),
	};
	let space = res!(space_of(&frame.comps, r.jfif, r.adobe));
	let w = ceil_div(frame.w, DCTSIZE);
	let h = ceil_div(frame.h, DCTSIZE);

	let mut chans: Vec<Vec<u8>> = Vec::with_capacity(frame.comps.len());
	for c in frame.comps.iter_mut() {
		let q = res!(quant_of(&r, c));
		let coef = std::mem::take(&mut c.coef);
		let mut plane = Plane {
			data: vec![0u8; c.bwp * c.bhp],
			stride: c.bwp,
			dw: c.bw,
			dh: c.bh,
		};
		for by in 0..c.bhp {
			for bx in 0..c.bwp {
				let at = (by * c.bwp + bx) * DCTSIZE2;
				plane.data[by * c.bwp + bx] = idct_dc(&coef[at..at + DCTSIZE2], &q);
			}
		}
		drop(coef);
		chans.push(rescale(&plane, w, h));
	}
	colourise(&chans, w, h, space, r.adobe.is_some())
}

/// Reads a JPEG's size without decoding a single block.
///
/// Only the marker segments up to the frame header are walked, so this costs a few hundred bytes of
/// reading whatever the size of the file.
pub fn dimensions(buf: &[u8]) -> Outcome<(usize, usize)> {
	if buf.len() < 2 || buf[0] != 0xFF || buf[1] != SOI {
		return Err(err!(
			"The bytes do not begin with a JPEG start-of-image marker."; Invalid, Input, Decode));
	}
	let mut pos = 2usize;
	loop {
		let (marker, next) = res!(next_marker(buf, pos));
		pos = next;
		match marker {
			SOI | TEM | RST0..=RST7 | 0xFF => (),
			EOI | SOS => return Err(err!(
				"The file reaches its {} at offset {} without a frame header.",
				if marker == EOI { "end" } else { "first scan" }, pos;
			Invalid, Input, Decode, Missing)),
			0xC0..=0xCF if marker != DHT && marker != DAC && marker != 0xC8 => {
				let (a, b) = res!(segment(buf, pos, marker));
				let f = res!(read_frame(&buf[a..b], marker, pos, false));
				return Ok((f.w, f.h));
			},
			_ => {
				let (_, b) = res!(segment(buf, pos, marker));
				pos = b;
			},
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE ENCODER                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// How much the two chrominance channels are reduced against the luminance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
	Full,		// 4:4:4 -- a chrominance sample for every pixel
	Half,		// 4:2:2 -- one for every two pixels across
	Quarter,	// 4:2:0 -- one for every two across and two down
}

impl Chroma {

	/// The horizontal and vertical sampling factors the luminance takes against it.
	fn factors(&self) -> (usize, usize) {
		match self {
			Self::Full	=> (1, 1),
			Self::Half	=> (2, 1),
			Self::Quarter	=> (2, 2),
		}
	}
}

/// What an encoder is asked for.
#[derive(Clone, Copy, Debug)]
pub struct Options {
	pub quality:	u8,		// 1 to 100, scaling the tables the way libjpeg's does
	pub chroma:	Chroma,		// ignored for a greyscale image
	pub grey:	bool,		// write one luminance component rather than three
}

impl Default for Options {

	/// Quality 85 with 4:2:0 chroma, which is what a photograph is usually wanted at.
	fn default() -> Self {
		Self {
			quality: 85,
			chroma: Chroma::Quarter,
			grey: false,
		}
	}
}

// The luminance quantisation table of the specification's Annex K, in natural order.
const QUANT_LUMA: [u16; DCTSIZE2] = [
	16, 11, 10, 16,  24,  40,  51,  61,
	12, 12, 14, 19,  26,  58,  60,  55,
	14, 13, 16, 24,  40,  57,  69,  56,
	14, 17, 22, 29,  51,  87,  80,  62,
	18, 22, 37, 56,  68, 109, 103,  77,
	24, 35, 55, 64,  81, 104, 113,  92,
	49, 64, 78, 87, 103, 121, 120, 101,
	72, 92, 95, 98, 112, 100, 103,  99,
];

// The chrominance quantisation table of the specification's Annex K, in natural order.
const QUANT_CHROMA: [u16; DCTSIZE2] = [
	17, 18, 24, 47, 99, 99, 99, 99,
	18, 21, 26, 66, 99, 99, 99, 99,
	24, 26, 56, 99, 99, 99, 99, 99,
	47, 66, 99, 99, 99, 99, 99, 99,
	99, 99, 99, 99, 99, 99, 99, 99,
	99, 99, 99, 99, 99, 99, 99, 99,
	99, 99, 99, 99, 99, 99, 99, 99,
	99, 99, 99, 99, 99, 99, 99, 99,
];

//// The Annex K Huffman tables. A BITS array is the count of codes at each length, indexed by that
//// length; a VALS array is the symbols those codes name, in canonical code order.
const DC_LUMA_BITS: [u8; 17] = [0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 17] = [0, 0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]; // the magnitude categories

const AC_LUMA_BITS: [u8; 17] = [0, 0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];

const AC_LUMA_VALS: [u8; 162] = [
	0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12,
	0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
	0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
	0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
	0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16,
	0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
	0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
	0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
	0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
	0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
	0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
	0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
	0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98,
	0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
	0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
	0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
	0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4,
	0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
	0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA,
	0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
	0xF9, 0xFA,
];

const AC_CHROMA_BITS: [u8; 17] = [0, 0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];

const AC_CHROMA_VALS: [u8; 162] = [
	0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21,
	0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
	0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
	0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
	0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34,
	0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
	0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38,
	0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
	0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
	0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
	0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78,
	0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
	0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96,
	0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
	0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4,
	0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
	0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2,
	0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
	0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9,
	0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
	0xF9, 0xFA,
];

/// A Huffman table in the form an encoder wants: a code and a length for each symbol.
struct Codes {
	code:	[u16; 256],		// the code of each symbol, zero where the table has none
	len:	[u8; 256],		// and its length
}

impl Codes {

	/// Derives the codes from a count of codes at each length and the symbols they name.
	fn new(counts: &[u8; 17], vals: &[u8]) -> Outcome<Self> {
		let mut t = Self { code: [0; 256], len: [0; 256] };
		let mut code = 0u32;
		let mut k = 0usize;
		for l in 1..=16usize {
			for _ in 0..counts[l] {
				let s = match vals.get(k) {
					Some(s) => *s as usize,
					None => return Err(err!(
						"A Huffman table declares more codes than it carries symbols.";
					Bug, Invalid, Input)),
				};
				t.code[s] = code as u16;
				t.len[s] = l as u8;
				code += 1;
				k += 1;
			}
			code <<= 1;
		}
		Ok(t)
	}
}

/// A writer of the bits of an entropy-coded segment, stuffing a zero after every 0xFF it emits.
struct BitWriter {
	out:	Vec<u8>,	// the bytes written so far
	acc:	u32,		// the bits not yet whole, in the low cnt positions
	cnt:	u32,		// how many bits of acc are pending
}

impl BitWriter {

	fn new() -> Self {
		Self { out: Vec::new(), acc: 0, cnt: 0 }
	}

	/// Appends the low `len` bits of a code, `len` being at most 16.
	fn put(&mut self, code: u32, len: u32) {
		if len == 0 {
			return;
		}
		self.acc = (self.acc << len) | (code & ((1u32 << len) - 1));
		self.cnt += len;
		while self.cnt >= 8 {
			self.cnt -= 8;
			let b = ((self.acc >> self.cnt) & 0xFF) as u8;
			self.out.push(b);
			if b == 0xFF {
				self.out.push(0x00);
			}
		}
	}

	/// Appends a symbol's Huffman code, refusing a symbol the table never gave one.
	fn sym(&mut self, t: &Codes, s: u8) -> Outcome<()> {
		let i = s as usize;
		if t.len[i] == 0 {
			return Err(err!(
				"The encoder's Huffman table has no code for the symbol {:#04X}.", s;
			Bug, Invalid, Encode));
		}
		self.put(t.code[i] as u32, t.len[i] as u32);
		Ok(())
	}

	/// Pads the last byte with one bits, as the specification requires, and yields the bytes.
	fn finish(mut self) -> Vec<u8> {
		if self.cnt > 0 {
			let pad = 8 - self.cnt;
			self.put((1u32 << pad) - 1, pad);
		}
		self.out
	}
}

/// The magnitude category of a coefficient difference, and the bits that follow it.
fn category(v: i32) -> (u8, u32) {
	let mag = v.unsigned_abs();
	let mut s = 0u8;
	let mut t = mag;
	while t > 0 {
		s += 1;
		t >>= 1;
	}
	let bits = if v < 0 {
		(v + (1i32 << s) - 1) as u32
	} else {
		v as u32
	};
	(s, bits & ((1u32 << s.max(1)) - 1))
}

/// The cosines the forward DCT needs, indexed by sample then frequency.
fn cos_table() -> [[f32; DCTSIZE]; DCTSIZE] {
	let mut t = [[0.0f32; DCTSIZE]; DCTSIZE];
	for (x, row) in t.iter_mut().enumerate() {
		for (u, c) in row.iter_mut().enumerate() {
			let s = if u == 0 {
				(0.5f32).sqrt()
			} else {
				1.0
			};
			*c = s * (((2 * x + 1) as f32) * (u as f32) * std::f32::consts::PI / 16.0).cos();
		}
	}
	t
}

/// Takes the forward DCT of one block of samples and quantises it, in natural order.
fn fdct(samples: &[u8], at: usize, stride: usize, q: &[u16; DCTSIZE2], cos: &[[f32; DCTSIZE]; DCTSIZE])
	-> [i32; DCTSIZE2]
{
	// The rows first, then the columns: the two-dimensional transform is separable.
	let mut rows = [0.0f32; DCTSIZE2];
	for y in 0..DCTSIZE {
		for u in 0..DCTSIZE {
			let mut s = 0.0f32;
			for x in 0..DCTSIZE {
				s += ((samples[at + y * stride + x] as f32) - 128.0) * cos[x][u];
			}
			rows[y * DCTSIZE + u] = s;
		}
	}
	let mut out = [0i32; DCTSIZE2];
	for u in 0..DCTSIZE {
		for v in 0..DCTSIZE {
			let mut s = 0.0f32;
			for y in 0..DCTSIZE {
				s += rows[y * DCTSIZE + u] * cos[y][v];
			}
			let c = s / 4.0;
			let d = q[v * DCTSIZE + u] as f32;
			out[v * DCTSIZE + u] = (c / d).round() as i32;
		}
	}
	out
}

/// Scales an Annex K quantisation table for a quality, the way libjpeg's `jpeg_set_quality` does.
fn scale_quant(base: &[u16; DCTSIZE2], quality: u8) -> [u16; DCTSIZE2] {
	let q = quality.clamp(1, 100) as i32;
	let scale = if q < 50 {
		5000 / q
	} else {
		200 - q * 2
	};
	let mut t = [0u16; DCTSIZE2];
	for (i, v) in base.iter().enumerate() {
		let s = ((*v as i32) * scale + 50) / 100;
		t[i] = s.clamp(1, 255) as u16;
	}
	t
}

/// Appends a marker segment: its marker, its length, and its payload.
fn seg(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
	out.push(0xFF);
	out.push(marker);
	let len = body.len() + 2;
	out.push((len >> 8) as u8);
	out.push((len & 0xFF) as u8);
	out.extend_from_slice(body);
}

/// One component being encoded: its samples, padded out to whole MCUs.
struct EncComp {
	id:	u8,				// 1 for luminance, 2 and 3 for the chrominances
	h:	usize,			// horizontal sampling factor
	v:	usize,			// vertical sampling factor
	tbl:	usize,		// table pair: 0 for luminance, 1 for chrominance
	data:	Vec<u8>,	// the samples, bw * 8 to a row
	stride:	usize,		// the distance between rows
}

/// Encodes a pixmap as a baseline JPEG at the default quality.
///
/// JPEG carries no alpha channel, so a pixel that is not opaque is composited over white first.
pub fn encode(pm: &Pixmap) -> Outcome<Vec<u8>> {
	encode_with(pm, &Options::default())
}

/// Encodes a pixmap as a baseline JPEG.
///
/// JPEG carries no alpha channel, so a pixel that is not opaque is composited over white first.
pub fn encode_with(pm: &Pixmap, opts: &Options) -> Outcome<Vec<u8>> {
	if opts.quality < 1 || opts.quality > 100 {
		return Err(err!(
			"A JPEG quality runs from 1 to 100, and {} was asked for.", opts.quality;
		Invalid, Input, Range));
	}
	let (w, h) = (pm.width(), pm.height());
	let (hf, vf) = if opts.grey {
		(1, 1)
	} else {
		opts.chroma.factors()
	};
	let mcux = ceil_div(w, DCTSIZE * hf);
	let mcuy = ceil_div(h, DCTSIZE * vf);

	// The colour planes, at full size, over white.
	let src = pm.data();
	let n = w * h;
	let mut y = vec![0u8; n];
	let mut cb = vec![0u8; n];
	let mut cr = vec![0u8; n];
	for i in 0..n {
		let a = src[i * 4 + 3] as u32;
		let over = |c: u8| -> i32 {
			if a == 255 {
				c as i32
			} else {
				(((c as u32) * a + 255 * (255 - a) + 127) / 255) as i32
			}
		};
		let (r, g, b) = (over(src[i * 4]), over(src[i * 4 + 1]), over(src[i * 4 + 2]));
		// The forward colour transform, at sixteen fractional bits, as libjpeg's is.
		y[i] = clamp8((19595 * r + 38470 * g + 7471 * b + 32768) >> 16);
		cb[i] = clamp8(((-11056 * r - 21712 * g + 32768 * b + (128 << 16) + 32768) >> 16).clamp(0, 255));
		cr[i] = clamp8(((32768 * r - 27440 * g - 5328 * b + (128 << 16) + 32768) >> 16).clamp(0, 255));
	}

	let mut comps: Vec<EncComp> = Vec::new();
	comps.push(EncComp {
		id: 1,
		h: hf,
		v: vf,
		tbl: 0,
		data: pad_plane(&y, w, h, mcux * hf * DCTSIZE, mcuy * vf * DCTSIZE),
		stride: mcux * hf * DCTSIZE,
	});
	if !opts.grey {
		let cw = ceil_div(w, hf);
		let chh = ceil_div(h, vf);
		let (dcb, dcr) = (box_down(&cb, w, h, hf, vf), box_down(&cr, w, h, hf, vf));
		for (id, plane) in [(2u8, dcb), (3u8, dcr)] {
			comps.push(EncComp {
				id,
				h: 1,
				v: 1,
				tbl: 1,
				data: pad_plane(&plane, cw, chh, mcux * DCTSIZE, mcuy * DCTSIZE),
				stride: mcux * DCTSIZE,
			});
		}
	}

	let ql = scale_quant(&QUANT_LUMA, opts.quality);
	let qc = scale_quant(&QUANT_CHROMA, opts.quality);
	let quants = [ql, qc];

	let mut out: Vec<u8> = Vec::with_capacity(n / 4 + 1024);
	out.push(0xFF);
	out.push(SOI);

	// A JFIF segment, declaring square pixels of no particular density.
	seg(&mut out, APP0, &[
		b'J', b'F', b'I', b'F', 0x00,
		0x01, 0x01, // Version 1.1.
		0x00, // Density in no units.
		0x00, 0x01, 0x00, 0x01, // One by one.
		0x00, 0x00, // No thumbnail.
	]);

	// The quantisation tables, in zigzag order, at eight bits.
	let ntbl = if opts.grey { 1 } else { 2 };
	for (i, q) in quants.iter().enumerate().take(ntbl) {
		let mut body = Vec::with_capacity(1 + DCTSIZE2);
		body.push(i as u8);
		for k in 0..DCTSIZE2 {
			body.push(q[NATURAL[k]] as u8);
		}
		seg(&mut out, DQT, &body);
	}

	// The frame header.
	let mut body = Vec::with_capacity(8 + comps.len() * 3);
	body.push(8);
	body.push((h >> 8) as u8);
	body.push((h & 0xFF) as u8);
	body.push((w >> 8) as u8);
	body.push((w & 0xFF) as u8);
	body.push(comps.len() as u8);
	for c in &comps {
		body.push(c.id);
		body.push(((c.h as u8) << 4) | (c.v as u8));
		body.push(c.tbl as u8);
	}
	seg(&mut out, 0xC0, &body);

	// The Huffman tables.
	let dc_codes = [
		res!(Codes::new(&DC_LUMA_BITS, &DC_VALS)),
		res!(Codes::new(&DC_CHROMA_BITS, &DC_VALS)),
	];
	let ac_codes = [
		res!(Codes::new(&AC_LUMA_BITS, &AC_LUMA_VALS)),
		res!(Codes::new(&AC_CHROMA_BITS, &AC_CHROMA_VALS)),
	];
	let decl: &[(u8, &[u8; 17], &[u8])] = &[
		(0x00, &DC_LUMA_BITS, &DC_VALS),
		(0x10, &AC_LUMA_BITS, &AC_LUMA_VALS),
		(0x01, &DC_CHROMA_BITS, &DC_VALS),
		(0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALS),
	];
	for (tc, bits, vals) in decl.iter().take(if opts.grey { 2 } else { 4 }) {
		let mut body = Vec::with_capacity(17 + vals.len());
		body.push(*tc);
		body.extend_from_slice(&bits[1..17]);
		body.extend_from_slice(vals);
		seg(&mut out, DHT, &body);
	}

	// The scan header.
	let mut body = Vec::with_capacity(4 + comps.len() * 2);
	body.push(comps.len() as u8);
	for c in &comps {
		body.push(c.id);
		body.push(((c.tbl as u8) << 4) | (c.tbl as u8));
	}
	body.push(0); // First coefficient of the band.
	body.push(63); // Last coefficient.
	body.push(0); // No successive approximation.
	seg(&mut out, SOS, &body);

	// The entropy-coded data, one MCU at a time.
	let cos = cos_table();
	let mut bw = BitWriter::new();
	let mut pred = vec![0i32; comps.len()];
	for my in 0..mcuy {
		for mx in 0..mcux {
			for (ci, c) in comps.iter().enumerate() {
				for by in 0..c.v {
					for bx in 0..c.h {
						let px = (mx * c.h + bx) * DCTSIZE;
						let py = (my * c.v + by) * DCTSIZE;
						let blk = fdct(
							&c.data,
							py * c.stride + px,
							c.stride,
							&quants[c.tbl],
							&cos,
						);
						res!(emit_block(
							&mut bw,
							&blk,
							&dc_codes[c.tbl],
							&ac_codes[c.tbl],
							&mut pred[ci],
						));
					}
				}
			}
		}
	}
	out.extend_from_slice(&bw.finish());
	out.push(0xFF);
	out.push(EOI);
	Ok(out)
}

/// Writes one quantised block: the DC difference, then the AC coefficients in zigzag order.
fn emit_block(
	bw:	&mut BitWriter,
	blk:	&[i32; DCTSIZE2],
	dc:	&Codes,
	ac:	&Codes,
	pred:	&mut i32,
)
	-> Outcome<()>
{
	let diff = blk[0] - *pred;
	*pred = blk[0];
	let (s, bits) = category(diff);
	res!(bw.sym(dc, s));
	if s > 0 {
		bw.put(bits, s as u32);
	}

	let mut run = 0u8;
	for k in 1..DCTSIZE2 {
		let v = blk[NATURAL[k]];
		if v == 0 {
			run += 1;
			continue;
		}
		while run > 15 {
			res!(bw.sym(ac, 0xF0)); // A run of sixteen zeros.
			run -= 16;
		}
		let (s, bits) = category(v);
		if s > 10 {
			return Err(err!(
				"A quantised coefficient of {} needs magnitude category {}, and a baseline AC \
				coefficient runs to 10.", v, s;
			Bug, Invalid, Encode, Range));
		}
		res!(bw.sym(ac, (run << 4) | s));
		bw.put(bits, s as u32);
		run = 0;
	}
	if run > 0 {
		res!(bw.sym(ac, 0x00)); // End of block.
	}
	Ok(())
}

/// Copies a plane into a larger one, replicating its last row and column into the padding.
///
/// The padding is what the blocks beyond the image's edge are filled with, and replication is what
/// libjpeg uses: it costs the fewest bits, because it adds no edge for the transform to describe.
fn pad_plane(src: &[u8], w: usize, h: usize, pw: usize, ph: usize) -> Vec<u8> {
	let mut out = vec![0u8; pw * ph];
	for y in 0..ph {
		let sy = y.min(h - 1);
		for x in 0..pw {
			out[y * pw + x] = src[sy * w + x.min(w - 1)];
		}
	}
	out
}

/// Reduces a plane by averaging each box of `hf` by `vf` samples.
fn box_down(src: &[u8], w: usize, h: usize, hf: usize, vf: usize) -> Vec<u8> {
	if hf == 1 && vf == 1 {
		return src.to_vec();
	}
	let (dw, dh) = (ceil_div(w, hf), ceil_div(h, vf));
	let mut out = vec![0u8; dw * dh];
	let half = ((hf * vf) / 2) as u32;
	for y in 0..dh {
		for x in 0..dw {
			let mut sum = 0u32;
			for j in 0..vf {
				let sy = (y * vf + j).min(h - 1);
				for i in 0..hf {
					let sx = (x * hf + i).min(w - 1);
					sum += src[sy * w + sx] as u32;
				}
			}
			out[y * dw + x] = ((sum + half) / ((hf * vf) as u32)) as u8;
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A small pixmap with a hard edge in it, so that the AC coefficients are not all zero.
	fn sample(w: usize, h: usize) -> Outcome<Pixmap> {
		let mut pm = res!(Pixmap::new(w, h));
		let d = pm.data_mut();
		for y in 0..h {
			for x in 0..w {
				let at = (y * w + x) * 4;
				let on = (x / 4 + y / 4) % 2 == 0;
				d[at] = if on { 220 } else { 30 };
				d[at + 1] = ((x * 255) / w.max(1)) as u8;
				d[at + 2] = ((y * 255) / h.max(1)) as u8;
				d[at + 3] = 255;
			}
		}
		Ok(pm)
	}

	#[test]
	fn test_the_start_of_image_marker_is_checked_00() {
		assert!(decode(&[]).is_err());
		assert!(decode(&[0xFF]).is_err());
		assert!(decode(&[0x00, 0x00, 0x00, 0x00]).is_err());
		assert!(decode(b"\x89PNG\r\n\x1a\n").is_err(), "a PNG must not decode as a JPEG");
		assert!(dimensions(&[0xFF, 0xD8]).is_err(), "a file with no frame header has no size");
	}

	#[test]
	fn test_a_round_trip_keeps_the_size_and_the_colours_01() -> Outcome<()> {
		for (w, h) in [(1usize, 1usize), (8, 8), (17, 13), (33, 9), (64, 48)] {
			let pm = res!(sample(w, h));
			let opts = Options { quality: 95, chroma: Chroma::Full, grey: false };
			let buf = res!(encode_with(&pm, &opts));
			let back = res!(decode(&buf));
			assert_eq!(back.width(), w, "the width of a {} by {} round trip", w, h);
			assert_eq!(back.height(), h, "the height of a {} by {} round trip", w, h);
			let (pw, ph) = res!(dimensions(&buf));
			assert_eq!((pw, ph), (w, h), "the probe's size for {} by {}", w, h);
		}
		Ok(())
	}

	#[test]
	fn test_every_chroma_mode_round_trips_02() -> Outcome<()> {
		let pm = res!(sample(24, 20));
		for chroma in [Chroma::Full, Chroma::Half, Chroma::Quarter] {
			for grey in [false, true] {
				let opts = Options { quality: 80, chroma, grey };
				let buf = res!(encode_with(&pm, &opts));
				let back = res!(decode(&buf));
				assert_eq!(back.width(), 24, "{:?}, grey {}", chroma, grey);
				assert_eq!(back.height(), 20, "{:?}, grey {}", chroma, grey);
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_quality_outside_1_to_100_is_refused_03() -> Outcome<()> {
		let pm = res!(sample(8, 8));
		let opts = Options { quality: 0, chroma: Chroma::Full, grey: false };
		assert!(encode_with(&pm, &opts).is_err(), "quality 0 must be refused");
		let opts = Options { quality: 101, chroma: Chroma::Full, grey: false };
		assert!(encode_with(&pm, &opts).is_err(), "quality 101 must be refused");
		Ok(())
	}

	#[test]
	fn test_the_quality_scale_matches_the_published_tables_04() {
		// At quality 50 the tables are the ones of Annex K, unscaled; at 100 every divisor is 1.
		let at50 = scale_quant(&QUANT_LUMA, 50);
		assert_eq!(at50, QUANT_LUMA, "quality 50 is the table as published");
		let at100 = scale_quant(&QUANT_LUMA, 100);
		assert!(at100.iter().all(|v| *v == 1), "quality 100 divides by one");
		// A divisor never reaches zero, whatever the quality, since it is divided by.
		for q in 1..=100u8 {
			let t = scale_quant(&QUANT_CHROMA, q);
			assert!(t.iter().all(|v| *v >= 1), "quality {} produced a zero divisor", q);
			assert!(t.iter().all(|v| *v <= 255), "quality {} overflowed eight bits", q);
		}
	}

	#[test]
	fn test_the_magnitude_categories_are_the_specifications_05() {
		// The category is the number of bits the magnitude needs, and the bits that follow are the
		// value itself for a positive difference and its complement for a negative one.
		assert_eq!(category(0).0, 0);
		assert_eq!(category(1), (1, 1));
		assert_eq!(category(-1), (1, 0));
		assert_eq!(category(2), (2, 2));
		assert_eq!(category(-2), (2, 1));
		assert_eq!(category(-3), (2, 0));
		assert_eq!(category(255).0, 8);
		assert_eq!(category(-255), (8, 0));
		// EXTEND is the inverse.
		for v in [-2047i32, -300, -5, -1, 1, 5, 300, 2047] {
			let (s, bits) = category(v);
			assert_eq!(extend(bits, s as u32), v, "EXTEND must invert the category of {}", v);
		}
	}

	#[test]
	fn test_the_standard_huffman_tables_are_prefix_codes_06() -> Outcome<()> {
		// Deriving each table both ways checks the counts against the symbols, and would catch a
		// mistyped digit in either.
		for (bits, vals) in [
			(&DC_LUMA_BITS, &DC_VALS[..]),
			(&DC_CHROMA_BITS, &DC_VALS[..]),
			(&AC_LUMA_BITS, &AC_LUMA_VALS[..]),
			(&AC_CHROMA_BITS, &AC_CHROMA_VALS[..]),
		] {
			let total: usize = bits[1..17].iter().map(|c| *c as usize).sum();
			assert_eq!(total, vals.len(), "the counts and the symbols must agree");
			res!(Huff::new(bits, vals.to_vec()));
			res!(Codes::new(bits, vals));
		}
		Ok(())
	}

	#[test]
	fn test_a_huffman_table_that_is_not_a_prefix_code_is_refused_07() {
		// Three codes of length one is one more than a binary tree of that depth holds.
		let mut counts = [0u8; 17];
		counts[1] = 3;
		assert!(Huff::new(&counts, vec![1, 2, 3]).is_err(), "an over-full length must be refused");
		// Counts and symbols that disagree.
		let mut counts = [0u8; 17];
		counts[2] = 2;
		assert!(Huff::new(&counts, vec![1]).is_err(), "a short symbol list must be refused");
		// No codes at all.
		assert!(Huff::new(&[0u8; 17], Vec::new()).is_err(), "an empty table must be refused");
	}

	#[test]
	fn test_a_truncated_file_is_refused_or_read_but_never_panics_08() -> Outcome<()> {
		let pm = res!(sample(32, 24));
		let buf = res!(encode(&pm));
		for cut in [2, 8, 40, 100, buf.len() / 2, buf.len() - 4, buf.len() - 1] {
			if cut >= buf.len() {
				continue;
			}
			// Either answer is acceptable; what is not is a panic or an allocation the size of the
			// header's arithmetic rather than the file's.
			let _ = decode(&buf[..cut]);
			let _ = dimensions(&buf[..cut]);
			let _ = decode_eighth(&buf[..cut]);
		}
		Ok(())
	}

	#[test]
	fn test_a_corrupted_file_is_refused_or_read_but_never_panics_09() -> Outcome<()> {
		let pm = res!(sample(24, 24));
		let buf = res!(encode(&pm));
		// Walk a flipped bit through the file. Most land in entropy-coded data, where the result is a
		// wrong picture rather than an error, and the point is that it is a picture and not a panic.
		for i in (0..buf.len()).step_by(7) {
			let mut b = buf.clone();
			b[i] ^= 0x5A;
			let _ = decode(&b);
		}
		Ok(())
	}

	#[test]
	fn test_the_modes_this_codec_does_not_implement_are_refused_by_name_10() -> Outcome<()> {
		let pm = res!(sample(16, 16));
		let buf = res!(encode(&pm));
		// The frame header this encoder wrote, found by its marker.
		let sof = match buf.windows(2).position(|w| w[0] == 0xFF && w[1] == 0xC0) {
			Some(i) => i,
			None => return Err(err!("The encoder wrote no SOF0 marker."; Test, Missing)),
		};

		// Arithmetic coding, lossless, and hierarchical modes each name themselves when refused.
		for (m, want) in [
			(0xC9u8, "arithmetic"),
			(0xC3, "lossless"),
			(0xC5, "differential"),
		] {
			let mut b = buf.clone();
			b[sof + 1] = m;
			match decode(&b) {
				Ok(_) => return Err(err!(
					"A frame marked {:#04X} decoded, and this codec does not implement it.", m;
				Test, Invalid)),
				Err(e) => {
					let s = fmt!("{}", e);
					assert!(
						s.contains(want),
						"refusing {:#04X} should name it '{}', and it said: {}", m, want, s,
					);
				},
			}
		}

		// Twelve bits a sample.
		let mut b = buf.clone();
		b[sof + 4] = 12; // The precision byte, after the marker and the two length bytes.
		match decode(&b) {
			Ok(_) => return Err(err!("A twelve-bit frame decoded."; Test, Invalid)),
			Err(e) => {
				let s = fmt!("{}", e);
				assert!(s.contains("12 bits"), "refusing 12 bits should say so: {}", s);
			},
		}
		Ok(())
	}

	#[test]
	fn test_the_inverse_dct_of_a_flat_block_is_flat_11() {
		// A block whose only coefficient is the DC one is a constant, and the constant is the DC
		// coefficient divided by eight, plus the level shift of 128.
		let q = [1u16; DCTSIZE2];
		for dc in [-1024i16, -8, 0, 8, 800] {
			let mut coef = [0i16; DCTSIZE2];
			coef[0] = dc;
			let mut out = [0u8; DCTSIZE2];
			idct(&coef, &q, &mut out, 0, DCTSIZE);
			let want = clamp8(((dc as i32) + 4) / 8 + 128);
			// The rounding of a division that truncates towards zero differs by one below zero.
			let want = if dc < 0 {
				clamp8(((dc as i32) + 4).div_euclid(8) + 128)
			} else {
				want
			};
			for v in out {
				assert_eq!(v, want, "a block with only DC {} must be flat at {}", dc, want);
			}
			assert_eq!(idct_dc(&coef, &q), want, "the DC-only path must agree with the full one");
		}
	}

	#[test]
	fn test_an_absurd_frame_header_is_refused_12() -> Outcome<()> {
		let pm = res!(sample(8, 8));
		let buf = res!(encode(&pm));
		let sof = match buf.windows(2).position(|w| w[0] == 0xFF && w[1] == 0xC0) {
			Some(i) => i,
			None => return Err(err!("The encoder wrote no SOF0 marker."; Test, Missing)),
		};
		// 65535 by 65535 is 4.29 billion pixels, over the ceiling.
		let mut b = buf.clone();
		b[sof + 5] = 0xFF;
		b[sof + 6] = 0xFF;
		b[sof + 7] = 0xFF;
		b[sof + 8] = 0xFF;
		assert!(decode(&b).is_err(), "a frame over the pixel ceiling must be refused");
		assert!(dimensions(&b).is_err(), "the probe must refuse it too, before it allocates");
		// A height of zero, which means the line count arrives in a DNL segment.
		let mut b = buf.clone();
		b[sof + 5] = 0;
		b[sof + 6] = 0;
		assert!(decode(&b).is_err(), "a frame with no height must be refused");
		Ok(())
	}

	#[test]
	fn test_a_zero_quantisation_divisor_is_refused_13() -> Outcome<()> {
		let pm = res!(sample(8, 8));
		let buf = res!(encode(&pm));
		let dqt = match buf.windows(2).position(|w| w[0] == 0xFF && w[1] == DQT) {
			Some(i) => i,
			None => return Err(err!("The encoder wrote no DQT marker."; Test, Missing)),
		};
		let mut b = buf.clone();
		b[dqt + 5] = 0; // The first divisor, after the marker, the length and the slot byte.
		assert!(decode(&b).is_err(), "a zero divisor must be refused, not divided by");
		Ok(())
	}

	#[test]
	fn test_the_eighth_scale_decode_is_an_eighth_of_the_size_14() -> Outcome<()> {
		for (w, h) in [(1usize, 1usize), (8, 8), (9, 9), (17, 13), (64, 48)] {
			let pm = res!(sample(w, h));
			let buf = res!(encode(&pm));
			let small = res!(decode_eighth(&buf));
			assert_eq!(small.width(), ceil_div(w, 8), "the eighth-scale width of {} by {}", w, h);
			assert_eq!(small.height(), ceil_div(h, 8), "the eighth-scale height of {} by {}", w, h);
		}
		Ok(())
	}

	#[test]
	fn test_alpha_is_composited_over_white_15() -> Outcome<()> {
		// JPEG has no alpha channel, so a transparent pixel has to become something. White is what a
		// viewer expects, and a decoder that silently kept the colour under the transparency would
		// produce a picture nobody drew.
		let mut pm = res!(Pixmap::new(16, 16));
		pm.fill(crate::colour::Rgba::new(255, 0, 0, 0)); // Fully transparent red.
		let opts = Options { quality: 95, chroma: Chroma::Full, grey: false };
		let buf = res!(encode_with(&pm, &opts));
		let back = res!(decode(&buf));
		let c = match back.pixel(8, 8) {
			Some(c) => c,
			None => return Err(err!("A 16 by 16 pixmap has a pixel at 8, 8."; Test, Missing)),
		};
		assert!(
			c.r > 250 && c.g > 250 && c.b > 250,
			"transparent red should encode as white, and came back as {:?}", c,
		);
		assert_eq!(c.a, 255, "a decoded JPEG is opaque");
		Ok(())
	}

	#[test]
	fn test_a_jfif_segment_outranks_an_adobe_one_16() -> Outcome<()> {
		// Files exist carrying both a JFIF segment and an Adobe segment declaring no transform. JFIF
		// is defined as YCbCr, so it settles the question and the Adobe segment is not consulted;
		// reading the Adobe segment first turns such a file into false colour, red for blue.
		let pm = res!(Pixmap::filled(32, 32, crate::colour::Rgba::new(220, 30, 40, 255)));
		let opts = Options { quality: 95, chroma: Chroma::Full, grey: false };
		let buf = res!(encode_with(&pm, &opts));

		// An Adobe APP14 segment declaring transform 0, spliced in after the JFIF segment.
		let adobe: [u8; 16] = [
			0xFF, 0xEE, 0x00, 0x0E,
			b'A', b'd', b'o', b'b', b'e',
			0x00, 0x64, // Version.
			0x00, 0x00, 0x00, 0x00, // Two flag words.
			0x00, // Transform: none.
		];
		let at = match buf.windows(2).position(|w| w[0] == 0xFF && w[1] == DQT) {
			Some(i) => i,
			None => return Err(err!("The encoder wrote no DQT marker."; Test, Missing)),
		};
		let mut spliced = Vec::with_capacity(buf.len() + adobe.len());
		spliced.extend_from_slice(&buf[..at]);
		spliced.extend_from_slice(&adobe);
		spliced.extend_from_slice(&buf[at..]);

		let back = res!(decode(&spliced));
		let c = match back.pixel(16, 16) {
			Some(c) => c,
			None => return Err(err!("A 32 by 32 pixmap has a pixel at 16, 16."; Test, Missing)),
		};
		assert!(
			(c.r as i32 - 220).abs() < 8 && (c.g as i32 - 30).abs() < 8 && (c.b as i32 - 40).abs() < 8,
			"a file with both segments is YCbCr, so it should decode near (220, 30, 40), not {:?}", c,
		);
		Ok(())
	}

	#[test]
	fn test_a_truncated_scan_leaves_the_rest_mid_grey_17() -> Outcome<()> {
		// When the entropy-coded data runs out, the coefficients not yet read stay at zero, which the
		// inverse DCT renders as a flat mid-grey. Carrying on with whatever the zero padding decodes
		// to would fill the tail of the picture with noise instead, which is what this codec did
		// until a truncated photograph was decoded beside another implementation's reading of it.
		let pm = res!(sample(64, 64));
		let opts = Options { quality: 90, chroma: Chroma::Full, grey: false };
		let buf = res!(encode_with(&pm, &opts));
		let sos = match buf.windows(2).position(|w| w[0] == 0xFF && w[1] == SOS) {
			Some(i) => i,
			None => return Err(err!("The encoder wrote no SOS marker."; Test, Missing)),
		};
		// Keep the scan header and a little of its data, and drop the rest along with the EOI.
		let cut = (sos + 40).min(buf.len());
		let back = res!(decode(&buf[..cut]));
		assert_eq!(back.width(), 64);
		assert_eq!(back.height(), 64);
		for x in 0..64 {
			let c = match back.pixel(x, 63) {
				Some(c) => c,
				None => return Err(err!("No pixel at ({}, 63).", x; Test, Missing)),
			};
			assert_eq!(
				(c.r, c.g, c.b), (128, 128, 128),
				"the last row of a truncated file is mid-grey, and pixel {} is {:?}", x, c,
			);
		}
		Ok(())
	}

	/// A three by three grid of blocks whose DC values ramp from left to right.
	fn ramp_blocks() -> Comp {
		let mut c = Comp {
			id: 1, h: 1, v: 1, tq: 0,
			dw: 24, dh: 24, bw: 3, bh: 3, bwp: 3, bhp: 3,
			coef: vec![0i16; 9 * DCTSIZE2],
		};
		for by in 0..3 {
			for bx in 0..3 {
				c.coef[(by * 3 + bx) * DCTSIZE2] = (100 + 100 * bx) as i16;
			}
		}
		c
	}

	#[test]
	fn test_block_smoothing_estimates_from_the_neighbouring_means_18() {
		// The middle block of a left-to-right ramp. Annex K.8 estimates the first horizontal AC
		// coefficient as 36 * Q00 * (left - right), scaled by its own quantiser: with Q00 of 16, a
		// ramp of 100 to 300 and a quantiser of 11 that is -41 before the approximation clamp.
		let c = ramp_blocks();
		let mut q = [1u16; DCTSIZE2];
		q[0] = 16;
		q[1] = 11;
		let mut seen = [0i8; DCTSIZE2];

		// A coefficient no scan ever carried takes the estimate whole.
		seen[1] = -1;
		let mut ws = [0i16; DCTSIZE2];
		ws.copy_from_slice(&c.coef[DCTSIZE2 * 4..DCTSIZE2 * 5]);
		smooth(&mut ws, &c.coef, &c, 1, 1, &q, &seen);
		assert_eq!(ws[1], -41, "the estimate follows the ramp, and downwards");

		// A coefficient received down to bit 1 is known to within two, so the estimate is clamped.
		seen[1] = 1;
		let mut ws = [0i16; DCTSIZE2];
		smooth(&mut ws, &c.coef, &c, 1, 1, &q, &seen);
		assert_eq!(ws[1], -1, "an estimate may not exceed what the scans left undetermined");

		// A coefficient a scan pinned down exactly is never estimated.
		seen[1] = 0;
		let mut ws = [0i16; DCTSIZE2];
		smooth(&mut ws, &c.coef, &c, 1, 1, &q, &seen);
		assert_eq!(ws[1], 0, "a coefficient known exactly must be left alone");

		// Nor is one that already carries a value.
		seen[1] = -1;
		let mut ws = [0i16; DCTSIZE2];
		ws[1] = 7;
		smooth(&mut ws, &c.coef, &c, 1, 1, &q, &seen);
		assert_eq!(ws[1], 7, "a coefficient already received must be left alone");
	}

	#[test]
	fn test_block_smoothing_applies_only_where_it_can_help_19() -> Outcome<()> {
		// A sequential frame is never smoothed, and neither is a progressive one whose scans all
		// reached the last approximation bit: there is then nothing left to estimate.
		let pm = res!(sample(32, 32));
		let buf = res!(encode(&pm));
		let r = res!(parse(&buf));
		let frame = match r.frame {
			Some(f) => f,
			None => return Err(err!("The file carries no frame header."; Test, Missing)),
		};
		assert!(!smoothing_helps(&frame), "a sequential frame is never smoothed");
		Ok(())
	}
}
