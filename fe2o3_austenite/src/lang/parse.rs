//! The Typst reader: line-oriented source into the surface tree of [`ast::Item`](super::ast::Item).
//!
//! The scan is deliberately simple, one pass over the lines. A line whose first non-blank character
//! is `=` is a heading, its level the run of leading `=`; a line opening with `-` or `+` (and a space)
//! is a list item, a run of them a bullet or numbered list; every other non-blank line accumulates into
//! a paragraph. A blank line, a heading, or the start of the other kind of block closes whatever is
//! open. Whitespace within a paragraph is made insignificant here, so the line breaker downstream owns
//! the measure. Byte offsets are tracked across the raw lines so each [`Item`] carries a true [`Span`].
//!
//! A closed paragraph's text is then scanned for inline emphasis by [`parse_inlines`]: `*strong*` and
//! `_emph_`. A delimiter pairs only when it flanks a word, so a stray asterisk, a date's slash, or
//! `and/or` is left as ordinary text rather than opening an emphasis that never closes.
//!
//! A Typst code statement (`#import`/`#let`/`#set`/`#show`) or a line-leading standalone template call
//! (`#name(...)` or `#name[...]`) is not set: it is skipped. When its delimiters do not balance on the
//! opening line -- a `#figure(...)`, `#table(...)`, `#aside-box[...]`, or a `#let x = (...)` data array
//! that spans many lines -- the reader consumes following lines, tracking nesting across `()`, `[]` and
//! `{}` and respecting string literals, until the delimiters balance, so the whole span renders nothing.
//!
//! Typst comments are stripped before a line is classified: a `//` runs to the line's end, and a
//! `/* ... */` spans lines, both dropped -- except within a `"..."` string or a `` `code` `` span, and a
//! `//` right after `:` is kept, so a bare URL survives. Inline glossary and index calls, defined in the
//! book template (`#gs`, `#gscap`, `#gsi`, `#gscapi`, `#glossind`, `#glossindcap`, `#idx`, `#idx-main`,
//! `#idx-as`, `#idx-main-as`, `#index`, `#index-main`, `#idx-nested`), are read by [`parse_inlines`]: a
//! glossary term sets its display text, bold-italic on its first document use; a visible index call sets
//! its display text plain; a pure index marker sets nothing.

use crate::ir::Length;
use crate::ir::Span;
use crate::table::Align;

use super::ast::{AlignSpec, FigureBody, Inline, Item, TableSpec};
use super::mathparse;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

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

	// A fenced code block, while one is open: the verbatim lines gathered so far and the byte offset it
	// began at. A ```-fence opens it, the next ```-fence closes it; between them every line is kept as it
	// stands, its indentation and markup untouched.
	let mut code:		Option<(Vec<String>, u32)>	= None;

	// A multi-line Typst code statement or standalone template call being skipped: the net bracket depth
	// still open across the lines consumed so far, and whether a string literal is currently open. `None`
	// when not skipping. While it is `Some`, every line is consumed and nothing is set until the delimiters
	// balance.
	let mut skip:		Option<SkipState>	= None;

	// A multi-line construct whose whole text is gathered so it can be parsed rather than skipped: a
	// `#figure(...)`, a bare `#table(...)`, or a `#let name = (...)` data array feeding a table. `None`
	// when none is open. The accumulated text is dispatched by its kind when the delimiters balance.
	let mut capture:	Option<Capture>		= None;

	// Data arrays declared by `#let name = (...)` and referenced by a table's `..name.flatten()` spread:
	// the name maps to the flat sequence of cell texts the array holds. Populated as the arrays are read,
	// so a later figure resolves its cells against them.
	let mut arrays:		HashMap<String, Vec<String>>	= HashMap::new();

	// Whether a `/* ... */` block comment is open across the line break. A `//` line comment never
	// straddles a line, so it needs no carried state.
	let mut comment	= CommentState { in_block: false };

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

		// Strip Typst comments before classifying the line, but not while a fenced code block or a
		// multi-line call skip is open: inside a fence a `//` is verbatim, and a skipped span is dropped
		// whole regardless. The span above is computed from the raw line, so a diagnostic caret still
		// points into the source.
		let stripped;
		let line = if code.is_none() && skip.is_none() {
			stripped = strip_comments(line, &mut comment);
			stripped.as_str()
		} else {
			line
		};

		let trimmed = line.trim_start();

		// A multi-line code statement or standalone call is being skipped: keep consuming lines, tracking
		// bracket nesting across `()`, `[]` and `{}` and respecting string literals, until the delimiters
		// balance. Nothing between the opener and its close is set. This takes precedence over every other
		// rule, since the span is code, not markup.
		if let Some(state) = skip.as_mut() {
			scan_brackets(line, state);
			if state.depth <= 0 {
				skip = None;
			}
			continue;
		}

		// A multi-line construct is being gathered whole: keep appending its lines and tracking the bracket
		// balance until the delimiters close, then dispatch the accumulated text by its kind. Like the skip
		// above, this takes precedence over the markup rules, since the span is a code construct.
		if let Some(cap) = capture.as_mut() {
			cap.buf.push_str(line);
			cap.buf.push('\n');
			scan_brackets(line, &mut cap.state);
			if cap.state.depth <= 0 {
				let done = capture.take();
				if let Some(cap) = done {
					dispatch_capture(cap, &mut items, &mut arrays);
				}
			}
			continue;
		}

		// A fenced code block takes precedence over every other rule: inside it, only a closing fence is
		// special and every other line is verbatim, so its own `=`, `-` or `*` carry no markup meaning.
		if let Some((buf, cstart)) = code.as_mut() {
			if is_fence(trimmed) {
				items.push(Item::Code { lines: std::mem::take(buf), span: Span::new(*cstart, end) });
				code = None;
			} else {
				buf.push(line.to_string());
			}
			continue;
		}
		if is_fence(trimmed) {
			// An opening fence closes any paragraph or list, then begins a verbatim block. The fence line
			// itself (and any language tag on it) is not kept.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			code = Some((Vec::new(), start));
			continue;
		}

		if trimmed.is_empty() {
			// A blank line closes the paragraph or list it follows.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
		} else if let Some(kind) = capture_opener(trimmed) {
			// A multi-line construct the reader sets rather than skips -- a figure, a bare table, or a data
			// array feeding a table. It closes any open block, then its whole text is gathered by the check
			// at the top of the loop until the delimiters balance, and parsed by [`dispatch_capture`].
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			let mut state	= SkipState { depth: 0, in_string: false };
			scan_brackets(line, &mut state);
			let mut buf		= String::new();
			buf.push_str(line);
			buf.push('\n');
			let cap = Capture { kind, buf, state };
			if cap.state.depth <= 0 {
				dispatch_capture(cap, &mut items, &mut arrays);	// the whole construct closed on one line
			} else {
				capture = Some(cap);
			}
		} else if let Some(decision) = code_skip(trimmed) {
			// A Typst code statement (`#import`, `#let`, `#set`, `#show`) or a line-leading standalone call
			// to a template function Austenite does not yet run: it closes any open block and is skipped.
			// The styling and computation layer is a later increment; the prose around it still sets. When
			// its delimiters do not balance on this line, the multi-line span is consumed by the check at the
			// top of the loop until they do.
			flush_para(&mut items, &mut lines, para_start, para_end);
			flush_list(&mut items, &mut list, list_ord, list_start, list_end);
			if let CodeSkip::Multi(state) = decision {
				skip = Some(state);
			}
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

	// A source that ends without a closing blank line still closes its last paragraph or list; an
	// unterminated code fence still yields the block it had gathered.
	flush_para(&mut items, &mut lines, para_start, para_end);
	flush_list(&mut items, &mut list, list_ord, list_start, list_end);
	if let Some((buf, cstart)) = code {
		items.push(Item::Code { lines: buf, span: Span::new(cstart, offset) });
	}
	// A construct left open at end of source is dispatched with what it gathered, so a missing closer
	// still yields its best-effort figure or table rather than swallowing the tail silently.
	if let Some(cap) = capture {
		dispatch_capture(cap, &mut items, &mut arrays);
	}
	Ok(items)
}

