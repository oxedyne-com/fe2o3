//! The inverse transforms, and the quantisation that feeds them.
//!
//! H.264's transforms are integer and exact: unlike a discrete cosine transform they are specified
//! as arithmetic rather than as a mathematical ideal, so two decoders that follow the clauses agree
//! sample for sample and there is no drift to accumulate. There are four of them, and a picture
//! uses all four:
//!
//! - The **four-by-four** transform (§8.5.12.2), which is what most residual is coded in.
//! - The **eight-by-eight** transform (§8.5.13.2), which High profile adds and which 861 films in
//!   the corpus turn on.
//! - A **four-by-four Hadamard** over the sixteen DC coefficients of an `Intra_16x16` macroblock
//!   (§8.5.10), which is how a flat region is coded cheaply: the sixteen blocks' DC terms are
//!   themselves transformed together.
//! - A **two-by-two Hadamard** over the four chroma DC coefficients (§8.5.11.1).
//!
//! # The part that quietly ruins a picture
//!
//! Quantisation. `LevelScale` is the product of a *weight* -- from the scaling lists, which are the
//! flat 16 only when the stream carries none at all -- and a *norm adjustment*, which is a
//! six-by-six table indexed by the quantisation parameter modulo six and by where in the block the
//! coefficient sits (§8.5.9). Both halves are easy to get subtly wrong, and neither failure looks
//! like a failure: the picture comes out, and it is the wrong picture. The norm adjustment table is
//! parsed out of the published specification by the tests rather than checked against the decoder
//! that uses it.

use crate::h264::{
	Scaling,
	DEFAULT_4X4_INTRA,
};

use oxedyne_fe2o3_core::prelude::*;

/// Where the four-by-four zig-zag scan puts each coefficient, as a raster index (Table 8-13).
///
/// The list is `idx` to `c_ij`, and `c_ij` is row `i` and column `j`, so the entry is `i * 4 + j`.
pub const ZIGZAG_4X4: [usize; 16] = [
	0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15,
];

/// The same for the eight-by-eight scan (Table 8-14).
pub const ZIGZAG_8X8: [usize; 64] = [
	 0,  1,  8, 16,  9,  2,  3, 10,
	17, 24, 32, 25, 18, 11,  4,  5,
	12, 19, 26, 33, 40, 48, 41, 34,
	27, 20, 13,  6,  7, 14, 21, 28,
	35, 42, 49, 56, 57, 50, 43, 36,
	29, 22, 15, 23, 30, 37, 44, 51,
	58, 59, 52, 45, 38, 31, 39, 46,
	53, 60, 61, 54, 47, 55, 62, 63,
];

/// The norm adjustment for the four-by-four transform, `v` of equation 8-315.
///
/// Six rows, one for each quantisation parameter modulo six, and three columns: the value for a
/// coefficient at an even row and even column, the value for an odd row and odd column, and the
/// value for everything else.
pub const NORM_4X4: [[i32; 3]; 6] = [
	[10, 16, 13],
	[11, 18, 14],
	[13, 20, 16],
	[14, 23, 18],
	[16, 25, 20],
	[18, 29, 23],
];

/// The norm adjustment for the eight-by-eight transform, `v` of equation 8-318.
///
/// Six rows again, and six columns for the six cases equation 8-317 distinguishes.
pub const NORM_8X8: [[i32; 6]; 6] = [
	[20, 18, 32, 19, 25, 24],
	[22, 19, 35, 21, 28, 26],
	[26, 23, 42, 24, 33, 31],
	[28, 25, 45, 26, 35, 33],
	[32, 28, 51, 30, 40, 38],
	[36, 32, 58, 34, 46, 43],
];

/// The chroma quantisation parameter for each luma one from 30 upward (Table 8-15).
///
/// Below 30 the two are equal; from 30 the chroma parameter climbs more slowly, so that chroma is
/// quantised less harshly than luma where luma is already coarse.
const CHROMA_QP: [i32; 22] = [
	29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36,
	36, 37, 37, 37, 38, 38, 38, 39, 39, 39, 39,
];

