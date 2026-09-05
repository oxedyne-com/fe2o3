//! The diagram sub-language: labelled nodes and routed edges, emitted as drawn vector paths.
//!
//! A diagram stands to Austenite as CeTZ stands to Typst, under one rule carried from the
//! architecture: elements are placed by anchor and relative offset in a single dependency-ordered
//! pass, with no constraint solver. A [`Diagram`] is built up as a list of nodes and edges and then
//! [`build`](Diagram::build) into a [`Graphic`] -- the same `fe2o3_graphics` outline paths the body
//! text emits, so a figure is first-class content in the SVG and the PDF, not an embedded picture.
//!
//! A node's label travels the prose's own shaping path: `fe2o3_font` shapes it, each glyph becomes an
//! outline, and the outlines are baked into the figure exactly as the SVG writer's `draw_text` bakes a
//! line of text -- flipped from the font's y-up frame onto the figure's y-down baseline. The box is
//! auto-sized to the label. An edge is a real polyline between two ports, routed once the nodes are
//! placed, ended with a filled-triangle arrowhead; an orthogonal edge keeps every segment axis
//! aligned. Because placement is one ordered pass, a node depends only on nodes placed before it, and
//! a bad reference names the node and says what is wrong.

pub mod layout;
pub mod shape;

use crate::diagram::layout::{
	Placement,
	Route,
};
use crate::diagram::shape::{
	Port,
	Rect,
	Shape,
};
use crate::font::ShapedText;
use crate::ir::{
	Dims,
	DrawOp,
	Graphic,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::Bounds,
	transform::Transform,
};

use std::collections::BTreeMap;
use std::sync::Arc;

/// One end of an edge: the node it attaches to, and optionally the named port. With no port the
/// builder chooses the side facing the other end once both are placed.
#[derive(Clone, Debug)]
pub struct Endpoint {
	pub node:	String,
	pub port:	Option<Port>,
}

impl Endpoint {
	/// An end attached to a node, its port chosen automatically at build.
	pub fn node<S: Into<String>>(node: S) -> Self {
		Self { node: node.into(), port: None }
	}

	/// An end attached to a named port of a node.
	pub fn port<S: Into<String>>(node: S, port: Port) -> Self {
		Self { node: node.into(), port: Some(port) }
	}
}

// How a node is placed, keyed by node id, before ids are resolved to indices at build.
#[derive(Clone, Debug)]
enum PlaceSpec {
	At { x: Sp, y: Sp },
	Below { of: String, gap: Sp },
	Right { of: String, gap: Sp },
}

#[derive(Clone, Debug)]
struct NodeSpec {
	id:		String,
	label:	String,
	place:	PlaceSpec,
	shape:	Shape,
}

#[derive(Clone, Debug)]
struct EdgeSpec {
	from:	Endpoint,
	to:		Endpoint,
	label:	Option<String>,
	route:	Route,
}

/// The lengths and colours a diagram is drawn to. Every length is scaled points, so the styling never
/// leaves the integer domain; the widths and the arrowhead sizes are points, as the stroke and the
/// graphics boundary take them.
#[derive(Clone, Copy, Debug)]
pub struct DiagramStyle {
	pub label_size:			Sp,				// the node label's body size
	pub edge_label_size:	Sp,				// the smaller size an edge label is set at
	pub pad_x:				Sp,				// horizontal gap between a label and its box side
	pub pad_y:				Sp,				// vertical gap between a label and its box side
	pub stub:				Sp,				// the perpendicular lead out of a port on an orthogonal edge
	pub node_stroke:		f32,			// node outline width, points
	pub edge_stroke:		f32,			// edge line width, points
	pub arrow_len:			f32,			// arrowhead length along the edge, points
	pub arrow_half:			f32,			// arrowhead half-width across the edge, points
	pub node_fill:			Option<Rgba>,	// the interior wash, or None to leave the box unfilled
	pub margin:				f32,			// clear border left around the whole figure, points
}

impl Default for DiagramStyle {
	fn default() -> Self {
		Self {
			label_size:			Sp::from_pt(11.0),
			edge_label_size:	Sp::from_pt(9.5),
			pad_x:				Sp::from_pt(9.0),
			pad_y:				Sp::from_pt(6.0),
			stub:				Sp::from_pt(10.0),
			node_stroke:		1.0,
			edge_stroke:		1.0,
			arrow_len:			7.0,
			arrow_half:			3.0,
			node_fill:			Some(Rgba::new(246, 246, 248, 255)),
			margin:				3.0,
		}
	}
}

