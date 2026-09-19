//! A global cell-index grid: a cube-sphere quadtree with great-circle cell edges.
//!
//! The sphere is wrapped in a cube.  A direction picks one of the cube's six faces by its
//! dominant axis; the other two coordinates, divided by the dominant one, give a gnomonic
//! `(u, v)` on that face.  A tangent warp `s = (2/pi)*atan(u) + 1/2` (and the matching one
//! for `v`) evens the cell areas out, so that every cell at a given level has an area within
//! a factor of sqrt(2) of every other.  Each face is then a `2^level x 2^level` grid indexed
//! by `(i, j)`, and a cell is a spherical quadrilateral whose four edges are great circles.
//!
//! Because the warp and the grid are applied per face in closed form, the scheme has two
//! properties Uber's H3 cannot offer: a cell's parent is an exact prefix of its index (a
//! bit mask, never a lookup), and containment is a closed-form four-half-space test.  The
//! poles are ordinary interior points of the +z and -z faces, so there is no polar special
//! case.
//!
//! An id packs into one `u64`:
//!
//! ```text
//!   [ 4 bits scheme | 3 bits face | 5 bits level | 52 bits interleaved Morton(i, j) ]
//! ```
//!
//! The Morton field is left-justified, so masking its low bits yields an ancestor's field
//! directly.  The scheme nibble is `0x1` ("cube-tan-v1"); other values are reserved for
//! future warps or face layouts.  The string form is sixteen lowercase hexadecimal digits.
//!
//! # Provenance
//!
//! The face and `(u, v)` conventions are those of Google's S2 geometry library, chosen
//! because adjacent faces share edges continuously under them; the tangent warp is S2's as
//! well.  The quadtree indexing, the `u64` layout, the exact-parent property and the
//! re-indexing neighbour walk are this library's own.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    f64::consts::{FRAC_2_PI, FRAC_PI_2},
    fmt,
    str::FromStr,
};

/// The deepest level; a level-26 cell is roughly 14 cm across at the equator.
pub const MAX_LEVEL: u8 = 26;

// Bit layout of the u64 id.
const SCHEME_SHIFT:	u64 = 60;
const FACE_SHIFT:	u64 = 57;
const LEVEL_SHIFT:	u64 = 52;
const FACE_MASK:	u64 = 0x7 << FACE_SHIFT;
const LEVEL_MASK:	u64 = 0x1F << LEVEL_SHIFT;
const FIELD_MASK:	u64 = (1u64 << LEVEL_SHIFT) - 1;	// low 52 bits
const SCHEME_CUBE_TAN_V1: u64 = 0x1;

// ---------------------------------------------------------------------------------------------
// Cube faces
// ---------------------------------------------------------------------------------------------

/// One of the six faces of the cube enclosing the sphere.
///
/// The discriminants are the on-wire face indices, so `PosX` is 0 and `NegZ` is 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    PosX,	// 0, +x dominant
    PosY,	// 1, +y dominant
    PosZ,	// 2, +z dominant (north pole is its interior)
    NegX,	// 3
    NegY,	// 4
    NegZ,	// 5, -z dominant (south pole is its interior)
}

impl Face {
    fn index(self) -> u8 {
        match self {
            Self::PosX	=> 0,
            Self::PosY	=> 1,
            Self::PosZ	=> 2,
            Self::NegX	=> 3,
            Self::NegY	=> 4,
            Self::NegZ	=> 5,
        }
    }

    fn from_index(n: u8) -> Outcome<Face> {
        match n {
            0	=> Ok(Self::PosX),
            1	=> Ok(Self::PosY),
            2	=> Ok(Self::PosZ),
            3	=> Ok(Self::NegX),
            4	=> Ok(Self::NegY),
            5	=> Ok(Self::NegZ),
            _	=> Err(err!("Cell face index {} is out of range 0..=5.", n; Invalid, Input)),
        }
    }

