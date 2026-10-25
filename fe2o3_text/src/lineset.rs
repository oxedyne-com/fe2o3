//! Text as a multiset of lines, for joining two copies of an append-mostly file.
//!
//! Written for the Daimond page log (10 Oct 2026), where two devices each appended a line to
//! the same day's log while apart and the merge kept only one side's. A row is a non-empty
//! line compared by its bytes, so `"a\r"` and `"a"` are two rows and a blank line is none. The
//! k-th equal line is its own row, so a join keeps the larger count of each line and never
//! their sum: two devices holding the same entry once hold it once after joining, while an
//! entry made twice on purpose survives as two.
//!
//! The content of a join commutes and associates; its bytes do not. The bytes follow the
//! arriving side, with the rows only here spliced in, so a join that adds nothing returns the
//! arriving bytes exactly and repeated gossip settles.

use std::collections::HashMap;


#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Joined {
	pub bytes:			Vec<u8>,
	pub here_adds:		usize,	// Rows only here had
	pub arriving_adds:	usize,	// Rows only arriving had
}

// One non-empty line: its bytes without the LF, and the offset just past its LF.
#[derive(Clone, Copy, Debug)]
struct Row<'a> {
	text:	&'a [u8],
	next:	usize,
}

fn rows(buf: &[u8]) -> Vec<Row<'_>> {
	let mut out = Vec::new();
	let mut start = 0;
	while start < buf.len() {
		let (end, next) = match buf[start..].iter().position(|b| *b == b'\n') {
			Some(i)	=> (start + i, start + i + 1),
			None	=> (buf.len(), buf.len()), // A torn fragment
		};
		if end > start {
			out.push(Row { text: &buf[start..end], next });
		}
		start = next;
	}
	out
}

fn counts<'a>(rows: &[Row<'a>]) -> HashMap<&'a [u8], usize> {
	let mut map = HashMap::new();
	for row in rows {
		*map.entry(row.text).or_insert(0) += 1;
	}
	map
}

// For each row, its occurrence number (1-based) and whether the other side holds that
// occurrence too.
fn shared<'a>(
	rows:	&[Row<'a>],
	other:	&HashMap<&'a [u8], usize>,
)
	-> Vec<(usize, bool)>
{
	let mut seen: HashMap<&[u8], usize> = HashMap::new();
	rows.iter().map(|row| {
		let k = seen.entry(row.text).or_insert(0);
		*k += 1;
		(*k, *k <= other.get(row.text).copied().unwrap_or(0))
	}).collect()
}

/// Joins two copies of a file as multisets of lines, holding each row as many times as the
/// side with more of it does.
///
/// The bytes are `arriving`'s, byte for byte when `here` has no row of its own. Otherwise each
/// row only `here` has is placed after its nearest preceding shared row (in `here`'s order) and
/// after the run of arriving-only rows that follows that row in `arriving`; a row with no
/// shared row before it goes after the arriving-only rows that open `arriving`. Rows sharing a
/// place keep `here`'s order. Each spliced row ends with LF, and an LF is added first where
/// `arriving` ends without one.
pub fn union_lines(arriving: &[u8], here: &[u8]) -> Joined {
	let rows_a = rows(arriving);
	let rows_h = rows(here);
	let count_a = counts(&rows_a);
	let count_h = counts(&rows_h);
	let shared_a = shared(&rows_a, &count_h);
	let shared_h = shared(&rows_h, &count_a);

	let arriving_adds = shared_a.iter().filter(|(_, s)| !s).count();
	let here_adds = shared_h.iter().filter(|(_, s)| !s).count();
	if here_adds == 0 {
		return Joined { bytes: arriving.to_vec(), here_adds, arriving_adds };
	}

	// Where each shared row sits in arriving, by (line, occurrence).
	let mut place: HashMap<(&[u8], usize), usize> = HashMap::new();
	for (i, row) in rows_a.iter().enumerate() {
		if shared_a[i].1 {
			place.insert((row.text, shared_a[i].0), i);
		}
	}
	// The byte offset after the run of arriving-only rows starting at index `from`.
	let after_run = |from: usize| -> usize {
		let mut i = from;
		while i < rows_a.len() && !shared_a[i].1 {
			i += 1;
		}
		if i == 0 { 0 } else { rows_a[i - 1].next }
	};

	let mut splices: Vec<(usize, &[u8])> = Vec::with_capacity(here_adds);
	let mut anchor: Option<usize> = None; // Index in arriving of the last shared row seen
	for (j, row) in rows_h.iter().enumerate() {
		let (k, is_shared) = shared_h[j];
		if is_shared {
			anchor = place.get(&(row.text, k)).copied();
		} else {
			let at = match anchor {
				Some(i)	=> after_run(i + 1),
				None	=> after_run(0),
			};
			splices.push((at, row.text));
		}
	}
	splices.sort_by_key(|(at, _)| *at); // Stable, so here's order holds within a place

	let extra: usize = splices.iter().map(|(_, t)| t.len() + 1).sum();
	let mut bytes = Vec::with_capacity(arriving.len() + extra + 1);
	let mut from = 0;
	for (at, text) in splices {
		if at > from {
			bytes.extend_from_slice(&arriving[from..at]);
			from = at;
		}
		if bytes.last().is_some_and(|b| *b != b'\n') {
			bytes.push(b'\n');
		}
		bytes.extend_from_slice(text);
		bytes.push(b'\n');
	}
	bytes.extend_from_slice(&arriving[from..]);
	Joined { bytes, here_adds, arriving_adds }
}

/// The rows of `old` that `new` does not hold, by occurrence and in `old`'s order: an `old`
/// with a line three times against a `new` with it once yields that line twice.
pub fn missing_lines(old: &[u8], new: &[u8]) -> Vec<Vec<u8>> {
	let rows_o = rows(old);
	let count_n = counts(&rows(new));
	shared(&rows_o, &count_n).into_iter()
		.zip(rows_o.iter())
		.filter(|((_, is_shared), _)| !is_shared)
		.map(|(_, row)| row.text.to_vec())
		.collect()
}
