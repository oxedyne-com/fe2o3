//! The placement layer: slots, how they divide, and how they order.
//!
//! A repository is an ordered set of slots, each claiming a run of content. The
//! order is Fugue's -- slots form a left-child / right-child tree whose in-order
//! traversal is the text -- with one departure: an origin names a content
//! identifier rather than an element, and is resolved through the claim register
//! at render time. That departure is the whole design, because it is what lets
//! an insertion follow the content it was written against when a move takes that
//! content elsewhere, into another file included.
//!
//! # One forest, and a file is a subtree
//!
//! There is one tree for the whole repository rather than one per file, and its
//! root children are exactly the **seed** slots: one per file, claiming that
//! file's origin anchor. A file is the subtree beneath its seed, so a slot's file
//! is read off the tree rather than off the record, and a move between files
//! needs no routing -- its destination anchor already names content in the file
//! it lands in.
//!
//! Resolving origins induces a directed graph over slots, an edge from S to T
//! when S's origin names content T owns. Where that graph is acyclic the order
//! is a function of the operation set and nothing else. Where it is not --
//! two moves whose destinations sit inside each other's sources, or a move whose
//! destination sits inside its own source -- one edge is demoted to the splice
//! that created the anchored content, which is strictly earlier in op order than
//! anything in the cycle and so cannot be in it.
//!
//! Demotion is the answer for a cycle inside one file, and only for that. A cycle
//! that crosses a file boundary is arbitrated instead, by the render, and
//! [`Slots::cycles`] is what tells it where the cycles are.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	Anchor,
	ContentId,
	ContentRange,
	OpId,
	Side,
};
use crate::op::Op;
use crate::seq::claim::Claims;
use crate::seq::OpOrder;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	BTreeSet,
};
use std::fmt;


/// Which of a slot's two origins is meant.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Origin {
	Left,	// what the slot follows
	Right,	// what the slot precedes
}

impl Origin {
	pub const fn code(&self) -> u8 {
		match self {
			Self::Left	=> 0,
			Self::Right	=> 1,
		}
	}

	pub fn from_code(code: u8)
		-> Outcome<Self>
	{
		match code {
			0 => Ok(Self::Left),
			1 => Ok(Self::Right),
			other => Err(err!(
				"An Origin code is 0 for Left or 1 for Right, got {}.", other;
			Decode, Input, Invalid)),
		}
	}
}

impl fmt::Display for Origin {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Left	=> write!(f, "left"),
			Self::Right	=> write!(f, "right"),
		}
	}
}


/// One placed view of a run of content.
///
/// `sub` is the byte offset of this piece within everything its placing
/// operation placed, which is what makes a split arithmetic: both halves keep
/// the placing operation and take `sub` values derived from the split point, so
/// nothing is minted and two replicas that split the same slot at different
/// points compose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot {
	pub place:	OpId,				// the operation that placed the slot
	pub sub:	u64,				// byte offset within its placement
	pub claim:	ContentRange,		// the content the slot shows
	pub left:	Option<Anchor>,		// left origin as recorded
	pub right:	Option<Anchor>,		// right origin as recorded
	pub seed:	bool,				// a file's origin anchor, and a root child
}

impl Slot {
	/// The key by which same-side siblings are ordered: op order of the placing
	/// operation, then offset within that placement.
	pub fn order_key(&self) -> (OpOrder, u64) {
		(OpOrder::of(&self.place), self.sub)
	}
}


/// Every slot an operation set places, divided at every anchor's cut point.
#[derive(Clone, Debug, Default)]
pub struct Slots {
	slots:		Vec<Slot>,					// in placement and division order
	by_place:	BTreeMap<OpId, Vec<usize>>,	// indices by placer, sorted by claim
	prev:		Vec<Option<usize>>,			// the preceding piece of a placement
	placed:		usize,						// slots placed before dividing
}

impl Slots {

	/// Places one seed slot per file, one slot per splice and one per source run
	/// of a move, then divides every slot at every anchor that falls strictly
	/// inside its claim.
	///
	/// Dividing at every anchor, whether or not the anchor is used, is what
	/// makes the division a function of the operation set rather than of the
	/// order the anchors arrived in.
	pub fn place(ops: &[(OpId, &Op)])
		-> Outcome<Self>
	{
		Self::place_without(ops, &BTreeSet::new())
	}

