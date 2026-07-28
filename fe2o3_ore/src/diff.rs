//! What has to be done to one version of a file's bytes to obtain another.
//!
//! The operation vocabulary records intent, and an author working in an editor
//! supplies it directly. An author working in a filesystem does not: what is
//! available is the file as it stood and the file as it now stands, and the
//! edit between them has to be recovered. That recovery is this module, and it
//! is the one place in the crate where a diff is a guess rather than a record.
//!
//! # Division of labour
//!
//! The splices returned here are **positional**, and every position is an
//! offset into the *old* bytes. That is deliberate. Content anchors are minted
//! from a render, and the module that holds the render --
//! [`crate::seq::render::Rendered`] -- already knows how to turn an offset into
//! the names of the bytes around it. So the split is:
//!
//! - here: old bytes and new bytes in, an ordered list of positional splices
//!   out, with no knowledge of operation identifiers, anchors or history;
//! - the caller: for each splice in turn, `rendered.splice(at, delete, insert)`,
//!   which resolves the offsets against the render and yields a content
//!   anchored [`crate::op::Op`].
//!
//! Because every offset is in the old bytes, and the render the caller holds is
//! the old bytes, the whole list is resolved against that one render. The
//! splices are ascending and non-overlapping, and consecutive splices are
//! separated by at least [`MIN_SPLICE_GAP`] unchanged bytes, so no splice
//! anchors itself on content another one removes.
//!
//! # How the diff is found
//!
//! Two levels. Lines first: the bytes are cut at every `\n`, each distinct line
//! is given a number, and Myers' O(ND) algorithm runs over the numbers. This is
//! what keeps a large file affordable, since the sequences it compares are as
//! long as the file has lines rather than bytes, and the cost is driven by the
//! size of the difference rather than the size of the file.
//!
//! Bytes second: each run of changed lines is trimmed of the bytes its two
//! sides share at either end, and, where what remains is small, Myers runs
//! again over those bytes. A one character change in the middle of a line
//! therefore stores one character, not the line and not the file.
//!
//! # The bound on cost
//!
//! Myers costs O(ND) in time and, as written here, O(D²) in memory, where D is
//! the length of the edit script. Both are fine while D is small and neither is
//! acceptable when it is not, so D is capped: [`MAX_LINE_SCRIPT`] line
//! insertions and deletions, and [`MAX_BYTE_SCRIPT`] within a refined run. A
//! line script that would exceed its cap means the two versions have little
//! left in common, and the answer falls back to a single splice covering
//! everything between the shared prefix and the shared suffix -- which is the
//! whole of what a diff-free frontend would have recorded anyway, arrived at in
//! linear time. The cap on the line script bounds the working memory at roughly
//! four megabytes.
//!
//! # Bytes, not text
//!
//! Nothing here decodes. Lines are runs ending in `\n` and a line is compared
//! to another by its bytes, so an encoding this module has never heard of costs
//! it nothing. Binary content is not detected and does not need to be: a file
//! with no newline in it is one line, the line level pass has nothing to work
//! with, and the result is the single trimmed splice -- the right answer for a
//! file whose bytes have no structure the caller has declared.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::ops::Range;


/// Greatest line level edit script the diff will compute before falling back to
/// a single trimmed splice.
pub const MAX_LINE_SCRIPT: usize = 1024;

/// Greatest byte level edit script the refinement of one changed run will
/// compute before leaving the run as a single splice.
pub const MAX_BYTE_SCRIPT: usize = 256;

/// Greatest changed run, in bytes on either side, the byte level refinement is
/// attempted on at all.
pub const MAX_REFINE_LEN: usize = 4096;

/// Fewest unchanged bytes that may separate two splices.
///
/// Two operations each carry their own identity, parents and origins, which
/// costs more than a handful of bytes that did not change; splices closer
/// together than this are merged into one. Keeping the gap non-zero is also
/// what stops one splice anchoring on content that another removes.
pub const MIN_SPLICE_GAP: usize = 16;


/// One positional edit against the old bytes: remove `delete` bytes at `at`,
/// and put `insert` in their place.
///
/// The three fields are independent -- any offset, any length and any payload
/// name an edit, even where the file cannot supply them -- so there is no
/// invariant here for a struct literal to break. What the offsets have to agree
/// with is the file, and [`apply`] is what says whether they do.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Splice {
	/// Offset into the old bytes at which the edit begins.
	pub at:		usize,
	/// Number of old bytes the edit removes.
	pub delete:	usize,
	/// Bytes the edit puts in their place.
	pub insert:	Vec<u8>,
}

