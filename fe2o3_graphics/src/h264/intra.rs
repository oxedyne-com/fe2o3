//! Predicting a block from the samples around it.
//!
//! An intra picture carries no samples directly. Every block is *predicted* from the row above it
//! and the column to its left -- both already reconstructed -- and what the bitstream carries is
//! only the difference. So the prediction is not an optimisation: it is most of the picture, and a
//! mode implemented slightly wrongly produces a picture that is recognisable and wrong rather than
//! an error.
//!
//! There are four families, and a real film uses all of them:
//!
//! - **Intra_4x4** (§8.3.1.2), nine modes over a four-by-four block.
//! - **Intra_8x8** (§8.3.2.2), the same nine modes over an eight-by-eight block, with the
//!   reference samples **filtered first**. High profile adds it and 861 films in the corpus turn it
//!   on.
//! - **Intra_16x16** (§8.3.3), four modes over the whole macroblock, for regions with little detail.
//! - **Chroma** (§8.3.4), four modes over an eight-by-eight chroma block, both components together.
//!
//! # Availability, which is the part that cannot be seen
//!
//! A neighbour may be predicted from only where it has already been decoded *and* belongs to the
//! same slice. A block at the left edge of a picture has no left neighbour; a block in the second
//! slice of a picture has no neighbour in the first, however close it sits. Each mode is defined
//! only for a particular set of available neighbours, and where they are missing the direct-current
//! mode falls back through three cases to the mid-grey of `1 << (bitDepth − 1)`. A decoder careless
//! about it predicts from samples that are still nought and produces a picture with a plausible
//! grid of dark blocks -- which is why availability is carried here as explicit flags on
//! [`Edges`] rather than inferred from whether a sample happens to be zero.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

/// The samples around a block, and which of them may be predicted from.
///
/// One shape serves all four families. `top` runs from the block's left edge rightwards and holds
/// up to sixteen samples: the eight or sixteen above the block, and then the ones above and to the
/// right where the mode reaches for them. `left` runs downwards from the block's top edge.
#[derive(Clone, Debug)]
pub struct Edges {
	pub top:	[i32; 16],	// p[x, −1] for x from nought
	pub top_ok:	bool,		// may the samples directly above the block be predicted from?
	pub right_ok:	bool,		// may those above and to the right of it be?
	pub left:	[i32; 16],	// p[−1, y] for y from nought
	pub left_ok:	bool,		// may the column to the left be predicted from?
	pub corner:	i32,		// p[−1, −1], the sample diagonally above and left
	pub corner_ok:	bool,		// may that one be?
}

impl Edges {

	/// Edges with nothing available, which is what a macroblock at the top-left corner of a slice
	/// has.
	pub fn none() -> Self {
		Self {
			top:		[0; 16],
			top_ok:		false,
			right_ok:	false,
			left:		[0; 16],
			left_ok:	false,
			corner:		0,
			corner_ok:	false,
		}
	}

	/// Extends the row above rightwards where the mode reaches past what is available (§8.3.1.2).
	///
	/// "When samples `p[x, −1]`, with `x` = 4..7, are marked as not available and the sample
	/// `p[3, −1]` is available, the sample value of `p[3, −1]` is substituted." A block at the right
	/// edge of a picture, or one whose upper-right neighbour has not been decoded yet, still uses
	/// the diagonal modes; it uses them against a repeated sample. Leaving the substitution out
	/// makes every such block predict from nought, which is a black wedge in the corner of it.
	pub fn pad_right(&mut self, from: usize, to: usize) {
		if self.right_ok || !self.top_ok || from == 0 {
			return;
		}
		let v = self.top[from - 1];
		for x in from..to.min(16) {
			self.top[x] = v;
		}
		self.right_ok = true;
	}
}

fn clip(v: i32, bit_depth: u32) -> i32 {
	v.clamp(0, (1i32 << bit_depth) - 1)
}

/// The value a block takes where nothing around it is available: mid-grey.
fn mid(bit_depth: u32) -> i32 {
	1 << (bit_depth - 1)
}

