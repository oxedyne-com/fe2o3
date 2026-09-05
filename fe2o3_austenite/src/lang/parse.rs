//! The Ingot parser: line-oriented source into the surface tree of [`ast::Item`](super::ast::Item).
//!
//! The scan is deliberately simple, one pass over the lines. A line whose first non-blank character
//! is `=` is a heading, its level the run of leading `=`; a line opening with `-` or `+` (and a space)
//! is a list item, a run of them a bullet or numbered list; every other non-blank line accumulates into
//! a paragraph. A blank line, a heading, or the start of the other kind of block closes whatever is
//! open. Whitespace within a paragraph is made insignificant here, so the line breaker downstream owns
//! the measure. Byte offsets are tracked across the raw lines so each [`Item`] carries a true [`Span`].
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

	// The current list, if one is open: its kind, its items so far, and the source it spans. A list is a
	// run of consecutive marker lines; a blank line, a heading, a paragraph line, or a marker of the
	// other kind closes it.
	let mut list:		Vec<Vec<Inline>>	= Vec::new();
	let mut list_ord					= false;
	let mut list_start:	u32				= 0;
	let mut list_end:	u32				= 0;

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
			// A blank line closes the paragraph or list it follows.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
		} else if is_code_line(trimmed) {
			// A Typst code statement (`#import`, `#let`, `#set`, `#show`) or a whole-line call to a
			// template function Austenite does not yet run: it closes any open block and is skipped. The
			// styling and computation layer is a later increment; the prose around it still sets.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
		} else if trimmed.starts_with('=') {
			// A heading closes any paragraph or list above it, then stands on its own line.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			let level = trimmed.chars().take_while(|&c| c == '=').count();
			let raw = trimmed[level..].trim();	// '=' is ASCII, so a byte slice at the count is safe
			if raw.is_empty() {
				return Err(err!(
					"Empty heading on line {}: a `=` marker must be followed by a title.", line_no;
					Input, Invalid, Missing));
			}
			let (title, label) = split_label(raw);
			if title.is_empty() {
				return Err(err!(
					"Heading on line {} has a label but no title.", line_no; Input, Invalid, Missing));
			}
			items.push(Item::Heading {
				level:	level as u8,
				text:	title,
				label,
				span:	Span::new(start, end),
			});
		} else if let Some((ord, text)) = marker(trimmed) {
			// A list item. It closes any open paragraph, and a list of the other kind, but joins a list of
			// its own kind. The item's text carries inline emphasis like any run.
			flush_para(&mut items, &mut lines, para_start, para_end);
			if !list.is_empty() && list_ord != ord {
				flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			}
			if list.is_empty() {
				list_ord	= ord;
				list_start	= start;
			}
			list.push(parse_inlines(&text));
			list_end = end;
		} else {
			// Any other non-blank line joins the running paragraph, closing a list first; its own line
			// break and indentation carry no meaning, only its words.
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			if lines.is_empty() {
				para_start = start;
			}
			lines.push(line.to_string());
			para_end = end;
		}
	}

	// A source that ends without a closing blank line still closes its last paragraph or list.
	flush_para(&mut items, &mut lines, para_start, para_end);
	flush_list(&mut items, &mut list, list_ord, list_start, list_end);
	Ok(items)
}

/// Reads a list marker at the start of an already-left-trimmed line: `-` opens a bullet item, `+` a
/// numbered one. The marker must be the whole line or be followed by whitespace, so a dash inside a word
/// or a `+1` is ordinary prose, not a marker. Returns the item's kind and its text with the marker and
/// surrounding whitespace removed.
fn marker(trimmed: &str) -> Option<(bool, String)> {
	let first	= trimmed.chars().next()?;
	let ordered	= match first {
		'-'	=> false,
		'+'	=> true,
		_	=> return None,
	};
	let rest = &trimmed[first.len_utf8()..];
	if rest.is_empty() {
		return Some((ordered, String::new()));
	}
	if rest.starts_with(|c: char| c.is_whitespace()) {
		return Some((ordered, rest.trim().to_string()));
	}
	None
}

