//! Turning coded coefficients back into the residual that is added to a prediction.
//!
//! Two steps, and both are integer arithmetic specified to the bit -- a decoder that rounds
//! differently from the specification does not produce a slightly different picture, it produces
//! one that drifts further from the encoder's with every block that predicts from it.
//!
//! **Scaling** (§8.6.3) undoes the quantiser: each coefficient is multiplied by a factor chosen by
//! the quantisation parameter, shifted left by that parameter divided by six, and rounded back down
//! by a shift that depends on the bit depth and the block size. The six factors are the sixth roots
//! of two to within a per cent, which is why the parameter's sixth part is a doubling.
//!
//! **The inverse transform** (§8.6.4) is a matrix multiplication down each column and then along
//! each row, with a rounding shift between the two. Two matrices exist: a four-point sine transform
//! used for the luma of four-by-four intra blocks, whose basis functions suit a residual that grows
//! away from the predicted edge, and the cosine transform everything else uses. The cosine matrix is
//! one thirty-two by thirty-two table for every size -- a sixteen-point transform reads every second
//! column of it, an eight-point every fourth, and so on -- which is what makes a single
//! implementation of the one-dimensional pass serve all four sizes.
//!
//! A block coded without a transform at all (`transform_skip_flag`) takes a rotation and a shift
//! instead, and one coded without the quantiser either (`cu_transquant_bypass_flag`) is the
//! residual already.

use oxedyne_fe2o3_core::prelude::*;

/// The largest transform this decoder will do, in samples each way.
pub const MAX_TB: usize = 32;

/// What multiplies a coefficient before the shift, by the quantisation parameter's remainder on
/// division by six (§8.6.3).
///
/// The six of them span one doubling: 40, 45, 51, 57, 64 and 72 are the sixth powers of two to
/// within a per cent, so six steps of the parameter double the step size exactly.
const LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 72];

/// The four-point sine transform, for the luma of an intra block of four (equation 8-316).
const DST_4: [[i16; 4]; 4] = [
	[29, 55, 74, 84],
	[74, 74, 0, -74],
	[84, -29, -74, 55],
	[55, -84, 74, -29],
];

/// Columns 0 to 15 of the transform matrix (§8.6.4.2, equation 8-319).
///
/// **The published table is transposed against the way the equation indexes it**: a printed row is
/// the matrix's *second* subscript and a printed column its first, so `transMatrix[m][n]` is this
/// array's `[n][m]`. [`matrix`] is the only place that knows it, and a check against the four-point
/// inverse everybody knows by heart is what settles that it is the right way round.
const DCT_COL_0_15: [[i16; 16]; 32] = [
	[64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64],
	[90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4],
	[90, 87, 80, 70, 57, 43, 25, 9, -9, -25, -43, -57, -70, -80, -87, -90],
	[90, 82, 67, 46, 22, -4, -31, -54, -73, -85, -90, -88, -78, -61, -38, -13],
	[89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89],
	[88, 67, 31, -13, -54, -82, -90, -78, -46, -4, 38, 73, 90, 85, 61, 22],
	[87, 57, 9, -43, -80, -90, -70, -25, 25, 70, 90, 80, 43, -9, -57, -87],
	[85, 46, -13, -67, -90, -73, -22, 38, 82, 88, 54, -4, -61, -90, -78, -31],
	[83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83],
	[82, 22, -54, -90, -61, 13, 78, 85, 31, -46, -90, -67, 4, 73, 88, 38],
	[80, 9, -70, -87, -25, 57, 90, 43, -43, -90, -57, 25, 87, 70, -9, -80],
	[78, -4, -82, -73, 13, 85, 67, -22, -88, -61, 31, 90, 54, -38, -90, -46],
	[75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75],
	[73, -31, -90, -22, 78, 67, -38, -90, -13, 82, 61, -46, -88, -4, 85, 54],
	[70, -43, -87, 9, 90, 25, -80, -57, 57, 80, -25, -90, -9, 87, 43, -70],
	[67, -54, -78, 38, 85, -22, -90, 4, 90, 13, -88, -31, 82, 46, -73, -61],
	[64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64],
	[61, -73, -46, 82, 31, -88, -13, 90, -4, -90, 22, 85, -38, -78, 54, 67],
	[57, -80, -25, 90, -9, -87, 43, 70, -70, -43, 87, 9, -90, 25, 80, -57],
	[54, -85, -4, 88, -46, -61, 82, 13, -90, 38, 67, -78, -22, 90, -31, -73],
	[50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50],
	[46, -90, 38, 54, -90, 31, 61, -88, 22, 67, -85, 13, 73, -82, 4, 78],
	[43, -90, 57, 25, -87, 70, 9, -80, 80, -9, -70, 87, -25, -57, 90, -43],
	[38, -88, 73, -4, -67, 90, -46, -31, 85, -78, 13, 61, -90, 54, 22, -82],
	[36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36],
	[31, -78, 90, -61, 4, 54, -88, 82, -38, -22, 73, -90, 67, -13, -46, 85],
	[25, -70, 90, -80, 43, 9, -57, 87, -87, 57, -9, -43, 80, -90, 70, -25],
	[22, -61, 85, -90, 73, -38, -4, 46, -78, 90, -82, 54, -13, -31, 67, -88],
	[18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18],
	[13, -38, 61, -78, 88, -90, 85, -73, 54, -31, 4, 22, -46, 67, -82, 90],
	[9, -25, 43, -57, 70, -80, 87, -90, 90, -87, 80, -70, 57, -43, 25, -9],
	[4, -13, 22, -31, 38, -46, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90],
];

