//! Safe `f32` kernels, and the single runtime dispatch that selects between a
//! fused-multiply-add code path and a baseline one.
//!
//! Every kernel here is ordinary safe Rust. The only `unsafe` token in the
//! crate is in [`run`], where a `#[target_feature]` function is called after
//! its features have been checked at runtime; that is a sanctioned exception
//! and is documented at the call site.
//!
//! # Why two code paths
//!
//! Rust will not contract `a * b + c` into a fused multiply-add on its own --
//! strict IEEE semantics forbid it -- so the only route to `vfmadd` from safe
//! code is [`f32::mul_add`]. On a target that has no FMA instruction,
//! `mul_add` becomes a libm call and costs about thirty times the arithmetic
//! it replaces. Both bodies therefore exist, selected by a const generic, and
//! the `mul_add` body is reachable only from a function compiled with the
//! feature enabled.

use crate::tensor::Tensor;

use oxedyne_fe2o3_core::prelude::*;

/// Height of the register tile in the blocked matrix kernel.
///
/// This is not a free parameter. Adjacent values differ by more than an order
/// of magnitude in throughput, because the code generator decides all-or-nothing
/// whether the `[[f32; NR]; MR]` accumulator lives in vector registers or spills
/// to the stack. The regression guard in `tests/guard.rs` exists to catch a
/// compiler upgrade that moves the boundary.
pub const MR: usize = 6;

/// Width of the register tile in the blocked matrix kernel. See [`MR`].
pub const NR: usize = 16;

/// Rows of `A` held in one cache block.
pub const MC: usize = 256;

/// Depth of one cache block.
pub const KC: usize = 512;

/// Columns of `B` held in one cache block.
pub const NC: usize = 1024;

/// The instruction set the kernels were dispatched onto.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cpu {
	/// No vector feature was detected, so `mul_add` must not be reached.
	Baseline,
	/// An `x86-64` part carrying both AVX2 and FMA.
	Avx2Fma,
}

impl Cpu {
	/// Detects the best available instruction set on this machine.
	pub fn detect() -> Self {
		#[cfg(target_arch = "x86_64")]
		{
			if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
				return Self::Avx2Fma;
			}
		}
		Self::Baseline
	}

	/// Whether this path may use [`f32::mul_add`].
	pub fn has_fma(&self) -> bool {
		matches!(self, Self::Avx2Fma)
	}
}

impl Default for Cpu {
	fn default() -> Self {
		Self::detect()
	}
}

/// Reusable packing buffers, so a graph run allocates once rather than per layer.
#[derive(Debug, Default)]
pub struct Scratch {
	/// Packed panels of `A`.
	ap:	Vec<f32>,
	/// Packed panels of `B`.
	bp:	Vec<f32>,
}

impl Scratch {
	/// Creates an empty scratch buffer.
	pub fn new() -> Self {
		Self { ap: Vec::new(), bp: Vec::new() }
	}

	/// Grows both buffers to at least the requested sizes.
	#[inline(always)]
	fn ensure(&mut self, na: usize, nb: usize) {
		if self.ap.len() < na {
			self.ap.resize(na, 0.0);
		}
		if self.bp.len() < nb {
			self.bp.resize(nb, 0.0);
		}
	}
}

