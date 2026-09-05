//! The surface syntax tree of an Ingot source file, before lowering to [`doc::Block`](crate::doc::Block).
//!
//! Increment 2 adds inline emphasis to the markup spine: a document is a sequence of headings and
//! paragraphs, and a paragraph is a sequence of [`Inline`] runs -- plain text, `*strong*`, `/emph/`.
//! The `#` code mode and the declared-query references of `sec_language` are later increments -- the
//! tree names only what the engine can already set.

use crate::ir::Span;

/// One block of Ingot markup. The byte span is carried for a future diagnostic caret; the driver's
/// `Span` model already reserves it, so the front end records it from the first increment.
#[derive(Clone, Debug)]
pub enum Item {
	Heading { level: u8, text: String, span: Span },
	Paragraph { runs: Vec<Inline>, span: Span },
}

/// One inline run of a paragraph: ordinary prose, or a run marked for emphasis. Nesting -- an emphasis
/// inside another -- is a later increment, so a run's text is flat.
#[derive(Clone, Debug)]
pub enum Inline {
	Text(String),
	Strong(String),	// *strong*, lowered to a bold segment
	Emph(String),	// /emph/, lowered to an italic segment
}
