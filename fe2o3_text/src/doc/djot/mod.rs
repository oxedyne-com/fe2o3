//! Djot -- a reader for the post-Markdown markup language, producing the neutral
//! [document tree](crate::doc).
//!
//! Djot is what a prose author reaches for when Markdown cannot name the thing they mean: a box around
//! an aside, a class on a span, an id on a heading. Its blocks and inlines carry [attributes](crate::doc::Attrs),
//! and a `:::` fence draws a [division](crate::doc::Block::Div) that Markdown has no way to write. What
//! this reader produces belongs to [`crate::doc`] and knows nothing of Djot, which is what lets a site
//! author the same page in either Djot or Markdown and reach the one tree.
//!
//! # Where Djot parts from Markdown
//!
//! The two front-ends agree on most of the tree, and part on a few points a reader of both should hold
//! in mind:
//!
//! - The emphasis markers are swapped and single. A single `_` is ordinary emphasis and a single `*`
//!   is strong, where Markdown reads a single marker as ordinary and doubles it for strong. So `_it_`
//!   is italic here and `*it*` is bold, and neither is doubled.
//! - Emphasis may fall inside a word, since Djot judges a marker by the whitespace against it and keeps
//!   no intraword exception.
//! - There is no two-space hard break and no indented code block. A break the author asked for is a
//!   backslash at the end of the line, and code is fenced.
//! - A `:::` fence draws a division, and a `{...}` group names attributes, on a division, on a span, or
//!   on its own line to attach to the block below. These are the constructs Djot exists for.
//!
//! # A soft line break says a space
//!
//! A single newline within a paragraph is a soft break, and it contributes a space rather than a
//! newline. Where an author's editor wrapped a line is not where the author asked for a break, and
//! preserving it would freeze prose at the width it was typed at instead of reflowing to the width it
//! is read at. [`Inline::Break`](crate::doc::Inline::Break) is only ever a break the author did ask
//! for -- here, a backslash at the end of a line.
//!
//! # The dialect
//!
//! The subset read is what prose actually uses: headings, paragraphs, block quotations, fenced code,
//! ordered and unordered lists (nested), thematic breaks, pipe tables (with or without a header, and
//! with a `^ ` caption), divisions, standalone attributes lines, and the inline run of ordinary and
//! strong emphasis, superscript and subscript (each with Djot's braced forms, `{_` and `_}`), verbatim
//! spans, links (inline and by reference), images (with an attribute block), attributed spans and hard
//! breaks.
//!
//! # Not yet read
//!
//! The following Djot constructs are not yet read, and their syntax survives as the literal text it is
//! written in rather than being interpreted: footnotes, inline and display maths (`$` and `$$`),
//! definition lists, inline symbols (`:name:`), smart punctuation, raw inline and raw blocks, comments
//! (`{% %}`), line blocks, and task lists. A document that uses them is read without error; the marks
//! simply stand as characters.
//!
//! # Writing
//!
//! [`render`] writes a tree back out, and `parse(&render(&d)) == d` for every tree `d` in the
//! vocabulary below that a reader could have produced: text merged into one run between other inlines,
//! no line ending inside text or inline code, no two code spans side by side, no whitespace at either end of a paragraph, heading,
//! cell or caption, no hard break at the end of a block, in a cell or in a caption, a code block's text
//! empty or ending in a line ending and holding no line of whitespace alone, and attribute names and
//! values Djot can spell (no space, `=`, `,`, brace or quote in a name; no quote, bracket, backtick or
//! pipe in a value). Where the tree holds something with two Djot spellings, the one written is in
//! the right-hand column.
//!
//! | Tree | Djot written |
//! |---|---|
//! | `Heading { level, content }` | `#` to `######`, a space, the content |
//! | `Para` | the content, one line, a hard break continuing it onto the next |
//! | `List { ordered: false }` | `- ` items; `+ ` for a list straight after another unordered one |
//! | `List { ordered: true }` | `1. ` items; `1) ` for a list straight after another ordered one |
//! | an item's later lines | indented by the marker's width, so nesting is the sum of the markers |
//! | an item whose marker line would read as a break (`- - -`) | the marker alone, the body from the next line |
//! | `Code { lang, text }` | a backtick fence one longer than any run inside, at least three, then `lang` |
//! | `Quote` | `> ` before each line, `>` alone on a blank one |
//! | `Table { head: Some(..) }` | header row, delimiter row, body rows |
//! | `Table { head: None }` | the delimiter row first, then the body rows |
//! | `cols` | `---`, `:--` (start), `:-:` (centre), `--:` (end) |
//! | `Table { caption }` | a blank line then `^ caption` |
//! | `Rule` | `***`, since `- ---` reads as a break and not an item holding one |
//! | `Div { attrs }` | `::: class` where the attrs are one class alone, else `::: {#id .class k="v"}`; `::: {}` when empty |
//! | nested `Div` | `:::` at every depth, each closing fence closing the innermost division open |
//! | `Attrs` | `{#id .class k="v"}`, in that order, every value quoted |
//! | `Text` | escaped: see the writer's own notes |
//! | `Emph { strong: false }` | `_x_`, or `{_x_}` where plain would not read back |
//! | `Emph { strong: true }` | `*x*`, or `{*x*}` likewise |
//! | `Sup`, `Sub` | `^x^`, `~x~`, braced likewise |
//! | `Link { to, content }` | `[content](to)`, never an autolink or a reference; `(<to>)` where `to` holds whitespace |
//! | `Image { src, alt, attrs }` | `![alt](src)`, then `{attrs}` where there are any |
//! | `Code` | a backtick run one longer than any inside, padded with a space where the reader would trim one |
//! | `Span { attrs, content }` | `[content]{attrs}`, `{}` included when the attrs are empty |
//! | `Break` | a backslash at the end of the line |
//! | `Figure`, `Terms`, a `Cell` with `header: true` | not yet: written as [`Block::as_plain`](crate::doc::Block::as_plain) gives, and the header flag dropped |
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::doc::djot;
//!
//! let tree = res!(djot::parse("# A heading\n\nA paragraph with *strength*.\n"));
//! let text = djot::render(&tree);
//! ```

pub mod block;
pub mod inline;
pub mod write;

pub use write::render;

use crate::doc::Doc;

use oxedyne_fe2o3_core::prelude::*;

/// Reads Djot text and produces its document tree.
///
/// Parsing does not fail on badly formed markup: Djot has no syntax errors, only text that means less
/// than the author hoped. An unclosed fence runs to the end, an unmatched bracket is literal text, and
/// a stray asterisk is an asterisk. The outcome is an error only when the input breaks a limit the
/// reader holds against a hostile document, such as nesting past [`block::DEPTH_LIMIT`].
pub fn parse(src: &str) -> Outcome<Doc> {
	let blocks = res!(block::parse(src));
	Ok(Doc { blocks })
}
