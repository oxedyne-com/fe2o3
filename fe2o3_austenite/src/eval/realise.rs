// U4 owns this file: content plus styles in, an ordered element stream out, with show rules applied,
// locations assigned and inline runs grouped. Flow calls it again for each container's body, so one call
// realises one level, not the whole tree.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

/// What the caller will do with the stream, which decides grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealiseMode {
	Document,	// page elements and pagebreaks surface; everything else as in Flow
	Flow,		// block-level: inline runs grouped into `par`, list items into `list`/`enum`/`terms`
	Inline,		// a paragraph's children: text runs merged, text/regex show rules applied
	Math,		// an equation's body
}

/// One realised element and the styles it is laid out under.
#[derive(Clone, Debug)]
pub struct Pair {
	pub content:	Content,
	pub styles:		StyleChain,
}

pub fn realise(
	_engine:	&mut Engine,
	_content:	&Content,
	_styles:	&StyleChain,
	_mode:		RealiseMode,
)
	-> Outcome<Vec<Pair>>
{
	Err(unimplemented("realise", "realise"))
}
