//! The arithmetic decoder, and the context variables it codes against.
//!
//! HEVC entropy coding is CABAC: every syntax element is a string of binary decisions, and each
//! decision is coded against a probability that adapts as the picture is read. Three kinds of
//! decision exist -- one against a context variable, which is the usual one; one *bypassed* at even
//! odds, for the parts of a value that carry no useful correlation; and one *terminating*, which is
//! how the end of a slice or of a row of blocks is found.
//!
//! What is in here is the decoder itself (§9.3.4.3), the probability state a context is in
//! (§9.3.2.2), the eighteen sets of context variables an intra still picture draws on, and the rule
//! that carries them from one row of coding tree blocks to the next under wavefront coding.
//!
//! The two properties this can be held to before any picture exists are asserted in `mod.rs`'s
//! tests: every context starts in a state the probability tables actually have, and the coding
//! interval stays between 256 and 510 after every bin, whatever is fed in.

use oxedyne_fe2o3_core::prelude::*;

/// The probability state a context variable is in: an index 0 to 62, and the more probable symbol.
///
/// One byte rather than two fields, because there are hundreds of these and they are copied whole
/// at the start of every row of blocks under wavefront coding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ctx(u8);

impl Ctx {

	/// The state a context starts in, from its initialisation value and the slice's quantisation
	/// parameter (§9.3.2.2).
	///
	/// The arithmetic is the specification's, and the clamp on the quantisation parameter is
	/// load-bearing rather than defensive: a slice may legally start at a negative one on a
	/// high-bit-depth picture, and the table this indexes has no entries there.
	pub fn start(init: u8, qp: i32) -> Self {
		let q = qp.clamp(0, 51);
		let slope = ((init >> 4) as i32) * 5 - 45;
		let offset = (((init & 15) as i32) << 3) - 16;
		let pre = ((slope * q) >> 4) + offset;
		let pre = pre.clamp(1, 126);
		if pre <= 63 {
			Self((((63 - pre) as u8) << 1) & 0x7e)
		} else {
			Self(((((pre - 64) as u8) << 1) | 1) & 0x7f)
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

/// How the range is narrowed for the less probable symbol, indexed by state and by the two bits
/// the current range contributes (§9.3.4.3.2.1, Table 9-46).
const LPS: [[u8; 4]; 64] = [
	[128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205],
	[116, 142, 169, 195], [111, 135, 160, 185], [105, 128, 152, 175], [100, 122, 144, 166],
	[95, 116, 137, 158],  [90, 110, 130, 150],  [85, 104, 123, 142],  [81, 99, 117, 135],
	[77, 94, 111, 128],   [73, 89, 105, 122],   [69, 85, 100, 116],   [66, 80, 95, 110],
	[62, 76, 90, 104],    [59, 72, 86, 99],     [56, 69, 81, 94],     [53, 65, 77, 89],
	[51, 62, 73, 85],     [48, 59, 69, 80],     [46, 56, 66, 76],     [43, 53, 63, 72],
	[41, 50, 59, 69],     [39, 48, 56, 65],     [37, 45, 54, 62],     [35, 43, 51, 59],
	[33, 41, 48, 56],     [32, 39, 46, 53],     [30, 37, 43, 50],     [29, 35, 41, 48],
	[27, 33, 39, 45],     [26, 31, 37, 43],     [24, 30, 35, 41],     [23, 28, 33, 39],
	[22, 27, 32, 37],     [21, 26, 30, 35],     [20, 24, 29, 33],     [19, 23, 27, 31],
	[18, 22, 26, 30],     [17, 21, 25, 28],     [16, 20, 23, 27],     [15, 19, 22, 25],
	[14, 18, 21, 24],     [14, 17, 20, 23],     [13, 16, 19, 22],     [12, 15, 18, 21],
	[12, 14, 17, 20],     [11, 14, 16, 19],     [11, 13, 15, 18],     [10, 12, 15, 17],
	[10, 12, 14, 16],     [9, 11, 13, 15],      [9, 11, 12, 14],      [8, 10, 12, 14],
	[8, 9, 11, 13],       [7, 9, 11, 12],       [7, 9, 10, 12],       [7, 8, 10, 11],
	[6, 8, 9, 11],        [6, 7, 9, 10],        [6, 7, 8, 9],         [2, 2, 2, 2],
];

/// The state to move to after coding the more probable symbol (Table 9-47).
const NEXT_MPS: [u8; 64] = [
	1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
	17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
	33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
	49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

/// The state to move to after coding the less probable symbol (Table 9-47).
const NEXT_LPS: [u8; 64] = [
	0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12,
	13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24,
	24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33,
	33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

/// The arithmetic decoder itself (§9.3.4.3).
///
/// It reads bits and answers questions of the form "was the next symbol a one?", where the odds are
/// carried by whichever context variable the syntax says applies. There are three ways to ask: with
/// a context, which is the usual one and adapts as it goes; **bypassed**, at even odds, for the
/// parts of a value that carry no useful correlation; and **terminating**, which is how the end of
/// a slice or a piece of one is found.
pub struct Cabac<'a> {
	/// The bytes being read.
	buf:	&'a [u8],
	/// The next byte to be taken into the window.
	at:	usize,
	/// The current interval's width.
	range:	u32,
	/// Where in the interval the coded value sits.
	offset:	u32,
	/// How many bits of the window have been used.
	bits:	i32,
}

impl<'a> Cabac<'a> {

	/// Starts the decoder at the beginning of a piece of entropy-coded data (§9.3.2.5).
	pub fn new(buf: &'a [u8]) -> Outcome<Self> {
		if buf.len() < 2 {
			return Err(err!(
				"An arithmetic decoder was started on {} bytes, and it reads two before it \
				answers anything.", buf.len();
			Invalid, Input, Decode));
		}
		Ok(Self {
			buf,
			at:	2,
			range:	510,
			// The specification reads nine bits into `ivlOffset` and compares it against the
			// interval directly, reading one more bit at every renormalisation. This decoder keeps
			// those bits **pre-read** instead -- `offset` holds `ivlOffset` shifted up by `bits`,
			// with `bits` more of the stream already in the low end -- so a renormalisation is a
			// subtraction from `bits` rather than a read. That is what makes it a byte-at-a-time
			// decoder rather than a bit-at-a-time one, and it means the two bytes go in whole:
			// `ivlOffset` is the top nine bits of them and the other seven are the window.
			//
			// Putting the unshifted nine-bit value here instead leaves every comparison against
			// `range << bits` too small by a factor of 128, so the first hundred or so bins all
			// come back as the more probable symbol and the picture is plausible and wrong.
			offset:	((buf[0] as u32) << 8) | (buf[1] as u32),
			bits:	7,
		})
	}

	/// The next byte, or zeroes past the end.
	///
	/// A decoder is allowed to read a little past the last byte of a slice -- the final bins are
	/// coded against bits the encoder never had to write -- so running out is not a fault. What
	/// would be a fault is reading far past it, and that is caught by the terminating bin, which
	/// says where the data ends.
	fn byte(&mut self) -> u32 {
		let b = self.buf.get(self.at).copied().unwrap_or(0) as u32;
		self.at += 1;
		b
	}

	/// One bin against a context, which is then moved on (§9.3.4.3.2).
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
				// State zero is where the two symbols are equally likely, so being wrong there
				// exchanges which one is called the more probable.
				ctx.0 ^= 1;
			}
			ctx.0 = (NEXT_LPS[state] << 1) | (ctx.0 & 1);
		} else {
			value = ctx.mps();
			ctx.0 = (NEXT_MPS[state] << 1) | (ctx.0 & 1);
		}
		// Renormalise: the interval is kept at nine bits or more.
		while self.range < 256 {
			self.range <<= 1;
			self.bits -= 1;
			if self.bits < 0 {
				self.offset = (self.offset << 8) | self.byte();
				self.bits += 8;
			}
		}
		value
	}

	/// One bin at even odds, with no context to move on (§9.3.4.3.4).
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

	/// The bin that says whether this is the end (§9.3.4.3.5).
	///
	/// One at the end of a slice, and at the end of each piece of one under wavefront coding.
	pub fn terminate(&mut self) -> u32 {
		self.range -= 2;
		if self.offset >= (self.range << self.bits) {
			1
		} else {
			while self.range < 256 {
				self.range <<= 1;
				self.bits -= 1;
				if self.bits < 0 {
					self.offset = (self.offset << 8) | self.byte();
					self.bits += 8;
				}
			}
			0
		}
	}

	/// How many bytes have been taken out of the buffer.
	///
	/// After a terminating bin says the piece has ended, this is where the next piece begins --
	/// which is how the entry point offsets in the slice header are checked against the data.
	pub fn consumed(&self) -> usize {
		self.at
	}
}

// ------------------------------------------------------- the context variables themselves

/// A set of context variables belonging to one syntax element (§9.3.2.2, Table 9-4).
///
/// **Only the sets an intra still picture uses, and only their intra initialisation values.** The
/// specification gives three initialisation types -- one for I slices and two for P and B -- and a
/// picture out of a HEIC file is one intra slice, so the other two are as much use here as the
/// motion vector syntax they mostly belong to. Where a table's intra column is a subset of its rows
/// (`sig_coeff_flag` uses 0 to 41 and then 126 and 127, and nothing between), that is what is kept,
/// and the gap is recorded in [`Set::runs`] so the whole lot can be checked back against the
/// published table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Set {
	/// Whether this block reuses the block to its left's or above's sample adaptive offset.
	SaoMerge,
	/// Which kind of sample adaptive offset a block has.
	SaoType,
	/// Whether a block of the coding quadtree splits into four.
	SplitCu,
	/// Whether a coding unit skips the transform and the quantiser entirely.
	TransquantBypass,
	/// How a coding unit is divided into prediction units.
	PartMode,
	/// Whether a block's intra mode is one of the three its neighbours suggest.
	PrevIntraLumaPred,
	/// Which intra mode the chroma blocks take.
	IntraChromaPredMode,
	/// Whether a block of the transform tree splits into four.
	SplitTransform,
	/// Whether a luma transform block has any coefficient in it at all.
	CbfLuma,
	/// The same for the two chroma blocks.
	CbfChroma,
	/// How far this block's quantisation parameter is from the one predicted for it.
	CuQpDeltaAbs,
	/// Whether a transform block is coded without its transform.
	TransformSkip,
	/// Where the last coefficient of a block sits, across.
	LastSigX,
	/// And down.
	LastSigY,
	/// Whether a four-by-four group within a transform block holds anything.
	CodedSubBlock,
	/// Whether one coefficient is not zero.
	SigCoeff,
	/// Whether a coefficient's magnitude is more than one.
	Greater1,
	/// Whether it is more than two.
	Greater2,
}

