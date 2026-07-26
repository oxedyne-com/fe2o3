//! Floating-point planar geometry: primitives and predicates for a dynamic-geometry editor.
//!
//! This module provides the continuous, real-valued counterpart to the integer UI-layout
//! types elsewhere in the crate. Where `dim`, `rect` and `shape` serve pixel-aligned widget
//! layout, this module serves geometric construction and constraint solving: the kind of
//! work a compass-and-straightedge editor performs when it places a point on a line, hangs a
//! circle off two others, or asks where two loci meet.
//!
//! The primitives are `Pt` (a point, also exported as `Point`), `Vec2` (a free vector),
//! `Line` (an infinite line), `Ray` (a half-line), `Segment` (a bounded line), `Circle` and
//! `Arc`. `Angle` and `Vec2` are first-class *types* rather than incidental pairs of floats,
//! so that a constraint and a back-solver can pass them around without re-deriving their
//! invariants each time.
//!
//! The predicates are the ones a constraint check and a back-solve both call: incidence of a
//! point on a line, intersection of line with line, line with circle, and circle with circle,
//! the foot of a perpendicular, the projection of a point onto a line or a (clamped) segment,
//! and the angle between two rays.
//!
//! # Tolerance
//!
//! Floating-point coordinates never compare exactly, so every predicate that answers a
//! yes/no or counts intersections takes an explicit `eps` tolerance, measured in the same
//! units as the coordinates (a *distance*, not a raw coordinate difference). A sensible
//! default is exposed as [`EPSILON`]. Because line and ray directions are normalised on
//! construction, the tolerance stays scale-independent: it is compared against genuine
//! perpendicular distances and against sines of angles, not against raw cross products.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    f64::consts::PI,
    ops::{
        Add,
        Div,
        Mul,
        Neg,
        Sub,
    },
};

/// The default distance tolerance for planar predicates.
///
/// Two points closer than this are treated as coincident, a point this near a line is treated
/// as lying on it, and an intersection this close to a tangent is treated as a single point.
pub const EPSILON: f64 = 1.0e-9;

/// Returns `true` when `a` and `b` are within `eps` of one another.
///
/// A small free helper so callers need not repeat the absolute-difference idiom.
pub fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

// ---------------------------------------------------------------------------------------------
// Vec2
// ---------------------------------------------------------------------------------------------

/// A free vector in the plane, with `f64` components.
///
/// Distinct from [`Pt`]: a `Vec2` is a displacement or direction, not a location. The type
/// distinction lets the arithmetic express intent -- subtracting two points yields a `Vec2`,
/// and adding a `Vec2` to a point yields a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f64,
    /// Vertical component.
    pub y: f64,
}

