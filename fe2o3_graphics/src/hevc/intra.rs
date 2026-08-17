//! Predicting a block from the samples already decoded around it.
//!
//! A still picture refers to nothing but itself, so every block in it is predicted from its own
//! neighbours: the row above and the column to the left, both taken from samples already
//! reconstructed and **before** the deblocking filter has touched them. Thirty-five ways of doing it
//! exist -- planar, flat, and thirty-three directions -- and which one a block uses is the largest
//! part of what its syntax says.
//!
//! Three things happen before a single sample is predicted, and each of them changes the answer:
//!
//! - **Substitution** (§8.4.4.2.2). A block at the edge of the picture, or one whose neighbours have
//!   not been decoded yet, has no samples to predict from. Rather than refuse, the missing ones are
//!   filled in from whichever neighbour *is* available, working round the boundary from the bottom
//!   left; a block with no available neighbour at all is predicted from half of full scale.
//! - **Filtering** (§8.4.4.2.3). For all but the smallest blocks and the flattest directions, the
//!   boundary is smoothed with a three-tap filter first, because the prediction is about to be
//!   stretched across as much as thirty-two samples and a step in the reference becomes a step in
//!   the block. At thirty-two, and only for luma, a boundary that is already nearly a straight line
//!   is replaced by the straight line exactly -- which is what keeps a clear sky from banding.
//! - **The boundary filter** (§8.4.4.2.5, §8.4.4.2.6). The first row or column of a flat, vertical or
//!   horizontal prediction is nudged towards the neighbour it abuts, because those three modes
//!   otherwise leave a visible edge at the block boundary.
//!
//! Every one of those has a "not for chroma" or "not at thirty-two" or "not at four" attached to it,
//! and getting one wrong produces a picture that is *almost* right -- which then predicts the next
//! block, and the next.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

pub const MAX_TB: usize = 32;	// the largest block predicted at once, samples each way
pub const DC: u8 = 1;			// the flat prediction: the average of the boundary
pub const PLANAR: u8 = 0;		// the plane through the boundary
pub const VERTICAL: u8 = 26;	// straight down from the row above
pub const HORIZONTAL: u8 = 10;	// straight across from the column to the left

// How far each direction moves, in thirty-seconds of a sample per row (§8.4.4.2.6, Table 8-5),
// indexed by the prediction mode. Modes 0 and 1 are planar and flat and have no angle; 10 and 26
// are exactly horizontal and vertical and so have an angle of nought.
const ANGLE: [i32; 35] = [
	0, 0,
	32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26,
	-32, -26, -21, -17, -13, -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32,
];

