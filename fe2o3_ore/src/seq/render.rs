//! Turning the ordered slots back into bytes, and saying what happened.
//!
//! The tree is built in the topological order of the anchor graph and walked in
//! order, and each slot emits the parts of its claim that it still owns and that
//! are still alive. Everything the walk noticed is returned with the bytes as a
//! [`Flag`], because a structure that always converges owes the reader an
//! account of what it converged to: a torn move, an anchor demoted to break a
//! cycle, or two operations that named the same content. Flags are facts derived
//! from the operation set, not a log of what the renderer happened to do, so two
//! replicas holding the same operations report the same flags.

use crate::id::{
	Anchor,
	ContentId,
	ContentRange,
	OpId,
};
use crate::seq::atom::Atoms;
use crate::seq::claim::{
	Claims,
	Dead,
};
use crate::seq::slot::{
	Order,
	Origin,
	Slots,
};
use crate::seq::{
	Edit,
	OpOrder,
};

use oxedyne_fe2o3_core::prelude::*;


/// Something the renderer noticed that the reader should be told.
///
/// Every flag is a function of the operation set, so the same set flags the same
/// things everywhere. None of them means the render failed; each means the
/// render made a choice that a person might want to revisit.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Flag {
	/// A move whose source is no longer wholly its own: a higher move in op
	/// order took part of it, so the block tore at the overlap and its pieces
	/// render in two places.
	Torn {
		/// The move that lost ground.
		op:		OpId,
		/// The content it named and no longer shows.
		lost:	Vec<ContentRange>,
	},
	/// An origin resolved against the splice that created its content rather
	/// than against the slot that now shows it, to break a cycle.
	///
	/// The consequence is that the placement landed where its anchor content was
	/// originally written rather than where it now lives, which is deterministic
	/// and surprising in equal measure.
	Demoted {
		/// The operation whose origin was demoted.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
		/// Which of the two origins.
		origin:	Origin,
	},
	/// An origin dropped entirely because demotion did not break the cycle, so
	/// the placement fell back to the start or the end of the file.
	Dropped {
		/// The operation whose origin was dropped.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
		/// Which of the two origins.
		origin:	Origin,
	},
	/// Two concurrent operations named overlapping content: both removed it,
	/// both moved it, or one removed what the other moved.
	///
	/// Concurrency is decided from the operations' own parents, so the flag now
	/// means what its name says: neither author could see what the other was
	/// doing. Two operations touching the same bytes where one was written in
	/// knowledge of the other are a sequence of edits and not a conflict, and
	/// raise nothing.
	Overlap {
		/// The operations involved, in ascending order of identifier.
		ops:	Vec<OpId>,
		/// The content they have in common.
		region:	ContentRange,
	},
}


/// What a render cost, in the terms the cost model is stated in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
	/// Operations in the set.
	pub ops:			usize,
	/// Atoms created.
	pub atoms:			usize,
	/// Bytes held in atoms, alive or dead.
	pub atom_bytes:		u64,
	/// Slots placed, one per splice and one per source run of a move.
	pub slots_placed:	usize,
	/// Slots after dividing at anchors.
	pub slots_divided:	usize,
	/// Intervals in the claim register, which is the standing cost of every move
	/// ever made.
	pub claim_intervals:	usize,
	/// Intervals in the tombstone set.
	pub dead_intervals:	usize,
	/// Deepest path in the Fugue tree.
	pub max_depth:		u32,
	/// Bytes rendered.
	pub rendered:		u64,
}


/// A run of rendered bytes, and the content it shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Run {
	/// Offset in the rendered bytes at which the run begins.
	pub at:			u64,
	/// The content the run shows.
	pub content:	ContentRange,
}


/// The bytes a render produced, what they are made of, and what the renderer
/// noticed while producing them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Rendered {
	/// The rendered bytes.
	bytes:	Vec<u8>,
	/// Provenance, in render order and coalesced.
	runs:	Vec<Run>,
	/// What the renderer noticed.
	flags:	Vec<Flag>,
	/// What the render cost.
	stats:	Stats,
}

impl Rendered {

