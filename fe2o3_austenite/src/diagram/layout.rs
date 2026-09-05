//! One-pass placement, edge routing and arrowheads.
//!
//! Placement is a single dependency-ordered pass, as the design requires: a node is placed
//! absolutely, or relative to a node already placed, and never by a solver. Nodes are aligned by
//! centre so a `below` chain reads as a column. An edge is a polyline between two ports -- straight,
//! or orthogonal with axis-aligned segments and a perpendicular stub out of each port -- ended with a
//! small filled triangle for the arrowhead.

use crate::diagram::shape::{
	port_of,
	Port,
	Rect,
};
use crate::ir::Sp;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::path::{
	Path,
	PathBuilder,
	Pt,
};

/// Where a node is placed. The relative arms name an earlier node by its index, so resolution is one
/// forward pass with no back-references.
#[derive(Clone, Copy, Debug)]
pub enum Placement {
	At { x: Sp, y: Sp },			// centre at an absolute point
	Below { of: usize, gap: Sp },	// centred under an earlier node, its box bottom plus the gap
	Right { of: usize, gap: Sp },	// centred to the right of an earlier node, its box right plus the gap
}

/// How an edge is routed between its two ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
	Straight,
	Orthogonal,
}

/// A cardinal direction, the way a port faces out of its box: an edge leaves and enters along it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir4 {
	Up,
	Down,
	Left,
	Right,
}

impl Dir4 {
	/// The unit step in the figure frame (y down), as integer signs.
	fn step(&self) -> (i32, i32) {
		match self {
			Dir4::Up	=> (0, -1),
			Dir4::Down	=> (0, 1),
			Dir4::Left	=> (-1, 0),
			Dir4::Right	=> (1, 0),
		}
	}

	fn is_vertical(&self) -> bool {
		matches!(self, Dir4::Up | Dir4::Down)
	}
}

/// The direction a port faces, or `None` for the centre, which faces nowhere.
pub fn facing(port: Port) -> Option<Dir4> {
	match port {
		Port::North		=> Some(Dir4::Up),
		Port::South		=> Some(Dir4::Down),
		Port::East		=> Some(Dir4::Right),
		Port::West		=> Some(Dir4::Left),
		Port::Centre	=> None,
	}
}

/// Places every node in one pass, aligning relative placements by centre. Each entry is a node's
/// placement and its already-sized box width and height; the result is the boxes in a provisional
/// frame, which the caller normalises to the figure origin once the ink is known.
pub fn place(specs: &[(Placement, Sp, Sp)]) -> Outcome<Vec<Rect>> {
	let mut rects: Vec<Rect> = Vec::with_capacity(specs.len());
	for (i, (placement, w, h)) in specs.iter().enumerate() {
		let (cx, cy) = match *placement {
			Placement::At { x, y } => (x, y),
			Placement::Below { of, gap } => {
				let anchor = res!(rects.get(of).ok_or_else(|| err!(
					"Node {} is placed below node {}, which is not yet placed.", i, of;
					Invalid, Input)));
				(anchor.centre_x(), anchor.bottom() + gap + Sp(h.raw() / 2))
			},
			Placement::Right { of, gap } => {
				let anchor = res!(rects.get(of).ok_or_else(|| err!(
					"Node {} is placed right of node {}, which is not yet placed.", i, of;
					Invalid, Input)));
				(anchor.right() + gap + Sp(w.raw() / 2), anchor.centre_y())
			},
		};
		// The centre fixes the box; store its top-left corner.
		rects.push(Rect::new(cx - Sp(w.raw() / 2), cy - Sp(h.raw() / 2), *w, *h));
	}
	Ok(rects)
}

/// The side port of `r` whose coordinate is nearest `target`, so an edge given a node but no explicit
/// port attaches on the side facing the other end.
pub fn nearest_port(r: &Rect, target: (Sp, Sp)) -> Port {
	let tx = target.0.to_pt() as f32;
	let ty = target.1.to_pt() as f32;
	let mut best = Port::North;
	let mut best_d = f32::INFINITY;
	for port in [Port::North, Port::South, Port::East, Port::West] {
		let (px, py) = port_of(r, port);
		let dx = (px.to_pt() as f32) - tx;
		let dy = (py.to_pt() as f32) - ty;
		let d = dx * dx + dy * dy;
		if d < best_d {
			best_d = d;
			best = port;
		}
	}
	best
}

/// The polyline an edge follows, from one port to the other. A straight edge is the two endpoints; an
/// orthogonal edge leaves each port along a short perpendicular stub, then bends once to join the two
/// stubs with axis-aligned segments, so every segment is horizontal or vertical.
pub fn route_points(
	from:		(Sp, Sp),
	from_dir:	Option<Dir4>,
	to:			(Sp, Sp),
	to_dir:		Option<Dir4>,
	route:		Route,
	stub:		Sp,
)
	-> Vec<(Sp, Sp)>
{
	match route {
		Route::Straight => vec![from, to],
		Route::Orthogonal => {
			let p1 = offset(from, from_dir, stub);
			let p2 = offset(to, to_dir, stub);
			// Bend so the segment arriving at the far stub is perpendicular to the near stub's axis: a
			// vertical near stub runs to the far y first, a horizontal one to the far x first.
			let vertical_first = match from_dir {
				Some(d) => d.is_vertical(),
				None    => (to.1.raw() - from.1.raw()).abs() >= (to.0.raw() - from.0.raw()).abs(),
			};
			let bend = if vertical_first {
				(p1.0, p2.1)
			} else {
				(p2.0, p1.1)
			};
			let mut pts = vec![from, p1, bend, p2, to];
			dedup(&mut pts);
			pts
		},
	}
}

