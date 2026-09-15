//! Convergence, which is the only thing a sync protocol is for.
//!
//! Every test here ends the same way: two logs that had diverged hold the same
//! operations and the same frontier. The divergences differ -- a clone, two
//! histories with nothing in common, one side far ahead, both sides a little
//! ahead -- and so do the modes, but the assertion does not.
//!
//! The pipe carries bytes and not values. Every message is encoded where it is
//! sent and decoded where it arrives, so the codec is exercised by every test
//! and the byte counts the mode comparison rests on are the bytes that would
//! cross a wire.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	OpId,
	ReplicaId,
};
use crate::log::OpLog;
use crate::op::{
	Header,
	Op,
	Record,
};
use crate::sync::msg::Message;
use crate::sync::session::{
	Growth,
	Mode,
	Session,
	Step,
	FANOUT,
};
use crate::sync::sketch::{
	Fallback,
	MIN_CELLS,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;


/// A small linear congruential generator, so a failure can be reproduced.
struct Rng(u64);

impl Rng {
	fn new(seed: u64) -> Self {
		Self(seed ^ 0x9e37_79b9_7f4a_7c15)
	}

	fn next(&mut self) -> usize {
		self.0 = self.0
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(self.0 >> 33) as usize
	}

	fn below(&mut self, n: usize) -> usize {
		if n == 0 { 0 } else { self.next() % n }
	}
}


/// What one exchange cost and how it went.
#[derive(Debug, Default)]
struct Tally {
	bytes:		usize,					// on the wire, both directions
	messages:	usize,					// on the wire, both directions
	ops:		usize,					// handed over, both directions
	fell_back:	Option<Fallback>,		// either side falling back to the walk
}


/// Both peers open at once, which is the harder case: neither has heard anything
/// when it works out what to say. A caller whose transport has a caller and a
/// callee simply does not call `open` on the callee's side.
fn exchange(a: &mut OpLog, b: &mut OpLog, mode: Mode)
	-> Outcome<Tally>
{
	let mut sa = Session::new(mode);
	let mut sb = Session::new(mode);
	let mut tally = Tally::default();
	// Each queue holds what is in flight towards that peer, as bytes.
	let mut to_a: Vec<Vec<u8>> = Vec::new();
	let mut to_b: Vec<Vec<u8>> = Vec::new();
	let put = |q: &mut Vec<Vec<u8>>, t: &mut Tally, msg: &Message| -> Outcome<()> {
		let bytes = res!(msg.encode());
		t.bytes += bytes.len();
		t.messages += 1;
		t.ops += msg.entries().len();
		q.push(bytes);
		Ok(())
	};
	res!(put(&mut to_b, &mut tally, &res!(sa.open(a))));
	res!(put(&mut to_a, &mut tally, &res!(sb.open(b))));
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		if guard > 64 {
			return Err(err!(
				"Two sessions exchanged {} times without converging.", guard;
			Test, Excessive));
		}
		if to_a.is_empty() && to_b.is_empty() {
			return Err(err!(
				"The pipe emptied with neither side converged: a is {}, b is {}.",
				sa.is_converged(), sb.is_converged();
			Test, Missing));
		}
		for bytes in std::mem::take(&mut to_a) {
			let turn = res!(sa.receive(a, res!(Message::decode(&bytes))));
			if let Step::FellBack(reason) = turn.step {
				tally.fell_back = Some(reason);
			}
			for msg in &turn.send {
				res!(put(&mut to_b, &mut tally, msg));
			}
		}
		for bytes in std::mem::take(&mut to_b) {
			let turn = res!(sb.receive(b, res!(Message::decode(&bytes))));
			if let Step::FellBack(reason) = turn.step {
				tally.fell_back = Some(reason);
			}
			for msg in &turn.send {
				res!(put(&mut to_a, &mut tally, msg));
			}
		}
	}
	Ok(tally)
}

/// Asserts that two logs hold the same operations and the same frontier.
fn agree(a: &OpLog, b: &OpLog)
	-> Outcome<()>
{
	let mut ids_a: Vec<OpId> = a.iter().map(|rec| rec.id()).collect();
	let mut ids_b: Vec<OpId> = b.iter().map(|rec| rec.id()).collect();
	ids_a.sort();
	ids_b.sort();
	if ids_a != ids_b {
		let missing: Vec<OpId> = ids_a.iter().filter(|id| !ids_b.contains(id)).copied().collect();
		let extra: Vec<OpId> = ids_b.iter().filter(|id| !ids_a.contains(id)).copied().collect();
		return Err(err!(
			"The logs hold {} and {} operations; {:?} is absent from the second and \
			{:?} from the first.", ids_a.len(), ids_b.len(), missing, extra;
		Test, Mismatch));
	}
	if a.frontier() != b.frontier() {
		return Err(err!(
			"The logs agree on their operations and not on their frontiers: {:?} \
			against {:?}.", a.frontier(), b.frontier();
		Test, Mismatch));
	}
	// And every record is the same record, not merely the same name.
	for id in &ids_a {
		if a.get(id) != b.get(id) {
			return Err(err!(
				"The logs hold different records for {}.", id; Test, Mismatch));
		}
	}
	Ok(())
}

fn write(log: &mut OpLog, replica: u64, n: usize, tag: &str)
	-> Outcome<()>
{
	let r = ReplicaId::new(replica);
	for i in 0..n {
		res!(log.author(r, Op::Mark { name: fmt!("{}{}", tag, i), body: None, time: None }));
	}
	Ok(())
}

