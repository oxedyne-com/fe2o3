//! The deblocking filter (§8.7).
//!
//! Every block of the picture was predicted and transformed on its own, so the samples either side
//! of a block boundary were arrived at by different routes and rarely meet smoothly. At a low
//! quantisation the step is invisible; at a high one the picture is a grid. The filter smooths those
//! steps -- and only those: it is told where the boundaries are, and it decides at each one whether
//! the step across it is small enough to be an artefact of coding rather than an edge that was in
//! the photograph.
//!
//! # It is not optional
//!
//! This is a normative in-loop filter, not a post-process. A decoder that leaves it out does not
//! produce a slightly softer picture; it produces a **different** picture, and every later frame
//! predicted from it diverges further. For a still frame drawn from the first picture of a film the
//! divergence stops there, but the samples still differ from what every other decoder produces, so
//! a decode that is held to FFmpeg sample for sample must run it.
//!
//! # How strongly, and where
//!
//! Two numbers govern each boundary. The **strength** `bS` says how much filtering the boundary may
//! take; in an all-intra picture it is 4 at a macroblock edge and 3 inside one, which are the two
//! strongest values, because there is no motion to weaken the case. The **thresholds** α and β come
//! from the two macroblocks' quantisation parameters (Table 8-16): the coarser the quantisation, the
//! larger a step the filter is willing to believe is an artefact. Where the step across the boundary
//! is larger than α, or the steps just inside either side are larger than β, nothing is filtered --
//! that is the test that keeps a real edge sharp.
//!
//! The order matters and is not obvious: **every vertical edge of a macroblock, left to right, then
//! every horizontal one, top to bottom**, one macroblock at a time in raster order, each working on
//! samples the macroblocks before it have already filtered. Filtering all the vertical edges of the
//! picture and then all the horizontal ones gives a different answer.

use crate::h264::decode::View;

use oxedyne_fe2o3_core::prelude::*;

/// The first threshold, α′, indexed by `indexA` (Table 8-16).
const ALPHA: [i32; 52] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20, 22, 25, 28,
	32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182,
	203, 226, 255, 255,
];

/// The second threshold, β′, indexed by `indexB` (Table 8-16).
const BETA: [i32; 52] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
	9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16,
	17, 17, 18, 18,
];

/// The clipping limit t′C0, by boundary strength (1, 2 or 3) and `indexA` (Table 8-17).
const TC0: [[i32; 52]; 3] = [
	[
		0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
		0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
		1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8,
		9, 10, 11, 13,
	],
	[
		0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
		0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2,
		2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 8, 10, 11,
		12, 13, 15, 17,
	],
	[
		0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
		0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
		3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16,
		18, 20, 23, 25,
	],
];

/// Which macroblock a luma sample sits in, and where inside it.
fn mb_of(x: usize, y: usize, mbs_w: usize) -> usize {
	(y / 16) * mbs_w + (x / 16)
}

/// Whether the four-by-four block holding a luma sample carries any coefficient.
///
/// With the eight-by-eight transform the question is asked of the eight-by-eight block instead,
/// which is four of these at once.
fn has_coeffs(v: &View, mb: usize, x: usize, y: usize) -> bool {
	let (bx, by) = ((x % 16) / 4, (y % 16) / 4);
	let coded = v.coded[mb];
	if v.big[mb] {
		let base = (by / 2) * 2 + (bx / 2);
		let quad = base * 4;
		(0..4).any(|k| coded & (1 << (quad + k)) != 0)
	} else {
		let quad = (by / 2) * 2 + (bx / 2);
		let within = (by % 2) * 2 + (bx % 2);
		coded & (1 << (quad * 4 + within)) != 0
	}
}

/// Runs the deblocking filter over a whole picture.
///
/// Every macroblock in this decoder is intra, which fixes the boundary strength: 4 at a macroblock
/// edge and 3 within one. The strengths that depend on motion vectors and reference pictures do not
/// arise, and are not implemented rather than being implemented and unreachable.
pub fn deblock(v: &mut View) -> Outcome<()> {
	for mb in 0..v.mbs_w * v.mbs_h {
		if v.slice_of[mb].is_none() {
			continue;
		}
		let (mx, my) = ((mb % v.mbs_w) * 16, (mb / v.mbs_w) * 16);
		let big = v.big[mb];
		// Vertical edges, left to right. The leftmost is the macroblock's own edge; the rest are
		// internal, and the eight-by-eight transform leaves out the ones at four and twelve
		// because there is no transform boundary there.
		let mut xs: Vec<usize> = vec![0];
		if big {
			xs.push(8);
		} else {
			xs.extend([4, 8, 12]);
		}
		for x in xs {
			if x == 0 && mx == 0 {
				continue;
			}
			res!(edge(v, mb, mx, my, x, 0, true));
		}
		// Then the horizontal ones, top to bottom.
		let mut ys: Vec<usize> = vec![0];
		if big {
			ys.push(8);
		} else {
			ys.extend([4, 8, 12]);
		}
		for y in ys {
			if y == 0 && my == 0 {
				continue;
			}
			res!(edge(v, mb, mx, my, 0, y, false));
		}
	}
	Ok(())
}

