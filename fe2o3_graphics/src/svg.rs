//! SVG path data: the `d` attribute, read into a [`Path`].
//!
//! A vector mark -- an icon, a logo -- is drawn in a drawing program and leaves it as SVG, where all
//! of the geometry sits in the `d` attribute of a `<path>` element: a terse string of one-letter
//! commands and numbers. This module reads that string with [`path_data`] and writes it back with
//! [`write_path_data`], and nothing else. No XML, no styling, no document: the caller keeps whatever
//! it wants of the file and hands the geometry here.
//!
//! That boundary is the point. Path data is a small, closed, fully specified grammar, and it is the
//! part every drawing program agrees on. Everything above it -- elements, attributes, gradients,
//! filters, referenced content -- is a document format, and a caller that wants an icon does not want
//! a document.
//!
//! The one concession above bare geometry is [`presentation`], which renders the paint a
//! [`crate::stroke::Stroke`] and an [`Rgba`] already model -- fill, stroke, width, caps, joins,
//! dashes -- as the attribute string a `<path>` carries alongside its `d`. It writes the attributes
//! and no element around them, for the same reason the reader stops at the `d`: the element tree is
//! the caller's format.
//!
//! Every command in the grammar is read, including elliptical arcs. An arc has no [`crate::path::Seg`]
//! of its own, so it is converted to cubic béziers on the way in and no caller has to know it was
//! ever an arc.

use crate::colour::Rgba;
use crate::path::{
	Path,
	PathBuilder,
	Pt,
	Seg,
};
use crate::stroke::{
	Cap,
	Join,
	Stroke,
};

use oxedyne_fe2o3_core::prelude::*;

use std::f64::consts::PI;

/// The most cubic segments one elliptical arc becomes.
///
/// An arc is cut at quadrant boundaries and its sweep cannot exceed a full turn, so four pieces
/// always suffice.
const ARC_SEGS: usize = 4;

/// The ellipse an arc travels on, and which of the four arcs between the endpoints to take.
///
/// These are the five arguments the `A` command carries before its endpoint. They travel together
/// because they mean nothing apart: a radius without its flags does not pick out an arc.
#[derive(Clone, Copy)]
struct Arc {
	/// Horizontal radius. Its sign is ignored, and a radius too small to span the ends is grown.
	rx: f32,
	/// Vertical radius.
	ry: f32,
	/// The ellipse's x-axis rotation, in degrees.
	rot: f32,
	/// Take the sweep greater than a half turn.
	large: bool,
	/// Take the sweep in the direction of increasing angle.
	sweep: bool,
}

/// The last curve's trailing control point, which `S` and `T` reflect.
///
/// The kind matters: `S` reflects only a cubic's control point and `T` only a quadratic's. After any
/// other command, or a curve of the other kind, the reflected point is the current point instead --
/// so a bare [`Self::None`] is not enough and the kind must be carried.
#[derive(Clone, Copy)]
enum Last {
	/// The previous command was not a curve, or was a curve of the other kind.
	None,
	/// The previous command was `C`, `c`, `S` or `s`, carrying its second control point.
	Cubic(Pt),
	/// The previous command was `Q`, `q`, `T` or `t`, carrying its control point.
	Quad(Pt),
}

