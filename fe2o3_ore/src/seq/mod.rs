//! A convergent ordered sequence in which a move is recorded as a move.
//!
//! Bytes take their identity from the splice that created them and never lose
//! it. Position is a separate, derived layer: an ordered set of slots, each
//! claiming a run of byte identities, ordered against each other by Fugue over
//! origins that name *content* rather than positions. A move mints new slots at
//! the destination and claims the moved bytes for them; a per-byte
//! last-writer-wins register decides which slot owns each byte, so two
//! concurrent moves of one run cannot duplicate it. Because an insertion's
//! origin names a byte, and that byte's owning slot is wherever it currently
//! lives, the insertion follows the move without anything being written to make
//! it do so.
//!
//! # What it guarantees, and what it does not
//!
//! Two replicas that have applied the same operations render the same bytes,
//! whatever order the operations arrived in. That is the whole of the promise.
//! It is not a promise that the result is what either author wanted:
//!
//! - Two moves of the same run leave one copy, at the destination of whichever
//!   move is higher in op order. The loser's intent is discarded, and flagged.
//! - Two moves of partly overlapping runs tear at the overlap. Both halves
//!   survive, in two places, which is deterministic and is almost certainly not
//!   what either author meant. It is flagged.
//! - Two moves whose destinations sit inside each other's sources form a cycle,
//!   and one of them lands where its anchor content was originally written
//!   rather than where it now lives. It is flagged.
//!
//! The posture is to converge always and to say what happened always. Every
//! [`render::Flag`] is a function of the operation set, so a flag is a fact
//! about the history rather than a note about this run of the renderer.
//!
//! # Transient by construction
//!
//! The durable record is the operation log. Everything here -- the atoms, the
//! claim register, the tombstones, the slots, their order -- is derived, and is
//! rebuilt from the operation set on every render. A [`Sequence`] is therefore
//! an accumulator and nothing more: applying an operation is set insertion, and
//! two sequences holding the same operations are the same sequence whatever
//! order they were built in.
//!
//! # Preconditions
//!
//! Rendering requires a **causally complete** operation set, and that is now
//! checked against the operations' own parents rather than inferred from what
//! they happen to name. Every parent must be present, and so must every atom
//! whose content an anchor or a range names. An anchor naming an atom that has
//! not arrived cannot be resolved, and rather than guess, the render fails and
//! says which operation named what. [`crate::log::OpLog`] supplies a closed set
//! by construction; a caller assembling operations by hand must arrange it.
//!
//! # Op order
//!
//! Every tie-break in the structure is decided by [`OpOrder`], the pair
//! `(counter, replica)` ascending. Convergence needs only that the order is
//! total, which it is for any counters at all; the intuition that a later edit
//! wins needs the counter to be a Lamport clock, one greater than the greatest
//! the replica has seen. [`crate::log::OpLog::next_counter`] mints exactly that,
//! so an author who takes identifiers from the log gets the intuition for
//! nothing; an author minting its own is responsible for the same rule.

pub mod atom;
pub mod claim;
pub mod render;
pub mod slot;

#[cfg(test)]
mod tests;

use crate::id::{
	Anchor,
	ContentRange,
	OpId,
	Side,
};
use crate::log::Causality;
use crate::op::{
	Header,
	Op,
	Record,
};
use crate::seq::atom::Atoms;
use crate::seq::claim::{
	Claims,
	Dead,
};
use crate::seq::render::{
	Flag,
	Rendered,
	Run,
	Stats,
};
use crate::seq::slot::Slots;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::interval::IntervalMap;

use std::collections::BTreeMap;


/// The total order every tie-break in the structure is decided by: the Lamport
/// counter first, the authoring replica second.
///
/// This is deliberately not the order on [`OpId`], which sorts by replica first
/// and is meant for indexing. Sorting by counter first is what makes "the later
/// edit wins" mean what it says.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpOrder {
	/// The Lamport counter, which decides.
	pub counter:	u64,
	/// The authoring replica, which breaks the tie.
	pub replica:	u64,
}

impl OpOrder {
	/// Returns the position of an operation in op order.
	pub fn of(id: &OpId) -> Self {
		Self {
			counter:	id.counter,
			replica:	id.replica.inner(),
		}
	}
}


