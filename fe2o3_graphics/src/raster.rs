//! The rasteriser: polygons in, per-pixel coverage out.
//!
//! # How it works
//!
//! The usual way to anti-alias is to sample a shape many times per pixel and count the hits, which
//! costs as many passes as samples and still only estimates the answer. This rasteriser computes
//! the answer instead.
//!
//! Every edge of the polygon is walked one scanline at a time. For each pixel the edge passes
//! through, the *signed area* the edge contributes is accumulated: positive where the edge runs
//! down the screen, negative where it runs up. Once every edge has been walked, a running sum
//! along each row turns those local contributions into the winding number at each pixel, weighted
//! by how much of the pixel the shape actually covers. A pixel wholly inside the shape sums to one;
//! a pixel the edge cuts in half sums to a half; a pixel outside sums to zero.
//!
//! # Fill rules
//!
//! What that running sum holds is easy to misread. It is not a winding number: it is the winding
//! number *averaged over the pixel's area*, so a pixel whose left half lies in a region wound once
//! and whose right half lies in a region wound twice sums to one and a half. A fill rule therefore
//! cannot be read off the sum by testing it -- ask "is this odd?" of one and a half and there is no
//! answer. The rule has to be extended from the integers, where it is defined, out to the reals,
//! where the sum lives, by a map that runs straight between them, so that a half-covered pixel
//! comes out half covered.
//!
//! The absolute value of the sum, clamped to one, is that extension for the **non-zero winding
//! rule**, which is what glyph outlines are drawn for: an inner contour wound the other way
//! subtracts, and a counter comes out hollow. A triangle wave of period two is the extension for
//! the **even-odd rule**: nothing at even windings, everything at odd ones, and a ramp between, so
//! that where two contours overlap a hole opens with a soft edge rather than a jagged one. See
//! [`FillRule`].
//!
//! # Why the buffer is wider than the window
//!
//! Geometry off the left of the window is clamped to the left edge, where its winding still counts:
//! a shape running off the left of the screen still fills the pixels that remain. Geometry off the
//! right is clamped into two slack columns past the right edge, where it lands harmlessly, because
//! a running sum that moves left to right can never be reached by anything to its right.

use crate::path::Pt;

/// Which points a path encloses, where its contours cross or overlap.
///
/// The two rules differ only where a point is wound more than once. A glyph, whose counters are
/// wound against their outer contour, wants [`FillRule::NonZero`]; a self-intersecting star or a
/// pair of overlapping rings, where the crossing is meant to read as a hole, wants
/// [`FillRule::EvenOdd`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillRule {
	/// A point is inside when the winding number is not zero. The default, and what an outline
	/// font means.
	#[default]
	NonZero,
	/// A point is inside when the winding number is odd, so a second layer of winding takes the
	/// paint back off.
	EvenOdd,
}

impl FillRule {

	/// Turns an accumulated winding, weighted by coverage, into coverage from 0 to 1.
	///
	/// See the module docs: the argument is an average of winding numbers over a pixel, not a
	/// winding number, so each rule is applied as the straight-line extension of itself off the
	/// integers. Non-zero saturates, even-odd folds back.
	pub fn coverage(&self, acc: f32) -> f32 {
		match self {
			Self::NonZero	=> acc.abs().min(1.0),
			Self::EvenOdd	=> {
				// One period of the wave, from 0 up to 2, direction thrown away as the rule is
				// blind to it.
				let w = (acc % 2.0).abs();
				if w > 1.0 { 2.0 - w } else { w }
			},
		}
	}
}

/// Accumulates the signed area of a set of edges over a rectangular window of pixels.
#[derive(Clone, Debug)]
pub struct Raster {
	/// Window width, in pixels.
	w:	usize,
	/// Window height, in pixels.
	h:	usize,
	/// The accumulation buffer, `(w + 2) * h`. See the module docs for the two slack columns.
	a:	Vec<f32>,
}

impl Raster {

