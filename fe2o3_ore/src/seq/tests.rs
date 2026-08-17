//! The adversarial cases, each with the answer the literature or the design
//! prescribes, and each checked under every delivery order.
//!
//! A case that merely converges proves little, because the state here is the
//! operation set and a render is a function of it, so agreement between
//! delivery orders is nearly free. What the cases test is that the answer
//! converged on is the right one: the one the published counter-examples say a
//! correct structure must give.
//!
//! Every case here is single-file, and every expectation is what it was before
//! file identity: the repository now holds a file rather than a bare sequence,
//! and a splice into that file anchors after its origin anchor rather than after
//! nothing, and none of the ten answers moves. The multi-file cases are beside
//! this file in `file_tests.rs`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	Anchor,
	ContentId,
	ContentRange,
	OpId,
	ReplicaId,
};
use crate::op::{
	Header,
	Mode,
	Op,
	Record,
};
use crate::seq::render::{
	Flag,
	Rendered,
	Repo,
	Run,
	Span,
	Stats,
};
use crate::seq::slot::Origin;
use crate::seq::Sequence;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


// Plain hyphens, so that byte offsets and character offsets coincide.
const LIST: &[u8] = b"- Eggs\n- Milk\n- Cheese\n";

// Twenty bytes whose order is easy to read off a rendered string.
const ALPHA: &[u8] = b"0123456789ABCDEFGHIJ";


/// One replica of a repository holding one file: the frontend that turns
/// index-based editing intent into content-anchored operations, which is what a
/// real editor would be.
struct Replica {
	id:		u64,			// every operation of this replica is named by it
	seq:	Sequence,
	file:	OpId,			// the file being edited
}

impl Replica {

	fn new(id: u64, file: OpId) -> Self {
		Self { id, seq: Sequence::new(), file }
	}

	/// A Lamport counter, and everything this replica can see as the operation's
	/// parents.
	fn next_head(&self)
		-> Outcome<Header>
	{
		let seen = self.seq.iter().map(|(id, _)| id.counter).max().unwrap_or(0);
		Header::new(
			OpId::new(ReplicaId::new(self.id), seen + 1),
			self.seq.causality().heads(),
		)
	}

	fn recv(&mut self, op: (Header, Op))
		-> Outcome<()>
	{
		self.seq.apply(op.0, op.1)
	}

	fn view(&self)
		-> Outcome<Rendered>
	{
		let repo = res!(self.seq.render());
		match repo.file(self.file) {
			Some(f)	=> Ok(f.clone()),
			None	=> Err(err!(
				"The replica has no file {}.", self.file; Test, Missing)),
		}
	}

	fn author(&mut self, op: Op)
		-> Outcome<(Header, Op)>
	{
		let head = res!(self.next_head());
		res!(self.seq.apply(head.clone(), op.clone()));
		Ok((head, op))
	}

	fn insert(&mut self, at: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view()).splice(at, 0, bytes.to_vec()));
		self.author(op)
	}

	fn delete(&mut self, at: usize, len: usize)
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view()).splice(at, len, Vec::new()));
		self.author(op)
	}

	/// One operation, not a deletion and an insertion.
	fn replace(&mut self, at: usize, len: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view()).splice(at, len, bytes.to_vec()));
		self.author(op)
	}

	fn move_range(&mut self, at: usize, len: usize, to: usize)
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view()).move_range(at, len, to));
		self.author(op)
	}

	fn note(&mut self, at: usize, len: usize, text: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view()).note_on(at, len, text.to_vec()));
		self.author(op)
	}
}


/// A repository staged with one file carrying some initial text, and the
/// replicas that have seen it.
struct Stage {
	reps:	Vec<Replica>,			// each holding everything staged
	ops:	Vec<(Header, Op)>,		// the file's creation, then the seeding splice
	file:	OpId,
	seed:	OpId,					// the splice that wrote the initial text
}

/// Creates one file, writes `text` into it, and hands out `replicas` replicas
/// that have seen both operations.
fn seed(text: &[u8], replicas: u64)
	-> Outcome<Stage>
{
	let mut origin = Replica::new(0, OpId::default());
	let create = res!(origin.author(Op::FileCreate { path: b"f".to_vec() }));
	let file = create.0.id();
	origin.file = file;
	let mut ops = vec![create];
	let seed = res!(origin.insert(0, text));
	let seed_id = seed.0.id();
	ops.push(seed);
	let mut out = Vec::new();
	for i in 1..=replicas {
		let mut r = Replica::new(i, file);
		for op in &ops {
			res!(r.recv(op.clone()));
		}
		out.push(r);
	}
	Ok(Stage { reps: out, ops, file, seed: seed_id })
}

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
/// render the same bytes in every file and raise the same flags, and returns
/// that render.
fn converge(ops: &[(Header, Op)])
	-> Outcome<Repo>
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
	let mut first: Option<Repo> = None;
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
				if listing(want) != listing(&got) {
					return Err(err!(
						"Delivery order changed the render: {} against {}.",
						listing(want), listing(&got);
					Test, Mismatch));
				}
				if want.flags() != got.flags() {
					return Err(err!(
						"Delivery order changed the flags: {:?} against {:?}.",
						want.flags(), got.flags();
					Test, Mismatch));
				}
				if want.notes() != got.notes() {
					return Err(err!(
						"Delivery order changed the notes: {:?} against {:?}.",
						want.notes(), got.notes();
					Test, Mismatch));
				}
				for (a, b) in want.files().iter().zip(got.files()) {
					if a.notes() != b.notes() {
						return Err(err!(
							"Delivery order changed the notes of the file {}: {:?} \
							against {:?}.", a.file(), a.notes(), b.notes();
						Test, Mismatch));
					}
				}
			},
		}
	}
	match first {
		Some(r)	=> Ok(r),
		None	=> Err(err!("No delivery order was tried."; Test, Bug)),
	}
}

