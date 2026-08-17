//! The file-identity cases: what happens when content crosses a file boundary,
//! when two branches create one path, and when a file is deleted around a move.
//!
//! Every case here was written and predicted in the throwaway multi-file oracle
//! before it was run, and every expectation below is that oracle's answer under
//! the rule this crate now implements: content operations carry no file, a file's
//! creation mints an origin anchor, and a file is the subtree beneath one.
//!
//! Two invariants are checked in every case, over the whole repository rather
//! than one file. No byte may render in two places at once, which is what a
//! per-file claim register produces and what no per-file check would see. And no
//! live byte may render nowhere. Both are the point of the exercise; the bytes
//! themselves are only how the answer is read.
//!
//! The ten single-file cases are beside this file in `tests.rs`, and they run
//! through this same engine: file identity must not perturb single-file
//! semantics, and that they are unchanged is the instrument for saying so.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
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
};
use crate::seq::render::{
	Flag,
	Rendered,
	Repo,
	Span,
};
use crate::seq::{
	OpOrder,
	Sequence,
};

use oxedyne_fe2o3_core::prelude::*;


// The shopping list of the published worked case.
const LIST: &[u8] = b"- Eggs\n- Milk\n- Cheese\n";


/// One replica of a repository of several files: the frontend that turns
/// index-based editing intent, in a named file, into content-anchored
/// operations.
///
/// Nothing here distinguishes a move within a file from a move between two,
/// except which render the destination gap is read from. That is the claim the
/// whole exercise was to test, stated as code.
struct Replica {
	id:		u64,			// every operation of this replica is named by it
	seq:	Sequence,
}

impl Replica {

	fn new(id: u64) -> Self {
		Self { id, seq: Sequence::new() }
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

	fn view(&self, file: OpId)
		-> Outcome<Rendered>
	{
		let repo = res!(self.seq.render());
		match repo.file(file) {
			Some(f)	=> Ok(f.clone()),
			None	=> Err(err!("The replica has no file {}.", file; Test, Missing)),
		}
	}

	fn author(&mut self, op: Op)
		-> Outcome<(Header, Op)>
	{
		let head = res!(self.next_head());
		res!(self.seq.apply(head.clone(), op.clone()));
		Ok((head, op))
	}

	fn create(&mut self, path: &[u8])
		-> Outcome<((Header, Op), OpId)>
	{
		let made = res!(self.author(Op::FileCreate { path: path.to_vec() }));
		let id = made.0.id();
		Ok((made, id))
	}

	fn rename(&mut self, file: OpId, path: &[u8])
		-> Outcome<(Header, Op)>
	{
		self.author(Op::FileRename { file, path: path.to_vec() })
	}

	fn remove(&mut self, file: OpId)
		-> Outcome<(Header, Op)>
	{
		self.author(Op::FileDelete { file })
	}

	fn set_mode(&mut self, file: OpId, mode: Mode)
		-> Outcome<(Header, Op)>
	{
		self.author(Op::FileMode { file, mode })
	}

	fn insert(&mut self, file: OpId, at: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, 0, bytes.to_vec()));
		self.author(op)
	}

	fn delete(&mut self, file: OpId, at: usize, len: usize)
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, len, Vec::new()));
		self.author(op)
	}

	/// One splice, not a deletion and an insertion.
	fn replace(&mut self, file: OpId, at: usize, len: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, len, bytes.to_vec()));
		self.author(op)
	}

	/// At an index of each file.
	fn move_across(
		&mut self,
		from:	OpId,
		at:		usize,
		len:	usize,
		to:		OpId,
		dest:	usize,
	)
		-> Outcome<(Header, Op)>
	{
		let src = res!(self.view(from));
		let dst = res!(self.view(to));
		let op = res!(src.move_into(at, len, &dst, dest));
		self.author(op)
	}

	fn note(&mut self, file: OpId, at: usize, len: usize, text: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).note_on(at, len, text.to_vec()));
		self.author(op)
	}
}


