//! Draws a world file from Natural Earth, and optionally a picture of it.
//!
//! ```text
//! cargo run --release -p oxedyne_fe2o3_geom --example world_gen -- \
//!     [--ne DIR] [--spec ochre] [--level TOL,ISLAND,LAKE,BORDER]... [--detail N] \
//!     [--places FILE --rank-max R] [--out FILE] [--compare FILE] [--svg FILE]
//! ```
//!
//! Natural Earth is public domain.  The GeoJSON is read from `DIR` (default
//! `~/.cache/natural-earth`), and a missing layer is named with the address to fetch it from;
//! nothing here reaches the network.
//!
//! `--spec ochre` (the default) is the three levels Ochre ships: thirty kilometres, six and
//! one, each with its least island in square kilometres, least lake and least border in
//! kilometres.  `--level` replaces them, once per level, coarsest first.  `--detail N` keeps
//! only level `N`.  `--compare FILE` checks the result against another world file byte for
//! byte after the eight-byte magic, which is how this is held to Ochre's `world.bin`.
//!
//! The numbers are Ochre's `dev/gen_world.py`'s, and so is the arithmetic, down to the order
//! of the additions: a ring's area decides whether it is drawn, and a rounding difference at a
//! threshold would drop an island.  Python's `sum` of floats is compensated since 3.12 and its
//! `%` takes the sign of the divisor, and both are reproduced below.