impl Vec2 {
    /// Creates a new vector from its components.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns the dot (scalar) product with `other`.
    pub fn dot(&self, other: &Vec2) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// Returns the `z` component of the 3D cross product, i.e. the signed area of the
    /// parallelogram spanned by the two vectors.
    ///
    /// Positive when `other` lies anticlockwise of `self`. Zero (within tolerance) means the
    /// two vectors are parallel.
    pub fn cross(&self, other: &Vec2) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// Returns the squared length, avoiding the square root where only comparison is needed.
    pub fn length_sq(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Returns the Euclidean length (magnitude).
    pub fn length(&self) -> f64 {
        self.length_sq().sqrt()
    }

    /// Returns the unit vector in the same direction.
    ///
    /// # Errors
    /// Fails when the vector is shorter than [`EPSILON`], as a zero vector has no direction.
    pub fn normalise(&self) -> Outcome<Vec2> {
        let len = self.length();
        if len < EPSILON {
            return Err(err!(
                "Cannot normalise a zero-length vector.";
            Invalid, Numeric, Range));
        }
        Ok(Vec2::new(self.x / len, self.y / len))
    }

    /// Returns the vector rotated a quarter turn anticlockwise (the left normal).
    pub fn perp(&self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    /// Returns the direction of the vector as an [`Angle`] measured from the positive `x` axis.
    pub fn angle(&self) -> Angle {
        Angle::from_radians(self.y.atan2(self.x))
    }

    /// Returns `true` when both components are within `eps` of `other`'s.
    pub fn approx_eq(&self, other: &Vec2, eps: f64) -> bool {
        (*self - *other).length() <= eps
    }
}

impl Add for Vec2 {
    type Output = Vec2;

    fn add(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;

    fn sub(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;

    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl Mul<f64> for Vec2 {
    type Output = Vec2;

    fn mul(self, scalar: f64) -> Vec2 {
        Vec2::new(self.x * scalar, self.y * scalar)
    }
}

impl Div<f64> for Vec2 {
    type Output = Vec2;

    fn div(self, scalar: f64) -> Vec2 {
        Vec2::new(self.x / scalar, self.y / scalar)
    }
}

// ---------------------------------------------------------------------------------------------
// Pt
// ---------------------------------------------------------------------------------------------

/// A point (location) in the plane, with `f64` coordinates.
///
/// Also exported as [`Point`] for callers who prefer the fuller name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

/// A fuller alias for [`Pt`].
pub type Point = Pt;

impl Pt {
    /// Creates a new point from its coordinates.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns the point as a position vector from the origin.
    pub fn to_vec(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Returns the squared distance to `other`, avoiding the square root where only
    /// comparison is needed.
    pub fn distance_sq(&self, other: &Pt) -> f64 {
        (*self - *other).length_sq()
    }

    /// Returns the Euclidean distance to `other`.
    pub fn distance(&self, other: &Pt) -> f64 {
        (*self - *other).length()
    }

    /// Returns the midpoint between this point and `other`.
    pub fn midpoint(&self, other: &Pt) -> Pt {
        Pt::new((self.x + other.x) / 2.0, (self.y + other.y) / 2.0)
    }

    /// Returns `true` when the two points are within `eps` of one another.
    pub fn approx_eq(&self, other: &Pt, eps: f64) -> bool {
        self.distance(other) <= eps
    }
}

impl Sub for Pt {
    type Output = Vec2;

    /// Point minus point is the displacement between them.
    fn sub(self, other: Pt) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

impl Add<Vec2> for Pt {
    type Output = Pt;

    /// Point plus vector is the translated point.
    fn add(self, v: Vec2) -> Pt {
        Pt::new(self.x + v.x, self.y + v.y)
    }
}

impl Sub<Vec2> for Pt {
    type Output = Pt;

    /// Point minus vector is the point translated in the opposite direction.
    fn sub(self, v: Vec2) -> Pt {
        Pt::new(self.x - v.x, self.y - v.y)
    }
}

// ---------------------------------------------------------------------------------------------
// Angle
// ---------------------------------------------------------------------------------------------

/// An angle, stored internally in radians.
///
/// A first-class type so that a bare `f64` cannot be mistaken for degrees, and so that
/// normalisation and trigonometry live in one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle {
    /// The angle in radians.
    rad: f64,
}

impl Angle {
    /// Creates an angle from a value in radians.
    pub fn from_radians(rad: f64) -> Self {
        Self { rad }
    }

    /// Creates an angle from a value in degrees.
    pub fn from_degrees(deg: f64) -> Self {
        Self { rad: deg * PI / 180.0 }
    }

    /// Returns the angle in radians.
    pub fn radians(&self) -> f64 {
        self.rad
    }

    /// Returns the angle in degrees.
    pub fn degrees(&self) -> f64 {
        self.rad * 180.0 / PI
    }

    /// Returns the sine of the angle.
    pub fn sin(&self) -> f64 {
        self.rad.sin()
    }

    /// Returns the cosine of the angle.
    pub fn cos(&self) -> f64 {
        self.rad.cos()
    }

    /// Returns the angle normalised into the half-open interval `[0, 2π)`.
    pub fn normalised(&self) -> Angle {
        let two_pi = 2.0 * PI;
        let mut r = self.rad % two_pi;
        if r < 0.0 {
            r += two_pi;
        }
        Angle::from_radians(r)
    }

    /// Returns `true` when the two angles are within `eps` radians of one another, comparing
    /// on the circle so that values astride the `2π` wrap are still recognised as equal.
    pub fn approx_eq(&self, other: &Angle, eps: f64) -> bool {
        let two_pi = 2.0 * PI;
        let mut d = (self.rad - other.rad).abs() % two_pi;
        if d > PI {
            d = two_pi - d;
        }
        d <= eps
    }
}

// ---------------------------------------------------------------------------------------------
// Line
// ---------------------------------------------------------------------------------------------

/// An infinite line, stored as a point on the line and a unit direction.
///
/// The direction is normalised on construction, which keeps perpendicular-distance and
/// angle tolerances scale-independent.
#[derive(Clone, Copy, Debug)]
pub struct Line {
    /// A point through which the line passes.
    pub origin: Pt,
    /// The unit direction of the line.
    pub dir: Vec2,
}

impl Line {
    /// Creates a line through `origin` in direction `dir`.
    ///
    /// # Errors
    /// Fails when `dir` has zero length, as a line needs a direction.
    pub fn new(origin: Pt, dir: Vec2) -> Outcome<Self> {
        let unit = res!(dir.normalise());
        Ok(Self { origin, dir: unit })
    }

    /// Creates a line through the two given points.
    ///
    /// # Errors
    /// Fails when the two points are within [`EPSILON`] of each other, as coincident points
    /// do not determine a line.
    pub fn through(a: Pt, b: Pt) -> Outcome<Self> {
        if a.approx_eq(&b, EPSILON) {
            return Err(err!(
                "Cannot build a line through two coincident points {:?} and {:?}.", a, b;
            Invalid, Input, Range));
        }
        Line::new(a, b - a)
    }

    /// Returns the signed perpendicular distance from `pt` to the line.
    ///
    /// The sign follows the left normal: positive when `pt` lies anticlockwise of the
    /// direction.
    pub fn signed_distance(&self, pt: &Pt) -> f64 {
        let w = *pt - self.origin;
        w.cross(&self.dir).neg()
    }

    /// Returns the (unsigned) perpendicular distance from `pt` to the line.
    pub fn distance_to(&self, pt: &Pt) -> f64 {
        let w = *pt - self.origin;
        w.cross(&self.dir).abs()
    }

    /// Returns `true` when `pt` lies on the line, within perpendicular distance `eps`.
    pub fn contains(&self, pt: &Pt, eps: f64) -> bool {
        self.distance_to(pt) <= eps
    }

    /// Returns the foot of the perpendicular dropped from `pt` onto the line: the closest
    /// point of the line to `pt`.
    pub fn foot_of_perpendicular(&self, pt: &Pt) -> Pt {
        let w = *pt - self.origin;
        let t = w.dot(&self.dir); // Projection parameter along the unit direction.
        self.origin + self.dir * t
    }

    /// Returns the orthogonal projection of `pt` onto the line.
    ///
    /// For an infinite line this is exactly the foot of the perpendicular; the alias exists
    /// so callers can name the operation as a projection.
    pub fn project_point(&self, pt: &Pt) -> Pt {
        self.foot_of_perpendicular(pt)
    }

    /// Intersects this line with `other`.
    ///
    /// Returns a single point, or reports the lines parallel (never meeting) or coincident
    /// (meeting everywhere). Parallelism is judged by the sine of the angle between the unit
    /// directions against `eps`; coincidence additionally requires `other`'s origin to lie
    /// within `eps` of this line.
    pub fn intersect_line(&self, other: &Line, eps: f64) -> LineIntersect {
        let denom = self.dir.cross(&other.dir); // sin of the angle between unit directions.
        if denom.abs() <= eps {
            // Directions parallel; decide coincident versus strictly parallel.
            if self.distance_to(&other.origin) <= eps {
                return LineIntersect::Coincident;
            }
            return LineIntersect::Parallel;
        }
        let w = other.origin - self.origin;
        let t = w.cross(&other.dir) / denom; // Parameter along this line.
        LineIntersect::Point(self.origin + self.dir * t)
    }

    /// Intersects this line with `circle`, returning zero, one (tangent) or two points.
    ///
    /// The point pair is ordered along the line's direction (the smaller parameter first).
    pub fn intersect_circle(&self, circle: &Circle, eps: f64) -> LineCircleIntersect {
        // Parameter of the foot of the perpendicular from the centre onto the line.
        let w = circle.centre - self.origin;
        let t0 = w.dot(&self.dir);
        let foot = self.origin + self.dir * t0;
        let d = foot.distance(&circle.centre); // Distance from centre to the line.
        if d > circle.radius + eps {
            return LineCircleIntersect::None;
        }
        if approx_eq(d, circle.radius, eps) {
            return LineCircleIntersect::Tangent(foot);
        }
        // Half-chord length; clamp the radicand to guard against a tiny negative from noise.
        let h = (circle.radius * circle.radius - d * d).max(0.0).sqrt();
        let p0 = self.origin + self.dir * (t0 - h);
        let p1 = self.origin + self.dir * (t0 + h);
        LineCircleIntersect::Secant(p0, p1)
    }
}

/// The outcome of intersecting two [`Line`]s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineIntersect {
    /// The lines cross at exactly one point.
    Point(Pt),
    /// The lines are parallel and distinct, so they never meet.
    Parallel,
    /// The lines are the same line, so they meet everywhere.
    Coincident,
}

/// The outcome of intersecting a [`Line`] with a [`Circle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineCircleIntersect {
    /// The line misses the circle.
    None,
    /// The line touches the circle at a single point.
    Tangent(Pt),
    /// The line cuts the circle at two points, ordered along the line's direction.
    Secant(Pt, Pt),
}

// ---------------------------------------------------------------------------------------------
// Ray
// ---------------------------------------------------------------------------------------------

/// A half-line: an origin and a unit direction, extending infinitely one way.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    /// The endpoint from which the ray extends.
    pub origin: Pt,
    /// The unit direction of the ray.
    pub dir: Vec2,
}

impl Ray {
    /// Creates a ray from `origin` in direction `dir`.
    ///
    /// # Errors
    /// Fails when `dir` has zero length, as a ray needs a direction.
    pub fn new(origin: Pt, dir: Vec2) -> Outcome<Self> {
        let unit = res!(dir.normalise());
        Ok(Self { origin, dir: unit })
    }

    /// Creates a ray from `origin` pointing towards `through`.
    ///
    /// # Errors
    /// Fails when the two points are within [`EPSILON`] of each other.
    pub fn towards(origin: Pt, through: Pt) -> Outcome<Self> {
        if origin.approx_eq(&through, EPSILON) {
            return Err(err!(
                "Cannot build a ray from {:?} towards a coincident point {:?}.", origin, through;
            Invalid, Input, Range));
        }
        Ray::new(origin, through - origin)
    }

    /// Returns the angle between this ray and `other`, in the range `[0, π]`.
    ///
    /// Both rays carry unit directions, so this is the arccosine of their dot product,
    /// clamped to guard against floating-point drift past the ends of the domain.
    pub fn angle_between(&self, other: &Ray) -> Angle {
        let d = self.dir.dot(&other.dir).clamp(-1.0, 1.0);
        Angle::from_radians(d.acos())
    }
}

// ---------------------------------------------------------------------------------------------
// Segment
// ---------------------------------------------------------------------------------------------

/// A bounded line between two endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The start endpoint.
    pub a: Pt,
    /// The end endpoint.
    pub b: Pt,
}

impl Segment {
    /// Creates a segment between the two endpoints.
    ///
    /// # Errors
    /// Fails when the endpoints are within [`EPSILON`] of each other, as a degenerate segment
    /// has no direction and cannot be projected onto meaningfully.
    pub fn new(a: Pt, b: Pt) -> Outcome<Self> {
        if a.approx_eq(&b, EPSILON) {
            return Err(err!(
                "Cannot build a segment between two coincident points {:?} and {:?}.", a, b;
            Invalid, Input, Range));
        }
        Ok(Self { a, b })
    }

    /// Returns the length of the segment.
    pub fn length(&self) -> f64 {
        self.a.distance(&self.b)
    }

    /// Returns the projection of `pt` onto the segment, clamped to lie between the endpoints.
    ///
    /// Unlike the projection onto an infinite line, the parameter is clamped to `[0, 1]`, so
    /// a point beyond an end projects to that end.
    pub fn project_point(&self, pt: &Pt) -> Pt {
        let ab = self.b - self.a;
        let l2 = ab.length_sq(); // Non-zero: the constructor forbids a degenerate segment.
        let t = ((*pt - self.a).dot(&ab) / l2).clamp(0.0, 1.0);
        self.a + ab * t
    }
}

// ---------------------------------------------------------------------------------------------
// Triangle
// ---------------------------------------------------------------------------------------------

/// A triangle, stored as its three corners in the order they were given.
///
/// Its reason for existing here is [`Triangle::barycentric`] and its inverse
/// [`Triangle::point_at`], which together are the standard way to carry a point from one
/// triangle into another: read the point's weights in the first, then rebuild it from the
/// second's corners. That is how a triangulated region warps whatever lies inside it, and it
/// is also how a value sampled at three points is interpolated across the space between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    /// The first corner.
    pub a: Pt,
    /// The second corner.
    pub b: Pt,
    /// The third corner.
    pub c: Pt,
}

impl Triangle {
    /// Creates a triangle from three corners.
    ///
    /// # Errors
    /// Fails when the three corners are collinear to within [`EPSILON`], since a degenerate
    /// triangle has no interior and no barycentric coordinates.
    pub fn new(a: Pt, b: Pt, c: Pt) -> Outcome<Self> {
        let t = Self { a, b, c };
        if t.signed_area().abs() <= EPSILON {
            return Err(err!(
                "Cannot build a triangle from the collinear points {:?}, {:?} and {:?}.", a, b, c;
            Invalid, Input, Range));
        }
        Ok(t)
    }