/// An operation in the form the sequence structure consumes: content-anchored,
/// so that nothing in it names a position.
///
/// A splice inserts bytes and kills runs; a move reclaims runs somewhere else.
/// Neither carries a numeric offset, a line number or a position identifier that
/// a concurrent move could invalidate, and that single property is what the rest
/// of the module is built on.
///
/// This is [`Op`] with the file and the header taken off: what is left is
/// exactly what decides where bytes go. The identity and the parents are not
/// part of it, because the same edit written by two authors is two operations,
/// so the [`Header`] is passed alongside to [`Sequence::apply`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Edit {
	/// Inserts bytes at a gap, and kills runs of existing content.
	///
	/// Either half may be empty: an insertion removes nothing, a deletion
	/// inserts nothing, and a replacement does both in one operation.
	Splice {
		/// Left origin of the inserted bytes; `None` is the start of the file.
		left:	Option<Anchor>,
		/// Right origin of the inserted bytes; `None` is the end of the file.
		right:	Option<Anchor>,
		/// What dies.
		remove:	Vec<ContentRange>,
		/// What is inserted.
		insert:	Vec<u8>,
	},
	/// Reclaims runs of existing content at a new position, in the order given.
	Move {
		/// What moves.
		src:	Vec<ContentRange>,
		/// Left origin of the destination; `None` is the start of the file.
		left:	Option<Anchor>,
		/// Right origin of the destination; `None` is the end of the file.
		right:	Option<Anchor>,
	},
}

impl Edit {

	/// Returns the operation's two origins.
	pub fn origins(&self) -> (Option<Anchor>, Option<Anchor>) {
		match self {
			Self::Splice { left, right, .. }	=> (*left, *right),
			Self::Move { left, right, .. }		=> (*left, *right),
		}
	}

	/// Returns the content the operation names: what a splice removes, or what a
	/// move takes with it.
	pub fn regions(&self) -> &[ContentRange] {
		match self {
			Self::Splice { remove, .. }	=> remove,
			Self::Move { src, .. }		=> src,
		}
	}

	/// Reports whether the operation is a move.
	pub fn is_move(&self) -> bool {
		matches!(self, Self::Move { .. })
	}

	/// Returns the variant name, for messages.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Splice { .. }	=> "Splice",
			Self::Move { .. }	=> "Move",
		}
	}

	/// Returns the total number of bytes the operation places, which is what a
	/// splice inserts or what a move brings with it.
	pub fn placed_len(&self) -> u64 {
		match self {
			Self::Splice { insert, .. }	=> insert.len() as u64,
			Self::Move { src, .. }		=> src.iter().map(|r| r.len()).sum(),
		}
	}

	/// Checks the operation is one the structure can resolve.
	///
	/// A left origin binds after a byte and a right origin before one; a move
	/// may not name the same byte twice, since a byte has exactly one owning
	/// slot and could not otherwise be shown once.
	pub fn validate(&self)
		-> Outcome<()>
	{
		let (left, right) = self.origins();
		if let Some(a) = left {
			if a.side != Side::After {
				return Err(err!(
					"An {} names {} as its left origin; a left origin binds after \
					a byte, not before it.", self.name(), a;
				Invalid, Input));
			}
		}
		if let Some(a) = right {
			if a.side != Side::Before {
				return Err(err!(
					"An {} names {} as its right origin; a right origin binds \
					before a byte, not after it.", self.name(), a;
				Invalid, Input));
			}
		}
		if let Self::Move { src, .. } = self {
			// Sorted by creating operation and then by offset, any overlap at all
			// shows up between neighbours.
			let mut spans: Vec<&ContentRange> = src.iter()
				.filter(|r| !r.is_empty())
				.collect();
			spans.sort_by_key(|r| (r.op, r.from));
			for pair in spans.windows(2) {
				if pair[0].intersects(pair[1]) {
					return Err(err!(
						"A Move names {} and {}, which overlap; one byte cannot be \
						moved to two places by one operation.", pair[0], pair[1];
					Invalid, Input, Conflict));
				}
			}
		}
		Ok(())
	}

	/// Reads an operation out of the durable vocabulary.
	///
	/// Both content operations cross unchanged, because both are anchored in
	/// content: this is a rename of the fields and nothing more. There is no
	/// translation step and no version to translate against, which is the point
	/// -- a positional splice would have needed to know what its author was
	/// looking at, and no such splice exists any more.
	///
	/// `None` is returned for an operation that says nothing about the order of
	/// bytes: a file created, deleted or renamed, or a mark. Those are history,
	/// and the log keeps them, but a sequence has no use for them.
	///
	/// The file an operation names is dropped here. A [`Sequence`] is one file's
	/// worth of operations and does not carry a path, so partitioning by
	/// [`Op::file`] is the caller's job.
	pub fn from_op(op: &Op)
		-> Option<Self>
	{
		match op {
			Op::Splice { left, right, remove, insert, .. } => Some(Self::Splice {
				left:	*left,
				right:	*right,
				remove:	remove.clone(),
				insert:	insert.clone(),
			}),
			Op::Move { src, left, right, .. } => Some(Self::Move {
				src:	src.clone(),
				left:	*left,
				right:	*right,
			}),
			Op::FileCreate { .. }	|
			Op::FileDelete { .. }	|
			Op::FileRename { .. }	|
			Op::Mark { .. }			=> None,
		}
	}
}


