//! Paths: the shapes a rasteriser fills.
//!
//! A path is a sequence of contours, each a run of lines and Bezier curves. Glyph outlines arrive
//! in exactly this form, and so do boxes, rules and borders, so one type serves both.

use crate::transform::Transform;

use oxedyne_fe2o3_core::prelude::*;

/// The default flattening tolerance, in pixels: the furthest a straight segment may stray from the
/// curve it stands in for.
///
/// A tenth of a pixel is below what an eye can resolve at any sane size, and well below what the
/// anti-aliasing can express.
pub const TOLERANCE: f32 = 0.1;

/// The most straight segments a single curve may be flattened into, however cruel its control
/// points. A curve needing more than this has been given nonsense coordinates.
const MAX_STEPS: usize = 1_000;

/// How far along each tangent a control point sits, for a cubic bézier that meets a quarter arc.
///
/// The magic constant `4/3 * (sqrt(2) - 1)`, which makes a bézier hug a quarter circle to about one
/// part in a thousand of the radius. Every arc this module draws is built from it.
const KAPPA: f32 = 0.552_284_75;

/// A point in two dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
	/// Horizontal coordinate.
	pub x:	f32,
	/// Vertical coordinate.
	pub y:	f32,
}

impl Pt {

	/// Creates a point.
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	/// The point midway between this point and another.
	pub fn midpoint(&self, other: Self) -> Self {
		Self::new(0.5 * (self.x + other.x), 0.5 * (self.y + other.y))
	}

	/// The straight-line distance from this point to another.
	pub fn distance(&self, other: Self) -> f32 {
		let dx = other.x - self.x;
		let dy = other.y - self.y;
		(dx * dx + dy * dy).sqrt()
	}

	/// Whether both coordinates are finite, which every point reaching the rasteriser must be.
	pub fn is_finite(&self) -> bool {
		self.x.is_finite() && self.y.is_finite()
	}
}

/// One step of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
	/// Begins a new contour at a point.
	MoveTo(Pt),
	/// A straight line to a point.
	LineTo(Pt),
	/// A quadratic Bezier: one control point, then the end point. TrueType outlines are these.
	QuadTo(Pt, Pt),
	/// A cubic Bezier: two control points, then the end point. PostScript outlines are these.
	CubicTo(Pt, Pt, Pt),
	/// Closes the current contour, returning to where it began.
	Close,
}

/// An axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
	/// Left edge.
	pub x0:	f32,
	/// Top edge.
	pub y0:	f32,
	/// Right edge, exclusive.
	pub x1:	f32,
	/// Bottom edge, exclusive.
	pub y1:	f32,
}

impl Bounds {

	/// Creates a bounding box, ordering the coordinates so that it is never inverted.
	pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
		Self {
			x0: x0.min(x1),
			y0: y0.min(y1),
			x1: x0.max(x1),
			y1: y0.max(y1),
		}
	}

	/// Whether the box encloses nothing.
	pub fn is_empty(&self) -> bool {
		self.x1 <= self.x0 || self.y1 <= self.y0
	}

	/// The overlap of this box with another, which may be empty.
	pub fn intersect(&self, other: Self) -> Self {
		Self {
			x0: self.x0.max(other.x0),
			y0: self.y0.max(other.y0),
			x1: self.x1.min(other.x1),
			y1: self.y1.min(other.y1),
		}
	}

	/// The smallest box holding both this one and another.
	///
	/// The counterpart of [`Bounds::intersect`], and what anything gathering several boxes into the
	/// one that contains them needs: a compositor totalling the damage of a frame, an accessibility
	/// tree giving a rectangle to a node that is made of several runs of text.
	pub fn union(&self, other: Self) -> Self {
		Self {
			x0: self.x0.min(other.x0),
			y0: self.y0.min(other.y0),
			x1: self.x1.max(other.x1),
			y1: self.y1.max(other.y1),
		}
	}

	/// The width of the box, or zero if it is empty.
	pub fn width(&self) -> f32 {
		(self.x1 - self.x0).max(0.0)
	}

	/// The height of the box, or zero if it is empty.
	pub fn height(&self) -> f32 {
		(self.y1 - self.y0).max(0.0)
	}
}

