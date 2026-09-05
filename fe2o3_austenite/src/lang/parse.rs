//! The Ingot parser: line-oriented source into the surface tree of [`ast::Item`](super::ast::Item).
//!
//! The scan is deliberately simple, one pass over the lines. A line whose first non-blank character
//! is `=` is a heading, its level the run of leading `=`. Every other non-blank line accumulates into
//! a paragraph that a blank line -- or the next heading, or end of input -- closes. Whitespace within
//! a paragraph is made insignificant here, so the line breaker downstream owns the measure. Byte
//! offsets are tracked across the raw lines so each [`Item`] carries a true source [`Span`].

use crate::ir::Span;

use super::ast::Item;

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
	items.push(Item::Paragraph { text, span: Span::new(start, end) });
	lines.clear();
}

/// Collapses every run of whitespace to a single space and trims the ends, so a paragraph's set width
/// is left to the line breaker rather than to the source's own line breaks and indentation.
fn normalise_ws(s: &str) -> String {
	s.split_whitespace().collect::<Vec<_>>().join(" ")
}
