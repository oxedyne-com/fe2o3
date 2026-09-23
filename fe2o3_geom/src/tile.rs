//! Web Mercator map tiles: the `z/x/y` grid street maps are cut into.
//!
//! Zoom `z` divides the square Web Mercator world into `2^z` by `2^z` tiles, `x` eastward from
//! the antimeridian and `y` southward from the map's top edge at
//! [`crate::proj::WEB_MERCATOR_MAX_LAT`].  This is the grid of every slippy map, of Mapbox
//! Vector Tiles and of PMTiles archives ([`pmtiles`]).
//!
//! A [`Viewport`] says which tiles it needs ([`Viewport::tiles_covering`]) and how to paint a
//! tile's bitmap onto the screen ([`Viewport::tile_affine`]): exactly on the flat map, and on
//! the globe as the flat map's transform with the error that costs, so a caller can paint
//! bitmaps where the error is below a pixel and reproject vectors where it is not.

pub mod pmtiles;

use crate::proj::{
    Projection,
    Viewport,
    unit_vec,
};

use oxedyne_fe2o3_core::prelude::*;

use std::f64::consts::{
    FRAC_PI_2,
    PI,
    TAU,
};

pub const MAX_ZOOM: u8 = 31; // the deepest zoom whose PMTiles ids fit a u64

/// One tile of the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z:  u8,
    pub x:  u32,    // 0 at the antimeridian, eastward
    pub y:  u32,    // 0 at the top of the map, southward
}

impl std::fmt::Display for TileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.z, self.x, self.y)
    }
}

/// A longitude's position across the grid at zoom `z`, in tiles: 0 at 180 degrees west.
pub fn lng_to_tx(lng: f64, z: u8) -> f64 {
    (lng + 180.0) / 360.0 * (1u64 << z) as f64
}

/// A latitude's position down the grid at zoom `z`, in tiles: 0 at the top of the map.
pub fn lat_to_ty(lat: f64, z: u8) -> f64 {
    let phi = lat.to_radians();
    (1.0 - phi.tan().asinh() / PI) / 2.0 * (1u64 << z) as f64
}

pub fn tx_to_lng(tx: f64, z: u8) -> f64 {
    tx / (1u64 << z) as f64 * 360.0 - 180.0
}

pub fn ty_to_lat(ty: f64, z: u8) -> f64 {
    (PI * (1.0 - 2.0 * ty / (1u64 << z) as f64)).sinh().atan().to_degrees()
}

impl TileId {
    pub fn new(z: u8, x: u32, y: u32) -> Outcome<Self> {
        if z > MAX_ZOOM {
            return Err(err!("Zoom {} is past the deepest, {}.", z, MAX_ZOOM; Invalid, Input, Range));
        }
        let n = 1u64 << z;
        if x as u64 >= n || y as u64 >= n {
            return Err(err!("Tile {}/{}/{} is off a grid {} tiles across.", z, x, y, n;
                Invalid, Input, Range));
        }
        Ok(Self { z, x, y })
    }

    /// The tile holding a position.  Latitude is clamped to the map and longitude wrapped, so
    /// a pole lands on the top or bottom row.
    pub fn at(lat: f64, lng: f64, z: u8) -> Outcome<Self> {
        if z > MAX_ZOOM {
            return Err(err!("Zoom {} is past the deepest, {}.", z, MAX_ZOOM; Invalid, Input, Range));
        }
        if !(lat.is_finite() && lng.is_finite()) {
            return Err(err!("{}, {} is not a position.", lat, lng; Invalid, Input));
        }
        let n = (1u64 << z) as f64;
        let lat = lat.clamp(-crate::proj::WEB_MERCATOR_MAX_LAT, crate::proj::WEB_MERCATOR_MAX_LAT);
        let tx = lng_to_tx(lng, z).rem_euclid(n);
        let ty = lat_to_ty(lat, z);
        let clamp = |t: f64| -> u32 { t.floor().clamp(0.0, n - 1.0) as u32 };
        Ok(Self { z, x: clamp(tx), y: clamp(ty) })
    }

