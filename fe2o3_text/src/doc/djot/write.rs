//! Writing the document tree back out as Djot.
//!
//! The counterpart to [`parse`](crate::doc::djot::parse), and what an editor is handed: a stored
//! section opens as Djot, the author changes it, and it reads back to a tree. The writer is faithful to
//! the tree, not to the bytes it was first read from, and holds one invariant: for a tree `d` in the
//! vocabulary the [module](crate::doc::djot) maps, `parse(&render(&d)) == d`.
//!
//! # Escaping is light, but total where it must be
//!
//! A character is escaped where it would otherwise read as markup, and nowhere else that a reader
//! would notice. The emphasis marks, brackets, braces, backticks and the backslash itself are escaped
//! wherever they fall in text, since any of them may pair with a mark further along the line; the
//! characters that only open a block -- `#`, `>`, `-`, `+`, `:`, `|`, and the full stop or parenthesis
//! after a leading number -- are escaped only where they begin a line.
//!
//! # Delimiters are forced only where they must be
//!
//! Emphasis, superscript and subscript are written plain, `_so_`, unless plain would not read back:
//! where the content begins or ends in whitespace, or where the same mark is already open around it.
//! Then the braced form Djot keeps for the purpose is written, `{_ so _}`.

use crate::doc::{
	Align,
	Attrs,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};

use oxedyne_fe2o3_core::prelude::*;

/// Renders a document as Djot.
pub fn render(doc: &Doc) -> String {
	let mut out = String::new();
	blocks(&mut out, &doc.blocks);
	while out.ends_with("\n\n") {
		out.pop();
	}
	if !out.is_empty() && !out.ends_with('\n') {
		out.push('\n');
	}
	out
}

/// Writes a run of blocks, a blank line between each.
///
/// Two lists of one kind side by side would read as one, so the second takes the other marker of its
/// kind, which is how Djot tells two lists apart.
fn blocks(out: &mut String, bs: &[Block]) {
	let mut prev: Option<(bool, bool)> = None;	// (ordered, alt marker) of a list just written
	for (i, b) in bs.iter().enumerate() {
		if i > 0 {
			out.push('\n');
		}
		let alt = match (prev, b) {
			(Some((o, a)), Block::List { ordered, .. }) if o == *ordered	=> !a,
			_								=> false,
		};
		one(out, b, alt);
		prev = match b {
			Block::List { ordered, .. }	=> Some((*ordered, alt)),
			_				=> None,
		};
	}
}

/// Writes one block, a list in its alternative marker where `alt` says so.
fn one(out: &mut String, block: &Block, alt: bool) {
	match block {
		Block::Heading { level, content }	=> {
			for _ in 0..(*level).clamp(1, 6) {
				out.push('#');
			}
			if !content.is_empty() {
				out.push(' ');
				inlines(out, content, false, 0, false);
			}
			out.push('\n');
		}
		Block::Para(content)			=> {
			inlines(out, content, true, 0, false);
			out.push('\n');
		}
		Block::Code { lang, text }		=> {
			// A fence longer than any run of backticks inside, so the listing cannot close it.
			let fence = "`".repeat((longest_run(text, '`') + 1).max(3));
			out.push_str(&fence);
			if let Some(lang) = lang {
				out.push_str(lang);
			}
			out.push('\n');
			out.push_str(text);
			if !text.is_empty() && !text.ends_with('\n') {
				out.push('\n');
			}
			out.push_str(&fence);
			out.push('\n');
		}
		Block::Quote(inner)			=> {
			let mut body = String::new();
			blocks(&mut body, inner);
			for line in body.lines() {
				match line.is_empty() {
					true	=> out.push_str(">\n"),
					false	=> {
						out.push_str("> ");
						out.push_str(line);
						out.push('\n');
					}
				}
			}
		}
		Block::List { ordered, items }	=> {
			for (n, item) in items.iter().enumerate() {
				let marker = match (ordered, alt) {
					(true,	false)	=> fmt!("{}.", n + 1),
					(true,	true)	=> fmt!("{})", n + 1),
					(false,	false)	=> "-".to_string(),
					(false,	true)	=> "+".to_string(),
				};
				// The item is written as though it stood alone, and indented here by the width of
				// its marker, so nesting is the sum of the markers above it.
				let mut body = String::new();
				blocks(&mut body, item);
				let pad = " ".repeat(marker.chars().count() + 1);
				if body.is_empty() {
					out.push_str(&marker);
					out.push('\n');
				}
				// Markers nested on one line, `- - -`, would read as a break, so such an item's body
				// starts on the line after its marker.
				let first = body.lines().next().unwrap_or("");
				let split = is_rule(&fmt!("{} {}", marker, first));
				if split {
					out.push_str(&marker);
					out.push('\n');
				}
				for (k, line) in body.lines().enumerate() {
					match (k, line.is_empty()) {
						(_, true)	=> out.push('\n'),
						(0, false) if !split => {
							out.push_str(&marker);
							out.push(' ');
							out.push_str(line);
							out.push('\n');
						}
						(_, false)	=> {
							out.push_str(&pad);
							out.push_str(line);
							out.push('\n');
						}
					}
				}
			}
		}
		// Not `---`, which after a `-` marker reads as a break rather than an item holding one.
		Block::Rule				=> out.push_str("***\n"),
		Block::Table { head, rows, cols, caption } => {
			table(out, head, rows, cols);
			if !caption.is_empty() {
				out.push_str("\n^ ");
				inlines(out, caption, false, 0, false);
				out.push('\n');
			}
		}
		Block::Div { attrs: a, content }	=> {
			let fence = ":::";
			out.push_str(fence);
			out.push(' ');
			match a.id.is_none() && a.pairs.is_empty() && a.classes.len() == 1 && bare(&a.classes[0]) {
				true	=> out.push_str(&a.classes[0]),
				false	=> attrs(out, a),
			}
			out.push('\n');
			blocks(out, content);
			out.push_str(fence);
			out.push('\n');
		}
		// The page constructs are written in F2b; until then they degrade as `Block::as_plain` says.
		Block::Figure { .. } | Block::Terms { .. } => blocks(out, &block.as_plain()),
	}
}

