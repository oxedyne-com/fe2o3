//! The viewport, clipped ring projection, cell outlines and cap covering.
//!
//! The screen transforms are held to PROJ 9.7.1's numbers, the covering to a brute-force
//! enumeration, and the ring clip to places that are sea or land as a matter of record: the
//! "sea stays sea at every turn" check Ochre's globe was held to, moved here with the clip and
//! extended to the flat map and to a globe turned and zoomed in.

use oxedyne_fe2o3_geom::{
    cell::{
        self,
        Cell,
    },
    planar::Pt,
    proj::{
        EARTH_RADIUS_M,
        Projection,
        RingMode,
        ScreenPaths,
        Viewport,
        unit_vec,
        vec_lat_lng,
    },
    world,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashSet,
    f64::consts::{
        PI,
        TAU,
    },
};

const WORLD: &[u8] = include_bytes!("data/world_coarse.bin");

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 { (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64) }
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

fn angle(a: &[f64; 3], b: &[f64; 3]) -> f64 { dot(a, b).clamp(-1.0, 1.0).acos() }

/// How many times the closed paths wind round a point, which is what a nonzero fill asks.
fn winding(paths: &ScreenPaths, x: f64, y: f64) -> f64 {
    let mut sum = 0.0;
    for i in 0..paths.len() {
        let (pts, closed) = match paths.path(i) {
            Some(p) => p,
            None => continue,
        };
        if !closed {
            continue;
        }
        let n = pts.len() / 2;
        for k in 0..n {
            let (ax, ay) = (pts[2 * k] as f64 - x, pts[2 * k + 1] as f64 - y);
            let m = (k + 1) % n;
            let (bx, by) = (pts[2 * m] as f64 - x, pts[2 * m + 1] as f64 - y);
            sum += (ax * by - ay * bx).atan2(ax * bx + ay * by);
        }
    }
    sum / TAU
}

fn view(kind: Projection, lat_0: f64, lon_0: f64, heading: f64, m_per_px: f64, w: f64, h: f64)
    -> Outcome<Viewport>
{
    Viewport::new(kind, lat_0, lon_0, heading, m_per_px, w, h)
}

fn ring(pts: &[(f64, f64)]) -> Vec<[f64; 3]> {
    pts.iter().map(|(lat, lng)| unit_vec(*lat, *lng)).collect()
}

// ---------------------------------------------------------------------------------------------
// The camera
// ---------------------------------------------------------------------------------------------

#[test]
fn test_the_viewport_puts_cities_where_proj_does_00() -> Outcome<()> {
    // Orthographic about Perth at 300 px to the Earth's radius; the unit-sphere numbers are
    // PROJ's (`+proj=ortho +R=1 +lat_0=-31.9535 +lon_0=115.8571`), with y flipped for a screen.
    let v = res!(view(Projection::Orthographic, -31.9535, 115.8571, 0.0,
        EARTH_RADIUS_M / 300.0, 800.0, 700.0));
    for (lat, lng, x, y) in [
        (-33.8688, 151.2093,  0.480421544470, -0.114447989448),
        ( 35.6895, 139.6917,  0.328204336296,  0.888173527162),
        (  3.1390, 101.6869, -0.244435840733,  0.558819302390),
        (-90.0,      0.0,     0.0,            -0.848477887693),
    ] {
        let p = res!(v.forward(lat, lng).ok_or_else(|| err!("{}, {} was hidden.", lat, lng; Test)));
        let near = (p.x - (400.0 + 300.0 * x)).abs() < 1.0e-6
            && (p.y - (350.0 - 300.0 * y)).abs() < 1.0e-6;
        req!(near, true, "{}, {} landed at {}, {}.", lat, lng, p.x, p.y);
    }
    // London is behind Perth.
    req!(v.forward(51.5074, -0.1278).is_none(), true, "London showed through the globe.");

    // Web Mercator about Perth at 100 px to the Earth's radius at the centre's parallel, so the
    // equator's radius is 100 / cos(lat_0).  PROJ: `+proj=webmerc +R=1 +lon_0=115.8571`.
    let phi = (-31.9535f64).to_radians();
    let v = res!(view(Projection::WebMercator, -31.9535, 115.8571, 0.0,
        EARTH_RADIUS_M * phi.cos() / 100.0, 800.0, 700.0));
    let perth_y = -0.589076140273;
    for (lat, lng, x, y) in [
        ( 35.6895,  139.6917,  0.415992245896,  0.667590039565),
        ( 51.5074,   -0.1278, -2.024318387596,  1.052273175629),
        (-33.9189,   18.4233, -1.700540612730, -0.629951578195),
        (-31.9535,  115.8571,  0.0,             perth_y),
    ] {
        let p = res!(v.forward(lat, lng).ok_or_else(|| err!("{}, {} was lost.", lat, lng; Test)));
        let near = (p.x - (400.0 + 100.0 * x)).abs() < 1.0e-6
            && (p.y - (350.0 - 100.0 * (y - perth_y))).abs() < 1.0e-6;
        req!(near, true, "{}, {} landed at {}, {}.", lat, lng, p.x, p.y);
    }
    Ok(())
}

