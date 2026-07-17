//! Rendering a document tree to HTML.
//!
//! The counterpart to a front-end: where [`markdown`](super::markdown) reads a syntax into the tree,
//! this walks the tree out to the form a browser reads. It is the first of the outputs the tree's own
//! documentation names, and it makes the tree's neutrality testable rather than merely asserted --
//! `Rule`, an image by its path and a code span within a line all reach HTML intact, and each is
//! something the signed-document consumer gives up.
//!
//! # A fragment, not a page
//!
//! What comes back is the document's blocks and nothing around them: no `<html>`, no `<head>`, no
//! stylesheet. A caller owns the page and drops this into it, because the furniture around a document
//! belongs to the site rather than to the prose.
//!
//! # Escaping
//!
//! Every run of text and every attribute value is escaped on the way out, so a document that says
//! `<script>` says it rather than does it. That is the whole of the safety here: **a link's
//! destination is written out as given**, and a tree carrying a `javascript:` destination will render
//! one. That is right for prose whose author is trusted, which is what this renders today. Untrusted
//! prose -- a comment, say -- wants a sanitiser between the front-end and here, which is a separate
//! thing and does not exist yet.
//!
//! # Depth
//!
//! The walk is recursive, on the same terms as [`text_of`](super::text_of): a tree's depth is bounded
//! by whatever built it, and the Markdown reader bounds its own. A tree assembled by hand deeper than
//! a stack can walk will overflow this, exactly as it would overflow `text_of` or `Drop`.

use crate::doc::{
	Align,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
};


/// Renders a document to an HTML fragment.
pub fn render(doc: &Doc) -> String {
	let mut out = String::new();
	blocks(&mut out, &doc.blocks);
	out
}

/// Writes an opening tag.
///
/// Tags are pushed rather than formatted because this crate's own [`fmt`](crate::fmt) module shadows
/// the `fmt!` macro, and because a tag is a handful of bytes that needs no allocation to say.
fn open(out: &mut String, tag: &str) {
	out.push('<');
	out.push_str(tag);
	out.push('>');
}

/// Writes a closing tag.
fn close(out: &mut String, tag: &str) {
	out.push_str("</");
	out.push_str(tag);
	out.push('>');
}

/// Renders a run of blocks, each on its own line.
fn blocks(out: &mut String, items: &[Block]) {
	for item in items {
		block(out, item);
		out.push('\n');
	}
}

/// Renders one block.
fn block(out: &mut String, item: &Block) {
	match item {
		Block::Heading { level, content } => {
			// The tree says 1 to 6 and the Markdown reader gives no other, but the type admits one, and
			// a heading is not worth a panic. Anything outside the range renders at the nearest end.
			let tag = match (*level).clamp(1, 6) {
				1	=> "h1",
				2	=> "h2",
				3	=> "h3",
				4	=> "h4",
				5	=> "h5",
				_	=> "h6",
			};
			open(out, tag);
			inlines(out, content);
			close(out, tag);
		}
		Block::Para(content) => {
			open(out, "p");
			inlines(out, content);
			close(out, "p");
		}
		Block::List { ordered, items } => {
			let tag = if *ordered { "ol" } else { "ul" };
			open(out, tag);
			out.push('\n');
			for item in items {
				out.push_str("<li>");
				// The tree carries no tight/loose distinction -- that is presentation, and the tree holds
				// meaning -- so it is inferred here: an item that is one paragraph and nothing else is
				// what a reader wrote as a plain item, and wrapping it in a `<p>` would space a shopping
				// list out like an essay. An item holding anything more is rendered as the blocks it is.
				match item.as_slice() {
					[Block::Para(content)]	=> inlines(out, content),
					_			=> {
						out.push('\n');
						blocks(out, item);
					}
				}
				out.push_str("</li>\n");
			}
			close(out, tag);
		}
		Block::Code { lang, text } => {
			match lang {
				Some(lang) => {
					// `language-x` is the convention every highlighter reads, and costs nothing to emit.
					out.push_str("<pre><code class=\"language-");
					escape_attr(out, lang);
					out.push_str("\">");
				}
				None => out.push_str("<pre><code>"),
			}
			escape_text(out, text);
			out.push_str("</code></pre>");
		}
		Block::Quote(inner) => {
			out.push_str("<blockquote>\n");
			blocks(out, inner);
			out.push_str("</blockquote>");
		}
		Block::Table { head, rows, cols } => {
			out.push_str("<table>\n");
			if let Some(head) = head {
				out.push_str("<thead>\n");
				row(out, head, cols, "th");
				out.push_str("</thead>\n");
			}
			out.push_str("<tbody>\n");
			for r in rows {
				row(out, r, cols, "td");
			}
			out.push_str("</tbody>\n</table>");
		}
		Block::Rule => out.push_str("<hr>"),
	}
}