/// A one-line rendering of every file, for test messages and comparison.
fn listing(repo: &Repo) -> String {
	let mut s = String::new();
	for f in repo.files() {
		s.push_str(&fmt!(
			"{}{}={:?} ",
			f.path_lossy(),
			if f.is_live() { "" } else { " (deleted)" },
			f.text_lossy(),
		));
	}
	s.trim_end().to_string()
}

/// Runs an operation set under every delivery order and checks one file's render
/// against the answer the case prescribes.
fn case(file: OpId, expect: &str, ops: &[(Header, Op)])
	-> Outcome<Rendered>
{
	let repo = res!(converge(ops));
	let got = match repo.file(file) {
		Some(f)	=> f.clone(),
		None	=> return Err(err!(
			"The render holds no file {}.", file; Test, Missing)),
	};
	assert_eq!(got.text_lossy(), expect);
	Ok(got)
}

fn count(repo: &Rendered, kind: fn(&Flag) -> bool) -> usize {
	repo.flags().iter().filter(|f| kind(f)).count()
}

fn is_torn(flag: &Flag) -> bool {
	matches!(flag, Flag::Torn { .. })
}

fn is_demoted(flag: &Flag) -> bool {
	matches!(flag, Flag::Demoted { .. })
}

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
	let mut st = res!(seed(LIST, 2));
	let (b, a) = st.reps.split_at_mut(1);
	// Replica 1 moves "- Milk\n" to the top.
	st.ops.push(res!(b[0].move_range(7, 7, 0)));
	// Replica 2 concurrently turns "Milk" into "Soy milk".
	st.ops.push(res!(a[0].replace(9, 1, b"Soy m")));
	let out = res!(case(st.file, "- Soy milk\n- Eggs\n- Cheese\n", &st.ops));
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
	let mut st = res!(seed(LIST, 2));
	let seed_id = st.seed;
	let (r1, r2) = st.reps.split_at_mut(1);
	let lost = res!(r1[0].move_range(7, 7, 0));		// replica 1 loses
	let won = res!(r2[0].move_range(7, 7, 23));		// replica 2 wins
	st.ops.push(lost.clone());
	st.ops.push(won);
	let out = res!(case(st.file, "- Eggs\n- Cheese\n- Milk\n", &st.ops));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		lost.0.id(),
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
	let mut st = res!(seed(ALPHA, 2));
	let seed_id = st.seed;
	let (r1, r2) = st.reps.split_at_mut(1);
	let torn = res!(r1[0].move_range(0, 10, 20));	// replica 1 loses the overlap
	let won = res!(r2[0].move_range(5, 10, 0));		// replica 2 wins it
	st.ops.push(torn.clone());
	st.ops.push(won);
	let out = res!(case(st.file, "FGHIJ56789ABCDE01234", &st.ops));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		torn.0.id(),
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
	let mut st = res!(seed(ALPHA, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	// Replica 1 moves "01234" into the middle of "ABCDE".
	let m1 = res!(r1[0].move_range(0, 5, 12));
	// Replica 2 moves "ABCDE" into the middle of "01234".
	let m2 = res!(r2[0].move_range(10, 5, 2));
	st.ops.push(m1.clone());
	st.ops.push(m2);
	let out = res!(case(st.file, "5678901ABCDE234FGHIJ", &st.ops));
	assert_eq!(out.len(), 20, "no byte may be lost to a cycle");
	assert_eq!(count(&out, is_dropped), 0, "demotion sufficed");
	// Both origins of the lower move in op order give way, and the higher move
	// keeps the destination it asked for. The cycle is inside one file, so
	// nothing crossed a boundary and no cross-file flag is raised.
	assert_eq!(out.flags(), &[
		Flag::Demoted { op: m1.0.id(), sub: 0, origin: Origin::Left },
		Flag::Demoted { op: m1.0.id(), sub: 0, origin: Origin::Right },
	]);
	Ok(())
}

#[test]
fn an_insertion_inside_a_moved_run_goes_with_it() -> Outcome<()> {
	let mut st = res!(seed(LIST, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(7, 7, 0)));
	st.ops.push(res!(r2[0].insert(11, b"!")));
	res!(case(st.file, "- Mi!lk\n- Eggs\n- Cheese\n", &st.ops));
	Ok(())
}

/// The runs stay whole and follow op order; interleaving them is the failure most
/// published algorithms exhibit.
#[test]
fn three_concurrent_runs_at_one_point_do_not_interleave() -> Outcome<()> {
	let mut st = res!(seed(b"AB", 3));
	for (i, r) in st.reps.iter_mut().enumerate() {
		let run = match i {
			0	=> b"xxx".to_vec(),
			1	=> b"yyy".to_vec(),
			_	=> b"zzz".to_vec(),
		};
		st.ops.push(res!(r.insert(1, &run)));
	}
	res!(case(st.file, "AxxxyyyzzzB", &st.ops));
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
	let mut st = res!(seed(LIST, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(7, 7, 0)));
	st.ops.push(res!(r2[0].insert(7, b"<")));
	st.ops.push(res!(r2[0].insert(15, b">")));
	res!(case(st.file, "- Milk\n>- Eggs\n<- Cheese\n", &st.ops));
	Ok(())
}

/// A move and a deletion inside the moved run need no tie-break between them.
/// The bytes move, and they are dead, and a dead byte renders as nothing
/// wherever it is.
#[test]
fn a_move_and_a_deletion_inside_it_compose() -> Outcome<()> {
	let mut st = res!(seed(LIST, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(7, 7, 0)));
	st.ops.push(res!(r2[0].delete(9, 4)));			// "Milk"
	res!(case(st.file, "- \n- Eggs\n- Cheese\n", &st.ops));
	Ok(())
}

/// Two replicas that disagree about where the block they are moving begins and
/// ends. The claim register gives each byte to the higher mover, so the lower
/// move keeps only what the higher one did not want, and its fragments render
/// apart from each other. Deterministic, flagged, and not what its author meant.
#[test]
fn moves_with_different_boundaries_split_between_them() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 2));
	let seed_id = st.seed;
	let (r1, r2) = st.reps.split_at_mut(1);
	// Replica 1 treats "0123456789" as the block.
	let m1 = res!(r1[0].move_range(0, 10, 20));
	// Replica 2 treats "012345" as the block.
	let m2 = res!(r2[0].move_range(0, 6, 20));
	st.ops.push(m1.clone());
	st.ops.push(m2);
	let out = res!(case(st.file, "ABCDEFGHIJ6789012345", &st.ops));
	assert_eq!(count(&out, is_torn), 1);
	assert!(out.flags().contains(&Flag::Torn {
		op:		m1.0.id(),
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
	let mut st = res!(seed(b"\n", 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	st.ops.push(res!(r1[0].insert(1, b"section A\n")));
	st.ops.push(res!(r1[0].insert(1, b"HEADING A\n")));
	st.ops.push(res!(r2[0].insert(1, b"section B\n")));
	st.ops.push(res!(r2[0].insert(1, b"HEADING B\n")));
	res!(case(
		st.file,
		"\nHEADING A\nsection A\nHEADING B\nsection B\n",
		&st.ops,
	));
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE TORN FLAG AND CAUSALITY                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// A move superseded by a later move of the same content, by the same author,
/// is a sequence of two decisions and not a race, and raises nothing.
///
/// The claim register cannot tell the two apart on its own: in both cases it
/// names somebody other than the earlier move. The parents can, and the flag
/// consults them. Before it did, every deliberate re-move reported a tear, which
/// is a flag nobody can act on sitting on top of the ones they can.
#[test]
fn a_move_superseded_on_purpose_does_not_tear() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	let first = res!(st.reps[0].move_range(7, 7, 0));
	// The same author, having seen the first move, moves the same line again.
	let second = res!(st.reps[0].move_range(0, 7, 23));
	assert!(second.0.parents().contains(&first.0.id()),
		"the second move was written knowing the first");
	st.ops.push(first.clone());
	st.ops.push(second);
	let out = res!(case(st.file, "- Eggs\n- Cheese\n- Milk\n", &st.ops));
	assert_eq!(count(&out, is_torn), 0, "flags were {:?}", out.flags());
	assert_eq!(count(&out, is_overlap), 0,
		"nor were the two moves in conflict, one having seen the other");
	Ok(())
}

/// The fix narrows the flag rather than removing it.
#[test]
fn genuinely_concurrent_moves_still_tear() -> Outcome<()> {
	let mut st = res!(seed(LIST, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	let lower = res!(r1[0].move_range(7, 7, 0));
	let higher = res!(r2[0].move_range(7, 7, 23));
	assert!(!lower.0.parents().contains(&higher.0.id()));
	assert!(!higher.0.parents().contains(&lower.0.id()));
	st.ops.push(lower.clone());
	st.ops.push(higher);
	let out = res!(case(st.file, "- Eggs\n- Cheese\n- Milk\n", &st.ops));
	assert_eq!(count(&out, is_torn), 1, "flags were {:?}", out.flags());
	match out.flags().iter().find(|f| is_torn(f)) {
		Some(Flag::Torn { op, .. })	=> assert_eq!(*op, lower.0.id()),
		_ => return Err(err!("The torn flag went missing."; Test, Missing)),
	}
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROPERTIES                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn every_delivery_order_of_a_mixed_set_agrees() -> Outcome<()> {
	let mut st = res!(seed(b"alpha beta gamma", 3));
	let (r1, rest) = st.reps.split_at_mut(1);
	let (r2, r3) = rest.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(0, 6, 16)));	// "alpha " to the end
	st.ops.push(res!(r2[0].insert(11, b"very ")));	// before "gamma"
	st.ops.push(res!(r3[0].delete(6, 4)));			// "beta"
	let out = res!(converge(&st.ops));
	assert_eq!(out.stats().ops, 5);
	let file = match out.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.len(), 16 + 5 - 4 - 6 + 6);
	Ok(())
}

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
		let mut st = res!(seed(b"0123456789abcdefghij", 3));
		let staged = st.ops.len();
		// How much of the operation list each replica has received. Operations
		// are delivered as a prefix, because an operation is anchored in what
		// its author could see, so a prefix is always causally complete.
		let mut upto = vec![staged; st.reps.len()];
		for _ in 0..16 {
			let who = next() % st.reps.len();
			let target = upto[who] + next() % (st.ops.len() - upto[who] + 1);
			while upto[who] < target {
				let op = st.ops[upto[who]].clone();
				res!(st.reps[who].recv(op));
				upto[who] += 1;
			}
			let view = res!(st.reps[who].view());
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
			let made = res!(st.reps[who].author(op));
			st.ops.push(made);
		}
		// Every replica ends up holding everything, in a different order each
		// time, and must agree.
		let mut want: Option<Repo> = None;
		for round in 0..4 {
			let mut seq = Sequence::new();
			let mut order: Vec<usize> = (0..st.ops.len()).collect();
			for i in (1..order.len()).rev() {
				order.swap(i, next() % (i + 1));
			}
			for i in order {
				res!(seq.apply(st.ops[i].0.clone(), st.ops[i].1.clone()));
			}
			let got = res!(seq.render());
			res!(seq.check_conservation(&got));
			match &want {
				None => want = Some(got),
				Some(first) => {
					assert_eq!(listing(first), listing(&got),
						"trial {} round {} disagreed on the bytes", trial, round);
					assert_eq!(first.flags(), got.flags(),
						"trial {} round {} disagreed on the flags", trial, round);
				},
			}
		}
	}
	Ok(())
}

/// Every byte created is either rendered exactly once or dead, where that is
/// hardest to hold: a torn move, a cycle and a deletion in one operation set.
#[test]
fn conservation_holds_through_a_tear_and_a_cycle() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 3));
	let (r1, rest) = st.reps.split_at_mut(1);
	let (r2, r3) = rest.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(0, 10, 20)));
	st.ops.push(res!(r2[0].move_range(5, 10, 0)));
	st.ops.push(res!(r3[0].move_range(10, 5, 2)));
	st.ops.push(res!(r3[0].delete(18, 2)));
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let out = res!(seq.render());
	res!(seq.check_conservation(&out));
	// Two bytes died, and one more is the file's origin anchor, which is born
	// dead and never rendered.
	assert_eq!(
		out.stats().rendered + 2 + 1,
		out.stats().atom_bytes,
		"every byte is rendered once or dead",
	);
	Ok(())
}

