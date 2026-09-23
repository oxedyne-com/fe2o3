// U6c owns this file: grid and table into rows over `table.rs`, with colspan/rowspan, gutters, per-cell
// fill/stroke/align (values or closures of x and y) and repeating headers and footers.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::Region;
use crate::ir::Node;

use oxedyne_fe2o3_core::prelude::*;

/// A grid or table as vertical material: rows that may break across pages.
pub fn layout_grid(
	_engine:	&mut Engine,
	_elem:		&Content,
	_styles:	&StyleChain,
	_region:	Region,
)
	-> Outcome<Vec<Node>>
{
	Err(unimplemented("flow", "grid"))
}