/// A contour after flattening: a polyline, and whether it ran back to where it began.
///
/// A fill can forget whether a contour was closed, since an interior is an interior either way. A
/// stroke cannot: a closed contour is joined all the way round and an open one is capped at both
/// ends, so the two give different ink.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
	/// The points, in order. The closing point of a closed contour is not repeated.
	pub pts:	Vec<Pt>,
	/// Whether the contour closes back onto its first point.
	pub closed:	bool,
}

/// A shape: a sequence of contours built from lines and curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
	segs: Vec<Seg>,
}

impl Path {

	/// The steps of the path.
	pub fn segs(&self) -> &[Seg] {
		&self.segs
	}

	/// Whether the path has no steps, and so paints nothing.
	pub fn is_empty(&self) -> bool {
		self.segs.is_empty()
	}

	/// An axis-aligned rectangle, as a closed path.
	pub fn rect(b: Bounds) -> Outcome<Self> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(b.x0, b.y0));
		pb.line_to(Pt::new(b.x1, b.y0));
		pb.line_to(Pt::new(b.x1, b.y1));
		pb.line_to(Pt::new(b.x0, b.y1));
		pb.close();
		pb.finish()
	}

	/// A circle centred at `(cx, cy)` with radius `r`, as a closed path.
	///
	/// A circle is drawn as four cubic segments, one per quadrant, which is the standard bézier
	/// approximation and is accurate to about one part in a thousand of the radius -- indistinguishable
	/// from a true circle at any size a screen shows. See [`Path::ellipse`], of which this is the case
	/// with equal radii.
	pub fn circle(cx: f32, cy: f32, r: f32) -> Outcome<Self> {
		Self::ellipse(cx, cy, r, r)
	}

	/// An axis-aligned ellipse centred at `(cx, cy)` with radii `rx` and `ry`, as a closed path.
	///
	/// Each quadrant is one cubic bézier whose control points sit `k` of the way along the tangent,
	/// where `k` is the magic constant `4/3 * (sqrt(2) - 1)` that makes a bézier hug a quarter circle.
	/// The contour runs clockwise from the rightmost point, which fills solid under either fill rule.
	pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Outcome<Self> {
		let (kx, ky) = (rx * KAPPA, ry * KAPPA);
		let mut pb = PathBuilder::new();
		// Rightmost point, then clockwise: down to the bottom, left to the leftmost, up to the top.
		pb.move_to(Pt::new(cx + rx, cy));
		pb.cubic_to(Pt::new(cx + rx, cy + ky), Pt::new(cx + kx, cy + ry), Pt::new(cx, cy + ry));
		pb.cubic_to(Pt::new(cx - kx, cy + ry), Pt::new(cx - rx, cy + ky), Pt::new(cx - rx, cy));
		pb.cubic_to(Pt::new(cx - rx, cy - ky), Pt::new(cx - kx, cy - ry), Pt::new(cx, cy - ry));
		pb.cubic_to(Pt::new(cx + kx, cy - ry), Pt::new(cx + rx, cy - ky), Pt::new(cx + rx, cy));
		pb.close();
		pb.finish()
	}

	/// An axis-aligned rectangle with rounded corners, as a closed path.
	///
	/// The radius is clamped to half the shorter side, so a radius larger than the box gives the
	/// stadium or the circle that box inscribes rather than a shape turned inside out. A radius of
	/// zero, or less, is a square corner and returns exactly [`Path::rect`], so a caller that rounds
	/// nothing draws precisely what it drew before rounding existed.
	///
	/// Each corner is one cubic bézier, the same quarter-arc approximation [`Path::ellipse`] uses, and
	/// the contour runs clockwise from the top-left corner's end, matching [`Path::rect`] so the two
	/// fill identically under either fill rule.
	pub fn round_rect(b: Bounds, r: f32) -> Outcome<Self> {
		if !r.is_finite() {
			return Err(err!(
				"A corner radius must be finite, but {} was given.", r; Invalid, Input));
		}
		// A square corner is the rectangle, and is the rectangle's own path: identical, not merely
		// equivalent.
		if r <= 0.0 {
			return Self::rect(b);
		}
		// A corner cannot eat more than half the side it turns, or the two corners of one side would
		// cross and the outline would fold through itself.
		let r = r.min(b.width() * 0.5).min(b.height() * 0.5);
		if r <= 0.0 {
			return Self::rect(b);
		}
		let k = r * KAPPA;
		let mut pb = PathBuilder::new();
		// Clockwise, in a frame whose y falls: along the top, then each corner in turn.
		pb.move_to(Pt::new(b.x0 + r, b.y0));
		pb.line_to(Pt::new(b.x1 - r, b.y0));
		pb.cubic_to(
			Pt::new(b.x1 - r + k,	b.y0),
			Pt::new(b.x1,		b.y0 + r - k),
			Pt::new(b.x1,		b.y0 + r),
		);
		pb.line_to(Pt::new(b.x1, b.y1 - r));
		pb.cubic_to(
			Pt::new(b.x1,		b.y1 - r + k),
			Pt::new(b.x1 - r + k,	b.y1),
			Pt::new(b.x1 - r,	b.y1),
		);
		pb.line_to(Pt::new(b.x0 + r, b.y1));
		pb.cubic_to(
			Pt::new(b.x0 + r - k,	b.y1),
			Pt::new(b.x0,		b.y1 - r + k),
			Pt::new(b.x0,		b.y1 - r),
		);
		pb.line_to(Pt::new(b.x0, b.y0 + r));
		pb.cubic_to(
			Pt::new(b.x0,		b.y0 + r - k),
			Pt::new(b.x0 + r - k,	b.y0),
			Pt::new(b.x0 + r,	b.y0),
		);
		pb.close();
		pb.finish()
	}

	/// The bounding box of the path's points under a transform.
	///
	/// Control points are included, so the box is conservative: it can be larger than the curve,
	/// never smaller, which is what a caller sizing a buffer needs.
	pub fn bounds(&self, t: &Transform) -> Option<Bounds> {
		let mut out: Option<Bounds> = None;
		let mut grow = |p: Pt| {
			let p = t.apply(p);
			out = Some(match out {
				None => Bounds { x0: p.x, y0: p.y, x1: p.x, y1: p.y },
				Some(b) => Bounds {
					x0: b.x0.min(p.x),
					y0: b.y0.min(p.y),
					x1: b.x1.max(p.x),
					y1: b.y1.max(p.y),
				},
			});
		};
		for seg in &self.segs {
			match *seg {
				Seg::MoveTo(p)			=> grow(p),
				Seg::LineTo(p)			=> grow(p),
				Seg::QuadTo(c, p)		=> { grow(c); grow(p); },
				Seg::CubicTo(c0, c1, p)	=> { grow(c0); grow(c1); grow(p); },
				Seg::Close			=> (),
			}
		}
		out
	}

	/// Flattens the path into closed polylines, one per contour, under a transform.
	///
	/// Every contour comes back closed, whether or not the path said [`Seg::Close`], because an
	/// unclosed contour has no interior and the rasteriser fills interiors. The tolerance is in
	/// pixels, and is divided by the transform's scale so that a shape enlarged tenfold is
	/// flattened ten times more finely rather than turning into a polygon.
	pub fn flatten(&self, t: &Transform, tol: f32) -> Vec<Vec<Pt>> {
		let scale = t.scale_factor().max(f32::EPSILON);
		let tol = (tol / scale).max(f32::EPSILON);
		let mut out: Vec<Vec<Pt>> = Vec::new();
		let mut cur: Vec<Pt> = Vec::new();
		let mut pos = Pt::default();
		let mut start = Pt::default();

		for seg in &self.segs {
			match *seg {
				Seg::MoveTo(p) => {
					if cur.len() > 1 {
						out.push(std::mem::take(&mut cur));
					} else {
						cur.clear();
					}
					cur.push(t.apply(p));
					pos = p;
					start = p;
				},
				Seg::LineTo(p) => {
					cur.push(t.apply(p));
					pos = p;
				},
				Seg::QuadTo(c, p) => {
					flatten_quad(&mut cur, t, tol, pos, c, p);
					pos = p;
				},
				Seg::CubicTo(c0, c1, p) => {
					flatten_cubic(&mut cur, t, tol, pos, c0, c1, p);
					pos = p;
				},
				Seg::Close => {
					if cur.len() > 1 {
						out.push(std::mem::take(&mut cur));
					} else {
						cur.clear();
					}
					pos = start;
				},
			}
		}
		if cur.len() > 1 {
			out.push(cur);
		}
		out
	}

	/// Flattens the path into polylines, one per contour, keeping which contours were closed.
	///
	/// This is what a stroker wants, where [`Path::flatten`] is what a filler wants. The two differ
	/// in what they throw away. A filler closes every contour and drops any that is a single point,
	/// since neither an open contour nor a point has an interior to fill. A stroker must keep both:
	/// an open contour takes caps, and a lone point takes a round cap and becomes a dot.
	pub fn flatten_contours(&self, t: &Transform, tol: f32) -> Vec<Polyline> {
		let scale = t.scale_factor().max(f32::EPSILON);
		let tol = (tol / scale).max(f32::EPSILON);
		let mut out: Vec<Polyline> = Vec::new();
		let mut cur: Vec<Pt> = Vec::new();
		let mut pos = Pt::default();
		let mut start = Pt::default();

		// A contour is worth keeping if it has a segment to stroke, or if it was closed on a single
		// point, which is how a path asks for a dot. A bare move_to with nothing after it is not.
		fn flush(cur: &mut Vec<Pt>, closed: bool, out: &mut Vec<Polyline>) {
			if cur.len() > 1 || (closed && !cur.is_empty()) {
				out.push(Polyline { pts: std::mem::take(cur), closed });
			} else {
				cur.clear();
			}
		}

		for seg in &self.segs {
			match *seg {
				Seg::MoveTo(p) => {
					flush(&mut cur, false, &mut out);
					cur.push(t.apply(p));
					pos = p;
					start = p;
				},
				Seg::LineTo(p) => {
					cur.push(t.apply(p));
					pos = p;
				},
				Seg::QuadTo(c, p) => {
					flatten_quad(&mut cur, t, tol, pos, c, p);
					pos = p;
				},
				Seg::CubicTo(c0, c1, p) => {
					flatten_cubic(&mut cur, t, tol, pos, c0, c1, p);
					pos = p;
				},
				Seg::Close => {
					flush(&mut cur, true, &mut out);
					pos = start;
				},
			}
		}
		flush(&mut cur, false, &mut out);
		out
	}
}