/// An operation naming the whole frontier, which is a merge wherever the frontier
/// is wider than one.
fn merge(log: &mut OpLog, replica: u64, tag: &str)
	-> Outcome<()>
{
	res!(log.author(ReplicaId::new(replica), Op::Mark { name: fmt!("{}", tag), body: None, time: None }));
	Ok(())
}

/// A shared prefix, then each side writes and merges: the everyday divergence.
fn diverged(prefix: usize, left: usize, right: usize)
	-> Outcome<(OpLog, OpLog)>
{
	let mut a = OpLog::new();
	res!(write(&mut a, 1, prefix, "shared"));
	let mut b = a.clone();
	res!(write(&mut a, 1, left, "a"));
	res!(write(&mut b, 2, right, "b"));
	// A merge on each side, so neither history is a straight line.
	if left > 1 {
		res!(a.append(Record::new(
			res!(Header::new(a.next_id(ReplicaId::new(3)), a.frontier())),
			Op::Mark { name: fmt!("merge-a"), body: None, time: None },
		)));
	}
	if right > 1 {
		res!(b.append(Record::new(
			res!(Header::new(b.next_id(ReplicaId::new(4)), b.frontier())),
			Op::Mark { name: fmt!("merge-b"), body: None, time: None },
		)));
	}
	Ok((a, b))
}


#[test]
fn a_fresh_peer_takes_the_whole_history() -> Outcome<()> {
	for mode in [Mode::Walk, Mode::sketch(64)] {
		let mut a = OpLog::new();
		res!(write(&mut a, 1, 12, "x"));
		res!(merge(&mut a, 2, "join"));
		let mut b = OpLog::new();
		let tally = res!(exchange(&mut a, &mut b, mode));
		res!(agree(&a, &b));
		assert_eq!(b.len(), 13, "{:?}", mode);
		assert_eq!(tally.ops, 13, "exactly the history, once, under {:?}", mode);
	}
	Ok(())
}

#[test]
fn disjoint_histories_join() -> Outcome<()> {
	for mode in [Mode::Walk, Mode::sketch(64)] {
		let mut a = OpLog::new();
		res!(write(&mut a, 1, 7, "a"));
		let mut b = OpLog::new();
		res!(write(&mut b, 2, 9, "b"));
		res!(exchange(&mut a, &mut b, mode));
		res!(agree(&a, &b));
		assert_eq!(a.len(), 16, "{:?}", mode);
		// Two roots and no merge between them, so the frontier is wide.
		assert_eq!(a.frontier().len(), 2);
	}
	Ok(())
}

/// The peer that is behind is the loose direction: it cannot subtract a head it
/// has never seen, so it offers its whole log back, all five operations of which
/// are dropped on arrival. That is the walk's cost, and the reason for the other
/// mode.
#[test]
fn one_sided_divergence_converges() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 5, "shared"));
	let mut b = a.clone();
	res!(write(&mut a, 1, 60, "ahead"));
	res!(merge(&mut a, 3, "tip"));
	let before = b.len();
	let mut sa = Session::new(Mode::Walk);
	let mut sb = Session::new(Mode::Walk);
	let mut to_a = vec![res!(sb.open(&b))];
	let mut to_b = vec![res!(sa.open(&a))];
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		assert!(guard < 16);
		for msg in std::mem::take(&mut to_a) {
			to_b.extend(res!(sa.receive(&mut a, msg)).send);
		}
		for msg in std::mem::take(&mut to_b) {
			to_a.extend(res!(sb.receive(&mut b, msg)).send);
		}
	}
	res!(agree(&a, &b));
	assert_eq!(b.len(), before + 61);
	assert_eq!(sa.ops_sent(), 61, "the news and nothing else");
	assert_eq!(sb.ops_sent(), 5, "and the shared prefix, loosely, the other way");
	assert_eq!(sa.ops_absorbed(), 0, "every one of which was already held");
	Ok(())
}

/// One clone, cut into sessions by a carrier that bounds its reply, with `carry`
/// saying whether one session hands the next what it learned.
///
/// The bound is the relay's: what does not fit is not sent and not remembered,
/// and `Done` claims only that this end will send no more this turn, so the
/// puller opens again. An append-order prefix of an owed set is causally closed
/// by construction, which is what makes cutting one safe.
fn clone_in_sessions(a: &mut OpLog, cut: usize, carry: bool)
	-> Outcome<(usize, usize, OpLog)>
{
	let mut b = OpLog::new();
	let mut known: BTreeSet<OpId> = BTreeSet::new();
	let mut offered = 0usize;
	let mut sessions = 0usize;
	while b.len() < a.len() {
		sessions += 1;
		if sessions > 32 {
			return Err(err!(
				"The sessions stopped making progress, at {} of {}.", b.len(), a.len();
			Test, Excessive));
		}
		let mut sa = Session::new(Mode::Walk);
		let mut sb = if carry {
			Session::knowing(Mode::Walk, known.clone())
		} else {
			Session::new(Mode::Walk)
		};
		let hello = res!(sb.open(&b));
		let mut room = cut;
		for msg in res!(sa.receive(a, hello)).send {
			let msg = match msg {
				Message::Send { entries } => {
					let take = std::cmp::min(room, entries.len());
					room -= take;
					Message::Send { entries: entries.into_iter().take(take).collect() }
				},
				other => other,
			};
			for out in &res!(sb.receive(&mut b, msg)).send {
				offered += out.entries().len();
			}
		}
		if !sb.is_converged() {
			return Err(err!(
				"A session ended unconverged after {} of them.", sessions; Test, Missing));
		}
		known = sb.known().clone();
	}
	Ok((sessions, offered, b))
}

