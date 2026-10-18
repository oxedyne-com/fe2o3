//! The CCITT fax codings TIFF carries: Modified Huffman, T.4 (Group 3) and T.6 (Group 4).
//!
//! A black and white scan of a letter is the case this exists for. Scanners and fax software store
//! a bilevel page in one of these three far more often than in anything else, because a page of
//! print is mostly long runs of white and the codes were designed around exactly that.
//!
//! All three describe a row as runs of alternating colour starting with white. Modified Huffman
//! codes each row by itself, with the run lengths in two fixed prefix codes, one for white and one
//! for black. T.4 adds an end-of-line code before each row and, optionally, rows coded against the
//! row above (two-dimensional coding), where each change of colour is said as an offset from the
//! nearest matching change above it. T.6 codes every row that way, with no end-of-line codes.
//!
//! The output is one bit a pixel, eight to a byte with the leftmost pixel in the high bit, each row
//! padded to a whole byte, a set bit meaning black -- TIFF's `WhiteIsZero`, the photometric these
//! files nearly always declare.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

/// Which of the three codings a strip is in, with T.4's options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
	Mh,		// TIFF compression 2: rows byte aligned, no end-of-line codes
	T4 {
		two_d:	bool,	// T4Options bit 0: rows may be coded against the row above
		aligned:	bool,	// T4Options bit 2: fill bits put each end-of-line on a byte end
	},
	T6,		// TIFF compression 4
}

// (code, length in bits, run)
type Code = (u16, u8, u16);

const WHITE: &[Code] = &[
	(0b00110101, 8, 0),	(0b000111, 6, 1),	(0b0111, 4, 2),		(0b1000, 4, 3),
	(0b1011, 4, 4),		(0b1100, 4, 5),		(0b1110, 4, 6),		(0b1111, 4, 7),
	(0b10011, 5, 8),	(0b10100, 5, 9),	(0b00111, 5, 10),	(0b01000, 5, 11),
	(0b001000, 6, 12),	(0b000011, 6, 13),	(0b110100, 6, 14),	(0b110101, 6, 15),
	(0b101010, 6, 16),	(0b101011, 6, 17),	(0b0100111, 7, 18),	(0b0001100, 7, 19),
	(0b0001000, 7, 20),	(0b0010111, 7, 21),	(0b0000011, 7, 22),	(0b0000100, 7, 23),
	(0b0101000, 7, 24),	(0b0101011, 7, 25),	(0b0010011, 7, 26),	(0b0100100, 7, 27),
	(0b0011000, 7, 28),	(0b00000010, 8, 29),	(0b00000011, 8, 30),	(0b00011010, 8, 31),
	(0b00011011, 8, 32),	(0b00010010, 8, 33),	(0b00010011, 8, 34),	(0b00010100, 8, 35),
	(0b00010101, 8, 36),	(0b00010110, 8, 37),	(0b00010111, 8, 38),	(0b00101000, 8, 39),
	(0b00101001, 8, 40),	(0b00101010, 8, 41),	(0b00101011, 8, 42),	(0b00101100, 8, 43),
	(0b00101101, 8, 44),	(0b00000100, 8, 45),	(0b00000101, 8, 46),	(0b00001010, 8, 47),
	(0b00001011, 8, 48),	(0b01010010, 8, 49),	(0b01010011, 8, 50),	(0b01010100, 8, 51),
	(0b01010101, 8, 52),	(0b00100100, 8, 53),	(0b00100101, 8, 54),	(0b01011000, 8, 55),
	(0b01011001, 8, 56),	(0b01011010, 8, 57),	(0b01011011, 8, 58),	(0b01001010, 8, 59),
	(0b01001011, 8, 60),	(0b00110010, 8, 61),	(0b00110011, 8, 62),	(0b00110100, 8, 63),
	// Make-up codes.
	(0b11011, 5, 64),	(0b10010, 5, 128),	(0b010111, 6, 192),	(0b0110111, 7, 256),
	(0b00110110, 8, 320),	(0b00110111, 8, 384),	(0b01100100, 8, 448),	(0b01100101, 8, 512),
	(0b01101000, 8, 576),	(0b01100111, 8, 640),	(0b011001100, 9, 704),	(0b011001101, 9, 768),
	(0b011010010, 9, 832),	(0b011010011, 9, 896),	(0b011010100, 9, 960),	(0b011010101, 9, 1024),
	(0b011010110, 9, 1088),	(0b011010111, 9, 1152),	(0b011011000, 9, 1216),	(0b011011001, 9, 1280),
	(0b011011010, 9, 1344),	(0b011011011, 9, 1408),	(0b010011000, 9, 1472),	(0b010011001, 9, 1536),
	(0b010011010, 9, 1600),	(0b011000, 6, 1664),	(0b010011011, 9, 1728),
];

