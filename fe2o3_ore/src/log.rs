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


// One u64 each, so 32 MiB.  A history that forks against itself thousands of
// times wants a column per run and is one the index should decline.
const VECTORS_ENTRIES_MAX: usize = 4 * 1024 * 1024;


/// The highest counter each run reached inside an operation's ancestry, so that
/// ancestry is two lookups rather than a walk.
///
/// A column is a run and not a replica because one counter can only stand for a
/// set that enters an ancestry with none skipped, and a replica that forks
/// against itself leaves gaps inside its own range. Twenty-nine such forks in a
/// 44,629 operation history, so the cut is what makes an index possible at all.
#[derive(Clone, Debug)]
struct Vectors {
	cols:	usize,						// runs, which is the width of a row
	rows:	Vec<u64>,					// one row of `cols` per operation
	at:		HashMap<OpId, (u32, u32)>,	// each operation's row, and the run it belongs to
}

impl Vectors {

	/// `None` wherever the graph does not admit an index, which is looked at
	/// rather than assumed.
	///
	/// Two passes, and the first earns the second: a column per replica gives
	/// maxima that hold whatever the shape, so it finds the forks without being
	/// safe to compare against, and the second gives each unbroken piece a
	/// column. A history that never forks stops after the first.
	fn build(parents: &BTreeMap<OpId, &[OpId]>, graded: bool)
		-> Option<Self>
	{
		// Counters are a topological order only where every edge steps down one,
		// and without a topological order a parent's vector is not finished when
		// its child comes to read it.
		if !graded {
			return None;
		}
		let n = parents.len();
		// A column apiece is the narrowest the matrix can be, so a set that will
		// not fit one column wide will not fit any, and nothing is allocated to
		// find that out. This also holds `n` inside a `u32`, which the row and run
		// numbers below are.
		if n == 0 || n > VECTORS_ENTRIES_MAX {
			return None;
		}
		// Ascending counter, which grading makes a topological order and which is
		// not the order `OpId` sorts in -- that is replica first.
		let mut order: Vec<OpId> = parents.keys().copied().collect();
		order.sort_unstable_by_key(|id| (id.counter, id.replica));
		let mut pos: HashMap<OpId, u32> = HashMap::with_capacity(n);
		for (i, id) in order.iter().enumerate() {
			pos.insert(*id, i as u32);
		}
		let mut column: BTreeMap<ReplicaId, usize> = BTreeMap::new();
		for id in order.iter() {
			let next = column.len();
			column.entry(id.replica).or_insert(next);
		}
		let reps = column.len();
		let mut runs: Vec<u32> = Vec::with_capacity(n);
		for id in order.iter() {
			match column.get(&id.replica) {
				Some(c)	=> runs.push(*c as u32),
				None	=> return None,
			}
		}
		let rows = match Self::fill(parents, &order, &pos, &runs, reps, false) {
			Some(r)	=> r,
			None	=> return None,
		};

		// Where each replica's run is broken, and so where it is cut. A break is
		// the parents between them reaching less of this replica than its
		// immediately preceding operation, and nothing else. Reading it back off
		// the finished matrix costs one visit per edge rather than per column,
		// since only the operation's own column is in question.
		let mut now: Vec<u32> = (0..reps as u32).collect();	// the run each replica is in
		let mut last: Vec<u64> = vec![0; reps];
		let mut cols = reps;
		for (i, id) in order.iter().enumerate() {
			let c = runs[i] as usize;
			let mut reached = 0u64;
			if let Some(ps) = parents.get(id) {
				for p in ps.iter() {
					if let Some(a) = pos.get(p) {
						let seen = rows[(*a as usize) * reps + c];
						if seen > reached {
							reached = seen;
						}
					}
				}
			}
			if reached != last[c] {
				now[c] = cols as u32;
				cols += 1;
			}
			runs[i] = now[c];
			last[c] = id.counter;
		}

		// Nothing cut means every check above passed and the first pass is already
		// the index. Otherwise each piece has a column of its own, and the pass
		// that fills them asks for the run to be unbroken rather than trusting the
		// cutting to have made it so.
		let rows = if cols == reps {
			rows
		} else {
			match Self::fill(parents, &order, &pos, &runs, cols, true) {
				Some(r)	=> r,
				None	=> return None,
			}
		};
		let at: HashMap<OpId, (u32, u32)> = order.iter()
			.enumerate()
			.map(|(i, id)| (*id, (i as u32, runs[i])))
			.collect();
		Some(Self { cols, rows, at })
	}