impl Splice {
	/// Returns the number of bytes the edit inserts.
	pub fn insert_len(&self) -> usize {
		self.insert.len()
	}
}


/// Returns the ordered splices that turn `old` into `new`.
///
/// The list is ascending by offset and non-overlapping, and is empty when the
/// two are already the same.
pub fn diff(old: &[u8], new: &[u8]) -> Vec<Splice> {
	diff_with_budget(old, new, MAX_LINE_SCRIPT)
}

/// Returns the ordered splices that turn `old` into `new`, with the line level
/// edit script capped at `budget` rather than at [`MAX_LINE_SCRIPT`].
///
/// A budget of zero forces the single trimmed splice, which is what the
/// fallback path produces.
pub fn diff_with_budget(old: &[u8], new: &[u8], budget: usize) -> Vec<Splice> {
	if old == new {
		return Vec::new();
	}
	let old_starts = line_starts(old);
	let new_starts = line_starts(new);
	let mut names: HashMap<&[u8], u32> = HashMap::new();
	let old_ids = line_ids(old, &old_starts, &mut names);
	let new_ids = line_ids(new, &new_starts, &mut names);
	let changes = match changed_runs(&old_ids, &new_ids, budget) {
		Some(c)	=> c,
		None	=> return vec![trimmed_splice(old, new)],
	};
	let mut out: Vec<Splice> = Vec::with_capacity(changes.len());
	for c in &changes {
		refine(
			old,
			new,
			old_starts[c.a.start]..old_starts[c.a.end],
			new_starts[c.b.start]..new_starts[c.b.end],
			&mut out,
		);
	}
	coalesce(old, &mut out);
	out
}

/// Applies splices to `old` and returns the bytes that result.
///
/// Fails where the list is not ascending, where two splices overlap, or where
/// one reaches past the end of the file: all three mean the list was not
/// produced against these bytes, and a caller had better hear so rather than
/// receive a plausible wrong answer.
pub fn apply(old: &[u8], splices: &[Splice])
	-> Outcome<Vec<u8>>
{
	let mut out: Vec<u8> = Vec::with_capacity(old.len());
	let mut at = 0usize;
	for (i, sp) in splices.iter().enumerate() {
		if sp.at < at {
			return Err(err!(
				"Splice {} begins at {}, behind the {} bytes of the old file already \
				consumed; splices must ascend and may not overlap.", i, sp.at, at;
			Invalid, Input, Order));
		}
		let end = match sp.at.checked_add(sp.delete) {
			Some(e) if e <= old.len()	=> e,
			_ => return Err(err!(
				"Splice {} removes {} bytes at {}, reaching past the {} bytes the old \
				file holds.", i, sp.delete, sp.at, old.len();
			Invalid, Input, Range)),
		};
		out.extend_from_slice(&old[at..sp.at]);
		out.extend_from_slice(&sp.insert);
		at = end;
	}
	out.extend_from_slice(&old[at..]);
	Ok(out)
}


/// Returns the number of leading bytes two slices share.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
	let limit = a.len().min(b.len());
	let mut n = 0;
	while n < limit && a[n] == b[n] {
		n += 1;
	}
	n
}

/// Returns the number of trailing bytes two slices share, without running back
/// past the `floor` bytes already accounted for at the front.
fn common_suffix(a: &[u8], b: &[u8], floor: usize) -> usize {
	let limit = (a.len() - floor).min(b.len() - floor);
	let mut n = 0;
	while n < limit && a[a.len() - 1 - n] == b[b.len() - 1 - n] {
		n += 1;
	}
	n
}

/// Returns the single splice covering everything between the shared prefix and
/// the shared suffix, which is what a frontend with no diff at all records.
fn trimmed_splice(old: &[u8], new: &[u8]) -> Splice {
	let front = common_prefix(old, new);
	let back = common_suffix(old, new, front);
	Splice {
		at:		front,
		delete:	old.len() - front - back,
		insert:	new[front..new.len() - back].to_vec(),
	}
}