/// Renders one row of a table, its cells tagged `th` or `td` and aligned by column.
fn row(out: &mut String, r: &Row, cols: &[Align], tag: &str) {
	out.push_str("<tr>");
	for (i, cell) in r.0.iter().enumerate() {
		out.push('<');
		out.push_str(tag);
		// A row may run wider than the columns the table declared; a cell past the end is aligned by
		// nothing, which is the same as a column that named no alignment.
		align(out, cols.get(i).copied().unwrap_or_default());
		out.push('>');
		cells(out, cell);
		close(out, tag);
	}
	out.push_str("</tr>\n");
}

/// Writes a cell's alignment, where it has one.
///
/// `start` and `end` are CSS's *logical* values, and are used here rather than `left` and `right` for
/// the same reason [`Align`] names its sides that way: the side text begins on depends on which way the
/// text runs, and only the thing laying it out knows. CSS resolves them against the element's own
/// direction, so a right-to-left table aligns to the right where a left-to-right one aligns to the
/// left, from one rendering. Anyone tempted to "fix" these to `left`/`right` should read [`Align`]
/// first.
///
/// Note the spelling: the variant is `Centre` and the CSS keyword is `center`. The keyword is not ours
/// to spell.
fn align(out: &mut String, a: Align) {
	let val = match a {
		Align::None	=> return,
		Align::Start	=> "start",
		Align::Centre	=> "center",
		Align::End	=> "end",
	};
	out.push_str(" style=\"text-align: ");
	out.push_str(val);
	out.push('"');
}

/// Renders a cell's inline content.
fn cells(out: &mut String, cell: &Cell) {
	inlines(out, &cell.0);
}

/// Renders a run of inlines.
fn inlines(out: &mut String, content: &[Inline]) {
	for item in content {
		inline(out, item);
	}
}

/// Renders one inline.
fn inline(out: &mut String, item: &Inline) {
	match item {
		Inline::Text(t) => escape_text(out, t),
		Inline::Emph { strong, content } => {
			let tag = if *strong { "strong" } else { "em" };
			open(out, tag);
			inlines(out, content);
			close(out, tag);
		}
		Inline::Link { to, content } => {
			out.push_str("<a href=\"");
			escape_attr(out, to);
			out.push_str("\">");
			inlines(out, content);
			out.push_str("</a>");
		}
		Inline::Image { src, alt } => {
			out.push_str("<img src=\"");
			escape_attr(out, src);
			out.push_str("\" alt=\"");
			escape_attr(out, alt);
			out.push_str("\">");
		}
		Inline::Code(c) => {
			out.push_str("<code>");
			escape_text(out, c);
			out.push_str("</code>");
		}
		// Only ever a hard break, so it is honoured unconditionally and no whitespace rule is needed
		// here. A wrap in the author's editor never reaches the tree as one of these.
		Inline::Break => out.push_str("<br>"),
	}
}

/// Escapes a run of text for an element's content.
fn escape_text(out: &mut String, s: &str) {
	for c in s.chars() {
		match c {
			'&'	=> out.push_str("&amp;"),
			'<'	=> out.push_str("&lt;"),
			'>'	=> out.push_str("&gt;"),
			_	=> out.push(c),
		}
	}
}

