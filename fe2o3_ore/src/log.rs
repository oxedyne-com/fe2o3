//! An append-only log of operations.
//!
//! The log is the history. Nothing is ever rewritten in place: an edit that
//! undoes an earlier one is itself an operation appended after it, so the
//! record of what happened is never lost. The log holds its entries in memory
//! and does no I/O of its own; where the bytes live is the caller's business.
//!
//! Each replica's counters must arrive strictly increasing. Gaps are permitted,
//! because a replica may not have shipped everything it has authored, but going
//! backwards is refused: a counter already spent cannot be spent again, and
//! silently accepting it would let one identifier name two different edits.

use crate::id::{
	OpId,
	ReplicaId,
};
use crate::op::Op;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;


/// An append-only, in-memory log of identified operations.
///
/// Entries keep their append order, which is the order the log was told about
/// them; it is not a causal order across replicas, and the log does not attempt
/// to impose one.
#[derive(Clone, Debug, Default)]
pub struct OpLog {
	/// Entries in the order they were appended.
	entries:	Vec<(OpId, Op)>,
	/// Position in `entries` of each identifier.
	index:		HashMap<OpId, usize>,
	/// Highest counter accepted so far for each replica.
	heads:		HashMap<ReplicaId, u64>,
}

impl OpLog {
	/// Constructs an empty log.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the number of operations in the log.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Reports whether the log holds no operations.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// Appends an operation under the given identifier.
	///
	/// Fails if the counter is zero, if the identifier is already present, or
	/// if the counter does not exceed the highest already accepted for that
	/// replica.
	pub fn append(&mut self, id: OpId, op: Op)
		-> Outcome<()>
	{
		if id.counter == 0 {
			return Err(err!(
				"Operation counters start at one, so {} is not a valid identifier.", id;
			Invalid, Input, Counter));
		}
		if self.index.contains_key(&id) {
			return Err(err!(
				"The log already holds an operation identified {}.", id;
			Invalid, Input, Duplicate));
		}
		if let Some(head) = self.heads.get(&id.replica) {
			if id.counter <= *head {
				return Err(err!(
					"Replica {} is at counter {}, so {} does not advance it; counters \
					must strictly increase.", id.replica, head, id;
				Invalid, Input, Order, Counter));
			}
		}
		self.index.insert(id, self.entries.len());
		self.heads.insert(id.replica, id.counter);
		self.entries.push((id, op));
		Ok(())
	}

	/// Returns the operation named by `id`, if the log holds it.
	pub fn get(&self, id: &OpId) -> Option<&Op> {
		self.index.get(id).and_then(|i| self.entries.get(*i)).map(|(_, op)| op)
	}

	/// Reports whether the log holds the operation named by `id`.
	pub fn contains(&self, id: &OpId) -> bool {
		self.index.contains_key(id)
	}

	/// Returns the entry at the given append position.
	pub fn at(&self, pos: usize) -> Option<&(OpId, Op)> {
		self.entries.get(pos)
	}

	/// Returns the append position of the operation named by `id`.
	pub fn position(&self, id: &OpId) -> Option<usize> {
		self.index.get(id).copied()
	}

	/// Iterates over the entries in append order.
	pub fn iter(&self) -> impl Iterator<Item = &(OpId, Op)> {
		self.entries.iter()
	}

	/// Iterates over the entries authored by one replica, in append order.
	pub fn iter_replica(&self, replica: ReplicaId) -> impl Iterator<Item = &(OpId, Op)> {
		self.entries.iter().filter(move |(id, _)| id.replica == replica)
	}

	/// Returns the highest counter accepted for `replica`, if it has authored
	/// anything the log has seen.
	pub fn head(&self, replica: ReplicaId) -> Option<u64> {
		self.heads.get(&replica).copied()
	}

	/// Returns the identifier a replica should use for its next operation,
	/// which is one past its current head.
	pub fn next_id(&self, replica: ReplicaId) -> OpId {
		OpId::new(replica, self.head(replica).unwrap_or(0) + 1)
	}

	/// Returns the highest counter seen per replica: the log's frontier, which
	/// is what a peer needs in order to work out what to send.
	pub fn frontier(&self) -> Vec<(ReplicaId, u64)> {
		let mut v: Vec<(ReplicaId, u64)> = self.heads
			.iter()
			.map(|(r, c)| (*r, *c))
			.collect();
		v.sort();
		v
	}