/// Creates the named files with the given contents on replica zero, then hands
/// out `n` further replicas that have seen all of it.
fn stage(files: &[(&[u8], &[u8])], n: u64)
	-> Outcome<(Vec<Replica>, Vec<(Header, Op)>, Vec<OpId>)>
{
	let mut origin = Replica::new(0);
	let mut ops: Vec<(Header, Op)> = Vec::new();
	let mut ids: Vec<OpId> = Vec::new();
	for (path, text) in files {
		let (made, id) = res!(origin.create(path));
		ops.push(made);
		ids.push(id);
		if !text.is_empty() {
			ops.push(res!(origin.insert(id, 0, text)));
		}
	}
	let mut reps: Vec<Replica> = Vec::new();
	for i in 1..=n {
		let mut r = Replica::new(i);
		for op in &ops {
			res!(r.recv(op.clone()));
		}
		reps.push(r);
	}
	Ok((reps, ops, ids))
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

/// A one-line listing of the live files, as a working copy would show them.
fn listing(repo: &Repo) -> String {
	let mut files: Vec<&Rendered> = repo.files().iter().filter(|f| f.is_live()).collect();
	files.sort_by_key(|f| (f.path().to_vec(), OpOrder::of(&f.file())));
	let mut s = String::new();
	for f in files {
		// A mode is shown only where it is not the one a file has by default, so
		// that every case written before the vocabulary had modes reads as it did
		// -- and every one of them now proves, for nothing, that the mode is a
		// function of the operation set and not of the delivery order.
		let mode = if f.mode().is_normal() {
			String::new()
		} else {
			fmt!("[{}]", f.mode())
		};
		s.push_str(&fmt!("{}{}={:?} ", f.path_lossy(), mode, f.text_lossy()));
	}
	s.trim_end().to_string()
}

/// Applies an operation set in every delivery order where the set is small
/// enough, or in every rotation and a spread of shuffles where it is not, and
/// requires that all of them render the same files and raise the same flags.
///
/// Conservation is checked repository-wide on every order: nothing rendered
/// twice, nothing live rendered nowhere.
fn converge(ops: &[(Header, Op)])
	-> Outcome<Repo>
{
	let n = ops.len();
	let mut orders: Vec<Vec<usize>> = Vec::new();
	if n <= 7 {
		let mut idx: Vec<usize> = (0..n).collect();
		permute(&mut idx, 0, &mut orders);
	} else {
		// Every rotation, the reverse, and a spread of shuffles, since
		// enumerating factorially many orders buys nothing over sampling them.
		for k in 0..n {
			orders.push((0..n).map(|i| (i + k) % n).collect());
		}
		orders.push((0..n).rev().collect());
		let mut state = 0x0f1e_2d3c_4b5a_6978u64.wrapping_add(n as u64);
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		for _ in 0..200 {
			let mut idx: Vec<usize> = (0..n).collect();
			for i in (1..idx.len()).rev() {
				idx.swap(i, next() % (i + 1));
			}
			orders.push(idx);
		}
	}
	let mut first: Option<Repo> = None;
	for order in &orders {
		let mut seq = Sequence::new();
		for i in order {
			res!(seq.apply(ops[*i].0.clone(), ops[*i].1.clone()));
		}
		let got = res!(seq.render());
		res!(seq.check_conservation(&got));
		assert_eq!(got.stats().orphaned, 0, "a slot belonged to no file");
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
				// A note is derived from the render, so this ought to come for
				// nothing; it is asserted anyway, in every case, because "ought to"
				// is not a test.
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

/// Runs an operation set under every delivery order and checks the live files
/// against the answer the case prescribes.
fn case(expect: &str, ops: &[(Header, Op)])
	-> Outcome<Repo>
{
	let repo = res!(converge(ops));
	assert_eq!(listing(&repo), expect);
	Ok(repo)
}

/// The text of one file, deleted or not.
fn text(repo: &Repo, file: OpId)
	-> Outcome<String>
{
	match repo.file(file) {
		Some(f)	=> Ok(f.text_lossy()),
		None	=> Err(err!("The repository has no file {}.", file; Test, Missing)),
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE FILE-IDENTITY CASES                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The headline claim: a concurrent edit inside a range that moves *between
/// files* follows it, for exactly the reason an in-file one does.
///
/// This is the case that decided the design. Where a content operation records a
/// file, the edit's anchor names content that has left that file and cannot
/// follow it out, so the insertion stays behind and the word is torn in half
/// across two files -- `Soy m` in one and `ilk` in the other, everything
/// converging and nothing lost, which is a placement failure of the kind a user
/// would call corruption. Naming no file at all is what makes the edit arrive.
#[test]
fn a_cross_file_move_carries_a_concurrent_edit_with_it() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", LIST), (b"b.txt", b"HEADER\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 moves "- Milk\n" out of a.txt and onto the end of b.txt.
	ops.push(res!(r1[0].move_across(a, 7, 7, b, 7)));
	// Replica 2 concurrently turns "Milk" into "Soy milk", in a.txt.
	ops.push(res!(r2[0].replace(a, 9, 1, b"Soy m")));
	res!(case(
		"a.txt=\"- Eggs\\n- Cheese\\n\" b.txt=\"HEADER\\n- Soy milk\\n\"",
		&ops,
	));
	Ok(())
}

#[test]
fn a_cross_file_move_arbitrates_against_an_edit_at_its_destination() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", LIST), (b"b.txt", b"HEADER\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 moves "- Milk\n" to the start of b.txt.
	ops.push(res!(r1[0].move_across(a, 7, 7, b, 0)));
	// Replica 2 concurrently types an X at the start of b.txt.
	ops.push(res!(r2[0].insert(b, 0, b"X")));
	res!(case(
		"a.txt=\"- Eggs\\n- Cheese\\n\" b.txt=\"- Milk\\nXHEADER\\n\"",
		&ops,
	));
	Ok(())
}

/// A walk that reconstructs the association from a path decides this by arrival
/// order: the later creation takes the path, and the earlier file keeps its
/// operations and its bytes and renders nowhere. Under identity both files exist,
/// both keep their bytes, and the collision is a naming question at
/// materialisation time rather than a silent loss.
#[test]
fn two_branches_creating_one_path_make_two_files() -> Outcome<()> {
	let mut r1 = Replica::new(1);
	let mut r2 = Replica::new(2);
	let (c1, f1) = res!(r1.create(b"notes.md"));
	let s1 = res!(r1.insert(f1, 0, b"one"));
	let (c2, f2) = res!(r2.create(b"notes.md"));
	let s2 = res!(r2.insert(f2, 0, b"two"));
	let repo = res!(converge(&[c1, s1, c2, s2]));
	assert_eq!(res!(text(&repo, f1)), "one");
	assert_eq!(res!(text(&repo, f2)), "two");
	// Both are live and both claim the path, which the repository reports rather
	// than resolving: which one a working copy writes there is a policy, and the
	// higher in op order keeping the name is one answer among several.
	let clashes = repo.clashes();
	assert_eq!(clashes.len(), 1);
	assert_eq!(clashes[0].0, b"notes.md");
	assert_eq!(repo.at_path(b"notes.md").len(), 2);
	assert!(OpOrder::of(&f2) > OpOrder::of(&f1),
		"the later creation is higher in op order");
	Ok(())
}

/// A file deleted just after content moved out of it. The bytes that left must
/// survive; the bytes that stayed must not render anywhere a reader looks.
///
/// This is the working form of the second cost: a file's deletion retires a file
/// and destroys no atom.
#[test]
fn deleting_a_file_keeps_what_moved_out_of_it() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 1));
	let (a, b) = (ids[0], ids[1]);
	ops.push(res!(reps[0].move_across(a, 5, 5, b, 0)));
	ops.push(res!(reps[0].remove(a)));
	let repo = res!(case("b.txt=\"move\\n\"", &ops));
	assert_eq!(res!(text(&repo, b)), "move\n");
	// What stayed behind is held rather than destroyed: the deleted file still
	// holds it, and the render says how much is withheld.
	assert_eq!(res!(text(&repo, a)), "keep\n");
	assert_eq!(repo.stats().withheld, 5);
	Ok(())
}

/// The same, with the deletion expressed the way it must not be: as a splice
/// removing everything the file held.
///
/// A tombstone is repository-global and follows the content, so a deletion that
/// kills bytes rather than retiring a file destroys what has just moved out of
/// it. That is why file lifecycle and content lifecycle are separate operations,
/// and this is the cost stated as a test.
#[test]
fn a_content_delete_destroys_what_moved_out() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].move_across(a, 5, 5, b, 0)));
	// Replica 2 empties a.txt by content, concurrently.
	ops.push(res!(r2[0].delete(a, 0, 10)));
	res!(case("a.txt=\"\" b.txt=\"\"", &ops));
	Ok(())
}

/// Two agents reorganising two files at once, each moving one file's contents
/// into the other: a genuine cycle in the anchor graph, across a file boundary.
///
/// The cycle is arbitrated rather than demoted. Both moves are treated as one
/// concurrent group, the higher in op order wins wholly, and the other is confined
/// -- its content stays where it was, and both files are told.
///
/// **b.txt is empty because its author emptied it**, having moved the whole of it
/// into a.txt, and that move completed. The string alone looks like the collapse
/// this rule exists to prevent, and it is the opposite of it: under demotion a.txt
/// was emptied by the renderer, which nobody asked for.
#[test]
fn a_cross_file_cycle_lets_the_later_move_win() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 moves the whole of a.txt into b.txt, after its first byte.
	let m1 = res!(r1[0].move_across(a, 0, 4, b, 1));
	let m1_id = m1.0.id();
	ops.push(m1);
	// Replica 2 concurrently moves the whole of b.txt into a.txt, after its
	// first byte.
	let m2 = res!(r2[0].move_across(b, 0, 4, a, 1));
	let m2_id = m2.0.id();
	ops.push(m2);
	assert!(OpOrder::of(&m2_id) > OpOrder::of(&m1_id), "the second move is higher");
	let repo = res!(case("a.txt=\"axyz\\nbc\\n\" b.txt=\"\"", &ops));
	assert_eq!(res!(text(&repo, b)), "");
	// The lower move did not happen: its block is still in a.txt, and b.txt is the
	// file it was aimed at and did not reach.
	assert!(repo.flags().contains(&Flag::Confined { op: m1_id, home: a, denied: b }),
		"flags were {:?}", repo.flags());
	assert!(repo.flags().contains(&Flag::Won { op: m2_id }),
		"flags were {:?}", repo.flags());
	// Nothing was demoted, so nothing crossed a boundary by demotion either.
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Demoted { .. })),
		"a cross-file cycle reached demotion: {:?}", repo.flags());
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::CrossedFile { .. })),
		"flags were {:?}", repo.flags());
	// Both files are told, since the flag is about the pair of them.
	for file in [a, b] {
		match repo.file(file) {
			Some(f) => assert!(
				f.flags().iter().any(|g| matches!(g, Flag::Confined { .. })),
				"the file {} was not told", file),
			None => return Err(err!("A file went missing."; Test, Missing)),
		}
	}
	Ok(())
}

