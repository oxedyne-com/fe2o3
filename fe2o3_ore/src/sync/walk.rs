//! What a peer at a given frontier is owed, and the closure that makes it safe
//! to send.
//!
//! # The owed set
//!
//! A peer says what its frontier is. Everything it holds is an ancestor of one
//! of those heads, because a log is causally closed and its frontier dominates
//! it. So of the operations we hold, the ones it demonstrably holds too are the
//! ancestors -- and the heads themselves -- of every head we can also see:
//!
//! ```text
//! covered = ancestors-or-self, within our log, of (their heads ∩ our log)
//! owed    = our log \ covered
//! ```
//!
//! That is sound: everything in `covered` is genuinely theirs, so nothing they
//! lack is ever left out. It is not tight. A head of theirs we have never seen
//! is news *for us*, and tells us nothing about what they hold, so the branch it
//! sits on cannot be subtracted; where both peers have written since they last
//! spoke, neither can subtract the other's tip and each sends its whole log.
//! What the receiver already holds it drops, so the cost of being loose is
//! bytes and never correctness.
//!
//! Two cases are exactly tight, and they are the common ones:
//!
//! - A peer that is behind us and has written nothing of its own -- a clone, a
//!   fetch after someone else pushed -- has heads we hold, so `covered` is
//!   precisely its log and `owed` is precisely the news.
//! - A peer that has everything we have gets an empty owed set and one message.
//!
//! Where both sides have written, [`crate::sync::sketch`] is what makes the
//! exchange proportional to the difference instead.
//!
//! # Closure at both ends
//!
//! The sender closes what it is about to send against what it believes the
//! receiver holds ([`close`]), and the receiver checks the property on arrival
//! rather than trusting it ([`arrival_gap`]). The first is a proof obligation
//! discharged where the information is; the second is what stops a peer that got
//! it wrong from leaving a hole in someone else's history.

use crate::id::OpId;
use crate::log::OpLog;
use crate::segment::Entry;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;


/// The operations of `log` that a peer whose frontier is `heads` demonstrably
/// holds.
///
/// Heads the log does not hold are skipped: they are operations the peer has and
/// we do not, and they say nothing about what we hold.
pub fn covered(log: &OpLog, heads: &[OpId]) -> BTreeSet<OpId> {
	let mut seen: BTreeSet<OpId> = BTreeSet::new();
	let mut stack: Vec<OpId> = heads
		.iter()
		.filter(|h| log.contains(h))
		.copied()
		.collect();
	while let Some(id) = stack.pop() {
		if !seen.insert(id) {
			continue;
		}
		// A record in the log has every parent in the log, by the log's own
		// append guard, so the walk never leaves it.
		if let Some(rec) = log.get(&id) {
			for p in rec.parents() {
				if !seen.contains(p) {
					stack.push(*p);
				}
			}
		}
	}
	seen
}

/// The operations a peer whose frontier is `heads` is owed, in the log's append
/// order.
///
/// Append order is a linear extension of the causal order, so a receiver places
/// the whole batch in one pass; nothing depends on it, since
/// [`OpLog::absorb`](crate::log::OpLog::absorb) takes a batch however it is
/// shuffled.
pub fn owed(log: &OpLog, heads: &[OpId]) -> Vec<OpId> {
	let held = covered(log, heads);
	log.iter()
		.map(|rec| rec.id())
		.filter(|id| !held.contains(id))
		.collect()
}

/// Extends a send set with every ancestor the receiver is not known to hold, so
/// that what arrives closes causally against what is already there.
///
/// `held` is what the sender believes the receiver has. Ancestors outside both
/// sets are pulled in; an identifier the log does not hold is dropped, since
/// nothing can be said about an operation nobody has.
///
/// For a send set worked out by [`owed`] this adds nothing, and the same is true
/// of a difference a sketch decoded in full: both are complements of an
/// ancestor-closed set, and the complement of an ancestor-closed set is closed
/// downwards by construction. The step is here because that is a property of the
/// *inputs*, and this function is what makes it a property of the output.
pub fn close(log: &OpLog, send: &[OpId], held: &BTreeSet<OpId>) -> BTreeSet<OpId> {
	let mut out: BTreeSet<OpId> = BTreeSet::new();
	let mut stack: Vec<OpId> = Vec::new();
	for id in send {
		if log.contains(id) && !held.contains(id) && out.insert(*id) {
			stack.push(*id);
		}
	}
	while let Some(id) = stack.pop() {
		if let Some(rec) = log.get(&id) {
			for p in rec.parents() {
				if !held.contains(p) && out.insert(*p) {
					stack.push(*p);
				}
			}
		}
	}
	out
}