/// Filters one edge of one macroblock, luma and both colour differences.
fn edge(v: &mut View, mb: usize, mx: usize, my: usize, ex: usize, ey: usize, vertical: bool)
	-> Outcome<()>
{
	let mb_edge = if vertical { ex == 0 } else { ey == 0 };
	// The macroblock on the other side of the edge.
	let other = if !mb_edge {
		mb
	} else if vertical {
		mb - 1
	} else {
		mb - v.mbs_w
	};
	if v.slice_of[other].is_none() {
		return Ok(());
	}
	// Every macroblock here is intra, so a macroblock edge is the strongest the filter has and an
	// internal one the next strongest. There is no case where an intra picture yields 2 or below.
	let bs = if mb_edge { 4 } else { 3 };
	// Only where nothing at all is coded either side does the strength drop, and for an intra
	// picture it does not: the intra condition is tested before the coefficient one.
	let _ = has_coeffs(v, mb, mx, my);

	let qp_p = v.qp[other];
	let qp_q = v.qp[mb];
	// Luma: sixteen sets of samples across the edge.
	for i in 0..16 {
		let (px, py) = if vertical {
			(mx + ex, my + i)
		} else {
			(mx + i, my + ey)
		};
		res!(one(v, px, py, vertical, bs, qp_p, qp_q, false));
	}
	// Chroma: eight sets, at half the resolution, so only the edges at even luma positions have a
	// chroma counterpart -- which for 4:2:0 is every edge this filter visits except the ones at
	// four and twelve, whose chroma position is not on a transform boundary.
	if (vertical && ex % 8 != 0) || (!vertical && ey % 8 != 0) {
		return Ok(());
	}
	for c in 0..2usize {
		let offset = if c == 0 { v.cb_qp_offset } else { v.cr_qp_offset };
		let cp = crate::h264::transform::chroma_qp(qp_p, offset);
		let cq = crate::h264::transform::chroma_qp(qp_q, offset);
		for i in 0..8 {
			let (px, py) = if vertical {
				(mx / 2 + ex / 2, my / 2 + i)
			} else {
				(mx / 2 + i, my / 2 + ey / 2)
			};
			res!(one_chroma(v, c, px, py, vertical, bs, cp, cq));
		}
	}
	Ok(())
}

/// Reads the four samples either side of an edge out of a plane.
fn take(px: &[u8], w: usize, h: usize, x: usize, y: usize, vertical: bool) -> Option<[i32; 8]> {
	let mut out = [0i32; 8];
	for k in 0..4usize {
		// `p` runs away from the edge on the near side, `q` away from it on the far side.
		let (pxx, pyy) = if vertical { (x.checked_sub(k + 1)?, y) } else { (x, y.checked_sub(k + 1)?) };
		let (qxx, qyy) = if vertical { (x + k, y) } else { (x, y + k) };
		if pxx >= w || pyy >= h || qxx >= w || qyy >= h {
			return None;
		}
		out[k] = px[pyy * w + pxx] as i32;
		out[4 + k] = px[qyy * w + qxx] as i32;
	}
	Some(out)
}

/// Writes the three samples either side of an edge back into a plane.
fn give(px: &mut [u8], w: usize, h: usize, x: usize, y: usize, vertical: bool, s: &[i32; 8]) {
	for k in 0..3usize {
		let (pxx, pyy) = if vertical {
			(match x.checked_sub(k + 1) { Some(v) => v, None => continue }, y)
		} else {
			(x, match y.checked_sub(k + 1) { Some(v) => v, None => continue })
		};
		let (qxx, qyy) = if vertical { (x + k, y) } else { (x, y + k) };
		if pxx < w && pyy < h {
			px[pyy * w + pxx] = s[k].clamp(0, 255) as u8;
		}
		if qxx < w && qyy < h {
			px[qyy * w + qxx] = s[4 + k].clamp(0, 255) as u8;
		}
	}
}