/// A conservation failure is reported rather than rendered. The check is fed a
/// render short of a byte, which is what a slot detached from the forest would
/// produce, and it says so.
#[test]
fn conservation_notices_a_missing_byte() -> Outcome<()> {
	let st = res!(seed(b"abcdef", 0));
	let seed_id = st.seed;
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let out = res!(seq.render());
	let file = match out.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	let short = Repo::new(
		vec![Rendered::new(
			st.file,
			b"f".to_vec(),
			Mode::Normal,
			true,
			file.bytes()[..5].to_vec(),
			vec![Run {
				at:			0,
				content:	res!(ContentRange::new(seed_id, 0, 5)),
			}],
			Vec::new(),
			Vec::new(),
		)],
		Vec::new(),
		Vec::new(),
		BTreeMap::new(),
		Stats::default(),
	);
	assert!(seq.check_conservation(&short).is_err(),
		"five of six bytes accounted for is not conservation");
	Ok(())
}

/// The refusal says which operation named what rather than guessing.
#[test]
fn a_causally_incomplete_set_is_refused() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	let ins = res!(st.reps[0].insert(9, b"!"));
	let mv = res!(st.reps[0].move_range(7, 7, 0));
	// The insertion names the seeding splice as its parent, and its anchor names
	// the content that splice created.
	assert_eq!(ins.0.parents(), &[st.seed]);
	let mut without_seed = Sequence::new();
	res!(without_seed.apply(ins.0.clone(), ins.1.clone()));
	assert!(without_seed.render().is_err(),
		"an operation without its parent cannot be resolved");
	// The move's source names the same absent content.
	let mut moved = Sequence::new();
	res!(moved.apply(mv.0, mv.1));
	assert!(moved.render().is_err(),
		"a move naming an absent atom cannot be resolved");
	// With the whole staging present, both render.
	let mut whole = Sequence::new();
	for op in &st.ops {
		res!(whole.apply(op.0.clone(), op.1.clone()));
	}
	res!(whole.apply(ins.0, ins.1));
	assert!(whole.render().is_ok());
	Ok(())
}