/// The same two-file cycle, with a third replica editing inside one of the
/// cycling blocks.
///
/// The composition question the arbitration has to answer. The edit's origin names
/// content, the content is owned by whoever owns it after the arbitration, and the
/// edit binds there without anything being written to make it do so.
#[test]
fn an_edit_inside_a_cycling_block_follows_the_block() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n")], 3));
	let (a, b) = (ids[0], ids[1]);
	ops.push(res!(reps[0].move_across(a, 0, 4, b, 1)));
	ops.push(res!(reps[1].move_across(b, 0, 4, a, 1)));
	// Concurrently with both, an exclamation mark between 'b' and 'c'.
	ops.push(res!(reps[2].insert(a, 2, b"!")));
	let repo = res!(case("a.txt=\"axyz\\nb!c\\n\" b.txt=\"\"", &ops));
	assert!(res!(text(&repo, a)).contains("b!c"), "the edit went astray");
	Ok(())
}

/// A cycle of three whose middle member never leaves its file, which is what
/// whole-cycle arbitration costs.
///
/// A two-cycle cannot be mixed -- each member lands inside the other's source, so
/// either both cross a boundary or neither does -- and three is the shortest cycle
/// that can hold an in-file move. **That in-file move is voided along with the
/// rest**, though it never went near a file boundary and was in the cycle only by
/// accident of what it anchored to. It is the strongest argument against the rule,
/// and the answer is that a voided move is flagged as not having happened whereas
/// a misplaced one has to be found: under demotion this same move is carried into
/// a file its author never named.
#[test]
fn a_mixed_cycle_voids_its_in_file_member_too() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"wxyz")], 3));
	let (a, b) = (ids[0], ids[1]);
	// The whole of a.txt into b.txt, after 'w': crossing.
	let m1 = res!(reps[0].move_across(a, 0, 4, b, 1));
	let m1_id = m1.0.id();
	ops.push(m1);
	// "wx" to after 'y', within b.txt: not crossing anything.
	let m2 = res!(reps[1].move_across(b, 0, 2, b, 3));
	let m2_id = m2.0.id();
	ops.push(m2);
	// "yz" out of b.txt into a.txt, after 'a': crossing back.
	let m3 = res!(reps[2].move_across(b, 2, 2, a, 1));
	let m3_id = m3.0.id();
	ops.push(m3);
	let repo = res!(case("a.txt=\"ayzbc\\n\" b.txt=\"wx\"", &ops));
	assert!(repo.flags().contains(&Flag::Won { op: m3_id }),
		"flags were {:?}", repo.flags());
	assert!(repo.flags().contains(&Flag::Confined { op: m1_id, home: a, denied: b }),
		"flags were {:?}", repo.flags());
	// The in-file move is confined with one file at both ends, which is the honest
	// way to say that it was voided for being in the cycle and nothing else.
	assert!(repo.flags().contains(&Flag::Confined { op: m2_id, home: b, denied: b }),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// An author re-moving their own block, having seen the move that raced it.
///
/// The first move is superseded and owns nothing, so it leaves the anchor graph;
/// the cycle is between the concurrent move and the re-move. The re-move wins, as
/// the op-order maximum and as the informed member both, and a block whose author
/// twice said "into b.txt" renders in b.txt.
///
/// This is also the case that makes the birth classifier necessary. Voiding the
/// whole cycle to ask which files it runs between resurrects the superseded move,
/// which carries the content over the boundary itself, and the cycle then looks as
/// though it stays inside one file. Asking where the bytes were *written* is what
/// sees through it.
#[test]
fn an_informed_re_move_wins_its_cycle() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	// The splices that wrote the two files' contents.
	let (sa, sb) = (ops[1].0.id(), ops[3].0.id());
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].move_across(a, 0, 4, b, 1)));
	let m2 = res!(r2[0].move_across(b, 0, 4, a, 1));
	let m2_id = m2.0.id();
	ops.push(m2.clone());
	// Replica 1 receives the move that raced it, and moves its own block again, to
	// the end of what b.txt originally held.
	res!(r1[0].recv(m2));
	let m3 = res!(r1[0].author(Op::Move {
		src:	vec![res!(ContentRange::new(sa, 0, 4))],
		left:	Some(Anchor::after(ContentId::new(sb, 3))),
		right:	None,
	}));
	let m3_id = m3.0.id();
	ops.push(m3);
	let repo = res!(case("a.txt=\"\" b.txt=\"xyz\\nabc\\n\"", &ops));
	assert!(repo.flags().contains(&Flag::Won { op: m3_id }),
		"flags were {:?}", repo.flags());
	// The confined move's own content stays in b.txt, which is what matters to its
	// author. The file it is reported as having been denied is b.txt as well, and
	// that is an artefact of the counterfactual worth stating rather than hiding:
	// the file a destination anchor is in is read off a layout with the cycle
	// voided, and voiding this cycle resurrects the superseded first move, which
	// carries the anchor into b.txt itself. The classification is still right,
	// because the birth reading sees the boundary the counterfactual has hidden.
	assert!(repo.flags().contains(&Flag::Confined { op: m2_id, home: b, denied: b }),
		"flags were {:?}", repo.flags());
	Ok(())
}

#[test]
fn a_move_out_of_a_file_survives_its_deletion() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].move_across(a, 5, 5, b, 0)));
	ops.push(res!(r2[0].remove(a)));
	let repo = res!(case("b.txt=\"move\\n\"", &ops));
	assert_eq!(res!(text(&repo, b)), "move\n");
	Ok(())
}

/// Neither is difficult now, and the case is here because the vocabulary this
/// replaces could not do it at all: a splice that names no existing content
/// could only be routed by the path it recorded, and the rename had just made
/// that path wrong. The splice names the file's origin anchor instead, and a
/// rename cannot touch an identity.
#[test]
fn an_insertion_survives_the_rename_of_its_file() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"a.txt", b"")], 2));
	let a = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].rename(a, b"b.txt")));
	ops.push(res!(r2[0].insert(a, 0, b"hi")));
	res!(case("b.txt=\"hi\"", &ops));
	Ok(())
}