/// Reads SVG path data -- the `d` attribute of a `<path>` element -- into a [`Path`].
///
/// # Arguments
/// * `d` - The path data, as it appears in the attribute.
///
/// # Returns
/// The path the data describes, or an error naming the byte the data went wrong at.
///
/// # Errors
/// Data that does not begin with a command, names a command outside the grammar, ends partway
/// through a command's arguments, or holds an arc flag that is not `0` or `1`.
pub fn path_data(d: &str) -> Outcome<Path> {
	let mut sc = Scan::new(d);
	let mut pb = PathBuilder::new();
	let mut cur = Pt::new(0.0, 0.0); // Where the pen is.
	let mut start = Pt::new(0.0, 0.0); // Where this contour began, which `Z` returns to.
	let mut prev = 0u8; // The last command, for the implicit-repeat rule.
	let mut last = Last::None;
	let mut open = false; // Whether a contour is under way.
	loop {
		if sc.done() {
			break;
		}
		// A command letter may be left out to repeat the last one. A repeated `moveto` is a
		// `lineto`, which is the grammar's one irregularity.
		let cmd = match sc.cmd() {
			Some(c) => c,
			None => match prev {
				0 => return Err(err!(
					"Path data must begin with a command letter, found '{}'.",
					sc.rest(); Invalid, Input)),
				b'M' => b'L',
				b'm' => b'l',
				c => c,
			},
		};
		// The grammar opens with a moveto, and nothing else will do: until one has been read there is
		// no pen for a drawing command to draw from.
		if prev == 0 && !matches!(cmd, b'M' | b'm') {
			return Err(err!(
				"Path data must begin with a moveto, found '{}'.", cmd as char; Invalid, Input));
		}
		prev = cmd;
		// A `Z` leaves the pen on the contour's first point but closes the contour. The
		// specification has the next subpath begin at that same point, so a drawing command
		// following a close opens one there rather than drawing from nowhere.
		if !open && !matches!(cmd, b'M' | b'm' | b'Z' | b'z') {
			pb.move_to(cur);
			open = true;
		}
		let rel = cmd.is_ascii_lowercase();
		match cmd {
			b'M' | b'm' => {
				let p = res!(sc.point(rel, cur));
				pb.move_to(p);
				cur = p;
				start = p;
				last = Last::None;
				open = true;
			},
			b'L' | b'l' => {
				let p = res!(sc.point(rel, cur));
				pb.line_to(p);
				cur = p;
				last = Last::None;
			},
			b'H' | b'h' => {
				let x = res!(sc.num());
				let p = Pt::new(if rel { cur.x + x } else { x }, cur.y);
				pb.line_to(p);
				cur = p;
				last = Last::None;
			},
			b'V' | b'v' => {
				let y = res!(sc.num());
				let p = Pt::new(cur.x, if rel { cur.y + y } else { y });
				pb.line_to(p);
				cur = p;
				last = Last::None;
			},
			b'C' | b'c' => {
				let c0 = res!(sc.point(rel, cur));
				let c1 = res!(sc.point(rel, cur));
				let p = res!(sc.point(rel, cur));
				pb.cubic_to(c0, c1, p);
				cur = p;
				last = Last::Cubic(c1);
			},
			b'S' | b's' => {
				let c1 = res!(sc.point(rel, cur));
				let p = res!(sc.point(rel, cur));
				let c0 = match last {
					Last::Cubic(q) => reflect(cur, q),
					_ => cur,
				};
				pb.cubic_to(c0, c1, p);
				cur = p;
				last = Last::Cubic(c1);
			},
			b'Q' | b'q' => {
				let c = res!(sc.point(rel, cur));
				let p = res!(sc.point(rel, cur));
				pb.quad_to(c, p);
				cur = p;
				last = Last::Quad(c);
			},
			b'T' | b't' => {
				let p = res!(sc.point(rel, cur));
				let c = match last {
					Last::Quad(q) => reflect(cur, q),
					_ => cur,
				};
				pb.quad_to(c, p);
				cur = p;
				last = Last::Quad(c);
			},
			b'A' | b'a' => {
				let rx = res!(sc.num());
				let ry = res!(sc.num());
				let rot = res!(sc.num());
				let large = res!(sc.flag());
				let sweep = res!(sc.flag());
				let p = res!(sc.point(rel, cur));
				arc(&mut pb, cur, Arc { rx, ry, rot, large, sweep }, p);
				cur = p;
				last = Last::None;
			},
			b'Z' | b'z' => {
				pb.close();
				cur = start;
				last = Last::None;
				open = false;
			},
			c => return Err(err!(
				"'{}' is not an SVG path command.", c as char; Invalid, Input)),
		}
	}
	pb.finish()
}

/// Reflects `q` through `p`, which is what `S` and `T` do to the previous control point to keep a
/// curve smooth across the join.
fn reflect(p: Pt, q: Pt) -> Pt {
	Pt::new(2.0 * p.x - q.x, 2.0 * p.y - q.y)
}