/// One of the nine directions a four-by-four or eight-by-eight block may be predicted in.
///
/// The numbering is the specification's, and the order matters: the most probable mode machinery
/// in §8.3.1.1 compares these numbers directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
	Vertical,			// straight down from the row above
	Horizontal,			// straight across from the column to the left
	Dc,				// the mean of whichever neighbours there are
	DiagonalDownLeft,		// at forty-five degrees
	DiagonalDownRight,
	VerticalRight,			// steeply down and right
	HorizontalDown,			// shallowly down and right
	VerticalLeft,			// steeply down and left
	HorizontalUp,			// shallowly up and right
}

impl Mode {

	pub fn of(n: u32) -> Outcome<Self> {
		Ok(match n {
			0	=> Self::Vertical,
			1	=> Self::Horizontal,
			2	=> Self::Dc,
			3	=> Self::DiagonalDownLeft,
			4	=> Self::DiagonalDownRight,
			5	=> Self::VerticalRight,
			6	=> Self::HorizontalDown,
			7	=> Self::VerticalLeft,
			8	=> Self::HorizontalUp,
			_	=> return Err(err!(
				"An intra prediction mode of {} was coded, and 0 to 8 are the only ones defined.",
				n; Invalid, Input, Decode)),
		})
	}

	pub fn number(self) -> u32 {
		match self {
			Self::Vertical			=> 0,
			Self::Horizontal		=> 1,
			Self::Dc			=> 2,
			Self::DiagonalDownLeft		=> 3,
			Self::DiagonalDownRight		=> 4,
			Self::VerticalRight		=> 5,
			Self::HorizontalDown		=> 6,
			Self::VerticalLeft		=> 7,
			Self::HorizontalUp		=> 8,
		}
	}
}

/// The direct-current prediction, which is the one mode a block may always use (§8.3.1.2.3).
///
/// Its whole substance is the four cases: both edges, the left alone, the top alone, and neither.
fn dc(e: &Edges, n: usize, bit_depth: u32) -> i32 {
	let top: i32 = e.top[..n].iter().sum();
	let left: i32 = e.left[..n].iter().sum();
	let shift = n.trailing_zeros();
	match (e.top_ok, e.left_ok) {
		(true, true)	=> (top + left + n as i32) >> (shift + 1),
		(false, true)	=> (left + (n as i32 >> 1)) >> shift,
		(true, false)	=> (top + (n as i32 >> 1)) >> shift,
		(false, false)	=> mid(bit_depth),
	}
}