#[test]
fn test_a_heading_turns_the_picture_01() -> Outcome<()> {
    for kind in [Projection::Orthographic, Projection::WebMercator] {
        // East up: a place due east is straight above the centre, and north is to the left.
        let v = res!(view(kind, 0.0, 0.0, 90.0, 1000.0, 400.0, 400.0));
        let e = res!(v.forward(0.0, 0.5).ok_or_else(|| err!("East was hidden."; Test)));
        let up = e.y < 200.0 - 10.0 && (e.x - 200.0).abs() < 1.0e-6;
        req!(up, true, "{:?}: east landed at {}, {}.", kind, e.x, e.y);
        let n = res!(v.forward(0.5, 0.0).ok_or_else(|| err!("North was hidden."; Test)));
        let left = n.x < 200.0 - 10.0 && (n.y - 200.0).abs() < 1.0e-3;
        req!(left, true, "{:?}: north landed at {}, {}.", kind, n.x, n.y);
    }
    Ok(())
}

#[test]
fn test_forward_and_inverse_undo_each_other_02() -> Outcome<()> {
    let mut rng = Rng(7);
    for kind in [Projection::Orthographic, Projection::WebMercator] {
        for (lat_0, lon_0, heading, m) in [
            (-31.9535, 115.8571, 37.0, 50.0),
            (64.0, -150.0, -120.0, 2_000.0),
            (0.0, 179.9, 0.0, 0.3),
            (-75.0, 10.0, 200.0, 20_000.0),
        ] {
            let v = res!(view(kind, lat_0, lon_0, heading, m, 390.0, 700.0));
            let wraps = kind == Projection::WebMercator && v.bounding_cap().1 >= PI;
            for _ in 0..200 {
                let pt = Pt::new(rng.unit() * 390.0, rng.unit() * 700.0);
                let (lat, lng) = match v.inverse(pt) {
                    Some(p) => p,
                    None => continue,
                };
                let back = res!(v.forward(lat, lng).ok_or_else(|| err!(
                    "{:?}: {}, {} came from the screen and then hid.", kind, lat, lng; Test)));
                let same = (back.x - pt.x).abs() < 1.0e-5 && (back.y - pt.y).abs() < 1.0e-5;
                if wraps && !same {
                    // A flat map wider than the world shows a place more than once, and
                    // forward answers with the copy nearest the centre.
                    let (a, b) = res!(v.inverse(back).ok_or_else(|| err!("Lost."; Test)));
                    let twin = (a - lat).abs() < 1.0e-9 && ((b - lng + 540.0) % 360.0 - 180.0).abs() < 1.0e-9;
                    req!(twin, true, "{:?} about {}, {}: ({}, {}) came back at ({}, {}), \
                        which is {}, {}.", kind, lat_0, lon_0, pt.x, pt.y, back.x, back.y, a, b);
                    continue;
                }
                req!(same, true, "{:?} about {}, {}: ({}, {}) came back at ({}, {}).",
                    kind, lat_0, lon_0, pt.x, pt.y, back.x, back.y);
            }
        }
    }
    Ok(())
}