    /// The face whose dominant axis best matches the direction `v`.
    ///
    /// On a tie between axes -- a point exactly on a cube edge or vertex -- the lowest axis
    /// index wins (x before y before z).  This is the single tie rule the whole grid shares,
    /// so that boundary points have exactly one owning cell.
    fn of_vec(v: &[f64; 3]) -> Face {
        let (ax, ay, az) = (v[0].abs(), v[1].abs(), v[2].abs());
        if ax >= ay && ax >= az {
            if v[0] >= 0.0 { Self::PosX } else { Self::NegX }
        } else if ay >= az {
            if v[1] >= 0.0 { Self::PosY } else { Self::NegY }
        } else if v[2] >= 0.0 {
            Self::PosZ
        } else {
            Self::NegZ
        }
    }

    /// Turns a face-local `(u, v)` into an (unnormalised) direction.
    ///
    /// The inverse of [`Face::vec_to_uv`] up to length.  These formulae are S2's, so that
    /// the shared edge between two faces is parameterised identically from both sides.
    fn uv_to_vec(self, u: f64, v: f64) -> [f64; 3] {
        match self {
            Self::PosX	=> [ 1.0,    u,    v],
            Self::PosY	=> [  -u,  1.0,    v],
            Self::PosZ	=> [  -u,   -v,  1.0],
            Self::NegX	=> [-1.0,   -v,   -u],
            Self::NegY	=> [   v, -1.0,   -u],
            Self::NegZ	=> [   v,    u, -1.0],
        }
    }

    /// Turns a direction known to lie on this face into its `(u, v)`.
    ///
    /// The dominant component is the divisor, so the result lies in `[-1, 1]^2`.
    fn vec_to_uv(self, p: &[f64; 3]) -> (f64, f64) {
        let (x, y, z) = (p[0], p[1], p[2]);
        match self {
            Self::PosX	=> ( y / x,  z / x),
            Self::PosY	=> (-x / y,  z / y),
            Self::PosZ	=> (-x / z, -y / z),
            Self::NegX	=> ( z / x,  y / x),
            Self::NegY	=> ( z / y, -x / y),
            Self::NegZ	=> (-y / z, -x / z),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Scalar geometry
// ---------------------------------------------------------------------------------------------

/// The tangent warp: face coordinate `u` in `[-1, 1]` to grid coordinate `s` in `[0, 1]`.
fn uv_to_st(u: f64) -> f64 { FRAC_2_PI * u.atan() + 0.5 }

/// The inverse tangent warp: grid coordinate `s` to face coordinate `u`.
fn st_to_uv(s: f64) -> f64 { ((s - 0.5) * FRAC_PI_2).tan() }

fn latlon_to_vec(lat_deg: f64, lon_deg: f64) -> [f64; 3] {
    let (la, lo) = (lat_deg.to_radians(), lon_deg.to_radians());
    let cl = la.cos();
    [cl * lo.cos(), cl * lo.sin(), la.sin()]
}

fn vec_to_latlon(v: &[f64; 3]) -> (f64, f64) {
    let len = (v[0]*v[0] + v[1]*v[1] + v[2]*v[2]).sqrt();
    let z = (v[2] / len).clamp(-1.0, 1.0);
    (z.asin().to_degrees(), v[1].atan2(v[0]).to_degrees())
}

fn normalise(v: &[f64; 3]) -> [f64; 3] {
    let len = (v[0]*v[0] + v[1]*v[1] + v[2]*v[2]).sqrt();
    [v[0]/len, v[1]/len, v[2]/len]
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]]
}

// ---------------------------------------------------------------------------------------------
// Morton interleave
// ---------------------------------------------------------------------------------------------

/// Interleaves two indices, `i` in the even bit positions and `j` in the odd.
///
/// Each input carries at most [`MAX_LEVEL`] bits, so the result carries at most 52.
fn interleave(i: u32, j: u32) -> u64 {
    let mut m = 0u64;
    let mut b = 0u32;
    while b < MAX_LEVEL as u32 {
        m |= (((i >> b) & 1) as u64) << (2 * b);
        m |= (((j >> b) & 1) as u64) << (2 * b + 1);
        b += 1;
    }
    m
}

/// The inverse of [`interleave`].
fn deinterleave(m: u64) -> (u32, u32) {
    let mut i = 0u32;
    let mut j = 0u32;
    let mut b = 0u32;
    while b < MAX_LEVEL as u32 {
        i |= (((m >> (2 * b)) & 1) as u32) << b;
        j |= (((m >> (2 * b + 1)) & 1) as u32) << b;
        b += 1;
    }
    (i, j)
}

// ---------------------------------------------------------------------------------------------
// Corner
// ---------------------------------------------------------------------------------------------

/// A cell corner as both a unit direction and its geodetic coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    pub vec:	[f64; 3],	// unit direction from the sphere's centre
    pub lat:	f64,		// degrees, positive north
    pub lon:	f64,		// degrees, positive east
}

