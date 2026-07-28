//! Who owns which byte, and which bytes are dead.
//!
//! Two interval structures over content-identifier space, and between them they
//! settle every race a move can start.
//!
//! The claim register is a last-writer-wins register per byte, keyed by content
//! identifier and valued by the operation whose slot shows that byte. An absent
//! entry means the byte is still shown by the splice that created it, so
//! unmoved content costs nothing to record. Because `max` over a total order is
//! commutative, associative and idempotent, the register is a join-semilattice
//! and needs no arbitration protocol; because the register is per byte rather
//! than per element, a move can claim part of a run, which is what makes a range
//! move expressible at all.
//!
//! The tombstone set is grow-only, so it commutes with everything including a
//! move. A dead byte renders as nothing wherever it is, which is why move
//! against delete needs no tie-break: the bytes move, and they are dead, and
//! both are true at once.

use crate::id::{
	ContentId,
	ContentRange,
	OpId,
};
use crate::seq::atom::Atoms;
use crate::seq::Edit;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::interval::IntervalMap;

use std::collections::BTreeMap;
use std::ops::Range;


/// Which operation's slot owns each byte that has ever been moved.
///
/// One interval line per atom, the line being offsets within that atom, so that
/// a move of a contiguous run costs one interval however long the run is.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Claims {
	/// Claims over each atom's offsets, by creating operation.
	map: BTreeMap<OpId, IntervalMap<OpId>>,
}

impl Claims {

	/// Constructs an empty register, in which every byte is still shown by the
	/// splice that created it.
	pub fn new() -> Self {
		Self { map: BTreeMap::new() }
	}

	/// Builds the register from an operation set given **in ascending op
	/// order**.
	///
	/// Insertion into an interval map is last-writer-wins over the ground it
	/// covers, so feeding the moves in op order leaves each byte claimed by the
	/// highest mover in op order, which is the register's merge rule stated as a
	/// loop.
	pub fn build(ops: &[(OpId, &Edit)])
		-> Outcome<Self>
	{
		let mut map: BTreeMap<OpId, IntervalMap<OpId>> = BTreeMap::new();
		for (id, op) in ops {
			if let Edit::Move { src, .. } = op {
				for r in src {
					if r.is_empty() {
						continue;
					}
					res!(map.entry(r.op()).or_default().insert(r.offsets(), *id));
				}
			}
		}
		Ok(Self { map })
	}

	/// Returns the operation whose slot owns the byte.
	///
	/// A byte no move has claimed is owned by the splice that created it.
	pub fn owner(&self, cid: &ContentId) -> OpId {
		self.map.get(&cid.op)
			.and_then(|m| m.get(cid.off))
			.copied()
			.unwrap_or(cid.op)
	}

	/// Returns the maximal runs of `range` and their owners, in ascending order
	/// of offset.
	///
	/// Runs are maximal because a gap between claims is owned by the creating
	/// splice, which is never a mover, so no two neighbouring runs can share an
	/// owner.
	pub fn runs(&self, range: &ContentRange)
		-> Vec<(Range<u64>, OpId)>
	{
		let mut out: Vec<(Range<u64>, OpId)> = Vec::new();
		let mut at = range.from();
		if let Some(m) = self.map.get(&range.op()) {
			for (iv, owner) in m.overlapping(range.offsets()) {
				let from = iv.start.max(range.from());
				let to = iv.end.min(range.to());
				if from > at {
					out.push((at..from, range.op()));
				}
				if to > from {
					out.push((from..to, *owner));
					at = to;
				}
			}
		}
		if at < range.to() {
			out.push((at..range.to(), range.op()));
		}
		out
	}

	/// Returns the number of intervals the register holds, which is the cost of
	/// every move ever made and of nothing else.
	pub fn intervals(&self) -> usize {
		self.map.values().map(|m| m.len()).sum()
	}
}


/// The bytes that have been deleted.
///
/// A grow-only interval set: deleting a four hundred line block costs one entry,
/// so the count tracks the number of edits rather than the volume of deleted
/// text. Identifiers are kept even where bytes are not, because an anchor may
/// name dead content and routinely does.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Dead {
	/// Dead offsets of each atom, by creating operation.
	map: BTreeMap<OpId, IntervalMap<()>>,
}

impl Dead {

	/// Constructs an empty tombstone set.
	pub fn new() -> Self {
		Self { map: BTreeMap::new() }
	}

	/// Builds the tombstone set from an operation set, in any order.
	pub fn build(ops: &[(OpId, &Edit)])
		-> Outcome<Self>
	{
		let mut map: BTreeMap<OpId, IntervalMap<()>> = BTreeMap::new();
		for (_, op) in ops {
			if let Edit::Splice { remove, .. } = op {
				for r in remove {
					if r.is_empty() {
						continue;
					}
					res!(map.entry(r.op()).or_default().insert(r.offsets(), ()));
				}
			}
		}
		Ok(Self { map })
	}

	/// Reports whether the byte has been deleted.
	pub fn is_dead(&self, cid: &ContentId) -> bool {
		self.map.get(&cid.op).map(|m| m.contains(cid.off)).unwrap_or(false)
	}

	/// Returns the live sub-runs of `span` within the atom created by `op`, in
	/// ascending order.
	pub fn live_runs(&self, op: &OpId, span: Range<u64>)
		-> Vec<Range<u64>>
	{
		let mut out: Vec<Range<u64>> = Vec::new();
		let mut at = span.start;
		if let Some(m) = self.map.get(op) {
			for (iv, _) in m.overlapping(span.clone()) {
				let from = iv.start.max(span.start);
				let to = iv.end.min(span.end);
				if from > at {
					out.push(at..from);
				}
				at = at.max(to);
			}
		}
		if at < span.end {
			out.push(at..span.end);
		}
		out
	}

	/// Returns the number of dead bytes that lie within an atom the operation
	/// set holds.
	///
	/// A tombstone naming content beyond the end of its atom, or naming an atom
	/// that is not present, is not counted: it describes bytes this set cannot
	/// account for, and the conservation check must not be told otherwise.
	pub fn within(&self, atoms: &Atoms) -> u64 {
		let mut total = 0u64;
		for (op, m) in &self.map {
			let len = atoms.run_len(op);
			for (iv, _) in m.iter() {
				let to = iv.end.min(len);
				if to > iv.start {
					total += to - iv.start;
				}
			}
		}
		total
	}

	/// Returns the number of intervals the set holds.
	pub fn intervals(&self) -> usize {
		self.map.values().map(|m| m.len()).sum()
	}
}
