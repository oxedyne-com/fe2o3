//! The adversarial cases, each with the answer the literature or the design
//! prescribes, and each checked under every delivery order.
//!
//! A case that merely converges proves little, because the state here is the
//! operation set and a render is a function of it, so agreement between
//! delivery orders is nearly free. What the cases test is that the answer
//! converged on is the right one: the one the published counter-examples say a
//! correct structure must give.

use crate::id::{
	Anchor,
	ContentId,
	ContentRange,
	OpId,
	ReplicaId,
};
use crate::op::{
	Header,
	Op,
	Record,
};
use crate::seq::render::{
	Flag,
	Rendered,
	Run,
};
use crate::seq::slot::Origin;
use crate::seq::{
	Edit,
	Sequence,
};

use oxedyne_fe2o3_core::prelude::*;


/// The shopping list of the published worked case, with plain hyphens so that
/// byte offsets and character offsets coincide.
const LIST: &[u8] = b"- Eggs\n- Milk\n- Cheese\n";

/// Twenty bytes whose order is easy to read off a rendered string.
const ALPHA: &[u8] = b"0123456789ABCDEFGHIJ";


/// One replica: the frontend that turns index-based editing intent into
/// content-anchored operations, which is what a real editor would be.
struct Replica {
	/// The replica number every operation of this replica is named by.
	id:		u64,
	/// The operations it holds.
	seq:	Sequence,
}

impl Replica {

	/// Constructs a replica holding nothing.
	fn new(id: u64) -> Self {
		Self { id, seq: Sequence::new() }
	}

	/// Mints the next header: a Lamport counter, and everything this replica can
	/// see as the operation's parents.
	fn next_head(&self)
		-> Outcome<Header>
	{
		let seen = self.seq.iter().map(|(id, _)| id.counter).max().unwrap_or(0);
		Header::new(
			OpId::new(ReplicaId::new(self.id), seen + 1),
			self.seq.causality().heads(),
		)
	}

	/// Receives an operation from another replica.
	fn recv(&mut self, op: (Header, Edit))
		-> Outcome<()>
	{
		self.seq.apply(op.0, op.1)
	}

	/// Renders the replica's own view.
	fn view(&self)
		-> Outcome<Rendered>
	{
		self.seq.render()
	}

	/// Records an operation of this replica's own, and applies it.
	fn author(&mut self, op: Edit)
		-> Outcome<(Header, Edit)>
	{
		let head = res!(self.next_head());
		res!(self.seq.apply(head.clone(), op.clone()));
		Ok((head, op))
	}

	/// Inserts bytes at a rendered index.
	fn insert(&mut self, at: usize, bytes: &[u8])
		-> Outcome<(Header, Edit)>
	{
		let op = res!(res!(self.view()).splice(at, 0, bytes.to_vec()));
		self.author(op)
	}

	/// Deletes a run at a rendered index.
	fn delete(&mut self, at: usize, len: usize)
		-> Outcome<(Header, Edit)>
	{
		let op = res!(res!(self.view()).splice(at, len, Vec::new()));
		self.author(op)
	}

	/// Replaces a run at a rendered index, in one operation.
	fn replace(&mut self, at: usize, len: usize, bytes: &[u8])
		-> Outcome<(Header, Edit)>
	{
		let op = res!(res!(self.view()).splice(at, len, bytes.to_vec()));
		self.author(op)
	}

	/// Moves a rendered run to a rendered index.
	fn move_range(&mut self, at: usize, len: usize, to: usize)
		-> Outcome<(Header, Edit)>
	{
		let op = res!(res!(self.view()).move_range(at, len, to));
		self.author(op)
	}
}

/// Seeds `replicas` replicas with one splice carrying the whole initial text.
fn seed(text: &[u8], replicas: u64)
	-> Outcome<(Vec<Replica>, (Header, Edit))>
{
	let mut origin = Replica::new(0);
	let op = res!(origin.insert(0, text));
	let mut out = Vec::new();
	for i in 1..=replicas {
		let mut r = Replica::new(i);
		res!(r.recv(op.clone()));
		out.push(r);
	}
	Ok((out, op))
}