/// Predicts a four-by-four luma block (§8.3.1.2).
///
/// The caller has already padded the row above where the mode reaches past it; see
/// [`Edges::pad_right`].
pub fn pred_4x4(mode: Mode, e: &Edges, bit_depth: u32) -> [i32; 16] {
	let mut p = [0i32; 16];
	let t = &e.top;
	let l = &e.left;
	let c = e.corner;
	match mode {
		Mode::Vertical => {
			for y in 0..4 {
				for x in 0..4 {
					p[y * 4 + x] = t[x];
				}
			}
		},
		Mode::Horizontal => {
			for y in 0..4 {
				for x in 0..4 {
					p[y * 4 + x] = l[y];
				}
			}
		},
		Mode::Dc => {
			let v = dc(e, 4, bit_depth);
			p = [v; 16];
		},
		Mode::DiagonalDownLeft => {
			for y in 0..4 {
				for x in 0..4 {
					p[y * 4 + x] = if x == 3 && y == 3 {
						(t[6] + 3 * t[7] + 2) >> 2
					} else {
						(t[x + y] + 2 * t[x + y + 1] + t[x + y + 2] + 2) >> 2
					};
				}
			}
		},
		Mode::DiagonalDownRight => {
			for y in 0..4i32 {
				for x in 0..4i32 {
					let (xu, yu) = (x as usize, y as usize);
					p[yu * 4 + xu] = if x > y {
						let d = (x - y) as usize;
						(at(t, c, d as i32 - 2) + 2 * at(t, c, d as i32 - 1) + t[d] + 2) >> 2
					} else if x < y {
						let d = (y - x) as usize;
						(at(l, c, d as i32 - 2) + 2 * at(l, c, d as i32 - 1) + l[d] + 2) >> 2
					} else {
						(t[0] + 2 * c + l[0] + 2) >> 2
					};
				}
			}
		},
		Mode::VerticalRight => {
			for y in 0..4i32 {
				for x in 0..4i32 {
					let z = 2 * x - y;
					let (xu, yu) = (x as usize, y as usize);
					let h = x - (y >> 1);
					p[yu * 4 + xu] = match z {
						0 | 2 | 4 | 6	=> (at(t, c, h - 1) + at(t, c, h) + 1) >> 1,
						1 | 3 | 5	=> (at(t, c, h - 2) + 2 * at(t, c, h - 1)
									+ at(t, c, h) + 2) >> 2,
						-1		=> (l[0] + 2 * c + t[0] + 2) >> 2,
						_		=> (at(l, c, y - 1) + 2 * at(l, c, y - 2)
									+ at(l, c, y - 3) + 2) >> 2,
					};
				}
			}
		},
		Mode::HorizontalDown => {
			for y in 0..4i32 {
				for x in 0..4i32 {
					let z = 2 * y - x;
					let (xu, yu) = (x as usize, y as usize);
					let v = y - (x >> 1);
					p[yu * 4 + xu] = match z {
						0 | 2 | 4 | 6	=> (at(l, c, v - 1) + at(l, c, v) + 1) >> 1,
						1 | 3 | 5	=> (at(l, c, v - 2) + 2 * at(l, c, v - 1)
									+ at(l, c, v) + 2) >> 2,
						-1		=> (l[0] + 2 * c + t[0] + 2) >> 2,
						_		=> (at(t, c, x - 1) + 2 * at(t, c, x - 2)
									+ at(t, c, x - 3) + 2) >> 2,
					};
				}
			}
		},
		Mode::VerticalLeft => {
			for y in 0..4 {
				for x in 0..4 {
					let h = x + (y >> 1);
					p[y * 4 + x] = if y % 2 == 0 {
						(t[h] + t[h + 1] + 1) >> 1
					} else {
						(t[h] + 2 * t[h + 1] + t[h + 2] + 2) >> 2
					};
				}
			}
		},
		Mode::HorizontalUp => {
			for y in 0..4 {
				for x in 0..4 {
					let z = x + 2 * y;
					let v = y + (x >> 1);
					p[y * 4 + x] = match z {
						0 | 2 | 4	=> (l[v] + l[v + 1] + 1) >> 1,
						1 | 3		=> (l[v] + 2 * l[v + 1] + l[v + 2] + 2) >> 2,
						5		=> (l[2] + 3 * l[3] + 2) >> 2,
						_		=> l[3],
					};
				}
			}
		},
	}
	for v in p.iter_mut() {
		*v = clip(*v, bit_depth);
	}
	p
}

/// One sample of an edge, where an index of −1 means the corner.
///
/// The diagonal modes index an edge from −1, which in the specification's notation is `p[−1, −1]`
/// for both edges at once. Writing that as a signed index into one array keeps each mode's
/// arithmetic the shape the clause gives it.
fn at(edge: &[i32; 16], corner: i32, i: i32) -> i32 {
	if i < 0 {
		corner
	} else {
		edge[(i as usize).min(15)]
	}
}

