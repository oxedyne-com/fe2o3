//! A convergent repository in which a move is recorded as a move.
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
//! # A file is a subtree
//!
//! There is one forest for the whole repository, and its root children are the
//! files' **origin anchors**: one byte per file, born dead, minted by the
//! [`Op::FileCreate`] whose identity is the file's identity. A file is the
//! subtree beneath one of them, so a slot's file is read off the tree rather
//! than off the record, and no operation carries a file at all.
//!
//! Two consequences are the point of the arrangement. A move between files needs
//! no routing, because its destination anchor already names content in the file
//! it lands in; and an edit made concurrently inside a range that moves between
//! files follows it, for exactly the reason it follows an in-file move -- its
//! anchor never mentioned a file either.
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
//!   rather than where it now lives. It is flagged. **Where the cycle runs
//!   between two files, the demoted move lands in the other file and the file it
//!   left is emptied**; that outcome is deterministic, loses nothing, and reads
//!   badly, so [`Flag::CrossedFile`] says so in those terms. A rule that confines
//!   a cross-file cycle instead is design work owed.
//! - Content moved into a file that has been deleted renders nowhere a reader
//!   looks. Nothing is lost and [`Flag::MovedIntoDeleted`] says so.
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
//! Rendering requires a **causally complete** operation set, and that is checked
//! against the operations' own parents rather than inferred from what they happen
//! to name. Every parent must be present, and so must every atom whose content an
//! anchor or a range names, the origin anchors of the files included. An anchor
//! naming an atom that has not arrived cannot be resolved, and rather than guess,
//! the render fails and says which operation named what. [`crate::log::OpLog`]
//! supplies a closed set by construction; a caller assembling operations by hand
//! must arrange it.
//!
//! Rendering also lays out the whole repository, because ordering is
//! repository-wide. Laying out only the closure of one file's origin anchor under
//! the claim register would suffice and would usually be the file's own
//! operations; that is design work owed, and until it is done, opening one file
//! costs the repository.
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
mod file_tests;
#[cfg(test)]
mod tests;

use crate::id::{
	ContentRange,
	OpId,
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
	Repo,
	Run,
	Stats,
};
use crate::seq::slot::{
	Origin,
	Slots,
};

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


/// An operation as the sequence holds it: what it says, and what its author had
/// seen when they said it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Applied {
	/// The author's frontier when the operation was written.
	parents:	Vec<OpId>,
	/// What the operation says.
	op:			Op,
}


/// What the lifecycle operations say about one file.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileInfo {
	/// Where the file sits, after every rename the set holds.
	path:	Vec<u8>,
	/// Whether it still exists.
	live:	bool,
}


/// A repository's worth of operations, and the state they describe.
///
/// The state is the operation set and nothing else, so applying an operation is
/// idempotent, commutative and cheap, and the files themselves are computed by
/// [`Sequence::render`] when they are wanted. There is one sequence per
/// repository rather than one per file: a file is a subtree of the forest the
/// render lays out, and which subtree a slot is in is not knowable until the
/// forest is laid out.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sequence {
	/// The operations, by identity.
	ops: BTreeMap<OpId, Applied>,
}

impl Sequence {

	/// Constructs an empty repository.
	pub fn new() -> Self {
		Self { ops: BTreeMap::new() }
	}

	/// Builds a repository from an operation set, in any order.
	pub fn build<I>(ops: I)
		-> Outcome<Self>
	where
		I: IntoIterator<Item = (Header, Op)>,
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
	pub fn apply(&mut self, head: Header, op: Op)
		-> Outcome<()>
	{
		res!(op.validate());
		let id = head.id();
		let applied = Applied { parents: head.parents().to_vec(), op };
		match self.ops.get(&id) {
			Some(seen) if *seen != applied => Err(err!(
				"The identity {} already names a different {}; an operation \
				identity names one operation.", id, seen.op.name();
			Invalid, Input, Conflict)),
			Some(_)	=> Ok(()),
			None	=> {
				self.ops.insert(id, applied);
				Ok(())
			},
		}
	}

