//! Draws a vector tile as an SVG, styled by class, to show what the decoder hands a painter.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_geom --example mvt_svg -- --mvt TILE.mvt --zoom Z [--out tile.svg]
//! ```
//!
//! The tile is a decompressed Mapbox Vector Tile in the Protomaps basemap schema.  Features are
//! sorted into classes by [`mvt::classify`] -- land use, water, buildings, roads by kind, rail
//! -- and painted in rule order, which is the order a map paints them.

use oxedyne_fe2o3_geom::mvt::{
    self,
    GeomKind,
    StyleRule,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::Write as _;

// Class, fill, stroke, stroke width in pixels at 1024 across.
const PAINT: [(u16, &str, &str, f64); 11] = [
    (11, "#f4f1ea", "none",   0.0),     // land, over the sea the background is
    (1, "#dfe9cf", "none",    0.0),     // parks and grass
    (2, "#c9dcb8", "none",    0.0),     // wood
    (3, "#bcd9ea", "none",    0.0),     // water
    (4, "#e3ddd3", "#cfc6b8", 0.6),     // buildings
    (5, "none",    "#e6e0d6", 1.2),     // paths
    (6, "none",    "#ffffff", 2.4),     // minor roads
    (7, "none",    "#f7e7b4", 4.0),     // major roads
    (8, "none",    "#f3c98b", 5.5),     // highways
    (9, "none",    "#9a9486", 1.4),     // rail
    (10, "none",   "#8fb3c8", 1.2),     // ferries
];

fn rule(layer: &str, values: &[&str], class: u16) -> StyleRule {
    StyleRule {
        layer:      layer.to_string(),
        key:        "kind".to_string(),
        values:     values.iter().map(|v| v.to_string()).collect(),
        min_zoom:   0,
        class,
    }
}

fn main() -> Outcome<()> {
    let (mut path, mut out, mut zoom) = (None, "tile.svg".to_string(), 14u8);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let v = res!(it.next().ok_or_else(|| err!("{} wants a value.", a; Input, Missing)));
        match a.as_str() {
            "--mvt"     => path = Some(v),
            "--out"     => out = v,
            "--zoom"    => zoom = res!(v.parse::<u8>(), Input, Invalid),
            other       => return Err(err!("Unknown argument {:?}.", other; Input, Invalid)),
        }
    }
    let path = res!(path.ok_or_else(|| err!("--mvt is needed."; Input, Missing)));
    let bytes = res!(std::fs::read(&path), File, Read);
    let tile = res!(mvt::decode(&bytes));
    let rules = vec![
        rule("earth", &["earth"], 11),
        rule("landuse", &["park", "grass", "garden", "playground", "pitch", "golf_course", "cemetery"], 1),
        rule("landuse", &["wood", "forest", "scrub"], 2),
        rule("water", &[], 3),
        rule("buildings", &[], 4),
        rule("roads", &["path"], 5),
        rule("roads", &["minor_road", "other"], 6),
        rule("roads", &["major_road"], 7),
        rule("roads", &["highway"], 8),
        rule("roads", &["rail"], 9),
        rule("roads", &["ferry"], 10),
    ];
    let hits = mvt::classify(&tile, &rules, zoom, Some("min_zoom"));
    let size = 1024.0;
    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{0}\" height=\"{1}\" \
        viewBox=\"0 0 {0} {1}\">\n<rect width=\"100%\" height=\"100%\" fill=\"#bcd9ea\"/>\n\
        <clipPath id=\"t\"><rect width=\"{0}\" height=\"{0}\"/></clipPath>\n<g clip-path=\"url(#t)\">\n",
        size, size + 30.0);
    let mut drawn = 0usize;
    for (class, fill, stroke, width) in PAINT.iter() {
        let mut d = String::new();
        for (c, li, fi) in &hits {
            if c != class {
                continue;
            }
            let layer = &tile.layers[*li];
            let f = &layer.features[*fi];
            // A fill class paints polygons and a stroke class lines: the water layer also
            // carries river and strait lines for labels, which filled would paint wedges.
            let fills = *fill != "none";
            let fits = match f.kind {
                GeomKind::Polygon       => fills,
                GeomKind::LineString    => !fills,
                _                       => false,
            };
            if !fits {
                continue;
            }
            let k = size / layer.extent as f64;
            for ring in res!(f.geometry()) {
                for (i, (x, y)) in ring.iter().enumerate() {
                    let _ = write!(d, "{}{:.1} {:.1}", if i == 0 { "M" } else { "L" }, *x as f64 * k, *y as f64 * k);
                }
                if f.kind == GeomKind::Polygon {
                    d.push('Z');
                }
            }
            drawn += 1;
        }
        let _ = write!(s, "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" \
            stroke-linecap=\"round\" stroke-linejoin=\"round\" fill-rule=\"nonzero\"/>\n",
            d, fill, stroke, width);
    }
    let _ = write!(s, "</g>\n<text x=\"8\" y=\"{}\" font-family=\"sans-serif\" font-size=\"14\">\
        {} -- {} features drawn of {}, decoded by fe2o3_geom::mvt. Map data (c) OpenStreetMap \
        contributors (ODbL), Protomaps build.</text>\n</svg>\n", size + 21.0,
        std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        drawn, tile.layers.iter().map(|l| l.features.len()).sum::<usize>());
    res!(std::fs::write(&out, s), File, Write);
    println!("wrote {} ({} features drawn)", out, drawn);
    Ok(())
}
