//! An offline world map: coastlines, lakes, borders and place names as unprojected positions.
//!
//! # Provenance
//!
//! The file layout, the delta coding and the spherical simplifier are Ochre's
//! (`web/apps/ochre/dev/gen_world.py` and `src/web/globe.rs`, August 2026), moved here when
//! Oxegen became the second program to draw the world.  Version 1 is Ochre's layout byte for
//! byte with the magic generalised from `OCHRWRLD` to `FE2O3WLD`; version 2 adds label layers.
//!
//! # The layout
//!
//! ```text
//!   magic "FE2O3WLD" | u16 version | u16 layer count
//!   one 20-byte directory entry a layer:
//!     u8 kind (0 fill, 1 stroke, 2 label) | u8 level of detail | u8 name length
//!     | 7 bytes of ASCII name, zero padded | u16 tolerance in hundreds of metres
//!     | u32 first ring (or label) | u32 ring (or label) count
//!   one u32 vertex count a ring, for every ring of every fill and stroke layer
//!   the rings: i32 lat, i32 lng in whole hundred-thousandths of a degree, then one step a
//!     vertex as i16 dlat, i16 dlng; a step of dlat = -32768 is a jump, and the eight bytes
//!     after it are the next position in full
//!   version 2 only, the labels: i32 lat, i32 lng, u8 rank, u8 name length, UTF-8 name
//! ```
//!
//! All numbers are little-endian.  A hundred-thousandth of a degree is 1.11 m of latitude.  A
//! step is taken between rounded positions rather than rounded itself, so error cannot build
//! up along a coastline, and a step too long for sixteen bits is written as a jump rather than
//! broken up, so every position in the file is a source position.  Longitude steps take the
//! short way round, so a ring's running longitude may walk past 180 degrees; to a sine that is
//! no trouble.
//!
//! # Simplification
//!
//! [`simplify_sphere`] is Douglas and Peucker's, measured in three dimensions against the
//! chord.  The orthographic projection is parallel, hence linear, so the straight line a
//! canvas draws between two kept vertices is exactly the chord the tolerance was measured
//! against: nothing needs densifying and nothing is cut at the antimeridian.

use crate::proj::{
    EARTH_RADIUS_M,
    unit_vec,
};

use oxedyne_fe2o3_core::prelude::*;

pub const MAGIC: &[u8; 8] = b"FE2O3WLD";
pub const VERSION_RINGS: u16 = 1;   // fill and stroke layers only: Ochre's layout
pub const VERSION_LABELS: u16 = 2;  // adds label layers
pub const Q: f64 = 100_000.0;       // positions are whole hundred-thousandths of a degree

const HEADER:       usize = 12;
const DIRENT:       usize = 20;
const NAME_MAX:     usize = 7;
const STEP_LIMIT:   i64 = 32_767;
const JUMP:         i16 = -32_768;  // a step that is not a step: a position in full follows
const HALF_TURN:    i64 = 180 * 100_000;

/// How a layer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Fill,   // closed rings, painted
    Stroke, // open lines, stroked
    Label,  // named points
}

impl LayerKind {
    fn code(self) -> u8 {
        match self {
            Self::Fill      => 0,
            Self::Stroke    => 1,
            Self::Label     => 2,
        }
    }

    fn from_code(c: u8) -> Outcome<Self> {
        match c {
            0   => Ok(Self::Fill),
            1   => Ok(Self::Stroke),
            2   => Ok(Self::Label),
            _   => Err(err!("A world layer of kind {} is none this reads.", c; Invalid, Input)),
        }
    }
}

/// A named point: a town, a city, a capital.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub lat:    f64,    // degrees
    pub lng:    f64,    // degrees
    pub rank:   u8,     // importance, 0 the most: Natural Earth's `scalerank`
    pub name:   String, // UTF-8, at most 255 bytes
}

