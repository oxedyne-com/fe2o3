//! The regression guard the matrix kernel needs, and cannot do without.
//!
//! Two failures are silent and expensive, and neither shows up in a correctness
//! test because both paths still compute the right answer:
//!
//! - **The register tile falls off its cliff.** Whether the `[[f32; NR]; MR]`
//!   accumulator lives in vector registers or spills to the stack is an
//!   all-or-nothing decision the code generator makes, and it moves with the
//!   tile shape, with the arithmetic form, and with the compiler version.
//!   Measured on this workload, adjacent tile heights differ by a factor of
//!   thirty-two. A one-character edit, or an upgrade nobody made, can cost
//!   thirty times the throughput.
//! - **`mul_add` reaches the path that has no fused multiply-add.** There it
//!   becomes a library call, and the same thirty-fold loss appears in the
//!   fallback rather than the fast path, where it is even easier to miss.
//!
//! Both are caught here by measurement, on the shape that dominates the
//! embedder: `(m, n, k) = (196, 512, 512)`, five of whose instances are
//! forty-five per cent of the whole network.
//!
//! The floors are deliberately far below what the kernel actually reaches, so
//! that a loaded or thermally throttled machine does not fail the build. They
//! are set to catch a collapse, not a slowdown.

use std::time::Instant;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_infer::kern::{self, Cpu, Scratch, Task, MR, NR};

/// The shape that dominates the embedder.
const SHAPE: (usize, usize, usize) = (196, 512, 512);

/// Below this, on a machine with AVX2 and FMA, the accumulator has spilled.
/// The kernel measures around fifty-eight on an idle machine, and around
/// forty-five with the rest of the suite running beside it; a spill takes it to
/// under two.
const FMA_FLOOR: f64 = 15.0;

/// Below this the baseline path has reached `mul_add` without the instruction,
/// which costs about thirty times. That path measures around twelve idle and
/// around four under load; the failure it is looking for measures half of one.
const BASELINE_FLOOR: f64 = 1.5;

/// A cheap reproducible generator, so the guard needs no dependency.
fn fill(n: usize, seed: u64) -> Vec<f32> {
	let mut s = seed;
	let mut v = Vec::with_capacity(n);
	for _ in 0..n {
		s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
		v.push(((s >> 40) as f32 / 8_388_608.0) - 1.0);
	}
	v
}

/// Times the kernel on one shape, answering billions of multiply-accumulates
/// per second.
fn rate(cpu: Cpu) -> f64 {
	let (m, n, k) = SHAPE;
	let a = fill(m * k, 1);
	let b = fill(k * n, 2);
	let mut c = vec![0.0f32; m * n];
	let mut s = Scratch::new();
	// Warm the buffers and the branch predictor before anything is timed.
	kern::run(cpu, Task::Gemm { m, n, k, a: &a, b: &b, c: &mut c, bias: None, scratch: &mut s });
	let macs = (m * n * k) as f64;
	let t = Instant::now();
	let mut reps = 0u64;
	// Time enough repeats to cover a fifth of a second, so the reading is
	// stable without the test being slow.
	while t.elapsed().as_secs_f64() < 0.2 {
		kern::run(cpu, Task::Gemm {
			m, n, k, a: &a, b: &b, c: &mut c, bias: None, scratch: &mut s });
		reps += 1;
	}
	macs * reps as f64 / t.elapsed().as_secs_f64() / 1e9
}

#[test]
fn the_register_tile_is_the_one_that_was_measured() -> Outcome<()> {
	// Not a performance claim -- a statement that the shape has not been
	// changed without someone reading why it is what it is.
	req!(MR, 6usize);
	req!(NR, 16usize);
	Ok(())
}

#[test]
fn the_dispatched_path_has_not_fallen_off_the_cliff() -> Outcome<()> {
	let cpu = Cpu::detect();
	if !cpu.has_fma() {
		println!("skipped: this machine has no fused multiply-add to dispatch onto");
		return Ok(());
	}
	let r = rate(cpu);
	println!("{}x{}x{} on {:?}: {:.2} GMAC/s", SHAPE.0, SHAPE.1, SHAPE.2, cpu, r);
	if r < FMA_FLOOR {
		return Err(err!(
			"The matrix kernel reached {:.2} GMAC/s against a floor of {:.1}. Either the \
			register tile now spills -- check {}x{} against the neighbouring shapes -- or \
			the specialised path is no longer reaching the inner loop.", r, FMA_FLOOR, MR, NR;
		Invalid, Excessive));
	}
	Ok(())
}

#[test]
fn the_baseline_path_has_not_reached_mul_add() -> Outcome<()> {
	let r = rate(Cpu::Baseline);
	println!("{}x{}x{} on baseline: {:.2} GMAC/s", SHAPE.0, SHAPE.1, SHAPE.2, r);
	if r < BASELINE_FLOOR {
		return Err(err!(
			"The baseline kernel reached {:.2} GMAC/s against a floor of {:.1}, which is \
			what happens when `mul_add` is compiled for a target with no fused \
			multiply-add and becomes a library call.", r, BASELINE_FLOOR;
		Invalid, Excessive));
	}
	Ok(())
}
