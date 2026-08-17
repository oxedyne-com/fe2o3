//! An append-only log of operations, and the causal graph they form.
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
//!
//! # Counters are a Lamport clock
//!
//! [`OpLog::next_counter`] mints one past the greatest counter the log has seen
//! from *any* replica, not one past the authoring replica's own. That is what
//! makes the op order in [`crate::seq`] -- the pair `(counter, replica)`
//! ascending -- mean what it is read as meaning: an edit written in knowledge of
//! another orders after it, whoever wrote it and whatever the two replica
//! numbers are. Minting per replica would leave two edits at the same counter
//! with the lower replica number winning, which is the opposite answer whenever
//! the higher-numbered replica wrote first.
//!
//! Nothing is given up by it. A replica's own counters still strictly increase,
//! because the greatest counter the log has seen is at least that replica's own
//! head, so the guard [`OpLog::append`] applies is unaffected; what the replica
//! loses is only that its counters are no longer consecutive, and gaps were
//! always permitted. Two replicas editing concurrently still mint the same
//! counter, and the replica number breaks that tie, which is all it is for.
//!
//! # Causality
//!
//! Every record names the frontier its author could see, so the log is a
//! directed acyclic graph and not merely a list. [`OpLog::append`] refuses a
//! record whose parents are not all present, which is the simplest rule that is
//! correct: a log that accepted an operation before its parents would be unable
//! to say what was concurrent with what, and the sequence structure would be
//! rendering a set it had been told was complete when it was not.
//!
//! Refusal alone would make an out-of-order delivery painful, so
//! [`OpLog::absorb`] takes a batch in any order, places everything it can, and
//! hands back what it could not. That is the whole of the buffering story: the
//! log holds nothing it has not accepted, and a caller that wants a pending
//! queue keeps the leftovers and offers them again.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	OpId,
	ReplicaId,
};
use crate::op::{
	Header,
	Op,
	Record,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	BTreeSet,
	HashMap,
	HashSet,
};


/// The parent graph over a set of operations, and the ancestry questions it
/// answers.
///
/// Built by borrowing each operation's parents rather than copying them, so
/// asking a question of a large history costs one map and no duplication of the
/// graph. Every question is asked by walking parents, which terminates because
/// an operation may not name itself and counters strictly increase along any
/// path a replica writes.
#[derive(Clone, Debug, Default)]
pub struct Causality<'a> {
	parents: BTreeMap<OpId, &'a [OpId]>, // each operation's parents, by operation
}

impl<'a> Causality<'a> {

