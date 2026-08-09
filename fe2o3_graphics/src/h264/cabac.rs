//! The arithmetic entropy coder, and the context variables it codes against.
//!
//! CABAC is the entropy coder H.264 uses when `entropy_coding_mode_flag` is on, and **947 films in
//! the corpus use it** -- every Main and every High one. Nothing about it resembles the code tables
//! beside it in [`super::cavlc`]. Every syntax element is a string of binary decisions, and each
//! decision is coded against a probability that adapts as the picture is read. Three kinds of
//! decision exist: one against a context variable, which is the usual one; one *bypassed* at even
//! odds, for the parts of a value that carry no useful correlation; and one *terminating*, which is
//! how the end of a slice and the raw-sample macroblock type are found.
//!
//! # What is in here
//!
//! The decoding engine of §9.3.3.2 and its two published tables, the context variables of §9.3.1.1
//! and their initialisation values, and the reading of one block of coefficients (§7.3.5.3.3). The
//! binarisation of everything above a block -- macroblock type, coded block pattern, quantisation
//! delta, prediction modes -- needs a macroblock's neighbours and so lives in [`super::decode`]
//! beside them.
//!
//! # The initialisation tables, and why they are the dangerous part
//!
//! A context variable starts from a pair of signed numbers `(m, n)` and the slice's quantisation
//! parameter (§9.3.1.1). There are 261 such pairs for an intra 4:2:0 slice, spread over eight
//! published tables, and **a wrong one produces a picture rather than an error**: the decoder stays
//! in step with the encoder, reads the same number of bins, and hands back samples that look
//! decoded. So they are held to the specification itself rather than to this decoder, entry by
//! entry, in this module's tests -- and separately to FFmpeg over the whole library, which is the
//! only check that exercises the values rather than merely comparing them.
//!
//! The engine's own two tables, `rangeTabLPS` (Table 9-44) and the state transitions (Table 9-45),
//! are the same numbers HEVC publishes as its Tables 9-46 and 9-47. They are transcribed again here
//! rather than borrowed, because the two codecs' entropy layers are independent and a shared table
//! would tie one to the other; the tests read both out of the H.264 specification.
//!
//! # What an intra slice does not need
//!
//! The specification gives four initialisation columns: one for I and SI slices, and three chosen by
//! `cabac_init_idc` for P, SP and B. Only the first is here. So are only the block categories a
//! 4:2:0 picture has -- 0 to 5 of Table 9-42 -- which is why Tables 9-14, 9-15, 9-22, 9-23 and
//! 9-25 to 9-33 are absent: they serve motion vectors, field coding, and the Cb and Cr blocks of a
//! 4:4:4 picture. An 8x8 luma block in 4:2:0 carries no `coded_block_flag` at all (§7.3.5.3.3), so
//! Table 9-33 is not needed either.

use oxedyne_fe2o3_core::prelude::*;

/// The probability state a context variable is in: an index 0 to 62, and the more probable symbol.
///
/// One byte rather than two fields, because a slice carries over a thousand of these and they are
/// initialised together at the head of every slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ctx(u8);

impl Ctx {

	/// The state a context starts in, from its initialisation pair and the slice's quantisation
	/// parameter (§9.3.1.1, equations 9-5 and 9-6).
	///
	/// The clamp on the quantisation parameter is the specification's own, and the clamp on
	/// `preCtxState` to 1..126 is what keeps the state index inside the 64 rows the probability
	/// tables have.
	pub fn start(m: i8, n: i8, qp: i32) -> Self {
		let q = qp.clamp(0, 51);
		let pre = (((m as i32 * q) >> 4) + n as i32).clamp(1, 126);
		if pre <= 63 {
			// The less probable symbol is a one.
			Self(((63 - pre) as u8) << 1)
		} else {
			Self((((pre - 64) as u8) << 1) | 1)
		}
	}

	/// The probability state index, 0 to 62.
	fn state(self) -> usize {
		(self.0 >> 1) as usize
	}

	/// The more probable symbol, 0 or 1.
	fn mps(self) -> u32 {
		(self.0 & 1) as u32
	}
}

/// How the range is narrowed for the less probable symbol, by state and by the two bits the current
/// range contributes (§9.3.3.2.1, Table 9-44).
const LPS: [[u8; 4]; 64] = [
	[128, 176, 208, 240],	[128, 167, 197, 227],	[128, 158, 187, 216],	[123, 150, 178, 205],
	[116, 142, 169, 195],	[111, 135, 160, 185],	[105, 128, 152, 175],	[100, 122, 144, 166],
	[95, 116, 137, 158],	[90, 110, 130, 150],	[85, 104, 123, 142],	[81, 99, 117, 135],
	[77, 94, 111, 128],	[73, 89, 105, 122],	[69, 85, 100, 116],	[66, 80, 95, 110],
	[62, 76, 90, 104],	[59, 72, 86, 99],	[56, 69, 81, 94],	[53, 65, 77, 89],
	[51, 62, 73, 85],	[48, 59, 69, 80],	[46, 56, 66, 76],	[43, 53, 63, 72],
	[41, 50, 59, 69],	[39, 48, 56, 65],	[37, 45, 54, 62],	[35, 43, 51, 59],
	[33, 41, 48, 56],	[32, 39, 46, 53],	[30, 37, 43, 50],	[29, 35, 41, 48],
	[27, 33, 39, 45],	[26, 31, 37, 43],	[24, 30, 35, 41],	[23, 28, 33, 39],
	[22, 27, 32, 37],	[21, 26, 30, 35],	[20, 24, 29, 33],	[19, 23, 27, 31],
	[18, 22, 26, 30],	[17, 21, 25, 28],	[16, 20, 23, 27],	[15, 19, 22, 25],
	[14, 18, 21, 24],	[14, 17, 20, 23],	[13, 16, 19, 22],	[12, 15, 18, 21],
	[12, 14, 17, 20],	[11, 14, 16, 19],	[11, 13, 15, 18],	[10, 12, 15, 17],
	[10, 12, 14, 16],	[9, 11, 13, 15],	[9, 11, 12, 14],	[8, 10, 12, 14],
	[8, 9, 11, 13],		[7, 9, 11, 12],		[7, 9, 10, 12],		[7, 8, 10, 11],
	[6, 8, 9, 11],		[6, 7, 9, 10],		[6, 7, 8, 9],		[2, 2, 2, 2],
];