/// Is this already-left-trimmed line a ```` ``` ```` code fence? An opening fence may carry a language
/// tag (```` ```rust ````); a closing fence is bare. Either way it opens with three backticks.
fn is_fence(trimmed: &str) -> bool {
	trimmed.starts_with("```")
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
		// An inline maths span between dollars. A `\$` was already turned into a literal above, so a `$`
		// reaching here opens maths. If it parses, it is a maths run; if not, the literal `$...$` is kept.
		if c == '$' {
			if let Some(close) = (i + 1..n).find(|&j| chars[j] == '$') {
				let inner: String = chars[i + 1..close].iter().collect();
				if let Ok(atom) = mathparse::parse(&inner) {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.push(Inline::Math(atom));
					i = close + 1;
					continue;
				}
			}
		}
		// An inline code span, `raw` between backticks: its content is verbatim, no markup within.
		if c == '`' {
			if let Some(close) = (i + 1..n).find(|&j| chars[j] == '`') {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Code(chars[i + 1..close].iter().collect()));
				i = close + 1;
				continue;
			}
		}
		// An inline footnote. Its bracketed content is markup, reduced here to display text, since the
		// engine sets a footnote's note as a plain small paragraph. The mark falls after the run before it.
		if c == '#' {
			if let Some((note, next)) = footnote_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Footnote(note));
				i = next;
				continue;
			}
		}
		// An inline glossary or index call defined in the book template. A glossary term is a run of its
		// own, so [`doc::author`] can set it bold-italic on first use; a visible index call sets its
		// display text, which may itself carry markup, so it is parsed and folded in; a pure index marker
		// sets nothing.
		if c == '#' {
			if let Some((call, next)) = glossary_call(&chars, i) {
				match call {
					Call::Glossary { term, display } => {
						if !plain.is_empty() {
							runs.push(Inline::Text(std::mem::take(&mut plain)));
						}
						runs.push(Inline::Glossary { term, display });
					},
					Call::Visible(display) => {
						let sub = parse_inlines(&display);
						// A plain display folds back into the running text, keeping the fast single-run
						// path; a display carrying markup becomes its own runs.
						if let [Inline::Text(t)] = sub.as_slice() {
							plain.push_str(t);
						} else {
							if !plain.is_empty() {
								runs.push(Inline::Text(std::mem::take(&mut plain)));
							}
							runs.extend(sub);
						}
					},
					Call::Invisible => {},	// a pure index marker sets nothing
				}
				i = next;
				continue;
			}
		}
		// An inline citation, `#cite(<key>)` or `#cite(<a>, <b>)`. Its keys become a cite run the block
		// layer resolves to "(Author Year)" against the bibliography; a citation with no readable key is
		// dropped rather than left as raw source.
		if c == '#' {
			if let Some((keys, next)) = cite_call(&chars, i) {
				if !keys.is_empty() {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.push(Inline::Cite(keys));
				}
				i = next;
				continue;
			}
		}
		// The `#raw("...")` call form of inline code.
		if c == '#' {
			if let Some((text, next)) = raw_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Code(text));
				i = next;
				continue;
			}
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