	pub fn new<I>(ops: I) -> Self
	where
		I: IntoIterator<Item = (OpId, &'a [OpId])>,
	{
		Self { parents: ops.into_iter().collect() }
	}

	pub fn len(&self) -> usize {
		self.parents.len()
	}

	pub fn is_empty(&self) -> bool {
		self.parents.is_empty()
	}

	pub fn contains(&self, id: &OpId) -> bool {
		self.parents.contains_key(id)
	}

	pub fn parents_of(&self, id: &OpId)
		-> Option<&'a [OpId]>
	{
		self.parents.get(id).copied()
	}

	/// The first parent the graph does not hold, and the operation that named it.
	///
	/// `None` means the set is causally closed: nothing in it points outside
	/// itself.
	pub fn gap(&self)
		-> Option<(OpId, OpId)>
	{
		for (id, parents) in &self.parents {
			for p in parents.iter() {
				if !self.parents.contains_key(p) {
					return Some((*id, *p));
				}
			}
		}
		None
	}

	pub fn is_closed(&self) -> bool {
		self.gap().is_none()
	}

	/// The operations nobody in the set names as a parent.
	///
	/// This is the frontier an author writes against, in ascending order of
	/// identifier.
	pub fn heads(&self) -> Vec<OpId> {
		let mut named: BTreeSet<OpId> = BTreeSet::new();
		for parents in self.parents.values() {
			for p in parents.iter() {
				named.insert(*p);
			}
		}
		self.parents.keys().filter(|id| !named.contains(id)).copied().collect()
	}

	/// Is `target` either `from` itself or one of its ancestors -- was `from`
	/// written in knowledge of `target`?
	///
	/// A parent the graph does not hold ends that branch of the walk; the answer
	/// is then only as good as the set, which is why a caller that needs it to
	/// mean something checks [`Causality::is_closed`] first.
	pub fn reaches(&self, from: &OpId, target: &OpId) -> bool {
		if from == target {
			return true;
		}
		let mut seen: HashSet<OpId> = HashSet::new();
		let mut stack: Vec<OpId> = vec![*from];
		while let Some(id) = stack.pop() {
			let parents = match self.parents.get(&id) {
				Some(p) => *p,
				None => continue,
			};
			for p in parents {
				if p == target {
					return true;
				}
				if seen.insert(*p) {
					stack.push(*p);
				}
			}
		}
		false
	}

	/// Distinct, and neither written in knowledge of the other?
	pub fn concurrent(&self, a: &OpId, b: &OpId) -> bool {
		a != b && !self.reaches(a, b) && !self.reaches(b, a)
	}
}


/// An append-only, in-memory log of operations.
///
/// Entries keep their append order, which is the order the log was told about
/// them. That is not a causal order, and the log does not impose one; what it
/// does insist on is that an operation arrives after everything it was written
/// against, so the append order is always *a* linear extension of the causal
/// order.
#[derive(Clone, Debug, Default)]
pub struct OpLog {
	entries:	Vec<Record>,				// in append order
	index:		HashMap<OpId, usize>,		// position of each identifier
	counters:	HashMap<ReplicaId, u64>,	// highest counter per replica
	top:		u64,						// greatest counter seen anywhere
	named:		HashSet<OpId>,				// identifiers named as a parent
}

impl OpLog {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn len(&self) -> usize {
		self.entries.len()
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// Refusing a record whose parents are absent is what keeps the log causally
	/// closed at every moment, and the error names the operation that is missing
	/// so a caller can go and fetch it.
	pub fn append(&mut self, rec: Record)
		-> Outcome<()>
	{
		let id = rec.head.id();
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
		if let Some(head) = self.counters.get(&id.replica) {
			if id.counter <= *head {
				return Err(err!(
					"Replica {} is at counter {}, so {} does not advance it; counters \
					must strictly increase.", id.replica, head, id;
				Invalid, Input, Order, Counter));
			}
		}
		for p in rec.parents() {
			if !self.index.contains_key(p) {
				return Err(err!(
					"The operation {} names the parent {}, which the log does not \
					hold; an operation may not arrive before its parents.", id, p;
				Invalid, Input, Missing, Order));
			}
		}
		self.index.insert(id, self.entries.len());
		self.counters.insert(id.replica, id.counter);
		self.top = self.top.max(id.counter);
		for p in rec.parents() {
			self.named.insert(*p);
		}
		self.entries.push(rec);
		Ok(())
	}

	/// A record is placed as soon as its parents are present, so a batch that is
	/// causally closed within itself is absorbed whole however it was shuffled.
	/// What comes back names operations the batch and the log between them do
	/// not hold; offering it again once those arrive places it.
	///
	/// A record that is malformed for any other reason -- a spent counter, a
	/// repeated identifier -- is an error, not a leftover.
	pub fn absorb<I>(&mut self, batch: I)
		-> Outcome<Vec<Record>>
	where
		I: IntoIterator<Item = Record>,
	{
		let mut pending: Vec<Record> = batch.into_iter().collect();
		loop {
			let mut held: Vec<Record> = Vec::new();
			let mut placed = 0usize;
			for rec in pending {
				if rec.parents().iter().all(|p| self.index.contains_key(p)) {
					res!(self.append(rec));
					placed += 1;
				} else {
					held.push(rec);
				}
			}
			pending = held;
			if placed == 0 || pending.is_empty() {
				return Ok(pending);
			}
		}
	}

	pub fn get(&self, id: &OpId)
		-> Option<&Record>
	{
		self.index.get(id).and_then(|i| self.entries.get(*i))
	}

	pub fn op(&self, id: &OpId)
		-> Option<&Op>
	{
		self.get(id).map(|rec| &rec.op)
	}

	pub fn contains(&self, id: &OpId) -> bool {
		self.index.contains_key(id)
	}

	pub fn at(&self, pos: usize)
		-> Option<&Record>
	{
		self.entries.get(pos)
	}

	pub fn position(&self, id: &OpId) -> Option<usize> {
		self.index.get(id).copied()
	}

	/// In append order.
	pub fn iter(&self) -> impl Iterator<Item = &Record> {
		self.entries.iter()
	}

	/// In append order.
	pub fn iter_replica(&self, replica: ReplicaId) -> impl Iterator<Item = &Record> {
		self.entries.iter().filter(move |rec| rec.head.id().replica == replica)
	}