/// Columns 16 to 31 of the same matrix (equation 8-321), indexed the same way.
const DCT_COL_16_31: [[i16; 16]; 32] = [
	[64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64],
	[-4, -13, -22, -31, -38, -46, -54, -61, -67, -73, -78, -82, -85, -88, -90, -90],
	[-90, -87, -80, -70, -57, -43, -25, -9, 9, 25, 43, 57, 70, 80, 87, 90],
	[13, 38, 61, 78, 88, 90, 85, 73, 54, 31, 4, -22, -46, -67, -82, -90],
	[89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89],
	[-22, -61, -85, -90, -73, -38, 4, 46, 78, 90, 82, 54, 13, -31, -67, -88],
	[-87, -57, -9, 43, 80, 90, 70, 25, -25, -70, -90, -80, -43, 9, 57, 87],
	[31, 78, 90, 61, 4, -54, -88, -82, -38, 22, 73, 90, 67, 13, -46, -85],
	[83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83],
	[-38, -88, -73, -4, 67, 90, 46, -31, -85, -78, -13, 61, 90, 54, -22, -82],
	[-80, -9, 70, 87, 25, -57, -90, -43, 43, 90, 57, -25, -87, -70, 9, 80],
	[46, 90, 38, -54, -90, -31, 61, 88, 22, -67, -85, -13, 73, 82, 4, -78],
	[75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75],
	[-54, -85, 4, 88, 46, -61, -82, 13, 90, 38, -67, -78, 22, 90, 31, -73],
	[-70, 43, 87, -9, -90, -25, 80, 57, -57, -80, 25, 90, 9, -87, -43, 70],
	[61, 73, -46, -82, 31, 88, -13, -90, -4, 90, 22, -85, -38, 78, 54, -67],
	[64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64],
	[-67, -54, 78, 38, -85, -22, 90, 4, -90, 13, 88, -31, -82, 46, 73, -61],
	[-57, 80, 25, -90, 9, 87, -43, -70, 70, 43, -87, -9, 90, -25, -80, 57],
	[73, 31, -90, 22, 78, -67, -38, 90, -13, -82, 61, 46, -88, 4, 85, -54],
	[50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50],
	[-78, -4, 82, -73, -13, 85, -67, -22, 88, -61, -31, 90, -54, -38, 90, -46],
	[-43, 90, -57, -25, 87, -70, -9, 80, -80, 9, 70, -87, 25, 57, -90, 43],
	[82, -22, -54, 90, -61, -13, 78, -85, 31, 46, -90, 67, 4, -73, 88, -38],
	[36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36],
	[-85, 46, 13, -67, 90, -73, 22, 38, -82, 88, -54, -4, 61, -90, 78, -31],
	[-25, 70, -90, 80, -43, -9, 57, -87, 87, -57, 9, 43, -80, 90, -70, 25],
	[88, -67, 31, 13, -54, 82, -90, 78, -46, 4, 38, -73, 90, -85, 61, -22],
	[18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18],
	[-90, 82, -67, 46, -22, -4, 31, -54, 73, -85, 90, -88, 78, -61, 38, -13],
	[-9, 25, -43, 57, -70, 80, -87, 90, -90, 87, -80, 70, -57, 43, -25, 9],
	[90, -90, 88, -85, 82, -78, 73, -67, 61, -54, 46, -38, 31, -22, 13, -4],
];
/// One entry of the cosine transform matrix, undoing the transposition of the published tables.
///
/// `m` is the equation's first subscript and `n` its second, both nought to thirty-one.
fn matrix(m: usize, n: usize) -> i32 {
	if m < 16 {
		DCT_COL_0_15[n][m] as i32
	} else {
		DCT_COL_16_31[n][m - 16] as i32
	}
}