impl Set {

	/// Every set, in the order their context variables are laid out.
	pub const ALL: [Self; 18] = [
		Self::SaoMerge,
		Self::SaoType,
		Self::SplitCu,
		Self::TransquantBypass,
		Self::PartMode,
		Self::PrevIntraLumaPred,
		Self::IntraChromaPredMode,
		Self::SplitTransform,
		Self::CbfLuma,
		Self::CbfChroma,
		Self::CuQpDeltaAbs,
		Self::TransformSkip,
		Self::LastSigX,
		Self::LastSigY,
		Self::CodedSubBlock,
		Self::SigCoeff,
		Self::Greater1,
		Self::Greater2,
	];

	/// The initialisation values of this set's context variables, for an intra slice.
	pub const fn init(self) -> &'static [u8] {
		match self {
			Self::SaoMerge		=> &[153],
			Self::SaoType		=> &[200],
			Self::SplitCu		=> &[139, 141, 157],
			Self::TransquantBypass	=> &[154],
			Self::PartMode		=> &[184],
			Self::PrevIntraLumaPred	=> &[184],
			Self::IntraChromaPredMode => &[63],
			Self::SplitTransform	=> &[153, 138, 138],
			Self::CbfLuma		=> &[111, 141],
			// Four by depth in the transform tree, and a fifth for the second chroma block of a
			// 4:2:2 picture, which this decoder will not meet but which sits in the same table.
			Self::CbfChroma		=> &[94, 138, 182, 154, 154],
			Self::CuQpDeltaAbs	=> &[154, 154],
			// One for luma and one for chroma; the specification numbers them 0 and 3.
			Self::TransformSkip	=> &[139, 139],
			Self::LastSigX		=> &[
				110, 110, 124, 125, 140, 153, 125, 127, 140,
				109, 111, 143, 127, 111, 79, 108, 123, 63,
			],
			Self::LastSigY		=> &[
				110, 110, 124, 125, 140, 153, 125, 127, 140,
				109, 111, 143, 127, 111, 79, 108, 123, 63,
			],
			Self::CodedSubBlock	=> &[91, 171, 134, 141],
			Self::SigCoeff		=> &[
				111, 111, 125, 110, 110, 94, 124, 108,
				124, 107, 125, 141, 179, 153, 125, 107,
				125, 141, 179, 153, 125, 107, 125, 141,
				179, 153, 125, 140, 139, 182, 182, 152,
				136, 152, 136, 153, 136, 139, 111, 136,
				139, 111,
				// The two the specification puts at 126 and 127, for a block coded without its
				// transform.
				141, 111,
			],
			Self::Greater1		=> &[
				140, 92, 137, 138, 140, 152, 138, 139,
				153, 74, 149, 92, 139, 107, 122, 152,
				140, 179, 166, 182, 140, 227, 122, 197,
			],
			Self::Greater2		=> &[138, 153, 136, 167, 152, 152],
		}
	}

	/// How many context variables the set holds.
	pub const fn len(self) -> usize {
		self.init().len()
	}

	/// Whether it holds none, which none of these do.
	pub const fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// Where the set's variables begin in the flat array [`Contexts`] holds.
	pub const fn base(self) -> usize {
		let mut at = 0;
		let mut i = 0;
		while i < Self::ALL.len() {
			if Self::ALL[i] as u8 == self as u8 {
				return at;
			}
			at += Self::ALL[i].len();
			i += 1;
		}
		at
	}

	/// Which published table the values in [`Set::init`] were taken from, as its number within
	/// clause 9 -- `5` for Table 9-5.
	///
	/// Kept so that the transcription can be checked against the document rather than against
	/// itself; `tests` does exactly that where a copy of the specification is to hand.
	pub const fn table(self) -> usize {
		match self {
			Self::SaoMerge		=> 5,
			Self::SaoType		=> 6,
			Self::SplitCu		=> 7,
			Self::TransquantBypass	=> 8,
			Self::PartMode		=> 11,
			Self::PrevIntraLumaPred	=> 12,
			Self::IntraChromaPredMode => 13,
			Self::SplitTransform	=> 20,
			Self::CbfLuma		=> 21,
			Self::CbfChroma		=> 22,
			Self::CuQpDeltaAbs	=> 24,
			Self::TransformSkip	=> 25,
			Self::LastSigX		=> 26,
			Self::LastSigY		=> 27,
			Self::CodedSubBlock	=> 28,
			Self::SigCoeff		=> 29,
			Self::Greater1		=> 30,
			Self::Greater2		=> 31,
		}
	}

	/// Which of that table's entries an intra slice takes, as runs of `(first, how many)`.
	///
	/// Table 9-4 gives these as ranges against the initialisation type, and for an intra slice they
	/// are the first ones -- except where a table serves two syntax elements at once, or where the
	/// entries a still picture wants are not next to each other.
	pub const fn runs(self) -> &'static [(usize, usize)] {
		match self {
			// Four by depth at 0..3, and the odd one out at 12.
			Self::CbfChroma		=> &[(0, 4), (12, 1)],
			// Luma at 0 and chroma at 3.
			Self::TransformSkip	=> &[(0, 1), (3, 1)],
			// Nought to forty-one, and then two a long way further on.
			Self::SigCoeff		=> &[(0, 42), (126, 2)],
			// Everything else is a run from the start of its table.
			other			=> match other.len() {
				// One run, as long as the set is. Written this way because a `const fn` cannot
				// hold a reference to a temporary, so each length that occurs gets its own.
				1	=> &[(0, 1)],
				2	=> &[(0, 2)],
				3	=> &[(0, 3)],
				4	=> &[(0, 4)],
				6	=> &[(0, 6)],
				18	=> &[(0, 18)],
				24	=> &[(0, 24)],
				_	=> &[],
			},
		}
	}
}

