//! Stroking: the ink a pen leaves as it travels a path.
//!
//! # A stroke is a fill
//!
//! Stroking is not a second kind of painting that needs a second rasteriser. The ink a pen leaves
//! is a region of the plane like any other, so the whole job here is to build that region as a
//! [`Path`], hand it to the filler, and add no code at all to the rasteriser. Everything the
//! rasteriser already knows -- the analytic anti-aliasing, the clipping, the compositing -- comes
//! for nothing.
//!
//! # How the region is built
//!
//! The tempting way is to offset the path to one side, offset it to the other, and sew the two
//! offsets into a single outline. That way lies grief. On the inside of a turn tighter than the pen
//! is wide the two offsets cross, and the outline ties itself into knots that only a
//! boolean-geometry engine can untie.
//!
//! So the region is built instead as a heap of convex pieces: a quadrilateral for each straight run
//! of the pen, a wedge or a triangle at each corner it turns, a cap at each loose end. Every piece
//! is wound the same way, by [`piece`], and that is the whole trick. Wound alike they add and never
//! cancel, so under the non-zero rule their union is exactly the ink -- knots, overlaps, hairpins
//! and all. It is also why the path [`Path::stroke`] returns must be filled with
//! [`crate::raster::FillRule::NonZero`]: fill it even-odd and every place two pieces overlap would
//! come out as a hole.
//!
//! # Why the pieces meet rather than overlap
//!
//! Where two pieces can be cut to share an edge, they are. A bevel or a miter join meets the two
//! runs it joins along the pen's end edge; a round join is a wedge of a disc rather than the whole
//! disc, and a round cap a half disc rather than a whole one.
//!
//! This is not tidiness. Because the rasteriser accumulates area rather than compositing coverage,
//! two pieces that meet edge to edge sum to exactly one across the seam, and the seam cannot be
//! seen. Two pieces that lie over each other along the union's own boundary would sum to two there,
//! and a pixel half covered would come out fully inked: a bright bead at every join. Cutting the
//! pieces to meet is what buys clean edges, and it costs nothing.

use crate::path::{
	Path,
	PathBuilder,
	Polyline,
	Pt,
	TOLERANCE,
};
use crate::transform::Transform;

use oxedyne_fe2o3_core::prelude::*;

/// The most straight segments one round join or cap may be flattened into, however large the pen.
const MAX_ARC_STEPS: usize = 256;

/// The most dashes one contour may be cut into: a ceiling against a pattern so fine, on a path so
/// long, that the outline would swallow the memory of the machine.
pub const MAX_DASHES: usize = 1 << 16;

/// Below this a cross product counts as zero and two directions as parallel. Both are unit vectors,
/// so this is the sine of the angle between them, and an angle this small bends nothing a pixel can
/// show.
const EPS_TURN: f32 = 1e-5;

/// Below this two points count as one, and a run of pen between them as having no length and so no
/// direction to be offset along.
const EPS_LEN: f32 = 1e-6;

/// The default miter limit, as SVG and PostScript both have it: a corner whose miter would reach
/// more than four line widths past it is bevelled instead.
pub const MITER_LIMIT: f32 = 4.0;

/// How a stroke finishes at the loose end of an open contour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cap {
	/// Stops dead on the end point. A contour with no length is left undrawn, since a butt cap
	/// reaches nowhere and there is nothing else to reach.
	#[default]
	Butt,
	/// A half disc past the end point, so a contour with no length comes out as a dot.
	Round,
	/// A half square past the end point, reaching out by half the line width.
	Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Join {
	/// Carries the two outer edges on until they meet in a point, unless that point would reach
	/// further past the corner than the miter limit allows, in which case the corner is bevelled.
	#[default]
	Miter,
	/// A wedge of a disc, rounding the corner off.
	Round,
	/// A straight cut across the corner, from one outer edge to the other.
	Bevel,
}

/// A dash pattern: alternating lengths of ink and gap, walked round and round along the contour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dash {
	/// Lengths of ink and gap in turn, beginning with ink. A pattern of odd length is walked twice
	/// over, so that ink and gap trade places on the second pass and the pattern only truly repeats
	/// after both, which is the rule SVG and PostScript share.
	pub pattern:	Vec<f32>,
	/// How far into the pattern the contour's first point already stands.
	pub offset:	f32,
}

impl Dash {

	/// Creates a dash pattern that begins at the start of its first length of ink.
	pub fn new(pattern: Vec<f32>) -> Self {
		Self { pattern, offset: 0.0 }
	}

	/// Sets how far into the pattern the contour begins.
	pub fn with_offset(mut self, offset: f32) -> Self {
		self.offset = offset;
		self
	}
}

/// The pen: everything that decides what ink a path leaves.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
	/// The width of the pen, in the coordinates the path is expressed in. Must be positive.
	pub width:	f32,
	/// How the loose ends of an open contour are finished. A closed contour has none.
	pub cap:	Cap,
	/// How the corners are turned.
	pub join:	Join,
	/// The furthest a miter may reach past a corner, as a multiple of the line width. A corner
	/// sharper than this is bevelled instead, which is what stops a path that nearly doubles back
	/// from throwing a spike clear across the page. Must be at least one, since even a right angle
	/// mitres to more than one line width.
	pub miter_limit: f32,
	/// The dash pattern, if the line is to be broken.
	pub dash:	Option<Dash>,
	/// The flattening tolerance, in the coordinates the path is expressed in: the furthest a
	/// straight segment may stray from the curve or the arc it stands in for. A caller who will
	/// then scale the stroked path up tenfold should divide this by ten, as [`Path::flatten`] does
	/// with its own tolerance, and as [`crate::pixmap::Pixmap::stroke_path`] does on the caller's
	/// behalf.
	pub tol:	f32,
}

