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
//! Two levels, over pieces and then over bytes.
//!
//! **Pieces first.** The bytes are cut into pieces, each distinct piece is given
//! a number, and Myers' O(ND) algorithm runs over the numbers. This is what keeps
//! a large file affordable, since the sequences it compares are as long as the
//! file has pieces rather than bytes, and the cost is driven by the size of the
//! difference rather than the size of the file.
//!
//! **Bytes second.** Each run of changed pieces is trimmed of the bytes its two
//! sides share at either end, and, where what remains is small, Myers runs again
//! over those bytes. A one character change in the middle of a line therefore
//! stores one character, not the line and not the file.
//!
//! # Where the pieces come from
//!
//! There are two ways of cutting, and which is used is [`Route`].
//!
//! For text, a piece is a **line**: the bytes are cut at every `\n`. That is the
//! right unit for anything a person edits a line at a time, and it costs one pass
//! to find.
//!
//! For everything else, a piece is a **content-defined chunk**. A gear rolling
//! hash reads the bytes, and a boundary is declared wherever the hash of the last
//! few dozen bytes hits a mask, so a boundary follows the content rather than an
//! offset: an edit perturbs only the chunks around it, and every chunk beyond the
//! disturbance re-synchronises on the same bytes and keeps the number it had.
//! That is what makes a small edit to a large binary cost the region it touched
//! rather than everything between the first edit and the last. Boundaries are
//! held between [`MIN_CHUNK`] and [`MAX_CHUNK`] and steered towards [`AVG_CHUNK`]
//! by normalised chunking, all three tuned well below what a storage layer would
//! use: a diff wants to localise an edit to a few kilobytes, not to a quarter of
//! a megabyte.
//!
//! The route is chosen by asking whether the line pass has anything to work with.
//! Content whose average line is longer than [`MAX_TEXT_LINE`] is one long line as
//! far as that pass is concerned, and where such content is also longer than
//! [`MIN_CHUNKED_LEN`] the line pass is skipped and the chunk pass runs instead.
//! Below that length nothing is chunked, because a single splice over so few bytes
//! costs less than the operations that would replace it.
//!
//! # The bound on cost
//!
//! Myers costs O(ND) in time and, as written here, O(D²) in memory, where D is
//! the length of the edit script. Both are fine while D is small and neither is
//! acceptable when it is not, so D is capped at [`MAX_PIECE_SCRIPT`] insertions
//! and deletions over pieces, and [`MAX_BYTE_SCRIPT`] within a refined run. A
//! piece script that would exceed its cap means the two versions have little left
//! in common. The line pass then hands over to the chunk pass where the content is
//! long enough to chunk, and beyond that the answer falls back to a single splice
//! covering everything between the shared prefix and the shared suffix -- which is
//! the whole of what a diff-free frontend would have recorded anyway, arrived at
//! in linear time. The cap on the piece script bounds the working memory at
//! roughly four megabytes.
//!
//! # Bytes, not text
//!
//! Nothing here decodes. A piece is compared to another by its bytes, so an
//! encoding this module has never heard of costs it nothing, and no route is
//! chosen by guessing what the content is: the question asked is only whether the
//! newlines are frequent enough to cut on, which is a fact about the bytes.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::ops::Range;


// Greatest piece level edit script the diff will compute on one route before
// handing on to the next. A piece is a line on one route and a content-defined
// chunk on the other, and the cap is the same for both: what it bounds is the
// working memory of the Myers pass, which does not care what it is comparing.
pub const MAX_PIECE_SCRIPT: usize = 1024;

// Greatest byte level edit script the refinement of one changed run will
// compute before leaving the run as a single splice.
pub const MAX_BYTE_SCRIPT: usize = 256;

// Greatest changed run, in bytes on either side, the byte level refinement is
// attempted on at all.
pub const MAX_REFINE_LEN: usize = 4096;

// Fewest unchanged bytes that may separate two splices. Two operations each
// carry their own identity, parents and origins, which costs more than a
// handful of bytes that did not change; splices closer together than this are
// merged into one. Keeping the gap non-zero is also what stops one splice
// anchoring on content that another removes.
pub const MIN_SPLICE_GAP: usize = 16;