/// How many context variables a picture's decoder carries altogether.
pub const CONTEXTS: usize = {
	let mut at = 0;
	let mut i = 0;
	while i < Set::ALL.len() {
		at += Set::ALL[i].len();
		i += 1;
	}
	at
};

/// Every context variable a still picture's decoder needs, in one array.
///
/// One flat array with a base per set, rather than a struct of named arrays, because the whole lot
/// is **copied** at the start of every row of blocks under wavefront coding -- which every
/// photograph in the corpus this was written against uses -- and a copy of one fixed-size array is
/// as cheap as copying gets.
#[derive(Clone, Copy, Debug)]
pub struct Contexts {
	v:	[Ctx; CONTEXTS],
}

impl Contexts {

	/// The state every context starts a slice in, given that slice's quantisation parameter.
	pub fn start(qp: i32) -> Self {
		let mut v = [Ctx::start(154, 26); CONTEXTS];
		for set in Set::ALL {
			let base = set.base();
			let init = set.init();
			let mut i = 0;
			while i < init.len() {
				v[base + i] = Ctx::start(init[i], qp);
				i += 1;
			}
		}
		Self { v }
	}

	/// One context variable: which set, and which of that set's variables the syntax says applies.
	///
	/// An index past the end of its set is a fault in whoever worked out the increment, not a thing
	/// to be clamped quietly into range: a decoder that reads the wrong context produces a picture
	/// rather than an error, and a picture that is subtly wrong is the hardest kind of fault to
	/// find. So it is refused, and the message says which set and which index.
	pub fn at(&mut self, set: Set, i: usize) -> Outcome<&mut Ctx> {
		if i >= set.len() {
			return Err(err!(
				"Context {} of {:?} was asked for, and that set holds {}.", i, set, set.len();
			Invalid, Input, Decode));
		}
		Ok(&mut self.v[set.base() + i])
	}
}