/// Returns the records named by `ids`, in the log's append order.
///
/// Fails where the log does not hold one of them, which would mean the sender
/// had worked out a set it cannot deliver.
pub fn entries_for(log: &OpLog, ids: &BTreeSet<OpId>)
	-> Outcome<Vec<Entry>>
{
	for id in ids {
		if !log.contains(id) {
			return Err(err!(
				"The log does not hold {}, so it cannot be sent.", id;
			Invalid, Input, Missing));
		}
	}
	Ok(log.iter()
		.filter(|rec| ids.contains(&rec.id()))
		.map(|rec| Entry::Bare(rec.clone()))
		.collect())
}

/// Returns the first operation of a batch whose parent neither the batch nor the
/// log holds, together with that parent.
///
/// `None` means the batch closes causally against the log: absorbing it leaves
/// no hole. This is the check a receiver makes before it absorbs anything, and
/// it is deliberately not [`crate::log::Causality::gap`], which sees only the set
/// it was built over and would have to be handed the whole log to answer the
/// question.
pub fn arrival_gap(log: &OpLog, entries: &[Entry])
	-> Outcome<Option<(OpId, OpId)>>
{
	let mut arriving: BTreeSet<OpId> = BTreeSet::new();
	let mut records = Vec::with_capacity(entries.len());
	for entry in entries {
		let rec = res!(entry.peek());
		arriving.insert(rec.id());
		records.push(rec);
	}
	for rec in &records {
		for p in rec.parents() {
			if !arriving.contains(p) && !log.contains(p) {
				return Ok(Some((rec.id(), *p)));
			}
		}
	}
	Ok(None)
}