// ---------------------------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------------------------

/// A single cell of the cube-sphere quadtree grid, addressed by one packed `u64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cell(u64);

impl Cell {
    /// The cell of the given level containing the point at `lat_deg`, `lon_deg`.
    ///
    /// Latitude is degrees north, longitude degrees east; neither need be pre-wrapped.  A
    /// point on a cell boundary is resolved by the shared tie rule (see [`Face::of_vec`] and
    /// the half-open grid convention), so every point has exactly one owning cell.
    pub fn at(lat_deg: f64, lon_deg: f64, level: u8) -> Outcome<Cell> {
        if level > MAX_LEVEL {
            return Err(err!("Cell level {} exceeds the maximum {}.", level, MAX_LEVEL;
                Invalid, Input, Range));
        }
        let v = latlon_to_vec(lat_deg, lon_deg);
        let face = Face::of_vec(&v);
        let (u, w) = face.vec_to_uv(&v);
        let (s, t) = (uv_to_st(u), uv_to_st(w));
        let n = 1u32 << level;
        let (i, j) = (clamp_index(s, n), clamp_index(t, n));
        Cell::from_face_ij(face.index(), level, i, j)
    }

    /// Builds a cell directly from its face, level and grid indices.
    pub fn from_face_ij(face: u8, level: u8, i: u32, j: u32) -> Outcome<Cell> {
        let f = res!(Face::from_index(face));
        if level > MAX_LEVEL {
            return Err(err!("Cell level {} exceeds the maximum {}.", level, MAX_LEVEL;
                Invalid, Input, Range));
        }
        let n = 1u32 << level;
        if i >= n || j >= n {
            return Err(err!("Cell index ({}, {}) is out of range for level {} (0..{}).",
                i, j, level, n; Invalid, Input, Range));
        }
        let field = interleave(i, j) << (LEVEL_SHIFT - 2 * level as u64);
        let bits = (SCHEME_CUBE_TAN_V1 << SCHEME_SHIFT)
            | ((f.index() as u64) << FACE_SHIFT)
            | ((level as u64) << LEVEL_SHIFT)
            | (field & FIELD_MASK);
        Ok(Cell(bits))
    }

    /// Validates and wraps a raw `u64`, rejecting every malformed field.
    pub fn from_bits(bits: u64) -> Outcome<Cell> {
        let scheme = bits >> SCHEME_SHIFT;
        if scheme != SCHEME_CUBE_TAN_V1 {
            return Err(err!("Cell scheme nibble {:#x} is not the cube-tan-v1 scheme {:#x}.",
                scheme, SCHEME_CUBE_TAN_V1; Invalid, Input));
        }
        let face = ((bits & FACE_MASK) >> FACE_SHIFT) as u8;
        res!(Face::from_index(face));
        let level = ((bits & LEVEL_MASK) >> LEVEL_SHIFT) as u8;
        if level > MAX_LEVEL {
            return Err(err!("Cell level {} exceeds the maximum {}.", level, MAX_LEVEL;
                Invalid, Input, Range));
        }
        let unused = (1u64 << (LEVEL_SHIFT - 2 * level as u64)) - 1; // low bits that must be zero
        if bits & unused != 0 {
            return Err(err!("Cell id has non-zero unused low bits below level {}.", level;
                Invalid, Input));
        }
        Ok(Cell(bits))
    }

    /// The raw packed id.
    pub fn bits(&self) -> u64 { self.0 }

    /// The level, `0..=MAX_LEVEL`.
    pub fn level(&self) -> u8 { ((self.0 & LEVEL_MASK) >> LEVEL_SHIFT) as u8 }

    /// The face index, `0..=5`.
    pub fn face(&self) -> u8 { ((self.0 & FACE_MASK) >> FACE_SHIFT) as u8 }