/// The state to move to after decoding the more probable symbol (Table 9-45, `transIdxMPS`).
const NEXT_MPS: [u8; 64] = [
	1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
	17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
	33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
	49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

/// The state to move to after decoding the less probable symbol (Table 9-45, `transIdxLPS`).
const NEXT_LPS: [u8; 64] = [
	0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12,
	13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24,
	24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33,
	33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

/// The arithmetic decoder itself (§9.3.3.2).
///
/// It reads bits and answers questions of the form "was the next bin a one?", where the odds are
/// carried by whichever context variable the syntax says applies.
pub struct Cabac<'a> {
	/// The bytes being read, from the first byte of the slice's entropy-coded data.
	buf:	&'a [u8],
	/// The next byte to be taken into the window.
	at:	usize,
	/// The current interval's width, `codIRange`.
	range:	u32,
	/// Where in the interval the coded value sits, `codIOffset`, shifted up by [`Cabac::bits`].
	offset:	u32,
	/// How many bits of the window are pre-read and not yet part of `codIOffset`.
	bits:	i32,
}

impl<'a> Cabac<'a> {

	/// Starts the decoder at the first byte of a slice's entropy-coded data (§9.3.1.2).
	///
	/// The specification reads nine bits into `codIOffset` and compares it against the interval
	/// directly, reading one more bit at every renormalisation. This decoder keeps those bits
	/// **pre-read** instead -- `offset` holds `codIOffset` shifted up by `bits`, with `bits` more of
	/// the stream already in the low end -- so a renormalisation is a subtraction from `bits` rather
	/// than a read, and the two bytes go in whole.
	pub fn new(buf: &'a [u8]) -> Outcome<Self> {
		if buf.len() < 2 {
			return Err(err!(
				"An arithmetic decoder was started on {} bytes, and it reads two before it answers \
				anything.", buf.len();
			Invalid, Input, Decode));
		}
		Ok(Self {
			buf,
			at:	2,
			range:	510,
			offset:	((buf[0] as u32) << 8) | (buf[1] as u32),
			bits:	7,
		})
	}

	/// The next byte, or zeroes past the end.
	///
	/// A decoder is allowed to read a little past the last byte of a slice -- the final bins are
	/// coded against bits the encoder never had to write -- so running out is not a fault. Reading
	/// far past it would be, and that is caught by the terminating bin, which says where the data
	/// ends.
	fn byte(&mut self) -> u32 {
		let b = self.buf.get(self.at).copied().unwrap_or(0) as u32;
		self.at += 1;
		b
	}

	/// Keeps the interval at nine bits or more (§9.3.3.2.2).
	fn renormalise(&mut self) {
		while self.range < 256 {
			self.range <<= 1;
			self.bits -= 1;
			if self.bits < 0 {
				self.offset = (self.offset << 8) | self.byte();
				self.bits += 8;
			}
		}
	}

	/// One bin against a context, which is then moved on (§9.3.3.2.1).
	pub fn bin(&mut self, ctx: &mut Ctx) -> u32 {
		let state = ctx.state();
		let lps = LPS[state][((self.range >> 6) & 3) as usize] as u32;
		self.range -= lps;
		let value;
		if self.offset >= (self.range << self.bits) {
			// The less probable symbol.
			self.offset -= self.range << self.bits;
			value = 1 - ctx.mps();
			self.range = lps;
			if state == 0 {
				// State nought is where the two symbols are equally likely, so being wrong there
				// exchanges which one is called the more probable.
				ctx.0 ^= 1;
			}
			ctx.0 = (NEXT_LPS[state] << 1) | (ctx.0 & 1);
		} else {
			value = ctx.mps();
			ctx.0 = (NEXT_MPS[state] << 1) | (ctx.0 & 1);
		}
		self.renormalise();
		value
	}

	/// One bin at even odds, with no context to move on (§9.3.3.2.3).
	pub fn bypass(&mut self) -> u32 {
		self.bits -= 1;
		if self.bits < 0 {
			self.offset = (self.offset << 8) | self.byte();
			self.bits += 8;
		}
		let scaled = self.range << self.bits;
		if self.offset >= scaled {
			self.offset -= scaled;
			1
		} else {
			0
		}
	}

	/// `n` bins at even odds, most significant first.
	pub fn bypass_bits(&mut self, n: usize) -> u32 {
		let mut v = 0u32;
		for _ in 0..n.min(32) {
			v = (v << 1) | self.bypass();
		}
		v
	}

	/// The bin that says whether this is the end (§9.3.3.2.4).
	///
	/// One at the end of a slice, and one for the bin of `mb_type` that names the raw-sample
	/// macroblock. Where it says the end has come, the interval is **not** renormalised, which is
	/// what leaves the bitstream pointer where the next syntax begins.
	pub fn terminate(&mut self) -> u32 {
		self.range -= 2;
		if self.offset >= (self.range << self.bits) {
			1
		} else {
			self.renormalise();
			0
		}
	}

	/// How many bits of the buffer the decoder has taken into `codIOffset`.
	///
	/// This is where the bitstream pointer sits, which a raw-sample macroblock needs: its samples
	/// begin at the next byte boundary after the terminating bin that named it.
	pub fn consumed_bits(&self) -> usize {
		(self.at * 8).saturating_sub(self.bits.max(0) as usize)
	}

	/// The interval's current width, for the tests that assert it stays in range.
	#[cfg(test)]
	fn width(&self) -> u32 {
		self.range
	}
}

// ------------------------------------------------------- the context variables themselves

/// How many context variables the syntax numbers, `ctxIdx` running 0 to 1023 (§9.3.3.1).
pub const CONTEXTS: usize = 1024;

/// The `ctxIdx` of `end_of_slice_flag`, and of the `mb_type` bin that names a raw-sample macroblock.
///
/// It carries no context variable at all: both are decoded by the terminating process.
pub const TERMINATE: usize = 276;

/// The `ctxIdxOffset` of each syntax element an intra 4:2:0 slice codes (Table 9-34).
pub mod offset {
	/// `mb_type` in an I slice.
	pub const MB_TYPE: usize = 3;
	/// `mb_qp_delta`.
	pub const MB_QP_DELTA: usize = 60;
	/// `intra_chroma_pred_mode`.
	pub const CHROMA_PRED: usize = 64;
	/// `prev_intra4x4_pred_mode_flag` and `prev_intra8x8_pred_mode_flag`.
	pub const PREV_PRED: usize = 68;
	/// `rem_intra4x4_pred_mode` and `rem_intra8x8_pred_mode`.
	pub const REM_PRED: usize = 69;
	/// The luma part of `coded_block_pattern`.
	pub const CBP_LUMA: usize = 73;
	/// Its chroma part.
	pub const CBP_CHROMA: usize = 77;
	/// `transform_size_8x8_flag`.
	pub const TRANSFORM_8X8: usize = 399;
}

/// The initialisation pairs an **intra** slice's context variables start from, as runs of
/// `(first ctxIdx, table number within clause 9, [(m, n), ...])`.
///
/// The table number is kept so that the transcription can be checked against the document rather
/// than against itself; this module's tests do exactly that where a text rendering of the
/// specification is to hand. Each run is the whole of that table's I-slice column, even where an
/// intra 4:2:0 decoder reads only part of it -- `mb_field_decoding_flag` at 70 to 72 is never coded
/// in a frame-only stream, and `significant_coeff_flag` stops at 151 rather than 165 -- because a
/// partial run would be a second place to make a mistake.
pub const INIT_I: [(usize, usize, &[(i8, i8)]); 8] = [
	// Table 9-12 gives ctxIdx 0 to 10 and has no cabac_init_idc column at all; 0 to 2 belong to
	// mb_type in an SI slice, so an I slice starts at 3.
	(3, 12, &[
		(20, -15),	(2, 54),	(3, 74),	(-28, 127),
		(-23, 104),	(-6, 53),	(-1, 54),	(7, 51),
	]),
	// Table 9-17: mb_qp_delta at 60 to 63, intra_chroma_pred_mode at 64 to 67, and the two
	// prediction mode elements at 68 and 69.
	(60, 17, &[
		(0, 41),	(0, 63),	(0, 63),	(0, 63),	(-9, 83),
		(4, 86),	(0, 97),	(-7, 72),	(13, 41),	(3, 62),
	]),
	// Table 9-18: mb_field_decoding_flag, coded_block_pattern and coded_block_flag.
	(70, 18, &[
		(0, 11),	(1, 55),	(0, 69),	(-17, 127),	(-13, 102),
		(0, 82),	(-7, 74),	(-21, 107),	(-27, 127),	(-31, 127),
		(-24, 127),	(-18, 95),	(-27, 127),	(-21, 114),	(-30, 127),
		(-17, 123),	(-12, 115),	(-16, 122),	(-11, 115),	(-12, 63),
		(-2, 68),	(-15, 84),	(-13, 104),	(-3, 70),	(-8, 93),
		(-10, 90),	(-30, 127),	(-1, 74),	(-6, 97),	(-7, 91),
		(-20, 127),	(-4, 56),	(-5, 82),	(-7, 76),	(-22, 125),
	]),
	// Table 9-19: significant_coeff_flag for a frame-coded block of category below five.
	(105, 19, &[
		(-7, 93),	(-11, 87),	(-3, 77),	(-5, 71),	(-4, 63),
		(-4, 68),	(-12, 84),	(-7, 62),	(-7, 65),	(8, 61),
		(5, 56),	(-2, 66),	(1, 64),	(0, 61),	(-2, 78),
		(1, 50),	(7, 52),	(10, 35),	(0, 44),	(11, 38),
		(1, 45),	(0, 46),	(5, 44),	(31, 17),	(1, 51),
		(7, 50),	(28, 19),	(16, 33),	(14, 62),	(-13, 108),
		(-15, 100),	(-13, 101),	(-13, 91),	(-12, 94),	(-10, 88),
		(-16, 84),	(-10, 86),	(-7, 83),	(-13, 87),	(-19, 94),
		(1, 70),	(0, 72),	(-5, 74),	(18, 59),	(-8, 102),
		(-15, 100),	(0, 95),	(-4, 75),	(2, 72),	(-11, 75),
		(-3, 71),	(15, 46),	(-13, 69),	(0, 62),	(0, 65),
		(21, 37),	(-15, 72),	(9, 57),	(16, 54),	(0, 62),
		(12, 72),
	]),
	// Table 9-20: last_significant_coeff_flag for the same blocks.
	(166, 20, &[
		(24, 0),	(15, 9),	(8, 25),	(13, 18),	(15, 9),
		(13, 19),	(10, 37),	(12, 18),	(6, 29),	(20, 33),
		(15, 30),	(4, 45),	(1, 58),	(0, 62),	(7, 61),
		(12, 38),	(11, 45),	(15, 39),	(11, 42),	(13, 44),
		(16, 45),	(12, 41),	(10, 49),	(30, 34),	(18, 42),
		(10, 55),	(17, 51),	(17, 46),	(0, 89),	(26, -19),
		(22, -17),	(26, -17),	(30, -25),	(28, -20),	(33, -23),
		(37, -27),	(33, -23),	(40, -28),	(38, -17),	(33, -11),
		(40, -15),	(41, -6),	(38, 1),	(41, 17),	(30, -6),
		(27, 3),	(26, 22),	(37, -16),	(35, -4),	(38, -8),
		(38, -3),	(37, 3),	(38, 5),	(42, 0),	(35, 16),
		(39, 22),	(14, 48),	(27, 37),	(21, 60),	(12, 68),
		(2, 97),
	]),
	// Table 9-21: coeff_abs_level_minus1 for the same blocks.
	(227, 21, &[
		(-3, 71),	(-6, 42),	(-5, 50),	(-3, 54),	(-2, 62),
		(0, 58),	(1, 63),	(-2, 72),	(-1, 74),	(-9, 91),
		(-5, 67),	(-5, 27),	(-3, 39),	(-2, 44),	(0, 46),
		(-16, 64),	(-8, 68),	(-10, 78),	(-6, 77),	(-10, 86),
		(-12, 92),	(-15, 55),	(-10, 60),	(-6, 62),	(-4, 65),
		(-12, 73),	(-8, 76),	(-7, 80),	(-9, 88),	(-17, 110),
		(-11, 97),	(-20, 84),	(-11, 79),	(-6, 73),	(-4, 74),
		(-13, 86),	(-13, 96),	(-11, 97),	(-19, 117),	(-8, 78),
		(-5, 33),	(-4, 48),	(-2, 53),	(-3, 62),	(-13, 71),
		(-10, 79),	(-12, 86),	(-13, 90),	(-14, 97),
	]),
	// Table 9-16 gives transform_size_8x8_flag its own three, and its I-slice row is the only one
	// of the four columns that is not "na" for 54 to 59.
	(399, 16, &[
		(31, 21),	(31, 31),	(25, 50),
	]),
	// Table 9-24: the residual of an eight-by-eight luma block -- significance at 402 to 416, the
	// last position at 417 to 425, the levels at 426 to 435. Its column is headed "I slices" rather
	// than "I and SI slices", since an SI slice has no eight-by-eight transform.
	(402, 24, &[
		(-17, 120),	(-20, 112),	(-18, 114),	(-11, 85),	(-15, 92),
		(-14, 89),	(-26, 71),	(-15, 81),	(-14, 80),	(0, 68),
		(-14, 70),	(-24, 56),	(-23, 68),	(-24, 50),	(-11, 74),
		(23, -13),	(26, -13),	(40, -15),	(49, -14),	(44, 3),
		(45, 6),	(44, 34),	(33, 54),	(19, 82),	(-3, 75),
		(-1, 23),	(1, 34),	(1, 43),	(0, 54),	(-2, 55),
		(0, 61),	(1, 64),	(0, 68),	(-9, 92),
	]),
];

/// Every context variable a slice carries, and which of them the initialisation tables reached.
///
/// One flat array indexed by `ctxIdx`, because that is how the syntax names them: a context is
/// `ctxIdxOffset` plus an increment worked out from the neighbours, and there is nothing to be
/// gained by grouping them.
#[derive(Clone)]
pub struct Contexts {
	/// The state of each context variable.
	v:	Vec<Ctx>,
	/// Whether the intra tables carry an initialisation value for it.
	known:	Vec<bool>,
}

impl Contexts {

	/// The state every context an intra slice uses starts in, given that slice's quantisation
	/// parameter (§9.3.1.1).
	pub fn start(qp: i32) -> Self {
		let mut v = vec![Ctx(0); CONTEXTS];
		let mut known = vec![false; CONTEXTS];
		for (first, _table, values) in INIT_I {
			for (i, (m, n)) in values.iter().enumerate() {
				v[first + i] = Ctx::start(*m, *n, qp);
				known[first + i] = true;
			}
		}
		Self { v, known }
	}

	/// One context variable, by the `ctxIdx` the syntax names.
	///
	/// A `ctxIdx` the intra tables never initialised is a fault in whoever worked out the increment,
	/// not a thing to be read quietly: a decoder that codes a bin against an uninitialised context
	/// produces a picture rather than an error, and a picture that is subtly wrong is the hardest
	/// kind of fault to find. So it is refused, and the message says which index.
	pub fn at(&mut self, ctx_idx: usize) -> Outcome<&mut Ctx> {
		match self.known.get(ctx_idx) {
			Some(true) => Ok(&mut self.v[ctx_idx]),
			_ => Err(err!(
				"A bin was to be coded against context {}, and the intra initialisation tables of \
				clause 9.3.1.1 do not carry it. Either the context increment is wrong or the stream \
				codes something this decoder does not read.", ctx_idx;
			Invalid, Input, Decode)),
		}
	}

	/// One bin against the context at a `ctxIdx`.
	pub fn bin(&mut self, c: &mut Cabac, ctx_idx: usize) -> Outcome<u32> {
		let ctx = res!(self.at(ctx_idx));
		Ok(c.bin(ctx))
	}
}

// ------------------------------------------------------------------ a block of coefficients

/// Which family of block a residual belongs to, which chooses its context variables (Table 9-42).
///
/// Only the six a 4:2:0 picture has. Categories 6 to 13 are the Cb and Cr blocks of a 4:4:4 picture,
/// which this decoder refuses where the chroma format is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
	/// The sixteen direct current terms of a macroblock predicted whole (`ctxBlockCat` 0).
	LumaDc,
	/// The fifteen alternating current terms of one of its blocks (1).
	LumaAc,
	/// A whole four-by-four luma block (2).
	Luma4x4,
	/// The four direct current terms of one colour difference component (3).
	ChromaDc,
	/// The fifteen alternating current terms of one of its blocks (4).
	ChromaAc,
	/// A whole eight-by-eight luma block (5).
	Luma8x8,
}

