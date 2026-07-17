//! Reading HTML into the document tree: the pass that turns tags into blocks and inlines, and throws
//! the exporter's whitespace away.
//!
//! # The dialect
//!
//! The elements read are the ones the tree has a node for, and no others: the six headings,
//! paragraphs, lists, `pre`, quotations, thematic breaks and tables, and the inline run of emphasis,
//! links, images, code spans and breaks. Comments, `head`, `script` and `style` hold no prose and are
//! dropped, content and all.
//!
//! # Everything else is unwrapped
//!
//! A `div`, a `span`, and every element this has never heard of, contribute nothing themselves and are
//! erased: the tag goes and the content is read exactly where the tag stood. There is no generic
//! container in the tree to put them in, and inventing one would make every consumer learn HTML --
//! which is the one thing the tree exists to prevent.
//!
//! Erasing rather than recursing is what makes this work in both directions at once. A `div` holding
//! paragraphs is read as those paragraphs; a `div` holding bare words is read as the paragraph those
//! words are; a `span` in the middle of a sentence stays in the middle of that sentence, with the text
//! either side of it joined across the hole the tag left. The rule is the same in each case, and no
//! prose is lost in any of them.
//!
//! # Robustness
//!
//! The input this is written for is a generator's output: well formed, and wrong in none of the ways a
//! browser must survive. So there is no error recovery here beyond the cheap kind -- a stray close tag
//! closes nothing and the document carries on, an element left open runs to the end of what encloses
//! it. The one refusal is [`DEPTH_LIMIT`].

use crate::doc::{
	Align,
	Block,
	Cell,
	Inline,
	Row,
};
use crate::html::decode_entities;

use oxedyne_fe2o3_core::prelude::*;

use std::mem::take;

/// How deep a document may nest its elements before the reader refuses it.
///
/// A quotation inside a list inside a quotation is legitimate; a thousand of them is a document built
/// to exhaust the stack of whatever reads it. The limit is generous beside anything a generator
/// exports and far below what would trouble the machine.
///
/// Only the elements the tree has a node for are counted, because only those are read by a recursive
/// walk. A `div` and an unknown element are erased where they stand and cost no stack at all, so a
/// document of a million nested `div`s is read as the flat prose it says rather than refused.
pub const DEPTH_LIMIT: usize = 32;

/// Reads HTML into the blocks it is made of.
pub fn parse(src: &str) -> Outcome<Vec<Block>> {
	let mut lex = Lex { src, i: 0, raw: None };
	blocks(&mut lex, None, 0)
}

/// What an element is, which is what decides where its content goes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
	/// A heading, of the level its name gives.
	Heading(u8),
	/// A paragraph.
	Para,
	/// A list, ordered where the flag says so.
	List(bool),
	/// A run of code, whose whitespace is what it says.
	Code,
	/// A quotation.
	Quote,
	/// A table.
	Table,
	/// A thematic break.
	Rule,
	/// Emphasis, strong where the flag says so.
	Emph(bool),
	/// A link.
	Link,
	/// An image.
	Image,
	/// A code span within a line.
	Span,
	/// A break the author asked for.
	Break,
	/// Content that is not prose, and goes nowhere.
	Drop,
	/// Anything else: the tag is erased and its content read in its place.
	Bare,
}

impl Kind {

	/// Whether the element belongs within a line rather than standing on its own.
	fn is_inline(&self) -> bool {
		matches!(self, Self::Emph(_) | Self::Link | Self::Image | Self::Span | Self::Break)
	}
}

/// What an element is, by the name it was written with.
///
/// The name is lowered for the match, because HTML does not care how a tag is written and neither does
/// this. Anything not named here is [`Kind::Bare`].
fn kind(name: &str) -> Kind {
	match name.to_ascii_lowercase().as_str() {
		"h1"				=> Kind::Heading(1),
		"h2"				=> Kind::Heading(2),
		"h3"				=> Kind::Heading(3),
		"h4"				=> Kind::Heading(4),
		"h5"				=> Kind::Heading(5),
		"h6"				=> Kind::Heading(6),
		"p"				=> Kind::Para,
		"ul"				=> Kind::List(false),
		"ol"				=> Kind::List(true),
		"pre"				=> Kind::Code,
		"blockquote"			=> Kind::Quote,
		"table"				=> Kind::Table,
		"hr"				=> Kind::Rule,
		"em" | "i"			=> Kind::Emph(false),
		"strong" | "b"			=> Kind::Emph(true),
		"a"				=> Kind::Link,
		"img"				=> Kind::Image,
		"code"				=> Kind::Span,
		"br"				=> Kind::Break,
		"head" | "script" | "style"	=> Kind::Drop,
		_				=> Kind::Bare,
	}
}

/// The elements that hold nothing, and so need no closing tag.
const VOID: [&str; 14] = [
	"area",
	"base",
	"br",
	"col",
	"embed",
	"hr",
	"img",
	"input",
	"link",
	"meta",
	"param",
	"source",
	"track",
	"wbr",
];

/// The elements whose content is text and not markup, so that a `<` within them opens nothing.
const RAW: [&str; 2] = [
	"script",
	"style",
];

/// Whether a name is the given element's, without regard to the case it was written in.
fn is(name: &str, want: &str) -> bool {
	name.eq_ignore_ascii_case(want)
}

// -----------------------------------------------------------------------------------------------
// The walk.
// -----------------------------------------------------------------------------------------------

/// Reads the children of an element into the blocks they are, until the given element closes or the
/// input ends.
///
/// Loose inline content -- words that reach block level with no paragraph around them, which is what a
/// `div` full of prose leaves behind once it is erased -- is gathered into the paragraph it is.
fn blocks(lex: &mut Lex, until: Option<&str>, depth: usize) -> Outcome<Vec<Block>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"HTML elements nest more than {} deep, which no document written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let mut out = Vec::new();
	let mut run = Run::default();	// Loose inline content, awaiting the paragraph it makes.
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(t)		=> run.text(t),
			Tok::Close(name)	=> {
				// A close tag that answers nothing closes nothing, and the document carries on.
				if let Some(until) = until {
					if is(name, until) {
						break;
					}
				}
			}
			Tok::Open { name, attrs, void } => {
				let k = kind(name);
				match k {
					Kind::Drop		=> skip(lex, name, void),
					// The tag is erased, and its content read as though it had never been there.
					Kind::Bare		=> {}
					_ if k.is_inline()	=> {
						res!(inline(lex, &mut run, k, name, attrs, void, depth));
					}
					_			=> {
						flush(&mut out, &mut run);
						if let Some(b) = res!(block(lex, k, name, attrs, void, depth)) {
							out.push(b);
						}
					}
				}
			}
		}
	}
	flush(&mut out, &mut run);
	Ok(out)
}

/// Turns the loose inline content gathered so far into the paragraph it is, where it says anything.
///
/// A run of nothing but the whitespace that lay between two blocks says nothing, and makes no
/// paragraph.
fn flush(out: &mut Vec<Block>, run: &mut Run) {
	let content = take(run).end();
	if !content.is_empty() {
		out.push(Block::Para(content));
	}
}