/// The context state carried from one row of blocks to the next, under wavefront coding.
///
/// Every photograph in the corpus is coded in wavefronts (`entropy_coding_sync_enabled_flag`),
/// which is the surprise that shapes this decoder: the arithmetic coder is **restarted at every row
/// of coding tree blocks**, and the contexts it restarts with are not fresh ones but the ones saved
/// after the *second* block of the row above. That is what lets an encoder code the rows in
/// parallel while still learning from what came before, and it means a decoder that treats a row
/// boundary as a fresh start decodes plausible rubbish from the second row onward.
///
/// This holds the one-slice, one-tile case, which is every picture in the corpus and every picture a
/// still image is likely to be. A second tile would need one of these each, since a tile boundary
/// breaks the dependency; the widening is a field, not a redesign, and is left until a picture
/// wants it.
#[derive(Clone, Copy, Debug)]
pub struct Rows {
	/// What a slice starts from, for the first row, which has nothing above it.
	qp:	i32,
	/// What was saved after the second block of the row above.
	saved:	Option<Contexts>,
}

impl Rows {

	/// A picture whose first row has nothing to inherit.
	pub fn new(qp: i32) -> Self {
		Self { qp, saved: None }
	}

	/// The contexts a row of blocks begins with.
	///
	/// The row above's second block where there was one, and a fresh set where there was not --
	/// which is the first row, and only the first row.
	pub fn begin(&self) -> Contexts {
		match self.saved {
			Some(ctxs) => ctxs,
			None => Contexts::start(self.qp),
		}
	}

