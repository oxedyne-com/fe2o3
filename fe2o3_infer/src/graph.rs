//! The operator set, the weight preparation that happens once at load, and the
//! runner that walks a prepared graph.
//!
//! # What preparation does
//!
//! A model as exported is not the shape a kernel wants. Loading therefore does
//! four things, all once:
//!
//! - permutes every convolution weight from `[O, I, kh, kw]` into the
//!   `[kh·kw·I, O]` matrix a channels-last product consumes;
//! - turns each `BatchNormalization` into a per-channel affine map, and folds
//!   that map into the convolution or matrix product in front of it wherever
//!   the intermediate value has no other reader;
//! - drops the operators that are identities at inference -- dropout, and the
//!   transpose-and-reshape pair a detection head uses to relabel its output,
//!   which costs nothing once the activation is already channels-last;
//! - resolves shapes that are written as an initialiser, such as a reshape
//!   target or a resize scale.
//!
//! What survives is a flat list of layers over a flat list of values.

use crate::kern::{
	self,
	Coord,
	Cpu,
	Sample,
	Scratch,
	Task,
};
use crate::onnx;
use crate::tensor::Tensor;

use oxedyne_fe2o3_core::prelude::*;

/// Everything a convolution needs, with its weights already permuted.
#[derive(Clone, Debug)]
pub struct Conv {
	/// Output channels.
	pub oc:		usize,
	/// Input channels. For a depthwise convolution this equals `oc`.
	pub ic:		usize,
	/// Kernel height.
	pub kh:		usize,
	/// Kernel width.
	pub kw:		usize,
	/// Vertical stride.
	pub sy:		usize,
	/// Horizontal stride.
	pub sx:		usize,
	/// Padding above.
	pub pt:		usize,
	/// Padding to the left.
	pub pl:		usize,
	/// Padding below.
	pub pb:		usize,
	/// Padding to the right.
	pub pr:		usize,
	/// Weights: `[kh·kw·ic, oc]` when dense, `[kh·kw, oc]` when depthwise.
	pub weight:	Vec<f32>,
	/// Per-output-channel bias, zero when the model gave none.
	pub bias:	Vec<f32>,
}

/// Everything a maximum pool needs.
#[derive(Clone, Copy, Debug)]
pub struct Pool {
	/// Kernel height.
	pub kh:	usize,
	/// Kernel width.
	pub kw:	usize,
	/// Vertical stride.
	pub sy:	usize,
	/// Horizontal stride.
	pub sx:	usize,
	/// Padding above.
	pub pt:	usize,
	/// Padding to the left.
	pub pl:	usize,
	/// Padding below.
	pub pb:	usize,
	/// Padding to the right.
	pub pr:	usize,
}

impl Pool {
	/// The extent one axis pools down to.
	pub fn out(&self, n: usize, k: usize, s: usize, before: usize, after: usize)
		-> Outcome<usize>
	{
		let padded = n + before + after;
		if padded < k {
			return Err(err!(
				"A pool of kernel {} wants at least that many samples, found {}.", k, padded;
			Invalid, Input, Mismatch));
		}
		Ok((padded - k) / s + 1)
	}
}

/// How large a resize makes its result.
///
/// Both forms occur in the models this crate carries: a face detector writes a
/// scale, and a category detector writes the size it wants outright.
#[derive(Clone, Copy, Debug)]
pub enum Extent {
	/// Multiply the height and the width.
	Scale(f32, f32),
	/// A fixed height and width.
	Size(usize, usize),
}

impl Extent {
	/// The output extents, given the input's.
	pub fn applied(&self, h: usize, w: usize) -> Outcome<(usize, usize)> {
		let (oh, ow) = match self {
			Self::Scale(sy, sx)	=> (
				(h as f32 * sy).round() as usize,
				(w as f32 * sx).round() as usize,
			),
			Self::Size(oh, ow)	=> (*oh, *ow),
		};
		if oh == 0 || ow == 0 {
			return Err(err!(
				"A resize of {} by {} to {} by {} has an empty axis.", h, w, oh, ow;
			Invalid, Input, Range));
		}
		Ok((oh, ow))
	}
}