#[test]
fn test_the_bounding_cap_holds_the_screen_03() -> Outcome<()> {
    for kind in [Projection::Orthographic, Projection::WebMercator] {
        for (lat_0, lon_0, heading, m) in [
            (-33.87, 151.21, 0.0, 1.0),
            (60.0, 10.0, 30.0, 300.0),
            (0.0, -179.0, 0.0, 30_000.0),
            (-80.0, 0.0, 0.0, 5_000.0),
            (20.0, 40.0, 0.0, 200_000.0),
        ] {
            let v = res!(view(kind, lat_0, lon_0, heading, m, 390.0, 700.0));
            let (c, r) = v.bounding_cap();
            for i in 0..=20 {
                for j in 0..=20 {
                    let pt = Pt::new(390.0 * i as f64 / 20.0, 700.0 * j as f64 / 20.0);
                    if let Some((lat, lng)) = v.inverse(pt) {
                        let a = angle(&c, &unit_vec(lat, lng));
                        req!((a <= r + 1.0e-12), true, "{:?} about {}, {} at {} m/px: a screen \
                            point is {} rad out, the cap {}.", kind, lat_0, lon_0, m, a, r);
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Rings
// ---------------------------------------------------------------------------------------------

#[test]
fn test_the_flat_map_joins_a_ring_across_the_antimeridian_04() -> Outcome<()> {
    let fiji = ring(&[(-17.0, 179.5), (-17.0, -179.5), (-16.0, -179.5), (-16.0, 179.5)]);
    let v = res!(view(Projection::WebMercator, -16.5, 180.0, 0.0, 400.0, 800.0, 800.0));
    let mut out = ScreenPaths::new();
    res!(v.project_rings(&[fiji], RingMode::Fill, 0.0, 0.25, &mut out));
    req!(out.len(), 1, "The island came out in {} pieces.", out.len());
    let (pts, closed) = res!(out.path(0).ok_or_else(|| err!("No path."; Test)));
    req!(closed, true);
    let xs: Vec<f64> = pts.chunks(2).map(|p| p[0] as f64).collect();
    let span = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    let narrow = span < 400.0;
    req!(narrow, true, "An island two degrees wide spans {} px.", span);
    let inside = winding(&out, 400.0, 400.0).abs() > 0.5;
    req!(inside, true, "The island's middle is not filled.");
    Ok(())
}

#[test]
fn test_the_flat_map_closes_a_ring_round_the_pole_05() -> Outcome<()> {
    let east: Vec<(f64, f64)> = (0..36).map(|k| (-70.0, -180.0 + 10.0 * k as f64)).collect();
    let west: Vec<(f64, f64)> = east.iter().rev().cloned().collect();
    // The whole map, square, 800 px on a side.
    let v = res!(view(Projection::WebMercator, 0.0, 0.0, 0.0, TAU * EARTH_RADIUS_M / 800.0,
        800.0, 800.0));
    for pts in [east, west] {
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&[ring(&pts)], RingMode::Fill, 0.0, 0.25, &mut out));
        for (lat, lng, filled) in [(-80.0, 45.0, true), (-75.0, -170.0, true), (-60.0, 0.0, false),
            (0.0, 90.0, false), (70.0, 0.0, false)]
        {
            let p = res!(v.forward(lat, lng).ok_or_else(|| err!("Lost."; Test)));
            let got = winding(&out, p.x, p.y).abs() > 0.5;
            req!(got, filled, "{}, {} came out {}.", lat, lng, if got { "filled" } else { "empty" });
        }
    }
    // A cell with the north pole in it, and one with the pole as a corner.
    let face = res!(Cell::from_face_ij(2, 0, 0, 0)).outline(8);
    let corner = res!(Cell::from_face_ij(2, 1, 0, 0)).outline(8);
    let mut out = ScreenPaths::new();
    res!(v.project_rings(&[face], RingMode::Fill, 0.0, 0.25, &mut out));
    for (lat, lng, filled) in [(80.0, 45.0, true), (40.0, 45.0, true), (40.0, 0.0, false),
        (0.0, 0.0, false)]
    {
        let p = res!(v.forward(lat, lng).ok_or_else(|| err!("Lost."; Test)));
        let got = winding(&out, p.x, p.y).abs() > 0.5;
        req!(got, filled, "Face 2: {}, {} came out {}.", lat, lng, if got { "filled" } else { "empty" });
    }
    let mut out = ScreenPaths::new();
    res!(v.project_rings(&[corner], RingMode::Fill, 0.0, 0.25, &mut out));
    for (lat, lng, filled) in [(80.0, 45.0, true), (80.0, 135.0, false), (80.0, -45.0, false)] {
        let p = res!(v.forward(lat, lng).ok_or_else(|| err!("Lost."; Test)));
        let got = winding(&out, p.x, p.y).abs() > 0.5;
        req!(got, filled, "Pole corner: {}, {} came out {}.", lat, lng,
            if got { "filled" } else { "empty" });
    }
    Ok(())
}

#[test]
fn test_the_flat_map_repeats_the_world_06() -> Outcome<()> {
    // 1200 px of a world 800 px round: a ring at 170 degrees east shows twice.
    let v = res!(view(Projection::WebMercator, 0.0, 0.0, 0.0, TAU * EARTH_RADIUS_M / 800.0,
        1200.0, 400.0));
    let isle = ring(&[(-1.0, 169.0), (-1.0, 171.0), (1.0, 171.0), (1.0, 169.0)]);
    let mut out = ScreenPaths::new();
    res!(v.project_rings(&[isle], RingMode::Fill, 0.0, 0.25, &mut out));
    req!(out.len(), 2, "The island showed {} times.", out.len());
    let (_, r) = v.bounding_cap();
    req!(r, PI, "A screen wider than the world has a cap of {} rad.", r);
    Ok(())
}

#[test]
fn test_the_globe_fills_the_view_from_inside_a_ring_07() -> Outcome<()> {
    let mut big = Vec::new();
    for k in 0..40 {
        big.push((-40.0, -40.0 + 2.0 * k as f64));
    }
    for k in 0..40 {
        big.push((-40.0 + 2.0 * k as f64, 40.0));
    }
    for k in 0..40 {
        big.push((40.0, 40.0 - 2.0 * k as f64));
    }
    for k in 0..40 {
        big.push((40.0 - 2.0 * k as f64, -40.0));
    }
    let rev: Vec<(f64, f64)> = big.iter().rev().cloned().collect();
    for pts in [big, rev] {
        let r = ring(&pts);
        // From right in the middle, close in: no vertex shows, and the screen is filled.
        let v = res!(view(Projection::Orthographic, 0.0, 0.0, 0.0, 100.0, 390.0, 700.0));
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&[r.clone()], RingMode::Fill, 0.0, 0.25, &mut out));
        let filled = winding(&out, 195.0, 350.0).abs() > 0.5 && winding(&out, 1.0, 1.0).abs() > 0.5;
        req!(filled, true, "The view from inside the ring is not filled.");
        // From outside it, close in, nothing is drawn.
        let v = res!(view(Projection::Orthographic, 0.0, 90.0, 0.0, 100.0, 390.0, 700.0));
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&[r.clone()], RingMode::Fill, 0.0, 0.25, &mut out));
        req!(out.len(), 0, "The view from outside drew {} paths.", out.len());
        // From the antipode, whole globe: the ring is behind it, and nothing is drawn.
        let v = res!(view(Projection::Orthographic, 0.0, 180.0, 0.0, EARTH_RADIUS_M / 300.0,
            700.0, 700.0));
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&[r], RingMode::Fill, 0.0, 0.25, &mut out));
        req!(out.len(), 0, "The far side drew {} paths.", out.len());
    }
    Ok(())
}