/// Returns the offset at which each line begins, with the length of the input
/// as a final entry, so that line `i` occupies `starts[i]..starts[i + 1]`.
///
/// A line carries its own terminating newline. A file whose last line has none
/// still ends in a line, and an empty file has no lines at all.
fn line_starts(bytes: &[u8]) -> Vec<usize> {
	let mut starts = vec![0usize];
	for (i, b) in bytes.iter().enumerate() {
		if *b == b'\n' {
			starts.push(i + 1);
		}
	}
	if starts[starts.len() - 1] != bytes.len() {
		starts.push(bytes.len());
	}
	starts
}

/// Numbers the lines, giving equal lines equal numbers across both files.
///
/// The numbering is what the line level pass compares, so that a comparison
/// costs one integer rather than the length of a line, and two lines that
/// differ can never be given one number.
fn line_ids<'a>(bytes: &'a [u8], starts: &[usize], names: &mut HashMap<&'a [u8], u32>)
	-> Vec<u32>
{
	let mut out = Vec::with_capacity(starts.len() - 1);
	for w in starts.windows(2) {
		let next = names.len() as u32;
		out.push(*names.entry(&bytes[w[0]..w[1]]).or_insert(next));
	}
	out
}


/// A run of `a` that has become a run of `b`. At least one of the two is
/// non-empty.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Change {
	/// What the run was, in the old sequence.
	a:	Range<usize>,
	/// What it has become, in the new sequence.
	b:	Range<usize>,
}


/// Returns the changed runs turning `a` into `b`, or `None` where the edit
/// script would exceed `budget`.
///
/// The shared head and tail are taken off first, since neither costs anything
/// to find and both shrink what Myers is asked to do.
fn changed_runs<T: PartialEq>(a: &[T], b: &[T], budget: usize)
	-> Option<Vec<Change>>
{
	let limit = a.len().min(b.len());
	let mut head = 0;
	while head < limit && a[head] == b[head] {
		head += 1;
	}
	let mut tail = 0;
	while tail < limit - head
		&& a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
	{
		tail += 1;
	}
	let (an, bn) = (a.len() - tail, b.len() - tail);
	if head == an && head == bn {
		return Some(Vec::new());
	}
	// Where one side has nothing left, every remaining element of the other is
	// the whole of the change and Myers has nothing to decide.
	if head == an || head == bn {
		return Some(vec![Change { a: head..an, b: head..bn }]);
	}
	let matched = match myers(&a[head..an], &b[head..bn], budget) {
		Some(m)	=> m,
		None	=> return None,
	};
	Some(runs_between(&matched, an - head, bn - head, head))
}

/// Turns the matched pairs into the runs that lie between them.
fn runs_between(matched: &[(usize, usize)], n: usize, m: usize, off: usize)
	-> Vec<Change>
{
	let mut out: Vec<Change> = Vec::new();
	let (mut i, mut j) = (0usize, 0usize);
	for (mi, mj) in matched {
		if *mi > i || *mj > j {
			out.push(Change {
				a:	off + i..off + mi,
				b:	off + j..off + mj,
			});
		}
		i = mi + 1;
		j = mj + 1;
	}
	if i < n || j < m {
		out.push(Change {
			a:	off + i..off + n,
			b:	off + j..off + m,
		});
	}
	out
}

/// Returns the pairs of positions a shortest edit script leaves matched,
/// ascending, or `None` where that script would be longer than `budget`.
///
/// This is Myers' greedy algorithm: for each script length `d` in turn, the
/// furthest point reachable on each diagonal is extended along whatever run of
/// equal elements follows it, and the first `d` that reaches the far corner is
/// the length of a shortest script. Keeping the furthest points of every `d`
/// makes the path itself recoverable by walking back through them, which is
/// where the matched pairs come from, and is also what costs O(D²) memory and
/// so what the budget is bounding.
fn myers<T: PartialEq>(a: &[T], b: &[T], budget: usize)
	-> Option<Vec<(usize, usize)>>
{
	let (n, m) = (a.len() as isize, b.len() as isize);
	let max = (a.len() + b.len()) as isize;
	let cap = budget.min(a.len() + b.len());
	// Furthest x reached on each diagonal k, shifted so that k = 0 sits at max.
	let mut v: Vec<isize> = vec![0; (2 * max + 1) as usize];
	let mut trace: Vec<Vec<isize>> = Vec::with_capacity(cap + 1);
	for d in 0..=cap {
		let dd = d as isize;
		// The state before this step, which is what the walk back reads.
		trace.push(v[(max - dd) as usize..=(max + dd) as usize].to_vec());
		let mut k = -dd;
		while k <= dd {
			let mut x = if k == -dd
				|| (k != dd && v[(max + k - 1) as usize] < v[(max + k + 1) as usize])
			{
				v[(max + k + 1) as usize]
			} else {
				v[(max + k - 1) as usize] + 1
			};
			let mut y = x - k;
			while x < n && y < m && a[x as usize] == b[y as usize] {
				x += 1;
				y += 1;
			}
			v[(max + k) as usize] = x;
			if x >= n && y >= m {
				return Some(walk_back(&trace, n, m, d));
			}
			k += 2;
		}
	}
	None
}