impl Default for Stroke {
	fn default() -> Self {
		Self {
			width:		1.0,
			cap:		Cap::default(),
			join:		Join::default(),
			miter_limit:	MITER_LIMIT,
			dash:		None,
			tol:		TOLERANCE,
		}
	}
}

impl Stroke {

	/// Creates a pen of the given width, refusing a width that cannot draw.
	pub fn new(width: f32) -> Outcome<Self> {
		let s = Self { width, ..Self::default() };
		res!(s.check());
		Ok(s)
	}

	/// Sets how the loose ends are finished.
	pub fn with_cap(mut self, cap: Cap) -> Self {
		self.cap = cap;
		self
	}

	/// Sets how the corners are turned.
	pub fn with_join(mut self, join: Join) -> Self {
		self.join = join;
		self
	}

	/// Sets the furthest a miter may reach past a corner, as a multiple of the line width.
	pub fn with_miter_limit(mut self, limit: f32) -> Self {
		self.miter_limit = limit;
		self
	}

	/// Sets the dash pattern.
	pub fn with_dash(mut self, dash: Dash) -> Self {
		self.dash = Some(dash);
		self
	}

	/// Sets the flattening tolerance.
	pub fn with_tolerance(mut self, tol: f32) -> Self {
		self.tol = tol;
		self
	}

	/// Refuses a pen that cannot draw.
	///
	/// The fields are public, so a pen can be assembled without passing through [`Stroke::new`].
	/// This is where every pen is checked all the same, once, at the moment it is asked to draw.
	pub fn check(&self) -> Outcome<()> {
		if !self.width.is_finite() || self.width <= 0.0 {
			return Err(err!(
				"A stroke width must be positive and finite, but {} was given.", self.width;
			Invalid, Input));
		}
		if !self.miter_limit.is_finite() || self.miter_limit < 1.0 {
			return Err(err!(
				"A miter limit must be at least one, since a miter reaches at least one line width \
				past even a right angle, but {} was given.", self.miter_limit;
			Invalid, Input, Range));
		}
		if !self.tol.is_finite() || self.tol <= 0.0 {
			return Err(err!(
				"A flattening tolerance must be positive and finite, but {} was given.", self.tol;
			Invalid, Input));
		}
		if let Some(d) = &self.dash {
			if d.pattern.is_empty() {
				return Err(err!(
					"A dash pattern must name at least one length of ink.";
				Invalid, Input, Missing));
			}
			if !d.offset.is_finite() {
				return Err(err!(
					"A dash offset must be finite, but {} was given.", d.offset;
				Invalid, Input));
			}
			let mut total = 0.0f32;
			for (i, len) in d.pattern.iter().enumerate() {
				if !len.is_finite() || *len < 0.0 {
					return Err(err!(
						"A dash length must be finite and no less than zero, but the one at {} is \
						{}.", i, len;
					Invalid, Input));
				}
				total += *len;
			}
			if total <= 0.0 {
				return Err(err!(
					"A dash pattern of {} lengths that are all zero never turns the ink on.",
					d.pattern.len();
				Invalid, Input));
			}
		}
		Ok(())
	}
}

impl Path {

	/// Strokes the path with a pen, returning the ink it leaves as a new path.
	///
	/// The result is a union of convex pieces all wound the same way, so it must be filled under
	/// [`crate::raster::FillRule::NonZero`] -- which is the default, and what
	/// [`crate::pixmap::Pixmap::fill_path`] uses. Filling it even-odd would open a hole wherever two
	/// pieces overlap.
	///
	/// # Errors
	///
	/// A pen that cannot draw is refused: see [`Stroke::check`]. A dash pattern fine enough to cut a
	/// contour into more than [`MAX_DASHES`] runs is refused too.
	pub fn stroke(&self, pen: &Stroke) -> Outcome<Self> {
		res!(pen.check());
		let r = 0.5 * pen.width; // Half the width: how far the pen reaches to either side.
		let mut pb = PathBuilder::new();
		for pl in self.flatten_contours(&Transform::IDENTITY, pen.tol) {
			match &pen.dash {
				None => stroke_contour(&mut pb, &pl, pen, r),
				Some(d) => {
					for run in res!(dash(&pl, d)) {
						stroke_contour(&mut pb, &run, pen, r);
					}
				},
			}
		}
		pb.finish()
	}
}

/// Strokes one flattened contour into the outline being built.
fn stroke_contour(pb: &mut PathBuilder, pl: &Polyline, pen: &Stroke, r: f32) {
	let pts = dedup(&pl.pts, pl.closed);
	if pts.is_empty() {
		return;
	}
	if pts.len() == 1 {
		// A contour with no length. It still leaves a mark, if the cap reaches anywhere.
		point_cap(pb, pts[0], pen.cap, r, pen.tol);
		return;
	}
	let n = pts.len();
	// The runs of pen: one for each edge, and for a closed contour the edge back to the start too.
	let edges = if pl.closed { n } else { n - 1 };
	let mut dirs: Vec<Pt> = Vec::with_capacity(edges);
	for i in 0..edges {
		match dir(pts[i], pts[(i + 1) % n]) {
			Some(d) => dirs.push(d),
			// Unreachable after `dedup`, but no offset here may ever divide by a zero length.
			None => return,
		}
	}

	// Each straight run of the pen, as a quadrilateral.
	for i in 0..edges {
		let (a, b) = (pts[i], pts[(i + 1) % n]);
		let nv = mul(left(dirs[i]), r);
		piece(pb, &[add(a, nv), add(b, nv), sub(b, nv), sub(a, nv)]);
	}

	if pl.closed {
		// A closed contour turns a corner at every point, its first included, and has no ends.
		for i in 0..edges {
			let prev = (i + edges - 1) % edges;
			join(pb, pts[i], dirs[prev], dirs[i], pen, r);
		}
	} else {
		for i in 1..(n - 1) {
			join(pb, pts[i], dirs[i - 1], dirs[i], pen, r);
		}
		// The two loose ends. The pen leaves the first one travelling backwards.
		end_cap(pb, pts[0], mul(dirs[0], -1.0), pen.cap, r, pen.tol);
		end_cap(pb, pts[n - 1], dirs[edges - 1], pen.cap, r, pen.tol);
	}
}