#[test]
fn test_clipped_paths_stay_by_the_screen_08() -> Outcome<()> {
    let w = res!(world::read(WORLD));
    let land = res!(w.layer("land", 0).ok_or_else(|| err!("No land."; Test))).unit_rings();
    let borders = res!(w.layer("borders", 0).ok_or_else(|| err!("No borders."; Test))).unit_rings();
    let mut rng = Rng(11);
    for _ in 0..40 {
        let lat = rng.unit() * 170.0 - 85.0;
        let lng = rng.unit() * 360.0 - 180.0;
        let m = 10f64.powf(1.0 + rng.unit() * 4.5);
        let hd = rng.unit() * 360.0;
        for kind in [Projection::Orthographic, Projection::WebMercator] {
            let v = res!(view(kind, lat, lng, hd, m, 390.0, 700.0));
            let mut out = ScreenPaths::new();
            res!(v.project_rings(&land, RingMode::Fill, 30_000.0, 0.25, &mut out));
            res!(v.project_rings(&borders, RingMode::Line, 30_000.0, 0.25, &mut out));
            // The clip lies eight pixels outside the screen, or round it on the globe.
            let reach = 195.0f64.hypot(350.0) + 8.0 + 0.01;
            for p in out.xy.chunks(2) {
                let (x, y) = (p[0] as f64, p[1] as f64);
                let ok = match kind {
                    Projection::Orthographic => (x - 195.0).hypot(y - 350.0) <= reach
                        .max(EARTH_RADIUS_M / m + 0.01),
                    Projection::WebMercator => x >= -8.01 && x <= 398.01 && y >= -8.01 && y <= 708.01,
                };
                req!(ok, true, "{:?} about {}, {} at {} m/px drew a point at {}, {}.",
                    kind, lat, lng, m, x, y);
            }
        }
    }
    Ok(())
}

