//! Category detection: what is in a photograph, from a fixed list of everyday
//! things.
//!
//! This is the second detector the crate carries and it answers a different
//! question from the first. The face detector finds one kind of thing and says
//! where its eyes are; this one finds eighty kinds and says only where each is.
//!
//! # What the network hands back, and what has to be done with it
//!
//! Three heads, at strides of eight, sixteen and thirty-two, each giving a class
//! score per position and a box per position. The box is not four numbers. It is
//! four *distributions* -- one per side, over eight bins -- and the distance to
//! that side is their mean, which is the arrangement a generalised focal loss
//! trains. So each side is a softmax and then a dot product against `0..8`,
//! multiplied by the stride, and the box is that distance out from the position
//! rather than a corner in its own right.
//!
//! The positions are not at the centres of their cells. They sit at
//! `i·stride + (stride − 1)/2`, which is half a sample short of the centre, and
//! every reference implementation of this model does the same. Taking the centre
//! instead moves every box by up to fifteen pixels at the coarsest head.
//!
//! # Channel order and normalisation
//!
//! The network was exported against blue-green-red input, and against the
//! ImageNet statistics in that order. [`Detector::input_tensor`] takes ordinary
//! red-green-blue pixels and puts them the way the network was trained, so a
//! caller never has to know, exactly as the face detector does.

use crate::face::{Image, Letterbox};
use crate::graph::Graph;
use crate::kern::Cpu;
use crate::tensor::Tensor;

use oxedyne_fe2o3_core::prelude::*;

/// The strides the three heads predict at.
pub const STRIDES: [usize; 3] = [8, 16, 32];

/// Bins in each side's distribution.
pub const BINS: usize = 8;

/// Sides of a box, in the order left, top, right, bottom.
pub const SIDES: usize = 4;

/// Categories the network was trained on.
pub const CATEGORIES: usize = 80;

/// The canvas the network wants, in pixels each way.
pub const SIDE: usize = 416;

/// The eighty categories, in the order the network scores them.
pub const NAMES: [&str; CATEGORIES] = [
	"person", "bicycle", "car", "motorcycle", "airplane", "bus", "train",
	"truck", "boat", "traffic light", "fire hydrant", "stop sign",
	"parking meter", "bench", "bird", "cat", "dog", "horse", "sheep", "cow",
	"elephant", "bear", "zebra", "giraffe", "backpack", "umbrella", "handbag",
	"tie", "suitcase", "frisbee", "skis", "snowboard", "sports ball", "kite",
	"baseball bat", "baseball glove", "skateboard", "surfboard",
	"tennis racket", "bottle", "wine glass", "cup", "fork", "knife", "spoon",
	"bowl", "banana", "apple", "sandwich", "orange", "broccoli", "carrot",
	"hot dog", "pizza", "donut", "cake", "chair", "couch", "potted plant",
	"bed", "dining table", "toilet", "tv", "laptop", "mouse", "remote",
	"keyboard", "cell phone", "microwave", "oven", "toaster", "sink",
	"refrigerator", "book", "clock", "vase", "scissors", "teddy bear",
	"hair drier", "toothbrush",
];

/// The categories that are animals, by index into [`NAMES`].
///
/// Every four-legged and winged thing the list carries, wild ones included: a
/// caller after somebody's pets wants the whole set, because a network that has
/// to choose between `dog` and `bear` for a large dark animal is answering a
/// question nobody asked.
pub const ANIMALS: [usize; 10] = [14, 15, 16, 17, 18, 19, 20, 21, 22, 23];

/// Whether a category is one of the animals.
pub fn is_animal(class: usize) -> bool {
	ANIMALS.contains(&class)
}

/// One detected thing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Object {
	/// Index into [`NAMES`].
	pub class:	usize,
	/// Confidence in `[0, 1]`.
	pub score:	f32,
	/// Left edge of the box, in canvas pixels.
	pub x:		f32,
	/// Top edge, in canvas pixels.
	pub y:		f32,
	/// Width, in canvas pixels.
	pub w:		f32,
	/// Height, in canvas pixels.
	pub h:		f32,
}

impl Object {
	/// The name of the category.
	pub fn name(&self) -> &'static str {
		NAMES.get(self.class).copied().unwrap_or("?")
	}

	/// Whether this is one of the animals.
	pub fn is_animal(&self) -> bool {
		is_animal(self.class)
	}

	/// Area of the box after truncation to whole pixels, which is what the
	/// suppression works on.
	fn int_box(&self) -> (i64, i64, i64, i64) {
		(self.x as i64, self.y as i64, self.w as i64, self.h as i64)
	}

	/// Maps the box back through a letterbox, into the original frame.
	pub fn unletterbox(&self, lb: &Letterbox) -> Self {
		let s = lb.scale as f32;
		let mut o = *self;
		o.x /= s;
		o.y /= s;
		o.w /= s;
		o.h /= s;
		o
	}
}

