//! The page and frame model.
//!
//! A page is a view onto a frame: the frame holds boxes placed at absolute positions, and the page
//! adds its geometry and its folio. The flat-memory property lives here. Pass A builds one frame,
//! hands the page to a writer, and drops it -- so the engine holds one window of frames plus the
//! ledger, never the document.

use crate::font::ShapedText;
use crate::ir::{
	Dims,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::rect::AbsSize;

/// A page's physical geometry: its trim size and, for Phase 0, one uniform margin. The text block is
/// the trim inset by that margin on every side.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
	pub width:	Sp,
	pub height:	Sp,
	pub margin:	Sp,
}

impl PageGeometry {
	pub fn new(width: Sp, height: Sp, margin: Sp) -> Self {
		Self { width, height, margin }
	}

	/// A4 portrait, 595.276 by 841.890 points, with a two-centimetre margin (56.9 points).
	pub fn a4() -> Self {
		Self {
			width:	Sp::from_pt(595.276),
			height:	Sp::from_pt(841.890),
			margin:	Sp::from_pt(56.9),
		}
	}

	pub fn content_left(&self) -> Sp { self.margin }

	pub fn content_top(&self) -> Sp { self.margin }

	/// The width available to a line of text.
	pub fn content_width(&self) -> Sp { self.width - self.margin * 2 }

	/// The height available to a column of vertical material before the page is full.
	pub fn content_height(&self) -> Sp { self.height - self.margin * 2 }

	/// The page extent in whole device points, for an SVG viewport. A viewport extent is non-negative
	/// device-space, which is what `fe2o3_geom`'s unsigned `Dim` models; rounding to whole points is
	/// harmless here.
	pub fn media_box(&self) -> AbsSize {
		let w = self.width.to_pt().round() as usize;
		let h = self.height.to_pt().round() as usize;
		AbsSize::from((w, h))
	}
}

/// What a placed box draws. A `Reserved` is a forward reference's held-open space, outlined faintly so
/// a proof shows where a value will land.
#[derive(Clone, Debug)]
pub enum PlacedKind {
	Rule,
	Reserved,
	Text(ShapedText),
}

/// A box set at an absolute position on a page. The position is the top-left of the box; the
/// baseline sits `dims.height` below it.
#[derive(Clone, Debug)]
pub struct Placed {
	pub x:		Sp,
	pub y:		Sp,
	pub dims:	Dims,
	pub kind:	PlacedKind,
}

impl Placed {
	pub fn new(x: Sp, y: Sp, dims: Dims, kind: PlacedKind) -> Self {
		Self { x, y, dims, kind }
	}
}

/// The placed material of one page. Built in Pass A, written, then dropped.
#[derive(Clone, Debug, Default)]
pub struct Frame {
	pub placed:	Vec<Placed>,
}

impl Frame {
	pub fn new() -> Self {
		Self { placed: Vec::new() }
	}

	pub fn push(&mut self, item: Placed) {
		self.placed.push(item);
	}

	pub fn is_empty(&self) -> bool {
		self.placed.is_empty()
	}
}

/// A page: its one-based folio, its geometry, and its frame.
#[derive(Clone, Debug)]
pub struct Page {
	pub number:	u32,
	pub geom:	PageGeometry,
	pub frame:	Frame,
}

impl Page {
	pub fn new(number: u32, geom: PageGeometry, frame: Frame) -> Self {
		Self { number, geom, frame }
	}
}