    fn face_enum(&self) -> Face {
        // The stored face is validated on every construction path, so it is always in range.
        match Face::from_index(self.face()) {
            Ok(f) => f,
            Err(_) => Face::PosX,
        }
    }

    /// The grid indices `(i, j)` at the cell's own level.
    pub fn coords(&self) -> (u32, u32) {
        let level = self.level();
        let morton = (self.0 & FIELD_MASK) >> (LEVEL_SHIFT - 2 * level as u64);
        deinterleave(morton)
    }

    /// The ancestor at level `level`, obtained by masking the Morton field.
    ///
    /// Because the field is left-justified, this is an exact prefix operation -- no
    /// re-projection and no rounding.  The target level must not exceed this cell's.
    pub fn parent(&self, level: u8) -> Outcome<Cell> {
        let own = self.level();
        if level > own {
            return Err(err!("Cannot take a level-{} parent of a level-{} cell.", level, own;
                Invalid, Input, Range));
        }
        // Clear the Morton bits below the ancestor level, then restamp the level field.
        let clear = if level == 0 { FIELD_MASK } else { (1u64 << (LEVEL_SHIFT - 2 * level as u64)) - 1 };
        let bits = (self.0 & !LEVEL_MASK & !clear) | ((level as u64) << LEVEL_SHIFT);
        Ok(Cell(bits))
    }

    /// The four child cells at the next level, in Morton order.
    pub fn children(&self) -> Outcome<[Cell; 4]> {
        let level = self.level();
        if level >= MAX_LEVEL {
            return Err(err!("A level-{} cell has no children.", level; Invalid, Input, Range));
        }
        let (i, j) = self.coords();
        let face = self.face();
        Ok([
            res!(Cell::from_face_ij(face, level + 1, 2*i,     2*j)),
            res!(Cell::from_face_ij(face, level + 1, 2*i + 1, 2*j)),
            res!(Cell::from_face_ij(face, level + 1, 2*i,     2*j + 1)),
            res!(Cell::from_face_ij(face, level + 1, 2*i + 1, 2*j + 1)),
        ])
    }

    /// The centre of the cell as a unit direction.
    pub fn centre_vec(&self) -> [f64; 3] {
        let level = self.level();
        let (i, j) = self.coords();
        let n = (1u32 << level) as f64;
        let s = (i as f64 + 0.5) / n;
        let t = (j as f64 + 0.5) / n;
        normalise(&self.face_enum().uv_to_vec(st_to_uv(s), st_to_uv(t)))
    }

    /// The centre of the cell as `(lat, lon)` in degrees.
    pub fn centre(&self) -> (f64, f64) { vec_to_latlon(&self.centre_vec()) }

    /// The four corners, counter-clockwise in the face's `(s, t)` frame.
    pub fn corners(&self) -> [Corner; 4] {
        let level = self.level();
        let (i, j) = self.coords();
        let n = (1u32 << level) as f64;
        let face = self.face_enum();
        let st = [
            (i as f64 / n,           j as f64 / n),
            ((i as f64 + 1.0) / n,   j as f64 / n),
            ((i as f64 + 1.0) / n,   (j as f64 + 1.0) / n),
            (i as f64 / n,           (j as f64 + 1.0) / n),
        ];
        let mut out = [Corner { vec: [0.0; 3], lat: 0.0, lon: 0.0 }; 4];
        for (k, (s, t)) in st.iter().enumerate() {
            let vec = normalise(&face.uv_to_vec(st_to_uv(*s), st_to_uv(*t)));
            let (lat, lon) = vec_to_latlon(&vec);
            out[k] = Corner { vec, lat, lon };
        }
        out
    }

    /// Does the cell contain the point at `lat_deg`, `lon_deg`?
    ///
    /// A cell's four edges are great circles, so containment is exact: the point belongs to
    /// exactly the cell that [`Cell::at`] returns for it at this level.  Both share the one
    /// tie rule, so a point on a shared boundary is contained by exactly one cell.
    pub fn contains(&self, lat_deg: f64, lon_deg: f64) -> Outcome<bool> {
        let owner = res!(Cell::at(lat_deg, lon_deg, self.level()));
        Ok(owner == *self)
    }