    /// The position at a fraction `u` across and `v` down the tile, as `(lat, lng)`.
    pub fn point(&self, u: f64, v: f64) -> (f64, f64) {
        (ty_to_lat(self.y as f64 + v, self.z), tx_to_lng(self.x as f64 + u, self.z))
    }

    /// The tile's edges: south, west, north and east, in degrees.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (north, west) = self.point(0.0, 0.0);
        let (south, east) = self.point(1.0, 1.0);
        (south, west, north, east)
    }

    /// The tile containing this one at a zoom no deeper than its own.
    pub fn ancestor(&self, z: u8) -> Outcome<Self> {
        if z > self.z {
            return Err(err!("A zoom-{} tile has no ancestor at zoom {}.", self.z, z; Invalid, Input, Range));
        }
        let d = self.z - z;
        Ok(Self { z, x: self.x >> d, y: self.y >> d })
    }

    pub fn children(&self) -> Outcome<[Self; 4]> {
        if self.z >= MAX_ZOOM {
            return Err(err!("A zoom-{} tile has no children.", self.z; Invalid, Input, Range));
        }
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        Ok([Self { z, x, y }, Self { z, x: x + 1, y }, Self { z, x, y: y + 1 }, Self { z, x: x + 1, y: y + 1 }])
    }

    /// Where this tile sits inside an ancestor, for drawing it from the ancestor's data past
    /// an archive's deepest zoom: the fraction across and down the ancestor at which it begins,
    /// and how many of it span the ancestor.  `None` if `ancestor` does not contain it.
    pub fn within(&self, ancestor: &TileId) -> Option<(f64, f64, f64)> {
        if ancestor.z > self.z {
            return None;
        }
        let d = self.z - ancestor.z;
        if self.x >> d != ancestor.x || self.y >> d != ancestor.y {
            return None;
        }
        let span = (1u64 << d) as f64;
        let mask = (1u64 << d) - 1;
        Some(((self.x as u64 & mask) as f64 / span, (self.y as u64 & mask) as f64 / span, span))
    }
}

/// A 2D affine transform in canvas order: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a:  f64,
    pub b:  f64,
    pub c:  f64,
    pub d:  f64,
    pub e:  f64,
    pub f:  f64,
}