/// Adds the piece that fills the corner at `v`, where the pen turns from direction `d0` to `d1`.
fn join(pb: &mut PathBuilder, v: Pt, d0: Pt, d1: Pt, pen: &Stroke, r: f32) {
	let cross = d0.x * d1.y - d0.y * d1.x;
	let dot = d0.x * d1.x + d0.y * d1.y;

	if cross.abs() < EPS_TURN {
		if dot > 0.0 {
			return; // Straight on. There is no corner here to fill.
		}
		// A hairpin: the pen doubles back along itself, and the corner is the whole half disc past
		// `v`. A miter here would reach to infinity and so is always over its limit, and the bevel
		// it falls back to is a triangle with no area, so only a round join leaves anything at all.
		if let Join::Round = pen.join {
			piece(pb, &round_cap(v, d0, r, pen.tol));
		}
		return;
	}

	// The outside of the turn is the side the pen sweeps the long way round: the right hand turning
	// one way, the left hand turning the other.
	let outer = if cross > 0.0 { right } else { left };
	let n0 = mul(outer(d0), r);
	let n1 = mul(outer(d1), r);
	// The signed angle the pen turns through, which is also the angle from `n0` to `n1`, a normal
	// being nothing but its direction under a quarter turn.
	let phi = cross.atan2(dot);

	match pen.join {
		Join::Bevel => piece(pb, &[v, add(v, n0), add(v, n1)]),
		Join::Round => {
			// A wedge of the disc, not the disc: its two straight edges are the end edges of the
			// two runs of pen it sits between, so it meets them instead of lying over them.
			let steps = arc_steps(r, phi, pen.tol);
			let step = phi / (steps as f32);
			let mut pts = Vec::with_capacity(steps + 2);
			pts.push(v);
			for k in 0..=steps {
				pts.push(add(v, rot(n0, (k as f32) * step)));
			}
			piece(pb, &pts);
		},
		Join::Miter => {
			// The miter reaches 1 / cos(phi / 2) line widths past the corner, which runs away to
			// nothing as the corner sharpens towards a hairpin. The limit is what keeps a needle
			// from becoming a spear.
			let c = (0.5 * phi).cos();
			let reach = if c > f32::EPSILON { 1.0 / c } else { f32::INFINITY };
			let bisect = add(n0, n1); // Two normals of a length bisect the angle between them.
			let len = (bisect.x * bisect.x + bisect.y * bisect.y).sqrt();
			if !reach.is_finite() || reach > pen.miter_limit || len <= EPS_LEN {
				piece(pb, &[v, add(v, n0), add(v, n1)]); // Over the limit: bevel it instead.
			} else {
				let m = add(v, mul(bisect, r * reach / len));
				piece(pb, &[v, add(v, n0), m, add(v, n1)]);
			}
		},
	}
}

/// Adds the piece that finishes a loose end at `e`, which the pen reached travelling in `d`.
fn end_cap(pb: &mut PathBuilder, e: Pt, d: Pt, cap: Cap, r: f32, tol: f32) {
	match cap {
		Cap::Butt	=> (),
		Cap::Round	=> piece(pb, &round_cap(e, d, r, tol)),
		Cap::Square	=> piece(pb, &square_cap(e, d, r)),
	}
}

/// Adds the mark a contour with no length leaves: a dot under a round cap, a square under a square
/// cap, and nothing at all under a butt cap, which reaches nowhere.
fn point_cap(pb: &mut PathBuilder, e: Pt, cap: Cap, r: f32, tol: f32) {
	match cap {
		Cap::Butt => (),
		Cap::Round => {
			// A whole disc, since there is no direction here to take half of.
			let steps = arc_steps(r, std::f32::consts::TAU, tol).max(3);
			let step = std::f32::consts::TAU / (steps as f32);
			let pts: Vec<Pt> = (0..steps)
				.map(|k| add(e, rot(Pt::new(r, 0.0), (k as f32) * step)))
				.collect();
			piece(pb, &pts);
		},
		Cap::Square => piece(pb, &[
			Pt::new(e.x - r, e.y - r),
			Pt::new(e.x + r, e.y - r),
			Pt::new(e.x + r, e.y + r),
			Pt::new(e.x - r, e.y + r),
		]),
	}
}

/// The points of a round cap: a half disc past `e`, bulging the way `d` points.
///
/// The straight edge of the half disc runs from one side of the line to the other, which is exactly
/// the end edge of the run of pen reaching `e`, so cap and run meet rather than overlap.
fn round_cap(e: Pt, d: Pt, r: f32, tol: f32) -> Vec<Pt> {
	let n = mul(left(d), r);
	// The left normal leads the direction by a quarter turn, so sweeping back by half a turn from
	// it passes through the direction itself, which is the way the cap must bulge. Sweeping forward
	// would put the cap behind the pen, inside the ink, where it would do nothing.
	let steps = arc_steps(r, std::f32::consts::PI, tol);
	let step = -std::f32::consts::PI / (steps as f32);
	(0..=steps).map(|k| add(e, rot(n, (k as f32) * step))).collect()
}