/// What the decode and the suppression are allowed to keep.
#[derive(Clone, Copy, Debug)]
pub struct Options {
	/// Lowest confidence worth reporting.
	pub score_threshold:	f32,
	/// Overlap above which the weaker of two boxes is dropped.
	pub nms_threshold:		f32,
	/// Most candidates carried out of one head, before the threshold.
	pub pre_k:				usize,
	/// Most candidates carried into the suppression.
	pub top_k:				usize,
}

impl Default for Options {
	/// The thresholds the reference implementation ships with.
	///
	/// They are the reference's and not a recommendation: on a real photograph
	/// library a score of `0.35` admits far more than it should, and a caller
	/// after a usable answer should raise it.
	fn default() -> Self {
		Self {
			score_threshold:	0.35,
			nms_threshold:		0.6,
			pre_k:				1000,
			top_k:				0,
		}
	}
}

/// A loaded category detector.
#[derive(Clone, Debug)]
pub struct Detector {
	/// The prepared graph.
	graph:	Graph,
}

impl Detector {
	/// Reads a model from the bytes of an `.onnx` file.
	pub fn load(onnx: &[u8]) -> Outcome<Self> {
		let graph = res!(Graph::load(onnx));
		Ok(Self { graph })
	}

	/// The prepared graph, for a caller that wants to run it itself.
	pub fn graph(&self) -> &Graph {
		&self.graph
	}

	/// Turns a letterboxed canvas into the tensor the network wants.
	///
	/// The image must already be the canvas: square, [`SIDE`] each way, with the
	/// photograph fitted into it. Channels are reversed and the ImageNet
	/// statistics applied in the network's own order.
	pub fn input_tensor(img: &Image<'_>) -> Outcome<Tensor> {
		if img.width != SIDE || img.height != SIDE {
			return Err(err!(
				"The detector wants a canvas of {} by {}, and was given {} by {}.",
				SIDE, SIDE, img.width, img.height;
			Invalid, Input, Mismatch));
		}
		if img.channels < 3 {
			return Err(err!(
				"The detector wants three channels, and was given {}.", img.channels;
			Invalid, Input, Mismatch));
		}
		// Blue, green, red -- the order the network was exported against.
		const MEAN: [f32; 3] = [103.53, 116.28, 123.675];
		const STD: [f32; 3] = [57.375, 57.12, 58.395];
		let n = SIDE * SIDE;
		let mut data = vec![0.0f32; n * 3];
		for p in 0..n {
			let src = p * img.channels;
			for c in 0..3 {
				// Channel `c` of the network is channel `2 - c` of the image.
				let v = img.pixels[src + (2 - c)] as f32;
				data[p * 3 + c] = (v - MEAN[c]) / STD[c];
			}
		}
		Tensor::new(vec![1, SIDE, SIDE, 3], data)
	}

	/// Runs the network over a canvas and answers what it found.
	pub fn detect(&self, cpu: Cpu, img: &Image<'_>, opts: &Options) -> Outcome<Vec<Object>> {
		let input = res!(Self::input_tensor(img));
		let out = res!(self.graph.run(cpu, input));
		decode(&out, opts)
	}
}

/// Turns the network's outputs into boxes.
///
/// Takes the tensors rather than the model, because nothing here needs the
/// weights: a caller holding the outputs from anywhere can decode them, which is
/// what lets the decode be checked against another implementation on its own.
///
/// The heads arrive in whatever order the model declared them, so they are
/// paired by what they are rather than by position: a head with [`CATEGORIES`]
/// values a position is the scores, one with `SIDES · BINS` is the boxes, and
/// the two belonging together have the same number of positions.
pub fn decode(out: &[Tensor], opts: &Options) -> Outcome<Vec<Object>> {
	let mut scores: Vec<(usize, &Tensor)> = Vec::new();
	let mut boxes: Vec<(usize, &Tensor)> = Vec::new();
	for t in out {
		if t.dims.len() != 3 {
			return Err(err!(
				"A head of shape {:?} is not a run of positions.", t.dims;
			Invalid, Input, Mismatch));
		}
		let (points, width) = (t.dims[1], t.dims[2]);
		if width == CATEGORIES {
			scores.push((points, t));
		} else if width == SIDES * BINS {
			boxes.push((points, t));
		} else {
			return Err(err!(
				"A head of {} values a position is neither scores nor boxes.", width;
			Invalid, Input, Mismatch));
		}
	}
	if scores.len() != boxes.len() {
		return Err(err!(
			"The network gave {} score heads and {} box heads.", scores.len(), boxes.len();
		Invalid, Input, Mismatch));
	}

	let mut cand: Vec<Object> = Vec::new();
	for (points, cls) in &scores {
		let bx = match boxes.iter().find(|(p, _)| p == points) {
			Some((_, t)) => *t,
			None => return Err(err!(
				"No box head has the {} positions the scores do.", points;
			Invalid, Input, Mismatch)),
		};
		res!(level(*points, cls, bx, opts, &mut cand));
	}

	Ok(suppress(cand, opts))
}