/// Walks the furthest points back from the far corner, collecting the pairs
/// matched on the way.
fn walk_back(trace: &[Vec<isize>], n: isize, m: isize, found: usize)
	-> Vec<(usize, usize)>
{
	let mut matched: Vec<(usize, usize)> = Vec::new();
	let (mut x, mut y) = (n, m);
	for d in (0..=found).rev() {
		let (prev_x, prev_y) = if d == 0 {
			(0, 0)
		} else {
			let dd = d as isize;
			let w = &trace[d];
			let at = |k: isize| w[(k + dd) as usize];
			let k = x - y;
			let prev_k = if k == -dd || (k != dd && at(k - 1) < at(k + 1)) {
				k + 1
			} else {
				k - 1
			};
			let px = at(prev_k);
			(px, px - prev_k)
		};
		while x > prev_x && y > prev_y {
			x -= 1;
			y -= 1;
			matched.push((x as usize, y as usize));
		}
		x = prev_x;
		y = prev_y;
	}
	matched.reverse();
	matched
}

/// Turns one changed run of lines into the splices that describe it, trimming
/// what its two sides share and, where the remainder is small enough to be
/// worth the second pass, splitting it further at the byte level.
fn refine(
	old:	&[u8],
	new:	&[u8],
	o:		Range<usize>,
	n:		Range<usize>,
	out:	&mut Vec<Splice>,
) {
	let front = common_prefix(&old[o.clone()], &new[n.clone()]);
	let back = common_suffix(&old[o.clone()], &new[n.clone()], front);
	let (oa, ob) = (o.start + front, o.end - back);
	let (na, nb) = (n.start + front, n.end - back);
	if oa == ob && na == nb {
		return;
	}
	if oa < ob && na < nb
		&& ob - oa <= MAX_REFINE_LEN
		&& nb - na <= MAX_REFINE_LEN
	{
		if let Some(inner) = changed_runs(&old[oa..ob], &new[na..nb], MAX_BYTE_SCRIPT) {
			for c in inner {
				out.push(Splice {
					at:		oa + c.a.start,
					delete:	c.a.end - c.a.start,
					insert:	new[na + c.b.start..na + c.b.end].to_vec(),
				});
			}
			return;
		}
	}
	out.push(Splice {
		at:		oa,
		delete:	ob - oa,
		insert:	new[na..nb].to_vec(),
	});
}

/// Merges splices lying closer together than [`MIN_SPLICE_GAP`], carrying the
/// unchanged bytes between them into the payload.
fn coalesce(old: &[u8], splices: &mut Vec<Splice>) {
	if splices.len() < 2 {
		return;
	}
	let mut out: Vec<Splice> = Vec::with_capacity(splices.len());
	for sp in splices.drain(..) {
		let near = match out.last() {
			Some(prev)	=> sp.at - (prev.at + prev.delete) < MIN_SPLICE_GAP,
			None		=> false,
		};
		match out.last_mut() {
			Some(prev) if near => {
				let end = prev.at + prev.delete;
				prev.insert.extend_from_slice(&old[end..sp.at]);
				prev.insert.extend_from_slice(&sp.insert);
				prev.delete = sp.at + sp.delete - prev.at;
			},
			_ => out.push(sp),
		}
	}
	*splices = out;
}


#[cfg(test)]
mod tests {
	use super::*;