/// Closes the list being accumulated, if any, into one [`Item::List`]. An empty accumulator flushes
/// nothing, so a stray flush between two paragraphs costs nothing.
fn flush_list(
	items:		&mut Vec<Item>,
	list:		&mut Vec<Vec<Inline>>,
	ordered:	bool,
	start:		u32,
	end:		u32,
)
{
	if list.is_empty() {
		return;
	}
	items.push(Item::List { ordered, items: std::mem::take(list), span: Span::new(start, end) });
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

/// Splits a whitespace-collapsed paragraph into inline runs in Typst's markup: `*strong*`, `_emph_`, a
/// `@label` cross-reference, and `\`-escapes. An emphasis delimiter pairs only when it flanks a word --
/// whitespace or an opening bracket before it and a non-space after to open, the reverse to close -- so
/// `fe2o3_net`, `5 * 3` and a lone `_` are ordinary text. A backslash sets the next character literally,
/// so `\$`, `\#`, `\_` and `\@` appear as themselves. An unpaired delimiter, or an `@` with no label
/// after it, is ordinary text. Nesting is a later increment: the first valid closer ends a run.
fn parse_inlines(text: &str) -> Vec<Inline> {
	let chars:	Vec<char>	= text.chars().collect();
	let n					= chars.len();
	let mut runs:	Vec<Inline>	= Vec::new();
	let mut plain			= String::new();	// ordinary text gathered before the next run
	let mut i				= 0usize;
	while i < n {
		let c = chars[i];
		// A backslash escapes the next character, which is then set as itself.
		if c == '\\' && i + 1 < n {
			plain.push(chars[i + 1]);
			i += 2;
			continue;
		}
		// A Typst cross-reference: `@` then a label.
		if c == '@' {
			if let Some((label, next)) = at_label(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::PageRef(label));
				i = next;
				continue;
			}
		}
		if (c == '*' || c == '_') && is_opener(&chars, i) {
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

/// Reads a Typst cross-reference at `i` (an `@`): the label of letters, digits and `- _ :` that follows,
/// and the index just past it. `None` when no label char follows, so a bare or escaped `@` is ordinary
/// text. A trailing `.` is not a label character, so `@intro.` at the end of a sentence keeps its stop.
fn at_label(chars: &[char], i: usize) -> Option<(String, usize)> {
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && is_label_char(chars[j]) {
		j += 1;
	}
	if j == start {
		return None;
	}
	Some((chars[start..j].iter().collect(), j))
}

/// A character legal within a Typst label. Deliberately excludes `.`, so a label does not swallow the
/// full stop that ends a sentence.
fn is_label_char(c: char) -> bool {
	c.is_alphanumeric() || matches!(c, '-' | '_' | ':')
}

/// Is this already-left-trimmed line a Typst code statement Austenite skips for now? The four block
/// statements (`#import`, `#let`, `#set`, `#show`) and a whole-line call to a template function -- a
/// line that opens `#name(` or `#name[` and closes with the matching bracket -- are the styling and
/// computation layer, a later increment; the prose around them still sets.
fn is_code_line(trimmed: &str) -> bool {
	for kw in ["#import ", "#import\"", "#let ", "#set ", "#show ", "#show:"] {
		if trimmed.starts_with(kw) {
			return true;
		}
	}
	standalone_call(trimmed)
}

/// Does the whole line consist of one `#name(...)` or `#name[...]` call? A crude test -- it opens with
/// `#`, an identifier, then `(` or `[`, and ends with `)` or `]` -- enough to skip a single-line call to
/// an unrecognised template function without mistaking a paragraph that merely contains an inline call.
fn standalone_call(trimmed: &str) -> bool {
	let mut cs = trimmed.chars();
	if cs.next() != Some('#') {
		return false;
	}
	let mut saw_ident = false;
	for c in cs {
		if c.is_alphanumeric() || c == '-' || c == '_' {
			saw_ident = true;
			continue;
		}
		// The first non-identifier character must open the call.
		return saw_ident
			&& (c == '(' || c == '[')
			&& (trimmed.ends_with(')') || trimmed.ends_with(']'));
	}
	false
}

/// Splits a trailing `<label>` off a heading title: a `<name>` with no inner whitespace at the very end
/// labels the heading and is removed from its text. A title that merely contains angle brackets, or a
/// `< >` with a space inside, keeps them as ordinary characters.
fn split_label(title: &str) -> (String, Option<String>) {
	let t = title.trim_end();
	if let Some(inner) = t.strip_suffix('>') {
		if let Some(p) = inner.rfind('<') {
			let label = &inner[p + 1..];
			if !label.is_empty() && !label.contains(char::is_whitespace) {
				return (inner[..p].trim_end().to_string(), Some(label.to_string()));
			}
		}
	}
	(t.to_string(), None)
}