    /// Returns twice the signed area, positive when the corners wind anticlockwise in a
    /// y-up frame. This is the cross product of two edges, so it is also the quantity a
    /// degeneracy test and an orientation test both want.
    pub fn signed_area2(&self) -> f64 {
        (self.b - self.a).cross(&(self.c - self.a))
    }

    /// Returns the signed area, positive when the corners wind anticlockwise in a y-up frame.
    pub fn signed_area(&self) -> f64 {
        self.signed_area2() / 2.0
    }

    /// Returns the area.
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    /// Returns the centroid, the average of the three corners.
    pub fn centroid(&self) -> Pt {
        Pt::new(
            (self.a.x + self.b.x + self.c.x) / 3.0,
            (self.a.y + self.b.y + self.c.y) / 3.0,
        )
    }

    /// Returns the barycentric coordinates of `pt`: the three weights that sum to one and
    /// rebuild the point from the corners.
    ///
    /// All three are non-negative exactly when the point lies inside the triangle or on its
    /// boundary, which is what [`Triangle::contains`] tests. A negative weight says which
    /// side the point fell out of, so the value is useful beyond the containment question.
    ///
    /// # Errors
    /// Fails on a degenerate triangle, which the constructor already forbids but which a
    /// caller can still produce by mutating the fields.
    pub fn barycentric(&self, pt: &Pt) -> Outcome<(f64, f64, f64)> {
        let d = self.signed_area2();
        if d.abs() <= EPSILON {
            return Err(err!(
                "Cannot take barycentric coordinates in a degenerate triangle {:?}.", self;
            Invalid, Input, Range));
        }
        // Each weight is the signed area of the sub-triangle opposite its corner, over the
        // whole, which is the definition rather than a rearrangement of it.
        let u = (self.b - *pt).cross(&(self.c - *pt)) / d;
        let v = (self.c - *pt).cross(&(self.a - *pt)) / d;
        Ok((u, v, 1.0 - u - v))
    }