/// Decodes one head.
fn level(
		points:	usize,
		cls:	&Tensor,
		bx:		&Tensor,
		opts:	&Options,
		out:	&mut Vec<Object>,
	)
		-> Outcome<()>
	{
		// The head is a square grid over the canvas, so its side gives its stride.
		let side = (points as f64).sqrt().round() as usize;
		if side * side != points || side == 0 {
			return Err(err!(
				"A head of {} positions is not a square grid.", points;
			Invalid, Input, Mismatch));
		}
		if SIDE % side != 0 {
			return Err(err!(
				"A grid of {} does not divide a canvas of {}.", side, SIDE;
			Invalid, Input, Mismatch));
		}
		let stride = SIDE / side;
		if !STRIDES.contains(&stride) {
			return Err(err!(
				"A head at stride {} is not one this detector predicts at.", stride;
			Invalid, Input, Unimplemented));
		}

		// The strongest class at each position, and the order to consider them.
		let mut best: Vec<(f32, usize)> = Vec::with_capacity(points);
		for p in 0..points {
			let row = &cls.data[p * CATEGORIES..(p + 1) * CATEGORIES];
			let mut top = (0.0f32, 0usize);
			for (c, v) in row.iter().enumerate() {
				if *v > top.0 {
					top = (*v, c);
				}
			}
			best.push(top);
		}
		let mut order: Vec<usize> = (0..points).collect();
		if opts.pre_k > 0 && points > opts.pre_k {
			// Only the strongest positions are decoded at all, which is what the
			// reference does before it thresholds.
			order.sort_by(|a, b| best[*b].0.partial_cmp(&best[*a].0)
				.unwrap_or(core::cmp::Ordering::Equal));
			order.truncate(opts.pre_k);
		}

		let limit = SIDE as f32;
		for p in order {
			let (score, class) = best[p];
			if score < opts.score_threshold {
				continue;
			}
			// The four sides, each the mean of its own distribution.
			let row = &bx.data[p * SIDES * BINS..(p + 1) * SIDES * BINS];
			let mut d = [0.0f32; SIDES];
			for (s, dist) in d.iter_mut().enumerate() {
				let bins = &row[s * BINS..(s + 1) * BINS];
				let top = bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
				let mut sum = 0.0f32;
				let mut acc = 0.0f32;
				for (i, v) in bins.iter().enumerate() {
					let e = (v - top).exp();
					sum += e;
					acc += e * i as f32;
				}
				*dist = if sum > 0.0 { acc / sum * stride as f32 } else { 0.0 };
			}

			// The position, which is half a sample short of the cell's centre.
			let (gx, gy) = (p % side, p / side);
			let cx = (gx * stride) as f32 + 0.5 * (stride as f32 - 1.0);
			let cy = (gy * stride) as f32 + 0.5 * (stride as f32 - 1.0);
			let x1 = (cx - d[0]).clamp(0.0, limit);
			let y1 = (cy - d[1]).clamp(0.0, limit);
			let x2 = (cx + d[2]).clamp(0.0, limit);
			let y2 = (cy + d[3]).clamp(0.0, limit);
			out.push(Object {
				class,
				score,
				x:	x1,
				y:	y1,
				w:	x2 - x1,
				h:	y2 - y1,
			});
		}
		Ok(())
}