	/// Takes every operation of another repository, and returns how many of them
	/// were new.
	///
	/// This is what a merge is. The state is the operation set and nothing else,
	/// so two branches meet by taking the union of their sets, and the render of
	/// the union is the convergent merge -- which is a fact about the two sets and
	/// not about which branch absorbed which.
	///
	/// Nothing is asked of the two sets causally. Closure is checked where it
	/// matters, at [`Sequence::render_with`], against the graph the caller holds.
	///
	/// An identity naming a different operation in each set is refused, for the
	/// reason [`Sequence::apply`] refuses it, and nothing at all is taken: the two
	/// sets are not two versions of one history and no part of the merge is worth
	/// keeping.
	pub fn absorb(&mut self, other: &Self)
		-> Outcome<usize>
	{
		let mut fresh: Vec<(OpId, &Applied)> = Vec::new();
		for (id, applied) in &other.ops {
			match self.ops.get(id) {
				Some(seen) if seen != applied => return Err(err!(
					"The identity {} names a {} in one repository and a {} in the \
					other; an operation identity names one operation, so the two are \
					not branches of one history.", id, seen.op.name(), applied.op.name();
				Invalid, Input, Conflict)),
				Some(_)	=> (),
				None	=> fresh.push((*id, applied)),
			}
		}
		let n = fresh.len();
		for (id, applied) in fresh {
			self.ops.insert(id, applied.clone());
		}
		Ok(n)
	}

