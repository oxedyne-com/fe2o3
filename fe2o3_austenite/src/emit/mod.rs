//! Page writers.
//!
//! A writer turns a placed frame into an output artefact. The architecture keeps the frame neutral
//! -- owing nothing to any one format -- so a second writer is a new arm here, not a change to the
//! engine. Phase 0 has one, SVG. PDF and Pearl are later arms over the same frames.

pub mod svg;
pub mod pdf;

use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

/// A choice of page writer. An enum rather than a trait object, per the house preference for
/// concrete types; a new format is a new variant.
///
/// TODO (Pearl phase): `Pearl`, content-addressed blocks with the ledger shipping inside the file.
#[derive(Clone, Copy, Debug)]
pub enum Emitter {
	Svg,
	Pdf,
}

impl Emitter {
	/// Renders one page to a string. SVG only: a PDF is one binary file across every page, so a
	/// PDF document is written with [`pdf::render_document`], not a string per page.
	pub fn render(&self, page: &Page) -> Outcome<String> {
		match self {
			Emitter::Svg => svg::render_page(page),
			Emitter::Pdf => Err(err!(
				"PDF is a whole-document format; call emit::pdf::render_document, not \
				Emitter::render."; Invalid, Input)),
		}
	}

	/// The file extension a page written by this emitter should carry.
	pub fn extension(&self) -> &'static str {
		match self {
			Emitter::Svg => "svg",
			Emitter::Pdf => "pdf",
		}
	}
}
