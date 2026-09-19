//! Closed-form correctness oracle for the cube-sphere quadtree cell grid.
//!
//! There is no external reference implementation: these invariants ARE the specification.
//! Every check is offline and closed-form, so the suite is deterministic and hermetic.

use oxedyne_fe2o3_geom::cell::{Cell, Corner};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------------------------
// Test-local deterministic RNG (SplitMix64) -- no dev-dependency introduced.
// ---------------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self { Rng(seed) }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f64(&mut self) -> f64 { // [0, 1)
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    fn lat(&mut self) -> f64 { self.next_f64() * 180.0 - 90.0 }
    fn lon(&mut self) -> f64 { self.next_f64() * 360.0 - 180.0 }

    fn cell_at(&mut self, level: u8) -> Cell {
        let n: u32 = 1u32 << level;
        let face = (self.next_u64() % 6) as u8;
        let i = (self.next_u64() % n as u64) as u32;
        let j = (self.next_u64() % n as u64) as u32;
        match Cell::from_face_ij(face, level, i, j) {
            Ok(c) => c,
            Err(e) => panic!("from_face_ij failed for a valid cell: {}", e),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Independent geometric helpers used as oracles (deliberately NOT the production code path).
// ---------------------------------------------------------------------------------------------

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]]
}

fn norm(v: &[f64; 3]) -> [f64; 3] {
    let len = dot(v, v).sqrt();
    [v[0]/len, v[1]/len, v[2]/len]
}

fn latlon_to_vec(lat: f64, lon: f64) -> [f64; 3] {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos()*lo.cos(), la.cos()*lo.sin(), la.sin()]
}

/// Independent point-in-cell test using the four great-circle edges of the cell.
///
/// Each cell edge is the intersection of a plane through the origin with the sphere, so a
/// point is inside when it lies on the interior side of all four edge planes.  This is a
/// wholly separate implementation from `Cell::contains`, used only to cross-check it.
fn naive_contains(cell: &Cell, p: &[f64; 3]) -> bool {
    let c: [Corner; 4] = cell.corners();
    let centre = cell.centre_vec();
    let vs = [&c[0].vec, &c[1].vec, &c[2].vec, &c[3].vec];
    for e in 0..4 {
        let a = vs[e];
        let b = vs[(e + 1) % 4];
        let n = cross(a, b); // edge-plane normal
        // Orient the normal so the cell centre is on the positive side.
        let s = if dot(&n, &centre) >= 0.0 { 1.0 } else { -1.0 };
        if s * dot(&n, p) < 0.0 {
            return false;
        }
    }
    true
}

/// Spherical-excess area of a spherical triangle of unit vectors (Van Oosterom & Strackee).
fn tri_area(a: &[f64; 3], b: &[f64; 3], c: &[f64; 3]) -> f64 {
    let triple = dot(a, &cross(b, c)).abs();
    let den = 1.0 + dot(a, b) + dot(b, c) + dot(c, a);
    2.0 * triple.atan2(den)
}

fn cell_area_oracle(cell: &Cell) -> f64 {
    let c = cell.corners();
    tri_area(&c[0].vec, &c[1].vec, &c[2].vec) + tri_area(&c[0].vec, &c[2].vec, &c[3].vec)
}

fn all_cells(level: u8) -> Vec<Cell> {
    let n: u32 = 1u32 << level;
    let mut out = Vec::new();
    for face in 0u8..6 {
        for i in 0..n {
            for j in 0..n {
                match Cell::from_face_ij(face, level, i, j) {
                    Ok(c) => out.push(c),
                    Err(e) => panic!("enumeration built an invalid cell: {}", e),
                }
            }
        }
    }
    out
}

fn same_vec(a: &[f64; 3], b: &[f64; 3]) -> bool { dot(a, b) > 1.0 - 1e-12 }

fn shared_corner_count(a: &Cell, b: &Cell) -> usize {
    let ca = a.corners();
    let cb = b.corners();
    let mut n = 0;
    for x in &ca {
        for y in &cb {
            if same_vec(&x.vec, &y.vec) {
                n += 1;
                break;
            }
        }
    }
    n
}

