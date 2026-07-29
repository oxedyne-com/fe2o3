//! Correctness against a reference implementation, and against tract's recorded
//! answer on a fixed input.
//!
//! Two of these tests want the model files. They are not in the repository --
//! weights never are -- so point `FE2O3_INFER_MODELS` at a directory holding
//! `face_detection_yunet_2023mar.onnx` and
//! `face_recognition_sface_2021dec.onnx`, and the tests will run. Without it
//! they report that they were skipped and pass, which is the only sane
//! behaviour for a test whose fixture is thirty-nine megabytes.
//!
//! The recorded vectors below came from tract 0.23.4 running the same ONNX file
//! on the same input, taken once and frozen. A test that only compared this
//! crate against itself would prove nothing; these numbers are what an
//! independent implementation answered.

use std::env;
use std::fs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_infer::face::{detect::Detector, embed::Embedder, Image};
use oxedyne_fe2o3_infer::kern::Cpu;

/// A reproducible byte generator, so the fixed input needs no fixture file.
fn pixels(n: usize, seed: u64) -> Vec<u8> {
	let mut s = seed;
	let mut v = Vec::with_capacity(n);
	for _ in 0..n {
		s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
		v.push((s >> 33) as u8);
	}
	v
}

/// Finds the model directory, or answers `None` so the test can skip.
fn models() -> Option<String> {
	match env::var("FE2O3_INFER_MODELS") {
		Ok(d) if !d.is_empty() => Some(d),
		_ => None,
	}
}

// SFace, recorded from tract 0.23.4 on the fixed input of `pixels(112*112*3, 999)`.
const SFACE_EXPECTED: [f32; 128] = [
	-2.269915e-1, -1.415737e-2, -2.509517e-1, 3.997844e-1,
	-2.476652e-1, 1.763874e-1, 1.363716e-1, -2.750306e-1,
	-8.319645e-1, -9.862685e-2, -1.967272e-1, 2.877449e-1,
	4.880038e-1, -4.574382e-1, 9.873680e-1, -1.919422e-1,
	-6.378446e-1, -5.909452e-1, -1.601325e-1, 3.697520e-1,
	8.097876e-2, -6.924052e-1, -4.088828e-1, -1.624882e-3,
	2.270993e-1, 1.198874e-1, 4.283409e-1, 2.593386e-1,
	-3.672851e-1, 9.240717e-3, -8.834651e-2, -4.717094e-1,
	3.875837e-1, 7.771777e-1, 2.067817e-1, -1.768308e-1,
	-3.115587e-1, -4.025498e-2, 3.658660e-1, 1.752793e-2,
	1.606840e-1, -5.139388e-1, 9.671890e-1, 3.963410e-1,
	-3.993112e-1, -6.236808e-1, 2.314502e-1, 2.565281e-1,
	3.500144e-1, -8.477663e-2, 2.539647e-2, -2.362562e-1,
	4.709096e-1, -3.142870e-1, 6.237337e-1, 3.424331e-1,
	-4.658711e-1, -5.021946e-1, -1.177402e-1, -5.383276e-1,
	-4.104891e-2, 3.315228e-1, -5.957409e-1, -7.728708e-1,
	3.792651e-1, -5.855272e-2, -2.673874e-1, 5.746964e-1,
	3.235634e-1, 2.786989e-1, -6.076752e-1, 3.722823e-1,
	-6.298876e-1, -2.459088e-1, -3.353854e-1, -2.661195e-1,
	2.093293e-1, 2.202750e-2, -7.259557e-2, 2.664874e-1,
	-2.680961e-1, 1.110036e-1, 3.179148e-1, -7.449403e-2,
	-4.279102e-1, 1.783608e-1, -1.274272e-2, -1.293271e-1,
	5.955364e-1, 1.073973e-1, 3.554508e-1, -2.166391e-1,
	-5.645101e-2, 6.639143e-1, 1.299650e-1, 6.170660e-1,
	7.098893e-1, 6.794992e-2, 6.472573e-2, 2.854365e-1,
	7.188817e-1, -3.434052e-1, -3.648337e-1, -1.419082e-1,
	-3.782669e-1, -3.707471e-1, -1.573869e-1, 1.157296e-1,
	-7.234098e-2, 5.041344e-1, 6.299373e-1, 9.830029e-1,
	9.201707e-1, -6.767877e-2, 1.296642e-1, 1.464763e-1,
	-2.185782e-1, 1.255877e-2, -8.435621e-1, 6.842389e-1,
	-1.327648e-2, 3.854927e-1, -4.899058e-1, 1.003857e-1,
	1.415506e-2, 3.207604e-1, 1.415001e-1, 9.006548e-2,
];