/// Reads an inline `#raw("...")` at `i`, returning its literal content and the index past the closing
/// `")`. `None` when the shape does not match, so a `#raw` written any other way is left as ordinary
/// text. Escaped quotes inside the string are not handled -- a later refinement.
fn raw_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open	= at_lit(chars, i, "#raw(\"")?;
	let close	= (open..chars.len()).find(|&j| chars[j] == '"')?;
	if chars.get(close + 1) != Some(&')') {
		return None;
	}
	Some((chars[open..close].iter().collect(), close + 2))
}

/// If the literal `s` sits at `i` in `chars`, the index just past it; otherwise `None`.
fn at_lit(chars: &[char], i: usize, s: &str) -> Option<usize> {
	let mut k = i;
	for ch in s.chars() {
		if chars.get(k) != Some(&ch) {
			return None;
		}
		k += 1;
	}
	Some(k)
}

/// The running bracket balance while a multi-line code statement or template call is skipped: `depth`
/// is the net count of unclosed `()`, `[]` and `{}` openers, and `in_string` records whether a `"..."`
/// literal is currently open so a bracket inside it does not count. Both persist across the lines of a
/// span, since a string or a nesting may straddle the line break.
struct SkipState {
	depth:		i32,
	in_string:	bool,
}

/// What to do with a line-leading Typst code statement or standalone template call.
enum CodeSkip {
	Line,				// the call closes on this line; skip the one line, as before
	Multi(SkipState),	// the delimiters are still open; begin a multi-line skip carrying the depth
}

/// If this already-left-trimmed line begins a Typst code statement Austenite skips for now, decides how
/// much to skip: `Line` for a statement or standalone call that closes on this line, `Multi` for one
/// whose delimiters are still open at the end of it. `None` when the line is not code the reader skips,
/// so the caller sets it as prose.
///
/// The four block statements (`#import`, `#let`, `#set`, `#show`) are always code; a line-leading call
/// (`#name(` or `#name[`) is skipped only as a whole -- either it closes on the line, or it opens a
/// multi-line span. A balanced `#name[...]` with prose trailing it (`#index-main[x]More prose...`) is
/// left to set, since its content is a marker within a real paragraph, not a standalone call.
fn code_skip(trimmed: &str) -> Option<CodeSkip> {
	let keyword	= code_keyword(trimmed);
	if !keyword && !opens_standalone_call(trimmed) {
		return None;
	}
	let mut state = SkipState { depth: 0, in_string: false };
	scan_brackets(trimmed, &mut state);
	if state.depth > 0 {
		return Some(CodeSkip::Multi(state));
	}
	// The delimiters balance on this line. A block statement is skipped whatever trails it; a standalone
	// call is skipped only when it truly ends with its own closer, so a marker inside a paragraph sets.
	if keyword || trimmed.ends_with(')') || trimmed.ends_with(']') {
		return Some(CodeSkip::Line);
	}
	None
}

/// Does this already-left-trimmed line open one of the four Typst block statements the reader skips?
fn code_keyword(trimmed: &str) -> bool {
	for kw in ["#import ", "#import\"", "#let ", "#set ", "#show ", "#show:"] {
		if trimmed.starts_with(kw) {
			return true;
		}
	}
	false
}

/// Does this already-left-trimmed line open with a standalone call -- `#`, an identifier, then `(` or
/// `[`? A crude test, enough to recognise the opener of a call to an unrecognised template function
/// without inspecting where or whether it closes; the balance decides single- versus multi-line.
fn opens_standalone_call(trimmed: &str) -> bool {
	let mut cs = trimmed.chars();
	if cs.next() != Some('#') {
		return false;
	}
	let mut ident	= String::new();
	for c in cs {
		if c.is_alphanumeric() || c == '-' || c == '_' {
			ident.push(c);
			continue;
		}
		// The first non-identifier character must open the call. A line-leading inline glossary or
		// index call is content, not a skippable standalone call, even when it happens to close on its
		// own line, so [`parse_inlines`] sets its display text rather than the reader dropping it.
		return !ident.is_empty() && (c == '(' || c == '[') && !is_inline_call(&ident);
	}
	false
}

/// Is this identifier one of the book template's inline glossary or index functions? These emit body
/// text (or an invisible marker) mid-paragraph, so a line that opens with one is prose the inline
/// scanner reads, never a standalone call the line scanner skips.
fn is_inline_call(name: &str) -> bool {
	matches!(name,
		"gs" | "gscap" | "gsi" | "gscapi" | "glossind" | "glossindcap"
		| "idx" | "idx-main" | "idx-as" | "idx-main-as" | "idx-nested"
		| "index" | "index-main" | "cite")
}