/// Filters the reference samples an eight-by-eight block predicts from (§8.3.2.2.1).
///
/// Every Intra_8x8 mode reads the *filtered* samples, not the reconstructed ones. This is the one
/// step Intra_4x4 has no equivalent of, and leaving it out gives a picture that is right in its
/// large shapes and wrong in every eight-by-eight block's texture.
pub fn filter_8x8(e: &Edges) -> Edges {
	let mut out = e.clone();
	if e.top_ok && e.right_ok {
		out.top[0] = if e.corner_ok {
			(e.corner + 2 * e.top[0] + e.top[1] + 2) >> 2
		} else {
			(3 * e.top[0] + e.top[1] + 2) >> 2
		};
		for x in 1..15 {
			out.top[x] = (e.top[x - 1] + 2 * e.top[x] + e.top[x + 1] + 2) >> 2;
		}
		out.top[15] = (e.top[14] + 3 * e.top[15] + 2) >> 2;
	}
	if e.corner_ok {
		out.corner = match (e.top_ok, e.left_ok) {
			(true, true)	=> (e.top[0] + 2 * e.corner + e.left[0] + 2) >> 2,
			(true, false)	=> (3 * e.corner + e.top[0] + 2) >> 2,
			(false, true)	=> (3 * e.corner + e.left[0] + 2) >> 2,
			// Not used by any mode in this case, but defined so that nothing reads a stale value.
			(false, false)	=> e.corner,
		};
	}
	if e.left_ok {
		out.left[0] = if e.corner_ok {
			(e.corner + 2 * e.left[0] + e.left[1] + 2) >> 2
		} else {
			(3 * e.left[0] + e.left[1] + 2) >> 2
		};
		for y in 1..7 {
			out.left[y] = (e.left[y - 1] + 2 * e.left[y] + e.left[y + 1] + 2) >> 2;
		}
		out.left[7] = (e.left[6] + 3 * e.left[7] + 2) >> 2;
	}
	out
}

/// Predicts an eight-by-eight luma block (§8.3.2.2).
///
/// `e` holds the **unfiltered** samples; the filtering of §8.3.2.2.1 is done here, because every
/// mode wants it and a caller that had to remember would eventually forget.
pub fn pred_8x8(mode: Mode, e: &Edges, bit_depth: u32) -> [i32; 64] {
	let f = filter_8x8(e);
	let t = &f.top;
	let l = &f.left;
	let c = f.corner;
	let mut p = [0i32; 64];
	match mode {
		Mode::Vertical => {
			for y in 0..8 {
				for x in 0..8 {
					p[y * 8 + x] = t[x];
				}
			}
		},
		Mode::Horizontal => {
			for y in 0..8 {
				for x in 0..8 {
					p[y * 8 + x] = l[y];
				}
			}
		},
		Mode::Dc => {
			let v = dc(&f, 8, bit_depth);
			p = [v; 64];
		},
		Mode::DiagonalDownLeft => {
			for y in 0..8 {
				for x in 0..8 {
					p[y * 8 + x] = if x == 7 && y == 7 {
						(t[14] + 3 * t[15] + 2) >> 2
					} else {
						(t[x + y] + 2 * t[x + y + 1] + t[x + y + 2] + 2) >> 2
					};
				}
			}
		},
		Mode::DiagonalDownRight => {
			for y in 0..8i32 {
				for x in 0..8i32 {
					let (xu, yu) = (x as usize, y as usize);
					p[yu * 8 + xu] = if x > y {
						let d = (x - y) as usize;
						(at(t, c, d as i32 - 2) + 2 * at(t, c, d as i32 - 1) + t[d] + 2) >> 2
					} else if x < y {
						let d = (y - x) as usize;
						(at(l, c, d as i32 - 2) + 2 * at(l, c, d as i32 - 1) + l[d] + 2) >> 2
					} else {
						(t[0] + 2 * c + l[0] + 2) >> 2
					};
				}
			}
		},
		Mode::VerticalRight => {
			for y in 0..8i32 {
				for x in 0..8i32 {
					let z = 2 * x - y;
					let (xu, yu) = (x as usize, y as usize);
					let h = x - (y >> 1);
					p[yu * 8 + xu] = if z >= 0 && z % 2 == 0 {
						(at(t, c, h - 1) + at(t, c, h) + 1) >> 1
					} else if z >= 0 {
						(at(t, c, h - 2) + 2 * at(t, c, h - 1) + at(t, c, h) + 2) >> 2
					} else if z == -1 {
						(l[0] + 2 * c + t[0] + 2) >> 2
					} else {
						let k = y - 2 * x;
						(at(l, c, k - 1) + 2 * at(l, c, k - 2) + at(l, c, k - 3) + 2) >> 2
					};
				}
			}
		},
		Mode::HorizontalDown => {
			for y in 0..8i32 {
				for x in 0..8i32 {
					let z = 2 * y - x;
					let (xu, yu) = (x as usize, y as usize);
					let v = y - (x >> 1);
					p[yu * 8 + xu] = if z >= 0 && z % 2 == 0 {
						(at(l, c, v - 1) + at(l, c, v) + 1) >> 1
					} else if z >= 0 {
						(at(l, c, v - 2) + 2 * at(l, c, v - 1) + at(l, c, v) + 2) >> 2
					} else if z == -1 {
						(l[0] + 2 * c + t[0] + 2) >> 2
					} else {
						let k = x - 2 * y;
						(at(t, c, k - 1) + 2 * at(t, c, k - 2) + at(t, c, k - 3) + 2) >> 2
					};
				}
			}
		},
		Mode::VerticalLeft => {
			for y in 0..8 {
				for x in 0..8 {
					let h = x + (y >> 1);
					p[y * 8 + x] = if y % 2 == 0 {
						(t[h] + t[h + 1] + 1) >> 1
					} else {
						(t[h] + 2 * t[h + 1] + t[h + 2] + 2) >> 2
					};
				}
			}
		},
		Mode::HorizontalUp => {
			for y in 0..8 {
				for x in 0..8 {
					let z = x + 2 * y;
					let v = y + (x >> 1);
					p[y * 8 + x] = if z <= 12 && z % 2 == 0 {
						(l[v] + l[v.min(6) + 1] + 1) >> 1
					} else if z <= 11 {
						(l[v] + 2 * l[(v + 1).min(7)] + l[(v + 2).min(7)] + 2) >> 2
					} else if z == 13 {
						(l[6] + 3 * l[7] + 2) >> 2
					} else {
						l[7]
					};
				}
			}
		},
	}
	for v in p.iter_mut() {
		*v = clip(*v, bit_depth);
	}
	p
}