/// Three agents reorganising three files at once, each emptying one into the
/// next: the cycle at length three, and across files.
///
/// One move completes and two are confined, so two of the three files keep their
/// own content. c.txt is empty because its author emptied it into a.txt, which is
/// the move that won.
#[test]
fn a_three_file_cycle_keeps_two_files() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n"), (b"c.txt", b"123\n")],
		3,
	));
	let (a, b, c) = (ids[0], ids[1], ids[2]);
	let m1 = res!(reps[0].move_across(a, 0, 4, b, 1));
	let m1_id = m1.0.id();
	ops.push(m1);
	let m2 = res!(reps[1].move_across(b, 0, 4, c, 1));
	let m2_id = m2.0.id();
	ops.push(m2);
	let m3 = res!(reps[2].move_across(c, 0, 4, a, 1));
	let m3_id = m3.0.id();
	ops.push(m3);
	let repo = res!(case(
		"a.txt=\"a123\\nbc\\n\" b.txt=\"xyz\\n\" c.txt=\"\"",
		&ops,
	));
	assert_eq!(res!(text(&repo, c)), "");
	assert_eq!(
		res!(text(&repo, a)).len() + res!(text(&repo, b)).len(),
		12,
		"every byte survives the cycle",
	);
	assert!(repo.flags().contains(&Flag::Won { op: m3_id }),
		"flags were {:?}", repo.flags());
	assert!(repo.flags().contains(&Flag::Confined { op: m1_id, home: a, denied: b }),
		"flags were {:?}", repo.flags());
	assert!(repo.flags().contains(&Flag::Confined { op: m2_id, home: b, denied: c }),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// The cycle at length four, which is where breaking a single edge stops helping:
/// three files empty under that rule and one does under this one.
#[test]
fn a_four_file_cycle_keeps_three_files() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[
			(b"a.txt", b"abc\n"),
			(b"b.txt", b"xyz\n"),
			(b"c.txt", b"123\n"),
			(b"d.txt", b"pqr\n"),
		],
		4,
	));
	let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
	ops.push(res!(reps[0].move_across(a, 0, 4, b, 1)));
	ops.push(res!(reps[1].move_across(b, 0, 4, c, 1)));
	ops.push(res!(reps[2].move_across(c, 0, 4, d, 1)));
	let m4 = res!(reps[3].move_across(d, 0, 4, a, 1));
	let m4_id = m4.0.id();
	ops.push(m4);
	let repo = res!(case(
		"a.txt=\"apqr\\nbc\\n\" b.txt=\"xyz\\n\" c.txt=\"123\\n\" d.txt=\"\"",
		&ops,
	));
	assert!(repo.flags().contains(&Flag::Won { op: m4_id }),
		"flags were {:?}", repo.flags());
	assert_eq!(
		repo.flags().iter().filter(|f| matches!(f, Flag::Confined { .. })).count(),
		3,
		"three of the four moves lose",
	);
	Ok(())
}

/// A cycle that stays inside one file is none of the arbitration's business, even
/// where the repository holds other files for it to have escaped into.
///
/// Two moves whose destinations sit inside each other's sources, both within one
/// file: demotion settles it, exactly as it did before the arbitration existed,
/// and nothing is confined.
#[test]
fn an_in_file_cycle_is_still_demoted() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"f.txt", b"0123456789ABCDEFGHIJ"), (b"other.txt", b"kept\n")], 2));
	let f = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].move_across(f, 0, 5, f, 12)));
	ops.push(res!(r2[0].move_across(f, 10, 5, f, 2)));
	let repo = res!(case(
		"f.txt=\"5678901ABCDE234FGHIJ\" other.txt=\"kept\\n\"",
		&ops,
	));
	assert!(repo.flags().iter().any(|f| matches!(f, Flag::Demoted { .. })),
		"an in-file cycle was not demoted: {:?}", repo.flags());
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Confined { .. })),
		"an in-file cycle was confined: {:?}", repo.flags());
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Won { .. })),
		"an in-file cycle was arbitrated: {:?}", repo.flags());
	Ok(())
}

/// The bytes are not lost -- a slot owns them and the log holds them -- but
/// nothing a reader looks at renders them. That is a hazard the design note did
/// not name until the oracle found it, and it is what the flag exists for.
#[test]
fn content_moved_into_a_deleted_file_goes_quiet_and_says_so() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_across(a, 5, 5, b, 0));
	let mv_id = mv.0.id();
	ops.push(mv);
	ops.push(res!(r2[0].remove(b)));
	let repo = res!(case("a.txt=\"keep\\n\"", &ops));
	// The bytes are in the deleted file, whole, and withheld.
	assert_eq!(res!(text(&repo, b)), "move\n");
	assert_eq!(repo.stats().withheld, 5);
	assert!(repo.flags().contains(&Flag::MovedIntoDeleted { op: mv_id, file: b }),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// A range moved from one file to a second and then to a third, with an edit
/// made concurrently with the first move. The edit has to arrive two files away
/// from where its author was looking, and it does.
///
/// This case is also where the torn flag's defect showed: the first move is
/// superseded by the second, by the same author, and every candidate reported it
/// as a race until the flag was made to consult causality.
#[test]
fn a_move_chained_through_a_third_file_carries_the_edit() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", LIST), (b"b.txt", b"B\n"), (b"c.txt", b"C\n")],
		2,
	));
	let (a, b, c) = (ids[0], ids[1], ids[2]);
	let (r1, r2) = reps.split_at_mut(1);
	let first = res!(r1[0].move_across(a, 7, 7, b, 2));
	let first_id = first.0.id();
	ops.push(first);
	ops.push(res!(r1[0].move_across(b, 2, 7, c, 2)));
	ops.push(res!(r2[0].replace(a, 9, 1, b"Soy m")));
	let repo = res!(case(
		"a.txt=\"- Eggs\\n- Cheese\\n\" b.txt=\"B\\n\" c.txt=\"C\\n- Soy milk\\n\"",
		&ops,
	));
	assert_eq!(res!(text(&repo, b)), "B\n");
	assert_eq!(res!(text(&repo, c)), "C\n- Soy milk\n");
	// Nothing tore: the second move was written knowing the first, so the first
	// was superseded on purpose rather than raced.
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Torn { op, .. } if *op == first_id)),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// A move whose source names a file's origin anchor.
///
/// No frontend can author this: the origin anchor is born dead and never appears
/// in a render, so nothing a user points at can name it. It is here because a
/// wire format has to say what a receiver does with an operation nobody meant to
/// send, and what it does turns out to be coherent -- the whole of one file lands
/// inside another, no byte lost and none duplicated. Whether the renderer should
/// refuse such an operation or adopt it as a file-concatenation primitive is an
/// open question; what is settled is that it converges and conserves.
#[test]
fn moving_a_files_origin_anchor_concatenates_the_files() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"AAA\n"), (b"b.txt", b"BBB\n")], 1));
	let (a, b) = (ids[0], ids[1]);
	ops.push(res!(reps[0].author(Op::Move {
		src:	vec![res!(ContentRange::new(b, 0, 1))],
		left:	Some(Anchor::origin(a)),
		right:	None,
	})));
	let repo = res!(case("a.txt=\"AAA\\nBBB\\n\" b.txt=\"\"", &ops));
	assert_eq!(res!(text(&repo, b)), "");
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROPERTIES ACROSS FILES                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// What this earns is not convergence, which is nearly free where the state is
/// the operation set, but the repository-wide conservation invariant over
/// operation sets nobody wrote by hand: no byte in two files, and no live byte
/// nowhere, across a few hundred cross-file moves.
#[test]
fn random_multi_file_sets_render_alike_and_conserve() -> Outcome<()> {
	let mut state = 0x2545_F491_4F6C_DD1Du64;
	let mut next = move || {
		state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(state >> 33) as usize
	};
	let mut moves = 0usize;
	let mut cross = 0usize;
	for trial in 0..40 {
		let (mut reps, mut ops, _) = res!(stage(
			&[(b"a.txt", b"alpha\n"), (b"b.txt", b"beta\n"), (b"c.txt", b"")],
			3,
		));
		let staged = ops.len();
		let mut upto = vec![staged; reps.len()];
		for _ in 0..14 {
			let who = next() % reps.len();
			// Operations are delivered as a prefix, an operation being anchored
			// in what its author could see, so a prefix is causally complete.
			let target = upto[who] + next() % (ops.len() - upto[who] + 1);
			while upto[who] < target {
				let op = ops[upto[who]].clone();
				res!(reps[who].recv(op));
				upto[who] += 1;
			}
			let live: Vec<OpId> = {
				let repo = res!(reps[who].seq.render());
				repo.live().iter().map(|f| f.file()).collect()
			};
			if live.is_empty() {
				continue;
			}
			let file = live[next() % live.len()];
			let n = res!(reps[who].view(file)).len();
			let at = if n == 0 { 0 } else { next() % (n + 1) };
			let made = match next() % 8 {
				0 => res!(reps[who].insert(file, at, b"[]")),
				1 if n > at => {
					let len = (1 + next() % 4).min(n - at);
					res!(reps[who].delete(file, at, len))
				},
				2 => res!(reps[who].create(
					&fmt!("f{}.txt", next() % 4).into_bytes())).0,
				3 => res!(reps[who].rename(file, &fmt!("r{}.txt", next() % 4).into_bytes())),
				4 if live.len() > 1 => res!(reps[who].remove(file)),
				5 if n > at => {
					let len = (1 + next() % 4).min(n - at);
					let dest = next() % (n - len + 1);
					moves += 1;
					res!(reps[who].move_across(file, at, len, file, dest))
				},
				_ if n > at => {
					let len = (1 + next() % 4).min(n - at);
					let to = live[next() % live.len()];
					let dn = res!(reps[who].view(to)).len();
					if to != file {
						cross += 1;
					}
					moves += 1;
					res!(reps[who].move_across(file, at, len, to, next() % (dn + 1)))
				},
				_ => continue,
			};
			ops.push(made);
		}
		let mut want: Option<Repo> = None;
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
			assert_eq!(got.stats().orphaned, 0,
				"trial {} round {} left a slot in no file", trial, round);
			match &want {
				None => want = Some(got),
				Some(first) => {
					assert_eq!(listing(first), listing(&got),
						"trial {} round {} disagreed on the files", trial, round);
					assert_eq!(first.flags(), got.flags(),
						"trial {} round {} disagreed on the flags", trial, round);
				},
			}
		}
	}
	assert!(moves > 50, "only {} moves were exercised", moves);
	assert!(cross > 10, "only {} cross-file moves were exercised", cross);
	Ok(())
}

