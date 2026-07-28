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
};
use crate::seq::render::{
	Flag,
	Rendered,
	Repo,
};
use crate::seq::{
	OpOrder,
	Sequence,
};

use oxedyne_fe2o3_core::prelude::*;


/// The shopping list of the published worked case.
const LIST: &[u8] = b"- Eggs\n- Milk\n- Cheese\n";


/// One replica of a repository of several files: the frontend that turns
/// index-based editing intent, in a named file, into content-anchored
/// operations.
///
/// Nothing here distinguishes a move within a file from a move between two,
/// except which render the destination gap is read from. That is the claim the
/// whole exercise was to test, stated as code.
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
	fn recv(&mut self, op: (Header, Op))
		-> Outcome<()>
	{
		self.seq.apply(op.0, op.1)
	}

	/// Renders the replica's view of one file.
	fn view(&self, file: OpId)
		-> Outcome<Rendered>
	{
		let repo = res!(self.seq.render());
		match repo.file(file) {
			Some(f)	=> Ok(f.clone()),
			None	=> Err(err!("The replica has no file {}.", file; Test, Missing)),
		}
	}

	/// Records an operation of this replica's own, and applies it.
	fn author(&mut self, op: Op)
		-> Outcome<(Header, Op)>
	{
		let head = res!(self.next_head());
		res!(self.seq.apply(head.clone(), op.clone()));
		Ok((head, op))
	}

	/// Creates a file, returning the operation and the file's identity.
	fn create(&mut self, path: &[u8])
		-> Outcome<((Header, Op), OpId)>
	{
		let made = res!(self.author(Op::FileCreate { path: path.to_vec() }));
		let id = made.0.id();
		Ok((made, id))
	}

	/// Renames a file.
	fn rename(&mut self, file: OpId, path: &[u8])
		-> Outcome<(Header, Op)>
	{
		self.author(Op::FileRename { file, path: path.to_vec() })
	}

	/// Deletes a file.
	fn remove(&mut self, file: OpId)
		-> Outcome<(Header, Op)>
	{
		self.author(Op::FileDelete { file })
	}

	/// Inserts bytes at an index of a file.
	fn insert(&mut self, file: OpId, at: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, 0, bytes.to_vec()));
		self.author(op)
	}

	/// Deletes a run at an index of a file.
	fn delete(&mut self, file: OpId, at: usize, len: usize)
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, len, Vec::new()));
		self.author(op)
	}

	/// Replaces a run at an index of a file, in one splice.
	fn replace(&mut self, file: OpId, at: usize, len: usize, bytes: &[u8])
		-> Outcome<(Header, Op)>
	{
		let op = res!(res!(self.view(file)).splice(at, len, bytes.to_vec()));
		self.author(op)
	}

	/// Moves a run out of one file and into another, at an index of each.
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

/// A one-line listing of the live files, as a working copy would show them.
fn listing(repo: &Repo) -> String {
	let mut files: Vec<&Rendered> = repo.files().iter().filter(|f| f.is_live()).collect();
	files.sort_by_key(|f| (f.path().to_vec(), OpOrder::of(&f.file())));
	let mut s = String::new();
	for f in files {
		s.push_str(&fmt!("{}={:?} ", f.path_lossy(), f.text_lossy()));
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

/// A cross-file move racing an in-file insertion at the same destination gap.
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

/// Two branches independently create the same path and splice into it.
///
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
/// The cycle rule breaks it and nothing is lost, but the shape of the outcome is
/// not the in-file one. The demoted move lands in the *other file*, the move that
/// follows it lands there too, and one file is emptied into the other. A user who
/// sees a file go from four bytes to none will not read that as a stale anchor,
/// which is why the flag names both files. Confining a cross-file cycle instead
/// is design work owed.
#[test]
fn a_cross_file_cycle_empties_one_file_into_the_other() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n")], 2));
	let (a, b) = (ids[0], ids[1]);
	let (r1, r2) = reps.split_at_mut(1);
	// Replica 1 moves the whole of a.txt into b.txt, after its first byte.
	ops.push(res!(r1[0].move_across(a, 0, 4, b, 1)));
	// Replica 2 concurrently moves the whole of b.txt into a.txt, after its
	// first byte.
	ops.push(res!(r2[0].move_across(b, 0, 4, a, 1)));
	let repo = res!(case("a.txt=\"\" b.txt=\"axyz\\nbc\\n\"", &ops));
	assert_eq!(res!(text(&repo, a)), "");
	// Breaking the cycle carried a.txt's content into b.txt, and the flag says
	// so in those terms: the file it was written into, and the file it renders in.
	let crossed: Vec<&Flag> = repo.flags().iter()
		.filter(|f| matches!(f, Flag::CrossedFile { .. }))
		.collect();
	assert!(!crossed.is_empty(), "flags were {:?}", repo.flags());
	for flag in &crossed {
		match flag {
			Flag::CrossedFile { from, to, .. } => {
				assert_eq!(*from, a, "the content was written into a.txt");
				assert_eq!(*to, b, "and it renders in b.txt");
			},
			other => return Err(err!(
				"Expected a CrossedFile, got {}.", other.name(); Test, Mismatch)),
		}
	}
	// Both files are told, since the flag is about the pair of them.
	for file in [a, b] {
		match repo.file(file) {
			Some(f) => assert!(
				f.flags().iter().any(|g| matches!(g, Flag::CrossedFile { .. })),
				"the file {} was not told", file),
			None => return Err(err!("A file went missing."; Test, Missing)),
		}
	}
	Ok(())
}

/// A move out of a file racing that file's deletion. What left survives.
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

/// An insertion into an empty file racing that file's rename.
///
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
#[test]
fn a_three_file_cycle_collapses_into_one_file() -> Outcome<()> {
	let (mut reps, mut ops, ids) = res!(stage(
		&[(b"a.txt", b"abc\n"), (b"b.txt", b"xyz\n"), (b"c.txt", b"123\n")],
		3,
	));
	let (a, b, c) = (ids[0], ids[1], ids[2]);
	ops.push(res!(reps[0].move_across(a, 0, 4, b, 1)));
	ops.push(res!(reps[1].move_across(b, 0, 4, c, 1)));
	ops.push(res!(reps[2].move_across(c, 0, 4, a, 1)));
	let repo = res!(case(
		"a.txt=\"\" b.txt=\"a1xyz\\n23\\nbc\\n\" c.txt=\"\"",
		&ops,
	));
	assert_eq!(res!(text(&repo, a)), "");
	assert_eq!(res!(text(&repo, c)), "");
	assert_eq!(res!(text(&repo, b)).len(), 12, "every byte survives the cycle");
	Ok(())
}

/// Content moved into a file that is concurrently deleted.
///
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

/// Three replicas editing, moving within files and moving between them at
/// random, agree on every file and conserve every byte the repository holds.
///
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

/// Every operation that placed anything landed in exactly one file, and the
/// derived index says which.
///
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