	/// One pass in topological order, giving each operation the highest counter
	/// every column reached in its ancestry.
	///
	/// Under `check` each column must arrive at exactly the counter the previous
	/// operation of that column left, which is what [`Vectors::reaches`] rests
	/// on. The first pass is not entitled to that and does not ask.
	fn fill(
		parents:	&BTreeMap<OpId, &[OpId]>,
		order:		&[OpId],
		pos:		&HashMap<OpId, u32>,
		runs:		&[u32],
		cols:		usize,
		check:		bool,
	)
		-> Option<Vec<u64>>
	{
		let n = order.len();
		let entries = match n.checked_mul(cols) {
			Some(e) if e <= VECTORS_ENTRIES_MAX	=> e,
			_									=> return None,
		};
		let mut rows: Vec<u64> = vec![0; entries];
		let mut last: Vec<u64> = vec![0; cols];
		for (i, id) in order.iter().enumerate() {
			let base = i * cols;
			let ps = match parents.get(id) {
				Some(p)	=> *p,
				None	=> return None,
			};
			for p in ps.iter() {
				// A parent with no position is one the graph does not hold, and one
				// positioned at or after its child would leave the row being read
				// unfinished. Closure forbids the first and grading the second, so
				// both are asked here for nothing -- but neither is enforced by
				// [`OpLog::append`], which is why they are asked at all.
				let from = match pos.get(p) {
					Some(a) if (*a as usize) < i	=> (*a as usize) * cols,
					_								=> return None,
				};
				for k in 0..cols {
					let reached = rows[from + k];
					if reached > rows[base + k] {
						rows[base + k] = reached;
					}
				}
			}
			let c = runs[i] as usize;
			if check && rows[base + c] != last[c] {
				return None;
			}
			rows[base + c] = id.counter;
			last[c] = id.counter;
		}
		Some(rows)
	}

	fn reaches(&self, from: &OpId, target: &OpId) -> bool {
		let row = match self.at.get(from) {
			Some((row, _))	=> *row as usize,
			None			=> return false,
		};
		// A target the graph does not hold has no run and is not reached, which has
		// to be looked up rather than left to the arithmetic: a replica's counters
		// may skip, since minting is against the whole log, and a vector standing
		// above a counter nobody ever spent would otherwise read as a reach.
		let col = match self.at.get(target) {
			Some((_, col))	=> *col as usize,
			None			=> return false,
		};
		self.rows[row * self.cols + col] >= target.counter
	}

	/// Roughly what the index occupies, for a caller pricing it.
	fn bytes(&self) -> usize {
		self.rows.len() * std::mem::size_of::<u64>()
			+ self.at.capacity() * (std::mem::size_of::<OpId>() + 2 * std::mem::size_of::<u32>())
	}
}


/// The parent graph over a set of operations, and the ancestry questions it
/// answers.
///
/// Built by borrowing each operation's parents rather than copying them, so
/// asking a question of a large history costs one map and no duplication of the
/// graph. Ancestry is answered from a version vector index where the graph
/// admits one, and otherwise by walking parents, which terminates because an
/// operation may not name itself and counters strictly increase along any path
/// a replica writes.
#[derive(Clone, Debug, Default)]
pub struct Causality<'a> {
	parents:	BTreeMap<OpId, &'a [OpId]>,	// each operation's parents, by operation
	// Whether every edge steps strictly downwards in counter, which is what lets
	// an ancestry walk stop.  Established by looking, never assumed.
	graded:		bool,
	vectors:	Option<Vectors>,			// ancestry in O(1), where the graph earns it
}

impl<'a> Causality<'a> {

	pub fn new<I>(ops: I) -> Self
	where
		I: IntoIterator<Item = (OpId, &'a [OpId])>,
	{
		let parents: BTreeMap<OpId, &'a [OpId]> = ops.into_iter().collect();
		// One pass over the edges, which is a few tens of microseconds on a
		// history of forty thousand operations and buys the bound in
		// [`Causality::reaches`].
		let mut graded = true;
		'outer: for (id, ps) in &parents {
			for p in ps.iter() {
				if p.counter >= id.counter {
					graded = false;
					break 'outer;
				}
			}
		}
		let vectors = Vectors::build(&parents, graded);
		Self { parents, graded, vectors }
	}