/// Lays an elliptical arc onto `pb` as cubic béziers.
///
/// SVG states an arc by where it ends and which of the four candidate arcs to take; a bézier needs
/// the centre and the angles spanned. The conversion between them is the one the SVG specification
/// sets out in its implementation notes (F.6.5 for the centre, F.6.6 for radii too small to reach),
/// after which each quadrant of the sweep takes one cubic.
///
/// The arithmetic runs in `f64` though the path is `f32`: the centre falls out of a difference of
/// squares that cancels badly near the degenerate cases, and the wider type costs nothing here.
///
/// # Arguments
/// * `pb` - The builder to append to. The pen is assumed to be at `p0`.
/// * `p0` - Where the arc starts.
/// * `a` - The ellipse to travel on, and which arc of it to take.
/// * `p1` - Where the arc ends.
fn arc(pb: &mut PathBuilder, p0: Pt, a: Arc, p1: Pt) {
	// An arc whose ends coincide is dropped, and one with no radius is a straight line. Both are
	// what the specification asks for, and both would otherwise divide by zero below.
	if p0 == p1 {
		return;
	}
	let (mut rx, mut ry) = ((a.rx as f64).abs(), (a.ry as f64).abs());
	if rx == 0.0 || ry == 0.0 {
		pb.line_to(p1);
		return;
	}
	let (large, sweep) = (a.large, a.sweep);
	let (x0, y0) = (p0.x as f64, p0.y as f64);
	let (x1, y1) = (p1.x as f64, p1.y as f64);
	let (sin_phi, cos_phi) = (a.rot as f64).to_radians().sin_cos();

	// The ends in the ellipse's own frame, with their midpoint at the origin.
	let dx = (x0 - x1) / 2.0;
	let dy = (y0 - y1) / 2.0;
	let xp = cos_phi * dx + sin_phi * dy;
	let yp = -sin_phi * dx + cos_phi * dy;

	// Radii too small to reach from one end to the other are grown until they just do (F.6.6).
	let lam = (xp * xp) / (rx * rx) + (yp * yp) / (ry * ry);
	if lam > 1.0 {
		let s = lam.sqrt();
		rx *= s;
		ry *= s;
	}

	// The centre, in that frame and then back in the caller's (F.6.5).
	let num = rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp;
	let den = rx * rx * yp * yp + ry * ry * xp * xp;
	// The max() holds the root real against rounding; lam has already made num non-negative.
	let mut co = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
	if large == sweep {
		co = -co;
	}
	let cxp = co * (rx * yp) / ry;
	let cyp = -co * (ry * xp) / rx;
	let cx = cos_phi * cxp - sin_phi * cyp + (x0 + x1) / 2.0;
	let cy = sin_phi * cxp + cos_phi * cyp + (y0 + y1) / 2.0;

	// Where the sweep starts and how far it goes.
	let ux = (xp - cxp) / rx;
	let uy = (yp - cyp) / ry;
	let vx = (-xp - cxp) / rx;
	let vy = (-yp - cyp) / ry;
	let th0 = angle(1.0, 0.0, ux, uy);
	let mut dth = angle(ux, uy, vx, vy);
	if !sweep && dth > 0.0 {
		dth -= 2.0 * PI;
	}
	if sweep && dth < 0.0 {
		dth += 2.0 * PI;
	}

	// One cubic per quadrant of the sweep. A bézier meets a circular arc closely only over a short
	// span, so the cut is what keeps the approximation honest.
	let n = ((dth.abs() / (PI / 2.0)).ceil() as usize).clamp(1, ARC_SEGS);
	let step = dth / n as f64;
	// How far along the tangent a control point sits, for a piece spanning this angle. At a quarter
	// turn this is the familiar 0.5523.
	let k = (4.0 / 3.0) * (step / 4.0).tan();
	// The point on the ellipse at an angle, and the derivative there.
	let at = |t: f64| -> (f64, f64, f64, f64) {
		let (s, c) = t.sin_cos();
		(
			cx + rx * cos_phi * c - ry * sin_phi * s,
			cy + rx * sin_phi * c + ry * cos_phi * s,
			-rx * cos_phi * s - ry * sin_phi * c,
			-rx * sin_phi * s + ry * cos_phi * c,
		)
	};
	for i in 0..n {
		let t0 = th0 + step * i as f64;
		let (px0, py0, dx0, dy0) = at(t0);
		let (px1, py1, dx1, dy1) = at(t0 + step);
		pb.cubic_to(
			Pt::new((px0 + k * dx0) as f32, (py0 + k * dy0) as f32),
			Pt::new((px1 - k * dx1) as f32, (py1 - k * dy1) as f32),
			Pt::new(px1 as f32, py1 as f32),
		);
	}
}

/// The signed angle from one vector to another, which is what the arc conversion measures its sweep
/// with.
fn angle(ux: f64, uy: f64, vx: f64, vy: f64) -> f64 {
	let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
	if len == 0.0 {
		return 0.0;
	}
	// The clamp holds acos in range against rounding, which a dot product of unit vectors can leave
	// a hair outside.
	let a = ((ux * vx + uy * vy) / len).clamp(-1.0, 1.0).acos();
	if ux * vy - uy * vx < 0.0 {
		-a
	} else {
		a
	}
}

/// Writes a [`Path`] out as SVG path data -- the `d` attribute of a `<path>` element.
///
/// The exact inverse of [`path_data`]: every segment becomes the one command that names it, and a
/// string this writes is one that reader reads back to the same geometry. Only absolute commands are
/// emitted -- `M`, `L`, `Q`, `C`, `Z` -- since those are the segments the path types hold, and the
/// relative and shorthand forms the reader also accepts are a convenience of hand-written data, not
/// a distinction the geometry keeps.
///
/// The commands are separated by spaces, and the two coordinates of a point by a comma, which is the
/// form drawing programs write and the eye reads most easily. No document, element or attribute is
/// written -- only the path data -- for the reason [`path_data`] reads only the same: the structure
/// above a `<path>` is the caller's format, not this crate's.
///
/// # Arguments
/// * `path` - The path to write.
///
/// # Returns
/// The path data, as it would appear in the attribute. An empty path writes an empty string.
pub fn write_path_data(path: &Path) -> String {
	let mut out = String::new();
	for seg in path.segs() {
		if !out.is_empty() {
			out.push(' ');
		}
		match *seg {
			Seg::MoveTo(p) => {
				out.push('M');
				point(&mut out, p);
			},
			Seg::LineTo(p) => {
				out.push('L');
				point(&mut out, p);
			},
			Seg::QuadTo(c, p) => {
				out.push('Q');
				point(&mut out, c);
				out.push(' ');
				point(&mut out, p);
			},
			Seg::CubicTo(c0, c1, p) => {
				out.push('C');
				point(&mut out, c0);
				out.push(' ');
				point(&mut out, c1);
				out.push(' ');
				point(&mut out, p);
			},
			Seg::Close => out.push('Z'),
		}
	}
	out
}