	/// `None` where the replica has authored nothing the log has seen.
	pub fn head(&self, replica: ReplicaId) -> Option<u64> {
		self.counters.get(&replica).copied()
	}

	/// Zero means the log is empty, since counters start at one.
	pub fn max_counter(&self) -> u64 {
		self.top
	}

	/// One past the greatest counter the log has seen from any replica, whoever
	/// authors next.
	///
	/// This is the Lamport clock: an operation minted here is later, in op order,
	/// than everything its author could see when it was minted.
	pub fn next_counter(&self) -> u64 {
		self.top + 1
	}

	/// The counter is [`OpLog::next_counter`] and not one past the replica's own
	/// head, so a replica's counters are strictly increasing but not
	/// consecutive.
	pub fn next_id(&self, replica: ReplicaId) -> OpId {
		OpId::new(replica, self.next_counter())
	}

	/// The highest counter seen per replica, which is what a peer needs in order
	/// to work out what to send.
	pub fn counters(&self) -> Vec<(ReplicaId, u64)> {
		let mut v: Vec<(ReplicaId, u64)> = self.counters
			.iter()
			.map(|(r, c)| (*r, *c))
			.collect();
		v.sort();
		v
	}

	/// The operations nobody in the log names as a parent, in ascending order of
	/// identifier.
	///
	/// This is what an author writes against, and what the log's causal graph
	/// terminates at.
	pub fn frontier(&self) -> Vec<OpId> {
		let mut v: Vec<OpId> = self.entries
			.iter()
			.map(|rec| rec.head.id())
			.filter(|id| !self.named.contains(id))
			.collect();
		v.sort();
		v
	}

	pub fn causality(&self)
		-> Causality<'_>
	{
		Causality::new(self.entries.iter().map(|rec| (rec.head.id(), rec.parents())))
	}

	/// Causally closed: every operation named is present, and so is every parent
	/// any of them names.
	///
	/// Fails if the log does not hold one of the identifiers, since a set the
	/// log cannot see is a question it cannot answer.
	pub fn is_closed(&self, ids: &[OpId])
		-> Outcome<bool>
	{
		let set: BTreeSet<OpId> = ids.iter().copied().collect();
		for id in &set {
			if !self.index.contains_key(id) {
				return Err(err!(
					"The log does not hold {}, so whether the set containing it is \
					causally closed cannot be decided.", id;
				Invalid, Input, Missing));
			}
		}
		for id in &set {
			let rec = match self.get(id) {
				Some(r) => r,
				None => continue,
			};
			for p in rec.parents() {
				if !set.contains(p) {
					return Ok(false);
				}
			}
		}
		Ok(true)
	}