    /// Returns the point with the given barycentric weights, the inverse of
    /// [`Triangle::barycentric`]. The weights are not required to sum to one, so a caller
    /// extrapolating deliberately outside the triangle gets the point it asked for.
    pub fn point_at(&self, u: f64, v: f64, w: f64) -> Pt {
        Pt::new(
            self.a.x * u + self.b.x * v + self.c.x * w,
            self.a.y * u + self.b.y * v + self.c.y * w,
        )
    }

    /// Whether `pt` lies inside the triangle or on its boundary, to within `eps` of the
    /// boundary. The tolerance is in barycentric weight, so it scales with the triangle
    /// rather than with the coordinate system.
    pub fn contains(&self, pt: &Pt, eps: f64) -> bool {
        match self.barycentric(pt) {
            Ok((u, v, w))   => u >= -eps && v >= -eps && w >= -eps,
            Err(_)          => false,
        }
    }

    /// Carries a point from this triangle into `other`, by reading its weights here and
    /// rebuilding it there. This is the mapping a warp is made of, and it is affine, so a
    /// straight line inside the triangle stays straight.
    ///
    /// # Errors
    /// Propagates the degeneracy failure of [`Triangle::barycentric`].
    pub fn map_point(&self, other: &Triangle, pt: &Pt) -> Outcome<Pt> {
        let (u, v, w) = res!(self.barycentric(pt));
        Ok(other.point_at(u, v, w))
    }
}

// ---------------------------------------------------------------------------------------------
// Circle
// ---------------------------------------------------------------------------------------------

/// A circle, stored as a centre and a radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    /// The centre of the circle.
    pub centre: Pt,
    /// The radius of the circle.
    pub radius: f64,
}