/// One of the four ways a whole macroblock's luma may be predicted (§8.3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode16 {
	Vertical,
	Horizontal,
	Dc,		// the mean
	Plane,		// a tilted plane fitted to the row above and the column to the left
}

impl Mode16 {

	pub fn of(n: u32) -> Outcome<Self> {
		Ok(match n {
			0	=> Self::Vertical,
			1	=> Self::Horizontal,
			2	=> Self::Dc,
			3	=> Self::Plane,
			_	=> return Err(err!(
				"An Intra_16x16 prediction mode of {} was coded, and 0 to 3 are the only ones \
				defined.", n; Invalid, Input, Decode)),
		})
	}
}

/// Predicts a whole macroblock's luma (§8.3.3).
pub fn pred_16x16(mode: Mode16, e: &Edges, bit_depth: u32) -> [i32; 256] {
	let mut p = [0i32; 256];
	match mode {
		Mode16::Vertical => {
			for y in 0..16 {
				for x in 0..16 {
					p[y * 16 + x] = e.top[x];
				}
			}
		},
		Mode16::Horizontal => {
			for y in 0..16 {
				for x in 0..16 {
					p[y * 16 + x] = e.left[y];
				}
			}
		},
		Mode16::Dc => {
			let v = dc(e, 16, bit_depth);
			p = [v; 256];
		},
		Mode16::Plane => {
			// A plane through the corner samples: `a` is twice the mean of the two far corners,
			// and `b` and `c` are the slopes, each a weighted difference across the edge.
			let mut h = 0i32;
			let mut v = 0i32;
			for i in 0..8i32 {
				let iu = i as usize;
				h += (i + 1) * (e.top[8 + iu] - at(&e.top, e.corner, 6 - i));
				v += (i + 1) * (e.left[8 + iu] - at(&e.left, e.corner, 6 - i));
			}
			let a = 16 * (e.left[15] + e.top[15]);
			let b = (5 * h + 32) >> 6;
			let c = (5 * v + 32) >> 6;
			for y in 0..16i32 {
				for x in 0..16i32 {
					p[(y * 16 + x) as usize] = (a + b * (x - 7) + c * (y - 7) + 16) >> 5;
				}
			}
		},
	}
	for s in p.iter_mut() {
		*s = clip(*s, bit_depth);
	}
	p
}

