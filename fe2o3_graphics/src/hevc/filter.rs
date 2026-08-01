//! The two passes over a finished picture, in the order they run.
//!
//! Both exist because a picture built block by block carries the seams of how it was built, and
//! both are **in the loop**: the encoder ran them too, so a decoder that skips them is not simply
//! showing a slightly rougher picture, it is showing a different one.
//!
//! **Deblocking** (§8.7.2) softens the boundary between adjacent blocks, and only where a boundary
//! actually is — a transform or prediction edge that falls on the eight-sample grid. How hard it
//! softens is set by the quantisation parameter either side: a coarsely quantised picture has
//! bigger steps to hide. It decides, per four lines of edge, between a strong filter that reaches
//! three samples in and a weak one that reaches one or two, and it declines entirely where the
//! step across the boundary looks like a real edge in the photograph rather than an artefact of
//! coding. That decision is the whole art of it: filtering a real edge is how a decoder turns a
//! window frame into a smear.
//!
//! **The sample adaptive offset** (§8.7.3) then adds a small number to samples, chosen per coding
//! tree block, in one of two ways. A *band* offset moves four adjacent slices of the range, which
//! is how a gently graded sky is put back after quantisation stepped it. An *edge* offset compares
//! each sample with two neighbours along a chosen direction and offsets peaks and valleys
//! differently from slopes, which recovers detail the transform rounded away.
//!
//! Both run over the whole picture rather than block by block. The specification allows either --
//! and says so -- and over a picture is what makes the ordering obvious: every vertical edge, then
//! every horizontal one, then the offsets, each pass reading what the one before it wrote.

use crate::hevc::decode::{
	Picture,
	Plane,
	Sao,
};

/// How far a sample may move, and how flat a boundary has to be before it is filtered at all
/// (§8.7.2.5.3, Table 8-12), indexed by the quantisation parameter.
///
/// `β` is the flatness bar: a boundary whose second differences add up to less than this is taken
/// to be flat, and therefore a place where a step is an artefact rather than a subject. `tC` is how
/// far any one sample may be moved. Both are nought below a parameter of sixteen, which is why a
/// finely quantised picture is not filtered at all.
const BETA: [i32; 52] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24,
	26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56,
	58, 60, 62, 64,
];

/// The companion table, which runs two entries longer because the boundary strength adds to its
/// index.
const TC: [i32; 54] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	1, 1, 1, 1, 1, 1, 1, 1, 1,
	2, 2, 2, 2,
	3, 3, 3, 3,
	4, 4, 4,
	5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24,
];

/// Where a boundary sits, and what a filter reads across it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
	/// A boundary between a block and the one to its left.
	Vertical,
	/// Between a block and the one above it.
	Horizontal,
}

/// What the deblocking filter needs to know about a picture beyond its samples.
pub struct Edges<'a> {
	/// The picture's width in four-sample blocks.
	pub gw:	usize,
	/// And its height.
	pub gh:	usize,
	/// Whether each four-sample block has a transform or prediction boundary on its left.
	pub vertical:	&'a [bool],
	/// The same for its top.
	pub horizontal:	&'a [bool],
	/// The luma quantisation parameter of the coding unit covering each.
	pub qp:	&'a [i8],
	/// What the picture adds to the chroma parameter, per component.
	pub chroma_offset:	[i32; 2],
}

/// Runs the deblocking filter over a whole picture (§8.7.2).
///
/// Every vertical edge first and then every horizontal one, which is the order the specification
/// gives and the reason it gives it: the horizontal pass reads samples the vertical pass has
/// already moved.
///
/// Every coding unit in a still picture is intra, so every boundary that is filtered at all is
/// filtered at the strongest setting. That is what makes this shorter than a decoder that has to
/// weigh motion vectors and reference indices to decide.
pub fn deblock(pic: &mut Picture, edges: &Edges<'_>, depth: u32) {
	for kind in [Edge::Vertical, Edge::Horizontal] {
		luma(pic, edges, kind, depth);
		chroma(pic, edges, kind, depth);
	}
}

/// Whether a boundary of the given kind sits at a luma position.
fn boundary(edges: &Edges<'_>, kind: Edge, x: usize, y: usize) -> bool {
	let (gx, gy) = (x / 4, y / 4);
	if gx >= edges.gw || gy >= edges.gh {
		return false;
	}
	let at = gy * edges.gw + gx;
	match kind {
		Edge::Vertical		=> edges.vertical[at],
		Edge::Horizontal	=> edges.horizontal[at],
	}
}