/// One unit of numerical work, named so that a single dispatch point can carry
/// every kernel across the feature boundary.
///
/// Passing the work as a value rather than calling each kernel directly is what
/// keeps the crate to one `unsafe` token: the whole set is monomorphised twice,
/// once inside a `#[target_feature]` function and once outside it, and the
/// caller picks between them by matching on [`Cpu`].
pub enum Task<'a> {
	/// `c[m, n] = bias + a[m, k] · b[k, n]`, all row-major.
	Gemm {
		/// Rows of `a` and of `c`.
		m:			usize,
		/// Columns of `b` and of `c`.
		n:			usize,
		/// Shared inner extent.
		k:			usize,
		/// Left operand, `[m, k]`.
		a:			&'a [f32],
		/// Right operand, `[k, n]`.
		b:			&'a [f32],
		/// Destination, `[m, n]`, overwritten.
		c:			&'a mut [f32],
		/// Optional per-column bias, prefilled into `c` before accumulation.
		bias:		Option<&'a [f32]>,
		/// Packing buffers.
		scratch:	&'a mut Scratch,
	},
	/// `c[n] = a[k] · bt[n, k]ᵀ`, the matrix--vector case an ONNX `Gemm` with
	/// `transB=1` presents, where each output is a contiguous dot product.
	MatVecT {
		/// Number of outputs.
		n:		usize,
		/// Length of each dot product.
		k:		usize,
		/// The vector, `[k]`.
		a:		&'a [f32],
		/// Weights, `[n, k]`.
		bt:		&'a [f32],
		/// Destination, `[n]`.
		c:		&'a mut [f32],
	},
	/// Gathers `[oh·ow, kh·kw·ch]` convolution patches out of an `NHWC` plane.
	Im2Col {
		/// Channels.
		ch:		usize,
		/// Input height.
		h:		usize,
		/// Input width.
		w:		usize,
		/// Kernel height.
		kh:		usize,
		/// Kernel width.
		kw:		usize,
		/// Vertical stride.
		sy:		usize,
		/// Horizontal stride.
		sx:		usize,
		/// Padding above.
		pt:		usize,
		/// Padding to the left.
		pl:		usize,
		/// Output height.
		oh:		usize,
		/// Output width.
		ow:		usize,
		/// Source plane, `[h, w, ch]`.
		x:		&'a [f32],
		/// Destination, `[oh·ow, kh·kw·ch]`.
		out:	&'a mut [f32],
	},
	/// Depthwise convolution in `NHWC`, one weight plane per channel.
	Depthwise {
		/// Channels.
		ch:		usize,
		/// Input height.
		h:		usize,
		/// Input width.
		w:		usize,
		/// Kernel height.
		kh:		usize,
		/// Kernel width.
		kw:		usize,
		/// Vertical stride.
		sy:		usize,
		/// Horizontal stride.
		sx:		usize,
		/// Padding above.
		pt:		usize,
		/// Padding to the left.
		pl:		usize,
		/// Output height.
		oh:		usize,
		/// Output width.
		ow:		usize,
		/// Source plane, `[h, w, ch]`.
		x:		&'a [f32],
		/// Weights, `[kh·kw, ch]`.
		wt:		&'a [f32],
		/// Optional per-channel bias.
		bias:	Option<&'a [f32]>,
		/// Destination, `[oh, ow, ch]`.
		y:		&'a mut [f32],
	},
	/// Per-channel affine map, `x = scale·x + bias`, over a channels-last buffer.
	Scale {
		/// Channels, the innermost extent.
		ch:		usize,
		/// Buffer, rewritten in place.
		x:		&'a mut [f32],
		/// Per-channel multiplier.
		scale:	&'a [f32],
		/// Per-channel offset.
		bias:	&'a [f32],
	},
	/// Parametric rectified linear unit with a per-channel slope, written
	/// branchlessly so that the loop still vectorises.
	PRelu {
		/// Channels, the innermost extent.
		ch:		usize,
		/// Buffer, rewritten in place.
		x:		&'a mut [f32],
		/// Per-channel negative slope.
		slope:	&'a [f32],
	},
	/// Rectified linear unit.
	Relu {
		/// Buffer, rewritten in place.
		x:	&'a mut [f32],
	},
	/// Logistic sigmoid.
	Sigmoid {
		/// Buffer, rewritten in place.
		x:	&'a mut [f32],
	},
	/// Two by two maximum pool, stride two, no padding, in `NHWC`.
	MaxPool2x2 {
		/// Channels.
		ch:	usize,
		/// Input height.
		h:	usize,
		/// Input width.
		w:	usize,
		/// Source, `[h, w, ch]`.
		x:	&'a [f32],
		/// Destination, `[h/2, w/2, ch]`.
		y:	&'a mut [f32],
	},
	/// Nearest-neighbour doubling in `NHWC`.
	Upsample2x {
		/// Channels.
		ch:	usize,
		/// Input height.
		h:	usize,
		/// Input width.
		w:	usize,
		/// Source, `[h, w, ch]`.
		x:	&'a [f32],
		/// Destination, `[2h, 2w, ch]`.
		y:	&'a mut [f32],
	},
	/// Element-wise sum, accumulated into the first operand.
	Add {
		/// Accumulator.
		x:	&'a mut [f32],
		/// Addend.
		y:	&'a [f32],
	},
}

/// Runs one unit of work on the given instruction set.
///
/// This is the crate's only dispatch point and its only `unsafe` token.
pub fn run(cpu: Cpu, task: Task<'_>) {
	match cpu {
		#[cfg(target_arch = "x86_64")]
		Cpu::Avx2Fma => {
			// The single sanctioned `unsafe` in this crate. Calling a
			// `#[target_feature]` function from an unfeatured context requires
			// it even though the body is entirely safe, and `Cpu::detect` has
			// already established that this machine has AVX2 and FMA.
			#[allow(unsafe_code)]
			unsafe { dispatch_avx2_fma(task) }
		},
		#[cfg(not(target_arch = "x86_64"))]
		Cpu::Avx2Fma => dispatch_baseline(task),
		Cpu::Baseline => dispatch_baseline(task),
	}
}

