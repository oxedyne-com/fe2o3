//! The image grid: a transform from RGB sampled on a 33-step lattice and interpolated
//! tetrahedrally, so that a bitmap costs one interpolation for each pixel instead of the pipeline.
//!
//! The nodes are the transform's exact result at 33 levels on each axis, red slowest, spaced evenly
//! in the rooted Lab companding of the channel's intensity. Vector colours take the exact path,
//! [`Transform::eval`]; only an image is converted here.
//!
//! Where the destination is CMYK the nodes hold the colour as Lab, in the encoding of the profile's
//! output table, and the table is read at the interpolated Lab. The table is a lattice of its own,
//! with a crease at every node where the inks turn, so a grid of the finished inks misses by up to
//! 15 levels in the darks; a grid of Lab is smooth and misses by a fraction of a level. LittleCMS,
//! which grids the finished inks, misses its own exact path by up to 14 levels.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::{
	lut::{
		locate,
		tetra,
	},
	read::{
		Curve,
		Lut,
	},
	transform::{
		Kind,
		Transform,
	},
};

use oxedyne_fe2o3_core::prelude::*;

pub const STEPS: usize = 33;

const F0: f64 = 16.0 / 116.0;	// the Lab companding function at zero

// The Lab companding function of a linear intensity, scaled to run from 0 to 1 and then rooted, so
// that a lattice even in it crowds the darks more than L* does. The darks are where the pipeline bends
// fastest, and the root brought the worst error over 20000 colours from 3.4 levels to 0.8.
fn comp(t: f64) -> f64 {
	let f = if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
	((f - F0) / (1.0 - F0)).sqrt()
}

// The linear intensity whose companded value is `w`.
fn uncomp(w: f64) -> f64 {
	let f = F0 + w * w * (1.0 - F0);
	if f > 6.0 / 29.0 { f * f * f } else { (116.0 * f - 16.0) * 27.0 / 24389.0 }
}

// Where an input level lies on the lattice axis: the lattice is even in the companded intensity.
fn place(trc: &Curve, x: f64) -> (usize, f64) {
	locate(comp(trc.eval(x)), STEPS)
}

#[derive(Debug)]
pub struct Grid {
	nodes:	Vec<[f32; 4]>,	// STEPS^3 nodes, red slowest
	tail:	Option<Lut>,	// the output table that reads the nodes, where the destination has one
	trc:	[Curve; 3],		// the source's curves, which place a level on each axis
	at8:	Vec<(usize, f64)>,	// the placing of each 8-bit level on each axis
}

impl Grid {
	/// Samples the transform at every node.
	///
	/// The nodes are even in the rooted Lab companding of each channel's intensity, so that they
	/// lie closest where a colour changes fastest, in the darks.
	pub fn build(t: &Transform) -> Self {
		let trc = match &t.src().kind {
			Kind::Rgb(m)	=> m.trc.clone(),
			_				=> [Curve::Identity, Curve::Identity, Curve::Identity],
		};
		let k = (STEPS - 1) as f64;
		let at: Vec<Vec<f64>> = trc.iter().map(|c| {
			(0..STEPS).map(|i| c.inverse(uncomp(i as f64 / k))).collect()
		}).collect();
		let mut nodes = Vec::with_capacity(STEPS * STEPS * STEPS);
		for r in 0..STEPS {
			for g in 0..STEPS {
				for b in 0..STEPS {
					let v = t.node([at[0][r], at[1][g], at[2][b], 0.0]);
					nodes.push([v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]);
				}
			}
		}
		let at8 = (0..3).flat_map(|c| {
			let trc = &trc;
			(0..256).map(move |i| place(&trc[c], i as f64 / 255.0))
		}).collect();
		Self { nodes, tail: t.tail().cloned(), trc, at8 }
	}

	/// The interpolated result for an RGB colour of fractions of full scale.
	pub fn eval(&self, rgb: [f64; 3]) -> [f32; 4] {
		self.read([place(&self.trc[0], rgb[0]), place(&self.trc[1], rgb[1]), place(&self.trc[2], rgb[2])])
	}

	// The result at a placed colour: the lattice cell and the fractions past it on each axis.
	fn read(&self, p: [(usize, f64); 3]) -> [f32; 4] {
		let (r, g, b) = (p[0], p[1], p[2]);
		let base = (r.0 * STEPS + g.0) * STEPS + b.0;
		let n = |i: usize| {
			let v = self.nodes[i];
			[v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64]
		};
		let v = tetra(n, base, [STEPS * STEPS, STEPS, 1], [r.1, g.1, b.1]);
		let v = match &self.tail {
			Some(t)	=> t.eval(v),
			None	=> v,
		};
		[v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
	}

	/// Converts 8-bit RGB triples; the result holds one colour for each triple.
	pub fn convert8(&self, rgb: &[u8]) -> Outcome<Vec<[f32; 4]>> {
		if rgb.len() % 3 != 0 {
			return Err(err!(
				"{} bytes of 8-bit RGB are not a whole number of pixels.", rgb.len();
				Invalid, Input, Size));
		}
		Ok(rgb.chunks_exact(3).map(|p| {
			self.read([self.at8[p[0] as usize], self.at8[256 + p[1] as usize], self.at8[512 + p[2] as usize]])
		}).collect())
	}
}