/// Reads the block an opening tag begins, where the tree has one for it.
fn block(lex: &mut Lex, k: Kind, name: &str, attrs: &str, void: bool, depth: usize)
	-> Outcome<Option<Block>>
{
	let b = match k {
		Kind::Rule		=> Block::Rule,
		Kind::Code		=> code(lex, attrs, void),
		Kind::List(ordered)	=> match void {
			true	=> Block::List { ordered, items: Vec::new() },
			false	=> res!(list(lex, ordered, name, depth)),
		},
		Kind::Table		=> match void {
			true	=> Block::Table { head: None, rows: Vec::new(), cols: Vec::new() },
			false	=> res!(table(lex, depth)),
		},
		Kind::Quote		=> match void {
			true	=> Block::Quote(Vec::new()),
			false	=> Block::Quote(res!(blocks(lex, Some(name), depth + 1))),
		},
		Kind::Heading(level)	=> Block::Heading {
			level,
			content: match void {
				true	=> Vec::new(),
				false	=> res!(inlines(lex, name, depth + 1)),
			},
		},
		Kind::Para		=> {
			let content = match void {
				true	=> Vec::new(),
				false	=> res!(inlines(lex, name, depth + 1)),
			};
			// A paragraph of nothing but the whitespace an exporter laid it out with says nothing,
			// and the tree is better off without it.
			if content.is_empty() {
				return Ok(None);
			}
			Block::Para(content)
		}
		// Everything else here is a part of a list or a table that only its own reader sees. One that
		// reaches this stood outside the thing it belongs to, where it says nothing: the tag is erased
		// and the content within it is read where it stands.
		_			=> return Ok(None),
	};
	Ok(Some(b))
}

/// Reads the children of an element into the run of inlines they are, until the given element closes
/// or the input ends.
fn inlines(lex: &mut Lex, until: &str, depth: usize) -> Outcome<Vec<Inline>> {
	if depth > DEPTH_LIMIT {
		return Err(err!(
			"HTML elements nest more than {} deep, which no prose written to be read \
			does.", DEPTH_LIMIT;
			Excessive, Input));
	}
	let mut run = Run::default();
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(t)		=> run.text(t),
			Tok::Close(name)	=> {
				if is(name, until) {
					break;
				}
			}
			Tok::Open { name, attrs, void } => {
				let k = kind(name);
				match k {
					Kind::Drop		=> skip(lex, name, void),
					Kind::Bare		=> {}
					_ if k.is_inline()	=> {
						res!(inline(lex, &mut run, k, name, attrs, void, depth));
					}
					// A block within a line is a thing the tree cannot hold: a cell is given inlines
					// and nothing else, deliberately. The tag is erased like any other it has no node
					// for, but it stands as the boundary it is, so that the words either side of it
					// stay words apart rather than running together into one.
					_			=> run.space(),
				}
			}
		}
	}
	Ok(run.end())
}

/// Reads the inline element an opening tag begins into the run it belongs to.
fn inline(lex: &mut Lex, run: &mut Run, k: Kind, name: &str, attrs: &str, void: bool, depth: usize)
	-> Outcome<()>
{
	match k {
		Kind::Break	=> run.push(Inline::Break),
		Kind::Image	=> run.push(Inline::Image {
			src:	attr(attrs, "src").unwrap_or_default(),
			alt:	attr(attrs, "alt").unwrap_or_default(),
		}),
		Kind::Span	=> {
			// A code span is not a `pre`: HTML collapses the whitespace within one exactly as it does
			// anywhere else, and so does this. What is not trimmed is the space at either end, because
			// a span sits in a line and the space beside it is the line's.
			let text = match void {
				true	=> String::new(),
				false	=> text_in(lex, name),
			};
			run.push(Inline::Code(collapse(&text)));
		}
		Kind::Emph(strong)	=> {
			let content = match void {
				true	=> Vec::new(),
				false	=> res!(inlines(lex, name, depth + 1)),
			};
			run.push(Inline::Emph { strong, content });
		}
		Kind::Link	=> match attr(attrs, "href") {
			Some(to) => {
				let content = match void {
					true	=> Vec::new(),
					false	=> res!(inlines(lex, name, depth + 1)),
				};
				run.push(Inline::Link { to, content });
			}
			// An `a` with no destination is an anchor and not a link: there is nowhere for a reader to
			// go. The tag is erased, and the words within it stay in the line they were in.
			None => {}
		},
		// Nothing else reaches here: this is called only where the kind is inline.
		_		=> {}
	}
	Ok(())
}

/// Reads a `ul` or an `ol` into the list it is.
fn list(lex: &mut Lex, ordered: bool, until: &str, depth: usize) -> Outcome<Block> {
	let mut items: Vec<Vec<Block>> = Vec::new();
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(t)		=> {
				// The whitespace an exporter lays a list out with says nothing. Words that reach a
				// list with no item to sit in are still words, and stand as an item of their own
				// rather than being dropped.
				let s = collapse(&decode_entities(t));
				let s = s.trim();
				if !s.is_empty() {
					items.push(vec![Block::Para(vec![Inline::Text(s.to_string())])]);
				}
			}
			Tok::Close(name)	=> {
				if is(name, until) {
					break;
				}
			}
			Tok::Open { name, void, .. } => {
				if is(name, "li") {
					items.push(match void {
						true	=> Vec::new(),
						false	=> res!(blocks(lex, Some(name), depth + 1)),
					});
				} else if kind(name) == Kind::Drop {
					skip(lex, name, void);
				}
				// Anything else between the items is erased, so a list whose items are wrapped in
				// something the tree does not know still finds them.
			}
		}
	}
	Ok(Block::List { ordered, items })
}

/// Reads a `table` into the grid it is.
fn table(lex: &mut Lex, depth: usize) -> Outcome<Block> {
	let mut head: Option<Row> = None;
	let mut rows: Vec<Row> = Vec::new();
	let mut cols: Vec<Align> = Vec::new();
	let mut in_head = false;	// Whether the rows being read are the table's header.
	while let Some(tok) = lex.next() {
		match tok {
			// The whitespace a table is laid out with says nothing, and a table has nowhere to put a
			// word that reached it without a cell to sit in.
			Tok::Text(_)		=> {}
			Tok::Close(name)	=> {
				if is(name, "table") {
					break;
				}
				if is(name, "thead") {
					in_head = false;
				}
			}
			Tok::Open { name, void, .. } => {
				if is(name, "thead") {
					in_head = true;
				} else if is(name, "tr") && !void {
					let (r, all_th) = res!(row(lex, &mut cols, depth + 1));
					// A table names its columns in a `thead`, or in a first row of nothing but `th`.
					// Both say the same thing, and an exporter picks whichever it likes.
					if head.is_none() && rows.is_empty() && (in_head || all_th) {
						head = Some(r);
					} else {
						rows.push(r);
					}
				} else if kind(name) == Kind::Drop {
					skip(lex, name, void);
				}
				// A `tbody` or a `tfoot` groups rows and says nothing else, so it is erased and the
				// rows within it are read where they stand.
			}
		}
	}
	Ok(Block::Table { head, rows, cols })
}