/// Which transform a block takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// The four-point sine transform: intra luma, four by four, and nothing else (§8.6.4.1).
	Sine,
	/// The cosine transform, at whatever size the block is.
	Cosine,
}

impl Kind {

	/// Which one a block of this description takes.
	pub fn of(intra: bool, size: usize, chroma: bool) -> Self {
		if intra && size == 4 && !chroma {
			Self::Sine
		} else {
			Self::Cosine
		}
	}
}

/// Undoes the quantiser (§8.6.3), in place.
///
/// `coeffs` is the block in raster order, `size` its side, `qp` the quantisation parameter that
/// applies to it and `depth` the bit depth of the component. Scaling lists are not applied: no
/// photograph in the corpus carries one, and a sequence that does is refused where it is read
/// rather than quietly decoded with the flat matrix a picture was not coded against.
pub fn scale(coeffs: &mut [i32], size: usize, qp: i32, depth: u32) {
	// Fifteen is the transform range every profile this decoder meets uses; the extended-precision
	// flag that widens it belongs to profiles that do not appear in a photograph.
	let shift = depth as i32 + log2(size) as i32 + 10 - 15;
	let scale = LEVEL_SCALE[(qp % 6) as usize];
	let up = qp / 6;
	let round = 1i64 << (shift - 1);
	for c in coeffs.iter_mut().take(size * size) {
		// In sixty-four bits because the intermediate overflows thirty-two: a coefficient may be
		// fifteen bits, the scale seven, and the shift up to eight more.
		let v = (((*c as i64) * 16 * (scale as i64)) << up) + round;
		*c = ((v >> shift).clamp(-32_768, 32_767)) as i32;
	}
}

/// The base-two logarithm of a power of two.
fn log2(n: usize) -> u32 {
	n.trailing_zeros()
}

/// One dimension of the inverse transform (§8.6.4.2).
///
/// `src` holds `size` coefficients and `dst` takes `size` samples. The cosine matrix is read with a
/// stride, which is what lets one table of thirty-two serve every size.
fn one_way(src: &[i32], dst: &mut [i32], size: usize, kind: Kind) {
	match kind {
		Kind::Sine => for i in 0..4 {
			let mut sum = 0i64;
			for j in 0..4 {
				sum += (DST_4[j][i] as i64) * (src[j] as i64);
			}
			dst[i] = sum as i32;
		},
		Kind::Cosine => {
			let stride = 32 / size;
			for i in 0..size {
				let mut sum = 0i64;
				for j in 0..size {
					sum += (matrix(i, j * stride) as i64) * (src[j] as i64);
				}
				dst[i] = sum as i32;
			}
		},
	}
}

