//! The image grid: a transform from RGB sampled on a 33-step lattice and interpolated
//! tetrahedrally, so that a bitmap costs one interpolation for each pixel instead of the pipeline.
//!
//! The nodes are the transform's exact result at `i / 32` on each axis, red slowest. Vector colours
//! take the exact path, [`Transform::eval`]; only an image is converted here.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::{
	lut::{
		locate,
		tetra,
	},
	transform::Transform,
};

use oxedyne_fe2o3_core::prelude::*;

pub const STEPS: usize = 33;

#[derive(Debug)]
pub struct Grid {
	nodes: Vec<[f32; 4]>,	// STEPS^3 results, red slowest
}

impl Grid {
	/// Samples the transform at every node.
	pub fn build(t: &Transform) -> Self {
		let mut nodes = Vec::with_capacity(STEPS * STEPS * STEPS);
		let k = (STEPS - 1) as f64;
		for r in 0..STEPS {
			for g in 0..STEPS {
				for b in 0..STEPS {
					let v = t.eval([r as f64 / k, g as f64 / k, b as f64 / k, 0.0]);
					nodes.push([v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]);
				}
			}
		}
		Self { nodes }
	}

	/// The interpolated result for an RGB colour of fractions of full scale.
	pub fn eval(&self, rgb: [f64; 3]) -> [f32; 4] {
		let (r, g, b) = (locate(rgb[0], STEPS), locate(rgb[1], STEPS), locate(rgb[2], STEPS));
		let base = (r.0 * STEPS + g.0) * STEPS + b.0;
		let n = |i: usize| {
			let v = self.nodes[i];
			[v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64]
		};
		let v = tetra(n, base, [STEPS * STEPS, STEPS, 1], [r.1, g.1, b.1]);
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
			self.eval([p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0])
		}).collect())
	}
}
