//! The Ingot parser: line-oriented source into the surface tree of [`ast::Item`](super::ast::Item).
//!
//! The scan is deliberately simple, one pass over the lines. A line whose first non-blank character
//! is `=` is a heading, its level the run of leading `=`. Every other non-blank line accumulates into
//! a paragraph that a blank line -- or the next heading, or end of input -- closes. Whitespace within
//! a paragraph is made insignificant here, so the line breaker downstream owns the measure. Byte
//! offsets are tracked across the raw lines so each [`Item`] carries a true source [`Span`].
//!
//! A closed paragraph's text is then scanned for inline emphasis by [`parse_inlines`]: `*strong*` and
//! `/emph/`. A delimiter pairs only when it flanks a word, so a stray asterisk, a date's slash, or
//! `and/or` is left as ordinary text rather than opening an emphasis that never closes.

use crate::ir::Span;

use super::ast::{Inline, Item};

use oxedyne_fe2o3_core::prelude::*;

/// Parses a whole Ingot source string into its surface items. The only error is an empty heading --
/// a `=` marker with no title -- which names the offending 1-based line.
pub fn document(src: &str) -> Outcome<Vec<Item>> {
	let mut items:		Vec<Item>	= Vec::new();
	let mut lines:		Vec<String>	= Vec::new();	// the current paragraph's constituent lines
	let mut para_start:	u32			= 0;			// byte offset of the paragraph's first line
	let mut para_end:	u32			= 0;			// byte offset just past its last line's content
	let mut offset:		u32			= 0;			// running byte offset of the current line's start
	let mut line_no					= 0usize;		// 1-based, for a diagnostic

	// `split_inclusive` keeps the trailing newline on each piece, so the running offset stays a true
	// byte position into the source rather than drifting by the count of stripped terminators.
	for raw in src.split_inclusive('\n') {
		line_no += 1;
		let start = offset;
		offset = offset.saturating_add(raw.len() as u32);

		// Strip the line terminator without consuming a real character: the final line may carry
		// neither a newline nor a carriage return.
		let mut line = raw;
		if let Some(s) = line.strip_suffix('\n') { line = s; }
		if let Some(s) = line.strip_suffix('\r') { line = s; }
		let end = start.saturating_add(line.len() as u32);

		let trimmed = line.trim_start();
		if trimmed.is_empty() {
			// A blank line closes the paragraph it follows.
			flush_para(&mut items, &mut lines, para_start, para_end);
		} else if trimmed.starts_with('=') {
			// A heading closes any paragraph above it, then stands on its own line.
			flush_para(&mut items, &mut lines, para_start, para_end);
			let level = trimmed.chars().take_while(|&c| c == '=').count();
			let title = trimmed[level..].trim();	// '=' is ASCII, so a byte slice at the count is safe
			if title.is_empty() {
				return Err(err!(
					"Empty heading on line {}: a `=` marker must be followed by a title.", line_no;
					Input, Invalid, Missing));
			}
			items.push(Item::Heading {
				level:	level as u8,
				text:	title.to_string(),
				span:	Span::new(start, end),
			});
		} else {
			// Any other non-blank line joins the running paragraph; its own line break and indentation
			// carry no meaning, only its words.
			if lines.is_empty() {
				para_start = start;
			}
			lines.push(line.to_string());
			para_end = end;
		}
	}

	// A source that ends without a closing blank line still closes its last paragraph.
	flush_para(&mut items, &mut lines, para_start, para_end);
	Ok(items)
}

/// Closes the paragraph being accumulated, if any: its lines are joined, their whitespace collapsed,
/// and the result pushed as one [`Item::Paragraph`] spanning the source it came from. An empty
/// accumulator flushes nothing, so a run of blank lines closes a paragraph only once.
fn flush_para(
	items:	&mut Vec<Item>,
	lines:	&mut Vec<String>,
	start:	u32,
	end:	u32,
)
{
	if lines.is_empty() {
		return;
	}
	let text = normalise_ws(&lines.join(" "));
	let runs = parse_inlines(&text);
	items.push(Item::Paragraph { runs, span: Span::new(start, end) });
	lines.clear();
}

/// Collapses every run of whitespace to a single space and trims the ends, so a paragraph's set width
/// is left to the line breaker rather than to the source's own line breaks and indentation.
fn normalise_ws(s: &str) -> String {
	s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits a whitespace-collapsed paragraph into inline runs, recognising `*strong*` and `/emph/`. A
/// delimiter opens emphasis only when it flanks the left of a word -- the start of the paragraph,
/// whitespace, or an opening bracket precedes it and a non-space follows -- and closes only when it
/// flanks the right -- a non-space precedes it and the end, whitespace, or closing punctuation follows.
/// An unpaired delimiter is ordinary text, so `5 * 3`, `12/25` and `and/or` are left untouched. Nesting
/// is a later increment: the first valid closer ends the run, and any delimiter inside it is literal.
fn parse_inlines(text: &str) -> Vec<Inline> {
	let chars:	Vec<char>	= text.chars().collect();
	let n					= chars.len();
	let mut runs:	Vec<Inline>	= Vec::new();
	let mut plain			= String::new();	// ordinary text gathered before the next emphasis run
	let mut i				= 0usize;
	while i < n {
		let c = chars[i];
		if (c == '*' || c == '/') && is_opener(&chars, i) {
			if let Some(close) = find_closer(&chars, i + 1, c) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				let inner: String = chars[i + 1..close].iter().collect();
				runs.push(if c == '*' { Inline::Strong(inner) } else { Inline::Emph(inner) });
				i = close + 1;
				continue;
			}
		}
		plain.push(c);
		i += 1;
	}
	if !plain.is_empty() {
		runs.push(Inline::Text(plain));
	}
	if runs.is_empty() {
		runs.push(Inline::Text(String::new()));	// a paragraph of pure delimiters keeps one empty run
	}
	runs
}

/// Does the delimiter at `i` flank the left of a word? A non-space must follow it, and the start of the
/// paragraph, whitespace, or an opening bracket must precede it.
fn is_opener(chars: &[char], i: usize) -> bool {
	match chars.get(i + 1) {
		Some(c) if !c.is_whitespace()	=> {},
		_								=> return false,
	}
	match i.checked_sub(1).and_then(|p| chars.get(p)) {
		None		=> true,
		Some(&p)	=> p.is_whitespace() || matches!(p, '(' | '[' | '{' | '"' | '\''),
	}
}

/// Does the delimiter at `j` flank the right of a word? A non-space must precede it, and the end of the
/// paragraph, whitespace, or closing punctuation must follow.
fn is_closer(chars: &[char], j: usize) -> bool {
	match j.checked_sub(1).and_then(|p| chars.get(p)) {
		Some(p) if !p.is_whitespace()	=> {},
		_								=> return false,
	}
	match chars.get(j + 1) {
		None		=> true,
		Some(&c)	=> c.is_whitespace()
			|| matches!(c, ')' | ']' | '}' | '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\''),
	}
}

/// The index of the first valid closing `delim` at or after `start`, or `None` when the run never
/// closes -- in which case the opener is ordinary text.
fn find_closer(chars: &[char], start: usize, delim: char) -> Option<usize> {
	(start..chars.len()).find(|&j| chars[j] == delim && is_closer(chars, j))
}