/// One prepared operator.
#[derive(Clone, Debug)]
pub enum Op {
	/// Dense convolution over every input channel.
	Conv(Conv),
	/// Depthwise convolution, one weight plane per channel.
	Depthwise(Conv),
	/// Per-channel affine map, which is what a batch normalisation becomes.
	Scale {
		/// Per-channel multiplier.
		scale:	Vec<f32>,
		/// Per-channel offset.
		bias:	Vec<f32>,
	},
	/// Parametric rectified linear unit with a per-channel slope.
	PRelu {
		/// Per-channel negative slope.
		slope:	Vec<f32>,
	},
	/// Rectified linear unit.
	Relu,
	/// Logistic sigmoid.
	Sigmoid,
	/// Maximum pool over any kernel, stride and padding.
	MaxPool(Pool),
	/// Resampling of the two spatial axes, up or down.
	Resize {
		/// How large the result is.
		extent:	Extent,
		/// How a value is taken.
		sample:	Sample,
		/// How an output position maps back into the source.
		coord:	Coord,
	},
	/// Element-wise sum of two activations of the same shape.
	Add,
	/// Subtracts a scalar from every value.
	SubScalar(f32),
	/// Multiplies every value by a scalar.
	MulScalar(f32),
	/// Flattens a channels-last activation into the channels-first row a
	/// matrix product downstream was trained against.
	Flatten,
	/// Matrix--vector product against weights stored `[n, k]`.
	MatVecT {
		/// Number of outputs.
		n:		usize,
		/// Length of each dot product.
		k:		usize,
		/// Weights, `[n, k]`.
		weight:	Vec<f32>,
		/// Per-output bias.
		bias:	Vec<f32>,
	},
	/// Passes the value through unchanged.
	Identity,
	/// Rewrites the shape without moving a value.
	Reshape(Vec<i64>),
}

/// One layer: an operator and the values it reads and writes.
#[derive(Clone, Debug)]
pub struct Layer {
	/// What the layer computes.
	pub op:			Op,
	/// Indices of the values it reads.
	pub inputs:		Vec<usize>,
	/// Indices of the values it writes.
	pub outputs:	Vec<usize>,
}

/// A prepared graph, ready to run.
#[derive(Clone, Debug)]
pub struct Graph {
	/// Layers, in execution order.
	pub layers:		Vec<Layer>,
	/// Name of each value, for error messages.
	pub names:		Vec<String>,
	/// Index of the value the caller supplies.
	pub input:		usize,
	/// Indices of the values the caller wants back, in declared order.
	pub outputs:	Vec<usize>,
}

/// Interns tensor names as indices, so the runner needs no map.
#[derive(Default)]
struct Names {
	/// Name of each value.
	names:	Vec<String>,
}

impl Names {
	/// Answers the index of a name, adding it if it is new.
	fn intern(&mut self, name: &str) -> usize {
		match self.names.iter().position(|n| n == name) {
			Some(i) => i,
			None => {
				self.names.push(name.to_string());
				self.names.len() - 1
			},
		}
	}
}

/// Reads a padding attribute, which ONNX writes as `[top, left, bottom, right]`.
fn pads(node: &onnx::Node) -> Outcome<(usize, usize, usize, usize)> {
	match node.attr("pads") {
		None => Ok((0, 0, 0, 0)),
		Some(a) => {
			let v = res!(a.ints());
			if v.len() != 4 {
				return Err(err!(
					"A two-dimensional convolution wants four padding values, found {}.", v.len();
				Invalid, Input, Mismatch));
			}
			for p in v {
				if *p < 0 {
					return Err(err!("A negative padding of {} is not supported.", p;
						Invalid, Input, Unimplemented));
				}
			}
			Ok((v[0] as usize, v[1] as usize, v[2] as usize, v[3] as usize))
		},
	}
}

/// Reads a stride or dilation attribute, defaulting to one on each axis.
fn pair(node: &onnx::Node, name: &str, default: usize) -> Outcome<(usize, usize)> {
	match node.attr(name) {
		None => Ok((default, default)),
		Some(a) => {
			let v = res!(a.ints());
			if v.len() != 2 {
				return Err(err!(
					"The {} attribute wants two values, found {}.", name, v.len();
				Invalid, Input, Mismatch));
			}
			if v[0] < 1 || v[1] < 1 {
				return Err(err!("The {} attribute holds {:?}, which is not positive.", name, v;
					Invalid, Input, Range));
			}
			Ok((v[0] as usize, v[1] as usize))
		},
	}
}

impl Graph {
	/// Reads and prepares a model from the bytes of an `.onnx` file.
	pub fn load(bytes: &[u8]) -> Outcome<Self> {
		let m = res!(onnx::Model::read(bytes));
		Self::prepare(&m)
	}