/// An operation as the sequence holds it: what it says, and what its author had
/// seen when they said it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Applied {
	/// The author's frontier when the operation was written.
	parents:	Vec<OpId>,
	/// What the operation says about the order of bytes.
	edit:		Edit,
}


/// A file's worth of operations, and the sequence they describe.
///
/// The state is the operation set and nothing else, so applying an operation is
/// idempotent, commutative and cheap, and the sequence itself is computed by
/// [`Sequence::render`] when it is wanted.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sequence {
	/// The operations, by identity.
	ops: BTreeMap<OpId, Applied>,
}

impl Sequence {

	/// Constructs an empty sequence.
	pub fn new() -> Self {
		Self { ops: BTreeMap::new() }
	}

	/// Builds a sequence from an operation set, in any order.
	pub fn build<I>(ops: I)
		-> Outcome<Self>
	where
		I: IntoIterator<Item = (Header, Edit)>,
	{
		let mut seq = Self::new();
		for (head, op) in ops {
			res!(seq.apply(head, op));
		}
		Ok(seq)
	}

	/// Applies an operation under the header that names it.
	///
	/// Applying the same operation twice does nothing the second time. Applying
	/// two different operations under one identity is refused: an identity names
	/// one operation, and a structure that quietly kept the first would converge
	/// on whichever replica saw which. Two headers differing only in their
	/// parents are two different operations for the same reason.
	pub fn apply(&mut self, head: Header, op: Edit)
		-> Outcome<()>
	{
		res!(op.validate());
		let applied = Applied { parents: head.parents, edit: op };
		match self.ops.get(&head.id) {
			Some(seen) if *seen != applied => Err(err!(
				"The identity {} already names a different {}; an operation \
				identity names one operation.", head.id, seen.edit.name();
			Invalid, Input, Conflict)),
			Some(_)	=> Ok(()),
			None	=> {
				self.ops.insert(head.id, applied);
				Ok(())
			},
		}
	}

	/// Applies a durable record, if it carries anything the sequence can use.
	///
	/// Returns whether the record said something about the order of bytes. This
	/// is the whole of the bridge between the log and the structure: a caller
	/// walks a causally ordered log, hands every record to the sequence for the
	/// file it names, and the sequence takes what belongs to it.
	pub fn apply_record(&mut self, rec: &Record)
		-> Outcome<bool>
	{
		match Edit::from_op(&rec.op) {
			Some(edit) => {
				res!(self.apply(rec.head.clone(), edit));
				Ok(true)
			},
			None => Ok(false),
		}
	}

	/// Returns the number of operations applied.
	pub fn len(&self) -> usize {
		self.ops.len()
	}

	/// Reports whether no operation has been applied.
	pub fn is_empty(&self) -> bool {
		self.ops.is_empty()
	}

	/// Reports whether the operation has been applied.
	pub fn contains(&self, id: &OpId) -> bool {
		self.ops.contains_key(id)
	}

	/// Returns the operation of that identity, if it has been applied.
	pub fn get(&self, id: &OpId)
		-> Option<&Edit>
	{
		self.ops.get(id).map(|a| &a.edit)
	}

	/// Returns the parents of an operation that has been applied.
	pub fn parents_of(&self, id: &OpId)
		-> Option<&[OpId]>
	{
		self.ops.get(id).map(|a| a.parents.as_slice())
	}

	/// Iterates the operations, in ascending order of identity.
	pub fn iter(&self)
		-> impl Iterator<Item = (&OpId, &Edit)>
	{
		self.ops.iter().map(|(id, a)| (id, &a.edit))
	}

	/// Returns the causal graph over the operations applied.
	pub fn causality(&self)
		-> Causality<'_>
	{
		Causality::new(self.ops.iter().map(|(id, a)| (*id, a.parents.as_slice())))
	}

	/// Returns the operations in op order, which is the order every stage of the
	/// render reads them in.
	fn in_op_order(&self) -> Vec<(OpId, &Edit)> {
		let mut ops: Vec<(OpId, &Edit)> = self.ops.iter()
			.map(|(id, a)| (*id, &a.edit))
			.collect();
		ops.sort_by_key(|(id, _)| OpOrder::of(id));
		ops
	}