/// Overlap of two boxes, each `(x, y, w, h)` in whole pixels.
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
/// The suppression does not know about categories, which is the reference's
/// behaviour and is right here: two boxes on the same animal, one calling it a
/// dog and the other a cat, are one animal and not two, and keeping both would
/// report the disagreement as a pair of findings.
fn suppress(mut cand: Vec<Object>, opts: &Options) -> Vec<Object> {
	if cand.len() <= 1 {
		return cand;
	}
	cand.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
	if opts.top_k > 0 && cand.len() > opts.top_k {
		cand.truncate(opts.top_k);
	}
	let mut kept: Vec<Object> = Vec::new();
	for o in cand {
		let b = o.int_box();
		if kept.iter().any(|k| iou(k.int_box(), b) > opts.nms_threshold) {
			continue;
		}
		kept.push(o);
	}
	kept
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_animals_are_the_ones_the_names_say_they_are() -> Outcome<()> {
		let want = ["bird", "cat", "dog", "horse", "sheep", "cow", "elephant",
			"bear", "zebra", "giraffe"];
		for (i, name) in ANIMALS.iter().zip(want.iter()) {
			req!(NAMES[*i], *name);
		}
		req!(is_animal(15), true, "A cat is an animal.");
		req!(is_animal(0), false, "A person is not one of the animals here.");
		Ok(())
	}

	#[test]
	fn a_distribution_decodes_to_its_mean() -> Outcome<()> {
		// The coarsest head: a 13 by 13 grid at stride 32. One position carries a
		// dog, and each of its four sides puts all its weight on bin 2, so every
		// distance is 2 x 32 = 64 out from the position.
		let side = 13;
		let points = side * side;
		let stride = SIDE / side;
		let at = 5 * side + 7;                      // grid column 7, row 5

		let mut c = vec![0.0f32; points * CATEGORIES];
		c[at * CATEGORIES + 16] = 0.9;              // dog
		let cls = res!(Tensor::new(vec![1, points, CATEGORIES], c));

		let mut b = vec![-30.0f32; points * SIDES * BINS];
		for s in 0..SIDES {
			b[at * SIDES * BINS + s * BINS + 2] = 30.0;
		}
		let bx = res!(Tensor::new(vec![1, points, SIDES * BINS], b));

		let opts = Options { score_threshold: 0.5, ..Options::default() };
		let mut out = Vec::new();
		res!(level(points, &cls, &bx, &opts, &mut out));

		req!(out.len(), 1, "One position was above the threshold.");
		let o = out[0];
		req!(o.name(), "dog");
		req!(o.is_animal(), true);

		// The position sits half a sample short of the cell's centre, and the box
		// reaches 64 out from it on every side.
		let cx = (7 * stride) as f32 + 0.5 * (stride as f32 - 1.0);
		let cy = (5 * stride) as f32 + 0.5 * (stride as f32 - 1.0);
		let want = 2.0 * stride as f32;
		let left = (o.x - (cx - want)).abs() < 1e-3;
		let top = (o.y - (cy - want)).abs() < 1e-3;
		let wide = (o.w - 2.0 * want).abs() < 1e-3;
		let tall = (o.h - 2.0 * want).abs() < 1e-3;
		req!(left, true, "The left edge is at {}, wanted {}.", o.x, cx - want);
		req!(top, true, "The top edge is at {}, wanted {}.", o.y, cy - want);
		req!(wide, true, "The box is {} wide, wanted {}.", o.w, 2.0 * want);
		req!(tall, true, "The box is {} tall, wanted {}.", o.h, 2.0 * want);

		// Taking the cell's centre instead of the position would move it by half
		// a sample, which is the fault this arithmetic is easiest to get wrong in.
		let centred = (7 * stride) as f32 + 0.5 * stride as f32;
		let apart = (centred - cx).abs() > 1e-3;
		req!(apart, true, "The position and the cell centre are not distinguishable.");
		Ok(())
	}

	#[test]
	fn overlap_is_measured_on_whole_pixels() -> Outcome<()> {
		req!(iou((0, 0, 10, 10), (0, 0, 10, 10)), 1.0f32);
		req!(iou((0, 0, 10, 10), (20, 20, 10, 10)), 0.0f32);
		let half = iou((0, 0, 10, 10), (5, 0, 10, 10));
		let third = (half - 1.0 / 3.0).abs() < 1e-6;
		req!(third, true);
		Ok(())
	}

	#[test]
	fn the_suppression_does_not_care_what_a_box_is_called() -> Outcome<()> {
		// The same animal, called two things. One finding, not two.
		let a = Object { class: 16, score: 0.6, x: 10.0, y: 10.0, w: 50.0, h: 50.0 };
		let b = Object { class: 15, score: 0.5, x: 11.0, y: 11.0, w: 50.0, h: 50.0 };
		let kept = suppress(vec![a, b], &Options::default());
		req!(kept.len(), 1, "Two names for one animal came back as two animals.");
		req!(kept[0].name(), "dog");
		Ok(())
	}
}
