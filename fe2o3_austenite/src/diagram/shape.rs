//! Node shapes: the outline a box draws, and the named points an edge joins.
//!
//! A shape sizes itself to its label plus padding, draws its outline as one closed
//! `fe2o3_graphics` path in the figure's own frame (y down, in points), and exposes the five ports.
//! The ports are the bounding box's four side midpoints and its centre, which for a diamond are its
//! four vertices and centre, so one port rule serves every shape.

use crate::ir::Sp;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	Path,
	PathBuilder,
	Pt,
};

/// A placed node's box in the figure frame: its top-left corner and its extent, all scaled points.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
	pub x:	Sp,	// left edge
	pub y:	Sp,	// top edge, the frame being y down
	pub w:	Sp,
	pub h:	Sp,
}

impl Rect {
	pub fn new(x: Sp, y: Sp, w: Sp, h: Sp) -> Self {
		Self { x, y, w, h }
	}

	pub fn centre_x(&self) -> Sp { self.x + Sp(self.w.raw() / 2) }
	pub fn centre_y(&self) -> Sp { self.y + Sp(self.h.raw() / 2) }
	pub fn right(&self) -> Sp { self.x + self.w }
	pub fn bottom(&self) -> Sp { self.y + self.h }
}

/// A named point on a node's box, where an edge attaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
	North,
	South,
	East,
	West,
	Centre,
}

/// The outline a node draws. An enum, not a trait object, so a new shape is a new arm the sizing and
/// the drawing must both answer -- the house preference for concrete types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
	Box,		// a plain rectangle, the default process step
	Stadium,	// a rectangle with fully rounded ends, a start or terminal
	Diamond,	// a rhombus on the box's side midpoints, a decision
}

impl Shape {
	/// The box that holds a label of the given width and vertical extent, once padded. A stadium adds
	/// its two semicircular caps to the width so the label clears the curve; a diamond is grown so the
	/// label's rectangle sits inside the rhombus rather than poking through its sloped sides.
	///
	/// # Arguments
	/// `label_ext` is the label's height plus depth, its full vertical extent; the pads are the gap
	/// wanted on each side, in scaled points.
	pub fn size_for_label(
		&self,
		label_w:	Sp,
		label_ext:	Sp,
		pad_x:		Sp,
		pad_y:		Sp,
	)
		-> (Sp, Sp)
	{
		match self {
			Shape::Box => (
				label_w + pad_x + pad_x,
				label_ext + pad_y + pad_y,
			),
			Shape::Stadium => (
				// The caps eat about half the height at each end, so a full extent of width is added.
				label_w + label_ext + pad_x + pad_x,
				label_ext + pad_y + pad_y,
			),
			Shape::Diamond => (
				// A rhombus twice the label's size holds the label rectangle with room to spare, since
				// the rectangle's corner then sits just inside the sloped side.
				Sp(label_w.raw() * 2) + Sp(pad_x.raw() * 4),
				Sp(label_ext.raw() * 2) + Sp(pad_y.raw() * 4),
			),
		}
	}

	/// The outline as one closed path, in the figure frame (y down, points).
	pub fn outline(&self, r: &Rect) -> Outcome<Path> {
		let x0 = r.x.to_pt() as f32;
		let y0 = r.y.to_pt() as f32;
		let x1 = r.right().to_pt() as f32;
		let y1 = r.bottom().to_pt() as f32;
		match self {
			Shape::Box => Path::rect(Bounds::new(x0, y0, x1, y1)),
			Shape::Stadium => {
				// A radius of half the height rounds the ends to true semicircles; round_rect clamps to
				// half the shorter side, so a tall narrow box degrades to its inscribed stadium.
				let radius = (r.h.to_pt() as f32) * 0.5;
				Path::round_rect(Bounds::new(x0, y0, x1, y1), radius)
			},
			Shape::Diamond => {
				let cx = r.centre_x().to_pt() as f32;
				let cy = r.centre_y().to_pt() as f32;
				let mut pb = PathBuilder::new();
				pb.move_to(Pt::new(cx, y0));	// north vertex
				pb.line_to(Pt::new(x1, cy));	// east
				pb.line_to(Pt::new(cx, y1));	// south
				pb.line_to(Pt::new(x0, cy));	// west
				pb.close();
				pb.finish()
			},
		}
	}

	/// A port's coordinate on the box. The rule is the bounding box's side midpoints and centre, which
	/// coincide with a diamond's own vertices, so the shape does not enter the calculation.
	pub fn port(&self, r: &Rect, port: Port) -> (Sp, Sp) {
		port_of(r, port)
	}
}

/// The bounding-box port coordinate, shared by every shape.
pub fn port_of(r: &Rect, port: Port) -> (Sp, Sp) {
	match port {
		Port::North		=> (r.centre_x(), r.y),
		Port::South		=> (r.centre_x(), r.bottom()),
		Port::East		=> (r.right(), r.centre_y()),
		Port::West		=> (r.x, r.centre_y()),
		Port::Centre	=> (r.centre_x(), r.centre_y()),
	}
}
