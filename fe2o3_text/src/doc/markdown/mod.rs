//! Markdown -- a reader for the lightweight markup language, producing the neutral
//! [document tree](crate::doc).
//!
//! Markdown is the form most prose is written in, and a great deal of prose already exists in it. This
//! is the front-end that reads it. What it produces belongs to [`crate::doc`] and knows nothing of
//! Markdown, which is what lets a second front-end produce the same tree.
//!
//! # The dialect
//!
//! The subset parsed is the CommonMark core that prose actually uses: ATX and setext headings,
//! paragraphs, fenced and indented code, block quotations, ordered and unordered lists (nested),
//! thematic breaks, and the inline run of emphasis, strong emphasis, links, images, code spans and
//! hard breaks. It is not a conformant CommonMark implementation and does not try to be: the reference
//! test suite is largely a catalogue of pathological nesting that no author writes. Where this reader
//! and CommonMark differ on such input, this reader is simply making its own choice.
//!
//! # A soft line break says a space
//!
//! A single newline within a paragraph is a soft break, and it contributes a space rather than a
//! newline. Where an author's editor wrapped a line is not where the author asked for a break, and
//! preserving it would freeze prose at the width it was typed at instead of reflowing to the width it
//! is read at.
//!
//! The trap is that CommonMark's own HTML output writes a soft break as a newline, which looks like a
//! licence to keep it. It is not: HTML collapses whitespace, so that newline is rendered as a space by
//! the reader that receives it. A renderer that honours whitespace would take it as a break the author
//! never asked for. So the tree carries the meaning and not the byte, and
//! [`Inline::Break`](crate::doc::Inline::Break) is only ever a break the author did ask for.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::doc::markdown;
//!
//! let tree = res!(markdown::parse("# A heading\n\nA paragraph with *emphasis*.\n"));
//! ```

pub mod block;
pub mod inline;

use crate::doc::Doc;

use oxedyne_fe2o3_core::prelude::*;

/// Reads Markdown text and produces its document tree.
///
/// Parsing does not fail on badly formed markup: Markdown has no syntax errors, only text that means
/// less than the author hoped. An unclosed fence runs to the end, an unmatched bracket is literal
/// text, and a stray asterisk is an asterisk. The outcome is an error only when the input breaks a
/// limit the reader holds against a hostile document, such as nesting past [`block::DEPTH_LIMIT`].
pub fn parse(src: &str) -> Outcome<Doc> {
	let blocks = res!(block::parse(src));
	Ok(Doc { blocks })
}