	/// Turns a read model into a prepared graph.
	pub fn prepare(m: &onnx::Model) -> Outcome<Self> {
		let mut names = Names::default();
		let mut layers: Vec<Layer> = Vec::with_capacity(m.nodes.len());

		for node in &m.nodes {
			let op = match node.op.as_str() {
				"Conv"					=> res!(load_conv(m, node)),
				"BatchNormalization"	=> res!(load_batch_norm(m, node)),
				"PRelu"					=> res!(load_prelu(m, node)),
				"Gemm"					=> res!(load_gemm(m, node)),
				"Relu"					=> Op::Relu,
				"Sigmoid"				=> Op::Sigmoid,
				"MaxPool"				=> res!(load_max_pool(node)),
				"Resize" | "Upsample"	=> res!(load_resize(m, node)),
				"Add"					=> Op::Add,
				"Sub"					=> Op::SubScalar(res!(scalar_operand(m, node))),
				"Mul"					=> Op::MulScalar(res!(scalar_operand(m, node))),
				"Flatten"				=> Op::Flatten,
				"Dropout"				=> Op::Identity,
				"Transpose"				=> res!(load_transpose(node)),
				"Reshape"				=> res!(load_reshape(m, node)),
				other => return Err(err!(
					"The operator {} is outside the subset this crate carries.", other;
				Invalid, Input, Unimplemented)),
			};
			// Only the operands that are values, not the ones that were
			// initialisers folded into the operator above.
			let inputs = node.inputs.iter()
				.filter(|n| m.init(n).is_none() && !n.is_empty())
				.map(|n| names.intern(n))
				.collect::<Vec<_>>();
			let outputs = node.outputs.iter()
				.map(|n| names.intern(n))
				.collect::<Vec<_>>();
			layers.push(Layer { op, inputs, outputs });
		}

		// The value the caller supplies is the first declared input with no
		// initialiser shadowing it. A model exported from some frameworks lists
		// every weight as an input as well.
		let mut input = None;
		for n in &m.inputs {
			if m.init(n).is_none() {
				input = Some(names.intern(n));
				break;
			}
		}
		let input = match input {
			Some(i) => i,
			None => return Err(err!(
				"The graph declares no input that is not also an initialiser.";
			Invalid, Input, Missing)),
		};
		let outputs = m.outputs.iter().map(|n| names.intern(n)).collect::<Vec<_>>();

		let mut g = Self { layers, names: names.names, input, outputs };
		res!(g.fold_scales());
		Ok(g)
	}

	/// Folds each per-channel affine map into the operator that produced its
	/// input, wherever that value has no other reader.
	fn fold_scales(&mut self) -> Outcome<()> {
		let mut i = 1;
		while i < self.layers.len() {
			let foldable = match (&self.layers[i - 1].op, &self.layers[i].op) {
				(Op::Conv(_), Op::Scale { .. })
				| (Op::Depthwise(_), Op::Scale { .. })
				| (Op::MatVecT { .. }, Op::Scale { .. }) => {
					let produced = self.layers[i - 1].outputs.first().copied();
					let consumed = self.layers[i].inputs.first().copied();
					produced.is_some()
						&& produced == consumed
						&& self.readers(produced.unwrap_or(usize::MAX)) == 1
						&& !self.outputs.contains(&produced.unwrap_or(usize::MAX))
				},
				_ => false,
			};
			if !foldable {
				i += 1;
				continue;
			}
			let (scale, bias) = match &self.layers[i].op {
				Op::Scale { scale, bias } => (scale.clone(), bias.clone()),
				_ => return Err(err!("A scale layer changed shape while folding.";
					Bug, Invalid)),
			};
			let out = self.layers[i].outputs.clone();
			match &mut self.layers[i - 1].op {
				Op::Conv(c) | Op::Depthwise(c) => {
					if scale.len() != c.oc {
						return Err(err!(
							"A scale of {} channels cannot fold into a convolution of {}.",
							scale.len(), c.oc;
						Invalid, Input, Mismatch));
					}
					let rows = c.weight.len() / c.oc;
					for r in 0..rows {
						for o in 0..c.oc {
							c.weight[r * c.oc + o] *= scale[o];
						}
					}
					for o in 0..c.oc {
						c.bias[o] = c.bias[o] * scale[o] + bias[o];
					}
				},
				Op::MatVecT { n, k, weight, bias: b } => {
					if scale.len() != *n {
						return Err(err!(
							"A scale of {} channels cannot fold into a product of {} outputs.",
							scale.len(), n;
						Invalid, Input, Mismatch));
					}
					for o in 0..*n {
						for p in 0..*k {
							weight[o * *k + p] *= scale[o];
						}
						b[o] = b[o] * scale[o] + bias[o];
					}
				},
				_ => return Err(err!("A layer changed shape while folding."; Bug, Invalid)),
			}
			self.layers[i - 1].outputs = out;
			self.layers.remove(i);
		}
		Ok(())
	}

	/// Counts the layers that read a value.
	fn readers(&self, value: usize) -> usize {
		self.layers.iter().filter(|l| l.inputs.contains(&value)).count()
	}