	/// Places the slots as [`Slots::place`] does, except that a move named in
	/// `voided` places none.
	///
	/// A voided move holds no claim either (see [`Claims::build_without`]), so no
	/// origin can resolve to it and nothing is left behind by leaving its slots
	/// out: the bytes render from whoever owns them now, which is where they were
	/// before the move.
	///
	/// **The cut points are still taken from every operation's origins, voided
	/// ones included.** Division has to be a function of the operation set alone,
	/// or two replicas that void at different moments would divide their slots
	/// differently and diverge, and the whole point of the render is that they do
	/// not.
	pub fn place_without(ops: &[(OpId, &Op)], voided: &BTreeSet<OpId>)
		-> Outcome<Self>
	{
		let mut slots: Vec<Slot> = Vec::new();
		for (id, op) in ops {
			if voided.contains(id) && op.is_move() {
				continue;
			}
			match op {
				Op::FileCreate { .. } => {
					slots.push(Slot {
						place:	*id,
						sub:	0,
						claim:	res!(ContentRange::new(*id, 0, 1)),
						left:	None,
						right:	None,
						seed:	true,
					});
				},
				Op::Splice { left, right, insert, .. } => {
					if insert.is_empty() {
						continue;
					}
					slots.push(Slot {
						place:	*id,
						sub:	0,
						claim:	res!(ContentRange::new(*id, 0, insert.len() as u64)),
						left:	*left,
						right:	*right,
						seed:	false,
					});
				},
				Op::Move { src, left, right } => {
					let mut sub = 0u64;
					for r in src {
						if r.is_empty() {
							continue;
						}
						slots.push(Slot {
							place:	*id,
							sub,
							claim:	*r,
							left:	*left,
							right:	*right,
							seed:	false,
						});
						sub += r.len();
					}
				},
				_ => (),
			}
		}
		let placed = slots.len();

		// One cut point in content space per anchor, on the side the anchor
		// binds to.
		let mut cuts: BTreeMap<OpId, BTreeSet<u64>> = BTreeMap::new();
		for (_, op) in ops {
			let (l, r) = op.origins();
			for a in [l, r].into_iter().flatten() {
				let at = match a.side {
					Side::Before	=> a.content.off,
					Side::After		=> a.content.off + 1,
				};
				cuts.entry(a.content.op).or_default().insert(at);
			}
		}

		let mut divided: Vec<Slot> = Vec::with_capacity(slots.len());
		for slot in slots.drain(..) {
			let mut from = slot.claim.from();
			if let Some(set) = cuts.get(&slot.claim.op()) {
				for cut in set.range((slot.claim.from() + 1)..slot.claim.to()) {
					divided.push(Slot {
						place:	slot.place,
						sub:	slot.sub + (from - slot.claim.from()),
						claim:	res!(ContentRange::new(slot.claim.op(), from, *cut)),
						left:	slot.left,
						right:	slot.right,
						seed:	slot.seed,
					});
					from = *cut;
				}
			}
			divided.push(Slot {
				place:	slot.place,
				sub:	slot.sub + (from - slot.claim.from()),
				claim:	res!(ContentRange::new(slot.claim.op(), from, slot.claim.to())),
				left:	slot.left,
				right:	slot.right,
				seed:	slot.seed,
			});
		}
		let slots = divided;
		let n = slots.len();

		// An index for owner lookup, and the chain of pieces of one placement.
		let mut by_place: BTreeMap<OpId, Vec<usize>> = BTreeMap::new();
		for (i, slot) in slots.iter().enumerate() {
			by_place.entry(slot.place).or_default().push(i);
		}
		let mut prev: Vec<Option<usize>> = vec![None; n];
		for idxs in by_place.values_mut() {
			idxs.sort_by_key(|i| (slots[*i].claim.op(), slots[*i].claim.from()));
			let mut chain: Vec<usize> = idxs.clone();
			chain.sort_by_key(|i| slots[*i].sub);
			for pair in chain.windows(2) {
				prev[pair[1]] = Some(pair[0]);
			}
		}

		Ok(Self { slots, by_place, prev, placed })
	}

	/// The slots, in no particular order.
	pub fn all(&self) -> &[Slot] {
		&self.slots
	}

