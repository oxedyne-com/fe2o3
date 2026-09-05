//! Function plots as a [`Graphic`](crate::ir::Graphic): axes, a light grid, numbered ticks and one or
//! more sampled curves, drawn in the same `fe2o3_graphics` paths the body text and diagrams use, so a
//! plot is first-class content -- stroked geometry and shaped tick labels, not a pasted raster. A
//! caller samples a function into a [`Series`] and places the result with [`Block::figure`](crate::doc).
//!
//! The frame is fixed points: a left and bottom margin hold the tick labels, the rest is the plot area.
//! Data coordinates map into that area with y flipped, since the page runs y downwards. Clipping a curve
//! to the area is a later refinement; a caller supplies samples within the ranges it declares.

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
	path::{
		PathBuilder,
		Pt,
	},
	transform::Transform,
};

use std::sync::Arc;

/// One curve: its points in data coordinates, its pen colour and its stroke width in points.
pub struct Series {
	pub points:	Vec<(f64, f64)>,
	pub colour:	Rgba,
	pub width:	f32,
}

/// A plot to be built into a figure. The ranges fix the data window; the ticks are where a grid line,
/// a mark and a numeric label are drawn on each axis.
pub struct Plot {
	pub width:		f32,		// overall figure width, points
	pub height:		f32,		// overall figure height, points
	pub x_range:	(f64, f64),
	pub y_range:	(f64, f64),
	pub x_ticks:	Vec<f64>,
	pub y_ticks:	Vec<f64>,
	pub series:		Vec<Series>,
}

impl Plot {
	/// Builds the plot into a [`Graphic`] sized to `width` x `height`, its ink drawn from the top-left of
	/// that box in page coordinates so the figure placement can treat it like any other drawn block.
	pub fn build(&self, fonts: Arc<FontSet>) -> Outcome<Graphic> {
		let ml = 34.0_f32;	// left margin, for the y tick labels
		let mb = 22.0_f32;	// bottom margin, for the x tick labels
		let mt = 10.0_f32;	// top and right margins, a little breathing room
		let mr = 12.0_f32;
		let pw = self.width - ml - mr;
		let ph = self.height - mt - mb;

		let (x0, x1) = self.x_range;
		let (y0, y1) = self.y_range;
		let sx = |x: f64| -> f32 { ml + ((x - x0) / (x1 - x0)) as f32 * pw };
		let sy = |y: f64| -> f32 { mt + (1.0 - ((y - y0) / (y1 - y0)) as f32) * ph };

		let grid	= Rgba::opaque(225, 225, 225);
		let axis	= Rgba::opaque(120, 120, 120);
		let frame	= Rgba::opaque(90, 90, 90);

		let mut ops: Vec<DrawOp> = Vec::new();

		// The grid: a faint line at each tick, right across the plot area.
		for &xt in &self.x_ticks {
			let px = sx(xt);
			ops.push(res!(seg(px, mt, px, mt + ph, grid, 0.4)));
		}
		for &yt in &self.y_ticks {
			let py = sy(yt);
			ops.push(res!(seg(ml, py, ml + pw, py, grid, 0.4)));
		}

		// The zero axes, drawn darker when zero falls inside the range.
		if y0 < 0.0 && y1 > 0.0 {
			let py = sy(0.0);
			ops.push(res!(seg(ml, py, ml + pw, py, axis, 0.6)));
		}
		if x0 < 0.0 && x1 > 0.0 {
			let px = sx(0.0);
			ops.push(res!(seg(px, mt, px, mt + ph, axis, 0.6)));
		}

		// The curves, each a stroked polyline through its mapped samples.
		for s in &self.series {
			let mut pb = PathBuilder::new();
			for (k, (x, y)) in s.points.iter().enumerate() {
				let p = Pt::new(sx(*x), sy(*y));
				if k == 0 {
					pb.move_to(p);
				} else {
					pb.line_to(p);
				}
			}
			let path = res!(pb.finish());
			ops.push(DrawOp::Stroke { path, colour: s.colour, width: s.width });
		}

		// The frame around the plot area, over the grid and under the labels.
		ops.push(res!(seg(ml, mt, ml + pw, mt, frame, 0.6)));
		ops.push(res!(seg(ml, mt + ph, ml + pw, mt + ph, frame, 0.6)));
		ops.push(res!(seg(ml, mt, ml, mt + ph, frame, 0.6)));
		ops.push(res!(seg(ml + pw, mt, ml + pw, mt + ph, frame, 0.6)));

		// The tick labels, shaped small and baked to glyph outlines like any other run.
		let size = Sp::from_pt(8.0);
		for &xt in &self.x_ticks {
			let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, &fmt_tick(xt)));
			let w		= shaped.dims().width.to_pt() as f32;
			let asc		= shaped.dims().height.to_pt() as f32;
			res!(bake(&mut ops, &shaped, sx(xt) - w / 2.0, mt + ph + 3.0 + asc));
		}
		for &yt in &self.y_ticks {
			let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, &fmt_tick(yt)));
			let w		= shaped.dims().width.to_pt() as f32;
			let asc		= shaped.dims().height.to_pt() as f32;
			res!(bake(&mut ops, &shaped, ml - 4.0 - w, sy(yt) + asc / 2.0));
		}

		Ok(Graphic {
			ops,
			dims: Dims::new(Sp::from_pt(self.width as f64), Sp::from_pt(self.height as f64), Sp::ZERO),
		})
	}
}

/// A straight stroked segment between two points in figure coordinates.
fn seg(x0: f32, y0: f32, x1: f32, y1: f32, colour: Rgba, width: f32) -> Outcome<DrawOp> {
	let mut pb = PathBuilder::new();
	pb.move_to(Pt::new(x0, y0));
	pb.line_to(Pt::new(x1, y1));
	Ok(DrawOp::Stroke { path: res!(pb.finish()), colour, width })
}

/// Bakes a shaped run as filled glyph outlines at a baseline, flipping the font-frame y-up outline onto
/// the page's y-down frame, exactly as the diagram labels and the SVG writer do.
fn bake(ops: &mut Vec<DrawOp>, shaped: &ShapedText, base_x: f32, base_y: f32) -> Outcome<()> {
	for glyph in &shaped.run().glyphs {
		let path = res!(shaped.outline(glyph));
		if path.is_empty() {
			continue;
		}
		let t = Transform::scale(1.0, -1.0)
			.then(&Transform::translate(base_x + glyph.x, base_y - glyph.y));
		ops.push(DrawOp::Fill { path: res!(path.transform(&t)), colour: Rgba::BLACK });
	}
	Ok(())
}

/// Formats a tick value compactly: an integer without a decimal point, otherwise to two places with the
/// trailing zeros and any bare point trimmed, so 0.50 shows as 0.5 and 2.0 as 2.
fn fmt_tick(v: f64) -> String {
	if (v - v.round()).abs() < 1e-9 {
		return fmt!("{}", v.round() as i64);
	}
	let s = fmt!("{:.2}", v);
	let s = s.trim_end_matches('0');
	s.trim_end_matches('.').to_string()
}