/// Reads a `tr` into the row it is, filling in the alignment its cells declare for their columns.
///
/// Whether every cell was a `th` comes back with the row, because a first row of nothing but `th` is a
/// table naming its columns whether or not anyone wrapped it in a `thead`.
fn row(lex: &mut Lex, cols: &mut Vec<Align>, depth: usize) -> Outcome<(Row, bool)> {
	let mut cells: Vec<Cell> = Vec::new();
	let mut all_th = true;
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(_)		=> {}
			Tok::Close(name)	=> {
				if is(name, "tr") {
					break;
				}
			}
			Tok::Open { name, attrs, void } => {
				let th = is(name, "th");
				if th || is(name, "td") {
					if !th {
						all_th = false;
					}
					let i = cells.len();
					while cols.len() <= i {
						cols.push(Align::None);
					}
					// A column takes its alignment from the first cell that declares one.
					if cols[i] == Align::None {
						cols[i] = align_of(attrs);
					}
					cells.push(Cell(match void {
						true	=> Vec::new(),
						false	=> res!(inlines(lex, name, depth + 1)),
					}));
				} else if kind(name) == Kind::Drop {
					skip(lex, name, void);
				}
			}
		}
	}
	// A row of no cells names no columns, whatever its cells were not.
	let named = all_th && !cells.is_empty();
	Ok((Row(cells), named))
}

/// Reads a `pre` into the run of code it holds.
fn code(lex: &mut Lex, attrs: &str, void: bool) -> Block {
	// A `pre` may name the language on itself, or on the `code` within it, which is where the
	// convention every highlighter reads puts it.
	let mut lang = lang_of(attrs);
	let mut text = String::new();
	if !void {
		while let Some(tok) = lex.next() {
			match tok {
				// Not collapsed, and not trimmed: within a `pre` the whitespace is the content, which
				// is the whole reason the element exists.
				Tok::Text(t)		=> text.push_str(&decode_entities(t)),
				Tok::Close(name)	=> {
					if is(name, "pre") {
						break;
					}
				}
				Tok::Open { name, attrs, void } => {
					if kind(name) == Kind::Drop {
						skip(lex, name, void);
					} else if is(name, "code") && lang.is_none() {
						lang = lang_of(attrs);
					}
					// Every other tag within is decoration -- a highlighter's own markup around a
					// keyword -- and what the block says is the text under it.
				}
			}
		}
	}
	// A line ending directly after the opening tag is the tag's own and not the code's. HTML's own
	// parser drops it, and a reader that kept it would grow a blank first line on every block written
	// the way most are.
	let text = match text.strip_prefix("\r\n") {
		Some(rest)	=> rest.to_string(),
		None		=> match text.strip_prefix('\n') {
			Some(rest)	=> rest.to_string(),
			None		=> text,
		},
	};
	Block::Code { lang, text }
}

/// The text of an element and everything within it, its entities decoded and its whitespace left as it
/// was written.
fn text_in(lex: &mut Lex, until: &str) -> String {
	let mut out = String::new();
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(t)		=> out.push_str(&decode_entities(t)),
			Tok::Close(name)	=> {
				if is(name, until) {
					break;
				}
			}
			Tok::Open { name, void, .. } => {
				if kind(name) == Kind::Drop {
					skip(lex, name, void);
				}
			}
		}
	}
	out
}

/// Skips an element and everything within it, tags and all.
///
/// The skip counts its own element's tags rather than recursing, so a document built to nest deeply
/// costs nothing here.
fn skip(lex: &mut Lex, name: &str, void: bool) {
	if void {
		return;
	}
	let mut depth = 0usize;
	while let Some(tok) = lex.next() {
		match tok {
			Tok::Text(_)	=> {}
			Tok::Open { name: n, void: v, .. } => {
				if !v && is(n, name) {
					depth += 1;
				}
			}
			Tok::Close(n)	=> {
				if is(n, name) {
					if depth == 0 {
						return;
					}
					depth -= 1;
				}
			}
		}
	}
}

// -----------------------------------------------------------------------------------------------
// Whitespace.
// -----------------------------------------------------------------------------------------------

/// A run of inline content, gathered with HTML's whitespace rules applied as it grows.
///
/// This is where the rule the module documentation states is actually kept: a run of whitespace says
/// one space, a space at the start of a run says nothing, and the space at the end is taken off when
/// the run closes. Only a `pre` escapes it, and a `pre` is read by [`code`] and never reaches here.
#[derive(Default)]
struct Run {
	/// The inlines gathered so far.
	out: Vec<Inline>,
}

impl Run {

	/// Whether a space would say nothing here: at the start of the run, where a space has been said
	/// already, or after a break, there is nothing for one to separate.
	fn open(&self) -> bool {
		match self.out.last() {
			None			=> true,
			Some(Inline::Break)	=> true,
			Some(Inline::Text(t))	=> t.ends_with(' '),
			Some(_)			=> false,
		}
	}

	/// Adds a run of text, its entities decoded and its whitespace collapsed.
	fn text(&mut self, raw: &str) {
		// Decoded first and collapsed second, which is the order that matters: a `&#10;` says a
		// newline, and a newline is whitespace like any other. Only a no-break space survives, and it
		// survives because it is not whitespace this collapses.
		let s = collapse(&decode_entities(raw));
		let s = match self.open() {
			true	=> s.strip_prefix(' ').unwrap_or(&s),
			false	=> s.as_str(),
		};
		if s.is_empty() {
			return;
		}
		self.push(Inline::Text(s.to_string()));
	}

	/// Says the space an erased block stands for, where the run does not say one already.
	fn space(&mut self) {
		if !self.open() {
			self.push(Inline::Text(" ".to_string()));
		}
	}

	/// Adds a settled inline, joining it to the run before it where both are text.
	fn push(&mut self, item: Inline) {
		if let Inline::Text(t) = &item {
			if let Some(Inline::Text(last)) = self.out.last_mut() {
				last.push_str(t);
				return;
			}
		}
		self.out.push(item);
	}

	/// The run, with the trailing space that HTML does not say taken off it.
	fn end(mut self) -> Vec<Inline> {
		if let Some(Inline::Text(t)) = self.out.last_mut() {
			while t.ends_with(' ') {
				t.pop();
			}
			if t.is_empty() {
				self.out.pop();
			}
		}
		self.out
	}
}