	pub fn get(&self, i: usize)
		-> Outcome<&Slot>
	{
		match self.slots.get(i) {
			Some(s) => Ok(s),
			None => Err(err!(
				"Slot {} does not exist; there are {}.", i, self.slots.len();
			Bug, Index, Range)),
		}
	}

	/// The number of slots after dividing.
	pub fn len(&self) -> usize {
		self.slots.len()
	}

	pub fn is_empty(&self) -> bool {
		self.slots.is_empty()
	}

	/// The number of slots placed before dividing, which is one per splice and one
	/// per source run of a move.
	pub fn placed(&self) -> usize {
		self.placed
	}

	/// How a flag raised against `(operation, offset)` finds the slot it is about,
	/// the pair being what the flags name and what a reader can act on.
	pub fn find(&self, place: &OpId, sub: u64)
		-> Option<usize>
	{
		self.by_place.get(place)
			.and_then(|idxs| idxs.iter().copied().find(|i| self.slots[*i].sub == sub))
	}

	/// The preceding piece of this placement, if this is not the first.
	pub fn prev(&self, i: usize)
		-> Option<usize>
	{
		self.prev.get(i).copied().flatten()
	}

	/// The slot that currently shows the named byte.
	///
	/// With `demoted` set, the claim register is ignored and the slot placed by
	/// the splice that created the byte is returned instead, which is the
	/// fallback the cycle rule falls back to.
	pub fn owner_slot(&self, cid: &ContentId, claims: &Claims, demoted: bool)
		-> Outcome<usize>
	{
		let owner = if demoted { cid.op } else { claims.owner(cid) };
		let idxs = match self.by_place.get(&owner) {
			Some(v) => v,
			None => return Err(err!(
				"No slot placed by {} shows {}; the operation set is not causally \
				complete.", owner, cid;
			Invalid, Input, Missing)),
		};
		// The index is sorted by claimed content, so the slot covering the byte,
		// if there is one, is the last whose claim starts at or before it.
		let key = (cid.op, cid.off);
		let pos = idxs.partition_point(
			|i| (self.slots[*i].claim.op(), self.slots[*i].claim.from()) <= key);
		if pos > 0 {
			let i = idxs[pos - 1];
			if self.slots[i].claim.contains(cid) {
				return Ok(i);
			}
		}
		Err(err!(
			"The slots placed by {} do not cover {}, which they are recorded as \
			showing.", owner, cid;
		Bug, Missing))
	}

	/// Resolves a slot's two origins, honouring the demotion state.
	///
	/// A left origin binds after a byte and a right origin before one. The
	/// reverse is refused rather than guessed at: a left origin bound before a
	/// byte would name the slot preceding that byte's owner, which is not
	/// determinable without first knowing the order the origin is being used to
	/// decide.
	fn origins_of(&self, i: usize, claims: &Claims, dem: &[(bool, bool)])
		-> Outcome<(Option<usize>, Option<usize>)>
	{
		let slot = res!(self.get(i));
		let mut left = None;
		let mut right = None;
		if let Some(a) = slot.left {
			if a.side != Side::After {
				return Err(err!(
					"The left origin {} binds before its byte; a left origin binds \
					after one.", a;
				Invalid, Input));
			}
			left = Some(res!(self.owner_slot(&a.content, claims, dem[i].0)));
		}
		if let Some(a) = slot.right {
			if a.side != Side::Before {
				return Err(err!(
					"The right origin {} binds after its byte; a right origin binds \
					before one.", a;
				Invalid, Input));
			}
			right = Some(res!(self.owner_slot(&a.content, claims, dem[i].1)));
		}
		Ok((left, right))
	}

	/// The cycles of the anchor graph, before anything is demoted.
	///
	/// The graph is the one [`Slots::order`] builds on its first pass, and a cycle
	/// is a strongly connected component with more than one member, or one member
	/// with an edge to itself. Each is returned in op order, and a slot that is not
	/// on a cycle is in none of them.
	///
	/// This is what the cross-file rule needs and demotion does not. Demotion asks
	/// only which slot is blocked, and a blocked slot need not be on a cycle -- it
	/// may merely sit downstream of one -- whereas a rule that voids a whole move
	/// has to name the moves the cycle actually runs through.
	pub fn cycles(&self, claims: &Claims)
		-> Outcome<Vec<Vec<usize>>>
	{
		let n = self.slots.len();
		let dem: Vec<(bool, bool)> = vec![(false, false); n];
		let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
		for (i, dep) in deps.iter_mut().enumerate() {
			if let Some(x) = self.prev(i) {
				dep.push(x);
				continue;
			}
			let (l, r) = res!(self.origins_of(i, claims, &dem));
			for x in [l, r].into_iter().flatten() {
				dep.push(x);
			}
		}
		Ok(self.components(&deps))
	}