const BLACK: &[Code] = &[
	(0b0000110111, 10, 0),	(0b010, 3, 1),		(0b11, 2, 2),		(0b10, 2, 3),
	(0b011, 3, 4),		(0b0011, 4, 5),		(0b0010, 4, 6),		(0b00011, 5, 7),
	(0b000101, 6, 8),	(0b000100, 6, 9),	(0b0000100, 7, 10),	(0b0000101, 7, 11),
	(0b0000111, 7, 12),	(0b00000100, 8, 13),	(0b00000111, 8, 14),	(0b000011000, 9, 15),
	(0b0000010111, 10, 16),	(0b0000011000, 10, 17),	(0b0000001000, 10, 18),	(0b00001100111, 11, 19),
	(0b00001101000, 11, 20),	(0b00001101100, 11, 21),	(0b00000110111, 11, 22),	(0b00000101000, 11, 23),
	(0b00000010111, 11, 24),	(0b00000011000, 11, 25),	(0b000011001010, 12, 26),	(0b000011001011, 12, 27),
	(0b000011001100, 12, 28),	(0b000011001101, 12, 29),	(0b000001101000, 12, 30),	(0b000001101001, 12, 31),
	(0b000001101010, 12, 32),	(0b000001101011, 12, 33),	(0b000011010010, 12, 34),	(0b000011010011, 12, 35),
	(0b000011010100, 12, 36),	(0b000011010101, 12, 37),	(0b000011010110, 12, 38),	(0b000011010111, 12, 39),
	(0b000001101100, 12, 40),	(0b000001101101, 12, 41),	(0b000011011010, 12, 42),	(0b000011011011, 12, 43),
	(0b000001010100, 12, 44),	(0b000001010101, 12, 45),	(0b000001010110, 12, 46),	(0b000001010111, 12, 47),
	(0b000001100100, 12, 48),	(0b000001100101, 12, 49),	(0b000001010010, 12, 50),	(0b000001010011, 12, 51),
	(0b000000100100, 12, 52),	(0b000000110111, 12, 53),	(0b000000111000, 12, 54),	(0b000000100111, 12, 55),
	(0b000000101000, 12, 56),	(0b000001011000, 12, 57),	(0b000001011001, 12, 58),	(0b000000101011, 12, 59),
	(0b000000101100, 12, 60),	(0b000001011010, 12, 61),	(0b000001100110, 12, 62),	(0b000001100111, 12, 63),
	// Make-up codes.
	(0b0000001111, 10, 64),		(0b000011001000, 12, 128),	(0b000011001001, 12, 192),
	(0b000001011011, 12, 256),	(0b000000110011, 12, 320),	(0b000000110100, 12, 384),
	(0b000000110101, 12, 448),	(0b0000001101100, 13, 512),	(0b0000001101101, 13, 576),
	(0b0000001001010, 13, 640),	(0b0000001001011, 13, 704),	(0b0000001001100, 13, 768),
	(0b0000001001101, 13, 832),	(0b0000001110010, 13, 896),	(0b0000001110011, 13, 960),
	(0b0000001110100, 13, 1024),	(0b0000001110101, 13, 1088),	(0b0000001110110, 13, 1152),
	(0b0000001110111, 13, 1216),	(0b0000001010010, 13, 1280),	(0b0000001010011, 13, 1344),
	(0b0000001010100, 13, 1408),	(0b0000001010101, 13, 1472),	(0b0000001011010, 13, 1536),
	(0b0000001011011, 13, 1600),	(0b0000001100100, 13, 1664),	(0b0000001100101, 13, 1728),
];

// The make-up codes past 1728, which white and black share.
const EXTENDED: &[Code] = &[
	(0b00000001000, 11, 1792),	(0b00000001100, 11, 1856),	(0b00000001101, 11, 1920),
	(0b000000010010, 12, 1984),	(0b000000010011, 12, 2048),	(0b000000010100, 12, 2112),
	(0b000000010101, 12, 2176),	(0b000000010110, 12, 2240),	(0b000000010111, 12, 2304),
	(0b000000011100, 12, 2368),	(0b000000011101, 12, 2432),	(0b000000011110, 12, 2496),
	(0b000000011111, 12, 2560),
];

const PEEK:	u32	= 13;		// the longest code, in bits
const EOL:	u16	= 0xffff;	// a run value marking the end-of-line code in a table