/// Generates every permutation of `idx`.
fn permute(idx: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
	if k == idx.len() {
		out.push(idx.clone());
		return;
	}
	for i in k..idx.len() {
		idx.swap(k, i);
		permute(idx, k + 1, out);
		idx.swap(k, i);
	}
}

/// Applies an operation set in every delivery order, requiring that all of them
/// render the same bytes and raise the same flags, and returns that render.
fn converge(ops: &[(Header, Edit)])
	-> Outcome<Rendered>
{
	let n = ops.len();
	let mut orders: Vec<Vec<usize>> = Vec::new();
	if n <= 6 {
		let mut idx: Vec<usize> = (0..n).collect();
		permute(&mut idx, 0, &mut orders);
	} else {
		for k in 0..n {
			orders.push((0..n).map(|i| (i + k) % n).collect());
		}
		orders.push((0..n).rev().collect());
	}
	let mut first: Option<Rendered> = None;
	for order in &orders {
		let mut seq = Sequence::new();
		for i in order {
			res!(seq.apply(ops[*i].0.clone(), ops[*i].1.clone()));
		}
		let got = res!(seq.render());
		res!(seq.check_conservation(&got));
		match &first {
			None => first = Some(got),
			Some(want) => {
				if want.bytes() != got.bytes() {
					return Err(err!(
						"Delivery order changed the render: {:?} against {:?}.",
						want.text_lossy(), got.text_lossy();
					Test, Mismatch));
				}
				if want.flags() != got.flags() {
					return Err(err!(
						"Delivery order changed the flags: {:?} against {:?}.",
						want.flags(), got.flags();
					Test, Mismatch));
				}
			},
		}
	}
	match first {
		Some(r)	=> Ok(r),
		None	=> Err(err!("No delivery order was tried."; Test, Bug)),
	}
}

/// Runs an operation set under every delivery order and checks the render
/// against the answer the case prescribes.
fn case(expect: &str, ops: &[(Header, Edit)])
	-> Outcome<Rendered>
{
	let got = res!(converge(ops));
	assert_eq!(got.text_lossy(), expect);
	Ok(got)
}

/// Counts the flags of one kind.
fn count(rendered: &Rendered, kind: fn(&Flag) -> bool) -> usize {
	rendered.flags().iter().filter(|f| kind(f)).count()
}

/// Whether a flag reports a torn move.
fn is_torn(flag: &Flag) -> bool {
	matches!(flag, Flag::Torn { .. })
}

/// Whether a flag reports a demoted origin.
fn is_demoted(flag: &Flag) -> bool {
	matches!(flag, Flag::Demoted { .. })
}

/// Whether a flag reports a dropped origin.
fn is_dropped(flag: &Flag) -> bool {
	matches!(flag, Flag::Dropped { .. })
}

/// Whether a flag reports two operations naming the same content.
fn is_overlap(flag: &Flag) -> bool {
	matches!(flag, Flag::Overlap { .. })
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE TEN ADVERSARIAL CASES                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// The case the structure exists for. One replica moves a list item to the top
/// while another rewrites a word inside it; the published construction loses the
/// rewrite, because the insertion is anchored to a position the move did not
/// touch. Here the anchor names content, the move claimed that content, and the
/// insertion goes where the content went.
#[test]
fn an_edit_inside_a_moved_line_travels_with_it() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let (b, a) = reps.split_at_mut(1);
	// Replica 1 moves "- Milk\n" to the top.
	let mv = res!(b[0].move_range(7, 7, 0));
	// Replica 2 concurrently turns "Milk" into "Soy milk".
	let ed = res!(a[0].replace(9, 1, b"Soy m"));
	let out = res!(case("- Soy milk\n- Eggs\n- Cheese\n", &[first, mv, ed]));
	assert_eq!(count(&out, is_torn), 0);
	assert_eq!(count(&out, is_demoted), 0);
	Ok(())
}