impl Affine {
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

impl Viewport {
    /// The tiles of zoom `z` the screen shows, nearest the centre first, refusing more than
    /// `max`.
    ///
    /// On the flat map these are the tiles under the screen, turned by its heading if it is
    /// turned, wrapping east and west, each listed once however many copies of the world the
    /// screen shows.  On the globe they are the tiles whose bounding caps meet the view's
    /// [`Viewport::bounding_cap`].
    pub fn tiles_covering(&self, z: u8, max: usize) -> Outcome<Vec<TileId>> {
        res!(self.check());
        if z > MAX_ZOOM {
            return Err(err!("Zoom {} is past the deepest, {}.", z, MAX_ZOOM; Invalid, Input, Range));
        }
        let n = 1u64 << z;
        let nf = n as f64;
        let mut out: Vec<(f64, TileId)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        match self.kind {
            Projection::WebMercator => {
                let f = self.frame();
                // The screen's corners in fractional tiles, longitude unwrapped.
                let corners = [(0.0, 0.0), (self.w, 0.0), (self.w, self.h), (0.0, self.h)];
                let mut quad = [(0.0f64, 0.0f64); 4];
                for (k, (sx, sy)) in corners.iter().enumerate() {
                    let (x, yd) = f.from_screen(*sx, *sy);
                    let lam = f.lam_0 + x / f.r_px;
                    let my = (-yd + f.y_0) / f.r_px;
                    quad[k] = ((lam + PI) / TAU * nf, (PI - my) / TAU * nf);
                }
                let (cx, cy) = {
                    let my = f.y_0 / f.r_px;
                    ((f.lam_0 + PI) / TAU * nf, (PI - my) / TAU * nf)
                };
                let xl = quad.iter().map(|p| p.0).fold(f64::MAX, f64::min).floor();
                // A screen wider than the world needs each column once.
                let xh = quad.iter().map(|p| p.0).fold(f64::MIN, f64::max).ceil().min(xl + nf);
                let yl = quad.iter().map(|p| p.1).fold(f64::MAX, f64::min).floor().max(0.0);
                let yh = quad.iter().map(|p| p.1).fold(f64::MIN, f64::max).ceil().min(nf);
                let span = (xh - xl) * (yh - yl);
                if span > (4 * max.max(1)) as f64 {
                    return Err(err!("The screen spans about {} zoom-{} tiles, more than {}.",
                        span as u64, z, max; Excessive, Size));
                }
                let turned = self.heading.rem_euclid(90.0) != 0.0;
                let mut ty = yl;
                while ty < yh {
                    let mut tx = xl;
                    while tx < xh {
                        let square = [(tx, ty), (tx + 1.0, ty), (tx + 1.0, ty + 1.0), (tx, ty + 1.0)];
                        if !turned || overlaps(&square, &quad) {
                            let t = TileId { z, x: (tx as i64).rem_euclid(n as i64) as u32, y: ty as u32 };
                            if seen.insert(t) {
                                let d = (tx + 0.5 - cx).hypot(ty + 0.5 - cy);
                                out.push((d, t));
                                if out.len() > max {
                                    return Err(err!("The screen shows more than {} zoom-{} tiles.",
                                        max, z; Excessive, Size));
                                }
                            }
                        }
                        tx += 1.0;
                    }
                    ty += 1.0;
                }
            },
            Projection::Orthographic => {
                let (c, r) = self.bounding_cap();
                let lat_c = self.lat_0.clamp(-90.0, 90.0);
                let lat_hi = (lat_c + r.to_degrees()).min(crate::proj::WEB_MERCATOR_MAX_LAT);
                let lat_lo = (lat_c - r.to_degrees()).max(-crate::proj::WEB_MERCATOR_MAX_LAT);
                let polar = lat_c + r.to_degrees() >= 90.0 || lat_c - r.to_degrees() <= -90.0;
                let (tx_lo, tx_hi) = if polar || r >= FRAC_PI_2 {
                    (0.0, nf)
                } else {
                    let half = (r.sin() / lat_c.to_radians().cos()).min(1.0).asin().to_degrees();
                    (lng_to_tx(self.lon_0 - half, z).floor(), lng_to_tx(self.lon_0 + half, z).ceil())
                };
                let (ty_lo, ty_hi) = (lat_to_ty(lat_hi, z).floor().max(0.0), lat_to_ty(lat_lo, z).ceil().min(nf));
                let (cx, cy) = (lng_to_tx(self.lon_0, z), lat_to_ty(lat_c.clamp(-85.0, 85.0), z));
                let span = (tx_hi - tx_lo).min(nf) * (ty_hi - ty_lo);
                if span > (16 * max.max(1)) as f64 {
                    return Err(err!("The globe spans about {} zoom-{} tiles, more than {}.",
                        span as u64, z, max; Excessive, Size));
                }
                let mut ty = ty_lo;
                while ty < ty_hi {
                    let mut tx = tx_lo;
                    while tx < tx_hi.min(tx_lo + nf) {
                        let t = TileId { z, x: (tx as i64).rem_euclid(n as i64) as u32, y: ty as u32 };
                        let (tc, tr) = tile_cap(&t);
                        let apart = (tc[0] * c[0] + tc[1] * c[1] + tc[2] * c[2]).clamp(-1.0, 1.0).acos();
                        if apart <= tr + r && seen.insert(t) {
                            let mut dx = (tx + 0.5 - cx).abs() % nf;
                            dx = dx.min(nf - dx);
                            out.push((dx.hypot(ty + 0.5 - cy), t));
                            if out.len() > max {
                                return Err(err!("The globe shows more than {} zoom-{} tiles.", max, z;
                                    Excessive, Size));
                            }
                        }
                        tx += 1.0;
                    }
                    ty += 1.0;
                }
            },
        }
        out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out.into_iter().map(|(_, t)| t).collect())
    }

    /// The zoom, fractional, at which a tile `tile_px` pixels across is drawn one tile pixel
    /// to one screen pixel at the centre.  Both projections share their scale at the centre,
    /// so the globe asks for the same tiles as the flat map with the same camera.
    pub fn tile_zoom(&self, tile_px: f64) -> f64 {
        let flat = Viewport { kind: Projection::WebMercator, ..*self };
        if flat.check().is_err() || !(tile_px > 0.0) {
            return 0.0;
        }
        let f = flat.frame();
        (TAU * f.r_px / tile_px).log2()
    }

