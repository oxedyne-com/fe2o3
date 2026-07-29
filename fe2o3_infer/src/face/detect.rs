//! Face detection: the anchor-free decode over three strides, and the
//! suppression that follows it.

use crate::face::Image;
use crate::graph::Graph;
use crate::kern::Cpu;
use crate::tensor::Tensor;

use oxedyne_fe2o3_core::prelude::*;

/// The three strides the detector's heads sit on.
pub const STRIDES: [usize; 3] = [8, 16, 32];

/// The canvas extent must be a multiple of this, because the deepest head is
/// reached by dividing by thirty-two.
pub const DIVISOR: usize = 32;

/// One detected face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
	/// Left edge of the box, in canvas pixels.
	pub x:			f32,
	/// Top edge of the box, in canvas pixels.
	pub y:			f32,
	/// Box width, in canvas pixels.
	pub w:			f32,
	/// Box height, in canvas pixels.
	pub h:			f32,
	/// Right eye, left eye, nose tip, right mouth corner, left mouth corner.
	pub landmarks:	[(f32, f32); 5],
	/// Confidence, the geometric mean of the classification and objectness
	/// heads, in `[0, 1]`.
	pub score:		f32,
}

impl Detection {
	/// Area of the box after truncation to whole pixels, which is what the
	/// suppression works on.
	fn int_box(&self) -> (i64, i64, i64, i64) {
		(self.x as i64, self.y as i64, self.w as i64, self.h as i64)
	}

	/// Maps the box and the landmarks back through a letterbox.
	pub fn unletterbox(&self, lb: &crate::face::Letterbox) -> Self {
		let s = lb.scale as f32;
		let mut d = *self;
		d.x /= s;
		d.y /= s;
		d.w /= s;
		d.h /= s;
		for p in d.landmarks.iter_mut() {
			p.0 /= s;
			p.1 /= s;
		}
		d
	}
}

/// What the decode and the suppression are allowed to keep.
#[derive(Clone, Copy, Debug)]
pub struct DetectorOptions {
	/// Lowest confidence worth reporting.
	pub score_threshold:	f32,
	/// Overlap above which the weaker of two boxes is dropped.
	pub nms_threshold:		f32,
	/// Most candidates carried into the suppression.
	pub top_k:				usize,
}

impl Default for DetectorOptions {
	/// The thresholds the reference implementation ships with.
	fn default() -> Self {
		Self { score_threshold: 0.9, nms_threshold: 0.3, top_k: 5000 }
	}
}

/// A loaded face detector.
#[derive(Clone, Debug)]
pub struct Detector {
	/// The prepared graph.
	graph:	Graph,
	/// Which head each declared output is, as `(stride index, kind)`.
	heads:	Vec<(usize, Head)>,
}

/// Which of the four quantities a head predicts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Head {
	/// Classification score.
	Cls,
	/// Objectness score.
	Obj,
	/// Box offsets and log extents.
	Box,
	/// Five landmark offsets.
	Kps,
}

impl Detector {
	/// Loads a detector from the bytes of an `.onnx` file.
	pub fn load(onnx: &[u8]) -> Outcome<Self> {
		let graph = res!(Graph::load(onnx));
		let mut heads = Vec::with_capacity(graph.outputs.len());
		for v in &graph.outputs {
			let name = &graph.names[*v];
			let (kind, tail) = if let Some(t) = name.strip_prefix("cls_") {
				(Head::Cls, t)
			} else if let Some(t) = name.strip_prefix("obj_") {
				(Head::Obj, t)
			} else if let Some(t) = name.strip_prefix("bbox_") {
				(Head::Box, t)
			} else if let Some(t) = name.strip_prefix("kps_") {
				(Head::Kps, t)
			} else {
				return Err(err!(
					"The graph output {} is not a head this detector knows.", name;
				Invalid, Input, Mismatch));
			};
			let stride = res!(tail.parse::<usize>().map_err(|e| err!(e,
				"The graph output {} does not name a stride.", name; Invalid, Input)));
			let si = some!(STRIDES.iter().position(|s| *s == stride),
				"The graph names a stride this detector does not carry.");
			heads.push((si, kind));
		}
		if heads.len() != STRIDES.len() * 4 {
			return Err(err!(
				"A detector wants {} heads, the graph declares {}.",
				STRIDES.len() * 4, heads.len();
			Invalid, Input, Mismatch));
		}
		Ok(Self { graph, heads })
	}