/// Two replicas move the identical run to different destinations. One copy
/// survives, at the destination of the move that is higher in op order, and the
/// loser is told its source is no longer its own. Duplication here is the
/// anomaly the published single-element construction exists to remove.
#[test]
fn two_moves_of_one_run_leave_one_copy() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let seed_id = first.0.id;
	let (r1, r2) = reps.split_at_mut(1);
	let lost = res!(r1[0].move_range(7, 7, 0));		// Replica 1 loses.
	let won = res!(r2[0].move_range(7, 7, 23));		// Replica 2 wins.
	let out = res!(case("- Eggs\n- Cheese\n- Milk\n", &[first, lost.clone(), won]));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		lost.0.id,
		lost:	vec![res!(ContentRange::new(seed_id, 7, 14))],
	}), "flags were {:?}", out.flags());
	assert_eq!(count(&out, is_overlap), 1,
		"the two moves named the same seven bytes");
	Ok(())
}

/// Two replicas move partly overlapping runs. The block tears at the overlap:
/// twenty bytes in, twenty bytes out, nothing duplicated, nothing lost, and
/// neither author's block in one piece. That is the prescribed outcome, and the
/// flag on the losing move is what makes it acceptable rather than merely
/// tolerable.
#[test]
fn overlapping_moves_tear_at_the_overlap() -> Outcome<()> {
	let (mut reps, first) = res!(seed(ALPHA, 2));
	let seed_id = first.0.id;
	let (r1, r2) = reps.split_at_mut(1);
	let torn = res!(r1[0].move_range(0, 10, 20));	// Replica 1 loses the overlap.
	let won = res!(r2[0].move_range(5, 10, 0));		// Replica 2 wins it.
	let out = res!(case("FGHIJ56789ABCDE01234", &[first, torn.clone(), won]));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		torn.0.id,
		lost:	vec![res!(ContentRange::new(seed_id, 5, 10))],
	}), "flags were {:?}", out.flags());
	Ok(())
}

/// Two moves whose destinations sit inside each other's sources: a cycle in the
/// anchor graph that neither replica could have known it was making. Two origins
/// are demoted to the splice that created their content, all twenty bytes
/// survive, and the lower move lands where its anchor content was written rather
/// than where it now lives.
#[test]
fn mutually_nested_destinations_break_the_cycle_without_loss() -> Outcome<()> {
	let (mut reps, first) = res!(seed(ALPHA, 2));
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 moves "01234" into the middle of "ABCDE".
	let m1 = res!(r1[0].move_range(0, 5, 12));
	// Replica 2 moves "ABCDE" into the middle of "01234".
	let m2 = res!(r2[0].move_range(10, 5, 2));
	let out = res!(case("5678901ABCDE234FGHIJ", &[first, m1.clone(), m2]));
	assert_eq!(out.len(), 20, "no byte may be lost to a cycle");
	assert_eq!(count(&out, is_dropped), 0, "demotion sufficed");
	// Both origins of the lower move in op order give way, and the higher move
	// keeps the destination it asked for.
	assert_eq!(out.flags(), &[
		Flag::Demoted { op: m1.0.id, sub: 0, origin: Origin::Left },
		Flag::Demoted { op: m1.0.id, sub: 0, origin: Origin::Right },
	]);
	Ok(())
}

/// An insertion strictly inside a concurrently moved run lands inside the run at
/// its destination.
#[test]
fn an_insertion_inside_a_moved_run_goes_with_it() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_range(7, 7, 0));
	let ed = res!(r2[0].insert(11, b"!"));
	res!(case("- Mi!lk\n- Eggs\n- Cheese\n", &[first, mv, ed]));
	Ok(())
}