	/// Runs the graph over one input, answering the declared outputs in order.
	///
	/// The input is channels-last, `[N, H, W, C]`, and so is every activation
	/// the runner passes between layers.
	pub fn run(&self, cpu: Cpu, input: Tensor) -> Outcome<Vec<Tensor>> {
		let mut left: Vec<usize> = vec![0; self.names.len()];
		for l in &self.layers {
			for v in &l.inputs {
				left[*v] += 1;
			}
		}
		// A declared output must survive to the end.
		for v in &self.outputs {
			left[*v] += 1;
		}
		let mut vals: Vec<Option<Tensor>> = vec![None; self.names.len()];
		vals[self.input] = Some(input);
		let mut scratch = Scratch::new();

		for (li, l) in self.layers.iter().enumerate() {
			let out = res!(self.step(cpu, l, &mut vals, &mut left, &mut scratch)
				.map_err(|e| err!(e,
					"Layer {} ({}) failed.", li, self.names.get(
						l.outputs.first().copied().unwrap_or(0)).map(|s| s.as_str())
						.unwrap_or("?");
				Invalid)));
			for (slot, t) in l.outputs.iter().zip(out.into_iter()) {
				vals[*slot] = Some(t);
			}
		}

		let mut answer = Vec::with_capacity(self.outputs.len());
		for v in &self.outputs {
			match vals[*v].take() {
				Some(t) => answer.push(t),
				None => return Err(err!(
					"The graph output {} was never produced.", self.names[*v];
				Invalid, Missing)),
			}
		}
		Ok(answer)
	}

	/// Runs one layer.
	fn step(
		&self,
		cpu:		Cpu,
		l:			&Layer,
		vals:		&mut Vec<Option<Tensor>>,
		left:		&mut Vec<usize>,
		scratch:	&mut Scratch,
	)
		-> Outcome<Vec<Tensor>>
	{
		let x = res!(take(vals, left, l.inputs.first().copied(), &self.names));
		let out = match &l.op {
			Op::Conv(c)			=> res!(run_conv(cpu, c, &x, scratch, false)),
			Op::Depthwise(c)	=> res!(run_conv(cpu, c, &x, scratch, true)),
			Op::Scale { scale, bias } => {
				let mut x = x;
				let ch = *some!(x.dims.last(), "An activation with no axes cannot be scaled.");
				kern::run(cpu, Task::Scale { ch, x: &mut x.data, scale, bias });
				x
			},
			Op::PRelu { slope } => {
				let mut x = x;
				let ch = *some!(x.dims.last(), "An activation with no axes has no channels.");
				kern::run(cpu, Task::PRelu { ch, x: &mut x.data, slope });
				x
			},
			Op::Relu => {
				let mut x = x;
				kern::run(cpu, Task::Relu { x: &mut x.data });
				x
			},
			Op::Sigmoid => {
				let mut x = x;
				kern::run(cpu, Task::Sigmoid { x: &mut x.data });
				x
			},
			Op::MaxPool(p) => {
				let (n, h, w, ch) = res!(x.nhwc());
				let oh = res!(p.out(h, p.kh, p.sy, p.pt, p.pb));
				let ow = res!(p.out(w, p.kw, p.sx, p.pl, p.pr));
				let mut y = Tensor::zeros(vec![n, oh, ow, ch]);
				kern::run(cpu, Task::MaxPool {
					ch, h, w,
					kh: p.kh, kw: p.kw, sy: p.sy, sx: p.sx, pt: p.pt, pl: p.pl,
					oh, ow,
					x: &x.data, y: &mut y.data,
				});
				y
			},
			Op::Resize { extent, sample, coord } => {
				let (n, h, w, ch) = res!(x.nhwc());
				let (oh, ow) = res!(extent.applied(h, w));
				let mut y = Tensor::zeros(vec![n, oh, ow, ch]);
				kern::run(cpu, Task::Resize {
					ch, h, w, oh, ow,
					sample: *sample, coord: *coord,
					x: &x.data, y: &mut y.data,
				});
				y
			},
			Op::Add => {
				let mut x = x;
				let second = some!(l.inputs.get(1).copied(), "An addition wants two operands.");
				let y = res!(take(vals, left, Some(second), &self.names));
				if y.len() != x.len() {
					return Err(err!(
						"An addition of {:?} and {:?} does not line up.", x.dims, y.dims;
					Invalid, Input, Mismatch));
				}
				kern::run(cpu, Task::Add { x: &mut x.data, y: &y.data });
				x
			},
			Op::SubScalar(v) => {
				let mut x = x;
				for e in x.data.iter_mut() {
					*e -= *v;
				}
				x
			},
			Op::MulScalar(v) => {
				let mut x = x;
				for e in x.data.iter_mut() {
					*e *= *v;
				}
				x
			},
			Op::Flatten			=> res!(kern::flatten_nchw(&x)),
			Op::MatVecT { n, k, weight, bias } => {
				if x.len() != *k {
					return Err(err!(
						"A product of inner extent {} was given {} values.", k, x.len();
					Invalid, Input, Mismatch));
				}
				let mut c = vec![0.0f32; *n];
				kern::run(cpu, Task::MatVecT { n: *n, k: *k, a: &x.data, bt: weight, c: &mut c });
				for (o, b) in c.iter_mut().zip(bias.iter()) {
					*o += *b;
				}
				res!(Tensor::new(vec![1, *n], c))
			},
			Op::Identity		=> x,
			Op::Reshape(spec)	=> {
				let mut x = x;
				let dims = res!(resolve_shape(spec, x.len(), &x.dims));
				res!(x.reshape(dims));
				x
			},
		};
		Ok(vec![out])
	}
}