/// **The defect this exists for.** A relay bounds its reply, so a clone larger
/// than the bound is a run of sessions, and every session that began again from
/// the frontier alone offered back the whole prefix the sessions before it had
/// just delivered. Nothing in a frontier says "you handed me this": a head the
/// puller has never seen subtracts nothing, so its own log looked entirely owed.
///
/// Measured on fe2o3 on 12026-08-22, 35,314 operations over sixteen sessions:
/// 166,224 operations offered, every one of them already at the far end, 717 MB
/// up against 87 MB down. Here the same shape at four sessions, with the loose
/// run kept beside the tight one so the cost has a number rather than a claim.
#[test]
fn a_bounded_clone_offers_back_nothing_it_was_given() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 40, "x"));

	let (sessions, offered, b) = res!(clone_in_sessions(&mut a, 10, true));
	res!(agree(&a, &b));
	assert!(sessions > 1, "the bound was never reached, so nothing here was exercised");
	assert_eq!(offered, 0, "a clone owes nothing and offered {} operations", offered);

	// The same clone, with each session starting from the frontier alone.
	let (loose_sessions, loose, b) = res!(clone_in_sessions(&mut a, 10, false));
	res!(agree(&a, &b));
	assert_eq!(loose_sessions, sessions, "remembering changed how many sessions it took");
	// Ten, then twenty, then thirty: the triangular number the fe2o3 clone paid
	// 717 MB of.
	assert_eq!(loose, 60, "the loose run's cost is not what it was");
	Ok(())
}

/// The result does not depend on the mode.
#[test]
fn a_symmetric_divergence_converges_either_way() -> Outcome<()> {
	let (mut wa, mut wb) = res!(diverged(20, 4, 5));
	res!(exchange(&mut wa, &mut wb, Mode::Walk));
	res!(agree(&wa, &wb));
	let (mut sa, mut sb) = res!(diverged(20, 4, 5));
	let tally = res!(exchange(&mut sa, &mut sb, Mode::sketch(16)));
	res!(agree(&sa, &sb));
	assert!(tally.fell_back.is_none(), "an estimate of 16 for a difference of 11");
	// The two modes reach the same place.
	res!(agree(&wa, &sa));
	assert_eq!(wa.len(), 31);
	Ok(())
}

/// One exchange, and no operations.
#[test]
fn logs_that_already_agree_send_nothing() -> Outcome<()> {
	for mode in [Mode::Walk, Mode::sketch(4)] {
		let mut a = OpLog::new();
		res!(write(&mut a, 1, 10, "x"));
		let mut b = a.clone();
		let tally = res!(exchange(&mut a, &mut b, mode));
		res!(agree(&a, &b));
		assert_eq!(tally.ops, 0, "{:?}", mode);
		assert_eq!(tally.messages, 4, "an opening and a done each, under {:?}", mode);
	}
	Ok(())
}

/// The degenerate case must not be a special one.
#[test]
fn two_empty_logs_converge() -> Outcome<()> {
	for mode in [Mode::Walk, Mode::sketch(0)] {
		let mut a = OpLog::new();
		let mut b = OpLog::new();
		let tally = res!(exchange(&mut a, &mut b, mode));
		res!(agree(&a, &b));
		assert!(a.is_empty());
		assert_eq!(tally.ops, 0);
	}
	Ok(())
}

/// The walk answers in the same turn.
#[test]
fn an_undersized_sketch_falls_back_and_still_converges() -> Outcome<()> {
	// Two hundred apiece with nothing in common: four hundred of difference,
	// sketched as though there were none.
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 200, "a"));
	let mut b = OpLog::new();
	res!(write(&mut b, 2, 200, "b"));
	let tally = res!(exchange(&mut a, &mut b, Mode::sketch(0)));
	res!(agree(&a, &b));
	assert_eq!(a.len(), 400);
	match tally.fell_back {
		Some(Fallback::Incomplete { remaining, .. }) => assert!(remaining > 0),
		Some(other) => return Err(err!(
			"Expected a stalled decode, got {}.", other.why(); Test, Mismatch)),
		None => return Err(err!(
			"A sketch of sixteen cells decoded a difference of four hundred.";
		Test, Mismatch)),
	}
	Ok(())
}

/// And a stall grows before it falls back, and then gives the walk the turn.
///
/// The order is the point. A table that stalls says the estimate was low and
/// says nothing about by how much, so doubling it is tried before a frontier is
/// walked -- against a peer whose head this end does not hold, the walk's owed
/// set is the whole log. What stops the climb is not a count of growths: every
/// growth at least doubles, and it ends at whichever of `GROW_CELLS` and the size
/// [`Mode::between`] would not have chosen -- a table sized for as much as the
/// smaller log -- comes first. Two logs that share nothing reach the second of
/// those in a handful of turns, which is this test.
#[test]
fn a_fallback_is_reported_and_remembered() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 120, "a"));
	let mut b = OpLog::new();
	res!(write(&mut b, 2, 120, "b"));
	let mut sa = Session::new(Mode::sketch(0));
	let mut sb = Session::new(Mode::sketch(0));
	// B is called upon: it never opens on its own account, and answers what it
	// is sent.
	let opening = res!(sa.open(&a));
	let turn = res!(sb.receive(&mut b, res!(Message::decode(&res!(opening.encode())))));
	// Sixteen cells against a difference of two hundred and forty: the first
	// answer is a bigger table and not the walk, and nothing is handed over with
	// it.
	match turn.step {
		Step::Grew { cells } => assert!(cells > MIN_CELLS,
			"it grew to {} cells, which is no larger than the table it answered",
			cells),
		other => return Err(err!("The first answer was {:?}.", other; Test, Mismatch)),
	}
	assert_eq!(turn.send.len(), 1, "a turn that grew handed something over as well");
	assert!(sb.fell_back().is_none(), "growing was recorded as falling back");
	assert!(!sb.is_converged());
	// The rest of the exchange, by hand, so that the sticky flag can be read at
	// the end. Both ends grow once, both stall again, and the walk answers.
	let mut queue = turn.send;
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		if guard > 16 {
			return Err(err!(
				"The exchange took {} turns to converge.", guard; Test, Excessive));
		}
		let mut back = Vec::new();
		for msg in std::mem::take(&mut queue) {
			for out in res!(sa.receive(&mut a, msg)).send {
				back.push(out);
			}
		}
		for msg in back {
			for out in res!(sb.receive(&mut b, msg)).send {
				queue.push(out);
			}
		}
	}
	assert!(sa.is_converged());
	assert!(sb.is_converged());
	assert!(sb.fell_back().is_some(), "the fallback is remembered past convergence");
	assert!(sa.fell_back().is_some(), "and both sides made one");
	res!(agree(&a, &b));
	Ok(())
}