/// A flowchart under construction: nodes placed by anchor and offset, and edges joining their ports.
/// Built once, then turned into a [`Graphic`].
#[derive(Clone, Debug, Default)]
pub struct Diagram {
	nodes:	Vec<NodeSpec>,
	edges:	Vec<EdgeSpec>,
}

impl Diagram {
	pub fn new() -> Self {
		Self::default()
	}

	/// Places a node with its centre at an absolute point in the diagram's own frame.
	pub fn node_at<I, L>(&mut self, id: I, label: L, x: Sp, y: Sp, shape: Shape) -> &mut Self
	where
		I: Into<String>,
		L: Into<String>,
	{
		self.nodes.push(NodeSpec {
			id:		id.into(),
			label:	label.into(),
			place:	PlaceSpec::At { x, y },
			shape,
		});
		self
	}

	/// Places a node centred beneath an already-added node, its box top a `gap` below the anchor's box
	/// bottom.
	pub fn node_below<I, L>(&mut self, id: I, label: L, of: &str, gap: Sp, shape: Shape) -> &mut Self
	where
		I: Into<String>,
		L: Into<String>,
	{
		self.nodes.push(NodeSpec {
			id:		id.into(),
			label:	label.into(),
			place:	PlaceSpec::Below { of: of.to_string(), gap },
			shape,
		});
		self
	}

	/// Places a node centred to the right of an already-added node, its box left a `gap` past the
	/// anchor's box right.
	pub fn node_right<I, L>(&mut self, id: I, label: L, of: &str, gap: Sp, shape: Shape) -> &mut Self
	where
		I: Into<String>,
		L: Into<String>,
	{
		self.nodes.push(NodeSpec {
			id:		id.into(),
			label:	label.into(),
			place:	PlaceSpec::Right { of: of.to_string(), gap },
			shape,
		});
		self
	}

	/// Joins two ports with an edge, optionally labelled, routed straight or orthogonally.
	pub fn edge(&mut self, from: Endpoint, to: Endpoint, label: Option<&str>, route: Route) -> &mut Self {
		self.edges.push(EdgeSpec {
			from,
			to,
			label:	label.map(|s| s.to_string()),
			route,
		});
		self
	}

	/// Composes the diagram into a graphic: shape every label, size and place every node, draw the
	/// boxes and the labels, route and arrow every edge, then normalise the whole to the figure origin.
	/// The returned [`Graphic`]'s dimensions are its bounding box, width and height the full extent and
	/// depth zero, so the block layer places it as one box.
	pub fn build(&self, fonts: Arc<FontSet>, style: &DiagramStyle) -> Outcome<Graphic> {
		// Every node id, so an edge or a relative placement can resolve to an index in one pass.
		let mut index: BTreeMap<String, usize> = BTreeMap::new();
		for (i, n) in self.nodes.iter().enumerate() {
			if index.insert(n.id.clone(), i).is_some() {
				return Err(err!(
					"Two nodes share the id \"{}\"; each node needs a distinct id.", n.id;
					Invalid, Input));
			}
		}

		// Shape each label and size its box. A relative placement is resolved to the anchor's index
		// here, which enforces the dependency order: the anchor must already be known.
		let mut labels:	Vec<ShapedText>				= Vec::with_capacity(self.nodes.len());
		let mut specs:	Vec<(Placement, Sp, Sp)>	= Vec::with_capacity(self.nodes.len());
		for (i, n) in self.nodes.iter().enumerate() {
			let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.label_size, &n.label));
			let ld		= shaped.dims();
			let ext		= ld.height + ld.depth;
			let (w, h)	= n.shape.size_for_label(ld.width, ext, style.pad_x, style.pad_y);
			labels.push(shaped);