	/// Creates a rasteriser over a window of the given size, in pixels.
	pub fn new(w: usize, h: usize) -> Self {
		Self {
			w,
			h,
			a: vec![0.0; (w + 2) * h],
		}
	}

	/// The window width, in pixels.
	pub fn width(&self) -> usize {
		self.w
	}

	/// The window height, in pixels.
	pub fn height(&self) -> usize {
		self.h
	}

	/// Adds a closed contour, whose points are in window coordinates.
	///
	/// The contour is closed whether or not its last point repeats its first, since only a closed
	/// contour has an interior.
	pub fn add_contour(&mut self, pts: &[Pt]) {
		if pts.len() < 2 {
			return;
		}
		for i in 0..pts.len() {
			let p0 = pts[i];
			let p1 = pts[(i + 1) % pts.len()];
			self.add_edge(p0, p1);
		}
	}

	/// Adds one edge, accumulating the signed area it contributes to each pixel it crosses.
	pub fn add_edge(&mut self, p0: Pt, p1: Pt) {
		if !p0.is_finite() || !p1.is_finite() {
			return;
		}
		if (p0.y - p1.y).abs() <= f32::EPSILON {
			return; // A horizontal edge sweeps no area.
		}
		// Walk downwards, remembering which way the edge really ran.
		let (dir, top, bot) = if p0.y < p1.y {
			(1.0f32, p0, p1)
		} else {
			(-1.0f32, p1, p0)
		};
		let hf = self.h as f32;
		if bot.y <= 0.0 || top.y >= hf {
			return; // Wholly above or below the window.
		}
		let dxdy = (bot.x - top.x) / (bot.y - top.y);
		let x_at = |y: f32| -> f32 { top.x + (y - top.y) * dxdy };

		let y_start = top.y.max(0.0);
		let y_end = bot.y.min(hf);
		let y0 = y_start.floor() as usize;
		let y1 = (y_end.ceil() as usize).min(self.h);
		let stride = self.w + 2;
		// Clamping to the window width, not past it, keeps every index this method writes inside
		// the two slack columns the buffer carries.
		let xmax = self.w as f32;

		for y in y0..y1 {
			let ytop = (y as f32).max(y_start);
			let ybot = ((y + 1) as f32).min(y_end);
			let dy = ybot - ytop;
			if dy <= 0.0 {
				continue;
			}
			let d = dy * dir;
			let xa = x_at(ytop).clamp(0.0, xmax);
			let xb = x_at(ybot).clamp(0.0, xmax);
			let (x0, x1) = if xa < xb { (xa, xb) } else { (xb, xa) };
			let row = y * stride;

			let x0floor = x0.floor();
			let x0i = x0floor as usize;
			let x1ceil = x1.ceil();
			let x1i = x1ceil as usize;

			if x1i <= x0i + 1 {
				// The edge crosses this scanline within a single column, so the area splits between
				// that column and the next by where the edge's midpoint sits.
				let xmf = 0.5 * (x0 + x1) - x0floor;
				self.a[row + x0i] += d * (1.0 - xmf);
				self.a[row + x0i + 1] += d * xmf;
			} else {
				// The edge spans several columns: a wedge at each end, and a uniform slope between.
				let s = (x1 - x0).recip();
				let x0f = x0 - x0floor;
				let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
				let x1f = x1 - x1ceil + 1.0;
				let am = 0.5 * s * x1f * x1f;
				self.a[row + x0i] += d * a0;
				if x1i == x0i + 2 {
					self.a[row + x0i + 1] += d * (1.0 - a0 - am);
				} else {
					let a1 = s * (1.5 - x0f);
					self.a[row + x0i + 1] += d * (a1 - a0);
					for xi in (x0i + 2)..(x1i - 1) {
						self.a[row + xi] += d * s;
					}
					let a2 = a1 + ((x1i - x0i - 3) as f32) * s;
					self.a[row + x1i - 1] += d * (1.0 - a2 - am);
				}
				self.a[row + x1i] += d * am;
			}
		}
	}