/// Folds one line's `()[]{}` into the running [`SkipState`], updating the depth and the in-string flag.
/// A bracket inside a `"..."` literal is ignored, and a `\`-escaped character within a string is passed
/// over, so a quote or bracket written `\"` or `\(` does not miscount. The state carries into the next
/// line, so a string or a nesting that straddles the break is tracked correctly.
fn scan_brackets(line: &str, state: &mut SkipState) {
	let mut escaped = false;
	for c in line.chars() {
		if state.in_string {
			if escaped {
				escaped = false;
			} else if c == '\\' {
				escaped = true;
			} else if c == '"' {
				state.in_string = false;
			}
			continue;
		}
		match c {
			'"'					=> state.in_string = true,
			'(' | '[' | '{'		=> state.depth += 1,
			')' | ']' | '}'		=> state.depth -= 1,
			_					=> {},
		}
	}
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

/// Reads an inline `#footnote[...]` at `i` (a `#`), returning the note's text -- its markup reduced to
/// display text by [`flatten_markup`] -- and the index just past the closing `]`. `None` when the shape
/// is not a footnote call or its bracket does not close, so anything else is left as ordinary text.
fn footnote_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#footnote")?;
	if chars.get(open) != Some(&'[') {
		return None;
	}
	let (inner, next) = read_group(chars, open)?;
	Some((flatten_markup(&inner), next))
}

/// Reads an inline `#cite(...)` at `i` (a `#`), returning the citation keys and the index past the
/// closing `)`. Every `<label>` token inside the parentheses is a key; a named argument such as
/// `form: "prose"` carries no label and is ignored. `None` when the shape is not a cite call or its
/// parentheses do not close, so anything else is left as ordinary text.
fn cite_call(chars: &[char], i: usize) -> Option<(Vec<String>, usize)> {
	let open = at_lit(chars, i, "#cite")?;
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let (inner, next) = read_group(chars, open)?;
	let keys = cite_keys(&inner);
	Some((keys, next))
}

/// Extracts the `<label>` citation keys from the inside of a `#cite(...)` call, in order. A `<` opens a
/// key and the next `>` closes it; anything outside a `<...>` pair (a named argument, a separating comma)
/// is skipped.
fn cite_keys(inner: &str) -> Vec<String> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut keys			= Vec::new();
	let mut i				= 0usize;
	while i < chars.len() {
		if chars[i] == '<' {
			if let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '>') {
				let key: String = chars[i + 1..close].iter().collect();
				let key = key.trim().to_string();
				if !key.is_empty() {
					keys.push(key);
				}
				i = close + 1;
				continue;
			}
		}
		i += 1;
	}
	keys
}

/// Reduces a run of markup to plain display text: the words a reader sees, with the emphasis, code,
/// glossary and index delimiters removed. A glossary term contributes its display, a visible index call
/// its text, a pure index marker nothing; `*strong*` and `_emph_` contribute their inner words. Inline
/// maths and cross-references, which have no plain form here, contribute nothing. Used where the engine
/// takes a plain string -- a footnote's note, a table cell, a figure caption -- and cannot yet carry the
/// runs themselves.
pub fn flatten_markup(text: &str) -> String {
	let mut out = String::new();
	for run in parse_inlines(text) {
		match run {
			Inline::Text(t)					=> out.push_str(&t),
			// A `*strong*` or `_emph_` inner run may still carry markup -- `*_word_*` nests emphasis in
			// strong -- so it is flattened again to strip the inner delimiters. Parsing does not nest, so
			// each pass removes one layer and the recursion terminates on plain text.
			Inline::Strong(t)				=> out.push_str(&flatten_markup(&t)),
			Inline::Emph(t)					=> out.push_str(&flatten_markup(&t)),
			Inline::Code(t)					=> out.push_str(&t),
			Inline::Glossary { display, .. }	=> out.push_str(&display),
			Inline::PageRef(_)				=> {},	// a page number has no plain form before layout
			Inline::Math(_)					=> {},	// maths is dropped from a flattened string
			Inline::Footnote(_)				=> {},	// a nested footnote is not set within a flattened string
			Inline::Cite(_)					=> {},	// a citation has no plain form before the bibliography resolves it
		}
	}
	out
}

/// What an inline glossary or index call sets into the running text.
enum Call {
	Glossary { term: String, display: String },	// a glossary term, keyed by `term` for first-use styling
	Visible(String),	// display text set plain, its markup parsed by the caller
	Invisible,			// a pure index marker: nothing is set
}

/// Reads an inline glossary or index call at `i` (a `#`), returning what it sets and the index just past
/// it, or `None` when the `#name` is not one the reader knows or its argument brackets do not close.
///
/// The visible glossary functions set their bracket content as the term, capitalising the display for
/// the `-cap` variants; `idx`/`idx-main` set the content plain; `idx-as`/`idx-main-as` take a second
/// argument as the display and set that; `index`/`index-main`/`idx-nested` are pure markers and set
/// nothing. First use is keyed by the term as written, matching the template's own case-sensitive
/// `glossary-seen` set. The term-dict lookups (`#g`, `#gcap`, `#gi`, `#gcapi`, `#t`, `#tcap`) need the
/// dictionary and are a later increment; they are not matched here, so they still set literally.
fn glossary_call(chars: &[char], i: usize) -> Option<(Call, usize)> {
	if chars.get(i) != Some(&'#') {
		return None;
	}
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
		j += 1;
	}
	if j == start {
		return None;
	}
	let name: String = chars[start..j].iter().collect();
	if !is_inline_call(&name) {
		return None;
	}
	match chars.get(j) {
		Some('(') | Some('[')	=> {},
		_						=> return None,
	}
	let (a1, next1) = read_group(chars, j)?;

	// The two-argument display functions take the second argument as the visible text.
	if name == "idx-as" || name == "idx-main-as" {
		let (a2, next2) = read_group(chars, next1)?;
		return Some((Call::Visible(unwrap_arg(&a2)), next2));
	}
	// A nested index entry is a pure marker; consume an optional second argument.
	if name == "idx-nested" {
		let end = match read_group(chars, next1) {
			Some((_, n2))	=> n2,
			None			=> next1,
		};
		return Some((Call::Invisible, end));
	}

	let arg = unwrap_arg(&a1);
	let call = match name.as_str() {
		"gs" | "gsi" | "glossind"			=> Call::Glossary { term: arg.clone(), display: arg },
		"gscap" | "gscapi" | "glossindcap"	=> Call::Glossary { term: arg.clone(), display: cap_first(&arg) },
		"idx" | "idx-main"					=> Call::Visible(arg),
		"index" | "index-main"				=> Call::Invisible,
		_									=> return None,
	};
	Some((call, next1))
}