	/// The prepared graph, for callers that want to time or inspect it.
	pub fn graph(&self) -> &Graph {
		&self.graph
	}

	/// Turns a red-green-blue canvas into the input tensor the detector was
	/// exported against, which is blue-green-red and unnormalised.
	pub fn input_tensor(img: &Image<'_>) -> Outcome<Tensor> {
		if img.channels != 3 {
			return Err(err!(
				"The detector wants three channels, the image has {}.", img.channels;
			Invalid, Input, Mismatch));
		}
		if img.width % DIVISOR != 0 || img.height % DIVISOR != 0 {
			return Err(err!(
				"The detector wants extents that are multiples of {}, found {} by {}.",
				DIVISOR, img.width, img.height;
			Invalid, Input, Range));
		}
		let mut data = vec![0.0f32; img.width * img.height * 3];
		for p in 0..img.width * img.height {
			data[p * 3] = img.pixels[p * 3 + 2] as f32;
			data[p * 3 + 1] = img.pixels[p * 3 + 1] as f32;
			data[p * 3 + 2] = img.pixels[p * 3] as f32;
		}
		Tensor::new(vec![1, img.height, img.width, 3], data)
	}

	/// Detects faces in a canvas whose extents are multiples of thirty-two.
	pub fn detect(&self, cpu: Cpu, img: &Image<'_>, opts: &DetectorOptions)
		-> Outcome<Vec<Detection>>
	{
		let input = res!(Self::input_tensor(img));
		let outs = res!(self.graph.run(cpu, input));
		self.decode(&outs, img.width, img.height, opts)
	}

	/// Decodes the twelve head outputs into detections and suppresses the
	/// overlapping ones.
	pub fn decode(
		&self,
		outs:	&[Tensor],
		width:	usize,
		height:	usize,
		opts:	&DetectorOptions,
	)
		-> Outcome<Vec<Detection>>
	{
		if outs.len() != self.heads.len() {
			return Err(err!(
				"The graph answered {} outputs, {} were expected.", outs.len(), self.heads.len();
			Invalid, Mismatch));
		}
		let mut cand: Vec<Detection> = Vec::new();
		for (si, stride) in STRIDES.iter().enumerate() {
			let cols = width / stride;
			let rows = height / stride;
			let cls = res!(self.head(outs, si, Head::Cls));
			let obj = res!(self.head(outs, si, Head::Obj));
			let bbox = res!(self.head(outs, si, Head::Box));
			let kps = res!(self.head(outs, si, Head::Kps));
			let n = rows * cols;
			if cls.len() < n || obj.len() < n || bbox.len() < n * 4 || kps.len() < n * 10 {
				return Err(err!(
					"The heads at stride {} hold too few values for a {} by {} grid.",
					stride, rows, cols;
				Invalid, Mismatch));
			}
			for r in 0..rows {
				for c in 0..cols {
					let idx = r * cols + c;
					let cs = cls[idx].clamp(0.0, 1.0);
					let os = obj[idx].clamp(0.0, 1.0);
					let score = (cs * os).sqrt();
					if score < opts.score_threshold {
						continue;
					}
					let sf = *stride as f32;
					let cx = (c as f32 + bbox[idx * 4]) * sf;
					let cy = (r as f32 + bbox[idx * 4 + 1]) * sf;
					let w = bbox[idx * 4 + 2].exp() * sf;
					let h = bbox[idx * 4 + 3].exp() * sf;
					let mut landmarks = [(0.0f32, 0.0f32); 5];
					for (l, lm) in landmarks.iter_mut().enumerate() {
						lm.0 = (kps[idx * 10 + 2 * l] + c as f32) * sf;
						lm.1 = (kps[idx * 10 + 2 * l + 1] + r as f32) * sf;
					}
					cand.push(Detection {
						x: cx - w / 2.0,
						y: cy - h / 2.0,
						w,
						h,
						landmarks,
						score,
					});
				}
			}
		}
		Ok(suppress(cand, opts))
	}

	/// Finds the values of one head at one stride.
	fn head<'t>(&self, outs: &'t [Tensor], si: usize, kind: Head) -> Outcome<&'t [f32]> {
		for (i, (s, k)) in self.heads.iter().enumerate() {
			if *s == si && *k == kind {
				return Ok(&outs[i].data);
			}
		}
		Err(err!("The graph declares no {:?} head at stride {}.", kind, STRIDES[si];
			Invalid, Missing))
	}
}

