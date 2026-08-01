//! The three orders coefficients are read in (§6.5.3 to §6.5.5).
//!
//! A transform block is read as a grid of four-by-four sub-blocks, and both the sub-blocks and the
//! coefficients within one are visited in the same order: up the diagonals, along the rows, or down
//! the columns. Which of the three a block uses is settled by its intra prediction mode -- a block
//! predicted nearly horizontally has its energy in the vertical direction, so it is read down the
//! columns, and the other way about.
//!
//! Every scan runs **backwards** in the bitstream: the coefficient furthest from the corner is coded
//! first and the direct one last, which is why the syntax begins by saying where the last one is.

/// Which way a block is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
	/// Up the diagonals from the top left, which is what all but the smallest blocks use.
	Diagonal,
	/// Along the rows, for a block predicted from nearly vertical.
	Horizontal,
	/// Down the columns, for one predicted from nearly horizontal.
	Vertical,
}

impl Order {

	/// Which order a block of this size and prediction mode is read in (§7.4.9.11).
	///
	/// Only the two smallest luma sizes, and the smallest chroma one, choose by mode at all;
	/// everything larger is read up the diagonals whatever it was predicted from.
	pub fn of(log2_size: u32, chroma: bool, mode: u8) -> Self {
		let chooses = log2_size == 2 || (log2_size == 3 && !chroma);
		if !chooses {
			return Self::Diagonal;
		}
		match mode {
			6..=14	=> Self::Vertical,
			22..=30	=> Self::Horizontal,
			_	=> Self::Diagonal,
		}
	}
}

/// The scan positions of a square of `size`, in the given order.
///
/// Position `i` of the result is where the `i`th coefficient in coding order sits, as `(x, y)`.
/// Sizes are 1, 2, 4 and 8 for the sub-block grid and always 4 for the coefficients within one.
pub fn positions(size: usize, order: Order) -> Vec<(u8, u8)> {
	let mut out = Vec::with_capacity(size * size);
	match order {
		Order::Horizontal => for y in 0..size {
			for x in 0..size {
				out.push((x as u8, y as u8));
			}
		},
		Order::Vertical => for x in 0..size {
			for y in 0..size {
				out.push((x as u8, y as u8));
			}
		},
		// Up and to the right, starting each diagonal at the bottom left of it: the loop in the
		// specification walks x up and y down, then restarts one row lower.
		Order::Diagonal => {
			let (mut x, mut y) = (0usize, 0usize);
			loop {
				loop {
					if x < size && y < size {
						out.push((x as u8, y as u8));
					}
					if y == 0 {
						break;
					}
					y -= 1;
					x += 1;
				}
				y = x + 1;
				x = 0;
				if out.len() >= size * size {
					break;
				}
			}
		},
	}
	out
}

/// Every scan a picture needs, worked out once.
///
/// Four sizes of sub-block grid -- a four-sample block is one sub-block, a thirty-two-sample block
/// is eight by eight of them -- in three orders each.
#[derive(Clone, Debug)]
pub struct Scans {
	/// Indexed by the base-two logarithm of the grid's side, then by the order.
	grids:	[[Vec<(u8, u8)>; 3]; 4],
}

impl Scans {

	/// Works them all out.
	pub fn new() -> Self {
		let orders = [Order::Diagonal, Order::Horizontal, Order::Vertical];
		let grids = std::array::from_fn(|log2| {
			std::array::from_fn(|o| positions(1 << log2, orders[o]))
		});
		Self { grids }
	}

	/// The scan of a square whose side is `1 << log2`, in `order`.
	pub fn of(&self, log2: u32, order: Order) -> &[(u8, u8)] {
		let o = match order {
			Order::Diagonal		=> 0,
			Order::Horizontal	=> 1,
			Order::Vertical		=> 2,
		};
		&self.grids[(log2 as usize).min(3)][o]
	}
}

impl Default for Scans {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_core::prelude::*;

	#[test]
	fn test_every_scan_visits_every_position_once_00() -> Outcome<()> {
		for size in [1usize, 2, 4, 8] {
			for order in [Order::Diagonal, Order::Horizontal, Order::Vertical] {
				let scan = positions(size, order);
				req!(scan.len(), size * size, "{:?} at {} is the wrong length", order, size);
				let mut seen = vec![false; size * size];
				for (x, y) in &scan {
					let at = *y as usize * size + *x as usize;
					req!(seen[at], false, "{:?} at {} visits ({}, {}) twice", order, size, x, y);
					seen[at] = true;
				}
			}
		}
		Ok(())
	}

	#[test]
	fn test_the_diagonal_goes_up_and_to_the_right_01() -> Outcome<()> {
		// The order a four-by-four block of coefficients is read in, written out. Getting this
		// backwards -- down and to the left, which is the other reasonable guess -- puts every
		// coefficient of every block in the wrong place.
		let scan = positions(4, Order::Diagonal);
		req!(scan[0], (0u8, 0u8));
		req!(scan[1], (0u8, 1u8));
		req!(scan[2], (1u8, 0u8));
		req!(scan[3], (0u8, 2u8));
		req!(scan[4], (1u8, 1u8));
		req!(scan[5], (2u8, 0u8));
		req!(scan[15], (3u8, 3u8));
		Ok(())
	}

	#[test]
	fn test_the_scan_order_follows_the_prediction_02() -> Outcome<()> {
		// A block predicted from nearly horizontal is read down its columns, and one predicted
		// from nearly vertical along its rows -- the opposite of the direction, because that is
		// where the residual's energy lies.
		req!(Order::of(2, false, 10), Order::Vertical, "horizontal prediction");
		req!(Order::of(2, false, 26), Order::Horizontal, "vertical prediction");
		req!(Order::of(2, false, 0), Order::Diagonal, "planar");
		// Only the smallest blocks choose. Eight-sample luma does, eight-sample chroma does not.
		req!(Order::of(3, false, 10), Order::Vertical);
		req!(Order::of(3, true, 10), Order::Diagonal);
		req!(Order::of(4, false, 10), Order::Diagonal);
		Ok(())
	}
}