use oxedyne_fe2o3_geom::{
    cell::Cell,
    proj::{
        EARTH_RADIUS_M,
        Projection,
        RingMode,
        ScreenPaths,
        Viewport,
        unit_vec,
    },
    world::{
        self,
        Label,
        Layer,
        LayerKind,
        World,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use std::{
    collections::BTreeMap,
    fmt::Write as _,
};

const NE_URL: &str = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/";

// Ochre's levels, as (tolerance m, least island km2, least lake km2, least border km).
const OCHRE_LEVELS: [[f64; 4]; 3] = [
    [30_000.0, 2_000.0, 20_000.0, 200.0],
    [ 6_000.0,   100.0,  2_000.0,  40.0],
    [ 1_000.0,     2.0,    550.0,   5.0],
];

// The layers, in file order: name, kind, sources, which of a level's thresholds applies.
const LAYERS: [(&str, LayerKind, &[&str], usize); 3] = [
    ("land",    LayerKind::Fill,    &["ne_10m_land", "ne_10m_minor_islands"],   1),
    ("lakes",   LayerKind::Fill,    &["ne_10m_lakes"],                          2),
    ("borders", LayerKind::Stroke,  &["ne_10m_admin_0_boundary_lines_land"],    3),
];

/// A GeoJSON coordinate as Python would hold it: the value, and whether it was written as a
/// whole number, which Python's `sum` treats differently.
#[derive(Clone, Copy)]
struct Num {
    v:      f64,
    int:    bool,
}

type Ring = Vec<(Num, Num)>; // (lng, lat), as GeoJSON orders them

struct Args {
    ne:         String,
    levels:     Vec<[f64; 4]>,
    detail:     Option<u8>,
    places:     Option<String>,
    rank_max:   u8,
    out:        Option<String>,
    compare:    Option<String>,
    svg:        Option<String>,
}

fn args() -> Outcome<Args> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut a = Args {
        ne: fmt!("{}/.cache/natural-earth", home), levels: Vec::new(), detail: None,
        places: None, rank_max: 3, out: None, compare: None, svg: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || -> Outcome<String> {
            it.next().ok_or_else(|| err!("{} wants a value.", flag; Input, Missing))
        };
        match flag.as_str() {
            "--ne"          => a.ne = res!(val()),
            "--spec"        => {
                let s = res!(val());
                if s != "ochre" {
                    return Err(err!("No spec called {:?}; there is `ochre`.", s; Input, Invalid));
                }
            },
            "--level"       => {
                let s = res!(val());
                let parts: Vec<f64> = res!(s.split(',').map(|p| p.trim().parse::<f64>())
                    .collect::<Result<Vec<f64>, _>>(), Input, Invalid);
                if parts.len() != 4 {
                    return Err(err!("--level {:?} wants four numbers.", s; Input, Invalid));
                }
                a.levels.push([parts[0], parts[1], parts[2], parts[3]]);
            },
            "--detail"      => a.detail = Some(res!(res!(val()).parse::<u8>(), Input, Invalid)),
            "--places"      => a.places = Some(res!(val())),
            "--rank-max"    => a.rank_max = res!(res!(val()).parse::<u8>(), Input, Invalid),
            "--out"         => a.out = Some(res!(val())),
            "--compare"     => a.compare = Some(res!(val())),
            "--svg"         => a.svg = Some(res!(val())),
            other           => return Err(err!("Unknown argument {:?}.", other; Input, Invalid)),
        }
    }
    if a.levels.is_empty() {
        a.levels = OCHRE_LEVELS.to_vec();
    }
    Ok(a)
}

fn main() -> Outcome<()> {
    let a = res!(args());
    let mut sources: BTreeMap<String, Vec<Ring>> = BTreeMap::new();
    for (_, _, names, _) in LAYERS.iter() {
        for name in names.iter() {
            if !sources.contains_key(*name) {
                let path = fmt!("{}/{}.geojson", a.ne, name);
                let dat = res!(load(&path, name));
                sources.insert(name.to_string(), res!(rings_of_file(&dat, name)));
            }
        }
    }

    let mut w = World::default();
    for (name, kind, names, which) in LAYERS.iter() {
        for (detail, level) in a.levels.iter().enumerate() {
            if a.detail.map_or(false, |d| d as usize != detail) {
                continue;
            }
            let mut rings: Vec<Vec<(f64, f64)>> = Vec::new();
            for src in names.iter() {
                if let Some(list) = sources.get(*src) {
                    prepare(list, *kind, level[0], level[*which], &mut rings);
                }
            }
            w.layers.push(Layer {
                name: name.to_string(), kind: *kind, detail: detail as u8, tol_m: level[0],
                rings, labels: Vec::new(),
            });
        }
    }
    if let Some(path) = &a.places {
        let dat = res!(load(path, "ne_10m_populated_places"));
        let labels = res!(places(&dat, a.rank_max));
        w.layers.push(Layer {
            name: "places".to_string(), kind: LayerKind::Label, detail: 0, tol_m: 0.0,
            rings: Vec::new(), labels,
        });
    }

    let bytes = res!(world::write(&w));
    report(&w, bytes.len());
    // What was written must read back as what was meant.
    let back = res!(world::read(&bytes));
    let again = res!(world::write(&back));
    if again != bytes {
        return Err(err!("The world did not survive a round trip through its own file."; Bug));
    }
    if let Some(path) = &a.out {
        res!(std::fs::write(path, &bytes), File, Write);
        println!("wrote {} ({} bytes)", path, bytes.len());
    }
    if let Some(path) = &a.compare {
        let other = res!(std::fs::read(path), File, Read);
        let same = other.len() == bytes.len() && other.get(8..) == bytes.get(8..);
        if same {
            println!("byte-identical to {} after the magic ({} bytes)", path, bytes.len());
        } else {
            let at = other.iter().skip(8).zip(bytes.iter().skip(8)).position(|(x, y)| x != y);
            return Err(err!("Differs from {}: {} bytes against {}, first difference at {:?}.",
                path, bytes.len(), other.len(), at.map(|p| p + 8); Mismatch));
        }
    }
    if let Some(path) = &a.svg {
        res!(svg(&back, path));
        println!("wrote {}", path);
    }
    Ok(())
}

fn load(path: &str, name: &str) -> Outcome<Dat> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Err(err!(e,
            "Cannot read {}.  Fetch it once with\n    curl -o {} {}{}.geojson",
            path, path, NE_URL, name; File, Read)),
    };
    let cfg = DecoderConfig::<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>::json(None);
    Dat::decode_string_with_config(text, &cfg)
}

