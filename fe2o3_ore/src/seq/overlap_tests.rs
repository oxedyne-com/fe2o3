//! The overlap-arbitration cases: what a file holds when two people rewrite one
//! region at once.
//!
//! Every case here is transcribed from the planted sweep that settled the rule,
//! and every expectation is that sweep's own render under the rule this crate now
//! implements: the connected components of the overlap graph are the arbitration
//! groups, components contended by the same replicas over the same files are one
//! group, the op-order maximum prevails, and every member concurrent with it
//! yields. The sweep's replicas were numbered from two because its first replica
//! typed the base text; here replica zero creates the file and writes it, so each
//! of the sweep's authors is one lower.
//!
//! Three properties are asserted throughout, because they are what the rule was
//! adopted for. The contended region holds **whole hunks and never an interleave**
//! -- which is not the same as holding one author's work, and the case that shows
//! the difference is here. Nothing is lost: a yielded insertion is dead, not
//! homeless, and the conservation check runs inside every delivery order of every
//! case. And both sides are told, by one flag that names the group and the
//! operation that prevailed rather than a pair that may never have met.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
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
use crate::seq::Sequence;

use oxedyne_fe2o3_core::prelude::*;


// The trial's own `src/util.rs`, which is round 3's base.
const PARSE: &str = "\
/// Parses a decimal string, treating anything malformed as zero.
pub fn parse_or_zero(s: &str) -> i64 {
\tmatch s.trim().parse::<i64>() {
\t\tOk(v) => v,
\t\tErr(_) => 0,
\t}
}
";

// Two functions in one file, which is what a two-component collision needs.
const TWO: &str = "\
pub fn parse_or_zero(s: &str) -> i64 {
\tmatch s.trim().parse::<i64>() {
\t\tOk(v) => v,
\t\tErr(_) => 0,
\t}
}

pub fn twice(n: i64) -> i64 {
\tlet m = n * 2;
\tm
}
";

// A paragraph, for the containment, chain and partial-sync cases.
const PROSE: &str = "The renderer places every run against the anchors it was \
written at, and the result is convergent, conserved and attributed. It is not a \
text.\n";

// The body of the parser, which several cases replace whole.
const BODY: &str = "\tmatch s.trim().parse::<i64>() {\n\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n";

// What the parser reads as once one author has had it.
const REWRITTEN: &str = "\
/// Parses a decimal string, treating anything malformed as zero.
pub fn parse_or_zero(s: &str) -> i64 {
\tlet cleaned = s.trim();
\tcleaned.parse::<i64>().unwrap_or(0)
}
";


/// One replica of a repository holding one file: the frontend that turns editing
/// intent into content-anchored operations, which is what an editor would be.
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
			None	=> Err(err!("The replica has no file {}.", self.file; Test, Missing)),
		}
	}

	fn author(&mut self, op: Op)
		-> Outcome<(Header, Op)>
	{
		let head = res!(self.next_head());
		res!(self.seq.apply(head.clone(), op.clone()));
		Ok((head, op))
	}

	/// The offset of the first occurrence of some text in this replica's view.
	fn at(&self, find: &str)
		-> Outcome<usize>
	{
		let text = res!(self.view()).text_lossy();
		match text.find(find) {
			Some(i)	=> Ok(i),
			None	=> Err(err!(
				"The replica's view does not hold {:?}.", find; Test, Missing)),
		}
	}

	/// One splice, which is the shape a capture emits for one hunk.
	fn rep(&mut self, find: &str, with: &str)
		-> Outcome<(Header, Op)>
	{
		let at = res!(self.at(find));
		let op = res!(res!(self.view()).splice(at, find.len(), with.as_bytes().to_vec()));
		self.author(op)
	}

	fn del(&mut self, find: &str)
		-> Outcome<(Header, Op)>
	{
		let at = res!(self.at(find));
		let op = res!(res!(self.view()).splice(at, find.len(), Vec::new()));
		self.author(op)
	}

	/// Immediately after the first occurrence.
	fn ins(&mut self, find: &str, with: &str)
		-> Outcome<(Header, Op)>
	{
		let at = res!(self.at(find)) + find.len();
		let op = res!(res!(self.view()).splice(at, 0, with.as_bytes().to_vec()));
		self.author(op)
	}
}