/// The points of a square cap: a half square past `e`, reaching out by `r` the way `d` points.
fn square_cap(e: Pt, d: Pt, r: f32) -> Vec<Pt> {
	let n = mul(left(d), r);
	let out = mul(d, r);
	let (a, b) = (add(e, n), sub(e, n));
	vec![a, add(a, out), add(b, out), b]
}

/// Adds one convex piece of the outline, wound the same way as every other piece.
///
/// The winding is settled here, by the sign of the shoelace area, and nowhere else. The union only
/// holds if nothing cancels: two pieces wound against each other would subtract where they overlap
/// and eat a hole out of the middle of a perfectly good stroke.
fn piece(pb: &mut PathBuilder, pts: &[Pt]) {
	if pts.len() < 3 {
		return; // Nothing with an interior.
	}
	let mut area = 0.0f32;
	for i in 0..pts.len() {
		let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
		area += a.x * b.y - b.x * a.y;
	}
	if area >= 0.0 {
		pb.move_to(pts[0]);
		for p in &pts[1..] {
			pb.line_to(*p);
		}
	} else {
		pb.move_to(pts[pts.len() - 1]);
		for p in pts[..pts.len() - 1].iter().rev() {
			pb.line_to(*p);
		}
	}
	pb.close();
}

/// Cuts a contour into the runs of ink a dash pattern leaves along it.
fn dash(pl: &Polyline, d: &Dash) -> Outcome<Vec<Polyline>> {
	// An odd pattern is walked twice, so that ink and gap trade places on the second pass.
	let mut pat = d.pattern.clone();
	if pat.len() % 2 == 1 {
		pat.extend_from_within(..);
	}
	let total: f32 = pat.iter().sum();
	let pts = &pl.pts;
	if pts.len() < 2 || total <= 0.0 {
		return Ok(vec![pl.clone()]);
	}

	// Where in the pattern the contour's first point already stands.
	let mut phase = d.offset % total;
	if phase < 0.0 {
		phase += total;
	}
	let mut i = 0usize;
	// The phase is less than the total, so it is spent before the pattern runs out.
	for _ in 0..pat.len() {
		if phase < pat[i] {
			break;
		}
		phase -= pat[i];
		i = (i + 1) % pat.len();
	}
	let mut on = i % 2 == 0; // Even lengths are ink, odd ones gap.
	let mut rest = pat[i] - phase; // How much of the current length has yet to run.

	let began_on = on;
	let n = pts.len();
	let edges = if pl.closed { n } else { n - 1 };
	let mut out: Vec<Polyline> = Vec::new();
	let mut cur: Vec<Pt> = if on { vec![pts[0]] } else { Vec::new() };

	for e in 0..edges {
		let (a, b) = (pts[e], pts[(e + 1) % n]);
		let len = a.distance(b);
		if !len.is_finite() || len <= EPS_LEN {
			continue; // Nothing to walk along, and nothing to divide by.
		}
		let mut t = 0.0f32; // How far along this edge the walk has come.
		while rest < len - t {
			t += rest;
			let p = lerp(a, b, t / len);
			if on {
				cur.push(p);
				out.push(Polyline { pts: std::mem::take(&mut cur), closed: false });
				if out.len() > MAX_DASHES {
					return Err(err!(
						"A dash pattern of total length {} cuts this contour into more than {} \
						runs of ink.", total, MAX_DASHES;
					Invalid, Input, Excessive));
				}
			} else {
				cur.clear();
				cur.push(p);
			}
			on = !on;
			i = (i + 1) % pat.len();
			rest = pat[i];
		}
		rest -= len - t;
		if on {
			cur.push(b);
		}
	}

	// Whatever the walk was still laying down when it ran out of contour.
	if on && !cur.is_empty() {
		if out.is_empty() {
			// The pattern never turned off, so the contour survives whole, and closed if it began
			// so: a dash long enough to swallow a ring leaves a ring, not a ring cut open.
			out.push(Polyline { pts: cur, closed: pl.closed });
		} else if pl.closed && began_on {
			// The walk began and ended inside the same length of ink, on either side of the
			// contour's first point. They are one run, and must be sewn back into one, or the
			// corner there would come out capped twice over instead of joined.
			let head = std::mem::take(&mut out[0].pts);
			let mut sewn = cur;
			sewn.extend_from_slice(&head[1..]);
			out[0].pts = sewn;
		} else {
			out.push(Polyline { pts: cur, closed: false });
		}
	}
	Ok(out)
}

/// Drops each point that repeats the one before it, and the closing point of a closed contour that
/// names its first point twice.
///
/// A run of pen with no length has no direction, and a direction is what every offset, every join
/// and every cap here is built from.
fn dedup(pts: &[Pt], closed: bool) -> Vec<Pt> {
	let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
	for p in pts {
		match out.last() {
			Some(q) if q.distance(*p) < EPS_LEN => (),
			_ => out.push(*p),
		}
	}
	if closed && out.len() > 1 {
		let (first, last) = (out[0], out[out.len() - 1]);
		if first.distance(last) < EPS_LEN {
			out.pop();
		}
	}
	out
}