/// Flattens a quadratic Bezier, appending the points after the first.
///
/// A straight line drawn between the ends of a quadratic strays from it by at most an eighth of the
/// length of the second difference of its control points, and the error falls with the square of
/// the number of steps, which is what fixes the step count.
fn flatten_quad(out: &mut Vec<Pt>, t: &Transform, tol: f32, p0: Pt, c: Pt, p1: Pt) {
	let dx = p0.x - 2.0 * c.x + p1.x;
	let dy = p0.y - 2.0 * c.y + p1.y;
	let dev = (dx * dx + dy * dy).sqrt();
	let n = steps((dev / (8.0 * tol)).sqrt());
	for i in 1..=n {
		let s = (i as f32) / (n as f32);
		let r = 1.0 - s;
		let p = Pt::new(
			r * r * p0.x + 2.0 * r * s * c.x + s * s * p1.x,
			r * r * p0.y + 2.0 * r * s * c.y + s * s * p1.y,
		);
		out.push(t.apply(p));
	}
}

/// Flattens a cubic Bezier, appending the points after the first.
fn flatten_cubic(out: &mut Vec<Pt>, t: &Transform, tol: f32, p0: Pt, c0: Pt, c1: Pt, p1: Pt) {
	let d0x = p0.x - 2.0 * c0.x + c1.x;
	let d0y = p0.y - 2.0 * c0.y + c1.y;
	let d1x = c0.x - 2.0 * c1.x + p1.x;
	let d1y = c0.y - 2.0 * c1.y + p1.y;
	let dev = (d0x * d0x + d0y * d0y).sqrt().max((d1x * d1x + d1y * d1y).sqrt());
	let n = steps((3.0 * dev / (4.0 * tol)).sqrt());
	for i in 1..=n {
		let s = (i as f32) / (n as f32);
		let r = 1.0 - s;
		let (rr, ss) = (r * r, s * s);
		let p = Pt::new(
			rr * r * p0.x + 3.0 * rr * s * c0.x + 3.0 * r * ss * c1.x + ss * s * p1.x,
			rr * r * p0.y + 3.0 * rr * s * c0.y + 3.0 * r * ss * c1.y + ss * s * p1.y,
		);
		out.push(t.apply(p));
	}
}