// A floor stops a run of unlucky hash hits producing a swarm of tiny pieces,
// each of which costs an entry in the sequence Myers runs over.
pub const MIN_CHUNK: usize = 2 * 1024;

// The granularity at which an edit is localised before the byte level
// refinement narrows it further, so it is the bound on what a diff re-stores
// when the refinement declines. A storage layer chunks an order of magnitude
// larger, because its cost is one address per chunk in every manifest; here the
// cost is one integer per chunk in one comparison, which buys the finer cut.
pub const AVG_CHUNK: usize = 8 * 1024;

// A ceiling bounds the damage where the hash finds no boundary at all, which is
// what a long run of identical bytes does to it.
pub const MAX_CHUNK: usize = 64 * 1024;

// Shortest input the chunk route is used on. Below this a single splice
// covering everything between the shared prefix and the shared suffix stores at
// most sixteen kilobytes, which is of the same order as the operations that
// would replace it, so there is nothing to win.
pub const MIN_CHUNKED_LEN: usize = 16 * 1024;

// Longest average line at which the line route is still worth taking. Above it
// the newlines are too sparse to cut on: the pieces are so long that a changed
// piece is most of the file, which is the trimmed splice with extra steps.
pub const MAX_TEXT_LINE: usize = 1024;

// Seed for the gear table's generator. The table has to be the same in every
// build, because a changed table changes every boundary and so which bytes a
// diff decides to re-store. Nothing here is content-addressed, so a changed
// table would cost efficiency rather than correctness -- the applier checks the
// answer either way -- but a diff that depends on which binary produced it is
// not a diff anyone can reason about. Fixing the seed here, and deriving the
// table from it rather than shipping a literal, is what pins it. The value is
// the golden-ratio constant splitmix64 conventionally uses.
const GEAR_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

// Normalisation level: how many bits the mask tightens below the average chunk
// size and loosens above it. Plain gear chunking gives an exponential spread of
// chunk sizes, so short chunks dominate and the tail is long. Cutting less
// readily before the average and more readily after it pulls the spread in
// towards the average without forcing boundaries at fixed offsets. Two is the
// level the FastCDC paper settles on.
const NORM_LEVEL: u32 = 2;

// The 256-entry gear table, one pseudorandom `u64` per byte value, generated at
// compile time from `GEAR_SEED` by splitmix64 so there is no dependency to pull
// in and no table to keep in the source.
static GEAR: [u64; 256] = gear_table();

// The masks a boundary must hit: the stricter below the average chunk size, the
// looser at and above it.
const MASK_S: u64 = high_mask(log2_floor(AVG_CHUNK) + NORM_LEVEL);
const MASK_L: u64 = high_mask(log2_floor(AVG_CHUNK) - NORM_LEVEL);


/// Which of the routes through the diff was taken.
///
/// Returned by [`diff_routed`] so that a caller measuring what a diff cost, or a
/// test asserting which pass ran, does not have to infer it from the shape of the
/// answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
	Same,	// the two versions are the same bytes and there is nothing to do
	Line,	// pieces cut at the newlines, then bytes within each changed run
	Chunk,	// pieces cut where a gear hash says the content changes, then bytes
	Whole,	// between the shared prefix and the shared suffix, where both give up
}

impl Route {
	pub fn name(&self) -> &'static str {
		match self {
			Self::Same	=> "Same",
			Self::Line	=> "Line",
			Self::Chunk	=> "Chunk",
			Self::Whole	=> "Whole",
		}
	}
}


/// One positional edit against the old bytes: remove `delete` bytes at `at`,
/// and put `insert` in their place.
///
/// The three fields are independent -- any offset, any length and any payload
/// name an edit, even where the file cannot supply them -- so there is no
/// invariant here for a struct literal to break. What the offsets have to agree
/// with is the file, and [`apply`] is what says whether they do.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Splice {
	pub at:		usize,
	pub delete:	usize,
	pub insert:	Vec<u8>,
}