/// The kernel set compiled for AVX2 and FMA.
///
/// Nothing here is `unsafe`. A `#[target_feature]` function may call another
/// carrying the same features without one, so this frame names the specialised
/// wrappers below and the token stays at the single boundary in [`run`].
///
/// Each kernel keeps its own function rather than being inlined into this one.
/// That is not a stylistic choice: folding the whole set into one body costs
/// about three quarters of the matrix throughput, because the register
/// allocator then has the entire dispatch to satisfy and gives up on holding
/// the accumulator tile in vector registers.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn dispatch_avx2_fma(task: Task<'_>) {
	match task {
		Task::Gemm { m, n, k, a, b, c, bias, scratch } =>
			gemm_tf(m, n, k, a, b, c, bias, scratch),
		Task::MatVecT { n, k, a, bt, c } =>
			matvec_t_tf(n, k, a, bt, c),
		Task::Im2Col { ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, out } =>
			im2col_tf(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, out),
		Task::Depthwise { ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, wt, bias, y } =>
			depthwise_tf(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, wt, bias, y),
		Task::Scale { ch, x, scale, bias } =>
			scale_bias_tf(ch, x, scale, bias),
		Task::PRelu { ch, x, slope } =>
			prelu_tf(ch, x, slope),
		Task::Relu { x } =>
			relu_tf(x),
		Task::Sigmoid { x } =>
			sigmoid_tf(x),
		Task::MaxPool2x2 { ch, h, w, x, y } =>
			maxpool2x2_tf(ch, h, w, x, y),
		Task::Upsample2x { ch, h, w, x, y } =>
			upsample2x_tf(ch, h, w, x, y),
		Task::Add { x, y } =>
			add_tf(x, y),
	}
}

/// The kernel set compiled for the baseline target, where `mul_add` is a
/// library call and must not be reached.
fn dispatch_baseline(task: Task<'_>) {
	match task {
		Task::Gemm { m, n, k, a, b, c, bias, scratch } =>
			gemm::<false>(m, n, k, a, b, c, bias, scratch),
		Task::MatVecT { n, k, a, bt, c } =>
			matvec_t::<false>(n, k, a, bt, c),
		Task::Im2Col { ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, out } =>
			im2col(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, out),
		Task::Depthwise { ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, wt, bias, y } =>
			depthwise::<false>(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, wt, bias, y),
		Task::Scale { ch, x, scale, bias } =>
			scale_bias::<false>(ch, x, scale, bias),
		Task::PRelu { ch, x, slope } =>
			prelu(ch, x, slope),
		Task::Relu { x } =>
			relu(x),
		Task::Sigmoid { x } =>
			sigmoid(x),
		Task::MaxPool2x2 { ch, h, w, x, y } =>
			maxpool2x2(ch, h, w, x, y),
		Task::Upsample2x { ch, h, w, x, y } =>
			upsample2x(ch, h, w, x, y),
		Task::Add { x, y } =>
			add(x, y),
	}
}

/// The blocked matrix product, compiled for AVX2 and FMA.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn gemm_tf(
	m:			usize,
	n:			usize,
	k:			usize,
	a:			&[f32],
	b:			&[f32],
	c:			&mut [f32],
	bias:		Option<&[f32]>,
	scratch:	&mut Scratch,
) {
	gemm::<true>(m, n, k, a, b, c, bias, scratch)
}

/// The matrix--vector product, compiled for AVX2 and FMA.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn matvec_t_tf(n: usize, k: usize, a: &[f32], bt: &[f32], c: &mut [f32]) {
	matvec_t::<true>(n, k, a, bt, c)
}

/// The patch gather, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(clippy::too_many_arguments)]
fn im2col_tf(
	ch:		usize,
	h:		usize,
	w:		usize,
	kh:		usize,
	kw:		usize,
	sy:		usize,
	sx:		usize,
	pt:		usize,
	pl:		usize,
	oh:		usize,
	ow:		usize,
	x:		&[f32],
	out:	&mut [f32],
) {
	im2col(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, out)
}

