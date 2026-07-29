//! The activation and weight container the graph runner passes between operators.

use oxedyne_fe2o3_core::prelude::*;

/// A dense `f32` tensor with row-major, contiguous data.
///
/// Four-dimensional activations are held as `[N, H, W, C]` -- channels last --
/// which is the layout every kernel in this crate expects. An ONNX model
/// declares its activations as `[N, C, H, W]`, so the loader permutes the
/// weights once and the runner never transposes an activation again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tensor {
	/// Extent along each axis, outermost first.
	pub dims:	Vec<usize>,
	/// Values, row-major over `dims`.
	pub data:	Vec<f32>,
}

impl Tensor {
	/// Creates a tensor from dimensions and values, checking that they agree.
	pub fn new(dims: Vec<usize>, data: Vec<f32>) -> Outcome<Self> {
		let want = dims.iter().product::<usize>();
		if want != data.len() {
			return Err(err!(
				"A tensor of shape {:?} holds {} values, but {} were given.",
				dims, want, data.len();
			Invalid, Input, Mismatch));
		}
		Ok(Self { dims, data })
	}

	/// Creates a zeroed tensor of the given shape.
	pub fn zeros(dims: Vec<usize>) -> Self {
		let n = dims.iter().product::<usize>();
		Self { dims, data: vec![0.0; n] }
	}

	/// Number of values in the tensor.
	pub fn len(&self) -> usize {
		self.data.len()
	}

	/// Whether the tensor holds no values.
	pub fn is_empty(&self) -> bool {
		self.data.is_empty()
	}

	/// Number of axes.
	pub fn rank(&self) -> usize {
		self.dims.len()
	}

	/// Reads the tensor as a four-dimensional `[N, H, W, C]` activation.
	pub fn nhwc(&self) -> Outcome<(usize, usize, usize, usize)> {
		if self.dims.len() != 4 {
			return Err(err!(
				"An activation of rank 4 was expected, found shape {:?}.", self.dims;
			Invalid, Input, Mismatch));
		}
		Ok((self.dims[0], self.dims[1], self.dims[2], self.dims[3]))
	}

	/// Rewrites the shape, keeping the values, and checking the element count.
	pub fn reshape(&mut self, dims: Vec<usize>) -> Outcome<()> {
		let want = dims.iter().product::<usize>();
		if want != self.data.len() {
			return Err(err!(
				"A reshape to {:?} wants {} values, but the tensor holds {}.",
				dims, want, self.data.len();
			Invalid, Input, Mismatch));
		}
		self.dims = dims;
		Ok(())
	}

	/// Converts an `[N, C, H, W]` tensor to the `[N, H, W, C]` layout the
	/// kernels use.
	pub fn nchw_to_nhwc(&self) -> Outcome<Self> {
		if self.dims.len() != 4 {
			return Err(err!(
				"A tensor of rank 4 was expected, found shape {:?}.", self.dims;
			Invalid, Input, Mismatch));
		}
		let (n, c, h, w) = (self.dims[0], self.dims[1], self.dims[2], self.dims[3]);
		let mut out = vec![0.0f32; self.data.len()];
		for bi in 0..n {
			for ci in 0..c {
				let src = (bi * c + ci) * h * w;
				for p in 0..h * w {
					out[(bi * h * w + p) * c + ci] = self.data[src + p];
				}
			}
		}
		Ok(Self { dims: vec![n, h, w, c], data: out })
	}

	/// Converts an `[N, H, W, C]` tensor back to the `[N, C, H, W]` layout an
	/// ONNX graph declares, which is what an external comparison wants.
	pub fn nhwc_to_nchw(&self) -> Outcome<Self> {
		if self.dims.len() != 4 {
			return Err(err!(
				"A tensor of rank 4 was expected, found shape {:?}.", self.dims;
			Invalid, Input, Mismatch));
		}
		let (n, h, w, c) = (self.dims[0], self.dims[1], self.dims[2], self.dims[3]);
		let mut out = vec![0.0f32; self.data.len()];
		for bi in 0..n {
			for ci in 0..c {
				let dst = (bi * c + ci) * h * w;
				for p in 0..h * w {
					out[dst + p] = self.data[(bi * h * w + p) * c + ci];
				}
			}
		}
		Ok(Self { dims: vec![n, c, h, w], data: out })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn layout_round_trip() -> Outcome<()> {
		let t = res!(Tensor::new(
			vec![1, 2, 2, 3],
			(0..12).map(|v| v as f32).collect(),
		));
		let nhwc = res!(t.nchw_to_nhwc());
		req!(nhwc.dims, vec![1, 2, 3, 2]);
		let back = res!(nhwc.nhwc_to_nchw());
		req!(back, t);
		Ok(())
	}
}