	/// The strongly connected components of a dependency graph that are cycles,
	/// each sorted by op order, by Tarjan's algorithm run iteratively.
	fn components(&self, deps: &[Vec<usize>]) -> Vec<Vec<usize>> {
		let n = deps.len();
		// Depth-first index and low link of each slot, and whether it is on the
		// component stack.
		let mut index: Vec<Option<usize>> = vec![None; n];
		let mut low: Vec<usize> = vec![0; n];
		let mut on: Vec<bool> = vec![false; n];
		let mut stack: Vec<usize> = Vec::new();
		let mut next = 0usize;
		let mut out: Vec<Vec<usize>> = Vec::new();
		// The explicit call stack: a slot, and how far through its dependencies the
		// walk had got when it descended.
		let mut work: Vec<(usize, usize)> = Vec::new();
		for root in 0..n {
			if index[root].is_some() {
				continue;
			}
			work.push((root, 0));
			while let Some((v, at)) = work.pop() {
				if at == 0 {
					index[v] = Some(next);
					low[v] = next;
					next += 1;
					stack.push(v);
					on[v] = true;
				}
				let mut descended = false;
				for (k, w) in deps[v].iter().enumerate().skip(at) {
					match index[*w] {
						None => {
							work.push((v, k + 1));
							work.push((*w, 0));
							descended = true;
							break;
						},
						Some(seen) => {
							if on[*w] {
								low[v] = low[v].min(seen);
							}
						},
					}
				}
				if descended {
					continue;
				}
				if Some(low[v]) == index[v] {
					let mut scc: Vec<usize> = Vec::new();
					while let Some(w) = stack.pop() {
						on[w] = false;
						scc.push(w);
						if w == v {
							break;
						}
					}
					if scc.len() > 1 || deps[v].contains(&v) {
						scc.sort_by_key(|i| {
							let (ord, sub) = self.slots[*i].order_key();
							(ord, sub, *i)
						});
						out.push(scc);
					}
				}
				// A finished slot hands its low link back to whoever descended into
				// it, which is the entry now on top of the work stack.
				if let Some((parent, _)) = work.last().copied() {
					low[parent] = low[parent].min(low[v]);
				}
			}
		}
		out.sort_by_key(|scc| scc.first().copied().unwrap_or(0));
		out
	}

