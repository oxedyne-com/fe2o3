//! Contract (U0, 2026-09-23): realised elements lower straight to [`crate::ir::Node`], never through
//! `doc::Block`, whose book idiom is what the evaluator replaces. Every function here only dispatches to
//! the owning unit's file, so none of the flow units edits this one. A container element (block, box,
//! grid cell) lays out its body by calling [`layout_block`] or `par::layout_par`, which realise one level
//! at a time through `eval::realise`; styles arrive resolved per element as its `StyleChain`.

pub mod block;
pub mod decorate;
pub mod grid;
pub mod inline;
pub mod math;
pub mod page;
pub mod par;
pub mod text;
pub mod visual;

pub use page::{
	PageBody,
	Paginator,
};

use crate::eval::content::Content;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Paint,
	Value,
};
use crate::eval::Engine;
use crate::ir::{
	Dims,
	Node,
	Sp,
};
use crate::page::PageGeometry;

use oxedyne_fe2o3_core::prelude::*;

/// The space an element is laid out into. `height` is what remains of the current column; `base` is the
/// size relative lengths resolve against (the container's, not the remainder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
	pub width:		Sp,
	pub height:		Sp,
	pub base:		(Sp, Sp),
	pub expand_x:	bool,	// fill the width rather than shrink to content
	pub expand_y:	bool,
}

impl Region {
	pub fn new(width: Sp, height: Sp) -> Self {
		Self { width, height, base: (width, height), expand_x: true, expand_y: false }
	}
}

/// Which parity a `pagebreak(to: ..)` asks the next page to have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
	Odd,
	Even,
}

/// How a page's `header` or `footer` is set. `auto` leaves the slot to the page numbering; `none` clears it,
/// the numbering with it.
#[derive(Clone, Debug)]
pub enum Slot {
	Auto,
	Off,
	On(Content),
}

/// What a run of pages under one `set page(..)` shares: the geometry, and the furniture [`decorate`]
/// evaluates per page with `here()` there. Its pages' bodies come from the [`Paginator`] one at a time, so
/// no run holds them.
#[derive(Clone, Debug)]
pub struct RunSetup {
	pub geom:		PageGeometry,
	pub columns:	usize,
	pub gutter:		Sp,
	pub fill:		Option<Paint>,
	pub numbering:	Value,			// `page.numbering`, none by default
	pub header:		Slot,
	pub footer:		Slot,
	pub background:	Option<Content>,
	pub foreground:	Option<Content>,
	pub styles:		StyleChain,		// the chain furniture is evaluated under
}

/// The document as pages, one at a time, from its content under `styles`.
pub fn paginate(content: &Content, styles: &StyleChain) -> Paginator {
	Paginator::new(content, styles)
}

/// Block-level content into a vertical list for a region.
pub fn layout_block(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	region:		Region,
)
	-> Outcome<Vec<Node>>
{
	block::layout_flow(engine, content, styles, region)
}

/// `measure(content)`: the natural size in a region, laid out and discarded.
pub fn measure(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	region:		Region,
)
	-> Outcome<Dims>
{
	block::measure(engine, content, styles, region)
}