/// The quantisation parameter either side of a boundary, averaged as the filter wants it.
fn across(edges: &Edges<'_>, kind: Edge, x: usize, y: usize) -> i32 {
	let (gx, gy) = (x / 4, y / 4);
	let (bx, by) = match kind {
		Edge::Vertical		=> (gx.saturating_sub(1), gy),
		Edge::Horizontal	=> (gx, gy.saturating_sub(1)),
	};
	let q = edges.qp[(gy.min(edges.gh - 1)) * edges.gw + gx.min(edges.gw - 1)] as i32;
	let p = edges.qp[(by.min(edges.gh - 1)) * edges.gw + bx.min(edges.gw - 1)] as i32;
	(q + p + 1) >> 1
}

/// One sample either side of a boundary, by how far from it and along it.
///
/// `i` counts away from the boundary -- negative into the block before it, nought and up into the
/// block after -- and `k` counts along it. Writing the two directions this way is what lets one
/// piece of arithmetic serve a vertical edge and a horizontal one.
fn at(plane: &Plane, kind: Edge, x: usize, y: usize, i: i32, k: usize) -> i32 {
	let (sx, sy) = match kind {
		Edge::Vertical		=> (x as i32 + i, (y + k) as i32),
		Edge::Horizontal	=> ((x + k) as i32, y as i32 + i),
	};
	if sx < 0 || sy < 0 {
		return 0;
	}
	plane.at(sx as usize, sy as usize).unwrap_or(0) as i32
}

/// Writes one sample either side of a boundary, in the same coordinates.
fn put(plane: &mut Plane, kind: Edge, x: usize, y: usize, i: i32, k: usize, v: i32, top: i32) {
	let (sx, sy) = match kind {
		Edge::Vertical		=> (x as i32 + i, (y + k) as i32),
		Edge::Horizontal	=> ((x + k) as i32, y as i32 + i),
	};
	if sx < 0 || sy < 0 {
		return;
	}
	let w = plane.w;
	let h = plane.h;
	let (sx, sy) = (sx as usize, sy as usize);
	if sx < w && sy < h {
		plane.px[sy * w + sx] = v.clamp(0, top) as u16;
	}
}