/// Everything that arrives closes causally against what the receiver already
/// holds. This is the property the whole protocol exists to preserve, so it is
/// asserted against the receiver's log at the moment of arrival rather than
/// inferred from the outcome.
#[test]
fn every_batch_closes_on_arrival() -> Outcome<()> {
	use crate::sync::walk::arrival_gap;

	for mode in [Mode::Walk, Mode::sketch(8)] {
		let (mut a, mut b) = res!(diverged(15, 6, 4));
		let mut sa = Session::new(mode);
		let mut sb = Session::new(mode);
		let mut to_a = vec![res!(sb.open(&b))];
		let mut to_b = vec![res!(sa.open(&a))];
		let mut checked = 0usize;
		let mut guard = 0usize;
		while !(sa.is_converged() && sb.is_converged()) {
			guard += 1;
			assert!(guard < 64, "no convergence under {:?}", mode);
			for msg in std::mem::take(&mut to_a) {
				if let Message::Send { entries } = &msg {
					assert!(!entries.is_empty());
					assert_eq!(
						res!(arrival_gap(&a, entries)), None,
						"a batch arriving at a under {:?} has a hole", mode,
					);
					checked += 1;
				}
				to_b.extend(res!(sa.receive(&mut a, msg)).send);
			}
			for msg in std::mem::take(&mut to_b) {
				if let Message::Send { entries } = &msg {
					assert_eq!(
						res!(arrival_gap(&b, entries)), None,
						"a batch arriving at b under {:?} has a hole", mode,
					);
					checked += 1;
				}
				to_a.extend(res!(sb.receive(&mut b, msg)).send);
			}
		}
		assert_eq!(checked, 2, "one batch each way under {:?}", mode);
		res!(agree(&a, &b));
	}
	Ok(())
}

/// Refused whole: the log is untouched.
#[test]
fn a_batch_with_a_hole_is_refused() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 6, "a"));
	let mut b = OpLog::new();
	let mut sa = Session::new(Mode::Walk);
	let mut sb = Session::new(Mode::Walk);
	let opening = res!(sb.open(&b));
	let turn = res!(sa.receive(&mut a, opening));
	// The batch a would have sent, with its first operation taken out.
	let mut holed = None;
	for msg in turn.send {
		if let Message::Send { entries } = msg {
			holed = Some(Message::Send { entries: entries[1..].to_vec() });
		}
	}
	let holed = match holed {
		Some(m) => m,
		None => return Err(err!("No batch was sent to hole."; Test, Missing)),
	};
	let e = match sb.receive(&mut b, holed) {
		Ok(_) => return Err(err!("A batch with a hole was absorbed."; Test, Mismatch)),
		Err(e) => e,
	};
	let text = fmt!("{}", e);
	assert!(text.contains("closures and not subsets"), "message was {}", text);
	assert!(b.is_empty(), "nothing was absorbed");
	Ok(())
}

/// Dropped rather than refused, which is what makes a loose owed set cost bytes
/// and not correctness.
#[test]
fn repeated_operations_are_dropped() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 5, "a"));
	let mut b = a.clone();
	let entries: Vec<crate::segment::Entry> = a.iter()
		.map(|rec| crate::segment::Entry::Bare(rec.clone()))
		.collect();
	let mut s = Session::new(Mode::Walk);
	// The same batch twice over, to catch a repetition within one message as
	// well as one the log already holds.
	let mut twice = entries.clone();
	twice.extend(entries);
	let turn = res!(s.receive(&mut b, Message::Send { entries: twice }));
	assert_eq!(turn.step, Step::NeedMore);
	assert_eq!(s.ops_absorbed(), 0);
	assert_eq!(b.len(), 5, "the log is as it was");
	// And into a log that holds none of it, a repeated batch places each once.
	let mut fresh = OpLog::new();
	let mut once: Vec<crate::segment::Entry> = a.iter()
		.map(|rec| crate::segment::Entry::Bare(rec.clone()))
		.collect();
	once.extend(once.clone());
	let mut s = Session::new(Mode::Walk);
	res!(s.receive(&mut fresh, Message::Send { entries: once }));
	assert_eq!(fresh.len(), 5);
	assert_eq!(s.ops_absorbed(), 5);
	Ok(())
}

