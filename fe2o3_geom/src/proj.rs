//! Map projections: turning a position on a sphere into a position on a page, and back.
//!
//! The projection functions work on the plane every projection reference uses: right-handed,
//! `y` increasing northward, scaled by a sphere radius the caller passes.  Equal Earth draws
//! the whole world at once, Web Mercator draws a flat map that street maps have made familiar,
//! and orthographic draws the half of the world a viewer is looking at, as a globe.  Each is
//! checked against PROJ 9.7.1.
//!
//! A [`Viewport`] puts the last two on a screen: one camera -- a centre, metres per pixel,
//! a heading -- read by either projection, with `y` downward as a canvas has it.  It projects
//! whole rings of unit vectors at a time, clipped and ready to paint, into a [`ScreenPaths`].
//! Further projections belong beside these in this module rather than in a module of their
//! own.

use crate::planar::Pt;

use oxedyne_fe2o3_core::prelude::*;

use std::f64::consts::{
    FRAC_PI_2,
    PI,
    TAU,
};

// ---------------------------------------------------------------------------------------------
// Equal Earth
// ---------------------------------------------------------------------------------------------

/// First polynomial coefficient of the Equal Earth projection.
///
/// The four coefficients are those published by Šavrič, Patterson and Jenny, *The Equal
/// Earth map projection*, International Journal of Geographical Information Science 33(3),
/// 2019, and are the same values PROJ carries for `+proj=eqearth`.
pub const EQUAL_EARTH_A1: f64 = 1.340264;

/// Second polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A2: f64 = -0.081106;

/// Third polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A3: f64 = 0.000893;

/// Fourth polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A4: f64 = 0.003796;

/// Half the width of the projected world on a unit sphere.
///
/// The value of `x` at the equator on the antimeridian, which is where the map is widest.
pub const EQUAL_EARTH_HALF_WIDTH: f64 = 2.706_629_983_696_074_3;

/// Half the height of the projected world on a unit sphere.
///
/// The value of `y` at a pole. The world is therefore 2.0546 times as wide as it is tall.
pub const EQUAL_EARTH_HALF_HEIGHT: f64 = 1.317_362_759_157_413;

/// Projects a position onto the Equal Earth plane.
///
/// Equal Earth is pseudocylindrical and equal-area: every square metre of ground occupies
/// the same area on the page wherever it is, which is what makes it honest about how much
/// of the world a continent is. Parallels are straight and evenly spaced enough to read,
/// meridians are curved, and the shape of the land is close to what a person expects.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lon_0` - The central meridian, in degrees. Zero for Greenwich down the middle.
/// * `radius` - The radius of the sphere, in whatever units the result should come out in.
///
/// # Returns
/// The projected point, `x` eastward and `y` northward from the map's centre.
///
/// Latitude is clamped to the poles and longitude wrapped into a half turn either side of
/// `lon_0`, so a fix arriving one part in a billion outside its range projects rather than
/// producing a not-a-number. A non-finite argument yields a non-finite result: the caller
/// is the one that knows whether that is a hole in its data or an error.
pub fn equal_earth(lat: f64, lng: f64, lon_0: f64, radius: f64) -> Pt {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();

    // The parametric latitude, on which the whole projection is a polynomial.
    let theta = ((3.0f64).sqrt() / 2.0 * phi.sin()).clamp(-1.0, 1.0).asin();
    let t2 = theta * theta;
    let t3 = t2 * theta;
    let t6 = t3 * t3;
    let t7 = t6 * theta;
    let t8 = t6 * t2;
    let t9 = t8 * theta;

    // The denominator of x is dy/dθ, which is what makes the projection equal-area.
    let dy = 9.0 * EQUAL_EARTH_A4 * t8
        + 7.0 * EQUAL_EARTH_A3 * t6
        + 3.0 * EQUAL_EARTH_A2 * t2
        + EQUAL_EARTH_A1;

    let x = 2.0 * (3.0f64).sqrt() * lam * theta.cos() / (3.0 * dy);
    let y = EQUAL_EARTH_A4 * t9
        + EQUAL_EARTH_A3 * t7
        + EQUAL_EARTH_A2 * t3
        + EQUAL_EARTH_A1 * theta;

    Pt::new(radius * x, radius * y)
}

/// Projects a position onto the Equal Earth plane of a unit sphere, Greenwich centred.
///
/// The bare form, for a caller that will scale the result itself.
pub fn equal_earth_unit(lat: f64, lng: f64) -> Pt {
    equal_earth(lat, lng, 0.0, 1.0)
}

/// The sphere radius that makes an Equal Earth map `2 * half_width` across.
///
/// A map is usually specified by the box it has to fill rather than by the size of the
/// world it draws, and this converts one into the other. The height that follows is
/// `half_width * EQUAL_EARTH_HALF_HEIGHT / EQUAL_EARTH_HALF_WIDTH`, or very nearly half
/// the width.
pub fn equal_earth_radius_for_half_width(half_width: f64) -> f64 {
    half_width / EQUAL_EARTH_HALF_WIDTH
}

// ---------------------------------------------------------------------------------------------
// Orthographic
// ---------------------------------------------------------------------------------------------

/// Half the width of the orthographic projection on a unit sphere.
///
/// The projection is the sphere seen from infinitely far away, so the map is the sphere's own
/// disc and its half width is the radius. A caller wanting a globe `2 * half_width` across
/// therefore passes `half_width` as the radius; there is no conversion to do.
pub const ORTHOGRAPHIC_HALF_WIDTH: f64 = 1.0;

/// Projects a position onto the orthographic plane, as seen from above `lat_0`, `lon_0`.
///
/// The orthographic projection is what a sphere looks like from far enough away that the rays
/// arrive parallel: the near hemisphere fills a disc, foreshortened towards the rim, and the
/// far hemisphere is behind it. It is neither equal-area nor conformal, and it is the only
/// projection that does not have to choose, because it is not flattening the world at all —
/// it is drawing the object. That makes it the honest one for a globe a viewer turns.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lat_0` - The latitude the viewer is above, in degrees.
/// * `lon_0` - The longitude the viewer is above, in degrees.
/// * `radius` - The radius of the sphere, in whatever units the result should come out in.
///
/// # Returns
/// The projected point, `x` rightward and `y` upward from the centre of the disc.
///
/// **A position on the far hemisphere still projects**, onto the point of the near hemisphere
/// directly in front of it, because the formula cannot tell them apart. Ask
/// [`orthographic_cos_c`] which side of the globe a position is on; it is separate so that a
/// caller clipping a coastline can interpolate along an edge to the horizon, where that
/// cosine is zero, rather than being handed a hole.
pub fn orthographic(lat: f64, lng: f64, lat_0: f64, lon_0: f64, radius: f64) -> Pt {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let phi_0 = lat_0.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();

    let x = phi.cos() * lam.sin();
    let y = phi_0.cos() * phi.sin() - phi_0.sin() * phi.cos() * lam.cos();

    Pt::new(radius * x, radius * y)
}

/// The cosine of the angle between a position and the centre of an orthographic projection.
///
/// One where the position is under the viewer, zero on the horizon, and negative on the far
/// side of the globe. A caller drawing a coastline keeps the vertices where this is positive
/// and finds the horizon crossing by interpolating an edge to where it is zero.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lat_0` - The latitude the viewer is above, in degrees.
/// * `lon_0` - The longitude the viewer is above, in degrees.
pub fn orthographic_cos_c(lat: f64, lng: f64, lat_0: f64, lon_0: f64) -> f64 {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let phi_0 = lat_0.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();
    phi_0.sin() * phi.sin() + phi_0.cos() * phi.cos() * lam.cos()
}

/// Brings an angle in degrees into the half turn either side of zero.
///
/// A longitude difference of 190° east is 170° west, and a map has to draw it there.
fn wrap_half_turn(deg: f64) -> f64 {
    if !deg.is_finite() {
        return deg;
    }
    let full = 360.0;
    let mut d = deg % full;
    if d > 180.0 {
        d -= full;
    } else if d < -180.0 {
        d += full;
    }
    d
}

// ---------------------------------------------------------------------------------------------
// Web Mercator
// ---------------------------------------------------------------------------------------------

pub const EARTH_RADIUS_M: f64 = 6_371_008.8;                    // IUGG mean radius
pub const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_59;    // atan(sinh(pi)): the map is square

/// Projects a position onto the spherical Web Mercator plane.
///
/// Web Mercator is conformal, so a small square of ground stays square on the page at every
/// latitude, and it is the projection street maps have taught everyone to read.  Its price is
/// the poles, which are infinitely far away: latitude is clamped to
/// [`WEB_MERCATOR_MAX_LAT`], where the map is exactly as tall as it is wide, `2 * pi * radius`.
/// Longitude is wrapped into a half turn either side of `lon_0`.
pub fn web_mercator(lat: f64, lng: f64, lon_0: f64, radius: f64) -> Pt {
    let phi = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();
    // PROJ writes the ordinate as asinh(tan(phi)); it equals ln(tan(pi/4 + phi/2)).
    Pt::new(radius * lam, radius * phi.tan().asinh())
}