#[test]
fn test_a_stroked_ring_is_cut_at_the_horizon_09() -> Outcome<()> {
    // A parallel at 10 degrees north, seen from the equator: the half facing us is drawn and
    // its two ends sit on the rim.
    let pts: Vec<(f64, f64)> = (0..72).map(|k| (10.0, -180.0 + 5.0 * k as f64)).collect();
    let v = res!(view(Projection::Orthographic, 0.0, 0.0, 0.0, EARTH_RADIUS_M / 300.0,
        700.0, 700.0));
    let mut out = ScreenPaths::new();
    res!(v.project_rings(&[ring(&pts)], RingMode::Outline, 0.0, 0.0, &mut out));
    req!(out.len(), 1, "The parallel came out in {} pieces.", out.len());
    let (xy, closed) = res!(out.path(0).ok_or_else(|| err!("No path."; Test)));
    req!(closed, false);
    let n = xy.len() / 2;
    for k in [0, n - 1] {
        let r = (xy[2 * k] as f64 - 350.0).hypot(xy[2 * k + 1] as f64 - 350.0);
        let on = (r - 300.0).abs() < 0.01;
        req!(on, true, "An end of the cut parallel is {} px from the centre.", r);
    }
    Ok(())
}

#[test]
fn test_the_sea_stays_sea_at_every_turn_10() -> Outcome<()> {
    // Open water and dry ground, both a matter of record and neither from this repository;
    // the ground is far enough from any coast that a thirty-kilometre outline cannot reach it.
    // The lists are Ochre's (`globe.rs`, `dev/verify_world.py`).
    let sea = [
        ("the middle of the Indian Ocean", -30.0,    80.0),
        ("the middle of the Pacific",        0.0,  -150.0),
        ("the middle of the Atlantic",     -20.0,   -25.0),
        ("Point Nemo",                     -48.8767, -123.3933),
        ("the north Pacific",               40.0,  -170.0),
        ("the Sargasso Sea",                35.0,   -45.0),
        ("the south Atlantic",             -40.0,   -20.0),
        ("the south Indian Ocean",         -45.0,    90.0),
        ("the Southern Ocean",             -60.0,     0.0),
        ("the south Pacific",              -30.0,  -140.0),
        ("the Arabian Sea",                 15.0,    65.0),
        ("the Arctic Ocean",                85.0,     0.0),
    ];
    let land = [
        ("Moscow",         55.7558,   37.6173),
        ("Ulaanbaatar",    47.8864,  106.9057),
        ("Cairo",          30.0444,   31.2357),
        ("Nairobi",        -1.2921,   36.8219),
        ("Fairbanks",      64.8378, -147.7164),
        ("Denver",         39.7392, -104.9903),
        ("Brasilia",      -15.7939,  -47.8828),
        ("Novosibirsk",    55.0084,   82.9357),
        ("Alice Springs", -23.6980,  133.8807),
    ];
    let places: Vec<(&str, f64, f64, bool)> = sea.iter().map(|(n, a, b)| (*n, *a, *b, false))
        .chain(land.iter().map(|(n, a, b)| (*n, *a, *b, true)))
        .collect();
    let w = res!(world::read(WORLD));
    let layer = res!(w.layer("land", 0).ok_or_else(|| err!("No land."; Test)));
    let rings = layer.unit_rings();
    let mut asked = 0usize;

    // The whole globe at 36 turns, and again with the picture turned.
    for heading in [0.0, 30.0] {
        for lat_0 in [-75.0, -45.0, -15.0, 15.0, 45.0, 75.0] {
            for lon_0 in [-165.0, -105.0, -45.0, 15.0, 75.0, 135.0] {
                let v = res!(view(Projection::Orthographic, lat_0, lon_0, heading,
                    EARTH_RADIUS_M / 300.0, 700.0, 700.0));
                let mut out = ScreenPaths::new();
                res!(v.project_rings(&rings, RingMode::Fill, layer.tol_m, 0.25, &mut out));
                for (name, lat, lng, dry) in &places {
                    let p = match v.forward(*lat, *lng) {
                        Some(p) => p,
                        None => continue,
                    };
                    // A band at the rim where a coastline is edge-on means nothing either way.
                    if (p.x - 350.0).hypot(p.y - 350.0) > 0.92 * 300.0 {
                        continue;
                    }
                    asked += 1;
                    let filled = winding(&out, p.x, p.y).abs() > 0.5;
                    req!(filled, *dry, "Globe turned to {}, {} heading {}: {} came out {}.",
                        lat_0, lon_0, heading, name, if filled { "land" } else { "sea" });
                }
            }
        }
    }
    // Close in over each place, where the clip is the circle round the screen and a place
    // inland has no coastline in view at all.
    for (name, lat, lng, dry) in &places {
        for m in [200.0, 5_000.0] {
            let v = res!(view(Projection::Orthographic, *lat, *lng, 15.0, m, 390.0, 700.0));
            let mut out = ScreenPaths::new();
            res!(v.project_rings(&rings, RingMode::Fill, layer.tol_m, 0.25, &mut out));
            asked += 1;
            let filled = winding(&out, 195.0, 350.0).abs() > 0.5;
            req!(filled, *dry, "Close over {} at {} m/px it came out {}.", name, m,
                if filled { "land" } else { "sea" });
        }
    }
    // The flat map, whole and close in.
    for lon_0 in [-165.0, -105.0, -45.0, 15.0, 75.0, 135.0] {
        let v = res!(view(Projection::WebMercator, 0.0, lon_0, 0.0,
            TAU * EARTH_RADIUS_M / 1000.0, 1000.0, 1000.0));
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&rings, RingMode::Fill, layer.tol_m, 0.25, &mut out));
        for (name, lat, lng, dry) in &places {
            if lat.abs() > 80.0 {
                continue;
            }
            let p = res!(v.forward(*lat, *lng).ok_or_else(|| err!("{} was lost.", name; Test)));
            asked += 1;
            let filled = winding(&out, p.x, p.y).abs() > 0.5;
            req!(filled, *dry, "Map about {}: {} came out {}.", lon_0, name,
                if filled { "land" } else { "sea" });
        }
    }
    for (name, lat, lng, dry) in &places {
        let v = res!(view(Projection::WebMercator, *lat, *lng, 0.0, 2_000.0, 390.0, 700.0));
        let mut out = ScreenPaths::new();
        res!(v.project_rings(&rings, RingMode::Fill, layer.tol_m, 0.25, &mut out));
        asked += 1;
        let filled = winding(&out, 195.0, 350.0).abs() > 0.5;
        req!(filled, *dry, "Map close over {} came out {}.", name, if filled { "land" } else { "sea" });
    }
    let plenty = asked > 400;
    req!(plenty, true, "Only {} places were asked about.", asked);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------------------------