/// That ratio is the whole reason the sketch mode exists.
///
/// Two logs of 204 operations differing by eight, both peers speaking: the walk
/// spends 29,608 bytes and moves 408 operations, the sketch spends 2,142 and
/// moves 8. Tripling the shared history leaves the sketch at 2,142 exactly and
/// would triple the walk.
#[test]
fn the_sketch_costs_the_difference_and_the_walk_costs_the_log() -> Outcome<()> {
	// Two hundred shared operations, three new on each side.
	let (mut wa, mut wb) = res!(diverged(200, 3, 3));
	let walk = res!(exchange(&mut wa, &mut wb, Mode::Walk));
	res!(agree(&wa, &wb));
	let (mut sa, mut sb) = res!(diverged(200, 3, 3));
	let sketch = res!(exchange(&mut sa, &mut sb, Mode::sketch(16)));
	res!(agree(&sa, &sb));
	assert!(sketch.fell_back.is_none());
	// The walk cannot subtract a head it has never seen, so each side sends its
	// whole log; the sketch sends the difference.
	assert_eq!(walk.ops, 2 * 204, "each side sent everything it holds");
	assert_eq!(sketch.ops, 8, "four operations each way");
	assert!(
		sketch.bytes * 8 < walk.bytes,
		"the sketch cost {} bytes and the walk {}: not the ratio the mode is for",
		sketch.bytes, walk.bytes,
	);
	// And the sketch's own cost does not grow with the history: the same
	// divergence over a longer shared prefix costs the same.
	let (mut la, mut lb) = res!(diverged(600, 3, 3));
	let longer = res!(exchange(&mut la, &mut lb, Mode::sketch(16)));
	res!(agree(&la, &lb));
	assert_eq!(longer.ops, sketch.ops);
	assert!(
		longer.bytes < sketch.bytes + 64,
		"a history three times longer cost {} bytes against {}",
		longer.bytes, sketch.bytes,
	);
	Ok(())
}

#[test]
fn a_soak_of_random_divergences_converges() -> Outcome<()> {
	let mut rng = Rng::new(0x5e_ed_50_ac);
	for trial in 0..60 {
		let prefix = rng.below(25);
		let left = rng.below(6);
		let right = rng.below(6);
		let mut a = OpLog::new();
		res!(write(&mut a, 1, prefix, "s"));
		let mut b = a.clone();
		// Each side writes its own, sometimes merging what it finds.
		for i in 0..left {
			if rng.below(4) == 0 && !a.is_empty() {
				res!(a.append(Record::new(
					res!(Header::new(a.next_id(ReplicaId::new(5)), a.frontier())),
					Op::Mark { name: fmt!("ma{}", i), body: None, time: None },
				)));
			} else {
				res!(write(&mut a, 1 + rng.below(2) as u64, 1, &fmt!("a{}", i)));
			}
		}
		for i in 0..right {
			if rng.below(4) == 0 && !b.is_empty() {
				res!(b.append(Record::new(
					res!(Header::new(b.next_id(ReplicaId::new(6)), b.frontier())),
					Op::Mark { name: fmt!("mb{}", i), body: None, time: None },
				)));
			} else {
				res!(write(&mut b, 3 + rng.below(2) as u64, 1, &fmt!("b{}", i)));
			}
		}
		let want = a.len() + b.len() - prefix;
		let mode = match trial % 3 {
			0 => Mode::Walk,
			1 => Mode::sketch(16),
			// Deliberately too small some of the time, so the fallback is soaked
			// as well as the happy path.
			_ => Mode::sketch(rng.below(3)),
		};
		let mut wa = a.clone();
		let mut wb = b.clone();
		match exchange(&mut wa, &mut wb, mode) {
			Ok(_) => {},
			Err(e) => return Err(err!(e,
				"Trial {} failed under {:?}: a prefix of {}, then {} and {}.",
				trial, mode, prefix, left, right; Test)),
		}
		match agree(&wa, &wb) {
			Ok(()) => {},
			Err(e) => return Err(err!(e,
				"Trial {} did not converge under {:?}.", trial, mode; Test)),
		}
		assert_eq!(wa.len(), want, "trial {} lost or invented an operation", trial);
	}
	Ok(())
}

/// There is no client and no server.
#[test]
fn either_side_may_open() -> Outcome<()> {
	// Only A opens; B answers what it was sent and opens in the same turn.
	let (mut a, mut b) = res!(diverged(10, 3, 2));
	let mut sa = Session::new(Mode::Walk);
	let mut sb = Session::new(Mode::Walk);
	let mut to_b = vec![res!(sa.open(&a))];
	let mut to_a: Vec<Message> = Vec::new();
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		assert!(guard < 16, "no convergence with a single opener");
		for msg in std::mem::take(&mut to_b) {
			to_a.extend(res!(sb.receive(&mut b, msg)).send);
		}
		for msg in std::mem::take(&mut to_a) {
			to_b.extend(res!(sa.receive(&mut a, msg)).send);
		}
	}
	res!(agree(&a, &b));
	assert!(sa.ops_sent() > 0 && sb.ops_sent() > 0, "both sides had news");
	Ok(())
}

#[test]
fn a_session_counts_what_it_moved() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 9, "a"));
	let mut b = OpLog::new();
	res!(write(&mut b, 2, 4, "b"));
	let mut sa = Session::new(Mode::Walk);
	let mut sb = Session::new(Mode::Walk);
	let mut to_a = vec![res!(sb.open(&b))];
	let mut to_b = vec![res!(sa.open(&a))];
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		assert!(guard < 16);
		for msg in std::mem::take(&mut to_a) {
			to_b.extend(res!(sa.receive(&mut a, msg)).send);
		}
		for msg in std::mem::take(&mut to_b) {
			to_a.extend(res!(sb.receive(&mut b, msg)).send);
		}
	}
	res!(agree(&a, &b));
	assert_eq!(sa.ops_sent(), 9);
	assert_eq!(sa.ops_absorbed(), 4);
	assert_eq!(sb.ops_sent(), 4);
	assert_eq!(sb.ops_absorbed(), 9);
	assert_eq!(sa.mode(), Mode::Walk);
	Ok(())
}