/// Appends a point as `x,y`, each coordinate in its shortest exact form.
fn point(out: &mut String, p: Pt) {
	out.push_str(&num(p.x));
	out.push(',');
	out.push_str(&num(p.y));
}

/// One coordinate, in the shortest decimal that reads back to the same `f32`.
///
/// Rust's own float formatting already gives the shortest round-tripping form -- `10` for `10.0`,
/// `0.15` for a fifth and a bit -- so a whole coordinate carries no trailing `.0` and the data stays
/// terse, exactly as a drawing program would write it.
fn num(v: f32) -> String {
	fmt!("{}", v)
}

/// Renders the SVG presentation attributes for a fill and a stroke, as one attribute string.
///
/// This is the counterpart to [`write_path_data`] for everything that is not geometry: the colours,
/// the pen width, the caps and joins and dashes that [`crate::stroke::Stroke`] and [`Rgba`] already
/// model. It writes the attributes and their values -- `fill="#..."`, `stroke-width="2"`, and so on
/// -- and nothing around them, so a caller drops the string straight into the `<path>` element its
/// own format builds.
///
/// The fill and the stroke are each optional, because a shape may be filled, stroked, or both:
/// * A fill of `Some(c)` writes `fill` and, where the colour is not opaque, `fill-opacity`. A fill
///   of `None` writes `fill="none"`, since SVG fills black by default and a caller that wants no
///   fill must say so.
/// * A stroke of `Some((c, pen))` writes the stroke colour, its opacity where it is not opaque, the
///   width, the cap, the join, the miter limit, and the dash pattern and offset where the pen
///   carries one. A stroke of `None` writes nothing, and the shape is filled only.
///
/// The stroke colour travels with the pen because neither draws a stroke without the other: a width
/// with no colour paints nothing, and a colour with no width has nothing to paint.
pub fn presentation(fill: Option<Rgba>, stroke: Option<(Rgba, &Stroke)>) -> String {
	let mut at: Vec<String> = Vec::new();
	match fill {
		None => at.push(fmt!("fill=\"none\"")),
		Some(c) => {
			at.push(fmt!("fill=\"{}\"", rgb(c)));
			if !c.is_opaque() {
				at.push(fmt!("fill-opacity=\"{}\"", opacity(c)));
			}
		},
	}
	if let Some((c, pen)) = stroke {
		at.push(fmt!("stroke=\"{}\"", rgb(c)));
		if !c.is_opaque() {
			at.push(fmt!("stroke-opacity=\"{}\"", opacity(c)));
		}
		at.push(fmt!("stroke-width=\"{}\"", num(pen.width)));
		at.push(fmt!("stroke-linecap=\"{}\"", cap(pen.cap)));
		at.push(fmt!("stroke-linejoin=\"{}\"", join(pen.join)));
		at.push(fmt!("stroke-miterlimit=\"{}\"", num(pen.miter_limit)));
		if let Some(d) = &pen.dash {
			let lens: Vec<String> = d.pattern.iter().map(|v| num(*v)).collect();
			at.push(fmt!("stroke-dasharray=\"{}\"", lens.join(",")));
			if d.offset != 0.0 {
				at.push(fmt!("stroke-dashoffset=\"{}\"", num(d.offset)));
			}
		}
	}
	at.join(" ")
}

/// A colour's `#rrggbb`, the paint value an SVG attribute takes. The alpha, if any, is carried
/// separately by an opacity attribute, which is the form every SVG renderer understands.
fn rgb(c: Rgba) -> String {
	fmt!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// A colour's alpha as an opacity from 0 to 1, to three places, which resolves every one of the 256
/// steps an eight-bit alpha can take.
fn opacity(c: Rgba) -> String {
	fmt!("{:.3}", (c.a as f32) / 255.0)
}

/// The SVG name of a line cap.
fn cap(c: Cap) -> &'static str {
	match c {
		Cap::Butt	=> "butt",
		Cap::Round	=> "round",
		Cap::Square	=> "square",
	}
}

/// The SVG name of a line join.
fn join(j: Join) -> &'static str {
	match j {
		Join::Miter	=> "miter",
		Join::Round	=> "round",
		Join::Bevel	=> "bevel",
	}
}

/// A cursor over path data.
struct Scan<'a> {
	/// The data. ASCII throughout, so a byte index is always a character boundary.
	s: &'a [u8],
	/// How far in the cursor has reached.
	i: usize,
}

impl<'a> Scan<'a> {
	/// Opens a cursor at the start of the data.
	fn new(s: &'a str) -> Self {
		Self { s: s.as_bytes(), i: 0 }
	}

	/// Steps over whitespace and commas, which separate numbers and mean nothing else.
	fn sep(&mut self) {
		while self.i < self.s.len() {
			match self.s[self.i] {
				b' ' | b'\t' | b'\n' | b'\r' | b'\x0C' | b',' => self.i += 1,
				_ => break,
			}
		}
	}