/// One layer of the world at one level of detail.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name:   String,                 // at most 7 ASCII bytes: `land`, `lakes`, `borders`
    pub kind:   LayerKind,
    pub detail: u8,                     // level of detail, 0 the coarsest
    pub tol_m:  f64,                    // simplification tolerance, kept in hundreds of metres
    pub rings:  Vec<Vec<(f64, f64)>>,   // (lat, lng) in degrees, for Fill and Stroke
    pub labels: Vec<Label>,             // for Label
}

impl Layer {
    /// The rings as unit vectors, which is what [`crate::proj::Viewport::project_rings`]
    /// draws.  A caller drawing every frame converts once and keeps the result.
    pub fn unit_rings(&self) -> Vec<Vec<[f64; 3]>> {
        self.rings.iter()
            .map(|r| r.iter().map(|(lat, lng)| unit_vec(*lat, *lng)).collect())
            .collect()
    }
}

/// A world: its layers, in the order the file holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    pub layers: Vec<Layer>,
}

impl World {
    pub fn layer(&self, name: &str, detail: u8) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name && l.detail == detail)
    }

    /// The number of levels of detail, one more than the finest.
    pub fn details(&self) -> u8 {
        self.layers.iter().map(|l| l.detail.saturating_add(1)).max().unwrap_or(0)
    }
}

/// Reads every level of a world file.
pub fn read(bytes: &[u8]) -> Outcome<World> {
    read_detail(bytes, None)
}

/// Reads a world file, building only the layers of one level of detail when `detail` names
/// one.
///
/// Every ring is walked whatever level it belongs to, because a jump makes a ring's length in
/// bytes unknowable from its length in vertices, but only the wanted rings are built.
pub fn read_detail(bytes: &[u8], detail: Option<u8>) -> Outcome<World> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return Err(err!("The world file does not begin as one."; Invalid, Input));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != VERSION_RINGS && version != VERSION_LABELS {
        return Err(err!("The world file is version {}, and this reads versions {} and {}.",
            version, VERSION_RINGS, VERSION_LABELS; Invalid, Input, Version));
    }
    let count = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    let table_at = HEADER + count * DIRENT;
    if bytes.len() < table_at {
        return Err(err!("The world file's directory is cut short at {} of {} bytes.",
            bytes.len(), table_at; Invalid, Input));
    }

    struct Dir {
        kind:   LayerKind,
        detail: u8,
        name:   String,
        tol_m:  f64,
        first:  usize,
        n:      usize,
    }
    let mut dirs: Vec<Dir> = Vec::with_capacity(count);
    let mut rings = 0usize;
    let mut labels = 0usize;
    for i in 0..count {
        let at = HEADER + i * DIRENT;
        let kind = res!(LayerKind::from_code(bytes[at]));
        if kind == LayerKind::Label && version == VERSION_RINGS {
            return Err(err!("Layer {} of a version {} world file holds labels.", i, version;
                Invalid, Input));
        }
        let len = bytes[at + 2] as usize;
        if len > NAME_MAX {
            return Err(err!("Layer {} claims a name of {} bytes, more than {}.", i, len, NAME_MAX;
                Invalid, Input));
        }
        let name = match std::str::from_utf8(&bytes[at + 3..at + 3 + len]) {
            Ok(s)   => s.to_string(),
            Err(_)  => return Err(err!("Layer {} has a name that is not text.", i; Invalid, Input)),
        };
        let tol_m = u16::from_le_bytes([bytes[at + 10], bytes[at + 11]]) as f64 * 100.0;
        let first = res!(u32_at(bytes, at + 12)) as usize;
        let n = res!(u32_at(bytes, at + 16)) as usize;
        match kind {
            LayerKind::Label    => labels = labels.max(first + n),
            _                   => rings = rings.max(first + n),
        }
        dirs.push(Dir { kind, detail: bytes[at + 1], name, tol_m, first, n });
    }
    let mut at = table_at + rings * 4;
    if bytes.len() < at {
        return Err(err!("The world file's ring table is cut short at {} of {} bytes.",
            bytes.len(), at; Invalid, Input));
    }

    let mut wanted = vec![false; rings];
    for d in &dirs {
        if d.kind != LayerKind::Label && detail.map_or(true, |w| w == d.detail) {
            for r in d.first..(d.first + d.n).min(rings) {
                wanted[r] = true;
            }
        }
    }
    let mut built: Vec<Vec<(f64, f64)>> = vec![Vec::new(); rings];
    for r in 0..rings {
        let n = res!(u32_at(bytes, table_at + r * 4)) as usize;
        let (drawn, next) = res!(read_ring(bytes, at, n, wanted[r], r));
        built[r] = drawn;
        at = next;
    }

    let mut read_labels: Vec<Label> = Vec::with_capacity(labels);
    for i in 0..labels {
        if at + 10 > bytes.len() {
            return Err(err!("The world file ends inside label {} of {}.", i, labels;
                Invalid, Input));
        }
        let lat = res!(u32_at(bytes, at)) as i32;
        let lng = res!(u32_at(bytes, at + 4)) as i32;
        let rank = bytes[at + 8];
        let len = bytes[at + 9] as usize;
        let end = at + 10 + len;
        if end > bytes.len() {
            return Err(err!("The world file ends inside the name of label {}.", i; Invalid, Input));
        }
        let name = match std::str::from_utf8(&bytes[at + 10..end]) {
            Ok(s)   => s.to_string(),
            Err(_)  => return Err(err!("Label {} has a name that is not UTF-8.", i; Invalid, Input)),
        };
        read_labels.push(Label { lat: lat as f64 / Q, lng: lng as f64 / Q, rank, name });
        at = end;
    }
    if at != bytes.len() {
        return Err(err!("The world file has {} bytes after its last position.",
            bytes.len() as i64 - at as i64; Invalid, Input));
    }

    let mut world = World::default();
    for d in dirs {
        if !detail.map_or(true, |w| w == d.detail) {
            continue;
        }
        let mut layer = Layer {
            name: d.name, kind: d.kind, detail: d.detail, tol_m: d.tol_m,
            rings: Vec::new(), labels: Vec::new(),
        };
        match d.kind {
            LayerKind::Label => {
                layer.labels = read_labels[d.first..d.first + d.n].to_vec();
            },
            _ => {
                layer.rings = built[d.first..d.first + d.n].to_vec();
            },
        }
        world.layers.push(layer);
    }
    Ok(world)
}