/// A file's origin anchor is content the set has to hold like any other.
#[test]
fn an_operation_anchored_in_an_absent_file_is_refused() -> Outcome<()> {
	let ghost = OpId::new(ReplicaId::new(9), 1);
	let mut seq = Sequence::new();
	res!(seq.apply(Header::root(OpId::new(ReplicaId::new(1), 1)), Op::Splice {
		left:	Some(Anchor::origin(ghost)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"orphan".to_vec(),
	}));
	assert!(seq.render().is_err(),
		"the origin anchor names a file no operation created");
	// So is a rename or a deletion of a file nobody created.
	for op in [
		Op::FileRename { file: ghost, path: b"g".to_vec() },
		Op::FileDelete { file: ghost },
	] {
		let mut seq = Sequence::new();
		res!(seq.apply(Header::root(OpId::new(ReplicaId::new(1), 1)), op));
		assert!(seq.render().is_err());
	}
	Ok(())
}

/// The causal precondition is read off the parents, so it is refused even where
/// every byte it names is present.
#[test]
fn an_operation_ahead_of_its_parent_is_refused() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	// Two operations from one replica: the second was written knowing the first.
	let one = res!(st.reps[0].insert(0, b"# "));
	let two = res!(st.reps[0].insert(0, b"! "));
	assert_eq!(two.0.parents(), &[one.0.id()]);
	let mut without_middle = Sequence::new();
	for op in &st.ops {
		res!(without_middle.apply(op.0.clone(), op.1.clone()));
	}
	res!(without_middle.apply(two.0.clone(), two.1.clone()));
	assert!(without_middle.render().is_err(),
		"the operation names a parent the set does not hold");
	// The same set with the middle operation restored renders.
	let mut whole = Sequence::new();
	for op in st.ops.iter().cloned().chain([one, two]) {
		res!(whole.apply(op.0, op.1));
	}
	assert!(whole.render().is_ok());
	Ok(())
}

/// For the same reason as the last: the set does not hold the byte.
#[test]
fn an_anchor_past_the_end_of_its_atom_is_refused() -> Outcome<()> {
	let st = res!(seed(b"abc", 0));
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let stray = Op::Splice {
		left:	Some(Anchor::after(ContentId::new(st.seed, 99))),
		right:	None,
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	};
	let head = res!(Header::new(OpId::new(ReplicaId::new(1), 3), vec![st.seed]));
	res!(seq.apply(head, stray));
	assert!(seq.render().is_err());
	Ok(())
}