/// The rule is judged on shapes alone and cannot see overlap, so two logs of a
/// size that in fact share nothing are still offered a sketch. That is exactly
/// the bad estimate the fallback exists for, and it costs no round trip.
#[test]
fn the_mode_is_chosen_from_the_two_shapes() -> Outcome<()> {
	// A clone: nothing here, a history there.
	assert_eq!(Mode::between(0, 0, 400, 1), Mode::Walk);
	// A small divergence over a large shared history.
	match Mode::between(400, 1, 402, 1) {
		Mode::Sketch { estimate, .. } => assert_eq!(estimate, 2 + FANOUT * 2),
		other => return Err(err!("A small divergence chose {:?}.", other; Test, Mismatch)),
	}
	// Two logs of a size, both wide open: the guess reaches the smaller log, so
	// the walk is the cheaper answer.
	assert_eq!(Mode::between(20, 4, 20, 4), Mode::Walk);
	// One head each over the same length is a sketch, whatever the two logs turn
	// out to share.
	assert!(matches!(Mode::between(30, 1, 30, 1), Mode::Sketch { .. }));
	// And two empty logs do not divide by nothing.
	assert_eq!(Mode::between(0, 0, 0, 0), Mode::Walk);
	Ok(())
}

/// A session refuses a piece of an operation rather than trying to place it.
///
/// Putting a run of pieces back together is the carrier's work, and a session
/// that took one would be placing something nobody signed. The refusal names
/// where the work belongs, because a caller meeting it has reached for the wrong
/// layer rather than made a mistake.
#[test]
fn a_session_refuses_a_piece_of_an_operation() -> Outcome<()> {
	let mut log = OpLog::default();
	let mut session = Session::new(Mode::Walk);
	let e = match session.receive(&mut log, Message::Part {
		id:		OpId::new(ReplicaId::new(1), 1),
		seq:	0,
		total:	4,
		bytes:	vec![0x01],
	}) {
		Ok(_) => return Err(err!("A session placed a piece of an operation."; Test)),
		Err(e) => e,
	};
	assert!(fmt!("{}", e).contains("Parts"), "the refusal does not say where the work belongs: {}", e);
	assert_eq!(log.len(), 0, "a refused piece changed the log");
	Ok(())
}

/// What every message kind comes to, measured and then encoded, and the two
/// numbers compared.
///
/// [`Message::encoded_len`] is what both ends decide a body's contents by, so a
/// number one short of the truth is a body over a bound that was published and a
/// proxy closing the connection. The corpus is every kind, at the shapes whose
/// lengths are decided by something other than the message itself: a frontier of
/// none and of many, a send of none, one and many, and the pieces an operation
/// too large for the carrier is cut into.
///
/// Proved red by adding one to the answer, and again by leaving the magic and the
/// version out of it.
#[test]
fn encoded_len_is_what_the_message_encodes_to() -> Outcome<()> {
	let head = res!(Header::new(
		OpId::new(ReplicaId::new(5), 7),
		vec![OpId::new(ReplicaId::new(1), 1)],
	));
	let entry = |len: usize| crate::segment::Entry::Bare(Record::new(head.clone(), Op::Proposal {
		title:	fmt!("of {} bytes", len),
		body:	vec![0x5a; len],
		voice:	fmt!("wren"),
		time:	1_755_400_000,
	}));
	let heads: Vec<OpId> = (1..=40).map(|i| OpId::new(ReplicaId::new(i), i)).collect();
	// One piece over a mebibyte, so the pieces are many and the last one short.
	let big = crate::segment::Entry::Bare(Record::new(head.clone(), Op::Proposal {
		title:	fmt!("a large one"),
		body:	vec![0xa5; 3_000_000],
		voice:	fmt!("wren"),
		time:	1_755_400_000,
	}));
	let mut corpus = vec![
		(fmt!("an empty hello"),	Message::hello(Vec::new())),
		(fmt!("a wide hello"),		Message::hello(heads.clone())),
		(fmt!("an empty sketch"),	Message::sketch(Vec::new(), Vec::new(), 0)),
		(fmt!("a sketch"),			Message::sketch(heads, vec![0x11; 4_000], u64::MAX)),
		(fmt!("an empty send"),		Message::Send { entries: Vec::new() }),
		(fmt!("a send of one"),		Message::Send { entries: vec![entry(0)] }),
		(fmt!("a send of many"),	Message::Send {
			entries: (0..64).map(|i| entry(i * 37)).collect(),
		}),
		(fmt!("a done"),			Message::Done),
	];
	let pieces = res!(Message::part(&big, 1 << 20));
	assert!(pieces.len() > 2, "the operation went in {} pieces", pieces.len());
	for (i, piece) in pieces.into_iter().enumerate() {
		corpus.push((fmt!("piece {}", i), piece));
	}
	let mut kinds = BTreeSet::new();
	for (name, msg) in corpus {
		kinds.insert(msg.kind());
		let said = res!(msg.encoded_len());
		let wrote = res!(msg.encode()).len();
		assert_eq!(said, wrote,
			"{} is measured at {} bytes and encodes to {}", name, said, wrote);
	}
	assert_eq!(kinds.len(), 5, "the corpus covers {} of the five message kinds", kinds.len());
	Ok(())
}