/// Creates one file, writes `text` into it on replica zero, and hands out `n`
/// replicas that have seen both operations.
fn seed(text: &str, n: u64)
	-> Outcome<(Vec<Replica>, Vec<(Header, Op)>, OpId)>
{
	let mut origin = Replica::new(0, OpId::default());
	let create = res!(origin.author(Op::FileCreate { path: b"util.rs".to_vec() }));
	let file = create.0.id();
	origin.file = file;
	let mut ops = vec![create];
	ops.push(res!(origin.rep("", text)));
	let mut reps: Vec<Replica> = Vec::new();
	for i in 1..=n {
		let mut r = Replica::new(i, file);
		for op in &ops {
			res!(r.recv(op.clone()));
		}
		reps.push(r);
	}
	Ok((reps, ops, file))
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

/// A one-line listing of the live files.
fn listing(repo: &Repo) -> String {
	let mut s = String::new();
	for f in repo.files().iter().filter(|f| f.is_live()) {
		s.push_str(&fmt!("{}={:?} ", f.path_lossy(), f.text_lossy()));
	}
	s.trim_end().to_string()
}

/// Applies an operation set in every delivery order where the set is small
/// enough, or in every rotation, the reverse and a spread of shuffles where it is
/// not, and requires that all of them render the same bytes and raise the same
/// flags.
///
/// Conservation is checked on every order, which is the whole of what says a
/// yielded insertion is dead rather than lost.
fn converge(ops: &[(Header, Op)])
	-> Outcome<Repo>
{
	let n = ops.len();
	let mut orders: Vec<Vec<usize>> = Vec::new();
	if n <= 7 {
		let mut idx: Vec<usize> = (0..n).collect();
		permute(&mut idx, 0, &mut orders);
	} else {
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
		for _ in 0..60 {
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

/// Checks the file's render against the answer the case prescribes, under every
/// delivery order.
fn case(file: OpId, expect: &str, ops: &[(Header, Op)])
	-> Outcome<Repo>
{
	let repo = res!(converge(ops));
	let got = match repo.file(file) {
		Some(f)	=> f.clone(),
		None	=> return Err(err!("The render holds no file {}.", file; Test, Missing)),
	};
	assert_eq!(got.text_lossy(), expect);
	Ok(repo)
}

fn id(replica: u64, counter: u64) -> OpId {
	OpId::new(ReplicaId::new(replica), counter)
}

/// Every yield the render decided, as `(yielder, prevailed, group, through)`.
fn yields(repo: &Repo) -> Vec<(OpId, OpId, Vec<OpId>, Option<OpId>)> {
	repo.flags().iter()
		.filter_map(|f| match f {
			Flag::Yielded { op, to, group, through }
				=> Some((*op, *to, group.clone(), *through)),
			_	=> None,
		})
		.collect()
}

fn yielded_to(repo: &Repo, op: OpId) -> Option<OpId> {
	yields(repo).into_iter().find(|(o, ..)| *o == op).map(|(_, to, ..)| to)
}

/// Whether the two operations were flagged as having named the same content.
fn overlapped(repo: &Repo, a: OpId, b: OpId) -> bool {
	repo.flags().iter().any(|f| match f {
		Flag::Overlap { ops, .. }	=> ops.contains(&a) && ops.contains(&b),
		_							=> false,
	})
}

fn count(repo: &Repo, kind: fn(&Flag) -> bool) -> usize {
	repo.flags().iter().filter(|f| kind(f)).count()
}


/// Round 3 of the self-hosting trial: two authors rewrite one function body,
/// each capture emitting two hunks.
///
/// The status quo renders the surviving fragments of the base interleaved with
/// two authors' insertions, which compiles for nobody. Under arbitration the two
/// hunks of each author form one component -- each author's second hunk is
/// causally after their first, so neither author's pair is a race with itself --
/// the op-order maximum prevails, and the region reads as whole hunks.
#[test]
fn two_authors_rewriting_one_body_leave_one_authors_function() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PARSE, 2));
	ops.push(res!(reps[0].rep(
		"\tmatch s.trim().parse::<i64>() {\n",
		"\ts.trim().parse::<i64>().unwrap_or(0)\n")));
	ops.push(res!(reps[0].del("\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n")));
	ops.push(res!(reps[1].rep(
		"\tmatch s.trim().parse::<i64>() {\n",
		"\tlet cleaned = s.trim();\n\tcleaned.parse::<i64>().unwrap_or(0)\n")));
	ops.push(res!(reps[1].del("\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n")));

	let repo = res!(case(file, REWRITTEN, &ops));
	// Replica 2 is the group's maximum on the replica tie-break, both authors
	// having minted the same counters, so replica 1's two hunks yield.
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(2, 4)));
	assert_eq!(yielded_to(&repo, id(1, 4)), Some(id(2, 4)));
	assert_eq!(yielded_to(&repo, id(2, 3)), None);
	assert_eq!(yielded_to(&repo, id(2, 4)), None);
	// The raw fact stays beneath the arbitration.
	assert!(count(&repo, |f| matches!(f, Flag::Overlap { .. })) > 0);
	Ok(())
}