	/// Mints the next identifier and takes the log's frontier as the parents.
	pub fn author(&mut self, replica: ReplicaId, op: Op)
		-> Outcome<Header>
	{
		let head = res!(Header::new(self.next_id(replica), self.frontier()));
		res!(self.append(Record::new(head.clone(), op)));
		Ok(head)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A distinguishable operation, for tests that only need entries to differ.
	fn mark(name: &str) -> Op {
		Op::Mark { name: fmt!("{}", name), body: None, time: None }
	}

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	fn rec(id: OpId, parents: Vec<OpId>, name: &str)
		-> Outcome<Record>
	{
		Ok(Record::new(res!(Header::new(id, parents)), mark(name)))
	}

	fn root(id: OpId, name: &str) -> Record {
		Record::root(id, mark(name))
	}

	#[test]
	fn empty_log_is_empty() -> Outcome<()> {
		let log = OpLog::new();
		assert!(log.is_empty());
		assert_eq!(log.len(), 0);
		assert_eq!(log.head(ReplicaId::new(1)), None);
		assert_eq!(log.max_counter(), 0);
		assert_eq!(log.next_counter(), 1);
		assert_eq!(log.next_id(ReplicaId::new(1)), oid(1, 1));
		assert!(log.counters().is_empty());
		assert!(log.frontier().is_empty());
		Ok(())
	}

	#[test]
	fn append_then_look_up() -> Outcome<()> {
		let mut log = OpLog::new();
		let a = oid(1, 1);
		let b = oid(2, 1);
		let c = oid(1, 2);
		res!(log.append(root(a, "a")));
		res!(log.append(res!(rec(b, vec![a], "b"))));
		res!(log.append(res!(rec(c, vec![b], "c"))));
		assert_eq!(log.len(), 3);
		assert_eq!(log.op(&b), Some(&mark("b")));
		assert!(log.contains(&c));
		assert_eq!(log.position(&c), Some(2));
		let order: Vec<OpId> = log.iter().map(|rec| rec.head.id()).collect();
		assert_eq!(order, vec![a, b, c]);
		Ok(())
	}

	#[test]
	fn lookup_of_an_absent_id_is_none() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 1), "a")));
		let missing = oid(9, 4);
		assert_eq!(log.op(&missing), None);
		assert_eq!(log.get(&missing), None);
		assert!(!log.contains(&missing));
		assert_eq!(log.position(&missing), None);
		Ok(())
	}

	#[test]
	fn counters_must_increase_per_replica() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(7, 1), "a")));
		res!(log.append(res!(rec(oid(7, 2), vec![oid(7, 1)], "b"))));
		// Backwards.
		assert!(log.append(root(oid(7, 1), "c")).is_err());
		// Equal to the head.
		assert!(log.append(root(oid(7, 2), "d")).is_err());
		// A gap forwards is allowed: the replica may not have shipped 3.
		res!(log.append(res!(rec(oid(7, 9), vec![oid(7, 2)], "e"))));
		assert_eq!(log.head(ReplicaId::new(7)), Some(9));
		// And now anything at or below 9 is refused.
		assert!(log.append(root(oid(7, 5), "f")).is_err());
		assert_eq!(log.len(), 3);
		Ok(())
	}

	#[test]
	fn counters_are_independent_across_replicas() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 100), "a")));
		res!(log.append(root(oid(2, 1), "b")));
		assert_eq!(log.head(ReplicaId::new(1)), Some(100));
		assert_eq!(log.head(ReplicaId::new(2)), Some(1));
		// The Lamport clock is not per replica, and stands at the greater.
		assert_eq!(log.max_counter(), 100);
		Ok(())
	}

	/// A zero counter is refused, since counters start at one.
	#[test]
	fn zero_counter_is_refused() -> Outcome<()> {
		let mut log = OpLog::new();
		assert!(log.append(root(oid(1, 0), "a")).is_err());
		assert!(log.is_empty());
		Ok(())
	}

	#[test]
	fn a_rejected_append_changes_nothing() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(3, 4), "a")));
		let before = log.counters();
		assert!(log.append(root(oid(3, 4), "b")).is_err());
		assert_eq!(log.len(), 1);
		assert_eq!(log.counters(), before);
		assert_eq!(log.op(&oid(3, 4)), Some(&mark("a")));
		Ok(())
	}

	/// Authoring mints the next counter and takes the frontier as parents, so a
	/// replica editing alone builds a chain of consecutive identifiers.
	#[test]
	fn author_mints_the_next_counter_and_parents_it() -> Outcome<()> {
		let r = ReplicaId::new(5);
		let mut log = OpLog::new();
		let a = res!(log.author(r, mark("a")));
		let b = res!(log.author(r, mark("b")));
		assert_eq!(a.id(), oid(5, 1));
		assert_eq!(b.id(), oid(5, 2));
		assert!(a.is_root());
		assert_eq!(b.parents(), vec![a.id()]);
		assert_eq!(log.next_id(r), oid(5, 3));
		assert_eq!(log.frontier(), vec![b.id()]);
		Ok(())
	}

	/// Minting is against the whole log and not against the author, so a replica
	/// that has seen another's work carries on from where that work left off.
	#[test]
	fn minting_is_against_every_replica_seen() -> Outcome<()> {
		let mut log = OpLog::new();
		// One replica is a long way ahead, and another has never written.
		res!(log.append(root(oid(2, 40), "far")));
		assert_eq!(log.max_counter(), 40);
		assert_eq!(log.next_counter(), 41);
		let fresh = res!(log.author(ReplicaId::new(1), mark("fresh")));
		assert_eq!(fresh.id(), oid(1, 41), "one past everything seen, not one past nothing");
		// And the replica that was ahead carries on past that in its turn.
		let on = res!(log.author(ReplicaId::new(2), mark("on")));
		assert_eq!(on.id(), oid(2, 42));
		Ok(())
	}

	/// A replica's own counters still strictly increase under Lamport minting,
	/// which is what the append guard insists on; only consecutiveness is lost.
	#[test]
	fn a_replicas_own_counters_still_increase() -> Outcome<()> {
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		let mut mine: Vec<u64> = Vec::new();
		for i in 0..5 {
			mine.push(res!(log.author(r1, mark(&fmt!("a{}", i)))).id().counter);
			res!(log.author(r2, mark(&fmt!("b{}", i))));
		}
		assert_eq!(mine, vec![1, 3, 5, 7, 9], "interleaved, so not consecutive");
		for pair in mine.windows(2) {
			assert!(pair[1] > pair[0], "{} does not advance {}", pair[1], pair[0]);
		}
		assert_eq!(log.head(r1), Some(9));
		assert_eq!(log.max_counter(), 10);
		Ok(())
	}

	/// An edit written in knowledge of another orders after it, whoever wrote it.
	///
	/// This is the whole point of the Lamport clock. Replica 9 writes first and
	/// replica 1 answers it; under minting per replica both would hold counter
	/// one and the op order would put the lower replica number first, which is
	/// the reverse of what happened.
	#[test]
	fn a_later_edit_orders_after_the_one_it_saw() -> Outcome<()> {
		use crate::seq::OpOrder;

		let nine = ReplicaId::new(9);
		let one = ReplicaId::new(1);
		// Replica 9 writes alone.
		let mut first = OpLog::new();
		let early = res!(first.author(nine, mark("early")));
		assert_eq!(early.id(), oid(9, 1));
		// Replica 1 absorbs that, and then edits having seen it.
		let mut second = OpLog::new();
		let batch: Vec<Record> = first.iter().cloned().collect();
		assert!(res!(second.absorb(batch)).is_empty());
		let late = res!(second.author(one, mark("late")));
		assert_eq!(late.parents(), vec![early.id()]);
		assert_eq!(late.id(), oid(1, 2));
		assert!(
			OpOrder::of(&late.id()) > OpOrder::of(&early.id()),
			"the edit written afterwards must order afterwards",
		);
		// What per-replica minting would have given, and why it is wrong.
		assert!(OpOrder::of(&oid(1, 1)) < OpOrder::of(&early.id()));
		Ok(())
	}

	#[test]
	fn authoring_after_a_merge_names_every_head() -> Outcome<()> {
		let mut log = OpLog::new();
		let seed = oid(1, 1);
		res!(log.append(root(seed, "seed")));
		// Two replicas write concurrently against the seed.
		res!(log.append(res!(rec(oid(2, 1), vec![seed], "left"))));
		res!(log.append(res!(rec(oid(3, 1), vec![seed], "right"))));
		assert_eq!(log.frontier(), vec![oid(2, 1), oid(3, 1)]);
		let merge = res!(log.author(ReplicaId::new(4), mark("merge")));
		assert_eq!(merge.id(), oid(4, 2), "the merge is later than what it merged");
		assert_eq!(merge.parents(), vec![oid(2, 1), oid(3, 1)]);
		assert_eq!(log.frontier(), vec![merge.id()]);
		Ok(())
	}

	#[test]
	fn an_operation_before_its_parent_is_refused() -> Outcome<()> {
		let mut log = OpLog::new();
		let orphan = res!(rec(oid(2, 1), vec![oid(1, 1)], "orphan"));
		assert!(log.append(orphan.clone()).is_err());
		assert!(log.is_empty());
		// With the parent present it goes in.
		res!(log.append(root(oid(1, 1), "parent")));
		res!(log.append(orphan));
		assert_eq!(log.len(), 2);
		Ok(())
	}

	/// A batch in any order is absorbed whole, and one missing an outside parent
	/// comes back rather than being half applied.
	#[test]
	fn absorb_places_a_shuffled_batch() -> Outcome<()> {
		let a = root(oid(1, 1), "a");
		let b = res!(rec(oid(2, 1), vec![oid(1, 1)], "b"));
		let c = res!(rec(oid(1, 2), vec![oid(2, 1)], "c"));
		let d = res!(rec(oid(3, 1), vec![oid(1, 2), oid(2, 1)], "d"));
		let mut log = OpLog::new();
		// Reverse order: nothing can be placed until the last is seen.
		let left = res!(log.absorb(vec![d.clone(), c.clone(), b.clone(), a.clone()]));
		assert!(left.is_empty(), "the batch is closed within itself");
		assert_eq!(log.len(), 4);
		assert_eq!(log.frontier(), vec![oid(3, 1)]);
		// A batch naming something nobody holds comes back.
		let mut other = OpLog::new();
		let left = res!(other.absorb(vec![c, d]));
		assert_eq!(left.len(), 2);
		assert!(other.is_empty());
		Ok(())
	}

	/// Absorption reports a malformed record rather than quietly holding it
	/// back.
	#[test]
	fn absorb_refuses_a_spent_counter() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 5), "a")));
		assert!(log.absorb(vec![root(oid(1, 2), "b")]).is_err());
		Ok(())
	}

	#[test]
	fn iter_replica_filters() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 1), "a")));
		res!(log.append(res!(rec(oid(2, 1), vec![oid(1, 1)], "b"))));
		res!(log.append(res!(rec(oid(1, 2), vec![oid(2, 1)], "c"))));
		let got: Vec<OpId> = log.iter_replica(ReplicaId::new(1))
			.map(|rec| rec.head.id())
			.collect();
		assert_eq!(got, vec![oid(1, 1), oid(1, 2)]);
		assert_eq!(log.iter_replica(ReplicaId::new(9)).count(), 0);
		Ok(())
	}

	#[test]
	fn counters_report_one_head_per_replica() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(2, 3), "a")));
		res!(log.append(root(oid(1, 1), "b")));
		res!(log.append(res!(rec(oid(2, 8), vec![oid(2, 3)], "c"))));
		assert_eq!(log.counters(), vec![
			(ReplicaId::new(1), 1),
			(ReplicaId::new(2), 8),
		]);
		Ok(())
	}

	#[test]
	fn the_frontier_is_what_nobody_names() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 1), "a")));
		res!(log.append(root(oid(2, 1), "b")));
		assert_eq!(log.frontier(), vec![oid(1, 1), oid(2, 1)]);
		res!(log.append(res!(rec(oid(3, 1), vec![oid(1, 1), oid(2, 1)], "join"))));
		assert_eq!(log.frontier(), vec![oid(3, 1)]);
		Ok(())
	}

	#[test]
	fn closure_is_decided_over_a_subset() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 1), "a")));
		res!(log.append(res!(rec(oid(1, 2), vec![oid(1, 1)], "b"))));
		res!(log.append(res!(rec(oid(2, 1), vec![oid(1, 2)], "c"))));
		assert!(res!(log.is_closed(&[oid(1, 1)])));
		assert!(res!(log.is_closed(&[oid(1, 1), oid(1, 2)])));
		assert!(!res!(log.is_closed(&[oid(1, 2)])), "its parent is missing");
		assert!(!res!(log.is_closed(&[oid(1, 1), oid(2, 1)])), "the middle is missing");
		// A set naming something the log has never seen cannot be decided.
		assert!(log.is_closed(&[oid(9, 9)]).is_err());
		Ok(())
	}

	#[test]
	fn concurrency_is_read_off_the_parents() -> Outcome<()> {
		let mut log = OpLog::new();
		let seed = oid(1, 1);
		res!(log.append(root(seed, "seed")));
		res!(log.append(res!(rec(oid(2, 2), vec![seed], "left"))));
		res!(log.append(res!(rec(oid(3, 2), vec![seed], "right"))));
		res!(log.append(res!(rec(oid(2, 3), vec![oid(2, 2), oid(3, 2)], "merge"))));
		let cause = log.causality();
		assert!(cause.is_closed());
		assert!(cause.concurrent(&oid(2, 2), &oid(3, 2)));
		assert!(!cause.concurrent(&seed, &oid(2, 2)));
		assert!(!cause.concurrent(&oid(2, 3), &oid(3, 2)), "the merge saw both");
		assert!(cause.reaches(&oid(2, 3), &seed));
		assert!(!cause.reaches(&seed, &oid(2, 3)));
		assert_eq!(cause.heads(), vec![oid(2, 3)]);
		Ok(())
	}

	#[test]
	fn a_graph_with_a_gap_names_it() -> Outcome<()> {
		let parents = vec![oid(1, 1)];
		let cause = Causality::new(vec![(oid(2, 1), &parents[..])]);
		assert!(!cause.is_closed());
		assert_eq!(cause.gap(), Some((oid(2, 1), oid(1, 1))));
		Ok(())
	}

	#[test]
	fn entries_are_addressable_by_position() -> Outcome<()> {
		let mut log = OpLog::new();
		res!(log.append(root(oid(1, 1), "a")));
		res!(log.append(res!(rec(oid(1, 2), vec![oid(1, 1)], "b"))));
		match log.at(1) {
			Some(rec) => {
				assert_eq!(rec.head.id(), oid(1, 2));
				assert_eq!(rec.op, mark("b"));
			},
			None => return Err(err!("Expected an entry at position 1."; Test, Missing)),
		}
		assert!(log.at(2).is_none());
		Ok(())
	}
}