fn num(d: &Dat) -> Outcome<Num> {
    let int = match d {
        Dat::U8(_) | Dat::U16(_) | Dat::U32(_) | Dat::U64(_) |
        Dat::I8(_) | Dat::I16(_) | Dat::I32(_) | Dat::I64(_) => true,
        _ => false,
    };
    match d.get_float64() {
        Some(f) => Ok(Num { v: f.0, int }),
        None    => Err(err!("A coordinate {:?} is not a number.", d; Input, Mismatch)),
    }
}

fn position(d: &Dat) -> Outcome<(Num, Num)> {
    match d {
        Dat::List(v) if v.len() >= 2 => Ok((res!(num(&v[0])), res!(num(&v[1])))),
        other => Err(err!("A position {:?} is not a pair.", other.kind(); Input, Mismatch)),
    }
}

fn line(d: &Dat) -> Outcome<Ring> {
    match d {
        Dat::List(v) => {
            let mut out = Vec::with_capacity(v.len());
            for p in v {
                out.push(res!(position(p)));
            }
            Ok(out)
        },
        other => Err(err!("A line {:?} is not a list.", other.kind(); Input, Mismatch)),
    }
}

fn list(d: &Dat) -> Outcome<&Vec<Dat>> {
    match d {
        Dat::List(v) => Ok(v),
        other => Err(err!("Expected a list, found {:?}.", other.kind(); Input, Mismatch)),
    }
}

/// Every ring or line of every feature, whichever geometry it carries, in file order.
fn rings_of_file(dat: &Dat, name: &str) -> Outcome<Vec<Ring>> {
    let features = res!(dat.map_get_list(&dat!("features")));
    let mut out = Vec::new();
    for f in features {
        let geom = match res!(f.map_get(&dat!("geometry"))) {
            Some(g @ Dat::Map(_)) | Some(g @ Dat::OrdMap(_)) => g,
            _ => continue,
        };
        let kind = res!(geom.map_get_string(&dat!("type")));
        let coords = res!(geom.map_get_must(&dat!("coordinates")));
        match kind.as_str() {
            "MultiPolygon" => for poly in res!(list(coords)) {
                for r in res!(list(poly)) {
                    out.push(res!(line(r)));
                }
            },
            "Polygon" | "MultiLineString" => for r in res!(list(coords)) {
                out.push(res!(line(r)));
            },
            "LineString" => out.push(res!(line(coords))),
            _ => (),
        }
    }
    println!("{}: {} rings", name, out.len());
    Ok(out)
}

/// Python's `sum` over floats and small integers: Neumaier's compensated sum for the floats,
/// integers added plainly, and an integer prefix summed exactly before the first float.
fn py_sum(vals: impl Iterator<Item = Num>) -> f64 {
    let mut i_sum: i64 = 0;
    let mut floating = false;
    let mut f = 0.0f64;
    let mut c = 0.0f64;
    for n in vals {
        if !floating {
            if n.int {
                i_sum += n.v as i64;
                continue;
            }
            floating = true;
            f = i_sum as f64 + n.v;
            continue;
        }
        if n.int {
            f += n.v;
            continue;
        }
        let t = f + n.v;
        if f.abs() >= n.v.abs() {
            c += (f - t) + n.v;
        } else {
            c += (n.v - t) + f;
        }
        f = t;
    }
    if !floating {
        return i_sum as f64;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// Python's `%` on floats, which takes the sign of the divisor.
fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) { m + b } else { m }
    } else {
        0.0f64.copysign(b)
    }
}

