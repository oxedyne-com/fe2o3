// U6a owns this file: a paragraph's realised children into justified lines through `linebreak.rs`, with
// Typst's `par` settings (leading, spacing, justify, first-line-indent, hanging-indent, linebreaks).

use crate::eval::func::unimplemented;
use crate::eval::realise::Pair;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::Node;

use oxedyne_fe2o3_core::prelude::*;

/// Sets one paragraph: its inline children (already realised in `Inline` mode) as a vertical list of
/// line boxes and interline glue.
pub fn layout_par(
	_engine:	&mut Engine,
	_children:	&[Pair],
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Vec<Node>>
{
	Err(unimplemented("flow", "par"))
}