/// Three replicas write runs at one point. The runs stay whole and follow op
/// order; interleaving them would be the failure most published algorithms
/// exhibit.
#[test]
fn three_concurrent_runs_at_one_point_do_not_interleave() -> Outcome<()> {
	let (mut reps, first) = res!(seed(b"AB", 3));
	let mut ops = vec![first];
	for (i, r) in reps.iter_mut().enumerate() {
		let run = match i {
			0	=> b"xxx".to_vec(),
			1	=> b"yyy".to_vec(),
			_	=> b"zzz".to_vec(),
		};
		ops.push(res!(r.insert(1, &run)));
	}
	res!(case("AxxxyyyzzzB", &ops));
	Ok(())
}

/// Insertions abutting a moved run, one immediately before its start and one
/// immediately after its end. The asymmetry is inherent and worth stating: an
/// insertion abutting the start stays where it was, one abutting the end travels
/// with the move. Under the published ordering rule read literally, the first of
/// them lands at the end of the file instead, which is the failure the successor
/// rule exists to prevent.
#[test]
fn edits_abutting_a_moved_run_stay_beside_their_neighbour() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_range(7, 7, 0));
	let before = res!(r2[0].insert(7, b"<"));
	let after = res!(r2[0].insert(15, b">"));
	res!(case("- Milk\n>- Eggs\n<- Cheese\n", &[first, mv, before, after]));
	Ok(())
}

/// A move and a deletion inside the moved run need no tie-break between them.
/// The bytes move, and they are dead, and a dead byte renders as nothing
/// wherever it is.
#[test]
fn a_move_and_a_deletion_inside_it_compose() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_range(7, 7, 0));
	let del = res!(r2[0].delete(9, 4));				// "Milk"
	res!(case("- \n- Eggs\n- Cheese\n", &[first, mv, del]));
	Ok(())
}

/// Two replicas that disagree about where the block they are moving begins and
/// ends. The claim register gives each byte to the higher mover, so the lower
/// move keeps only what the higher one did not want, and its fragments render
/// apart from each other. Deterministic, flagged, and not what its author meant.
#[test]
fn moves_with_different_boundaries_split_between_them() -> Outcome<()> {
	let (mut reps, first) = res!(seed(ALPHA, 2));
	let seed_id = first.0.id;
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 treats "0123456789" as the block.
	let m1 = res!(r1[0].move_range(0, 10, 20));
	// Replica 2 treats "012345" as the block.
	let m2 = res!(r2[0].move_range(0, 6, 20));
	let out = res!(case("ABCDEFGHIJ6789012345", &[first, m1.clone(), m2]));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		m1.0.id,
		lost:	vec![res!(ContentRange::new(seed_id, 0, 6))],
	}), "flags were {:?}", out.flags());
	Ok(())
}