/// Takes a value out of the pool, cloning only when another layer still wants it.
fn take(
	vals:	&mut Vec<Option<Tensor>>,
	left:	&mut Vec<usize>,
	idx:	Option<usize>,
	names:	&[String],
)
	-> Outcome<Tensor>
{
	let i = some!(idx, "A layer names no input.");
	if left[i] > 0 {
		left[i] -= 1;
	}
	if left[i] == 0 {
		match vals[i].take() {
			Some(t) => Ok(t),
			None => Err(err!("The value {} was read before it was written.", names[i];
				Invalid, Missing)),
		}
	} else {
		match &vals[i] {
			Some(t) => Ok(t.clone()),
			None => Err(err!("The value {} was read before it was written.", names[i];
				Invalid, Missing)),
		}
	}
}

/// Resolves a reshape target, which may name a free axis with minus one and an
/// axis to copy with zero.
fn resolve_shape(spec: &[i64], total: usize, from: &[usize]) -> Outcome<Vec<usize>> {
	let mut dims = Vec::with_capacity(spec.len());
	let mut free = None;
	let mut known = 1usize;
	for (i, v) in spec.iter().enumerate() {
		match *v {
			-1 => {
				if free.is_some() {
					return Err(err!("A reshape names more than one free axis.";
						Invalid, Input));
				}
				free = Some(i);
				dims.push(1);
			},
			0 => {
				let d = *some!(from.get(i), "A reshape copies an axis the input does not have.");
				known *= d;
				dims.push(d);
			},
			n if n > 0 => {
				known *= n as usize;
				dims.push(n as usize);
			},
			n => return Err(err!("A reshape names a negative extent of {}.", n;
				Invalid, Input, Range)),
		}
	}
	match free {
		Some(i) => {
			if known == 0 || total % known != 0 {
				return Err(err!(
					"A reshape of {} values into {:?} does not divide.", total, spec;
				Invalid, Input, Mismatch));
			}
			dims[i] = total / known;
		},
		None => {
			if known != total {
				return Err(err!(
					"A reshape of {} values into {:?} wants {}.", total, spec, known;
				Invalid, Input, Mismatch));
			}
		},
	}
	Ok(dims)
}

/// Reads a convolution node, permuting its weights into the layout the kernels
/// consume.
fn load_conv(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let wname = some!(node.inputs.get(1), "A convolution names no weight.");
	let w = some!(m.init(wname), "The convolution weight is not an initialiser.");
	let wd = w.dims();
	if wd.len() != 4 {
		return Err(err!(
			"A two-dimensional convolution wants a weight of rank 4, found {:?}.", wd;
		Invalid, Input, Mismatch));
	}
	let wv = res!(w.floats());
	let (oc, icg, kh, kw) = (wd[0], wd[1], wd[2], wd[3]);
	let group = match node.attr("group") {
		Some(a) => res!(a.int()) as usize,
		None => 1,
	};
	let (dy, dx) = res!(pair(node, "dilations", 1));
	if dy != 1 || dx != 1 {
		return Err(err!("A dilated convolution is outside the subset this crate carries.";
			Invalid, Input, Unimplemented));
	}
	let (sy, sx) = res!(pair(node, "strides", 1));
	let (pt, pl, pb, pr) = res!(pads(node));
	let bias = match node.inputs.get(2) {
		Some(bn) if !bn.is_empty() => {
			let b = some!(m.init(bn), "The convolution bias is not an initialiser.");
			res!(b.floats()).to_vec()
		},
		_ => vec![0.0; oc],
	};
	if bias.len() != oc {
		return Err(err!(
			"A convolution of {} outputs has a bias of {}.", oc, bias.len();
		Invalid, Input, Mismatch));
	}

	if group == 1 {
		// Dense: `[O, I, kh, kw]` becomes `[kh·kw·I, O]`.
		let ic = icg;
		let mut weight = vec![0.0f32; kh * kw * ic * oc];
		for o in 0..oc {
			for ci in 0..ic {
				for ky in 0..kh {
					for kx in 0..kw {
						let src = ((o * ic + ci) * kh + ky) * kw + kx;
						let dst = ((ky * kw + kx) * ic + ci) * oc + o;
						weight[dst] = wv[src];
					}
				}
			}
		}
		Ok(Op::Conv(Conv { oc, ic, kh, kw, sy, sx, pt, pl, pb, pr, weight, bias }))
	} else if group == oc && icg == 1 {
		// Depthwise: `[C, 1, kh, kw]` becomes `[kh·kw, C]`.
		let mut weight = vec![0.0f32; kh * kw * oc];
		for c in 0..oc {
			for ky in 0..kh {
				for kx in 0..kw {
					weight[(ky * kw + kx) * oc + c] = wv[(c * kh + ky) * kw + kx];
				}
			}
		}
		Ok(Op::Depthwise(Conv {
			oc,
			ic:	oc,
			kh, kw, sy, sx, pt, pl, pb, pr, weight, bias,
		}))
	} else {
		Err(err!(
			"A grouped convolution with {} groups over {} input channels is outside the \
			subset this crate carries.", group, icg * group;
		Invalid, Input, Unimplemented))
	}
}