/// Every operation the sequence consumes, written down as a record and put
/// through the wire codec, comes back as the same operation and renders the same
/// repository.
#[test]
fn the_wire_vocabulary_renders_the_same_repository() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	st.ops.push(res!(st.reps[0].move_range(7, 7, 0)));
	st.ops.push(res!(st.reps[0].replace(2, 4, b"Soy")));
	let mut replayed = Sequence::new();
	for (head, op) in &st.ops {
		let rec = Record::new(head.clone(), op.clone());
		let back = res!(Record::decode_all(&res!(rec.encode())));
		assert_eq!(rec, back);
		assert_eq!(res!(Record::from_dat(&rec.to_dat())), rec);
		res!(replayed.apply_record(&back));
	}
	let repo = res!(replayed.render());
	match repo.file(st.file) {
		Some(f)	=> assert_eq!(f.text_lossy(), "- Soy\n- Eggs\n- Cheese\n"),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	Ok(())
}

/// What a frontend authors names content and never a file, and the file it lands
/// in is what the render works out.
#[test]
fn an_authored_operation_names_no_file() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	let (_, mv) = res!(st.reps[0].move_range(7, 7, 0));
	let (_, sp) = res!(st.reps[0].replace(2, 4, b"Soy"));
	for op in [&mv, &sp] {
		assert_eq!(op.names_file(), None);
		res!(op.check_placement());
	}
	// A splice at the start of a file is anchored after that file's origin
	// anchor, which is how it says where it lands without saying which file.
	let (_, ins) = res!(st.reps[0].insert(0, b"x"));
	assert_eq!(ins.origins().0, Some(Anchor::origin(st.file)));
	assert_eq!(ins.names_file(), None);
	Ok(())
}

/// Every operation the log holds belongs to the repository, including the
/// lifecycle changes, and a mark says nothing about any byte.
#[test]
fn every_operation_crosses_into_the_repository() -> Outcome<()> {
	let mut seq = Sequence::new();
	let file = OpId::new(ReplicaId::new(1), 1);
	let ops = vec![
		(Header::root(file), Op::FileCreate { path: b"f".to_vec() }),
		(
			res!(Header::new(OpId::new(ReplicaId::new(1), 2), vec![file])),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		),
		(
			res!(Header::new(OpId::new(ReplicaId::new(1), 3), vec![
				OpId::new(ReplicaId::new(1), 2),
			])),
			Op::FileRename { file, path: b"g".to_vec() },
		),
		(
			res!(Header::new(OpId::new(ReplicaId::new(1), 4), vec![
				OpId::new(ReplicaId::new(1), 3),
			])),
			Op::FileDelete { file },
		),
	];
	for (head, op) in &ops {
		res!(seq.apply_record(&Record::new(head.clone(), op.clone())));
	}
	assert_eq!(seq.len(), 4, "a mark is kept, so the causal graph has no holes");
	let repo = res!(seq.render());
	match repo.file(file) {
		Some(f) => {
			assert_eq!(f.path(), b"g", "the rename moved it");
			assert!(!f.is_live(), "the deletion retired it");
			assert!(f.is_empty());
		},
		None => return Err(err!("The file went missing."; Test, Missing)),
	}
	assert!(repo.live().is_empty());
	Ok(())
}

/// Two branches meet by absorbing one another's operations, and what the union
/// renders is what each branch renders once it has heard the other.
#[test]
fn two_divergent_branches_absorb_into_one_repository() -> Outcome<()> {
	let mut st = res!(seed(LIST, 2));
	// Each branch edits without having seen the other.
	let left = res!(st.reps[0].insert(0, b"- Bread\n"));
	let right = res!(st.reps[1].delete(7, 7));
	// A third party takes the union, and the staging is not taken twice.
	let mut both = Sequence::new();
	assert_eq!(res!(both.absorb(&st.reps[0].seq)), 3,
		"the file, the seeding splice and the branch's own edit");
	assert_eq!(res!(both.absorb(&st.reps[1].seq)), 1, "the staging is already held");
	assert_eq!(both.len(), 4);
	// Which is what each branch renders once it has received the other's edit.
	res!(st.reps[0].recv(right));
	res!(st.reps[1].recv(left));
	let merged = res!(both.render());
	let want = match merged.file(st.file) {
		Some(f)	=> f.text_lossy(),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(want, res!(st.reps[0].view()).text_lossy());
	assert_eq!(want, res!(st.reps[1].view()).text_lossy());
	assert_eq!(want, "- Bread\n- Eggs\n- Cheese\n");
	// Absorbing what is already held says so, and changes nothing.
	let before = both.clone();
	assert_eq!(res!(both.absorb(&st.reps[0].seq)), 0);
	assert_eq!(res!(both.absorb(&st.reps[1].seq)), 0);
	assert_eq!(both, before);
	Ok(())
}

/// Absorption is the union of two sets, so it does not matter which way round it
/// is taken, nor in how many steps.
#[test]
fn absorbing_either_way_round_gives_one_repository() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 2));
	res!(st.reps[0].insert(4, b"xy"));
	res!(st.reps[1].move_range(0, 3, 10));
	let mut left = st.reps[0].seq.clone();
	res!(left.absorb(&st.reps[1].seq));
	let mut right = st.reps[1].seq.clone();
	res!(right.absorb(&st.reps[0].seq));
	assert_eq!(left, right, "the union is the union");
	assert_eq!(listing(&res!(left.render())), listing(&res!(right.render())));
	Ok(())
}