/// The whole inverse transform: columns, a rounding shift, then rows (§8.6.4.1).
///
/// `block` arrives holding scaled coefficients in raster order and leaves holding the transform's
/// output, which is **not yet the residual** -- [`finish`] applies the last shift, and it applies it
/// to a skipped block too, which is what keeps the two paths in the same units.
///
/// The clip between the two passes is the specification's and is load-bearing: doing both passes
/// and rounding once at the end is arithmetically similar and produces a different picture.
pub fn inverse(block: &mut [i32], size: usize, kind: Kind) -> Outcome<()> {
	if size > MAX_TB || !size.is_power_of_two() || size < 4 {
		return Err(err!("A transform of {} samples was asked for.", size; Invalid, Input));
	}
	if block.len() < size * size {
		return Err(err!(
			"A transform of {0} wants {1} coefficients and was given {2}.",
			size, size * size, block.len(); Invalid, Input));
	}
	let mut col = [0i32; MAX_TB];
	let mut out = [0i32; MAX_TB];
	// Down each column.
	for x in 0..size {
		for y in 0..size {
			col[y] = block[y * size + x];
		}
		one_way(&col[..size], &mut out[..size], size, kind);
		for y in 0..size {
			// Clipped to sixteen bits, which is what keeps the second pass inside the range its
			// matrix was designed for.
			block[y * size + x] = ((out[y] + 64) >> 7).clamp(-32_768, 32_767);
		}
	}
	// Then along each row, whose output the specification does not clip.
	for y in 0..size {
		col[..size].copy_from_slice(&block[y * size..y * size + size]);
		one_way(&col[..size], &mut out[..size], size, kind);
		block[y * size..y * size + size].copy_from_slice(&out[..size]);
	}
	Ok(())
}

/// A block coded without its transform (§8.6.2, equation 8-298): one shift left.
///
/// It stands where [`inverse`] would, and [`finish`] follows it just the same.
pub fn skipped(block: &mut [i32], size: usize) {
	let shift = 5 + log2(size);
	for v in block.iter_mut().take(size * size) {
		*v <<= shift;
	}
}