/// The depthwise convolution, compiled for AVX2 and FMA.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(clippy::too_many_arguments)]
fn depthwise_tf(
	ch:		usize,
	h:		usize,
	w:		usize,
	kh:		usize,
	kw:		usize,
	sy:		usize,
	sx:		usize,
	pt:		usize,
	pl:		usize,
	oh:		usize,
	ow:		usize,
	x:		&[f32],
	wt:		&[f32],
	bias:	Option<&[f32]>,
	y:		&mut [f32],
) {
	depthwise::<true>(ch, h, w, kh, kw, sy, sx, pt, pl, oh, ow, x, wt, bias, y)
}

/// The per-channel affine map, compiled for AVX2 and FMA.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn scale_bias_tf(ch: usize, x: &mut [f32], sc: &[f32], bi: &[f32]) {
	scale_bias::<true>(ch, x, sc, bi)
}

/// The parametric rectifier, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn prelu_tf(ch: usize, x: &mut [f32], slope: &[f32]) {
	prelu(ch, x, slope)
}

/// The rectifier, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn relu_tf(x: &mut [f32]) {
	relu(x)
}

/// The sigmoid, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn sigmoid_tf(x: &mut [f32]) {
	sigmoid(x)
}

/// The maximum pool, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn maxpool2x2_tf(ch: usize, h: usize, w: usize, x: &[f32], y: &mut [f32]) {
	maxpool2x2(ch, h, w, x, y)
}

/// The nearest-neighbour doubling, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn upsample2x_tf(ch: usize, h: usize, w: usize, x: &[f32], y: &mut [f32]) {
	upsample2x(ch, h, w, x, y)
}

/// The element-wise sum, compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
fn add_tf(x: &mut [f32], y: &[f32]) {
	add(x, y)
}

/// Fused multiply-add in whichever form the compiled path may use.
///
/// Under `FMA` this is `f32::mul_add`, which lowers to a single instruction and
/// rounds once. Without it, the plain form, because `mul_add` would become a
/// library call.
#[inline(always)]
fn fma<const FMA: bool>(a: f32, b: f32, c: f32) -> f32 {
	if FMA {
		a.mul_add(b, c)
	} else {
		a * b + c
	}
}

/// Packs a `kc × nc` block of `b` into `NR`-wide panels, zero padded.
#[inline(always)]
fn pack_b(
	b:		&[f32],
	ldb:	usize,
	p0:		usize,
	kc:		usize,
	j0:		usize,
	nc:		usize,
	out:	&mut [f32],
) {
	let panels = (nc + NR - 1) / NR;
	for p in 0..panels {
		let jbase = j0 + p * NR;
		let nv = core::cmp::min(NR, nc - p * NR);
		let dst = &mut out[p * kc * NR..(p + 1) * kc * NR];
		for kk in 0..kc {
			let src = &b[(p0 + kk) * ldb + jbase..(p0 + kk) * ldb + jbase + nv];
			let slot = &mut dst[kk * NR..kk * NR + NR];
			for j in 0..nv {
				slot[j] = src[j];
			}
			for j in nv..NR {
				slot[j] = 0.0;
			}
		}
	}
}

/// Packs an `mc × kc` block of `a` into `MR`-tall panels, zero padded.
#[inline(always)]
fn pack_a(
	a:		&[f32],
	lda:	usize,
	i0:		usize,
	mc:		usize,
	p0:		usize,
	kc:		usize,
	out:	&mut [f32],
) {
	let panels = (mc + MR - 1) / MR;
	for p in 0..panels {
		let ibase = i0 + p * MR;
		let mv = core::cmp::min(MR, mc - p * MR);
		let dst = &mut out[p * kc * MR..(p + 1) * kc * MR];
		for i in 0..mv {
			let src = &a[(ibase + i) * lda + p0..(ibase + i) * lda + p0 + kc];
			for kk in 0..kc {
				dst[kk * MR + i] = src[kk];
			}
		}
		for i in mv..MR {
			for kk in 0..kc {
				dst[kk * MR + i] = 0.0;
			}
		}
	}
}

/// The register-tile microkernel, `c[0..MR, 0..NR] += ap · bp`.
///
/// The accumulator is a fixed-size array so that the code generator can hold it
/// in vector registers, and `chunks_exact` proves the inner extents without an
/// index that could fail.
#[inline(always)]
fn micro<const FMA: bool>(
	kc:		usize,
	ap:		&[f32],
	bp:		&[f32],
	c:		&mut [f32],
	ldc:	usize,
	i0:		usize,
	j0:		usize,
	mv:		usize,
	nv:		usize,
) {
	let mut acc = [[0.0f32; NR]; MR];
	let asub = &ap[..kc * MR];
	let bsub = &bp[..kc * NR];
	for (achunk, bchunk) in asub.chunks_exact(MR).zip(bsub.chunks_exact(NR)) {
		for i in 0..MR {
			let av = achunk[i];
			for j in 0..NR {
				acc[i][j] = fma::<FMA>(av, bchunk[j], acc[i][j]);
			}
		}
	}
	for i in 0..mv {
		let base = (i0 + i) * ldc + j0;
		let row = &mut c[base..base + nv];
		let src = &acc[i];
		for j in 0..nv {
			row[j] += src[j];
		}
	}
}