/// One ring as degrees, and where the next one begins.
fn read_ring(bytes: &[u8], at: usize, n: usize, keep: bool, r: usize)
    -> Outcome<(Vec<(f64, f64)>, usize)>
{
    if at + 8 > bytes.len() || n == 0 {
        return Err(err!("The world file ends inside ring {} at byte {}.", r, at; Invalid, Input));
    }
    let mut lat = res!(u32_at(bytes, at)) as i32;
    let mut lng = res!(u32_at(bytes, at + 4)) as i32;
    let mut out = Vec::with_capacity(if keep { n } else { 0 });
    if keep {
        out.push((lat as f64 / Q, lng as f64 / Q));
    }
    let mut step = at + 8;
    for _ in 1..n {
        if step + 4 > bytes.len() {
            return Err(err!("The world file ends inside ring {} at byte {}.", r, step;
                Invalid, Input));
        }
        let dlat = i16::from_le_bytes([bytes[step], bytes[step + 1]]);
        if dlat == JUMP {
            if step + 12 > bytes.len() {
                return Err(err!("The world file ends inside a jump in ring {}.", r; Invalid, Input));
            }
            lat = res!(u32_at(bytes, step + 4)) as i32;
            lng = res!(u32_at(bytes, step + 8)) as i32;
            step += 12;
        } else {
            lat = lat.wrapping_add(dlat as i32);
            lng = lng.wrapping_add(i16::from_le_bytes([bytes[step + 2], bytes[step + 3]]) as i32);
            step += 4;
        }
        if keep {
            out.push((lat as f64 / Q, lng as f64 / Q));
        }
    }
    Ok((out, step))
}