/// Cycles planted deliberately, at every length from two to four, with and
/// without a concurrent edit inside a cycling block: each converges, conserves,
/// and leaves every confined move's content in the file its flag names.
///
/// The last of those is the property the whole rule rests on, and it is not the
/// same claim as convergence. A move that did not happen has to leave its bytes
/// where they were, and a check that only compared two replicas would agree with
/// itself while they went somewhere else together.
#[test]
fn planted_cross_file_cycles_leave_confined_content_at_home() -> Outcome<()> {
	const NAMES: [&[u8]; 4] = [b"a.txt", b"b.txt", b"c.txt", b"d.txt"];
	const TEXTS: [&[u8]; 4] = [b"abcd\n", b"wxyz\n", b"1234\n", b"pqrs\n"];
	let mut state = 0x9e37_79b9_7f4a_7c15u64;
	let mut next = move || {
		state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(state >> 33) as usize
	};
	let mut planted = 0usize;
	let mut confinements = 0usize;
	for trial in 0..12 {
		let k = 2 + trial % 3;
		let staged: Vec<(&[u8], &[u8])> = (0..k).map(|i| (NAMES[i], TEXTS[i])).collect();
		// One replica per move, and one more for the edit where there is one.
		let (mut reps, mut ops, ids) = res!(stage(&staged, k as u64 + 1));
		// Each replica moves the whole of one file into the next, at an index
		// strictly inside what that file holds, which is what closes the loop.
		for i in 0..k {
			let n = res!(reps[i].view(ids[(i + 1) % k])).len();
			let at = 1 + next() % (n - 1);
			ops.push(res!(reps[i].move_across(ids[i], 0, n, ids[(i + 1) % k], at)));
			planted += 1;
		}
		if trial % 2 == 0 {
			// An edit inside the first file's block, concurrent with every move.
			ops.push(res!(reps[k].insert(ids[0], 2, b"!")));
		}
		let repo = res!(converge(&ops));
		for flag in repo.flags() {
			let (op, home) = match flag {
				Flag::Confined { op, home, .. }	=> (*op, *home),
				_				=> continue,
			};
			confinements += 1;
			let named = match ops.iter().find(|(head, _)| head.id() == op) {
				Some((_, o))	=> o.regions().to_vec(),
				None		=> return Err(err!(
					"A flag named an operation the case did not author."; Test, Bug)),
			};
			let file = match repo.file(home) {
				Some(f)	=> f,
				None	=> return Err(err!(
					"A confinement named a file the render does not hold."; Test, Missing)),
			};
			for r in &named {
				for off in r.from()..r.to() {
					let cid = ContentId::new(r.op(), off);
					assert!(
						file.runs().iter().any(|run| run.content.contains(&cid)),
						"trial {}: the confined move {} lost {} out of {}",
						trial, op, cid, file.path_lossy(),
					);
				}
			}
		}
	}
	assert!(planted >= 12, "only {} moves were planted", planted);
	assert!(confinements > 10, "only {} moves were confined", confinements);
	Ok(())
}

/// A cycle with a single move in it, which winner-takes-all cannot arbitrate.
///
/// Whole-cycle arbitration keeps the member highest in op order, so a cycle whose
/// only move *is* that member would keep it and break nothing. Such a move is
/// confined instead, which is what every alternative rule does with it anyway.
///
/// The construction also shows the classifier's known false positive, stated
/// rather than hidden. The move is entirely within a.txt at the time it is made,
/// and it is judged to cross a boundary because the block it moves is by then made
/// of bytes born in two files and it anchors inside the part born in the other
/// one. The price of that reading is a confined move, which is flagged and can be
/// re-issued; the price of not having it is a cycle whose members supersede an
/// earlier move going unseen, and a file emptying itself.
#[test]
fn a_cycle_with_one_move_in_it_confines_that_move() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n")], 1));
	let (a, b) = (ids[0], ids[1]);
	// "xyz" out of b.txt and into the middle of a.txt, which leaves a.txt holding
	// bytes born in two files.
	ops.push(res!(reps[0].move_across(b, 0, 3, a, 1)));
	// The whole of a.txt to an index inside itself, which is a cycle of length one,
	// and the index falls inside the part born in b.txt.
	let mv = res!(reps[0].move_across(a, 0, 7, a, 3));
	let mv_id = mv.0.id();
	ops.push(mv);
	let repo = res!(case("a.txt=\"axyzbc\\n\" b.txt=\"\\n\"", &ops));
	assert!(repo.flags().contains(&Flag::Confined { op: mv_id, home: a, denied: a }),
		"flags were {:?}", repo.flags());
	// Confined, not won: there was nothing for it to win against.
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Won { .. })),
		"a cycle of one move named a winner: {:?}", repo.flags());
	assert_eq!(res!(text(&repo, b)), "\n");
	Ok(())
}