/// How many straight segments an arc needs to stay within the tolerance.
///
/// A chord subtending an angle `a` on a circle of radius `r` bulges away from it by `r(1 -
/// cos(a/2))`, so holding that below the tolerance fixes the angle, and the angle fixes the count.
fn arc_steps(r: f32, sweep: f32, tol: f32) -> usize {
	let sweep = sweep.abs();
	if !sweep.is_finite() || sweep <= 0.0 {
		return 1;
	}
	let cos = (1.0 - tol / r).clamp(-1.0, 1.0);
	let a = 2.0 * cos.acos(); // The widest angle one chord may span.
	if !a.is_finite() || a <= 0.0 {
		return MAX_ARC_STEPS;
	}
	((sweep / a).ceil().max(1.0) as usize).min(MAX_ARC_STEPS)
}

/// The unit direction from `a` to `b`, or `None` where there is none because they are one point.
fn dir(a: Pt, b: Pt) -> Option<Pt> {
	let d = sub(b, a);
	let len = (d.x * d.x + d.y * d.y).sqrt();
	if !len.is_finite() || len < EPS_LEN {
		return None;
	}
	Some(mul(d, 1.0 / len))
}

/// The sum of two points, taken as vectors.
fn add(a: Pt, b: Pt) -> Pt {
	Pt::new(a.x + b.x, a.y + b.y)
}

/// The difference of two points, taken as vectors.
fn sub(a: Pt, b: Pt) -> Pt {
	Pt::new(a.x - b.x, a.y - b.y)
}

/// A point scaled, taken as a vector.
fn mul(a: Pt, s: f32) -> Pt {
	Pt::new(a.x * s, a.y * s)
}

/// A point a fraction of the way from `a` to `b`.
fn lerp(a: Pt, b: Pt, s: f32) -> Pt {
	Pt::new(a.x + (b.x - a.x) * s, a.y + (b.y - a.y) * s)
}

/// The left normal of a direction: the direction under a quarter turn.
fn left(d: Pt) -> Pt {
	Pt::new(-d.y, d.x)
}

/// The right normal of a direction: the direction under a quarter turn the other way.
fn right(d: Pt) -> Pt {
	Pt::new(d.y, -d.x)
}

