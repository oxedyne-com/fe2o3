//! Lowering from the Ingot surface tree to the block layer.
//!
//! The seam between front end and engine: a surface [`Item`](super::ast::Item) becomes a
//! [`doc::Block`](crate::doc::Block) the two-pass driver already knows how to set. Increment 1 is a
//! one-to-one mapping -- a heading to a heading, a paragraph to a paragraph -- and the source spans
//! are dropped here, since the block layer does not yet carry them. As the surface language grows,
//! this is where a richer item collapses to the blocks the engine can compose.

use crate::doc::Block;

use super::ast::Item;

/// Lowers a surface item list to the block list the driver authors from.
pub fn blocks(items: &[Item]) -> Vec<Block> {
	let mut out = Vec::with_capacity(items.len());
	for item in items {
		match item {
			Item::Heading { level, text, .. }	=> out.push(Block::heading(*level, text.clone())),
			Item::Paragraph { text, .. }		=> out.push(Block::paragraph(text.clone())),
		}
	}
	out
}
