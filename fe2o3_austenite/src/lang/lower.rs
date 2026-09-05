//! Lowering from the Ingot surface tree to the block layer.
//!
//! The seam between front end and engine: a surface [`Item`](super::ast::Item) becomes a
//! [`doc::Block`](crate::doc::Block) the two-pass driver already knows how to set. A heading lowers to
//! a heading; a paragraph of plain prose to a plain [`Block::Paragraph`], and a paragraph carrying any
//! emphasis to a [`Block::RichParagraph`] of [`Segment`]s -- keeping the fast single-role break path
//! for the common case. The source spans are dropped here, since the block layer does not yet carry
//! them. As the surface language grows, this is where a richer item collapses to what the engine sets.

use crate::doc::{
	Block,
	Segment,
};

use super::ast::{
	Inline,
	Item,
};

/// Lowers a surface item list to the block list the driver authors from.
pub fn blocks(items: &[Item]) -> Vec<Block> {
	let mut out = Vec::with_capacity(items.len());
	for item in items {
		match item {
			Item::Heading { level, text, .. }	=> out.push(Block::heading(*level, text.clone())),
			Item::Paragraph { runs, .. }		=> out.push(lower_paragraph(runs)),
		}
	}
	out
}

/// Lowers a paragraph's inline runs. A paragraph of one plain text run keeps the plain-paragraph path
/// (a single-role Knuth-Plass break); the moment it carries an emphasis run it becomes a rich paragraph
/// of segments, which the driver breaks with a face per run.
fn lower_paragraph(runs: &[Inline]) -> Block {
	match runs {
		[Inline::Text(text)]	=> Block::paragraph(text.clone()),
		_						=> Block::rich(runs.iter().map(lower_inline).collect()),
	}
}

fn lower_inline(run: &Inline) -> Segment {
	match run {
		Inline::Text(text)		=> Segment::text(text.clone()),
		Inline::Strong(text)	=> Segment::strong(text.clone()),
		Inline::Emph(text)		=> Segment::emph(text.clone()),
	}
}