/// Two authors each write a section and then go back to put a heading above it.
/// Every surveyed algorithm but one interleaves the four runs here; this
/// structure keeps each heading with its own section, at run granularity rather
/// than the per-element granularity the published proof is stated over.
#[test]
fn a_heading_added_after_the_fact_stays_with_its_section() -> Outcome<()> {
	let (mut reps, first) = res!(seed(b"\n", 2));
	let (r1, r2) = reps.split_at_mut(1);
	let s1 = res!(r1[0].insert(1, b"section A\n"));
	let h1 = res!(r1[0].insert(1, b"HEADING A\n"));
	let s2 = res!(r2[0].insert(1, b"section B\n"));
	let h2 = res!(r2[0].insert(1, b"HEADING B\n"));
	res!(case(
		"\nHEADING A\nsection A\nHEADING B\nsection B\n",
		&[first, s1, h1, s2, h2],
	));
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROPERTIES                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Every delivery order of a set mixing all three kinds of edit renders the same
/// bytes and raises the same flags.
#[test]
fn every_delivery_order_of_a_mixed_set_agrees() -> Outcome<()> {
	let (mut reps, first) = res!(seed(b"alpha beta gamma", 3));
	let (r1, rest) = reps.split_at_mut(1);
	let (r2, r3) = rest.split_at_mut(1);
	let mv = res!(r1[0].move_range(0, 6, 16));		// "alpha " to the end.
	let ins = res!(r2[0].insert(11, b"very "));		// Before "gamma".
	let del = res!(r3[0].delete(6, 4));				// "beta".
	let out = res!(converge(&[first, mv, ins, del]));
	assert_eq!(out.stats().ops, 4);
	assert_eq!(out.len(), 16 + 5 - 4 - 6 + 6);
	Ok(())
}

/// Three replicas editing and moving at random, exchanging operations at
/// random, agree on the bytes and on the flags however the operations are
/// shuffled, and conserve every byte they wrote.
///
/// Convergence is nearly free here, since the state is the operation set; what
/// this earns is the breadth. It walks the renderer over operation sets nobody
/// wrote by hand, including the ones that tear, cycle and anchor into
/// themselves.
#[test]
fn random_operation_sets_render_alike_and_conserve() -> Outcome<()> {
	// A small linear congruential generator, so a failure can be reproduced.
	let mut seed_state = 0x2545_F491_4F6C_DD1Du64;
	let mut next = move || {
		seed_state = seed_state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(seed_state >> 33) as usize
	};
	for trial in 0..60 {
		let (mut reps, first) = res!(seed(b"0123456789abcdefghij", 3));
		let mut ops = vec![first];
		// How much of the operation list each replica has received. Operations
		// are delivered as a prefix, because an operation is anchored in what
		// its author could see, so a prefix is always causally complete.
		let mut upto = vec![1usize; reps.len()];
		for _ in 0..16 {
			let who = next() % reps.len();
			let target = upto[who] + next() % (ops.len() - upto[who] + 1);
			while upto[who] < target {
				res!(reps[who].recv(ops[upto[who]].clone()));
				upto[who] += 1;
			}
			let view = res!(reps[who].view());
			let n = view.len();
			if n == 0 {
				continue;
			}
			let at = next() % (n + 1);
			let op = match next() % 3 {
				0 => res!(view.splice(at, 0, b"[]".to_vec())),
				1 => {
					let len = (1 + next() % 4).min(n - at);
					if len == 0 {
						continue;
					}
					res!(view.splice(at, len, Vec::new()))
				},
				_ => {
					let len = (1 + next() % 5).min(n - at);
					if len == 0 {
						continue;
					}
					res!(view.move_range(at, len, next() % (n + 1)))
				},
			};
			ops.push(res!(reps[who].author(op)));
		}
		// Every replica ends up holding everything, in a different order each
		// time, and must agree.
		let mut want: Option<Rendered> = None;
		for round in 0..4 {
			let mut seq = Sequence::new();
			let mut order: Vec<usize> = (0..ops.len()).collect();
			for i in (1..order.len()).rev() {
				order.swap(i, next() % (i + 1));
			}
			for i in order {
				res!(seq.apply(ops[i].0.clone(), ops[i].1.clone()));
			}
			let got = res!(seq.render());
			res!(seq.check_conservation(&got));
			match &want {
				None => want = Some(got),
				Some(first) => {
					assert_eq!(first.bytes(), got.bytes(),
						"trial {} round {} disagreed on the bytes", trial, round);
					assert_eq!(first.flags(), got.flags(),
						"trial {} round {} disagreed on the flags", trial, round);
				},
			}
		}
	}
	Ok(())
}

/// Conservation holds where it is hardest: a torn move, a cycle and a deletion
/// in one operation set. Every byte created is either rendered exactly once or
/// dead.
#[test]
fn conservation_holds_through_a_tear_and_a_cycle() -> Outcome<()> {
	let (mut reps, first) = res!(seed(ALPHA, 3));
	let (r1, rest) = reps.split_at_mut(1);
	let (r2, r3) = rest.split_at_mut(1);
	let m1 = res!(r1[0].move_range(0, 10, 20));
	let m2 = res!(r2[0].move_range(5, 10, 0));
	let m3 = res!(r3[0].move_range(10, 5, 2));
	let del = res!(r3[0].delete(18, 2));
	let mut seq = Sequence::new();
	for op in [first, m1, m2, m3, del] {
		res!(seq.apply(op.0, op.1));
	}
	let out = res!(seq.render());
	res!(seq.check_conservation(&out));
	assert_eq!(
		out.len() as u64 + 2,
		out.stats().atom_bytes,
		"every byte is rendered once or dead",
	);
	Ok(())
}

/// A conservation failure is reported rather than rendered. The check is fed a
/// render short of a byte, which is what a slot detached from the tree would
/// produce, and it says so.
#[test]
fn conservation_notices_a_missing_byte() -> Outcome<()> {
	let (_, first) = res!(seed(b"abcdef", 0));
	let seed_id = first.0.id;
	let mut seq = Sequence::new();
	res!(seq.apply(first.0, first.1));
	let out = res!(seq.render());
	let short = Rendered::new(
		out.bytes()[..5].to_vec(),
		vec![Run {
			at:			0,
			content:	res!(ContentRange::new(seed_id, 0, 5)),
		}],
		Vec::new(),
		*out.stats(),
	);
	assert!(seq.check_conservation(&short).is_err(),
		"five of six bytes accounted for is not conservation");
	Ok(())
}

/// An operation set missing what its operations were written against cannot be
/// resolved, and says which operation named what rather than guessing.
#[test]
fn a_causally_incomplete_set_is_refused() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 1));
	let ins = res!(reps[0].insert(9, b"!"));
	let mv = res!(reps[0].move_range(7, 7, 0));
	// The insertion names the seeding splice as its parent, and its anchor names
	// the content that splice created.
	assert_eq!(ins.0.parents, vec![first.0.id]);
	let mut without_seed = Sequence::new();
	res!(without_seed.apply(ins.0.clone(), ins.1.clone()));
	assert!(without_seed.render().is_err(),
		"an operation without its parent cannot be resolved");
	// The move's source names the same absent content.
	let mut moved = Sequence::new();
	res!(moved.apply(mv.0, mv.1));
	assert!(moved.render().is_err(),
		"a move naming an absent atom cannot be resolved");
	// With the seeding splice present, both render.
	let mut whole = Sequence::new();
	res!(whole.apply(first.0, first.1));
	res!(whole.apply(ins.0, ins.1));
	assert!(whole.render().is_ok());
	Ok(())
}

