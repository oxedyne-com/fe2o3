//! Draws the cell grid on a globe and on a flat map, as an SVG.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_geom --example cells_svg -- [--out cells.svg]
//! ```
//!
//! The left of the picture is an orthographic globe turned to Perth, the right a Web Mercator
//! strip of the whole world; both show the six cube faces in bold, level 2 in full and level 4
//! over the globe's middle, all through [`Viewport::project_rings`], the routine a map draws
//! with.  Nothing here reads a file.

use oxedyne_fe2o3_geom::{
    cell::{
        self,
        Cell,
    },
    proj::{
        EARTH_RADIUS_M,
        Projection,
        RingMode,
        ScreenPaths,
        Viewport,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::Write as _;

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

fn level(l: u8, segs: u32) -> Outcome<Vec<Vec<[f64; 3]>>> {
    let n = 1u32 << l;
    let mut out = Vec::new();
    for face in 0..6u8 {
        for i in 0..n {
            for j in 0..n {
                out.push(res!(Cell::from_face_ij(face, l, i, j)).outline(segs));
            }
        }
    }
    Ok(out)
}

fn main() -> Outcome<()> {
    let mut out_path = "cells.svg".to_string();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out_path = res!(it.next().ok_or_else(|| err!("--out wants a path."; Input, Missing))),
            other   => return Err(err!("Unknown argument {:?}.", other; Input, Invalid)),
        }
    }
    let (gw, mw, h) = (560.0, 900.0, 560.0);
    let globe = res!(Viewport::new(Projection::Orthographic, -31.9535, 115.8571, 0.0,
        EARTH_RADIUS_M / 260.0, gw, h));
    let flat = res!(Viewport::new(Projection::WebMercator, 0.0, 115.8571, 0.0,
        std::f64::consts::TAU * EARTH_RADIUS_M / mw, mw, h));
    let faces = res!(level(0, 32));
    let l2 = res!(level(2, 8));
    // Level 4 only over the middle of the globe, found with the covering the map would use.
    let (c, _) = globe.bounding_cap();
    let near = res!(cell::cover_cap(c, 0.6, 4, 2_000));
    let l4: Vec<Vec<[f64; 3]>> = near.iter().map(|cell| cell.outline(4)).collect();
    let home = res!(Cell::at(-31.9535, 115.8571, 4));

    let mut s = String::new();
    let (tw, th) = (gw + mw + 20.0, h + 50.0);
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
        viewBox=\"0 0 {} {}\">\n<rect width=\"100%\" height=\"100%\" fill=\"#f4f1ea\"/>\n\
        <text x=\"10\" y=\"26\" font-family=\"sans-serif\" font-size=\"15\">Cube-sphere cells: \
        the six faces (bold), level 2, level 4 near Perth ({} cells covered), and Perth's own \
        level-4 cell (yellow)</text>\n", tw, th, tw, th, near.len());
    for (view, dx) in [(globe, 0.0), (flat, gw + 20.0)] {
        let dy = 50.0;
        // Each panel is clipped to itself, since the flat map carries a margin off its edge.
        let _ = write!(s, "<clipPath id=\"p{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" \
            height=\"{}\"/></clipPath>\n<g clip-path=\"url(#p{})\">\n",
            dx as u32, dx, dy, view.w, view.h, dx as u32);
        if view.kind == Projection::Orthographic {
            let _ = write!(s, "<circle cx=\"{}\" cy=\"{}\" r=\"260\" fill=\"#bcd9ea\"/>\n",
                dx + gw / 2.0, dy + h / 2.0);
        } else {
            let _ = write!(s, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#bcd9ea\"/>\n",
                dx, dy, mw, h);
        }
        let mut pick = ScreenPaths::new();
        res!(view.project_rings(&[home.outline(8)], RingMode::Fill, 0.0, 0.3, &mut pick));
        let _ = write!(s, "<path d=\"{}\" fill=\"#ffd400\" stroke=\"#b08f00\"/>\n", path_d(&pick, dx, dy));
        for (rings, width, colour) in [(&l4, 0.5, "#5aa9bf"), (&l2, 0.8, "#0096b4"), (&faces, 2.0, "#004e63")] {
            let mut out = ScreenPaths::new();
            res!(view.project_rings(rings, RingMode::Outline, 0.0, 0.3, &mut out));
            let _ = write!(s, "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                path_d(&out, dx, dy), colour, width);
        }
        s.push_str("</g>\n");
    }
    s.push_str("</svg>\n");
    res!(std::fs::write(&out_path, s), File, Write);
    println!("wrote {}", out_path);
    Ok(())
}