/// One identity naming two different operations is not two branches of one
/// history, and the merge is refused whole rather than half taken.
#[test]
fn absorbing_a_clashing_identity_is_refused() -> Outcome<()> {
	let st = res!(seed(b"abc", 0));
	let seed_id = st.seed;
	let head = res!(Header::new(OpId::new(ReplicaId::new(1), 3), vec![seed_id]));
	let insert = |bytes: &[u8]| Op::Splice {
		left:	Some(Anchor::after(ContentId::new(seed_id, 0))),
		right:	None,
		remove:	Vec::new(),
		insert:	bytes.to_vec(),
	};
	let mut mine = Sequence::new();
	for op in &st.ops {
		res!(mine.apply(op.0.clone(), op.1.clone()));
	}
	res!(mine.apply(head.clone(), insert(b"one")));
	// The other repository holds the staging, something new, and a clash.
	let mut theirs = Sequence::new();
	for op in &st.ops {
		res!(theirs.apply(op.0.clone(), op.1.clone()));
	}
	res!(theirs.apply(head, insert(b"two")));
	let other = res!(Header::new(OpId::new(ReplicaId::new(2), 4), vec![seed_id]));
	res!(theirs.apply(other.clone(), insert(b"three")));
	assert!(mine.absorb(&theirs).is_err());
	assert_eq!(mine.len(), 3, "nothing at all was taken");
	assert!(!mine.contains(&other.id()));
	Ok(())
}

/// There is no routing step: every record goes to the same place, and which file
/// each operation landed in is what the render works out and reports, which is
/// the association a wire field would have asserted.
#[test]
fn a_repository_of_two_files_replays_from_the_log() -> Outcome<()> {
	use crate::log::OpLog;

	let mut log = OpLog::new();
	let r1 = ReplicaId::new(1);
	let r2 = ReplicaId::new(2);
	// A file each, written alternately, so that every operation's parents name
	// operations of the other file.
	let a = res!(log.author(r1, Op::FileCreate { path: b"a.txt".to_vec() }));
	let b = res!(log.author(r2, Op::FileCreate { path: b"b.txt".to_vec() }));
	let a_seed = res!(log.author(r1, Op::Splice {
		left:	Some(Anchor::origin(a.id())),
		right:	None,
		remove:	Vec::new(),
		insert:	b"alpha".to_vec(),
	}));
	let b_seed = res!(log.author(r2, Op::Splice {
		left:	Some(Anchor::origin(b.id())),
		right:	None,
		remove:	Vec::new(),
		insert:	b"beta".to_vec(),
	}));
	// Each operation is written against the whole frontier, which after the
	// second file was created is that creation alone.
	assert_eq!(b.parents(), &[a.id()]);
	assert_eq!(a_seed.parents(), &[b.id()]);
	assert_eq!(b_seed.parents(), &[a_seed.id()]);
	let tail = res!(log.author(r1, Op::Splice {
		left:	Some(Anchor::after(ContentId::new(a_seed.id(), 4))),
		right:	None,
		remove:	Vec::new(),
		insert:	b" and omega".to_vec(),
	}));
	// Replay: every record goes to the one repository.
	let mut seq = Sequence::new();
	for rec in log.iter() {
		res!(seq.apply_record(rec));
	}
	assert_eq!(seq.len(), 5);
	let repo = res!(seq.render());
	match repo.file(a.id()) {
		Some(f)	=> assert_eq!(f.text_lossy(), "alpha and omega"),
		None	=> return Err(err!("The file a.txt went missing."; Test, Missing)),
	}
	match repo.file(b.id()) {
		Some(f)	=> assert_eq!(f.text_lossy(), "beta"),
		None	=> return Err(err!("The file b.txt went missing."; Test, Missing)),
	}
	// The derived association: which file each placement landed in, computed by
	// the render rather than asserted on the wire.
	assert_eq!(repo.file_of(&a_seed.id()), Some(a.id()));
	assert_eq!(repo.file_of(&b_seed.id()), Some(b.id()));
	assert_eq!(repo.file_of(&tail.id()), Some(a.id()));
	assert_eq!(repo.index().len(), 5, "two files and three placements");
	// Rendering against the log's own graph gives the same answer.
	let cause = log.causality();
	assert_eq!(listing(&res!(seq.render_with(&cause))), listing(&repo));
	// A graph that does not describe an operation the sequence holds is refused
	// rather than guessed at.
	let empty = Sequence::new();
	assert!(seq.render_with(&empty.causality()).is_err());
	Ok(())
}