	/// Keeps the state as it stands, which the caller does once the **second** block of a row has
	/// been decoded (§9.3.2.3).
	///
	/// Not the first: the row above has to be two blocks ahead before the row below may start, or
	/// the two would be coding the same neighbourhood at once.
	pub fn after_second(&mut self, ctxs: &Contexts) {
		self.saved = Some(*ctxs);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_every_context_starts_in_a_state_that_exists_03() -> Outcome<()> {
		// Two hundred and fifty-six initialisation values against every quantisation parameter a
		// slice may carry, including the negative ones a high-bit-depth picture allows. The state
		// this yields indexes a table of sixty-four rows, and the arithmetic that produces it is
		// the specification's own -- so an index outside it is a transcription error, and the only
		// way to find one before the whole decoder exists is to try them all.
		for init in 0..=255u8 {
			for qp in -12..=51i32 {
				let ctx = Ctx::start(init, qp);
				let state = ctx.state();
				if state > 62 {
					return Err(err!(
						"An initialisation value of {} at a quantisation parameter of {} starts in \
						state {}, and 62 is the highest.", init, qp, state;
					Test, Invalid));
				}
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_interval_is_renormalised_after_every_bin_04() -> Outcome<()> {
		// The invariant the whole arithmetic decoder rests on: the interval is at least 256 and at
		// most 510 whenever a bin has been answered. A renormalisation that stops one shift short
		// decodes plausible rubbish rather than failing, which is exactly the sort of fault that
		// survives until a picture comes out wrong, so it is asserted directly.
		//
		// The data is a run of bytes from a small linear congruential sequence: it is not a coded
		// picture and does not have to be, since the invariant holds over any input at all.
		let mut seed = 0x2545_f491_4f6c_dd1du64;
		let mut data = Vec::with_capacity(4096);
		for _ in 0..4096 {
			seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			data.push((seed >> 33) as u8);
		}
		let mut cabac = res!(Cabac::new(&data));
		let mut ctxs: Vec<Ctx> = (0..16).map(|i| Ctx::start(i * 17, 26)).collect();
		let mut ones = 0usize;
		let total = 20_000usize;
		for i in 0..total {
			match i % 8 {
				7 => {
					// A terminating bin, which is asked once a block. Where it says the data has
					// ended, the decoder stops -- and on random bytes it will, eventually.
					if cabac.terminate() == 1 {
						break;
					}
				},
				6 => {
					ones += cabac.bypass() as usize;
				},
				k => {
					ones += cabac.bin(&mut ctxs[k * 2]) as usize;
				},
			}
			if cabac.range < 256 || cabac.range > 510 {
				return Err(err!(
					"After {} bins the interval is {}, outside 256 to 510.", i + 1, cabac.range;
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
	fn test_the_context_tables_are_the_published_ones_06() -> Outcome<()> {
		// Two hundred and thirty numbers copied out of a document by hand, every one of which
		// silently ruins a picture if it is wrong. Checking them against the decoder that uses them
		// proves nothing at all -- the only thing worth checking them against is the specification
		// they came from, so this reads it.
		//
		//   pdftotext -layout T-REC-H.265-202108.pdf h265.txt
		//   HEVC_SPEC_TEXT=~/.cache/specs/h265.txt cargo test -p oxedyne_fe2o3_graphics hevc
		//
		// Absent, it says so rather than passing quietly: a check that skipped in silence would be
		// a check nobody ran.
		let path = match std::env::var("HEVC_SPEC_TEXT") {
			Ok(p) => p,
			Err(_) => {
				println!("  skipped: set HEVC_SPEC_TEXT to a text rendering of Rec. ITU-T H.265");
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

		for set in Set::ALL {
			// The table's own numbers, in ctxIdx order: every "initValue" row under the heading,
			// until the next table begins. The document lays a wide table out as alternating rows
			// of indices and values, so the values arrive in several pieces and in order.
			let heading = fmt!("Table 9-{} – Values of initValue", set.table());
			// The heading occurs twice: once in the table of contents, trailing dot leaders and a
			// page number, and once over the table itself. Taking whichever one has values under it
			// needs no rule about which is which.
			let mut published: Vec<u8> = Vec::new();
			for (start, _) in lines.iter().enumerate().filter(|(_, l)| l.contains(&heading)) {
				let mut found: Vec<u8> = Vec::new();
				for line in lines.iter().skip(start + 1) {
					let trimmed = line.trim_start();
					if trimmed.starts_with("Table 9-") {
						break;
					}
					if !trimmed.starts_with("initValue") {
						continue;
					}
					for word in trimmed.trim_start_matches("initValue").split_whitespace() {
						match word.parse::<u16>() {
							// Every initValue is a byte. A page number caught in the same row
							// would not be, and shows up here rather than as a wrong picture.
							Ok(n) if n <= 255 => found.push(n as u8),
							_ => return Err(err!(
								"Table 9-{} holds {:?}, which is not an initialisation value.",
								set.table(), word; Test, Invalid)),
						}
					}
				}
				if !found.is_empty() {
					published = found;
					break;
				}
			}
			if published.is_empty() {
				return Err(err!(
					"Table 9-{} is not in {}, or holds no values.", set.table(), path;
				Test, Missing));
			}
			// What an intra slice takes out of it, per Table 9-4.
			let mut wanted: Vec<u8> = Vec::new();
			for (first, count) in set.runs() {
				let end = first + count;
				if end > published.len() {
					return Err(err!(
						"{:?} wants entries {}..{} of Table 9-{}, which holds {}.",
						set, first, end, set.table(), published.len(); Test, Invalid));
				}
				wanted.extend_from_slice(&published[*first..end]);
			}
			let held: Vec<u8> = set.init().to_vec();
			if held != wanted {
				return Err(err!(
					"{:?} is initialised from {:?}, and Table 9-{} entries {:?} are {:?}.",
					set, held, set.table(), set.runs(), wanted; Test, Mismatch));
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_row_of_blocks_inherits_the_row_above_it_07() -> Outcome<()> {
		// The wavefront rule, which is the one every photograph in the corpus depends on: a row of
		// blocks starts from the contexts saved after the *second* block of the row above, not from
		// fresh ones. A decoder that starts each row afresh decodes rubbish from the second row on,
		// and this is the smallest statement of the difference.
		let mut rows = Rows::new(26);
		let fresh = rows.begin();
		let mut moved = fresh;
		// Something the first row learned, which the second must not lose.
		let ctx = res!(moved.at(Set::SigCoeff, 0));
		let before = *ctx;
		*ctx = Ctx::start(200, 51);
		let learned = *res!(moved.at(Set::SigCoeff, 0));
		let changed = learned != before;
		req!(changed, true, "the fixture did not change anything, so it proves nothing");

		rows.after_second(&moved);
		let next = rows.begin();
		let carried = next.v[Set::SigCoeff.base()];
		req!(carried, learned, "a row began afresh instead of from the row above it");

		// And a picture whose first row has nothing above it starts from the table.
		let first = Rows::new(26).begin();
		req!(first.v[Set::SigCoeff.base()], before);
		Ok(())
	}

	#[test]
	fn test_the_context_sets_do_not_overlap_or_leave_gaps_08() -> Outcome<()> {
		// The bases are worked out by summing the lengths in front of each set, so a set added in
		// the middle of the list moves every one after it. That is the intended behaviour and it is
		// also exactly how one set would come to share variables with another, so it is asserted.
		let mut at = 0usize;
		for set in Set::ALL {
			req!(set.base(), at, "{:?} does not begin where the set before it ends", set);
			at += set.len();
			let empty = set.is_empty();
			req!(empty, false, "{:?} holds no context variables at all", set);
		}
		req!(at, CONTEXTS);
		// And the last one is reachable, while one past it is refused rather than read.
		let mut ctxs = Contexts::start(26);
		let last = Set::Greater2.len() - 1;
		req!(ctxs.at(Set::Greater2, last).is_ok(), true);
		req!(ctxs.at(Set::Greater2, last + 1).is_err(), true,
			"a context past the end of its set was handed out");
		Ok(())
	}

	#[test]
	fn test_a_decoder_needs_two_bytes_to_begin_05() -> Outcome<()> {
		req!(Cabac::new(&[0x40]).is_err(), true,
			"A decoder started on one byte, and it reads two before answering anything.");
		Ok(())
	}
}