/// Roughly how much ground a ring encloses, in square kilometres: the shoelace formula in a
/// plane laid on the ring's own middle latitude.
fn area_km2(ring: &Ring) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mid = py_sum(ring.iter().map(|c| c.1)) / n as f64;
    let k = mid.to_radians().cos();
    let mut a = 0.0;
    for i in 0..n {
        let (x1, y1) = (ring[i].0.v * k, ring[i].1.v);
        let (x2, y2) = (ring[(i + 1) % n].0.v * k, ring[(i + 1) % n].1.v);
        a += x1 * y2 - x2 * y1;
    }
    a.abs() / 2.0 * 111.32f64.powf(2.0)
}

/// Roughly how long a line is, in kilometres.
fn length_km(ring: &Ring) -> f64 {
    let mut total = 0.0;
    for i in 0..ring.len().saturating_sub(1) {
        let k = ((ring[i].1.v + ring[i + 1].1.v) / 2.0).to_radians().cos();
        let dx = (py_mod(ring[i + 1].0.v - ring[i].0.v + 180.0, 360.0) - 180.0) * k;
        let dy = ring[i + 1].1.v - ring[i].1.v;
        total += dx.hypot(dy);
    }
    total * 111.32
}

/// Every ring of a source, pruned by size and simplified, appended as `(lat, lng)`.
fn prepare(src: &[Ring], kind: LayerKind, eps_m: f64, least: f64, out: &mut Vec<Vec<(f64, f64)>>) {
    for ring in src {
        if ring.len() < 2 {
            continue;
        }
        let small = match kind {
            LayerKind::Fill => area_km2(ring) < least,
            _               => length_km(ring) < least,
        };
        if small {
            continue;
        }
        let vecs: Vec<[f64; 3]> = ring.iter().map(|(lng, lat)| unit_vec(lat.v, lng.v)).collect();
        let keep = world::simplify_sphere(&vecs, eps_m);
        let least_n = if kind == LayerKind::Fill { 4 } else { 2 };
        if keep.len() < least_n {
            continue;
        }
        out.push(keep.iter().map(|i| (ring[*i].1.v, ring[*i].0.v)).collect());
    }
}