/// Turns an ideal step count into a usable one: at least one, never absurd, never a NaN.
fn steps(n: f32) -> usize {
	if !n.is_finite() {
		return MAX_STEPS;
	}
	(n.ceil().max(1.0) as usize).min(MAX_STEPS)
}

/// Builds a [`Path`] one step at a time.
///
/// The builder refuses a path that is not well formed rather than letting the rasteriser meet it:
/// a line before any move, or a point that is not finite.
#[derive(Clone, Debug, Default)]
pub struct PathBuilder {
	segs:	Vec<Seg>,
	open:	bool,
	bad:	Option<String>,
}

impl PathBuilder {

	/// Creates an empty builder.
	pub fn new() -> Self {
		Self::default()
	}

	/// Records the first fault met, so that [`PathBuilder::finish`] can report it. Nothing panics
	/// and nothing is silently dropped.
	fn fault(&mut self, msg: String) {
		if self.bad.is_none() {
			self.bad = Some(msg);
		}
	}

	/// Checks a point, recording a fault if it is not finite.
	fn check(&mut self, p: Pt, what: &str) -> bool {
		if p.is_finite() {
			return true;
		}
		self.fault(fmt!("The {} point ({}, {}) is not finite.", what, p.x, p.y));
		false
	}

	/// Begins a new contour at a point.
	pub fn move_to(&mut self, p: Pt) {
		if self.check(p, "move_to") {
			self.segs.push(Seg::MoveTo(p));
			self.open = true;
		}
	}