/// Blocked, packed general matrix product.
#[inline(always)]
fn gemm<const FMA: bool>(
	m:			usize,
	n:			usize,
	k:			usize,
	a:			&[f32],
	b:			&[f32],
	c:			&mut [f32],
	bias:		Option<&[f32]>,
	scratch:	&mut Scratch,
) {
	match bias {
		Some(bs) => {
			for row in c.chunks_exact_mut(n) {
				row.copy_from_slice(&bs[..n]);
			}
		},
		None => {
			for v in c.iter_mut() {
				*v = 0.0;
			}
		},
	}
	let mcb = core::cmp::min(MC, m);
	let kcb = core::cmp::min(KC, k);
	let ncb = core::cmp::min(NC, n);
	scratch.ensure(
		((mcb + MR - 1) / MR) * kcb * MR,
		((ncb + NR - 1) / NR) * kcb * NR,
	);
	let mut jc = 0;
	while jc < n {
		let nn = core::cmp::min(ncb, n - jc);
		let mut pc = 0;
		while pc < k {
			let kk = core::cmp::min(kcb, k - pc);
			pack_b(b, n, pc, kk, jc, nn, &mut scratch.bp);
			let mut ic = 0;
			while ic < m {
				let mm = core::cmp::min(mcb, m - ic);
				pack_a(a, k, ic, mm, pc, kk, &mut scratch.ap);
				let jpan = (nn + NR - 1) / NR;
				let ipan = (mm + MR - 1) / MR;
				for jp in 0..jpan {
					let nv = core::cmp::min(NR, nn - jp * NR);
					let bpan = &scratch.bp[jp * kk * NR..(jp + 1) * kk * NR];
					for ip in 0..ipan {
						let mv = core::cmp::min(MR, mm - ip * MR);
						let apan = &scratch.ap[ip * kk * MR..(ip + 1) * kk * MR];
						micro::<FMA>(
							kk,
							apan,
							bpan,
							c,
							n,
							ic + ip * MR,
							jc + jp * NR,
							mv,
							nv,
						);
					}
				}
				ic += mcb;
			}
			pc += kcb;
		}
		jc += ncb;
	}
}

/// Matrix--vector product against a transposed weight matrix.
///
/// Eight partial accumulators break the latency chain of the multiply-add unit.
/// This layer is bandwidth bound rather than compute bound, so the win over the
/// general kernel comes from reading each weight exactly once.
#[inline(always)]
fn matvec_t<const FMA: bool>(n: usize, k: usize, a: &[f32], bt: &[f32], c: &mut [f32]) {
	for j in 0..n {
		let w = &bt[j * k..j * k + k];
		let mut s = [0.0f32; 8];
		let mut it_a = a.chunks_exact(8);
		let mut it_w = w.chunks_exact(8);
		for (ca, cw) in it_a.by_ref().zip(it_w.by_ref()) {
			for l in 0..8 {
				s[l] = fma::<FMA>(ca[l], cw[l], s[l]);
			}
		}
		let mut tail = 0.0f32;
		for (x, y) in it_a.remainder().iter().zip(it_w.remainder().iter()) {
			tail = fma::<FMA>(*x, *y, tail);
		}
		c[j] = ((s[0] + s[1]) + (s[2] + s[3])) + ((s[4] + s[5]) + (s[6] + s[7])) + tail;
	}
}