/// Reads a bracket or paren group whose opener sits at `i`, returning its inner content and the index
/// just past the matching closer. Nesting of the same delimiter and `"..."` strings are respected, so a
/// bracket inside a quoted argument or a nested group does not close the group early. `None` when the
/// group never closes, so a malformed call is left as ordinary text.
fn read_group(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open	= *chars.get(i)?;
	let close	= match open {
		'['	=> ']',
		'('	=> ')',
		_	=> return None,
	};
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut escaped	= false;
	let mut inner	= String::new();
	let mut j		= i;
	while j < chars.len() {
		let c = chars[j];
		if in_str {
			inner.push(c);
			if escaped				{ escaped = false; }
			else if c == '\\'		{ escaped = true; }
			else if c == '"'		{ in_str = false; }
			j += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			inner.push(c);
		} else if c == open {
			depth += 1;
			if depth > 1 { inner.push(c); }	// keep a nested opener, drop the outer one
		} else if c == close {
			depth -= 1;
			if depth == 0 {
				return Some((inner, j + 1));
			}
			inner.push(c);
		} else {
			inner.push(c);
		}
		j += 1;
	}
	None
}

/// Strips a `"..."` wrapper from a paren-string argument, so `#gs("surplus")` reads the same term as
/// `#gs[surplus]`. A bracket argument has no quotes to strip and is returned unchanged.
fn unwrap_arg(inner: &str) -> String {
	let t = inner.trim();
	if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
		return t[1..t.len() - 1].to_string();
	}
	inner.to_string()
}

/// Capitalises the first character of a term for the `-cap` glossary variants, leaving the rest as it
/// stands. The template capitalises the first grapheme cluster; the first `char` matches it for every
/// term in these books.
fn cap_first(s: &str) -> String {
	let mut cs = s.chars();
	match cs.next() {
		Some(c)	=> c.to_uppercase().collect::<String>() + cs.as_str(),
		None	=> String::new(),
	}
}

/// Whether a `/* ... */` block comment is open across the line break.
struct CommentState {
	in_block:	bool,
}

/// Removes Typst comments from one line: a `//` to the line's end, and any `/* ... */` span, which may
/// have opened on an earlier line ([`CommentState::in_block`] carries that across). A `//` or `/*`
/// inside a `"..."` string or a `` `code` `` span is not a comment and is kept, and a `//` immediately
/// after `:` is kept so a bare URL survives. Quotes and backticks are treated as span delimiters here,
/// which is what the reader's markup needs; a real Typst code line with string literals is skipped whole
/// by the caller, so stripping it never reaches the output.
fn strip_comments(line: &str, st: &mut CommentState) -> String {
	let chars:	Vec<char>	= line.chars().collect();
	let mut out				= String::new();
	let mut in_str			= false;
	let mut in_raw			= false;
	let mut prev			= '\0';
	let mut i				= 0usize;
	while i < chars.len() {
		let c = chars[i];
		if st.in_block {
			if c == '*' && chars.get(i + 1) == Some(&'/') {
				st.in_block = false;
				i += 2;
				prev = '\0';
				continue;
			}
			i += 1;
			continue;
		}
		if in_str {
			out.push(c);
			if c == '"' { in_str = false; }
			prev = c;
			i += 1;
			continue;
		}
		if in_raw {
			out.push(c);
			if c == '`' { in_raw = false; }
			prev = c;
			i += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			out.push(c);
			prev = c;
			i += 1;
			continue;
		}
		if c == '`' {
			in_raw = true;
			out.push(c);
			prev = c;
			i += 1;
			continue;
		}
		if c == '/' && chars.get(i + 1) == Some(&'/') {
			if prev == ':' {
				out.push(c);	// a `://` is part of a URL, not a comment
				prev = c;
				i += 1;
				continue;
			}
			break;	// a line comment: drop the rest of the line
		}
		if c == '/' && chars.get(i + 1) == Some(&'*') {
			st.in_block = true;
			i += 2;
			prev = '\0';
			continue;
		}
		out.push(c);
		prev = c;
		i += 1;
	}
	out
}

// -- Multi-line figure, table and data-array capture ----------------------------------------------

/// A multi-line construct gathered whole so it can be parsed. The buffer accumulates its lines; the
/// bracket state closes it when the delimiters balance; the kind decides how the buffer is dispatched.
struct Capture {
	kind:	CaptureKind,
	buf:	String,
	state:	SkipState,
}

/// Which multi-line construct is being gathered.
enum CaptureKind {
	Figure,			// a `#figure(...)` call, possibly wrapping a table or an image
	Table,			// a bare `#table(...)` call
	Let(String),	// a `#let name = (...)` data array bound to this name
}