/// A table indexed by the next thirteen bits, giving the code's length and its run.
struct Table {
	slots: Vec<(u8, u16)>,	// length 0 means no code begins with these bits
}

impl Table {

	fn build(codes: &[Code]) -> Outcome<Self> {
		let mut slots = vec![(0u8, 0u16); 1 << PEEK];
		let all = codes.iter().chain(EXTENDED.iter()).chain(std::iter::once(&(1u16, 12u8, EOL)));
		for &(code, len, run) in all {
			let span = 1usize << (PEEK - len as u32);
			let base = (code as usize) << (PEEK - len as u32);
			for s in &mut slots[base..base + span] {
				if s.0 != 0 {
					return Err(err!(
						"The fax code for run {} collides with another; the table is wrong.", run;
					Bug));
				}
				*s = (len, run);
			}
		}
		Ok(Self { slots })
	}
}

/// Reads bits most significant first, from any position.
struct Bits<'a> {
	data:	&'a [u8],
	pos:	usize,	// in bits
}

impl<'a> Bits<'a> {

	fn bit(&self, at: usize) -> u32 {
		match self.data.get(at >> 3) {
			Some(b) => ((b >> (7 - (at & 7))) & 1) as u32,
			None => 0,
		}
	}

	fn peek(&self, n: u32) -> u32 {
		let mut v = 0;
		for i in 0..n as usize {
			v = (v << 1) | self.bit(self.pos + i);
		}
		v
	}

	fn left(&self) -> usize {
		(self.data.len() * 8).saturating_sub(self.pos)
	}

	fn align(&mut self) {
		self.pos = (self.pos + 7) & !7;
	}

	/// Steps over an end-of-line code, with any fill zeros before it, if one is next.
	fn eol(&mut self) -> bool {
		let mut z = 0;
		while z < self.left() && self.bit(self.pos + z) == 0 {
			z += 1;
		}
		if z >= 11 && z < self.left() {
			self.pos += z + 1;
			return true;
		}
		false
	}
}

/// Reads one run of the given colour: any make-up codes, then a terminating one.
fn run(b: &mut Bits, t: &Table, white: bool) -> Outcome<usize> {
	let mut total = 0usize;
	loop {
		if b.left() == 0 {
			return Err(err!(
				"A fax strip ends in the middle of a {} run.", if white { "white" } else { "black" };
			Invalid, Input, Decode));
		}
		let (len, v) = t.slots[b.peek(PEEK) as usize];
		if len == 0 || v == EOL {
			return Err(err!(
				"A fax strip holds no valid {} run code at bit {}.",
				if white { "white" } else { "black" }, b.pos;
			Invalid, Input, Decode));
		}
		b.pos += len as usize;
		total += v as usize;
		if v < 64 {
			return Ok(total);
		}
	}
}

// Two-dimensional modes, with the vertical offset where one applies.
enum Mode {
	Pass,
	Horizontal,
	Vertical(isize),
}

fn mode(b: &mut Bits) -> Outcome<Mode> {
	let v = b.peek(7);
	let (len, m) = if v >> 6 == 1 {
		(1, Mode::Vertical(0))
	} else if v >> 4 == 0b011 {
		(3, Mode::Vertical(1))
	} else if v >> 4 == 0b010 {
		(3, Mode::Vertical(-1))
	} else if v >> 4 == 0b001 {
		(3, Mode::Horizontal)
	} else if v >> 3 == 0b0001 {
		(4, Mode::Pass)
	} else if v >> 1 == 0b000011 {
		(6, Mode::Vertical(2))
	} else if v >> 1 == 0b000010 {
		(6, Mode::Vertical(-2))
	} else if v == 0b0000011 {
		(7, Mode::Vertical(3))
	} else if v == 0b0000010 {
		(7, Mode::Vertical(-3))
	} else {
		return Err(err!(
			"A fax strip holds no two-dimensional mode code at bit {} (uncompressed mode is not \
			read).", b.pos;
		Invalid, Input, Decode, Unimplemented));
	};
	b.pos += len;
	Ok(m)
}

/// Decodes a row coded by itself, as changes of colour.
fn row_1d(b: &mut Bits, wt: &Table, bt: &Table, w: usize, out: &mut Vec<usize>) -> Outcome<()> {
	out.clear();
	let mut x = 0usize;
	let mut white = true;
	while x < w {
		let r = res!(run(b, if white { wt } else { bt }, white));
		x = (x + r).min(w);
		out.push(x);
		white = !white;
	}
	Ok(())
}