/// This is the association a recorded file field would have asserted on the
/// wire, computed by the render instead. A lazy fetcher wanting one file's
/// operations reads it and caches it beside the log; an index that is wrong can
/// be rebuilt, and a wire field cannot.
#[test]
fn the_derived_index_names_one_file_per_placement() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", LIST), (b"b.txt", b"HEADER\n")], 1));
	let (a, b) = (ids[0], ids[1]);
	let mv = res!(reps[0].move_across(a, 7, 7, b, 7));
	let mv_id = mv.0.id();
	ops.push(mv);
	let ed = res!(reps[0].insert(b, 0, b"X"));
	let ed_id = ed.0.id();
	ops.push(ed);
	let mut seq = Sequence::new();
	for op in &ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let repo = res!(seq.render());
	// The move named content in a.txt and landed in b.txt, which nothing on the
	// wire said and the forest did.
	assert_eq!(repo.file_of(&mv_id), Some(b));
	assert_eq!(repo.file_of(&ed_id), Some(b));
	assert_eq!(repo.file_of(&a), Some(a), "a file's own seed is in it");
	// Every placement is accounted for exactly once.
	assert_eq!(repo.index().len(), 6, "two files, two seeding splices, a move, an edit");
	Ok(())
}

/// A splice anchored to a file's origin anchor lands in that file and no other,
/// which is how an operation says where it goes without saying which file.
#[test]
fn the_origin_anchor_is_what_says_which_file() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"a.txt", b""), (b"b.txt", b"")], 1));
	let (a, b) = (ids[0], ids[1]);
	// Two splices, identical but for the file their origin names.
	ops.push(res!(reps[0].author(Op::Splice {
		left:	Some(Anchor::origin(a)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"into a".to_vec(),
	})));
	ops.push(res!(reps[0].author(Op::Splice {
		left:	Some(Anchor::origin(b)),
		right:	None,
		remove:	Vec::new(),
		insert:	b"into b".to_vec(),
	})));
	let repo = res!(case("a.txt=\"into a\" b.txt=\"into b\"", &ops));
	assert_eq!(res!(text(&repo, a)), "into a");
	assert_eq!(res!(text(&repo, b)), "into b");
	// The origin anchors are content, one byte each, and neither renders.
	assert_eq!(repo.stats().atom_bytes, 2 + 12, "one byte per file, and the two splices");
	assert_eq!(repo.stats().rendered, 12);
	Ok(())
}

/// A file created and never written to renders empty, and its origin anchor is
/// still there to be named.
#[test]
fn an_empty_file_is_not_empty_in_identifier_space() -> Outcome<()> {
	let (_, ops, ids) = res!(stage(&[(b"empty.txt", b"")], 0));
	let mut seq = Sequence::new();
	for op in &ops {
		res!(seq.apply(op.0.clone(), op.1.clone()));
	}
	let repo = res!(seq.render());
	let file = match repo.file(ids[0]) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert!(file.is_empty());
	assert!(file.is_live());
	assert!(file.runs().is_empty());
	// The gap at index zero names the origin anchor, which is the one thing an
	// empty file has to anchor to.
	assert_eq!(res!(file.gap(0)), (Some(Anchor::origin(ids[0])), None));
	// Which is content the set holds, one byte of it, dead.
	assert_eq!(repo.stats().atom_bytes, 1);
	assert_eq!(repo.stats().rendered, 0);
	assert_eq!(
		res!(file.splice(0, 0, b"x".to_vec())).origins().0,
		Some(Anchor::after(ContentId::origin(ids[0]))),
	);
	Ok(())
}


/// This is the claim the whole vocabulary rests on, stated for notes: a note
/// names content, content is repository-wide, and nothing in the note ever
/// mentioned a file.
#[test]
fn a_note_crosses_a_file_boundary_with_its_content() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[
		(b"a.txt", b"alpha beta gamma"),
		(b"b.txt", b"one two"),
	], 1));
	let (a, b) = (ids[0], ids[1]);
	// A note about "beta", then "beta " taken to the end of the other file.
	ops.push(res!(reps[0].note(a, 6, 4, b"beta needs a citation")));
	let repo = res!(converge(&ops));
	match repo.file(a) {
		Some(f)	=> assert_eq!(f.notes()[0].spans(), &[Span::new(6, 4)]),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	ops.push(res!(reps[0].move_across(a, 6, 5, b, 7)));
	let repo = res!(case("a.txt=\"alpha gamma\" b.txt=\"one twobeta \"", &ops));
	// The note is no longer in the file it was written in.
	match repo.file(a) {
		Some(f)	=> assert!(f.notes().is_empty(), "the content left"),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	// It is in the file the content went to, over the bytes it named.
	let dest = match repo.file(b) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(dest.notes().len(), 1);
	assert_eq!(dest.notes()[0].spans(), &[Span::new(7, 4)]);
	assert_eq!(dest.notes()[0].text_lossy(), "beta needs a citation");
	let span = dest.notes()[0].spans()[0];
	assert_eq!(&dest.bytes()[span.at as usize..span.end() as usize], b"beta");
	// And the repository lists it once, in the one file it reaches.
	assert_eq!(repo.notes().len(), 1);
	assert!(!repo.notes()[0].on_dead());
	assert_eq!(repo.notes()[0].files().len(), 1);
	assert_eq!(repo.notes()[0].files()[0].file, b);
	Ok(())
}

/// Each file says where its share of the content is, and the repository says the
/// note is in two places.
#[test]
fn a_note_torn_across_two_files_is_listed_once() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[
		(b"a.txt", b"0123456789"),
		(b"b.txt", b"----"),
	], 1));
	let (a, b) = (ids[0], ids[1]);
	ops.push(res!(reps[0].note(a, 2, 6, b"about 234567")));
	// Half of the noted run goes to the other file.
	ops.push(res!(reps[0].move_across(a, 4, 3, b, 2)));
	let repo = res!(case("a.txt=\"0123789\" b.txt=\"--456--\"", &ops));
	// "23" and "7" are what is left in a, and the move closed the gap between
	// them, so they are one region of the render and one span: a span is a run of
	// rendered bytes, not a run of content.
	match repo.file(a) {
		Some(f)	=> assert_eq!(f.notes()[0].spans(), &[Span::new(2, 3)]),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	// Put something between them, and the note is in two places in one file.
	ops.push(res!(reps[0].insert(a, 4, b"|")));
	let repo = res!(case("a.txt=\"0123|789\" b.txt=\"--456--\"", &ops));
	let left = match repo.file(a) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	let right = match repo.file(b) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	// "23" stayed, "456" left, "7" stayed: two spans in one file, one in the
	// other, and six bytes in all.
	assert_eq!(left.notes().len(), 1);
	assert_eq!(left.notes()[0].spans(), &[Span::new(2, 2), Span::new(5, 1)]);
	assert_eq!(right.notes().len(), 1);
	assert_eq!(right.notes()[0].spans(), &[Span::new(2, 3)]);
	// One note, in two files, over the six bytes it named.
	assert_eq!(repo.notes().len(), 1);
	let note = &repo.notes()[0];
	assert!(!note.on_dead());
	assert_eq!(note.files().len(), 2);
	assert_eq!(note.files()[0].file, a);
	assert_eq!(note.files()[1].file, b);
	let total: u64 = note.files().iter()
		.flat_map(|p| p.spans.iter())
		.map(|s| s.len)
		.sum();
	assert_eq!(total, 6, "no byte of the noted run went missing");
	assert_eq!(note.spans_in(b), &[Span::new(2, 3)]);
	assert!(note.spans_in(OpId::default()).is_empty());
	Ok(())
}

