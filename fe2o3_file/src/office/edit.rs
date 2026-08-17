//! Editing a document somebody else wrote: what the two prose vocabularies share.
//!
//! # The text a document holds is not in one place
//!
//! A sentence in a `.docx` is spread across as many `<w:t>` elements as the writer felt like -- a
//! spell-check mark, a bookmark, a language change or a single bold word splits a run -- and an `.odt`
//! spreads it across character data and `<text:span>` and `<text:s>`. So a find that looked inside one
//! element at a time would miss every phrase a writer had touched, which is most of the interesting
//! ones.
//!
//! The answer here is a [`Piece`]: one span of the source and the text it holds. A paragraph is a
//! *group* of pieces, its text is their concatenation, and a match is found in the concatenation and
//! then pushed back down onto the pieces it covered. The replacement lands whole in the piece holding
//! the START of the match, so it keeps that run's formatting, and the rest of the match is removed
//! from the pieces after it. Which is what a person doing it by hand would do.
//!
//! # Everything else in the file is copied
//!
//! Nothing here rebuilds a document. Each changed piece becomes one
//! [`Splice`](oxedyne_fe2o3_text::xml::Splice), and a splice replaces bytes -- so the comments, the
//! bookmarks, the tracked changes, the theme and the parts this code has never heard of arrive at the
//! other end exactly as they left. See [`crate::office`] on why that is the whole point of the third
//! verb.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::Span;

/// One find-and-replace, as a caller asks for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Find {
	pub find:	String,
	pub replace:	String,
	pub nth:	Option<usize>,	// 1-based; None means every occurrence
}

impl Find {

	/// A replacement of every occurrence.
	pub fn every(find: impl Into<String>, replace: impl Into<String>) -> Self {
		Self { find: find.into(), replace: replace.into(), nth: None }
	}

	/// A replacement of one occurrence, counted from one in document order.
	pub fn at(find: impl Into<String>, replace: impl Into<String>, nth: usize) -> Self {
		Self { find: find.into(), replace: replace.into(), nth: Some(nth) }
	}
}

/// One run of text in the source, and where it sits.
///
/// `span` is what a splice replaces when the text changes, and what it covers is the format's own
/// affair: a `<w:t>` is claimed whole, because a replacement with a space at either end needs an
/// attribute putting on the element, and an `.odt`'s character data is claimed as itself.
#[derive(Clone, Debug)]
pub struct Piece {
	pub span:	Span,
	pub text:	String,	// with entity references resolved
}

impl Piece {

	pub fn new(span: Span, text: impl Into<String>) -> Self {
		Self { span, text: text.into() }
	}
}

/// A piece whose text has changed, and what it now says.
#[derive(Clone, Debug)]
pub struct Change {
	pub group:	usize,	// which group it belonged to
	pub at:	usize,	// where in the group
	pub piece:	Piece,
	// Empty where the whole of it was taken by a replacement in an earlier piece, which is the
	// ordinary result of a match that spanned two runs.
	pub text:	String,
}

/// What one [`Find`] did, so a caller can refuse a `find` that matched nothing.
#[derive(Clone, Debug, Default)]
pub struct Tally {
	pub found:	usize,	// occurrences the document held
	pub changed:	usize,	// occurrences replaced
}