// The reciprocal of the angle, in two hundred and fifty-sixths (Table 8-6). Only the modes whose
// angle is negative need it, which is 11 to 25; it is what projects the other boundary into the
// reference array, so that a direction pointing up and to the left can still be followed past the
// corner. Nought where it does not apply.
const INV_ANGLE: [i32; 35] = [
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	-4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, -1638, -4096,
	0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// How far from vertical or horizontal a direction has to be before the boundary is smoothed,
/// by block size (§8.4.4.2.3, Table 8-4).
///
/// Indexed by the base-two logarithm of the size. At four the boundary is never filtered, at eight
/// only the diagonals are, at sixteen nearly everything is and at thirty-two everything is.
fn smoothing_threshold(size: usize) -> Option<i32> {
	match size {
		8	=> Some(7),
		16	=> Some(1),
		32	=> Some(0),
		_	=> None,
	}
}

/// The samples around a block, in the arrangement the prediction reads them.
///
/// The specification indexes these as `p[x][y]` with one of the two being −1, which is one row
/// above and one column to the left of the block, each twice as long as the block is wide. Here
/// they are three runs in one array, because that is what they are.
#[derive(Clone, Debug)]
pub struct Around {
	size:	usize,				// the block's side
	corner:	i32,				// p[-1][-1]
	left:	[i32; MAX_TB * 2],	// p[-1][y] for y = 0 to 2 * size - 1, going down
	top:	[i32; MAX_TB * 2],	// p[x][-1] for x = 0 to 2 * size - 1, going right
}

impl Around {

	/// Room for a block of `size`, with nothing in it and nothing available.
	pub fn new(size: usize) -> Self {
		Self {
			size,
			corner:	-1,
			left:	[-1; MAX_TB * 2],
			top:	[-1; MAX_TB * 2],
		}
	}

	/// Takes a sample known to be available. A negative value means it is not.
	pub fn set_corner(&mut self, v: i32) {
		self.corner = v;
	}

	/// The same for one of the column to the left, `y` from nought.
	pub fn set_left(&mut self, y: usize, v: i32) {
		if y < self.left.len() {
			self.left[y] = v;
		}
	}

	/// And for one of the row above, `x` from nought.
	pub fn set_top(&mut self, x: usize, v: i32) {
		if x < self.top.len() {
			self.top[x] = v;
		}
	}

	/// `p[-1][-1]`.
	pub fn corner(&self) -> i32 {
		self.corner
	}

	/// `p[-1][y]`, where `y` of −1 is the corner.
	pub fn left(&self, y: i32) -> i32 {
		if y < 0 {
			self.corner
		} else {
			self.left[(y as usize).min(self.left.len() - 1)]
		}
	}

	/// `p[x][-1]`, where `x` of −1 is the corner.
	pub fn top(&self, x: i32) -> i32 {
		if x < 0 {
			self.corner
		} else {
			self.top[(x as usize).min(self.top.len() - 1)]
		}
	}

	/// Fills in every sample that is not available, from the ones that are (§8.4.4.2.2).
	///
	/// The walk is anticlockwise from the bottom of the left column, round the corner and along the
	/// top: each missing sample takes the value of the one behind it on that path. So a block with
	/// only a row above it predicts from that row extended downwards, and one with nothing at all
	/// predicts from mid-grey.
	pub fn substitute(&mut self, depth: u32) {
		let n = self.size * 2;
		let none = self.corner < 0
			&& self.left[..n].iter().all(|v| *v < 0)
			&& self.top[..n].iter().all(|v| *v < 0);
		if none {
			let half = 1i32 << (depth - 1);
			self.corner = half;
			self.left[..n].fill(half);
			self.top[..n].fill(half);
			return;
		}
		// The bottom of the left column is the start of the path, so if it is missing it takes the
		// first available sample found anywhere along the path.
		if self.left[n - 1] < 0 {
			let mut found = -1;
			for y in (0..n - 1).rev() {
				if self.left[y] >= 0 {
					found = self.left[y];
					break;
				}
			}
			if found < 0 && self.corner >= 0 {
				found = self.corner;
			}
			if found < 0 {
				for x in 0..n {
					if self.top[x] >= 0 {
						found = self.top[x];
						break;
					}
				}
			}
			self.left[n - 1] = found;
		}
		// Then up the column, round the corner, and along the row.
		for y in (0..n - 1).rev() {
			if self.left[y] < 0 {
				self.left[y] = self.left[y + 1];
			}
		}
		if self.corner < 0 {
			self.corner = self.left[0];
		}
		for x in 0..n {
			if self.top[x] < 0 {
				self.top[x] = if x == 0 { self.corner } else { self.top[x - 1] };
			}
		}
	}

	/// Smooths the boundary where the mode and the size call for it (§8.4.4.2.3).
	///
	/// `strong` is the sequence's strong-smoothing flag, which only ever applies to a
	/// thirty-two-sample luma block whose boundary is already within a small step of a straight
	/// line -- there it is replaced by the straight line exactly, which is what stops a clear sky
	/// from banding.
	pub fn smooth(&mut self, mode: u8, chroma: bool, strong: bool, depth: u32) {
		if chroma || mode == DC {
			return;
		}
		let n = self.size * 2;
		let threshold = match smoothing_threshold(self.size) {
			Some(t) => t,
			None => return,
		};
		if mode != PLANAR {
			let from_flat = ((mode as i32) - 26).abs().min(((mode as i32) - 10).abs());
			if from_flat <= threshold {
				return;
			}
		}
		if strong && self.size == 32 {
			let step = 1i32 << (depth - 5);
			let flat_top = (self.corner + self.top[n - 1] - 2 * self.top[self.size - 1]).abs() < step;
			let flat_left =
				(self.corner + self.left[n - 1] - 2 * self.left[self.size - 1]).abs() < step;
			if flat_top && flat_left {
				let (c, r, b) = (self.corner, self.top[n - 1], self.left[n - 1]);
				for y in 0..n - 1 {
					self.left[y] = ((63 - y as i32) * c + (y as i32 + 1) * b + 32) >> 6;
				}
				for x in 0..n - 1 {
					self.top[x] = ((63 - x as i32) * c + (x as i32 + 1) * r + 32) >> 6;
				}
				return;
			}
		}
		// The ordinary three-tap filter. Taken from copies, because each output reads its
		// neighbours' unfiltered values.
		let (was_corner, was_left, was_top) = (self.corner, self.left, self.top);
		self.corner = (was_left[0] + 2 * was_corner + was_top[0] + 2) >> 2;
		for y in 0..n - 1 {
			let above = if y == 0 { was_corner } else { was_left[y - 1] };
			self.left[y] = (was_left[y + 1] + 2 * was_left[y] + above + 2) >> 2;
		}
		for x in 0..n - 1 {
			let before = if x == 0 { was_corner } else { was_top[x - 1] };
			self.top[x] = (was_top[x + 1] + 2 * was_top[x] + before + 2) >> 2;
		}
	}
}

/// Predicts a block of `size` in `mode` from the samples around it.
///
/// `out` takes `size * size` samples in raster order. `depth` is the bit depth of the component,
/// which is what the prediction is clipped to.
pub fn predict(
	around:	&Around,
	mode:	u8,
	size:	usize,
	chroma:	bool,
	depth:	u32,
	out:	&mut [i32],
)
	-> Outcome<()>
{
	if mode as usize >= ANGLE.len() {
		return Err(err!("Intra prediction mode {} does not exist.", mode; Invalid, Input));
	}
	if out.len() < size * size {
		return Err(err!(
			"A block of {0} wants {1} samples and was given {2}.",
			size, size * size, out.len(); Invalid, Input));
	}
	match mode {
		PLANAR	=> planar(around, size, out),
		DC	=> flat(around, size, chroma, out),
		_	=> angular(around, mode, size, chroma, depth, out),
	}
	Ok(())
}

/// The plane through the four boundaries (§8.4.4.2.4).
///
/// Each sample is a weighted average of the four samples the block's edges point at: left, right,
/// above and below. The right and below ones do not exist, so the sample past the end of the row
/// above stands in for the right edge and the one past the bottom of the left column for the
/// bottom -- which is why the boundary is twice as long as the block.
fn planar(around: &Around, size: usize, out: &mut [i32]) {
	let n = size as i32;
	let shift = size.trailing_zeros() + 1;
	let right = around.top(n);
	let below = around.left(n);
	for y in 0..size {
		for x in 0..size {
			let v = (n - 1 - x as i32) * around.left(y as i32)
				+ (x as i32 + 1) * right
				+ (n - 1 - y as i32) * around.top(x as i32)
				+ (y as i32 + 1) * below
				+ n;
			out[y * size + x] = v >> shift;
		}
	}
}

/// The average of the boundary, with the first row and column pulled towards it (§8.4.4.2.5).
fn flat(around: &Around, size: usize, chroma: bool, out: &mut [i32]) {
	let n = size as i32;
	let mut sum = n;
	for i in 0..size {
		sum += around.top(i as i32) + around.left(i as i32);
	}
	let dc = sum >> (size.trailing_zeros() + 1);
	for v in out.iter_mut().take(size * size) {
		*v = dc;
	}
	// The boundary filter, which is luma only and not at the largest size: a flat block against a
	// detailed neighbour otherwise leaves a visible step exactly at the block edge.
	if chroma || size >= 32 {
		return;
	}
	out[0] = (around.left(0) + 2 * dc + around.top(0) + 2) >> 2;
	for x in 1..size {
		out[x] = (around.top(x as i32) + 3 * dc + 2) >> 2;
	}
	for y in 1..size {
		out[y * size] = (around.left(y as i32) + 3 * dc + 2) >> 2;
	}
}

/// One of the thirty-three directions (§8.4.4.2.6).
///
/// The samples are read along the boundary the direction points at, at a position that moves by
/// `intraPredAngle` thirty-seconds of a sample for every row (or column) crossed, with a two-tap
/// interpolation where the position lands between two samples. Where the direction points up and to
/// the left, the *other* boundary is projected into the reference array behind the corner, so that
/// following the direction backwards still finds samples.
fn angular(around: &Around, mode: u8, size: usize, chroma: bool, depth: u32, out: &mut [i32]) {
	let angle = ANGLE[mode as usize];
	let n = size as i32;
	// The reference runs from -size to 2*size, and index 0 of the array is position -size.
	let mut ref_: [i32; MAX_TB * 4 + 1] = [0; MAX_TB * 4 + 1];
	let base = size;
	let down = mode >= 18;
	let main = |i: i32| if down { around.top(i) } else { around.left(i) };
	let side = |i: i32| if down { around.left(i) } else { around.top(i) };

	for x in 0..=n {
		ref_[base + x as usize] = main(x - 1);
	}
	if angle < 0 {
		let reach = (n * angle) >> 5;
		if reach < -1 {
			let inv = INV_ANGLE[mode as usize];
			for x in reach..=-1 {
				let at = ((x * inv + 128) >> 8) - 1;
				ref_[(base as i32 + x) as usize] = side(at);
			}
		}
	} else {
		for x in n + 1..=2 * n {
			ref_[base + x as usize] = main(x - 1);
		}
	}

	for y in 0..size {
		for x in 0..size {
			// Which of the two axes counts the steps depends on which boundary is being read.
			let along = if down { y as i32 } else { x as i32 };
			let across = if down { x as i32 } else { y as i32 };
			let pos = (along + 1) * angle;
			let idx = pos >> 5;
			let frac = pos & 31;
			let at = base as i32 + across + idx + 1;
			let v = if frac != 0 {
				((32 - frac) * ref_[at as usize] + frac * ref_[(at + 1) as usize] + 16) >> 5
			} else {
				ref_[at as usize]
			};
			out[y * size + x] = v;
		}
	}

	// The boundary filter for exactly vertical and exactly horizontal, luma only, under
	// thirty-two: the prediction copies one edge and would otherwise ignore the other entirely.
	if chroma || size >= 32 {
		return;
	}
	let top = (1i32 << depth) - 1;
	if mode == VERTICAL {
		for y in 0..size {
			let v = around.top(0) + ((around.left(y as i32) - around.corner()) >> 1);
			out[y * size] = v.clamp(0, top);
		}
	} else if mode == HORIZONTAL {
		for x in 0..size {
			let v = around.left(0) + ((around.top(x as i32) - around.corner()) >> 1);
			out[x] = v.clamp(0, top);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A boundary whose every sample is the same value, all of it available.
	fn flat_around(size: usize, v: i32) -> Around {
		let mut a = Around::new(size);
		a.set_corner(v);
		for i in 0..size * 2 {
			a.set_left(i, v);
			a.set_top(i, v);
		}
		a
	}

	#[test]
	fn test_a_flat_neighbourhood_predicts_a_flat_block_00() -> Outcome<()> {
		// The property every mode shares and none may break: if everything around a block is the
		// same value, the block is that value. It holds for planar, for flat and for all
		// thirty-three directions, and it catches an interpolation whose weights do not sum to
		// thirty-two, a reference array read one sample out of step, and a boundary filter applied
		// where it should not be.
		for size in [4usize, 8, 16, 32] {
			for mode in 0..35u8 {
				let mut around = flat_around(size, 128);
				around.smooth(mode, false, true, 8);
				let mut out = vec![0i32; size * size];
				res!(predict(&around, mode, size, false, 8, &mut out));
				for (i, v) in out.iter().enumerate() {
					req!(*v, 128,
						"mode {} at {} put {} at sample {} of a uniform neighbourhood",
						mode, size, v, i);
				}
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_two_flat_directions_copy_the_boundary_they_point_at_01() -> Outcome<()> {
		// Mode 26 is straight down and mode 10 is straight across, so each column (or row) of the
		// block is a copy of the sample it points at. Checked on chroma, where the boundary filter
		// that would otherwise nudge the first row or column is not applied -- the filter itself is
		// checked separately below.
		let size = 8;
		let mut around = Around::new(size);
		around.set_corner(100);
		for i in 0..size * 2 {
			around.set_left(i, 40 + i as i32);
			around.set_top(i, 10 * (i as i32 + 1));
		}
		let mut out = vec![0i32; size * size];
		res!(predict(&around, VERTICAL, size, true, 8, &mut out));
		for y in 0..size {
			for x in 0..size {
				req!(out[y * size + x], around.top(x as i32),
					"straight down put the wrong sample at ({}, {})", x, y);
			}
		}
		res!(predict(&around, HORIZONTAL, size, true, 8, &mut out));
		for y in 0..size {
			for x in 0..size {
				req!(out[y * size + x], around.left(y as i32),
					"straight across put the wrong sample at ({}, {})", x, y);
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_two_diagonals_shift_by_one_sample_a_row_02() -> Outcome<()> {
		// Modes 2 and 34 are the forty-five degree directions, whose angle is exactly thirty-two
		// thirty-seconds -- one whole sample a row, so no interpolation happens and the block is
		// the boundary shifted. That makes them the two directions whose answer can be written down
		// without doing any arithmetic, which is what makes them worth asserting.
		let size = 8;
		let mut around = Around::new(size);
		around.set_corner(1);
		for i in 0..size * 2 {
			around.set_left(i, 100 + i as i32);
			around.set_top(i, 200 + i as i32);
		}
		let mut out = vec![0i32; size * size];
		// Thirty-four points up and to the right, reading the row above.
		res!(predict(&around, 34, size, true, 8, &mut out));
		for y in 0..size {
			for x in 0..size {
				req!(out[y * size + x], around.top((x + y + 1) as i32),
					"mode 34 at ({}, {})", x, y);
			}
		}
		// Two points down and to the left, reading the column.
		res!(predict(&around, 2, size, true, 8, &mut out));
		for y in 0..size {
			for x in 0..size {
				req!(out[y * size + x], around.left((x + y + 1) as i32),
					"mode 2 at ({}, {})", x, y);
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_missing_neighbourhood_is_filled_from_what_there_is_03() -> Outcome<()> {
		// A block at the very top left of a picture has nothing around it and predicts from half
		// scale; one with only a row above it extends that row round the corner and down the side.
		// Both are the ordinary case at a picture's edge, not an error.
		let size = 4;
		let mut nothing = Around::new(size);
		nothing.substitute(8);
		req!(nothing.corner(), 128);
		req!(nothing.left(0), 128);
		req!(nothing.top(7), 128);

		let mut only_above = Around::new(size);
		only_above.set_corner(60);
		for x in 0..size * 2 {
			only_above.set_top(x, 70 + x as i32);
		}
		only_above.substitute(8);
		// The corner was available, so the whole left column takes it -- the walk runs from the
		// bottom of the column upwards, and every one of them is missing.
		for y in 0..size * 2 {
			req!(only_above.left(y as i32), 60, "the left column at {} was not filled", y);
		}
		req!(only_above.top(0), 70, "an available sample was overwritten");

		// And a block with only a left column extends the bottom sample nowhere but keeps what it
		// has, filling the row above from the corner along.
		let mut only_left = Around::new(size);
		for y in 0..size * 2 {
			only_left.set_left(y, 90 + y as i32);
		}
		only_left.substitute(8);
		req!(only_left.corner(), 90, "the corner did not take the top of the column");
		for x in 0..size * 2 {
			req!(only_left.top(x as i32), 90, "the row above at {} was not filled", x);
		}
		Ok(())
	}

	#[test]
	fn test_the_boundary_is_smoothed_only_where_it_should_be_04() -> Outcome<()> {
		// The three-tap filter is not applied to chroma, nor to the smallest blocks, nor to
		// directions close to vertical or horizontal, nor to the flat mode. Each of those
		// exceptions is a line in the specification and each changes the picture.
		let step = |size: usize| {
			let mut a = Around::new(size);
			a.set_corner(0);
			for i in 0..size * 2 {
				// A step in the middle of each boundary, which a filter would round off.
				a.set_left(i, if i < size { 0 } else { 255 });
				a.set_top(i, if i < size { 0 } else { 255 });
			}
			a
		};
		// Four is never filtered.
		let mut small = step(4);
		small.smooth(PLANAR, false, false, 8);
		req!(small.left(4), 255, "a four-sample boundary was filtered");
		// Chroma is never filtered.
		let mut chroma = step(16);
		chroma.smooth(PLANAR, true, false, 8);
		req!(chroma.left(16), 255, "a chroma boundary was filtered");
		// The flat mode never filters.
		let mut dc = step(16);
		dc.smooth(DC, false, false, 8);
		req!(dc.left(16), 255, "the flat mode filtered its boundary");
		// Straight down at eight is within the threshold of seven, so it does not filter.
		let mut near = step(8);
		near.smooth(VERTICAL, false, false, 8);
		req!(near.left(8), 255, "a direction inside the threshold was filtered");
		// A diagonal at eight is outside it, so it does.
		let mut far = step(8);
		far.smooth(2, false, false, 8);
		let softened = far.left(8) != 255;
		req!(softened, true, "a diagonal at eight was not filtered");
		Ok(())
	}

	#[test]
	fn test_the_flat_mode_pulls_its_first_row_towards_the_neighbour_05() -> Outcome<()> {
		// The boundary filter on the flat mode, which is luma only and not at thirty-two. A block
		// whose average is one thing and whose neighbour is another must not meet that neighbour
		// with a step, so the first row and column are moved three quarters of the way to the
		// average and a quarter of the way to the neighbour.
		let size = 8;
		let mut around = Around::new(size);
		around.set_corner(0);
		for i in 0..size * 2 {
			around.set_left(i, 0);
			around.set_top(i, 80);
		}
		let mut out = vec![0i32; size * size];
		res!(predict(&around, DC, size, false, 8, &mut out));
		// The average of a boundary half nought and half eighty.
		let dc = 40;
		req!(out[size + 1], dc, "the middle of the block is not the average");
		req!(out[1], (80 + 3 * dc + 2) >> 2, "the first row was not pulled towards the row above");
		req!(out[size], (0 + 3 * dc + 2) >> 2, "the first column was not pulled");
		req!(out[0], (0 + 2 * dc + 80 + 2) >> 2, "the corner sample is wrong");

		// At thirty-two it does not happen at all.
		let mut big = Around::new(32);
		big.set_corner(0);
		for i in 0..64 {
			big.set_left(i, 0);
			big.set_top(i, 80);
		}
		let mut out = vec![0i32; 32 * 32];
		res!(predict(&big, DC, 32, false, 8, &mut out));
		req!(out[1], out[33], "a thirty-two block had its first row filtered");
		Ok(())
	}

	#[test]
	fn test_planar_is_symmetric_and_lands_where_the_equation_says_06() -> Outcome<()> {
		// Planar is a bilinear surface fitted to the four boundaries. It does **not** reproduce an
		// arbitrary plane -- the right and bottom edges are single samples taken from past the end
		// of the two boundaries, so a ramp comes back bent -- and a test claiming otherwise says
		// more about the person writing it than about the decoder.
		//
		// What does hold, and what catches the mistake worth catching, is symmetry: a
		// neighbourhood that is unchanged by swapping x and y must predict a block that is
		// unchanged by swapping x and y. An implementation with the two axes crossed fails it.
		let size = 8;
		let mut around = Around::new(size);
		around.set_corner(0);
		for i in 0..size * 2 {
			around.set_left(i, i as i32 + 1);
			around.set_top(i, i as i32 + 1);
		}
		let mut out = vec![0i32; size * size];
		res!(predict(&around, PLANAR, size, true, 8, &mut out));
		for y in 0..size {
			for x in 0..size {
				req!(out[y * size + x], out[x * size + y],
					"the plane is not symmetric at ({}, {})", x, y);
			}
		}
		// And one sample worked through the published equation by hand, which is what says the
		// weights are the right way round rather than merely symmetric. At (3, 1) with this
		// boundary: (4*2 + 4*9 + 6*4 + 2*9 + 8) >> 4.
		req!(out[1 * size + 3], (4 * 2 + 4 * 9 + 6 * 4 + 2 * 9 + 8) >> 4);
		Ok(())
	}
}
