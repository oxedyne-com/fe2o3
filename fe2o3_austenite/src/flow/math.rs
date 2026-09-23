// U7 owns this file: an equation's realised maths elements into `math.rs` atoms and out as one box. A
// block equation's numbering and alignment are applied by flow/block from the element's fields.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::Node;

use oxedyne_fe2o3_core::prelude::*;

pub fn layout_equation(
	_engine:	&mut Engine,
	_elem:		&Content,
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Node>
{
	Err(unimplemented("flow", "equation"))
}