/// The chroma quantisation parameter for a luma one and a component's offset (§8.5.8).
pub fn chroma_qp(luma_qp: i32, offset: i32) -> i32 {
	let qpi = (luma_qp + offset).clamp(0, 51);
	if qpi < 30 {
		qpi
	} else {
		// The index cannot escape the table: `qpi` is clamped to 51 above, and 51 − 30 is 21.
		CHROMA_QP[(qpi - 30) as usize]
	}
}

/// The weights one block quantises against, already inverse-scanned into raster order.
///
/// Held per macroblock kind rather than looked up per coefficient, because the lookup is the same
/// for every coefficient of a block and the inverse scan is not free.
#[derive(Clone, Debug)]
pub struct Weights {
	/// `LevelScale4x4[m][i * 4 + j]`, for the six quantisation residues and the sixteen positions.
	pub l4:	[[i32; 16]; 6],
	/// `LevelScale8x8[m][i * 8 + j]`.
	pub l8:	[[i32; 64]; 6],
}

impl Weights {

	/// The weights for one colour component of an intra macroblock.
	///
	/// `component` is 0 for luma, 1 for Cb and 2 for Cr, which is the `iYCbCr` of §8.5.9. Only the
	/// intra lists are read, because every macroblock this decoder meets is intra.
	pub fn intra(scaling: &Scaling, component: usize) -> Self {
		let flat4 = scaling.l4[component.min(2)];
		let flat8 = scaling.l8[(2 * component.min(2)).min(5)];
		let mut weight4 = [0i32; 16];
		for (idx, at) in ZIGZAG_4X4.iter().enumerate() {
			weight4[*at] = flat4[idx] as i32;
		}
		let mut weight8 = [0i32; 64];
		for (idx, at) in ZIGZAG_8X8.iter().enumerate() {
			weight8[*at] = flat8[idx] as i32;
		}
		let mut l4 = [[0i32; 16]; 6];
		for (m, row) in l4.iter_mut().enumerate() {
			for i in 0..4 {
				for j in 0..4 {
					let which = match (i % 2, j % 2) {
						(0, 0)	=> 0,
						(1, 1)	=> 1,
						_	=> 2,
					};
					row[i * 4 + j] = weight4[i * 4 + j] * NORM_4X4[m][which];
				}
			}
		}
		let mut l8 = [[0i32; 64]; 6];
		for (m, row) in l8.iter_mut().enumerate() {
			for i in 0..8 {
				for j in 0..8 {
					// The six cases of equation 8-317, in the order the specification lists them.
					let which = if i % 4 == 0 && j % 4 == 0 {
						0
					} else if i % 2 == 1 && j % 2 == 1 {
						1
					} else if i % 4 == 2 && j % 4 == 2 {
						2
					} else if (i % 4 == 0 && j % 2 == 1) || (i % 2 == 1 && j % 4 == 0) {
						3
					} else if (i % 4 == 0 && j % 4 == 2) || (i % 4 == 2 && j % 4 == 0) {
						4
					} else {
						5
					};
					row[i * 8 + j] = weight8[i * 8 + j] * NORM_8X8[m][which];
				}
			}
		}
		Self { l4, l8 }
	}
}

/// Scales a four-by-four block's coefficients (§8.5.12.1).
///
/// `skip_dc` is set where the block's DC term has already been scaled elsewhere -- in an
/// `Intra_16x16` macroblock, where the sixteen DC terms are transformed together, and in every
/// chroma block, where the four are.
pub fn scale_4x4(c: &[i32; 16], w: &Weights, qp: i32, skip_dc: bool) -> [i32; 16] {
	let m = (qp.rem_euclid(6)) as usize;
	let shift = qp.div_euclid(6);
	let mut d = [0i32; 16];
	for at in 0..16 {
		if at == 0 && skip_dc {
			d[0] = c[0];
			continue;
		}
		let scaled = c[at] * w.l4[m][at];
		d[at] = if shift >= 4 {
			scaled << (shift - 4)
		} else {
			(scaled + (1 << (3 - shift))) >> (4 - shift)
		};
	}
	d
}

