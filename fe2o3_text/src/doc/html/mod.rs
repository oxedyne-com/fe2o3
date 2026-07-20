//! HTML -- a reader for the form a generator exports, and a writer for the form a browser reads.
//!
//! This module faces both ways. [`parse`] reads HTML into the neutral [document tree](crate::doc), as
//! [`markdown`](crate::doc::markdown) reads Markdown into it; [`render`] walks the tree back out. The
//! two are not inverses and are not meant to be -- what the tree does not carry, no reader can invent
//! and no writer can restore -- but between them they make the tree's neutrality testable rather than
//! merely asserted.
//!
//! # Why read HTML at all
//!
//! Because a great deal of prose is *exported* to it rather than written in it. A typesetter that
//! evaluates an author's own macros and emits HTML has already done the hard half of the work: what
//! comes out is the prose, with the author's abbreviations, cross-references and templates resolved.
//! Reading that HTML is how such prose reaches the tree without the reader having to understand the
//! language it was written in.
//!
//! That is what this reads: HTML a generator wrote. It is not a browser's parser, does not recover
//! from mis-nesting the way a browser must, and does not try to. See [`read`] for what it does with
//! the tags it does not know, which is the part worth knowing.
//!
//! # HTML collapses whitespace
//!
//! A run of spaces, tabs and newlines between two words says one space, and the whitespace at either
//! end of a block says nothing at all. So an exporter's indentation and line endings are not the
//! author's, and are not kept.
//!
//! This is the mirror of the rule [`markdown`](crate::doc::markdown) states, and it exists for the
//! same reason. There a newline within a paragraph had to *become* a space; here a run of whitespace
//! has to *collapse to* one. Both say that where a line ended in the source is not something the
//! author asked for, and that prose should reflow to the width it is read at rather than freeze at the
//! width it was written or exported at. Get this wrong and every paragraph of a book carries the
//! exporter's line breaks for ever.
//!
//! The one exception is `<pre>`, where whitespace is exactly what the content means. Its line
//! structure reaches [`Block::Code`](crate::doc::Block::Code) intact.
//!
//! [`Inline::Break`](crate::doc::Inline::Break) therefore only ever comes from a `<br>`, and never
//! from a newline in the source.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::doc::html;
//!
//! let tree = res!(html::parse("<h1>A heading</h1>\n<p>A paragraph with <em>emphasis</em>.</p>\n"));
//! let out = html::render(&tree);
//! ```

pub mod read;
pub mod write;

use crate::doc::Doc;

use oxedyne_fe2o3_core::prelude::*;

pub use self::write::{
	Opts,
	escape_attr,
	escape_text,
	render,
	render_with,
};

/// Reads HTML and produces its document tree.
///
/// Parsing does not fail on markup that means less than it might: a tag the tree has no node for is
/// unwrapped, a stray close tag closes nothing, and an element left open runs to the end of what
/// encloses it. The outcome is an error only when the input breaks the one limit the reader holds
/// against a hostile document, [`read::DEPTH_LIMIT`].
pub fn parse(src: &str) -> Outcome<Doc> {
	let blocks = res!(read::parse(src));
	Ok(Doc { blocks })
}