/// Two logs that already agree cost a handful of hundred bytes, whatever the
/// history behind them.
///
/// The number nothing else in this file pins. A sync that carries nothing is the
/// ordinary outcome of a repository somebody syncs often, and what it costs is
/// the whole argument for sketching: the exchange is proportional to the
/// difference and not to the history, so a thousand-fold larger history has to
/// cost the same nothing.
#[test]
fn a_sync_that_carries_nothing_costs_almost_nothing() -> Outcome<()> {
	let mut a = OpLog::new();
	res!(write(&mut a, 1, 1_400, "x"));
	res!(merge(&mut a, 2, "join"));
	let mut b = a.clone();
	let mode = Mode::between(a.len(), a.frontier().len(), b.len(), 0);
	let tally = res!(exchange(&mut a, &mut b, mode));
	res!(agree(&a, &b));
	assert_eq!(tally.ops, 0, "an exchange between equals handed something over");
	assert!(tally.bytes < 4 << 10,
		"a no-op sync of {} operations cost {} bytes over {} messages",
		a.len(), tally.bytes, tally.messages);
	assert!(tally.fell_back.is_none(), "it fell back: {:?}", tally.fell_back);
	Ok(())
}

/// A two-sided divergence reconciles by sketch, at every size worth trying, and
/// never reaches the walk.
///
/// The case the estimate cannot see. Each side holds a head the other does not,
/// so the two logs' lengths say nothing about how far apart they are -- k against
/// k + 2 is two, and the truth is 2k + 2. Where the first table is too small the
/// answer is a larger table, so what this asserts is that the sketch path
/// finishes the job: the logs agree, nothing fell back to the walk, and the whole
/// exchange stays far under the history it reconciles.
#[test]
fn a_two_sided_divergence_decodes() -> Outcome<()> {
	for k in [1usize, 2, 5, 9, 17, 33, 64] {
		let (mut a, mut b) = res!(diverged(300, k, k + 2));
		let whole = a.len();
		// What each end knows before it opens: its own shape, the other's length,
		// and that the other's frontier is news to it.
		let mode = Mode::between(a.len(), a.frontier().len(), b.len(), b.frontier().len());
		let tally = res!(exchange(&mut a, &mut b, mode));
		res!(agree(&a, &b));
		assert!(tally.fell_back.is_none(),
			"at k = {} the exchange fell back to the walk: {:?}", k, tally.fell_back);
		// The walk would have offered the whole log from each side, since neither
		// can subtract the other's tip.
		assert!(tally.ops < whole,
			"at k = {} the exchange handed over {} operations of a {} operation log",
			k, tally.ops, whole);
	}
	Ok(())
}

/// A cursor carries a bounded walk forward, against a peer whose head the log
/// does not hold.
///
/// The fault in one place. A peer that has written anything of its own presents
/// a frontier this log cannot subtract, so the owed set is the whole log however
/// much of it that peer already holds -- and a carrier with a bounded reply sends
/// the same prefix every session, for ever. The cursor is what a session with no
/// memory is told instead, and it is one identifier: everything at or before it
/// in the append order is held, so the next owed set begins where the last one
/// stopped.
#[test]
fn a_cursor_moves_a_bounded_walk_along() -> Outcome<()> {
	let mut here = OpLog::new();
	res!(write(&mut here, 1, 60, "x"));
	// A peer holding the whole of it and one operation of its own, which is the
	// head this log has never seen.
	let mut there = here.clone();
	res!(write(&mut there, 2, 1, "mine"));
	let heads = there.frontier();
	assert_eq!(heads.len(), 1);
	assert!(!here.contains(&heads[0]), "the peer's head is one this log holds");

	// Without a cursor, every session owes the same whole log.
	let mut first = Session::new(Mode::Walk);
	let turn = res!(first.receive(&mut here.clone(), Message::hello(heads.clone())));
	let offered = turn.send.iter().map(|m| m.entries().len()).sum::<usize>();
	assert_eq!(offered, 60, "the walk offered {} of a 60 operation log", offered);

	// With one, the owed set begins after the operation it names. Twenty at a
	// time, which is what a bounded reply leaves behind.
	let mut at = 0usize;
	let mut sessions = 0usize;
	while at < 60 {
		sessions += 1;
		if sessions > 8 {
			return Err(err!(
				"{} sessions carried the walk to {} of 60.", sessions, at;
			Test, Excessive));
		}
		let mut session = Session::new(Mode::Walk);
		if at > 0 {
			let cursor = match here.at(at - 1) {
				Some(rec)	=> rec.id(),
				None		=> return Err(err!("The log lost operation {}.", at; Test, Missing)),
			};
			res!(session.receive(&mut here.clone(), Message::Resume { at: cursor }));
		}
		let turn = res!(session.receive(&mut here.clone(), Message::hello(heads.clone())));
		let mut sent: Vec<OpId> = Vec::new();
		for msg in &turn.send {
			for entry in msg.entries() {
				sent.push(res!(entry.id()));
			}
		}
		assert_eq!(sent.len(), 60 - at,
			"at cursor {} the session owed {} operations", at, sent.len());
		match here.at(at) {
			Some(rec)	=> assert_eq!(sent[0], rec.id(),
				"the session began at {} rather than at the cursor", sent[0]),
			None		=> return Err(err!("The log lost operation {}.", at; Test, Missing)),
		}
		// What a bounded reply would have carried of it.
		at += 20;
	}
	assert_eq!(sessions, 3, "the walk took {} sessions at twenty a turn", sessions);
	Ok(())
}