/// Scales an eight-by-eight block's coefficients (§8.5.13.1).
pub fn scale_8x8(c: &[i32; 64], w: &Weights, qp: i32) -> [i32; 64] {
	let m = (qp.rem_euclid(6)) as usize;
	let shift = qp.div_euclid(6);
	let mut d = [0i32; 64];
	for at in 0..64 {
		let scaled = c[at] * w.l8[m][at];
		d[at] = if shift >= 6 {
			scaled << (shift - 6)
		} else {
			(scaled + (1 << (5 - shift))) >> (6 - shift)
		};
	}
	d
}

/// The inverse four-by-four transform (§8.5.12.2), including the final rounding shift.
pub fn inverse_4x4(d: &[i32; 16]) -> [i32; 16] {
	let mut f = [0i32; 16];
	// Each row.
	for i in 0..4 {
		let r = i * 4;
		let e0 = d[r] + d[r + 2];
		let e1 = d[r] - d[r + 2];
		let e2 = (d[r + 1] >> 1) - d[r + 3];
		let e3 = d[r + 1] + (d[r + 3] >> 1);
		f[r] = e0 + e3;
		f[r + 1] = e1 + e2;
		f[r + 2] = e1 - e2;
		f[r + 3] = e0 - e3;
	}
	let mut h = [0i32; 16];
	// Then each column.
	for j in 0..4 {
		let g0 = f[j] + f[8 + j];
		let g1 = f[j] - f[8 + j];
		let g2 = (f[4 + j] >> 1) - f[12 + j];
		let g3 = f[4 + j] + (f[12 + j] >> 1);
		h[j] = g0 + g3;
		h[4 + j] = g1 + g2;
		h[8 + j] = g1 - g2;
		h[12 + j] = g0 - g3;
	}
	let mut r = [0i32; 16];
	for at in 0..16 {
		r[at] = (h[at] + 32) >> 6;
	}
	r
}

/// One pass of the inverse eight-by-eight transform along a row of eight (§8.5.13.2).
fn pass_8(d: [i32; 8]) -> [i32; 8] {
	let e0 = d[0] + d[4];
	let e1 = -d[3] + d[5] - d[7] - (d[7] >> 1);
	let e2 = d[0] - d[4];
	let e3 = d[1] + d[7] - d[3] - (d[3] >> 1);
	let e4 = (d[2] >> 1) - d[6];
	let e5 = -d[1] + d[7] + d[5] + (d[5] >> 1);
	let e6 = d[2] + (d[6] >> 1);
	let e7 = d[3] + d[5] + d[1] + (d[1] >> 1);
	let f0 = e0 + e6;
	let f1 = e1 + (e7 >> 2);
	let f2 = e2 + e4;
	let f3 = e3 + (e5 >> 2);
	let f4 = e2 - e4;
	let f5 = (e3 >> 2) - e5;
	let f6 = e0 - e6;
	let f7 = e7 - (e1 >> 2);
	[
		f0 + f7,
		f2 + f5,
		f4 + f3,
		f6 + f1,
		f6 - f1,
		f4 - f3,
		f2 - f5,
		f0 - f7,
	]
}

/// The inverse eight-by-eight transform (§8.5.13.2), including the final rounding shift.
pub fn inverse_8x8(d: &[i32; 64]) -> [i32; 64] {
	let mut g = [0i32; 64];
	for i in 0..8 {
		let mut row = [0i32; 8];
		row.copy_from_slice(&d[i * 8..i * 8 + 8]);
		let out = pass_8(row);
		g[i * 8..i * 8 + 8].copy_from_slice(&out);
	}
	let mut m = [0i32; 64];
	for j in 0..8 {
		let mut col = [0i32; 8];
		for i in 0..8 {
			col[i] = g[i * 8 + j];
		}
		let out = pass_8(col);
		for i in 0..8 {
			m[i * 8 + j] = out[i];
		}
	}
	let mut r = [0i32; 64];
	for at in 0..64 {
		r[at] = (m[at] + 32) >> 6;
	}
	r
}