/// Filters one set of luma samples across an edge (§8.7.2.3, §8.7.2.4).
#[allow(clippy::too_many_arguments)]
fn one(v: &mut View, x: usize, y: usize, vertical: bool, bs: i32, qp_p: i32, qp_q: i32,
	chroma_style: bool) -> Outcome<()>
{
	let (w, h) = (v.pic.y.w, v.pic.y.h);
	let mut s = match take(&v.pic.y.px, w, h, x, y, vertical) {
		Some(s) => s,
		None => return Ok(()),
	};
	if filter(&mut s, bs, qp_p, qp_q, v.alpha, v.beta, chroma_style) {
		give(&mut v.pic.y.px, w, h, x, y, vertical, &s);
	}
	Ok(())
}

/// The same for one colour difference plane, which is always filtered in the chroma style.
#[allow(clippy::too_many_arguments)]
fn one_chroma(v: &mut View, c: usize, x: usize, y: usize, vertical: bool, bs: i32, qp_p: i32,
	qp_q: i32) -> Outcome<()>
{
	let plane = if c == 0 { &mut v.pic.cb } else { &mut v.pic.cr };
	let (w, h) = (plane.w, plane.h);
	let mut s = match take(&plane.px, w, h, x, y, vertical) {
		Some(s) => s,
		None => return Ok(()),
	};
	if filter(&mut s, bs, qp_p, qp_q, v.alpha, v.beta, true) {
		give(&mut plane.px, w, h, x, y, vertical, &s);
	}
	Ok(())
}