/// An end that grows against a peer that cannot answer a grown table strands it,
/// so it does not grow.
///
/// **The compatibility failure of the whole idea, and it is not symmetric.** A
/// grown table is a question, and the answer to it is the peer opening again at
/// the new shape. A peer built before growth existed opens once: fed a grown
/// table it decodes it, hands over what it owes, and the end that grew is left
/// holding a table nobody will answer -- it said only the table, so it never
/// worked out what it owed, and a carrier that keeps nothing between requests has
/// no opening to answer on the visit after.
///
/// Both halves are asserted here, because the second is the reason the first is a
/// knob rather than a rule. Told that its peer opens once, the end that would
/// have grown walks instead and the two logs agree; told nothing, it grows and
/// the pipe empties with it unconverged, which is the state a carrier reports as
/// unfinished.
#[test]
fn growth_against_a_peer_that_opens_once_is_refused() -> Outcome<()> {
	// A divergence the first table cannot hold, which is what makes the question
	// arise at all.
	let spread = |a: &OpLog, b: &OpLog| Mode::between(
		a.len(), a.frontier().len(), b.len(), b.frontier().len());

	// A opens once and never again, which is every build before the cursor. B
	// answers it, and is told what A is.
	let (mut a, mut b) = res!(diverged(300, 30, 32));
	let mode = spread(&a, &b);
	let mut sa = Session::new(mode).with_growth(Growth::Refused);
	let mut sb = Session::new(mode).with_growth(Growth::Refused);
	assert!(res!(called_upon(&mut sa, &mut a, &mut sb, &mut b)),
		"an exchange with a peer that opens once did not finish");
	res!(agree(&a, &b));
	assert!(sb.fell_back().is_some(), "B grew against a peer that opens once");

	// The same exchange with B told nothing: it grows, and A -- which opens once
	// -- answers the grown table and then has nothing more to say. B never made
	// its own opening count, so it is left unconverged with the pipe empty.
	let (mut a, mut b) = res!(diverged(300, 30, 32));
	let mut sa = Session::new(mode).with_growth(Growth::Refused);
	let mut sb = Session::new(mode);
	assert!(!res!(called_upon(&mut sa, &mut a, &mut sb, &mut b)),
		"a peer that opens once answered a grown table, so growth costs nothing \
		against it and this knob is not needed");
	assert!(!sb.is_converged(), "B is the end left holding a table nobody answered");

	// And between two ends that both re-open the same divergence settles by
	// sketch, so what is being refused above is a saving and not the exchange.
	let (mut c, mut d) = res!(diverged(300, 30, 32));
	let tally = res!(exchange(&mut c, &mut d, mode));
	res!(agree(&c, &d));
	assert!(tally.fell_back.is_none(),
		"two ends that both re-open fell back: {:?}", tally.fell_back);
	Ok(())
}

/// Runs an exchange where `sa` speaks first and `sb` answers, and says whether
/// both ends converged before the pipe emptied.
///
/// Unlike [`exchange`] it is not an error for the pipe to empty with one end
/// unfinished, because that is the outcome one of its callers is asserting.
fn called_upon(
	sa:	&mut Session,
	a:	&mut OpLog,
	sb:	&mut Session,
	b:	&mut OpLog,
)
	-> Outcome<bool>
{
	let mut queue = vec![res!(sa.open(a))];
	let mut guard = 0usize;
	while !(sa.is_converged() && sb.is_converged()) {
		guard += 1;
		if guard > 16 {
			return Err(err!(
				"An exchange took {} turns without converging.", guard; Test, Excessive));
		}
		if queue.is_empty() {
			return Ok(false);
		}
		let mut back = Vec::new();
		for msg in std::mem::take(&mut queue) {
			for out in res!(sb.receive(b, msg)).send {
				back.push(out);
			}
		}
		for msg in back {
			for out in res!(sa.receive(a, msg)).send {
				queue.push(out);
			}
		}
	}
	Ok(true)
}

/// Reading the sizing rule backwards lands exactly where it started.
///
/// Both directions cost something and they are not the same something. A cell
/// short of the table being answered is a stall put straight back on the wire; a
/// cell over is a round trip, because an arriving table wider than the last one
/// sent is exactly what a re-opening is read off, so a peer that answers with one
/// cell more than it was given is answered again.
#[test]
fn a_width_is_a_fixed_point_of_the_sizing_rule() -> Outcome<()> {
	use crate::sync::sketch::{
		cells_for,
		estimate_for,
		MAX_CELLS,
		MIN_CELLS,
	};

	// Every width a sketch can actually declare, which is the image of the sizing
	// rule and not every number between its ends.
	let mut estimate = 0usize;
	let mut seen = 0usize;
	while estimate <= (2 * MAX_CELLS) / 3 + 4 {
		let cells = cells_for(estimate);
		assert!(cells >= MIN_CELLS && cells <= MAX_CELLS);
		assert_eq!(cells_for(estimate_for(cells)), cells,
			"a table of {} cells is read back as an estimate of {}, which sizes {}",
			cells, estimate_for(cells), cells_for(estimate_for(cells)));
		seen += 1;
		estimate += 1;
	}
	assert!(seen > 600_000, "only {} widths were tried", seen);
	// And a width the rule cannot produce is rounded up rather than down, since
	// narrower is the direction that stalls.
	for cells in MIN_CELLS..4096 {
		assert!(cells_for(estimate_for(cells)) >= cells,
			"a table of {} cells is answered with {}", cells,
			cells_for(estimate_for(cells)));
	}
	Ok(())
}
