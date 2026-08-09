//! A category detector's graph, held sample for sample against another engine.
//!
//! The face models are checked against `tract` on a synthetic input. This one is
//! checked against OpenCV's own DNN engine on a real photograph, because it
//! exercises the operators `tract` never made this crate need: a pooled window
//! with padding, a bilinear resample under PyTorch's half-pixel rule, a channel
//! split, a concatenation and a channel shuffle. Each of those is written
//! against a channels-last layout while the model names channels-first axes, so
//! agreeing with an independent engine is the only thing that says they are
//! right.
//!
//! The fixture is made by `oracle.py` in the scoping directory, which runs the
//! same `.onnx` through `cv2.dnn` and writes each tensor as raw little-endian
//! `f32` beside a `.shape` file. Point `FE2O3_INFER_DETECTOR` at the model and
//! `FE2O3_INFER_DETECTOR_REF` at the directory of tensors; without both, the
//! test says it was skipped and passes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_infer::graph::Graph;
use oxedyne_fe2o3_infer::kern::Cpu;
use oxedyne_fe2o3_infer::object::{self, Options};
use oxedyne_fe2o3_infer::tensor::Tensor;

/// The model and the reference directory, or `None` so the test can skip.
fn fixture() -> Option<(PathBuf, PathBuf)> {
	let model = env::var("FE2O3_INFER_DETECTOR").ok()?;
	let refs = env::var("FE2O3_INFER_DETECTOR_REF").ok()?;
	if model.is_empty() || refs.is_empty() {
		return None;
	}
	Some((PathBuf::from(model), PathBuf::from(refs)))
}

/// Reads one reference tensor: raw `f32` beside a file naming its shape.
fn reference(dir: &Path, name: &str) -> Outcome<Tensor> {
	let shape = res!(fs::read_to_string(dir.join(format!("{}.shape", name))));
	let mut dims = Vec::new();
	for word in shape.split_whitespace() {
		dims.push(res!(word.parse::<usize>()));
	}
	let raw = res!(fs::read(dir.join(format!("{}.f32", name))));
	if raw.len() % 4 != 0 {
		return Err(err!("The reference {} is {} bytes, not a run of f32.", name, raw.len();
			Invalid, Input, Mismatch));
	}
	let mut data = Vec::with_capacity(raw.len() / 4);
	for chunk in raw.chunks_exact(4) {
		let mut b = [0u8; 4];
		b.copy_from_slice(chunk);
		data.push(f32::from_le_bytes(b));
	}
	Tensor::new(dims, data)
}

/// The outputs the reference holds. Only used to check that every one of the
/// graph's own outputs is covered: the two engines list them in different
/// orders, so they are paired by name and never by position.
fn output_names(dir: &Path) -> Outcome<Vec<String>> {
	let meta = res!(fs::read_to_string(dir.join("meta.txt")));
	for line in meta.lines() {
		if let Some(rest) = line.strip_prefix("outputs ") {
			return Ok(rest.split_whitespace().map(|s| s.to_string()).collect());
		}
	}
	Err(err!("The reference directory names no outputs."; Invalid, Input, Missing))
}

#[test]
fn the_detector_answers_what_another_engine_answered() -> Outcome<()> {
	let (model, root) = match fixture() {
		Some(f) => f,
		None => {
			println!("skipped: set FE2O3_INFER_DETECTOR and FE2O3_INFER_DETECTOR_REF");
			return Ok(());
		},
	};

	let bytes = res!(fs::read(&model));
	let g = res!(Graph::load(&bytes));

	// A directory of tensors, or a directory of directories of them.
	let mut cases = Vec::new();
	if root.join("meta.txt").exists() {
		cases.push(root.clone());
	} else {
		for entry in res!(fs::read_dir(&root)) {
			let path = res!(entry).path();
			if path.join("meta.txt").exists() {
				cases.push(path);
			}
		}
		cases.sort();
	}
	if cases.is_empty() {
		return Err(err!("No reference tensors under {:?}.", root; Invalid, Input, Missing));
	}

	let mut over = 0.0f32;
	let mut over_at = String::new();
	for refs in &cases {
		let (worst, at) = res!(one_case(&g, refs));
		let name = refs.file_name().map(|s| s.to_string_lossy().to_string())
			.unwrap_or_else(|| fmt!("{:?}", refs));
		println!("{:8} worst difference {:e}, at {}", name, worst, at);
		if worst > over {
			over = worst;
			over_at = fmt!("{}: {}", name, at);
		}
	}

	println!("{} photographs, worst of all {:e}, at {}", cases.len(), over, over_at);
	// Two engines summing the same convolution in different orders differ in the
	// last bits and no more. A wrong operator is not a rounding difference: the
	// faults this test exists to catch move a value by whole units, and dropping
	// the channel shuffle or reading the resize by the other coordinate rule was
	// measured at 16.9 and 2.7 against the 5e-6 two right answers differ by.
	let close = over < 2.0e-3;
	req!(close, true, "The detector differs from the reference by {}, at {}.", over, over_at);
	Ok(())
}