	/// Draws a straight line to a point.
	pub fn line_to(&mut self, p: Pt) {
		if !self.open {
			self.fault(fmt!("A line_to at ({}, {}) precedes any move_to.", p.x, p.y));
			return;
		}
		if self.check(p, "line_to") {
			self.segs.push(Seg::LineTo(p));
		}
	}

	/// Draws a quadratic Bezier through one control point to an end point.
	pub fn quad_to(&mut self, c: Pt, p: Pt) {
		if !self.open {
			self.fault(fmt!("A quad_to at ({}, {}) precedes any move_to.", p.x, p.y));
			return;
		}
		if self.check(c, "quad_to control") && self.check(p, "quad_to end") {
			self.segs.push(Seg::QuadTo(c, p));
		}
	}

	/// Draws a cubic Bezier through two control points to an end point.
	pub fn cubic_to(&mut self, c0: Pt, c1: Pt, p: Pt) {
		if !self.open {
			self.fault(fmt!("A cubic_to at ({}, {}) precedes any move_to.", p.x, p.y));
			return;
		}
		if self.check(c0, "cubic_to first control")
			&& self.check(c1, "cubic_to second control")
			&& self.check(p, "cubic_to end")
		{
			self.segs.push(Seg::CubicTo(c0, c1, p));
		}
	}

	/// Closes the current contour.
	pub fn close(&mut self) {
		if self.open {
			self.segs.push(Seg::Close);
			self.open = false;
		}
	}