	/// Whether the data is spent.
	fn done(&mut self) -> bool {
		self.sep();
		self.i >= self.s.len()
	}

	/// What is left, for an error to quote. Truncated, since path data runs long.
	fn rest(&self) -> String {
		let end = (self.i + 16).min(self.s.len());
		String::from_utf8_lossy(&self.s[self.i..end]).into_owned()
	}

	/// Takes the next byte if it is a command letter, and leaves the cursor alone if not.
	fn cmd(&mut self) -> Option<u8> {
		self.sep();
		if self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
			self.i += 1;
			Some(self.s[self.i - 1])
		} else {
			None
		}
	}

	/// Reads one number.
	///
	/// The grammar is looser than Rust's: a sign is optional, either side of the point may be empty,
	/// and there is no separator requirement -- so `1.5.5` is two numbers and `-1-2` is two more.
	/// The scanner therefore stops at the second point rather than trusting `parse` to complain.
	fn num(&mut self) -> Outcome<f32> {
		self.sep();
		let from = self.i;
		if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
			self.i += 1;
		}
		let mut any = false; // A number needs at least one digit, on one side or the other.
		while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
			self.i += 1;
			any = true;
		}
		if self.i < self.s.len() && self.s[self.i] == b'.' {
			self.i += 1;
			while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
				self.i += 1;
				any = true;
			}
		}
		if !any {
			return Err(err!(
				"Expected a number at byte {} of the path data, found '{}'.",
				from, self.rest(); Invalid, Input));
		}
		// An exponent counts only if digits follow it. Otherwise the 'e' is not ours -- path data
		// has no command by that name, but being strict here keeps the error at the right byte.
		if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
			let mark = self.i;
			self.i += 1;
			if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
				self.i += 1;
			}
			if self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
				while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
					self.i += 1;
				}
			} else {
				self.i = mark;
			}
		}
		let txt = res!(std::str::from_utf8(&self.s[from..self.i]));
		match txt.parse::<f32>() {
			Ok(v) => Ok(v),
			Err(e) => Err(err!(e,
				"'{}' at byte {} of the path data is not a number.", txt, from;
			Invalid, Input)),
		}
	}

	/// Reads an arc flag: a single `0` or `1`.
	///
	/// A flag is one character and needs no separator, so `0 011` is two flags and the start of a
	/// number. Reading it with [`Self::num`] would swallow the digits that follow it.
	fn flag(&mut self) -> Outcome<bool> {
		self.sep();
		if self.i >= self.s.len() {
			return Err(err!("The path data ended where an arc flag was expected."; Invalid, Input));
		}
		self.i += 1;
		match self.s[self.i - 1] {
			b'0' => Ok(false),
			b'1' => Ok(true),
			c => Err(err!(
				"An arc flag is '0' or '1', found '{}' at byte {} of the path data.",
				c as char, self.i - 1; Invalid, Input)),
		}
	}

	/// Reads a coordinate pair, offset from `from` when the command was relative.
	fn point(&mut self, rel: bool, from: Pt) -> Outcome<Pt> {
		let x = res!(self.num());
		let y = res!(self.num());
		Ok(if rel {
			Pt::new(from.x + x, from.y + y)
		} else {
			Pt::new(x, y)
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		path::{
			Seg,
			TOLERANCE,
		},
		transform::Transform,
	};

	#[test]
	fn test_a_moveto_and_a_lineto_place_the_pen_00() -> Outcome<()> {
		let p = res!(path_data("M 10 20 L 30 40"));
		assert_eq!(p.segs(), &[Seg::MoveTo(Pt::new(10.0, 20.0)), Seg::LineTo(Pt::new(30.0, 40.0))]);
		Ok(())
	}

	#[test]
	fn test_a_lower_case_command_is_relative_to_the_pen_01() -> Outcome<()> {
		let p = res!(path_data("M 10 10 l 5 5 l 5 5"));
		assert_eq!(p.segs(), &[
			Seg::MoveTo(Pt::new(10.0, 10.0)),
			Seg::LineTo(Pt::new(15.0, 15.0)),
			Seg::LineTo(Pt::new(20.0, 20.0)),
		]);
		Ok(())
	}

	#[test]
	fn test_a_repeated_moveto_is_a_lineto_02() -> Outcome<()> {
		// The grammar's one irregularity: extra pairs after a moveto are linetos, not movetos. Read
		// as movetos they would be three contours of one point each, and nothing would be drawn.
		let p = res!(path_data("M 0 0 1 1 2 2"));
		assert_eq!(p.segs(), &[
			Seg::MoveTo(Pt::new(0.0, 0.0)),
			Seg::LineTo(Pt::new(1.0, 1.0)),
			Seg::LineTo(Pt::new(2.0, 2.0)),
		]);
		Ok(())
	}

	#[test]
	fn test_a_command_letter_may_be_left_out_to_repeat_it_03() -> Outcome<()> {
		let p = res!(path_data("M 0 0 L 1 1 2 2 3 3"));
		assert_eq!(p.segs().len(), 4);
		assert_eq!(p.segs()[3], Seg::LineTo(Pt::new(3.0, 3.0)));
		Ok(())
	}

	#[test]
	fn test_two_numbers_may_share_a_point_04() -> Outcome<()> {
		// `1.5.5` is 1.5 then 0.5: the grammar needs no separator between numbers, so a second point
		// ends the first number. A scanner that read greedily to the next separator would see one
		// malformed number and reject a legal file.
		let p = res!(path_data("M1.5.5L.5 1"));
		assert_eq!(p.segs()[0], Seg::MoveTo(Pt::new(1.5, 0.5)));
		assert_eq!(p.segs()[1], Seg::LineTo(Pt::new(0.5, 1.0)));
		Ok(())
	}

	#[test]
	fn test_a_sign_separates_numbers_05() -> Outcome<()> {
		// `-1-2` is two numbers, for the same reason.
		let p = res!(path_data("M0 0L-1-2"));
		assert_eq!(p.segs()[1], Seg::LineTo(Pt::new(-1.0, -2.0)));
		Ok(())
	}

	#[test]
	fn test_an_exponent_is_read_06() -> Outcome<()> {
		let p = res!(path_data("M 0 0 L 1e2 1.5e-1"));
		assert_eq!(p.segs()[1], Seg::LineTo(Pt::new(100.0, 0.15)));
		Ok(())
	}

	#[test]
	fn test_an_arc_flag_needs_no_separator_07() -> Outcome<()> {
		// `0 011 1` is two flags then the endpoint. A flag read as a number would swallow `011` whole
		// and the arc would land somewhere else entirely -- silently, with no error to notice.
		let a = res!(path_data("M 0 0 a 1 1 0 011 1"));
		let b = res!(path_data("M 0 0 a 1 1 0 0 1 1 1"));
		assert_eq!(a.segs(), b.segs());
		Ok(())
	}

	#[test]
	fn test_a_smooth_cubic_reflects_the_last_control_point_08() -> Outcome<()> {
		// After C with its second control at (2,2) and the pen at (3,3), S's first control must be
		// the reflection, (4,4).
		let p = res!(path_data("M 0 0 C 1 1 2 2 3 3 S 5 5 6 6"));
		match p.segs()[2] {
			Seg::CubicTo(c0, _, _) => assert_eq!(c0, Pt::new(4.0, 4.0)),
			s => return Err(err!("Expected a cubic, found {:?}.", s; Test, Invalid)),
		}
		Ok(())
	}

	#[test]
	fn test_a_smooth_cubic_after_a_non_curve_uses_the_pen_09() -> Outcome<()> {
		// There is nothing to reflect, so the first control coincides with the current point. A
		// reader that reflected a stale control point would bend the curve the wrong way.
		let p = res!(path_data("M 0 0 L 3 3 S 5 5 6 6"));
		match p.segs()[2] {
			Seg::CubicTo(c0, _, _) => assert_eq!(c0, Pt::new(3.0, 3.0)),
			s => return Err(err!("Expected a cubic, found {:?}.", s; Test, Invalid)),
		}
		Ok(())
	}

	#[test]
	fn test_a_smooth_cubic_does_not_reflect_a_quadratics_control_10() -> Outcome<()> {
		// `S` reflects only a cubic's control point. After a `Q`, there is nothing of its kind to
		// reflect, so the pen is used -- which is why the last control point carries its kind.
		let p = res!(path_data("M 0 0 Q 1 1 3 3 S 5 5 6 6"));
		match p.segs()[2] {
			Seg::CubicTo(c0, _, _) => assert_eq!(c0, Pt::new(3.0, 3.0)),
			s => return Err(err!("Expected a cubic, found {:?}.", s; Test, Invalid)),
		}
		Ok(())
	}

	#[test]
	fn test_close_returns_the_pen_to_where_the_contour_began_11() -> Outcome<()> {
		// The `l 1 0` after `Z` is relative to (2,2), where the contour started, not to (5,5) where
		// the pen last drew. The close also ends the contour, so the next subpath opens at that same
		// point -- which is what the implicit moveto records.
		let p = res!(path_data("M 2 2 L 5 5 Z l 1 0"));
		assert_eq!(p.segs(), &[
			Seg::MoveTo(Pt::new(2.0, 2.0)),
			Seg::LineTo(Pt::new(5.0, 5.0)),
			Seg::Close,
			Seg::MoveTo(Pt::new(2.0, 2.0)),
			Seg::LineTo(Pt::new(3.0, 2.0)),
		]);
		Ok(())
	}

	#[test]
	fn test_horizontal_and_vertical_hold_the_other_axis_12() -> Outcome<()> {
		let p = res!(path_data("M 1 2 H 5 V 8 h -1 v -1"));
		assert_eq!(p.segs()[1], Seg::LineTo(Pt::new(5.0, 2.0)));
		assert_eq!(p.segs()[2], Seg::LineTo(Pt::new(5.0, 8.0)));
		assert_eq!(p.segs()[3], Seg::LineTo(Pt::new(4.0, 8.0)));
		assert_eq!(p.segs()[4], Seg::LineTo(Pt::new(4.0, 7.0)));
		Ok(())
	}

	#[test]
	fn test_an_arc_stays_on_its_radius_13() -> Outcome<()> {
		// Two half-turn arcs make a circle of radius 100 about the origin. Every flattened point must
		// sit on that radius: this is the whole arc conversion -- centre, angles and all -- checked
		// against geometry rather than against itself.
		let p = res!(path_data("M 100 0 A 100 100 0 0 1 -100 0 A 100 100 0 0 1 100 0 Z"));
		let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
		let mut n = 0;
		for c in &cs {
			for q in c {
				let r = (q.x * q.x + q.y * q.y).sqrt();
				assert!((r - 100.0).abs() < 0.5, "point ({}, {}) sits at radius {}", q.x, q.y, r);
				n += 1;
			}
		}
		assert!(n > 16, "a circle of radius 100 flattened to only {} points", n);
		Ok(())
	}

	#[test]
	fn test_the_sweep_flag_picks_the_side_the_arc_bulges_14() -> Outcome<()> {
		// The same ends and radii, opposite sweeps: one arc must bow above the chord and the other
		// below. Getting this backwards mirrors every rounded shape in a drawing.
		let up = res!(path_data("M 0 0 A 50 50 0 0 1 100 0"));
		let dn = res!(path_data("M 0 0 A 50 50 0 0 0 100 0"));
		let mid = |p: &Path| -> f32 {
			let cs = p.flatten(&Transform::IDENTITY, TOLERANCE);
			let mut y = 0.0;
			for c in &cs {
				for q in c {
					if (q.x - 50.0).abs() < 2.0 {
						y = q.y;
					}
				}
			}
			y
		};
		assert!(mid(&up) < -40.0, "sweep 1 should bow to negative y, reached {}", mid(&up));
		assert!(mid(&dn) > 40.0, "sweep 0 should bow to positive y, reached {}", mid(&dn));
		Ok(())
	}

	#[test]
	fn test_an_arc_with_no_radius_is_a_straight_line_15() -> Outcome<()> {
		let p = res!(path_data("M 0 0 A 0 0 0 0 1 10 10"));
		assert_eq!(p.segs()[1], Seg::LineTo(Pt::new(10.0, 10.0)));
		Ok(())
	}

	#[test]
	fn test_an_arc_that_ends_where_it_starts_is_dropped_16() -> Outcome<()> {
		// The specification says so, and the conversion would divide by zero otherwise.
		let p = res!(path_data("M 5 5 A 10 10 0 1 1 5 5"));
		assert_eq!(p.segs(), &[Seg::MoveTo(Pt::new(5.0, 5.0))]);
		Ok(())
	}

	#[test]
	fn test_radii_too_small_to_reach_are_grown_17() -> Outcome<()> {
		// The ends are 100 apart but the radii say 10. The specification grows them rather than
		// failing, so the arc must still land on its endpoint.
		let p = res!(path_data("M 0 0 A 10 10 0 0 1 100 0"));
		let end = match p.segs().last() {
			Some(Seg::CubicTo(_, _, e)) => *e,
			s => return Err(err!("Expected a cubic last, found {:?}.", s; Test, Invalid)),
		};
		assert!((end.x - 100.0).abs() < 0.01 && end.y.abs() < 0.01,
			"the arc ended at ({}, {}) rather than (100, 0)", end.x, end.y);
		Ok(())
	}

	#[test]
	fn test_data_that_does_not_begin_with_a_command_is_refused_18() -> Outcome<()> {
		assert!(path_data("10 20 L 30 40").is_err());
		Ok(())
	}

	#[test]
	fn test_an_unknown_command_is_refused_19() -> Outcome<()> {
		assert!(path_data("M 0 0 X 1 1").is_err());
		Ok(())
	}

	#[test]
	fn test_a_command_missing_an_argument_is_refused_20() -> Outcome<()> {
		assert!(path_data("M 0 0 L 5").is_err());
		Ok(())
	}

	#[test]
	fn test_an_arc_flag_that_is_not_zero_or_one_is_refused_21() -> Outcome<()> {
		assert!(path_data("M 0 0 a 1 1 0 2 1 1 1").is_err());
		Ok(())
	}

	#[test]
	fn test_empty_data_is_an_empty_path_22() -> Outcome<()> {
		let p = res!(path_data("   "));
		assert!(p.is_empty());
		Ok(())
	}

	#[test]
	fn test_data_that_does_not_begin_with_a_moveto_is_refused_23() -> Outcome<()> {
		// A drawing command has nowhere to draw from until a moveto has named a pen position.
		// Quietly starting at the origin would put the shape somewhere the author never asked for.
		assert!(path_data("L 30 40").is_err());
		assert!(path_data("C 1 1 2 2 3 3").is_err());
		Ok(())
	}

	#[test]
	fn test_a_first_relative_moveto_is_absolute_24() -> Outcome<()> {
		// It is measured from a pen at the origin, so it lands on its own coordinates.
		let p = res!(path_data("m 10 20 l 1 1"));
		assert_eq!(p.segs()[0], Seg::MoveTo(Pt::new(10.0, 20.0)));
		Ok(())
	}

	#[test]
	fn test_a_line_writes_the_expected_data_25() -> Outcome<()> {
		// The hand-known case: a move to the origin and a line to (10, 0) is exactly "M0,0 L10,0".
		// Whole coordinates carry no decimal point, and a comma joins each pair, a space each
		// command.
		let p = res!(path_data("M 0 0 L 10 0"));
		assert_eq!(write_path_data(&p), "M0,0 L10,0");
		Ok(())
	}

	#[test]
	fn test_every_command_writes_its_letter_26() -> Outcome<()> {
		// One of each segment kind, so the writer's whole command vocabulary is pinned to a known
		// string.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(1.0, 2.0));
		pb.line_to(Pt::new(3.0, 4.0));
		pb.quad_to(Pt::new(5.0, 6.0), Pt::new(7.0, 8.0));
		pb.cubic_to(Pt::new(9.0, 10.0), Pt::new(11.0, 12.0), Pt::new(13.0, 14.0));
		pb.close();
		let p = res!(pb.finish());
		assert_eq!(write_path_data(&p), "M1,2 L3,4 Q5,6 7,8 C9,10 11,12 13,14 Z");
		Ok(())
	}

	#[test]
	fn test_an_empty_path_writes_an_empty_string_27() -> Outcome<()> {
		let p = res!(PathBuilder::new().finish());
		assert_eq!(write_path_data(&p), "");
		Ok(())
	}

	#[test]
	fn test_the_writer_round_trips_through_the_reader_28() -> Outcome<()> {
		// The writer is the reader's inverse: a path written to data and read back is the path it
		// began as, segment for segment. The reader is the external oracle here -- the geometry is
		// checked against the module that already reads what every drawing program writes, not
		// against the writer restated. Fractional coordinates are used deliberately, so the test
		// bites on the number formatting and not only on round integers.
		let mut pb = PathBuilder::new();
		pb.move_to(Pt::new(1.5, -2.25));
		pb.line_to(Pt::new(10.0, 0.5));
		pb.quad_to(Pt::new(12.5, 3.75), Pt::new(20.0, -1.5));
		pb.cubic_to(Pt::new(21.0, 2.0), Pt::new(23.5, 4.5), Pt::new(30.0, 0.0));
		pb.close();
		let p = res!(pb.finish());
		let back = res!(path_data(&write_path_data(&p)));
		assert_eq!(p.segs(), back.segs(), "the path did not survive the round trip");
		Ok(())
	}

	#[test]
	fn test_a_curved_shape_round_trips_29() -> Outcome<()> {
		// A whole built shape -- a rounded rectangle, all lines and cubics -- survives the round
		// trip through data and back, so the writer holds up on geometry it did not itself hand-pick.
		use crate::path::Bounds;
		let p = res!(Path::round_rect(Bounds::new(2.0, 3.0, 40.0, 25.0), 6.0));
		let back = res!(path_data(&write_path_data(&p)));
		assert_eq!(p.segs(), back.segs());
		Ok(())
	}

	#[test]
	fn test_presentation_writes_fill_and_stroke_attributes_30() -> Outcome<()> {
		use crate::stroke::{
			Cap,
			Dash,
			Join,
		};
		let pen = res!(Stroke::new(2.0))
			.with_cap(Cap::Round)
			.with_join(Join::Bevel)
			.with_dash(Dash::new(vec![4.0, 2.0]).with_offset(1.0));
		let attrs = presentation(Some(res!(Rgba::from_hex("#ff8800"))), Some((Rgba::BLACK, &pen)));
		assert!(attrs.contains("fill=\"#ff8800\""), "the fill colour, found: {}", attrs);
		assert!(attrs.contains("stroke=\"#000000\""), "the stroke colour");
		assert!(attrs.contains("stroke-width=\"2\""), "the pen width");
		assert!(attrs.contains("stroke-linecap=\"round\""), "the cap");
		assert!(attrs.contains("stroke-linejoin=\"bevel\""), "the join");
		assert!(attrs.contains("stroke-dasharray=\"4,2\""), "the dash pattern");
		assert!(attrs.contains("stroke-dashoffset=\"1\""), "the dash offset");
		Ok(())
	}

	#[test]
	fn test_presentation_says_none_for_no_fill_and_carries_alpha_31() -> Outcome<()> {
		// No fill must be stated outright, since SVG fills black by default. A translucent stroke
		// splits into a colour and a separate opacity, the form every renderer reads.
		let pen = res!(Stroke::new(1.0));
		let attrs = presentation(None, Some((Rgba::new(0, 0, 0, 128), &pen)));
		assert!(attrs.contains("fill=\"none\""), "no fill, found: {}", attrs);
		assert!(attrs.contains("stroke=\"#000000\""), "the stroke colour without its alpha");
		assert!(attrs.contains("stroke-opacity=\"0.502\""), "the alpha as an opacity, found: {}", attrs);
		Ok(())
	}
}