/// One of the four ways a macroblock's chroma may be predicted (§8.3.4).
///
/// The numbering is not the luma one: chroma codes the direct current first and the two straight
/// directions the other way about. Reading a chroma mode as though it were a luma one swaps every
/// picture's horizontal and vertical gradients, which is the kind of fault that looks almost right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeC {
	Dc,		// the mean, taken separately over each four-by-four quarter
	Horizontal,
	Vertical,
	Plane,
}

impl ModeC {

	pub fn of(n: u32) -> Outcome<Self> {
		Ok(match n {
			0	=> Self::Dc,
			1	=> Self::Horizontal,
			2	=> Self::Vertical,
			3	=> Self::Plane,
			_	=> return Err(err!(
				"An intra_chroma_pred_mode of {} was coded, and 0 to 3 are the only ones defined.",
				n; Invalid, Input, Decode)),
		})
	}
}

/// Predicts one eight-by-eight chroma block of a 4:2:0 macroblock (§8.3.4).
pub fn pred_chroma(mode: ModeC, e: &Edges, bit_depth: u32) -> [i32; 64] {
	let mut p = [0i32; 64];
	match mode {
		ModeC::Horizontal => {
			for y in 0..8 {
				for x in 0..8 {
					p[y * 8 + x] = e.left[y];
				}
			}
		},
		ModeC::Vertical => {
			for y in 0..8 {
				for x in 0..8 {
					p[y * 8 + x] = e.top[x];
				}
			}
		},
		ModeC::Dc => {
			// Each four-by-four quarter takes its own mean, and *which* edges it prefers depends
			// on where in the block it sits: the two quarters off the diagonal look first along
			// the edge they touch, and only then at the other one. A decoder that averaged the
			// whole block would give a chroma plane that is smooth where the picture is not.
			for by in 0..2usize {
				for bx in 0..2usize {
					let (xo, yo) = (bx * 4, by * 4);
					let top: i32 = e.top[xo..xo + 4].iter().sum();
					let left: i32 = e.left[yo..yo + 4].iter().sum();
					let v = if (bx == 0 && by == 0) || (bx > 0 && by > 0) {
						match (e.top_ok, e.left_ok) {
							(true, true)	=> (top + left + 4) >> 3,
							(false, true)	=> (left + 2) >> 2,
							(true, false)	=> (top + 2) >> 2,
							(false, false)	=> mid(bit_depth),
						}
					} else if bx > 0 {
						// Upper right: along the top first.
						match (e.top_ok, e.left_ok) {
							(true, _)	=> (top + 2) >> 2,
							(false, true)	=> (left + 2) >> 2,
							(false, false)	=> mid(bit_depth),
						}
					} else {
						// Lower left: down the side first.
						match (e.left_ok, e.top_ok) {
							(true, _)	=> (left + 2) >> 2,
							(false, true)	=> (top + 2) >> 2,
							(false, false)	=> mid(bit_depth),
						}
					};
					for y in 0..4 {
						for x in 0..4 {
							p[(yo + y) * 8 + xo + x] = v;
						}
					}
				}
			}
		},
		ModeC::Plane => {
			let mut h = 0i32;
			let mut v = 0i32;
			for i in 0..4i32 {
				let iu = i as usize;
				h += (i + 1) * (e.top[4 + iu] - at(&e.top, e.corner, 2 - i));
				v += (i + 1) * (e.left[4 + iu] - at(&e.left, e.corner, 2 - i));
			}
			let a = 16 * (e.left[7] + e.top[7]);
			let b = (34 * h + 32) >> 6;
			let c = (34 * v + 32) >> 6;
			for y in 0..8i32 {
				for x in 0..8i32 {
					p[(y * 8 + x) as usize] = (a + b * (x - 3) + c * (y - 3) + 16) >> 5;
				}
			}
		},
	}
	for s in p.iter_mut() {
		*s = clip(*s, bit_depth);
	}
	p
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Edges with a known ramp along each, and everything available.
	fn ramp() -> Edges {
		let mut e = Edges::none();
		for i in 0..16 {
			e.top[i] = 10 + i as i32;
			e.left[i] = 100 + i as i32;
		}
		e.top_ok = true;
		e.right_ok = true;
		e.left_ok = true;
		e.corner = 50;
		e.corner_ok = true;
		e
	}

	#[test]
	fn test_the_straight_modes_copy_the_edge_they_name_01() -> Outcome<()> {
		// Vertical copies the row above down every column, horizontal copies the column across
		// every row. The two are one transposition apart, and swapping them is the single easiest
		// mistake to make here -- so both are checked against an edge whose samples are all
		// different from each other's.
		let e = ramp();
		let v = pred_4x4(Mode::Vertical, &e, 8);
		for y in 0..4 {
			for x in 0..4 {
				req!(v[y * 4 + x], e.top[x], "vertical at ({}, {})", x, y);
			}
		}
		let h = pred_4x4(Mode::Horizontal, &e, 8);
		for y in 0..4 {
			for x in 0..4 {
				req!(h[y * 4 + x], e.left[y], "horizontal at ({}, {})", x, y);
			}
		}
		// And chroma numbers them the other way about, which is the fault this guards against.
		let cv = pred_chroma(ModeC::Vertical, &e, 8);
		let ch = pred_chroma(ModeC::Horizontal, &e, 8);
		req!(cv[1], e.top[1], "chroma vertical did not take the row above");
		req!(ch[8], e.left[1], "chroma horizontal did not take the column beside");
		let same = cv == ch;
		req!(same, false, "the two chroma directions are the same prediction");
		Ok(())
	}

	#[test]
	fn test_the_mean_falls_back_through_its_four_cases_02() -> Outcome<()> {
		// The direct current mode is the only one a block may always use, and it is the only one
		// whose answer depends on what is *missing*. Each of the four cases is a different
		// divisor, and taking the wrong one is a block that is uniformly too bright or too dark.
		let mut e = ramp();
		// Both edges: the mean of eight samples.
		let both = pred_4x4(Mode::Dc, &e, 8)[0];
		let want = (10 + 11 + 12 + 13 + 100 + 101 + 102 + 103 + 4) >> 3;
		req!(both, want);
		// The left alone.
		e.top_ok = false;
		req!(pred_4x4(Mode::Dc, &e, 8)[0], (100 + 101 + 102 + 103 + 2) >> 2);
		// The top alone.
		e.top_ok = true;
		e.left_ok = false;
		req!(pred_4x4(Mode::Dc, &e, 8)[0], (10 + 11 + 12 + 13 + 2) >> 2);
		// Neither: mid-grey, and *not* nought, which is what a decoder that read the unavailable
		// samples anyway would produce.
		e.top_ok = false;
		req!(pred_4x4(Mode::Dc, &e, 8)[0], 128, "a block with no neighbours came out black");
		req!(pred_4x4(Mode::Dc, &e, 10)[0], 512, "mid-grey is not scaled to the bit depth");
		Ok(())
	}

	#[test]
	fn test_an_eight_by_eight_block_predicts_from_filtered_samples_03() -> Outcome<()> {
		// The step Intra_4x4 has no equivalent of. A vertical prediction of an eight-by-eight block
		// does not copy the row above; it copies the row above smoothed by a three-tap filter. The
		// difference is invisible on a flat edge and obvious on a step, so the fixture is a step.
		let mut e = Edges::none();
		for i in 0..16 {
			e.top[i] = if i < 8 { 0 } else { 200 };
			e.left[i] = 60;
		}
		e.top_ok = true;
		e.right_ok = true;
		e.left_ok = true;
		e.corner = 60;
		e.corner_ok = true;
		let p = pred_8x8(Mode::Vertical, &e, 8);
		// The filter spreads the step over the two samples either side of it.
		let unfiltered = p[7] == e.top[7];
		req!(unfiltered, false, "an eight-by-eight block predicted from unfiltered samples");
		req!(p[7], (0 + 2 * 0 + 200 + 2) >> 2, "the filter is not the published three-tap one");
		// Away from the step it changes nothing, which is why the fault survives a flat test.
		req!(p[2], 0);
		req!(p[6], 0);
		// The leftmost column takes the corner into the filter, so it is not the sample above it.
		req!(p[0], (60 + 2 * 0 + 0 + 2) >> 2, "the corner was left out of the filter");
		// And a vertical prediction repeats down every row.
		req!(p[8], p[0]);
		Ok(())
	}

	#[test]
	fn test_an_unavailable_upper_right_is_repeated_not_read_04() -> Outcome<()> {
		// A block at the right edge of a picture has no samples above and to the right, and the
		// diagonal modes reach for them anyway. The substitution rule repeats `p[3, −1]`; without
		// it the modes read whatever is in the array, which for a fresh one is nought -- a black
		// wedge in the corner of every block along the right edge.
		let mut e = ramp();
		e.right_ok = false;
		for x in 4..16 {
			e.top[x] = 0;
		}
		e.pad_right(4, 8);
		let padded = e.right_ok;
		req!(padded, true, "the substitution did not mark the samples available");
		for x in 4..8 {
			req!(e.top[x], 13, "the sample at {} was not the repeat of p[3, -1]", x);
		}
		let p = pred_4x4(Mode::DiagonalDownLeft, &e, 8);
		// Bottom right corner, which reads only the repeated samples.
		req!(p[15], (13 + 3 * 13 + 2) >> 2);
		let black = p[15] == 0;
		req!(black, false, "a block at the right edge predicted from nothing");
		Ok(())
	}

	#[test]
	fn test_the_plane_modes_fit_a_ramp_exactly_05() -> Outcome<()> {
		// A plane fitted to edges that lie on a plane must reproduce it. The fixture is a linear
		// ramp across and down, so every predicted sample is determined, and a sign error in
		// either slope shows up as a picture that leans the wrong way.
		let mut e = Edges::none();
		for i in 0..16i32 {
			e.top[i as usize] = 100 + 2 * i;
			e.left[i as usize] = 100 + 2 * i;
		}
		e.top_ok = true;
		e.right_ok = true;
		e.left_ok = true;
		e.corner = 98;
		e.corner_ok = true;
		let p = pred_16x16(Mode16::Plane, &e, 8);
		// Along the top row the prediction should climb at the same rate the edge does.
		let step = p[1] - p[0];
		req!(step, 2, "the plane climbs across at {} where the edge climbs at 2", step);
		let down = p[16] - p[0];
		req!(down, 2, "the plane climbs down at {} where the edge climbs at 2", down);
		// And it must climb *up* to the right, not down: a sign error passes the step check.
		let rising = p[15] > p[0];
		req!(rising, true, "the plane leans the wrong way across");
		let falling_down = p[240] > p[0];
		req!(falling_down, true, "the plane leans the wrong way down");
		Ok(())
	}

	#[test]
	fn test_the_chroma_mean_takes_each_quarter_on_its_own_06() -> Outcome<()> {
		// Chroma's direct current mode is four means and not one, and the two quarters off the
		// diagonal prefer the edge they touch. A decoder that took one mean over the whole block
		// gives a chroma plane that is smooth where the picture is not, which shows as colour
		// bleeding across a hard edge.
		let mut e = Edges::none();
		for i in 0..8 {
			e.top[i] = if i < 4 { 20 } else { 200 };
			e.left[i] = if i < 4 { 30 } else { 210 };
		}
		e.top_ok = true;
		e.left_ok = true;
		let p = pred_chroma(ModeC::Dc, &e, 8);
		// Upper left: both edges.
		req!(p[0], (4 * 20 + 4 * 30 + 4) >> 3);
		// Upper right: the top alone, even though the left is available.
		req!(p[4], (4 * 200 + 2) >> 2);
		// Lower left: the left alone.
		req!(p[32], (4 * 210 + 2) >> 2);
		// Lower right: both again.
		req!(p[36], (4 * 200 + 4 * 210 + 4) >> 3);
		let uniform = p.iter().all(|v| *v == p[0]);
		req!(uniform, false, "the whole chroma block took one mean");
		Ok(())
	}
}