impl Splice {
	pub fn insert_len(&self) -> usize {
		self.insert.len()
	}
}


/// The list is ascending by offset and non-overlapping, and is empty when the
/// two are already the same.
pub fn diff(old: &[u8], new: &[u8]) -> Vec<Splice> {
	diff_with_budget(old, new, MAX_PIECE_SCRIPT)
}

/// As [`diff`], with the piece level edit script capped at `budget` rather than
/// at [`MAX_PIECE_SCRIPT`].
///
/// A budget of zero forces the single trimmed splice: no piece route can produce
/// a script of no steps for two versions that differ, so both give up and the
/// fallback is what is left.
pub fn diff_with_budget(old: &[u8], new: &[u8], budget: usize) -> Vec<Splice> {
	diff_routed(old, new, budget).0
}

/// The routes are tried in the order the content deserves. Where the newlines
/// are frequent enough to cut on, lines are tried first, because a line is the
/// unit an author edits in and cutting there gives the finest changed runs. Where
/// they are not, and the content is long enough for it to be worth the pass,
/// content-defined chunks are tried instead, and are tried anyway when the line
/// route exhausts its budget. What is left is the single trimmed splice.
pub fn diff_routed(old: &[u8], new: &[u8], budget: usize)
	-> (Vec<Splice>, Route)
{
	if old == new {
		return (Vec::new(), Route::Same);
	}
	let old_lines = line_starts(old);
	let new_lines = line_starts(new);
	// Long enough that chunking has room to localise an edit within it.
	let chunkable = old.len().max(new.len()) >= MIN_CHUNKED_LEN;
	// The line route is skipped only where it has nothing to work with *and*
	// there is a chunk route to skip it in favour of.
	let texty = texty(old, &old_lines) && texty(new, &new_lines);
	if texty || !chunkable {
		if let Some(sp) = diff_over(old, new, &old_lines, &new_lines, budget) {
			return (sp, Route::Line);
		}
	}
	if chunkable {
		let old_chunks = chunk_starts(old);
		let new_chunks = chunk_starts(new);
		if let Some(sp) = diff_over(old, new, &old_chunks, &new_chunks, budget) {
			return (sp, Route::Chunk);
		}
	}
	(vec![trimmed_splice(old, new)], Route::Whole)
}

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


fn common_prefix(a: &[u8], b: &[u8]) -> usize {
	let limit = a.len().min(b.len());
	let mut n = 0;
	while n < limit && a[n] == b[n] {
		n += 1;
	}
	n
}

/// The `floor` bytes already accounted for at the front are never run back past.
fn common_suffix(a: &[u8], b: &[u8], floor: usize) -> usize {
	let limit = (a.len() - floor).min(b.len() - floor);
	let mut n = 0;
	while n < limit && a[a.len() - 1 - n] == b[b.len() - 1 - n] {
		n += 1;
	}
	n
}

/// The single splice covering everything between the shared prefix and the
/// shared suffix, which is what a frontend with no diff at all records.
fn trimmed_splice(old: &[u8], new: &[u8]) -> Splice {
	let front = common_prefix(old, new);
	let back = common_suffix(old, new, front);
	Splice {
		at:		front,
		delete:	old.len() - front - back,
		insert:	new[front..new.len() - back].to_vec().into(),
	}
}

