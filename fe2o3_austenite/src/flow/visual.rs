// U6d owns this file: images, shapes, curves and transforms into one atomic box each, over `image.rs`,
// `fe2o3_graphics` paths and `ir::TransformNode`/`ir::ClipNode`.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::Node;

use oxedyne_fe2o3_core::prelude::*;

/// A visual element as one box, placeable inline or as a block.
pub fn layout_visual(
	_engine:	&mut Engine,
	_elem:		&Content,
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Node>
{
	Err(unimplemented("flow", "visual"))
}