/// The filter itself, over one set of eight samples: `p3..p0` then `q0..q3`.
///
/// Returns whether anything changed, so that a caller need not write back a set the thresholds
/// rejected.
fn filter(s: &mut [i32; 8], bs: i32, qp_p: i32, qp_q: i32, off_a: i32, off_b: i32,
	chroma_style: bool) -> bool
{
	let (p0, p1, p2, p3) = (s[0], s[1], s[2], s[3]);
	let (q0, q1, q2, q3) = (s[4], s[5], s[6], s[7]);
	let qp_av = (qp_p + qp_q + 1) >> 1;
	let index_a = (qp_av + off_a).clamp(0, 51) as usize;
	let index_b = (qp_av + off_b).clamp(0, 51) as usize;
	let alpha = ALPHA[index_a];
	let beta = BETA[index_b];
	// The test that keeps a real edge sharp: a step larger than α across the boundary, or larger
	// than β just inside either side, is a thing that was in the photograph.
	if bs == 0 || (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
		return false;
	}
	let ap = (p2 - p0).abs();
	let aq = (q2 - q0).abs();
	if bs < 4 {
		let tc0 = TC0[(bs - 1) as usize][index_a];
		let tc = if chroma_style {
			tc0 + 1
		} else {
			tc0 + i32::from(ap < beta) + i32::from(aq < beta)
		};
		let delta = ((((q0 - p0) << 2) + (p1 - q1) + 4) >> 3).clamp(-tc, tc);
		s[0] = p0 + delta;
		s[4] = q0 - delta;
		if !chroma_style && ap < beta {
			s[1] = p1 + ((p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1).clamp(-tc0, tc0);
		}
		if !chroma_style && aq < beta {
			s[5] = q1 + ((q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1).clamp(-tc0, tc0);
		}
		return true;
	}
	// The strongest case, at a macroblock edge, which may move three samples either side.
	let close = (p0 - q0).abs() < ((alpha >> 2) + 2);
	if !chroma_style && ap < beta && close {
		s[0] = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
		s[1] = (p2 + p1 + p0 + q0 + 2) >> 2;
		s[2] = (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3;
	} else {
		s[0] = (2 * p1 + p0 + q1 + 2) >> 2;
	}
	if !chroma_style && aq < beta && close {
		s[4] = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
		s[5] = (p0 + q0 + q1 + q2 + 2) >> 2;
		s[6] = (2 * q3 + 3 * q2 + q1 + q0 + p0 + 4) >> 3;
	} else {
		s[4] = (2 * q1 + q0 + p1 + 2) >> 2;
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A set of samples with a step of `step` across the boundary and flat either side.
	fn step(level: i32, step: i32) -> [i32; 8] {
		[level, level, level, level, level + step, level + step, level + step, level + step]
	}

	#[test]
	fn test_a_real_edge_is_left_alone_01() -> Outcome<()> {
		// The whole point of the thresholds. A step larger than α is an edge that was in the
		// photograph, and a filter that smoothed it would blur the picture wherever it is sharp.
		// At a quantisation parameter of 30, α is 25.
		let mut s = step(60, 200);
		let changed = filter(&mut s, 4, 30, 30, 0, 0, false);
		req!(changed, false, "a step of 200 was filtered at an alpha of {}", ALPHA[30]);
		req!(s, step(60, 200), "the samples were altered anyway");
		// And a small step at the same quantisation is filtered.
		let mut s = step(60, 8);
		let changed = filter(&mut s, 4, 30, 30, 0, 0, false);
		req!(changed, true, "a step of 8 was left alone at an alpha of {}", ALPHA[30]);
		let softened = s[0] > 60 && s[4] < 68;
		req!(softened, true, "the step was not softened: {:?}", s);
		Ok(())
	}

	#[test]
	fn test_a_coarse_picture_is_filtered_harder_02() -> Outcome<()> {
		// The thresholds climb with the quantisation parameter, because a coarsely quantised
		// picture has larger coding steps in it and the filter must be willing to believe more of
		// them are artefacts. At a parameter of 20 the same step is left alone that at 40 is
		// filtered, and that difference is the whole of Table 8-16.
		let mut fine = step(100, 20);
		let mut coarse = step(100, 20);
		let fine_changed = filter(&mut fine, 4, 20, 20, 0, 0, false);
		let coarse_changed = filter(&mut coarse, 4, 40, 40, 0, 0, false);
		req!(fine_changed, false, "a step of 20 was filtered at an alpha of {}", ALPHA[20]);
		req!(coarse_changed, true, "a step of 20 was not filtered at an alpha of {}", ALPHA[40]);
		// And below sixteen nothing is filtered at all, whatever the step.
		let mut off = step(100, 1);
		req!(filter(&mut off, 4, 15, 15, 0, 0, false), false,
			"the filter ran at a quantisation parameter where alpha is nought");
		Ok(())
	}

	#[test]
	fn test_the_strongest_filter_moves_three_samples_03() -> Outcome<()> {
		// At a macroblock edge the filter may reach three samples deep either side; inside a
		// macroblock it reaches two. Confusing the two smooths a macroblock's interior more than
		// the specification allows, which shows as a picture that is soft in patches.
		let mut strong = step(100, 6);
		let mut weak = step(100, 6);
		req!(filter(&mut strong, 4, 30, 30, 0, 0, false), true);
		req!(filter(&mut weak, 3, 30, 30, 0, 0, false), true);
		let strong_deep = strong[2] != 100 || strong[6] != 106;
		req!(strong_deep, true, "the macroblock-edge filter left the third sample alone");
		let weak_deep = weak[2] != 100 || weak[6] != 106;
		req!(weak_deep, false, "the internal filter reached the third sample: {:?}", weak);
		Ok(())
	}

	#[test]
	fn test_chroma_is_filtered_in_its_own_style_04() -> Outcome<()> {
		// Chroma never moves more than the sample nearest the edge, whatever the strength. A
		// decoder that filtered chroma as luma would soften colour two samples deep on every
		// block boundary, which is visible as colour bleeding on a hard edge.
		let mut s = step(100, 6);
		req!(filter(&mut s, 4, 30, 30, 0, 0, true), true);
		req!(s[1], 100, "chroma's second sample was moved");
		req!(s[5], 106, "chroma's second sample on the far side was moved");
		let near_moved = s[0] != 100 && s[4] != 106;
		req!(near_moved, true, "chroma's nearest sample was not filtered at all");
		Ok(())
	}

	#[test]
	fn test_the_tables_are_monotone_and_the_right_length_05() -> Outcome<()> {
		// Fifty-two entries each, three of them, and every one climbs. A transcription that
		// dropped or duplicated an entry would shift the tail, and a shifted tail filters every
		// coarsely quantised picture with the wrong threshold. Monotonicity is the cheapest
		// property that a shift breaks.
		for (name, table) in [("alpha", &ALPHA[..]), ("beta", &BETA[..]),
				("tc0 at 1", &TC0[0][..]), ("tc0 at 2", &TC0[1][..]),
				("tc0 at 3", &TC0[2][..])] {
			req!(table.len(), 52, "{} holds the wrong number of entries", name);
			for i in 1..52 {
				let rising = table[i] >= table[i - 1];
				req!(rising, true, "{} falls from {} to {} at {}",
					name, table[i - 1], table[i], i);
			}
			// The first sixteen are nought, which is what turns the filter off at a fine
			// quantisation.
			for i in 0..16 {
				req!(table[i], 0, "{} is not nought at {}", name, i);
			}
		}
		// A stronger boundary never clips less than a weaker one.
		for i in 0..52 {
			let ordered = TC0[2][i] >= TC0[1][i] && TC0[1][i] >= TC0[0][i];
			req!(ordered, true, "the strengths are out of order at indexA {}", i);
		}
		Ok(())
	}
}