/// Runs one piece level pass and the byte level refinement beneath it, or gives
/// `None` where the edit script over the pieces would exceed `budget`.
///
/// Both routes are this function with different cut points, which is the whole of
/// what separates them: what a piece is changes, and nothing else does.
fn diff_over(
	old:		&[u8],
	new:		&[u8],
	old_starts:	&[usize],
	new_starts:	&[usize],
	budget:		usize,
)
	-> Option<Vec<Splice>>
{
	let mut names: HashMap<&[u8], u32> = HashMap::new();
	let old_ids = piece_ids(old, old_starts, &mut names);
	let new_ids = piece_ids(new, new_starts, &mut names);
	let changes = match changed_runs(&old_ids, &new_ids, budget) {
		Some(c)	=> c,
		None	=> return None,
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
	Some(out)
}

/// Are the newlines frequent enough for the line route to have anything to cut
/// on?
///
/// An empty side is no obstacle: it has no lines, and it is the other side that
/// decides. A side of one line has nothing for the pass to match against, and a
/// side whose lines average more than [`MAX_TEXT_LINE`] bytes has pieces so
/// coarse that a changed piece is most of the file.
fn texty(bytes: &[u8], starts: &[usize]) -> bool {
	if bytes.is_empty() {
		return true;
	}
	let lines = starts.len() - 1;
	if lines < 2 {
		return false;
	}
	bytes.len() / lines <= MAX_TEXT_LINE
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

/// Returns the offset at which each content-defined chunk begins, under the same
/// contract [`line_starts`] keeps: the length of the input is the final entry,
/// and an empty input has no chunks.
fn chunk_starts(bytes: &[u8]) -> Vec<usize> {
	let mut starts = vec![0usize];
	let mut at = 0usize;
	while at < bytes.len() {
		at += cut(&bytes[at..]);
		starts.push(at);
	}
	starts
}

/// Returns the length of the first chunk of `data`, always at least one byte so
/// that [`chunk_starts`] terminates.
///
/// The gear hash reads one byte at a time, keeping a value that depends only on
/// the last few dozen bytes, and a boundary is declared where that value hits the
/// mask. The bytes before [`MIN_CHUNK`] are not hashed at all, no boundary being
/// acceptable there, which is the cut-point skipping that makes the scan cheap.
/// The mask is the stricter of the two below [`AVG_CHUNK`] and the looser at and
/// above it, which pulls the spread of chunk sizes in towards the average.
fn cut(data: &[u8]) -> usize {
	let n = data.len();
	if n <= MIN_CHUNK {
		return n;	// too short to cut: the whole remainder is one chunk
	}
	let end = MAX_CHUNK.min(n);		// forced boundary
	let mid = AVG_CHUNK.min(end);	// where the mask loosens
	let mut fp = 0u64;				// the rolling gear hash
	let mut i = MIN_CHUNK;			// skip: no boundary may land below
	while i < mid {
		fp = (fp << 1).wrapping_add(GEAR[data[i] as usize]);
		if fp & MASK_S == 0 {
			return i + 1;
		}
		i += 1;
	}
	while i < end {
		fp = (fp << 1).wrapping_add(GEAR[data[i] as usize]);
		if fp & MASK_L == 0 {
			return i + 1;
		}
		i += 1;
	}
	end
}

/// Splitmix64 is a handful of multiplies and shifts, which is why it can run in
/// a `const` context and save the crate a dependency and a 2 KiB literal.
const fn gear_table() -> [u64; 256] {
	let mut table = [0u64; 256];
	let mut state = GEAR_SEED;
	let mut i = 0;
	while i < 256 {
		state = state.wrapping_add(GEAR_SEED);
		let mut z = state;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		table[i] = z ^ (z >> 31);
		i += 1;
	}
	table
}

/// Builds a mask over the top `bits` bits of a `u64`, for `1 <= bits <= 63`.
///
/// The top bits are the ones to test. A gear hash shifts left by one per byte,
/// so bit `k` of the hash depends on the last `k + 1` bytes; testing the high
/// bits therefore tests a window dozens of bytes wide, while testing the low bits
/// would decide a boundary on almost nothing.
const fn high_mask(bits: u32) -> u64 {
	((1u64 << bits) - 1) << (64 - bits)
}

/// Floor of the base-2 logarithm of a non-zero value.
const fn log2_floor(n: usize) -> u32 {
	(usize::BITS - 1) - n.leading_zeros()
}

/// Numbers the pieces, giving equal pieces equal numbers across both files.
///
/// The numbering is what the piece level pass compares, so that a comparison
/// costs one integer rather than the length of a piece. Equality is over the
/// bytes themselves rather than over a digest of them, so two pieces that differ
/// can never be given one number and no route can be led into a wrong answer by
/// a collision.
fn piece_ids<'a>(bytes: &'a [u8], starts: &[usize], names: &mut HashMap<&'a [u8], u32>)
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
	a:	Range<usize>,	// what the run was, in the old sequence
	b:	Range<usize>,	// what it has become, in the new sequence
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
					insert:	new[na + c.b.start..na + c.b.end].to_vec().into(),
				});
			}
			return;
		}
	}
	out.push(Splice {
		at:		oa,
		delete:	ob - oa,
		insert:	new[na..nb].to_vec().into(),
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
	/// to the old bytes, produce the new bytes exactly.
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

	fn inserted(splices: &[Splice]) -> usize {
		splices.iter().map(|s| s.insert_len()).sum()
	}

	fn deleted(splices: &[Splice]) -> usize {
		splices.iter().map(|s| s.delete).sum()
	}

	#[test]
	fn identical_inputs_diff_to_nothing() -> Outcome<()> {
		for s in [&b""[..], b"a", b"a\n", b"one\ntwo\nthree\n", b"no trailing newline"] {
			assert!(res!(diffed(s, s)).is_empty(), "{:?}", Bytes(s));
		}
		Ok(())
	}

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

	/// The module makes no claim to notice a move; the operation vocabulary has
	/// one, and finding it is not this module's business.
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

	#[test]
	fn apply_refuses_a_list_that_does_not_fit() -> Outcome<()> {
		let old = b"alpha\nbeta\n";
		assert!(apply(old, &[Splice { at: 12, delete: 0, insert: Vec::new() }]).is_err());
		assert!(apply(old, &[Splice { at: 0, delete: 12, insert: Vec::new() }]).is_err());
		assert!(apply(old, &[
			Splice { at: 6, delete: 1, insert: Vec::new() }.into(),
			Splice { at: 2, delete: 1, insert: Vec::new() }.into(),
		]).is_err());
		assert!(apply(old, &[
			Splice { at: 0, delete: 4, insert: Vec::new() }.into(),
			Splice { at: 2, delete: 1, insert: Vec::new() }.into(),
		]).is_err(), "overlapping splices");
		assert!(apply(old, &[
			Splice { at: usize::MAX, delete: usize::MAX, insert: Vec::new() }.into(),
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

	/// A pseudorandom byte string of `n` bytes with no newline in it, which is
	/// what a compressed or otherwise structureless payload looks like to the
	/// line pass: one enormous line.
	fn binary(n: usize, seed: u64) -> Vec<u8> {
		let mut state = seed;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		(0..n).map(|_| {
			let b = (next() % 255) as u8;
			if b >= b'\n' { b + 1 } else { b }
		}).collect()
	}

	/// The route is chosen by what the content is, not by what it is called.
	#[test]
	fn the_route_follows_the_content() -> Outcome<()> {
		// Nothing to do.
		assert_eq!(diff_routed(b"same", b"same", MAX_PIECE_SCRIPT).1, Route::Same);
		// Text, of any size, cuts at the newlines.
		let mut lines = String::new();
		for i in 0..4000 {
			lines.push_str(&fmt!("line {} of a perfectly ordinary text file\n", i));
		}
		let old = lines.clone().into_bytes();
		let new = lines.replace("line 2000 ", "line 2000! ").into_bytes();
		assert!(old.len() > MIN_CHUNKED_LEN, "large enough to have a choice");
		assert_eq!(diff_routed(&old, &new, MAX_PIECE_SCRIPT).1, Route::Line);
		// Newline-poor content long enough to chunk takes the chunk route.
		let old = binary(MIN_CHUNKED_LEN * 4, 0x2545_F491_4F6C_DD1D);
		let mut new = old.clone();
		new[MIN_CHUNKED_LEN] ^= 0xff;
		assert_eq!(diff_routed(&old, &new, MAX_PIECE_SCRIPT).1, Route::Chunk);
		// The same content, too short to be worth chunking, falls to one splice.
		let short = &old[..MIN_CHUNKED_LEN - 1];
		let mut new = short.to_vec();
		new[10] ^= 0xff;
		new[MIN_CHUNKED_LEN - 100] ^= 0xff;
		let (sp, route) = diff_routed(short, &new, MAX_PIECE_SCRIPT);
		assert_eq!(route, Route::Line, "one line, refined by the byte pass");
		assert_eq!(res!(apply(short, &sp)), new);
		// And a budget no piece route can meet falls through both of them.
		assert_eq!(diff_routed(&old, &new, 0).1, Route::Whole);
		Ok(())
	}

	#[test]
	fn an_exhausted_line_budget_hands_on_to_the_chunks() -> Outcome<()> {
		// Every other line changes, over enough lines that a small budget cannot
		// describe it, in a file long enough to chunk.
		let mut old = String::new();
		let mut new = String::new();
		for i in 0..900 {
			if i % 2 == 0 {
				old.push_str(&fmt!("old line {}, with a few words after it\n", i));
				new.push_str(&fmt!("new line {}, with a few words after it\n", i));
			} else {
				let same = fmt!("line {} is left exactly as it was, unchanged\n", i);
				old.push_str(&same);
				new.push_str(&same);
			}
		}
		let (old, new) = (old.into_bytes(), new.into_bytes());
		assert!(old.len() > MIN_CHUNKED_LEN);
		assert_eq!(diff_routed(&old, &new, MAX_PIECE_SCRIPT).1, Route::Line,
			"a full budget stays on the line route");
		let (sp, route) = diff_routed(&old, &new, 16);
		assert_eq!(route, Route::Chunk, "a starved line pass is not the end of it");
		assert_eq!(res!(apply(&old, &sp)), new);
		assert_well_formed(&old, &sp);
		Ok(())
	}

	/// This is the property the whole route rests on, and it is the one thing a
	/// fixed-size chunker cannot do: there, one inserted byte shifts every
	/// boundary after it and nothing downstream matches anything.
	#[test]
	fn a_chunk_boundary_follows_the_content() -> Outcome<()> {
		let old = binary(1 << 20, 0x9E37_79B9_7F4A_7C15);
		let mut new = old.clone();
		new.insert(1000, 0x42);
		let before = chunk_starts(&old);
		let after = chunk_starts(&new);
		assert!(before.len() > 40, "a megabyte should cut into many chunks");
		// Every boundary beyond the disturbance is one byte along from where it
		// was, which is to say it is the same place in the content.
		let shifted: Vec<usize> = after.iter()
			.filter(|b| **b > MAX_CHUNK)
			.map(|b| b - 1)
			.collect();
		let kept = before.iter().filter(|b| shifted.contains(b)).count();
		assert!(
			kept as f64 > 0.9 * shifted.len() as f64,
			"only {} of {} boundaries re-synchronised", kept, shifted.len(),
		);
		Ok(())
	}

	/// This is the claim the feature table makes, measured. The comparison is
	/// against what the same pair costs without the chunk route, which is the
	/// whole span between the outermost edits.
	#[test]
	fn scattered_edits_in_a_large_binary_cost_their_regions() -> Outcome<()> {
		let old = binary(4 << 20, 0x0f1e_2d3c_4b5a_6978);
		let mut new = old.clone();
		for at in [100_000usize, 2_000_000, 4_000_000] {
			new[at] ^= 0xff;
		}
		let (sp, route) = diff_routed(&old, &new, MAX_PIECE_SCRIPT);
		assert_eq!(route, Route::Chunk);
		assert_eq!(res!(apply(&old, &sp)), new);
		assert_well_formed(&old, &sp);
		assert_eq!(sp.len(), 3, "one region per edit");
		let cost = inserted(&sp) + deleted(&sp);
		// The bound the route guarantees is three refined regions, so at worst
		// three chunks; a few hundred kilobytes was the claim.
		assert!(
			cost < 3 * AVG_CHUNK,
			"three edits in four megabytes cost {} bytes", cost,
		);
		// What it actually costs is the three bytes, because trimming what the
		// two sides of a changed chunk share finds the edit exactly. Frozen,
		// because a change to the chunker that quietly stopped doing this is
		// precisely what this test is for.
		assert_eq!(cost, 6, "three bytes replaced by three bytes");
		// What the same pair costs with no piece route at all: everything between
		// the first change and the last.
		let bare = trimmed_splice(&old, &new);
		assert!(bare.insert.len() > 3_800_000, "the whole point of the exercise");
		Ok(())
	}

	/// Randomised binary pairs with edits scattered through them, all applied
	/// back: the oracle is that the result is the new bytes exactly, on the route
	/// that produced them.
	#[test]
	fn random_binary_pairs_on_the_chunk_route_apply_back() -> Outcome<()> {
		let mut state = 0x1234_5678_9abc_def0u64;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		let mut chunked = 0;
		for trial in 0..24 {
			let n = MIN_CHUNKED_LEN + next() % (200 * 1024);
			let old = binary(n, 0xdead_beef_0000_0000 + trial as u64);
			let mut new = old.clone();
			// A handful of edits at unrelated offsets, of every kind.
			for _ in 0..1 + next() % 8 {
				if new.is_empty() {
					break;
				}
				let at = next() % new.len();
				match next() % 4 {
					0 => { new[at] ^= 0xff; },
					1 => {
						let run: Vec<u8> = (0..1 + next() % 900)
							.map(|k| (at + k) as u8 | 1)
							.collect();
						let tail = new.split_off(at);
						new.extend_from_slice(&run);
						new.extend_from_slice(&tail);
					},
					2 => {
						let len = (next() % 3000).min(new.len() - at);
						new.drain(at..at + len);
					},
					_ => {
						// Move a run to another offset, which the diff has no verb
						// for and must describe as a deletion and an insertion.
						let len = (next() % 1500).min(new.len() - at);
						let run: Vec<u8> = new.drain(at..at + len).collect();
						let to = if new.is_empty() { 0 } else { next() % new.len() };
						let tail = new.split_off(to);
						new.extend_from_slice(&run);
						new.extend_from_slice(&tail);
					},
				}
			}
			let (sp, route) = diff_routed(&old, &new, MAX_PIECE_SCRIPT);
			assert_eq!(res!(apply(&old, &sp)), new, "trial {} on {}", trial, route.name());
			assert_well_formed(&old, &sp);
			if route == Route::Chunk {
				chunked += 1;
			}
			// And under a budget that forces the fallback, it still applies back.
			let (bare, route) = diff_routed(&old, &new, 0);
			assert_eq!(route, Route::Whole);
			assert_eq!(res!(apply(&old, &bare)), new, "trial {} under fallback", trial);
		}
		assert_eq!(chunked, 24, "every one of these is newline-free and large");
		Ok(())
	}

	/// Chunks are cut where the content says, the last one carrying whatever is
	/// left, and an empty input has none.
	#[test]
	fn chunks_are_cut_within_their_bounds() -> Outcome<()> {
		assert_eq!(chunk_starts(b""), vec![0]);
		assert_eq!(chunk_starts(b"short"), vec![0, 5]);
		let bytes = binary(1 << 20, 0xabcd_ef01_2345_6789);
		let starts = chunk_starts(&bytes);
		assert_eq!(starts[0], 0);
		assert_eq!(starts[starts.len() - 1], bytes.len());
		for (i, w) in starts.windows(2).enumerate() {
			let len = w[1] - w[0];
			assert!(len > 0, "chunk {} is empty", i);
			assert!(len <= MAX_CHUNK, "chunk {} is {} bytes", i, len);
			// Every chunk but the last reaches the minimum.
			if i + 2 < starts.len() {
				assert!(len > MIN_CHUNK, "chunk {} is {} bytes", i, len);
			}
		}
		// A run of identical bytes offers the hash nothing, so the ceiling is
		// what cuts it.
		let flat = vec![0x5au8; 5 * MAX_CHUNK];
		let starts = chunk_starts(&flat);
		assert_eq!(starts.len(), 6);
		assert_eq!(starts[1], MAX_CHUNK);
		Ok(())
	}
}