	/// Finishes the path, or reports the first fault met while building it.
	pub fn finish(self) -> Outcome<Path> {
		match self.bad {
			Some(msg) => Err(err!("{}", msg; Invalid, Input)),
			None => Ok(Path { segs: self.segs }),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_rect_has_four_corners_00() -> Outcome<()> {
		let p = res!(Path::rect(Bounds::new(0.0, 0.0, 10.0, 5.0)));
		let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 1);
		assert_eq!(cs[0].len(), 4);
		Ok(())
	}

	#[test]
	fn test_a_circle_stays_on_its_radius_08() -> Outcome<()> {
		// Every flattened point of a circle must sit close to the radius from the centre: the bézier
		// quadrants approximate the arc to about a part in a thousand, so a tolerance of one percent of
		// the radius is generous and still catches a control point put in the wrong place.
		let (cx, cy, r) = (40.0, 30.0, 20.0);
		let p = res!(Path::circle(cx, cy, r));
		let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 1, "a circle is one contour");
		for pt in &cs[0] {
			let d = ((pt.x - cx).powi(2) + (pt.y - cy).powi(2)).sqrt();
			assert!((d - r).abs() < r * 0.01, "a point at distance {} is off the radius {}", d, r);
		}
		// And its bounding box is the square the radius inscribes.
		let b = match p.bounds(&Transform::IDENTITY) {
			Some(b) => b,
			None => return Err(err!("The circle has no bounds."; Test)),
		};
		assert!((b.x0 - (cx - r)).abs() < 0.01 && (b.x1 - (cx + r)).abs() < 0.01, "width spans 2r");
		assert!((b.y0 - (cy - r)).abs() < 0.01 && (b.y1 - (cy + r)).abs() < 0.01, "height spans 2r");
		Ok(())
	}

	#[test]
	fn test_a_round_rect_of_no_radius_is_the_rectangle_09() -> Outcome<()> {
		// Not "looks the same": IS the same path. A caller that asks for no rounding must be able to
		// rely on getting back exactly what it would have got from Path::rect.
		let b = Bounds::new(3.0, 7.0, 40.0, 25.0);
		assert_eq!(res!(Path::round_rect(b, 0.0)), res!(Path::rect(b)));
		assert_eq!(res!(Path::round_rect(b, -5.0)), res!(Path::rect(b)));
		Ok(())
	}

