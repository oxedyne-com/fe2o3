//! The surface syntax tree of an Ingot source file, before lowering to [`doc::Block`](crate::doc::Block).
//!
//! Increment 1 is the markup spine: a document is a sequence of headings and paragraphs. Inline
//! emphasis, the `#` code mode, and the declared-query references of `sec_language` are later
//! increments -- the tree names only what the engine can already set.

use crate::ir::Span;

/// One block of Ingot markup. The byte span is carried for a future diagnostic caret; the driver's
/// `Span` model already reserves it, so the front end records it from the first increment.
#[derive(Clone, Debug)]
pub enum Item {
	Heading { level: u8, text: String, span: Span },
	Paragraph { text: String, span: Span },
}