/// The brightness plane, one four-line segment of edge at a time.
fn luma(pic: &mut Picture, edges: &Edges<'_>, kind: Edge, depth: u32) {
	let (w, h) = (pic.y.w, pic.y.h);
	let top = (1i32 << depth) - 1;
	let scale = 1i32 << (depth - 8);
	// Along the edge in fours, across it in eights: only every eighth line of samples carries a
	// boundary that is filtered.
	let (xs, ys): (usize, usize) = match kind {
		Edge::Vertical		=> (8, 4),
		Edge::Horizontal	=> (4, 8),
	};
	let mut y = 0usize;
	while y < h {
		let mut x = 0usize;
		while x < w {
			let first = match kind {
				Edge::Vertical		=> x == 0,
				Edge::Horizontal	=> y == 0,
			};
			if first || !boundary(edges, kind, x, y) {
				x += xs;
				continue;
			}
			let qp = across(edges, kind, x, y);
			let beta = BETA[qp.clamp(0, 51) as usize] * scale;
			// The boundary strength is two everywhere in a still picture, which adds two to the
			// index of the other table.
			let tc = TC[(qp + 2).clamp(0, 53) as usize] * scale;
			if beta == 0 {
				x += xs;
				continue;
			}
			// Every sample the segment needs, read before any of them is written -- which is how
			// the specification states it, and it matters: the strong filter's new p2 must not
			// feed the same line's new p0.
			let mut ps = [[0i32; 4]; 4];
			let mut qs = [[0i32; 4]; 4];
			for k in 0..4 {
				for i in 0..4 {
					ps[k][i] = at(&pic.y, kind, x, y, -(i as i32) - 1, k);
					qs[k][i] = at(&pic.y, kind, x, y, i as i32, k);
				}
			}
			// The two outer lines of the four decide for all of them, which is what keeps the
			// filter from following a diagonal edge line by line.
			let p = |i: usize, k: usize| ps[k][i];
			let q = |i: usize, k: usize| qs[k][i];
			let second = |f: &dyn Fn(usize, usize) -> i32, k: usize| {
				(f(2, k) - 2 * f(1, k) + f(0, k)).abs()
			};
			let (dp0, dp3) = (second(&p, 0), second(&p, 3));
			let (dq0, dq3) = (second(&q, 0), second(&q, 3));
			let (dpq0, dpq3) = (dp0 + dq0, dp3 + dq3);
			let (dp, dq) = (dp0 + dp3, dq0 + dq3);
			if dpq0 + dpq3 >= beta {
				// The step across this boundary is too big to be an artefact of coding: it is
				// something in the photograph, and smearing it is the one thing this must not do.
				x += xs;
				continue;
			}
			let flat = |k: usize, dpq: i32| {
				dpq < (beta >> 2)
					&& ((p(3, k) - p(0, k)).abs() + (q(0, k) - q(3, k)).abs()) < (beta >> 3)
					&& (p(0, k) - q(0, k)).abs() < ((5 * tc + 1) >> 1)
			};
			let strong = flat(0, 2 * dpq0) && flat(3, 2 * dpq3);
			let thin = (beta + (beta >> 1)) >> 3;
			let (near_p, near_q) = (dp < thin, dq < thin);

			for k in 0..4 {
				let (p0, p1, p2, p3) = (p(0, k), p(1, k), p(2, k), p(3, k));
				let (q0, q1, q2, q3) = (q(0, k), q(1, k), q(2, k), q(3, k));
				if strong {
					let clip = |v: i32, was: i32| v.clamp(was - 2 * tc, was + 2 * tc);
					let _ = (p3, q3);
					let np0 = clip((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3, p0);
					let np1 = clip((p2 + p1 + p0 + q0 + 2) >> 2, p1);
					let np2 = clip((2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3, p2);
					let nq0 = clip((p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3, q0);
					let nq1 = clip((p0 + q0 + q1 + q2 + 2) >> 2, q1);
					let nq2 = clip((p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3, q2);
					for (i, v) in [(0i32, np0), (1, np1), (2, np2)] {
						put(&mut pic.y, kind, x, y, -i - 1, k, v, top);
					}
					for (i, v) in [(0i32, nq0), (1, nq1), (2, nq2)] {
						put(&mut pic.y, kind, x, y, i, k, v, top);
					}
				} else {
					let mut delta = (9 * (q0 - p0) - 3 * (q1 - p1) + 8) >> 4;
					if delta.abs() >= tc * 10 {
						continue;
					}
					delta = delta.clamp(-tc, tc);
					put(&mut pic.y, kind, x, y, -1, k, p0 + delta, top);
					put(&mut pic.y, kind, x, y, 0, k, q0 - delta, top);
					if near_p {
						let half = tc >> 1;
						let d = (((p2 + p0 + 1) >> 1) - p1 + delta) >> 1;
						put(&mut pic.y, kind, x, y, -2, k, p1 + d.clamp(-half, half), top);
					}
					if near_q {
						let half = tc >> 1;
						let d = (((q2 + q0 + 1) >> 1) - q1 - delta) >> 1;
						put(&mut pic.y, kind, x, y, 1, k, q1 + d.clamp(-half, half), top);
					}
				}
			}
			x += xs;
		}
		y += ys;
	}
}

/// The two colour planes, which take a simpler filter and only every second boundary.
///
/// Colour is coded at half resolution in both directions, so its own eight-sample grid falls on
/// every sixteenth luma sample; and because a boundary in a still picture is always at full
/// strength, no other test decides whether to filter.
fn chroma(pic: &mut Picture, edges: &Edges<'_>, kind: Edge, depth: u32) {
	let top = (1i32 << depth) - 1;
	let scale = 1i32 << (depth - 8);
	let (xs, ys): (usize, usize) = match kind {
		Edge::Vertical		=> (16, 8),
		Edge::Horizontal	=> (8, 16),
	};
	let (w, h) = (pic.y.w, pic.y.h);
	for c in 0..2usize {
		let mut y = 0usize;
		while y < h {
			let mut x = 0usize;
			while x < w {
				let first = match kind {
					Edge::Vertical		=> x == 0,
					Edge::Horizontal	=> y == 0,
				};
				if first || !boundary(edges, kind, x, y) {
					x += xs;
					continue;
				}
				let qpi = across(edges, kind, x, y) + edges.chroma_offset[c];
				let tc = TC[(crate::hevc::decode::chroma_qp(qpi.clamp(0, 57)) + 2)
					.clamp(0, 53) as usize] * scale;
				if tc == 0 {
					x += xs;
					continue;
				}
				let plane = if c == 0 { &mut pic.cb } else { &mut pic.cr };
				let (cx, cy) = (x / 2, y / 2);
				for k in 0..4 {
					let p0 = at(plane, kind, cx, cy, -1, k);
					let p1 = at(plane, kind, cx, cy, -2, k);
					let q0 = at(plane, kind, cx, cy, 0, k);
					let q1 = at(plane, kind, cx, cy, 1, k);
					let d = ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3).clamp(-tc, tc);
					put(plane, kind, cx, cy, -1, k, p0 + d, top);
					put(plane, kind, cx, cy, 0, k, q0 - d, top);
				}
				x += xs;
			}
			y += ys;
		}
	}
}

/// Which two neighbours an edge offset compares a sample with (§8.7.3.2, Table 8-13).
///
/// Across, down, and the two diagonals.
const NEIGHBOURS: [[(i32, i32); 2]; 4] = [
	[(-1, 0), (1, 0)],
	[(0, -1), (0, 1)],
	[(-1, -1), (1, 1)],
	[(1, -1), (-1, 1)],
];

/// Runs the sample adaptive offset over a whole picture (§8.7.3).
///
/// **It reads the picture as the deblocking filter left it and writes somewhere else.** A sample
/// that has already been offset must not be what its neighbour is compared against, or the offsets
/// walk across the picture; the copy is what stops that, and it is why this cannot be done in
/// place however tempting the memory saving.
pub fn sao(pic: &mut Picture, per_ctb: &[Sao], ctbs_w: usize, ctb: usize, depth: u32) {
	let source = pic.clone();
	let top = (1i32 << depth) - 1;
	for c in 0..3usize {
		let (src, dst) = match c {
			0	=> (&source.y, &mut pic.y),
			1	=> (&source.cb, &mut pic.cb),
			_	=> (&source.cr, &mut pic.cr),
		};
		// Colour is half size both ways, so its blocks are half as wide and half as tall.
		let side = if c == 0 { ctb } else { ctb / 2 };
		if side == 0 {
			continue;
		}
		for (i, sao) in per_ctb.iter().enumerate() {
			if sao.kind[c] == 0 {
				continue;
			}
			let (rx, ry) = (i % ctbs_w, i / ctbs_w);
			let (x0, y0) = (rx * side, ry * side);
			for y in y0..(y0 + side).min(src.h) {
				for x in x0..(x0 + side).min(src.w) {
					let here = match src.at(x, y) {
						Some(v) => v as i32,
						None => continue,
					};
					let offset = if sao.kind[c] == 2 {
						let pair = NEIGHBOURS[(sao.class[c] as usize).min(3)];
						let mut idx = 2i32;
						let mut outside = false;
						for (dx, dy) in pair {
							let (nx, ny) = (x as i32 + dx, y as i32 + dy);
							if nx < 0 || ny < 0
								|| nx as usize >= src.w || ny as usize >= src.h
							{
								outside = true;
								break;
							}
							let other = src.at(nx as usize, ny as usize).unwrap_or(0) as i32;
							idx += (here - other).signum();
						}
						if outside {
							continue;
						}
						// Two of the five categories are relabelled so that "the same as both
						// neighbours" is the one with no offset.
						let category = if idx <= 2 {
							if idx == 2 { 0 } else { idx + 1 }
						} else {
							idx
						};
						if category == 0 {
							continue;
						}
						sao.offset[c][(category - 1) as usize]
					} else {
						// A band offset moves four adjacent thirty-seconds of the range and
						// nothing else.
						let band = (here >> (depth - 5)) as usize & 31;
						let start = sao.band[c] as usize;
						let which = (band + 32 - start) & 31;
						if which >= 4 {
							continue;
						}
						sao.offset[c][which]
					};
					if offset == 0 {
						continue;
					}
					let w = dst.w;
					if x < w && y < dst.h {
						dst.px[y * w + x] = (here + offset).clamp(0, top) as u16;
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_core::prelude::*;

	/// A picture of one level throughout, at the given size.
	fn flat(w: usize, h: usize, level: u16) -> Picture {
		let plane = |w: usize, h: usize| Plane { w, h, px: vec![level; w * h] };
		Picture { y: plane(w, h), cb: plane(w / 2, h / 2), cr: plane(w / 2, h / 2), depth: 8 }
	}

	/// Edges everywhere, at one quantisation parameter.
	fn all_edges(w: usize, h: usize, qp: i8) -> (Vec<bool>, Vec<bool>, Vec<i8>) {
		let (gw, gh) = (w / 4, h / 4);
		(vec![true; gw * gh], vec![true; gw * gh], vec![qp; gw * gh])
	}

	#[test]
	fn test_the_thresholds_are_the_published_ones_06() -> Outcome<()> {
		// A hundred and six numbers out of a document, and one of them wrong by one place moves
		// every filtering decision above that quantisation parameter. This was written with the
		// second table shifted from index twenty-six on, and the fault it produced was a filter
		// that smeared a real edge -- which the test above caught only because that edge was put
		// there deliberately.
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
		// The table is printed as three bands of Q, each with a row of β′ under it and a row of
		// tC′ under that, so the numbers of each arrive in order across the bands.
		let (mut beta, mut tc): (Vec<i32>, Vec<i32>) = (Vec::new(), Vec::new());
		let mut found = false;
		for line in text.lines() {
			let t = line.trim();
			if t.starts_with("Table 8-12") && t.contains("threshold") {
				found = true;
				continue;
			}
			if !found {
				continue;
			}
			if t.starts_with("8.7.2.5.4") {
				break;
			}
			// A row of the table starts with its name; a dash stands for an entry that does not
			// exist, and there are two of those at the end of the first row.
			let into = if t.starts_with("β′") {
				&mut beta
			} else if t.starts_with("t C′") || t.starts_with("tC′") {
				&mut tc
			} else {
				continue;
			};
			for word in t.split_whitespace().skip(1) {
				if word == "-" || word == "\u{2212}" {
					continue;
				}
				if let Ok(n) = word.parse::<i32>() {
					into.push(n);
				}
			}
		}
		if beta.is_empty() || tc.is_empty() {
			println!("  skipped: Table 8-12 is not in {} in a shape this can read", path);
			return Ok(());
		}
		req!(beta.len(), BETA.len(), "the document lists {} values of the flatness bar", beta.len());
		req!(tc.len(), TC.len(), "the document lists {} values of the movement bound", tc.len());
		for (q, want) in beta.iter().enumerate() {
			req!(BETA[q], *want, "the flatness bar at a parameter of {}", q);
		}
		for (q, want) in tc.iter().enumerate() {
			req!(TC[q], *want, "the movement bound at a parameter of {}", q);
		}
		Ok(())
	}

	#[test]
	fn test_a_flat_picture_is_left_alone_00() -> Outcome<()> {
		// The property that catches a filter applied where no step exists: there is nothing to
		// soften in a picture of one level, so every sample must come back as it went in. A sign
		// error in the weak filter's delta, or a clip written the wrong way round, shows here.
		let mut pic = flat(32, 32, 128);
		let (v, hz, qp) = all_edges(32, 32, 40);
		let edges = Edges {
			gw: 8, gh: 8, vertical: &v, horizontal: &hz, qp: &qp, chroma_offset: [0, 0],
		};
		deblock(&mut pic, &edges, 8);
		for (i, s) in pic.y.px.iter().enumerate() {
			req!(*s, 128u16, "sample {} of a flat picture moved", i);
		}
		Ok(())
	}

	#[test]
	fn test_a_real_edge_survives_and_a_coding_step_does_not_01() -> Outcome<()> {
		// The distinction the whole filter exists to make. A boundary with a big step across it is
		// something in the photograph and must be left; one with a small step is an artefact of
		// coding and must be softened. Both are put at the same place with the same settings, so
		// nothing but the size of the step can account for the difference.
		let mut real = flat(32, 32, 60);
		for y in 0..32 {
			for x in 16..32 {
				real.y.px[y * 32 + x] = 200;
			}
		}
		let mut coded = flat(32, 32, 60);
		for y in 0..32 {
			for x in 16..32 {
				coded.y.px[y * 32 + x] = 64;
			}
		}
		let (v, hz, qp) = all_edges(32, 32, 37);
		let edges = Edges {
			gw: 8, gh: 8, vertical: &v, horizontal: &hz, qp: &qp, chroma_offset: [0, 0],
		};
		let (was_real, was_coded) = (real.y.px.clone(), coded.y.px.clone());
		deblock(&mut real, &edges, 8);
		deblock(&mut coded, &edges, 8);

		let kept = real.y.px[16 * 32 + 15] == was_real[16 * 32 + 15]
			&& real.y.px[16 * 32 + 16] == was_real[16 * 32 + 16];
		req!(kept, true, "a step of 140 levels was filtered, which smears a real edge");
		let softened = coded.y.px[16 * 32 + 15] != was_coded[16 * 32 + 15];
		req!(softened, true, "a step of four levels was left, which is a visible block edge");
		Ok(())
	}

	#[test]
	fn test_nothing_is_filtered_where_there_is_no_boundary_02() -> Outcome<()> {
		// Only a transform or prediction edge is a boundary. A filter that ran on the eight-sample
		// grid regardless would soften the inside of every large block, which is a picture that has
		// been quietly blurred.
		let mut pic = flat(32, 32, 60);
		for y in 0..32 {
			for x in 16..32 {
				pic.y.px[y * 32 + x] = 64;
			}
		}
		let (gw, gh) = (8usize, 8usize);
		let (v, hz, qp) = (vec![false; gw * gh], vec![false; gw * gh], vec![37i8; gw * gh]);
		let edges = Edges {
			gw, gh, vertical: &v, horizontal: &hz, qp: &qp, chroma_offset: [0, 0],
		};
		let was = pic.y.px.clone();
		deblock(&mut pic, &edges, 8);
		req!(pic.y.px, was, "a picture with no block boundaries in it was filtered anyway");
		Ok(())
	}

	#[test]
	fn test_a_finely_quantised_picture_is_not_filtered_03() -> Outcome<()> {
		// Below a parameter of sixteen both thresholds are nought, which is the specification
		// saying that a picture coded this finely has no steps worth hiding.
		let mut pic = flat(32, 32, 60);
		for y in 0..32 {
			for x in 16..32 {
				pic.y.px[y * 32 + x] = 64;
			}
		}
		let (v, hz, qp) = all_edges(32, 32, 10);
		let edges = Edges {
			gw: 8, gh: 8, vertical: &v, horizontal: &hz, qp: &qp, chroma_offset: [0, 0],
		};
		let was = pic.y.px.clone();
		deblock(&mut pic, &edges, 8);
		req!(pic.y.px, was, "a picture at a quantisation parameter of ten was filtered");
		Ok(())
	}

	#[test]
	fn test_a_band_offset_moves_its_four_bands_and_no_others_04() -> Outcome<()> {
		// Four adjacent thirty-seconds of the range move and the rest do not, which is what makes
		// this a correction to a graded sky rather than a change of exposure.
		let mut pic = flat(8, 8, 0);
		for i in 0..64 {
			// One sample in each of the first eight bands.
			pic.y.px[i] = (i as u16 % 32) * 8;
		}
		let was = pic.y.px.clone();
		let mut sao = Sao::default();
		sao.kind[0] = 1;
		sao.band[0] = 3;
		sao.offset[0] = [5, 5, 5, 5];
		super::sao(&mut pic, &[sao], 1, 8, 8);
		for (i, (before, after)) in was.iter().zip(pic.y.px.iter()).enumerate() {
			let band = (*before >> 3) & 31;
			let moved = *after != *before;
			let wanted = (3..7).contains(&band);
			req!(moved, wanted, "sample {} in band {} moved {}", i, band, moved);
		}
		Ok(())
	}

	#[test]
	fn test_an_edge_offset_leaves_a_slope_alone_05() -> Outcome<()> {
		// The five categories an edge offset sorts samples into: a peak, a valley, the two sides of
		// a step, and everything else. Everything else gets no offset at all, which is most of a
		// photograph -- so a ramp must come back untouched while a single spike in it does not.
		let mut pic = flat(8, 8, 0);
		for y in 0..8 {
			for x in 0..8 {
				pic.y.px[y * 8 + x] = 40 + x as u16 * 5;
			}
		}
		// One sample raised into a peak.
		pic.y.px[3 * 8 + 4] = 200;
		let was = pic.y.px.clone();
		let mut sao = Sao::default();
		sao.kind[0] = 2;
		sao.class[0] = 0;
		sao.offset[0] = [7, 3, -3, -7];
		super::sao(&mut pic, &[sao], 1, 8, 8);
		// The slope, away from the edges of the picture where the filter declines.
		req!(pic.y.px[2 * 8 + 3], was[2 * 8 + 3], "a sample on a slope was offset");
		let peak_moved = pic.y.px[3 * 8 + 4] != was[3 * 8 + 4];
		req!(peak_moved, true, "a peak was not offset");
		Ok(())
	}
}