/// Gathers convolution patches out of a channels-last plane.
///
/// Only a kernel larger than one by one needs this. A one by one convolution in
/// `NHWC` is already the `[m = h·w, k = ch]` matrix the product consumes, which
/// is why twenty-six of the twenty-seven convolutions in a MobileFaceNet-shaped
/// embedder skip it entirely.
#[inline(always)]
fn im2col(
	ch:		usize,
	h:		usize,
	w:		usize,
	kh:		usize,
	kw:		usize,
	sy:		usize,
	sx:		usize,
	pt:		usize,
	pl:		usize,
	oh:		usize,
	ow:		usize,
	x:		&[f32],
	out:	&mut [f32],
) {
	let kk = kh * kw * ch;
	for oy in 0..oh {
		let iy0 = (oy * sy) as isize - pt as isize;
		for ox in 0..ow {
			let ix0 = (ox * sx) as isize - pl as isize;
			let row = &mut out[(oy * ow + ox) * kk..(oy * ow + ox) * kk + kk];
			for ky in 0..kh {
				let iy = iy0 + ky as isize;
				for kx in 0..kw {
					let ix = ix0 + kx as isize;
					let dst = &mut row[(ky * kw + kx) * ch..(ky * kw + kx) * ch + ch];
					if iy < 0 || iy as usize >= h || ix < 0 || ix as usize >= w {
						for v in dst.iter_mut() {
							*v = 0.0;
						}
					} else {
						let base = (iy as usize * w + ix as usize) * ch;
						dst.copy_from_slice(&x[base..base + ch]);
					}
				}
			}
		}
	}
}

/// Depthwise convolution in `NHWC`.
///
/// The channel loop is innermost, which makes it unit stride in the activation,
/// the weights and the output at once. The same arithmetic written channels-first
/// runs about nine times slower, because nothing there vectorises.
#[inline(always)]
fn depthwise<const FMA: bool>(
	ch:		usize,
	h:		usize,
	w:		usize,
	kh:		usize,
	kw:		usize,
	sy:		usize,
	sx:		usize,
	pt:		usize,
	pl:		usize,
	oh:		usize,
	ow:		usize,
	x:		&[f32],
	wt:		&[f32],
	bias:	Option<&[f32]>,
	y:		&mut [f32],
) {
	for oy in 0..oh {
		let iy0 = (oy * sy) as isize - pt as isize;
		for ox in 0..ow {
			let ix0 = (ox * sx) as isize - pl as isize;
			let out = &mut y[(oy * ow + ox) * ch..(oy * ow + ox) * ch + ch];
			match bias {
				Some(bs) => out.copy_from_slice(&bs[..ch]),
				None => {
					for v in out.iter_mut() {
						*v = 0.0;
					}
				},
			}
			for ky in 0..kh {
				let iy = iy0 + ky as isize;
				if iy < 0 || iy as usize >= h {
					continue;
				}
				for kx in 0..kw {
					let ix = ix0 + kx as isize;
					if ix < 0 || ix as usize >= w {
						continue;
					}
					let base = (iy as usize * w + ix as usize) * ch;
					let src = &x[base..base + ch];
					let kv = &wt[(ky * kw + kx) * ch..(ky * kw + kx) * ch + ch];
					for c in 0..ch {
						out[c] = fma::<FMA>(src[c], kv[c], out[c]);
					}
				}
			}
		}
	}
}

/// Per-channel affine map over a channels-last buffer.
#[inline(always)]
fn scale_bias<const FMA: bool>(ch: usize, x: &mut [f32], sc: &[f32], bi: &[f32]) {
	let sc = &sc[..ch];
	let bi = &bi[..ch];
	for row in x.chunks_exact_mut(ch) {
		for j in 0..ch {
			row[j] = fma::<FMA>(sc[j], row[j], bi[j]);
		}
	}
}

/// Parametric rectified linear unit, branchless.
///
/// Written as a comparison the loop keeps `v.max(0) + slope·v.min(0)`, because
/// the obvious `if v >= 0` form does not vectorise and costs an order of
/// magnitude over a whole network.
#[inline(always)]
fn prelu(ch: usize, x: &mut [f32], slope: &[f32]) {
	let sl = &slope[..ch];
	for row in x.chunks_exact_mut(ch) {
		for j in 0..ch {
			let v = row[j];
			row[j] = v.max(0.0) + sl[j] * v.min(0.0);
		}
	}
}

/// Rectified linear unit.
#[inline(always)]
fn relu(x: &mut [f32]) {
	for v in x.iter_mut() {
		*v = v.max(0.0);
	}
}

/// Logistic sigmoid.
#[inline(always)]
fn sigmoid(x: &mut [f32]) {
	for v in x.iter_mut() {
		*v = 1.0 / (1.0 + (-*v).exp());
	}
}