	/// Orders the slots topologically over the anchor graph, breaking any cycle
	/// by demotion.
	///
	/// Cycles are broken one edge at a time: the blocked slot lowest in op order
	/// has its left origin demoted to the creating splice, then its right, and
	/// only then are the edges dropped. Each demotion strictly reduces the number
	/// of cycle edges, because the fallback target precedes every slot in the
	/// cycle in op order and so cannot be in it, which is why this terminates.
	///
	/// The graph is rebuilt after each demotion, so the cost is the number of
	/// demotions times the number of slots. Finding the strongly connected
	/// components once and demoting every cycle's lowest edge in a single pass
	/// would reach the same answer for less, and is the obvious thing to do when
	/// this becomes the render's cost centre.
	///
	/// **Every cycle this sees is inside one file.** A cycle that crosses a file
	/// boundary is arbitrated before the order is asked for, by
	/// [`crate::seq::Sequence::render_with`], and its losing moves are voided, so
	/// by the time this runs no such cycle is left. Demoting an origin inside one
	/// file lands a placement at a stale position, which is deterministic,
	/// flagged and safe; demoting one across two would land it in the other file,
	/// and that is the outcome the arbitration exists to prevent.
	pub fn order(&self, claims: &Claims)
		-> Outcome<Order>
	{
		let n = self.slots.len();
		// Whether each origin of each slot has been demoted, and then dropped.
		let mut dem: Vec<(bool, bool)> = vec![(false, false); n];
		let mut cut: Vec<(bool, bool)> = vec![(false, false); n];
		let mut demoted: Vec<(OpId, u64, Origin)> = Vec::new();
		let mut dropped: Vec<(OpId, u64, Origin)> = Vec::new();

		loop {
			// Dependencies under the current demotion state. A slot that is not
			// the first piece of its placement chains to its predecessor and
			// resolves no origins of its own.
			let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
			for i in 0..n {
				if let Some(x) = self.prev(i) {
					deps[i].push(x);
					continue;
				}
				// A self-edge is a cycle of length one, which is what a move
				// whose destination names content the move itself claims
				// produces. It is left in so that the demotion rule sees it;
				// unseen, it would detach the move's slots from the tree and
				// lose their bytes.
				let (l, r) = res!(self.origins_of(i, claims, &dem));
				if let Some(x) = l {
					if !cut[i].0 {
						deps[i].push(x);
					}
				}
				if let Some(x) = r {
					if !cut[i].1 {
						deps[i].push(x);
					}
				}
			}

			// Kahn's algorithm, ties broken by op order then offset within the
			// placement, so the order is a function of the operation set.
			let mut indeg: Vec<usize> = vec![0; n];
			let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
			for i in 0..n {
				indeg[i] = deps[i].len();
				for d in &deps[i] {
					rev[*d].push(i);
				}
			}
			let mut ready: BTreeSet<(OpOrder, u64, usize)> = BTreeSet::new();
			for i in 0..n {
				if indeg[i] == 0 {
					let (ord, sub) = self.slots[i].order_key();
					ready.insert((ord, sub, i));
				}
			}
			let mut order: Vec<usize> = Vec::with_capacity(n);
			while let Some(key) = ready.iter().next().copied() {
				ready.remove(&key);
				let i = key.2;
				order.push(i);
				for j in &rev[i] {
					indeg[*j] -= 1;
					if indeg[*j] == 0 {
						let (ord, sub) = self.slots[*j].order_key();
						ready.insert((ord, sub, *j));
					}
				}
			}

			if order.len() == n {
				let mut left = vec![None; n];
				let mut right = vec![None; n];
				for i in 0..n {
					if self.prev(i).is_some() {
						continue;
					}
					let (l, r) = res!(self.origins_of(i, claims, &dem));
					left[i] = if cut[i].0 { None } else { l };
					right[i] = if cut[i].1 { None } else { r };
				}
				return Ok(Order { order, left, right, demoted, dropped });
			}

			// A cycle remains. Demote the lowest blocked slot in op order.
			let mut stuck: Vec<usize> = (0..n)
				.filter(|i| indeg[*i] > 0 && self.prev(*i).is_none())
				.collect();
			stuck.sort_by_key(|i| {
				let (ord, sub) = self.slots[*i].order_key();
				(ord, sub, *i)
			});
			let victim = match stuck.first() {
				Some(v) => *v,
				None => return Err(err!(
					"The topological sort stalled with no blocked slot, so a cycle \
					runs through slots that chain within a placement."; Bug)),
			};
			let slot = &self.slots[victim];
			if !dem[victim].0 && slot.left.is_some() {
				dem[victim].0 = true;
				demoted.push((slot.place, slot.sub, Origin::Left));
			} else if !dem[victim].1 && slot.right.is_some() {
				dem[victim].1 = true;
				demoted.push((slot.place, slot.sub, Origin::Right));
			} else if !cut[victim].0 && slot.left.is_some() {
				cut[victim].0 = true;
				dropped.push((slot.place, slot.sub, Origin::Left));
			} else if !cut[victim].1 && slot.right.is_some() {
				cut[victim].1 = true;
				dropped.push((slot.place, slot.sub, Origin::Right));
			} else {
				return Err(err!(
					"A cycle through the slot placed by {} at offset {} survives \
					both demotion and dropping.", slot.place, slot.sub;
				Bug));
			}
		}
	}
}


/// A topological order over the anchor graph, with the origins it was resolved
/// against.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Order {
	pub order:		Vec<usize>,					// each after its origins
	pub left:		Vec<Option<usize>>,			// resolved left origins
	pub right:		Vec<Option<usize>>,			// resolved right origins
	// Origins given up to break a cycle, named by placing operation, offset
	// within that placement, and which of the two origins.
	pub demoted:	Vec<(OpId, u64, Origin)>,	// demoted to the creating splice
	pub dropped:	Vec<(OpId, u64, Origin)>,	// dropped where demotion failed
}
