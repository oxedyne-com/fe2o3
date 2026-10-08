//! Evaluation of the `mft1` and `mft2` lookup tables the reader holds as integers: the input
//! curves, the lattice by interpolation, the output curves.
//!
//! Values go in and come out as fractions of full scale, which is how a table holds its encoding:
//! a Lab colour is placed on that scale by [`crate::icc::lab::lab_enc`], and a device value is
//! already there. The curves are interpolated linearly between their entries. The lattice is
//! interpolated tetrahedrally on its last three axes, as LittleCMS does, and linearly on a fourth
//! axis before them; a lattice of fewer axes uses the same rules (one is linear, two bilinear).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::icc::read::Lut;

// Where a coordinate lies on a lattice axis: the node below it and the fraction past that node.
fn locate(x: f64, grid: usize) -> (usize, f64) {
	let at = x.clamp(0.0, 1.0) * (grid - 1) as f64;
	let i = (at.floor() as usize).min(grid - 2);
	(i, at - i as f64)
}

// A table's curve at `x`, as a fraction of full scale.
fn curve(t: &[u16], max: f64, x: f64) -> f64 {
	let at = x.clamp(0.0, 1.0) * (t.len() - 1) as f64;
	let i = (at.floor() as usize).min(t.len() - 2);
	let f = at - i as f64;
	(t[i] as f64 + (t[i + 1] as f64 - t[i] as f64) * f) / max
}

/// Tetrahedral interpolation in the cell of a 3-D lattice whose lowest node is `base`.
///
/// `s` is the distance in nodes between neighbours along each axis, `f` the fractions past the
/// lowest node, and `node` gives the four values at a node. The cell is cut into the six
/// tetrahedra that share its main diagonal, and the point is placed in the one its fractions order.
pub fn tetra<F: Fn(usize) -> [f64; 4]>(node: F, base: usize, s: [usize; 3], f: [f64; 3]) -> [f64; 4] {
	let mut o = [0usize, 1, 2];
	if f[o[0]] < f[o[1]] { o.swap(0, 1); }
	if f[o[1]] < f[o[2]] { o.swap(1, 2); }
	if f[o[0]] < f[o[1]] { o.swap(0, 1); }
	let p0 = base;
	let p1 = p0 + s[o[0]];
	let p2 = p1 + s[o[1]];
	let p3 = p2 + s[o[2]];
	let w = [1.0 - f[o[0]], f[o[0]] - f[o[1]], f[o[1]] - f[o[2]], f[o[2]]];
	let (n0, n1, n2, n3) = (node(p0), node(p1), node(p2), node(p3));
	let mut r = [0.0; 4];
	for j in 0..4 {
		r[j] = w[0] * n0[j] + w[1] * n1[j] + w[2] * n2[j] + w[3] * n3[j];
	}
	r
}

impl Lut {
	/// The table's output for inputs `v`, each a fraction of full scale; only the first `nin`
	/// inputs are read and only the first `nout` outputs are meaningful.
	pub fn eval(&self, v: [f64; 4]) -> [f64; 4] {
		let max = self.max();
		let mut at = [(0usize, 0.0f64); 4];
		let mut stride = [0usize; 4];
		let mut s = self.nout;
		for c in (0..self.nin).rev() {
			let t = &self.ins[c * self.ine..(c + 1) * self.ine];
			at[c] = locate(curve(t, max, v[c]), self.grid);
			stride[c] = s;
			s *= self.grid;
		}
		let mut y = self.lattice(&at[..self.nin], &stride[..self.nin], 0);
		for j in 0..self.nout {
			let t = &self.outs[j * self.oute..(j + 1) * self.oute];
			y[j] = curve(t, max, y[j]);
		}
		y
	}

	// The node at flat offset `off`, as fractions of full scale.
	fn at(&self, off: usize) -> [f64; 4] {
		let max = self.max();
		let mut r = [0.0; 4];
		for j in 0..self.nout {
			r[j] = self.clut[off + j] as f64 / max;
		}
		r
	}

	// The lattice at the axes still to interpolate, from the cell whose lowest node is `base`.
	fn lattice(&self, axes: &[(usize, f64)], stride: &[usize], base: usize) -> [f64; 4] {
		match axes.len() {
			0	=> self.at(base),
			1	=> {
				let (i, f) = axes[0];
				let (a, b) = (self.at(base + i * stride[0]), self.at(base + (i + 1) * stride[0]));
				lerp(a, b, f)
			},
			2	=> {
				let (i, f) = axes[0];
				let lo = base + i * stride[0];
				let a = self.lattice(&axes[1..], &stride[1..], lo);
				let b = self.lattice(&axes[1..], &stride[1..], lo + stride[0]);
				lerp(a, b, f)
			},
			3	=> {
				let base = base + axes[0].0 * stride[0] + axes[1].0 * stride[1] + axes[2].0 * stride[2];
				tetra(|n| self.at(n), base, [stride[0], stride[1], stride[2]], [axes[0].1, axes[1].1, axes[2].1])
			},
			_	=> {
				let (i, f) = axes[0];
				let lo = base + i * stride[0];
				let a = self.lattice(&axes[1..], &stride[1..], lo);
				let b = self.lattice(&axes[1..], &stride[1..], lo + stride[0]);
				lerp(a, b, f)
			},
		}
	}
}

fn lerp(a: [f64; 4], b: [f64; 4], f: f64) -> [f64; 4] {
	[a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f]
}