/// Moves a point out along a facing direction by `stub`; a centre port (no facing) is left where it is.
fn offset(p: (Sp, Sp), dir: Option<Dir4>, stub: Sp) -> (Sp, Sp) {
	match dir {
		Some(d) => {
			let (sx, sy) = d.step();
			(p.0 + Sp(stub.raw() * sx), p.1 + Sp(stub.raw() * sy))
		},
		None => p,
	}
}

/// Drops points equal to their predecessor, which a degenerate bend or a zero stub can leave.
fn dedup(pts: &mut Vec<(Sp, Sp)>) {
	pts.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
}

/// A stroked polyline as an open path in the figure frame.
pub fn stroke_path(pts: &[(Sp, Sp)]) -> Outcome<Path> {
	if pts.len() < 2 {
		return Err(err!(
			"An edge needs at least two points to stroke, but {} were given.", pts.len();
			Invalid, Input));
	}
	let mut pb = PathBuilder::new();
	pb.move_to(sp_pt(pts[0]));
	for p in &pts[1..] {
		pb.line_to(sp_pt(*p));
	}
	pb.finish()
}

/// The point pulled back from `tip` towards `prev` by `len` points, so a stroke stops at the base of
/// the arrowhead rather than running under its tip.
pub fn retract(tip: (Sp, Sp), prev: (Sp, Sp), len: f32) -> (Sp, Sp) {
	let (ux, uy) = match unit(prev, tip) {
		Some(u) => u,
		None    => return tip,	// coincident points: nothing to pull back along
	};
	(
		Sp::from_pt((tip.0.to_pt() as f32 - ux * len) as f64),
		Sp::from_pt((tip.1.to_pt() as f32 - uy * len) as f64),
	)
}

/// A filled triangle for the arrowhead: tip at the edge's end, base two corners back along the
/// incoming direction, spread `half` either side of the centreline.
pub fn arrowhead(
	tip:	(Sp, Sp),
	prev:	(Sp, Sp),
	len:	f32,
	half:	f32,
)
	-> Outcome<Path>
{
	let (ux, uy) = res!(unit(prev, tip).ok_or_else(|| err!(
		"An arrowhead has no direction: its edge ends where it begins."; Invalid, Input)));
	let tx = tip.0.to_pt() as f32;
	let ty = tip.1.to_pt() as f32;
	let bx = tx - ux * len;	// base centre
	let by = ty - uy * len;
	let (px, py) = (-uy, ux);	// unit perpendicular
	let mut pb = PathBuilder::new();
	pb.move_to(Pt::new(tx, ty));
	pb.line_to(Pt::new(bx + px * half, by + py * half));
	pb.line_to(Pt::new(bx - px * half, by - py * half));
	pb.close();
	pb.finish()
}

/// The midpoint of the polyline's longest segment, and the unit perpendicular to it, so a caller can
/// seat an edge label clear of the line. `None` for a polyline of one point.
pub fn label_anchor(pts: &[(Sp, Sp)]) -> Option<((Sp, Sp), (f32, f32))> {
	let mut best = 0usize;
	let mut best_len = -1.0f32;
	for i in 1..pts.len() {
		let dx = (pts[i].0.raw() - pts[i - 1].0.raw()) as f32;
		let dy = (pts[i].1.raw() - pts[i - 1].1.raw()) as f32;
		let l = dx * dx + dy * dy;
		if l > best_len {
			best_len = l;
			best = i;
		}
	}
	if best == 0 {
		return None;
	}
	let a = pts[best - 1];
	let b = pts[best];
	let mid = (
		Sp((a.0.raw() + b.0.raw()) / 2),
		Sp((a.1.raw() + b.1.raw()) / 2),
	);
	let perp = match unit(a, b) {
		Some((ux, uy)) => (-uy, ux),
		None           => (0.0, -1.0),
	};
	Some((mid, perp))
}

/// The unit vector from `a` to `b` in points, or `None` if they coincide.
fn unit(a: (Sp, Sp), b: (Sp, Sp)) -> Option<(f32, f32)> {
	let dx = (b.0.to_pt() - a.0.to_pt()) as f32;
	let dy = (b.1.to_pt() - a.1.to_pt()) as f32;
	let d = (dx * dx + dy * dy).sqrt();
	if d <= f32::EPSILON {
		return None;
	}
	Some((dx / d, dy / d))
}

/// A scaled-point pair as a graphics point.
fn sp_pt(p: (Sp, Sp)) -> Pt {
	Pt::new(p.0.to_pt() as f32, p.1.to_pt() as f32)
}