	/// Assembles a render from its parts.
	pub(super) fn new(bytes: Vec<u8>, runs: Vec<Run>, flags: Vec<Flag>, stats: Stats) -> Self {
		Self { bytes, runs, flags, stats }
	}

	/// Returns the rendered bytes.
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Returns the provenance of the rendered bytes, in render order.
	///
	/// Runs are maximal: a run continues for as long as the content it shows is
	/// contiguous, whatever the slot structure underneath.
	pub fn runs(&self) -> &[Run] {
		&self.runs
	}

	/// Returns what the renderer noticed.
	pub fn flags(&self) -> &[Flag] {
		&self.flags
	}

	/// Returns what the render cost.
	pub fn stats(&self) -> &Stats {
		&self.stats
	}

	/// Returns the number of bytes rendered.
	pub fn len(&self) -> usize {
		self.bytes.len()
	}

	/// Reports whether nothing was rendered.
	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}

	/// Returns the rendered bytes as a string, with anything that is not valid
	/// UTF-8 replaced. For messages and tests; the bytes themselves are the
	/// record.
	pub fn text_lossy(&self) -> String {
		String::from_utf8_lossy(&self.bytes).into_owned()
	}

	/// Returns the content identifier of the byte at a rendered index.
	pub fn content_at(&self, index: usize)
		-> Outcome<ContentId>
	{
		let at = index as u64;
		let pos = self.runs.partition_point(|r| r.at <= at);
		if pos > 0 {
			let run = self.runs[pos - 1];
			if at < run.at + run.content.len() {
				return Ok(ContentId::new(run.content.op, run.content.from + (at - run.at)));
			}
		}
		Err(err!(
			"Rendered index {} is beyond the {} bytes rendered.", index, self.bytes.len();
		Invalid, Input, Range))
	}

	/// Returns the content a rendered span is made of, as the fewest runs that
	/// name it.
	pub fn span(&self, at: usize, len: usize)
		-> Outcome<Vec<ContentRange>>
	{
		let end = match at.checked_add(len) {
			Some(e) => e,
			None => return Err(err!(
				"A span of {} bytes at index {} overflows.", len, at;
			Invalid, Input, Overflow)),
		};
		if end > self.bytes.len() {
			return Err(err!(
				"A span of {}..{} reaches beyond the {} bytes rendered.",
				at, end, self.bytes.len();
			Invalid, Input, Range));
		}
		// Runs are already maximal, so no two of them can be joined and the walk
		// is one step per run touched.
		let mut out: Vec<ContentRange> = Vec::new();
		let mut pos = at as u64;
		let end = end as u64;
		let mut next = self.runs.partition_point(|r| r.at <= pos);
		while pos < end {
			if next == 0 {
				return Err(err!(
					"Rendered index {} lies in no run, though {} bytes were \
					rendered.", pos, self.bytes.len();
				Bug, Missing));
			}
			let run = self.runs[next - 1];
			let within = pos - run.at;
			if within >= run.content.len() {
				return Err(err!(
					"Rendered index {} falls in the gap after the run at {}; runs \
					must cover the render.", pos, run.at;
				Bug, Missing));
			}
			let take = (run.content.len() - within).min(end - pos);
			out.push(res!(ContentRange::new(
				run.content.op,
				run.content.from + within,
				run.content.from + within + take,
			)));
			pos += take;
			next += 1;
		}
		Ok(out)
	}

	/// Returns the two origins bracketing the gap at a rendered index.
	///
	/// The left origin binds after the byte before the gap and the right origin
	/// before the byte after it; an origin at the start or the end of the file is
	/// absent, there being no byte to name it by.
	pub fn gap(&self, at: usize)
		-> Outcome<(Option<Anchor>, Option<Anchor>)>
	{
		if at > self.bytes.len() {
			return Err(err!(
				"The gap at index {} is beyond the {} bytes rendered.",
				at, self.bytes.len();
			Invalid, Input, Range));
		}
		let left = if at > 0 {
			Some(Anchor::after(res!(self.content_at(at - 1))))
		} else {
			None
		};
		let right = if at < self.bytes.len() {
			Some(Anchor::before(res!(self.content_at(at))))
		} else {
			None
		};
		Ok((left, right))
	}

	/// Builds a content-anchored splice from index-based editing intent:
	/// replace `len` bytes at `at` with `insert`.
	///
	/// This is the bridge a frontend crosses. An editor knows where the cursor
	/// is; the structure knows only what the bytes are called, and the render is
	/// the one place both are known at once.
	pub fn splice(&self, at: usize, len: usize, insert: Vec<u8>)
		-> Outcome<Edit>
	{
		let remove = res!(self.span(at, len));
		// A splice inserting nothing places no slot, so its origins would say
		// nothing about where anything goes.
		let (left, right) = if insert.is_empty() {
			(None, None)
		} else {
			res!(self.gap(at))
		};
		Ok(Edit::Splice { left, right, remove, insert })
	}

	/// Builds a content-anchored move from index-based editing intent: take
	/// `len` bytes at `at` to the gap at `to`.
	pub fn move_range(&self, at: usize, len: usize, to: usize)
		-> Outcome<Edit>
	{
		let src = res!(self.span(at, len));
		let (left, right) = res!(self.gap(to));
		Ok(Edit::Move { src, left, right })
	}
}