	/// Resolves the accumulated areas into per-pixel coverage under the non-zero winding rule, from
	/// 0 to 1, row-major, `w * h`.
	pub fn coverage(&self) -> Vec<f32> {
		self.coverage_with(FillRule::NonZero)
	}

	/// Resolves the accumulated areas into per-pixel coverage under a fill rule, from 0 to 1,
	/// row-major, `w * h`.
	///
	/// The running sum restarts on every row. A closed contour makes each row sum back to zero, so
	/// restarting costs nothing and stops any drift from crossing into the row below.
	pub fn coverage_with(&self, rule: FillRule) -> Vec<f32> {
		let stride = self.w + 2;
		let mut out = vec![0.0f32; self.w * self.h];
		for y in 0..self.h {
			let row = y * stride;
			let orow = y * self.w;
			let mut acc = 0.0f32;
			for x in 0..self.w {
				acc += self.a[row + x];
				out[orow + x] = rule.coverage(acc);
			}
		}
		out
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A square covering exactly the middle four pixels of an 8x8 window.
	fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Pt> {
		vec![
			Pt::new(x0, y0),
			Pt::new(x1, y0),
			Pt::new(x1, y1),
			Pt::new(x0, y1),
		]
	}

	#[test]
	fn test_whole_pixels_are_fully_covered_00() {
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(2.0, 2.0, 6.0, 6.0));
		let cov = r.coverage();
		// Inside.
		assert!((cov[3 * 8 + 3] - 1.0).abs() < 1e-4, "found {}", cov[3 * 8 + 3]);
		// Outside.
		assert!(cov[0] < 1e-4, "found {}", cov[0]);
		assert!(cov[7 * 8 + 7] < 1e-4);
	}

	#[test]
	fn test_half_covered_pixel_is_half_01() {
		// A square covering the left half of every pixel in column 0.
		let mut r = Raster::new(4, 4);
		r.add_contour(&square(0.0, 0.0, 0.5, 4.0));
		let cov = r.coverage();
		for y in 0..4 {
			assert!(
				(cov[y * 4] - 0.5).abs() < 1e-3,
				"row {} should be half covered, found {}", y, cov[y * 4],
			);
		}
	}

	#[test]
	fn test_winding_is_direction_blind_02() {
		// The same square wound the other way covers the same pixels.
		let mut cw = Raster::new(8, 8);
		cw.add_contour(&square(2.0, 2.0, 6.0, 6.0));
		let mut ccw = Raster::new(8, 8);
		let mut pts = square(2.0, 2.0, 6.0, 6.0);
		pts.reverse();
		ccw.add_contour(&pts);
		let (a, b) = (cw.coverage(), ccw.coverage());
		for i in 0..a.len() {
			assert!((a[i] - b[i]).abs() < 1e-4, "pixel {} differs: {} then {}", i, a[i], b[i]);
		}
	}

	#[test]
	fn test_reversed_inner_contour_cuts_a_hole_03() {
		// The non-zero rule: an inner contour wound the other way subtracts.
		let mut r = Raster::new(10, 10);
		r.add_contour(&square(1.0, 1.0, 9.0, 9.0));
		let mut hole = square(3.0, 3.0, 7.0, 7.0);
		hole.reverse();
		r.add_contour(&hole);
		let cov = r.coverage();
		assert!((cov[2 * 10 + 2] - 1.0).abs() < 1e-4, "the ring should be solid");
		assert!(cov[5 * 10 + 5] < 1e-4, "the counter should be hollow, found {}", cov[5 * 10 + 5]);
	}