/// Round 3 again, planted as the minimal hunks a real diff emits, so that the
/// base's common substrings stay alive between the two authors' fragments.
///
/// This is the faithful shape of the trial's defect: six provenance runs of
/// nobody's function. The rule is the same and so is the answer.
#[test]
fn the_minimal_hunks_of_round_three_leave_one_authors_function() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PARSE, 2));
	// Replica 1, towards `s.trim().parse::<i64>().unwrap_or(0)`.
	ops.push(res!(reps[0].del("match ")));
	ops.push(res!(reps[0].rep(") {\n", ").unwrap_or(0)\n")));
	ops.push(res!(reps[0].del("\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n")));
	// Replica 2, towards the `let cleaned = ...` form.
	ops.push(res!(reps[1].rep("match ", "let cleaned = ")));
	ops.push(res!(reps[1].ins("s.trim()", ";\n\tcleaned")));
	ops.push(res!(reps[1].rep(") {\n", ").unwrap_or(0)\n")));
	ops.push(res!(reps[1].del("\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n")));

	let repo = res!(case(file, REWRITTEN, &ops));
	assert!(!yields(&repo).is_empty());
	// Every rendered byte of the body is replica 2's or the base's: no fragment of
	// replica 1 survives between them, which is what "whole hunks" means.
	for (op, ..) in yields(&repo) {
		assert_eq!(op.replica, ReplicaId::new(1));
	}
	Ok(())
}

/// Two authors rewrite the same two functions, in opposite order, so that the two
/// components have different op-order maxima.
///
/// Separate components would leave one author's doubler beside the other's
/// parser: two whole hunks by different people, which is the known weakness of
/// the component rule and is what the same-contenders merge exists to remove. The
/// two components are contended by the same pair of replicas over the same file,
/// so they are one group; its maximum is replica 2's parser, and replica 2's
/// doubler survives on the causal exemption.
#[test]
fn two_functions_rewritten_in_opposite_order_read_as_one_author() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(TWO, 2));
	let parser = "\tmatch s.trim().parse::<i64>() {\n\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n";
	let doubler = "\tlet m = n * 2;\n\tm\n";
	// Replica 1 does the parser first, then the doubler.
	ops.push(res!(reps[0].rep(parser, "\ts.trim().parse::<i64>().unwrap_or(0)\n")));
	ops.push(res!(reps[0].rep(doubler, "\tn * 2\n")));
	// Replica 2 does the doubler first, then the parser.
	ops.push(res!(reps[1].rep(doubler, "\tn + n\n")));
	ops.push(res!(reps[1].rep(parser,
		"\tlet cleaned = s.trim();\n\tcleaned.parse::<i64>().unwrap_or(0)\n")));

	let repo = res!(case(file, "\
pub fn parse_or_zero(s: &str) -> i64 {
\tlet cleaned = s.trim();
\tcleaned.parse::<i64>().unwrap_or(0)
}

pub fn twice(n: i64) -> i64 {
\tn + n
}
", &ops));
	// One group of four, whose maximum is replica 2's parser hunk.
	let ys = yields(&repo);
	assert_eq!(ys.len(), 2);
	for (_, to, group, through) in &ys {
		assert_eq!(*to, id(2, 4));
		assert_eq!(*group, vec![id(1, 3), id(2, 3), id(1, 4), id(2, 4)]);
		assert_eq!(*through, None);
	}
	// Replica 2's own doubler is in the winner's causal past, so the exemption
	// keeps it and the file reads as one author throughout.
	assert_eq!(yielded_to(&repo, id(2, 3)), None);
	Ok(())
}

