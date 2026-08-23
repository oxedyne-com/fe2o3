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
	Mode,
	Session,
	Step,
	FANOUT,
};
use crate::sync::sketch::Fallback;

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
	assert!(matches!(turn.step, Step::FellBack(_)), "step was {:?}", turn.step);
	assert!(sb.fell_back().is_some());
	assert!(!sb.is_converged(), "it has told, and not heard");
	// The rest of the exchange, by hand, so that the sticky flag can be read at
	// the end.
	for msg in turn.send {
		let back = res!(sa.receive(&mut a, msg));
		for msg in back.send {
			res!(sb.receive(&mut b, msg));
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