/// Overlap of two whole-pixel boxes, as intersection over union.
fn iou(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> f32 {
	let x0 = a.0.max(b.0);
	let y0 = a.1.max(b.1);
	let x1 = (a.0 + a.2).min(b.0 + b.2);
	let y1 = (a.1 + a.3).min(b.1 + b.3);
	if x1 <= x0 || y1 <= y0 {
		return 0.0;
	}
	let inter = ((x1 - x0) * (y1 - y0)) as f64;
	let union = (a.2 * a.3) as f64 + (b.2 * b.3) as f64 - inter;
	if union <= 0.0 {
		return 0.0;
	}
	(inter / union) as f32
}

/// Greedy non-maximum suppression, strongest box first.
///
/// The boxes are truncated to whole pixels before the overlap is measured,
/// which is what the reference implementation does and is worth matching,
/// because on small faces the truncation changes which box survives.
fn suppress(mut cand: Vec<Detection>, opts: &DetectorOptions) -> Vec<Detection> {
	if cand.len() <= 1 {
		return cand;
	}
	cand.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
	if opts.top_k > 0 && cand.len() > opts.top_k {
		cand.truncate(opts.top_k);
	}
	let mut kept: Vec<Detection> = Vec::new();
	for d in cand {
		let db = d.int_box();
		let mut keep = true;
		for k in &kept {
			if iou(db, k.int_box()) > opts.nms_threshold {
				keep = false;
				break;
			}
		}
		if keep {
			kept.push(d);
		}
	}
	kept
}

#[cfg(test)]
mod tests {
	use super::*;

	fn det(x: f32, y: f32, w: f32, h: f32, score: f32) -> Detection {
		Detection { x, y, w, h, landmarks: [(0.0, 0.0); 5], score }
	}

	#[test]
	fn identical_boxes_collapse_to_the_stronger() -> Outcome<()> {
		let c = vec![det(10.0, 10.0, 20.0, 20.0, 0.9), det(10.0, 10.0, 20.0, 20.0, 0.95)];
		let k = suppress(c, &DetectorOptions::default());
		req!(k.len(), 1);
		req!(k[0].score, 0.95f32);
		Ok(())
	}

	#[test]
	fn separate_boxes_both_survive() -> Outcome<()> {
		let c = vec![det(0.0, 0.0, 10.0, 10.0, 0.9), det(100.0, 100.0, 10.0, 10.0, 0.91)];
		let k = suppress(c, &DetectorOptions::default());
		req!(k.len(), 2);
		Ok(())
	}

	#[test]
	fn overlap_is_measured_on_whole_pixels() -> Outcome<()> {
		req!(iou((0, 0, 10, 10), (0, 0, 10, 10)), 1.0f32);
		req!(iou((0, 0, 10, 10), (20, 20, 10, 10)), 0.0f32);
		let half = iou((0, 0, 10, 10), (5, 0, 10, 10));
		req!(((half - 1.0 / 3.0).abs() < 1e-6), true);
		Ok(())
	}
}