impl Cat {

	/// The `ctxBlockCat` the specification numbers this category with.
	pub fn number(self) -> usize {
		match self {
			Self::LumaDc	=> 0,
			Self::LumaAc	=> 1,
			Self::Luma4x4	=> 2,
			Self::ChromaDc	=> 3,
			Self::ChromaAc	=> 4,
			Self::Luma8x8	=> 5,
		}
	}

	/// How many coefficients the block holds, `maxNumCoeff` (Table 9-42).
	///
	/// For a chroma direct current block this is `4 * NumC8x8`, which in 4:2:0 is four.
	pub fn coeffs(self) -> usize {
		match self {
			Self::LumaDc | Self::Luma4x4	=> 16,
			Self::LumaAc | Self::ChromaAc	=> 15,
			Self::ChromaDc			=> 4,
			Self::Luma8x8			=> 64,
		}
	}

	/// Where this category's `coded_block_flag` contexts begin, offset included (Tables 9-34, 9-40).
	pub fn cbf_base(self) -> usize {
		match self {
			Self::LumaDc	=> 85,
			Self::LumaAc	=> 89,
			Self::Luma4x4	=> 93,
			Self::ChromaDc	=> 97,
			Self::ChromaAc	=> 101,
			// Never read in 4:2:0: an eight-by-eight block's flag is not coded (§7.3.5.3.3).
			Self::Luma8x8	=> 1012,
		}
	}

