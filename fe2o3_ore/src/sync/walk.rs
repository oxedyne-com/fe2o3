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
//! roots   = (their heads ∪ what they have handed us) ∩ our log
//! covered = ancestors-or-self, within our log, of roots
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
//! # What they handed us
//!
//! The second root set is the one a frontier cannot report. A peer that hands an
//! operation over holds it -- a proof, not a claim -- and its heads say nothing
//! about it, since a head we have never seen subtracts nothing. Held in one
//! session that costs nothing, because a session sends what it owes once. It
//! costs a carrier that runs several: a bounded reply makes a large clone into a
//! run of sessions, and a session that started again from the frontier alone
//! offered back, every time, the whole prefix the sessions before it had just
//! delivered.
//!
//! Measured on the clone of fe2o3 of 12026-08-22, 35,314 operations over sixteen
//! sessions: 166,224 operations offered, every one of them already held at the
//! far end, 717 MB up against 87 MB down. [`crate::sync::Session::knowing`] is
//! how a carrier carries the answer across the boundary.
//!
//! # Closure at both ends
//!
//! The sender closes what it is about to send against what it believes the
//! receiver holds ([`close`]), and the receiver checks the property on arrival
//! rather than trusting it ([`arrival_gap`]). The first is a proof obligation
//! discharged where the information is; the second is what stops a peer that got
//! it wrong from leaving a hole in someone else's history.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::OpId;
use crate::log::OpLog;
use crate::segment::Entry;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;