    /// The transform that paints a tile's bitmap, from its own coordinates -- `u` across and
    /// `v` down, 0 to 1 -- onto the screen, with how far it strays from the true projection in
    /// pixels.
    ///
    /// The transform is the flat map's for the same camera, the copy of the tile nearest the
    /// centre.  On the flat map it is exact.  On the globe it is exact at the centre, where the
    /// two projections share their scale, and the error is measured at the tile's corners,
    /// edge midpoints and middle; `None` if any of those is behind the globe.  At 32 degrees a
    /// view 12 km high keeps the error under half a pixel on a phone, which is where a globe
    /// can paint the same bitmaps as the map with nothing to hand off.
    pub fn tile_affine(&self, t: &TileId) -> Option<(Affine, f64)> {
        if self.check().is_err() {
            return None;
        }
        let flat = Viewport { kind: Projection::WebMercator, ..*self };
        let f = flat.frame();
        let n = (1u64 << t.z) as f64;
        let s = TAU * f.r_px / n;
        // The tile's north-west corner on the unturned plane, on the copy nearest the centre.
        let mut lam = t.x as f64 / n * TAU - PI - f.lam_0;
        let mid = lam + PI / n;
        lam -= TAU * ((mid + PI).div_euclid(TAU));
        let x_nw = lam * f.r_px;
        let y_nw = f.r_px * (PI - TAU * t.y as f64 / n) - f.y_0;
        let aff = Affine {
            a: s * f.ch,
            b: -s * f.sh,
            c: s * f.sh,
            d: s * f.ch,
            e: f.cx + x_nw * f.ch - y_nw * f.sh,
            f: f.cy - x_nw * f.sh - y_nw * f.ch,
        };
        if self.kind == Projection::WebMercator {
            return Some((aff, 0.0));
        }
        let mut err: f64 = 0.0;
        for (u, v) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (0.5, 0.5), (1.0, 0.5),
            (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)]
        {
            let (lat, lng) = t.point(u, v);
            let p = match self.forward(lat, lng) {
                Some(p) => p,
                None => return None,
            };
            let (x, y) = aff.apply(u, v);
            err = err.max((p.x - x).hypot(p.y - y));
        }
        Some((aff, err))
    }
}

/// A cap about a tile's middle holding the whole tile.  A tile's north and south edges are
/// parallels, not great circles, so the edge midpoints are measured as well as the corners,
/// and a little is added for the bulge between them.
fn tile_cap(t: &TileId) -> ([f64; 3], f64) {
    let (lat, lng) = t.point(0.5, 0.5);
    let c = unit_vec(lat, lng);
    let mut far: f64 = 0.0;
    for (u, v) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (1.0, 0.5), (0.0, 1.0),
        (0.5, 1.0), (1.0, 1.0)]
    {
        let (a, b) = t.point(u, v);
        let p = unit_vec(a, b);
        far = far.max((p[0] * c[0] + p[1] * c[1] + p[2] * c[2]).clamp(-1.0, 1.0).acos());
    }
    (c, far * 1.02 + 1.0e-9)
}

/// Do two convex quadrilaterals overlap?  Separating axes: the edge normals of both.
fn overlaps(a: &[(f64, f64); 4], b: &[(f64, f64); 4]) -> bool {
    for poly in [a, b] {
        for k in 0..4 {
            let (p, q) = (poly[k], poly[(k + 1) % 4]);
            let (nx, ny) = (q.1 - p.1, p.0 - q.0);
            let proj = |pts: &[(f64, f64); 4]| -> (f64, f64) {
                let mut lo = f64::MAX;
                let mut hi = f64::MIN;
                for r in pts.iter() {
                    let d = r.0 * nx + r.1 * ny;
                    lo = lo.min(d);
                    hi = hi.max(d);
                }
                (lo, hi)
            };
            let (alo, ahi) = proj(a);
            let (blo, bhi) = proj(b);
            if ahi < blo || bhi < alo {
                return false;
            }
        }
    }
    true
}