/// Turns a batch normalisation into the per-channel affine map it is at
/// inference.
fn load_batch_norm(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let eps = match node.attr("epsilon") {
		Some(a) => res!(a.float()),
		None => 1e-5,
	};
	let mut got = Vec::with_capacity(4);
	for i in 1..5 {
		let name = some!(node.inputs.get(i), "A batch normalisation wants four parameters.");
		let init = some!(m.init(name), "A batch normalisation parameter is not an initialiser.");
		got.push(res!(init.floats()));
	}
	let (gamma, beta, mean, var) = (got[0], got[1], got[2], got[3]);
	let c = gamma.len();
	if beta.len() != c || mean.len() != c || var.len() != c {
		return Err(err!(
			"A batch normalisation has parameters of {}, {}, {} and {} channels.",
			c, beta.len(), mean.len(), var.len();
		Invalid, Input, Mismatch));
	}
	let mut scale = vec![0.0f32; c];
	let mut bias = vec![0.0f32; c];
	for i in 0..c {
		let denom = (var[i] + eps).sqrt();
		if denom == 0.0 {
			return Err(err!("A batch normalisation has a zero variance in channel {}.", i;
				Invalid, Input, Range));
		}
		scale[i] = gamma[i] / denom;
		bias[i] = beta[i] - mean[i] * scale[i];
	}
	Ok(Op::Scale { scale, bias })
}

/// Reads a parametric rectifier, whose slope the model stores as `[C, 1, 1]`.
fn load_prelu(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let name = some!(node.inputs.get(1), "A parametric rectifier names no slope.");
	let init = some!(m.init(name), "The rectifier slope is not an initialiser.");
	Ok(Op::PRelu { slope: res!(init.floats()).to_vec() })
}

/// Reads a matrix product. Only the transposed-weight form appears in the
/// models this crate targets, and it is the form a contiguous dot product wants.
fn load_gemm(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let trans_a = match node.attr("transA") {
		Some(a) => res!(a.int()),
		None => 0,
	};
	let trans_b = match node.attr("transB") {
		Some(a) => res!(a.int()),
		None => 0,
	};
	let alpha = match node.attr("alpha") {
		Some(a) => res!(a.float()),
		None => 1.0,
	};
	let beta = match node.attr("beta") {
		Some(a) => res!(a.float()),
		None => 1.0,
	};
	if trans_a != 0 || trans_b != 1 {
		return Err(err!(
			"A matrix product with transA={} and transB={} is outside the subset this \
			crate carries.", trans_a, trans_b;
		Invalid, Input, Unimplemented));
	}
	let wname = some!(node.inputs.get(1), "A matrix product names no weight.");
	let w = some!(m.init(wname), "The matrix product weight is not an initialiser.");
	let wd = w.dims();
	if wd.len() != 2 {
		return Err(err!(
			"A matrix product wants a weight of rank 2, found {:?}.", wd;
		Invalid, Input, Mismatch));
	}
	let (n, k) = (wd[0], wd[1]);
	let mut weight = res!(w.floats()).to_vec();
	if alpha != 1.0 {
		for v in weight.iter_mut() {
			*v *= alpha;
		}
	}
	let mut bias = match node.inputs.get(2) {
		Some(bn) if !bn.is_empty() => {
			let b = some!(m.init(bn), "The matrix product bias is not an initialiser.");
			res!(b.floats()).to_vec()
		},
		_ => vec![0.0; n],
	};
	if beta != 1.0 {
		for v in bias.iter_mut() {
			*v *= beta;
		}
	}
	if bias.len() != n {
		return Err(err!(
			"A matrix product of {} outputs has a bias of {}.", n, bias.len();
		Invalid, Input, Mismatch));
	}
	Ok(Op::MatVecT { n, k, weight, bias })
}