impl Circle {
    /// Creates a circle from a centre and a radius.
    ///
    /// # Errors
    /// Fails when the radius is not strictly positive (greater than [`EPSILON`]).
    pub fn new(centre: Pt, radius: f64) -> Outcome<Self> {
        if radius <= EPSILON {
            return Err(err!(
                "Circle radius must be positive, got {}.", radius;
            Invalid, Input, Range));
        }
        Ok(Self { centre, radius })
    }

    /// Returns `true` when `pt` lies on the circle, within `eps` of the boundary.
    pub fn contains(&self, pt: &Pt, eps: f64) -> bool {
        approx_eq(self.centre.distance(pt), self.radius, eps)
    }

    /// Intersects this circle with `line` (delegates to [`Line::intersect_circle`]).
    pub fn intersect_line(&self, line: &Line, eps: f64) -> LineCircleIntersect {
        line.intersect_circle(self, eps)
    }

    /// Intersects this circle with `other`.
    ///
    /// Returns zero points (separate, one wholly inside the other, or concentric), a single
    /// tangent point, or two points. Two identical circles are reported as coincident, having
    /// infinitely many common points.
    pub fn intersect_circle(&self, other: &Circle, eps: f64) -> CircleIntersect {
        let between = other.centre - self.centre;
        let d = between.length(); // Distance between centres.
        let r0 = self.radius;
        let r1 = other.radius;
        if d <= eps {
            // Concentric centres.
            if approx_eq(r0, r1, eps) {
                return CircleIntersect::Coincident;
            }
            return CircleIntersect::None;
        }
        if d > r0 + r1 + eps {
            return CircleIntersect::None; // Too far apart to meet.
        }
        if d < (r0 - r1).abs() - eps {
            return CircleIntersect::None; // One circle lies wholly inside the other.
        }
        // Tangent: externally when d == r0 + r1, internally when d == |r0 - r1|.
        if approx_eq(d, r0 + r1, eps) || approx_eq(d, (r0 - r1).abs(), eps) {
            let t = r0 / d;
            return CircleIntersect::Tangent(self.centre + between * t);
        }
        // Two intersection points. `a` is the distance from this centre to the chord midpoint
        // along the line of centres; `h` is the half-chord perpendicular to it.
        let a = (d * d + r0 * r0 - r1 * r1) / (2.0 * d);
        let h = (r0 * r0 - a * a).max(0.0).sqrt();
        let unit = between / d; // Unit vector along the line of centres.
        let mid = self.centre + unit * a; // Midpoint of the common chord.
        let perp = unit.perp() * h; // Half-chord offset.
        CircleIntersect::Two(mid + perp, mid - perp)
    }
}

/// The outcome of intersecting two [`Circle`]s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CircleIntersect {
    /// The circles do not meet (separate, nested, or concentric with unequal radii).
    None,
    /// The circles touch at a single point.
    Tangent(Pt),
    /// The circles cross at two points.
    Two(Pt, Pt),
    /// The circles are identical, meeting at every point.
    Coincident,
}

// ---------------------------------------------------------------------------------------------
// Arc
// ---------------------------------------------------------------------------------------------

/// An arc: a portion of a circle swept anticlockwise from a start angle to an end angle.
///
/// The angles are measured from the positive `x` axis about the circle's centre. The swept
/// interval runs anticlockwise from `start` to `end`; when `end` precedes `start` the arc
/// wraps through `2π`.
#[derive(Clone, Copy, Debug)]
pub struct Arc {
    /// The circle the arc lies on.
    pub circle: Circle,
    /// The start angle, measured anticlockwise from the positive `x` axis.
    pub start: Angle,
    /// The end angle, measured anticlockwise from the positive `x` axis.
    pub end: Angle,
}

impl Arc {
    /// Creates an arc on `circle` swept anticlockwise from `start` to `end`.
    pub fn new(circle: Circle, start: Angle, end: Angle) -> Self {
        Self { circle, start, end }
    }

    /// Returns the point on the circle at the given angle.
    fn point_at(&self, ang: &Angle) -> Pt {
        self.circle.centre + Vec2::new(ang.cos(), ang.sin()) * self.circle.radius
    }

    /// Returns the point at the start of the arc.
    pub fn start_point(&self) -> Pt {
        self.point_at(&self.start)
    }

    /// Returns the point at the end of the arc.
    pub fn end_point(&self) -> Pt {
        self.point_at(&self.end)
    }

    /// Returns the anticlockwise angular sweep of the arc, as an [`Angle`] in `[0, 2π)`.
    pub fn sweep(&self) -> Angle {
        Angle::from_radians(self.end.radians() - self.start.radians()).normalised()
    }