/// Three authors in a chain: the first overlaps the second, the second the third,
/// and the first and the third are disjoint.
///
/// A component is a component-wide decision, so the first yields to the third --
/// an operation it never named a byte of. The flag has to say that the *group*
/// prevailed and name its maximum, because "this operation rewrote your region"
/// is simply false here, and the sweep found it false of roughly three yields in
/// ten.
#[test]
fn a_chain_yields_to_a_group_maximum_it_never_met() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PROSE, 3));
	ops.push(res!(reps[0].rep("renderer places every run", "engine puts each run")));
	ops.push(res!(reps[1].rep("every run against the anchors",
		"each run where its anchors say")));
	ops.push(res!(reps[2].rep("against the anchors it was written at",
		"at the anchors it was authored against")));

	let repo = res!(case(file, "The renderer places every run at the anchors it was \
authored against, and the result is convergent, conserved and attributed. It is \
not a text.\n", &ops));
	// All three are one component; the maximum is replica 3.
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(3, 3)));
	assert_eq!(yielded_to(&repo, id(2, 3)), Some(id(3, 3)));
	assert_eq!(yielded_to(&repo, id(3, 3)), None);
	// And replica 1 yielded to an operation it never overlapped.
	assert!(!overlapped(&repo, id(1, 3), id(3, 3)));
	assert!(overlapped(&repo, id(1, 3), id(2, 3)));
	Ok(())
}

/// Three parties over two functions, both contended by all three.
///
/// This is the merge's honest loss, and it is worth having in the suite for that
/// reason. Under separate components replica 2 would take the parser and replica
/// 3 the doubler, and the file would read as two authors; merged, replica 3 takes
/// both, and replica 2 -- which had won a collision outright -- shows nothing.
/// What the merge does not do is void the work of somebody who was not
/// contending: every merged component has the same contender set by construction.
#[test]
fn three_parties_over_two_functions_read_as_one_author() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(TWO, 3));
	let parser = "\tmatch s.trim().parse::<i64>() {\n\t\tOk(v) => v,\n\t\tErr(_) => 0,\n\t}\n";
	let doubler = "\tlet m = n * 2;\n\tm\n";
	// Replica 1: parser then doubler.
	ops.push(res!(reps[0].rep(parser, "\ts.trim().parse::<i64>().unwrap_or(0)\n")));
	ops.push(res!(reps[0].rep(doubler, "\tn * 2\n")));
	// Replica 2: doubler then parser.
	ops.push(res!(reps[1].rep(doubler, "\tn + n\n")));
	ops.push(res!(reps[1].rep(parser,
		"\tlet cleaned = s.trim();\n\tcleaned.parse::<i64>().unwrap_or(0)\n")));
	// Replica 3: parser then doubler.
	ops.push(res!(reps[2].rep(parser, "\ts.trim().parse().unwrap_or_default()\n")));
	ops.push(res!(reps[2].rep(doubler, "\tn.saturating_mul(2)\n")));

	let repo = res!(case(file, "\
pub fn parse_or_zero(s: &str) -> i64 {
\ts.trim().parse().unwrap_or_default()
}

pub fn twice(n: i64) -> i64 {
\tn.saturating_mul(2)
}
", &ops));
	// Four yields, all to replica 3's doubler hunk, which is the merged group's
	// op-order maximum; replica 3's own parser hunk is exempt.
	let ys = yields(&repo);
	assert_eq!(ys.len(), 4);
	for (op, to, ..) in &ys {
		assert_eq!(*to, id(3, 4));
		assert!(op.replica != ReplicaId::new(3));
	}
	assert_eq!(yielded_to(&repo, id(3, 3)), None);
	Ok(())
}