/// The operations of `log` that a peer whose frontier is `heads` demonstrably
/// holds. Heads the log does not hold are skipped: they are operations the peer
/// has and we do not, and they say nothing about what we hold.
///
/// `known` is the rest of what the peer has shown it holds, which is what it
/// handed over earlier in the same exchange. Those are roots of the same walk and
/// not a separate kind of thing: a log is causally closed, so a peer that holds an
/// operation holds every ancestor of it, exactly as it does of a head.
pub fn covered(log: &OpLog, heads: &[OpId], known: &BTreeSet<OpId>) -> BTreeSet<OpId> {
	let mut seen: BTreeSet<OpId> = BTreeSet::new();
	let mut stack: Vec<OpId> = heads
		.iter()
		.chain(known.iter())
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

/// In the log's append order, which is a linear extension of the causal order,
/// so a receiver places the whole batch in one pass; nothing depends on it,
/// since [`OpLog::absorb`](crate::log::OpLog::absorb) takes a batch however it
/// is shuffled.
pub fn owed(log: &OpLog, heads: &[OpId], known: &BTreeSet<OpId>) -> Vec<OpId> {
	let held = covered(log, heads, known);
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

/// In the log's append order. Fails where the log does not hold one of them,
/// which would mean the sender had worked out a set it cannot deliver.
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

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// Nothing remembered beyond the frontier, which is what a first session knows.
	fn nil() -> BTreeSet<OpId> {
		BTreeSet::new()
	}

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

	/// The ancestors of a head are subtracted whichever branch they sit on.
	#[test]
	fn a_head_we_hold_covers_its_ancestors() -> Outcome<()> {
		let log = res!(forked());
		assert_eq!(
			covered(&log, &[oid(1, 2)], &nil()).into_iter().collect::<Vec<_>>(),
			vec![oid(1, 1), oid(1, 2)],
		);
		assert_eq!(owed(&log, &[oid(1, 2)], &nil()), vec![oid(2, 3), oid(1, 4)]);
		// A merge covers both branches, so nothing is owed.
		assert!(owed(&log, &[oid(1, 4)], &nil()).is_empty());
		// Two heads together cover their union.
		assert!(owed(&log, &[oid(1, 2), oid(2, 3)], &nil()) == vec![oid(1, 4)]);
		Ok(())
	}

	/// The clone case, and the answer is exactly the log.
	#[test]
	fn an_empty_peer_is_owed_everything() -> Outcome<()> {
		let log = res!(forked());
		assert!(covered(&log, &[], &nil()).is_empty());
		assert_eq!(owed(&log, &[], &nil()), vec![oid(1, 1), oid(1, 2), oid(2, 3), oid(1, 4)]);
		Ok(())
	}

	/// It is news for us, and says nothing about what the peer holds, so the owed
	/// set is loose in that case and never wrong.
	#[test]
	fn a_head_we_do_not_hold_covers_nothing() -> Outcome<()> {
		let log = res!(forked());
		assert!(covered(&log, &[oid(9, 9)], &nil()).is_empty());
		assert_eq!(owed(&log, &[oid(9, 9)], &nil()).len(), log.len(), "the whole log, loosely");
		// Mixed: the head we hold still does its work.
		assert_eq!(owed(&log, &[oid(9, 9), oid(1, 2)], &nil()), vec![oid(2, 3), oid(1, 4)]);
		Ok(())
	}

	/// The half a frontier cannot report. A peer's heads say what it holds *now*;
	/// nothing in them says "you handed me this", and a head nobody here has seen
	/// subtracts nothing at all.
	#[test]
	fn what_they_handed_us_is_not_offered_back() -> Outcome<()> {
		let log = res!(forked());
		// Their tip is news to us, so on the frontier alone the whole log is owed.
		let theirs = vec![oid(9, 9)];
		assert_eq!(owed(&log, &theirs, &nil()).len(), log.len());
		// Remember two they handed over, and it is exactly the rest.
		let handed: BTreeSet<OpId> = [oid(1, 1), oid(1, 2)].into_iter().collect();
		assert_eq!(
			covered(&log, &theirs, &handed).into_iter().collect::<Vec<_>>(),
			vec![oid(1, 1), oid(1, 2)],
		);
		assert_eq!(owed(&log, &theirs, &handed), vec![oid(2, 3), oid(1, 4)]);
		// One that they handed over covers its ancestors as a head does, since a log
		// is causally closed: the merge alone leaves nothing owed.
		let handed: BTreeSet<OpId> = [oid(1, 4)].into_iter().collect();
		assert!(owed(&log, &theirs, &handed).is_empty());
		// And an operation nobody here holds says nothing, exactly as their tip does.
		let handed: BTreeSet<OpId> = [oid(8, 8)].into_iter().collect();
		assert_eq!(owed(&log, &theirs, &handed).len(), log.len());
		Ok(())
	}

	/// Which is what makes it safe to subtract, and the reason a remembered root is
	/// walked rather than merely removed: a peer that holds an operation holds every
	/// ancestor of it, so what is subtracted has to be closed downwards or the
	/// remainder arrives at the far end with a hole in it.
	#[test]
	fn what_is_subtracted_is_closed_downwards() -> Outcome<()> {
		let log = res!(forked());
		for handed in [
			vec![],
			vec![oid(1, 1)],
			vec![oid(1, 2)],
			vec![oid(2, 3)],
			vec![oid(1, 2), oid(2, 3)],
			// The merge, whose parents were never handed over by name.
			vec![oid(1, 4)],
			vec![oid(1, 4), oid(8, 8)],
		] {
			let handed: BTreeSet<OpId> = handed.into_iter().collect();
			let held = covered(&log, &[oid(9, 9)], &handed);
			for id in &held {
				let rec = match log.get(id) {
					Some(r) => r,
					None => return Err(err!("The log lost {}.", id; Test, Missing)),
				};
				for p in rec.parents() {
					assert!(held.contains(p),
						"{} is subtracted and its parent {} is not, remembering {:?}",
						id, p, handed);
				}
			}
			// So a peer holding exactly that much takes what is left of the log with
			// no hole in it, which is the check the receiver makes for itself.
			let mut peer = OpLog::new();
			let mut have = Vec::new();
			for rec in log.iter() {
				if held.contains(&rec.id()) {
					have.push(rec.clone());
				}
			}
			res!(peer.absorb(have));
			let ids: BTreeSet<OpId> = owed(&log, &[oid(9, 9)], &handed).into_iter().collect();
			let entries = res!(entries_for(&log, &ids));
			assert_eq!(res!(arrival_gap(&peer, &entries)), None,
				"what is left over does not close, remembering {:?}", handed);
		}
		Ok(())
	}

	/// Which is what makes it safe to send in any order.
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
			let held = covered(&log, &heads, &nil());
			let ids = owed(&log, &heads, &nil());
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

	/// Pulling in what it is missing, which is what the step is for.
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

	/// The gap names both operations, the one that arrived and the parent nobody
	/// holds.
	#[test]
	fn arrival_names_the_hole() -> Outcome<()> {
		let log = res!(forked());
		let mut fresh = OpLog::new();
		let all = res!(entries_for(&log, &owed(&log, &[], &nil()).into_iter().collect()));
		assert!(res!(closes(&fresh, &all)), "a whole history closes against nothing");
		// Drop the root, and the batch no longer closes.
		let short: Vec<Entry> = all[1..].to_vec();
		assert_eq!(res!(arrival_gap(&fresh, &short)), Some((oid(1, 2), oid(1, 1))));
		// Absorb the root, and it does.
		res!(fresh.absorb(vec![res!(all[0].peek())]));
		assert!(res!(closes(&fresh, &short)));
		Ok(())
	}

	/// And a set naming what the log does not hold is refused rather than half
	/// delivered.
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