/// Applying an operation twice does nothing the second time; applying two
/// different operations under one identity is refused.
#[test]
fn an_identity_names_one_operation() -> Outcome<()> {
	let st = res!(seed(b"abc", 0));
	let first = st.ops[1].clone();
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	res!(seq.apply(first.0.clone(), first.1.clone()));
	assert_eq!(seq.len(), 2);
	let other = Op::Splice {
		left:	Some(Anchor::origin(st.file)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"different".to_vec(),
	};
	assert!(seq.apply(first.0.clone(), other).is_err());
	// Two headers differing only in their parents are two operations too.
	let reparented = res!(Header::new(first.0.id(), vec![OpId::new(ReplicaId::new(9), 1)]));
	assert!(seq.apply(reparented, first.1).is_err());
	Ok(())
}

/// Origins bind on one side each, a move may not name a byte twice, and an
/// operation that places bytes names at least one origin.
#[test]
fn an_operation_the_structure_cannot_resolve_is_refused() -> Outcome<()> {
	let id = OpId::new(ReplicaId::new(1), 1);
	let cid = ContentId::new(id, 0);
	let mut seq = Sequence::new();
	let head = || Header::root(OpId::new(ReplicaId::new(2), 2));
	assert!(seq.apply(head(), Op::Splice {
		left:	Some(Anchor::before(cid)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	}).is_err());
	assert!(seq.apply(head(), Op::Splice {
		left:	None,
		right:	Some(Anchor::after(cid)),
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	}).is_err());
	assert!(seq.apply(head(), Op::Move {
		src:	vec![
			res!(ContentRange::new(id, 0, 4)),
			res!(ContentRange::new(id, 2, 6)),
		],
		left:	Some(Anchor::origin(id)),
		right:	None,
	}).is_err());
	// And one that places bytes without naming where.
	assert!(seq.apply(head(), Op::Splice {
		left:	None,
		right:	None,
		remove:	Vec::new(),
		insert:	b"x".to_vec(),
	}).is_err());
	assert!(seq.is_empty());
	Ok(())
}

/// A move whose destination sits inside its own source is a cycle of length one.
/// Left unseen it detaches the move's slots from the forest and loses their
/// bytes; the demotion rule sees it, and every byte survives.
#[test]
fn a_move_into_its_own_source_keeps_its_bytes() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	let view = res!(st.reps[0].view());
	// Take "0123456789" and land it in the middle of itself.
	let src = res!(view.span(0, 10));
	let (left, right) = res!(view.gap(5));
	let op = res!(st.reps[0].author(Op::Move { src, left, right }));
	let op_id = op.0.id();
	let mut seq = Sequence::new();
	for staged in &st.ops {
		res!(seq.apply(staged.0.clone(), staged.1.clone()));
	}
	res!(seq.apply(op.0, op.1));
	let out = res!(seq.render());
	res!(seq.check_conservation(&out));
	let file = match out.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.len(), 20, "the moved bytes must not vanish");
	assert_eq!(out.flags(), &[
		Flag::Demoted { op: op_id, sub: 0, origin: Origin::Left },
		Flag::Demoted { op: op_id, sub: 0, origin: Origin::Right },
	]);
	Ok(())
}

/// In the terms the cost model is stated in.
#[test]
fn the_render_reports_what_it_cost() -> Outcome<()> {
	let mut st = res!(seed(LIST, 2));
	let (r1, r2) = st.reps.split_at_mut(1);
	st.ops.push(res!(r1[0].move_range(7, 7, 0)));
	st.ops.push(res!(r2[0].replace(9, 1, b"Soy m")));
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let out = res!(seq.render());
	let stats = out.stats();
	assert_eq!(stats.ops, 4);
	assert_eq!(stats.files, 1);
	assert_eq!(stats.atoms, 3, "the file's origin anchor and the two splices");
	assert_eq!(stats.atom_bytes, LIST.len() as u64 + 5 + 1,
		"the origin anchor is a byte like any other, and is born dead");
	assert!(stats.slots_divided >= stats.slots_placed,
		"dividing a slot never yields fewer");
	assert_eq!(stats.claim_intervals, 1, "one contiguous run moved");
	assert_eq!(stats.dead_intervals, 2, "one byte died, and the origin anchor");
	assert_eq!(stats.withheld, 0, "no file was deleted");
	assert_eq!(stats.orphaned, 0);
	match out.file(st.file) {
		Some(f)	=> assert_eq!(stats.rendered, f.len() as u64),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	Ok(())
}

/// The rendered runs still name the content that made them, so an index in the
/// render can be turned back into a name.
#[test]
fn provenance_follows_the_bytes() -> Outcome<()> {
	let mut st = res!(seed(LIST, 1));
	let seed_id = st.seed;
	st.ops.push(res!(st.reps[0].move_range(7, 7, 0)));
	let mut seq = Sequence::new();
	for op in &st.ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let out = res!(seq.render());
	let file = match out.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "- Milk\n- Eggs\n- Cheese\n");
	// The first rendered byte is now the eighth byte the seeding splice made.
	assert_eq!(res!(file.content_at(0)), ContentId::new(seed_id, 7));
	assert_eq!(res!(file.content_at(7)), ContentId::new(seed_id, 0));
	assert_eq!(res!(file.span(0, 7)), vec![res!(ContentRange::new(seed_id, 7, 14))]);
	assert!(file.content_at(file.len()).is_err());
	// The gap at the start of a file names that file's origin anchor, which is
	// what an operation binds to when there is nothing else to bind to.
	let (left, _) = res!(file.gap(0));
	assert_eq!(left, Some(Anchor::origin(st.file)));
	Ok(())
}

/// The run is taken to the front of the file, and the note's spans move with it.
///
/// Nothing was written to make this happen. A note names bytes, the render says
/// where each byte is, and the move had already changed the answer.
#[test]
fn a_note_follows_a_move() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].note(5, 5, b"why five?")));
	let before = res!(converge(&st.ops));
	let file = match before.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.notes().len(), 1);
	assert_eq!(file.notes()[0].spans(), &[Span::new(5, 5)]);
	assert_eq!(file.notes()[0].text_lossy(), "why five?");
	// Take the noted run to the front.
	st.ops.push(res!(st.reps[0].move_range(5, 5, 0)));
	let after = res!(converge(&st.ops));
	let file = match after.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "5678901234ABCDEFGHIJ");
	assert_eq!(file.notes().len(), 1);
	assert_eq!(file.notes()[0].spans(), &[Span::new(0, 5)],
		"the note went where the bytes went");
	// And the repository says the same, once.
	assert_eq!(after.notes().len(), 1);
	assert!(!after.notes()[0].on_dead());
	assert_eq!(after.notes()[0].files().len(), 1);
	assert_eq!(after.notes()[0].spans_in(st.file), &[Span::new(0, 5)]);
	Ok(())
}