/// Which side of its parent a node sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildSide {
	/// Visited before the parent.
	Left,
	/// Visited after the parent.
	Right,
}


/// The bytes a traversal produced, and what it cost.
pub(super) struct Traversal {
	/// The rendered bytes.
	pub bytes:		Vec<u8>,
	/// Provenance, in render order and coalesced.
	pub runs:		Vec<Run>,
	/// Deepest path in the tree.
	pub max_depth:	u32,
}

/// Builds the Fugue tree in topological order and walks it in order.
///
/// Where a slot's two origins are still adjacent the published rule applies
/// unchanged. Where a move has separated them, the rule is re-run against the
/// left origin's current in-order successor, which is Fugue's own Algorithm 1
/// with "the next element" read at render time rather than taken from the
/// recorded anchor. Without that, an insertion abutting a moved range lands at
/// the far end of its left origin's subtree, which for a document of any size is
/// the end of the file.
pub(super) fn traverse(
	slots:	&Slots,
	ord:	&Order,
	claims:	&Claims,
	dead:	&Dead,
	atoms:	&Atoms,
)
	-> Outcome<Traversal>
{
	let sl = slots.all();
	let n = sl.len();
	if n >= u32::MAX as usize {
		return Err(err!(
			"A file of {} slots exceeds what the tree's indices can address.", n;
		Excessive, Size));
	}
	let root = n;
	// Ancestor jumps for the subtree test, in powers of two.
	let mut log = 1usize;
	while (1usize << log) <= n {
		log += 1;
	}
	let mut parent: Vec<u32> = vec![root as u32; n + 1];
	let mut side: Vec<ChildSide> = vec![ChildSide::Right; n + 1];
	let mut depth: Vec<u32> = vec![0; n + 1];
	let mut up: Vec<u32> = vec![root as u32; (n + 1) * log];
	let mut kids_l: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
	let mut kids_r: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
	let mut max_depth = 0u32;

	for &i in &ord.order {
		// A piece that is not the first of its placement hangs off its
		// predecessor, so a divided slot stays in one piece in the order.
		let (par, sd) = match slots.prev(i) {
			Some(prev) => (prev, ChildSide::Right),
			None => match (ord.left[i], ord.right[i]) {
				(None, None)		=> (root, ChildSide::Right),
				(None, Some(r))		=> (r, ChildSide::Left),
				(Some(l), None)		=> (l, ChildSide::Right),
				(Some(l), Some(r))	=> {
					if in_right_subtree(l, r, &depth, &up, &parent, &side, log) {
						(r, ChildSide::Left)
					} else {
						// The origins have been torn apart by a move, so the
						// recorded right origin says nothing about where this
						// belongs. The left origin's successor does.
						match successor(l, &kids_l, &kids_r, &parent, &side, root) {
							Some(s) if in_right_subtree(
								l, s, &depth, &up, &parent, &side, log)
								=> (s, ChildSide::Left),
							_	=> (l, ChildSide::Right),
						}
					}
				},
			},
		};
		if par == i {
			return Err(err!(
				"The slot placed by {} at offset {} resolved to itself as its own \
				parent.", sl[i].place, sl[i].sub;
			Bug));
		}
		parent[i] = par as u32;
		side[i] = sd;
		depth[i] = depth[par] + 1;
		max_depth = max_depth.max(depth[i]);
		up[i * log] = par as u32;
		for k in 1..log {
			up[i * log + k] = up[up[i * log + k - 1] as usize * log + k - 1];
		}
		// Same-side siblings sit in op order, then in placement offset, which is
		// Fugue's sibling rule and the last of the five tie-breaks.
		let key = (OpOrder::of(&sl[i].place), sl[i].sub, i);
		let list = match sd {
			ChildSide::Left		=> &mut kids_l[par],
			ChildSide::Right	=> &mut kids_r[par],
		};
		let pos = list.partition_point(
			|j| (OpOrder::of(&sl[*j].place), sl[*j].sub, *j) < key);
		list.insert(pos, i);
	}

	let mut bytes: Vec<u8> = Vec::new();
	let mut runs: Vec<Run> = Vec::new();
	let mut stack: Vec<(usize, bool)> = vec![(root, false)];
	while let Some((i, emit)) = stack.pop() {
		if emit {
			if i == root {
				continue;
			}
			let slot = &sl[i];
			// A slot shows the parts of its claim it still owns, minus whatever
			// has died. A slot that has lost all of its claim shows nothing and
			// stays as an anchor target.
			for (span, owner) in claims.runs(&slot.claim) {
				if owner != slot.place {
					continue;
				}
				for live in dead.live_runs(&slot.claim.op, span.clone()) {
					let run = res!(ContentRange::new(slot.claim.op, live.start, live.end));
					let at = bytes.len() as u64;
					bytes.extend_from_slice(res!(atoms.slice(&run)));
					match runs.last_mut() {
						Some(last) if last.content.op == run.op
							&& last.content.to == run.from
							=> last.content.to = run.to,
						_ => runs.push(Run { at, content: run }),
					}
				}
			}
			continue;
		}
		for c in kids_r[i].iter().rev() {
			stack.push((*c, false));
		}
		stack.push((i, true));
		for c in kids_l[i].iter().rev() {
			stack.push((*c, false));
		}
	}

	Ok(Traversal { bytes, runs, max_depth })
}