/// The causal precondition is read off the parents, so an operation delivered
/// ahead of one it was written against is refused even where every byte it names
/// is present.
#[test]
fn an_operation_ahead_of_its_parent_is_refused() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 1));
	// Two operations from one replica: the second was written knowing the first.
	let one = res!(reps[0].insert(0, b"# "));
	let two = res!(reps[0].insert(0, b"! "));
	assert_eq!(two.0.parents, vec![one.0.id]);
	let mut without_middle = Sequence::new();
	res!(without_middle.apply(first.0.clone(), first.1.clone()));
	res!(without_middle.apply(two.0.clone(), two.1.clone()));
	assert!(without_middle.render().is_err(),
		"the operation names a parent the set does not hold");
	// The same set with the middle operation restored renders.
	let mut whole = Sequence::new();
	for op in [first, one, two] {
		res!(whole.apply(op.0, op.1));
	}
	assert!(whole.render().is_ok());
	Ok(())
}

/// An anchor reaching past the end of an atom it does name is refused for the
/// same reason: the set does not hold the byte.
#[test]
fn an_anchor_past_the_end_of_its_atom_is_refused() -> Outcome<()> {
	let (_, first) = res!(seed(b"abc", 0));
	let seed_id = first.0.id;
	let mut seq = Sequence::new();
	res!(seq.apply(first.0, first.1));
	let stray = Edit::Splice {
		left:	Some(Anchor::after(ContentId::new(seed_id, 99))),
		right:	None,
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	};
	let head = res!(Header::new(OpId::new(ReplicaId::new(1), 2), vec![seed_id]));
	res!(seq.apply(head, stray));
	assert!(seq.render().is_err());
	Ok(())
}