/// The position a Web Mercator point came from, as `(lat, lng)` in degrees.
///
/// Longitude comes back wrapped into `[-180, 180]`, so a point on a copy of the world to either
/// side of the central one answers with the same position as its twin.  A point above or below
/// the square map still answers, with a latitude past [`WEB_MERCATOR_MAX_LAT`]; a caller that
/// draws only the square asks [`Viewport::inverse`] instead.
pub fn web_mercator_inverse(pt: Pt, lon_0: f64, radius: f64) -> (f64, f64) {
    let lat = (pt.y / radius).sinh().atan().to_degrees();
    let lng = wrap_half_turn(lon_0 + (pt.x / radius).to_degrees());
    (lat, lng)
}

/// The position an orthographic point came from, as `(lat, lng)` in degrees.
///
/// The inverse of [`orthographic`] for the near hemisphere, which is the only one a point on
/// the disc can mean.  A point off the disc is off the globe and answers `None`; one within a
/// part in ten billion of the rim is taken to be on it.
pub fn orthographic_inverse(pt: Pt, lat_0: f64, lon_0: f64, radius: f64) -> Option<(f64, f64)> {
    let rho = pt.x.hypot(pt.y) / radius;
    if !rho.is_finite() || rho > 1.0 + 1.0e-10 {
        return None;
    }
    let phi_0 = lat_0.clamp(-90.0, 90.0).to_radians();
    if rho < 1.0e-15 {
        return Some((lat_0.clamp(-90.0, 90.0), wrap_half_turn(lon_0)));
    }
    let sin_c = rho.min(1.0);
    let cos_c = (1.0 - sin_c * sin_c).max(0.0).sqrt();
    let (x, y) = (pt.x / radius, pt.y / radius);
    // Snyder, Map Projections: A Working Manual, equations 20-14 and 20-15.
    let phi = (cos_c * phi_0.sin() + y * sin_c * phi_0.cos() / rho).clamp(-1.0, 1.0).asin();
    let lam = (x * sin_c).atan2(rho * cos_c * phi_0.cos() - y * sin_c * phi_0.sin());
    Some((phi.to_degrees(), wrap_half_turn(lon_0 + lam.to_degrees())))
}

// ---------------------------------------------------------------------------------------------
// Unit vectors
// ---------------------------------------------------------------------------------------------

/// A position as a unit vector: `x` towards Greenwich on the equator, `z` towards the north
/// pole.  The same frame [`crate::cell`] uses.
pub fn unit_vec(lat: f64, lng: f64) -> [f64; 3] {
    let (a, b) = (lat.to_radians(), lng.to_radians());
    let c = a.cos();
    [c * b.cos(), c * b.sin(), a.sin()]
}

/// The position of a vector of any non-zero length, as `(lat, lng)` in degrees.
pub fn vec_lat_lng(v: &[f64; 3]) -> (f64, f64) {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let z = (v[2] / len).clamp(-1.0, 1.0);
    (z.asin().to_degrees(), v[1].atan2(v[0]).to_degrees())
}

fn dot3(a: &[f64; 3], b: &[f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

// ---------------------------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------------------------

/// The projection a [`Viewport`] draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    WebMercator,    // a flat map, north up unless turned, wrapping east-west without end
    Orthographic,   // a globe seen from above the centre
}

/// How [`Viewport::project_rings`] treats each ring it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingMode {
    Fill,       // closed ring, painted: clipped pieces are stitched along the clip edge
    Outline,    // closed ring, stroked: its closing edge is drawn, and it is cut where clipped
    Line,       // open line, stroked, cut where clipped
}

/// One camera over the sphere, shared by the flat map and the globe.
///
/// A viewport is the centre being looked at, how much ground a pixel covers there, which way
/// is up, and the size of the screen.  The two projections read the same numbers, so a caller
/// switching between them keeps the place and the scale and changes only the picture.
///
/// Screen coordinates are pixels from the top left corner, `y` downward, as a canvas has them.
/// `heading` is the compass bearing, in degrees clockwise from north, that points up the
/// screen; zero is north up.  `m_per_px` is ground metres per pixel at the centre, on a sphere
/// of [`EARTH_RADIUS_M`], so a Mercator map at 60 degrees draws the world twice as wide as the
/// same `m_per_px` does at the equator, and a cell the same size on the ground stays the same
/// size on the screen either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub kind:       Projection,
    pub lat_0:      f64,    // degrees, the centre of the screen
    pub lon_0:      f64,    // degrees
    pub heading:    f64,    // degrees clockwise from north, pointing up the screen
    pub m_per_px:   f64,    // ground metres per pixel at the centre
    pub w:          f64,    // screen width in pixels
    pub h:          f64,    // screen height in pixels
}

/// The clip and the margin around the screen, in pixels, inside which geometry is kept.
///
/// Wide enough that a stroke's own width never shows the cut, and small enough that nothing
/// far off the screen is carried.
const CLIP_MARGIN_PX: f64 = 8.0;

/// A viewport's numbers turned into the arithmetic that draws with them, once per call.
struct Frame {
    cx:         f64,        // screen centre
    cy:         f64,
    ch:         f64,        // cosine and sine of the heading
    sh:         f64,
    r_px:       f64,        // the sphere's radius in pixels (Mercator: at the equator)
    c:          [f64; 3],   // orthographic basis: centre, east, north
    e:          [f64; 3],
    n:          [f64; 3],
    cos_clip:   f64,        // orthographic clip cap, as the cosine of its radius
    rho_clip:   f64,        // and as a circle on the screen, in pixels
    lam_0:      f64,        // Mercator central meridian, radians
    y_0:        f64,        // Mercator ordinate of the centre, pixels
    x_lo:       f64,        // Mercator: the screen and its margin on the unturned plane
    x_hi:       f64,
    y_lo:       f64,
    y_hi:       f64,
}

impl Frame {
    /// A point on the unturned plane, `yd` already downward, to the screen.
    fn to_screen(&self, x: f64, yd: f64) -> (f64, f64) {
        (self.cx + x * self.ch + yd * self.sh, self.cy - x * self.sh + yd * self.ch)
    }

    /// A screen point back to the unturned plane, `yd` downward.
    fn from_screen(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (dx, dy) = (sx - self.cx, sy - self.cy);
        (dx * self.ch - dy * self.sh, dx * self.sh + dy * self.ch)
    }

    /// An orthographic vertex: where it lands on the unturned plane, and how far in front of
    /// the clip it is.  The last is linear in the vector, which is what lets a crossing be
    /// found by one division along the straight edge drawn between two vertices.
    fn ortho(&self, p: &[f64; 3]) -> (f64, f64, f64) {
        (self.r_px * dot3(p, &self.e), -self.r_px * dot3(p, &self.n), dot3(p, &self.c) - self.cos_clip)
    }

    /// A Mercator vertex's longitude from the central meridian, in radians, and its ordinate,
    /// both unscaled.  `None` for a pole, whose longitude is undefined.
    fn merc(&self, p: &[f64; 3]) -> Option<(f64, f64)> {
        if p[0] * p[0] + p[1] * p[1] < 1.0e-24 {
            return None;
        }
        let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        let y = (p[2] / len).clamp(-1.0, 1.0).atanh().clamp(-PI, PI);
        Some((wrap_pi(p[1].atan2(p[0]) - self.lam_0), y))
    }
}

/// Brings an angle in radians into the half turn either side of zero.
fn wrap_pi(a: f64) -> f64 {
    let mut d = a % TAU;
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    d
}

impl Viewport {
    /// A viewport, refusing numbers that could not draw anything.
    pub fn new(
        kind:       Projection,
        lat_0:      f64,
        lon_0:      f64,
        heading:    f64,
        m_per_px:   f64,
        w:          f64,
        h:          f64,
    )
        -> Outcome<Self>
    {
        let v = Self { kind, lat_0, lon_0, heading, m_per_px, w, h };
        res!(v.check());
        Ok(v)
    }

    fn check(&self) -> Outcome<()> {
        let finite = self.lat_0.is_finite() && self.lon_0.is_finite() && self.heading.is_finite();
        if !finite {
            return Err(err!("A viewport centred on {}, {} with heading {} is not a place.",
                self.lat_0, self.lon_0, self.heading; Invalid, Input));
        }
        if !(self.m_per_px > 0.0 && self.m_per_px.is_finite()) {
            return Err(err!("A viewport of {} m per pixel draws nothing.", self.m_per_px;
                Invalid, Input, Range));
        }
        if !(self.w > 0.0 && self.h > 0.0 && self.w.is_finite() && self.h.is_finite()) {
            return Err(err!("A viewport {} by {} pixels has no screen.", self.w, self.h;
                Invalid, Input, Range));
        }
        Ok(())
    }