/// Decodes a row coded against the one above, `refs`, as changes of colour.
fn row_2d(b: &mut Bits, wt: &Table, bt: &Table, w: usize, refs: &[usize], out: &mut Vec<usize>)
	-> Outcome<()>
{
	out.clear();
	let mut a0: isize = -1;
	let mut white = true;
	let mut i = 0usize; // where the search for b1 resumes
	while a0 < w as isize {
		// b1 is the first change above, right of a0, to the colour opposite a0's; changes at even
		// indices are to black and at odd ones to white.
		let want = if white { 0 } else { 1 };
		while i < refs.len() && (refs[i] as isize <= a0 || i % 2 != want) {
			i += 1;
		}
		let b1 = refs.get(i).copied().unwrap_or(w) as isize;
		let b2 = refs.get(i + 1).copied().unwrap_or(w) as isize;
		match res!(mode(b)) {
			Mode::Pass => {
				a0 = b2;
			}
			Mode::Horizontal => {
				let start = a0.max(0) as usize;
				let r1 = res!(run(b, if white { wt } else { bt }, white));
				let r2 = res!(run(b, if white { bt } else { wt }, !white));
				let a1 = (start + r1).min(w);
				let a2 = (a1 + r2).min(w);
				out.push(a1);
				out.push(a2);
				a0 = a2 as isize;
			}
			Mode::Vertical(d) => {
				let a1 = b1 + d;
				if a1 < 0 || a1 < a0 || a1 > w as isize {
					return Err(err!(
						"A fax row's vertical code puts a change at {}, outside the row of {} or \
						behind {}.", a1, w, a0;
					Invalid, Input, Decode));
				}
				out.push(a1 as usize);
				a0 = a1;
				white = !white;
			}
		}
		i = i.saturating_sub(1);
	}
	Ok(())
}

/// Paints a row's changes of colour as packed bits, black set.
fn paint(changes: &[usize], w: usize, row: &mut [u8]) {
	let mut x = 0usize;
	let mut black = false;
	for &c in changes {
		let c = c.min(w);
		if black {
			for p in x..c {
				row[p >> 3] |= 0x80 >> (p & 7);
			}
		}
		x = c;
		black = !black;
	}
	if black {
		for p in x..w {
			row[p >> 3] |= 0x80 >> (p & 7);
		}
	}
}

/// Decodes a strip of `h` rows of `w` pixels.
pub fn decode(data: &[u8], w: usize, h: usize, coding: Coding) -> Outcome<Vec<u8>> {
	let wt = res!(Table::build(WHITE));
	let bt = res!(Table::build(BLACK));
	let stride = (w + 7) / 8;
	let mut out = vec![0u8; stride * h];
	let mut b = Bits { data, pos: 0 };
	let mut refs: Vec<usize> = Vec::new();	// an imaginary white row above the first
	let mut cur: Vec<usize> = Vec::new();
	for y in 0..h {
		match coding {
			Coding::Mh => {
				res!(row_1d(&mut b, &wt, &bt, w, &mut cur));
				b.align();
			}
			Coding::T4 { two_d, aligned: _ } => {
				// Fill bits before an end-of-line are zeros, which `eol` steps over whether or
				// not the option says they are there; a row with no end-of-line before it is
				// read from where the last one ended.
				b.eol();
				let one_d = if two_d {
					let tag = b.bit(b.pos);
					b.pos += 1;
					tag == 1
				} else {
					true
				};
				if one_d {
					res!(row_1d(&mut b, &wt, &bt, w, &mut cur));
				} else {
					res!(row_2d(&mut b, &wt, &bt, w, &refs, &mut cur));
				}
			}
			Coding::T6 => {
				res!(row_2d(&mut b, &wt, &bt, w, &refs, &mut cur));
			}
		}
		paint(&cur, w, &mut out[y * stride..(y + 1) * stride]);
		std::mem::swap(&mut refs, &mut cur);
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Kraft sum of a complete prefix code is one; with the EOL and the shared make-up codes
	/// beside them neither table is complete, but both must be prefix-free, which `build` checks.
	#[test]
	fn test_the_code_tables_are_prefix_free_00() -> Outcome<()> {
		let _ = res!(Table::build(WHITE));
		let _ = res!(Table::build(BLACK));
		assert_eq!(WHITE.len(), 64 + 27);
		assert_eq!(BLACK.len(), 64 + 27);
		for (i, c) in WHITE.iter().enumerate().take(64) {
			assert_eq!(c.2 as usize, i);
		}
		for (i, c) in BLACK.iter().enumerate().take(64) {
			assert_eq!(c.2 as usize, i);
		}
		Ok(())
	}
}
