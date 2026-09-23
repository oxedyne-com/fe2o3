//! The world file and the spherical simplifier.
//!
//! The external check is byte identity with Ochre's `world.bin`, which Python wrote: run
//! `cargo run --release -p oxedyne_fe2o3_geom --example world_gen -- --compare
//! <ochre>/src/web/assets/world.bin` with Natural Earth cached.  It needs the downloads, so it
//! is not a unit test; what is here needs nothing but the coarse fixture that example wrote.

use oxedyne_fe2o3_geom::{
    proj::{
        EARTH_RADIUS_M,
        unit_vec,
    },
    world::{
        self,
        Label,
        Layer,
        LayerKind,
        World,
        MAGIC,
        Q,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

const WORLD: &[u8] = include_bytes!("data/world_coarse.bin");

#[test]
fn test_the_coarse_world_reads_back_as_the_world_00() -> Outcome<()> {
    let w = res!(world::read(WORLD));
    let named: Vec<&str> = w.layers.iter().map(|l| l.name.as_str()).collect();
    req!(named, vec!["land", "lakes", "borders"]);
    req!(w.layers[0].kind, LayerKind::Fill);
    req!(w.layers[2].kind, LayerKind::Stroke);
    req!(w.layers[0].tol_m, 30_000.0);
    req!(w.details(), 1);
    // Ochre's report for this level: 190 land rings of 4202 vertices, 11 lakes of 129, 183
    // borders of 559.
    for (layer, rings, verts) in [(0usize, 190usize, 4202usize), (1, 11, 129), (2, 183, 559)] {
        let l = &w.layers[layer];
        req!(l.rings.len(), rings, "{} has {} rings.", l.name, l.rings.len());
        let n: usize = l.rings.iter().map(|r| r.len()).sum();
        req!(n, verts, "{} has {} vertices.", l.name, n);
        for r in &l.rings {
            for (lat, lng) in r {
                let real = lat.abs() <= 90.0 && lng.abs() <= 540.0;
                req!(real, true, "{}, {} is not on the Earth.", lat, lng);
            }
        }
    }
    // And writing what was read gives the same bytes.
    let again = res!(world::write(&w));
    req!((again == WORLD), true, "The world did not write back as it was read.");
    // Reading one level alone gives that level.
    let only = res!(world::read_detail(WORLD, Some(0)));
    req!(only, w);
    let none = res!(world::read_detail(WORLD, Some(1)));
    req!(none.layers.len(), 0);
    Ok(())
}

#[test]
fn test_every_position_written_is_a_source_position_01() -> Outcome<()> {
    // A ring with steps long enough to need a jump, one across the antimeridian, and a vertex
    // that rounds onto its neighbour.
    let ring = vec![
        (10.0, 170.0), (10.4, 179.9), (10.4, -179.9), (-5.0, -170.0),
        (-5.000_001, -170.000_001), (-20.0, 150.0), (10.0, 170.0),
    ];
    let w = World { layers: vec![Layer {
        name: "land".to_string(), kind: LayerKind::Fill, detail: 0, tol_m: 1_000.0,
        rings: vec![ring.clone()], labels: Vec::new(),
    }]};
    let bytes = res!(world::write(&w));
    req!(&bytes[..8], &MAGIC[..]);
    req!(u16::from_le_bytes([bytes[8], bytes[9]]), world::VERSION_RINGS);
    let back = res!(world::read(&bytes));
    let source: HashSet<(i64, i64)> = ring.iter()
        .map(|(a, b)| ((a * Q).round() as i64, (b * Q).round() as i64))
        .collect();
    let got = &back.layers[0].rings[0];
    req!(got.len(), ring.len() - 1, "The duplicate vertex was not dropped.");
    for (lat, lng) in got {
        let q = ((lat * Q).round() as i64, (lng * Q).round() as i64);
        // Longitude may come back a turn away, walked the short way round.
        let turned = (q.0, (q.1 + 180 * 100_000).rem_euclid(360 * 100_000) - 180 * 100_000);
        let known = source.contains(&q) || source.contains(&turned);
        req!(known, true, "{}, {} is not a position the ring was given.", lat, lng);
    }
    Ok(())
}

#[test]
fn test_labels_round_trip_in_version_2_02() -> Outcome<()> {
    let labels = vec![
        Label { lat: -31.9535, lng: 115.8571, rank: 0, name: "Perth".to_string() },
        Label { lat: 35.6895, lng: 139.6917, rank: 0, name: "東京".to_string() },
        Label { lat: 64.8378, lng: -147.7164, rank: 4, name: "Fairbanks".to_string() },
    ];
    let mut w = res!(world::read(WORLD));
    w.layers.push(Layer {
        name: "places".to_string(), kind: LayerKind::Label, detail: 0, tol_m: 0.0,
        rings: Vec::new(), labels: labels.clone(),
    });
    let bytes = res!(world::write(&w));
    req!(u16::from_le_bytes([bytes[8], bytes[9]]), world::VERSION_LABELS);
    let back = res!(world::read(&bytes));
    req!(back, w);
    let again = res!(world::write(&back));
    req!((again == bytes), true, "A labelled world did not write back as it was read.");
    let places = res!(back.layer("places", 0).ok_or_else(|| err!("No places."; Test)));
    req!(places.labels[1].name.as_str(), "東京");
    Ok(())
}

#[test]
fn test_a_world_file_that_is_not_one_is_refused_03() -> Outcome<()> {
    req!(world::read(b"nothing at all").is_err(), true, "Rubbish was accepted.");
    let mut ochre = WORLD.to_vec();
    ochre[..8].copy_from_slice(b"OCHRWRLD");
    req!(world::read(&ochre).is_err(), true, "Another magic was accepted.");
    let mut later = WORLD.to_vec();
    later[8] = 9;
    req!(world::read(&later).is_err(), true, "A later version was accepted.");
    let cut = &WORLD[..WORLD.len() / 2];
    req!(world::read(cut).is_err(), true, "A truncated file was accepted.");
    let mut long = WORLD.to_vec();
    long.push(0);
    req!(world::read(&long).is_err(), true, "A file with a trailing byte was accepted.");
    // A label layer in a version 1 file.
    let mut mislabelled = WORLD.to_vec();
    mislabelled[12] = 2;
    req!(world::read(&mislabelled).is_err(), true, "Labels in version 1 were accepted.");
    let named = World { layers: vec![Layer {
        name: "coastline".to_string(), kind: LayerKind::Stroke, detail: 0, tol_m: 0.0,
        rings: Vec::new(), labels: Vec::new(),
    }]};
    req!(world::write(&named).is_err(), true, "A name longer than seven bytes was written.");
    Ok(())
}

#[test]
fn test_the_simplifier_keeps_every_edge_within_its_tolerance_04() -> Outcome<()> {
    // A wobbly line round a quarter of the equator: every dropped vertex lies within the
    // tolerance of the chord between the kept vertices either side of it.
    let n = 4_000;
    let pts: Vec<[f64; 3]> = (0..n).map(|k| {
        let t = k as f64 / n as f64;
        unit_vec(0.3 * (t * 157.0).sin() + 0.05 * (t * 1_031.0).cos(), 90.0 * t)
    }).collect();
    for eps_m in [1_000.0, 6_000.0, 30_000.0] {
        let keep = world::simplify_sphere(&pts, eps_m);
        req!(keep[0], 0);
        req!(keep[keep.len() - 1], n - 1);
        let fewer = keep.len() < n && keep.len() > 2;
        req!(fewer, true, "At {} m, {} of {} vertices were kept.", eps_m, keep.len(), n);
        let eps = eps_m / EARTH_RADIUS_M;
        for pair in keep.windows(2) {
            let (a, b) = (pts[pair[0]], pts[pair[1]]);
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            for k in pair[0] + 1..pair[1] {
                let p = pts[k];
                let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1] + (p[2] - a[2]) * d[2]) / dd)
                    .clamp(0.0, 1.0);
                let off = ((p[0] - a[0] - t * d[0]).powi(2) + (p[1] - a[1] - t * d[1]).powi(2)
                    + (p[2] - a[2] - t * d[2]).powi(2)).sqrt();
                req!((off <= eps), true, "At {} m a dropped vertex is {} m off.",
                    eps_m, off * EARTH_RADIUS_M);
            }
        }
    }
    // Nothing to do below three vertices, or at no tolerance.
    req!(world::simplify_sphere(&pts[..2], 1_000.0), vec![0, 1]);
    req!(world::simplify_sphere(&pts[..5], 0.0), vec![0, 1, 2, 3, 4]);
    Ok(())
}