/// Whether a character is whitespace that HTML collapses.
///
/// A no-break space is deliberately not among them. It is not this whitespace, it does not collapse,
/// and an author who wrote one meant it.
fn is_ws(c: char) -> bool {
	matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Collapses every run of whitespace in a run of text to the single space it says.
fn collapse(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	let mut ws = false;	// Whether whitespace has been passed over since the last character kept.
	for c in s.chars() {
		if is_ws(c) {
			ws = true;
		} else {
			if ws {
				out.push(' ');
				ws = false;
			}
			out.push(c);
		}
	}
	if ws {
		out.push(' ');
	}
	out
}

// -----------------------------------------------------------------------------------------------
// Attributes.
// -----------------------------------------------------------------------------------------------

/// The value of one attribute from a tag's unparsed run of them, where the tag carries it.
///
/// Names are matched without regard to case, and a value is taken from double quotes, single quotes or
/// no quotes at all, because all three are HTML and a generator picks whichever it likes. An attribute
/// written with no value at all is present, and says the empty string. What comes back has its entities
/// decoded, so a destination written `a&amp;b` is read as the `a&b` it names.
fn attr(attrs: &str, want: &str) -> Option<String> {
	let b = attrs.as_bytes();
	let mut i = 0;
	while i < b.len() {
		while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
			i += 1;
		}
		let ns = i;
		while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
			i += 1;
		}
		let name = &attrs[ns..i];
		while i < b.len() && b[i].is_ascii_whitespace() {
			i += 1;
		}
		let mut val = "";
		if i < b.len() && b[i] == b'=' {
			i += 1;
			while i < b.len() && b[i].is_ascii_whitespace() {
				i += 1;
			}
			if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
				let q = b[i];
				i += 1;
				let vs = i;
				while i < b.len() && b[i] != q {
					i += 1;
				}
				val = &attrs[vs..i];
				if i < b.len() {
					i += 1;
				}
			} else {
				let vs = i;
				while i < b.len() && !b[i].is_ascii_whitespace() {
					i += 1;
				}
				val = &attrs[vs..i];
			}
		}
		if !name.is_empty() && name.eq_ignore_ascii_case(want) {
			return Some(decode_entities(val));
		}
		if name.is_empty() && val.is_empty() {
			// Nothing was read, so nothing more will be: this is the end of the run.
			break;
		}
	}
	None
}

/// The language a `class="language-x"` names, where the class names one.
fn lang_of(attrs: &str) -> Option<String> {
	let class = match attr(attrs, "class") {
		Some(c)	=> c,
		None	=> return None,
	};
	for word in class.split_ascii_whitespace() {
		if let Some(lang) = word.strip_prefix("language-") {
			if !lang.is_empty() {
				return Some(lang.to_string());
			}
		}
	}
	None
}

/// The alignment a cell declares, where it declares one this can honour.
///
/// Only the logical keywords are read: `start`, `end`, and `center`, which is unambiguous. CSS's
/// `left` and `right` are deliberately not mapped, for the reason [`Align`] gives at length -- the
/// tree does not know which way its text runs, so it cannot know which side `left` is on. A cell that
/// says `left` is read as a cell that says nothing, and the consumer's own default stands, which is
/// wrong for nobody. Mapping it would be wrong for half the world's prose, and silently.
fn align_of(attrs: &str) -> Align {
	let style = match attr(attrs, "style") {
		Some(s)	=> s,
		None	=> return Align::None,
	};
	// The lowered copy is only used to find the property, and an ASCII lowering does not move a byte,
	// so the index it gives is an index into the original.
	let at = match style.to_ascii_lowercase().find("text-align") {
		Some(i)	=> i + "text-align".len(),
		None	=> return Align::None,
	};
	let val = match style[at..].trim_start().strip_prefix(':') {
		Some(v)	=> v,
		None	=> return Align::None,
	};
	let val = val.split(';').next().unwrap_or("").trim();
	match val.to_ascii_lowercase().as_str() {
		"start"		=> Align::Start,
		"center"	=> Align::Centre,
		"end"		=> Align::End,
		_		=> Align::None,
	}
}

// -----------------------------------------------------------------------------------------------
// The lexer.
// -----------------------------------------------------------------------------------------------

/// One thing the reader takes from the source: a tag, or the text between tags.
enum Tok<'a> {
	/// An opening tag.
	Open {
		/// The element's name, in whatever case it was written.
		name:	&'a str,
		/// The attributes, unparsed: [`attr`] reads one out where something wants it.
		attrs:	&'a str,
		/// Whether the tag holds nothing, either by being a void element or by closing itself.
		void:	bool,
	},
	/// A closing tag, by its name.
	Close(&'a str),
	/// The text between two tags, its entities undecoded and its whitespace uncollapsed.
	Text(&'a str),
}

/// The source, and how far into it the reader has come.
struct Lex<'a> {
	/// The HTML being read.
	src:	&'a str,
	/// Where the next token begins.
	i:	usize,
	/// The raw text element being read, where one is.
	raw:	Option<&'a str>,
}

impl<'a> Lex<'a> {

	/// The next token, or nothing where the source is spent.
	///
	/// Comments, doctypes and processing instructions are passed over here rather than being handed on,
	/// because there is nothing above this that would do anything with them but drop them.
	fn next(&mut self) -> Option<Tok<'a>> {
		let b = self.src.as_bytes();
		loop {
			if self.i >= b.len() {
				return None;
			}
			// Within a script or a stylesheet everything up to the closing tag is text, so a `<` in
			// the code opens nothing. The content is dropped above, but it must be read as what it is
			// or a comparison in a script would be read as an element.
			if let Some(name) = self.raw.take() {
				let end = self.raw_end(name);
				let t = &self.src[self.i..end];
				self.i = end;
				return Some(Tok::Text(t));
			}
			if b[self.i] == b'<' && opens_tag(b, self.i) {
				if self.src[self.i..].starts_with("<!--") {
					// A comment says nothing to a document tree.
					self.i = match self.src[self.i..].find("-->") {
						Some(k)	=> self.i + k + 3,
						None	=> b.len(),
					};
					continue;
				}
				if b[self.i + 1] == b'!' || b[self.i + 1] == b'?' {
					// A doctype or a processing instruction says nothing either.
					self.i = self.to_gt(self.i + 1);
					continue;
				}
				if b[self.i + 1] == b'/' {
					let ns = self.i + 2;
					let ne = name_end(b, ns);
					let name = &self.src[ns..ne];
					self.i = self.to_gt(ne);
					return Some(Tok::Close(name));
				}
				let ns = self.i + 1;
				let ne = name_end(b, ns);
				let name = &self.src[ns..ne];
				let (attrs, end) = self.attrs_of(ne);
				self.i = end;
				// A trailing slash closes the tag itself; a void element closes itself whether or not
				// anyone wrote one.
				let slash = attrs.ends_with('/');
				let attrs = match slash {
					true	=> &attrs[..attrs.len() - 1],
					false	=> attrs,
				};
				let void = slash || VOID.iter().any(|v| is(name, v));
				if !void && RAW.iter().any(|r| is(name, r)) {
					self.raw = Some(name);
				}
				return Some(Tok::Open { name, attrs, void });
			}
			// Text, up to the next tag. A `<` that opens nothing -- a less-than in prose that nobody
			// escaped -- is text like any other, so the scan steps over it and carries on.
			let mut j = self.i;
			let end = loop {
				match self.src[j..].find('<') {
					None		=> break b.len(),
					Some(k)	=> {
						let at = j + k;
						if opens_tag(b, at) {
							break at;
						}
						j = at + 1;
					}
				}
			};
			let t = &self.src[self.i..end];
			self.i = end;
			return Some(Tok::Text(t));
		}
	}

	/// Where the current raw text element's content ends: at its closing tag, or at the end of the
	/// source where it has none.
	fn raw_end(&self, name: &str) -> usize {
		let mut j = self.i;
		loop {
			match self.src[j..].find('<') {
				None		=> return self.src.len(),
				Some(k)	=> {
					let at = j + k;
					let rest = &self.src[at..];
					if rest.starts_with("</") && rest[2..].to_ascii_lowercase().starts_with(name) {
						return at;
					}
					j = at + 1;
				}
			}
		}
	}

	/// The run of attributes a tag carries, and where the tag ends.
	fn attrs_of(&self, from: usize) -> (&'a str, usize) {
		let end = self.to_gt(from);
		// `to_gt` steps past the `>`, which is not the tag's to give away.
		let close = match end > from && self.src.as_bytes()[end - 1] == b'>' {
			true	=> end - 1,
			false	=> end,
		};
		(self.src[from..close].trim(), end)
	}

	/// Where the tag that is being read ends: just past its `>`, or at the end of the source where it
	/// has none. A `>` within a quoted value ends nothing.
	fn to_gt(&self, from: usize) -> usize {
		let b = self.src.as_bytes();
		let mut k = from;
		let mut q = 0u8;	// The quote mark a value is sitting in, or zero for none.
		while k < b.len() {
			let c = b[k];
			if q != 0 {
				if c == q {
					q = 0;
				}
			} else if c == b'"' || c == b'\'' {
				q = c;
			} else if c == b'>' {
				return k + 1;
			}
			k += 1;
		}
		b.len()
	}
}

