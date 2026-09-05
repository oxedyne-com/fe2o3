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

use crate::ir::Span;

use super::ast::{Inline, Item};
use super::mathparse;

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

	// A fenced code block, while one is open: the verbatim lines gathered so far and the byte offset it
	// began at. A ```-fence opens it, the next ```-fence closes it; between them every line is kept as it
	// stands, its indentation and markup untouched.
	let mut code:		Option<(Vec<String>, u32)>	= None;

	// A multi-line Typst code statement or standalone template call being skipped: the net bracket depth
	// still open across the lines consumed so far, and whether a string literal is currently open. `None`
	// when not skipping. While it is `Some`, every line is consumed and nothing is set until the delimiters
	// balance.
	let mut skip:		Option<SkipState>	= None;

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
		| "index" | "index-main")
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