	/// Does every parent edge step strictly downwards in counter?
	///
	/// True of anything this tool mints, since an operation takes the counter one
	/// past the highest the log holds. Not true by construction, though, because
	/// [`OpLog::append`] forbids neither a hand-assembled graph whose child sits
	/// below its parent nor one that sits level with it. So it is measured.
	pub fn is_graded(&self) -> bool {
		self.graded
	}

	/// Is ancestry answered from the version vector index rather than by walking?
	///
	/// The index is built where the graph admits it and refused where it does
	/// not, so this reports which of the two [`Causality::reaches`] is using. The
	/// answers are the same either way; only the cost differs.
	pub fn is_indexed(&self) -> bool {
		self.vectors.is_some()
	}

	/// Roughly what the index occupies, and `None` where there is none.
	pub fn index_bytes(&self) -> Option<usize> {
		self.vectors.as_ref().map(|vv| vv.bytes())
	}

	/// One per replica, and one more for every fork after that.  The index is
	/// linear in this.
	pub fn index_runs(&self) -> Option<usize> {
		self.vectors.as_ref().map(|vv| vv.cols)
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
		// Two lookups and a comparison, and the walk below never runs.  It took a
		// 44,629 operation render from 4.9 s to 0.28 s for 12.4 MB.
		if let Some(vv) = &self.vectors {
			return vv.reaches(from, target);
		}
		// Every parent edge steps strictly downwards in counter, so nothing at or
		// below the target's counter can reach it and a branch that has fallen
		// there is done.  That bounds the walk by the window between the two
		// rather than by the size of the history, and it matters most where a
		// `false` answer would otherwise have to exhaust the whole ancestor set.
		//
		// The rule is not enforced ([`Causality::is_graded`]), so the graph is
		// asked rather than assumed.
		if self.graded && from.counter <= target.counter {
			return false;
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
				if self.graded && p.counter <= target.counter {
					continue;
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

	/// Ancestry by exhaustive walk, with no bound of any kind.
	///
	/// This is what [`Causality::reaches`] did before it was allowed to stop at
	/// the target's counter, kept here as the definition the fast one is judged
	/// against. It is the only honest way to assert that a bound changed the
	/// cost and not the answer.
	fn reaches_exhaustively(cause: &Causality<'_>, from: &OpId, target: &OpId) -> bool {
		if from == target {
			return true;
		}
		let mut seen: HashSet<OpId> = HashSet::new();
		let mut stack: Vec<OpId> = vec![*from];
		while let Some(id) = stack.pop() {
			let parents = match cause.parents_of(&id) {
				Some(p) => p,
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

	/// Every ordered pair, against the exhaustive walk, naming the pair that
	/// disagrees.
	fn every_pair_agrees(cause: &Causality<'_>, ids: &[OpId], what: &str) {
		for a in ids {
			for b in ids {
				assert_eq!(
					cause.reaches(a, b),
					reaches_exhaustively(cause, a, b),
					"{}: reaches({}, {}) disagrees with the exhaustive walk",
					what, a, b);
			}
		}
	}

	/// A graph over borrowed parent lists, which is how every caller builds one.
	fn built<'b>(ids: &'b [OpId], parents: &'b [Vec<OpId>])
		-> Causality<'b>
	{
		Causality::new(ids.iter().zip(parents.iter()).map(|(id, p)| (*id, p.as_slice())))
	}

	/// Whichever way ancestry is answered -- the version vector index, the
	/// bounded walk, the unbounded one -- the answer is the same, over every pair
	/// of a few hundred randomly shaped histories.
	///
	/// The shapes are the point, and two thirds of them are shapes the tool
	/// cannot mint. A minted history names a replica's own previous operation, so
	/// no replica ever forks against itself and the index needs no cut. A graded
	/// history -- every parent below its child in counter -- takes arbitrary
	/// earlier parents, so it grades but forks constantly, and the index has to
	/// cut it into runs to stay exact. An ungraded one is what [`OpLog::append`]
	/// nevertheless accepts, since it requires only that a parent be present and
	/// that a replica's own counters rise; on those both the index and the bound
	/// must switch themselves off, and the only way to know they do is to build
	/// them and ask.
	///
	/// All three exercise long chains, wide merges, several replicas authoring at
	/// once, and operations sharing a counter because two replicas minted one
	/// concurrently.
	#[test]
	fn a_bounded_ancestry_walk_answers_exactly_as_an_exhaustive_one()
		-> Outcome<()>
	{
		// A small deterministic generator, so a failure is reproducible from the
		// seed printed beside it rather than from a run nobody can repeat.
		let mut state = 0x9e3779b97f4a7c15u64;
		let mut next = move || {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			state
		};
		let mut pairs = 0usize;
		let mut saw_graded = 0usize;
		let mut saw_ungraded = 0usize;
		let mut saw_indexed = 0usize;
		let mut saw_walked = 0usize;
		let mut saw_cut = 0usize;
		for shape in 0..201u64 {
			let kind = shape % 3;
			let minted = kind == 0;
			let graded = kind != 2;
			let replicas = 1 + (shape % 4);
			let n = 2 + (next() % 40) as usize;
			let mut ids: Vec<OpId> = Vec::new();
			let mut parents: Vec<Vec<OpId>> = Vec::new();
			let mut top = 0u64;
			for i in 0..n {
				let replica = next() % replicas;
				// Two replicas minting at once take the same counter, which is
				// exactly the case the bound has to get right.
				let counter = if i > 0 && next() % 5 == 0 { top } else { top + 1 };
				top = top.max(counter);
				let id = oid(replica, counter);
				if ids.contains(&id) {
					continue;
				}
				let mut ps: Vec<OpId> = Vec::new();
				// A replica authors against its own frontier, so its own previous
				// operation is always among the parents. That is the whole of what
				// the index needs, and the only family here that supplies it.
				if minted {
					if let Some(prev) = ids.iter().rev().find(|c| c.replica == id.replica) {
						ps.push(*prev);
					}
				}
				for (j, cand) in ids.iter().enumerate() {
					// A graded shape takes only parents strictly below, which is
					// what minting guarantees. An ungraded one takes any earlier
					// operation, which is what the log actually permits.
					let allowed = if graded { cand.counter < counter } else { true };
					if allowed && next() % (2 + (j as u64 % 3)) == 0 {
						ps.push(*cand);
					}
				}
				ps.sort();
				ps.dedup();
				ids.push(id);
				parents.push(ps);
			}
			let cause = Causality::new(
				ids.iter().zip(parents.iter()).map(|(id, p)| (*id, p.as_slice())));
			if cause.is_graded() { saw_graded += 1; } else { saw_ungraded += 1; }
			if cause.is_indexed() { saw_indexed += 1; } else { saw_walked += 1; }
			let distinct: BTreeSet<ReplicaId> = ids.iter().map(|id| id.replica).collect();
			match cause.index_runs() {
				Some(runs) if runs > distinct.len()	=> saw_cut += 1,
				Some(runs)							=> assert!(!minted || runs == distinct.len(),
					"shape {}: a minted history forks against nothing, so it wants \
					{} runs and not {}", shape, distinct.len(), runs),
				None								=> assert!(!cause.is_graded(),
					"shape {}: a graded, closed graph admits an index", shape),
			}
			assert!(!minted || cause.is_indexed(),
				"shape {}: a minted history leaves every replica's run unbroken, so \
				the index must have been built", shape);
			for a in &ids {
				for b in &ids {
					pairs += 1;
					assert_eq!(
						cause.reaches(a, b),
						reaches_exhaustively(&cause, a, b),
						"shape {}: ancestry as answered ({}) and the exhaustive walk \
						disagree about whether {} reaches {}",
						shape,
						if cause.is_indexed() { "indexed" } else { "walked" },
						a, b);
				}
			}
		}
		assert!(pairs > 50_000, "only {} pairs were compared, which is too few to \
			have covered the shapes", pairs);
		// Without both kinds this test would agree with the bound instead of
		// checking it: every graph graded means the fallback never ran, and
		// every graph ungraded means the bound never did.
		assert!(saw_graded >= 20 && saw_ungraded >= 20,
			"the shapes did not cover both cases: {} graded, {} ungraded",
			saw_graded, saw_ungraded);
		// And the same argument for the index: all of them indexed and the walk
		// never ran, none of them and the index never did. The cut is counted
		// separately, since an index that never had to cut anything would agree
		// with the walk for a reason that says nothing about forks.
		assert!(saw_indexed >= 20 && saw_walked >= 20,
			"the shapes did not cover both cases: {} indexed, {} walked",
			saw_indexed, saw_walked);
		assert!(saw_cut >= 20, "only {} shapes forked against themselves, which is \
			too few to have exercised the cut", saw_cut);
		Ok(())
	}

	/// A replica that forks against itself is cut at the fork, and the index goes
	/// on answering exactly.
	///
	/// The cut is the whole reason the index exists on real histories: a 44,629
	/// operation repository turned out to hold twenty-nine of these forks, and
	/// one column per replica would have been refused on every one of them.
	#[test]
	fn a_replica_that_forks_against_itself_is_cut_at_the_fork()
		-> Outcome<()>
	{
		// A replica skipping one of its own: (1,3) names (1,1) and not (1,2), so
		// counter 2 is outside its ancestry while counter 3 is inside it, and no
		// single highest counter for replica 1 could say that. Cut at (1,3) there
		// are two runs, and each is a number that can be compared.
		let ids = vec![oid(1, 1), oid(1, 2), oid(1, 3)];
		let parents = vec![vec![], vec![oid(1, 1)], vec![oid(1, 1)]];
		let cause = built(&ids, &parents);
		assert!(cause.is_graded());
		assert!(cause.is_indexed(), "a fork is cut, not refused");
		assert_eq!(cause.index_runs(), Some(2), "one replica, cut once");
		assert!(!cause.reaches(&oid(1, 3), &oid(1, 2)), "it never saw it");
		assert!(cause.reaches(&oid(1, 3), &oid(1, 1)));
		assert!(cause.concurrent(&oid(1, 2), &oid(1, 3)));
		every_pair_agrees(&cause, &ids, "a replica skipping one of its own");

		// A replica whose first operation in the graph is not its first: (1,2) is
		// a root though (1,1) is present, which is the same fork at the start.
		let ids = vec![oid(1, 1), oid(1, 2)];
		let parents = vec![vec![], vec![]];
		let cause = built(&ids, &parents);
		assert_eq!(cause.index_runs(), Some(2));
		assert!(!cause.reaches(&oid(1, 2), &oid(1, 1)));
		every_pair_agrees(&cause, &ids, "a root that is not the replica's first");

		// Forking twice cuts twice, and the pieces do not run together: (1,5) sees
		// the first piece only, (1,4) the second only.
		let ids = vec![oid(1, 1), oid(1, 2), oid(1, 3), oid(1, 4), oid(1, 5)];
		let parents = vec![
			vec![],
			vec![oid(1, 1)],
			vec![oid(1, 2)],
			vec![],
			vec![oid(1, 3)],
		];
		let cause = built(&ids, &parents);
		assert_eq!(cause.index_runs(), Some(3), "one replica, cut twice");
		assert!(cause.reaches(&oid(1, 5), &oid(1, 3)));
		assert!(!cause.reaches(&oid(1, 5), &oid(1, 4)));
		assert!(!cause.reaches(&oid(1, 4), &oid(1, 1)));
		every_pair_agrees(&cause, &ids, "a replica forking twice");

		// One replica, its run unbroken, which needs no cut and gets none.
		let ids = vec![oid(1, 1), oid(1, 2), oid(1, 3)];
		let parents = vec![vec![], vec![oid(1, 1)], vec![oid(1, 2)]];
		let cause = built(&ids, &parents);
		assert_eq!(cause.index_runs(), Some(1), "nothing to cut");
		assert!(cause.reaches(&oid(1, 3), &oid(1, 1)));
		assert!(!cause.reaches(&oid(1, 1), &oid(1, 3)));
		every_pair_agrees(&cause, &ids, "one replica");

		// Two replicas sharing a counter, which is what concurrent minting gives.
		let ids = vec![oid(1, 1), oid(2, 1), oid(1, 2), oid(2, 2)];
		let parents = vec![
			vec![],
			vec![],
			vec![oid(1, 1), oid(2, 1)],
			vec![oid(1, 1), oid(2, 1)],
		];
		let cause = built(&ids, &parents);
		assert_eq!(cause.index_runs(), Some(2), "one run per replica");
		assert!(cause.concurrent(&oid(1, 1), &oid(2, 1)));
		assert!(cause.concurrent(&oid(1, 2), &oid(2, 2)));
		assert!(cause.reaches(&oid(1, 2), &oid(2, 1)), "the merge saw both");
		every_pair_agrees(&cause, &ids, "two replicas sharing a counter");
		Ok(())
	}

	/// The index is refused wherever the graph does not admit one at all, and the
	/// walk answers instead.
	///
	/// Each shape here is one way it can be inadmissible, written out rather than
	/// waited for: the random families above will produce them, but not reliably,
	/// and a case that only sometimes runs is a case nobody is checking.
	#[test]
	fn the_index_is_refused_where_the_graph_does_not_admit_one()
		-> Outcome<()>
	{
		// A parent the graph does not hold. The walk still answers `true` for the
		// missing parent itself, since it is named on an edge the graph holds, and
		// the index has no row to read that off.
		let ids = vec![oid(1, 1), oid(2, 3)];
		let parents = vec![vec![], vec![oid(1, 1), oid(1, 2)]];
		let cause = built(&ids, &parents);
		assert!(!cause.is_closed());
		assert!(!cause.is_indexed(), "an open graph must refuse the index");
		assert_eq!(cause.index_bytes(), None);
		assert!(cause.reaches(&oid(2, 3), &oid(1, 2)), "it is named as a parent");
		every_pair_agrees(&cause, &ids, "a parent the graph does not hold");

		// Ungraded, so counters give no topological order to build in.
		let ids = vec![oid(1, 1), oid(2, 1)];
		let parents = vec![vec![], vec![oid(1, 1)]];
		let cause = built(&ids, &parents);
		assert!(!cause.is_graded());
		assert!(!cause.is_indexed(), "an ungraded graph must refuse the index");
		assert!(cause.reaches(&oid(2, 1), &oid(1, 1)));
		every_pair_agrees(&cause, &ids, "an ungraded graph");

		// Nothing at all, which has no row to be the answer.
		let none: Vec<(OpId, &[OpId])> = Vec::new();
		let cause = Causality::new(none);
		assert!(!cause.is_indexed());
		assert!(!cause.reaches(&oid(1, 1), &oid(1, 2)));
		assert!(cause.reaches(&oid(1, 1), &oid(1, 1)));
		Ok(())
	}

	/// A counter no replica ever spent is not reached, though the vector for its
	/// replica stands above it.
	///
	/// Minting is against the whole log, so a replica's own counters skip
	/// whenever another writes in between: two replicas taking turns leaves one
	/// with the odd counters and the other with the even. The index reads only
	/// the highest counter a replica reached, which stands above the counters
	/// nobody minted as much as above the ones somebody did, so an identifier the
	/// graph does not hold has to be refused by name rather than by arithmetic.
	#[test]
	fn a_counter_nobody_spent_is_not_reached()
		-> Outcome<()>
	{
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		for i in 0..6 {
			res!(log.author(r1, mark(&fmt!("a{}", i))));
			res!(log.author(r2, mark(&fmt!("b{}", i))));
		}
		let ids: Vec<OpId> = log.iter().map(|rec| rec.head.id()).collect();
		let cause = log.causality();
		assert!(cause.is_indexed(), "a minted history is what the index is for");
		assert_eq!(log.head(r1), Some(11));
		assert_eq!(log.head(r2), Some(12));
		let head = oid(1, 11);
		assert!(cause.reaches(&head, &oid(1, 9)), "its own previous");
		assert!(cause.reaches(&head, &oid(2, 10)), "and what it was written against");
		for gap in [2u64, 4, 6, 8, 10] {
			assert!(!cause.reaches(&head, &oid(1, gap)),
				"replica 1 never spent counter {}, so nothing reaches it", gap);
		}
		// And above everything, and a replica the graph has never heard of.
		assert!(!cause.reaches(&head, &oid(1, 99)));
		assert!(!cause.reaches(&head, &oid(7, 1)));
		every_pair_agrees(&cause, &ids, "a minted history");
		Ok(())
	}

	/// The index costs what the arithmetic on it says, which is worth knowing
	/// before it is switched on over a history a hundred times this size.
	#[test]
	fn the_index_costs_a_row_per_operation()
		-> Outcome<()>
	{
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut log = OpLog::new();
		for i in 0..50 {
			res!(log.author(r1, mark(&fmt!("a{}", i))));
			res!(log.author(r2, mark(&fmt!("b{}", i))));
		}
		let cause = log.causality();
		let bytes = match cause.index_bytes() {
			Some(b) => b,
			None => return Err(err!("The index was refused on a minted history."; Test)),
		};
		// A hundred operations over two replicas: 100 x 2 x 8 = 1,600 bytes of
		// matrix, and the row map on top of it.
		assert!(bytes >= 1_600, "{} bytes is below the matrix itself", bytes);
		assert!(bytes < 16_000, "{} bytes is more than the matrix and a map", bytes);
		let empty = Causality::default();
		assert_eq!(empty.index_bytes(), None);
		assert!(!empty.is_indexed());
		assert!(!empty.reaches(&oid(1, 1), &oid(1, 2)));
		Ok(())
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