/// Whether the `<` at the given index opens a tag, rather than being a less-than nobody escaped.
fn opens_tag(b: &[u8], i: usize) -> bool {
	match b.get(i + 1) {
		Some(c)	=> c.is_ascii_alphabetic() || *c == b'!' || *c == b'/' || *c == b'?',
		None	=> false,
	}
}

/// Where the element name beginning at the given index ends.
fn name_end(b: &[u8], from: usize) -> usize {
	let mut k = from;
	while k < b.len() && (b[k].is_ascii_alphanumeric() || b[k] == b'-' || b[k] == b':') {
		k += 1;
	}
	k
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::doc::text_of;

	/// A run of literal text, for the tests that expect one.
	fn t(s: &str) -> Inline {
		Inline::Text(s.to_string())
	}

	/// The text of a block's inlines, for tests that care what a block says and not how.
	fn said(blocks: &[Block]) -> Vec<String> {
		blocks.iter().map(|b| match b {
			Block::Para(c)			=> text_of(c),
			Block::Heading { content, .. }	=> text_of(content),
			Block::Code { text, .. }	=> text.clone(),
			_				=> String::new(),
		}).collect()
	}

	/// What a table's rows say, cell by cell, the header first.
	fn grid(b: &Block) -> Vec<Vec<String>> {
		match b {
			Block::Table { head, rows, .. }	=> {
				let mut out = Vec::new();
				if let Some(head) = head {
					out.push(head.0.iter().map(|c| c.text_of()).collect());
				}
				for row in rows {
					out.push(row.0.iter().map(|c| c.text_of()).collect());
				}
				out
			}
			other				=> panic!("expected a table, got {:?}", other),
		}
	}

	/// The six headings reach the six levels, whatever case they were written in.
	#[test]
	fn test_the_headings_give_their_levels_00() -> Outcome<()> {
		let b = res!(parse("<h1>One</h1><h3>Three</h3><H6>Six</H6>"));
		assert_eq!(b, vec![
			Block::Heading { level: 1, content: vec![t("One")] },
			Block::Heading { level: 3, content: vec![t("Three")] },
			Block::Heading { level: 6, content: vec![t("Six")] },
		]);
		Ok(())
	}

	/// A `p` is a paragraph, and the blank space an exporter laid it out with is not part of it.
	#[test]
	fn test_a_paragraph_is_a_paragraph_01() -> Outcome<()> {
		let b = res!(parse("<p>One.</p>\n\n<p>Two.</p>\n"));
		assert_eq!(b, vec![Block::Para(vec![t("One.")]), Block::Para(vec![t("Two.")])]);
		Ok(())
	}

	/// THE RULE. A run of spaces, tabs and newlines between two words says one space, whatever the
	/// exporter wrote. A reader that kept them would freeze a book at the width it was exported at.
	#[test]
	fn test_a_run_of_whitespace_says_one_space_02() -> Outcome<()> {
		// A newline the exporter's line wrapping put there.
		assert_eq!(said(&res!(parse("<p>One line\nand its continuation.</p>"))),
			vec!["One line and its continuation."]);
		// Indentation, on its own line, as an exporter lays a document out.
		assert_eq!(said(&res!(parse("<p>\n\tOne line\n\tand its continuation.\n</p>"))),
			vec!["One line and its continuation."]);
		// Spaces, tabs and newlines together, in a run of any length.
		assert_eq!(said(&res!(parse("<p>a  \t \n\r\n   b</p>"))), vec!["a b"]);
		// And the break the exporter's wrapping made is never a break the author asked for.
		let b = res!(parse("<p>One line\nand its continuation.</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("One line and its continuation.")])]);
		Ok(())
	}

	/// The whitespace at either end of a block does not survive it.
	#[test]
	fn test_a_block_does_not_keep_the_space_at_its_ends_03() -> Outcome<()> {
		assert_eq!(said(&res!(parse("<p>   padded   </p>"))), vec!["padded"]);
		assert_eq!(said(&res!(parse("<h2>\n  A Heading\n</h2>"))), vec!["A Heading"]);
		// A paragraph of nothing but whitespace says nothing, and is not a paragraph.
		assert_eq!(res!(parse("<p>  \n  </p>")), Vec::<Block>::new());
		assert_eq!(res!(parse("<p></p>")), Vec::<Block>::new());
		Ok(())
	}

	/// The space between two inlines is a space, and the space at the start of a run is not.
	#[test]
	fn test_whitespace_around_an_inline_collapses_04() -> Outcome<()> {
		// A newline between two emphasised words says the space that divides them.
		let b = res!(parse("<p><em>a</em>\n   <em>b</em></p>"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Emph { strong: false, content: vec![t("a")] },
			t(" "),
			Inline::Emph { strong: false, content: vec![t("b")] },
		])]);
		// The whitespace an exporter put before the first inline says nothing.
		let b = res!(parse("<p>\n  <em>a</em> b\n</p>"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Emph { strong: false, content: vec![t("a")] },
			t(" b"),
		])]);
		Ok(())
	}

	/// A `pre` is the exception the rule is written around: its whitespace is what it says.
	#[test]
	fn test_a_pre_keeps_its_whitespace_exactly_05() -> Outcome<()> {
		let b = res!(parse("<pre>fn main() {\n\tlet x = 1;\n}\n</pre>"));
		assert_eq!(b, vec![Block::Code {
			lang:	None,
			text:	"fn main() {\n\tlet x = 1;\n}\n".to_string(),
		}]);
		Ok(())
	}

	/// A `pre` names its language by the class every highlighter reads, on the `pre` or on the `code`.
	#[test]
	fn test_a_code_block_names_its_language_06() -> Outcome<()> {
		let b = res!(parse("<pre><code class=\"language-rust\">let x = 1 &lt; 2;\n</code></pre>"));
		assert_eq!(b, vec![Block::Code {
			lang:	Some("rust".to_string()),
			text:	"let x = 1 < 2;\n".to_string(),
		}]);
		// On the `pre` itself, and beside other classes.
		let b = res!(parse("<pre class=\"highlight language-c\">int x;</pre>"));
		assert_eq!(b, vec![Block::Code { lang: Some("c".to_string()), text: "int x;".to_string() }]);
		// And a block that names none says none.
		let b = res!(parse("<pre><code>plain</code></pre>"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "plain".to_string() }]);
		Ok(())
	}

	/// A line ending directly after the opening tag is the tag's own, and does not become a blank first
	/// line of code.
	#[test]
	fn test_a_pre_drops_the_line_ending_that_opens_it_07() -> Outcome<()> {
		let b = res!(parse("<pre><code>\nfirst\nsecond\n</code></pre>"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "first\nsecond\n".to_string() }]);
		Ok(())
	}

	/// A `blockquote` holds blocks, and nests.
	#[test]
	fn test_a_quotation_holds_blocks_08() -> Outcome<()> {
		let b = res!(parse("<blockquote><p>One.</p><p>Two.</p></blockquote>"));
		assert_eq!(b, vec![Block::Quote(vec![
			Block::Para(vec![t("One.")]),
			Block::Para(vec![t("Two.")]),
		])]);
		let b = res!(parse("<blockquote><blockquote><p>Deep.</p></blockquote></blockquote>"));
		assert_eq!(b, vec![Block::Quote(vec![Block::Quote(vec![Block::Para(vec![t("Deep.")])])])]);
		Ok(())
	}

	/// An `hr` is a thematic break.
	#[test]
	fn test_a_rule_is_a_rule_09() -> Outcome<()> {
		assert_eq!(res!(parse("<p>a</p><hr><p>b</p>")), vec![
			Block::Para(vec![t("a")]),
			Block::Rule,
			Block::Para(vec![t("b")]),
		]);
		Ok(())
	}

	/// A `ul` is an unordered list and an `ol` an ordered one, and an item holds the blocks it holds.
	#[test]
	fn test_a_list_is_ordered_or_not_10() -> Outcome<()> {
		let b = res!(parse("<ul>\n  <li>one</li>\n  <li>two</li>\n</ul>"));
		assert_eq!(b, vec![Block::List {
			ordered:	false,
			items:		vec![
				vec![Block::Para(vec![t("one")])],
				vec![Block::Para(vec![t("two")])],
			],
		}]);
		let b = res!(parse("<ol><li><p>one</p><p>still one</p></li></ol>"));
		assert_eq!(b, vec![Block::List {
			ordered:	true,
			items:		vec![vec![Block::Para(vec![t("one")]), Block::Para(vec![t("still one")])]],
		}]);
		Ok(())
	}

	/// A list nests within an item of a list.
	#[test]
	fn test_a_list_nests_within_a_list_11() -> Outcome<()> {
		let b = res!(parse("<ul><li>one<ul><li>inner</li></ul></li><li>two</li></ul>"));
		let want = vec![Block::List {
			ordered:	false,
			items:		vec![
				vec![
					Block::Para(vec![t("one")]),
					Block::List {
						ordered:	false,
						items:		vec![vec![Block::Para(vec![t("inner")])]],
					},
				],
				vec![Block::Para(vec![t("two")])],
			],
		}];
		assert_eq!(b, want);
		Ok(())
	}

	/// A table takes its header from a `thead`, and its cells reach the grid in order.
	#[test]
	fn test_a_table_names_its_columns_in_a_thead_12() -> Outcome<()> {
		let src = "<table>\n<thead>\n<tr><th>Name</th><th>Age</th></tr>\n</thead>\n\
			<tbody>\n<tr><td>Alice</td><td>30</td></tr>\n<tr><td>Bob</td><td>4</td></tr>\n\
			</tbody>\n</table>";
		let b = res!(parse(src));
		assert_eq!(b.len(), 1);
		assert_eq!(grid(&b[0]), vec![
			vec!["Name", "Age"],
			vec!["Alice", "30"],
			vec!["Bob", "4"],
		]);
		match &b[0] {
			Block::Table { head, rows, cols }	=> {
				assert!(head.is_some(), "the table lost its header row");
				assert_eq!(rows.len(), 2);
				// A column nobody aligned is aligned by nothing.
				assert_eq!(cols, &vec![Align::None, Align::None]);
			}
			other					=> panic!("expected a table, got {:?}", other),
		}
		Ok(())
	}

	/// A first row of nothing but `th` names the columns whether or not anyone wrapped it in a `thead`,
	/// and a table without one has no header at all.
	#[test]
	fn test_a_row_of_th_names_the_columns_13() -> Outcome<()> {
		let b = res!(parse("<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>"));
		assert_eq!(grid(&b[0]), vec![vec!["A", "B"], vec!["1", "2"]]);
		match &b[0] {
			Block::Table { head, rows, .. }	=> {
				assert!(head.is_some(), "a row of th did not name the columns");
				assert_eq!(rows.len(), 1);
			}
			other				=> panic!("expected a table, got {:?}", other),
		}
		// A grid of figures is a table whether or not anything stands at the head of it.
		let b = res!(parse("<table><tr><td>1</td><td>2</td></tr></table>"));
		match &b[0] {
			Block::Table { head, rows, .. }	=> {
				assert!(head.is_none(), "a table with no header grew one");
				assert_eq!(rows.len(), 1);
			}
			other				=> panic!("expected a table, got {:?}", other),
		}
		Ok(())
	}

	/// A cell's alignment is read from the logical keywords only. `left` and `right` are not mapped,
	/// because the tree does not know which side they are on.
	#[test]
	fn test_a_column_aligns_by_logical_side_only_14() -> Outcome<()> {
		let src = "<table><tr>\
			<td style=\"text-align: start\">a</td>\
			<td style=\"text-align: center\">b</td>\
			<td style=\"text-align: end\">c</td>\
			<td style=\"text-align: left\">d</td>\
			</tr></table>";
		let b = res!(parse(src));
		match &b[0] {
			Block::Table { cols, .. }	=> assert_eq!(
				cols,
				&vec![Align::Start, Align::Centre, Align::End, Align::None],
			),
			other				=> panic!("expected a table, got {:?}", other),
		}
		Ok(())
	}

	/// Emphasis is `em` or `i`, strong emphasis is `strong` or `b`, and they nest.
	#[test]
	fn test_emphasis_is_ordinary_or_strong_15() -> Outcome<()> {
		let b = res!(parse("<p><em>a</em> <i>b</i> <strong>c</strong> <b>d</b></p>"));
		assert_eq!(b, vec![Block::Para(vec![
			Inline::Emph { strong: false, content: vec![t("a")] },
			t(" "),
			Inline::Emph { strong: false, content: vec![t("b")] },
			t(" "),
			Inline::Emph { strong: true, content: vec![t("c")] },
			t(" "),
			Inline::Emph { strong: true, content: vec![t("d")] },
		])]);
		let b = res!(parse("<p><strong><em>both</em></strong></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Emph {
			strong:		true,
			content:	vec![Inline::Emph { strong: false, content: vec![t("both")] }],
		}])]);
		Ok(())
	}

	/// An `a` with a destination is a link; one without is an anchor, and its words stay in the line.
	#[test]
	fn test_a_link_carries_its_destination_16() -> Outcome<()> {
		let b = res!(parse("<p>See <a href=\"https://example.com\">here</a> now.</p>"));
		assert_eq!(b, vec![Block::Para(vec![
			t("See "),
			Inline::Link { to: "https://example.com".to_string(), content: vec![t("here")] },
			t(" now."),
		])]);
		// An anchor is not a link: there is nowhere for a reader to go, so only the tag is lost.
		let b = res!(parse("<p>An <a name=\"x\">anchor</a> here.</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("An anchor here.")])]);
		Ok(())
	}

	/// An `img` is an image, by its source and the text that stands for it.
	#[test]
	fn test_an_image_carries_its_source_and_alt_17() -> Outcome<()> {
		let b = res!(parse("<p><img src=\"fig.png\" alt=\"a figure\"></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Image {
			src:	"fig.png".to_string(),
			alt:	"a figure".to_string(),
		}])]);
		// An image that stands for nothing says nothing, and is still an image.
		let b = res!(parse("<p><img src=\"fig.png\"></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Image {
			src:	"fig.png".to_string(),
			alt:	String::new(),
		}])]);
		Ok(())
	}

	/// A `code` within a line is a code span, and a `br` is the one thing that makes a break.
	#[test]
	fn test_a_code_span_and_a_break_18() -> Outcome<()> {
		let b = res!(parse("<p>Call <code>inline()</code> now.</p>"));
		assert_eq!(b, vec![Block::Para(vec![
			t("Call "),
			Inline::Code("inline()".to_string()),
			t(" now."),
		])]);
		let b = res!(parse("<p>one<br>two</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("one"), Inline::Break, t("two")])]);
		Ok(())
	}

	/// A break comes from a `br` and from nothing else. A newline in the source is not one.
	#[test]
	fn test_only_a_br_makes_a_break_19() -> Outcome<()> {
		let b = res!(parse("<p>one\ntwo\n\nthree</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("one two three")])]);
		assert!(!b.iter().any(|blk| match blk {
			Block::Para(c)	=> c.contains(&Inline::Break),
			_		=> false,
		}), "a newline became a break");
		// And the whitespace after a break is the line's, not a word's.
		let b = res!(parse("<p>one<br>\n  two</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("one"), Inline::Break, t("two")])]);
		Ok(())
	}

	/// Entities are decoded, named and numeric alike, and a decoded newline collapses like any other.
	#[test]
	fn test_entities_are_decoded_20() -> Outcome<()> {
		assert_eq!(said(&res!(parse("<p>Tom &amp; Jerry &lt;3</p>"))), vec!["Tom & Jerry <3"]);
		assert_eq!(said(&res!(parse("<p>it&#8217;s</p>"))), vec!["it\u{2019}s"]);
		assert_eq!(said(&res!(parse("<p>it&#x2019;s</p>"))), vec!["it\u{2019}s"]);
		assert_eq!(said(&res!(parse("<p>a&nbsp;b</p>"))), vec!["a b"]);
		// An entity in an attribute is decoded too, so a destination says what it names.
		let b = res!(parse("<p><a href=\"?a=1&amp;b=2\">x</a></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Link {
			to:		"?a=1&b=2".to_string(),
			content:	vec![t("x")],
		}])]);
		// A `&` that begins nothing is an ampersand.
		assert_eq!(said(&res!(parse("<p>a & b</p>"))), vec!["a & b"]);
		Ok(())
	}

	/// A `div` and a `span` are unwrapped: the tag goes and the content stays exactly where it stood.
	#[test]
	fn test_a_div_and_a_span_are_unwrapped_21() -> Outcome<()> {
		// A div holding paragraphs is those paragraphs, at no depth of their own.
		let b = res!(parse("<div><div><p>one</p><p>two</p></div></div>"));
		assert_eq!(b, vec![Block::Para(vec![t("one")]), Block::Para(vec![t("two")])]);
		// A div holding bare words is the paragraph those words are.
		let b = res!(parse("<div><em>Just words.</em></div>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Emph {
			strong:		false,
			content:	vec![t("Just words.")],
		}])]);
		// A span in the middle of a sentence leaves the sentence whole across the hole it left.
		let b = res!(parse("<p>a <span>b</span> c</p>"));
		assert_eq!(b, vec![Block::Para(vec![t("a b c")])]);
		// The same at block level, where there is no paragraph to sit in.
		let b = res!(parse("<div>a <span>b</span> c</div>"));
		assert_eq!(b, vec![Block::Para(vec![t("a b c")])]);
		Ok(())
	}

	/// An element the reader has never heard of is unwrapped, and the prose within it is kept.
	#[test]
	fn test_an_unknown_element_is_unwrapped_22() -> Outcome<()> {
		let b = res!(parse("<html><body><section><p>Kept.</p></section></body></html>"));
		assert_eq!(b, vec![Block::Para(vec![t("Kept.")])]);
		let b = res!(parse("<p>a <mark>b</mark> <custom-tag attr=\"x\">c</custom-tag> d</p>"));
		assert_eq!(said(&b), vec!["a b c d"]);
		// Even one carrying blocks, and one that stands where a block would.
		let b = res!(parse("<figure><figcaption>A caption.</figcaption></figure>"));
		assert_eq!(said(&b), vec!["A caption."]);
		Ok(())
	}

	/// A script, a stylesheet, a head and a comment hold no prose, and go entirely.
	#[test]
	fn test_what_holds_no_prose_is_dropped_23() -> Outcome<()> {
		let src = "<html><head><title>Title</title><meta charset=\"utf-8\"></head>\
			<body><script>if (a < b) { drop(); }</script>\
			<style>p { colour: red; }</style>\
			<!-- a comment, with <p>markup</p> in it -->\
			<p>Kept.</p></body></html>";
		assert_eq!(res!(parse(src)), vec![Block::Para(vec![t("Kept.")])]);
		// A `<` within a script opens nothing, so what follows it is not swallowed.
		let src = "<script>for (i = 0; i < n; i++) { x(); }</script><p>After.</p>";
		assert_eq!(res!(parse(src)), vec![Block::Para(vec![t("After.")])]);
		Ok(())
	}

	/// A void element holds nothing, whether or not anyone closed it, and however it was written.
	#[test]
	fn test_void_elements_close_themselves_24() -> Outcome<()> {
		let b = res!(parse("<p>a<br>b<br/>c<br />d</p>"));
		assert_eq!(b, vec![Block::Para(vec![
			t("a"), Inline::Break, t("b"), Inline::Break, t("c"), Inline::Break, t("d"),
		])]);
		assert_eq!(res!(parse("<hr><hr/>")), vec![Block::Rule, Block::Rule]);
		// A trailing slash is the tag's and not the attribute's.
		let b = res!(parse("<p><img src=\"a.png\"/></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Image {
			src:	"a.png".to_string(),
			alt:	String::new(),
		}])]);
		Ok(())
	}

	/// An attribute's value is read from double quotes, single quotes or none at all, and its name is
	/// read whatever case it was written in.
	#[test]
	fn test_attributes_take_every_quoting_25() -> Outcome<()> {
		let want = vec![Block::Para(vec![Inline::Link {
			to:		"x.html".to_string(),
			content:	vec![t("go")],
		}])];
		assert_eq!(res!(parse("<p><a href=\"x.html\">go</a></p>"), ), want);
		assert_eq!(res!(parse("<p><a href='x.html'>go</a></p>")), want);
		assert_eq!(res!(parse("<p><a href=x.html>go</a></p>")), want);
		assert_eq!(res!(parse("<p><A HREF = \"x.html\" >go</A></p>")), want);
		// A quote mark the other kind does not close a value, and a `>` within one ends no tag.
		let b = res!(parse("<p><a href=\"a'b>c\" title='say \"x\"'>go</a></p>"));
		assert_eq!(b, vec![Block::Para(vec![Inline::Link {
			to:		"a'b>c".to_string(),
			content:	vec![t("go")],
		}])]);
		Ok(())
	}

	/// A close tag that answers nothing closes nothing, and the rest of the document survives it.
	#[test]
	fn test_a_stray_close_tag_loses_nothing_26() -> Outcome<()> {
		let b = res!(parse("<p>one</p></div></em></p><p>two</p>"));
		assert_eq!(said(&b), vec!["one", "two"]);
		let b = res!(parse("</p><h2>A Heading</h2><p>After.</p>"));
		assert_eq!(said(&b), vec!["A Heading", "After."]);
		Ok(())
	}

	/// Nesting past the limit is refused, which is the reader's one refusal.
	#[test]
	fn test_nesting_past_the_limit_is_refused_27() -> Outcome<()> {
		// A quotation for every level the limit allows is read.
		let ok = format!("{}deep{}",
			"<blockquote>".repeat(DEPTH_LIMIT - 1),
			"</blockquote>".repeat(DEPTH_LIMIT - 1));
		assert!(parse(&ok).is_ok());
		// Past it, and past it by far, is not.
		let deep = format!("{}deep", "<blockquote>".repeat(DEPTH_LIMIT + 8));
		assert!(parse(&deep).is_err());
		let very = format!("{}deep", "<blockquote>".repeat(2000));
		assert!(parse(&very).is_err());
		// Inlines are held to the same limit.
		let very = format!("<p>{}deep", "<em>".repeat(2000));
		assert!(parse(&very).is_err());
		Ok(())
	}

	/// An element the tree has no node for costs no stack, so a document built to exhaust one is read
	/// as the flat prose it says rather than refused. This is why the limit can be as low as it is.
	#[test]
	fn test_unwrapped_elements_cost_no_depth_28() -> Outcome<()> {
		let deep = format!("{}<p>Kept.</p>{}", "<div>".repeat(50_000), "</div>".repeat(50_000));
		assert_eq!(res!(parse(&deep)), vec![Block::Para(vec![t("Kept.")])]);
		Ok(())
	}

	/// An empty document is a document with nothing in it, and not a failure.
	#[test]
	fn test_an_empty_document_holds_nothing_29() -> Outcome<()> {
		assert_eq!(res!(parse("")), Vec::<Block>::new());
		assert_eq!(res!(parse("   \n  \t ")), Vec::<Block>::new());
		assert_eq!(res!(parse("<!DOCTYPE html>\n<html>\n<body>\n</body>\n</html>\n")),
			Vec::<Block>::new());
		Ok(())
	}

	/// A less-than that nobody escaped is a less-than, and does not open an element.
	#[test]
	fn test_a_bare_less_than_is_text_30() -> Outcome<()> {
		assert_eq!(said(&res!(parse("<p>a < b and c > d</p>"))), vec!["a < b and c > d"]);
		assert_eq!(said(&res!(parse("<p>1 <2</p><p>after</p>"))), vec!["1 <2", "after"]);
		Ok(())
	}

	/// A cell is given inlines and nothing else, so a block within one is unwrapped -- and stands as
	/// the boundary it is, rather than running two words together.
	#[test]
	fn test_a_block_within_a_cell_is_unwrapped_31() -> Outcome<()> {
		let b = res!(parse("<table><tr><td><p>one</p><p>two</p></td></tr></table>"));
		assert_eq!(grid(&b[0]), vec![vec!["one two"]]);
		Ok(())
	}

	/// A tree written out by the sibling writer and read back is the tree that went in.
	///
	/// This is worth more than it looks. The reader and the writer were written apart and agree on
	/// nothing but the tree between them, so a round trip that holds is two implementations checking
	/// each other rather than one checking itself. It is also the claim the tree's own documentation
	/// makes -- that a second front-end produces the same tree -- put to a test rather than asserted.
	#[test]
	fn test_a_tree_survives_a_round_trip_through_the_writer_32() -> Outcome<()> {
		use crate::doc::{Doc, html::render, markdown};

		let src = "\
			# A Heading\n\
			\n\
			A paragraph with *emphasis*, **strong emphasis**, a [link](https://example.com), \
			`a code span`, and an ![image](fig.png).\n\
			\n\
			A line that ends hard  \nand carries on.\n\
			\n\
			> A quotation.\n\
			>\n\
			> - with a list\n\
			> - of two items\n\
			\n\
			1. An ordered item\n\
			2. Another, holding\n\
			   - a nested list\n\
			\n\
			```rust\n\
			let x = 1 < 2;\n\
			```\n\
			\n\
			| Name  | Age |\n\
			| :---- | --: |\n\
			| Alice | 30  |\n\
			\n\
			---\n";
		let doc = res!(markdown::parse(src));
		// The source is worth having only if it exercises the tree, so check that it did.
		assert!(doc.blocks.len() >= 8, "the round trip is not testing much: {:?}", doc);
		let out = render(&doc);
		let back = Doc { blocks: res!(parse(&out)) };
		assert_eq!(back, doc, "\n--- html ---\n{}\n", out);
		Ok(())
	}

	/// A `pre` that never closes runs to the end, and an element left open runs to the end of what
	/// encloses it. Neither is a failure, and neither loses the prose.
	#[test]
	fn test_an_unclosed_element_runs_to_the_end_33() -> Outcome<()> {
		let b = res!(parse("<pre>code and more"));
		assert_eq!(b, vec![Block::Code { lang: None, text: "code and more".to_string() }]);
		let b = res!(parse("<blockquote><p>inside"));
		assert_eq!(b, vec![Block::Quote(vec![Block::Para(vec![t("inside")])])]);
		Ok(())
	}
}