/// Two concurrent pure deletions over overlapping text.
///
/// The yielding operation's removals do not bury, so the text only the loser
/// deleted comes back and the arbitrating render is **larger** than the
/// unarbitrated one. Yielding is not only subtractive, and a reader of the rule
/// who expects it only ever to take things off the disk is wrong.
#[test]
fn two_concurrent_deletions_render_more_than_the_status_quo_would() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PROSE, 2));
	ops.push(res!(reps[0].del("places every run against the anchors it was")));
	ops.push(res!(reps[1].del("against the anchors it was written at, and")));

	let want = "The renderer places every run  the result is convergent, conserved \
and attributed. It is not a text.\n";
	let repo = res!(case(file, want, &ops));
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(2, 3)));
	// The union of the two deletions is 101 bytes shorter than the base; only the
	// prevailing one buries, so the render keeps 17 bytes the status quo removed.
	assert!(want.len() > PROSE.len() - 101);
	assert_eq!(want.len(), 101);
	// A pure deletion places no slot, so the flag belongs to no file and is the
	// repository's alone.
	assert!(repo.files().iter().all(|f|
		!f.flags().iter().any(|g| matches!(g, Flag::Yielded { .. }))));
	Ok(())
}

/// One author deletes the region another rewrites, and the deleter is higher in
/// op order.
///
/// The rewriter yields, its insertion is buried, and the region is simply empty.
/// This is the trial's "delete beats edit", decided rather than accidental, and
/// told to both. It is the honest loss in the other direction: arbitration can
/// take off the disk text the status quo would have shown, and the bytes are then
/// in the log and one flag away.
#[test]
fn a_deletion_prevails_over_a_rewrite_it_raced() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PARSE, 2));
	ops.push(res!(reps[0].rep(BODY, "\ts.trim().parse::<i64>().unwrap_or(0)\n")));
	ops.push(res!(reps[1].del(BODY)));

	let repo = res!(case(file, "\
/// Parses a decimal string, treating anything malformed as zero.
pub fn parse_or_zero(s: &str) -> i64 {
}
", &ops));
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(2, 3)));
	Ok(())
}

/// A third author who synced with one side of a collision and not the other.
///
/// This is what falsifies the guarantee the rule was first stated with. Replica 1
/// and replica 2 collide; replica 3 has seen replica 1 and not replica 2, and
/// edits an adjacent stretch replica 2 had also named. All three are one
/// component, the maximum is replica 3, replica 2 yields -- and replica 1 does
/// **not**, because it is in replica 3's causal past and the exemption protects
/// it. The region then holds two authors' hunks, each whole.
///
/// The exemption is load-bearing, so this is a consequence of the rule and not a
/// defect in it. What the rule guarantees is whole hunks, never an interleave; it
/// does not guarantee one author.
#[test]
fn a_third_party_synced_with_one_side_composes_two_authors_whole_hunks() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PROSE, 3));
	let first = res!(reps[0].rep("convergent", "correct"));
	ops.push(first.clone());
	ops.push(res!(reps[1].rep("convergent, conserved and attributed",
		"right, and unreadable")));
	// Replica 3 has seen replica 1's edit and not replica 2's.
	res!(reps[2].recv(first));
	ops.push(res!(reps[2].rep("conserved and attributed",
		"conserved, ordered and attributed")));

	let repo = res!(case(file, "The renderer places every run against the anchors it \
was written at, and the result is correct, conserved, ordered and attributed. It \
is not a text.\n", &ops));
	// One yield, and the exempt author's hunk is in the region beside the winner's.
	assert_eq!(yielded_to(&repo, id(2, 3)), Some(id(3, 4)));
	assert_eq!(yielded_to(&repo, id(1, 3)), None);
	assert_eq!(yields(&repo).len(), 1);
	// Whole hunks, never an interleave: each author's insertion renders as one run
	// of its own, and no run holds bytes of two operations.
	let f = match repo.file(file) {
		Some(f)	=> f.clone(),
		None	=> return Err(err!("The render holds no file {}.", file; Test, Missing)),
	};
	let mine: Vec<_> = f.runs().iter().filter(|r| r.content.op() == id(1, 3)).collect();
	let theirs: Vec<_> = f.runs().iter().filter(|r| r.content.op() == id(3, 4)).collect();
	assert_eq!(mine.len(), 1);
	assert_eq!(theirs.len(), 1);
	Ok(())
}