/// Both content operations, recorded in the durable vocabulary and put through
/// the wire codec, come back as the same operations and render the same file.
#[test]
fn the_wire_vocabulary_renders_the_same_file() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 1));
	let mv = res!(reps[0].move_range(7, 7, 0));
	let ed = res!(reps[0].replace(2, 4, b"Soy"));
	// Every operation of the sequence is written down as a record, encoded, and
	// read back through the log's own vocabulary.
	let mut replayed = Sequence::new();
	for (head, edit) in [first, mv, ed] {
		let op = match &edit {
			Edit::Splice { left, right, remove, insert } => Op::Splice {
				file:	fmt!("shopping.txt"),
				left:	*left,
				right:	*right,
				remove:	remove.clone(),
				insert:	insert.clone(),
			},
			Edit::Move { src, left, right } => Op::Move {
				file:	fmt!("shopping.txt"),
				src:	src.clone(),
				left:	*left,
				right:	*right,
			},
		};
		let rec = Record::new(head, op);
		let back = res!(Record::decode_all(&res!(rec.encode())));
		assert_eq!(rec, back);
		assert_eq!(res!(Record::from_dat(&rec.to_dat())), rec);
		// The recovered operation is the edit it started as, field for field.
		match Edit::from_op(&back.op) {
			Some(recovered)	=> assert_eq!(recovered, edit),
			None => return Err(err!(
				"An Op::{} should have crossed into the sequence.", back.op.name();
			Test, Mismatch)),
		}
		assert!(res!(replayed.apply_record(&back)), "a content operation crosses");
	}
	assert_eq!(res!(replayed.render()).text_lossy(), "- Soy\n- Eggs\n- Cheese\n");
	Ok(())
}

/// An operation that says nothing about the order of bytes does not cross into
/// the sequence, and the sequence says so rather than failing.
#[test]
fn a_lifecycle_operation_does_not_cross_into_the_sequence() -> Outcome<()> {
	let mut seq = Sequence::new();
	for op in [
		Op::FileCreate { path: fmt!("f") },
		Op::FileDelete { path: fmt!("f") },
		Op::FileRename { from: fmt!("f"), to: fmt!("g") },
		Op::Mark { name: fmt!("v1") },
	] {
		assert!(Edit::from_op(&op).is_none(), "an Op::{} carries no order", op.name());
		let rec = Record::root(OpId::new(ReplicaId::new(1), 1), op);
		assert!(!res!(seq.apply_record(&rec)));
	}
	assert!(seq.is_empty(), "nothing of any of them reached the structure");
	Ok(())
}

/// Applying an operation twice does nothing the second time; applying two
/// different operations under one identity is refused.
#[test]
fn an_identity_names_one_operation() -> Outcome<()> {
	let (_, first) = res!(seed(b"abc", 0));
	let mut seq = Sequence::new();
	res!(seq.apply(first.0.clone(), first.1.clone()));
	res!(seq.apply(first.0.clone(), first.1.clone()));
	assert_eq!(seq.len(), 1);
	let other = Edit::Splice {
		left:	None,
		right:	None,
		remove:	Vec::new(),
		insert:	b"different".to_vec(),
	};
	assert!(seq.apply(first.0.clone(), other).is_err());
	// Two headers differing only in their parents are two operations too.
	let reparented = res!(Header::new(first.0.id, vec![OpId::new(ReplicaId::new(9), 1)]));
	assert!(seq.apply(reparented, first.1).is_err());
	Ok(())
}