/// Two by two maximum pool, stride two, in `NHWC`.
#[inline(always)]
fn maxpool2x2(ch: usize, h: usize, w: usize, x: &[f32], y: &mut [f32]) {
	let (oh, ow) = (h / 2, w / 2);
	for oy in 0..oh {
		for ox in 0..ow {
			let o = &mut y[(oy * ow + ox) * ch..(oy * ow + ox) * ch + ch];
			let ra = (2 * oy * w + 2 * ox) * ch;
			let rb = (2 * oy * w + 2 * ox + 1) * ch;
			let rc = ((2 * oy + 1) * w + 2 * ox) * ch;
			let rd = ((2 * oy + 1) * w + 2 * ox + 1) * ch;
			let a = &x[ra..ra + ch];
			let b = &x[rb..rb + ch];
			let c = &x[rc..rc + ch];
			let d = &x[rd..rd + ch];
			for i in 0..ch {
				o[i] = a[i].max(b[i]).max(c[i].max(d[i]));
			}
		}
	}
}

/// Nearest-neighbour doubling in `NHWC`.
#[inline(always)]
fn upsample2x(ch: usize, h: usize, w: usize, x: &[f32], y: &mut [f32]) {
	let ow = w * 2;
	for iy in 0..h {
		for ix in 0..w {
			let src = &x[(iy * w + ix) * ch..(iy * w + ix) * ch + ch];
			for dy in 0..2 {
				let orow = (iy * 2 + dy) * ow;
				for dx in 0..2 {
					let o = (orow + ix * 2 + dx) * ch;
					y[o..o + ch].copy_from_slice(src);
				}
			}
		}
	}
}

/// Element-wise sum, accumulated into the first operand.
#[inline(always)]
fn add(x: &mut [f32], y: &[f32]) {
	for (a, b) in x.iter_mut().zip(y.iter()) {
		*a += *b;
	}
}