/// The last shift, which turns either path's output into residual samples (equation 8-299).
pub fn finish(block: &mut [i32], size: usize, depth: u32) {
	let shift = 20 - depth as i32;
	if shift <= 0 {
		return;
	}
	let round = 1i32 << (shift - 1);
	for v in block.iter_mut().take(size * size) {
		*v = (*v + round) >> shift;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The four-point inverse transforms, written out as the arithmetic everybody who has
	/// implemented one knows by heart, rather than as a table lookup.
	///
	/// This is what settles the orientation of the published matrices, which are printed
	/// transposed against the way the equation subscripts them. A decoder that reads them the other
	/// way round still produces a picture -- a wrong one, in a way no amount of staring at the table
	/// reveals.
	fn known_dct_4(x: [i32; 4]) -> [i32; 4] {
		[
			64 * x[0] + 83 * x[1] + 64 * x[2] + 36 * x[3],
			64 * x[0] + 36 * x[1] - 64 * x[2] - 83 * x[3],
			64 * x[0] - 36 * x[1] - 64 * x[2] + 83 * x[3],
			64 * x[0] - 83 * x[1] + 64 * x[2] - 36 * x[3],
		]
	}

	fn known_dst_4(x: [i32; 4]) -> [i32; 4] {
		[
			29 * x[0] + 74 * x[1] + 84 * x[2] + 55 * x[3],
			55 * x[0] + 74 * x[1] - 29 * x[2] - 84 * x[3],
			74 * x[0] - 74 * x[2] + 74 * x[3],
			84 * x[0] - 74 * x[1] + 55 * x[2] - 29 * x[3],
		]
	}

	#[test]
	fn test_the_four_point_transforms_are_the_ones_everybody_knows_00() -> Outcome<()> {
		let cases: [[i32; 4]; 5] = [
			[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1], [17, -9, 240, -1000],
		];
		for x in cases {
			let mut got = [0i32; 4];
			one_way(&x, &mut got, 4, Kind::Cosine);
			req!(got, known_dct_4(x), "the cosine matrix is the wrong way round for {:?}", x);
			let mut got = [0i32; 4];
			one_way(&x, &mut got, 4, Kind::Sine);
			req!(got, known_dst_4(x), "the sine matrix is the wrong way round for {:?}", x);
		}
		Ok(())
	}

	#[test]
	fn test_every_size_of_the_cosine_matrix_is_orthogonal_01() -> Outcome<()> {
		// The property that makes it a transform at all, and one this decoder's own arithmetic
		// cannot fake: the rows are at right angles to each other and all the same length. A
		// transposition, a wrong stride or a mistyped entry breaks it, and none of those is visible
		// by reading the table.
		//
		// The integer matrix approximates an orthogonal one, so the off-diagonal terms are not
		// quite nought. Measured against the diagonal, the worst of them is one part in 550 at
		// sixteen points, 682 at thirty-two, 1,310 at eight and 2,340 at four, and the worst error
		// in a row's own length is one part in 1,560. The bounds below sit just outside those. A
		// single mistyped entry is worth far more than either: the smallest entry in the table is 4
		// against a diagonal of 16,384, and getting one wrong moves a dot product by hundreds.
		for size in [4usize, 8, 16, 32] {
			let stride = 32 / size;
			let mut worst = 0i64;
			let diagonal = 64i64 * 64 * size as i64;
			for i in 0..size {
				for k in 0..size {
					let mut dot = 0i64;
					for j in 0..size {
						dot += (matrix(i, j * stride) as i64) * (matrix(k, j * stride) as i64);
					}
					if i == k {
						// Every row the same length, which is what keeps the picture's gain flat.
						let off = (dot - diagonal).abs();
						let near = off * 1_000 <= diagonal;
						req!(near, true,
							"row {} of the {}-point matrix has length squared {}, not {}",
							i, size, dot, diagonal);
					} else if dot.abs() > worst {
						worst = dot.abs();
					}
				}
			}
			let square = worst * 500 <= diagonal;
			req!(square, true,
				"two rows of the {}-point matrix are {} out of square, against a diagonal of {}",
				size, worst, diagonal);
		}
		Ok(())
	}

	#[test]
	fn test_a_flat_block_comes_out_of_one_coefficient_02() -> Outcome<()> {
		// What a decoder does far more often than anything else: a block whose only coefficient is
		// the direct one, which must come back as a single level with no pattern in it. A stride
		// worked out wrongly puts ripples in this, and ripples in a flat sky are exactly the
		// artefact somebody reports.
		for size in [4usize, 8, 16, 32] {
			let mut block = vec![0i32; size * size];
			block[0] = 512;
			// Not the sine transform: that one is only ever used on four-by-four intra luma, where
			// its basis is deliberately not flat.
			res!(inverse(&mut block, size, Kind::Cosine));
			finish(&mut block, size, 8);
			let first = block[0];
			for (i, v) in block.iter().enumerate() {
				req!(*v, first, "sample {} of a {}-point flat block is not flat", i, size);
			}
			let something = first != 0;
			req!(something, true, "a {}-point block of 512 came back empty", size);
		}
		Ok(())
	}

	#[test]
	fn test_the_quantiser_steps_by_the_sixth_root_of_two_03() -> Outcome<()> {
		// The published factors, against the arithmetic they approximate. Six steps of the
		// quantisation parameter is meant to be one doubling of the step size, so consecutive
		// factors should stand in the ratio of the sixth root of two -- and the sixth one, stepped
		// once more, should land on twice the first. Nothing in this decoder is consulted; the
		// oracle is the arithmetic the table was built from.
		let root = 2f64.powf(1.0 / 6.0);
		for k in 0..5 {
			let ratio = LEVEL_SCALE[k + 1] as f64 / LEVEL_SCALE[k] as f64;
			let close = (ratio - root).abs() < 0.02;
			req!(close, true,
				"levelScale {} to {} is a ratio of {:.4}, and the sixth root of two is {:.4}",
				LEVEL_SCALE[k], LEVEL_SCALE[k + 1], ratio, root);
		}
		let wrapped = LEVEL_SCALE[5] as f64 * root;
		let octave = (wrapped - 2.0 * LEVEL_SCALE[0] as f64).abs() < 2.0;
		req!(octave, true,
			"one step past the last factor is {:.1}, and twice the first is {}",
			wrapped, 2 * LEVEL_SCALE[0]);

		// And the scaling itself follows the table: six steps up doubles what comes out, to within
		// the rounding the shift cannot avoid.
		// A coefficient of one, so that the highest parameter still lands well inside the
		// sixteen-bit range the scaled coefficients are clipped to -- a saturated value would
		// double into itself and prove nothing.
		for qp in 0..46i32 {
			let mut low = [1i32; 16];
			let mut high = [1i32; 16];
			scale(&mut low, 4, qp, 8);
			scale(&mut high, 4, qp + 6, 8);
			let doubled = (high[0] - low[0] * 2).abs() <= 1;
			req!(doubled, true,
				"qp {} scales to {} and qp {} to {}", qp, low[0], qp + 6, high[0]);
		}
		Ok(())
	}

	#[test]
	fn test_a_skipped_block_lands_in_the_same_units_04() -> Outcome<()> {
		// A block coded without its transform takes a shift instead, and the two paths have to
		// arrive in the same units or a picture that mixes them is a picture with a step in it.
		// A flat block through the transform and the same block through the shift agree.
		for size in [4usize, 8] {
			let mut through = vec![0i32; size * size];
			// The coefficient that yields a flat block of one at this size.
			through[0] = 1;
			res!(inverse(&mut through, size, Kind::Cosine));
			finish(&mut through, size, 8);

			let mut around = vec![0i32; size * size];
			around[0] = 1;
			skipped(&mut around, size);
			finish(&mut around, size, 8);
			// The transform spreads the one over the block and the shift leaves it in the corner,
			// so what is compared is the level, not the position.
			req!(around[0], through[0] * (size as i32),
				"the two paths disagree at {} by more than the transform's own gain", size);
		}
		Ok(())
	}

	#[test]
	fn test_the_matrix_is_the_published_one_05() -> Outcome<()> {
		// The same discipline the context tables are held to: a thousand and twenty-four numbers
		// copied out of a document, checked against the document.
		//
		//   HEVC_SPEC_TEXT=~/.cache/specs/h265.txt cargo test -p oxedyne_fe2o3_graphics hevc
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
		for (marker, held) in [
			("transMatrixCol0to15 =", &DCT_COL_0_15),
			("transMatrixCol16to31 =", &DCT_COL_16_31),
		] {
			let at = match text.find(marker) {
				Some(at) => at,
				None => return Err(err!("{} is not in {}.", marker, path; Test, Missing)),
			};
			let mut rows: Vec<Vec<i32>> = Vec::new();
			for line in text[at..].lines() {
				let t = line.trim();
				if !t.starts_with('{') || !t.trim_end_matches(',').ends_with('}') {
					continue;
				}
				let inner = t.trim_end_matches(',').trim_start_matches('{').trim_end_matches('}');
				let mut row = Vec::new();
				let mut ok = true;
				for word in inner.split_whitespace() {
					// The document writes a minus sign as U+2212, not as a hyphen.
					match word.replace('\u{2212}', "-").parse::<i32>() {
						Ok(n) => row.push(n),
						Err(_) => { ok = false; break; },
					}
				}
				if ok && row.len() == 16 {
					rows.push(row);
				}
				if rows.len() == 32 {
					break;
				}
			}
			req!(rows.len(), 32usize, "{} does not have thirty-two rows under it", marker);
			for (n, row) in rows.iter().enumerate() {
				for (m, v) in row.iter().enumerate() {
					req!(held[n][m] as i32, *v,
						"{} row {} column {} is {} and the document says {}",
						marker, n, m, held[n][m], v);
				}
			}
		}
		Ok(())
	}
}