/// The inverse Hadamard and scaling over an `Intra_16x16` macroblock's sixteen DC terms (§8.5.10).
pub fn luma_dc(c: &[i32; 16], w: &Weights, qp: i32) -> [i32; 16] {
	let mut f = [0i32; 16];
	// Rows.
	for i in 0..4 {
		let r = i * 4;
		let a = c[r] + c[r + 1] + c[r + 2] + c[r + 3];
		let b = c[r] + c[r + 1] - c[r + 2] - c[r + 3];
		let d = c[r] - c[r + 1] - c[r + 2] + c[r + 3];
		let e = c[r] - c[r + 1] + c[r + 2] - c[r + 3];
		f[r] = a;
		f[r + 1] = b;
		f[r + 2] = d;
		f[r + 3] = e;
	}
	let mut g = [0i32; 16];
	// Columns.
	for j in 0..4 {
		let a = f[j] + f[4 + j] + f[8 + j] + f[12 + j];
		let b = f[j] + f[4 + j] - f[8 + j] - f[12 + j];
		let d = f[j] - f[4 + j] - f[8 + j] + f[12 + j];
		let e = f[j] - f[4 + j] + f[8 + j] - f[12 + j];
		g[j] = a;
		g[4 + j] = b;
		g[8 + j] = d;
		g[12 + j] = e;
	}
	let m = (qp.rem_euclid(6)) as usize;
	let shift = qp.div_euclid(6);
	let level = w.l4[m][0];
	let mut out = [0i32; 16];
	for at in 0..16 {
		out[at] = if qp >= 36 {
			(g[at] * level) << (shift - 6)
		} else {
			(g[at] * level + (1 << (5 - shift))) >> (6 - shift)
		};
	}
	out
}

/// The inverse Hadamard and scaling over a 4:2:0 chroma block's four DC terms (§8.5.11).
pub fn chroma_dc(c: &[i32; 4], w: &Weights, qp: i32) -> [i32; 4] {
	// The two-by-two transform of equation 8-324, with `c` in raster order.
	let f = [
		c[0] + c[1] + c[2] + c[3],
		c[0] - c[1] + c[2] - c[3],
		c[0] + c[1] - c[2] - c[3],
		c[0] - c[1] - c[2] + c[3],
	];
	let m = (qp.rem_euclid(6)) as usize;
	let shift = qp.div_euclid(6);
	let level = w.l4[m][0];
	let mut out = [0i32; 4];
	for at in 0..4 {
		// Equation 8-326: a left shift by the whole part and then a right shift by five. Written as
		// one shift it would be wrong, because the left shift happens first and may carry bits into
		// the top that a combined shift would drop.
		out[at] = ((f[at] * level) << shift) >> 5;
	}
	out
}

/// The default weights an intra picture uses when nothing else applies.
///
/// Kept beside the transforms because a caller assembling a picture wants the flat case without
/// building a whole [`Scaling`] to get it.
pub fn flat_intra_weights() -> Weights {
	Weights::intra(&Scaling::flat(), 0)
}

/// Whether a set of weights is the flat one, which is what most films quantise against.
pub fn is_flat(scaling: &Scaling) -> bool {
	scaling.l4.iter().all(|l| l.iter().all(|v| *v == 16))
		&& scaling.l8.iter().all(|l| l.iter().all(|v| *v == 16))
}

/// A reminder that the default intra list is not flat, used by the tests.
pub const DEFAULT_IS_NOT_FLAT: [u8; 16] = DEFAULT_4X4_INTRA;

#[cfg(test)]
mod tests {
	use super::*;

	/// The lines of a text rendering of the specification, where one is to hand.
	fn spec() -> Option<Vec<String>> {
		let path = match std::env::var("H264_SPEC_TEXT") {
			Ok(p) => p,
			Err(_) => {
				println!("  skipped: set H264_SPEC_TEXT to a text rendering of Rec. ITU-T H.264");
				return None;
			},
		};
		match std::fs::read_to_string(&path) {
			Ok(t) => Some(t.lines().map(|l| l.to_string()).collect()),
			Err(e) => {
				println!("  skipped: {} would not read ({})", path, e);
				None
			},
		}
	}

