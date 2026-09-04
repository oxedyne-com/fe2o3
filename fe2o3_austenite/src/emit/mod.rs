//! Page writers.
//!
//! A writer turns a placed frame into an output artefact. The architecture keeps the frame neutral
//! -- owing nothing to any one format -- so a second writer is a new arm here, not a change to the
//! engine. Phase 0 has one, SVG. PDF and Pearl are later arms over the same frames.

pub mod svg;

use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

/// A choice of page writer. An enum rather than a trait object, per the house preference for
/// concrete types; a new format is a new variant.
///
/// TODO Phase 6: `Pdf`, extending `fe2o3_graphics` as a writer beside SVG and PNG.
/// TODO (Pearl phase): `Pearl`, content-addressed blocks with the ledger shipping inside the file.
#[derive(Clone, Copy, Debug)]
pub enum Emitter {
	Svg,
}

impl Emitter {
	/// Renders one page to a string in the chosen format.
	pub fn render(&self, page: &Page) -> Outcome<String> {
		match self {
			Emitter::Svg => svg::render_page(page),
		}
	}

	/// The file extension a page written by this emitter should carry.
	pub fn extension(&self) -> &'static str {
		match self {
			Emitter::Svg => "svg",
		}
	}
}
