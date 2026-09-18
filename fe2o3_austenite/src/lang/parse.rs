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
//! Every such skip is recorded by name into a [`SkipSummary`] the parse returns beside its items, so a
//! caller reports the constructs it dropped rather than losing them silently. A `#columns(n)[ ... ]`
//! wrapper is the exception the reader does not drop whole: its body is re-parsed and set single-column.
//!
//! Typst comments are stripped before a line is classified: a `//` runs to the line's end, and a
//! `/* ... */` spans lines, both dropped -- except within a `"..."` string or a `` `code` `` span, and a
//! `//` right after `:` is kept, so a bare URL survives. Inline glossary and index calls, defined in the
//! book template (`#gs`, `#gscap`, `#gsi`, `#gscapi`, `#glossind`, `#glossindcap`, the term-dictionary
//! family `#g`, `#gcap`, `#gi`, `#gcapi`, `#t`, `#tcap`, `#graw`, and `#idx`, `#idx-main`, `#idx-as`,
//! `#idx-main-as`, `#index`, `#index-main`, `#idx-nested`), plus a `#link(dest)[text]` hyperlink, are read
//! by [`parse_inlines`]: a glossary term sets its display text, bold-italic on its first document use; a
//! visible term or index call sets its display text plain; a link sets its text and drops the destination;
//! a pure index marker sets nothing. An inline `#func[...]` the reader does not know is consumed, recorded
//! in the summary, and its bracketed body folded in, so its words survive but its raw markup never leaks.

use crate::ir::Length;
use crate::ir::Span;
use crate::table::Align;

use super::ast::{AlignSpec, ClosureAlign, FigureBody, Inline, Item, ListItem, TableSpec};
use super::mathparse;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::RwLock;

// The book's `term-dict`, set once by the loader before parsing so the term-dictionary glossary family
// (`t`, `tcap`, `graw`, `g`, `gi`, `gcap`, `gcapi`) resolves a key to its display value while the key
// identity is still known. A process-global rather than a threaded argument, mirroring the image base:
// the inline reader sets one run at a time and carries no book context of its own. `None` until the
// loader installs one, in which case every key falls back to its own text.
static TERM_DICT: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Records the book's `term-dict`, read from a sibling `terms.typ`, so the term-dictionary glossary
/// family resolves each key to its value at parse time. Installing a fresh map replaces any prior one.
pub fn set_term_dict(dict: HashMap<String, String>) -> Outcome<()> {
	let mut guard = lock_write!(TERM_DICT, "While recording the term dictionary");
	*guard = Some(dict);
	Ok(())
}

/// The display value a `term-dict` key resolves to, or `None` when no map is installed or it holds no
/// such key. A poisoned lock reads as absent rather than failing the parse: a missing value falls back
/// to the key text, which is exactly the safe degradation here.
pub(crate) fn term_value(key: &str) -> Option<String> {
	match TERM_DICT.read() {
		Ok(guard)	=> guard.as_ref().and_then(|m| m.get(key).cloned()),
		Err(_)		=> None,
	}
}

/// A tally of the constructs the reader skipped rather than set, keyed by the source name each was
/// written with (`#show`, `#let`, `#columns`, an unknown `#func`) and counted. A caller prints it so a
/// dropped construct is a visible report rather than a silent gap. Empty when the reader set everything
/// it met. Names carry their leading `#`, so the report reads back as source.
#[derive(Clone, Debug, Default)]
pub struct SkipSummary {
	counts:	BTreeMap<String, usize>,
}

impl SkipSummary {
	/// Records one skipped construct by the source name it was written with (with its leading `#`).
	fn record(&mut self, name: &str) {
		*self.counts.entry(name.to_string()).or_insert(0) += 1;
	}

	pub fn is_empty(&self) -> bool { self.counts.is_empty() }

	/// The number of distinct construct names skipped.
	pub fn kinds(&self) -> usize { self.counts.len() }

	/// The total count of skipped constructs across every name.
	pub fn total(&self) -> usize { self.counts.values().sum() }

	/// Each skipped construct name with its count, ordered by descending count then name, so the report
	/// leads with the construct that cost the most.
	pub fn entries(&self) -> Vec<(String, usize)> {
		let mut v: Vec<(String, usize)> = self.counts.iter().map(|(k, &c)| (k.clone(), c)).collect();
		v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		v
	}

	/// Folds another summary's counts into this one, so a caller assembling several chapters reports one
	/// total rather than a summary per file.
	pub fn merge(&mut self, other: &SkipSummary) {
		for (k, &c) in &other.counts {
			*self.counts.entry(k.clone()).or_insert(0) += c;
		}
	}

	/// A one-line report -- "skipped 3 unsupported constructs: #show (2), #columns (1)" -- or `None` when
	/// nothing was skipped, so a caller prints the line only when it has something to say.
	pub fn report(&self) -> Option<String> {
		if self.counts.is_empty() {
			return None;
		}
		let parts: Vec<String> = self.entries().into_iter()
			.map(|(n, c)| fmt!("{} ({})", n, c))
			.collect();
		let n = self.total();
		Some(fmt!("skipped {} unsupported construct{}: {}",
			n, if n == 1 { "" } else { "s" }, parts.join(", ")))
	}
}

/// Parses a whole Ingot source string into its surface items. The only error is an empty heading --
/// a `=` marker with no title -- which names the offending 1-based line.
pub fn document(src: &str) -> Outcome<Vec<Item>> {
	let (items, _) = res!(document_with_skips(src));
	Ok(items)
}

