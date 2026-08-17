//! Writing the document tree back out as Markdown.
//!
//! The counterpart to [`parse`](crate::doc::markdown::parse), and the output a *model* wants. HTML is
//! for a browser and this is for a reader that thinks in prose: a language model handed the text of a
//! Word document reads `## The second heading` and knows what it is, where it would have to be told
//! what `<h2>` means, and pays for the telling in every request.
//!
//! # What it is faithful to
//!
//! The tree, not the source. Markdown read and written back is not the bytes it was -- `_emphasis_`
//! comes back as `*emphasis*`, a setext heading comes back as an ATX one, and the amount of
//! whitespace is this writer's own. What survives is what the tree carries, which is what a document
//! *says*. Anything that needs the original bytes back wants [`crate::xml`] and its spans, not this.
//!
//! # Escaping is deliberately light
//!
//! Enough that a round trip holds: the characters that would start a construct where they stand, and
//! no others. Escaping every asterisk in a document would make prose unreadable to the reader this
//! exists for, which defeats the point of choosing Markdown over HTML.

use oxedyne_fe2o3_core::prelude::*;

use crate::doc::{
	Align,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};

/// Renders a document as Markdown.
pub fn render(doc: &Doc) -> String {
	let mut out = String::new();
	blocks(&mut out, &doc.blocks);
	// One trailing newline, however the last block ended.
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
/// With one exception: a list straight after a paragraph gets no blank line, which is what makes a
/// list item holding a nested list read as one item rather than as two lists with a gap.
fn blocks(out: &mut String, blocks: &[Block]) {
	for (i, block) in blocks.iter().enumerate() {
		let tight = matches!(
			(blocks.get(i.wrapping_sub(1)), block),
			(Some(Block::Para(_)), Block::List { .. }),
		);
		if i > 0 && !tight {
			out.push('\n');
		}
		one(out, block);
	}
}

/// Writes one block.
fn one(out: &mut String, block: &Block) {
	match block {
		Block::Heading { level, content }	=> {
			for _ in 0..(*level).clamp(1, 6) {
				out.push('#');
			}
			out.push(' ');
			inlines(out, content, false);
			out.push('\n');
		}
		Block::Para(content)			=> {
			inlines(out, content, true);
			out.push('\n');
		}
		Block::Code { lang, text }		=> {
			// A fence longer than any run of backticks inside, or a listing about Markdown closes
			// its own fence three characters in.
			let n = longest_run(text, '`').max(2) + 1;
			let fence: String = "`".repeat(n);
			out.push_str(&fence);
			if let Some(lang) = lang {
				out.push_str(lang);
			}
			out.push('\n');
			out.push_str(text);
			if !text.ends_with('\n') {
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
				let marker = match ordered {
					true	=> fmt!("{}. ", n + 1),
					false	=> "- ".to_string(),
				};
				// An item's content is written as though it stood alone, and the indent it needs is
				// added here. Nesting is therefore the sum of the markers above it, which is what
				// lines a nested list up under its parent's text rather than under its bullet -- and
				// what an item does NOT need is a second helping of the depth it is already inside.
				let mut body = String::new();
				blocks(&mut body, item);
				let pad = " ".repeat(marker.chars().count());
				for (k, line) in body.lines().enumerate() {
					match (k, line.is_empty()) {
						(_, true)	=> out.push('\n'),
						(0, false)	=> {
							out.push_str(&marker);
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
		Block::Rule				=> out.push_str("---\n"),
		Block::Table { head, rows, cols }	=> table(out, head, rows, cols),
		// A division names a region and Markdown has no syntax for one. Its content stands where it
		// stood, which is what the HTML writer does with an attribute-less division too.
		Block::Div { content, .. }		=> blocks(out, content),
	}
}

/// Writes a table as a pipe table.
fn table(out: &mut String, head: &Option<Row>, rows: &[Row], cols: &[Align]) {
	let n = head.iter().chain(rows).map(|r| r.0.len()).max().unwrap_or(0).max(cols.len());
	if n == 0 {
		return;
	}
	// A pipe table has to have a header. A table that carried none gets an empty one, because the
	// alternative is a body that reads as prose.
	let empty = Row::default();
	let head = head.as_ref().unwrap_or(&empty);
	row(out, head, n);
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
		let mut cell = String::new();
		// A cell is never the start of a line, whatever it looks like: `3.40` in a cell opens no
		// ordered list, and escaping it there would put a backslash in front of every price in the
		// document.
		inlines(&mut cell, &r.0.get(i).unwrap_or(&empty).0, false);
		// A pipe inside a cell would end it.
		out.push_str(&cell.replace('|', "\\|"));
		out.push(' ');
		out.push('|');
	}
	out.push('\n');
}

/// Writes a run of inline content.
///
/// `start` says whether what follows begins a line, which is the whole of what decides an escape: a
/// `-` or a `1.` opens a block where a line begins and is punctuation everywhere else. A writer that
/// did not track it would escape every hyphen in the document, or none of the ones that matter.
fn inlines(out: &mut String, content: &[Inline], start: bool) {
	let mut start = start;
	for item in content {
		match item {
			Inline::Text(text)			=> {
				out.push_str(&escape(text, start));
				start = text.ends_with('\n');
			}
			Inline::Emph { strong, content }	=> {
				let mark = match strong {
					true	=> "**",
					false	=> "*",
				};
				out.push_str(mark);
				inlines(out, content, false);
				out.push_str(mark);
				start = false;
			}
			Inline::Link { to, content }		=> {
				out.push('[');
				inlines(out, content, false);
				out.push_str("](");
				out.push_str(to);
				out.push(')');
				start = false;
			}
			Inline::Image { src, alt }		=> {
				out.push_str("![");
				out.push_str(&escape(alt, false));
				out.push_str("](");
				out.push_str(src);
				out.push(')');
				start = false;
			}
			Inline::Code(code)			=> {
				let n = longest_run(code, '`') + 1;
				let fence: String = "`".repeat(n);
				out.push_str(&fence);
				// A span that begins or ends with a backtick needs a space, which the reader eats.
				if code.starts_with('`') || code.ends_with('`') {
					out.push(' ');
					out.push_str(code);
					out.push(' ');
				} else {
					out.push_str(code);
				}
				out.push_str(&fence);
				start = false;
			}
			// A span names a region and Markdown has no syntax for one.
			Inline::Span { content, .. }		=> {
				inlines(out, content, start);
				start = false;
			}
			// Two spaces then a newline: the only hard break Markdown has that survives a reader that
			// treats a lone newline as a space, which is what this crate's own reader does.
			Inline::Break				=> {
				out.push_str("  \n");
				start = true;
			}
		}
	}
}

/// The text with the characters that would start a construct where they stand escaped.
///
/// Light on purpose -- see the module's own note. A `*` between two letters starts nothing and is left
/// alone; one that could open emphasis is escaped.
fn escape(text: &str, start: bool) -> String {
	let mut out = String::with_capacity(text.len());
	let b = text.as_bytes();
	for (i, c) in text.char_indices() {
		let at_start = match i {
			0	=> start,
			_	=> out.ends_with('\n'),
		};
		match c {
			'\\' | '`' | '*' | '_' | '[' | ']'	=> {
				out.push('\\');
				out.push(c);
			}
			// These open a block only at the start of a line.
			'#' | '>' | '-' | '+' if at_start	=> {
				out.push('\\');
				out.push(c);
			}
			// A digit followed by a full stop opens an ordered list, at the start of a line.
			'.' if start && at_start_number(b, i)	=> out.push_str("\\."),
			_					=> out.push(c),
		}
	}
	out
}

/// Whether a full stop at this offset closes a run of digits that begins its line, which is what would
/// open an ordered list.
fn at_start_number(b: &[u8], i: usize) -> bool {
	let mut k = i;
	while k > 0 && b[k - 1].is_ascii_digit() {
		k -= 1;
	}
	k < i && (k == 0 || b[k - 1] == b'\n')
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