	#[test]
	fn test_the_scans_are_the_published_ones_01() -> Outcome<()> {
		// Eighty positions copied out of a document, every one of which puts a coefficient in the
		// wrong place if it is wrong -- and a coefficient in the wrong place is a picture with a
		// texture that is not the one that was photographed, not an error. Tables 8-13 and 8-14
		// give them as `cij` labels in a row, so this reads the labels.
		//
		// The two tables must be told apart by their headings and not by the labels, because every
		// label in the four-by-four table is a legal eight-by-eight one: reading Table 8-13's row
		// as though it were Table 8-14's yields a plausible sixteen-entry scan that is wrong.
		let lines = match spec() {
			Some(l) => l,
			None => return Ok(()),
		};
		for (want, number, side) in [
			(&ZIGZAG_4X4[..], 13usize, 4usize),
			(&ZIGZAG_8X8[..], 14usize, 8usize),
		] {
			let heading = fmt!("Table 8-{} ", number);
			let mut found: Vec<usize> = Vec::new();
			let mut inside = false;
			for line in &lines {
				let trimmed = line.trim_start();
				if trimmed.starts_with("Table 8-") {
					// A heading, and its continuations, belong to whichever table they name.
					inside = trimmed.starts_with(&heading);
					continue;
				}
				if !inside || !trimmed.starts_with("zig-zag") {
					continue;
				}
				for word in trimmed.trim_start_matches("zig-zag").split_whitespace() {
					let digits: Vec<char> = word.chars().skip(1).collect();
					if !word.starts_with('c') || digits.len() != 2 {
						return Err(err!(
							"Table 8-{}'s zig-zag row holds {:?}, which is not a coefficient \
							label.", number, word; Test, Invalid));
					}
					let (i, j) = match (digits[0].to_digit(10), digits[1].to_digit(10)) {
						(Some(i), Some(j)) if (i as usize) < side && (j as usize) < side =>
							(i as usize, j as usize),
						_ => return Err(err!(
							"Table 8-{} names {}, which is outside a {} by {} block.",
							number, word, side, side; Test, Invalid)),
					};
					found.push(i * side + j);
				}
			}
			// The table of contents carries the heading too, with nothing under it, so what is
			// gathered is one whole scan and no more.
			if found != want {
				return Err(err!(
					"Table 8-{}: this decoder scans {:?} and the specification scans {:?}.",
					number, want, found; Test, Mismatch));
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_flat_direct_current_survives_the_transform_02() -> Outcome<()> {
		// The one property of the inverse transform that can be stated without a second decoder: a
		// block whose only coefficient is the direct current term comes out flat, at a level the
		// coefficient sets. A transform with a sign wrong somewhere in the butterfly still produces
		// a flat block from a flat input, so this is checked with a second, uneven input too.
		let mut d = [0i32; 16];
		d[0] = 64;
		let r = inverse_4x4(&d);
		for v in r {
			req!(v, 1, "a block with only a direct current term did not come out flat");
		}
		let mut d = [0i32; 64];
		d[0] = 64;
		let r = inverse_8x8(&d);
		for v in r {
			req!(v, 1, "an eight-by-eight block with only a direct current term was not flat");
		}
		// A single coefficient one place along the first row is a horizontal ramp, which is to say
		// the left half and the right half of every row differ in sign.
		let mut d = [0i32; 16];
		d[1] = 128;
		let r = inverse_4x4(&d);
		for i in 0..4 {
			let left = r[i * 4];
			let right = r[i * 4 + 3];
			let opposed = left > 0 && right < 0;
			req!(opposed, true, "row {} of a horizontal ramp runs {:?}", i, &r[i * 4..i * 4 + 4]);
		}
		Ok(())
	}

	#[test]
	fn test_the_norm_adjustments_are_the_published_ones_03() -> Outcome<()> {
		// Fifty-four numbers, each of which multiplies every coefficient it touches. One wrong
		// entry quantises one class of coefficient wrongly at one of the six quantisation
		// residues, which is a picture that is right five times in six and wrong the sixth -- the
		// hardest kind of fault to see and the easiest to introduce by copying a matrix out of a
		// PDF whose rows have been reflowed.
		//
		// The matrices are printed as bracketed rows around equations 8-315 and 8-318, and the
		// brackets, the `v=` and the equation number land on lines of their own. So what is
		// gathered is every line that is nothing but the right count of integers, and what is
		// looked for is six of them in a row that say what this module says.
		let lines = match spec() {
			Some(l) => l,
			None => return Ok(()),
		};
		for (want, width, eq) in [
			(NORM_4X4.iter().flatten().copied().collect::<Vec<i32>>(), 3usize, "8-315"),
			(NORM_8X8.iter().flatten().copied().collect::<Vec<i32>>(), 6usize, "8-318"),
		] {
			let rows: Vec<Vec<i32>> = lines.iter()
				.filter_map(|l| {
					// The PDF draws the matrix's brackets as glyphs in the private use area, and
					// they arrive stuck to the numbers beside them, so a token like `\u{f0ea}13`
					// parses as nothing at all. Everything outside ASCII becomes a space.
					let plain: String = l.chars()
						.map(|c| if c.is_ascii() { c } else { ' ' })
						.collect();
					let words: Vec<&str> = plain.split_whitespace().collect();
					if words.len() != width {
						return None;
					}
					let nums: Vec<i32> = words.iter().filter_map(|w| w.parse::<i32>().ok())
						.collect();
					if nums.len() == width { Some(nums) } else { None }
				})
				.collect();
			let mut got = false;
			for start in 0..rows.len().saturating_sub(5) {
				let run: Vec<i32> = rows[start..start + 6].iter().flatten().copied().collect();
				if run == want {
					got = true;
					break;
				}
			}
			if !got {
				return Err(err!(
					"Equation {}'s matrix, {:?}, is not in the specification text as six rows of \
					{}.", eq, want, width; Test, Mismatch));
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_chroma_quantiser_bends_away_from_the_luma_one_04() -> Outcome<()> {
		// Below thirty the two are equal; above it the chroma one climbs more slowly, and at the
		// top it stops at 39 while luma runs on to 51. A decoder that used the luma parameter for
		// chroma would produce a picture whose colour is quantised far too coarsely wherever the
		// picture is already coarse, which looks like blotches of colour in shadow.
		for qp in 0..30 {
			req!(chroma_qp(qp, 0), qp, "below thirty the two parameters part company");
		}
		req!(chroma_qp(30, 0), 29);
		req!(chroma_qp(39, 0), 35);
		req!(chroma_qp(51, 0), 39, "the chroma parameter runs past its ceiling");
		// The offset is applied before the table, and the sum is clipped into range first.
		req!(chroma_qp(51, 12), 39);
		req!(chroma_qp(0, -12), 0, "a negative index was not clipped");
		req!(chroma_qp(40, -12), 28);
		Ok(())
	}

	#[test]
	fn test_the_weights_are_not_flat_where_the_lists_are_not_05() -> Outcome<()> {
		// The whole point of carrying the scaling lists through: a stream whose lists are the
		// default intra matrix must quantise differently from one whose lists are flat. If these
		// two agreed, every one of the 33 films in the corpus that carries lists would decode as
		// though it carried none.
		let flat = Weights::intra(&Scaling::flat(), 0);
		let mut defaults = Scaling::flat();
		defaults.l4[0] = DEFAULT_4X4_INTRA;
		let scaled = Weights::intra(&defaults, 0);
		let same = flat.l4[0] == scaled.l4[0];
		req!(same, false, "the default intra list quantises exactly as a flat one does");
		// And the direct current term is the one place they agree, since the default list's first
		// entry is 6 and a flat one's is 16 -- so in fact they must differ there too.
		let dc_same = flat.l4[0][0] == scaled.l4[0][0];
		req!(dc_same, false, "the direct current weight is the same either way");
		Ok(())
	}
}