// ---------------------------------------------------------------------------------------------
// Invariant 1: centre round-trip is exact at every level.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv01_centre_round_trip() {
    let mut rng = Rng::new(0x1234_5678);
    for level in 0u8..=26 {
        for _ in 0..200 {
            let cell = rng.cell_at(level);
            let (lat, lon) = cell.centre();
            let got = match Cell::at(lat, lon, level) {
                Ok(c) => c,
                Err(e) => panic!("at() failed on a cell centre: {}", e),
            };
            assert_eq!(got, cell, "centre round-trip broke at level {}", level);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 2: parent is exact -- parent(at(p, b), a) == at(p, a) for all a < b.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv02_parent_exact() {
    let mut rng = Rng::new(0xDEAD_BEEF);
    for _ in 0..4000 {
        let lat = rng.lat();
        let lon = rng.lon();
        let b = (rng.next_u64() % 27) as u8;
        if b == 0 { continue; }
        let a = (rng.next_u64() % b as u64) as u8; // a < b
        let deep = match Cell::at(lat, lon, b) { Ok(c) => c, Err(_) => continue };
        let via_parent = match deep.parent(a) { Ok(c) => c, Err(e) => panic!("parent: {}", e) };
        let direct = match Cell::at(lat, lon, a) { Ok(c) => c, Err(e) => panic!("at: {}", e) };
        assert_eq!(via_parent, direct, "parent inexact: a={} b={}", a, b);
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 3: children -- exactly 4, each parents back, (i,j) partition the parent's cell.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv03_children_partition() {
    let mut rng = Rng::new(0x0BADF00D);
    for _ in 0..2000 {
        let level = (rng.next_u64() % 26) as u8; // 0..=25 so children exist
        let parent = rng.cell_at(level);
        let kids = match parent.children() { Ok(k) => k, Err(e) => panic!("children: {}", e) };
        let (pi, pj) = parent.coords();
        let mut seen = HashSet::new();
        for kid in &kids {
            assert_eq!(kid.level(), level + 1);
            assert_eq!(kid.face(), parent.face());
            let back = match kid.parent(level) { Ok(c) => c, Err(e) => panic!("{}", e) };
            assert_eq!(back, parent, "child did not parent back");
            let (ci, cj) = kid.coords();
            assert!(ci / 2 == pi && cj / 2 == pj, "child outside parent's index block");
            assert!(seen.insert((ci, cj)), "duplicate child");
        }
        assert_eq!(seen.len(), 4);
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 4: neighbour symmetry and the cube-vertex census (exactly 24 cells with 7).
// ---------------------------------------------------------------------------------------------

#[test]
fn inv04_neighbour_symmetry_and_census() {
    // Level 0: each face touches its four side faces only.
    for cell in all_cells(0) {
        let nb = match cell.neighbours() { Ok(n) => n, Err(e) => panic!("{}", e) };
        assert_eq!(nb.len(), 4, "level-0 face should have four neighbours");
    }

    for level in 1u8..=6 {
        let cells = all_cells(level);
        let mut nmap: HashMap<u64, HashSet<u64>> = HashMap::new();
        for cell in &cells {
            let nb = match cell.neighbours() { Ok(n) => n, Err(e) => panic!("{}", e) };
            let set: HashSet<u64> = nb.iter().map(|c| c.bits()).collect();
            assert!(!set.contains(&cell.bits()), "cell listed itself as neighbour");
            nmap.insert(cell.bits(), set);
        }
        // Symmetry: b in N(a) iff a in N(b).
        for (a, set) in &nmap {
            for b in set {
                let back = match nmap.get(b) { Some(s) => s, None => panic!("neighbour off census") };
                assert!(back.contains(a), "neighbour relation not symmetric at level {}", level);
            }
        }
        // Census: exactly 24 cells have 7 neighbours, the rest have 8.
        let mut sevens = 0usize;
        for set in nmap.values() {
            match set.len() {
                7 => sevens += 1,
                8 => {},
                other => panic!("cell has {} neighbours at level {}", other, level),
            }
        }
        assert_eq!(sevens, 24, "vertex census wrong at level {}", level);
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 5: geometric adjacency -- neighbours equal the cells hit just outside the boundary.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv05_geometric_adjacency() {
    let mut rng = Rng::new(0xFEED_FACE);
    for _ in 0..600 {
        let level = 2 + (rng.next_u64() % 5) as u8; // 2..=6
        let cell = rng.cell_at(level);
        let neigh: HashSet<u64> = match cell.neighbours() {
            Ok(n) => n.iter().map(|c| c.bits()).collect(),
            Err(e) => panic!("{}", e),
        };
        // Sample points just outside each edge midpoint and each corner of the boundary.
        let corners = cell.corners();
        let centre = norm(&cell.centre_vec());
        let mut geo: HashSet<u64> = HashSet::new();
        let boundary_samples = {
            let mut pts: Vec<[f64; 3]> = Vec::new();
            // Edge midpoints.
            for e in 0..4 {
                let a = &corners[e].vec;
                let b = &corners[(e + 1) % 4].vec;
                let mid = norm(&[a[0]+b[0], a[1]+b[1], a[2]+b[2]]);
                pts.push(nudge_out(&mid, &centre));
            }
            // Corners.
            for e in 0..4 {
                pts.push(nudge_out(&corners[e].vec, &centre));
            }
            pts
        };
        for p in &boundary_samples {
            let (lat, lon) = vec_latlon(p);
            let hit = match Cell::at(lat, lon, level) { Ok(c) => c, Err(e) => panic!("{}", e) };
            if hit.bits() != cell.bits() {
                geo.insert(hit.bits());
            }
        }
        assert_eq!(geo, neigh, "geometric adjacency disagreed with neighbours()");
    }
}

fn nudge_out(p: &[f64; 3], centre: &[f64; 3]) -> [f64; 3] {
    // Move from the boundary point a hair further from the centre.
    let d = 1e-5;
    let out = [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]];
    let outn = norm(&out);
    norm(&[p[0] + d*outn[0], p[1] + d*outn[1], p[2] + d*outn[2]])
}

fn vec_latlon(v: &[f64; 3]) -> (f64, f64) {
    let u = norm(v);
    (u[2].clamp(-1.0, 1.0).asin().to_degrees(), u[1].atan2(u[0]).to_degrees())
}

// ---------------------------------------------------------------------------------------------
// Invariant 6: edge-neighbours share exactly 2 corners, corner-neighbours exactly 1.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv06_shared_corners() {
    let mut rng = Rng::new(0xC0FF_EE00);
    let mut tested = 0;
    while tested < 200 {
        let level = 3 + (rng.next_u64() % 4) as u8; // 3..=6
        let n = 1u32 << level;
        // Choose an interior cell (not touching a cube vertex) so it has 8 neighbours.
        let i = 1 + (rng.next_u64() % (n as u64 - 2)) as u32;
        let j = 1 + (rng.next_u64() % (n as u64 - 2)) as u32;
        let face = (rng.next_u64() % 6) as u8;
        let cell = match Cell::from_face_ij(face, level, i, j) { Ok(c) => c, Err(e) => panic!("{}", e) };
        let nb = match cell.neighbours() { Ok(n) => n, Err(e) => panic!("{}", e) };
        assert_eq!(nb.len(), 8, "interior cell should have 8 neighbours");
        let mut twos = 0;
        let mut ones = 0;
        for other in &nb {
            match shared_corner_count(&cell, other) {
                2 => twos += 1,
                1 => ones += 1,
                k => panic!("neighbour shares {} corners", k),
            }
        }
        assert_eq!(twos, 4, "expected 4 edge-neighbours");
        assert_eq!(ones, 4, "expected 4 corner-neighbours");
        tested += 1;
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 7: tiling and evenness -- areas sum to 4*pi and lie in [avg/sqrt2, avg*sqrt2].
// ---------------------------------------------------------------------------------------------

#[test]
fn inv07_tiling_and_evenness() {
    let root2 = 2.0f64.sqrt();
    for level in 1u8..=5 {
        let cells = all_cells(level);
        let count = cells.len() as f64;
        let mut sum = 0.0;
        let mut min = f64::INFINITY;
        let mut max = 0.0f64;
        for cell in &cells {
            let a = cell.area();
            // The production area must match the independent oracle.
            let ao = cell_area_oracle(cell);
            assert!((a - ao).abs() < 1e-12, "area disagrees with oracle");
            sum += a;
            if a < min { min = a; }
            if a > max { max = a; }
        }
        assert!((sum - 4.0*std::f64::consts::PI).abs() < 1e-9,
            "areas did not tile the sphere at level {}: sum={}", level, sum);
        let avg = 4.0*std::f64::consts::PI / count;
        assert!(min >= avg/root2 - 1e-12 && max <= avg*root2 + 1e-12,
            "evenness band violated at level {}: min/avg={} max/avg={}", level, min/avg, max/avg);
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 8: contains agrees with at, including at poles, antimeridian, edges and vertices.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv08_contains_boundaries() {
    // Random interior points: contains, at and the independent naive oracle must all agree.
    let mut rng = Rng::new(0xABCD_1234);
    for _ in 0..3000 {
        let level = (rng.next_u64() % 12) as u8;
        let lat = rng.lat();
        let lon = rng.lon();
        let cell = match Cell::at(lat, lon, level) { Ok(c) => c, Err(e) => panic!("{}", e) };
        assert!(match cell.contains(lat, lon) { Ok(b) => b, Err(e) => panic!("{}", e) },
            "cell does not contain the point it was built from");
        let p = latlon_to_vec(lat, lon);
        // The naive geometric oracle must agree for a clearly-interior point.
        assert!(naive_contains(&cell, &p), "naive oracle disagreed with at()");
    }

    // Special boundary points at several levels: resolution is total and deterministic.
    let mut specials: Vec<(f64, f64)> = vec![
        (90.0, 0.0), (-90.0, 0.0),        // poles
        (0.0, 180.0), (0.0, -180.0),      // antimeridian
        (0.0, 45.0), (0.0, 135.0),        // face edges (|x|=|y|)
        (45.0, 0.0),                      // face edge (|x|=|z|)
    ];
    let vlat = (1.0f64/3.0).sqrt().asin().to_degrees(); // 35.264 deg
    for k in 0..4 {
        let lon = 45.0 + 90.0 * k as f64;
        specials.push((vlat, lon));
        specials.push((-vlat, lon));
    }
    for (lat, lon) in specials {
        for level in 0u8..=8 {
            let c1 = match Cell::at(lat, lon, level) { Ok(c) => c, Err(e) => panic!("{}", e) };
            let c2 = match Cell::at(lat, lon, level) { Ok(c) => c, Err(e) => panic!("{}", e) };
            assert_eq!(c1, c2, "at() not deterministic at boundary ({}, {})", lat, lon);
            assert!(c1.face() < 6 && c1.level() == level);
            assert!(match c1.contains(lat, lon) { Ok(b) => b, Err(e) => panic!("{}", e) },
                "boundary point not contained by its own cell ({}, {})", lat, lon);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 9: fixtures -- face centres, string round-trip, malformed ids refused.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv09_fixtures() {
    // Six face centres at level 0.
    let expect: [(f64, f64); 6] = [
        (0.0, 0.0),      // +x
        (0.0, 90.0),     // +y
        (90.0, 0.0),     // +z  (north pole)
        (0.0, 180.0),    // -x
        (0.0, -90.0),    // -y
        (-90.0, 0.0),    // -z  (south pole)
    ];
    for (face, (elat, elon)) in expect.iter().enumerate() {
        let c = match Cell::from_face_ij(face as u8, 0, 0, 0) { Ok(c) => c, Err(e) => panic!("{}", e) };
        let (lat, lon) = c.centre();
        assert!((lat - elat).abs() < 1e-9, "face {} centre lat {} != {}", face, lat, elat);
        // Longitude is undefined at the poles, so only check away from them; +/-180 name
        // the same meridian, so compare modulo a full turn.
        if elat.abs() < 89.0 {
            let d = ((lon - elon).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
            assert!(d.abs() < 1e-9, "face {} centre lon {} != {}", face, lon, elon);
        }
    }

    // String round-trip over random cells at all levels.
    let mut rng = Rng::new(0x5150_5150);
    for level in 0u8..=26 {
        for _ in 0..50 {
            let c = rng.cell_at(level);
            let s = c.to_string();
            assert_eq!(s.len(), 16, "id string not 16 hex chars");
            let back: Cell = match s.parse() { Ok(c) => c, Err(e) => panic!("parse: {}", e) };
            assert_eq!(back, c, "string round-trip failed");
        }
    }

    // Malformed ids are refused.
    let good = match Cell::from_face_ij(2, 5, 7, 11) { Ok(c) => c, Err(e) => panic!("{}", e) };
    let bits = good.bits();
    // Unknown scheme (nibble != 1).
    assert!(Cell::from_bits((bits & !(0xFu64 << 60)) | (0x2u64 << 60)).is_err(), "bad scheme accepted");
    assert!(Cell::from_bits(bits & !(0xFu64 << 60)).is_err(), "zero scheme accepted");
    // Face >= 6.
    assert!(Cell::from_bits((bits & !(0x7u64 << 57)) | (6u64 << 57)).is_err(), "face 6 accepted");
    assert!(Cell::from_bits((bits & !(0x7u64 << 57)) | (7u64 << 57)).is_err(), "face 7 accepted");
    // Level > 26.
    assert!(Cell::from_bits((bits & !(0x1Fu64 << 52)) | (27u64 << 52)).is_err(), "level 27 accepted");
    // Non-zero unused low bit (level 5 uses top 10 of the 52-bit field; bit 0 must be zero).
    assert!(Cell::from_bits(bits | 1u64).is_err(), "dirty low bit accepted");

    // Bad string forms.
    assert!("".parse::<Cell>().is_err());
    assert!("xyz".parse::<Cell>().is_err());
    assert!("deadbeef".parse::<Cell>().is_err(), "short string accepted");
    assert!("00000000000000001".parse::<Cell>().is_err(), "long string accepted");
    // A well-formed level-0 +x cell string.
    let z = match Cell::from_face_ij(0, 0, 0, 0) { Ok(c) => c, Err(e) => panic!("{}", e) };
    assert_eq!(z.to_string(), "1000000000000000");
}

// ---------------------------------------------------------------------------------------------
// Invariant 10: k-ring BFS -- ring(0) is the cell, rings are disjoint, ring(1) == neighbours.
// ---------------------------------------------------------------------------------------------

#[test]
fn inv10_ring() {
    let mut rng = Rng::new(0x9999_1111);
    for _ in 0..80 {
        let level = 3 + (rng.next_u64() % 3) as u8; // 3..=5
        let cell = rng.cell_at(level);
        let r0 = match cell.ring(0) { Ok(r) => r, Err(e) => panic!("{}", e) };
        assert_eq!(r0.len(), 1);
        assert_eq!(r0[0], cell);

        let r1: HashSet<u64> = match cell.ring(1) {
            Ok(r) => r.iter().map(|c| c.bits()).collect(),
            Err(e) => panic!("{}", e),
        };
        let nb: HashSet<u64> = match cell.neighbours() {
            Ok(n) => n.iter().map(|c| c.bits()).collect(),
            Err(e) => panic!("{}", e),
        };
        assert_eq!(r1, nb, "ring(1) must equal the neighbour set");

        // Rings 0..=3 are pairwise disjoint.
        let mut seen: HashSet<u64> = HashSet::new();
        for k in 0..=3u32 {
            let rk = match cell.ring(k) { Ok(r) => r, Err(e) => panic!("{}", e) };
            for c in rk {
                assert!(seen.insert(c.bits()), "rings overlapped at k={}", k);
            }
        }
    }
}