/// The note is about content, and some of that content is gone.
#[test]
fn a_note_narrows_to_the_surviving_content() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].note(5, 5, b"about 56789")));
	// Delete "67" from the middle of the noted run.
	st.ops.push(res!(st.reps[0].delete(6, 2)));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "01234589ABCDEFGHIJ");
	assert_eq!(file.notes().len(), 1);
	// Three of the five bytes are left, and they are still adjacent.
	assert_eq!(file.notes()[0].spans(), &[Span::new(5, 3)]);
	assert_eq!(file.notes()[0].len(), 3);
	// An insertion inside the run is not part of the note: the note is about the
	// bytes it named, and those are not among them.
	st.ops.push(res!(st.reps[0].insert(6, b"xx")));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "012345xx89ABCDEFGHIJ");
	assert_eq!(file.notes()[0].spans(), &[Span::new(5, 1), Span::new(8, 2)]);
	Ok(())
}

/// A note whose content has been deleted entirely is not lost: it is reported as
/// a note on dead content, and it shows in no file.
#[test]
fn a_note_on_deleted_content_says_so() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].note(5, 5, b"doomed")));
	st.ops.push(res!(st.reps[0].delete(5, 5)));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "01234ABCDEFGHIJ");
	assert!(file.notes().is_empty(), "no margin has anything to point at");
	assert_eq!(repo.notes().len(), 1, "the note itself is not lost");
	assert!(repo.notes()[0].on_dead());
	assert!(repo.notes()[0].files().is_empty());
	assert_eq!(repo.notes()[0].text_lossy(), "doomed");
	assert_eq!(repo.dead_notes().len(), 1);
	assert_eq!(repo.stats().notes, 1);
	Ok(())
}

/// Two spans, because that is where its content is.
#[test]
fn a_note_tears_with_its_content() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].note(5, 5, b"one run, for now")));
	// Take the middle two bytes of the noted run to the end of the file.
	st.ops.push(res!(st.reps[0].move_range(7, 2, 20)));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "01234569ABCDEFGHIJ78");
	assert_eq!(file.notes().len(), 1);
	assert_eq!(file.notes()[0].spans().len(), 2,
		"the noted run is in two places, so the note is in two places");
	assert_eq!(file.notes()[0].len(), 5, "and no byte of it was lost");
	Ok(())
}

/// Two notes on one file are handed over in the order a margin would draw them,
/// and a note lands on the exact bytes it named.
#[test]
fn notes_arrive_in_render_order() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].note(12, 4, b"second")));
	st.ops.push(res!(st.reps[0].note(2, 3, b"first")));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	let texts: Vec<String> = file.notes().iter().map(|n| n.text_lossy()).collect();
	assert_eq!(texts, vec![fmt!("first"), fmt!("second")]);
	// The repository lists them in identifier order instead, which is what every
	// other list in the render is in.
	let ids: Vec<OpId> = repo.notes().iter().map(|n| n.note()).collect();
	let mut sorted = ids.clone();
	sorted.sort();
	assert_eq!(ids, sorted);
	// The span names exactly the bytes the note was written against.
	let n = match file.note(ids[0]) {
		Some(n)	=> n,
		None	=> return Err(err!("A note went missing."; Test, Missing)),
	};
	let span = n.spans()[0];
	assert_eq!(
		&file.bytes()[span.at as usize..span.end() as usize],
		match n.text_lossy().as_str() {
			"second"	=> &b"CDEF"[..],
			_			=> &b"234"[..],
		},
	);
	Ok(())
}

/// A note about nothing is refused by the frontend that would have written it,
/// and by the structure that would have held it.
#[test]
fn a_note_about_nothing_is_refused() -> Outcome<()> {
	let st = res!(seed(ALPHA, 1));
	let view = res!(st.reps[0].view());
	assert!(view.note_on(4, 0, b"about what?".to_vec()).is_err());
	// And beyond the file, which is the other way to name nothing.
	assert!(view.note_on(40, 2, b"beyond".to_vec()).is_err());
	Ok(())
}

/// The render read backwards puts named content where a reader will find it,
/// wherever a move has since taken it.
///
/// This is the lookup a note resolves through, asked directly, because a flag
/// names content too and its reader wants a position in a file rather than an
/// offset into an operation.
#[test]
fn content_is_found_where_it_now_renders() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	// Take "56789" to the front, so that the seeded content renders in three runs
	// and none of them where it was written.
	st.ops.push(res!(st.reps[0].move_range(5, 5, 0)));
	let repo = res!(converge(&st.ops));
	let file = match repo.file(st.file) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(file.text_lossy(), "5678901234ABCDEFGHIJ");
	let placed = repo.placement();
	// The moved run, which is at the front now.
	let found = placed.find(&[res!(ContentRange::new(st.seed, 5, 10))]);
	assert_eq!(found.len(), 1, "one file shows it");
	assert_eq!(found[0].file, st.file);
	assert_eq!(found[0].spans, vec![Span::new(0, 5)]);
	// A range straddling the move renders in two places, and the two runs that
	// abut are reported as one span rather than as the seam between them.
	let found = placed.find(&[res!(ContentRange::new(st.seed, 3, 12))]);
	assert_eq!(found.len(), 1);
	assert_eq!(found[0].spans, vec![Span::new(0, 5), Span::new(8, 4)]);
	Ok(())
}

/// Content that renders nowhere is answered with nowhere, which is what lets a
/// caller say so rather than invent a place.
#[test]
fn dead_content_is_found_in_no_file() -> Outcome<()> {
	let mut st = res!(seed(ALPHA, 1));
	st.ops.push(res!(st.reps[0].delete(5, 5)));
	let repo = res!(converge(&st.ops));
	let placed = repo.placement();
	assert!(placed.find(&[res!(ContentRange::new(st.seed, 5, 10))]).is_empty(),
		"the bytes are dead, so no file shows them");
	// The live neighbours of the dead run are still found, so the emptiness is
	// about the content and not about the lookup.
	let found = placed.find(&[res!(ContentRange::new(st.seed, 0, 20))]);
	assert_eq!(found.len(), 1);
	assert_eq!(found[0].spans, vec![Span::new(0, 15)]);
	Ok(())
}