	/// Where its `significant_coeff_flag` contexts begin, for a frame-coded block.
	pub fn sig_base(self) -> usize {
		match self {
			Self::LumaDc	=> 105,
			Self::LumaAc	=> 120,
			Self::Luma4x4	=> 134,
			Self::ChromaDc	=> 149,
			Self::ChromaAc	=> 152,
			Self::Luma8x8	=> 402,
		}
	}

	/// Where its `last_significant_coeff_flag` contexts begin.
	pub fn last_base(self) -> usize {
		match self {
			Self::LumaDc	=> 166,
			Self::LumaAc	=> 181,
			Self::Luma4x4	=> 195,
			Self::ChromaDc	=> 210,
			Self::ChromaAc	=> 213,
			Self::Luma8x8	=> 417,
		}
	}

	/// Where its `coeff_abs_level_minus1` contexts begin.
	pub fn level_base(self) -> usize {
		match self {
			Self::LumaDc	=> 227,
			Self::LumaAc	=> 237,
			Self::Luma4x4	=> 247,
			Self::ChromaDc	=> 257,
			Self::ChromaAc	=> 266,
			Self::Luma8x8	=> 426,
		}
	}

	/// The context increment for the significance of the coefficient at a scan position (§9.3.3.1.3).
	fn sig_inc(self, at: usize) -> usize {
		match self {
			// A chroma direct current block's four positions share three contexts.
			Self::ChromaDc	=> at.min(2),
			Self::Luma8x8	=> SIG_8X8[at.min(62)] as usize,
			_		=> at,
		}
	}

