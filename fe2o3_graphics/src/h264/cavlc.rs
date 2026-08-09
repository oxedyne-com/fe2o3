//! The variable-length code tables, and reading a block of coefficients with them.
//!
//! CAVLC is the entropy coder H.264 uses when `entropy_coding_mode_flag` is off, and **711 films in
//! the corpus use it** -- every Baseline one. It is not a fallback and not a legacy path: it is two
//! films in every five. Nothing about it resembles the arithmetic coder beside it. Where CABAC
//! carries a probability that adapts with the picture, CAVLC carries published code tables and
//! switches between them on what has already been decoded.
//!
//! # How a block is read
//!
//! A four-by-four block's coefficients are read **backwards**, from the highest frequency down:
//!
//! 1. `coeff_token` says how many coefficients are not zero and how many of them, at the end of the
//!    block, are exactly ±1. Which of the six tables reads it depends on `nC`, the mean of the
//!    counts in the blocks above and to the left -- so a block's *table* depends on its
//!    neighbours, and getting that wrong reads the right bits with the wrong code and desynchronises
//!    everything after it.
//! 2. Each trailing ±1 costs one bit: its sign.
//! 3. Every other level is a prefix of zeroes, a suffix whose width **grows as the levels do**, and
//!    an escape for the large ones.
//! 4. `total_zeros` says how many zeroes lie among the coefficients, and `run_before` distributes
//!    them.
//!
//! Step 3 is where the coder earns its keep and where a decoder goes wrong quietly: `suffixLength`
//! starts at nought or one depending on the token, and climbs each time a level exceeds
//! `3 << (suffixLength − 1)`. A decoder that never climbs it decodes small blocks perfectly and
//! busy ones as noise.
//!
//! # Where the tables came from
//!
//! Parsed out of Rec. ITU-T H.264 (08/2021) rather than typed in: Table 9-5 is 372 codewords and a
//! transcription shifted by one place in one column is a picture that is right until it meets a
//! busy block. The tests re-read the same tables out of the specification and, separately, assert
//! that every column is a prefix code -- which a misread codeword almost always breaks.

use crate::h264::Bits;

use oxedyne_fe2o3_core::prelude::*;