	/// Appends an operation authored by `replica`, minting the next identifier
	/// for it, and returns that identifier.
	pub fn author(&mut self, replica: ReplicaId, op: Op)
		-> Outcome<OpId>
	{
		let id = self.next_id(replica);
		res!(self.append(id, op));
		Ok(id)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A distinguishable operation, for tests that only need entries to differ.
	fn mark(name: &str) -> Op {
		Op::Mark { name: fmt!("{}", name) }
	}

	/// A fresh log is empty and knows nothing of any replica.
	#[test]
	fn empty_log_is_empty() -> Outcome<()> {
		let log = OpLog::new();
		assert!(log.is_empty());
		assert_eq!(log.len(), 0);
		assert_eq!(log.head(ReplicaId::new(1)), None);
		assert_eq!(log.next_id(ReplicaId::new(1)), OpId::new(ReplicaId::new(1), 1));
		assert!(log.frontier().is_empty());
		Ok(())
	}

	/// Appended operations are retrievable by identifier and in append order.
	#[test]
	fn append_then_look_up() -> Outcome<()> {
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		let a = OpId::new(r1, 1);
		let b = OpId::new(r2, 1);
		let c = OpId::new(r1, 2);
		res!(log.append(a, mark("a")));
		res!(log.append(b, mark("b")));
		res!(log.append(c, mark("c")));
		assert_eq!(log.len(), 3);
		assert_eq!(log.get(&b), Some(&mark("b")));
		assert!(log.contains(&c));
		assert_eq!(log.position(&c), Some(2));
		let order: Vec<OpId> = log.iter().map(|(id, _)| *id).collect();
		assert_eq!(order, vec![a, b, c]);
		Ok(())
	}

	/// An identifier the log has never seen is absent, not an error.
	#[test]
	fn lookup_of_an_absent_id_is_none() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(OpId::new(ReplicaId::new(1), 1), mark("a")));
		let missing = OpId::new(ReplicaId::new(9), 4);
		assert_eq!(log.get(&missing), None);
		assert!(!log.contains(&missing));
		assert_eq!(log.position(&missing), None);
		Ok(())
	}

	/// Counters must strictly increase per replica; repeating or going
	/// backwards is refused.
	#[test]
	fn counters_must_increase_per_replica() -> Outcome<()> {
		let r = ReplicaId::new(7);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r, 1), mark("a")));
		res!(log.append(OpId::new(r, 2), mark("b")));
		// Backwards.
		assert!(log.append(OpId::new(r, 1), mark("c")).is_err());
		// Equal to the head.
		assert!(log.append(OpId::new(r, 2), mark("d")).is_err());
		// A gap forwards is allowed: the replica may not have shipped 3.
		res!(log.append(OpId::new(r, 9), mark("e")));
		assert_eq!(log.head(r), Some(9));
		// And now anything at or below 9 is refused.
		assert!(log.append(OpId::new(r, 5), mark("f")).is_err());
		assert_eq!(log.len(), 3);
		Ok(())
	}

	/// The guard is per replica: one replica's counters do not constrain
	/// another's.
	#[test]
	fn counters_are_independent_across_replicas() -> Outcome<()> {
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r1, 100), mark("a")));
		res!(log.append(OpId::new(r2, 1), mark("b")));
		assert_eq!(log.head(r1), Some(100));
		assert_eq!(log.head(r2), Some(1));
		Ok(())
	}

	/// A zero counter is refused, since counters start at one.
	#[test]
	fn zero_counter_is_refused() -> Outcome<()> {
		let mut log = OpLog::new();
		assert!(log.append(OpId::new(ReplicaId::new(1), 0), mark("a")).is_err());
		assert!(log.is_empty());
		Ok(())
	}

	/// A rejected append leaves the log exactly as it was.
	#[test]
	fn a_rejected_append_changes_nothing() -> Outcome<()> {
		let r = ReplicaId::new(3);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r, 4), mark("a")));
		let before = log.frontier();
		assert!(log.append(OpId::new(r, 4), mark("b")).is_err());
		assert_eq!(log.len(), 1);
		assert_eq!(log.frontier(), before);
		assert_eq!(log.get(&OpId::new(r, 4)), Some(&mark("a")));
		Ok(())
	}

	/// Authoring mints consecutive identifiers for a replica.
	#[test]
	fn author_mints_consecutive_ids() -> Outcome<()> {
		let r = ReplicaId::new(5);
		let mut log = OpLog::new();
		let a = res!(log.author(r, mark("a")));
		let b = res!(log.author(r, mark("b")));
		assert_eq!(a, OpId::new(r, 1));
		assert_eq!(b, OpId::new(r, 2));
		assert_eq!(log.next_id(r), OpId::new(r, 3));
		Ok(())
	}

	/// Per-replica iteration returns only that replica's entries, in order.
	#[test]
	fn iter_replica_filters() -> Outcome<()> {
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r1, 1), mark("a")));
		res!(log.append(OpId::new(r2, 1), mark("b")));
		res!(log.append(OpId::new(r1, 2), mark("c")));
		let got: Vec<OpId> = log.iter_replica(r1).map(|(id, _)| *id).collect();
		assert_eq!(got, vec![OpId::new(r1, 1), OpId::new(r1, 2)]);
		assert_eq!(log.iter_replica(ReplicaId::new(9)).count(), 0);
		Ok(())
	}

	/// The frontier reports one head per replica, sorted by replica.
	#[test]
	fn frontier_reports_one_head_per_replica() -> Outcome<()> {
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r2, 3), mark("a")));
		res!(log.append(OpId::new(r1, 1), mark("b")));
		res!(log.append(OpId::new(r2, 8), mark("c")));
		assert_eq!(log.frontier(), vec![(r1, 1), (r2, 8)]);
		Ok(())
	}

	/// Entries are addressable by append position.
	#[test]
	fn entries_are_addressable_by_position() -> Outcome<()> {
		let r = ReplicaId::new(1);
		let mut log = OpLog::new();
		res!(log.append(OpId::new(r, 1), mark("a")));
		res!(log.append(OpId::new(r, 2), mark("b")));
		match log.at(1) {
			Some((id, op)) => {
				assert_eq!(*id, OpId::new(r, 2));
				assert_eq!(*op, mark("b"));
			},
			None => return Err(err!("Expected an entry at position 1."; Test, Missing)),
		}
		assert!(log.at(2).is_none());
		Ok(())
	}
}