/// Does the class name stand alone after a division's colons, as `::: note` does?
fn bare(class: &str) -> bool {
	!class.is_empty()
		&& !class.starts_with('.')
		&& !class.starts_with('{')
		&& !class.chars().any(|c| c.is_whitespace())
}

/// Writes a brace group: the id, the classes, then the pairs, each value quoted.
fn attrs(out: &mut String, a: &Attrs) {
	out.push('{');
	let mut first = true;
	let mut sep = |out: &mut String| {
		if !first {
			out.push(' ');
		}
		first = false;
	};
	if let Some(id) = &a.id {
		sep(out);
		out.push('#');
		out.push_str(id);
	}
	for c in &a.classes {
		sep(out);
		out.push('.');
		out.push_str(c);
	}
	for (k, v) in &a.pairs {
		sep(out);
		out.push_str(k);
		out.push_str("=\"");
		out.push_str(v);
		out.push('"');
	}
	out.push('}');
}

/// Writes a pipe table: the header row where there is one, the delimiter row, then the body.
///
/// A table with no header starts at its delimiter row, which Djot reads as aligning the columns and
/// heading none.
fn table(out: &mut String, head: &Option<Row>, rows: &[Row], cols: &[Align]) {
	let n = head.iter().chain(rows).map(|r| r.0.len()).max().unwrap_or(0).max(cols.len());
	if n == 0 {
		return;
	}
	if let Some(h) = head {
		row(out, h, n);
	}
	out.push('|');
	for i in 0..n {
		let bar = match cols.get(i).copied().unwrap_or(Align::None) {
			Align::None	=> " --- ",
			Align::Start	=> " :-- ",
			Align::Centre	=> " :-: ",
			Align::End	=> " --: ",
		};
		out.push_str(bar);
		out.push('|');
	}
	out.push('\n');
	for r in rows {
		row(out, r, n);
	}
}

/// Writes one row of a table, padded to the width of the widest.
fn row(out: &mut String, r: &Row, n: usize) {
	let empty = Cell::default();
	out.push('|');
	for i in 0..n {
		out.push(' ');
		inlines(out, &r.0.get(i).unwrap_or(&empty).content, false, 0, true);
		out.push_str(" |");
	}
	out.push('\n');
}

// Marks open around the inline being written.
const EMPH:	u8 = 1;
const STRONG:	u8 = 2;
const SUP:	u8 = 4;
const SUB:	u8 = 8;