/// Escapes a string for a quoted attribute value.
///
/// Both quote marks are escaped, not only the double the renderer happens to use, so a value stays
/// inert if it is ever moved into single quotes.
fn escape_attr(out: &mut String, s: &str) {
	for c in s.chars() {
		match c {
			'&'	=> out.push_str("&amp;"),
			'<'	=> out.push_str("&lt;"),
			'>'	=> out.push_str("&gt;"),
			'"'	=> out.push_str("&quot;"),
			'\''	=> out.push_str("&#39;"),
			_	=> out.push(c),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::doc::markdown;

	use oxedyne_fe2o3_core::prelude::*;

	fn text(s: &str) -> Vec<Inline> {
		vec![Inline::Text(s.to_string())]
	}

	/// Text that would otherwise close a tag or open one is written out as what it says.
	#[test]
	fn test_text_is_escaped_00() -> Outcome<()> {
		let doc = Doc { blocks: vec![Block::Para(text("Tom & Jerry <3 </p><script>"))] };
		assert_eq!(
			render(&doc),
			"<p>Tom &amp; Jerry &lt;3 &lt;/p&gt;&lt;script&gt;</p>\n",
		);
		Ok(())
	}

	/// A quote mark in a destination cannot break out of the attribute it sits in.
	#[test]
	fn test_attributes_are_escaped_01() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::Para(vec![Inline::Link {
				to:		"a\" onclick=\"steal()".to_string(),
				content:	text("here"),
			}])],
		};
		assert_eq!(
			render(&doc),
			"<p><a href=\"a&quot; onclick=&quot;steal()\">here</a></p>\n",
		);
		Ok(())
	}

	/// An alignment reaches HTML as a logical value, so a table aligns correctly whichever way its
	/// prose runs. `Centre` is spelled `center`, because the keyword is CSS's and not ours.
	#[test]
	fn test_a_table_aligns_by_logical_side_02() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::Table {
				head:	Some(Row(vec![Cell(text("Name")), Cell(text("Age"))])),
				rows:	vec![Row(vec![Cell(text("Alice")), Cell(text("30"))])],
				cols:	vec![Align::Start, Align::End],
			}],
		};
		let out = render(&doc);
		assert!(out.contains("<th style=\"text-align: start\">Name</th>"), "got: {}", out);
		assert!(out.contains("<th style=\"text-align: end\">Age</th>"), "got: {}", out);
		assert!(out.contains("<td style=\"text-align: start\">Alice</td>"), "got: {}", out);
		// Never left or right: the tree does not know which side the text begins on.
		assert!(!out.contains("left"), "got: {}", out);
		assert!(!out.contains("right"), "got: {}", out);
		Ok(())
	}

	/// A column that named no alignment, and a cell past the end of the columns, are aligned by nothing.
	#[test]
	fn test_an_unaligned_cell_carries_no_style_03() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::Table {
				head:	None,
				rows:	vec![Row(vec![Cell(text("a")), Cell(text("b"))])],
				cols:	vec![Align::None],
			}],
		};
		let out = render(&doc);
		assert!(out.contains("<td>a</td><td>b</td>"), "got: {}", out);
		assert!(!out.contains("<thead>"), "a table with no head grew one: {}", out);
		Ok(())
	}

	/// An item that is one paragraph is what a reader wrote as a plain item, and is not spaced out.
	#[test]
	fn test_a_single_paragraph_item_renders_tight_04() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::List {
				ordered:	false,
				items:		vec![vec![Block::Para(text("one"))], vec![Block::Para(text("two"))]],
			}],
		};
		assert_eq!(render(&doc), "<ul>\n<li>one</li>\n<li>two</li>\n</ul>\n");
		Ok(())
	}

	/// An item holding more than a paragraph is rendered as the blocks it is.
	#[test]
	fn test_a_richer_item_renders_its_blocks_05() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::List {
				ordered:	true,
				items:		vec![vec![Block::Para(text("one")), Block::Para(text("still one"))]],
			}],
		};
		let out = render(&doc);
		assert!(out.starts_with("<ol>\n<li>\n<p>one</p>\n<p>still one</p>\n</li>"), "got: {}", out);
		Ok(())
	}

	/// A heading outside the levels HTML has renders at the nearest one it does, rather than panicking.
	#[test]
	fn test_a_heading_level_is_clamped_06() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![
				Block::Heading { level: 0, content: text("low") },
				Block::Heading { level: 9, content: text("high") },
			],
		};
		let out = render(&doc);
		assert!(out.contains("<h1>low</h1>"), "got: {}", out);
		assert!(out.contains("<h6>high</h6>"), "got: {}", out);
		Ok(())
	}

	/// A fence's language reaches the class every highlighter reads; a fence without one says nothing.
	#[test]
	fn test_a_code_block_names_its_language_07() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![
				Block::Code { lang: Some("rust".to_string()), text: "let x = 1 < 2;\n".to_string() },
				Block::Code { lang: None, text: "plain\n".to_string() },
			],
		};
		let out = render(&doc);
		assert!(out.contains("<pre><code class=\"language-rust\">let x = 1 &lt; 2;\n</code></pre>"),
			"got: {}", out);
		assert!(out.contains("<pre><code>plain\n</code></pre>"), "got: {}", out);
		Ok(())
	}

	/// A hard break is honoured unconditionally, needing no rule of its own about whitespace.
	#[test]
	fn test_a_hard_break_is_a_br_08() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::Para(vec![
				Inline::Text("one".to_string()),
				Inline::Break,
				Inline::Text("two".to_string()),
			])],
		};
		assert_eq!(render(&doc), "<p>one<br>two</p>\n");
		Ok(())
	}

	/// The three things a signed document gives up all reach HTML intact. This is what makes the
	/// tree's neutrality a tested claim rather than an asserted one.
	#[test]
	fn test_what_sbj_drops_survives_to_html_09() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![
				Block::Rule,
				Block::Para(vec![
					Inline::Image { src: "fig.png".to_string(), alt: "a figure".to_string() },
					Inline::Code("inline()".to_string()),
				]),
			],
		};
		let out = render(&doc);
		assert!(out.contains("<hr>"), "a rule was lost: {}", out);
		assert!(out.contains("<img src=\"fig.png\" alt=\"a figure\">"), "an image by path was lost: {}", out);
		assert!(out.contains("<code>inline()</code>"), "an inline code span was lost: {}", out);
		Ok(())
	}

	/// Emphasis and strong emphasis are distinct, and nest.
	#[test]
	fn test_emphasis_nests_10() -> Outcome<()> {
		let doc = Doc {
			blocks: vec![Block::Para(vec![Inline::Emph {
				strong:		true,
				content:	vec![Inline::Emph { strong: false, content: text("both") }],
			}])],
		};
		assert_eq!(render(&doc), "<p><strong><em>both</em></strong></p>\n");
		Ok(())
	}

	/// End to end, through the reader that exists: prose in, HTML out.
	#[test]
	fn test_markdown_reaches_html_11() -> Outcome<()> {
		let src = "# A heading\n\nA paragraph with *emphasis* and a [link](https://example.com).\n";
		let doc = res!(markdown::parse(src));
		let out = render(&doc);
		assert!(out.contains("<h1>A heading</h1>"), "got: {}", out);
		assert!(out.contains("<em>emphasis</em>") || out.contains("<strong>emphasis</strong>"),
			"got: {}", out);
		assert!(out.contains("<a href=\"https://example.com\">link</a>"), "got: {}", out);
		Ok(())
	}

	/// A hard-wrapped paragraph reaches HTML as one paragraph of flowing prose. The wrap the author's
	/// editor made is not a break the author asked for, and must not become one.
	#[test]
	fn test_a_hard_wrapped_paragraph_flows_12() -> Outcome<()> {
		let doc = res!(markdown::parse("One line\nand its continuation.\n"));
		let out = render(&doc);
		assert!(!out.contains("<br>"), "a soft wrap became a hard break: {}", out);
		assert_eq!(out, "<p>One line and its continuation.</p>\n");
		Ok(())
	}
}