/// Returns the in-order successor of `v` among the nodes placed so far.
fn successor(
	v:		usize,
	kids_l:	&[Vec<usize>],
	kids_r:	&[Vec<usize>],
	parent:	&[u32],
	side:	&[ChildSide],
	root:	usize,
)
	-> Option<usize>
{
	if let Some(c) = kids_r[v].first() {
		let mut cur = *c;
		while let Some(x) = kids_l[cur].first() {
			cur = *x;
		}
		return Some(cur);
	}
	let mut cur = v;
	loop {
		let p = parent[cur] as usize;
		if p == cur || p == root {
			return None;
		}
		if side[cur] == ChildSide::Left {
			return Some(p);
		}
		cur = p;
	}
}

/// Reports whether `r` lies in the right subtree of `l`, by climbing `r`'s
/// ancestors in powers of two.
fn in_right_subtree(
	l:		usize,
	r:		usize,
	depth:	&[u32],
	up:		&[u32],
	parent:	&[u32],
	side:	&[ChildSide],
	log:	usize,
)
	-> bool
{
	if depth[r] <= depth[l] {
		return false;
	}
	// Climb to the child of `l`'s depth, then ask whether that is `l`'s right
	// child.
	let mut climb = depth[r] - depth[l] - 1;
	let mut cur = r;
	let mut k = 0usize;
	while climb > 0 && k < log {
		if climb & 1 == 1 {
			cur = up[cur * log + k] as usize;
		}
		climb >>= 1;
		k += 1;
	}
	parent[cur] as usize == l && side[cur] == ChildSide::Right
}
