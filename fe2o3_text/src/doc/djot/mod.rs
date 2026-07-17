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
//! ordered and unordered lists (nested), thematic breaks, pipe tables, divisions, standalone
//! attributes lines, and the inline run of ordinary and strong emphasis, verbatim spans, links
//! (inline and by reference), images, attributed spans and hard breaks.
//!
//! # Not yet read
//!
//! The following Djot constructs are not yet read, and their syntax survives as the literal text it is
//! written in rather than being interpreted: footnotes, inline and display maths (`$` and `$$`),
//! definition lists, superscript and subscript (`^` and `~`), inline symbols (`:name:`), smart
//! punctuation, raw inline and raw blocks, comments (`{% %}`), line blocks, and task lists. A document
//! that uses them is read without error; the marks simply stand as characters.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::doc::djot;
//!
//! let tree = res!(djot::parse("# A heading\n\nA paragraph with *strength*.\n"));
//! ```

pub mod block;
pub mod inline;

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