	/// Asserts the property everything else here rests on: the splices, applied
	/// to the old bytes, produce the new bytes exactly. Returns them so a test
	/// can go on to say something about their shape.
	fn diffed(old: &[u8], new: &[u8])
		-> Outcome<Vec<Splice>>
	{
		let splices = diff(old, new);
		let got = res!(apply(old, &splices));
		assert_eq!(
			got, new,
			"applying {} splices to {:?} gave {:?}",
			splices.len(), Bytes(old), Bytes(&got),
		);
		Ok(splices)
	}

	/// A readable form for the byte strings in a failure message.
	struct Bytes<'a>(&'a [u8]);

	impl std::fmt::Debug for Bytes<'_> {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			match std::str::from_utf8(self.0) {
				Ok(s)	=> write!(f, "{:?}", s),
				Err(_)	=> write!(f, "{:?}", self.0),
			}
		}
	}

	/// Sums what a list of splices inserts.
	fn inserted(splices: &[Splice]) -> usize {
		splices.iter().map(|s| s.insert_len()).sum()
	}

	/// Sums what a list of splices removes.
	fn deleted(splices: &[Splice]) -> usize {
		splices.iter().map(|s| s.delete).sum()
	}

	/// Identical inputs, of every shape, are no edit at all.
	#[test]
	fn identical_inputs_diff_to_nothing() -> Outcome<()> {
		for s in [&b""[..], b"a", b"a\n", b"one\ntwo\nthree\n", b"no trailing newline"] {
			assert!(res!(diffed(s, s)).is_empty(), "{:?}", Bytes(s));
		}
		Ok(())
	}

	/// An empty file on either side is one splice covering everything.
	#[test]
	fn empty_sides_are_one_splice() -> Outcome<()> {
		let text = b"one\ntwo\nthree\n";
		let born = res!(diffed(b"", text));
		assert_eq!(born.len(), 1);
		assert_eq!(born[0].at, 0);
		assert_eq!(born[0].delete, 0);
		assert_eq!(born[0].insert, text.to_vec());
		let died = res!(diffed(text, b""));
		assert_eq!(died.len(), 1);
		assert_eq!(died[0].at, 0);
		assert_eq!(died[0].delete, text.len());
		assert!(died[0].insert.is_empty());
		// Two empty files are the same file.
		assert!(res!(diffed(b"", b"")).is_empty());
		Ok(())
	}

	/// A line inserted in the middle costs exactly that line.
	#[test]
	fn an_inserted_line_costs_one_line() -> Outcome<()> {
		let old = b"alpha\nbeta\ngamma\n";
		let new = b"alpha\nbeta\ndelta\ngamma\n";
		let sp = res!(diffed(old, new));
		assert_eq!(sp.len(), 1);
		assert_eq!(sp[0].at, 11, "the gap before gamma");
		assert_eq!(sp[0].delete, 0);
		assert_eq!(sp[0].insert, b"delta\n".to_vec());
		Ok(())
	}

	/// A line deleted from the middle removes exactly that line.
	#[test]
	fn a_deleted_line_costs_one_line() -> Outcome<()> {
		let old = b"alpha\nbeta\ngamma\n";
		let new = b"alpha\ngamma\n";
		let sp = res!(diffed(old, new));
		assert_eq!(sp.len(), 1);
		assert_eq!(sp[0].at, 6, "the start of beta");
		assert_eq!(sp[0].delete, 5, "beta and its newline");
		assert!(sp[0].insert.is_empty());
		Ok(())
	}

	/// A change within a line stores the changed bytes, not the line.
	#[test]
	fn an_edit_within_a_line_costs_the_changed_bytes() -> Outcome<()> {
		let old = b"alpha\nthe quick brown fox\ngamma\n";
		let new = b"alpha\nthe quick brawn fox\ngamma\n";
		let sp = res!(diffed(old, new));
		assert_eq!(sp.len(), 1);
		assert_eq!(sp[0].delete, 1);
		assert_eq!(sp[0].insert, b"a".to_vec());
		Ok(())
	}

	/// Two changes in one line, far enough apart, stay two splices; brought
	/// close together they become one.
	#[test]
	fn nearby_changes_merge_and_distant_ones_do_not() -> Outcome<()> {
		let old = b"X-------------------------X\n";
		let new = b"Y-------------------------Y\n";
		let apart = res!(diffed(old, new));
		assert_eq!(apart.len(), 2, "twenty five unchanged bytes keep them apart");
		assert_eq!(inserted(&apart), 2);
		let old = b"X---X\n";
		let new = b"Y---Y\n";
		let close = res!(diffed(old, new));
		assert_eq!(close.len(), 1, "three unchanged bytes do not");
		assert_eq!(close[0].insert, b"Y---Y".to_vec());
		Ok(())
	}

	/// A move shaped edit -- one line taken from the top of a file to the
	/// bottom -- is described as a deletion and an insertion. The module makes
	/// no claim to notice a move; the operation vocabulary has one, and finding
	/// it is not this module's business.
	#[test]
	fn a_move_shaped_edit_becomes_a_delete_and_an_insert() -> Outcome<()> {
		let old = b"moved\nalpha\nbeta\ngamma\ndelta\nepsilon\n";
		let new = b"alpha\nbeta\ngamma\ndelta\nepsilon\nmoved\n";
		let sp = res!(diffed(old, new));
		assert_eq!(sp.len(), 2);
		assert_eq!(sp[0].at, 0);
		assert_eq!(sp[0].delete, 6);
		assert!(sp[0].insert.is_empty());
		assert_eq!(sp[1].delete, 0);
		assert_eq!(sp[1].insert, b"moved\n".to_vec());
		Ok(())
	}

	/// A file with nothing in common with its successor is replaced wholesale.
	#[test]
	fn a_whole_file_rewrite_is_one_splice() -> Outcome<()> {
		let old = b"alpha\nbeta\ngamma\n";
		let new = b"one\ntwo\nthree\n";
		let sp = res!(diffed(old, new));
		assert_eq!(sp.len(), 1);
		assert_eq!(sp[0].at, 0);
		// The final newline is the one thing the two versions still share.
		assert_eq!(sp[0].delete, old.len() - 1);
		assert_eq!(sp[0].insert, new[..new.len() - 1].to_vec());
		Ok(())
	}

	/// A missing trailing newline, gained or lost, is one byte either way.
	#[test]
	fn a_trailing_newline_is_one_byte() -> Outcome<()> {
		let bare = b"alpha\nbeta";
		let ended = b"alpha\nbeta\n";
		let gained = res!(diffed(bare, ended));
		assert_eq!(gained.len(), 1);
		assert_eq!(gained[0].at, bare.len());
		assert_eq!(gained[0].delete, 0);
		assert_eq!(gained[0].insert, b"\n".to_vec());
		let lost = res!(diffed(ended, bare));
		assert_eq!(lost.len(), 1);
		assert_eq!(lost[0].at, bare.len());
		assert_eq!(lost[0].delete, 1);
		assert!(lost[0].insert.is_empty());
		Ok(())
	}

	/// Content with no newline in it at all is one line, and comes out as the
	/// single trimmed splice.
	#[test]
	fn newline_free_content_is_one_trimmed_splice() -> Outcome<()> {
		let old: Vec<u8> = (0u8..=255).chain(0u8..=255).collect();
		let mut new = old.clone();
		new[300] = 0xff;
		new[301] = 0xff;
		let sp = res!(diffed(&old, &new));
		assert_eq!(sp.len(), 1);
		assert_eq!(sp[0].at, 300);
		assert_eq!(sp[0].delete, 2);
		assert_eq!(sp[0].insert, vec![0xff, 0xff]);
		Ok(())
	}

	/// Two distant edits in a large file cost the two edits, which is the whole
	/// point of the exercise: a single trimmed splice would have stored
	/// everything between them.
	#[test]
	fn two_distant_edits_in_a_large_file_cost_two_edits() -> Outcome<()> {
		let mut lines: Vec<String> = Vec::with_capacity(1000);
		for i in 0..1000 {
			lines.push(fmt!("line {} of the file, with some words on it\n", i));
		}
		let old: Vec<u8> = lines.concat().into_bytes();
		lines[99] = fmt!("line 99 has been rewritten entirely\n");
		lines[899] = fmt!("line 899 of the file, with other words on it\n");
		let new: Vec<u8> = lines.concat().into_bytes();
		let sp = res!(diffed(&old, &new));
		assert_eq!(sp.len(), 2, "one splice per changed line");
		// One line was rewritten, so it is stored; the other lost a word, so
		// only the word is. Neither is anywhere near the size of the file.
		assert!(inserted(&sp) <= 40, "inserted {} bytes", inserted(&sp));
		assert!(deleted(&sp) <= 40, "deleted {} bytes", deleted(&sp));
		// What the frontend does without a diff, for comparison: everything
		// between the first change and the last.
		let bare = trimmed_splice(&old, &new);
		assert!(bare.insert.len() > 30_000, "the whole point of the exercise");
		Ok(())
	}

	/// A budget too small for the edit script falls back to the single trimmed
	/// splice rather than computing a longer one.
	#[test]
	fn an_exhausted_budget_falls_back_to_one_splice() -> Outcome<()> {
		let mut old = String::new();
		let mut new = String::new();
		for i in 0..200 {
			// Every other line changes, so the edit script is two hundred long
			// and the changes are spread across two hundred runs.
			if i % 2 == 0 {
				old.push_str(&fmt!("old line {}, with words after it\n", i));
				new.push_str(&fmt!("new line {}, with words after it\n", i));
			} else {
				let same = fmt!("line {} is left exactly as it was\n", i);
				old.push_str(&same);
				new.push_str(&same);
			}
		}
		let (old, new) = (old.into_bytes(), new.into_bytes());
		assert!(diff_with_budget(&old, &new, 512).len() > 1, "a full budget diffs");
		for budget in [0usize, 1, 8, 64] {
			let sp = diff_with_budget(&old, &new, budget);
			assert_eq!(sp.len(), 1, "budget {}", budget);
			assert_eq!(res!(apply(&old, &sp)), new, "budget {}", budget);
			assert_eq!(sp[0], trimmed_splice(&old, &new), "budget {}", budget);
		}
		Ok(())
	}

	/// The refinement inside a changed run is bounded too: a run longer than
	/// the refinement limit is left as one splice rather than diffed byte by
	/// byte.
	#[test]
	fn a_long_changed_run_is_left_whole() -> Outcome<()> {
		// One line, longer than the refinement limit, differing in two places
		// far apart.
		let mut old = vec![b'-'; MAX_REFINE_LEN * 2];
		old.push(b'\n');
		let mut new = old.clone();
		new[10] = b'X';
		new[MAX_REFINE_LEN * 2 - 10] = b'X';
		let sp = res!(diffed(&old, &new));
		assert_eq!(sp.len(), 1, "the run is too long to refine");
		assert!(sp[0].insert.len() > MAX_REFINE_LEN);
		Ok(())
	}

	/// The applier refuses a list that was not produced against these bytes.
	#[test]
	fn apply_refuses_a_list_that_does_not_fit() -> Outcome<()> {
		let old = b"alpha\nbeta\n";
		assert!(apply(old, &[Splice { at: 12, delete: 0, insert: Vec::new() }]).is_err());
		assert!(apply(old, &[Splice { at: 0, delete: 12, insert: Vec::new() }]).is_err());
		assert!(apply(old, &[
			Splice { at: 6, delete: 1, insert: Vec::new() },
			Splice { at: 2, delete: 1, insert: Vec::new() },
		]).is_err());
		assert!(apply(old, &[
			Splice { at: 0, delete: 4, insert: Vec::new() },
			Splice { at: 2, delete: 1, insert: Vec::new() },
		]).is_err(), "overlapping splices");
		assert!(apply(old, &[
			Splice { at: usize::MAX, delete: usize::MAX, insert: Vec::new() },
		]).is_err(), "an offset that would overflow");
		Ok(())
	}

	/// Whatever the diff returns is ascending, non-overlapping, and separated
	/// by at least the minimum gap.
	fn assert_well_formed(old: &[u8], splices: &[Splice]) {
		let mut at = 0usize;
		let mut first = true;
		for sp in splices {
			assert!(sp.at + sp.delete <= old.len(), "a splice reaches past the file");
			if first {
				first = false;
			} else {
				assert!(sp.at >= at + MIN_SPLICE_GAP,
					"splices at {} and {} are less than {} bytes apart",
					at, sp.at, MIN_SPLICE_GAP);
			}
			assert!(sp.delete > 0 || !sp.insert.is_empty(), "a splice does nothing");
			at = sp.at + sp.delete;
		}
	}

	/// Two hundred random pairs of files, all of them applied back: the oracle
	/// is that the result is the new bytes exactly, every time.
	#[test]
	fn random_pairs_apply_back_to_the_new_bytes() -> Outcome<()> {
		// A small linear congruential generator, so a failure can be reproduced.
		let mut state = 0x2545_F491_4F6C_DD1Du64;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		// A small alphabet of lines, so that lines repeat and the line level
		// pass has real matching to do.
		let words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "", "x"];
		for trial in 0..220 {
			let mut old: Vec<u8> = Vec::new();
			for _ in 0..(next() % 40) {
				old.extend_from_slice(words[next() % words.len()].as_bytes());
				// Most lines end; some do not, which is how a file ends up with
				// a fragment on the end and how two lines end up joined.
				if next() % 8 != 0 {
					old.push(b'\n');
				}
			}
			// The new file is the old one put through a handful of edits, so
			// that the two are usually related, and occasionally not at all.
			let mut new = old.clone();
			for _ in 0..(next() % 6) {
				match next() % 4 {
					0 if !new.is_empty() => {
						// Delete a run.
						let at = next() % new.len();
						let len = (next() % 12).min(new.len() - at);
						new.drain(at..at + len);
					},
					1 => {
						// Insert a line.
						let at = if new.is_empty() { 0 } else { next() % new.len() };
						let mut ins = words[next() % words.len()].as_bytes().to_vec();
						ins.push(b'\n');
						let tail = new.split_off(at);
						new.extend_from_slice(&ins);
						new.extend_from_slice(&tail);
					},
					2 if !new.is_empty() => {
						// Change one byte.
						let at = next() % new.len();
						new[at] = b'a' + (next() % 26) as u8;
					},
					_ => {
						// Move a run to the front.
						if new.len() > 4 {
							let at = next() % (new.len() - 2);
							let len = (next() % 10).min(new.len() - at);
							let run: Vec<u8> = new.drain(at..at + len).collect();
							let to = if new.is_empty() { 0 } else { next() % new.len() };
							let tail = new.split_off(to);
							new.extend_from_slice(&run);
							new.extend_from_slice(&tail);
						}
					},
				}
			}
			let splices = diff(&old, &new);
			let got = res!(apply(&old, &splices));
			assert_eq!(
				got, new,
				"trial {}: {:?} -> {:?} gave {:?}",
				trial, Bytes(&old), Bytes(&new), Bytes(&got),
			);
			assert_well_formed(&old, &splices);
			// The same pair under a budget that forces the fallback still
			// applies back.
			let bare = diff_with_budget(&old, &new, 0);
			assert_eq!(res!(apply(&old, &bare)), new, "trial {} under fallback", trial);
		}
		Ok(())
	}

	/// Random byte soup, where the line level pass has almost nothing to work
	/// with, still applies back.
	#[test]
	fn random_binary_pairs_apply_back() -> Outcome<()> {
		let mut state = 0x9E37_79B9_7F4A_7C15u64;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		for trial in 0..60 {
			let n = next() % 500;
			let old: Vec<u8> = (0..n).map(|_| (next() % 256) as u8).collect();
			let mut new = old.clone();
			for _ in 0..(next() % 8) {
				if new.is_empty() {
					new.push((next() % 256) as u8);
					continue;
				}
				let at = next() % new.len();
				match next() % 3 {
					0 => { new[at] = (next() % 256) as u8; },
					1 => { new.insert(at, (next() % 256) as u8); },
					_ => { new.remove(at); },
				}
			}
			let splices = diff(&old, &new);
			assert_eq!(res!(apply(&old, &splices)), new, "trial {}", trial);
			assert_well_formed(&old, &splices);
		}
		Ok(())
	}

	/// Lines are cut where the newlines are, the last one carrying whatever is
	/// left, and an empty file has none.
	#[test]
	fn lines_are_cut_at_the_newlines() -> Outcome<()> {
		assert_eq!(line_starts(b""), vec![0]);
		assert_eq!(line_starts(b"\n"), vec![0, 1]);
		assert_eq!(line_starts(b"a"), vec![0, 1]);
		assert_eq!(line_starts(b"a\nb\n"), vec![0, 2, 4]);
		assert_eq!(line_starts(b"a\nb"), vec![0, 2, 3]);
		Ok(())
	}
}