#[test]
fn test_a_cell_outline_runs_along_its_edges_11() -> Outcome<()> {
    let mut rng = Rng(3);
    for level in [0u8, 2, 7, 15, 22] {
        for _ in 0..20 {
            let n = 1u32 << level;
            let face = (rng.next_u64() % 6) as u8;
            let cell = res!(Cell::from_face_ij(face, level,
                (rng.next_u64() % n as u64) as u32, (rng.next_u64() % n as u64) as u32));
            let segs = 1 + (rng.next_u64() % 9) as u32;
            let out = cell.outline(segs);
            req!(out.len(), 4 * segs as usize);
            let c = cell.corners();
            let centre = cell.centre_vec();
            for (k, p) in out.iter().enumerate() {
                let (a, b) = (c[k / segs as usize].vec, c[(k / segs as usize + 1) % 4].vec);
                // On the great circle through the edge's two corners.
                let nrm = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                let len = dot(&nrm, &nrm).sqrt();
                let off = (dot(p, &nrm) / len).abs();
                // The normal of a short edge is itself only good to a unit in the last
                // place over the edge's length, so the tolerance grows as cells shrink.
                let tol = 1.0e-12 + 1.0e-15 / len;
                req!((off < tol), true, "Level {} outline point {} is {} off its edge.", level, k, off);
                // And a hair inside it is the cell itself.
                let t = 1.0e-4; // a ten-thousandth of the way to the centre
                let q = [p[0] + t * (centre[0] - p[0]), p[1] + t * (centre[1] - p[1]),
                    p[2] + t * (centre[2] - p[2])];
                let (lat, lng) = vec_lat_lng(&q);
                let owner = res!(Cell::at(lat, lng, level));
                req!(owner, cell, "Level {} outline point {} belongs to {}.", level, k, owner);
            }
        }
    }
    Ok(())
}