/// Reads a maximum pool, which this crate carries only in its two by two,
/// stride two, unpadded form.
fn load_max_pool(node: &onnx::Node) -> Outcome<Op> {
	let k = res!(res!(node.need("kernel_shape")).ints()).to_vec();
	let (sy, sx) = res!(pair(node, "strides", 1));
	let (pt, pl, pb, pr) = res!(pads(node));
	let ceil = match node.attr("ceil_mode") {
		Some(a) => res!(a.int()),
		None => 0,
	};
	if k.len() != 2 {
		return Err(err!(
			"A two-dimensional maximum pool wants a kernel of two extents, found {:?}.", k;
		Invalid, Input, Mismatch));
	}
	// Rounding the output up rather than down would need the kernel to run off
	// the end of the source, which nothing here does.
	if ceil != 0 {
		return Err(err!(
			"A maximum pool rounding its output up is outside the subset this crate carries.";
		Invalid, Input, Unimplemented));
	}
	Ok(Op::MaxPool(Pool {
		kh: k[0] as usize,
		kw: k[1] as usize,
		sy, sx, pt, pl, pb, pr,
	}))
}

/// Reads a resize of the two spatial axes.
///
/// ONNX writes the target either as a scale or as an outright size, and the
/// convention mapping an output position back into the source as an attribute.
/// All three are read rather than assumed: the two models this crate carries
/// disagree on every one of them, and a half-sample error in the mapping moves
/// every box a detector predicts.
fn load_resize(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let sample = match node.attr("mode") {
		None => Sample::Nearest,
		Some(a) => match res!(a.text()) {
			"nearest"	=> Sample::Nearest,
			"linear"	=> Sample::Bilinear,
			other => return Err(err!(
				"A resize in {} mode is outside the subset this crate carries, which is \
				nearest and linear.", other;
			Invalid, Input, Unimplemented)),
		},
	};
	let coord = match node.attr("coordinate_transformation_mode") {
		None => Coord::Asymmetric,
		Some(a) => match res!(a.text()) {
			"asymmetric"			=> Coord::Asymmetric,
			"pytorch_half_pixel"	=> Coord::HalfPixel,
			// `half_pixel` differs from PyTorch's only for an output of one,
			// which no model here asks for, but it is not silently accepted.
			other => return Err(err!(
				"A resize transforming coordinates by {} is outside the subset this crate \
				carries.", other;
			Invalid, Input, Unimplemented)),
		},
	};

	// The target is the last constant operand: floats are scales, integers are
	// the size outright. An empty initialiser is the `roi` operand, skipped.
	let mut extent: Option<Extent> = None;
	for name in node.inputs.iter().skip(1) {
		let init = match m.init(name) {
			Some(i) => i,
			None => continue,
		};
		if let Ok(v) = init.floats() {
			if v.len() == 4 {
				extent = Some(Extent::Scale(v[2], v[3]));
				continue;
			}
		}
		if let Ok(v) = init.ints() {
			if v.len() == 4 {
				extent = Some(Extent::Size(v[2] as usize, v[3] as usize));
			}
		}
	}
	let extent = some!(extent, "A resize names neither a scale nor a size operand.");
	Ok(Op::Resize { extent, sample, coord })
}

/// Reads the scalar second operand of an element-wise node.
fn scalar_operand(m: &onnx::Model, node: &onnx::Node) -> Outcome<f32> {
	let name = some!(node.inputs.get(1), "An element-wise node names no second operand.");
	let init = some!(m.init(name), "The second operand is not an initialiser.");
	let v = res!(init.floats());
	if v.len() != 1 {
		return Err(err!(
			"An element-wise node with a second operand of {} values is outside the subset \
			this crate carries, which is a scalar.", v.len();
		Invalid, Input, Unimplemented));
	}
	Ok(v[0])
}

/// Reads a transpose. Channels-first to channels-last is what the activations
/// already are, so it costs nothing; anything else would need a real permutation.
fn load_transpose(node: &onnx::Node) -> Outcome<Op> {
	let perm = res!(res!(node.need("perm")).ints()).to_vec();
	if perm == vec![0, 2, 3, 1] {
		Ok(Op::Identity)
	} else {
		Err(err!(
			"A transpose by {:?} is outside the subset this crate carries, which is the \
			channels-first to channels-last relabelling.", perm;
		Invalid, Input, Unimplemented))
	}
}