	#[test]
	fn test_a_round_rect_keeps_its_box_and_rounds_its_corners_10() -> Outcome<()> {
		let (b, r) = (Bounds::new(0.0, 0.0, 60.0, 40.0), 8.0);
		let p = res!(Path::round_rect(b, r));
		// The shape still occupies exactly the box it was given: rounding takes corners away, it does
		// not move edges.
		let bb = match p.bounds(&Transform::IDENTITY) {
			Some(bb) => bb,
			None => return Err(err!("The rounded rectangle has no bounds."; Test)),
		};
		assert!((bb.x0 - b.x0).abs() < 0.01 && (bb.x1 - b.x1).abs() < 0.01, "the width is the box's");
		assert!((bb.y0 - b.y0).abs() < 0.01 && (bb.y1 - b.y1).abs() < 0.01, "the height is the box's");

		// And the corner itself is gone: no point of the outline lies in the square the radius cuts off
		// at the top-left, beyond the arc's own centre distance.
		let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 1, "a rounded rectangle is one contour");
		let (cx, cy) = (b.x0 + r, b.y0 + r); // The top-left corner's arc centre.
		for pt in &cs[0] {
			if pt.x < cx && pt.y < cy {
				let d = ((pt.x - cx).powi(2) + (pt.y - cy).powi(2)).sqrt();
				assert!(
					(d - r).abs() < r * 0.01,
					"the point ({}, {}) is inside the corner square at distance {} from the arc \
					centre, which is not on the radius {}", pt.x, pt.y, d, r,
				);
			}
		}
		Ok(())
	}

	#[test]
	fn test_a_radius_larger_than_the_box_is_clamped_11() -> Outcome<()> {
		// A radius of half the shorter side is the most a box can take. Beyond that the corners would
		// cross, so the radius is clamped and the shape stays inside its box.
		let b = Bounds::new(0.0, 0.0, 40.0, 20.0);
		let p = res!(Path::round_rect(b, 500.0));
		let bb = match p.bounds(&Transform::IDENTITY) {
			Some(bb) => bb,
			None => return Err(err!("The clamped rounded rectangle has no bounds."; Test)),
		};
		assert!(bb.x0 >= b.x0 - 0.01 && bb.x1 <= b.x1 + 0.01, "a clamped radius stays in its box");
		assert!(bb.y0 >= b.y0 - 0.01 && bb.y1 <= b.y1 + 0.01, "in both axes");
		// Half the shorter side: a stadium, whose ends are semicircles of the box's half-height.
		assert_eq!(p, res!(Path::round_rect(b, b.height() * 0.5)), "the radius clamps to half the side");
		Ok(())
	}

	#[test]
	fn test_line_before_move_is_rejected_01() {
		let mut pb = PathBuilder::new();
		pb.line_to(Pt::new(1.0, 1.0));
		assert!(pb.finish().is_err());
	}

	#[test]
	fn test_infinite_point_is_rejected_02() {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(f32::INFINITY, 0.0));
		assert!(pb.finish().is_err());
	}

	#[test]
	fn test_curve_flattens_more_finely_when_scaled_03() -> Outcome<()> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(0.0, 0.0));
		pb.quad_to(Pt::new(50.0, 100.0), Pt::new(100.0, 0.0));
		pb.close();
		let p = res!(pb.finish());
		let small = p.flatten(&Transform::IDENTITY, TOLERANCE);
		let big = p.flatten(&Transform::scale(10.0, 10.0), TOLERANCE);
		assert!(
			big[0].len() > small[0].len(),
			"a tenfold enlargement should need more segments, found {} then {}",
			small[0].len(), big[0].len(),
		);
		Ok(())
	}

	#[test]
	fn test_unclosed_contour_still_flattens_04() -> Outcome<()> {
		// An unclosed contour has an interior all the same; the rasteriser closes it.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(0.0, 0.0));
		pb.line_to(Pt::new(10.0, 0.0));
		pb.line_to(Pt::new(10.0, 10.0));
		let p = res!(pb.finish());
		let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 1);
		assert_eq!(cs[0].len(), 3);
		Ok(())
	}

	#[test]
	fn test_flatten_contours_remembers_what_was_closed_06() -> Outcome<()> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(0.0, 0.0));
		pb.line_to(Pt::new(10.0, 0.0));
		pb.line_to(Pt::new(10.0, 10.0));
		pb.close();
		pb.move_to(Pt::new(20.0, 0.0));
		pb.line_to(Pt::new(30.0, 0.0));
		let p = res!(pb.finish());
		let cs = p.flatten_contours(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 2);
		assert!(cs[0].closed, "the first contour was closed");
		assert_eq!(cs[0].pts.len(), 3, "and its closing point is not repeated");
		assert!(!cs[1].closed, "the second was left open");
		// The filler throws the distinction away, and still sees two contours.
		assert_eq!(p.flatten(&Transform::IDENTITY, TOLERANCE).len(), 2);
		Ok(())
	}

	#[test]
	fn test_a_move_closed_on_itself_is_a_point_but_a_lone_move_is_nothing_07() -> Outcome<()> {
		// A stroker needs both of these, and they differ: a path may ask for a dot, and a path may
		// pick the pen up and put it down again without asking for anything.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(5.0, 5.0));
		pb.close();
		let dot = res!(pb.finish());
		let cs = dot.flatten_contours(&Transform::IDENTITY, TOLERANCE);
		assert_eq!(cs.len(), 1, "a move closed on itself is a contour of one point");
		assert_eq!(cs[0].pts.len(), 1);
		assert!(cs[0].closed);

		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(5.0, 5.0));
		let lone = res!(pb.finish());
		assert!(lone.flatten_contours(&Transform::IDENTITY, TOLERANCE).is_empty());
		// The filler drops both, since a point has no interior.
		assert!(dot.flatten(&Transform::IDENTITY, TOLERANCE).is_empty());
		Ok(())
	}

	#[test]
	fn test_bounds_are_conservative_05() -> Outcome<()> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(0.0, 0.0));
		pb.quad_to(Pt::new(50.0, 100.0), Pt::new(100.0, 0.0));
		let p = res!(pb.finish());
		let b = match p.bounds(&Transform::IDENTITY) {
			Some(b) => b,
			None => return Err(err!("The path has points, so it must have bounds."; Bug)),
		};
		// The curve only reaches y = 50, but the control point at y = 100 is counted.
		assert_eq!(b.y1, 100.0);
		Ok(())
	}
}