/// Reports whether a batch closes causally against the log.
pub fn closes(log: &OpLog, entries: &[Entry])
	-> Outcome<bool>
{
	Ok(res!(arrival_gap(log, entries)).is_none())
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::id::ReplicaId;
	use crate::op::{
		Header,
		Op,
		Record,
	};

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A record of a mark, with the given parents.
	fn rec(id: OpId, parents: Vec<OpId>, name: &str)
		-> Outcome<Record>
	{
		Ok(Record::new(
			res!(Header::new(id, parents)),
			Op::Mark { name: fmt!("{}", name), body: None, time: None },
		))
	}

	/// A log of a chain, a fork, and a merge:
	///
	/// ```text
	/// a -- b -- d
	///  \       /
	///   ------c
	/// ```
	fn forked()
		-> Outcome<OpLog>
	{
		let mut log = OpLog::new();
		res!(log.append(Record::root(oid(1, 1), Op::Mark { name: fmt!("a"), body: None, time: None })));
		res!(log.append(res!(rec(oid(1, 2), vec![oid(1, 1)], "b"))));
		res!(log.append(res!(rec(oid(2, 3), vec![oid(1, 1)], "c"))));
		res!(log.append(res!(rec(oid(1, 4), vec![oid(1, 2), oid(2, 3)], "d"))));
		Ok(log)
	}

	/// A peer at a head we hold is owed everything that head does not cover, and
	/// the ancestors of that head are subtracted whichever branch they sit on.
	#[test]
	fn a_head_we_hold_covers_its_ancestors() -> Outcome<()> {
		let log = res!(forked());
		assert_eq!(
			covered(&log, &[oid(1, 2)]).into_iter().collect::<Vec<_>>(),
			vec![oid(1, 1), oid(1, 2)],
		);
		assert_eq!(owed(&log, &[oid(1, 2)]), vec![oid(2, 3), oid(1, 4)]);
		// A merge covers both branches, so nothing is owed.
		assert!(owed(&log, &[oid(1, 4)]).is_empty());
		// Two heads together cover their union.
		assert!(owed(&log, &[oid(1, 2), oid(2, 3)]) == vec![oid(1, 4)]);
		Ok(())
	}

	/// A peer with no history at all is owed the whole log, which is the clone
	/// case, and the answer is exactly the log.
	#[test]
	fn an_empty_peer_is_owed_everything() -> Outcome<()> {
		let log = res!(forked());
		assert!(covered(&log, &[]).is_empty());
		assert_eq!(owed(&log, &[]), vec![oid(1, 1), oid(1, 2), oid(2, 3), oid(1, 4)]);
		Ok(())
	}

	/// A head we have never seen covers nothing: it is news for us, and says
	/// nothing about what the peer holds. The owed set is loose in that case and
	/// never wrong.
	#[test]
	fn a_head_we_do_not_hold_covers_nothing() -> Outcome<()> {
		let log = res!(forked());
		assert!(covered(&log, &[oid(9, 9)]).is_empty());
		assert_eq!(owed(&log, &[oid(9, 9)]).len(), log.len(), "the whole log, loosely");
		// Mixed: the head we hold still does its work.
		assert_eq!(owed(&log, &[oid(9, 9), oid(1, 2)]), vec![oid(2, 3), oid(1, 4)]);
		Ok(())
	}

	/// The owed set is causally closed against what the peer holds, which is what
	/// makes it safe to send in any order.
	#[test]
	fn the_owed_set_closes_against_the_peer() -> Outcome<()> {
		let log = res!(forked());
		for heads in [
			vec![],
			vec![oid(1, 1)],
			vec![oid(1, 2)],
			vec![oid(2, 3)],
			vec![oid(1, 2), oid(2, 3)],
			vec![oid(1, 4)],
			vec![oid(9, 9)],
		] {
			let held = covered(&log, &heads);
			let ids = owed(&log, &heads);
			// Every parent of everything sent is either sent or held.
			for id in &ids {
				let rec = match log.get(id) {
					Some(r) => r,
					None => return Err(err!("The log lost {}.", id; Test, Missing)),
				};
				for p in rec.parents() {
					assert!(
						ids.contains(p) || held.contains(p),
						"{} names {}, which is neither sent nor held at {:?}", id, p, heads,
					);
				}
			}
			// Which is what closing adds nothing to.
			let closed = close(&log, &ids, &held);
			assert_eq!(
				closed.into_iter().collect::<Vec<_>>(),
				{ let mut v = ids.clone(); v.sort(); v },
				"closing an owed set at {:?} added something", heads,
			);
		}
		Ok(())
	}

	/// Closing a set that is *not* closed pulls in what it is missing, which is
	/// what the step is for.
	#[test]
	fn closing_repairs_a_hole() -> Outcome<()> {
		let log = res!(forked());
		// The merge alone, with the peer holding nothing: its parents and their
		// parent all have to go too.
		let closed = close(&log, &[oid(1, 4)], &BTreeSet::new());
		// A set is a set, so it comes back in identifier order rather than in the
		// log's append order.
		assert_eq!(
			closed.into_iter().collect::<Vec<_>>(),
			vec![oid(1, 1), oid(1, 2), oid(1, 4), oid(2, 3)],
		);
		// With the peer holding one branch, only the other is pulled in.
		let held: BTreeSet<OpId> = [oid(1, 1), oid(1, 2)].into_iter().collect();
		let closed = close(&log, &[oid(1, 4)], &held);
		assert_eq!(closed.into_iter().collect::<Vec<_>>(), vec![oid(1, 4), oid(2, 3)]);
		// An identifier nobody holds is dropped rather than invented.
		assert!(close(&log, &[oid(9, 9)], &BTreeSet::new()).is_empty());
		Ok(())
	}

	/// A batch closes against a log that already holds the parents, and does not
	/// when it does not; the gap names both operations.
	#[test]
	fn arrival_names_the_hole() -> Outcome<()> {
		let log = res!(forked());
		let mut fresh = OpLog::new();
		let all = res!(entries_for(&log, &owed(&log, &[]).into_iter().collect()));
		assert!(res!(closes(&fresh, &all)), "a whole history closes against nothing");
		// Drop the root, and the batch no longer closes.
		let short: Vec<Entry> = all[1..].to_vec();
		assert_eq!(res!(arrival_gap(&fresh, &short)), Some((oid(1, 2), oid(1, 1))));
		// Absorb the root, and it does.
		res!(fresh.absorb(vec![res!(all[0].peek())]));
		assert!(res!(closes(&fresh, &short)));
		Ok(())
	}

	/// The entries a send set names come back in append order, and a set naming
	/// what the log does not hold is refused rather than half delivered.
	#[test]
	fn entries_follow_the_append_order() -> Outcome<()> {
		let log = res!(forked());
		let ids: BTreeSet<OpId> = [oid(1, 4), oid(1, 1)].into_iter().collect();
		let got = res!(entries_for(&log, &ids));
		let mut names: Vec<OpId> = Vec::new();
		for entry in &got {
			names.push(res!(entry.peek()).id());
		}
		assert_eq!(names, vec![oid(1, 1), oid(1, 4)]);
		let absent: BTreeSet<OpId> = [oid(9, 9)].into_iter().collect();
		assert!(entries_for(&log, &absent).is_err());
		Ok(())
	}
}