    fn frame(&self) -> Frame {
        let lat_0 = self.lat_0.clamp(-90.0, 90.0);
        let (phi, lam) = (lat_0.to_radians(), self.lon_0.to_radians());
        let hd = self.heading.to_radians();
        let (cx, cy) = (self.w / 2.0, self.h / 2.0);
        let reach = cx.hypot(cy) + CLIP_MARGIN_PX;
        let mut f = Frame {
            cx, cy, ch: hd.cos(), sh: hd.sin(), r_px: 0.0,
            c: [0.0; 3], e: [0.0; 3], n: [0.0; 3], cos_clip: 0.0, rho_clip: 0.0,
            lam_0: lam, y_0: 0.0, x_lo: 0.0, x_hi: 0.0, y_lo: 0.0, y_hi: 0.0,
        };
        match self.kind {
            Projection::Orthographic => {
                f.r_px = EARTH_RADIUS_M / self.m_per_px;
                f.c = unit_vec(lat_0, self.lon_0);
                f.e = [-lam.sin(), lam.cos(), 0.0];
                f.n = [-phi.sin() * lam.cos(), -phi.sin() * lam.sin(), phi.cos()];
                // Clip to the smaller of the horizon and the circle round the screen, so that
                // close in, a ring is cut just off the screen rather than at a horizon
                // thousands of pixels away.
                if reach >= f.r_px {
                    f.cos_clip = 0.0;
                    f.rho_clip = f.r_px;
                } else {
                    let s = reach / f.r_px;
                    f.cos_clip = (1.0 - s * s).sqrt();
                    f.rho_clip = reach;
                }
            },
            Projection::WebMercator => {
                let phi_c = lat_0.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT).to_radians();
                // The Mercator scale factor at the centre is sec(phi), so the plane is drawn
                // cos(phi) as large as the ground resolution alone would say.
                f.r_px = EARTH_RADIUS_M * phi_c.cos() / self.m_per_px;
                f.y_0 = f.r_px * phi_c.tan().asinh();
                let m = CLIP_MARGIN_PX;
                let corners = [(-m, -m), (self.w + m, -m), (-m, self.h + m), (self.w + m, self.h + m)];
                let (mut xl, mut xh, mut yl, mut yh) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for (sx, sy) in corners {
                    let (x, yd) = f.from_screen(sx, sy);
                    xl = xl.min(x);
                    xh = xh.max(x);
                    yl = yl.min(-yd);
                    yh = yh.max(-yd);
                }
                f.x_lo = xl;
                f.x_hi = xh;
                f.y_lo = yl;
                f.y_hi = yh;
            },
        }
        f
    }

    /// Where a position lands on the screen, or `None` behind the globe.
    ///
    /// On the flat map every position lands somewhere, on the copy of the world nearest the
    /// centre; whether that is within the screen is the caller's question.
    pub fn forward(&self, lat: f64, lng: f64) -> Option<Pt> {
        if self.check().is_err() {
            return None;
        }
        let f = self.frame();
        match self.kind {
            Projection::Orthographic => {
                let p = unit_vec(lat, lng);
                if dot3(&p, &f.c) < 0.0 {
                    return None;
                }
                let (x, yd, _) = f.ortho(&p);
                let (sx, sy) = f.to_screen(x, yd);
                Some(Pt::new(sx, sy))
            },
            Projection::WebMercator => {
                let q = web_mercator(lat, lng, self.lon_0, f.r_px);
                let (sx, sy) = f.to_screen(q.x, -(q.y - f.y_0));
                Some(Pt::new(sx, sy))
            },
        }
    }

    /// The position under a screen point, as `(lat, lng)` in degrees, or `None` where the
    /// point is off the globe or above or below the square flat map.
    pub fn inverse(&self, pt: Pt) -> Option<(f64, f64)> {
        if self.check().is_err() {
            return None;
        }
        let f = self.frame();
        let (x, yd) = f.from_screen(pt.x, pt.y);
        match self.kind {
            Projection::Orthographic =>
                orthographic_inverse(Pt::new(x, -yd), self.lat_0, self.lon_0, f.r_px),
            Projection::WebMercator => {
                let y = -yd + f.y_0;
                if y.abs() > PI * f.r_px {
                    return None;
                }
                Some(web_mercator_inverse(Pt::new(x, y), self.lon_0, f.r_px))
            },
        }
    }

    /// A spherical cap holding everything the screen shows, as its centre and its angular
    /// radius in radians.
    ///
    /// It is what a caller asks for the cells in view ([`crate::cell::cover_cap`]) and what
    /// culls a ring before it is projected.  On the globe it is exact: the circle round the
    /// screen, or the horizon if that is nearer.  On the flat map it is the furthest point of
    /// the screen's edge from the centre, with two per cent to spare, and the whole sphere
    /// once the screen spans half the world either side.
    pub fn bounding_cap(&self) -> ([f64; 3], f64) {
        let lat_0 = self.lat_0.clamp(-90.0, 90.0);
        let centre = unit_vec(lat_0, self.lon_0);
        if self.check().is_err() {
            return (centre, PI);
        }
        let f = self.frame();
        match self.kind {
            Projection::Orthographic => {
                let reach = f.cx.hypot(f.cy) + CLIP_MARGIN_PX;
                if reach >= f.r_px {
                    (centre, FRAC_PI_2)
                } else {
                    (centre, (reach / f.r_px).asin())
                }
            },
            Projection::WebMercator => {
                let half = PI * f.r_px;
                if f.x_lo <= -half || f.x_hi >= half {
                    return (centre, PI);
                }
                // Walk the screen's edge, clamped to the square map, for the furthest point.
                const STEPS: usize = 64;
                let m = CLIP_MARGIN_PX;
                let mut far: f64 = 0.0;
                for k in 0..(4 * STEPS) {
                    let t = (k % STEPS) as f64 / STEPS as f64;
                    let (sx, sy) = match k / STEPS {
                        0 => (-m + t * (self.w + 2.0 * m), -m),
                        1 => (self.w + m, -m + t * (self.h + 2.0 * m)),
                        2 => (self.w + m - t * (self.w + 2.0 * m), self.h + m),
                        _ => (-m, self.h + m - t * (self.h + 2.0 * m)),
                    };
                    let (x, yd) = f.from_screen(sx, sy);
                    let y = (-yd + f.y_0).clamp(-half, half);
                    let (lat, lng) = web_mercator_inverse(Pt::new(x, y), self.lon_0, f.r_px);
                    let p = unit_vec(lat, lng);
                    far = far.max(dot3(&p, &centre).clamp(-1.0, 1.0).acos());
                }
                (centre, (far * 1.02 + 1.0e-9).min(PI))
            },
        }
    }

    /// Projects rings of unit vectors onto the screen, clipped, into `out`.
    ///
    /// This is the one routine that draws a coastline, a border or a cell outline, on either
    /// projection, so that a caller painting a frame hands over its rings and gets back
    /// screen paths with nothing left to work out.
    ///
    /// On the globe a ring is clipped against a cap: the horizon, or the circle round the
    /// screen when that is nearer.  A filled ring's visible stretches are stitched together
    /// along that circle's rim in the ring's own winding, so a continent cut by the horizon
    /// is still a closed shape, and a ring with nothing showing still fills the screen when the
    /// view is inside it.  On the flat map a ring is unwrapped across the antimeridian, closed
    /// round the pole it encircles if it encircles one, repeated for every copy of the world
    /// the screen shows, and clipped to the screen.
    ///
    /// # Arguments
    /// * `tol_m` - How far the rings may already stray from the truth, in metres of ground: a
    ///   simplified coastline's own tolerance, or zero for exact rings such as cell outlines.
    ///   A visible stretch of a filled ring no larger than this is noise from simplification
    ///   and is dropped, so that a ring which crosses itself cannot break the stitching.
    /// * `eps_px` - A vertex this close to the one before it on the screen is dropped.
    pub fn project_rings<R: AsRef<[[f64; 3]]>>(
        &self,
        rings:  &[R],
        mode:   RingMode,
        tol_m:  f64,
        eps_px: f64,
        out:    &mut ScreenPaths,
    )
        -> Outcome<()>
    {
        res!(self.check());
        let f = self.frame();
        let eps = if eps_px.is_finite() { eps_px.max(0.0) } else { 0.0 };
        let mut pen = Pen { out, eps2: eps * eps, open: false };
        match self.kind {
            Projection::Orthographic => {
                let slack = if tol_m.is_finite() { f.r_px * tol_m.max(0.0) / EARTH_RADIUS_M } else { 0.0 };
                // The rim is walked in steps whose sagitta is under the vertex tolerance.
                let sag = eps.max(0.25).min(f.rho_clip);
                let step = (2.0 * (1.0 - sag / f.rho_clip).acos()).clamp(1.0e-4, 0.1);
                for ring in rings {
                    let ring = ring.as_ref();
                    match mode {
                        RingMode::Fill  => ortho_fill(&f, ring, slack, step, &mut pen),
                        _               => ortho_line(&f, ring, mode == RingMode::Outline, &mut pen),
                    }
                }
            },
            Projection::WebMercator => {
                for ring in rings {
                    merc_ring(&f, ring.as_ref(), mode, &mut pen);
                }
            },
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Screen paths
// ---------------------------------------------------------------------------------------------

/// One path in a [`ScreenPaths`]: a run of its points, and whether to close it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSpan {
    pub start:  u32,    // index of the first point, in points rather than floats
    pub len:    u32,    // number of points
    pub closed: bool,   // join the last point back to the first
}

/// Screen paths as flat `f32` pairs, the shape a canvas painter or a typed array wants.
///
/// Kept by the caller and cleared between frames, so a frame allocates nothing once the
/// buffers have grown to fit.
#[derive(Clone, Debug, Default)]
pub struct ScreenPaths {
    pub xy:     Vec<f32>,       // x0, y0, x1, y1, ... in screen pixels
    pub spans:  Vec<PathSpan>,
}

impl ScreenPaths {
    pub fn new() -> Self { Self::default() }

    pub fn clear(&mut self) {
        self.xy.clear();
        self.spans.clear();
    }

    pub fn len(&self) -> usize { self.spans.len() }

    pub fn is_empty(&self) -> bool { self.spans.is_empty() }

    /// The points of the `i`th path as `x, y` pairs, and whether it is closed.
    pub fn path(&self, i: usize) -> Option<(&[f32], bool)> {
        let s = match self.spans.get(i) {
            Some(s) => s,
            None    => return None,
        };
        let a = 2 * s.start as usize;
        let b = a + 2 * s.len as usize;
        match self.xy.get(a..b) {
            Some(pts)   => Some((pts, s.closed)),
            None        => None,
        }
    }
}

/// Writes paths into a [`ScreenPaths`], dropping vertices within the tolerance of the last.
struct Pen<'a> {
    out:    &'a mut ScreenPaths,
    eps2:   f64,
    open:   bool,
}