/// `coeff_token`, as `(bits, code)` by column of Table 9-5, trailing ones and total.
///
/// The six columns are the six ranges of `nC`: below two, below four, below eight, eight
/// and over, and the two chroma direct-current tables at −1 and −2. An entry of no bits is a
/// combination the table does not code.
pub const COEFF_TOKEN: [[[(u8, u16); 17]; 4]; 6] = [
	[
		[(1, 0b1), (6, 0b000101), (8, 0b00000111), (9, 0b000000111), (10, 0b0000000111), (11, 0b00000000111), (13, 0b0000000001111), (13, 0b0000000001011), (13, 0b0000000001000), (14, 0b00000000001111), (14, 0b00000000001011), (15, 0b000000000001111), (15, 0b000000000001011), (16, 0b0000000000001111), (16, 0b0000000000001011), (16, 0b0000000000000111), (16, 0b0000000000000100)],
		[(0, 0), (2, 0b01), (6, 0b000100), (8, 0b00000110), (9, 0b000000110), (10, 0b0000000110), (11, 0b00000000110), (13, 0b0000000001110), (13, 0b0000000001010), (14, 0b00000000001110), (14, 0b00000000001010), (15, 0b000000000001110), (15, 0b000000000001010), (15, 0b000000000000001), (16, 0b0000000000001110), (16, 0b0000000000001010), (16, 0b0000000000000110)],
		[(0, 0), (0, 0), (3, 0b001), (7, 0b0000101), (8, 0b00000101), (9, 0b000000101), (10, 0b0000000101), (11, 0b00000000101), (13, 0b0000000001101), (13, 0b0000000001001), (14, 0b00000000001101), (14, 0b00000000001001), (15, 0b000000000001101), (15, 0b000000000001001), (16, 0b0000000000001101), (16, 0b0000000000001001), (16, 0b0000000000000101)],
		[(0, 0), (0, 0), (0, 0), (5, 0b00011), (6, 0b000011), (7, 0b0000100), (8, 0b00000100), (9, 0b000000100), (10, 0b0000000100), (11, 0b00000000100), (13, 0b0000000001100), (14, 0b00000000001100), (14, 0b00000000001000), (15, 0b000000000001100), (15, 0b000000000001000), (16, 0b0000000000001100), (16, 0b0000000000001000)],
	],
	[
		[(2, 0b11), (6, 0b001011), (6, 0b000111), (7, 0b0000111), (8, 0b00000111), (8, 0b00000100), (9, 0b000000111), (11, 0b00000001111), (11, 0b00000001011), (12, 0b000000001111), (12, 0b000000001011), (12, 0b000000001000), (13, 0b0000000001111), (13, 0b0000000001011), (13, 0b0000000000111), (14, 0b00000000001001), (14, 0b00000000000111)],
		[(0, 0), (2, 0b10), (5, 0b00111), (6, 0b001010), (6, 0b000110), (7, 0b0000110), (8, 0b00000110), (9, 0b000000110), (11, 0b00000001110), (11, 0b00000001010), (12, 0b000000001110), (12, 0b000000001010), (13, 0b0000000001110), (13, 0b0000000001010), (14, 0b00000000001011), (14, 0b00000000001000), (14, 0b00000000000110)],
		[(0, 0), (0, 0), (3, 0b011), (6, 0b001001), (6, 0b000101), (7, 0b0000101), (8, 0b00000101), (9, 0b000000101), (11, 0b00000001101), (11, 0b00000001001), (12, 0b000000001101), (12, 0b000000001001), (13, 0b0000000001101), (13, 0b0000000001001), (13, 0b0000000000110), (14, 0b00000000001010), (14, 0b00000000000101)],
		[(0, 0), (0, 0), (0, 0), (4, 0b0101), (4, 0b0100), (5, 0b00110), (6, 0b001000), (6, 0b000100), (7, 0b0000100), (9, 0b000000100), (11, 0b00000001100), (11, 0b00000001000), (12, 0b000000001100), (13, 0b0000000001100), (13, 0b0000000001000), (13, 0b0000000000001), (14, 0b00000000000100)],
	],
	[
		[(4, 0b1111), (6, 0b001111), (6, 0b001011), (6, 0b001000), (7, 0b0001111), (7, 0b0001011), (7, 0b0001001), (7, 0b0001000), (8, 0b00001111), (8, 0b00001011), (9, 0b000001111), (9, 0b000001011), (9, 0b000001000), (10, 0b0000001101), (10, 0b0000001001), (10, 0b0000000101), (10, 0b0000000001)],
		[(0, 0), (4, 0b1110), (5, 0b01111), (5, 0b01100), (5, 0b01010), (5, 0b01000), (6, 0b001110), (6, 0b001010), (7, 0b0001110), (8, 0b00001110), (8, 0b00001010), (9, 0b000001110), (9, 0b000001010), (9, 0b000000111), (10, 0b0000001100), (10, 0b0000001000), (10, 0b0000000100)],
		[(0, 0), (0, 0), (4, 0b1101), (5, 0b01110), (5, 0b01011), (5, 0b01001), (6, 0b001101), (6, 0b001001), (7, 0b0001101), (7, 0b0001010), (8, 0b00001101), (8, 0b00001001), (9, 0b000001101), (9, 0b000001001), (10, 0b0000001011), (10, 0b0000000111), (10, 0b0000000011)],
		[(0, 0), (0, 0), (0, 0), (4, 0b1100), (4, 0b1011), (4, 0b1010), (4, 0b1001), (4, 0b1000), (5, 0b01101), (6, 0b001100), (7, 0b0001100), (8, 0b00001100), (8, 0b00001000), (9, 0b000001100), (10, 0b0000001010), (10, 0b0000000110), (10, 0b0000000010)],
	],
	[
		[(6, 0b000011), (6, 0b000000), (6, 0b000100), (6, 0b001000), (6, 0b001100), (6, 0b010000), (6, 0b010100), (6, 0b011000), (6, 0b011100), (6, 0b100000), (6, 0b100100), (6, 0b101000), (6, 0b101100), (6, 0b110000), (6, 0b110100), (6, 0b111000), (6, 0b111100)],
		[(0, 0), (6, 0b000001), (6, 0b000101), (6, 0b001001), (6, 0b001101), (6, 0b010001), (6, 0b010101), (6, 0b011001), (6, 0b011101), (6, 0b100001), (6, 0b100101), (6, 0b101001), (6, 0b101101), (6, 0b110001), (6, 0b110101), (6, 0b111001), (6, 0b111101)],
		[(0, 0), (0, 0), (6, 0b000110), (6, 0b001010), (6, 0b001110), (6, 0b010010), (6, 0b010110), (6, 0b011010), (6, 0b011110), (6, 0b100010), (6, 0b100110), (6, 0b101010), (6, 0b101110), (6, 0b110010), (6, 0b110110), (6, 0b111010), (6, 0b111110)],
		[(0, 0), (0, 0), (0, 0), (6, 0b001011), (6, 0b001111), (6, 0b010011), (6, 0b010111), (6, 0b011011), (6, 0b011111), (6, 0b100011), (6, 0b100111), (6, 0b101011), (6, 0b101111), (6, 0b110011), (6, 0b110111), (6, 0b111011), (6, 0b111111)],
	],
	[
		[(2, 0b01), (6, 0b000111), (6, 0b000100), (6, 0b000011), (6, 0b000010), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (1, 0b1), (6, 0b000110), (7, 0b0000011), (8, 0b00000011), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (0, 0), (3, 0b001), (7, 0b0000010), (8, 0b00000010), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (0, 0), (0, 0), (6, 0b000101), (7, 0b0000000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	],
	[
		[(1, 0b1), (7, 0b0001111), (7, 0b0001110), (9, 0b000000111), (9, 0b000000110), (10, 0b0000000111), (11, 0b00000000111), (12, 0b000000000111), (13, 0b0000000000111), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (2, 0b01), (7, 0b0001101), (7, 0b0001100), (9, 0b000000101), (10, 0b0000000110), (11, 0b00000000110), (12, 0b000000000110), (12, 0b000000000101), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (0, 0), (3, 0b001), (7, 0b0001011), (7, 0b0001010), (9, 0b000000100), (10, 0b0000000101), (11, 0b00000000101), (12, 0b000000000100), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
		[(0, 0), (0, 0), (0, 0), (5, 0b00001), (6, 0b000001), (7, 0b0001001), (7, 0b0001000), (10, 0b0000000100), (11, 0b00000000100), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	],
];

/// `total_zeros` for a block of sixteen coefficients, by `tzVlcIndex` and by the count
/// (Tables 9-7 and 9-8).
pub const TOTAL_ZEROS: [[(u8, u16); 16]; 15] = [
	[(1, 0b1), (3, 0b011), (3, 0b010), (4, 0b0011), (4, 0b0010), (5, 0b00011), (5, 0b00010), (6, 0b000011), (6, 0b000010), (7, 0b0000011), (7, 0b0000010), (8, 0b00000011), (8, 0b00000010), (9, 0b000000011), (9, 0b000000010), (9, 0b000000001)],
	[(3, 0b111), (3, 0b110), (3, 0b101), (3, 0b100), (3, 0b011), (4, 0b0101), (4, 0b0100), (4, 0b0011), (4, 0b0010), (5, 0b00011), (5, 0b00010), (6, 0b000011), (6, 0b000010), (6, 0b000001), (6, 0b000000), (0, 0)],
	[(4, 0b0101), (3, 0b111), (3, 0b110), (3, 0b101), (4, 0b0100), (4, 0b0011), (3, 0b100), (3, 0b011), (4, 0b0010), (5, 0b00011), (5, 0b00010), (6, 0b000001), (5, 0b00001), (6, 0b000000), (0, 0), (0, 0)],
	[(5, 0b00011), (3, 0b111), (4, 0b0101), (4, 0b0100), (3, 0b110), (3, 0b101), (3, 0b100), (4, 0b0011), (3, 0b011), (4, 0b0010), (5, 0b00010), (5, 0b00001), (5, 0b00000), (0, 0), (0, 0), (0, 0)],
	[(4, 0b0101), (4, 0b0100), (4, 0b0011), (3, 0b111), (3, 0b110), (3, 0b101), (3, 0b100), (3, 0b011), (4, 0b0010), (5, 0b00001), (4, 0b0001), (5, 0b00000), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(6, 0b000001), (5, 0b00001), (3, 0b111), (3, 0b110), (3, 0b101), (3, 0b100), (3, 0b011), (3, 0b010), (4, 0b0001), (3, 0b001), (6, 0b000000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(6, 0b000001), (5, 0b00001), (3, 0b101), (3, 0b100), (3, 0b011), (2, 0b11), (3, 0b010), (4, 0b0001), (3, 0b001), (6, 0b000000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(6, 0b000001), (4, 0b0001), (5, 0b00001), (3, 0b011), (2, 0b11), (2, 0b10), (3, 0b010), (3, 0b001), (6, 0b000000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(6, 0b000001), (6, 0b000000), (4, 0b0001), (2, 0b11), (2, 0b10), (3, 0b001), (2, 0b01), (5, 0b00001), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(5, 0b00001), (5, 0b00000), (3, 0b001), (2, 0b11), (2, 0b10), (2, 0b01), (4, 0b0001), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(4, 0b0000), (4, 0b0001), (3, 0b001), (3, 0b010), (1, 0b1), (3, 0b011), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(4, 0b0000), (4, 0b0001), (2, 0b01), (1, 0b1), (3, 0b001), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(3, 0b000), (3, 0b001), (1, 0b1), (2, 0b01), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(2, 0b00), (2, 0b01), (1, 0b1), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(1, 0b0), (1, 0b1), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
];

/// `total_zeros` for a 4:2:0 chroma direct-current block of four (Table 9-9(a)).
pub const TOTAL_ZEROS_CHROMA: [[(u8, u16); 4]; 3] = [
	[(1, 0b1), (2, 0b01), (3, 0b001), (3, 0b000)],
	[(1, 0b1), (2, 0b01), (2, 0b00), (0, 0)],
	[(1, 0b1), (1, 0b0), (0, 0), (0, 0)],
];

/// `run_before`, by how many zeroes are left and by the run (Table 9-10).
///
/// The seventh row serves every `zerosLeft` above six, which is why it runs to fourteen
/// where the others stop at their own count.
pub const RUN_BEFORE: [[(u8, u16); 15]; 7] = [
	[(1, 0b1), (1, 0b0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(1, 0b1), (2, 0b01), (2, 0b00), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(2, 0b11), (2, 0b10), (2, 0b01), (2, 0b00), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(2, 0b11), (2, 0b10), (2, 0b01), (3, 0b001), (3, 0b000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(2, 0b11), (2, 0b10), (3, 0b011), (3, 0b010), (3, 0b001), (3, 0b000), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(2, 0b11), (3, 0b000), (3, 0b001), (3, 0b011), (3, 0b010), (3, 0b101), (3, 0b100), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
	[(3, 0b111), (3, 0b110), (3, 0b101), (3, 0b100), (3, 0b011), (3, 0b010), (3, 0b001), (4, 0b0001), (5, 0b00001), (6, 0b000001), (7, 0b0000001), (8, 0b00000001), (9, 0b000000001), (10, 0b0000000001), (11, 0b00000000001)],
];

/// The result of reading one block's coefficients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
	/// The coefficients in scan order, from the direct current term upward, zeroes and all.
	pub levels:	Vec<i32>,
	/// How many of them are not zero, which the next block's `nC` is derived from.
	pub total:	usize,
}

/// Reads a value out of a code table, given the table's entries as `(bits, code)`.
///
/// Every table here is a prefix code, so at most one entry matches whatever comes next, and the
/// longest entry is sixteen bits. A run of bits matching nothing is a desynchronised decoder and is
/// refused rather than guessed at: from that point on every later block would be noise, and the
/// picture that came out would look decoded.
fn lookup(b: &mut Bits, table: &[(u8, u16)], what: &str) -> Outcome<usize> {
	let peeked = b.peek(16);
	for (i, (bits, code)) in table.iter().enumerate() {
		if *bits == 0 {
			continue;
		}
		if (peeked >> (16 - *bits as u32)) == *code as u32 {
			res!(b.skip(*bits as usize));
			return Ok(i);
		}
	}
	Err(err!(
		"The next bits, {:016b}, are not a {} codeword in this table.", peeked, what;
	Invalid, Input, Decode))
}

/// Which column of Table 9-5 a block's `coeff_token` is read from, given `nC` (§9.2.1).
fn token_column(nc: i32) -> usize {
	match nc {
		-1		=> 4,
		n if n <= -2	=> 5,
		n if n < 2	=> 0,
		n if n < 4	=> 1,
		n if n < 8	=> 2,
		_		=> 3,
	}
}

/// Reads `coeff_token`: how many coefficients are not zero, and how many trailing ±1s (§9.2.1).
pub fn coeff_token(b: &mut Bits, nc: i32) -> Outcome<(usize, usize)> {
	let col = token_column(nc);
	let peeked = b.peek(16);
	for t1 in 0..4usize {
		for tc in 0..17usize {
			let (bits, code) = COEFF_TOKEN[col][t1][tc];
			if bits == 0 {
				continue;
			}
			if (peeked >> (16 - bits as u32)) == code as u32 {
				res!(b.skip(bits as usize));
				return Ok((t1, tc));
			}
		}
	}
	Err(err!(
		"The next bits, {:016b}, are not a coeff_token in the table nC of {} selects.", peeked, nc;
	Invalid, Input, Decode))
}

/// Reads `level_prefix`: the count of zeroes before the next one bit (§9.2.2.1).
fn level_prefix(b: &mut Bits) -> Outcome<u32> {
	let mut zeros = 0u32;
	while res!(b.u(1)) == 0 {
		zeros += 1;
		// A prefix beyond this is not a level, it is a decoder that has lost the bitstream. The
		// profiles in the corpus cap it at 15, and the widest any profile allows is 11 plus the
		// bit depth.
		if zeros > 32 {
			return Err(err!(
				"A level_prefix ran past 32 zeroes, so the bitstream is no longer being read \
				where its syntax is.";
			Invalid, Input, Decode));
		}
	}
	Ok(zeros)
}

/// Reads one block of transform coefficient levels (§9.2).
///
/// `max_coeffs` is how many the block holds -- sixteen for a whole four-by-four block, fifteen for
/// the alternating-current part of one whose direct current term is coded elsewhere, and four for a
/// 4:2:0 chroma direct-current block. `nc` selects the table, and for a chroma direct-current block
/// it is −1 rather than a count.
pub fn residual(b: &mut Bits, nc: i32, max_coeffs: usize) -> Outcome<Block> {
	let (trailing, total) = res!(coeff_token(b, nc));
	let mut out = Block { levels: vec![0; max_coeffs], total };
	if total == 0 {
		return Ok(out);
	}
	if total > max_coeffs {
		return Err(err!(
			"A block of {} coefficients codes {} of them as non-zero.", max_coeffs, total;
		Invalid, Input, Decode));
	}
	// The levels, highest frequency first.
	let mut levels = vec![0i32; total];
	for level in levels.iter_mut().take(trailing) {
		*level = if res!(b.u(1)) == 1 { -1 } else { 1 };
	}
	// The suffix starts one wide in a block busy enough that most levels will need it.
	let mut suffix_len: u32 = if total > 10 && trailing < 3 { 1 } else { 0 };
	for i in trailing..total {
		let prefix = res!(level_prefix(b));
		let suffix_size = if prefix == 14 && suffix_len == 0 {
			4
		} else if prefix >= 15 {
			prefix - 3
		} else {
			suffix_len
		};
		let suffix = if suffix_size > 0 {
			res!(b.u(suffix_size as usize))
		} else {
			0
		};
		let mut code = ((prefix.min(15) << suffix_len) + suffix) as i64;
		if prefix >= 15 && suffix_len == 0 {
			code += 15;
		}
		if prefix >= 16 {
			code += (1i64 << (prefix - 3)) - 4096;
		}
		// The first level after the trailing ones cannot be ±1, since a ±1 there would have been
		// coded as a trailing one, so its magnitude is offset by one.
		if i == trailing && trailing < 3 {
			code += 2;
		}
		levels[i] = if code % 2 == 0 {
			((code + 2) >> 1) as i32
		} else {
			((-code - 1) >> 1) as i32
		};
		// The suffix widens as the levels do. A decoder that leaves this out reads a quiet block
		// perfectly and a busy one as noise.
		if suffix_len == 0 {
			suffix_len = 1;
		}
		if levels[i].unsigned_abs() > (3u32 << (suffix_len - 1)) && suffix_len < 6 {
			suffix_len += 1;
		}
	}
	// Where the zeroes are.
	let mut zeros_left = if total < max_coeffs {
		let idx = total - 1;
		if max_coeffs == 4 {
			res!(lookup(b, &TOTAL_ZEROS_CHROMA[idx], "total_zeros")) as i32
		} else {
			res!(lookup(b, &TOTAL_ZEROS[idx], "total_zeros")) as i32
		}
	} else {
		0
	};
	let mut runs = vec![0i32; total];
	for i in 0..total.saturating_sub(1) {
		runs[i] = if zeros_left > 0 {
			let row = (zeros_left.min(7) - 1) as usize;
			res!(lookup(b, &RUN_BEFORE[row], "run_before")) as i32
		} else {
			0
		};
		zeros_left -= runs[i];
		if zeros_left < 0 {
			return Err(err!(
				"The runs of zeroes in a block add to more than the block holds.";
			Invalid, Input, Decode));
		}
	}
	if let Some(last) = runs.last_mut() {
		*last = zeros_left;
	}
	// Lay the levels out (§9.2.4). The levels were read from the *highest* frequency down, and the
	// runs count the zeroes in front of each, so the walk goes backwards through both and forwards
	// through the block: the last level read is the one nearest the direct current term.
	let mut at: i32 = -1;
	for i in (0..total).rev() {
		at += runs[i] + 1;
		if at < 0 || at as usize >= max_coeffs {
			return Err(err!(
				"A coefficient lands at position {} of a block of {}.", at, max_coeffs;
			Invalid, Input, Decode));
		}
		out.levels[at as usize] = levels[i];
	}
	Ok(out)
}

/// The `nC` a block reads its `coeff_token` with, from the counts in its two neighbours (§9.2.1).
///
/// Where both neighbours are available it is their mean rounded up; where one is, it is that one's;
/// where neither is, it is nought. This is the single most load-bearing number in CAVLC parsing:
/// the wrong `nC` picks the wrong column of Table 9-5, which reads a different number of bits, and
/// every block after it in the slice is then read from the wrong place.
pub fn nc(left: Option<usize>, above: Option<usize>) -> i32 {
	match (left, above) {
		(Some(a), Some(b))	=> ((a + b + 1) >> 1) as i32,
		(Some(a), None)		=> a as i32,
		(None, Some(b))		=> b as i32,
		(None, None)		=> 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every codeword one table holds, as a bit string.
	fn codes(table: &[(u8, u16)]) -> Vec<String> {
		table.iter()
			.filter(|(bits, _)| *bits > 0)
			.map(|(bits, code)| fmt!("{:01$b}", code, *bits as usize))
			.collect()
	}

	#[test]
	fn test_every_table_is_a_prefix_code_01() -> Outcome<()> {
		// The property that makes a variable-length code readable at all: no codeword is the start
		// of another, so the decoder always knows where one ends. It is also the property a
		// mistranscribed table almost always breaks -- a codeword one bit short, or one place out
		// of its column, collides with a neighbour -- so this catches a bad table without needing
		// a picture to decode.
		let mut sets: Vec<(String, Vec<String>)> = Vec::new();
		for col in 0..6 {
			let mut all = Vec::new();
			for t1 in 0..4 {
				all.extend(codes(&COEFF_TOKEN[col][t1]));
			}
			sets.push((fmt!("coeff_token column {}", col), all));
		}
		for i in 0..15 {
			sets.push((fmt!("total_zeros {}", i + 1), codes(&TOTAL_ZEROS[i])));
		}
		for i in 0..3 {
			sets.push((fmt!("chroma total_zeros {}", i + 1), codes(&TOTAL_ZEROS_CHROMA[i])));
		}
		for i in 0..7 {
			sets.push((fmt!("run_before {}", i + 1), codes(&RUN_BEFORE[i])));
		}
		for (name, all) in &sets {
			if all.is_empty() {
				return Err(err!("{} holds no codewords at all.", name; Test, Missing));
			}
			for (i, a) in all.iter().enumerate() {
				for b in all.iter().skip(i + 1) {
					if a.starts_with(b.as_str()) || b.starts_with(a.as_str()) {
						return Err(err!(
							"{}: the codeword {} is a prefix of {}, so neither can be read.",
							name, a, b; Test, Invalid));
					}
				}
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_tables_are_the_published_ones_02() -> Outcome<()> {
		// Five hundred and fifty-odd codewords, held against the document they came from. A
		// prefix code that is internally consistent and simply *wrong* -- two columns swapped,
		// say -- passes every other check here and decodes a plausible picture out of the wrong
		// bits, so the only worthwhile oracle is the specification itself.
		//
		//   pdftotext -layout T-REC-H.264-202108.pdf h264.txt
		//   H264_SPEC_TEXT=~/.cache/specs/h264.txt cargo test -p oxedyne_fe2o3_graphics h264
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
		// A row of one of these tables is a run of fields separated by two or more spaces; a
		// *single* space inside a field joins the four-bit groups the document prints a codeword
		// in. That is the whole of the layout, and it is what makes the tables readable at all.
		let row = |line: &str| -> Vec<String> {
			let mut out = Vec::new();
			let mut field = String::new();
			let mut gap = 0usize;
			for c in line.trim().chars() {
				if c == ' ' {
					gap += 1;
					continue;
				}
				if gap >= 2 && !field.is_empty() {
					out.push(field.clone());
					field.clear();
				}
				gap = 0;
				field.push(c);
			}
			if !field.is_empty() {
				out.push(field);
			}
			out
		};
		let lines: Vec<&str> = text.lines().collect();
		// Table 9-5, gathered across the pages it spans.
		let mut found = 0usize;
		let mut inside = false;
		for line in &lines {
			let trimmed = line.trim();
			if trimmed.starts_with("Table 9-") {
				inside = trimmed.starts_with("Table 9-5 – coeff_token mapping");
				continue;
			}
			if !inside {
				continue;
			}
			let f = row(line);
			if f.len() != 8 {
				continue;
			}
			let (t1, tc) = match (f[0].parse::<usize>(), f[1].parse::<usize>()) {
				(Ok(a), Ok(b)) if a < 4 && b < 17 => (a, b),
				_ => continue,
			};
			for (col, word) in f[2..].iter().enumerate() {
				let (bits, code) = COEFF_TOKEN[col][t1][tc];
				if word == "-" {
					if bits != 0 {
						return Err(err!(
							"Table 9-5 column {} has no codeword for {} trailing ones of {}, and \
							this decoder holds {} bits.", col, t1, tc, bits; Test, Mismatch));
					}
					// A dash is an entry too: the table saying this combination is not coded in
					// this column, which is as much a fact to be checked as a codeword is.
					found += 1;
					continue;
				}
				let held = fmt!("{:01$b}", code, bits as usize);
				if bits == 0 || &held != word {
					return Err(err!(
						"Table 9-5 column {}, {} trailing ones of {}: the specification codes {} \
						and this decoder holds {}.", col, t1, tc, word, held; Test, Mismatch));
				}
				found += 1;
			}
		}
		// Six columns, and the sixty-two combinations of trailing ones and total that exist.
		req!(found, 6 * 62, "Table 9-5 gave up {} codewords, and it holds {}", found, 6 * 62);
		Ok(())
	}

	#[test]
	fn test_the_suffix_widens_as_the_levels_grow_03() -> Outcome<()> {
		// `suffixLength` climbs each time a level exceeds `3 << (suffixLength − 1)`, and a decoder
		// that never climbs it reads a quiet block perfectly and a busy one as noise. This is the
		// smallest statement of the rule: the same bits read with and without it.
		//
		// A block of five coefficients, no trailing ones, whose levels climb. What is asserted is
		// that the decode uses more bits than a fixed one-bit suffix would -- which is only true
		// if the width grew.
		//
		// The bits: coeff_token for nC 0, TotalCoeff 5, TrailingOnes 0, then five levels.
		let (bits, code) = COEFF_TOKEN[0][0][5];
		let present = bits > 0;
		req!(present, true, "the fixture's coeff_token is not in the table");
		let mut stream: Vec<bool> = (0..bits).map(|i| (code >> (bits - 1 - i)) & 1 == 1).collect();
		// Five levels, each coded as a prefix of zeroes and a one, with a suffix whose width is
		// whatever the decoder believes it to be. Feeding a long run of level_prefix zeroes makes
		// each level large, which is exactly what drives the width up.
		for _ in 0..5 {
			for _ in 0..6 {
				stream.push(false);
			}
			stream.push(true);
			for _ in 0..6 {
				stream.push(false);
			}
		}
		// total_zeros of nought for a five-coefficient block.
		let (tzb, tzc) = TOTAL_ZEROS[4][0];
		for i in 0..tzb {
			stream.push((tzc >> (tzb - 1 - i)) & 1 == 1);
		}
		let mut buf = vec![0u8; stream.len().div_ceil(8) + 4];
		for (i, bit) in stream.iter().enumerate() {
			if *bit {
				buf[i / 8] |= 0x80 >> (i % 8);
			}
		}
		let mut b = Bits::new(&buf);
		let block = res!(residual(&mut b, 0, 16));
		req!(block.total, 5);
		let magnitudes: Vec<i32> = block.levels.iter().filter(|v| **v != 0).map(|v| v.abs())
			.collect();
		req!(magnitudes.len(), 5);
		// With the width fixed at one, every level would decode the same way; with it growing,
		// they do not.
		let all_same = magnitudes.iter().all(|m| *m == magnitudes[0]);
		req!(all_same, false,
			"every level came out the same magnitude {:?}, so the suffix never widened",
			magnitudes);
		Ok(())
	}

	#[test]
	fn test_the_neighbour_count_picks_the_table_04() -> Outcome<()> {
		// `nC` is the mean of the counts above and to the left, and it chooses which of six code
		// tables reads the next token. The four cases are the whole of it, and the rounding is
		// *up*: `(a + b + 1) >> 1`, not down. Rounding down picks a lower column for half the
		// blocks in a picture, and a lower column reads a different number of bits.
		req!(nc(Some(3), Some(4)), 4, "the mean of three and four rounded down");
		req!(nc(Some(4), Some(3)), 4);
		req!(nc(Some(2), Some(2)), 2);
		req!(nc(Some(5), None), 5);
		req!(nc(None, Some(5)), 5);
		req!(nc(None, None), 0, "a block with no neighbours did not read the first table");
		// And the columns each range picks.
		req!(token_column(0), 0);
		req!(token_column(1), 0);
		req!(token_column(2), 1);
		req!(token_column(3), 1);
		req!(token_column(4), 2);
		req!(token_column(7), 2);
		req!(token_column(8), 3);
		req!(token_column(64), 3);
		req!(token_column(-1), 4, "a 4:2:0 chroma direct-current block read a luma table");
		req!(token_column(-2), 5);
		Ok(())
	}

	#[test]
	fn test_a_block_of_nothing_costs_one_bit_05() -> Outcome<()> {
		// The commonest block in a picture is the empty one, and in the first table it is coded as
		// a single set bit. That is worth asserting on its own, because it is the one codeword
		// whose length a table shifted by one place changes without breaking the prefix property.
		let buf = [0b1000_0000u8, 0, 0, 0];
		let mut b = Bits::new(&buf);
		let block = res!(residual(&mut b, 0, 16));
		req!(block.total, 0);
		req!(b.consumed(), 1, "an empty block cost {} bits and it costs one", b.consumed());
		req!(block.levels, vec![0i32; 16]);
		Ok(())
	}
}