			let placement = match &n.place {
				PlaceSpec::At { x, y } => Placement::At { x: *x, y: *y },
				PlaceSpec::Below { of, gap } => {
					Placement::Below { of: res!(anchor(&index, &n.id, of, i, "below")), gap: *gap }
				},
				PlaceSpec::Right { of, gap } => {
					Placement::Right { of: res!(anchor(&index, &n.id, of, i, "right of")), gap: *gap }
				},
			};
			specs.push((placement, w, h));
		}
		let rects = res!(layout::place(&specs));

		let mut ops: Vec<DrawOp> = Vec::new();

		// The boxes and their labels. Fill first, so the black outline and the black label sit over the
		// wash rather than under it.
		for (i, n) in self.nodes.iter().enumerate() {
			let r		= &rects[i];
			let outline	= res!(n.shape.outline(r));
			if let Some(fill) = style.node_fill {
				ops.push(DrawOp::Fill { path: outline.clone(), colour: fill });
			}
			ops.push(DrawOp::Stroke { path: outline, colour: Rgba::BLACK, width: style.node_stroke });
			res!(bake_label_centred(&mut ops, &labels[i], r));
		}

		// The edges, each routed once its ports are placed.
		for e in &self.edges {
			res!(self.draw_edge(&mut ops, &index, &rects, fonts.clone(), style, e));
		}

		// Normalise: translate the whole so its bounding box sits at (margin, margin), and report that
		// padded box as the figure's dimensions.
		let bb = res!(ink_bounds(&ops));
		let t = Transform::translate(-bb.x0 + style.margin, -bb.y0 + style.margin);
		let mut placed: Vec<DrawOp> = Vec::with_capacity(ops.len());
		for op in ops {
			placed.push(res!(shift(op, &t)));
		}
		let w = bb.width() + 2.0 * style.margin;
		let h = bb.height() + 2.0 * style.margin;
		Ok(Graphic::new(placed, Dims::new(
			Sp::from_pt(w as f64),
			Sp::from_pt(h as f64),
			Sp::ZERO,
		)))
	}

	/// Draws one edge: resolve its two ends to coordinates and facings, route the polyline, stroke it
	/// short of the arrowhead, fill the arrowhead, and bake any label clear of the longest segment.
	fn draw_edge(
		&self,
		ops:	&mut Vec<DrawOp>,
		index:	&BTreeMap<String, usize>,
		rects:	&[Rect],
		fonts:	Arc<FontSet>,
		style:	&DiagramStyle,
		e:		&EdgeSpec,
	)
		-> Outcome<()>
	{
		let ai = res!(edge_node(index, &e.from.node));
		let bi = res!(edge_node(index, &e.to.node));
		let ra = &rects[ai];
		let rb = &rects[bi];

		// A reference point for each end -- its explicit port, or its centre -- so an end with no port
		// can pick the side facing the other end.
		let a_ref = ref_point(ra, &e.from);
		let b_ref = ref_point(rb, &e.to);
		let a_port = match e.from.port {
			Some(p) => p,
			None    => layout::nearest_port(ra, b_ref),
		};
		let b_port = match e.to.port {
			Some(p) => p,
			None    => layout::nearest_port(rb, a_ref),
		};

		let a_xy = self.nodes[ai].shape.port(ra, a_port);
		let b_xy = self.nodes[bi].shape.port(rb, b_port);
		let pts = layout::route_points(
			a_xy, layout::facing(a_port), b_xy, layout::facing(b_port), e.route, style.stub);

		let n = pts.len();
		if n < 2 {
			return Err(err!(
				"An edge from \"{}\" to \"{}\" routed to fewer than two points.",
				e.from.node, e.to.node; Bug));
		}
		let tip = pts[n - 1];
		let prev = pts[n - 2];

		// Stroke stops at the arrowhead's base, so the line does not run under the tip.
		let mut stroke_pts = pts.clone();
		stroke_pts[n - 1] = layout::retract(tip, prev, style.arrow_len);
		ops.push(DrawOp::Stroke {
			path:	res!(layout::stroke_path(&stroke_pts)),
			colour:	Rgba::BLACK,
			width:	style.edge_stroke,
		});
		ops.push(DrawOp::Fill {
			path:	res!(layout::arrowhead(tip, prev, style.arrow_len, style.arrow_half)),
			colour:	Rgba::BLACK,
		});

		if let Some(text) = &e.label {
			res!(self.bake_edge_label(ops, fonts, style, &pts, text));
		}
		Ok(())
	}

	/// Bakes an edge label, centred on the midpoint of the edge's longest segment and nudged clear of
	/// the line along its perpendicular.
	fn bake_edge_label(
		&self,
		ops:	&mut Vec<DrawOp>,
		fonts:	Arc<FontSet>,
		style:	&DiagramStyle,
		pts:	&[(Sp, Sp)],
		text:	&str,
	)
		-> Outcome<()>
	{
		let shaped	= res!(ShapedText::new(fonts, Role::Italic, Dir::Ltr, style.edge_label_size, text));
		let ld		= shaped.dims();
		let (mid, perp) = match layout::label_anchor(pts) {
			Some(a) => a,
			None    => return Ok(()),	// a zero-length edge carries no label
		};

		// The clear gap is half the label height plus a little, pushed along the perpendicular so the
		// label sits beside a vertical edge and above or below a horizontal one.
		let ext		= ld.height + ld.depth;
		let gap		= Sp(ext.raw() * 3 / 4);
		let off_x	= Sp::from_pt((perp.0 * (gap.to_pt() as f32)) as f64);
		let off_y	= Sp::from_pt((perp.1 * (gap.to_pt() as f32)) as f64);
		let cx		= mid.0 + off_x;
		let cy		= mid.1 + off_y;

		let base_x	= cx - Sp(ld.width.raw() / 2);
		let base_y	= cy + Sp((ld.height.raw() - ld.depth.raw()) / 2);
		bake_label(ops, &shaped, base_x, base_y)
	}
}