/// Detects the opener of a multi-line construct the reader parses rather than skips: a `#figure(`, a
/// bare `#table(`, or a `#let name = (` data array. `None` for any other line, which the caller then
/// offers to [`code_skip`].
fn capture_opener(trimmed: &str) -> Option<CaptureKind> {
	if trimmed.starts_with("#figure(") {
		return Some(CaptureKind::Figure);
	}
	if trimmed.starts_with("#table(") {
		return Some(CaptureKind::Table);
	}
	let_array_name(trimmed).map(CaptureKind::Let)
}

/// If the line is a `#let name = (` binding whose value opens a paren group, its name; else `None`. Only
/// an array or tuple value is captured -- a scalar or a function `#let` (whose name carries `(`) is left
/// to [`code_skip`].
fn let_array_name(trimmed: &str) -> Option<String> {
	let rest	= trimmed.strip_prefix("#let ")?;
	let eq		= rest.find('=')?;
	let name	= rest[..eq].trim();
	if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
		return None;
	}
	let value = rest[eq + 1..].trim_start();
	if value.starts_with('(') {
		Some(name.to_string())
	} else {
		None
	}
}

/// Dispatches a completed capture: a data array is evaluated and stored under its name; a table or a
/// figure is parsed into an [`Item`]. A construct that does not parse -- an unresolved spread, an empty
/// table -- yields no item rather than an error, so a stray call never fails the whole document.
fn dispatch_capture(
	cap:	Capture,
	items:	&mut Vec<Item>,
	arrays:	&mut HashMap<String, Vec<String>>,
)
{
	match cap.kind {
		CaptureKind::Let(name) => {
			arrays.insert(name, parse_let_array(&cap.buf));
		},
		CaptureKind::Table => {
			if let Some(inner) = call_inner(&cap.buf, "table") {
				if let Some(spec) = parse_table_spec(&inner, arrays) {
					items.push(Item::Table { spec, span: Span::new(0, 0) });
				}
			}
		},
		CaptureKind::Figure => {
			if let Some(item) = parse_figure(&cap.buf, arrays) {
				items.push(item);
			}
		},
	}
}

/// Evaluates a `#let name = (...)` value into the flat sequence of cell texts it holds. The value is the
/// paren group after the `=`; every `[...]` group within it, at any depth, is one cell -- which is what
/// `array.flatten()` yields for an array of content tuples.
fn parse_let_array(buf: &str) -> Vec<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let eq = match chars.iter().position(|&c| c == '=') {
		Some(e)	=> e,
		None	=> return Vec::new(),
	};
	let open = match (eq + 1..chars.len()).find(|&j| !chars[j].is_whitespace()) {
		Some(v) if chars[v] == '('	=> v,
		_							=> return Vec::new(),
	};
	match read_group(&chars, open) {
		Some((inner, _))	=> collect_cells(&inner),
		None				=> Vec::new(),
	}
}

/// Collects every `[...]` group in `inner`, in order, as flattened cell text. A `[` inside a string is
/// not a cell. Once a group opens, its whole content is one cell and is not descended into.
fn collect_cells(inner: &str) -> Vec<String> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut cells			= Vec::new();
	let mut in_str			= false;
	let mut esc				= false;
	let mut i				= 0usize;
	while i < chars.len() {
		let c = chars[i];
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			i += 1;
			continue;
		}
		if c == '[' {
			if let Some((content, next)) = read_group(&chars, i) {
				cells.push(flatten_markup(&content));
				i = next;
				continue;
			}
		}
		i += 1;
	}
	cells
}

/// Parses the inner text of a `#table(...)` call into a [`TableSpec`]. `columns:` fixes the column
/// count, `align:` the alignment, a `fill:` keyed on `row == 0` marks a header row; cells come from
/// inline `[...]` groups and from a `..name.flatten()` spread resolved against the data arrays. `None`
/// when no cells are found, so an empty or unresolved table sets nothing.
fn parse_table_spec(inner: &str, arrays: &HashMap<String, Vec<String>>) -> Option<TableSpec> {
	let mut ncols	= 1usize;
	let mut align	= AlignSpec::Uniform(Align::Left);
	let mut header	= false;
	let mut cells:	Vec<String>	= Vec::new();
	for arg in split_top_args(inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"columns"	=> ncols = parse_columns(&val),
				"align"		=> align = parse_align(&val),
				"fill"		=> if mentions_row0(&val) { header = true; },
				_			=> {},	// inset, stroke, gutter and the rest are not modelled
			}
			continue;
		}
		if let Some(name) = spread_name(a) {
			if let Some(v) = arrays.get(&name) {
				cells.extend(v.iter().cloned());
			}
			continue;
		}
		if a.starts_with('[') {
			let ch: Vec<char> = a.chars().collect();
			if let Some((content, _)) = read_group(&ch, 0) {
				cells.push(flatten_markup(&content));
			}
		}
	}
	if cells.is_empty() {
		return None;
	}
	Some(TableSpec { ncols: ncols.max(1), header, align, cells })
}

/// Parses a `#figure(...)` call (its buffer, a trailing `<label>` and all) into an [`Item::Figure`]. The
/// positional argument is the body -- a wrapped `#table(...)` set in full, or an image call stood in for
/// by a placeholder; `caption:` sets the caption, `supplement:`/`kind:` the "Figure" or "Table" label.
fn parse_figure(buf: &str, arrays: &HashMap<String, Vec<String>>) -> Option<Item> {
	let (body_src, label)	= strip_trailing_label(buf);
	let inner				= call_inner(&body_src, "figure")?;

	let mut caption:	Option<String>	= None;
	let mut supplement:	Option<String>	= None;
	let mut kind:		Option<String>	= None;
	let mut positional:	Option<String>	= None;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"caption"		=> caption = Some(caption_text(&val)),
				"supplement"	=> supplement = Some(unquote(&val)),
				"kind"			=> kind = Some(unquote(&val)),
				_				=> {},	// placement and the rest do not affect the set figure
			}
			continue;
		}
		if positional.is_none() {
			positional = Some(a.to_string());	// the first positional argument is the figure body
		}
	}

	let body_text	= positional.unwrap_or_default();
	let body		= figure_body(&body_text, arrays);
	let supplement	= supplement.unwrap_or_else(|| match kind.as_deref() {
		Some("table")	=> "Table".to_string(),
		_				=> "Figure".to_string(),
	});
	Some(Item::Figure { body, caption, supplement, label, span: Span::new(0, 0) })
}