/// Parses a source string into its surface items and, alongside, the [`SkipSummary`] of every construct
/// the reader passed over rather than set -- a `#let`/`#set`/`#show`/`#import` code line, an unknown
/// line-leading `#func(...)` call, a `#columns` wrapper, and any unhandled inline `#func[...]`. The
/// caller prints the summary so a dropped construct is reported rather than lost silently.
pub fn document_with_skips(src: &str) -> Outcome<(Vec<Item>, SkipSummary)> {
	let mut skips:		SkipSummary	= SkipSummary::default();
	let mut items:		Vec<Item>	= Vec::new();
	let mut lines:		Vec<String>	= Vec::new();	// the current paragraph's constituent lines
	let mut para_start:	u32			= 0;			// byte offset of the paragraph's first line
	let mut para_end:	u32			= 0;			// byte offset just past its last line's content
	let mut offset:		u32			= 0;			// running byte offset of the current line's start
	let mut line_no					= 0usize;		// 1-based, for a diagnostic

	// The stack of open list levels, innermost last. Each level records the leading-space indent of its
	// markers, so a deeper marker opens a sub-list under the current item and a shallower one closes back
	// to the matching level; an empty stack means no list is open. A list is a run of marker lines that a
	// blank line does not break (Typst continues an enum across a gap), but any other content flushes.
	let mut stack:		Vec<ListFrame>	= Vec::new();

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
	// the name maps to the flat sequence of cells the array holds, each cell a run of inline markup.
	// Populated as the arrays are read, so a later figure resolves its cells against them.
	let mut arrays:		HashMap<String, Vec<Vec<Inline>>>	= HashMap::new();

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
			if !state.has_open_bracket() {
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
			if !cap.state.has_open_bracket() {
				let done = capture.take();
				if let Some(cap) = done {
					dispatch_capture(cap, &mut items, &mut arrays, &mut skips);
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
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
			code = Some((Vec::new(), start));
			continue;
		}

		// A display maths block ($ ... $) spans several source lines, but every branch below classifies a
		// line on its own -- so, left unchecked, a `=`-lead alignment row reads as a heading, a `-`-lead row
		// as a list marker, and a blank row between stacked lines flushes the paragraph early, each stealing
		// the line before the paragraph ever reaches `mathparse` whole. While an odd number of unescaped `$`
		// have accumulated in the running paragraph the block is still open, so the blank/heading/list checks
		// below are skipped for as long as it is: a fence opening or a capture opener (a `#`-led construct)
		// still takes precedence regardless, since neither shape occurs inside genuine display maths.
		let math_block_open = math_open(&lines);

		if trimmed.is_empty() && !math_block_open {
			// A blank line closes the paragraph it follows, but not an open list: Typst continues an enum
			// (or bullet list) across a blank line between items, restarting the numbering only when other
			// content intervenes. The list is therefore held open here; the marker branch joins a following
			// item of the same kind, while any other line -- a paragraph, heading, figure, fence or code
			// line -- flushes it first, so two lists parted by real content still restart.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
		} else if let Some(kind) = capture_opener(trimmed) {
			// A multi-line construct the reader sets rather than skips -- a figure, a bare table, or a data
			// array feeding a table. It closes any open block, then its whole text is gathered by the check
			// at the top of the loop until the delimiters balance, and parsed by [`dispatch_capture`].
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
			let mut state	= SkipState::new();
			scan_brackets(line, &mut state);
			let mut buf		= String::new();
			buf.push_str(line);
			buf.push('\n');
			let cap = Capture { kind, buf, state };
			if !cap.state.has_open_bracket() {
				dispatch_capture(cap, &mut items, &mut arrays, &mut skips);	// the whole construct closed on one line
			} else {
				capture = Some(cap);
			}
		} else if trimmed.starts_with("#line(") && call_inner(trimmed, "line").is_some() {
			// A standalone `#line(length:.., stroke:..)` horizontal divider (the appendix brackets a note
			// with one above and below). It closes any open block and sets a stroked rule; a multi-line
			// `#line(` that does not close on this line falls through to the skip path below.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
			if let Some(rule) = parse_line_rule(trimmed) {
				items.push(rule);
			}
		} else if trimmed.starts_with("#print-glossary(") {
			// A line-leading `#print-glossary()`: the glossary section's Term/Definition table. It closes any
			// open block and emits a placeholder the book layer fills once the whole document's glossary terms
			// are known -- unlike the surrounding template calls it is set in place, not recorded as a skip.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
			items.push(Item::PrintGlossary { span: Span::new(start, end) });
		} else if let Some(decision) = code_skip(trimmed) {
			// A Typst code statement (`#import`, `#let`, `#set`, `#show`) or a line-leading standalone call
			// to a template function Austenite does not yet run: it closes any open block and is skipped.
			// The styling and computation layer is a later increment; the prose around it still sets. When
			// its delimiters do not balance on this line, the multi-line span is consumed by the check at the
			// top of the loop until they do.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
			skips.record(&construct_name(trimmed));
			if let CodeSkip::Multi(state) = decision {
				skip = Some(state);
			}
		} else if trimmed.starts_with('=') && !math_block_open {
			// A heading closes any paragraph or list above it, then stands on its own line.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
			flush_list(&mut items, &mut stack);
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
			// The title carries inline markup like any run, so a glossary term, an index call, emphasis or a
			// maths span in a heading sets its display text rather than leaking its raw source into the head
			// and the table of contents.
			items.push(Item::Heading {
				level:	level as u8,
				runs:	parse_inlines_in(&title, &mut skips),
				label,
				span:	Span::new(start, end),
			});
		} else {
			// A list marker joins the list stack, unless a display maths block is open, in which case a
			// `-`/`+`-lead row is part of the equation, not a bullet: it falls through to the paragraph arm
			// below like every other captured line.
			match if math_block_open { None } else { marker(trimmed) } {
				Some((ord, text)) => {
					// A list item. It closes any open paragraph, then joins the list stack by its indentation:
					// a deeper marker opens a sub-list under the current item, a shallower one closes back to
					// the matching level, and a same-indent marker of the other kind ends the list and starts
					// one of the new kind. The item's text carries inline emphasis like any run.
					flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
					let indent = line.chars().take_while(|c| c.is_whitespace()).count();
					let runs = parse_inlines_in(&text, &mut skips);
					list_marker(&mut items, &mut stack, indent, ord, runs, start, end);
				},
				None => {
					// Any other non-blank line joins the running paragraph, closing a list first; its own line
					// break and indentation carry no meaning, only its words. This is also where a blank,
					// heading-lead or list-marker-lead line lands while a display maths block is open.
					flush_list(&mut items, &mut stack);
					if lines.is_empty() {
						para_start = start;
					}
					lines.push(line.to_string());
					para_end = end;
				},
			}
		}
	}

	// A source that ends without a closing blank line still closes its last paragraph or list; an
	// unterminated code fence still yields the block it had gathered.
	flush_para(&mut items, &mut lines, para_start, para_end, &mut skips);
	flush_list(&mut items, &mut stack);
	if let Some((buf, cstart)) = code {
		items.push(Item::Code { lines: buf, span: Span::new(cstart, offset) });
	}
	// A construct left open at end of source is dispatched with what it gathered, so a missing closer
	// still yields its best-effort figure or table rather than swallowing the tail silently.
	if let Some(cap) = capture {
		dispatch_capture(cap, &mut items, &mut arrays, &mut skips);
	}
	Ok((items, skips))
}

/// Is a display maths block still open across the paragraph lines gathered so far? A `$` toggles the
/// state; a `\`-escaped one (`\$`) is skipped, mirroring the same escape in [`parse_inlines_in`], so it
/// never toggles. An odd running count means the block opened on some earlier line and has not yet met
/// its close, which is what lets [`document_with_skips`] keep capturing lines the per-line classifier
/// would otherwise steal as a heading, a list marker, or a paragraph-flushing blank.
fn math_open(lines: &[String]) -> bool {
	let mut open = false;
	for line in lines {
		let mut chars = line.chars();
		while let Some(c) = chars.next() {
			if c == '\\' {
				chars.next();	// the escaped character, taken literally
			} else if c == '$' {
				open = !open;
			}
		}
	}
	open
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

/// One open level of a possibly-nested list while the reader accumulates it. `indent` is the leading-space
/// width of the level's markers, so a deeper marker opens a child level and a shallower one closes this
/// level back into the item it hung under.
struct ListFrame {
	indent:		usize,
	ordered:	bool,
	items:		Vec<ListItem>,
	start:		u32,
	end:		u32,
}

/// Attaches a marker line to the open list stack by its `indent`, opening or closing nested levels as the
/// indentation and kind require. A deeper marker opens a sub-list under the current item; a shallower one
/// closes the deeper level(s) first; a same-indent marker continues the level when its kind matches and
/// otherwise ends it and starts a fresh list of the new kind, as the flat reader did.
fn list_marker(
	items:	&mut Vec<Item>,
	stack:	&mut Vec<ListFrame>,
	indent:	usize,
	ord:	bool,
	runs:	Vec<Inline>,
	start:	u32,
	end:	u32,
)
{
	// Close every open level deeper than this marker: a dedent ends the nested list(s), each folding into
	// the item it hung under.
	while stack.last().map_or(false, |f| f.indent > indent) {
		if let Some(frame) = stack.pop() {
			fold(items, stack, frame);
		}
	}
	match stack.last_mut() {
		Some(top) if top.indent == indent && top.ordered == ord => {
			// Same level, same kind: another item of the open list.
			top.items.push(ListItem { runs, children: Vec::new() });
			top.end = end;
		},
		Some(top) if top.indent == indent => {
			// Same indent, the other kind: the open list ends and a fresh one of the new kind begins.
			if let Some(frame) = stack.pop() {
				fold(items, stack, frame);
			}
			stack.push(ListFrame {
				indent, ordered: ord, items: vec![ListItem { runs, children: Vec::new() }], start, end });
		},
		// Deeper than the current level (a sub-list), or the first marker of a list: open a new level. A
		// deeper level becomes a child of the current item when it folds.
		_ => stack.push(ListFrame {
			indent, ordered: ord, items: vec![ListItem { runs, children: Vec::new() }], start, end }),
	}
}

/// Folds a closed list level into the tree: it becomes an [`Item::List`] hanging under the current item of
/// the level below, or a top-level item when no level remains open.
fn fold(items: &mut Vec<Item>, stack: &mut Vec<ListFrame>, frame: ListFrame) {
	let list = Item::List {
		ordered:	frame.ordered,
		items:		frame.items,
		span:		Span::new(frame.start, frame.end),
	};
	match stack.last_mut() {
		Some(parent) => match parent.items.last_mut() {
			Some(item)	=> {
				item.children.push(list);
				parent.end = frame.end;
			},
			// A nested level always opens after its parent item exists, so this arm is unreachable in
			// practice; a stray level is kept as a top-level item rather than dropped.
			None		=> items.push(list),
		},
		None => items.push(list),
	}
}

/// Closes every open list level into the item tree. The deepest level folds into its parent's current item
/// first, so a nested list lands under the item it hung under; the outermost becomes a top-level
/// [`Item::List`]. An empty stack flushes nothing, so a stray flush between two paragraphs costs nothing.
fn flush_list(items: &mut Vec<Item>, stack: &mut Vec<ListFrame>) {
	while let Some(frame) = stack.pop() {
		fold(items, stack, frame);
	}
}

/// Closes the paragraph being accumulated, if any: its lines are joined, their whitespace collapsed,
/// and the result pushed as one [`Item::Paragraph`] spanning the source it came from. An empty
/// accumulator flushes nothing, so a run of blank lines closes a paragraph only once.
fn flush_para(
	items:	&mut Vec<Item>,
	lines:	&mut Vec<String>,
	start:	u32,
	end:	u32,
	skips:	&mut SkipSummary,
)
{
	if lines.is_empty() {
		return;
	}
	let text = normalise_ws(&lines.join(" "));
	// A trailing `<name>` labels the block -- in practice a display equation, `$ ... $ <eq_x>` -- and is
	// stripped before the runs are read, so the maths span stands alone and lowers to a numbered equation
	// rather than a rich paragraph. Ordinary prose ends in a full stop, so the conservative `split_label`
	// (a single whitespace-free token in angle brackets at the very end) does not fire on it.
	let (body, label) = split_label(&text);
	let runs = parse_inlines_in(&body, skips);
	items.push(Item::Paragraph { runs, label, span: Span::new(start, end) });
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
pub(crate) fn parse_inlines(text: &str) -> Vec<Inline> {
	let mut skips = SkipSummary::default();
	parse_inlines_in(text, &mut skips)
}

/// The inline scanner proper, recording every unhandled inline call into `skips` so a `#func[...]` the
/// reader cannot set is reported rather than leaked into the running text. [`parse_inlines`] is the thin
/// wrapper for callers -- table cells, captions, flattening -- that do not surface the summary.
fn parse_inlines_in(text: &str, skips: &mut SkipSummary) -> Vec<Inline> {
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
		// An inline footnote. Its bracketed content is markup, carried as inline runs so the note sets its
		// own emphasis at the foot of the page. The mark falls after the run before it.
		if c == '#' {
			if let Some((note, next)) = footnote_call(&chars, i, skips) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Footnote(note));
				i = next;
				continue;
			}
		}
		// The function-call form of emphasis, `#emph[...]`, set exactly as `_..._`: its bracketed content is
		// markup, so it takes the same expansion, and a call nested within it renders its display text.
		if c == '#' {
			if let Some((inner, next)) = emph_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				push_emphasis(&mut runs, false, &inner, skips);
				i = next;
				continue;
			}
		}
		// Typst's superscript, `#super[...]` or `#super("...")`. Its content is usually a short string or
		// number, reduced to display text here and set raised and smaller by the block layer.
		if c == '#' {
			if let Some((text, next)) = super_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Super(text));
				i = next;
				continue;
			}
		}
		// An inline glossary or index call defined in the book template. A glossary term is a run of its
		// own, so [`doc::author`] can set it bold-italic on first use; a visible index call sets its
		// display text, which may itself carry markup, so it is parsed and folded in; a pure index marker
		// sets nothing.
		if c == '#' {
			if let Some((call, next)) = glossary_call(&chars, i, skips) {
				match call {
					Call::Glossary { term, display } => {
						if !plain.is_empty() {
							runs.push(Inline::Text(std::mem::take(&mut plain)));
						}
						runs.push(Inline::Glossary { term, display });
					},
					Call::Visible(display) => {
						let sub = parse_inlines_in(&display, skips);
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
		// An inline claim marker, `#claim-label(...)` or `#claim-refs(...)`, from the book's claims
		// machinery. A `claim-label` registers invisible metadata and sets a small code in the outside
		// margin; a `claim-refs` registers metadata only. Neither places anything in the body text column,
		// so the marker is consumed and nothing set where it stood, matching Typst's body flow -- the
		// marginal code is not reproduced, the page having a single text column and no margin placement.
		if c == '#' {
			if let Some(next) = claim_call(&chars, i) {
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
		// A Typst hyperlink, `#link("url")[text]` or `#link(<label>)[text]`. The link text is what a print
		// reader sees, so its markup is parsed and folded into the running line; the destination has no place
		// on a page with no clickable annotation and is dropped. A `#link("url")` with no bracket sets the
		// URL itself as its text, as Typst does.
		if c == '#' {
			if let Some((body, next)) = link_call(&chars, i, skips) {
				if let [Inline::Text(t)] = body.as_slice() {
					plain.push_str(t);
				} else {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.extend(body);
				}
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
				push_emphasis(&mut runs, c == '*', &inner, skips);
				i = close + 1;
				continue;
			}
		}
		// An inline `#func(...)` or `#func[...]` call none of the handlers above claimed: a template function
		// the reader cannot yet run. It is recorded by name and consumed whole, so its raw markup no longer
		// leaks into the set text. When it wraps a single `[...]` content group -- the common shape of a Typst
		// content function -- that body is parsed and folded in, keeping its words rather than dropping them;
		// a call with only paren arguments (`#v(1em)`, `#colbreak()`) sets nothing where it stood.
		if c == '#' {
			if let Some((body, next, name)) = unknown_call(&chars, i, skips) {
				skips.record(&name);
				if let Some(body) = body {
					if let [Inline::Text(t)] = body.as_slice() {
						plain.push_str(t);
					} else {
						if !plain.is_empty() {
							runs.push(Inline::Text(std::mem::take(&mut plain)));
						}
						runs.extend(body);
					}
				}
				i = next;
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

/// Pushes an emphasised run (`*strong*` when `strong`, else `_emph_`) onto `runs`, reading its inner
/// markup. When the inner is plain text the run keeps the fast flat path -- one [`Inline::Strong`] or
/// [`Inline::Emph`]. When it carries a glossary term, an index call or a maths span -- as
/// `*The captation #gsi[attractor]*` does -- the emphasis is expanded: its plain stretches take the
/// emphasis face and the embedded calls become their own runs, so a call nested in emphasis renders its
/// display text rather than leaking its raw source. A glossary term keeps its own first-use bold-italic
/// (which subsumes the surrounding emphasis), so only the plain stretches carry the emphasis face.
fn push_emphasis(runs: &mut Vec<Inline>, strong: bool, inner: &str, skips: &mut SkipSummary) {
	let sub = parse_inlines_in(inner, skips);
	if let [Inline::Text(t)] = sub.as_slice() {
		runs.push(if strong { Inline::Strong(t.clone()) } else { Inline::Emph(t.clone()) });
		return;
	}
	for run in sub {
		match run {
			// A plain stretch takes the emphasis face.
			Inline::Text(t)					=> runs.push(if strong { Inline::Strong(t) } else { Inline::Emph(t) }),
			// An inner run of the opposite face (`*_word_*`, `_*word*_`) multiplies the two faces to a
			// bold-italic run rather than losing the outer; a run of the same face, a glossary term or a
			// maths span keeps its own face, one level of nesting being all the flat vocabulary carries.
			Inline::Emph(t) if strong		=> runs.push(Inline::BoldItalic(t)),
			Inline::Strong(t) if !strong	=> runs.push(Inline::BoldItalic(t)),
			other							=> runs.push(other),
		}
	}
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

/// Reads an inline `#link(dest)[text]` (or a bare `#link(dest)`) at `i` (a `#`), returning the link's
/// display runs and the index just past it. The destination -- a `"url"` string or a `<label>` -- is read
/// and discarded, the page carrying no clickable annotation; the bracketed text is what the reader sees,
/// so it is parsed for its own markup. A `#link(dest)` with no following `[...]` sets the destination
/// string itself as its text, as Typst does. Any unhandled inline call within the text is recorded into
/// `skips`. `None` when the shape is not a link call or its arguments do not close.
fn link_call(chars: &[char], i: usize, skips: &mut SkipSummary) -> Option<(Vec<Inline>, usize)> {
	let Some(open) = at_lit(chars, i, "#link") else { return None; };
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let Some((dest, after_dest)) = read_group(chars, open) else { return None; };
	// A following `[...]` group is the link text; without one, the destination stands as the text.
	if chars.get(after_dest) == Some(&'[') {
		let Some((body, next)) = read_group(chars, after_dest) else { return None; };
		return Some((parse_inlines_in(&body, skips), next));
	}
	let text = link_dest_text(&dest);
	Some((vec![Inline::Text(text)], after_dest))
}

/// The display text of a bare `#link(dest)` with no bracketed body: a `"url"` string loses its quotes, a
/// `<label>` its angle brackets, and anything else stands as written.
fn link_dest_text(dest: &str) -> String {
	let t = dest.trim();
	if let Some(inner) = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
		return inner.to_string();
	}
	unwrap_arg(t)
}

/// Reads an inline `#name(...)`/`#name[...]` call at `i` (a `#`) that no earlier handler claimed, so the
/// reader can consume it whole rather than leak its raw markup. Returns the bracketed body's runs (parsed,
/// so its own markup survives) when the call is a single `[...]` content group, `None` for the body when it
/// carries only paren arguments, together with the index just past the call and its `#name` for the skip
/// report. The final `None` is returned when `i` does not open a `#name(`/`#name[` call at all, so a bare
/// `#` or a `#variable` interpolation is left as ordinary text.
fn unknown_call(chars: &[char], i: usize, skips: &mut SkipSummary)
	-> Option<(Option<Vec<Inline>>, usize, String)>
{
	if chars.get(i) != Some(&'#') {
		return None;
	}
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_' || chars[j] == '.') {
		j += 1;
	}
	if j == start {
		return None;
	}
	let name: String = chars[start..j].iter().collect();
	match chars.get(j) {
		// A `#name[body]`: the bracketed content is the call's displayable body.
		Some('[') => {
			let Some((body, next)) = read_group(chars, j) else { return None; };
			Some((Some(parse_inlines_in(&body, skips)), next, fmt!("#{}", name)))
		},
		// A `#name(args)` and any following `[body]`: read the arguments away, then fold a body if one trails.
		Some('(') => {
			let Some((_, after_args)) = read_group(chars, j) else { return None; };
			if chars.get(after_args) == Some(&'[') {
				let Some((body, next)) = read_group(chars, after_args) else { return None; };
				return Some((Some(parse_inlines_in(&body, skips)), next, fmt!("#{}", name)));
			}
			Some((None, after_args, fmt!("#{}", name)))
		},
		_ => None,
	}
}

/// The `#name` of a skipped line-leading code statement or standalone call, for the skip report: the
/// keyword itself for a block statement (`#let`, `#set`, `#show`, `#import`), or `#` and the identifier of
/// a standalone call. Falls back to the first whitespace-delimited token when neither shape reads, so the
/// tally always names something rather than nothing.
fn construct_name(trimmed: &str) -> String {
	for kw in ["#import", "#let", "#set", "#show"] {
		if trimmed.starts_with(kw) {
			return kw.to_string();
		}
	}
	let mut cs = trimmed.chars();
	if cs.next() == Some('#') {
		let mut ident = String::new();
		for c in cs {
			if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
				ident.push(c);
			} else {
				break;
			}
		}
		if !ident.is_empty() {
			return fmt!("#{}", ident);
		}
	}
	trimmed.split_whitespace().next().unwrap_or(trimmed).to_string()
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

/// One open delimiter context on the scanner's stack. Which frame is on top decides whether the next
/// bracket is structural: a `(` inside a `[...]` content block is author prose, not nesting, and a `(`
/// or `$` inside a `"..."` string or a `$...$` maths span never counts at all. This is what lets a caption
/// whose prose carries an unbalanced `(` still close at its `]`, where a flat depth counter stuck open to
/// end of source and swallowed the figure and everything after it.
#[derive(Clone, Copy, PartialEq)]
enum Frame {
	Code,		// a `(...)`/`{...}`/`#name(...)` group, or the top level: brackets nest, `,`/`:` part
	Content,	// a `[...]` content block: only `[` `]` nest; author `(` `)` `{` `}` are literal prose
	Str,		// a `"..."` string literal: every character is literal until the closing quote
	Math,		// a `$...$` maths span: every character is literal until the closing `$`
}

/// The running delimiter balance while a bracketed span is scanned. The stack of [`Frame`]s replaces the
/// old flat `depth`: the span is closed when the stack is empty (was `depth <= 0`), and a multi-line code
/// skip is still open while it is not. `escaped` records that the previous character was a `\` inside a
/// string, maths span or content block, so a `\"`, `\$` or `\]` is passed over rather than closing its
/// frame. Both persist across the lines of a span, since a frame may straddle the line break.
struct SkipState {
	frames:		Vec<Frame>,
	escaped:	bool,
}

impl SkipState {
	fn new() -> Self {
		SkipState { frames: Vec::new(), escaped: false }
	}

	/// Is any frame still open? The top-level test for [`read_group`], [`split_top_args`] and [`named_arg`],
	/// where a comma or colon parts only when nothing at all is open and a group closes when the stack empties.
	fn is_open(&self) -> bool {
		!self.frames.is_empty()
	}

	/// Is a structural bracket -- a `(`/`{`/`[` group -- still unclosed? This is the multi-line skip and
	/// capture test, matching the old flat `depth > 0`: a dangling `"` or `$` left open at the end of a line
	/// does not keep a construct open, since in prose a stray quote (an author's `"no bound"` split across
	/// two lines after an inline `#raw("...")`) or a lone `$` is a character, not the start of a code span.
	fn has_open_bracket(&self) -> bool {
		self.frames.iter().any(|f| matches!(f, Frame::Code | Frame::Content))
	}

	/// Folds the character (or, in content mode, the `#ident` run) at `i` into the stack, returning how
	/// many characters were consumed from `chars` -- always at least one, more for a `#name(`/`#name[`/`#x`
	/// run whose opener decides the frame it enters. All four scanners share this one transition so a
	/// bracket is counted at exactly one place, whatever their outer loops do with the characters.
	fn step(&mut self, chars: &[char], i: usize) -> usize {
		let c = chars[i];
		match self.frames.last().copied() {
			Some(Frame::Str) => {
				if self.escaped			{ self.escaped = false; }
				else if c == '\\'		{ self.escaped = true; }
				else if c == '"'		{ self.frames.pop(); }
				1
			},
			Some(Frame::Math) => {
				if self.escaped			{ self.escaped = false; }
				else if c == '\\'		{ self.escaped = true; }
				else if c == '$'		{ self.frames.pop(); }
				1
			},
			Some(Frame::Content) => {
				// A `\`-escaped `\$ \[ \] \#` is literal content, so the escaped character is passed over
				// before any of the structural cases below can act on it.
				if self.escaped {
					self.escaped = false;
					return 1;
				}
				match c {
					'\\'	=> { self.escaped = true; 1 },
					'['		=> { self.frames.push(Frame::Content); 1 },
					']'		=> { self.frames.pop(); 1 },
					'$'		=> { self.frames.push(Frame::Math); 1 },
					'#'		=> self.content_hash(chars, i),
					// A `(` `)` `{` `}` in content mode is author prose, never nesting: this is the whole
					// point of tracking the frame, so a caption's unbalanced paren does not stick.
					_		=> 1,
				}
			},
			// A code frame, or the top level (an empty stack): brackets nest as the flat counter had them,
			// the closer kind is not checked, and a `[` opens a content child, a `$` a maths span.
			_ => {
				match c {
					'"'				=> { self.frames.push(Frame::Str); },
					'(' | '{'		=> { self.frames.push(Frame::Code); },
					'['				=> { self.frames.push(Frame::Content); },
					'$'				=> { self.frames.push(Frame::Math); },
					')' | '}'		=> { self.frames.pop(); },
					_				=> {},
				}
				1
			},
		}
	}

	/// Handles a `#` met in content mode: a `#name` identifier follows, and its first non-identifier
	/// character decides the frame -- `(` opens the call's code arguments, `[` a content block, anything
	/// else (or end of input) is a bare `#name` field access with no group. Returns the count consumed:
	/// the `#`, the identifier, and, for a call or content opener, that opener too.
	fn content_hash(&mut self, chars: &[char], i: usize) -> usize {
		let mut j = i + 1;
		while j < chars.len() && is_call_ident(chars[j]) {
			j += 1;
		}
		match chars.get(j) {
			Some('(')	=> { self.frames.push(Frame::Code); j + 1 - i },
			Some('[')	=> { self.frames.push(Frame::Content); j + 1 - i },
			_			=> j - i,	// a bare `#name` (or a lone `#`): open no frame
		}
	}
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
	let mut state = SkipState::new();
	scan_brackets(trimmed, &mut state);
	if state.has_open_bracket() {
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

/// Is this identifier one of the book template's inline functions the reader sets in place -- a glossary
/// or index call, a term-dictionary lookup, a hyperlink, a citation or an emphasis call? These emit body
/// text (or an invisible marker) mid-paragraph, so a line that opens with one is prose the inline scanner
/// reads, never a standalone call the line scanner skips.
fn is_inline_call(name: &str) -> bool {
	matches!(name,
		// The simple string-keyed glossary/index family, keyed on their own display text.
		"gs" | "gscap" | "gsi" | "gscapi" | "glossind" | "glossindcap"
		// The term-dictionary family, keyed on a `term-dict` entry: `g`/`gcap`/`gi`/`gcapi` set the value
		// with first-use styling, `t`/`tcap` set it plain, `graw` sets it in the mono face.
		| "g" | "gcap" | "gi" | "gcapi" | "t" | "tcap" | "graw"
		| "idx" | "idx-main" | "idx-as" | "idx-main-as" | "idx-nested"
		| "index" | "index-main" | "cite" | "link"
		| "emph" | "super"
		| "claim-label" | "claim-refs")
}

/// Folds one line's delimiters into the running [`SkipState`]. A bracket inside a `"..."` string, a `$...$`
/// maths span or a `[...]` content block is not counted as structural nesting; the frame stack decides.
/// The state carries into the next line, so a frame that straddles the break is tracked correctly.
fn scan_brackets(line: &str, state: &mut SkipState) {
	let chars: Vec<char> = line.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		i += state.step(&chars, i);
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

/// Reads an inline `#footnote[...]` at `i` (a `#`), returning the note's inline markup -- parsed so a
/// `*strong*` or `_emph_` in the note sets with its own face -- and the index just past the closing `]`.
/// `None` when the shape is not a footnote call or its bracket does not close, so anything else is left as
/// ordinary text.
fn footnote_call(chars: &[char], i: usize, skips: &mut SkipSummary) -> Option<(Vec<Inline>, usize)> {
	let Some(open) = at_lit(chars, i, "#footnote") else { return None; };
	if chars.get(open) != Some(&'[') {
		return None;
	}
	let Some((inner, next)) = read_group(chars, open) else { return None; };
	Some((parse_inlines_in(&inner, skips), next))
}

/// Reads an inline `#emph[...]` at `i` (a `#`), returning its inner markup unreduced -- it is the call
/// form of `_..._` and the caller expands it the same way -- and the index just past the closing `]`.
/// `None` when the shape is not an emph call or its bracket does not close, so anything else is left as
/// ordinary text.
fn emph_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#emph")?;
	if chars.get(open) != Some(&'[') {
		return None;
	}
	read_group(chars, open)
}

/// Reads an inline `#super[...]` or `#super("...")` at `i` (a `#`), returning its content reduced to
/// display text by [`flatten_markup`] -- usually a short string or number -- and the index just past the
/// closing bracket. `None` when the shape is not a super call or its argument does not close.
fn super_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#super")?;
	match chars.get(open) {
		Some('[') | Some('(')	=> {},
		_						=> return None,
	}
	let (inner, next) = read_group(chars, open)?;
	Some((flatten_markup(&unwrap_arg(&inner)), next))
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

/// Reads an inline `#claim-label(...)` or `#claim-refs(...)` at `i` (a `#`), returning the index just
/// past the closing `)`. Both are the book's claims plumbing: `claim-label` registers invisible metadata
/// and sets a compressed code string in the outside margin, `claim-refs` registers metadata only, and
/// neither sets anything in the body text column. The reader consumes the call and sets nothing where it
/// stood, so the raw markup no longer leaks and the surrounding prose closes over the gap as Typst's body
/// does. The marginal code annotation is not reproduced: the page carries a single text column with no
/// margin-placement facility. `None` when the shape is not a claim call or its parentheses do not close.
fn claim_call(chars: &[char], i: usize) -> Option<usize> {
	let open = at_lit(chars, i, "#claim-label")
		.or_else(|| at_lit(chars, i, "#claim-refs"))?;
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let (_, next) = read_group(chars, open)?;
	Some(next)
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
			Inline::BoldItalic(t)			=> out.push_str(&t),	// already the flat inner of a nested run
			Inline::Super(t)				=> out.push_str(&t),	// a flattened string cannot raise; keep its text
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
/// `glossary-seen` set.
///
/// The term-dictionary family keys a `term-dict` entry rather than carrying its own display, and the
/// reader translates the key to that value at parse time (the loader installs the map from `terms.typ`
/// before parsing): `g`/`gi` set the value bold-italic on first use, `gcap`/`gcapi` capitalised, `t`/`tcap`
/// plain and `graw` plain (its mono face is not reproduced). A key with no `term-dict` entry falls back to
/// the key text and is recorded in `skips`, so an unknown key is visible on the terse skip line rather
/// than silently wrong -- the template panics on a miss, which the reader must not.
fn glossary_call(chars: &[char], i: usize, skips: &mut SkipSummary) -> Option<(Call, usize)> {
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
		// The simple family keys its own display text (a `term-defs` entry), so no translation applies.
		"gs" | "gsi"							=> Call::Glossary { term: arg.clone(), display: arg },
		"gscap" | "gscapi"						=> Call::Glossary { term: arg.clone(), display: cap_first(&arg) },
		// `glossind`/`glossindcap` auto-detect: a key that is in `term-dict` sets its value, otherwise the
		// key stands as its own display, matching the template's `if key in term-dict` branch.
		"glossind"								=> {
			let display = term_value(&arg).unwrap_or_else(|| arg.clone());
			Call::Glossary { term: arg.clone(), display }
		},
		"glossindcap"							=> {
			let display = term_value(&arg).unwrap_or_else(|| arg.clone());
			Call::Glossary { term: arg.clone(), display: cap_first(&display) }
		},
		// The term-dictionary family translates the key to its value; first use is keyed by the key, as the
		// template keys `glossary-seen` by the key name rather than the value.
		"g" | "gi"								=> Call::Glossary { term: arg.clone(), display: resolve_term(&arg, &name, skips) },
		"gcap" | "gcapi"						=> Call::Glossary { term: arg.clone(), display: cap_first(&resolve_term(&arg, &name, skips)) },
		"t" | "graw"							=> Call::Visible(resolve_term(&arg, &name, skips)),
		"tcap"									=> Call::Visible(cap_first(&resolve_term(&arg, &name, skips))),
		"idx" | "idx-main"						=> Call::Visible(arg),
		"index" | "index-main"					=> Call::Invisible,
		_									=> return None,
	};
	Some((call, next1))
}

/// Resolves a term-dictionary key to its display value, or -- when no map is installed or it holds no
/// such key -- falls back to the key text and records the miss in `skips` under the calling function's
/// name, so an unknown key shows on the terse skip line rather than rendering silently as the raw key.
fn resolve_term(key: &str, func: &str, skips: &mut SkipSummary) -> String {
	match term_value(key) {
		Some(value)	=> value,
		None		=> {
			skips.record(&fmt!("#{} unknown term-dict key {:?}", func, key));
			key.to_string()
		},
	}
}

/// Reads a bracket or paren group whose opener sits at `i`, returning its inner content and the index
/// just past the matching closer. The [`SkipState`] frame stack decides what nests: strings, maths spans
/// and content blocks are respected, so a bracket inside a quoted argument, a `$...$` span or the prose of
/// a `[...]` caption does not close the group early -- a `(` an author left unbalanced in caption prose is
/// literal, and the group still closes at its own delimiter. `None` when the group never closes, so a
/// malformed call is left as ordinary text.
pub(crate) fn read_group(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = *chars.get(i)?;
	if open != '[' && open != '(' {
		return None;
	}
	let mut state	= SkipState::new();
	let mut inner	= String::new();
	// The opener pushes its frame (`[` a content block, `(` a code group) but is not part of the inner
	// content, so it is stepped over here and never appended.
	let mut j = i + state.step(chars, i);
	while j < chars.len() {
		let consumed = state.step(chars, j);
		if !state.is_open() {
			// This character closed the outer group -- the matching closer -- so the group ends just past
			// it, and the closer is dropped from the inner as the outer opener was.
			return Some((inner, j + consumed));
		}
		// Every other character is inner content, verbatim: a nested opener or closer, a string with its
		// quotes, a maths span, or a `#name(` run in content mode.
		for k in j..j + consumed {
			inner.push(chars[k]);
		}
		j += consumed;
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
	Image,			// a line-leading `#padded-image(...)` or `#image(...)` set without a figure number
	SectionBanner,	// a line-leading `#section-banner("logo")`, a full-width grey bar carrying a section logo
	Let(String),	// a `#let name = (...)` data array bound to this name
	Columns,		// a `#columns(n)[ ... ]` wrapper: its body is set single-column
	StyledBox,		// a `#styled-box[ ... ]` callout: its body is set inside a filled, padded box
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
	if trimmed.starts_with("#columns(") {
		return Some(CaptureKind::Columns);
	}
	// A `#styled-box[ ... ]` callout: a full-measure filled box wrapping running prose. Its body opens with
	// the `[` on this line and closes on a later one, so it is gathered whole and re-parsed rather than
	// skipped -- otherwise the bracket span reads as an unbalanced standalone call and its text is dropped.
	if trimmed.starts_with("#styled-box[") {
		return Some(CaptureKind::StyledBox);
	}
	// A documentation section opens with a line-leading `#section-banner("logo")` -- a full-width grey bar
	// carrying the section's logo -- captured here so the bar is drawn rather than the call dropped. Tried
	// before `#image(`, since the name contains none of the others as a prefix.
	if trimmed.starts_with("#section-banner(") {
		return Some(CaptureKind::SectionBanner);
	}
	// A section opener draws its logo with a line-leading `#padded-image(...)` (the Pearl section's pearlite
	// mark), and a bare `#image(...)` places a graphic likewise. Both are set centred without a figure
	// number, so they are captured here rather than skipped. The hyphen keeps `#image(` from matching the
	// tail of `#padded-image(`, which is tried first.
	if trimmed.starts_with("#padded-image(") || trimmed.starts_with("#image(") {
		return Some(CaptureKind::Image);
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
	arrays:	&mut HashMap<String, Vec<Vec<Inline>>>,
	skips:	&mut SkipSummary,
)
{
	match cap.kind {
		CaptureKind::Let(name) => {
			arrays.insert(name, parse_let_array(&cap.buf));
		},
		CaptureKind::Table => {
			if let Some(inner) = call_inner(&cap.buf, "table") {
				if let Some(spec) = parse_table_spec(&inner, arrays, outer_text_size(&cap.buf)) {
					items.push(Item::Table { spec, span: Span::new(0, 0) });
				}
			}
		},
		CaptureKind::Figure => {
			if let Some(item) = parse_figure(&cap.buf, arrays) {
				items.push(item);
			}
		},
		CaptureKind::Image => {
			// A line-leading image call: its path and sizing are read the same way a figure's image body is,
			// then set as a plain centred image with no figure number. A call naming no path draws nothing.
			let (path, width, height, scale) = image_call(&cap.buf);
			if !path.is_empty() {
				items.push(Item::Image { path, width, height, scale, span: Span::new(0, 0) });
			}
		},
		CaptureKind::SectionBanner => {
			// The first positional argument is the logo path; a call naming none draws nothing.
			if let Some(path) = call_inner(&cap.buf, "section-banner").as_deref().and_then(first_string) {
				items.push(Item::SectionBanner { path, span: Span::new(0, 0) });
			}
		},
		CaptureKind::Columns => {
			// The reader has no column model: the `#columns(n)[ ... ]` wrapper is recorded as skipped and
			// its body set single-column, so the words survive even though the multi-column layout does not.
			// The body is a block sequence, so it is read through the document parser again and its items
			// spliced in; a nested skip (a `#colbreak()`, an unknown call) folds into the same summary.
			skips.record("#columns");
			if let Some(body) = columns_body(&cap.buf) {
				if let Ok((mut inner, sub)) = document_with_skips(&body) {
					skips.merge(&sub);
					items.append(&mut inner);
				}
			}
		},
		CaptureKind::StyledBox => {
			// A `#styled-box[ ... ]` callout. Its body is a block sequence, so it is read through the document
			// parser again and wrapped in a single [`Item::Box`] the lowering sets in a filled, padded box --
			// unlike `#columns`, whose body splices in flat. The construct is set, not skipped, so it is not
			// recorded in the summary; a nested skip within the body (an unknown inline call) still folds in.
			if let Some(body) = styled_box_body(&cap.buf) {
				if let Ok((inner, sub)) = document_with_skips(&body) {
					skips.merge(&sub);
					items.push(Item::Box { items: inner, span: Span::new(0, 0) });
				}
			}
		},
	}
}

/// The `[ ... ]` body of a captured `#columns(n)[ ... ]` wrapper: the column count arguments are read and
/// dropped, and the bracketed block content returned for re-parsing. `None` when no `[...]` group follows
/// the arguments, so a malformed wrapper contributes no body.
fn columns_body(buf: &str) -> Option<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let Some(at) = find_lit(&chars, "#columns") else { return None; };
	let open	= at + "#columns".chars().count();
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let Some((_, after_args)) = read_group(&chars, open) else { return None; };
	let mut j = after_args;
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	if chars.get(j) != Some(&'[') {
		return None;
	}
	read_group(&chars, j).map(|(body, _)| body)
}

/// The `[ ... ]` body of a captured `#styled-box[ ... ]` callout, returned for re-parsing. The call takes
/// no arguments, so the bracket group opens immediately after the name. `None` when no `[...]` follows, so
/// a malformed callout contributes no body.
fn styled_box_body(buf: &str) -> Option<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let Some(at) = find_lit(&chars, "#styled-box") else { return None; };
	let open = at + "#styled-box".chars().count();
	if chars.get(open) != Some(&'[') {
		return None;
	}
	read_group(&chars, open).map(|(body, _)| body)
}

/// The index of the first occurrence of the literal `s` in `chars`, or `None`.
fn find_lit(chars: &[char], s: &str) -> Option<usize> {
	let pat:	Vec<char>	= s.chars().collect();
	if pat.is_empty() || chars.len() < pat.len() {
		return None;
	}
	(0..=chars.len() - pat.len()).find(|&start| chars[start..start + pat.len()] == pat[..])
}

/// Evaluates a `#let name = (...)` value into the flat sequence of cells it holds, each cell its inline
/// runs. The value is the paren group after the `=`; every `[...]` group within it, at any depth, is one
/// cell -- which is what `array.flatten()` yields for an array of content tuples.
fn parse_let_array(buf: &str) -> Vec<Vec<Inline>> {
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

/// Collects every `[...]` group in `inner`, in order, each parsed into its inline runs. A `[` inside a
/// string is not a cell. Once a group opens, its whole content is one cell and is not descended into.
///
/// A `table.cell(colspan: n)[...]` wrapper is expanded to `n` grid cells: the bracketed content, then
/// `n - 1` empty span placeholders. A spanning cell consumes several columns in Typst, so without the
/// placeholders every cell after it slides one column to the left and the last column overruns the
/// table; the placeholders keep the flat cell stream aligned to the column grid. A bare `table.cell(...)`
/// (no `colspan`) is one cell, like a plain `[...]`.
fn collect_cells(inner: &str) -> Vec<Vec<Inline>> {
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
		// A `table.cell(args)[content]` wrapper: read the args for a `colspan`, then take the following
		// `[...]` as the content and emit it across that many columns.
		if let Some(after) = at_lit(&chars, i, "table.cell") {
			if chars.get(after) == Some(&'(') {
				if let Some((args, past_args)) = read_group(&chars, after) {
					let span	= cell_colspan(&args);
					let mut j	= past_args;
					while j < chars.len() && chars[j].is_whitespace() {
						j += 1;
					}
					if chars.get(j) == Some(&'[') {
						if let Some((content, next)) = read_group(&chars, j) {
							cells.push(parse_inlines(&content));
							for _ in 1..span {
								cells.push(vec![Inline::Text(String::new())]);
							}
							i = next;
							continue;
						}
					}
					// No content group followed the wrapper; step past its args and carry on.
					i = past_args;
					continue;
				}
			}
		}
		if c == '[' {
			if let Some((content, next)) = read_group(&chars, i) {
				cells.push(parse_inlines(&content));
				i = next;
				continue;
			}
		}
		i += 1;
	}
	cells
}

/// The `colspan:` of a `table.cell(...)` argument list, at least one. A missing or unreadable `colspan`
/// is a single column.
fn cell_colspan(args: &str) -> usize {
	for arg in split_top_args(args) {
		if let Some((key, val)) = named_arg(arg.trim()) {
			if key == "colspan" {
				if let Ok(n) = val.trim().parse::<usize>() {
					return n.max(1);
				}
			}
		}
	}
	1
}

/// Parses the inner text of a `#table(...)` call into a [`TableSpec`]. `columns:` fixes the column
/// count, `align:` the alignment, a `fill:` keyed on `row == 0` marks a header row; cells come from
/// inline `[...]` groups and from a `..name.flatten()` spread resolved against the data arrays. `None`
/// when no cells are found, so an empty or unresolved table sets nothing.
fn parse_table_spec(
	inner:			&str,
	arrays:			&HashMap<String, Vec<Vec<Inline>>>,
	outer_text_pt:	Option<f64>,
)
	-> Option<TableSpec>
{
	let mut ncols		= 1usize;
	let mut align		= AlignSpec::Uniform(Align::Left);
	let mut header		= false;
	let mut inset_pt:	Option<f64>			= None;
	let mut weights:	Vec<f64>			= Vec::new();
	let mut cells:		Vec<Vec<Inline>>	= Vec::new();
	for arg in split_top_args(inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"columns"	=> { ncols = parse_columns(&val); weights = parse_column_weights(&val); },
				"align"		=> align = parse_align(&val),
				"fill"		=> if fill_marks_header(&val) { header = true; },
				"inset"		=> inset_pt = parse_length(&val).map(length_pt),
				_			=> {},	// stroke, gutter and the rest are not modelled
			}
			continue;
		}
		if let Some(name) = spread_name(a) {
			if let Some(v) = arrays.get(&name) {
				cells.extend(v.iter().cloned());
			}
			continue;
		}
		// An inline positional cell, or a `table.header(...)`/`table.cell(...)` wrapper whose bracketed
		// content is the cell -- each `[...]` group in the argument is one cell, in order.
		if a.contains('[') {
			cells.extend(collect_cells(a));
		}
	}
	if cells.is_empty() {
		return None;
	}
	Some(TableSpec { ncols: ncols.max(1), header, align, weights, text_pt: outer_text_pt, inset_pt, cells })
}

/// The absolute point value of a [`Length`], resolving a percentage against a nominal 100 pt so a
/// percentage inset still yields a sensible padding; a table's inset is in practice an absolute length.
pub(crate) fn length_pt(len: Length) -> f64 {
	match len {
		Length::Abs(pt)	=> pt,
		Length::Rel(f)	=> f * 100.0,
	}
}

/// The size of a `text(size: Npt)[...]` (or `#text(...)`) wrapper at the start of `text`, in points, or
/// `None` when the body is not wrapped in a sized `text` call. This lets a figure's small-set table --
/// `text(size: 7pt)[#table(...)]` -- carry its reduced size, which Typst applies to the whole table.
fn outer_text_size(text: &str) -> Option<f64> {
	let inner = call_inner(text, "text")?;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if let Some((key, val)) = named_arg(a) {
			if key == "size" {
				return parse_length(&val).map(length_pt);
			}
		} else if a.ends_with("pt") || a.ends_with("em") {
			// The first positional length is the size, as in `text(7pt)[...]`.
			return parse_length(a).map(length_pt);
		}
	}
	None
}

/// Parses a `#figure(...)` call (its buffer, a trailing `<label>` and all) into an [`Item::Figure`]. The
/// positional argument is the body -- a wrapped `#table(...)` set in full, or an image call stood in for
/// by a placeholder; `caption:` sets the caption, `supplement:`/`kind:` the "Figure" or "Table" label.
fn parse_figure(buf: &str, arrays: &HashMap<String, Vec<Vec<Inline>>>) -> Option<Item> {
	let (body_src, label)	= strip_trailing_label(buf);
	let inner				= call_inner(&body_src, "figure")?;

	let mut caption:	Option<Vec<Inline>>	= None;
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
				"caption"		=> caption = Some(caption_inlines(&val)),
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
fn figure_body(text: &str, arrays: &HashMap<String, Vec<Vec<Inline>>>) -> FigureBody {
	if let Some(inner) = call_inner(text, "table") {
		if let Some(spec) = parse_table_spec(&inner, arrays, outer_text_size(text)) {
			return FigureBody::Table(spec);
		}
	}
	// A CeTZ/Fletcher diagram, bar chart or line plot drawn inline is read into a builder that draws it
	// for real; only when the body is none of these does it fall through to the image/placeholder path.
	if let Some(cf) = super::codefig::parse_code_figure(text) {
		return FigureBody::Code(cf);
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
pub(crate) fn parse_length(val: &str) -> Option<Length> {
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

/// Parses a standalone `#line(length:.., stroke:..)` into an [`Item::Rule`]. The length is a fraction of
/// the measure (`100%`) or an absolute length; the stroke gives the rule's thickness (a `pt` length) and
/// its grey (a `luma(N)` component). A missing length fills the measure; a missing thickness is a hairline
/// half-point; a missing colour is black, Typst's default stroke.
fn parse_line_rule(trimmed: &str) -> Option<Item> {
	let inner		= call_inner(trimmed, "line")?;
	let mut width	= Length::Rel(1.0);
	let mut thickness	= 0.5;
	let mut grey	= 0u8;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"length"	=> if let Some(l) = parse_length(&val) { width = l; },
				"stroke"	=> {
					let (t, g) = parse_stroke(&val);
					if let Some(t) = t { thickness = t; }
					if let Some(g) = g { grey = g; }
				},
				_			=> {},	// start, end, angle and the rest do not affect a horizontal divider
			}
		}
	}
	Some(Item::Rule { width, thickness, grey, span: Span::new(0, 0) })
}

/// The thickness (a `pt` length) and grey (a `luma(N)` value, 0-255) of a `stroke:` value such as
/// `0.5pt + luma(180)`; either component may be absent. A `luma` is read as a grey level; a bare colour
/// name or an `rgb(...)` is not modelled and leaves the grey unset.
fn parse_stroke(val: &str) -> (Option<f64>, Option<u8>) {
	let mut thickness:	Option<f64>	= None;
	let mut grey:		Option<u8>	= None;
	for part in val.split('+') {
		let p = part.trim();
		if let Some(inner) = call_inner(p, "luma") {
			if let Ok(n) = inner.trim().trim_end_matches('%').trim().parse::<f64>() {
				grey = Some(n.clamp(0.0, 255.0) as u8);
			}
		} else if let Some(Length::Abs(pt)) = parse_length(p) {
			thickness = Some(pt);
		}
	}
	(thickness, grey)
}

/// Reads a percentage argument (`100%`) into a fraction (`1.0`), or `None` when it is not a percentage.
fn parse_percent(val: &str) -> Option<f64> {
	val.trim().strip_suffix('%').and_then(|p| p.trim().parse::<f64>().ok()).map(|n| n / 100.0)
}

/// The content of the first `name(...)` call in `text`, balanced across nesting and strings, or `None`.
/// `name` must sit at a word boundary, so a short name does not match inside a longer identifier.
pub(crate) fn call_inner(text: &str, name: &str) -> Option<String> {
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
pub(crate) fn first_string(text: &str) -> Option<String> {
	let chars:	Vec<char>	= text.chars().collect();
	let start				= chars.iter().position(|&c| c == '"')?;
	let end					= (start + 1..chars.len()).find(|&j| chars[j] == '"')?;
	Some(chars[start + 1..end].iter().collect())
}

/// Splits the inner text of a call by its top-level commas, respecting `()[]{}` nesting and `"..."`
/// strings, so a comma inside a nested group or a string does not part an argument.
pub(crate) fn split_top_args(inner: &str) -> Vec<String> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut args:	Vec<String>	= Vec::new();
	let mut cur					= String::new();
	let mut state				= SkipState::new();
	let mut i					= 0;
	while i < chars.len() {
		// A comma parts the arguments only at the top level; inside any frame -- a nested group, a string,
		// a maths span or a content block -- it is literal and joins the current argument.
		if !state.is_open() && chars[i] == ',' {
			args.push(std::mem::take(&mut cur));
			i += 1;
			continue;
		}
		let consumed = state.step(&chars, i);
		for k in i..i + consumed {
			cur.push(chars[k]);
		}
		i += consumed;
	}
	if !cur.trim().is_empty() {
		args.push(cur);
	}
	args
}

/// Splits a `key: value` argument at its top-level colon, returning the key and the trimmed value, or
/// `None` when there is no top-level colon or the key is not a bare identifier -- so a positional cell
/// or a spread is not mistaken for a named argument.
pub(crate) fn named_arg(arg: &str) -> Option<(String, String)> {
	let chars:	Vec<char>	= arg.chars().collect();
	let mut state			= SkipState::new();
	let mut i				= 0;
	while i < chars.len() {
		// A colon names the argument only at the top level; inside any frame it is part of the value (an
		// alignment `align: (col, row) => ...`, a ratio in a caption, a dictionary key in code).
		if !state.is_open() && chars[i] == ':' {
			let key: String = chars[..i].iter().collect();
			let key = key.trim().to_string();
			if !key.is_empty() && key.chars().all(is_call_ident) {
				let val: String = chars[i + 1..].iter().collect();
				return Some((key, val.trim().to_string()));
			}
			return None;
		}
		i += state.step(&chars, i);
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

/// The per-column fractional weights of a `columns:` track list: a track `Nfr` (or a bare `fr`, weight 1)
/// contributes its weight, an `auto` or a fixed length contributes `0.0` so the column is sized to its
/// content. A bare `columns: N` gives no weights (an empty vector), leaving every column content-sized.
/// The weights let [`table::lower`](crate::table) reproduce Typst's fractional column sizing rather than
/// sizing every column from its widest cell.
fn parse_column_weights(val: &str) -> Vec<f64> {
	let v = val.trim();
	if !v.starts_with('(') {
		return Vec::new();
	}
	let ch: Vec<char> = v.chars().collect();
	let inner = match read_group(&ch, 0) {
		Some((inner, _))	=> inner,
		None				=> return Vec::new(),
	};
	let mut out = Vec::new();
	for track in split_top_args(&inner) {
		let t = track.trim();
		if t.is_empty() {
			continue;
		}
		out.push(track_weight(t));
	}
	out
}

/// The fractional weight of one `columns:` track: `Nfr` reads as `N`, a bare `fr` as `1`, and any other
/// track -- `auto`, `3cm`, `40pt`, `20%` -- as `0.0`, which marks the column content-sized.
fn track_weight(track: &str) -> f64 {
	match track.strip_suffix("fr") {
		Some(num) => {
			let n = num.trim();
			if n.is_empty() { 1.0 } else { n.parse::<f64>().unwrap_or(0.0) }
		},
		None => 0.0,
	}
}

/// Parses an `align:` value: a `(col, row) => ...` closure as [`AlignSpec::Closure`] (its parameter
/// names and body captured for per-cell evaluation), a tuple of column alignments as
/// [`AlignSpec::PerColumn`], a single alignment word as [`AlignSpec::Uniform`].
fn parse_align(val: &str) -> AlignSpec {
	let v = val.trim();
	if let Some(arrow) = v.find("=>") {
		let params	= v[..arrow].trim();
		let body	= v[arrow + 2..].trim().to_string();
		// The parameter list `(col, row)`; a bare single parameter has no parentheses. The first names the
		// column, the second the row, matching Typst's `(col, row)` order.
		let names: Vec<String> = {
			let pch: Vec<char> = params.chars().collect();
			match read_group(&pch, 0) {
				Some((inner, _))	=> split_top_args(&inner).iter().map(|s| s.trim().to_string()).collect(),
				None				=> vec![params.trim().to_string()],
			}
		};
		let col_var = names.first().cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "col".to_string());
		let row_var = names.get(1).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "row".to_string());
		return AlignSpec::Closure(ClosureAlign { col_var, row_var, body });
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

/// Does a `fill:` value key on the first row, marking a header? A `fill: (col, row) => ...` closure whose
/// body tests the row index against zero (`row == 0` or the common `y == 0`) fills the first row, which is
/// the books' header idiom; a `y < n` band likewise begins at the first row. Written with or without
/// spaces, and matching either name the closure gives its second (row) parameter.
fn fill_marks_header(val: &str) -> bool {
	let compact: String = val.chars().filter(|c| !c.is_whitespace()).collect();
	compact.contains("row==0")
		|| compact.contains("y==0")
		|| compact.contains("row<")
		|| compact.contains("y<")
}

/// Parses a `caption: [...]` value into its inline runs: the bracket content scanned for markup, or the
/// whole value scanned when it is not a bracket group, so a caption's emphasis, superscript or in-caption
/// maths sets with its own face rather than flattening to upright text.
fn caption_inlines(val: &str) -> Vec<Inline> {
	let v = val.trim();
	let ch: Vec<char> = v.chars().collect();
	if ch.first() == Some(&'[') {
		if let Some((content, _)) = read_group(&ch, 0) {
			return parse_inlines(&content);
		}
	}
	parse_inlines(v)
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::{Atom, MatKind};

	/// A `#claim-label`/`#claim-refs` marker sets nothing in the body, and the prose on either side
	/// closes over the gap as one text run, so no raw markup leaks.
	#[test]
	fn claim_marker_sets_nothing_inline() {
		let runs = parse_inlines("clinical authority#claim-label(<LS8>) bites hardest.");
		assert_eq!(runs.len(), 1);
		match &runs[0] {
			Inline::Text(t) => assert_eq!(t, "clinical authority bites hardest."),
			other => panic!("expected one text run, got {:?}", other),
		}
		// The multi-code and metadata-only forms are consumed the same way.
		assert_eq!(
			flatten_markup("margins#claim-label(<CD14>, <CD15>, <CD4>) formalised#claim-refs(<A1>)."),
			"margins formalised.");
	}

	/// A line that opens with a claim marker is prose, not a standalone call the line scanner skips, so
	/// the sentence that follows the marker is set rather than dropped with it.
	#[test]
	fn line_leading_claim_marker_is_prose() {
		assert!(is_inline_call("claim-label"));
		assert!(is_inline_call("claim-refs"));
		assert!(code_skip("#claim-label(<CD18>). Equilibrium appropriation follows.").is_none());
	}

	/// A line-leading `#padded-image(...)` (a section opener's logo) is set as an [`Item::Image`] carrying
	/// its path and scale, not skipped as a template call and not wrapped in a numbered figure.
	#[test]
	fn line_leading_padded_image_reads_as_image() -> Outcome<()> {
		let (items, _skips) = res!(document_with_skips(
			"= Pearl\n\n#padded-image(\"assets/svg/pearlite_logo_text_right.svg\", scale: 45%)\n\nPearl is the format.\n"));
		let img = res!(items.iter().find_map(|it| match it {
			Item::Image { path, scale, .. }	=> Some((path.clone(), *scale)),
			_								=> None,
		}).ok_or_else(|| err!("no Item::Image was produced for the standalone padded-image"; Test, Bug)));
		assert_eq!(img.0, "assets/svg/pearlite_logo_text_right.svg", "the image path is read");
		assert_eq!(img.1, Some(0.45), "the padded-image scale is read as a fraction");
		// The line must not have been swallowed as a skipped construct, nor turned into a figure.
		assert!(!items.iter().any(|it| matches!(it, Item::Figure { .. })), "a section logo is not a figure");
		Ok(())
	}

	/// A line-leading `#section-banner("logo")` (a documentation section opener) is read as an
	/// [`Item::SectionBanner`] carrying its logo path, not skipped as a template call and not confused with a
	/// plain `#image`.
	#[test]
	fn line_leading_section_banner_reads_as_banner() -> Outcome<()> {
		let (items, skips) = res!(document_with_skips(
			"#section-banner(\"assets/svg/fe2o3_logo_text_right.svg\")\n\n= Steel Server\n\nSteel is the server.\n"));
		let path = res!(items.iter().find_map(|it| match it {
			Item::SectionBanner { path, .. }	=> Some(path.clone()),
			_								=> None,
		}).ok_or_else(|| err!("no Item::SectionBanner was produced for the standalone section-banner"; Test, Bug)));
		assert_eq!(path, "assets/svg/fe2o3_logo_text_right.svg", "the banner logo path is read");
		// The call must not have been swallowed as a skipped construct, nor read as a plain centred image.
		assert!(!skips.entries().iter().any(|(name, _)| name == "#section-banner"),
			"a section banner must not be reported as a skipped construct");
		assert!(!items.iter().any(|it| matches!(it, Item::Image { .. })), "a section banner is not a plain image");
		Ok(())
	}

	/// A line-leading `#styled-box[...]` callout, its body opening on the marker line and closing on a later
	/// one, is gathered whole and read as an [`Item::Box`] holding the re-parsed body -- not skipped as an
	/// unbalanced standalone call, which would drop the callout's text. The construct is set, so it is not
	/// reported as a skipped construct.
	#[test]
	fn line_leading_styled_box_reads_as_box() -> Outcome<()> {
		let (items, skips) = res!(document_with_skips(
			"Lead prose.\n\n#styled-box[\n*Principle.* Every participant is accountable.\n]\n\nTrailing prose.\n"));
		let inner = res!(items.iter().find_map(|it| match it {
			Item::Box { items, .. }	=> Some(items.clone()),
			_						=> None,
		}).ok_or_else(|| err!("no Item::Box was produced for the standalone styled-box"; Test, Bug)));
		// The body re-parses to a paragraph, and its lead-in bold survives as a strong run.
		let has_para = inner.iter().any(|it| matches!(it, Item::Paragraph { .. }));
		assert!(has_para, "the styled-box body must re-parse to a paragraph, got: {:?}", inner);
		let has_strong = inner.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if runs.iter().any(|r| matches!(r, Inline::Strong(t) if t == "Principle."))));
		assert!(has_strong, "the body's bold lead-in must survive, got: {:?}", inner);
		// The callout must not have been swallowed as a skipped construct.
		assert!(!skips.entries().iter().any(|(name, _)| name == "#styled-box"),
			"a styled-box must not be reported as a skipped construct");
		// The prose either side of the callout still sets.
		assert!(items.iter().any(|it| matches!(it, Item::Paragraph { runs, .. }
			if runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Lead prose."))))),
			"prose before the callout is dropped");
		Ok(())
	}

	/// `#emph[...]` is the call form of `_..._`: it yields an [`Inline::Emph`] run with the same inner
	/// text, and nothing raw leaks.
	#[test]
	fn emph_call_reads_as_emphasis() {
		let runs = parse_inlines("The cube asks #emph[who] does the extracting.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Emph(t) if t == "who")),
			"emph run missing: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#emph"))),
			"raw #emph leaked: {:?}", runs);
		// A call carrying its own markup expands the same way `_..._` does.
		let nested = parse_inlines("#emph[the #idx[Harvard Business Review] weekly]");
		assert!(nested.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#emph") || t.contains("#idx"))),
			"raw markup leaked from nested emph: {:?}", nested);
	}

	/// Emphasis nested one level -- `*_x_*` or `_*x*_` -- collapses to a single [`Inline::BoldItalic`]
	/// run rather than dropping the outer face, so a bold-italic term sets in the bold-italic face in
	/// prose, a footnote or a table cell alike.
	#[test]
	fn nested_emphasis_reads_as_bold_italic() {
		for src in ["here *_both_* faces", "here _*both*_ faces"] {
			let runs = parse_inlines(src);
			assert!(runs.iter().any(|r| matches!(r, Inline::BoldItalic(t) if t == "both")),
				"expected a bold-italic run from {:?}, got {:?}", src, runs);
			assert!(runs.iter().all(|r| !matches!(r, Inline::Emph(t) if t == "both")),
				"outer face dropped to plain emph in {:?}: {:?}", src, runs);
		}
		// A lone `*strong*` or `_emph_` still yields its single face, unchanged.
		assert!(parse_inlines("just *strong* here").iter().any(|r| matches!(r, Inline::Strong(t) if t == "strong")));
		assert!(parse_inlines("just _emph_ here").iter().any(|r| matches!(r, Inline::Emph(t) if t == "emph")));
	}

	/// A `(col, row) => ...` alignment closure is evaluated per cell: a row-keyed closure centres the
	/// header and flushes the body left; a per-column closure honours each column's own alignment,
	/// including an `or` over several columns.
	#[test]
	fn align_closure_evaluates_per_cell() {
		let row_keyed = parse_align("(col, row) => { if row == 0 { center } else { left } }");
		match row_keyed {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 0), Align::Centre);	// header, column 0
				assert_eq!(cl.align_at(0, 1), Align::Left);	// body, column 0 -- was wrongly centred before
				assert_eq!(cl.align_at(2, 3), Align::Left);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
		let per_col = parse_align("(col, row) => { if row == 0 { center } else if col == 0 or col == 4 { left } else { center } }");
		match per_col {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 1), Align::Left);
				assert_eq!(cl.align_at(4, 1), Align::Left);
				assert_eq!(cl.align_at(1, 1), Align::Centre);
				assert_eq!(cl.align_at(3, 2), Align::Centre);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
		// A tuple pick indexed by the column parameter.
		let tuple = parse_align("(x, y) => (left, center, right).at(x)");
		match tuple {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 5), Align::Left);
				assert_eq!(cl.align_at(1, 5), Align::Centre);
				assert_eq!(cl.align_at(2, 5), Align::Right);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
	}

	/// `#super[...]` yields an [`Inline::Super`] run of its content, in both the bracket and the string
	/// argument forms, and never leaves raw source behind.
	#[test]
	fn super_call_reads_as_superscript() {
		let runs = parse_inlines("The area is 10#super[6] units, split#super[†] on the case.");
		let sups: Vec<&String> = runs.iter().filter_map(|r| match r {
			Inline::Super(t) => Some(t),
			_ => None,
		}).collect();
		assert_eq!(sups, vec!["6", "†"], "unexpected superscript runs: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#super"))),
			"raw #super leaked: {:?}", runs);
		// The string-argument form reads the same, its quotes stripped.
		let quoted = parse_inlines("x#super(\"2\")");
		assert!(quoted.iter().any(|r| matches!(r, Inline::Super(t) if t == "2")),
			"quoted super run missing: {:?}", quoted);
	}

	/// A citation nested in emphasis keeps its own [`Inline::Cite`] run rather than leaking its source,
	/// while the surrounding words take the emphasis face.
	#[test]
	fn cite_survives_inside_emphasis() {
		let runs = parse_inlines("the classic _early work #cite(<coase1937nature>) here_.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Cite(k) if k == &vec!["coase1937nature".to_string()])),
			"cite run missing: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#cite"))),
			"raw #cite leaked: {:?}", runs);
	}

	/// `#link("url")[text]` sets the link text in the running line and drops the URL, so no raw `#link`
	/// leaks; a bare `#link("url")` with no bracket sets the URL as its own text.
	#[test]
	fn link_call_renders_text_not_markup() {
		let runs = parse_inlines("See #link(\"https://aistatement.com\")[Centre for AI Safety, May 2023] on risk.");
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#link") || t.contains("http"))),
			"raw link markup leaked: {:?}", runs);
		let joined = flatten_markup("See #link(\"https://aistatement.com\")[Centre for AI Safety, May 2023] on risk.");
		assert_eq!(joined, "See Centre for AI Safety, May 2023 on risk.");
		// The label-destination form and the bare no-body form both set text without leaking.
		assert_eq!(flatten_markup("read #link(<intro>)[the opening]"), "read the opening");
		assert_eq!(flatten_markup("at #link(\"elearnity.io\")"), "at elearnity.io");
		// A line-leading link is prose the inline scanner reads, not a standalone call the line scanner drops.
		assert!(is_inline_call("link"));
		assert!(code_skip("#link(\"https://x.io\")[click]").is_none());
	}

	/// The term-dictionary aliases set their argument text with the styling of their `gs` siblings: `g`/`gi`
	/// a first-use glossary term, `t`/`tcap` plain text, and none of them leak raw markup. A line opening
	/// with one is prose, not a skipped standalone call.
	/// Installs a fixed term dictionary so the term-dictionary family resolves deterministically. The map
	/// is a process-global shared across the parallel tests, so every term-dependent test installs the
	/// same one and their order cannot matter.
	fn install_test_terms() {
		let mut m = HashMap::new();
		m.insert("org".to_string(),			"Elearnity Pty Ltd".to_string());
		m.insert("org_short".to_string(),	"Elearnity".to_string());
		m.insert("website".to_string(),		"elearnity.oxegen.io".to_string());
		m.insert("iniverse".to_string(),	"iniverse".to_string());
		set_term_dict(m).expect("install test term dict");
	}

	#[test]
	fn term_dict_aliases_render_without_leaking() {
		install_test_terms();
		// `iniverse` translates to itself, so the display equals the key here whether or not a map is set.
		let runs = parse_inlines("Call it the #g[iniverse], your inner universe.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Glossary { term, display } if term == "iniverse" && display == "iniverse")),
			"glossary alias missing: {:?}", runs);
		// A key that differs from its value now sets the value, not the key.
		assert_eq!(flatten_markup("Visit #t[website] today"), "Visit elearnity.oxegen.io today");
		// A key absent from the dictionary falls back to its own text, capitalised for the `-cap` form.
		assert_eq!(flatten_markup("#tcap[donate] to help"), "Donate to help");
		assert!(is_inline_call("g") && is_inline_call("t") && is_inline_call("graw"));
		assert!(code_skip("#t[website]").is_none());
	}

	/// The term-dictionary family sets a key's value: `t`/`graw` plain, `tcap` capitalised, `g` bold-italic
	/// on first use keyed by the key; an unknown key falls back to the key text and is recorded, never a
	/// panic (the template panics on a miss, the reader must not).
	#[test]
	fn term_dict_resolves_key_to_value() {
		install_test_terms();
		assert_eq!(flatten_markup("Visit #t[website]"), "Visit elearnity.oxegen.io");
		assert_eq!(flatten_markup("#graw[org]"), "Elearnity Pty Ltd");
		let runs = parse_inlines("The #g[org] view.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Glossary { term, display }
				if term == "org" && display == "Elearnity Pty Ltd")),
			"g did not translate the key to its value: {:?}", runs);
		// An unknown key: the key text stands and the miss is recorded on the skip tally.
		let mut skips = SkipSummary::default();
		let runs = parse_inlines_in("A #t[nonesuch] term.", &mut skips);
		assert!(runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("nonesuch"))),
			"unknown term-dict key did not fall back to its text: {:?}", runs);
		assert_eq!(skips.total(), 1, "an unknown term-dict key was not recorded");
	}

	/// A blank line between numbered items does not restart the enum: Typst continues the numbering across
	/// the gap, so the items form one list. Real content between two lists still starts a fresh one.
	#[test]
	fn blank_line_between_enum_items_continues_one_list() {
		let src = "+ first item\n\n+ second item\n\n+ third item\n";
		let (items, _) = document_with_skips(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "blank lines split the enum: {:?}", items);
		match lists[0] {
			Item::List { ordered, items, .. } => {
				assert!(*ordered, "the continued list lost its ordered kind");
				assert_eq!(items.len(), 3, "the enum dropped items across the blanks: {:?}", items);
			},
			_ => unreachable!(),
		}
		// A paragraph between two lists still restarts, so genuinely separate lists are not merged.
		let src2 = "+ a\n\n+ b\n\nA paragraph between.\n\n+ c\n";
		let (items2, _) = document_with_skips(src2).expect("parse");
		let lists2 = items2.iter().filter(|it| matches!(it, Item::List { .. })).count();
		assert_eq!(lists2, 2, "prose between two lists did not restart them: {:?}", items2);
	}

	/// An indented `-` sub-bullet between two `+` steps nests under the step it follows rather than closing
	/// the enum: the ordered list stays one list of three items, and the sub-bullet hangs under the first.
	#[test]
	fn indented_sub_bullet_nests_and_enum_continues() {
		let src = "+ step one\n  - a sub point\n  - another sub point\n+ step two\n+ step three\n";
		let (items, _) = document_with_skips(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "the sub-bullet split the enum into several lists: {:?}", items);
		match lists[0] {
			Item::List { ordered, items, .. } => {
				assert!(*ordered, "the parent list lost its ordered kind");
				assert_eq!(items.len(), 3, "the enum did not keep three steps: {:?}", items);
				// The sub-bullets hang under the first step, as an unordered child list of two items.
				assert_eq!(items[0].children.len(), 1, "the first step lost its sub-list: {:?}", items[0]);
				match &items[0].children[0] {
					Item::List { ordered: cord, items: citems, .. } => {
						assert!(!*cord, "the sub-list should be unordered");
						assert_eq!(citems.len(), 2, "the sub-list dropped an item: {:?}", citems);
					},
					other => panic!("the child was not a nested list: {:?}", other),
				}
				assert!(items[1].children.is_empty(), "step two wrongly gained children");
			},
			_ => unreachable!(),
		}
	}

	/// Two levels of indentation parse to two levels of nesting: a `-` under a `+`, and a deeper `-` under
	/// that `-`, so the tree is enum -> bullet -> bullet.
	#[test]
	fn two_level_nesting_parses_to_two_levels() {
		let src = "+ outer step\n  - middle bullet\n    - inner bullet\n+ next step\n";
		let (items, _) = document_with_skips(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "the deep nesting split the list: {:?}", items);
		match lists[0] {
			Item::List { items, .. } => {
				assert_eq!(items.len(), 2, "the outer enum did not keep two steps: {:?}", items);
				let mid = &items[0].children;
				assert_eq!(mid.len(), 1, "the middle level is missing: {:?}", items[0]);
				match &mid[0] {
					Item::List { items: mid_items, .. } => {
						assert_eq!(mid_items.len(), 1, "the middle list should hold one bullet");
						let inner = &mid_items[0].children;
						assert_eq!(inner.len(), 1, "the inner level is missing: {:?}", mid_items[0]);
						match &inner[0] {
							Item::List { items: inner_items, .. } =>
								assert_eq!(inner_items.len(), 1, "the inner list should hold one bullet"),
							other => panic!("the inner child was not a list: {:?}", other),
						}
					},
					other => panic!("the middle child was not a list: {:?}", other),
				}
			},
			_ => unreachable!(),
		}
	}

	/// An unhandled inline `#func[...]` is consumed and recorded rather than left as raw markup, its
	/// bracketed body folded in so its words survive; a paren-only call sets nothing where it stood.
	#[test]
	fn unknown_inline_call_is_recorded_not_leaked() {
		let mut skips = SkipSummary::default();
		let runs = parse_inlines_in("a #smallcaps[Nato] treaty and a #v(2pt) gap", &mut skips);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#smallcaps") || t.contains("#v("))),
			"raw unknown call leaked: {:?}", runs);
		assert!(runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Nato"))),
			"smallcaps body dropped: {:?}", runs);
		assert_eq!(skips.total(), 2);
		let names: Vec<String> = skips.entries().into_iter().map(|(n, _)| n).collect();
		assert!(names.contains(&"#smallcaps".to_string()) && names.contains(&"#v".to_string()),
			"unexpected skip names: {:?}", names);
	}

	/// The reader tallies the code lines and unknown calls it skips, and reports them one line, so a
	/// dropped construct is visible rather than silent. Handled inline calls do not appear in the tally.
	#[test]
	fn skip_summary_reports_skipped_constructs() {
		install_test_terms();	// so `#g[iniverse]` resolves and adds no term-dict miss to the tally
		let src = "#import \"x.typ\": *\n#set page(margin: 1cm)\n\nBody with #g[iniverse] and a #footnote[note].\n\n#show heading: it => it\n";
		let (_, skips) = document_with_skips(src).expect("parse");
		assert_eq!(skips.total(), 3);
		let report = skips.report().expect("a report");
		assert!(report.starts_with("skipped 3 unsupported constructs:"), "report was {:?}", report);
		for name in ["#import", "#set", "#show"] {
			assert!(report.contains(name), "{} missing from {:?}", name, report);
		}
		// A source the reader sets whole has nothing to report.
		let (_, clean) = document_with_skips("Just prose with #g[iniverse].\n").expect("parse");
		assert!(clean.is_empty() && clean.report().is_none());
	}

	/// A `#columns(n)[ ... ]` wrapper is recorded as skipped and its body set single-column, so the words
	/// survive and no raw wrapper leaks into the block stream.
	#[test]
	fn columns_wrapper_flattens_to_single_column() {
		let src = "#columns(2)[\nFirst paragraph here.\n\nSecond paragraph here.\n]\n";
		let (items, skips) = document_with_skips(src).expect("parse");
		let paras = items.iter().filter(|it| matches!(it, Item::Paragraph { .. })).count();
		assert_eq!(paras, 2, "column body not set as paragraphs: {:?}", items);
		assert_eq!(skips.entries(), vec![("#columns".to_string(), 1)]);
	}

	/// Reads a single [`Item::Paragraph`]'s runs out of a parse, failing loudly with the whole item list
	/// when the source did not yield exactly one paragraph -- the shape every `math_open` regression test
	/// below expects, since a display block that leaked a line would instead split the source into a
	/// paragraph plus a stray heading or list.
	fn one_paragraph(items: &[Item]) -> Outcome<(Vec<Inline>, Option<String>)> {
		let paras: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::Paragraph { .. })).collect();
		match paras.as_slice() {
			[Item::Paragraph { runs, label, .. }] => Ok((runs.clone(), label.clone())),
			_ => Err(err!("expected exactly one paragraph, got: {:?}", items; Test, Bug)),
		}
	}

	/// A source line inside an open `$...$` display block that begins `=` (an alignment row such as
	/// `=> 2N &= ...`, Oxegen TechSpec `app_maths.typ:533-534`) must stay in the equation, not be read as
	/// a heading -- the heading branch is gated off by `math_open` for exactly this shape.
	#[test]
	fn equals_lead_row_inside_display_math_stays_in_block() -> Outcome<()> {
		let src = "Total hashes.\n\n$\nN &= sum_(j=1)^J n_j \\\n=> 2N &= sum_(j=2)^(J+1) 2^(j-1) \\\n=> 2N - N &= 2^J - 1 \\\n$\n\nEnd of block.\n";
		let (items, _skips) = res!(document_with_skips(src));
		assert!(!items.iter().any(|it| matches!(it, Item::Heading { .. })),
			"a `=`-lead row inside the block must not become a heading: {:?}", items);
		let has_align = items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if matches!(runs.as_slice(),
				[Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })])));
		assert!(has_align, "no single-run display alignment paragraph was produced: {:?}", items);
		Ok(())
	}

	/// A source line inside an open `$...$` display block that begins `-` (a row such as
	/// `- tilde(N)_(1 0) u (...) = 0`, Oxegen TechSpec `ch04_nodes.typ:1657`) must stay in the equation,
	/// not be read as a bullet -- the list-marker branch is gated off by `math_open` for exactly this shape.
	#[test]
	fn dash_lead_row_inside_display_math_stays_in_block() -> Outcome<()> {
		let src = "$\na &= b \\\n- tilde(N)_(1 0) u (x) = 0 \\\nc &= d\n$\n";
		let (items, _skips) = res!(document_with_skips(src));
		assert!(!items.iter().any(|it| matches!(it, Item::List { .. })),
			"a `-`-lead row inside the block must not become a list: {:?}", items);
		let (runs, _label) = res!(one_paragraph(&items));
		assert!(matches!(runs.as_slice(), [Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })]),
			"expected one display alignment run, got: {:?}", runs);
		Ok(())
	}

	/// A blank source line inside an open `$...$` display block (Oxegen TechSpec `ch04_nodes.typ:1661-1666`)
	/// must stay in the equation rather than flush the paragraph early, and a closing `$ <label>` still
	/// labels the resulting equation.
	#[test]
	fn blank_line_inside_display_math_stays_in_block_and_label_still_attaches() -> Outcome<()> {
		let src = "$\na &= b \\\n\nc &= d\n$ <eq_test>\n";
		let (items, _skips) = res!(document_with_skips(src));
		let (runs, label) = res!(one_paragraph(&items));
		assert!(matches!(runs.as_slice(), [Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })]),
			"expected one display alignment run, got: {:?}", runs);
		assert_eq!(label, Some("eq_test".to_string()), "the closing label must still attach");
		Ok(())
	}

	/// The plain text of a run of inline markup, for asserting a caption or paragraph's words without
	/// caring how they were split into text, emphasis, glossary or maths runs.
	fn plain(runs: &[Inline]) -> String {
		let mut s = String::new();
		for r in runs {
			match r {
				Inline::Text(t) | Inline::Strong(t) | Inline::Emph(t)
				| Inline::BoldItalic(t) | Inline::Super(t) | Inline::Code(t)	=> s.push_str(t),
				Inline::Glossary { display, .. }							=> s.push_str(display),
				_															=> {},
			}
		}
		s
	}

	/// The one [`Item::Figure`] in a parse, failing loudly with the item list when there is not exactly one.
	fn one_figure(items: &[Item]) -> Outcome<(Option<Vec<Inline>>, String, Option<String>)> {
		let figs: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::Figure { .. })).collect();
		match figs.as_slice() {
			[Item::Figure { caption, supplement, label, .. }]	=>
				Ok((caption.clone(), supplement.clone(), label.clone())),
			_	=> Err(err!("expected exactly one figure, got: {:?}", items; Test, Bug)),
		}
	}

	/// A `#figure(...)` whose caption prose carries an author's unbalanced `(` (Oxegen TechSpec
	/// `app_maths.typ:550-566`: "...network messages (latency $100 unit(\"ms\")$ ... chunk sizes $d$.")
	/// must still close at its own `)`: content mode treats the stray paren as literal prose, where the old
	/// flat depth counter stuck open to end of source and swallowed both the figure and the tail after it.
	#[test]
	fn figure_caption_with_unbalanced_paren_closes_and_tail_survives() -> Outcome<()> {
		let src = "\
#figure(
  block(width: 70%)[
    #table(
      columns: (10fr, 10fr),
      [a], [b],
    )
  ],
  caption: [Representative processing times for hashing and network messages (latency $100 unit(\"ms\")$ per message for a Merkle tree of $1 unit(\"GiB\")$ of datastate with various chunk sizes $d$.],
  kind: \"table\",
  supplement: \"Table\",
) <merkle_tree_chunk_size>

Trailing prose.
";
		let (items, _skips) = res!(document_with_skips(src));
		let (caption, supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the figure lost its caption"; Test, Bug)));
		assert!(plain(&caption).contains("Representative processing times"),
			"the caption prose was lost: {:?}", caption);
		assert_eq!(supplement, "Table", "the supplement was not read from the figure");
		assert_eq!(label, Some("merkle_tree_chunk_size".to_string()), "the figure label was lost");
		// The tail after the figure must survive rather than be swallowed by a stuck bracket count.
		let tail = items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Trailing prose")));
		assert!(tail, "the prose after the figure was swallowed: {:?}", items);
		Ok(())
	}

	/// A `(`, a `)`, a `[` or a `]` inside a `$...$` maths span in a caption is literal maths, never a
	/// structural bracket, so an interval or a parenthesised function does not miscount and close the
	/// caption early. Without the maths frame, the `]` in `$[a, b]$` would pop the caption content block.
	#[test]
	fn caption_maths_parens_do_not_count() -> Outcome<()> {
		let src = "\
#figure(
  image(\"fig.png\"),
  caption: [see $f(x)$ over $[a, b]$ where it holds.],
) <fig_maths>

After the figure.
";
		let (items, _skips) = res!(document_with_skips(src));
		let (caption, _supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the maths caption was lost"; Test, Bug)));
		assert!(plain(&caption).contains("see") && plain(&caption).contains("where it holds"),
			"the caption prose around the maths was truncated: {:?}", caption);
		assert!(caption.iter().any(|r| matches!(r, Inline::Math(_))),
			"the caption maths span was not parsed as maths: {:?}", caption);
		assert_eq!(label, Some("fig_maths".to_string()), "the label after a maths caption was lost");
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("After the figure"))),
			"the tail after a maths caption was swallowed: {:?}", items);
		Ok(())
	}

	/// A `(` or `)` inside a `"..."` string argument -- here an image path `image("a(b).png")` -- is literal
	/// string content, not a structural paren, so a single-line figure carrying such a path closes on its
	/// line rather than opening a run-away capture.
	#[test]
	fn code_string_paren_does_not_count() -> Outcome<()> {
		let src = "#figure(image(\"a(b).png\"), caption: [c])\n\nNext paragraph.\n";
		let (items, _skips) = res!(document_with_skips(src));
		let (caption, _supplement, _label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the figure lost its caption"; Test, Bug)));
		assert_eq!(plain(&caption), "c", "the caption was misread past the string paren: {:?}", caption);
		let path = items.iter().find_map(|it| match it {
			Item::Figure { body: FigureBody::Image { path, .. }, .. }	=> Some(path.clone()),
			_														=> None,
		});
		assert_eq!(path, Some("a(b).png".to_string()), "the image path with parens was misread: {:?}", path);
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Next paragraph"))),
			"the paragraph after a single-line figure was swallowed: {:?}", items);
		Ok(())
	}

	/// A `#name(...)` call met inside a caption's content re-enters code mode, so the string inside it is
	/// protected and a following unbalanced `(` in the surrounding prose is still literal: the caption
	/// closes at its own `]` and the tail survives.
	#[test]
	fn content_call_inside_caption_reenters_code() -> Outcome<()> {
		let src = "\
#figure(
  image(\"g.png\"),
  caption: [see #link(\"http://x\")[y] and note (z],
) <fig_call>

Following text.
";
		let (items, _skips) = res!(document_with_skips(src));
		let (caption, _supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the call caption was lost"; Test, Bug)));
		assert!(plain(&caption).contains("see") && plain(&caption).contains("note (z"),
			"the caption prose around the call was truncated: {:?}", caption);
		assert_eq!(label, Some("fig_call".to_string()), "the label after a call caption was lost");
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Following text"))),
			"the tail after a call caption was swallowed: {:?}", items);
		Ok(())
	}

	/// [`read_group`] and [`split_top_args`] on a caption argument list directly: an unbalanced `(` in the
	/// caption prose is literal, so the group closes at its own `]` and the top-level commas still part the
	/// arguments, with the maths span and its string protected throughout.
	#[test]
	fn read_group_and_split_handle_caption_prose_paren() -> Outcome<()> {
		// read_group on a caption bracket whose prose carries an unbalanced `(` and a `$...$` span with a
		// quoted `unit("ms")` inside: the group must end at the caption's own `]`, keeping its prose and
		// stopping before the trailing text.
		let s: Vec<char> = "[messages (latency $100 unit(\"ms\")$ per $d$.] more".chars().collect();
		let (inner, next) = res!(read_group(&s, 0)
			.ok_or_else(|| err!("the caption group did not close"; Test, Bug)));
		assert!(inner.contains("(latency"), "the caption prose was lost: {:?}", inner);
		assert!(!inner.contains("more"), "the caption group over-ran its closing bracket: {:?}", inner);
		assert_eq!(s[next..].iter().collect::<String>(), " more", "the index past the closer is wrong");

		// split_top_args across the same shape: three arguments, the middle a caption whose unbalanced paren
		// and maths span do not part it, and named_arg reads the caption key back off it.
		let args = split_top_args("image(\"p.png\"), caption: [x (y $z(w)$.], kind: \"table\"");
		assert_eq!(args.len(), 3, "the unbalanced caption paren split the arg list wrong: {:?}", args);
		let named = res!(named_arg(args[1].trim())
			.ok_or_else(|| err!("the caption argument did not read as named: {:?}", args[1]; Test, Bug)));
		assert_eq!(named.0, "caption", "the caption key was misread: {:?}", named);
		assert!(named.1.contains("(y"), "the caption value lost its unbalanced paren: {:?}", named);
		Ok(())
	}

	/// A prose line that opens with an inline `#raw("...")` and carries a stray `"` from an author's
	/// quotation split across the line break must be set as prose, not read as the start of a multi-line
	/// code skip: a dangling quote is a character, not an open code string. Only an unclosed structural
	/// bracket keeps a skip open (Hematite `sec_net.typ:679-681`, where `express "no\nbound".` split a
	/// quotation over two lines after an inline `#raw`).
	#[test]
	fn prose_line_with_inline_raw_and_dangling_quote_is_not_skipped() -> Outcome<()> {
		let src = "passes its own pair to #raw(\"read_message\"), or to\n\
#raw(\"WebSocket::with_limits\"). There is deliberately no way to express \"no\n\
bound\".\n";
		let (items, _skips) = res!(document_with_skips(src));
		let text: String = items.iter()
			.filter_map(|it| match it {
				Item::Paragraph { runs, .. }	=> Some(plain(runs)),
				_							=> None,
			})
			.collect::<Vec<_>>()
			.join(" ");
		assert!(text.contains("no way to express"),
			"the prose after an inline #raw was swallowed as a skip: {:?}", items);
		assert!(text.contains("bound"), "the continuation line was swallowed: {:?}", items);
		Ok(())
	}
}