/// Writes a run of inline content.
///
/// `start` says whether what follows begins a line, which decides the block escapes; `open` holds the
/// marks already open around the run, which decides whether a mark must be braced; and `cell` says the
/// run is a table cell, where a pipe would end it.
fn inlines(out: &mut String, content: &[Inline], start: bool, open: u8, cell: bool) {
	let mut start = start;
	let mut bare_img: Option<usize> = None;	// where an image with no attrs ended
	for item in content {
		let after_img = bare_img.take();
		match item {
			Inline::Text(text)			=> escape(out, text, start, cell),
			Inline::Emph { strong, content }	=> match strong {
				true	=> mark(out, '*', STRONG, content, open, cell),
				false	=> mark(out, '_', EMPH, content, open, cell),
			},
			Inline::Sup(content)			=> mark(out, '^', SUP, content, open, cell),
			Inline::Sub(content)			=> mark(out, '~', SUB, content, open, cell),
			Inline::Link { to, content }		=> {
				// A bracket is read as a run of its own, so no mark is open inside it.
				out.push('[');
				inlines(out, content, false, 0, cell);
				out.push_str("](");
				dest(out, to, cell);
				out.push(')');
			}
			Inline::Image { src, alt, attrs: a }	=> {
				out.push_str("![");
				escape(out, alt, false, cell);
				out.push_str("](");
				dest(out, src, cell);
				out.push(')');
				if !a.is_empty() {
					attrs(out, a);
				}
				if a.is_empty() {
					bare_img = Some(out.len());
				}
			}
			Inline::Code(code)			=> {
				let fence = "`".repeat(longest_run(code, '`') + 1);
				// The reader takes one space off each end where both ends have one, so a span that
				// would lose a space, or that begins or ends with a backtick, is padded.
				let pad = code.starts_with('`') || code.ends_with('`')
					|| (code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty());
				out.push_str(&fence);
				if pad {
					out.push(' ');
				}
				out.push_str(code);
				if pad {
					out.push(' ');
				}
				out.push_str(&fence);
			}
			Inline::Span { attrs: a, content }	=> {
				out.push('[');
				inlines(out, content, false, 0, cell);
				out.push(']');
				attrs(out, a);
			}
			Inline::Break				=> out.push_str("\\\n"),
		}
		start = matches!(item, Inline::Break);
		// An image takes a brace group against it as its attrs, so one that a forced mark follows is
		// given its empty group.
		if let Some(at) = after_img {
			if out[at..].starts_with('{') {
				out.insert_str(at, "{}");
			}
		}
	}
}

/// Writes a pair of marks around their content, braced where plain marks would not read back.
fn mark(out: &mut String, ch: char, bit: u8, content: &[Inline], open: u8, cell: bool) {
	let mut inner = String::new();
	inlines(&mut inner, content, false, open | bit, cell);
	let braced = open & bit != 0
		|| inner.is_empty()
		|| inner.starts_with(char::is_whitespace)
		|| inner.ends_with(char::is_whitespace);
	if braced {
		out.push('{');
	}
	out.push(ch);
	out.push_str(&inner);
	out.push(ch);
	if braced {
		out.push('}');
	}
}

/// Writes a link or image destination: plain, or in angle brackets where it holds whitespace.
///
/// Every character the inline pass could take for markup is escaped, including the brackets and
/// backticks, since a destination inside a bracket is scanned by the bracket's own search for its end.
fn dest(out: &mut String, to: &str, cell: bool) {
	let angled = to.chars().any(char::is_whitespace);
	if angled {
		out.push('<');
	}
	for c in to.chars() {
		let esc = match c {
			'\\' | '(' | ')' | '<' | '>' | '[' | ']' | '`'	=> true,
			'|'						=> cell,
			_						=> false,
		};
		if esc {
			out.push('\\');
		}
		out.push(c);
	}
	if angled {
		out.push('>');
	}
}

/// Writes text with the characters that would read as markup escaped.
fn escape(out: &mut String, text: &str, start: bool, cell: bool) {
	// A number opening a line is a list marker when a full stop or parenthesis follows it.
	let digits = text.bytes().take_while(|b| b.is_ascii_digit()).count();
	let last = text.char_indices().last().map(|(i, _)| i);
	for (i, c) in text.char_indices() {
		let esc = match c {
			'\\' | '`' | '*' | '_' | '^' | '~' | '[' | ']' | '{' | '}' | '<'	=> true,
			// Before a bracket it would make an image of a link.
			'!'							=> Some(i) == last,
			'|'							=> cell || (start && i == 0),
			'#' | '>' | '+' | ':' | '-' if i == 0				=> start || (cell && matches!(c, '-' | ':')),
			'.' | ')' if start && digits > 0 && i == digits			=> true,
			_								=> false,
		};
		if esc {
			out.push('\\');
		}
		out.push(c);
	}
}

/// Would the line read as a thematic break: three or more of one of `-`, `*` or `_`, and spaces?
fn is_rule(line: &str) -> bool {
	let t = line.trim();
	let ch = match t.chars().next() {
		Some(c @ ('-' | '*' | '_'))	=> c,
		_				=> return false,
	};
	t.chars().all(|c| c == ch || c == ' ' || c == '\t') && t.chars().filter(|c| *c == ch).count() >= 3
}

/// The longest unbroken run of a character in a string.
fn longest_run(s: &str, c: char) -> usize {
	let mut best = 0;
	let mut run = 0;
	for k in s.chars() {
		match k == c {
			true	=> {
				run += 1;
				best = best.max(run);
			}
			false	=> run = 0,
		}
	}
	best
}