/// Applies a list of find-and-replace edits to grouped pieces, giving the pieces that changed.
///
/// The groups are the units a match may not cross -- a paragraph, a cell -- and they are in document
/// order, because `nth` counts occurrences through the document and not through a paragraph.
///
/// An edit whose `find` is nowhere is an ERROR NAMING THE STRING, and so is an `nth` past the end.  A
/// silent no-op is the failure mode this is written against: a caller told "the document was edited"
/// has no way to discover that one of its four replacements did nothing, and will report the document
/// as changed.
pub fn apply(groups: &[Vec<Piece>], edits: &[Find]) -> Outcome<(Vec<Change>, Vec<Tally>)> {
	// The working text of every piece, which each edit in turn reads and writes. Sequential rather
	// than parallel, so an edit sees what the one before it did -- the same rule a text editor's
	// find-and-replace follows, and the only one under which two edits on overlapping text have a
	// defined result.
	let mut now: Vec<Vec<String>> = groups.iter()
		.map(|g| g.iter().map(|p| p.text.clone()).collect())
		.collect();
	let mut tallies = Vec::with_capacity(edits.len());
	for e in edits {
		if e.find.is_empty() {
			return Err(err!(
				"An edit asked for an empty string to be found, which is every position in the \
				document at once."; Invalid, Input));
		}
		if let Some(0) = e.nth {
			return Err(err!(
				"An edit asked for occurrence 0 of '{}'. Occurrences are counted from one.", e.find;
				Invalid, Input, Range));
		}
		let mut tally = Tally::default();
		for g in 0..now.len() {
			let (joined, bounds) = join(&now[g]);
			// Found before anything is decided, so the count is of the document as this edit met it.
			let hits = hits_of(&joined, &e.find);
			if hits.is_empty() {
				continue;
			}
			let mut wanted = Vec::new();
			for h in hits {
				tally.found += 1;
				match e.nth {
					None		=> wanted.push(h),
					Some(n) if n == tally.found	=> wanted.push(h),
					Some(_)	=> {}
				}
			}
			if wanted.is_empty() {
				continue;
			}
			tally.changed += wanted.len();
			now[g] = spread(&joined, &bounds, &wanted, e.find.len(), &e.replace);
		}
		if tally.found == 0 {
			return Err(err!(
				"'{}' is not in this document, so there was nothing to replace. Nothing has been \
				changed. Read the document and quote a phrase it actually holds -- and remember that \
				a writer's own formatting splits a sentence into runs, so a phrase broken by a \
				footnote or a field will not be found as one string.", e.find;
				Invalid, Input, NotFound));
		}
		if let Some(n) = e.nth {
			if tally.changed == 0 {
				return Err(err!(
					"Occurrence {} of '{}' was asked for and the document holds {}. Nothing has been \
					changed.", n, e.find, tally.found; Invalid, Input, Range));
			}
		}
		tallies.push(tally);
	}
	// Only what actually moved becomes a splice, so a document whose edits all replaced text with
	// itself is written back as the bytes it arrived as.
	let mut out = Vec::new();
	for (g, group) in groups.iter().enumerate() {
		for (i, piece) in group.iter().enumerate() {
			if now[g][i] != piece.text {
				out.push(Change {
					group:	g,
					at:	i,
					piece:	piece.clone(),
					text:	now[g][i].clone(),
				});
			}
		}
	}
	Ok((out, tallies))
}

/// The group's text, and where each piece starts and ends within it.
fn join(pieces: &[String]) -> (String, Vec<Span>) {
	let mut joined = String::new();
	let mut bounds = Vec::with_capacity(pieces.len());
	for p in pieces {
		let start = joined.len();
		joined.push_str(p);
		bounds.push(start..joined.len());
	}
	(joined, bounds)
}

/// Where a needle occurs, non-overlapping, left to right.
fn hits_of(haystack: &str, needle: &str) -> Vec<usize> {
	let mut out = Vec::new();
	let mut from = 0;
	while let Some(k) = haystack[from..].find(needle) {
		let at = from + k;
		out.push(at);
		from = at + needle.len();
	}
	out
}

/// The group's pieces, with the matches replaced.
///
/// The replacement goes in the piece holding the match's START -- so it wears that run's formatting,
/// which is the formatting of the first character of what was replaced -- and the pieces after it lose
/// only the part of the match they held.
fn spread(
	joined:	&str,
	bounds:	&[Span],
	hits:	&[usize],
	len:	usize,
	replace:	&str,
)
	-> Vec<String>
{
	let mut out = Vec::with_capacity(bounds.len());
	for b in bounds {
		let mut text = String::new();
		let mut at = b.start;
		for hit in hits {
			let (s, e) = (*hit, hit + len);
			if e <= b.start || s >= b.end {
				continue;
			}
			let from = s.max(b.start);
			let to = e.min(b.end);
			if at < from {
				text.push_str(&joined[at..from]);
			}
			// The piece holding the start takes the replacement; the others take nothing, which is
			// how a match spread over three runs collapses into the first of them.
			if s >= b.start {
				text.push_str(replace);
			}
			at = to;
		}
		if at < b.end {
			text.push_str(&joined[at..b.end]);
		}
		out.push(text);
	}
	out
}