    /// The cell's area in steradians, by spherical excess of its two triangles.
    pub fn area(&self) -> f64 {
        let c = self.corners();
        tri_area(&c[0].vec, &c[1].vec, &c[2].vec) + tri_area(&c[0].vec, &c[2].vec, &c[3].vec)
    }

    /// The eight (or, at a cube vertex, seven) cells sharing an edge or corner with this one.
    ///
    /// A step within the face is exact index arithmetic; a step that leaves the face is
    /// re-indexed through the sphere, which resolves the face change and its orientation flip
    /// automatically.  At the 24 cells per level that touch a cube vertex, the diagonal step
    /// across the vertex lands on an already-listed cell, so those cells have seven neighbours.
    pub fn neighbours(&self) -> Outcome<Vec<Cell>> {
        const DIRS: [(i32, i32); 8] = [
            (1, 0), (-1, 0), (0, 1), (0, -1),	// edge-adjacent
            (1, 1), (1, -1), (-1, 1), (-1, -1),	// corner-adjacent
        ];
        let level = self.level();
        let (i, j) = self.coords();
        let face = self.face_enum();
        let n = 1i64 << level;
        let mut out: Vec<Cell> = Vec::with_capacity(8);
        for (di, dj) in DIRS.iter() {
            let ni = i as i64 + *di as i64;
            let nj = j as i64 + *dj as i64;
            let cell = if ni >= 0 && ni < n && nj >= 0 && nj < n {
                res!(Cell::from_face_ij(face.index(), level, ni as u32, nj as u32))
            } else {
                // Leave the face: a representative point a quarter-cell past the crossed edge,
                // centred on the in-range axis.  A quarter cell never reaches the far edge
                // (which would be a singular tangent), so no infinity ever arises.
                let nf = n as f64;
                let s = axis_rep(ni, n, nf);
                let t = axis_rep(nj, n, nf);
                let vec = face.uv_to_vec(st_to_uv(s), st_to_uv(t));
                let (lat, lon) = vec_to_latlon(&vec);
                res!(Cell::at(lat, lon, level))
            };
            if cell != *self && !out.contains(&cell) {
                out.push(cell);
            }
        }
        Ok(out)
    }

    /// The cells at exactly graph-distance `k` from this one, over the neighbour relation.
    ///
    /// `ring(0)` is the cell itself; `ring(1)` is [`Cell::neighbours`].
    pub fn ring(&self, k: u32) -> Outcome<Vec<Cell>> {
        let mut seen: std::collections::HashSet<Cell> = std::collections::HashSet::new();
        let mut frontier = vec![*self];
        seen.insert(*self);
        for _ in 0..k {
            let mut next = Vec::new();
            for cell in &frontier {
                for nb in res!(cell.neighbours()) {
                    if seen.insert(nb) {
                        next.push(nb);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        Ok(frontier)
    }
}

/// The `(s, t)` coordinate representing a stepped neighbour along one axis.
///
/// In range, the cell centre; below the face, a quarter-cell before the near edge; above it,
/// a quarter-cell past the far edge.
fn axis_rep(idx: i64, n: i64, nf: f64) -> f64 {
    if idx < 0 {
        -0.25 / nf
    } else if idx >= n {
        1.0 + 0.25 / nf
    } else {
        (idx as f64 + 0.5) / nf
    }
}

/// Floors a warped coordinate `s` in `[0, 1]` to a grid index, clamped to `0..n`.
fn clamp_index(s: f64, n: u32) -> u32 {
    let raw = (s * n as f64).floor();
    if raw < 0.0 {
        0
    } else if raw >= n as f64 {
        n - 1
    } else {
        raw as u32
    }
}

/// Spherical-excess area of a unit-vector triangle (Van Oosterom & Strackee).
fn tri_area(a: &[f64; 3], b: &[f64; 3], c: &[f64; 3]) -> f64 {
    let triple = dot(a, &cross(b, c)).abs();
    let den = 1.0 + dot(a, b) + dot(b, c) + dot(c, a);
    2.0 * triple.atan2(den)
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl FromStr for Cell {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> Outcome<Cell> {
        if s.len() != 16 {
            return Err(err!("A cell id must be 16 hex characters, got {}.", s.len();
                Invalid, Input));
        }
        let bits = res!(u64::from_str_radix(s, 16), Invalid, Input);
        Cell::from_bits(bits)
    }
}