/// The losing author had already refined its own new text before syncing, so its
/// second operation is anchored wholly inside its first.
///
/// Burying the first without the second leaves the second rendering as a fragment
/// at a dead site, and no flag fires for it, because no concurrent operation
/// deleted anything -- which is the smaller scramble one round later. Yielding is
/// therefore transitive: a splice anchored wholly within buried content yields
/// too. The sweep counted 807 such fragments across four thousand trials without
/// the completion and none with it.
#[test]
fn an_edit_inside_a_buried_insertion_yields_with_it() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PARSE, 2));
	ops.push(res!(reps[0].rep(BODY, "\ts.trim().parse::<i64>().unwrap_or(0)\n")));
	// The same author, having seen nobody, refines its own new line.
	ops.push(res!(reps[0].ins("\ts.trim()", ".to_owned()")));
	ops.push(res!(reps[1].rep(BODY,
		"\tlet cleaned = s.trim();\n\tcleaned.parse::<i64>().unwrap_or(0)\n")));

	let repo = res!(case(file, REWRITTEN, &ops));
	// The refinement never contended with anybody, and yields through its host.
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(2, 3)));
	let (_, to, group, through) = match yields(&repo).into_iter().find(|(o, ..)| *o == id(1, 4)) {
		Some(y)	=> y,
		None	=> return Err(err!("The refinement did not yield."; Test, Missing)),
	};
	assert_eq!(to, id(2, 3));
	assert_eq!(through, Some(id(1, 3)));
	assert_eq!(group, vec![id(1, 3), id(2, 3)]);
	// Nothing is stranded, because nothing is left rendering at a dead site.
	assert_eq!(count(&repo, |f| matches!(f, Flag::Stranded { .. })), 0);
	assert_eq!(count(&repo, |f| matches!(f, Flag::Orphaned { .. })), 0);
	Ok(())
}