/// The bytes still render, into a file no reader looks at, and that is what the
/// flag beside it says.
#[test]
fn a_note_in_a_deleted_file_is_not_a_note_on_dead_content() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[
		(b"a.txt", b"keep this line"),
		(b"b.txt", b""),
	], 1));
	let (a, b) = (ids[0], ids[1]);
	ops.push(res!(reps[0].note(a, 5, 4, b"about this")));
	ops.push(res!(reps[0].move_across(a, 5, 4, b, 0)));
	ops.push(res!(reps[0].remove(b)));
	let repo = res!(converge(&ops));
	// The deleted file still holds the bytes, and still resolves the note.
	let gone = match repo.file(b) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert!(!gone.is_live());
	assert_eq!(gone.text_lossy(), "this");
	assert_eq!(gone.notes().len(), 1);
	assert_eq!(gone.notes()[0].spans(), &[Span::new(0, 4)]);
	// So the note is placed, not dead, and the repository says which file.
	assert_eq!(repo.notes().len(), 1);
	assert!(!repo.notes()[0].on_dead());
	assert!(repo.dead_notes().is_empty());
	assert_eq!(repo.notes()[0].files()[0].file, b);
	Ok(())
}

/// A note names content, and content the operation set does not hold is a hole
/// in the history rather than a note on nothing.
#[test]
fn a_note_on_content_the_set_lacks_is_refused() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"a.txt", b"alpha beta")], 1));
	let note = res!(reps[0].note(ids[0], 0, 5, b"about alpha"));
	// The whole set renders; the set without the splice the note names does not.
	ops.push(note.clone());
	res!(converge(&ops));
	let mut seq = Sequence::new();
	res!(seq.apply(ops[0].0.clone(), ops[0].1.clone()));
	res!(seq.apply(note.0.clone(), note.1.clone()));
	assert!(seq.render().is_err(),
		"a note whose subject has not arrived cannot be told from a note on \
		content that has died");
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SILENT-DELETION CASES                                                 │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The self-hosting trial's two silent rounds, reproduced exactly: work that is
// durable in the log and invisible in the converged tree must be flagged, and a
// deletion causally ordered with the work -- supersession -- must not be. The
// rendered bytes in every case here are what the engine rendered before the
// flags existed; the flags are reportage, not placement.

/// The trial's round four. Replica one records a cross-file block move the way
/// a diff-based capture does -- a deletion in one file and a fresh insertion in
/// another, with no move operation -- while replica two concurrently edits
/// inside the block. The edit's anchors die with the block, its bytes strand at
/// the deletion site, and the flag names both operations.
#[test]
fn an_edit_inside_a_captured_move_strands_and_is_flagged() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	// The captured "move": splice the block out of a.txt, splice its bytes into
	// b.txt, and say nothing about the two being one intent.
	let del = res!(r1[0].delete(a, 5, 5));
	let del_id = del.0.id();
	ops.push(del);
	ops.push(res!(r1[0].insert(b, 0, b"move\n")));
	// The concurrent edit inside the block.
	let edit = res!(r2[0].insert(a, 7, b"XY"));
	let edit_id = edit.0.id();
	ops.push(edit);
	// The block lands unedited, and the edit strands where the block was.
	let repo = res!(case("a.txt=\"keep\\nXY\" b.txt=\"move\\n\"", &ops));
	assert!(repo.flags().contains(&Flag::Stranded { op: edit_id, by: del_id }),
		"flags were {:?}", repo.flags());
	// The file the sliver renders in keeps the flag, so a reader of a.txt is
	// told without asking the repository.
	let stranded_in_a = match repo.file(a) {
		Some(f)	=> f.flags().contains(&Flag::Stranded { op: edit_id, by: del_id }),
		None	=> false,
	};
	assert!(stranded_in_a, "a.txt did not keep the flag");
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::SplicedIntoDeleted { .. })),
		"no file was deleted: {:?}", repo.flags());
	Ok(())
}

/// The trial's round five. Replica one deletes a file; replica two concurrently
/// edits inside it. The file is gone from both trees, the edit renders only
/// into the deleted file, and the flag names the edit, the file and the
/// deletion.
#[test]
fn an_edit_inside_a_concurrently_deleted_file_is_flagged() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"fn main() {}\n")], 2));
	let a = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	let del = res!(r1[0].remove(a));
	let del_id = del.0.id();
	ops.push(del);
	let edit = res!(r2[0].insert(a, 11, b" ok"));
	let edit_id = edit.0.id();
	ops.push(edit);
	// No live files, and the edit whole inside the withheld render.
	let repo = res!(case("", &ops));
	assert_eq!(res!(text(&repo, a)), "fn main() { ok}\n");
	assert_eq!(repo.stats().withheld, 16);
	assert!(repo.flags().contains(
		&Flag::SplicedIntoDeleted { op: edit_id, file: a, del: del_id }),
		"flags were {:?}", repo.flags());
	// The deleted file keeps the flag, so a recovery verb has it to hand.
	let kept = match repo.file(a) {
		Some(f)	=> f.flags().contains(
			&Flag::SplicedIntoDeleted { op: edit_id, file: a, del: del_id }),
		None	=> false,
	};
	assert!(kept, "a.txt did not keep the flag");
	// The file's own deletion killed no anchor, so nothing is stranded.
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Stranded { .. })),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// The region-level supersession control: the same shape as the stranded case,
/// but the deletion was written in knowledge of the edit. A deleter who could
/// see the insertion chose to remove the region around it, and is owed no flag
/// for a race that did not happen.
#[test]
fn a_deletion_that_saw_the_edit_supersedes_and_raises_nothing() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n")], 2));
	let a = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	let edit = res!(r2[0].insert(a, 7, b"XY"));
	res!(r1[0].recv(edit.clone()));
	ops.push(edit);
	// Replica one deletes the whole block, the insertion included, seeing it.
	ops.push(res!(r1[0].delete(a, 5, 7)));
	let repo = res!(case("a.txt=\"keep\\n\"", &ops));
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::Stranded { .. })),
		"an informed deletion was flagged as a race: {:?}", repo.flags());
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::SplicedIntoDeleted { .. })),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// The file-level supersession controls, both ways round. A deleter who had
/// seen the edit deleted it on purpose; an editor who had seen the deletion
/// wrote into a file they knew was dead. Neither is a race, and neither flags.
#[test]
fn a_file_deletion_ordered_with_the_edit_raises_nothing() -> Outcome<()> {
	// The deletion after the edit, seeing it.
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"fn main() {}\n")], 2));
	let a = ids[0];
	{
		let (r1, r2) = reps.split_at_mut(1);
		let edit = res!(r2[0].insert(a, 11, b" ok"));
		res!(r1[0].recv(edit.clone()));
		ops.push(edit);
		ops.push(res!(r1[0].remove(a)));
	}
	let repo = res!(case("", &ops));
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::SplicedIntoDeleted { .. })),
		"an informed deletion was flagged as a race: {:?}", repo.flags());

	// The edit after the deletion, seeing it.
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"fn main() {}\n")], 2));
	let a = ids[0];
	{
		let (r1, r2) = reps.split_at_mut(1);
		let del = res!(r1[0].remove(a));
		res!(r2[0].recv(del.clone()));
		ops.push(del);
		ops.push(res!(r2[0].insert(a, 11, b" ok")));
	}
	let repo = res!(case("", &ops));
	assert!(!repo.flags().iter().any(|f| matches!(f, Flag::SplicedIntoDeleted { .. })),
		"an informed edit was flagged as a race: {:?}", repo.flags());
	Ok(())
}