/// Every cell of a level, by enumeration, whose bounding cap meets the cap.
fn brute(centre: &[f64; 3], radius: f64, level: u8) -> Outcome<HashSet<Cell>> {
    let n = 1u32 << level;
    let mut out = HashSet::new();
    for face in 0..6u8 {
        for i in 0..n {
            for j in 0..n {
                let cell = res!(Cell::from_face_ij(face, level, i, j));
                let (v, r) = cell.bounding_cap();
                if angle(&v, centre) <= r + radius + 1.0e-12 {
                    out.insert(cell);
                }
            }
        }
    }
    Ok(out)
}

/// A random point within a cap, uniform over its area.
fn in_cap(rng: &mut Rng, c: &[f64; 3], radius: f64) -> [f64; 3] {
    let z = 1.0 - rng.unit() * (1.0 - radius.cos());
    let t = rng.unit() * TAU;
    let s = (1.0 - z * z).max(0.0).sqrt();
    // A frame about the centre.
    let up = if c[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let mut e = [up[1] * c[2] - up[2] * c[1], up[2] * c[0] - up[0] * c[2], up[0] * c[1] - up[1] * c[0]];
    let el = dot(&e, &e).sqrt();
    e = [e[0] / el, e[1] / el, e[2] / el];
    let nn = [c[1] * e[2] - c[2] * e[1], c[2] * e[0] - c[0] * e[2], c[0] * e[1] - c[1] * e[0]];
    let (a, b) = (s * t.cos(), s * t.sin());
    [z * c[0] + a * e[0] + b * nn[0], z * c[1] + a * e[1] + b * nn[1], z * c[2] + a * e[2] + b * nn[2]]
}

fn check_samples(rng: &mut Rng, c: &[f64; 3], radius: f64, level: u8, got: &[Cell]) -> Outcome<()> {
    let set: HashSet<Cell> = got.iter().cloned().collect();
    for _ in 0..500 {
        let p = in_cap(rng, c, radius);
        let (lat, lng) = vec_lat_lng(&p);
        let owner = res!(Cell::at(lat, lng, level));
        req!(set.contains(&owner), true, "A point in the cap lies in {}, not covered.", owner);
        let mut holders = 0;
        for cell in got {
            if res!(cell.contains(lat, lng)) {
                holders += 1;
            }
        }
        req!(holders, 1, "A point in the cap lies in {} covered cells.", holders);
    }
    Ok(())
}

#[test]
fn test_cover_cap_agrees_with_enumeration_12() -> Outcome<()> {
    let mut rng = Rng(5);
    // Level 3, every cell, caps from tiny to a third of the sphere, some on cube edges and
    // corners.
    let mut caps: Vec<([f64; 3], f64)> = vec![
        (unit_vec(35.26439, 45.0), 0.2),    // a cube vertex
        (unit_vec(0.0, 45.0), 0.05),        // a cube edge
        (unit_vec(90.0, 0.0), 0.7),         // the north pole
        (unit_vec(-31.9535, 115.8571), 1.1),
    ];
    for _ in 0..8 {
        let v = [rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5];
        caps.push((v, rng.unit()));
    }
    for (c, r) in &caps {
        let len = dot(c, c).sqrt();
        let cn = [c[0] / len, c[1] / len, c[2] / len];
        let got = res!(cell::cover_cap(*c, *r, 3, 10_000));
        let set: HashSet<Cell> = got.iter().cloned().collect();
        req!(set.len(), got.len(), "The cover listed a cell twice.");
        let want = res!(brute(&cn, *r, 3));
        req!(set, want, "Level 3 cover of {:?} radius {} differs from enumeration.", c, r);
        res!(check_samples(&mut rng, &cn, *r, 3, &got));
    }
    // Level 9, every one of its 1.6 million cells, for a cap across a cube vertex.
    let c = unit_vec(35.0, 44.0);
    let got = res!(cell::cover_cap(c, 0.02, 9, 100_000));
    let set: HashSet<Cell> = got.iter().cloned().collect();
    let want = res!(brute(&c, 0.02, 9));
    req!(set, want, "Level 9 cover across a cube vertex differs from enumeration.");
    let faces: HashSet<u8> = got.iter().map(|c| c.face()).collect();
    req!(faces.len(), 3, "The level 9 cap was meant to touch three faces, touched {}.", faces.len());
    res!(check_samples(&mut rng, &c, 0.02, 9, &got));
    Ok(())
}

#[test]
fn test_cover_cap_at_street_level_13() -> Outcome<()> {
    // Level 15 about Sydney, 1.5 km: enumerate a window of the face round the centre wide
    // enough that nothing on its border qualifies.
    let mut rng = Rng(9);
    let c = unit_vec(-33.8688, 151.2093);
    let r = 1_500.0 / EARTH_RADIUS_M;
    let got = res!(cell::cover_cap(c, r, 15, 10_000));
    let home = res!(Cell::at(-33.8688, 151.2093, 15));
    let (hi, hj) = home.coords();
    let k = 20i64;
    let mut want = HashSet::new();
    for di in -k..=k {
        for dj in -k..=k {
            let cell = res!(Cell::from_face_ij(home.face(), 15, (hi as i64 + di) as u32,
                (hj as i64 + dj) as u32));
            let (v, cr) = cell.bounding_cap();
            if angle(&v, &c) <= cr + r + 1.0e-12 {
                let edge = di.abs() == k || dj.abs() == k;
                req!(edge, false, "The enumeration window was too small.");
                want.insert(cell);
            }
        }
    }
    let set: HashSet<Cell> = got.iter().cloned().collect();
    req!(set, want, "Level 15 cover about Sydney differs from enumeration.");
    let ok = got.len() > 50 && got.len() < 400;
    req!(ok, true, "A 1.5 km cap took {} level 15 cells.", got.len());
    res!(check_samples(&mut rng, &c, r, 15, &got));
    // Nearest first: the first cell holds the centre.
    req!(got[0], home);
    // A cap too big for the bound is refused rather than walked.
    let refused = cell::cover_cap(c, 1.0, 9, 100).is_err();
    req!(refused, true, "A cap of a radian at level 9 was walked into a cover of 100.");
    Ok(())
}