/// Reads a reshape, whose target the model stores as an initialiser.
fn load_reshape(m: &onnx::Model, node: &onnx::Node) -> Outcome<Op> {
	let name = some!(node.inputs.get(1), "A reshape names no target.");
	let init = some!(m.init(name), "The reshape target is not an initialiser.");
	Ok(Op::Reshape(res!(init.ints()).to_vec()))
}

/// Runs one convolution, gathering patches first when the kernel is larger than
/// one by one.
fn run_conv(
	cpu:		Cpu,
	c:			&Conv,
	x:			&Tensor,
	scratch:	&mut Scratch,
	depthwise:	bool,
)
	-> Outcome<Tensor>
{
	let (n, h, w, ch) = res!(x.nhwc());
	if n != 1 {
		return Err(err!("A batch of {} is outside the subset this crate carries.", n;
			Invalid, Input, Unimplemented));
	}
	if ch != c.ic {
		return Err(err!(
			"A convolution over {} input channels was given an activation of {}.", c.ic, ch;
		Invalid, Input, Mismatch));
	}
	let oh = res!(out_extent(h, c.kh, c.sy, c.pt, c.pb));
	let ow = res!(out_extent(w, c.kw, c.sx, c.pl, c.pr));
	let mut y = Tensor::zeros(vec![1, oh, ow, c.oc]);

	if depthwise {
		kern::run(cpu, Task::Depthwise {
			ch:		c.oc,
			h, w,
			kh:		c.kh,
			kw:		c.kw,
			sy:		c.sy,
			sx:		c.sx,
			pt:		c.pt,
			pl:		c.pl,
			oh, ow,
			x:		&x.data,
			wt:		&c.weight,
			bias:	Some(&c.bias),
			y:		&mut y.data,
		});
		return Ok(y);
	}

	let unit = c.kh == 1 && c.kw == 1 && c.sy == 1 && c.sx == 1
		&& c.pt == 0 && c.pl == 0 && c.pb == 0 && c.pr == 0;
	if unit {
		// In this layout the activation already *is* the patch matrix.
		kern::run(cpu, Task::Gemm {
			m:			oh * ow,
			n:			c.oc,
			k:			c.ic,
			a:			&x.data,
			b:			&c.weight,
			c:			&mut y.data,
			bias:		Some(&c.bias),
			scratch,
		});
		return Ok(y);
	}

	let k = c.kh * c.kw * c.ic;
	let mut patches = vec![0.0f32; oh * ow * k];
	kern::run(cpu, Task::Im2Col {
		ch,
		h, w,
		kh:		c.kh,
		kw:		c.kw,
		sy:		c.sy,
		sx:		c.sx,
		pt:		c.pt,
		pl:		c.pl,
		oh, ow,
		x:		&x.data,
		out:	&mut patches,
	});
	kern::run(cpu, Task::Gemm {
		m:			oh * ow,
		n:			c.oc,
		k,
		a:			&patches,
		b:			&c.weight,
		c:			&mut y.data,
		bias:		Some(&c.bias),
		scratch,
	});
	Ok(y)
}

/// The extent of one output axis of a convolution.
fn out_extent(n: usize, k: usize, stride: usize, before: usize, after: usize)
	-> Outcome<usize>
{
	let padded = n + before + after;
	if padded < k {
		return Err(err!(
			"A kernel of {} does not fit an axis of {} padded to {}.", k, n, padded;
		Invalid, Input, Range));
	}
	Ok((padded - k) / stride + 1)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_free_axis_is_resolved() -> Outcome<()> {
		req!(res!(resolve_shape(&[1, -1, 4], 48, &[1, 12, 4])), vec![1, 12, 4]);
		req!(res!(resolve_shape(&[0, 6], 48, &[8, 6])), vec![8, 6]);
		req!(resolve_shape(&[1, -1, -1], 48, &[1, 12, 4]).is_err(), true);
		req!(resolve_shape(&[5, 5], 48, &[1]).is_err(), true);
		Ok(())
	}

	#[test]
	fn an_output_extent_follows_the_padding() -> Outcome<()> {
		req!(res!(out_extent(640, 3, 2, 1, 1)), 320);
		req!(res!(out_extent(112, 3, 1, 1, 1)), 112);
		req!(res!(out_extent(112, 1, 1, 0, 0)), 112);
		req!(out_extent(1, 3, 1, 0, 0).is_err(), true);
		Ok(())
	}
}
