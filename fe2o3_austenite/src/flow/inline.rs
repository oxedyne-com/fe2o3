// U6a owns this file: inline items -- shaped text, spacing, inline boxes, equations, footnote marks,
// link spans -- collected from a paragraph's children for the line breaker. Internal to U6a.

use crate::eval::func::unimplemented;
use crate::eval::realise::Pair;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::Node;

use oxedyne_fe2o3_core::prelude::*;

/// One unit the line breaker sees.
#[derive(Clone, Debug)]
pub enum InlineItem {
	Node(Node),		// shaped text, an inline box, a mark
	Space,			// an inter-word space, stretchable
	Linebreak { justify: bool },
}

pub fn collect(_engine: &mut Engine, _children: &[Pair], _region: Region) -> Outcome<Vec<InlineItem>> {
	Err(unimplemented("flow", "inline"))
}