fn u32_at(bytes: &[u8], at: usize) -> Outcome<u32> {
    match bytes.get(at..at + 4) {
        Some(b) => Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        None    => Err(err!("The world file ends inside a number at byte {}.", at; Invalid, Input)),
    }
}

/// Writes a world file: version 1, Ochre's layout, when it holds no labels, and version 2
/// otherwise.
///
/// Positions are rounded to the nearest hundred-thousandth of a degree, ties to even, and a
/// vertex that rounds onto the one before it is dropped; a ring left with fewer than two
/// vertices is dropped with it.  Reading the result back and writing it again gives the same
/// bytes.
pub fn write(world: &World) -> Outcome<Vec<u8>> {
    let labelled = world.layers.iter().any(|l| l.kind == LayerKind::Label);
    let version = if labelled { VERSION_LABELS } else { VERSION_RINGS };
    if world.layers.len() > u16::MAX as usize {
        return Err(err!("A world of {} layers is more than a file can list.", world.layers.len();
            Invalid, Input, Excessive));
    }
    let mut head: Vec<u8> = Vec::with_capacity(HEADER + DIRENT * world.layers.len());
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&version.to_le_bytes());
    head.extend_from_slice(&(world.layers.len() as u16).to_le_bytes());
    let mut table: Vec<u8> = Vec::new();
    let mut coords: Vec<u8> = Vec::new();
    let mut names: Vec<u8> = Vec::new();
    let mut ring_at = 0u32;
    let mut label_at = 0u32;
    for (i, layer) in world.layers.iter().enumerate() {
        let name = layer.name.as_bytes();
        if name.len() > NAME_MAX || !layer.name.is_ascii() {
            return Err(err!("Layer {} is named {:?}; a name is at most {} ASCII bytes.",
                i, layer.name, NAME_MAX; Invalid, Input));
        }
        let tol = (layer.tol_m / 100.0).round();
        if !(tol >= 0.0 && tol <= u16::MAX as f64) {
            return Err(err!("Layer {} has a tolerance of {} m, outside 0 to {} m.",
                i, layer.tol_m, u16::MAX as f64 * 100.0; Invalid, Input, Range));
        }
        let (first, n) = match layer.kind {
            LayerKind::Label => {
                for (k, label) in layer.labels.iter().enumerate() {
                    let text = label.name.as_bytes();
                    if text.len() > u8::MAX as usize {
                        return Err(err!("Label {} of layer {} has a name of {} bytes, more than {}.",
                            k, i, text.len(), u8::MAX; Invalid, Input, Excessive));
                    }
                    names.extend_from_slice(&res!(quantise(label.lat, i)).to_le_bytes());
                    names.extend_from_slice(&res!(quantise(label.lng, i)).to_le_bytes());
                    names.push(label.rank);
                    names.push(text.len() as u8);
                    names.extend_from_slice(text);
                }
                let first = label_at;
                label_at += layer.labels.len() as u32;
                (first, layer.labels.len() as u32)
            },
            _ => {
                let mut kept = 0u32;
                for ring in &layer.rings {
                    if let Some(n) = res!(write_ring(ring, i, &mut coords)) {
                        table.extend_from_slice(&n.to_le_bytes());
                        kept += 1;
                    }
                }
                let first = ring_at;
                ring_at += kept;
                (first, kept)
            },
        };
        head.push(layer.kind.code());
        head.push(layer.detail);
        head.push(name.len() as u8);
        let mut padded = [0u8; NAME_MAX];
        padded[..name.len()].copy_from_slice(name);
        head.extend_from_slice(&padded);
        head.extend_from_slice(&(tol as u16).to_le_bytes());
        head.extend_from_slice(&first.to_le_bytes());
        head.extend_from_slice(&n.to_le_bytes());
    }
    head.extend_from_slice(&table);
    head.extend_from_slice(&coords);
    head.extend_from_slice(&names);
    Ok(head)
}

/// A coordinate in whole hundred-thousandths of a degree, rounded half to even.
fn quantise(deg: f64, layer: usize) -> Outcome<i32> {
    let q = (deg * Q).round_ties_even();
    if !(q >= i32::MIN as f64 && q <= i32::MAX as f64) {
        return Err(err!("A coordinate of {} degrees in layer {} cannot be written.", deg, layer;
            Invalid, Input, Range));
    }
    Ok(q as i32)
}