impl<'a> Pen<'a> {
    fn begin(&mut self) {
        if self.open {
            self.end(false);
        }
        let start = (self.out.xy.len() / 2) as u32;
        self.out.spans.push(PathSpan { start, len: 0, closed: false });
        self.open = true;
    }

    fn push(&mut self, x: f64, y: f64) {
        if !self.open {
            self.begin();
        }
        let n = self.out.xy.len();
        if let Some(s) = self.out.spans.last() {
            if s.len > 0 {
                let dx = x - self.out.xy[n - 2] as f64;
                let dy = y - self.out.xy[n - 1] as f64;
                if dx * dx + dy * dy <= self.eps2 {
                    return;
                }
            }
        }
        self.out.xy.push(x as f32);
        self.out.xy.push(y as f32);
        if let Some(s) = self.out.spans.last_mut() {
            s.len += 1;
        }
    }

    /// Ends the open path, discarding it if it has too few points to draw.
    fn end(&mut self, closed: bool) {
        if !self.open {
            return;
        }
        self.open = false;
        let least = if closed { 3 } else { 2 };
        let keep = match self.out.spans.last_mut() {
            Some(s) => {
                s.closed = closed;
                s.len >= least
            },
            None => return,
        };
        if !keep {
            if let Some(s) = self.out.spans.pop() {
                self.out.xy.truncate(2 * s.start as usize);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The globe: clip to a cap and walk the rim
// ---------------------------------------------------------------------------------------------
//
// Lifted from Ochre's globe (`web/apps/ochre/src/web/globe.rs`, August 2026), which drew it
// against the horizon alone and verified it against PROJ and against the sea staying sea at
// every turn.  Two things are new here: the clip cap may be smaller than the horizon, so that
// close in the rim being walked is the circle just round the screen rather than one thousands
// of pixels across; and the rim is walked out as points, so the result is plain polylines.

/// Which way round a ring is wound, seen from outside the sphere.
///
/// Measured by the vector area of the ring's own chords, whose component along the ring's own
/// centre is twice the signed area it covers.  A polygon's outline and the holes in it come in
/// the same list of rings, and a hole winds the other way.
fn wound_cw(ring: &[[f64; 3]]) -> bool {
    let mut sum = [0.0; 3];
    let mut area = [0.0; 3];
    let n = ring.len();
    for i in 0..n {
        let a = &ring[i];
        let b = &ring[(i + 1) % n];
        sum = [sum[0] + a[0], sum[1] + a[1], sum[2] + a[2]];
        area = [
            area[0] + a[1] * b[2] - a[2] * b[1],
            area[1] + a[2] * b[0] - a[0] * b[2],
            area[2] + a[0] * b[1] - a[1] * b[0],
        ];
    }
    let m = dot3(&sum, &sum).sqrt().max(f64::EPSILON);
    dot3(&area, &sum) / m < 0.0
}

/// Is the view inside a ring none of whose boundary shows?
///
/// The bearing of the ring about the centre is swept right round, and a ring wound the
/// shapefile way sweeps a whole turn forwards when the view is inside it.  A view whose
/// antipode is inside sweeps a whole turn backwards, so the direction is the answer and the
/// size alone is not.
fn inside(seen: &[(f64, f64, f64)], cw: bool) -> bool {
    let mut sweep = 0.0;
    let mut last = 0.0;
    for i in 0..=seen.len() {
        let p = seen[i % seen.len()];
        let angle = p.1.atan2(p.0);
        if i > 0 {
            let mut step = angle - last;
            if step > PI {
                step -= TAU;
            }
            if step < -PI {
                step += TAU;
            }
            sweep += step;
        }
        last = angle;
    }
    if cw { sweep > PI } else { sweep < -PI }
}

/// A visible stretch of a filled ring, between two crossings of the clip.
struct Run {
    pts:    Vec<(f64, f64)>,    // on the unturned plane, the ends on the clip circle
    entry:  f64,                // angle on the clip circle where it starts
    exit:   f64,                // and where it ends
    done:   bool,               // already drawn into a loop
}

/// Where the edge from `was` to `here` crosses the clip, on the clip circle.
fn crossing(f: &Frame, was: (f64, f64, f64), here: (f64, f64, f64)) -> (f64, f64, f64) {
    let s = was.2 / (was.2 - here.2);
    let (ex, ey) = (was.0 + s * (here.0 - was.0), was.1 + s * (here.1 - was.1));
    // The crossing lies on the chord, a hair inside the sphere, and is put on the rim.
    let m = ex.hypot(ey).max(f64::EPSILON);
    let (ex, ey) = (ex / m * f.rho_clip, ey / m * f.rho_clip);
    (ex, ey, ey.atan2(ex))
}

/// Walks the clip circle from angle `from` through `span` radians (signed), excluding both
/// ends.
fn rim(f: &Frame, from: f64, span: f64, step: f64, pen: &mut Pen) {
    let k = (span.abs() / step).ceil() as usize;
    for i in 1..k {
        let a = from + span * i as f64 / k as f64;
        let (sx, sy) = f.to_screen(f.rho_clip * a.cos(), f.rho_clip * a.sin());
        pen.push(sx, sy);
    }
}

fn ortho_fill(f: &Frame, ring: &[[f64; 3]], slack: f64, step: f64, pen: &mut Pen) {
    let n = ring.len();
    if n < 3 {
        return;
    }
    let seen: Vec<(f64, f64, f64)> = ring.iter().map(|p| f.ortho(p)).collect();
    let first = match seen.iter().position(|p| p.2 >= 0.0) {
        Some(at) => at,
        None => {
            // Nothing of the boundary shows: the ring is out of sight, or the whole view is
            // inside it and the clip circle is filled.
            let cw = wound_cw(ring);
            if inside(&seen, cw) {
                let way = if cw { TAU } else { -TAU };
                pen.begin();
                let (sx, sy) = f.to_screen(f.rho_clip, 0.0);
                pen.push(sx, sy);
                rim(f, 0.0, way, step, pen);
                pen.end(true);
            }
            return;
        },
    };
    let cw = wound_cw(ring);

    // Gather the visible stretches, each ending exactly where the ring meets the clip.
    let mut runs: Vec<Run> = Vec::new();
    let mut open = false;
    let mut hidden = false;
    for k in 0..=n {
        let here = seen[(first + k) % n];
        if k > 0 {
            let was = seen[(first + k - 1) % n];
            if (was.2 >= 0.0) != (here.2 >= 0.0) {
                let (ex, ey, angle) = crossing(f, was, here);
                if here.2 < 0.0 {
                    if let Some(run) = runs.last_mut() {
                        run.pts.push((ex, ey));
                        run.exit = angle;
                    }
                    open = false;
                } else if k == n {
                    // The wrap: this is where the first run began.
                    runs[0].pts.insert(0, (ex, ey));
                    runs[0].entry = angle;
                } else {
                    runs.push(Run { pts: vec![(ex, ey)], entry: angle, exit: 0.0, done: false });
                    open = true;
                }
            }
        }
        if here.2 < 0.0 {
            hidden = true;
            continue;
        }
        if k == n {
            break;
        }
        if !open {
            runs.push(Run { pts: Vec::new(), entry: 0.0, exit: 0.0, done: false });
            open = true;
        }
        if let Some(run) = runs.last_mut() {
            run.pts.push((here.0, here.1));
        }
    }
    if !hidden {
        // Wholly in view: one plain closed outline.
        pen.begin();
        for (x, y) in &runs[0].pts {
            let (sx, sy) = f.to_screen(*x, *y);
            pen.push(sx, sy);
        }
        pen.end(true);
        return;
    }
    if open && runs.len() > 1 {
        // Ended visible, so the open run wraps onto the first one.
        let head = runs.remove(0);
        if let Some(run) = runs.last_mut() {
            run.pts.extend(head.pts);
            run.exit = head.exit;
        }
    }
    // A run no bigger than the rings' own tolerance is a stub from simplification, and goes
    // with the two crossings that bound it.  Entries and exits have to alternate round the rim
    // for the nearest one to be the right one, and a ring that crosses itself breaks that.
    runs.retain(|run| {
        let (mut lox, mut hix) = (f64::MAX, f64::MIN);
        let (mut loy, mut hiy) = (f64::MAX, f64::MIN);
        for (x, y) in &run.pts {
            lox = lox.min(*x);
            hix = hix.max(*x);
            loy = loy.min(*y);
            hiy = hiy.max(*y);
        }
        hix - lox >= slack || hiy - loy >= slack
    });
    if runs.is_empty() {
        return;
    }
    // Slack as an angle on the clip circle, for deciding that two crossings are one.
    let slack_a = slack / f.rho_clip;

    // Stitch: from each run's exit the rim is walked, in the ring's own winding, to the
    // nearest entry, and loops close where they began.
    for start in 0..runs.len() {
        if runs[start].done {
            continue;
        }
        let mut at = start;
        pen.begin();
        for _ in 0..=runs.len() {
            runs[at].done = true;
            for (x, y) in &runs[at].pts {
                let (sx, sy) = f.to_screen(*x, *y);
                pen.push(sx, sy);
            }
            let exit = runs[at].exit;
            let (mut best, mut near) = (at, f64::MAX);
            for i in 0..runs.len() {
                if runs[i].done && i != start {
                    continue;
                }
                let raw = if cw { runs[i].entry - exit } else { exit - runs[i].entry };
                let mut d = raw.rem_euclid(TAU);
                if d > TAU - slack_a {
                    d -= TAU;
                }
                if d < near {
                    near = d;
                    best = i;
                }
            }
            if near > 0.0 {
                rim(f, exit, if cw { near } else { -near }, step, pen);
            }
            if best == start {
                break;
            }
            at = best;
        }
        pen.end(true);
    }
}

fn ortho_line(f: &Frame, ring: &[[f64; 3]], closed: bool, pen: &mut Pen) {
    let n = ring.len();
    if n < 2 {
        return;
    }
    let seen: Vec<(f64, f64, f64)> = ring.iter().map(|p| f.ortho(p)).collect();
    let start = match seen.iter().position(|p| p.2 < 0.0) {
        Some(at) => at,
        None => {
            // Wholly in view.
            pen.begin();
            for p in &seen {
                let (sx, sy) = f.to_screen(p.0, p.1);
                pen.push(sx, sy);
            }
            pen.end(closed);
            return;
        },
    };
    // A closed outline is walked from a hidden vertex round to itself, so that no path is
    // broken where the ring happens to begin.
    let (from, count) = if closed { (start, n + 1) } else { (0, n) };
    let mut open = false;
    for k in 0..count {
        let here = seen[(from + k) % n];
        if k > 0 {
            let was = seen[(from + k - 1) % n];
            if (was.2 >= 0.0) != (here.2 >= 0.0) {
                let (ex, ey, _) = crossing(f, was, here);
                let (sx, sy) = f.to_screen(ex, ey);
                if here.2 >= 0.0 {
                    pen.begin();
                    pen.push(sx, sy);
                    open = true;
                } else {
                    pen.push(sx, sy);
                    pen.end(false);
                    open = false;
                }
            }
        }
        if here.2 >= 0.0 {
            if !open {
                pen.begin();
                open = true;
            }
            let (sx, sy) = f.to_screen(here.0, here.1);
            pen.push(sx, sy);
        }
    }
    if open {
        pen.end(false);
    }
}

// ---------------------------------------------------------------------------------------------
// The flat map: unwrap, repeat and clip
// ---------------------------------------------------------------------------------------------

/// A ring on the unturned Mercator plane, in pixels from the centre with `y` north, its
/// longitudes unwrapped so that it runs continuously across the antimeridian.
///
/// A vertex at a pole has no longitude, so it becomes two points on the map's top or bottom
/// edge, under the vertices either side of it.
fn merc_unwrap(f: &Frame, ring: &[[f64; 3]], closed: bool) -> Vec<(f64, f64)> {
    let n = ring.len();
    // A closed ring is started at a vertex that has a longitude.
    let from = if closed {
        match ring.iter().position(|p| f.merc(p).is_some()) {
            Some(at) => at,
            None => return Vec::new(),
        }
    } else {
        0
    };
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(n + 2);
    let mut last: Option<(f64, f64)> = None;  // unwrapped longitude, raw longitude
    let mut pole: Option<f64> = None;          // the ordinate of a pole awaiting a longitude
    for k in 0..n {
        let p = &ring[(from + k) % n];
        match f.merc(p) {
            None => {
                let y = if p[2] > 0.0 { PI } else { -PI };
                if let Some((u, _)) = last {
                    out.push((u, y));
                }
                pole = Some(y);
            },
            Some((lam, y)) => {
                let u = match last {
                    Some((u, l)) => u + wrap_pi(lam - l),
                    None => lam,
                };
                if let Some(py) = pole.take() {
                    out.push((u, py));
                }
                out.push((u, y));
                last = Some((u, lam));
            },
        }
    }
    if closed {
        if let (Some(py), Some((u_last, l_last))) = (pole, last) {
            // A closed ring ending on a pole: the pole's second point sits under the first
            // vertex, reached from the last.
            if let Some(Some((lam0, _))) = ring.get(from).map(|p| f.merc(p)) {
                out.push((u_last + wrap_pi(lam0 - l_last), py));
            }
        }
    }
    for p in out.iter_mut() {
        p.0 *= f.r_px;
        p.1 = p.1 * f.r_px - f.y_0;
    }
    out
}

fn merc_ring(f: &Frame, ring: &[[f64; 3]], mode: RingMode, pen: &mut Pen) {
    let closed = mode != RingMode::Line;
    let mut pts = merc_unwrap(f, ring, closed);
    if pts.len() < 2 {
        return;
    }
    let world = TAU * f.r_px;
    match mode {
        RingMode::Fill => {
            // A ring that winds once round the world encircles a pole, and is closed round
            // the side of the map where that pole is.  Which pole is the one its vertices lean
            // towards: no land ring encloses more than a hemisphere.
            let first = pts[0];
            let last = pts[pts.len() - 1];
            let lam_first = first.0 / f.r_px;
            let lam_last = last.0 / f.r_px;
            let wind = lam_last + wrap_pi(lam_first - lam_last) - lam_first;
            if wind.abs() > PI {
                let lean: f64 = ring.iter().map(|p| p[2]).sum();
                let cap = if lean > 0.0 { PI * f.r_px - f.y_0 } else { -PI * f.r_px - f.y_0 };
                let u_end = first.0 + wind * f.r_px;
                pts.push((u_end, first.1));
                pts.push((u_end, cap));
                pts.push((first.0, cap));
            }
        },
        RingMode::Outline => pts.push(pts[0]),
        RingMode::Line => (),
    }
    let (mut xl, mut xh, mut yl, mut yh) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in &pts {
        xl = xl.min(*x);
        xh = xh.max(*x);
        yl = yl.min(*y);
        yh = yh.max(*y);
    }
    if yh < f.y_lo || yl > f.y_hi {
        return;
    }
    let k_lo = ((f.x_lo - xh) / world).ceil() as i64;
    let k_hi = ((f.x_hi - xl) / world).floor() as i64;
    if k_hi < k_lo || k_hi - k_lo > 64 {
        return;
    }
    let (w, h) = (2.0 * f.cx, 2.0 * f.cy);
    let rect = (-CLIP_MARGIN_PX, -CLIP_MARGIN_PX, w + CLIP_MARGIN_PX, h + CLIP_MARGIN_PX);
    let mut screen: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for k in k_lo..=k_hi {
        let dx = k as f64 * world;
        screen.clear();
        for (x, y) in &pts {
            screen.push(f.to_screen(x + dx, -y));
        }
        if mode == RingMode::Fill {
            let clipped = clip_polygon(&screen, rect);
            if clipped.len() >= 3 {
                pen.begin();
                for (sx, sy) in &clipped {
                    pen.push(*sx, *sy);
                }
                pen.end(true);
            }
        } else {
            clip_polyline(&screen, rect, pen);
        }
    }
}

/// Sutherland and Hodgman's polygon clip against an axis-aligned rectangle.
///
/// It keeps the winding, so holes stay holes under a nonzero fill, and where a polygon leaves
/// and re-enters it leaves degenerate edges along the rectangle, which fill nothing.
fn clip_polygon(poly: &[(f64, f64)], rect: (f64, f64, f64, f64)) -> Vec<(f64, f64)> {
    let (x0, y0, x1, y1) = rect;
    let mut cur: Vec<(f64, f64)> = poly.to_vec();
    for edge in 0..4 {
        if cur.is_empty() {
            break;
        }
        let keep = |p: &(f64, f64)| -> bool {
            match edge {
                0 => p.0 >= x0,
                1 => p.0 <= x1,
                2 => p.1 >= y0,
                _ => p.1 <= y1,
            }
        };
        let cut = |a: &(f64, f64), b: &(f64, f64)| -> (f64, f64) {
            let t = match edge {
                0 => (x0 - a.0) / (b.0 - a.0),
                1 => (x1 - a.0) / (b.0 - a.0),
                2 => (y0 - a.1) / (b.1 - a.1),
                _ => (y1 - a.1) / (b.1 - a.1),
            };
            (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1))
        };
        let mut next = Vec::with_capacity(cur.len() + 4);
        let n = cur.len();
        for i in 0..n {
            let a = &cur[(i + n - 1) % n];
            let b = &cur[i];
            match (keep(a), keep(b)) {
                (true, true)    => next.push(*b),
                (true, false)   => next.push(cut(a, b)),
                (false, true)   => {
                    next.push(cut(a, b));
                    next.push(*b);
                },
                (false, false)  => (),
            }
        }
        cur = next;
    }
    cur
}

/// Liang and Barsky's segment clip, run along a polyline, cutting it into the pieces that lie
/// within the rectangle.
fn clip_polyline(line: &[(f64, f64)], rect: (f64, f64, f64, f64), pen: &mut Pen) {
    let (x0, y0, x1, y1) = rect;
    let mut open = false;
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut gone = false;
        for (p, q) in [(-dx, a.0 - x0), (dx, x1 - a.0), (-dy, a.1 - y0), (dy, y1 - a.1)] {
            if p == 0.0 {
                if q < 0.0 {
                    gone = true;
                    break;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if gone || t0 > t1 {
            if open {
                pen.end(false);
                open = false;
            }
            continue;
        }
        if !open || t0 > 0.0 {
            if open {
                pen.end(false);
            }
            pen.begin();
            pen.push(a.0 + t0 * dx, a.1 + t0 * dy);
            open = true;
        }
        pen.push(a.0 + t1 * dx, a.1 + t1 * dy);
        if t1 < 1.0 {
            pen.end(false);
            open = false;
        }
    }
    if open {
        pen.end(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How close a projected coordinate has to be to the oracle's, on a unit sphere.
    ///
    /// The oracle is printed to twelve decimal places, so this is loose enough to absorb its
    /// own rounding and tight enough that a wrong coefficient could not pass.
    const TOL: f64 = 1.0e-11;

    /// Positions and their projections on a unit sphere, Greenwich centred.
    ///
    /// The expected values come from PROJ 9.7.1, an implementation outside this crate:
    ///
    /// ```text
    /// proj -f "%.12f" +proj=eqearth +R=1 +lon_0=0
    /// ```
    ///
    /// PROJ is the reference implementation the projection's authors' work was folded into,
    /// and it is not derived from anything here.
    const ORACLE: [(f64, f64, f64, f64); 18] = [
        // Perth.
        (-31.9535,  115.8571,   1.614621210487, -0.629372441721),
        // London.
        ( 51.5074,   -0.1278,  -0.001565514410,  0.965105647318),
        // New York.
        ( 40.7128,  -74.0060,  -0.981861449986,  0.787064220049),
        // Tokyo.
        ( 35.6895,  139.6917,   1.909462941700,  0.697844715778),
        // Cape Town.
        (-33.9189,   18.4233,   0.254226330215, -0.665601683690),
        // Buenos Aires.
        (-34.6037,  -58.3816,  -0.802723755324, -0.678117876992),
        // Sydney.
        (-33.8688,  151.2093,   2.087106438222, -0.664683776716),
        // Honolulu.
        ( 21.3069, -157.8580,  -2.295788127853,  0.426387151159),
        // The origin, and the widest and tallest points of the map.
        (  0.0,        0.0,     0.0,             0.0),
        (  0.0,      180.0,     2.706629983696,  0.0),
        (  0.0,     -180.0,    -2.706629983696,  0.0),
        ( 90.0,        0.0,     0.0,             1.317362759157),
        (-90.0,        0.0,     0.0,            -1.317362759157),
        // Round numbers in all four quadrants.
        ( 45.0,       90.0,     1.159854499103,  0.860231085522),
        (-45.0,      -90.0,    -1.159854499103, -0.860231085522),
        ( 60.0,       30.0,     0.339843347929,  1.088300835505),
        (-60.0,      120.0,     1.359373391714, -1.088300835505),
        // A small angle, where the polynomial's high terms contribute nothing.
        (  1.0,        2.0,     0.030071478499,  0.020257546047),
    ];

    #[test]
    fn test_equal_earth_agrees_with_proj_00() -> Outcome<()> {
        for (lat, lng, x, y) in ORACLE {
            let p = equal_earth_unit(lat, lng);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        Ok(())
    }

    #[test]
    fn test_the_stated_extremes_are_the_extremes_01() -> Outcome<()> {
        let east = equal_earth_unit(0.0, 180.0);
        let near = (east.x - EQUAL_EARTH_HALF_WIDTH).abs() < TOL;
        req!(near, true, "The map is {} wide, not {}.", east.x, EQUAL_EARTH_HALF_WIDTH);
        let north = equal_earth_unit(90.0, 0.0);
        let near = (north.y - EQUAL_EARTH_HALF_HEIGHT).abs() < TOL;
        req!(near, true, "The map is {} tall, not {}.", north.y, EQUAL_EARTH_HALF_HEIGHT);
        // No point escapes the box those two describe.
        let mut lat = -90.0;
        while lat <= 90.0 {
            let mut lng = -180.0;
            while lng <= 180.0 {
                let p = equal_earth_unit(lat, lng);
                let in_x = p.x.abs() <= EQUAL_EARTH_HALF_WIDTH + TOL;
                let in_y = p.y.abs() <= EQUAL_EARTH_HALF_HEIGHT + TOL;
                req!(in_x, true, "x escaped at {}, {}.", lat, lng);
                req!(in_y, true, "y escaped at {}, {}.", lat, lng);
                lng += 3.0;
            }
            lat += 3.0;
        }
        Ok(())
    }

    #[test]
    fn test_the_projection_is_symmetric_02() -> Outcome<()> {
        // A pseudocylindrical projection is symmetric about both axes, so the same
        // latitude north and south is the same height, and east and west the same width.
        for (lat, lng) in [(31.9535, 115.8571), (12.0, 5.0), (78.0, 179.0)] {
            let a = equal_earth_unit(lat, lng);
            let b = equal_earth_unit(-lat, lng);
            let c = equal_earth_unit(lat, -lng);
            let same_x = (a.x - b.x).abs() < TOL;
            let flip_y = (a.y + b.y).abs() < TOL;
            let flip_x = (a.x + c.x).abs() < TOL;
            let same_y = (a.y - c.y).abs() < TOL;
            req!(same_x, true, "x differed across the equator at {}.", lat);
            req!(flip_y, true, "y was not mirrored across the equator at {}.", lat);
            req!(flip_x, true, "x was not mirrored across Greenwich at {}.", lng);
            req!(same_y, true, "y differed across Greenwich at {}.", lng);
        }
        // A parallel is straight: every longitude on it has the same y.
        let y = equal_earth_unit(20.0, 0.0).y;
        for lng in [-170.0, -60.0, 45.0, 179.9] {
            let p = equal_earth_unit(20.0, lng);
            let level = (p.y - y).abs() < TOL;
            req!(level, true, "The 20° parallel bent at {}.", lng);
        }
        Ok(())
    }

    #[test]
    fn test_a_central_meridian_moves_the_map_and_not_its_shape_03() -> Outcome<()> {
        // Recentring on 150° puts Sydney where Greenwich centring puts 1.209° east.
        let a = equal_earth(-33.8688, 151.2093, 150.0, 1.0);
        let b = equal_earth(-33.8688, 1.2093, 0.0, 1.0);
        let same_x = (a.x - b.x).abs() < TOL;
        let same_y = (a.y - b.y).abs() < TOL;
        req!(same_x, true, "Recentring moved x to {} rather than {}.", a.x, b.x);
        req!(same_y, true, "Recentring moved y at all: {} against {}.", a.y, b.y);
        // And a longitude that wraps past the antimeridian lands on the far side rather
        // than off the map.
        let west = equal_earth(0.0, -170.0, 20.0, 1.0);
        let wrapped = west.x > 0.0;
        req!(wrapped, true, "170° west of a map centred on 20° east came out at {}.", west.x);
        Ok(())
    }

    #[test]
    fn test_a_radius_scales_the_map_uniformly_04() -> Outcome<()> {
        let r = equal_earth_radius_for_half_width(180.0);
        let east = equal_earth(0.0, 180.0, 0.0, r);
        let wide = (east.x - 180.0).abs() < 1.0e-9;
        req!(wide, true, "The map came out {} wide.", east.x);
        let north = equal_earth(90.0, 0.0, 0.0, r);
        let want = 180.0 * EQUAL_EARTH_HALF_HEIGHT / EQUAL_EARTH_HALF_WIDTH;
        let tall = (north.y - want).abs() < 1.0e-9;
        req!(tall, true, "The map came out {} tall.", north.y);
        // Scaling is uniform, or the projection would stop being equal-area.
        let one = equal_earth_unit(-31.9535, 115.8571);
        let big = equal_earth(-31.9535, 115.8571, 0.0, r);
        let scaled_x = (big.x - one.x * r).abs() < 1.0e-9;
        let scaled_y = (big.y - one.y * r).abs() < 1.0e-9;
        req!(scaled_x, true, "x did not scale by the radius.");
        req!(scaled_y, true, "y did not scale by the radius.");
        Ok(())
    }

    /// Positions and their orthographic projections on a unit sphere centred on the origin.
    ///
    /// From PROJ 9.7.1, an implementation outside this crate:
    ///
    /// ```text
    /// proj -f "%.12f" +proj=ortho +R=1 +lat_0=0 +lon_0=0
    /// ```
    ///
    /// Every position here is on the near hemisphere, because PROJ answers `*` for the far
    /// one rather than a coordinate; that refusal is what
    /// [`test_the_far_side_of_the_globe_is_known_from_the_near_07`] checks against.
    const ORTHO_ORACLE: [(f64, f64, f64, f64); 11] = [
        // The centre itself.
        (  0.0,       0.0,     0.0,             0.0),
        // London, Cape Town, Buenos Aires, New York: all within a quarter turn of Greenwich.
        ( 51.5074,   -0.1278, -0.001388311442,  0.782688550807),
        (-33.9189,   18.4233,  0.262254675949, -0.558018872482),
        (-34.6037,  -58.3816, -0.700917639761, -0.567896899696),
        ( 40.7128,  -74.0060, -0.728647317901,  0.652267756393),
        // The rim, a tenth of a degree short of it.
        (  0.0,      89.9,     0.999998476913,  0.0),
        // Both poles, which sit at the top and bottom of the disc from the equator.
        ( 90.0,       0.0,     0.0,             1.0),
        (-90.0,       0.0,     0.0,            -1.0),
        // Round numbers, and a small angle where the foreshortening is negligible.
        (  1.0,       2.0,     0.034894181340,  0.017452406437),
        ( 60.0,      30.0,     0.250000000000,  0.866025403784),
        (-45.0,     -90.0,    -0.707106781187, -0.707106781187),
    ];

    /// The same, seen from above Perth, so the centre is oblique in both coordinates.
    ///
    /// ```text
    /// proj -f "%.12f" +proj=ortho +R=1 +lat_0=-31.9535 +lon_0=115.8571
    /// ```
    const ORTHO_ORACLE_PERTH: [(f64, f64, f64, f64); 8] = [
        // Perth, which is the centre.
        (-31.9535,  115.8571,  0.0,             0.0),
        // Sydney, Tokyo, Kuala Lumpur, Auckland, Suva.
        (-33.8688,  151.2093,  0.480421544470, -0.114447989448),
        ( 35.6895,  139.6917,  0.328204336296,  0.888173527162),
        (  3.1390,  101.6869, -0.244435840733,  0.558819302390),
        (-36.8485,  174.7633,  0.685250212424, -0.290118911137),
        (-18.1416,  178.4419,  0.843565957319, -0.032624197369),
        // Fremantle, sixteen kilometres away and barely off the centre.
        (-32.0569,  115.7439, -0.001674457753, -0.001805544881),
        // The south pole, which from this latitude sits low on the disc and not on its rim.
        (-90.0,       0.0,     0.0,            -0.848477887693),
    ];

    #[test]
    fn test_orthographic_agrees_with_proj_06() -> Outcome<()> {
        for (lat, lng, x, y) in ORTHO_ORACLE {
            let p = orthographic(lat, lng, 0.0, 0.0, 1.0);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        for (lat, lng, x, y) in ORTHO_ORACLE_PERTH {
            let p = orthographic(lat, lng, -31.9535, 115.8571, 1.0);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        Ok(())
    }

    #[test]
    fn test_the_far_side_of_the_globe_is_known_from_the_near_07() -> Outcome<()> {
        // Perth is a quarter turn and more from Greenwich, so PROJ answers `*` for it on a
        // Greenwich-centred globe. The cosine is what says so here.
        let behind = orthographic_cos_c(-31.9535, 115.8571, 0.0, 0.0);
        let far = behind < 0.0;
        req!(far, true, "Perth came out in front of Greenwich, at {}.", behind);
        // The centre is directly under the viewer, and its antipode directly behind.
        let under = orthographic_cos_c(-31.9535, 115.8571, -31.9535, 115.8571);
        let full = (under - 1.0).abs() < TOL;
        req!(full, true, "The centre came out at {}.", under);
        let anti = orthographic_cos_c(31.9535, -64.1429, -31.9535, 115.8571);
        let behind_us = (anti + 1.0).abs() < TOL;
        req!(behind_us, true, "The antipode came out at {}.", anti);
        // On the horizon it is zero, and the point lands exactly on the rim.
        let rim = orthographic_cos_c(0.0, 90.0, 0.0, 0.0);
        let level = rim.abs() < TOL;
        req!(level, true, "A quarter turn away came out at {}.", rim);
        let p = orthographic(0.0, 90.0, 0.0, 0.0, 1.0);
        let on = (p.x.hypot(p.y) - ORTHOGRAPHIC_HALF_WIDTH).abs() < TOL;
        req!(on, true, "The horizon came out {} from the centre.", p.x.hypot(p.y));
        // And nothing escapes the disc, near side or far.
        let mut lat = -90.0;
        while lat <= 90.0 {
            let mut lng = -180.0;
            while lng <= 180.0 {
                let p = orthographic(lat, lng, -31.9535, 115.8571, 1.0);
                let inside = p.x.hypot(p.y) <= ORTHOGRAPHIC_HALF_WIDTH + TOL;
                req!(inside, true, "{}, {} escaped the disc at {}, {}.", lat, lng, p.x, p.y);
                lng += 3.0;
            }
            lat += 3.0;
        }
        Ok(())
    }

    #[test]
    fn test_the_globe_turns_without_changing_shape_08() -> Outcome<()> {
        // Turning the globe by a degree of longitude and asking for a position a degree
        // further east is the same picture, because only the difference matters.
        let a = orthographic(12.0, 45.0, 0.0, 30.0, 1.0);
        let b = orthographic(12.0, 15.0, 0.0, 0.0, 1.0);
        let same_x = (a.x - b.x).abs() < TOL;
        let same_y = (a.y - b.y).abs() < TOL;
        req!(same_x, true, "Turning moved x to {} rather than {}.", a.x, b.x);
        req!(same_y, true, "Turning moved y to {} rather than {}.", a.y, b.y);
        // A radius scales the disc and nothing else.
        let one = orthographic(-33.8688, 151.2093, -31.9535, 115.8571, 1.0);
        let big = orthographic(-33.8688, 151.2093, -31.9535, 115.8571, 320.0);
        let scaled_x = (big.x - one.x * 320.0).abs() < 1.0e-9;
        let scaled_y = (big.y - one.y * 320.0).abs() < 1.0e-9;
        req!(scaled_x, true, "x did not scale by the radius.");
        req!(scaled_y, true, "y did not scale by the radius.");
        // A longitude past the antimeridian comes round rather than off the globe.
        let round = orthographic(0.0, 190.0, 0.0, 170.0, 1.0);
        let west = orthographic(0.0, -170.0, 0.0, 170.0, 1.0);
        let wrapped = (round.x - west.x).abs() < TOL;
        req!(wrapped, true, "190° east did not wrap to 170° west.");
        Ok(())
    }

    #[test]
    fn test_a_fix_just_outside_its_range_still_projects_05() -> Outcome<()> {
        let over = equal_earth_unit(90.000_000_1, 0.0);
        let finite = over.y.is_finite();
        req!(finite, true, "A latitude a hair over the pole gave {}.", over.y);
        let clamped = (over.y - EQUAL_EARTH_HALF_HEIGHT).abs() < TOL;
        req!(clamped, true, "It did not clamp to the pole.");
        let round = equal_earth_unit(0.0, 190.0);
        let west = equal_earth_unit(0.0, -170.0);
        let wrapped = (round.x - west.x).abs() < TOL;
        req!(wrapped, true, "190° east did not wrap to 170° west.");
        Ok(())
    }
    /// Positions and their spherical Web Mercator projections on a unit sphere, at four
    /// central meridians, from PROJ 9.7.1:
    ///
    /// ```text
    /// proj -f "%.12f" +proj=webmerc +R=1 +lon_0=<lon_0>
    /// ```
    const MERC_POINTS: [(f64, f64); 10] = [
        (-31.9535,  115.8571),  // Perth
        ( 51.5074,   -0.1278),  // London
        ( 40.7128,  -74.0060),  // New York
        ( 35.6895,  139.6917),  // Tokyo
        ( 21.3069, -157.8580),  // Honolulu
        (-33.9189,   18.4233),  // Cape Town
        (  0.0,        0.0),
        ( 85.0,      179.9),    // near the top corner
        (-85.0,     -179.9),
        ( 60.0,       30.0),
    ];
    const MERC_ORACLE: [(f64, [(f64, f64); 10]); 4] = [
        (0.0, [
            ( 2.022087856812, -0.589076140273), (-0.002230530784,  1.052273175629),
            (-1.291648366231,  0.779235626193), ( 2.438080102708,  0.667590039565),
            (-2.755141850613,  0.380755537637), ( 0.321547244083, -0.629951578195),
            ( 0.000000000000,  0.000000000000), ( 3.139847324338,  3.131301331472),
            (-3.139847324338, -3.131301331472), ( 0.523598775598,  1.316957896925),
        ]),
        (115.8571, [
            ( 0.000000000000, -0.589076140273), (-2.024318387596,  1.052273175629),
            ( 2.969449084136,  0.779235626193), ( 0.415992245896,  0.667590039565),
            ( 1.505955599754,  0.380755537637), (-1.700540612730, -0.629951578195),
            (-2.022087856812,  0.000000000000), ( 1.117759467525,  3.131301331472),
            ( 1.121250126029, -3.131301331472), (-1.498489081214,  1.316957896925),
        ]),
        (-74.006, [
            (-2.969449084136, -0.589076140273), ( 1.289417835447,  1.052273175629),
            ( 0.000000000000,  0.779235626193), (-2.553456838240,  0.667590039565),
            (-1.463493484382,  0.380755537637), ( 1.613195610314, -0.629951578195),
            ( 1.291648366231,  0.000000000000), (-1.851689616611,  3.131301331472),
            (-1.848198958107, -3.131301331472), ( 1.815247141829,  1.316957896925),
        ]),
        (-150.0, [
            (-1.643103572376, -0.589076140273), ( 2.615763347207,  1.052273175629),
            ( 1.326345511761,  0.779235626193), (-1.227111326480,  0.667590039565),
            (-0.137147972622,  0.380755537637), ( 2.939541122074, -0.629951578195),
            ( 2.617993877991,  0.000000000000), (-0.525344104850,  3.131301331472),
            (-0.521853446346, -3.131301331472), ( 3.141592653590,  1.316957896925),
        ]),
    ];

    #[test]
    fn test_web_mercator_agrees_with_proj_09() -> Outcome<()> {
        for (lon_0, want) in MERC_ORACLE {
            for ((lat, lng), (x, y)) in MERC_POINTS.iter().zip(want.iter()) {
                let p = web_mercator(*lat, *lng, lon_0, 1.0);
                let near = (p.x - x).abs() < TOL && (p.y - y).abs() < TOL;
                req!(near, true, "{}, {} about {} came out at ({:.12}, {:.12}), wanted ({}, {}).",
                    lat, lng, lon_0, p.x, p.y, x, y);
            }
        }
        // The map is square at its stated limit, and a pole clamps to it.
        let top = web_mercator(90.0, 0.0, 0.0, 1.0);
        let square = (top.y - PI).abs() < 1.0e-12;
        req!(square, true, "The map's top came out at {}, not pi.", top.y);
        Ok(())
    }

    #[test]
    fn test_web_mercator_inverse_agrees_with_proj_10() -> Outcome<()> {
        // proj -I -f "%.12f" +proj=webmerc +R=1 +lon_0=115.8571, which answers lng, lat.
        for (x, y, lng, lat) in [
            ( 0.3, -0.6,  133.045833853925, -32.483015050012),
            (-1.2,  1.1,   47.102164584301,  53.177781879084),
            ( 2.5, -2.9, -100.903451217294, -83.701155006501),
            ( 0.0,  0.0,  115.857100000000,   0.000000000000),
        ] {
            let (a, b) = web_mercator_inverse(Pt::new(x, y), 115.8571, 1.0);
            let near = (a - lat).abs() < 1.0e-9 && (b - lng).abs() < 1.0e-9;
            req!(near, true, "({}, {}) came back as {}, {}; PROJ says {}, {}.", x, y, a, b, lat, lng);
        }
        // Forward and back is the identity, including across the antimeridian.
        for (lat, lng) in MERC_POINTS {
            let p = web_mercator(lat, lng, -150.0, 6.0);
            let (a, b) = web_mercator_inverse(p, -150.0, 6.0);
            let same = (a - lat).abs() < 1.0e-9 && (wrap_half_turn(b - lng)).abs() < 1.0e-9;
            req!(same, true, "{}, {} came back as {}, {}.", lat, lng, a, b);
        }
        Ok(())
    }

    #[test]
    fn test_orthographic_inverse_agrees_with_proj_11() -> Outcome<()> {
        // proj -I -f "%.12f" +proj=ortho +R=1 +lat_0=<lat_0> +lon_0=<lon_0>, answering lng,
        // lat, at a centre on the equator, two oblique centres, a third far north, the north
        // pole, and a southern one.  The last input of each is off the disc, where PROJ
        // answers `*`.
        let xy = [(0.0, 0.0), (0.3, -0.2), (0.7, 0.5), (-0.9, 0.1), (0.0, 0.99), (-0.45, -0.8),
            (0.001, 0.002)];
        let oracle: [((f64, f64), [(f64, f64); 7]); 6] = [
            ((0.0, 0.0), [
                (0.000000000000, 0.000000000000), (17.829543848069, -11.536959032815),
                (53.929231349204, 30.000000000000), (-64.760598179321, 5.739170477267),
                (0.000000000000, 81.890385544006), (-48.590377890729, -53.130102354156),
                (0.057295903654, 0.114591635421),
            ]),
            ((-31.9535, 115.8571), [
                (115.857100000000, -31.953500000000), (139.491172018755, -41.554276226097),
                (160.969622305304, 8.881020396659), (50.501798095373, -8.029667432931),
                (115.857100000000, 49.936885544006), (14.957210228986, -62.724626645594),
                (115.924543725221, -31.838890517892),
            ]),
            ((51.5074, -0.1278), [
                (-0.127800000000, 51.507400000000), (22.018911603216, 37.269190760395),
                (95.904696130209, 45.259426785352), (-78.463343669197, 23.222821817975),
                (179.872200000000, 46.602214455994), (-27.392900860832, -10.795895994418),
                (-0.035513551106, 51.621955519567),
            ]),
            ((35.6895, 139.6917), [
                (139.691700000000, 35.689500000000), (158.631638139189, 22.439898158901),
                (-140.229884106383, 44.713981955783), (67.334907163601, 19.191798955953),
                (-40.308300000000, 62.420114455994), (109.995075616966, -24.722618420153),
                (139.762346389474, 35.804071028094),
            ]),
            ((90.0, 0.0), [
                (0.000000000000, 90.000000000000), (56.309932474020, 68.865707785214),
                (125.537677791974, 30.657298992941), (-96.340191745910, 25.104090250221),
                (180.000000000000, 8.109614455994), (-29.357753542791, 23.382196425509),
                (153.434948822922, 89.871882635420),
            ]),
            ((-60.0, -70.0), [
                (-70.000000000000, -60.000000000000), (-24.339703634894, -65.199623342309),
                (-24.503147524221, -11.045475113038), (-141.637585260840, -18.507178106808),
                (-70.000000000000, 21.890385544006), (152.308915595694, -48.046975207444),
                (-69.885803894134, -59.885358916100),
            ]),
        ];
        for ((lat_0, lon_0), want) in oracle {
            for ((x, y), (lng, lat)) in xy.iter().zip(want.iter()) {
                let got = match orthographic_inverse(Pt::new(*x, *y), lat_0, lon_0, 1.0) {
                    Some(g) => g,
                    None => return Err(err!("({}, {}) about {}, {} fell off the globe.",
                        x, y, lat_0, lon_0; Test)),
                };
                // At the pole longitude is arbitrary; everywhere else it is compared too.
                let near_lat = (got.0 - lat).abs() < 1.0e-9;
                let near_lng = lat.abs() > 89.999_999 || wrap_half_turn(got.1 - lng).abs() < 1.0e-9;
                req!(near_lat && near_lng, true,
                    "({}, {}) about {}, {} came back as {}, {}; PROJ says {}, {}.",
                    x, y, lat_0, lon_0, got.0, got.1, lat, lng);
            }
            let off = orthographic_inverse(Pt::new(0.8, 0.7), lat_0, lon_0, 1.0);
            req!(off.is_none(), true, "A point off the disc came back as {:?}.", off);
        }
        Ok(())
    }

}