	/// Applies a durable record.
	///
	/// Everything the log holds belongs here, because the repository is what the
	/// structure now models: a file's creation mints its origin anchor, a rename
	/// changes its path, a deletion retires it, and the two content operations
	/// place bytes. Only a mark says nothing about any of it, and it is kept
	/// anyway so that the causal graph the render is judged by is not full of
	/// holes.
	pub fn apply_record(&mut self, rec: &Record)
		-> Outcome<()>
	{
		self.apply(rec.head.clone(), rec.op.clone())
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
		-> Option<&Op>
	{
		self.ops.get(id).map(|a| &a.op)
	}

	/// Returns the parents of an operation that has been applied.
	pub fn parents_of(&self, id: &OpId)
		-> Option<&[OpId]>
	{
		self.ops.get(id).map(|a| a.parents.as_slice())
	}

	/// Iterates the operations, in ascending order of identity.
	pub fn iter(&self)
		-> impl Iterator<Item = (&OpId, &Op)>
	{
		self.ops.iter().map(|(id, a)| (id, &a.op))
	}

	/// Returns the causal graph over the operations applied.
	pub fn causality(&self)
		-> Causality<'_>
	{
		Causality::new(self.ops.iter().map(|(id, a)| (*id, a.parents.as_slice())))
	}

	/// Returns the operations in op order, which is the order every stage of the
	/// render reads them in.
	fn in_op_order(&self) -> Vec<(OpId, &Op)> {
		let mut ops: Vec<(OpId, &Op)> = self.ops.iter()
			.map(|(id, a)| (*id, &a.op))
			.collect();
		ops.sort_by_key(|(id, _)| OpOrder::of(id));
		ops
	}

	/// Renders the repository, judging causality by the operations it holds.
	///
	/// This is the ordinary path, the sequence being the whole history. Where a
	/// caller holds the graph elsewhere -- a log covering more than this set --
	/// use [`Sequence::render_with`].
	pub fn render(&self)
		-> Outcome<Repo>
	{
		self.render_with(&self.causality())
	}

	/// Renders the repository against a causal graph the caller holds.
	///
	/// Fails if the graph is not causally closed, if it does not hold every
	/// operation the sequence does, or if the operation set names content or a
	/// file it does not hold. Under a debug build the render is checked for
	/// conservation before it is returned; see [`Sequence::check_conservation`].
	pub fn render_with(&self, cause: &Causality<'_>)
		-> Outcome<Repo>
	{
		let ops = self.in_op_order();
		res!(self.check_described(cause));
		res!(Self::check_parents(cause));
		let atoms = res!(Atoms::build(&ops));
		res!(Self::check_complete(&ops, &atoms));
		let files = res!(Self::files(&ops));
		let dead = res!(Dead::build(&ops));
		let claims = res!(Claims::build(&ops));
		let slots = res!(Slots::place(&ops));
		let order = res!(slots.order(&claims));
		let walk = res!(render::traverse(&slots, &order, &claims, &dead, &atoms));

		// The association a wire field would have asserted, derived instead.
		let mut index: BTreeMap<OpId, OpId> = BTreeMap::new();
		for (i, slot) in slots.all().iter().enumerate() {
			if let Some(f) = walk.owner[i] {
				index.insert(slot.place, f);
			}
		}

		let mut flags: Vec<Flag> = Vec::new();
		for (op, sub, origin) in &order.demoted {
			flags.push(Flag::Demoted { op: *op, sub: *sub, origin: *origin });
			if let Some(f) = Self::crossed_file(
				&slots, &claims, &walk.owner, *op, *sub, *origin)
			{
				flags.push(f);
			}
		}
		for (op, sub, origin) in &order.dropped {
			flags.push(Flag::Dropped { op: *op, sub: *sub, origin: *origin });
		}
		for (op, sub) in &walk.orphans {
			flags.push(Flag::Orphaned { op: *op, sub: *sub });
		}
		for (id, op) in &ops {
			if !op.is_move() {
				continue;
			}
			if let Some(f) = index.get(id) {
				if !files.get(f).map(|i| i.live).unwrap_or(false) {
					flags.push(Flag::MovedIntoDeleted { op: *id, file: *f });
				}
			}
		}
		flags.extend(res!(Self::torn(&ops, &claims, cause)));
		flags.extend(res!(Self::overlaps(&ops, cause)));
		flags.sort();
		flags.dedup();

		// Each file keeps the flags that concern it, and the repository keeps all
		// of them; a flag naming an operation that reached no file is the
		// repository's alone.
		let mut per_file: BTreeMap<OpId, Vec<Flag>> = BTreeMap::new();
		for flag in &flags {
			for f in Self::flag_files(flag, &index) {
				per_file.entry(f).or_default().push(flag.clone());
			}
		}

		let mut walk_files = walk.files;
		let mut out: Vec<Rendered> = Vec::new();
		let mut rendered = 0u64;
		let mut withheld = 0u64;
		for (id, info) in &files {
			let (bytes, runs) = walk_files.remove(id).unwrap_or_default();
			rendered += bytes.len() as u64;
			if !info.live {
				withheld += bytes.len() as u64;
			}
			let mut flags = per_file.remove(id).unwrap_or_default();
			flags.dedup();
			out.push(Rendered::new(*id, info.path.clone(), info.live, bytes, runs, flags));
		}

		let stats = Stats {
			ops:				ops.len(),
			files:				files.len(),
			atoms:				atoms.count(),
			atom_bytes:			atoms.total(),
			slots_placed:		slots.placed(),
			slots_divided:		slots.len(),
			claim_intervals:	claims.intervals(),
			dead_intervals:		dead.intervals(),
			max_depth:			walk.max_depth,
			rendered,
			withheld,
			orphaned:			walk.orphaned,
		};
		let repo = Repo::new(out, flags, index, stats);
		if cfg!(debug_assertions) {
			res!(Self::conserved(&repo, &atoms, &dead));
		}
		Ok(repo)
	}

	/// Checks that the render accounts for every byte the operation set created:
	/// each is either rendered exactly once, somewhere in the repository, or
	/// dead, or owned by a slot that reached no file at all.
	///
	/// The check is repository-wide because that is where it bites. A byte
	/// rendered in two files at once is what a claim register scoped to one file
	/// produces, and no per-file check would see it: each file would agree with
	/// itself while the same bytes appeared in both. The render runs this under a
	/// debug build; a caller wanting it in a release build calls it.
	pub fn check_conservation(&self, repo: &Repo)
		-> Outcome<()>
	{
		let ops = self.in_op_order();
		let atoms = res!(Atoms::build(&ops));
		let dead = res!(Dead::build(&ops));
		Self::conserved(repo, &atoms, &dead)
	}

	/// Collects what the lifecycle operations say about each file, in op order.
	fn files(ops: &[(OpId, &Op)])
		-> Outcome<BTreeMap<OpId, FileInfo>>
	{
		let mut files: BTreeMap<OpId, FileInfo> = BTreeMap::new();
		for (id, op) in ops {
			match op {
				Op::FileCreate { path } => {
					files.insert(*id, FileInfo { path: path.clone(), live: true });
				},
				Op::FileRename { file, path } => {
					match files.get_mut(file) {
						Some(info)	=> info.path = path.clone(),
						None		=> return Err(err!(
							"The operation {} renames the file {}, which no operation \
							in the set created; the set is not causally complete.",
							id, file;
						Invalid, Input, Missing)),
					}
				},
				Op::FileDelete { file } => {
					match files.get_mut(file) {
						Some(info)	=> info.live = false,
						None		=> return Err(err!(
							"The operation {} deletes the file {}, which no operation \
							in the set created; the set is not causally complete.",
							id, file;
						Invalid, Input, Missing)),
					}
				},
				_ => (),
			}
		}
		Ok(files)
	}

	/// Raises a cross-file flag where a cycle demotion put a placement in a
	/// different file from the one its anchor content lives in.
	fn crossed_file(
		slots:	&Slots,
		claims:	&Claims,
		owner:	&[Option<OpId>],
		op:		OpId,
		sub:	u64,
		origin:	Origin,
	)
		-> Option<Flag>
	{
		let i = match slots.find(&op, sub) {
			Some(i)	=> i,
			None	=> return None,
		};
		let slot = match slots.get(i) {
			Ok(s)	=> s,
			Err(_)	=> return None,
		};
		let anchor = match origin {
			Origin::Left	=> slot.left,
			Origin::Right	=> slot.right,
		}?;
		// Where the anchored content lives now, which is where the placement
		// would have gone had the cycle not forced it elsewhere.
		let target = slots.owner_slot(&anchor.content, claims, false).ok()?;
		let from = (*owner.get(target)?)?;
		let to = (*owner.get(i)?)?;
		if from == to {
			return None;
		}
		Some(Flag::CrossedFile { op, sub, origin, from, to })
	}

	/// Returns the files a flag concerns, so that each file keeps its own.
	fn flag_files(flag: &Flag, index: &BTreeMap<OpId, OpId>) -> Vec<OpId> {
		let mut out: Vec<OpId> = Vec::new();
		match flag {
			Flag::Overlap { ops, .. } => {
				for id in ops {
					if let Some(f) = index.get(id) {
						out.push(*f);
					}
				}
			},
			Flag::CrossedFile { from, to, .. } => {
				out.push(*from);
				out.push(*to);
			},
			Flag::MovedIntoDeleted { file, .. } => out.push(*file),
			other => {
				if let Some(id) = other.op() {
					if let Some(f) = index.get(&id) {
						out.push(*f);
					}
				}
			},
		}
		out.sort();
		out.dedup();
		out
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
	///
	/// A file's origin anchor is one such identifier, so an operation anchored at
	/// the start of a file the set does not hold is refused here along with
	/// everything else it might have named.
	fn check_complete(ops: &[(OpId, &Op)], atoms: &Atoms)
		-> Outcome<()>
	{
		for (id, op) in ops {
			for r in op.regions() {
				if r.to() > atoms.run_len(&r.op()) {
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

	/// Finds the moves whose source was taken from them by a **concurrent** move.
	///
	/// The claim register alone cannot tell a race from a sequence: a move
	/// superseded on purpose, by a later move of the same content, looks exactly
	/// like one that lost a race, because in both cases the register names
	/// somebody else. The parents say which it was, and only a concurrent claim
	/// tears. An author who moved a block and then moved it again is owed no flag,
	/// and a flag they cannot act on is noise that hides the ones they can.
	fn torn(ops: &[(OpId, &Op)], claims: &Claims, cause: &Causality<'_>)
		-> Outcome<Vec<Flag>>
	{
		let mut out: Vec<Flag> = Vec::new();
		for (id, op) in ops {
			if !op.is_move() {
				continue;
			}
			let mut lost: Vec<ContentRange> = Vec::new();
			for r in op.regions() {
				for (span, holder) in claims.runs(r) {
					if holder == *id || !cause.concurrent(&holder, id) {
						continue;
					}
					let gone = res!(ContentRange::new(r.op(), span.start, span.end));
					match lost.last_mut() {
						Some(last) if last.op() == gone.op() && last.to() == gone.from()
							=> res!(last.set_to(gone.to())),
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
	fn overlaps(ops: &[(OpId, &Op)], cause: &Causality<'_>)
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
		named.sort_by_key(|(r, id)| (r.op(), r.from(), r.to(), *id));
		let mut out: Vec<Flag> = Vec::new();
		// Runs still open at the current position, oldest first.
		let mut open: Vec<(ContentRange, OpId)> = Vec::new();
		for (r, id) in named {
			open.retain(|(o, _)| o.op() == r.op() && o.to() > r.from());
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

	/// The conservation check proper, over the whole repository.
	fn conserved(repo: &Repo, atoms: &Atoms, dead: &Dead)
		-> Outcome<()>
	{
		let mut seen: BTreeMap<OpId, IntervalMap<()>> = BTreeMap::new();
		let mut emitted = 0u64;
		for file in repo.files() {
			for Run { content, .. } in file.runs() {
				if content.is_empty() {
					continue;
				}
				emitted += content.len();
				res!(seen.entry(content.op()).or_default().insert(content.offsets(), ()));
			}
		}
		let distinct: u64 = seen.values()
			.flat_map(|m| m.iter())
			.map(|(iv, _)| iv.end - iv.start)
			.sum();
		if distinct != emitted {
			return Err(err!(
				"Conservation failed: {} bytes were rendered across the repository \
				but only {} of them are distinct, so a byte was shown in two places.",
				emitted, distinct;
			Bug, Conflict));
		}
		let buried = dead.within(atoms);
		let orphaned = repo.stats().orphaned;
		if distinct + buried + orphaned != atoms.total() {
			return Err(err!(
				"Conservation failed: {} bytes rendered plus {} dead plus {} orphaned \
				against {} created.", distinct, buried, orphaned, atoms.total();
			Bug, Mismatch));
		}
		Ok(())
	}
}