/// Origins bind on one side each, and a move may not name a byte twice.
#[test]
fn an_operation_the_structure_cannot_resolve_is_refused() -> Outcome<()> {
	let id = OpId::new(ReplicaId::new(1), 1);
	let cid = ContentId::new(id, 0);
	let mut seq = Sequence::new();
	let wrong_left = Edit::Splice {
		left:	Some(Anchor::before(cid)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	};
	assert!(seq.apply(Header::root(OpId::new(ReplicaId::new(2), 2)), wrong_left).is_err());
	let wrong_right = Edit::Splice {
		left:	None,
		right:	Some(Anchor::after(cid)),
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	};
	assert!(seq.apply(Header::root(OpId::new(ReplicaId::new(2), 2)), wrong_right).is_err());
	let twice = Edit::Move {
		src:	vec![
			res!(ContentRange::new(id, 0, 4)),
			res!(ContentRange::new(id, 2, 6)),
		],
		left:	None,
		right:	None,
	};
	assert!(seq.apply(Header::root(OpId::new(ReplicaId::new(2), 2)), twice).is_err());
	Ok(())
}

/// A move whose destination sits inside its own source is a cycle of length one.
/// Left unseen it detaches the move's slots from the tree and loses their bytes;
/// the demotion rule sees it, and every byte survives.
#[test]
fn a_move_into_its_own_source_keeps_its_bytes() -> Outcome<()> {
	let (mut reps, first) = res!(seed(ALPHA, 1));
	let view = res!(reps[0].view());
	// Take "0123456789" and land it in the middle of itself.
	let src = res!(view.span(0, 10));
	let (left, right) = res!(view.gap(5));
	let op = res!(reps[0].author(Edit::Move { src, left, right }));
	let op_id = op.0.id;
	let mut seq = Sequence::new();
	res!(seq.apply(first.0, first.1));
	res!(seq.apply(op.0, op.1));
	let out = res!(seq.render());
	res!(seq.check_conservation(&out));
	assert_eq!(out.len(), 20, "the moved bytes must not vanish");
	assert_eq!(out.flags(), &[
		Flag::Demoted { op: op_id, sub: 0, origin: Origin::Left },
		Flag::Demoted { op: op_id, sub: 0, origin: Origin::Right },
	]);
	Ok(())
}

/// The render reports what it cost in the terms the cost model is stated in.
#[test]
fn the_render_reports_what_it_cost() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 2));
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_range(7, 7, 0));
	let ed = res!(r2[0].replace(9, 1, b"Soy m"));
	let mut seq = Sequence::new();
	for op in [first, mv, ed] {
		res!(seq.apply(op.0, op.1));
	}
	let out = res!(seq.render());
	let stats = out.stats();
	assert_eq!(stats.ops, 3);
	assert_eq!(stats.atoms, 2, "the two splices, not the move");
	assert_eq!(stats.atom_bytes, LIST.len() as u64 + 5);
	assert!(stats.slots_divided >= stats.slots_placed,
		"dividing a slot never yields fewer");
	assert_eq!(stats.claim_intervals, 1, "one contiguous run moved");
	assert_eq!(stats.dead_intervals, 1, "one byte died");
	assert_eq!(stats.rendered, out.len() as u64);
	Ok(())
}

/// Provenance survives a move: the rendered runs still name the content that
/// made them, so an index in the render can be turned back into a name.
#[test]
fn provenance_follows_the_bytes() -> Outcome<()> {
	let (mut reps, first) = res!(seed(LIST, 1));
	let seed_id = first.0.id;
	let mv = res!(reps[0].move_range(7, 7, 0));
	let mut seq = Sequence::new();
	res!(seq.apply(first.0, first.1));
	res!(seq.apply(mv.0, mv.1));
	let out = res!(seq.render());
	assert_eq!(out.text_lossy(), "- Milk\n- Eggs\n- Cheese\n");
	// The first rendered byte is now the eighth byte the seeding splice made.
	assert_eq!(res!(out.content_at(0)), ContentId::new(seed_id, 7));
	assert_eq!(res!(out.content_at(7)), ContentId::new(seed_id, 0));
	assert_eq!(res!(out.span(0, 7)), vec![res!(ContentRange::new(seed_id, 7, 14))]);
	assert!(out.content_at(out.len()).is_err());
	Ok(())
}