    /// Returns `true` when the given angle lies within the arc's anticlockwise sweep.
    ///
    /// The `eps` tolerance, in radians, widens each end of the sweep so an angle sitting
    /// exactly on an endpoint counts as inside.
    pub fn contains_angle(&self, ang: &Angle, eps: f64) -> bool {
        let sweep = self.sweep().radians();
        // Offset of the query angle from the start, brought into [0, 2π).
        let offset = Angle::from_radians(ang.radians() - self.start.radians())
            .normalised()
            .radians();
        offset <= sweep + eps || offset >= 2.0 * PI - eps
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The tolerance used to check oracle values in these tests.
    const T: f64 = 1.0e-9;

    // -- point-on-line incidence --------------------------------------------------------------

    #[test]
    fn test_point_on_line_00() -> Outcome<()> {
        // The line y = x, built through the origin and (1, 1).
        let line = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 1.0)));
        // (2, 2) lies on it.
        assert!(line.contains(&Pt::new(2.0, 2.0), T));
        // (2, 3) does not.
        assert!(!line.contains(&Pt::new(2.0, 3.0), T));
        Ok(())
    }

    #[test]
    fn test_point_on_line_coincident_endpoints() {
        // A line cannot be built through two coincident points.
        let res = Line::through(Pt::new(1.0, 1.0), Pt::new(1.0, 1.0));
        assert!(res.is_err());
    }

    // -- line-line intersection ---------------------------------------------------------------

    #[test]
    fn test_line_line_point_00() -> Outcome<()> {
        // y = x and y = -x + 2 meet at (1, 1).
        let l1 = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 1.0)));
        let l2 = res!(Line::through(Pt::new(0.0, 2.0), Pt::new(1.0, 1.0)));
        match l1.intersect_line(&l2, T) {
            LineIntersect::Point(p) => {
                assert!(p.approx_eq(&Pt::new(1.0, 1.0), T));
            },
            other => return Err(err!("Expected a single point, got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn test_line_line_parallel() -> Outcome<()> {
        // y = x and y = x + 1 are parallel and distinct.
        let l1 = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 1.0)));
        let l2 = res!(Line::through(Pt::new(0.0, 1.0), Pt::new(1.0, 2.0)));
        assert_eq!(l1.intersect_line(&l2, T), LineIntersect::Parallel);
        Ok(())
    }

    #[test]
    fn test_line_line_coincident() -> Outcome<()> {
        // y = x described two different ways is the same line.
        let l1 = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 1.0)));
        let l2 = res!(Line::through(Pt::new(2.0, 2.0), Pt::new(3.0, 3.0)));
        assert_eq!(l1.intersect_line(&l2, T), LineIntersect::Coincident);
        Ok(())
    }

    // -- line-circle intersection -------------------------------------------------------------

    #[test]
    fn test_line_circle_secant() -> Outcome<()> {
        // The unit circle and the line x = 0 meet at (0, ±1).
        let circle = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let line = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(0.0, 1.0)));
        match line.intersect_circle(&circle, T) {
            LineCircleIntersect::Secant(p0, p1) => {
                // Ordered along +y: (0, -1) then (0, 1).
                assert!(p0.approx_eq(&Pt::new(0.0, -1.0), T));
                assert!(p1.approx_eq(&Pt::new(0.0, 1.0), T));
            },
            other => return Err(err!("Expected two points, got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn test_line_circle_tangent() -> Outcome<()> {
        // The unit circle and the line y = 1 touch at (0, 1).
        let circle = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let line = res!(Line::through(Pt::new(0.0, 1.0), Pt::new(1.0, 1.0)));
        match line.intersect_circle(&circle, T) {
            LineCircleIntersect::Tangent(p) => {
                assert!(p.approx_eq(&Pt::new(0.0, 1.0), T));
            },
            other => return Err(err!("Expected a tangent point, got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn test_line_circle_none() -> Outcome<()> {
        // The unit circle and the line y = 2 do not meet.
        let circle = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let line = res!(Line::through(Pt::new(0.0, 2.0), Pt::new(1.0, 2.0)));
        assert_eq!(line.intersect_circle(&circle, T), LineCircleIntersect::None);
        Ok(())
    }

    // -- circle-circle intersection -----------------------------------------------------------

    #[test]
    fn test_circle_circle_two() -> Outcome<()> {
        // Unit circles centred at (0, 0) and (1, 0) meet at (0.5, ±√3/2).
        let c0 = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let c1 = res!(Circle::new(Pt::new(1.0, 0.0), 1.0));
        let root3_2 = 3.0_f64.sqrt() / 2.0;
        match c0.intersect_circle(&c1, T) {
            CircleIntersect::Two(p0, p1) => {
                // The two returned points, in either order, are (0.5, ±√3/2).
                let up = Pt::new(0.5, root3_2);
                let dn = Pt::new(0.5, -root3_2);
                let ok = (p0.approx_eq(&up, T) && p1.approx_eq(&dn, T))
                    || (p0.approx_eq(&dn, T) && p1.approx_eq(&up, T));
                assert!(ok, "Got {:?} and {:?}.", p0, p1);
            },
            other => return Err(err!("Expected two points, got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn test_circle_circle_tangent() -> Outcome<()> {
        // Unit circles at (0, 0) and (2, 0) touch externally at (1, 0).
        let c0 = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let c1 = res!(Circle::new(Pt::new(2.0, 0.0), 1.0));
        match c0.intersect_circle(&c1, T) {
            CircleIntersect::Tangent(p) => {
                assert!(p.approx_eq(&Pt::new(1.0, 0.0), T));
            },
            other => return Err(err!("Expected a tangent point, got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn test_circle_circle_none() -> Outcome<()> {
        // Unit circles at (0, 0) and (5, 0) are too far apart to meet.
        let c0 = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let c1 = res!(Circle::new(Pt::new(5.0, 0.0), 1.0));
        assert_eq!(c0.intersect_circle(&c1, T), CircleIntersect::None);
        Ok(())
    }

    #[test]
    fn test_circle_circle_coincident() -> Outcome<()> {
        // The same circle described twice meets everywhere.
        let c0 = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        let c1 = res!(Circle::new(Pt::new(0.0, 0.0), 1.0));
        assert_eq!(c0.intersect_circle(&c1, T), CircleIntersect::Coincident);
        Ok(())
    }

    // -- foot of perpendicular ----------------------------------------------------------------

    #[test]
    fn test_foot_of_perpendicular_00() -> Outcome<()> {
        // The foot of the perpendicular from (0, 2) to y = 0 is (0, 0).
        let line = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 0.0)));
        let foot = line.foot_of_perpendicular(&Pt::new(0.0, 2.0));
        assert!(foot.approx_eq(&Pt::new(0.0, 0.0), T));
        Ok(())
    }

    #[test]
    fn test_foot_of_perpendicular_diagonal() -> Outcome<()> {
        // The foot of the perpendicular from (0, 2) onto y = x is (1, 1).
        let line = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 1.0)));
        let foot = line.foot_of_perpendicular(&Pt::new(0.0, 2.0));
        assert!(foot.approx_eq(&Pt::new(1.0, 1.0), T));
        Ok(())
    }

    // -- projection onto a line ---------------------------------------------------------------

    #[test]
    fn test_project_point_onto_line() -> Outcome<()> {
        // Projecting (2, 5) onto y = 0 gives (2, 0).
        let line = res!(Line::through(Pt::new(0.0, 0.0), Pt::new(1.0, 0.0)));
        let p = line.project_point(&Pt::new(2.0, 5.0));
        assert!(p.approx_eq(&Pt::new(2.0, 0.0), T));
        Ok(())
    }

    // -- projection onto a segment (clamped) --------------------------------------------------

    #[test]
    fn test_project_point_onto_segment_interior() -> Outcome<()> {
        // Projecting (0.5, 2) onto the segment (0,0)-(1,0) lands at (0.5, 0).
        let seg = res!(Segment::new(Pt::new(0.0, 0.0), Pt::new(1.0, 0.0)));
        let p = seg.project_point(&Pt::new(0.5, 2.0));
        assert!(p.approx_eq(&Pt::new(0.5, 0.0), T));
        Ok(())
    }

    #[test]
    fn test_project_point_onto_segment_clamped_ends() -> Outcome<()> {
        // Beyond an end, the projection clamps to that endpoint.
        let seg = res!(Segment::new(Pt::new(0.0, 0.0), Pt::new(1.0, 0.0)));
        let past_b = seg.project_point(&Pt::new(2.0, 2.0));
        assert!(past_b.approx_eq(&Pt::new(1.0, 0.0), T));
        let past_a = seg.project_point(&Pt::new(-1.0, 2.0));
        assert!(past_a.approx_eq(&Pt::new(0.0, 0.0), T));
        Ok(())
    }

    // -- angle between two rays ---------------------------------------------------------------

    #[test]
    fn test_angle_between_rays_right_angle() -> Outcome<()> {
        // The +x and +y rays are a quarter turn apart: π/2.
        let rx = res!(Ray::new(Pt::new(0.0, 0.0), Vec2::new(1.0, 0.0)));
        let ry = res!(Ray::new(Pt::new(0.0, 0.0), Vec2::new(0.0, 1.0)));
        let ang = rx.angle_between(&ry);
        assert!(approx_eq(ang.radians(), PI / 2.0, T), "Got {} rad.", ang.radians());
        Ok(())
    }

    #[test]
    fn test_angle_between_rays_opposite() -> Outcome<()> {
        // The +x and -x rays are a half turn apart: π (the degenerate straight-angle case).
        let rx = res!(Ray::new(Pt::new(0.0, 0.0), Vec2::new(1.0, 0.0)));
        let rev = res!(Ray::new(Pt::new(0.0, 0.0), Vec2::new(-1.0, 0.0)));
        let ang = rx.angle_between(&rev);
        assert!(approx_eq(ang.radians(), PI, T), "Got {} rad.", ang.radians());
        Ok(())
    }

    // -- supporting types ---------------------------------------------------------------------

    #[test]
    fn test_vec2_normalise_zero_fails() {
        // A zero vector has no direction and cannot be normalised.
        let res = Vec2::new(0.0, 0.0).normalise();
        assert!(res.is_err());
    }

    #[test]
    fn test_circle_zero_radius_fails() {
        // A circle needs a positive radius.
        let res = Circle::new(Pt::new(0.0, 0.0), 0.0);
        assert!(res.is_err());
    }

    #[test]
    fn test_angle_degrees_radians() {
        // 180 degrees is π radians, and the conversion round-trips.
        let a = Angle::from_degrees(180.0);
        assert!(approx_eq(a.radians(), PI, T));
        assert!(approx_eq(a.degrees(), 180.0, T));
    }

    // -- triangle: area, barycentric coordinates and the affine map ----------------------------

    #[test]
    fn test_triangle_area_00() -> Outcome<()> {
        // The 3-4-5 right triangle: area 6 by the half-base-times-height oracle.
        let t = res!(Triangle::new(Pt::new(0.0, 0.0), Pt::new(4.0, 0.0), Pt::new(0.0, 3.0)));
        assert!(approx_eq(t.area(), 6.0, T));
        // Anticlockwise in a y-up frame, so the signed area is positive.
        assert!(t.signed_area() > 0.0);
        // Reversing two corners reverses the winding and nothing else.
        let r = res!(Triangle::new(Pt::new(0.0, 0.0), Pt::new(0.0, 3.0), Pt::new(4.0, 0.0)));
        assert!(approx_eq(r.signed_area(), -6.0, T));
        assert!(approx_eq(r.area(), 6.0, T));
        Ok(())
    }

    #[test]
    fn test_triangle_collinear_fails() {
        // Three points on the line y = 2x have no interior.
        let res = Triangle::new(Pt::new(0.0, 0.0), Pt::new(1.0, 2.0), Pt::new(3.0, 6.0));
        assert!(res.is_err());
    }

    #[test]
    fn test_triangle_barycentric_corners_and_centroid() -> Outcome<()> {
        let (a, b, c) = (Pt::new(0.0, 0.0), Pt::new(4.0, 0.0), Pt::new(0.0, 3.0));
        let t = res!(Triangle::new(a, b, c));
        // Each corner carries all of its own weight.
        for (p, want) in [(a, (1.0, 0.0, 0.0)), (b, (0.0, 1.0, 0.0)), (c, (0.0, 0.0, 1.0))] {
            let (u, v, w) = res!(t.barycentric(&p));
            assert!(approx_eq(u, want.0, T) && approx_eq(v, want.1, T) && approx_eq(w, want.2, T),
                "corner {:?} gave ({}, {}, {})", p, u, v, w);
        }
        // The centroid is a third of each.
        let (u, v, w) = res!(t.barycentric(&t.centroid()));
        assert!(approx_eq(u, 1.0 / 3.0, T) && approx_eq(v, 1.0 / 3.0, T) && approx_eq(w, 1.0 / 3.0, T));
        // A midpoint of an edge splits its two corners and excludes the third.
        let (u, v, w) = res!(t.barycentric(&a.midpoint(&b)));
        assert!(approx_eq(u, 0.5, T) && approx_eq(v, 0.5, T) && approx_eq(w, 0.0, T));
        Ok(())
    }

    #[test]
    fn test_triangle_contains_00() -> Outcome<()> {
        let t = res!(Triangle::new(Pt::new(0.0, 0.0), Pt::new(4.0, 0.0), Pt::new(0.0, 3.0)));
        // Inside, on an edge, on a corner, and outside past the hypotenuse.
        assert!(t.contains(&Pt::new(1.0, 1.0), T));
        assert!(t.contains(&Pt::new(2.0, 0.0), T));
        assert!(t.contains(&Pt::new(0.0, 3.0), T));
        assert!(!t.contains(&Pt::new(3.0, 3.0), T));
        assert!(!t.contains(&Pt::new(-0.5, 1.0), T));
        Ok(())
    }

    #[test]
    fn test_triangle_barycentric_round_trips() -> Outcome<()> {
        // Reading a point's weights and rebuilding it returns the same point.
        let t = res!(Triangle::new(Pt::new(-2.0, 1.0), Pt::new(5.0, -3.0), Pt::new(1.0, 6.0)));
        for p in [Pt::new(1.0, 1.0), Pt::new(0.0, 0.0), Pt::new(4.0, 2.0)] {
            let (u, v, w) = res!(t.barycentric(&p));
            assert!(approx_eq(u + v + w, 1.0, T), "weights must sum to one");
            let q = t.point_at(u, v, w);
            assert!(p.approx_eq(&q, 1.0e-9), "{:?} rebuilt as {:?}", p, q);
        }
        Ok(())
    }

    #[test]
    fn test_triangle_map_point_is_affine() -> Outcome<()> {
        // Map the unit right triangle onto one scaled by two in x, three in y and shifted.
        // The image of (0.25, 0.5) is therefore (10 + 0.5, 20 + 1.5) by hand.
        let src = res!(Triangle::new(Pt::new(0.0, 0.0), Pt::new(1.0, 0.0), Pt::new(0.0, 1.0)));
        let dst = res!(Triangle::new(Pt::new(10.0, 20.0), Pt::new(12.0, 20.0), Pt::new(10.0, 23.0)));
        let q = res!(src.map_point(&dst, &Pt::new(0.25, 0.5)));
        assert!(q.approx_eq(&Pt::new(10.5, 21.5), 1.0e-9), "mapped to {:?}", q);
        // Being affine, the image of a midpoint is the midpoint of the images.
        let (p0, p1) = (Pt::new(0.1, 0.1), Pt::new(0.6, 0.2));
        let (i0, i1) = (res!(src.map_point(&dst, &p0)), res!(src.map_point(&dst, &p1)));
        let mid = res!(src.map_point(&dst, &p0.midpoint(&p1)));
        assert!(mid.approx_eq(&i0.midpoint(&i1), 1.0e-9));
        Ok(())
    }
}
