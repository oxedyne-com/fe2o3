//! Face embedding: an aligned crop in, a unit vector out.

use crate::face::align;
use crate::face::Image;
use crate::graph::Graph;
use crate::kern::Cpu;
use crate::tensor::Tensor;

use oxedyne_fe2o3_core::prelude::*;

/// Length of the vector the embedder answers.
pub const DIM: usize = 128;

/// A face, as a point on the unit sphere.
///
/// Two of these are compared with [`cosine`], which is a dot product because
/// the vector is already normalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Embedding {
	/// The unit vector.
	pub v:	[f32; DIM],
}

impl Embedding {
	/// Normalises a raw network output onto the unit sphere.
	pub fn from_raw(raw: &[f32]) -> Outcome<Self> {
		if raw.len() != DIM {
			return Err(err!(
				"An embedding of {} values was expected, found {}.", DIM, raw.len();
			Invalid, Input, Mismatch));
		}
		let norm = raw.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>().sqrt();
		if norm <= 0.0 {
			return Err(err!("An embedding of zero length cannot be normalised.";
				Invalid, Input, Range));
		}
		let mut v = [0.0f32; DIM];
		for (o, r) in v.iter_mut().zip(raw.iter()) {
			*o = (*r as f64 / norm) as f32;
		}
		Ok(Self { v })
	}
}

/// Cosine similarity of two embeddings, in `[-1, 1]`.
///
/// The reference implementation calls two faces the same person above `0.363`.
/// A clustering threshold should sit higher than a verification one, because a
/// cluster that splits is easy to mend and a cluster that merges is not.
pub fn cosine(a: &Embedding, b: &Embedding) -> f32 {
	let mut s = 0.0f64;
	for i in 0..DIM {
		s += a.v[i] as f64 * b.v[i] as f64;
	}
	s as f32
}

/// A loaded face embedder.
#[derive(Clone, Debug)]
pub struct Embedder {
	/// The prepared graph.
	graph:	Graph,
}

impl Embedder {
	/// Loads an embedder from the bytes of an `.onnx` file.
	pub fn load(onnx: &[u8]) -> Outcome<Self> {
		let graph = res!(Graph::load(onnx));
		if graph.outputs.len() != 1 {
			return Err(err!(
				"An embedder wants one output, the graph declares {}.", graph.outputs.len();
			Invalid, Input, Mismatch));
		}
		Ok(Self { graph })
	}

	/// The prepared graph, for callers that want to time or inspect it.
	pub fn graph(&self) -> &Graph {
		&self.graph
	}

	/// Turns an aligned crop into the input tensor the embedder was exported
	/// against, which is red-green-blue and unnormalised -- the subtraction and
	/// the scaling are the first two operators of the graph itself.
	pub fn input_tensor(crop: &[u8]) -> Outcome<Tensor> {
		let want = align::CROP * align::CROP * 3;
		if crop.len() != want {
			return Err(err!(
				"An aligned crop of {} bytes was expected, found {}.", want, crop.len();
			Invalid, Input, Mismatch));
		}
		let data = crop.iter().map(|v| *v as f32).collect::<Vec<_>>();
		Tensor::new(vec![1, align::CROP, align::CROP, 3], data)
	}

	/// Embeds a crop that has already been warped onto the template.
	pub fn embed_aligned(&self, cpu: Cpu, crop: &[u8]) -> Outcome<Embedding> {
		let input = res!(Self::input_tensor(crop));
		let outs = res!(self.graph.run(cpu, input));
		let raw = some!(outs.first(), "The embedder answered no output.");
		Embedding::from_raw(&raw.data)
	}

	/// Embeds a face out of a photograph, given its five landmarks in that
	/// photograph's own coordinates.
	pub fn embed(&self, cpu: Cpu, img: &Image<'_>, landmarks: &[(f32, f32); 5])
		-> Outcome<Embedding>
	{
		if img.channels != 3 {
			return Err(err!(
				"The embedder wants three channels, the image has {}.", img.channels;
			Invalid, Input, Mismatch));
		}
		let crop = res!(align::align_crop(img, landmarks));
		self.embed_aligned(cpu, &crop)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_vector_normalises_and_matches_itself() -> Outcome<()> {
		let raw = (0..DIM).map(|i| (i as f32) - 63.5).collect::<Vec<_>>();
		let e = res!(Embedding::from_raw(&raw));
		let n = e.v.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>();
		req!(((n - 1.0).abs() < 1e-6), true);
		req!(((cosine(&e, &e) - 1.0).abs() < 1e-5), true);
		Ok(())
	}

	#[test]
	fn an_opposite_vector_scores_minus_one() -> Outcome<()> {
		let raw = (0..DIM).map(|i| (i as f32) - 63.5).collect::<Vec<_>>();
		let neg = raw.iter().map(|v| -*v).collect::<Vec<_>>();
		let a = res!(Embedding::from_raw(&raw));
		let b = res!(Embedding::from_raw(&neg));
		req!(((cosine(&a, &b) + 1.0).abs() < 1e-5), true);
		Ok(())
	}
}