	/// The same for whether that coefficient is the last one.
	fn last_inc(self, at: usize) -> usize {
		match self {
			Self::ChromaDc	=> at.min(2),
			Self::Luma8x8	=> LAST_8X8[at.min(62)] as usize,
			_		=> at,
		}
	}
}

/// The context increment for the significance of each scan position of a frame-coded eight-by-eight
/// block (Table 9-43, the first of its three columns).
///
/// Sixty-four positions share fifteen contexts, and not in scan order: the mapping is a published
/// table because it groups positions by how likely a coefficient there is, which the scan does not.
pub const SIG_8X8: [u8; 63] = [
	0, 1, 2, 3, 4, 5, 5, 4,
	4, 3, 3, 4, 4, 4, 5, 5,
	4, 4, 4, 4, 3, 3, 6, 7,
	7, 7, 8, 9, 10, 9, 8, 7,
	7, 6, 11, 12, 13, 11, 6, 7,
	8, 9, 14, 10, 9, 8, 6, 11,
	12, 13, 11, 6, 9, 14, 10, 9,
	11, 12, 13, 11, 14, 10, 12,
];

/// The same for whether the coefficient there is the last one (Table 9-43, the third column).
pub const LAST_8X8: [u8; 63] = [
	0, 1, 1, 1, 1, 1, 1, 1,
	1, 1, 1, 1, 1, 1, 1, 1,
	2, 2, 2, 2, 2, 2, 2, 2,
	2, 2, 2, 2, 2, 2, 2, 2,
	3, 3, 3, 3, 3, 3, 3, 3,
	4, 4, 4, 4, 4, 4, 4, 4,
	5, 5, 5, 5, 6, 6, 6, 6,
	7, 7, 7, 7, 8, 8, 8,
];

/// How far the unary prefix of a coefficient's magnitude runs before the Exp-Golomb suffix begins
/// (Table 9-34, `uCoff` of the UEG0 binarisation).
const LEVEL_PREFIX_MAX: usize = 14;

/// Reads one block of transform coefficient levels (§7.3.5.3.3).
///
/// `cbf_inc` is the context increment for `coded_block_flag`, worked out from the neighbouring
/// blocks by the caller; `None` says the flag is not coded at all and is inferred to be one, which
/// is what an eight-by-eight luma block in 4:2:0 does. `levels` is filled in scan order and must be
/// as long as the category says the block is.
///
/// Returns whether the block holds anything, which the neighbours of the *next* block ask about.
pub fn residual(c: &mut Cabac, x: &mut Contexts, cat: Cat, cbf_inc: Option<u32>, levels: &mut [i32])
	-> Outcome<bool>
{
	if levels.len() != cat.coeffs() {
		return Err(err!(
			"A {:?} block was read into {} coefficients, and Table 9-42 says it holds {}.",
			cat, levels.len(), cat.coeffs(); Bug));
	}
	for v in levels.iter_mut() {
		*v = 0;
	}
	if let Some(inc) = cbf_inc {
		if res!(x.bin(c, cat.cbf_base() + inc as usize)) == 0 {
			return Ok(false);
		}
	}
	// The significance map, forwards from the direct current term. A set last flag says the block
	// ends there, and the coefficient at that position is significant without a flag of its own.
	let n = levels.len();
	let mut sig = vec![false; n];
	let mut num = n;
	let mut at = 0usize;
	while at + 1 < num {
		if res!(x.bin(c, cat.sig_base() + cat.sig_inc(at))) == 1 {
			sig[at] = true;
			if res!(x.bin(c, cat.last_base() + cat.last_inc(at))) == 1 {
				num = at + 1;
			}
		}
		at += 1;
	}
	sig[num - 1] = true;
	// The magnitudes, backwards from the last significant position. The order matters: the context
	// a magnitude is coded against counts how many magnitudes of one and how many above one have
	// already been read *in this block*, so reading them forwards gives every one the wrong context.
	let mut ones = 0usize;
	let mut bigger = 0usize;
	for k in (0..num).rev() {
		if !sig[k] {
			continue;
		}
		let magnitude = res!(level(c, x, cat, ones, bigger));
		if magnitude == 1 {
			ones += 1;
		} else {
			bigger += 1;
		}
		levels[k] = if c.bypass() == 1 {
			-(magnitude as i32)
		} else {
			magnitude as i32
		};
	}
	Ok(true)
}