/// A vector turned through an angle.
fn rot(v: Pt, a: f32) -> Pt {
	let (s, c) = a.sin_cos();
	Pt::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::{
		colour::Rgba,
		pixmap::Pixmap,
	};

	/// Renders a stroke, black on white, so that a test can read the ink back pixel by pixel.
	fn ink(path: &Path, pen: &Stroke, w: usize, h: usize) -> Outcome<Pixmap> {
		let mut pm = res!(Pixmap::filled(w, h, Rgba::WHITE));
		res!(pm.stroke_path(path, &Transform::IDENTITY, Rgba::BLACK, None, pen));
		Ok(pm)
	}

	/// How dark a pixel came out, from 0 for untouched to 1 for solid.
	fn dark(pm: &Pixmap, x: usize, y: usize) -> f32 {
		match pm.pixel(x, y) {
			Some(c) => 1.0 - (c.r as f32) / 255.0,
			None => 0.0,
		}
	}

	/// The topmost row holding any real ink, which is how far a corner reaches.
	fn top_row(pm: &Pixmap) -> Option<usize> {
		for y in 0..pm.height() {
			for x in 0..pm.width() {
				if dark(pm, x, y) > 0.5 {
					return Some(y);
				}
			}
		}
		None
	}

	/// A straight line, as an open contour.
	fn line(a: Pt, b: Pt) -> Outcome<Path> {
		let mut pb = PathBuilder::new();
		pb.move_to(a);
		pb.line_to(b);
		pb.finish()
	}

	/// A square, as a closed contour, with the option of leaving it open.
	fn square(x0: f32, y0: f32, x1: f32, y1: f32, closed: bool) -> Outcome<Path> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(x0, y0));
		pb.line_to(Pt::new(x1, y0));
		pb.line_to(Pt::new(x1, y1));
		pb.line_to(Pt::new(x0, y1));
		if closed {
			pb.close();
		}
		pb.finish()
	}

	/// A narrow V, whose apex is sharp enough to mitre past the default limit.
	///
	/// The arms meet at about 28 degrees, so the miter reaches about 4.1 line widths past the apex:
	/// over the default limit of 4, and under a limit of 6.
	fn vee() -> Outcome<Path> {
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(14.0, 38.0));
		pb.line_to(Pt::new(20.0, 14.0));
		pb.line_to(Pt::new(26.0, 38.0));
		pb.finish()
	}

	#[test]
	fn test_a_width_that_cannot_draw_is_refused_00() -> Outcome<()> {
		let path = res!(line(Pt::new(2.0, 8.0), Pt::new(14.0, 8.0)));
		assert!(Stroke::new(0.0).is_err(), "a zero width");
		assert!(Stroke::new(-3.0).is_err(), "a negative width");
		assert!(Stroke::new(f32::NAN).is_err(), "a width that is not a number");
		assert!(Stroke::new(f32::INFINITY).is_err(), "an infinite width");
		// The fields are public, so the check must also bite at the moment of drawing.
		let bad = Stroke { width: -1.0, ..Stroke::default() };
		assert!(path.stroke(&bad).is_err(), "a negative width set after construction");
		Ok(())
	}

	#[test]
	fn test_a_miter_limit_below_one_is_refused_01() -> Outcome<()> {
		let path = res!(vee());
		let pen = res!(Stroke::new(4.0)).with_miter_limit(0.5);
		assert!(pen.check().is_err(), "a limit no miter could ever meet");
		assert!(path.stroke(&pen).is_err());
		Ok(())
	}

	#[test]
	fn test_a_line_strokes_to_a_band_02() -> Outcome<()> {
		// A pen four wide, run from (2, 8) to (14, 8), inks the band x in [2, 14], y in [6, 10].
		let path = res!(line(Pt::new(2.0, 8.0), Pt::new(14.0, 8.0)));
		let pen = res!(Stroke::new(4.0));
		let pm = res!(ink(&path, &pen, 16, 16));
		assert!(dark(&pm, 8, 6) > 0.99, "the top row of the band, found {}", dark(&pm, 8, 6));
		assert!(dark(&pm, 8, 9) > 0.99, "the bottom row of the band");
		assert!(dark(&pm, 8, 5) < 0.01, "above the band, found {}", dark(&pm, 8, 5));
		assert!(dark(&pm, 8, 10) < 0.01, "below the band, found {}", dark(&pm, 8, 10));
		assert!(dark(&pm, 2, 8) > 0.99, "the first column of the band");
		assert!(dark(&pm, 13, 8) > 0.99, "the last column of the band");
		Ok(())
	}

	#[test]
	fn test_a_butt_cap_reaches_nowhere_but_the_others_reach_out_03() -> Outcome<()> {
		let path = res!(line(Pt::new(2.0, 8.0), Pt::new(14.0, 8.0)));
		let base = res!(Stroke::new(4.0));

		let butt = res!(ink(&path, &base.clone().with_cap(Cap::Butt), 16, 16));
		assert!(dark(&butt, 1, 8) < 0.01, "a butt cap stops dead, found {}", dark(&butt, 1, 8));

		let square = res!(ink(&path, &base.clone().with_cap(Cap::Square), 16, 16));
		assert!(dark(&square, 1, 8) > 0.99, "a square cap reaches out by half the width");
		assert!(dark(&square, 0, 6) > 0.99, "and squarely, right into its corner");

		let round = res!(ink(&path, &base.with_cap(Cap::Round), 16, 16));
		assert!(dark(&round, 1, 8) > 0.99, "a round cap reaches out too");
		assert!(
			dark(&round, 0, 6) < 0.5,
			"but roundly, so it does not fill the corner, found {}", dark(&round, 0, 6),
		);
		Ok(())
	}

	#[test]
	fn test_a_point_is_a_dot_under_a_round_cap_and_nothing_under_a_butt_04() -> Outcome<()> {
		// A contour with no length: the pen is set down and lifted in the same place.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(8.0, 8.0));
		pb.line_to(Pt::new(8.0, 8.0));
		let path = res!(pb.finish());
		let base = res!(Stroke::new(4.0));

		let butt = res!(path.stroke(&base.clone().with_cap(Cap::Butt)));
		assert!(butt.is_empty(), "a butt cap on a point reaches nowhere, so there is no ink");

		let round = res!(ink(&path, &base.clone().with_cap(Cap::Round), 16, 16));
		assert!(dark(&round, 8, 8) > 0.99, "a round cap on a point is a dot");
		assert!(dark(&round, 8, 4) < 0.01, "and no larger than the pen");
		assert!(
			dark(&round, 6, 6) < 0.5,
			"and round, so its corner is bitten off, found {}", dark(&round, 6, 6),
		);

		let sq = res!(ink(&path, &base.with_cap(Cap::Square), 16, 16));
		assert!(dark(&sq, 8, 8) > 0.99, "a square cap on a point is a square");
		assert!(dark(&sq, 6, 6) > 0.99, "with its corner still on, found {}", dark(&sq, 6, 6));
		Ok(())
	}

	#[test]
	fn test_a_closed_contour_is_joined_all_the_way_round_05() -> Outcome<()> {
		// The corner at the contour's first point is the one that tells the tale. Closed, the pen
		// turns it and the miter fills the outer corner. Open, the pen starts and stops there, and
		// two butt caps leave the outer corner bare.
		let pen = res!(Stroke::new(2.0));
		let shut = res!(ink(&res!(square(4.0, 4.0, 12.0, 12.0, true)), &pen, 16, 16));
		let open = res!(ink(&res!(square(4.0, 4.0, 12.0, 12.0, false)), &pen, 16, 16));
		assert!(
			dark(&shut, 3, 3) > 0.99,
			"a closed contour joins its first corner, found {}", dark(&shut, 3, 3),
		);
		assert!(
			dark(&open, 3, 3) < 0.01,
			"an open one caps it instead, found {}", dark(&open, 3, 3),
		);
		// The other three corners are turned either way, so they must agree.
		assert!(dark(&shut, 12, 3) > 0.99, "the far corner, closed");
		assert!(dark(&open, 12, 3) > 0.99, "the far corner, open");
		Ok(())
	}

	#[test]
	fn test_a_miter_over_the_limit_falls_back_to_a_bevel_06() -> Outcome<()> {
		let path = res!(vee());
		let base = res!(Stroke::new(4.0));

		// Under a limit of six the apex mitres, throwing the ink well above the apex at y = 14.
		let long = res!(ink(&path, &base.clone().with_miter_limit(6.0), 40, 40));
		let far = match top_row(&long) {
			Some(y) => y,
			None => return Err(err!("The stroke of a V must leave some ink."; Bug)),
		};
		assert!(far < 9, "a miter should reach far past the apex, but stopped at row {}", far);

		// Under the default limit of four the same apex is over the limit, and is bevelled: the ink
		// stops within half a line width of the apex.
		let cut = res!(ink(&path, &base.clone(), 40, 40));
		let near = match top_row(&cut) {
			Some(y) => y,
			None => return Err(err!("The stroke of a V must leave some ink."; Bug)),
		};
		assert!(
			near >= 12,
			"a miter over its limit should be bevelled back, but reached row {}", near,
		);

		// A bevel asked for outright must land in the same place as the miter that fell back to one.
		let bevel = res!(ink(&path, &base.with_join(Join::Bevel), 40, 40));
		assert_eq!(cut, bevel, "a miter over its limit must be exactly a bevel");
		Ok(())
	}

	#[test]
	fn test_a_round_join_stays_within_half_a_width_of_the_corner_07() -> Outcome<()> {
		// A round join is a wedge of a disc of half the line width, so however sharp the corner, it
		// can never reach further than that. The apex is at y = 14 and the pen is 4 wide.
		let path = res!(vee());
		let pen = res!(Stroke::new(4.0)).with_join(Join::Round);
		let pm = res!(ink(&path, &pen, 40, 40));
		let top = match top_row(&pm) {
			Some(y) => y,
			None => return Err(err!("The stroke of a V must leave some ink."; Bug)),
		};
		assert!(top >= 11, "a round join cannot reach past row 12, but reached row {}", top);
		assert!(top <= 13, "nor should it fall short of the corner, found row {}", top);
		Ok(())
	}

	#[test]
	fn test_a_hairpin_is_rounded_off_but_not_mitred_08() -> Outcome<()> {
		// The pen runs out to (20, 8) and doubles straight back. A round join must put a half disc
		// past the turn, bulging the way the pen was going, not the way it came. A miter there
		// would reach to infinity, so it must fall back to a bevel, which at a hairpin is nothing.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(4.0, 8.0));
		pb.line_to(Pt::new(20.0, 8.0));
		pb.line_to(Pt::new(4.0, 8.0));
		let path = res!(pb.finish());
		let base = res!(Stroke::new(4.0));

		// The pen is 4 wide, so the half disc has a radius of 2. The pixel at (20, 8) lies wholly
		// inside it, and the one at (21, 8) hangs over its rim.
		let round = res!(ink(&path, &base.clone().with_join(Join::Round), 24, 16));
		assert!(
			dark(&round, 20, 8) > 0.99,
			"a round join must bulge past the turn, found {}", dark(&round, 20, 8),
		);
		assert!(
			dark(&round, 21, 8) > 0.8,
			"and reach nearly a full radius past it, found {}", dark(&round, 21, 8),
		);

		let mitre = res!(ink(&path, &base.with_join(Join::Miter), 24, 16));
		assert!(
			dark(&mitre, 20, 8) < 0.01,
			"a miter at a hairpin must fall back to a bevel, and so to nothing, found {}",
			dark(&mitre, 20, 8),
		);
		Ok(())
	}

	#[test]
	fn test_the_seams_between_the_pieces_do_not_show_09() -> Outcome<()> {
		// The outline is a heap of pieces, and the joins between them run right through the ink. If
		// the pieces were composited the seams would show as light or dark lines; because the
		// rasteriser adds areas, they cannot. Every pixel well inside the band must be solid.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(4.0, 8.0));
		pb.line_to(Pt::new(20.0, 8.0));
		pb.line_to(Pt::new(20.0, 24.0));
		let path = res!(pb.finish());
		let pen = res!(Stroke::new(6.0)).with_join(Join::Round);
		let pm = res!(ink(&path, &pen, 32, 32));
		for x in 5..19 {
			assert!(dark(&pm, x, 8) > 0.99, "a seam shows at ({}, 8): {}", x, dark(&pm, x, 8));
		}
		for y in 10..22 {
			assert!(dark(&pm, 20, y) > 0.99, "a seam shows at (20, {}): {}", y, dark(&pm, 20, y));
		}
		// And the corner itself, which is where three pieces meet.
		assert!(dark(&pm, 19, 9) > 0.99, "the corner is not solid: {}", dark(&pm, 19, 9));
		Ok(())
	}

	#[test]
	fn test_a_curve_is_stroked_10() -> Outcome<()> {
		// The pen follows the flattened curve, so the ink must be a band about it and nothing more.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(4.0, 28.0));
		pb.quad_to(Pt::new(16.0, 0.0), Pt::new(28.0, 28.0));
		let path = res!(pb.finish());
		let pen = res!(Stroke::new(3.0)).with_cap(Cap::Round);
		let pm = res!(ink(&path, &pen, 32, 32));
		assert!(dark(&pm, 16, 14) > 0.9, "the crown of the arc, found {}", dark(&pm, 16, 14));
		assert!(dark(&pm, 16, 26) < 0.01, "under the arc, found {}", dark(&pm, 16, 26));
		assert!(dark(&pm, 16, 8) < 0.01, "over the arc, found {}", dark(&pm, 16, 8));
		Ok(())
	}

	#[test]
	fn test_a_dash_breaks_the_line_11() -> Outcome<()> {
		// Four on, four off, from x = 0.
		let path = res!(line(Pt::new(0.0, 8.0), Pt::new(32.0, 8.0)));
		let pen = res!(Stroke::new(4.0)).with_dash(Dash::new(vec![4.0, 4.0]));
		let pm = res!(ink(&path, &pen, 32, 16));
		assert!(dark(&pm, 2, 8) > 0.99, "the first dash");
		assert!(dark(&pm, 6, 8) < 0.01, "the first gap, found {}", dark(&pm, 6, 8));
		assert!(dark(&pm, 10, 8) > 0.99, "the second dash");
		assert!(dark(&pm, 14, 8) < 0.01, "the second gap");
		Ok(())
	}

	#[test]
	fn test_an_odd_dash_pattern_is_walked_twice_12() -> Outcome<()> {
		// A pattern of one length means four on, four off, as if it had been written out in full.
		let path = res!(line(Pt::new(0.0, 8.0), Pt::new(32.0, 8.0)));
		let pen = res!(Stroke::new(4.0));
		let odd = res!(ink(&path, &pen.clone().with_dash(Dash::new(vec![4.0])), 32, 16));
		let even = res!(ink(&path, &pen.with_dash(Dash::new(vec![4.0, 4.0])), 32, 16));
		assert_eq!(odd, even, "an odd pattern must be walked twice over");
		Ok(())
	}

	#[test]
	fn test_a_dash_offset_shifts_the_pattern_13() -> Outcome<()> {
		let path = res!(line(Pt::new(0.0, 8.0), Pt::new(32.0, 8.0)));
		let dash = Dash::new(vec![4.0, 4.0]).with_offset(4.0);
		let pen = res!(Stroke::new(4.0)).with_dash(dash);
		let pm = res!(ink(&path, &pen, 32, 16));
		assert!(dark(&pm, 2, 8) < 0.01, "the pattern begins in a gap, found {}", dark(&pm, 2, 8));
		assert!(dark(&pm, 6, 8) > 0.99, "and the first dash follows it");
		Ok(())
	}

	#[test]
	fn test_a_dash_that_spans_a_closed_contour_leaves_it_closed_14() -> Outcome<()> {
		// The ring is 32 round and the ink runs for 1000, so the pattern never turns off. The ring
		// must come back whole, joined at its first corner and not cut open and capped there.
		let path = res!(square(4.0, 4.0, 12.0, 12.0, true));
		let pen = res!(Stroke::new(2.0)).with_dash(Dash::new(vec![1000.0]));
		let pm = res!(ink(&path, &pen, 16, 16));
		assert!(
			dark(&pm, 3, 3) > 0.99,
			"the first corner must still be joined, found {}", dark(&pm, 3, 3),
		);
		Ok(())
	}

	#[test]
	fn test_a_dash_wrapping_a_closed_contour_is_sewn_back_together_15() -> Outcome<()> {
		// The ring is 32 round. Ten on, five off, walked from its first corner, ends with the ink
		// still on as the walk comes back round to where it began. That last run and the first are
		// one run, on either side of the corner, and must be sewn into one: sewn, the corner is
		// joined; unsewn, it comes out as two butt caps with a notch between them.
		let path = res!(square(4.0, 4.0, 12.0, 12.0, true));
		let pen = res!(Stroke::new(2.0)).with_dash(Dash::new(vec![10.0, 5.0]));
		let pm = res!(ink(&path, &pen, 16, 16));
		assert!(
			dark(&pm, 3, 3) > 0.99,
			"the wrapping dash must be sewn back into one, found {}", dark(&pm, 3, 3),
		);
		Ok(())
	}

	#[test]
	fn test_a_dash_pattern_that_never_inks_is_refused_16() -> Outcome<()> {
		let path = res!(line(Pt::new(0.0, 8.0), Pt::new(32.0, 8.0)));
		let base = res!(Stroke::new(4.0));
		let empty = base.clone().with_dash(Dash::new(vec![]));
		assert!(path.stroke(&empty).is_err(), "a pattern of no lengths");
		let zeros = base.clone().with_dash(Dash::new(vec![0.0, 0.0]));
		assert!(path.stroke(&zeros).is_err(), "a pattern that is all zeroes");
		let neg = base.clone().with_dash(Dash::new(vec![4.0, -1.0]));
		assert!(path.stroke(&neg).is_err(), "a pattern with a negative length");
		let nan = base.with_dash(Dash::new(vec![4.0, 4.0]).with_offset(f32::NAN));
		assert!(path.stroke(&nan).is_err(), "an offset that is not a number");
		Ok(())
	}

	#[test]
	fn test_an_empty_path_strokes_to_nothing_17() -> Outcome<()> {
		let pen = res!(Stroke::new(4.0)).with_cap(Cap::Round);
		let empty = res!(PathBuilder::new().finish());
		assert!(res!(empty.stroke(&pen)).is_empty());
		// A move with nothing after it goes nowhere and leaves nothing, where a move closed on
		// itself is a path asking for a dot.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(8.0, 8.0));
		let lone = res!(pb.finish());
		assert!(res!(lone.stroke(&pen)).is_empty(), "a lone move leaves nothing");
		Ok(())
	}

	#[test]
	fn test_the_outline_is_filled_non_zero_not_even_odd_18() -> Outcome<()> {
		// The pieces overlap, so an even-odd fill of the outline would eat holes out of it. This
		// pins the contract that [`Path::stroke`] documents.
		use crate::raster::FillRule;
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(4.0, 8.0));
		pb.line_to(Pt::new(20.0, 8.0));
		pb.line_to(Pt::new(20.0, 24.0));
		let path = res!(pb.finish());
		let pen = res!(Stroke::new(6.0)).with_join(Join::Round);
		let outline = res!(path.stroke(&pen));

		let mut nz = res!(Pixmap::filled(32, 32, Rgba::WHITE));
		res!(nz.fill_path(&outline, &Transform::IDENTITY, Rgba::BLACK, None));
		let mut eo = res!(Pixmap::filled(32, 32, Rgba::WHITE));
		res!(eo.fill_path_with(
			&outline, &Transform::IDENTITY, Rgba::BLACK, None, FillRule::EvenOdd,
		));
		assert!(dark(&nz, 19, 9) > 0.99, "the corner is solid under the non-zero rule");
		assert!(
			dark(&eo, 19, 9) < 0.5,
			"and eaten away under the even-odd rule, found {}", dark(&eo, 19, 9),
		);
		Ok(())
	}
}