/// A capture that emits two hunks authors the second with the first as its
/// parent, so without the exemption a winner would void its own other hunk every
/// time a diff produced more than one. Replica 2 rewrites the head of the body and
/// then its tail; replica 1 rewrites the whole body at once, concurrently with
/// both. Replica 2's pair is a sequence, not a race, and both of its hunks
/// survive.
#[test]
fn the_winners_own_earlier_hunk_survives_the_arbitration() -> Outcome<()> {
	let (mut reps, mut ops, file) = res!(seed(PARSE, 2));
	ops.push(res!(reps[1].rep("\tmatch s.trim().parse::<i64>() {\n",
		"\tmatch s.trim().parse() {\n")));
	ops.push(res!(reps[1].rep("\t\tErr(_) => 0,\n", "\t\tErr(_) => -1,\n")));
	ops.push(res!(reps[0].rep(BODY, "\ts.trim().parse::<i64>().unwrap_or(0)\n")));

	let repo = res!(case(file, "\
/// Parses a decimal string, treating anything malformed as zero.
pub fn parse_or_zero(s: &str) -> i64 {
\tmatch s.trim().parse() {
\t\tOk(v) => v,
\t\tErr(_) => -1,
\t}
}
", &ops));
	assert_eq!(yields(&repo).len(), 1);
	assert_eq!(yielded_to(&repo, id(1, 3)), Some(id(2, 4)));
	assert_eq!(yielded_to(&repo, id(2, 3)), None);
	Ok(())
}

/// This is the shape the sweep found the merge worse in, five trials in 3,252,
/// and it is expected rather than a defect. Two components are contended by the
/// same pair of replicas and merge; the merged group's maximum is replica 1's
/// later hunk rather than replica 2's deletion, so replica 1's earlier hunk --
/// which separate components would have buried under replica 2's deletion -- is
/// now in the winner's causal past, the exemption protects it, and its bytes
/// render again. A third component contended by a different pair does not merge,
/// keeps its own winner, and the file then reads as two authors.
///
/// Merging is therefore not monotone in the shape of the file. Guarding against
/// it would need a rule with no precedent in the design, and recording it is
/// enough.
#[test]
fn merging_two_components_can_revive_an_edit_separation_would_have_buried()
	-> Outcome<()>
{
	let (mut reps, mut ops, file) = res!(seed("one\ntwo\nthree\nfour\n", 3));
	// Replica 1: a hunk it would lose outright on its own, an unrelated edit that
	// lifts its counter, and then the hunk that becomes the merged group's maximum.
	ops.push(res!(reps[0].rep("one\n", "uno\n")));
	ops.push(res!(reps[0].rep("four\n", "cuatro\n")));
	ops.push(res!(reps[0].rep("two\n", "dos\n")));
	// Replica 2 contends over the first two regions, and over the third with
	// replica 3.
	ops.push(res!(reps[1].del("one\n")));
	ops.push(res!(reps[1].rep("two\n", "zwei\n")));
	ops.push(res!(reps[1].rep("three\n", "drei\n")));
	// Replica 3 contends over the third region only.
	ops.push(res!(reps[2].rep("three\n", "tres\n")));

	let repo = res!(case(file, "uno\ndos\ndrei\ncuatro\n", &ops));
	// The first two components are contended by replicas 1 and 2 alike, so they
	// are one group of four whose maximum is replica 1's third operation.
	let ys = yields(&repo);
	assert_eq!(yielded_to(&repo, id(2, 3)), Some(id(1, 5)));
	assert_eq!(yielded_to(&repo, id(2, 4)), Some(id(1, 5)));
	// Replica 2's deletion never named a byte replica 1's third operation named.
	assert!(!overlapped(&repo, id(2, 3), id(1, 5)));
	// Replica 1's first hunk is in the merged winner's causal past, so it is
	// exempt and renders, although its own component's maximum was concurrent with
	// it and higher.
	assert_eq!(yielded_to(&repo, id(1, 3)), None);
	// The third component keeps its own winner, and the file reads as two authors,
	// each hunk whole.
	assert_eq!(yielded_to(&repo, id(3, 3)), Some(id(2, 5)));
	assert_eq!(ys.len(), 3);
	Ok(())
}

/// The flag survives the wire, group and host included.
#[test]
fn a_yield_flag_round_trips_through_its_dat_form() -> Outcome<()> {
	for through in [None, Some(id(1, 3))] {
		let flag = Flag::Yielded {
			op:		id(1, 4),
			to:		id(2, 7),
			group:	vec![id(1, 3), id(2, 3), id(2, 7)],
			through,
		};
		let back = res!(Flag::from_dat(&flag.to_dat()));
		assert_eq!(back, flag);
		assert_eq!(flag.code(), crate::seq::render::CODE_YIELDED);
		assert_eq!(flag.name(), "Yielded");
		assert_eq!(flag.op(), Some(id(1, 4)));
	}
	Ok(())
}

/// A planted sweep: several authors rewriting overlapping regions of one file at
/// once, rendered under permuted delivery.
///
/// The planted cases say the rule gives the right answer on the shapes it was
/// designed against. This says the rule cannot be inert and cannot leave the
/// hazard its second completion exists for. Three properties, over every trial
/// that planted a collision:
///
/// - **The arbitration fires.** A rule nothing reaches is a rule nothing tests,
///   and the count is asserted rather than hoped for.
/// - **No fragment renders at a dead site.** A splice both of whose anchors name
///   content inside a buried insertion is the smaller scramble one round later,
///   and the transitive completion exists to bury it too; the sweep that settled
///   the rule counted 807 of these without the completion and none with it.
/// - **Nothing diverges and nothing is lost**, which [`converge`] checks on every
///   delivery order.
#[test]
fn a_planted_sweep_of_overlapping_rewrites_converges_and_strands_nothing()
	-> Outcome<()>
{
	let mut state = 0x51ed_3c9a_7b2f_0e41u64;
	let mut next = move || {
		state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(state >> 33) as usize
	};
	let mut collisions = 0usize;
	let mut yielded = 0usize;
	for _ in 0..100 {
		let lines = 6 + next() % 5;
		let mut base = String::new();
		for i in 0..lines {
			base.push_str(&fmt!("line {} of the file\n", i));
		}
		let k = 2 + next() % 3;
		let (mut reps, mut ops, file) = res!(seed(&base, k as u64));
		for r in 0..k {
			// The order an author works through its hunks in is not the order the
			// hunks sit in the file, which is what two people fixing one file
			// actually do and is what lets two components take different winners.
			let mut which: Vec<usize> = (0..lines).collect();
			for i in (1..which.len()).rev() {
				which.swap(i, next() % (i + 1));
			}
			which.truncate(1 + next() % 3);
			for (n, l) in which.iter().enumerate() {
				let find = fmt!("line {} of the file\n", l);
				if next() % 4 == 0 {
					ops.push(res!(reps[r].del(&find)));
					continue;
				}
				let with = fmt!("row {} by {}\n", l, r + 1);
				ops.push(res!(reps[r].rep(&find, &with)));
				// A quarter of the authors refine their own new text before
				// syncing, which is what the transitive completion is for.
				if n == 0 && next() % 4 == 0 {
					let host = fmt!("row {}", l);
					ops.push(res!(reps[r].ins(&host, " (revised)")));
				}
			}
		}
		let repo = res!(converge(&ops));
		let ys = yields(&repo);
		if ys.is_empty() {
			continue;
		}
		collisions += 1;
		yielded += ys.len();
		assert_eq!(count(&repo, |f| matches!(f, Flag::Orphaned { .. })), 0);

		// Nothing renders at a dead site. The insertions that were buried whole,
		// and then every operation that still shows bytes: none of them may be
		// anchored inside one.
		let buried: Vec<OpId> = ys.iter()
			.map(|(op, ..)| *op)
			.filter(|op| ops.iter().any(|(h, o)| h.id() == *op && match o {
				Op::Splice { insert, .. }	=> !insert.is_empty(),
				_							=> false,
			}))
			.collect();
		let f = match repo.file(file) {
			Some(f)	=> f.clone(),
			None	=> return Err(err!("The render holds no file {}.", file; Test, Missing)),
		};
		for run in f.runs() {
			let op = match ops.iter().find(|(h, _)| h.id() == run.content.op()) {
				Some((_, o))	=> o,
				None			=> continue,
			};
			let (l, r) = op.origins();
			let hosted = |a: &Option<crate::id::Anchor>| a.as_ref()
				.map(|x| buried.contains(&x.content.op))
				.unwrap_or(false);
			assert!(!(hosted(&l) && hosted(&r)),
				"{} renders inside buried content", run.content.op());
		}
	}
	assert!(collisions > 10,
		"the sweep planted too few collisions to say anything: {}", collisions);
	assert!(yielded > collisions,
		"the arbitration barely fired: {} yields over {} collisions",
		yielded, collisions);
	Ok(())
}