	/// Renders the file, judging causality by the operations the sequence itself
	/// holds.
	///
	/// This is right where the sequence holds the whole history, which is to say
	/// a repository of one file. Where it holds one file of several, the parents
	/// of its operations name operations in the other files, and the graph to
	/// judge by is the repository's; use [`Sequence::render_with`] and give it
	/// one, from [`crate::log::OpLog::causality`] or from wherever else the whole
	/// history is held.
	pub fn render(&self)
		-> Outcome<Rendered>
	{
		self.render_with(&self.causality())
	}

	/// Renders the file against a causal graph the caller holds.
	///
	/// Fails if the graph is not causally closed, if it does not hold every
	/// operation the sequence does, or if the operation set names content it does
	/// not hold. Under a debug build the render is checked for conservation
	/// before it is returned; see [`Sequence::check_conservation`].
	pub fn render_with(&self, cause: &Causality<'_>)
		-> Outcome<Rendered>
	{
		let ops = self.in_op_order();
		res!(self.check_described(cause));
		res!(Self::check_parents(cause));
		let atoms = res!(Atoms::build(&ops));
		res!(Self::check_complete(&ops, &atoms));
		let dead = res!(Dead::build(&ops));
		let claims = res!(Claims::build(&ops));
		let slots = res!(Slots::place(&ops));
		let order = res!(slots.order(&claims));
		let walk = res!(render::traverse(&slots, &order, &claims, &dead, &atoms));

		let mut flags: Vec<Flag> = Vec::new();
		for (op, sub, origin) in &order.demoted {
			flags.push(Flag::Demoted { op: *op, sub: *sub, origin: *origin });
		}
		for (op, sub, origin) in &order.dropped {
			flags.push(Flag::Dropped { op: *op, sub: *sub, origin: *origin });
		}
		flags.extend(res!(Self::torn(&ops, &claims)));
		flags.extend(res!(Self::overlaps(&ops, cause)));
		flags.sort();
		flags.dedup();

		let stats = Stats {
			ops:				ops.len(),
			atoms:				atoms.count(),
			atom_bytes:			atoms.total(),
			slots_placed:		slots.placed(),
			slots_divided:		slots.len(),
			claim_intervals:	claims.intervals(),
			dead_intervals:		dead.intervals(),
			max_depth:			walk.max_depth,
			rendered:			walk.bytes.len() as u64,
		};
		let rendered = Rendered::new(walk.bytes, walk.runs, flags, stats);
		if cfg!(debug_assertions) {
			res!(Self::conserved(&rendered, &atoms, &dead));
		}
		Ok(rendered)
	}

	/// Checks that the render accounts for every byte the operation set created:
	/// each is either rendered exactly once or dead.
	///
	/// This is the property that catches a structural mistake the eye cannot,
	/// because a slot detached from the tree renders nothing and says nothing.
	/// The render runs it under a debug build; a caller wanting it in a release
	/// build calls it.
	pub fn check_conservation(&self, rendered: &Rendered)
		-> Outcome<()>
	{
		let ops = self.in_op_order();
		let atoms = res!(Atoms::build(&ops));
		let dead = res!(Dead::build(&ops));
		Self::conserved(rendered, &atoms, &dead)
	}

	/// Checks that the graph describes every operation the sequence holds.
	fn check_described(&self, cause: &Causality<'_>)
		-> Outcome<()>
	{
		for id in self.ops.keys() {
			if !cause.contains(id) {
				return Err(err!(
					"The causal graph does not hold the operation {}, which the \
					sequence does, so it cannot judge what that operation was \
					written against.", id;
				Invalid, Input, Missing));
			}
		}
		Ok(())
	}

	/// Checks that every operation an operation was written against is present.
	///
	/// This is the causal precondition proper, read off the parents rather than
	/// guessed at from the content. An operation whose parent is absent has been
	/// delivered ahead of history it depends on, and no amount of anchor
	/// resolution can make up the difference.
	fn check_parents(cause: &Causality<'_>)
		-> Outcome<()>
	{
		if let Some((id, missing)) = cause.gap() {
			return Err(err!(
				"The operation {} was written against {}, which the operation set \
				does not hold; the set is not causally complete.", id, missing;
			Invalid, Input, Missing));
		}
		Ok(())
	}

	/// Checks that every content identifier an operation names exists.
	fn check_complete(ops: &[(OpId, &Edit)], atoms: &Atoms)
		-> Outcome<()>
	{
		for (id, op) in ops {
			for r in op.regions() {
				if r.to > atoms.run_len(&r.op) {
					return Err(err!(
						"The operation {} names the content {}, which the operation \
						set does not hold; the set is not causally complete.", id, r;
					Invalid, Input, Missing));
				}
			}
			let (left, right) = op.origins();
			for a in [left, right].into_iter().flatten() {
				if a.content.off >= atoms.run_len(&a.content.op) {
					return Err(err!(
						"The operation {} is anchored {}, which the operation set \
						does not hold; the set is not causally complete.", id, a;
					Invalid, Input, Missing));
				}
			}
		}
		Ok(())
	}

	/// Finds the moves whose source is no longer wholly their own.
	fn torn(ops: &[(OpId, &Edit)], claims: &Claims)
		-> Outcome<Vec<Flag>>
	{
		let mut out: Vec<Flag> = Vec::new();
		for (id, op) in ops {
			if !op.is_move() {
				continue;
			}
			let mut lost: Vec<ContentRange> = Vec::new();
			for r in op.regions() {
				for (span, owner) in claims.runs(r) {
					if owner == *id {
						continue;
					}
					let gone = res!(ContentRange::new(r.op, span.start, span.end));
					match lost.last_mut() {
						Some(last) if last.op == gone.op && last.to == gone.from
							=> last.to = gone.to,
						_ => lost.push(gone),
					}
				}
			}
			if !lost.is_empty() {
				out.push(Flag::Torn { op: *id, lost });
			}
		}
		Ok(out)
	}

	/// Finds the pairs of concurrent operations that named the same content.
	///
	/// A sweep over the named runs, atom by atom: two operations overlap when one
	/// starts before another ends. Origins are not counted, because an origin
	/// names a gap rather than a claim on content, and two insertions at one gap
	/// are ordered rather than in conflict.
	///
	/// A pair that overlaps is then asked of the parents graph whether it was
	/// concurrent, and only a concurrent pair is flagged. An author who deleted a
	/// run they could already see was not in conflict with whoever wrote it, and
	/// saying so would make the flag noise. The concurrency test costs a walk of
	/// the graph per overlapping pair, and overlapping pairs are rare, so nothing
	/// is paid for the histories that have no conflict in them.
	fn overlaps(ops: &[(OpId, &Edit)], cause: &Causality<'_>)
		-> Outcome<Vec<Flag>>
	{
		let mut named: Vec<(ContentRange, OpId)> = Vec::new();
		for (id, op) in ops {
			for r in op.regions() {
				if !r.is_empty() {
					named.push((*r, *id));
				}
			}
		}
		named.sort_by_key(|(r, id)| (r.op, r.from, r.to, *id));
		let mut out: Vec<Flag> = Vec::new();
		// Runs still open at the current position, oldest first.
		let mut open: Vec<(ContentRange, OpId)> = Vec::new();
		for (r, id) in named {
			open.retain(|(o, _)| o.op == r.op && o.to > r.from);
			for (o, other) in &open {
				if *other == id {
					continue;
				}
				if !cause.concurrent(other, &id) {
					continue;
				}
				if let Some(region) = o.intersection(&r) {
					let mut pair = vec![*other, id];
					pair.sort();
					out.push(Flag::Overlap { ops: pair, region });
				}
			}
			open.push((r, id));
		}
		Ok(out)
	}

	/// The conservation check proper.
	fn conserved(rendered: &Rendered, atoms: &Atoms, dead: &Dead)
		-> Outcome<()>
	{
		let mut seen: BTreeMap<OpId, IntervalMap<()>> = BTreeMap::new();
		let mut emitted = 0u64;
		for Run { content, .. } in rendered.runs() {
			if content.is_empty() {
				continue;
			}
			emitted += content.len();
			res!(seen.entry(content.op).or_default().insert(content.offsets(), ()));
		}
		let distinct: u64 = seen.values()
			.flat_map(|m| m.iter())
			.map(|(iv, _)| iv.end - iv.start)
			.sum();
		if distinct != emitted {
			return Err(err!(
				"Conservation failed: {} bytes were rendered but only {} of them \
				are distinct, so a byte was shown in two places.", emitted, distinct;
			Bug, Conflict));
		}
		let buried = dead.within(atoms);
		if distinct + buried != atoms.total() {
			return Err(err!(
				"Conservation failed: {} bytes rendered plus {} dead against {} \
				created.", distinct, buried, atoms.total();
			Bug, Mismatch));
		}
		Ok(())
	}
}