// YuNet, recorded from tract 0.23.4 on the fixed input of `pixels(640*640*3, 12345)`.
// Each row is one declared output: mean, maximum, and four sampled values.
const YUNET_EXPECTED: [[f32; 6]; 12] = [
	[3.225795e-1, 5.002567e-1, 4.502176e-1, 4.432113e-1, 4.305283e-1, 4.268720e-1],
	[4.158589e-1, 5.052462e-1, 4.737059e-1, 4.389420e-1, 4.639358e-1, 4.423938e-1],
	[6.329900e-1, 7.860411e-1, 5.499988e-1, 5.024071e-1, 4.832734e-1, 5.247698e-1],
	[1.384022e-3, 3.692466e-2, 2.051294e-4, 9.417534e-6, 6.592274e-4, 5.739927e-5],
	[3.710113e-4, 1.439953e-2, 4.290044e-4, 2.458692e-5, 2.228022e-4, 7.522106e-5],
	[7.854924e-4, 6.979972e-3, 1.356632e-3, 7.806122e-4, 1.905799e-3, 4.732013e-4],
	[2.135477e-1, 1.080065e0, 5.861937e-1, 2.982946e-1, 5.664475e-1, 4.919586e-1],
	[-1.063826e-1, 8.579210e-1, 5.123827e-1, 4.617671e-1, 5.297288e-1, 5.952517e-1],
	[9.833903e-1, 2.517147e0, 1.240810e0, 1.038598e0, 1.207002e0, 1.187358e0],
	[6.133286e-1, 1.641716e0, -2.000998e-1, -3.537228e-1, -2.693225e-1, -2.384279e-1],
	[4.718952e-1, 9.449253e-1, 2.159463e-1, 1.881350e-1, 2.207145e-1, 2.293447e-1],
	[-5.286866e-1, 5.250635e0, -6.274422e-1, -3.532023e-1, -5.473853e-1, -4.717106e-1],
];

#[test]
fn the_embedder_answers_what_tract_answered() -> Outcome<()> {
	let dir = match models() {
		Some(d) => d,
		None => {
			println!("skipped: set FE2O3_INFER_MODELS to the directory holding the models");
			return Ok(());
		},
	};
	let bytes = res!(fs::read(format!("{}/face_recognition_sface_2021dec.onnx", dir)));
	let emb = res!(Embedder::load(&bytes));
	let crop = pixels(112 * 112 * 3, 999);
	let input = res!(Embedder::input_tensor(&crop));
	let outs = res!(emb.graph().run(Cpu::detect(), input));
	let got = &outs[0].data;
	req!(got.len(), 128);
	let mut worst = 0.0f32;
	for (g, w) in got.iter().zip(SFACE_EXPECTED.iter()) {
		worst = worst.max((g - w).abs());
	}
	if worst > 2e-4 {
		return Err(err!(
			"The embedding differs from tract's by {:.3e}, which is more than rounding.", worst;
		Invalid, Mismatch));
	}
	println!("largest difference from tract: {:.3e}", worst);
	Ok(())
}

#[test]
fn the_detector_answers_what_tract_answered() -> Outcome<()> {
	let dir = match models() {
		Some(d) => d,
		None => {
			println!("skipped: set FE2O3_INFER_MODELS to the directory holding the models");
			return Ok(());
		},
	};
	let bytes = res!(fs::read(format!("{}/face_detection_yunet_2023mar.onnx", dir)));
	let det = res!(Detector::load(&bytes));
	let px = pixels(640 * 640 * 3, 12345);
	let img = res!(Image::new(&px, 640, 640, 3));
	let input = res!(Detector::input_tensor(&img));
	let outs = res!(det.graph().run(Cpu::detect(), input));
	req!(outs.len(), 12);
	let mut worst = 0.0f32;
	for (i, o) in outs.iter().enumerate() {
		let v = &o.data;
		let mean = (v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64) as f32;
		let max = v.iter().fold(f32::MIN, |a, b| a.max(*b));
		let s: Vec<f32> = (1..5).map(|j| v[j * v.len() / 5]).collect();
		let got = [mean, max, s[0], s[1], s[2], s[3]];
		for (g, w) in got.iter().zip(YUNET_EXPECTED[i].iter()) {
			worst = worst.max((g - w).abs());
		}
	}
	if worst > 1e-4 {
		return Err(err!(
			"A head differs from tract's by {:.3e}, which is more than rounding.", worst;
		Invalid, Mismatch));
	}
	println!("largest difference from tract: {:.3e}", worst);
	Ok(())
}

#[test]
fn both_code_paths_reach_the_same_embedding() -> Outcome<()> {
	let dir = match models() {
		Some(d) => d,
		None => {
			println!("skipped: set FE2O3_INFER_MODELS to the directory holding the models");
			return Ok(());
		},
	};
	let bytes = res!(fs::read(format!("{}/face_recognition_sface_2021dec.onnx", dir)));
	let emb = res!(Embedder::load(&bytes));
	let crop = pixels(112 * 112 * 3, 4242);
	let fast = res!(emb.embed_aligned(Cpu::detect(), &crop));
	let slow = res!(emb.embed_aligned(Cpu::Baseline, &crop));
	let mut worst = 0.0f32;
	for i in 0..128 {
		worst = worst.max((fast.v[i] - slow.v[i]).abs());
	}
	if worst > 1e-5 {
		return Err(err!(
			"The dispatched path and the baseline path differ by {:.3e}.", worst;
		Invalid, Mismatch));
	}
	println!("largest difference between the two paths: {:.3e}", worst);
	Ok(())
}