/// Resolves a relative placement's anchor to its index, enforcing the single-pass rule: the anchor
/// must exist and must be declared before the node that leans on it.
fn anchor(
	index:	&BTreeMap<String, usize>,
	id:		&str,
	of:		&str,
	i:		usize,
	rel:	&str,
)
	-> Outcome<usize>
{
	let j = res!(index.get(of).ok_or_else(|| err!(
		"Node \"{}\" is placed {} \"{}\", but no such node exists.", id, rel, of;
		Invalid, Input, Missing)));
	if *j >= i {
		return Err(err!(
			"Node \"{}\" is placed {} \"{}\", which is not declared before it; placement is a single \
			forward pass, so a node may lean only on nodes already placed.", id, rel, of;
			Invalid, Input));
	}
	Ok(*j)
}

/// Resolves an edge endpoint's node to its index.
fn edge_node(index: &BTreeMap<String, usize>, id: &str) -> Outcome<usize> {
	Ok(*res!(index.get(id).ok_or_else(|| err!(
		"An edge names node \"{}\", but no such node exists.", id; Invalid, Input, Missing))))
}

/// The reference point of an edge end: its explicit port, or the box centre when the port is left open.
fn ref_point(r: &Rect, end: &Endpoint) -> (Sp, Sp) {
	match end.port {
		Some(p) => shape::port_of(r, p),
		None    => shape::port_of(r, Port::Centre),
	}
}

/// Bakes a shaped run centred within a node's box: horizontally on the centre, vertically on the
/// baseline that seats the text block's middle on the box centre.
fn bake_label_centred(ops: &mut Vec<DrawOp>, shaped: &ShapedText, r: &Rect) -> Outcome<()> {
	let ld		= shaped.dims();
	let base_x	= r.centre_x() - Sp(ld.width.raw() / 2);
	// The block runs from the baseline up by height and down by depth; centring the block on the box
	// centre puts the baseline half a (height - depth) below it.
	let base_y	= r.centre_y() + Sp((ld.height.raw() - ld.depth.raw()) / 2);
	bake_label(ops, shaped, base_x, base_y)
}

/// Bakes a shaped run as filled glyph outlines at a baseline, exactly as the SVG writer draws a line
/// of text: the outline is font-frame and y up, so it is flipped in y and moved onto the baseline at
/// the glyph's own offset. `base_x` is the run's left, `base_y` its baseline, both in the figure frame.
fn bake_label(ops: &mut Vec<DrawOp>, shaped: &ShapedText, base_x: Sp, base_y: Sp) -> Outcome<()> {
	let bx = base_x.to_pt() as f32;
	let by = base_y.to_pt() as f32;
	for glyph in &shaped.run().glyphs {
		let path = res!(shaped.outline(glyph));
		// A glyph with no ink -- a space -- carries an advance but nothing to fill.
		if path.is_empty() {
			continue;
		}
		let t = Transform::scale(1.0, -1.0)
			.then(&Transform::translate(bx + glyph.x, by - glyph.y));
		let placed = res!(path.transform(&t));
		ops.push(DrawOp::Fill { path: placed, colour: Rgba::BLACK });
	}
	Ok(())
}

/// The bounding box of every op's ink, in the provisional frame, from which the figure is normalised.
fn ink_bounds(ops: &[DrawOp]) -> Outcome<Bounds> {
	let mut bb: Option<Bounds> = None;
	for op in ops {
		let path = match op {
			DrawOp::Fill { path, .. }	=> path,
			DrawOp::Stroke { path, .. }	=> path,
		};
		if let Some(b) = path.bounds(&Transform::IDENTITY) {
			bb = Some(match bb {
				None		=> b,
				Some(cur)	=> cur.union(b),
			});
		}
	}
	bb.ok_or_else(|| err!("The diagram produced no ink to bound."; Bug, Missing))
}

/// One op with its path carried into a new frame.
fn shift(op: DrawOp, t: &Transform) -> Outcome<DrawOp> {
	Ok(match op {
		DrawOp::Fill { path, colour } => DrawOp::Fill {
			path:	res!(path.transform(t)),
			colour,
		},
		DrawOp::Stroke { path, colour, width } => DrawOp::Stroke {
			path:	res!(path.transform(t)),
			colour,
			width,
		},
	})
}