/// Runs one photograph and answers the worst disagreement and where it was.
fn one_case(g: &Graph, refs: &Path) -> Outcome<(f32, String)> {
	// The reference input is `[1, 3, 416, 416]`, channels first, as the model
	// declares it; this crate wants it channels last.
	let blob = res!(reference(refs, "input"));
	if blob.dims.len() != 4 {
		return Err(err!("The reference input has shape {:?}.", blob.dims; Invalid, Input));
	}
	let (n, c, h, w) = (blob.dims[0], blob.dims[1], blob.dims[2], blob.dims[3]);
	let mut nhwc = vec![0.0f32; blob.len()];
	for ci in 0..c {
		for p in 0..h * w {
			nhwc[p * c + ci] = blob.data[ci * h * w + p];
		}
	}
	let input = res!(Tensor::new(vec![n, h, w, c], nhwc));

	let got = res!(g.run(Cpu::detect(), input));
	let held = res!(output_names(refs));
	if got.len() != held.len() {
		return Err(err!(
			"The graph answered {} outputs and the reference holds {}.", got.len(), held.len();
		Invalid, Mismatch));
	}
	// Each output is paired with the reference of the same name. The two engines
	// declare the six in different orders, and pairing by position silently
	// compares a box prediction with a class score.
	let names = g.outputs.iter()
		.map(|i| g.names[*i].clone())
		.collect::<Vec<_>>();
	for name in &names {
		if !held.contains(name) {
			return Err(err!(
				"The graph answers an output {} that the reference does not hold.", name;
			Invalid, Mismatch));
		}
	}

	let mut worst = 0.0f32;
	let mut worst_at = String::new();
	for (name, mine) in names.iter().zip(got.iter()) {
		let want = res!(reference(refs, name));
		if mine.len() != want.len() {
			return Err(err!(
				"Output {} has {} values against the reference's {} (shapes {:?} and {:?}).",
				name, mine.len(), want.len(), mine.dims, want.dims;
			Invalid, Mismatch));
		}
		for (i, (a, b)) in mine.data.iter().zip(want.data.iter()).enumerate() {
			let d = (a - b).abs();
			if d > worst {
				worst = d;
				worst_at = fmt!("{} at {} ({} against {})", name, i, a, b);
			}
		}
	}

	// The decode is held to the reference's own boxes, which is a separate claim
	// from the network agreeing: the distribution integral, the anchor positions
	// and the suppression are all this crate's and none of them is exercised by
	// comparing tensors.
	res!(check_detections(refs, &got));

	Ok((worst, worst_at))
}

/// One line of the reference's decoded output.
struct Ref {
	/// Index into the category names.
	class:	usize,
	/// Confidence.
	score:	f32,
	/// Box, `(x, y, w, h)` in canvas pixels.
	rect:	(f32, f32, f32, f32),
}

/// Compares this crate's decode with the reference's, box for box.
fn check_detections(refs: &Path, out: &[Tensor]) -> Outcome<()> {
	let path = refs.join("detections.txt");
	if !path.exists() {
		return Ok(());
	}
	let text = res!(fs::read_to_string(&path));
	let mut want = Vec::new();
	for line in text.lines() {
		let f = line.split('\t').collect::<Vec<_>>();
		if f.len() != 6 {
			return Err(err!("A reference detection has {} fields.", f.len();
				Invalid, Input, Mismatch));
		}
		want.push(Ref {
			class:	res!(f[0].parse::<usize>()),
			score:	res!(f[1].parse::<f32>()),
			rect:	(
				res!(f[2].parse::<f32>()),
				res!(f[3].parse::<f32>()),
				res!(f[4].parse::<f32>()),
				res!(f[5].parse::<f32>()),
			),
		});
	}

	let got = res!(object::decode(out, &Options::default()));
	if got.len() != want.len() {
		let mine = got.iter()
			.map(|o| fmt!("{} {:.2}", o.name(), o.score))
			.collect::<Vec<_>>().join(", ");
		let theirs = want.iter()
			.map(|r| fmt!("{} {:.2}", object::NAMES[r.class], r.score))
			.collect::<Vec<_>>().join(", ");
		return Err(err!(
			"The decode found {} objects and the reference {}. Mine: [{}]. Theirs: [{}].",
			got.len(), want.len(), mine, theirs;
		Invalid, Mismatch));
	}

	// Both are strongest first, so they line up.
	for (i, (mine, theirs)) in got.iter().zip(want.iter()).enumerate() {
		if mine.class != theirs.class {
			return Err(err!(
				"Detection {} is a {} here and a {} in the reference.",
				i, mine.name(), object::NAMES[theirs.class];
			Invalid, Mismatch));
		}
		let ds = (mine.score - theirs.score).abs();
		if ds > 1.0e-5 {
			return Err(err!(
				"Detection {} scores {} here and {} in the reference.",
				i, mine.score, theirs.score;
			Invalid, Mismatch));
		}
		let (x, y, w, h) = theirs.rect;
		let off = (mine.x - x).abs()
			.max((mine.y - y).abs())
			.max((mine.w - w).abs())
			.max((mine.h - h).abs());
		// A quarter of a pixel. The faults worth catching -- the anchor half a
		// sample out, the distribution read the wrong way round -- move an edge
		// by whole pixels at the finest head and by tens at the coarsest.
		if off > 0.25 {
			return Err(err!(
				"Detection {} ({}) is at ({}, {}, {}, {}) here and ({}, {}, {}, {}) in the \
				reference, {} pixels apart.",
				i, mine.name(), mine.x, mine.y, mine.w, mine.h, x, y, w, h, off;
			Invalid, Mismatch));
		}
	}
	Ok(())
}