/// Populated places up to a rank, most important first.
fn places(dat: &Dat, rank_max: u8) -> Outcome<Vec<Label>> {
    let features = res!(dat.map_get_list(&dat!("features")));
    let mut out = Vec::new();
    for f in features {
        let props = res!(f.map_get_must(&dat!("properties")));
        let rank = match res!(props.map_get(&dat!("SCALERANK"))) {
            Some(r) => r,
            None => res!(props.map_get_must(&dat!("scalerank"))),
        };
        let rank = match rank.get_float64() {
            Some(r) => r.0,
            None => continue,
        };
        if !(rank >= 0.0 && rank <= rank_max as f64) {
            continue;
        }
        let name = match res!(props.map_get(&dat!("NAME"))) {
            Some(Dat::Str(s)) => s.clone(),
            _ => res!(props.map_get_string(&dat!("name"))),
        };
        let geom = res!(f.map_get_must(&dat!("geometry")));
        let (lng, lat) = res!(position(res!(geom.map_get_must(&dat!("coordinates")))));
        out.push(Label { lat: lat.v, lng: lng.v, rank: rank as u8, name });
    }
    out.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn report(w: &World, size: usize) {
    println!("{:<9} {:<6} {:>9} {:>7} {:>9}", "layer", "drawn", "tolerance", "rings", "vertices");
    for l in &w.layers {
        let verts: usize = l.rings.iter().map(|r| r.len()).sum();
        println!("{:<9} {:<6} {:>7.0} m {:>7} {:>9}",
            l.name, fmt!("{:?}", l.kind).to_lowercase(), l.tol_m,
            if l.kind == LayerKind::Label { l.labels.len() } else { l.rings.len() }, verts);
    }
    println!("total {:.1} KiB", size as f64 / 1024.0);
}

// ---------------------------------------------------------------------------------------------
// The picture
// ---------------------------------------------------------------------------------------------

fn path_d(paths: &ScreenPaths, dx: f64, dy: f64) -> String {
    let mut d = String::new();
    for i in 0..paths.len() {
        if let Some((pts, closed)) = paths.path(i) {
            for (k, p) in pts.chunks(2).enumerate() {
                let _ = write!(d, "{}{:.1} {:.1}", if k == 0 { "M" } else { "L" },
                    p[0] as f64 + dx, p[1] as f64 + dy);
            }
            if closed {
                d.push('Z');
            }
        }
    }
    d
}

/// The world on a globe and on a flat map, with level-3 cells over both.
fn svg(w: &World, path: &str) -> Outcome<()> {
    let detail = if w.details() > 1 { 1 } else { 0 };
    let (gw, mw, h) = (620.0, 1000.0, 620.0);
    let globe = res!(Viewport::new(Projection::Orthographic, 10.0, 110.0, 0.0,
        EARTH_RADIUS_M / 290.0, gw, h));
    let flat = res!(Viewport::new(Projection::WebMercator, 20.0, 150.0, 0.0,
        std::f64::consts::TAU * EARTH_RADIUS_M / mw * (20.0f64).to_radians().cos(), mw, h));
    let mut cells: Vec<Vec<[f64; 3]>> = Vec::new();
    for face in 0..6u8 {
        for i in 0..8u32 {
            for j in 0..8u32 {
                cells.push(res!(Cell::from_face_ij(face, 3, i, j)).outline(8));
            }
        }
    }
    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
        viewBox=\"0 0 {} {}\">\n<rect width=\"100%\" height=\"100%\" fill=\"#f4f1ea\"/>\n",
        gw + mw + 20.0, h + 40.0, gw + mw + 20.0, h + 40.0);
    let _ = write!(s, "<text x=\"10\" y=\"24\" font-family=\"sans-serif\" font-size=\"15\">\
        Natural Earth, level of detail {} ({:.0} km), with level-3 cells: orthographic globe and \
        Web Mercator map</text>\n", detail,
        w.layer("land", detail).map_or(0.0, |l| l.tol_m / 1000.0));
    for (view, dx) in [(globe, 0.0), (flat, gw + 20.0)] {
        let dy = 40.0;
        // Each panel is clipped to itself, since the flat map carries a margin off its edge.
        let _ = write!(s, "<clipPath id=\"p{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" \
            height=\"{}\"/></clipPath>\n<g clip-path=\"url(#p{})\">\n",
            dx as u32, dx, dy, view.w, view.h, dx as u32);
        if view.kind == Projection::Orthographic {
            let _ = write!(s, "<circle cx=\"{}\" cy=\"{}\" r=\"290\" fill=\"#bcd9ea\"/>\n",
                dx + gw / 2.0, dy + h / 2.0);
        } else {
            let _ = write!(s, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#bcd9ea\"/>\n",
                dx, dy, mw, h);
        }
        for (name, fill, stroke, mode) in [
            ("land",    "#e8e2cf", "#8a8270", RingMode::Fill),
            ("lakes",   "#bcd9ea", "none",    RingMode::Fill),
            ("borders", "none",    "#a39a86", RingMode::Line),
        ] {
            if let Some(layer) = w.layer(name, detail) {
                let mut out = ScreenPaths::new();
                res!(view.project_rings(&layer.unit_rings(), mode, layer.tol_m, 0.3, &mut out));
                let _ = write!(s, "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"0.6\" \
                    fill-rule=\"nonzero\"/>\n", path_d(&out, dx, dy), fill, stroke);
            }
        }
        let mut out = ScreenPaths::new();
        res!(view.project_rings(&cells, RingMode::Outline, 0.0, 0.3, &mut out));
        let _ = write!(s, "<path d=\"{}\" fill=\"none\" stroke=\"#0096b4\" stroke-width=\"0.7\" \
            stroke-opacity=\"0.8\"/>\n", path_d(&out, dx, dy));
        s.push_str("</g>\n");
    }
    s.push_str("</svg>\n");
    res!(std::fs::write(path, s), File, Write);
    Ok(())
}