/// Decides a figure's body from its positional text: a wrapped `#table(...)` if one is present and
/// parses, otherwise an image carrying the path and any declared sizing (empty path when none is found).
fn figure_body(text: &str, arrays: &HashMap<String, Vec<String>>) -> FigureBody {
	if let Some(inner) = call_inner(text, "table") {
		if let Some(spec) = parse_table_spec(&inner, arrays) {
			return FigureBody::Table(spec);
		}
	}
	let (path, width, height, scale) = image_call(text);
	FigureBody::Image { path, width, height, scale }
}

/// The path and sizing of a `padded-image("...")` or `image("...")` call in `text`. The custom wrapper is
/// tried first, since `image` is a word boundary within it only after the hyphen. The first positional
/// argument is the path; `width`/`height` size an `image(...)`, `scale` a `padded-image(...)`. A path
/// that is not found gives an empty string, which the block layer stands in for with a placeholder.
fn image_call(text: &str) -> (String, Option<Length>, Option<Length>, Option<f64>) {
	for name in ["padded-image", "image"] {
		if let Some(inner) = call_inner(text, name) {
			let mut path:	Option<String>	= None;
			let mut width:	Option<Length>	= None;
			let mut height:	Option<Length>	= None;
			let mut scale:	Option<f64>		= None;
			for arg in split_top_args(&inner) {
				let a = arg.trim();
				if a.is_empty() {
					continue;
				}
				if let Some((key, val)) = named_arg(a) {
					match key.as_str() {
						"width"		=> width = parse_length(&val),
						"height"	=> height = parse_length(&val),
						"scale"		=> scale = parse_percent(&val),
						_			=> {},	// padding and the rest do not size the set image
					}
					continue;
				}
				if path.is_none() {
					path = first_string(a);
				}
			}
			if let Some(p) = path {
				return (p, width, height, scale);
			}
		}
	}
	(String::new(), None, None, None)
}

/// Reads a Typst length argument into a [`Length`]: a percentage as a fraction of the measure, a `pt`,
/// `mm`, `cm` or `in` length as absolute points, a bare number as points. `auto` and anything unreadable
/// give `None`, so the figure falls back to filling the measure.
fn parse_length(val: &str) -> Option<Length> {
	let v = val.trim();
	if let Some(pct) = v.strip_suffix('%') {
		return pct.trim().parse::<f64>().ok().map(|n| Length::Rel(n / 100.0));
	}
	for (unit, per_pt) in [("pt", 1.0), ("mm", 72.0 / 25.4), ("cm", 72.0 / 2.54), ("in", 72.0)] {
		if let Some(num) = v.strip_suffix(unit) {
			return num.trim().parse::<f64>().ok().map(|n| Length::Abs(n * per_pt));
		}
	}
	v.parse::<f64>().ok().map(Length::Abs)
}

/// Reads a percentage argument (`100%`) into a fraction (`1.0`), or `None` when it is not a percentage.
fn parse_percent(val: &str) -> Option<f64> {
	val.trim().strip_suffix('%').and_then(|p| p.trim().parse::<f64>().ok()).map(|n| n / 100.0)
}

/// The content of the first `name(...)` call in `text`, balanced across nesting and strings, or `None`.
/// `name` must sit at a word boundary, so a short name does not match inside a longer identifier.
fn call_inner(text: &str, name: &str) -> Option<String> {
	let chars:	Vec<char>	= text.chars().collect();
	let namev:	Vec<char>	= name.chars().collect();
	let paren				= find_call(&chars, &namev, 0)?;
	read_group(&chars, paren).map(|(inner, _)| inner)
}

/// The index of the `(` of the first `name(` at a word boundary at or after `from`, or `None`.
fn find_call(chars: &[char], name: &[char], from: usize) -> Option<usize> {
	if name.is_empty() {
		return None;
	}
	let mut i = from;
	while i + name.len() < chars.len() {
		if chars[i..].starts_with(name) && chars.get(i + name.len()) == Some(&'(') {
			let boundary = i == 0 || !is_call_ident(chars[i - 1]);
			if boundary {
				return Some(i + name.len());
			}
		}
		i += 1;
	}
	None
}

/// A character that continues a Typst identifier, for the word-boundary test in [`find_call`].
fn is_call_ident(c: char) -> bool {
	c.is_alphanumeric() || c == '-' || c == '_'
}

/// The first `"..."` string literal's content in `text`, or `None`.
fn first_string(text: &str) -> Option<String> {
	let chars:	Vec<char>	= text.chars().collect();
	let start				= chars.iter().position(|&c| c == '"')?;
	let end					= (start + 1..chars.len()).find(|&j| chars[j] == '"')?;
	Some(chars[start + 1..end].iter().collect())
}