/// Reads one coefficient's magnitude: `coeff_abs_level_minus1` plus one (§9.3.2.3, §9.3.3.1.3).
///
/// A truncated unary prefix of up to fourteen bins, and past that an Exp-Golomb suffix read at even
/// odds. `ones` and `bigger` are how many magnitudes of exactly one and of more than one have been
/// read in this block already, which is the whole of what chooses the contexts.
fn level(c: &mut Cabac, x: &mut Contexts, cat: Cat, ones: usize, bigger: usize) -> Outcome<u32> {
	let base = cat.level_base();
	let first = if bigger != 0 { 0 } else { 4.min(1 + ones) };
	if res!(x.bin(c, base + first)) == 0 {
		return Ok(1);
	}
	// Every bin after the first shares one context, since maxBinIdxCtx is one.
	let rest = base + 5 + (4 - usize::from(cat == Cat::ChromaDc)).min(bigger);
	let mut prefix = 1usize;
	while prefix < LEVEL_PREFIX_MAX && res!(x.bin(c, rest)) == 1 {
		prefix += 1;
	}
	if prefix < LEVEL_PREFIX_MAX {
		return Ok(prefix as u32 + 1);
	}
	// The suffix: a run of ones says how wide the remainder is, then the remainder itself.
	let mut k = 0u32;
	let mut suffix = 0u32;
	while c.bypass() == 1 {
		suffix += 1 << k;
		k += 1;
		if k > 20 {
			return Err(err!(
				"A coefficient's Exp-Golomb suffix ran past 20 bins, so the arithmetic decoder is no \
				longer reading the syntax where it is.";
			Invalid, Input, Decode));
		}
	}
	suffix += c.bypass_bits(k as usize);
	Ok(LEVEL_PREFIX_MAX as u32 + suffix + 1)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_every_context_starts_in_a_state_that_exists_01() -> Outcome<()> {
		// Two hundred and sixty-one initialisation pairs against every quantisation parameter a
		// slice may carry. The state they yield indexes a table of sixty-four rows, and the
		// arithmetic that produces it is the specification's own -- so an index outside it is a
		// transcription error, and trying them all is the only way to find one before a picture
		// exists.
		let mut pairs = 0usize;
		for (first, _table, values) in INIT_I {
			for (i, (m, n)) in values.iter().enumerate() {
				pairs += 1;
				for qp in 0..=51i32 {
					let ctx = Ctx::start(*m, *n, qp);
					let state = ctx.state();
					if state > 62 {
						return Err(err!(
							"Context {} starts from ({}, {}) and at a quantisation parameter of {} \
							that is state {}, where 62 is the highest.",
							first + i, m, n, qp, state; Test, Invalid));
					}
				}
			}
		}
		req!(pairs, 261, "the intra initialisation tables hold {} pairs and clause 9 gives 261",
			pairs);
		Ok(())
	}

	#[test]
	fn test_the_interval_is_renormalised_after_every_bin_02() -> Outcome<()> {
		// The invariant the whole arithmetic decoder rests on: the interval is at least 256 and at
		// most 510 whenever a bin has been answered. A renormalisation that stops one shift short
		// decodes plausible rubbish rather than failing, which is exactly the sort of fault that
		// survives until a picture comes out wrong, so it is asserted directly.
		//
		// The data is a run of bytes from a small linear congruential sequence: it is not a coded
		// slice and does not have to be, since the invariant holds over any input at all.
		let mut seed = 0x2545_f491_4f6c_dd1du64;
		let mut data = Vec::with_capacity(4096);
		for _ in 0..4096 {
			seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			data.push((seed >> 33) as u8);
		}
		let mut c = res!(Cabac::new(&data));
		let mut x = Contexts::start(26);
		let mut ones = 0usize;
		for i in 0..20_000usize {
			match i % 8 {
				7 => {
					// The terminating bin, which a slice asks once a macroblock. On random bytes it
					// will eventually say the data has ended.
					if c.terminate() == 1 {
						break;
					}
				},
				6 => {
					ones += c.bypass() as usize;
				},
				k => {
					ones += res!(x.bin(&mut c, 105 + k)) as usize;
				},
			}
			let w = c.width();
			if w < 256 || w > 510 {
				return Err(err!(
					"After {} bins the interval is {}, outside 256 to 510.", i + 1, w;
				Test, Invalid));
			}
		}
		// A decoder that answered every bin the same way would satisfy the invariant above and be
		// useless, so the answers are required to be mixed.
		if ones == 0 {
			return Err(err!("Not one bin came back as a one."; Test, Invalid));
		}
		Ok(())
	}

	#[test]
	fn test_an_uninitialised_context_is_refused_03() -> Outcome<()> {
		// The guard against a wrong context increment. Every ctxIdx an intra 4:2:0 slice may name is
		// initialised; anything else is a fault in the increment, and reading it would produce a
		// picture rather than an error.
		let mut x = Contexts::start(26);
		req!(x.at(3).is_ok(), true, "mb_type's first context is not initialised");
		req!(x.at(435).is_ok(), true, "the last of Table 9-24 is not initialised");
		let past = x.at(436).is_err();
		req!(past, true, "a context the intra tables do not carry was handed out");
		let motion = x.at(40).is_err();
		req!(motion, true, "a motion vector context was handed out to an intra slice");
		let outside = x.at(CONTEXTS).is_err();
		req!(outside, true, "a context past the 1024 the syntax numbers was handed out");
		Ok(())
	}

	#[test]
	fn test_a_decoder_needs_two_bytes_to_begin_04() -> Outcome<()> {
		req!(Cabac::new(&[0x40]).is_err(), true,
			"A decoder started on one byte, and it reads nine bits before answering anything.");
		Ok(())
	}

	#[test]
	fn test_the_scan_position_tables_cover_every_position_05() -> Outcome<()> {
		// Table 9-43 maps 63 scan positions of an eight-by-eight block onto contexts, and the counts
		// are what the initialisation tables were sized against: fifteen significance contexts at
		// 402 to 416 and nine last-position ones at 417 to 425. A mapping that reached further would
		// read a context belonging to the next syntax element.
		let sig_top = SIG_8X8.iter().copied().max().unwrap_or(0);
		req!(sig_top as usize, 14, "significance reaches context {} and Table 9-24 gives 15 of them",
			sig_top);
		let last_top = LAST_8X8.iter().copied().max().unwrap_or(0);
		req!(last_top as usize, 8, "the last position reaches context {} and there are 9", last_top);
		// Every context in each range is used by some position, which a table with a value missing
		// would break.
		for want in 0..=14u8 {
			let used = SIG_8X8.contains(&want);
			req!(used, true, "no scan position maps to significance context {}", want);
		}
		for want in 0..=8u8 {
			let used = LAST_8X8.contains(&want);
			req!(used, true, "no scan position maps to last-position context {}", want);
		}
		// The last-position mapping rises without ever falling, which is what makes it a mapping
		// from frequency band to context; a transposed row would break it.
		for i in 1..LAST_8X8.len() {
			let rising = LAST_8X8[i] >= LAST_8X8[i - 1];
			req!(rising, true, "the last-position mapping falls at scan position {}", i);
		}
		Ok(())
	}

	#[test]
	fn test_no_two_categories_share_a_context_06() -> Outcome<()> {
		// Table 9-40's offsets are what keeps six block categories out of each other's context
		// variables, and an offset one place out is a fault that changes no bin count: the decoder
		// stays in step and adapts the wrong probabilities. So the ranges are laid out and checked
		// for overlap.
		//
		// The ranges each category actually reaches, given the increments of §9.3.3.1.3.
		let cats = [Cat::LumaDc, Cat::LumaAc, Cat::Luma4x4, Cat::ChromaDc, Cat::ChromaAc];
		let mut spans: Vec<(String, usize, usize)> = Vec::new();
		for cat in cats {
			let n = cat.coeffs();
			let sig_top = (0..n - 1).map(|i| cat.sig_inc(i)).max().unwrap_or(0);
			let last_top = (0..n - 1).map(|i| cat.last_inc(i)).max().unwrap_or(0);
			// A magnitude's contexts: nought to four for the first bin, and five upward for the
			// rest, one fewer for a chroma direct current block.
			let level_top = 5 + (4 - usize::from(cat == Cat::ChromaDc));
			spans.push((fmt!("{:?} significance", cat), cat.sig_base(), cat.sig_base() + sig_top));
			spans.push((fmt!("{:?} last", cat), cat.last_base(), cat.last_base() + last_top));
			spans.push((fmt!("{:?} levels", cat), cat.level_base(), cat.level_base() + level_top));
			spans.push((fmt!("{:?} flag", cat), cat.cbf_base(), cat.cbf_base() + 3));
		}
		for (i, (an, a0, a1)) in spans.iter().enumerate() {
			for (bn, b0, b1) in spans.iter().skip(i + 1) {
				if a0 <= b1 && b0 <= a1 {
					return Err(err!(
						"{} uses contexts {} to {} and {} uses {} to {}, so they share.",
						an, a0, a1, bn, b0, b1; Test, Invalid));
				}
			}
		}
		// And every context each span reaches is one the tables initialised.
		let mut x = Contexts::start(26);
		for (name, a0, a1) in &spans {
			for i in *a0..=*a1 {
				if x.at(i).is_err() {
					return Err(err!(
						"{} reaches context {}, which the intra tables do not carry.", name, i;
					Test, Missing));
				}
			}
		}
		Ok(())
	}

	/// A signed integer as the document prints one.
	///
	/// The minus sign is U+2212 rather than the hyphen a parser would expect, and a word that is not
	/// a number at all -- "na" where a column does not apply, or a word of a row heading -- is not
	/// one.
	fn number(w: &str) -> Option<i32> {
		w.replace('\u{2212}', "-").parse::<i32>().ok()
	}

	/// The lines of one published table, gathered across every page it spans.
	///
	/// A heading occurs in the table of contents as well as over the table, trailing dot leaders and
	/// a page number, so an occurrence carrying those is not the table. A wide table repeats its
	/// heading on each page it continues onto, which is why the pieces are gathered rather than the
	/// first one taken.
	fn region<'a>(lines: &[&'a str], heading: &str) -> Vec<&'a str> {
		let mut out = Vec::new();
		for (start, _) in lines.iter().enumerate()
			.filter(|(_, l)| l.contains(heading) && !l.contains("...."))
		{
			for line in lines.iter().skip(start + 1) {
				if line.trim_start().starts_with("Table 9-") {
					break;
				}
				out.push(*line);
			}
		}
		out
	}

	/// The numbers of one initialisation table, as a flat run in `ctxIdx` order.
	///
	/// Two shapes occur, and which a table uses is a fact about the document rather than about the
	/// values, so the caller states it in `by_row` and the count then proves it.
	///
	/// Tables 9-12, 9-16 and 9-17 lay their values out as a row of every `m` followed by a row of
	/// every `n`, the I-slice row first. The rest give one line per `ctxIdx`: the index, then the
	/// I-slice column's `m` and `n`, then three pairs for the values of `cabac_init_idc` -- nine
	/// numbers, and eighteen where the page prints two halves of the table side by side. **Nothing
	/// but a line of exactly nine or exactly eighteen numbers is read as data**, which is what keeps
	/// a value out of the row above or below it: a looser rule reads the neighbouring column's `m` as
	/// this row's `ctxIdx` and then quietly agrees with a wrong transcription.
	fn published(lines: &[&str], table: usize, first: usize, count: usize, by_row: bool)
		-> Outcome<Vec<(i32, i32)>>
	{
		let heading = fmt!("Table 9-{} – Values of variables m and n for ctxIdx", table);
		let body = region(lines, &heading);
		let mut rows: std::collections::BTreeMap<usize, (i32, i32)> =
			std::collections::BTreeMap::new();
		let mut ms: Vec<i32> = Vec::new();
		let mut ns: Vec<i32> = Vec::new();
		for line in &body {
			let words: Vec<&str> = line.split_whitespace().collect();
			if !by_row {
				// The label sits after the row's own heading words -- "I slices" in Table 9-16 -- so
				// it is found rather than assumed to be first, and a run of "na" where a column does
				// not apply starts the values again.
				let at = match words.iter().position(|w| *w == "m" || *w == "n") {
					Some(at) => at,
					None => continue,
				};
				let mut got: Vec<i32> = Vec::new();
				for w in &words[at + 1..] {
					match number(w) {
						Some(v) => got.push(v),
						None => got.clear(),
					}
				}
				if got.len() == count {
					if words[at] == "m" && ms.is_empty() {
						ms = got;
					} else if words[at] == "n" && ns.is_empty() {
						ns = got;
					}
				}
				continue;
			}
			let got: Vec<Option<i32>> = words.iter().map(|w| number(w)).collect();
			if got.is_empty() || got.iter().any(|v| v.is_none()) {
				continue;
			}
			let v: Vec<i32> = got.into_iter().flatten().collect();
			let halves: &[usize] = match v.len() {
				9	=> &[0],
				18	=> &[0, 9],
				_	=> continue,
			};
			for at in halves {
				let idx = v[*at];
				if idx < first as i32 || idx >= (first + count) as i32 {
					return Err(err!(
						"Table 9-{} has a row for ctxIdx {}, and it is meant to cover {} to {}.",
						table, idx, first, first + count - 1; Test, Invalid));
				}
				if rows.insert(idx as usize, (v[at + 1], v[at + 2])).is_some() {
					return Err(err!(
						"Table 9-{} gives ctxIdx {} twice.", table, idx; Test, Invalid));
				}
			}
		}
		if !by_row && ms.len() == count && ns.len() == count {
			return Ok(ms.into_iter().zip(ns).collect());
		}
		if by_row && rows.len() == count {
			return Ok(rows.into_values().collect());
		}
		Err(err!(
			"Table 9-{} gave up {} rows and {} m and {} n values, and it covers ctxIdx {} to {}.",
			table, rows.len(), ms.len(), ns.len(), first, first + count - 1;
		Test, Missing))
	}

	#[test]
	fn test_the_context_tables_are_the_published_ones_07() -> Outcome<()> {
		// Two hundred and sixty-one pairs of numbers copied out of a document by hand, every one of
		// which silently ruins a picture if it is wrong. Checking them against the decoder that uses
		// them proves nothing at all -- the only thing worth checking them against is the
		// specification they came from, so this reads it:
		//
		//   pdftotext -layout T-REC-H.264-202108.pdf h264.txt
		//   H264_SPEC_TEXT=~/.cache/specs/h264.txt cargo test -p oxedyne_fe2o3_graphics h264
		//
		// Absent, it says so rather than passing quietly: a check that skipped in silence would be a
		// check nobody ran.
		let path = match std::env::var("H264_SPEC_TEXT") {
			Ok(p) => p,
			Err(_) => {
				println!("  skipped: set H264_SPEC_TEXT to a text rendering of Rec. ITU-T H.264");
				return Ok(());
			},
		};
		let text = match std::fs::read_to_string(&path) {
			Ok(t) => t,
			Err(e) => {
				println!("  skipped: {} would not read ({})", path, e);
				return Ok(());
			},
		};
		let lines: Vec<&str> = text.lines().collect();
		let mut checked = 0usize;
		for (first, table, values) in INIT_I {
			// Where the table starts, how many entries it holds, and whether it prints one line per
			// ctxIdx. Table 9-12 holds more than an I slice reads -- 0 to 2 are an SI slice's -- and
			// Table 9-24 holds the eight-by-eight residual of the P and B slices too, so the whole
			// table is read and the run taken out of it.
			let (from, count, by_row) = match table {
				12	=> (0usize, 11usize, false),
				16	=> (399, 3, false),
				17	=> (60, 10, false),
				18	=> (70, 35, true),
				19	=> (105, 61, true),
				20	=> (166, 61, true),
				21	=> (227, 49, true),
				24	=> (402, 58, true),
				other	=> return Err(err!(
					"Table 9-{} is not one this test knows how to read.", other; Test, Bug)),
			};
			let all = res!(published(&lines, table, from, count, by_row));
			for (i, (m, n)) in values.iter().enumerate() {
				let at = first + i - from;
				let (pm, pn) = match all.get(at) {
					Some(v) => *v,
					None => return Err(err!(
						"Table 9-{} holds {} entries and ctxIdx {} would be the {}th.",
						table, all.len(), first + i, at + 1; Test, Missing)),
				};
				if pm != *m as i32 || pn != *n as i32 {
					return Err(err!(
						"Context {} is initialised from ({}, {}) and Table 9-{} publishes ({}, {}).",
						first + i, m, n, table, pm, pn; Test, Mismatch));
				}
				checked += 1;
			}
		}
		req!(checked, 261, "{} of 261 initialisation pairs were held to the document", checked);

		// And the engine's own two tables, which are not initialisation values but are transcribed
		// by hand just the same. Table 9-44 prints two halves side by side: a state and its four
		// values, then a second state and its four.
		let mut rows = 0usize;
		for line in region(&lines, "Table 9-44 – Specification of rangeTabLPS") {
			let got: Vec<Option<u32>> = line.split_whitespace()
				.map(|w| w.parse::<u32>().ok())
				.collect();
			if got.is_empty() || got.iter().any(|v| v.is_none()) {
				continue;
			}
			let v: Vec<u32> = got.into_iter().flatten().collect();
			let halves: &[usize] = match v.len() {
				5	=> &[0],
				10	=> &[0, 5],
				_	=> continue,
			};
			for at in halves {
				let state = v[*at] as usize;
				if state >= 64 {
					return Err(err!(
						"Table 9-44 has a row for state {}, and there are 64.", state; Test, Invalid));
				}
				let want = [v[at + 1] as u8, v[at + 2] as u8, v[at + 3] as u8, v[at + 4] as u8];
				if LPS[state] != want {
					return Err(err!(
						"rangeTabLPS row {} is {:?} here and {:?} in Table 9-44.",
						state, LPS[state], want; Test, Mismatch));
				}
				rows += 1;
			}
		}
		req!(rows, 64, "{} of 64 rangeTabLPS rows were held to Table 9-44", rows);

		// Table 9-45 prints itself as four blocks of a states row, a transIdxLPS row and a
		// transIdxMPS row.
		let mut moved = 0usize;
		let mut states: Vec<usize> = Vec::new();
		for line in region(&lines, "Table 9-45 – State transition table") {
			let words: Vec<&str> = line.split_whitespace().collect();
			if words.is_empty() {
				continue;
			}
			let got: Vec<u8> = words[1..].iter().filter_map(|w| w.parse::<u8>().ok()).collect();
			match words[0] {
				"pStateIdx" => {
					states = got.into_iter().map(usize::from).collect();
				},
				"transIdxLPS" | "transIdxMPS" if got.len() == states.len() => {
					let held = if words[0] == "transIdxLPS" { &NEXT_LPS } else { &NEXT_MPS };
					for (s, v) in states.iter().zip(got.iter()) {
						if held[*s] != *v {
							return Err(err!(
								"{}({}) is {} here and {} in Table 9-45.",
								words[0], s, held[*s], v; Test, Mismatch));
						}
						moved += 1;
					}
				},
				_ => {},
			}
		}
		req!(moved, 128, "{} of 128 state transitions were held to Table 9-45", moved);
		Ok(())
	}

	#[test]
	fn test_the_scan_position_table_is_the_published_one_08() -> Outcome<()> {
		// Table 9-43, which is laid out as two halves of a wide table: a scan position, three
		// context increments, then a second scan position and three more. Only the first and third
		// of the three are held here, since this decoder reads frames.
		let path = match std::env::var("H264_SPEC_TEXT") {
			Ok(p) => p,
			Err(_) => {
				println!("  skipped: set H264_SPEC_TEXT to a text rendering of Rec. ITU-T H.264");
				return Ok(());
			},
		};
		let text = match std::fs::read_to_string(&path) {
			Ok(t) => t,
			Err(_) => return Ok(()),
		};
		let lines: Vec<&str> = text.lines().collect();
		let mut seen: std::collections::BTreeMap<usize, (u8, u8)> =
			std::collections::BTreeMap::new();
		for line in region(&lines, "Table 9-43 – Mapping of scanning position to ctxIdxInc") {
			let got: Vec<Option<u8>> = line.split_whitespace()
				.map(|w| w.parse::<u8>().ok())
				.collect();
			if got.is_empty() || got.iter().any(|v| v.is_none()) {
				continue;
			}
			let v: Vec<u8> = got.into_iter().flatten().collect();
			// Each half is four numbers: the scan position and the three context increments.
			let halves: &[usize] = match v.len() {
				4	=> &[0],
				8	=> &[0, 4],
				_	=> continue,
			};
			for at in halves {
				let pos = v[*at] as usize;
				if pos > 62 {
					return Err(err!(
						"Table 9-43 has a row for scan position {}, and it covers 0 to 62.", pos;
					Test, Invalid));
				}
				if seen.insert(pos, (v[at + 1], v[at + 3])).is_some() {
					return Err(err!(
						"Table 9-43 gives scan position {} twice.", pos; Test, Invalid));
				}
			}
		}
		req!(seen.len(), 63, "Table 9-43 gave up {} of its 63 scan positions", seen.len());
		for (pos, (sig, last)) in &seen {
			if SIG_8X8[*pos] != *sig {
				return Err(err!(
					"Scan position {} maps to significance context {} here and {} in Table 9-43.",
					pos, SIG_8X8[*pos], sig; Test, Mismatch));
			}
			if LAST_8X8[*pos] != *last {
				return Err(err!(
					"Scan position {} maps to last-position context {} here and {} in Table 9-43.",
					pos, LAST_8X8[*pos], last; Test, Mismatch));
			}
		}
		Ok(())
	}
}
