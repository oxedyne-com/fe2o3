// U6b owns this file: block-level flow -- block, pad, align, stack, v with fr, place (float and parent
// scope), columns and colbreak -- dispatching inline runs to `par` and atoms to `grid`, `visual`, `math`.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::{
	Dims,
	Node,
};

use oxedyne_fe2o3_core::prelude::*;

pub fn layout_flow(
	_engine:	&mut Engine,
	_content:	&Content,
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Vec<Node>>
{
	Err(unimplemented("flow", "block"))
}

pub fn measure(
	_engine:	&mut Engine,
	_content:	&Content,
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Dims>
{
	Err(unimplemented("flow", "measure"))
}