/// Splits the inner text of a call by its top-level commas, respecting `()[]{}` nesting and `"..."`
/// strings, so a comma inside a nested group or a string does not part an argument.
fn split_top_args(inner: &str) -> Vec<String> {
	let mut args:	Vec<String>	= Vec::new();
	let mut cur					= String::new();
	let mut depth				= 0i32;
	let mut in_str				= false;
	let mut esc					= false;
	for c in inner.chars() {
		if in_str {
			cur.push(c);
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'					=> { in_str = true; cur.push(c); },
			'(' | '[' | '{'		=> { depth += 1; cur.push(c); },
			')' | ']' | '}'		=> { depth -= 1; cur.push(c); },
			',' if depth == 0	=> { args.push(std::mem::take(&mut cur)); },
			_					=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		args.push(cur);
	}
	args
}

/// Splits a `key: value` argument at its top-level colon, returning the key and the trimmed value, or
/// `None` when there is no top-level colon or the key is not a bare identifier -- so a positional cell
/// or a spread is not mistaken for a named argument.
fn named_arg(arg: &str) -> Option<(String, String)> {
	let chars:	Vec<char>	= arg.chars().collect();
	let mut depth			= 0i32;
	let mut in_str			= false;
	let mut esc				= false;
	for (i, &c) in chars.iter().enumerate() {
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'				=> in_str = true,
			'(' | '[' | '{'	=> depth += 1,
			')' | ']' | '}'	=> depth -= 1,
			':' if depth == 0 => {
				let key: String = chars[..i].iter().collect();
				let key = key.trim().to_string();
				if !key.is_empty() && key.chars().all(is_call_ident) {
					let val: String = chars[i + 1..].iter().collect();
					return Some((key, val.trim().to_string()));
				}
				return None;
			},
			_				=> {},
		}
	}
	None
}

/// The array name of a `..name` or `..name.flatten()` spread argument, or `None`.
fn spread_name(arg: &str) -> Option<String> {
	let rest		= arg.trim().strip_prefix("..")?;
	let name: String	= rest.chars().take_while(|&c| is_call_ident(c)).collect();
	if name.is_empty() {
		None
	} else {
		Some(name)
	}
}

/// Parses a `columns:` value into a column count: an integer as itself, a track list `(a, b, c)` as its
/// entry count, anything else as one column.
fn parse_columns(val: &str) -> usize {
	let v = val.trim();
	if let Ok(n) = v.parse::<usize>() {
		return n.max(1);
	}
	if v.starts_with('(') {
		let ch: Vec<char> = v.chars().collect();
		if let Some((inner, _)) = read_group(&ch, 0) {
			let cnt = split_top_args(&inner).iter().filter(|p| !p.trim().is_empty()).count();
			return cnt.max(1);
		}
	}
	1
}

/// Parses an `align:` value: a `(col, row) => ...` closure as [`AlignSpec::Closure`], a tuple of column
/// alignments as [`AlignSpec::PerColumn`], a single alignment word as [`AlignSpec::Uniform`].
fn parse_align(val: &str) -> AlignSpec {
	let v = val.trim();
	if v.contains("=>") {
		return AlignSpec::Closure;
	}
	if v.starts_with('(') {
		let ch: Vec<char> = v.chars().collect();
		if let Some((inner, _)) = read_group(&ch, 0) {
			let cols: Vec<Align> = split_top_args(&inner).iter().map(|p| word_align(p)).collect();
			if !cols.is_empty() {
				return AlignSpec::PerColumn(cols);
			}
		}
	}
	AlignSpec::Uniform(word_align(v))
}

/// Maps a Typst alignment word to an [`Align`], ignoring a `+ horizon`/`+ top` vertical component and
/// treating `start`/`end` as left/right. An unknown word is left-aligned.
fn word_align(s: &str) -> Align {
	let first = s.trim().split(|c: char| c.is_whitespace() || c == '+').next().unwrap_or("").trim();
	match first {
		"center" | "centre"	=> Align::Centre,
		"right" | "end"		=> Align::Right,
		_					=> Align::Left,
	}
}

/// Does a `fill:` value key on the first row, marking a header? True for `row == 0` written with or
/// without spaces.
fn mentions_row0(val: &str) -> bool {
	let compact: String = val.chars().filter(|c| !c.is_whitespace()).collect();
	compact.contains("row==0")
}

/// Reduces a `caption: [...]` value to display text: the bracket content flattened, or the whole value
/// flattened when it is not a bracket group.
fn caption_text(val: &str) -> String {
	let v = val.trim();
	let ch: Vec<char> = v.chars().collect();
	if ch.first() == Some(&'[') {
		if let Some((content, _)) = read_group(&ch, 0) {
			return flatten_markup(&content);
		}
	}
	flatten_markup(v)
}

/// Strips a trailing `<label>` from a captured call, returning the call text without it and the label.
/// A `<name>` with no inner whitespace at the very end labels the figure; anything else keeps the text.
fn strip_trailing_label(buf: &str) -> (String, Option<String>) {
	let t = buf.trim_end();
	if let Some(inner) = t.strip_suffix('>') {
		if let Some(p) = inner.rfind('<') {
			let label = &inner[p + 1..];
			if !label.is_empty() && !label.contains(char::is_whitespace) {
				return (inner[..p].to_string(), Some(label.to_string()));
			}
		}
	}
	(buf.to_string(), None)
}

/// Strips a surrounding `"..."` from a string-literal argument value, leaving anything else unchanged.
fn unquote(val: &str) -> String {
	let t = val.trim();
	if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
		return t[1..t.len() - 1].to_string();
	}
	t.to_string()
}
