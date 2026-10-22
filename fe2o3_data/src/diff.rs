//! Myers' shortest edit script over any two slices of comparable elements.
//!
//! Moved here from `fe2o3_ore::diff`, which runs it over line and byte pieces,
//! so that any caller wanting a structural diff (a page history, a list of
//! sections) shares the one implementation.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use std::ops::Range;


/// A run of `a` that has become a run of `b`. At least one of the two is
/// non-empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
	pub a:	Range<usize>,	// what the run was, in the old sequence
	pub b:	Range<usize>,	// what it has become, in the new sequence
}


/// Returns the changed runs turning `a` into `b`, or `None` where the edit
/// script would exceed `budget`.
///
/// The shared head and tail are taken off first, since neither costs anything
/// to find and both shrink what Myers is asked to do.
pub fn changed_runs<T: PartialEq>(a: &[T], b: &[T], budget: usize)
	-> Option<Vec<Change>>
{
	let limit = a.len().min(b.len());
	let mut head = 0;
	while head < limit && a[head] == b[head] {
		head += 1;
	}
	let mut tail = 0;
	while tail < limit - head
		&& a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
	{
		tail += 1;
	}
	let (an, bn) = (a.len() - tail, b.len() - tail);
	if head == an && head == bn {
		return Some(Vec::new());
	}
	// Where one side has nothing left, every remaining element of the other is
	// the whole of the change and Myers has nothing to decide.
	if head == an || head == bn {
		return Some(vec![Change { a: head..an, b: head..bn }]);
	}
	let matched = match matches(&a[head..an], &b[head..bn], budget) {
		Some(m)	=> m,
		None	=> return None,
	};
	Some(runs_between(&matched, an - head, bn - head, head))
}

fn runs_between(matched: &[(usize, usize)], n: usize, m: usize, off: usize)
	-> Vec<Change>
{
	let mut out: Vec<Change> = Vec::new();
	let (mut i, mut j) = (0usize, 0usize);
	for (mi, mj) in matched {
		if *mi > i || *mj > j {
			out.push(Change {
				a:	off + i..off + mi,
				b:	off + j..off + mj,
			});
		}
		i = mi + 1;
		j = mj + 1;
	}
	if i < n || j < m {
		out.push(Change {
			a:	off + i..off + n,
			b:	off + j..off + m,
		});
	}
	out
}

/// Returns the pairs of positions a shortest edit script leaves matched,
/// ascending, or `None` where that script would be longer than `budget`.
///
/// This is Myers' greedy algorithm: for each script length `d` in turn, the
/// furthest point reachable on each diagonal is extended along whatever run of
/// equal elements follows it, and the first `d` that reaches the far corner is
/// the length of a shortest script. Keeping the furthest points of every `d`
/// makes the path itself recoverable by walking back through them, which is
/// where the matched pairs come from, and is also what costs O(D²) memory and
/// so what the budget is bounding.
pub fn matches<T: PartialEq>(a: &[T], b: &[T], budget: usize)
	-> Option<Vec<(usize, usize)>>
{
	let (n, m) = (a.len() as isize, b.len() as isize);
	let max = (a.len() + b.len()) as isize;
	let cap = budget.min(a.len() + b.len());
	// Furthest x reached on each diagonal k, shifted so that k = 0 sits at max.
	// The step at d = 0 reads diagonal k = 1, so one slot beyond 2 * max is
	// needed for two empty sides, where max is zero.
	let mut v: Vec<isize> = vec![0; (2 * max + 2) as usize];
	let mut trace: Vec<Vec<isize>> = Vec::with_capacity(cap + 1);
	for d in 0..=cap {
		let dd = d as isize;
		// The state before this step, which is what the walk back reads.
		trace.push(v[(max - dd) as usize..=(max + dd) as usize].to_vec());
		let mut k = -dd;
		while k <= dd {
			let mut x = if k == -dd
				|| (k != dd && v[(max + k - 1) as usize] < v[(max + k + 1) as usize])
			{
				v[(max + k + 1) as usize]
			} else {
				v[(max + k - 1) as usize] + 1
			};
			let mut y = x - k;
			while x < n && y < m && a[x as usize] == b[y as usize] {
				x += 1;
				y += 1;
			}
			v[(max + k) as usize] = x;
			if x >= n && y >= m {
				return Some(walk_back(&trace, n, m, d));
			}
			k += 2;
		}
	}
	None
}

/// Walks the furthest points back from the far corner, collecting the pairs
/// matched on the way.
fn walk_back(trace: &[Vec<isize>], n: isize, m: isize, found: usize)
	-> Vec<(usize, usize)>
{
	let mut matched: Vec<(usize, usize)> = Vec::new();
	let (mut x, mut y) = (n, m);
	for d in (0..=found).rev() {
		let (prev_x, prev_y) = if d == 0 {
			(0, 0)
		} else {
			let dd = d as isize;
			let w = &trace[d];
			let at = |k: isize| w[(k + dd) as usize];
			let k = x - y;
			let prev_k = if k == -dd || (k != dd && at(k - 1) < at(k + 1)) {
				k + 1
			} else {
				k - 1
			};
			let px = at(prev_k);
			(px, px - prev_k)
		};
		while x > prev_x && y > prev_y {
			x -= 1;
			y -= 1;
			matched.push((x as usize, y as usize));
		}
		x = prev_x;
		y = prev_y;
	}
	matched.reverse();
	matched
}