/// Appends one ring, returning how many vertices it came to, or `None` if it came to fewer
/// than two and was not written.
fn write_ring(pts: &[(f64, f64)], layer: usize, out: &mut Vec<u8>) -> Outcome<Option<u32>> {
    let mut grid: Vec<(i64, i64)> = Vec::with_capacity(pts.len());
    for (lat, lng) in pts {
        let q = (res!(quantise(*lat, layer)) as i64, res!(quantise(*lng, layer)) as i64);
        if grid.last() != Some(&q) {
            grid.push(q);
        }
    }
    if grid.len() < 2 {
        return Ok(None);
    }
    out.extend_from_slice(&(grid[0].0 as i32).to_le_bytes());
    out.extend_from_slice(&(grid[0].1 as i32).to_le_bytes());
    let (mut py, mut px) = grid[0];
    for (qy, qx) in grid.iter().skip(1) {
        let dy = qy - py;
        // The short way round: a ring crossing the antimeridian steps a whole turn in the data
        // and a few hundred metres on the ground.
        let dx = (qx - px + HALF_TURN).rem_euclid(2 * HALF_TURN) - HALF_TURN;
        let (ny, nx) = (py + dy, px + dx);
        if dy.abs() > STEP_LIMIT || dx.abs() > STEP_LIMIT {
            if ny < i32::MIN as i64 || ny > i32::MAX as i64 || nx < i32::MIN as i64 || nx > i32::MAX as i64 {
                return Err(err!("A ring in layer {} wanders past what a jump can hold.", layer;
                    Invalid, Input, Range));
            }
            out.extend_from_slice(&JUMP.to_le_bytes());
            out.extend_from_slice(&0i16.to_le_bytes());
            out.extend_from_slice(&(ny as i32).to_le_bytes());
            out.extend_from_slice(&(nx as i32).to_le_bytes());
        } else {
            out.extend_from_slice(&(dy as i16).to_le_bytes());
            out.extend_from_slice(&(dx as i16).to_le_bytes());
        }
        py = ny;
        px = nx;
    }
    Ok(Some(grid.len() as u32))
}

/// Douglas and Peucker's simplification of a run of unit vectors, against the chord, returning
/// the indices kept in order.
///
/// The distance measured is the perpendicular in three dimensions from a vertex to the straight
/// segment through the kept pair either side of it, on a sphere of [`EARTH_RADIUS_M`], which is
/// the distance from the edge a globe actually draws.  The first and last vertices are always
/// kept.  Iterative, so a coastline of a million vertices cannot overflow the stack.
pub fn simplify_sphere(pts: &[[f64; 3]], eps_m: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 3 || !(eps_m > 0.0) {
        return (0..n).collect();
    }
    let eps = eps_m / EARTH_RADIUS_M;
    let e2 = eps * eps;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack: Vec<(usize, usize)> = vec![(0, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let a = pts[i];
        let b = pts[j];
        let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        let dd = dx * dx + dy * dy + dz * dz;
        let mut best = -1.0;
        let mut at = i;
        for k in (i + 1)..j {
            let p = pts[k];
            let d2 = if dd == 0.0 {
                sq(p[0] - a[0]) + sq(p[1] - a[1]) + sq(p[2] - a[2])
            } else {
                let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy + (p[2] - a[2]) * dz) / dd)
                    .clamp(0.0, 1.0);
                sq(p[0] - a[0] - t * dx) + sq(p[1] - a[1] - t * dy) + sq(p[2] - a[2] - t * dz)
            };
            if d2 > best {
                best = d2;
                at = k;
            }
        }
        if best > e2 {
            keep[at] = true;
            stack.push((i, at));
            stack.push((at, j));
        }
    }
    (0..n).filter(|k| keep[*k]).collect()
}

fn sq(x: f64) -> f64 { x * x }
