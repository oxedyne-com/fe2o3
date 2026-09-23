// U6a owns this file: text shaping under resolved `text` styles, and the font store `Engine::fonts`
// holds -- family lookup by name, weight, style and stretch over the embedded faces and every font file
// in the vfs, with Typst's fallback list and a warning on a missing family.

use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;
use crate::font::ShapedText;

use oxedyne_fe2o3_core::prelude::*;

/// The faces available to one compilation. Its contents are U6a's.
#[derive(Clone, Debug, Default)]
pub struct FontStore {
	pub families:	Vec<String>,	// family names known, for diagnostics
}

/// Shapes one run of text under the chain's `text` properties.
pub fn shape(_engine: &mut Engine, _text: &str, _styles: &StyleChain) -> Outcome<ShapedText> {
	Err(unimplemented("flow", "shape"))
}
