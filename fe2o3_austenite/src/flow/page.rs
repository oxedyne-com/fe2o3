// U6b owns this file: the document's top level into page runs, one per `set page(..)` span or explicit
// `page(..)`, honouring `pagebreak(weak:, to:)`. The driver (U6b's `driver.rs` change) takes the runs.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::flow::PageRun;

use oxedyne_fe2o3_core::prelude::*;

pub fn layout_document(
	_engine:	&mut Engine,
	_content:	&Content,
	_styles:	&StyleChain,
)
	-> Outcome<Vec<PageRun>>
{
	Err(unimplemented("flow", "document"))
}