/// Rewrites an `[n, h, w, c]` activation as the `[n, c·h·w]` row an ONNX
/// `Flatten` produces, which is channels-first order.
///
/// This is the one place the channels-last layout has to be undone, and it is
/// cheap: one gather per embedding, not per layer.
pub fn flatten_nchw(t: &Tensor) -> Outcome<Tensor> {
	let (n, h, w, c) = res!(t.nhwc());
	let plane = h * w;
	let mut out = vec![0.0f32; t.len()];
	for bi in 0..n {
		for ci in 0..c {
			let dst = (bi * c + ci) * plane;
			for p in 0..plane {
				out[dst + p] = t.data[(bi * plane + p) * c + ci];
			}
		}
	}
	Tensor::new(vec![n, c * plane], out)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Reference product in `f64`, so the comparison is against something the
	/// kernel does not share code with.
	fn reference(m: usize, n: usize, k: usize, a: &[f32], b: &[f32]) -> Vec<f32> {
		let mut c = vec![0.0f32; m * n];
		for i in 0..m {
			for j in 0..n {
				let mut s = 0.0f64;
				for p in 0..k {
					s += a[i * k + p] as f64 * b[p * n + j] as f64;
				}
				c[i * n + j] = s as f32;
			}
		}
		c
	}

	/// A cheap reproducible generator, so a test needs no dependency.
	fn fill(n: usize, seed: u64) -> Vec<f32> {
		let mut s = seed;
		let mut v = Vec::with_capacity(n);
		for _ in 0..n {
			s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			v.push(((s >> 40) as f32 / 8_388_608.0) - 1.0);
		}
		v
	}

	#[test]
	fn gemm_matches_a_wider_reference() -> Outcome<()> {
		for &(m, n, k) in &[(1, 1, 1), (6, 16, 5), (7, 17, 33), (196, 512, 128), (49, 71, 200)] {
			let a = fill(m * k, 1);
			let b = fill(k * n, 2);
			let want = reference(m, n, k, &a, &b);
			for cpu in [Cpu::Baseline, Cpu::detect()] {
				let mut c = vec![0.0f32; m * n];
				let mut s = Scratch::new();
				run(cpu, Task::Gemm {
					m, n, k,
					a:			&a,
					b:			&b,
					c:			&mut c,
					bias:		None,
					scratch:	&mut s,
				});
				for i in 0..m * n {
					let d = (c[i] - want[i]).abs();
					if d > 1e-3 {
						return Err(err!(
							"On {}x{}x{} under {:?}, element {} read {} against {}.",
							m, n, k, cpu, i, c[i], want[i];
						Invalid, Mismatch));
					}
				}
			}
		}
		Ok(())
	}

	#[test]
	fn both_paths_agree() -> Outcome<()> {
		let (m, n, k) = (37, 53, 71);
		let a = fill(m * k, 11);
		let b = fill(k * n, 12);
		let bias = fill(n, 13);
		let mut c0 = vec![0.0f32; m * n];
		let mut c1 = vec![0.0f32; m * n];
		let mut s = Scratch::new();
		run(Cpu::Baseline, Task::Gemm {
			m, n, k, a: &a, b: &b, c: &mut c0, bias: Some(&bias), scratch: &mut s });
		run(Cpu::detect(), Task::Gemm {
			m, n, k, a: &a, b: &b, c: &mut c1, bias: Some(&bias), scratch: &mut s });
		for i in 0..m * n {
			if (c0[i] - c1[i]).abs() > 1e-4 {
				return Err(err!(
					"The baseline and dispatched paths disagree at {}: {} against {}.",
					i, c0[i], c1[i];
				Invalid, Mismatch));
			}
		}
		Ok(())
	}

	#[test]
	fn matvec_matches_the_general_kernel() -> Outcome<()> {
		let (n, k) = (128, 501);
		let a = fill(k, 21);
		let bt = fill(n * k, 22);
		let mut want = vec![0.0f32; n];
		for j in 0..n {
			let mut s = 0.0f64;
			for p in 0..k {
				s += a[p] as f64 * bt[j * k + p] as f64;
			}
			want[j] = s as f32;
		}
		for cpu in [Cpu::Baseline, Cpu::detect()] {
			let mut c = vec![0.0f32; n];
			run(cpu, Task::MatVecT { n, k, a: &a, bt: &bt, c: &mut c });
			for j in 0..n {
				if (c[j] - want[j]).abs() > 1e-3 {
					return Err(err!(
						"Under {:?}, output {} read {} against {}.", cpu, j, c[j], want[j];
					Invalid, Mismatch));
				}
			}
		}
		Ok(())
	}

	#[test]
	fn depthwise_matches_a_direct_loop() -> Outcome<()> {
		let (ch, h, w) = (5, 7, 9);
		let x = fill(h * w * ch, 31);
		let wt = fill(9 * ch, 32);
		for stride in [1usize, 2] {
			let oh = (h + 2 - 3) / stride + 1;
			let ow = (w + 2 - 3) / stride + 1;
			let mut want = vec![0.0f32; oh * ow * ch];
			for oy in 0..oh {
				for ox in 0..ow {
					for c in 0..ch {
						let mut s = 0.0f64;
						for ky in 0..3isize {
							for kx in 0..3isize {
								let iy = (oy * stride) as isize - 1 + ky;
								let ix = (ox * stride) as isize - 1 + kx;
								if iy < 0 || iy as usize >= h || ix < 0 || ix as usize >= w {
									continue;
								}
								s += x[(iy as usize * w + ix as usize) * ch + c] as f64
									* wt[(ky as usize * 3 + kx as usize) * ch + c] as f64;
							}
						}
						want[(oy * ow + ox) * ch + c] = s as f32;
					}
				}
			}
			for cpu in [Cpu::Baseline, Cpu::detect()] {
				let mut y = vec![0.0f32; oh * ow * ch];
				run(cpu, Task::Depthwise {
					ch, h, w,
					kh:		3,
					kw:		3,
					sy:		stride,
					sx:		stride,
					pt:		1,
					pl:		1,
					oh, ow,
					x:		&x,
					wt:		&wt,
					bias:	None,
					y:		&mut y,
				});
				for i in 0..y.len() {
					if (y[i] - want[i]).abs() > 1e-5 {
						return Err(err!(
							"Depthwise stride {} under {:?} differs at {}: {} against {}.",
							stride, cpu, i, y[i], want[i];
						Invalid, Mismatch));
					}
				}
			}
		}
		Ok(())
	}

	#[test]
	fn pooling_and_doubling_invert_a_constant() -> Outcome<()> {
		let (ch, h, w) = (3, 4, 6);
		let x = fill(h * w * ch, 41);
		let mut pooled = vec![0.0f32; (h / 2) * (w / 2) * ch];
		run(Cpu::detect(), Task::MaxPool2x2 { ch, h, w, x: &x, y: &mut pooled });
		let mut back = vec![0.0f32; h * w * ch];
		run(Cpu::detect(), Task::Upsample2x { ch, h: h / 2, w: w / 2, x: &pooled, y: &mut back });
		// Each pooled maximum is repeated over the two by two block it came from.
		for oy in 0..h / 2 {
			for ox in 0..w / 2 {
				for c in 0..ch {
					let want = pooled[(oy * (w / 2) + ox) * ch + c];
					for dy in 0..2 {
						for dx in 0..2 {
							let got = back[((oy * 2 + dy) * w + ox * 2 + dx) * ch + c];
							req!(got, want);
						}
					}
				}
			}
		}
		Ok(())
	}
}