/// The same collision with the move recorded as a move: the edit follows the
/// block into the other file, nothing strands, and neither of the deletion
/// flags has anything to say. This is the round the trial says the engine was
/// built for, and the new flags must leave it alone.
#[test]
fn a_recorded_move_carries_the_edit_and_the_new_flags_stay_quiet() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].move_across(a, 5, 5, b, 0)));
	ops.push(res!(r2[0].insert(a, 7, b"XY")));
	let repo = res!(case("a.txt=\"keep\\n\" b.txt=\"moXYve\\n\"", &ops));
	assert!(!repo.flags().iter().any(|f| matches!(f,
		Flag::Stranded { .. } | Flag::SplicedIntoDeleted { .. })),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// A move into a concurrently deleted file is [`Flag::MovedIntoDeleted`]'s
/// territory, and stays so: the new flags concern splices, and the file's own
/// content ops, all causally before the deletion, raise nothing either.
#[test]
fn moved_into_deleted_keeps_its_territory() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"keep\nmove\n"), (b"b.txt", b"B\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	let mv = res!(r1[0].move_across(a, 5, 5, b, 0));
	let mv_id = mv.0.id();
	ops.push(mv);
	ops.push(res!(r2[0].remove(b)));
	let repo = res!(case("a.txt=\"keep\\n\"", &ops));
	assert!(repo.flags().contains(&Flag::MovedIntoDeleted { op: mv_id, file: b }),
		"flags were {:?}", repo.flags());
	// b.txt's own seed content went dark too, but its author's edit was seen by
	// the deleter: supersession, not a race, and not a flag.
	assert!(!repo.flags().iter().any(|f| matches!(f,
		Flag::Stranded { .. } | Flag::SplicedIntoDeleted { .. })),
		"flags were {:?}", repo.flags());
	Ok(())
}

/// A mode is asserted of a file's identity, so it survives everything that
/// happens to the file's path and to its bytes.
///
/// This is the whole argument for [`Op::FileMode`] being shaped like a rename
/// rather than being a field on one. The script is made executable once; it is
/// then renamed, edited, and edited again by another replica, and it is still
/// executable, because none of those operations said anything about what it is.
#[test]
fn a_mode_survives_a_rename_and_an_edit() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"build.sh", b"#!/bin/sh\n")], 2));
	let a = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	ops.push(res!(r1[0].set_mode(a, Mode::Executable)));
	ops.push(res!(r1[0].rename(a, b"tools/build.sh")));
	ops.push(res!(r2[0].insert(a, 10, b"set -e\n")));
	let repo = res!(case(
		"tools/build.sh[executable]=\"#!/bin/sh\\nset -e\\n\"", &ops));
	let f = match repo.file(a) {
		Some(f)	=> f,
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	};
	assert_eq!(f.mode(), Mode::Executable);
	Ok(())
}

#[test]
fn a_file_nobody_named_is_normal() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"a.txt", b"hello\n")], 1));
	let a = ids[0];
	ops.push(res!(reps[0].insert(a, 5, b" there")));
	let repo = res!(case("a.txt=\"hello there\\n\"", &ops));
	match repo.file(a) {
		Some(f)	=> assert_eq!(f.mode(), Mode::Normal),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	Ok(())
}

/// Two replicas saying different things about one file at once settle the way
/// two concurrent renames settle: by operation order, the same answer whatever
/// order the operations arrive in.
#[test]
fn concurrent_modes_settle_by_op_order() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(&[(b"s.sh", b"x\n")], 2));
	let a = ids[0];
	let (r1, r2) = reps.split_at_mut(1);
	let one = res!(r1[0].set_mode(a, Mode::Executable));
	let two = res!(r2[0].set_mode(a, Mode::Symlink));
	// Neither saw the other, and the later in op order is the one that stands.
	let later = if OpOrder::of(&one.0.id()) > OpOrder::of(&two.0.id()) {
		Mode::Executable
	} else {
		Mode::Symlink
	};
	ops.push(one);
	ops.push(two);
	let repo = res!(converge(&ops));
	match repo.file(a) {
		Some(f)	=> assert_eq!(f.mode(), later, "the later assertion stands"),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	// Setting a mode back is an ordinary later assertion, not a special case.
	ops.push(res!(reps[0].set_mode(a, Mode::Normal)));
	let repo = res!(converge(&ops));
	match repo.file(a) {
		Some(f)	=> assert_eq!(f.mode(), Mode::Normal),
		None	=> return Err(err!("The file went missing."; Test, Missing)),
	}
	Ok(())
}

/// The message says the set is not causally complete.
///
/// The same refusal a rename and a delete get, for the same reason: the render
/// cannot say what it does not hold.
#[test]
fn a_mode_of_an_absent_file_is_refused() -> Outcome<()> {
	let mut seq = Sequence::new();
	let ghost = OpId::new(ReplicaId::new(9), 1);
	res!(seq.apply(
		Header::root(OpId::new(ReplicaId::new(1), 1)),
		Op::FileMode { file: ghost, mode: Mode::Executable },
	));
	let e = match seq.render() {
		Ok(_) => return Err(err!("A mode of an absent file rendered."; Test)),
		Err(e) => e,
	};
	let msg = fmt!("{}", e);
	assert!(msg.contains("causally complete"), "message was {}", msg);
	assert!(msg.contains(&fmt!("{}", ghost)), "message was {}", msg);
	Ok(())
}

/// The point of carrying it there: a checkout reads a snapshot instead of the
/// log.
#[test]
fn a_mode_rides_the_snapshot() -> Outcome<()> {
	use crate::snapshot::{
		FileState,
		Snapshot,
	};
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"run.sh", b"#!/bin/sh\n"), (b"link", b"run.sh"), (b"plain.txt", b"hi\n")],
		1,
	));
	ops.push(res!(reps[0].set_mode(ids[0], Mode::Executable)));
	ops.push(res!(reps[0].set_mode(ids[1], Mode::Symlink)));
	let repo = res!(converge(&ops));
	let states: Vec<FileState> = repo.live().into_iter().map(FileState::of).collect();
	let frontier: Vec<OpId> = ops.iter().map(|o| o.0.id()).collect();
	let snap = res!(Snapshot::new(frontier, states));
	let back = res!(Snapshot::decode(&res!(snap.encode())));
	assert_eq!(back, snap);
	let modes: Vec<(String, Mode)> = back.files()
		.iter()
		.map(|f| (f.path_lossy(), f.mode))
		.collect();
	assert!(modes.contains(&(fmt!("run.sh"), Mode::Executable)), "modes were {:?}", modes);
	assert!(modes.contains(&(fmt!("link"), Mode::Symlink)), "modes were {:?}", modes);
	assert!(modes.contains(&(fmt!("plain.txt"), Mode::Normal)), "modes were {:?}", modes);
	Ok(())
}