	#[test]
	fn test_same_wound_overlap_does_not_exceed_one_04() {
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(1.0, 1.0, 7.0, 7.0));
		r.add_contour(&square(2.0, 2.0, 6.0, 6.0));
		let cov = r.coverage();
		for (i, c) in cov.iter().enumerate() {
			assert!(*c <= 1.0 + 1e-6, "pixel {} exceeds full coverage at {}", i, c);
		}
		assert!((cov[4 * 8 + 4] - 1.0).abs() < 1e-4);
	}

	#[test]
	fn test_geometry_off_the_window_is_clamped_not_crashed_05() {
		// A shape running far off every edge must fill the window and index nothing out of range.
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(-1000.0, -1000.0, 1000.0, 1000.0));
		let cov = r.coverage();
		for (i, c) in cov.iter().enumerate() {
			assert!((c - 1.0).abs() < 1e-3, "pixel {} should be filled, found {}", i, c);
		}
	}

	#[test]
	fn test_shape_beyond_the_right_edge_paints_nothing_06() {
		// Entirely off to the right: the slack columns swallow it.
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(20.0, 0.0, 30.0, 8.0));
		let cov = r.coverage();
		for (i, c) in cov.iter().enumerate() {
			assert!(*c < 1e-4, "pixel {} should be empty, found {}", i, c);
		}
	}

	#[test]
	fn test_a_triangle_is_antialiased_07() {
		// The diagonal must produce partial coverage somewhere, or there is no anti-aliasing.
		let mut r = Raster::new(16, 16);
		r.add_contour(&[Pt::new(0.0, 0.0), Pt::new(16.0, 0.0), Pt::new(0.0, 16.0)]);
		let cov = r.coverage();
		let partial = cov.iter().filter(|c| **c > 0.05 && **c < 0.95).count();
		assert!(partial > 8, "expected a soft diagonal, found {} partial pixels", partial);
	}

	/// A five-pointed star drawn as one self-crossing contour, whose middle is wound twice.
	fn star(cx: f32, cy: f32, r: f32) -> Vec<Pt> {
		let mut pts = Vec::with_capacity(5);
		for k in 0..5 {
			// Every second vertex of a pentagon, so the contour crosses itself.
			let a = -std::f32::consts::FRAC_PI_2
				+ (k as f32) * 2.0 * std::f32::consts::TAU / 5.0;
			pts.push(Pt::new(cx + r * a.cos(), cy + r * a.sin()));
		}
		pts
	}

	#[test]
	fn test_non_zero_is_the_default_and_is_unchanged_09() {
		// The rule-taking method must agree with the old one to the last bit, or every golden
		// image downstream shifts.
		let mut r = Raster::new(16, 16);
		r.add_contour(&square(1.5, 1.5, 12.25, 9.75));
		r.add_contour(&[Pt::new(2.0, 3.0), Pt::new(15.0, 4.5), Pt::new(6.0, 14.0)]);
		let old = r.coverage();
		let new = r.coverage_with(FillRule::NonZero);
		assert_eq!(old, new, "the non-zero rule must be bit-identical");
		assert_eq!(FillRule::default(), FillRule::NonZero);
	}

	#[test]
	fn test_even_odd_makes_a_hole_of_an_overlap_10() {
		// Two squares wound the same way. Non-zero unions them; even-odd cancels the overlap.
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(0.0, 0.0, 6.0, 8.0));
		r.add_contour(&square(3.5, 0.0, 8.0, 8.0));
		let nz = r.coverage_with(FillRule::NonZero);
		let eo = r.coverage_with(FillRule::EvenOdd);
		// Column 5 lies in the overlap, wound twice.
		assert!((nz[4 * 8 + 5] - 1.0).abs() < 1e-4, "non-zero should fill the overlap");
		assert!(eo[4 * 8 + 5] < 1e-4, "even-odd should hollow it, found {}", eo[4 * 8 + 5]);
		// Columns 1 and 7 lie under one square only, so both rules fill them.
		assert!((eo[4 * 8 + 1] - 1.0).abs() < 1e-4, "found {}", eo[4 * 8 + 1]);
		assert!((eo[4 * 8 + 7] - 1.0).abs() < 1e-4, "found {}", eo[4 * 8 + 7]);
	}

	#[test]
	fn test_even_odd_softens_the_edge_of_the_hole_11() {
		// The subtlety, pinned. Column 3 is half in the singly wound part and half in the doubly
		// wound part, so the running sum there is 1.5: a number that is neither odd nor even. The
		// answer is half coverage, which only a rule ramped between the integers can give. Testing
		// the parity of the rounded sum would say nothing here, and clamping it would say one.
		let mut r = Raster::new(8, 8);
		r.add_contour(&square(0.0, 0.0, 6.0, 8.0));
		r.add_contour(&square(3.5, 0.0, 8.0, 8.0));
		let eo = r.coverage_with(FillRule::EvenOdd);
		let c = eo[4 * 8 + 3];
		assert!((c - 0.5).abs() < 1e-3, "the hole's edge should be half covered, found {}", c);
	}

	#[test]
	fn test_even_odd_is_blind_to_direction_12() {
		// Winding the second square the other way changes the sum's sign but not its parity, so
		// even-odd hollows the overlap either way, where non-zero would only hollow one of them.
		let mut same = Raster::new(8, 8);
		same.add_contour(&square(0.0, 0.0, 6.0, 8.0));
		same.add_contour(&square(3.5, 0.0, 8.0, 8.0));
		let mut anti = Raster::new(8, 8);
		anti.add_contour(&square(0.0, 0.0, 6.0, 8.0));
		let mut back = square(3.5, 0.0, 8.0, 8.0);
		back.reverse();
		anti.add_contour(&back);
		let (a, b) = (same.coverage_with(FillRule::EvenOdd), anti.coverage_with(FillRule::EvenOdd));
		for i in 0..a.len() {
			assert!((a[i] - b[i]).abs() < 1e-4, "pixel {} differs: {} then {}", i, a[i], b[i]);
		}
	}

	#[test]
	fn test_even_odd_hollows_a_self_crossing_star_13() {
		// The classic case: the pentagon at the heart of a five-pointed star is wound twice.
		let mut r = Raster::new(32, 32);
		r.add_contour(&star(16.0, 16.0, 14.0));
		let nz = r.coverage_with(FillRule::NonZero);
		let eo = r.coverage_with(FillRule::EvenOdd);
		assert!((nz[16 * 32 + 16] - 1.0).abs() < 1e-4, "non-zero should fill the heart");
		assert!(eo[16 * 32 + 16] < 1e-4, "even-odd should hollow it, found {}", eo[16 * 32 + 16]);
		// The arms are wound once, so they stand under either rule.
		assert!((eo[5 * 32 + 16] - 1.0).abs() < 1e-3, "the top arm, found {}", eo[5 * 32 + 16]);
		assert!((nz[5 * 32 + 16] - 1.0).abs() < 1e-3);
	}

	#[test]
	fn test_the_rules_agree_where_nothing_overlaps_14() {
		// A single simple contour is wound once or not at all, and one is odd, so the rules must
		// give the same picture, anti-aliased edges and all.
		let mut r = Raster::new(16, 16);
		r.add_contour(&[Pt::new(1.3, 0.7), Pt::new(14.8, 2.2), Pt::new(5.5, 15.1)]);
		let nz = r.coverage_with(FillRule::NonZero);
		let eo = r.coverage_with(FillRule::EvenOdd);
		for i in 0..nz.len() {
			assert!((nz[i] - eo[i]).abs() < 1e-4, "pixel {} differs: {} then {}", i, nz[i], eo[i]);
		}
	}

	#[test]
	fn test_degenerate_input_is_survived_08() {
		let mut r = Raster::new(4, 4);
		r.add_contour(&[]);
		r.add_contour(&[Pt::new(1.0, 1.0)]);
		r.add_edge(Pt::new(f32::NAN, 0.0), Pt::new(1.0, 1.0));
		r.add_edge(Pt::new(0.0, 2.0), Pt::new(4.0, 2.0)); // Horizontal.
		let cov = r.coverage();
		assert!(cov.iter().all(|c| *c < 1e-6));
	}
}
