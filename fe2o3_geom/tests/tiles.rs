//! Map tiles: the grid, PMTiles and Mapbox Vector Tiles.
//!
//! The oracles come from outside this library: PROJ 9.7.1 for the tile corners, the vector
//! tile specification's own fixtures (`@mapbox/mvt-fixtures` 4.0.0, CC0) for the decoder, and
//! for real data the Protomaps planet build as read by the reference JavaScript readers
//! (`tests/data/protomaps/NOTICE.md`).

use oxedyne_fe2o3_geom::{
    mvt::{
        self,
        GeomKind,
        StyleRule,
        Value,
    },
    planar::Pt,
    proj::{
        Projection,
        Viewport,
    },
    tile::{
        self,
        pmtiles::{
            self,
            Archive,
            Compression,
            FileSource,
            RangeSource,
            TileType,
        },
        TileId,
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
    collections::{
        BTreeMap,
        HashSet,
    },
    path::PathBuf,
};

fn data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(rel)
}

fn json(text: &str) -> Outcome<Dat> {
    let cfg = DecoderConfig::<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>::json(None);
    Dat::decode_string_with_config(text, &cfg)
}

fn json_file(rel: &str) -> Outcome<Dat> {
    let text = res!(std::fs::read_to_string(data(rel)), File, Read);
    json(&text)
}

fn get<'a>(d: &'a Dat, key: &str) -> Outcome<Option<&'a Dat>> {
    d.map_get(&dat!(key))
}

fn list(d: &Dat) -> Outcome<&Vec<Dat>> {
    match d {
        Dat::List(v) => Ok(v),
        other => Err(err!("Expected a list, found {:?}.", other.kind(); Test, Mismatch)),
    }
}

/// A JSON integer, exactly, whatever width the decoder gave it.
fn int(d: &Dat) -> Outcome<i128> {
    match d {
        Dat::U8(v)  => Ok(*v as i128),
        Dat::U16(v) => Ok(*v as i128),
        Dat::U32(v) => Ok(*v as i128),
        Dat::U64(v) => Ok(*v as i128),
        Dat::I8(v)  => Ok(*v as i128),
        Dat::I16(v) => Ok(*v as i128),
        Dat::I32(v) => Ok(*v as i128),
        Dat::I64(v) => Ok(*v as i128),
        Dat::U128(v) => Ok(*v as i128),
        Dat::I128(v) => Ok(*v),
        other => Err(err!("Expected an integer, found {:?}.", other; Test, Mismatch)),
    }
}

fn num(d: &Dat) -> Outcome<f64> {
    match d.get_float64() {
        Some(f) => Ok(f.0),
        None => Err(err!("Expected a number, found {:?}.", d; Test, Mismatch)),
    }
}

fn string(d: &Dat) -> Outcome<String> {
    match d {
        Dat::Str(s) => Ok(s.clone()),
        other => Err(err!("Expected a string, found {:?}.", other; Test, Mismatch)),
    }
}

// ---------------------------------------------------------------------------------------------
// The grid
// ---------------------------------------------------------------------------------------------

#[test]
fn test_tile_corners_agree_with_proj_00() -> Outcome<()> {
    // proj -I -f "%.12f" +proj=webmerc +R=1 of each tile's north-west and south-east corners,
    // as lng, lat.
    for (z, x, y, nw, se) in [
        (0u8, 0u32, 0u32, (-180.0, 85.051128779807), (180.0, -85.051128779807)),
        (15, 26929, 19456, (115.850830078125, -31.952162238025), (115.861816406250, -31.961483557269)),
        (12, 3768, 2457, (151.171875000000, -33.797408767572), (151.259765625000, -33.870415550942)),
        (7, 0, 127, (-180.0, -84.802473724335), (-177.1875, -85.051128779807)),
        (20, 1048575, 0, (179.999656677246, 85.051128779807), (180.0, 85.051099162384)),
    ] {
        let t = res!(TileId::new(z, x, y));
        for ((u, v), (lng, lat)) in [((0.0, 0.0), nw), ((1.0, 1.0), se)] {
            let (a, b) = t.point(u, v);
            let near = (a - lat).abs() < 1.0e-9 && (b - lng).abs() < 1.0e-9;
            req!(near, true, "{} at ({}, {}) is {}, {}; PROJ says {}, {}.", t, u, v, a, b, lat, lng);
        }
    }
    // proj -f "%.15f" +proj=webmerc +R=1, and the grid position that follows from it at z15.
    let n = (1u64 << 15) as f64;
    for (lat, lng, xm, ym) in [
        (-31.9535, 115.8571, 2.022087856812322, -0.589076140272944),
        (-33.8688, 151.2093, 2.639100144635862, -0.628898163728307),
        (0.0, 0.0, 0.0, 0.0),
    ] {
        let tx = (xm + std::f64::consts::PI) / std::f64::consts::TAU * n;
        let ty = (std::f64::consts::PI - ym) / std::f64::consts::TAU * n;
        let near = (tile::lng_to_tx(lng, 15) - tx).abs() < 1.0e-6 && (tile::lat_to_ty(lat, 15) - ty).abs() < 1.0e-6;
        req!(near, true, "{}, {} lands at {}, {}; PROJ says {}, {}.", lat, lng,
            tile::lng_to_tx(lng, 15), tile::lat_to_ty(lat, 15), tx, ty);
        let t = res!(TileId::at(lat, lng, 15));
        req!((t.x, t.y), (tx.floor() as u32, ty.floor() as u32));
    }
    // A pole lands on the edge row, and the antimeridian wraps.
    req!(res!(TileId::at(90.0, 0.0, 3)).y, 0);
    req!(res!(TileId::at(-90.0, 0.0, 3)).y, 7);
    req!(res!(TileId::at(0.0, 180.0, 3)).x, 0);
    req!(res!(TileId::at(0.0, -180.0, 3)).x, 0);
    req!(TileId::new(3, 8, 0).is_err(), true, "A tile off the grid was made.");
    Ok(())
}

#[test]
fn test_overzoom_finds_the_part_of_an_ancestor_01() -> Outcome<()> {
    let t = res!(TileId::new(17, 107718, 77826));
    let a = res!(t.ancestor(15));
    req!((a.z, a.x, a.y), (15, 26929, 19456));
    let (u, v, span) = res!(t.within(&a).ok_or_else(|| err!("Not within."; Test)));
    req!((u, v, span), (0.5, 0.5, 4.0));
    req!(t.within(&res!(TileId::new(15, 0, 0))).is_none(), true, "A stranger contained it.");
    let kids = res!(a.children());
    for k in kids.iter() {
        req!(res!(k.ancestor(15)), a);
    }
    Ok(())
}

#[test]
fn test_the_screen_is_covered_by_its_tiles_02() -> Outcome<()> {
    for (lat, lng, heading, m, z) in [
        (-31.9535, 115.8571, 0.0, 5.0, 15u8),
        (-31.9535, 115.8571, 30.0, 5.0, 15),
        (51.5, -0.12, 200.0, 40.0, 13),
        (0.5, 179.99, 0.0, 20_000.0, 3),
        (70.0, 20.0, 0.0, 800.0, 9),
    ] {
        let v = res!(Viewport::new(Projection::WebMercator, lat, lng, heading, m, 390.0, 700.0));
        let got = res!(v.tiles_covering(z, 10_000));
        let set: HashSet<TileId> = got.iter().cloned().collect();
        req!(set.len(), got.len(), "A tile was listed twice.");
        req!(got[0], res!(TileId::at(lat, lng, z)), "The first tile is not the centre's.");
        // Every pixel's tile is listed.
        for i in 0..=39 {
            for j in 0..=70 {
                let pt = Pt::new(390.0 * i as f64 / 39.0, 700.0 * j as f64 / 70.0);
                if let Some((a, b)) = v.inverse(pt) {
                    let t = res!(TileId::at(a, b, z));
                    req!(set.contains(&t), true, "{} under ({}, {}) is not listed.", t, pt.x, pt.y);
                }
            }
        }
        // And every listed tile reaches the screen.
        for t in &got {
            let (aff, _) = res!(v.tile_affine(t).ok_or_else(|| err!("No transform."; Test)));
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (u, w) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                let (x, y) = aff.apply(u, w);
                xs.push(x);
                ys.push(y);
            }
            let reach = xs.iter().cloned().fold(f64::MIN, f64::max) >= 0.0
                && xs.iter().cloned().fold(f64::MAX, f64::min) <= 390.0
                && ys.iter().cloned().fold(f64::MIN, f64::max) >= 0.0
                && ys.iter().cloned().fold(f64::MAX, f64::min) <= 700.0;
            // A wrapped copy may lie a world away from the one the transform picks.
            let wraps = v.bounding_cap().1 >= std::f64::consts::PI;
            req!(reach || wraps, true, "{} is listed but off the screen.", t);
        }
    }
    // The globe: every pixel's tile is listed.
    for (lat, lng, m, z) in [(-31.9535, 115.8571, 50.0, 12u8), (60.0, -150.0, 20_000.0, 4),
        (-89.0, 0.0, 5_000.0, 6)]
    {
        let v = res!(Viewport::new(Projection::Orthographic, lat, lng, 25.0, m, 390.0, 700.0));
        let got = res!(v.tiles_covering(z, 20_000));
        let set: HashSet<TileId> = got.iter().cloned().collect();
        for i in 0..=39 {
            for j in 0..=70 {
                let pt = Pt::new(390.0 * i as f64 / 39.0, 700.0 * j as f64 / 70.0);
                if let Some((a, b)) = v.inverse(pt) {
                    if a.abs() > 85.05 {
                        continue;
                    }
                    let t = res!(TileId::at(a, b, z));
                    req!(set.contains(&t), true, "Globe: {} under ({}, {}) is not listed.", t, pt.x, pt.y);
                }
            }
        }
    }
    // Too many tiles are refused.
    let v = res!(Viewport::new(Projection::WebMercator, 0.0, 0.0, 0.0, 20_000.0, 390.0, 700.0));
    req!(v.tiles_covering(15, 1000).is_err(), true, "A world of zoom-15 tiles was listed.");
    Ok(())
}

#[test]
fn test_a_tile_bitmap_lands_where_the_projection_does_03() -> Outcome<()> {
    // On the flat map exactly, at any heading.
    let v = res!(Viewport::new(Projection::WebMercator, -31.9535, 115.8571, 37.0, 3.0, 390.0, 700.0));
    for t in res!(v.tiles_covering(15, 1000)) {
        let (aff, err) = res!(v.tile_affine(&t).ok_or_else(|| err!("No transform."; Test)));
        req!(err, 0.0);
        for (u, w) in [(0.0, 0.0), (1.0, 0.0), (0.3, 0.8), (1.0, 1.0)] {
            let (lat, lng) = t.point(u, w);
            let p = res!(v.forward(lat, lng).ok_or_else(|| err!("Lost."; Test)));
            let (x, y) = aff.apply(u, w);
            let near = (p.x - x).abs() < 1.0e-6 && (p.y - y).abs() < 1.0e-6;
            req!(near, true, "{} at ({}, {}) paints at {}, {}, the map at {}, {}.", t, u, w, x, y, p.x, p.y);
        }
    }
    // On the globe the error grows with the view, as the square of its size: under half a
    // pixel for a 12 km view at Perth, over a pixel at 100 km.
    let mut worst = Vec::new();
    for km in [12.0, 20.0, 100.0] {
        let m = km * 1000.0 / 700.0;
        let v = res!(Viewport::new(Projection::Orthographic, -31.9535, 115.8571, 0.0, m, 390.0, 700.0));
        let z = 15 - (km / 12.0f64).log2().ceil().max(0.0) as u8;
        let mut e: f64 = 0.0;
        for t in res!(v.tiles_covering(z, 1000)) {
            if let Some((_, err)) = v.tile_affine(&t) {
                // Only the part of a tile on the screen matters.
                let (aff, _) = res!(v.tile_affine(&t).ok_or_else(|| err!("No transform."; Test)));
                let (x, y) = aff.apply(0.5, 0.5);
                if x >= 0.0 && x <= 390.0 && y >= 0.0 && y <= 700.0 {
                    e = e.max(err);
                }
            }
        }
        worst.push(e);
    }
    println!("globe against map at Perth: 12 km {:.3} px, 20 km {:.3} px, 100 km {:.3} px",
        worst[0], worst[1], worst[2]);
    req!((worst[0] < 0.5), true, "A 12 km globe view strays {} px.", worst[0]);
    let grows = worst[0] < worst[1] && worst[1] < worst[2];
    req!(grows, true, "The error does not grow with the view: {:?}.", worst);
    req!((worst[2] > 1.0), true, "A 100 km globe view strays only {} px.", worst[2]);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// PMTiles
// ---------------------------------------------------------------------------------------------

/// The byte ranges the reference reader read, served back; any other range is refused, so
/// the reader under test cannot have found a tile the reference did not.
struct Sparse {
    ranges: Vec<(u64, Vec<u8>)>,
}

impl Sparse {
    fn load(rel: &str) -> Outcome<Self> {
        let b = res!(std::fs::read(data(rel)), File, Read);
        let mut ranges = Vec::new();
        let mut i = 0usize;
        while i + 12 <= b.len() {
            let mut o = [0u8; 8];
            o.copy_from_slice(&b[i..i + 8]);
            let n = u32::from_le_bytes([b[i + 8], b[i + 9], b[i + 10], b[i + 11]]) as usize;
            ranges.push((u64::from_le_bytes(o), b[i + 12..i + 12 + n].to_vec()));
            i += 12 + n;
        }
        Ok(Self { ranges })
    }
}

impl RangeSource for Sparse {
    fn read(&self, offset: u64, len: u64) -> Outcome<Vec<u8>> {
        for (o, b) in &self.ranges {
            if offset >= *o && offset + len <= *o + b.len() as u64 {
                let a = (offset - o) as usize;
                return Ok(b[a..a + len as usize].to_vec());
            }
        }
        Err(err!("Bytes {}+{} were never read by the reference reader.", offset, len; Test, Missing))
    }
}

fn fnv1a64(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    fmt!("{:016x}", h)
}

#[test]
fn test_pmtiles_reads_the_planet_as_the_reference_does_04() -> Outcome<()> {
    let expect = res!(json_file("protomaps/expect.json"));
    let head = res!(res!(get(&expect, "header")).ok_or_else(|| err!("No header."; Test)));
    let a = res!(Archive::open(res!(Sparse::load("protomaps/ocean_ranges.bin"))));
    let h = a.header().clone();
    for (key, got) in [
        ("rootDirectoryOffset", h.root_offset), ("rootDirectoryLength", h.root_length),
        ("jsonMetadataOffset", h.metadata_offset), ("jsonMetadataLength", h.metadata_length),
        ("leafDirectoryOffset", h.leaf_offset), ("leafDirectoryLength", h.leaf_length),
        ("tileDataOffset", h.data_offset), ("tileDataLength", h.data_length),
        ("numAddressedTiles", h.addressed_tiles), ("numTileEntries", h.tile_entries),
        ("numTileContents", h.tile_contents),
        ("minZoom", h.min_zoom as u64), ("maxZoom", h.max_zoom as u64),
    ] {
        let want = res!(int(res!(res!(get(head, key)).ok_or_else(|| err!("No {}.", key; Test)))));
        req!(got as i128, want, "Header {} is {}, the reference says {}.", key, got, want);
    }
    req!(h.internal_compression, Compression::Gzip);
    req!(h.tile_compression, Compression::Gzip);
    req!(h.tile_type, TileType::Mvt);
    req!(h.clustered, true);
    let b = h.bounds();
    for (key, got) in [("minLon", b[0]), ("minLat", b[1]), ("maxLon", b[2]), ("maxLat", b[3])] {
        let want = res!(num(res!(res!(get(head, key)).ok_or_else(|| err!("No {}.", key; Test)))));
        req!(got, want, "Header {} is {}, the reference says {}.", key, got, want);
    }
    // Fifty tiles' ids, and the open-ocean ones in full: both are found, share one stored
    // content through the run-length entries, and decompress to the reference's bytes.
    let tiles = res!(list(res!(res!(get(&expect, "tiles")).ok_or_else(|| err!("No tiles."; Test)))));
    req!(tiles.len(), 50);
    let mut fetched = 0;
    for t in tiles {
        let z = res!(int(res!(res!(get(t, "z")).ok_or_else(|| err!("z"; Test))))) as u8;
        let x = res!(int(res!(res!(get(t, "x")).ok_or_else(|| err!("x"; Test))))) as u32;
        let y = res!(int(res!(res!(get(t, "y")).ok_or_else(|| err!("y"; Test))))) as u32;
        let id = res!(int(res!(res!(get(t, "id")).ok_or_else(|| err!("id"; Test)))));
        req!(res!(pmtiles::zxy_to_id(z, x, y)) as i128, id, "{}/{}/{} has the wrong id.", z, x, y);
        req!(res!(pmtiles::id_to_zxy(id as u64)), (z, x, y));
        let len = res!(int(res!(res!(get(t, "len")).ok_or_else(|| err!("len"; Test)))));
        if len >= 0 && len < 200 {
            let tile = res!(res!(a.tile_decoded(z, x, y)).ok_or_else(|| err!("{}/{}/{} is missing.", z, x, y; Test)));
            req!(tile.len() as i128, len);
            let fnv = res!(string(res!(res!(get(t, "fnv")).ok_or_else(|| err!("fnv"; Test)))));
            req!(fnv1a64(&tile), fnv, "{}/{}/{} decompressed to other bytes.", z, x, y);
            fetched += 1;
        }
    }
    req!(fetched, 2, "Expected the two ocean tiles.");
    // Ids the other way, at the corners of grids and deep zooms.
    let ids = res!(list(res!(res!(get(&expect, "ids")).ok_or_else(|| err!("No ids."; Test)))));
    for t in ids {
        let z = res!(int(res!(res!(get(t, "z")).ok_or_else(|| err!("z"; Test))))) as u8;
        let x = res!(int(res!(res!(get(t, "x")).ok_or_else(|| err!("x"; Test))))) as u32;
        let y = res!(int(res!(res!(get(t, "y")).ok_or_else(|| err!("y"; Test))))) as u32;
        let id = res!(int(res!(res!(get(t, "id")).ok_or_else(|| err!("id"; Test)))));
        req!(res!(pmtiles::zxy_to_id(z, x, y)) as i128, id, "{}/{}/{} has the wrong id.", z, x, y);
        req!(res!(pmtiles::id_to_zxy(id as u64)), (z, x, y), "Id {} names the wrong tile.", id);
    }
    // A tile beyond the archive's zooms is simply absent.
    req!(res!(a.tile(16, 0, 0)), None::<Vec<u8>>);
    Ok(())
}

#[test]
fn test_pmtiles_reads_an_archive_the_reference_writer_wrote_09() -> Outcome<()> {
    let a = res!(Archive::open(res!(FileSource::open(data("protomaps/sample.pmtiles")))));
    let h = a.header().clone();
    req!(h.tile_type, TileType::Mvt);
    req!(h.tile_compression, Compression::Gzip);
    req!(h.internal_compression, Compression::Gzip);
    req!((h.min_zoom, h.max_zoom), (12, 15));
    req!(h.bounds_e7(), [1_156_000_000, -326_000_000, 1_163_000_000, -316_000_000]);
    req!((h.addressed_tiles, h.tile_entries, h.tile_contents), (5, 3, 3));
    for name in ["15_26930_19457", "13_6729_4865"] {
        let parts: Vec<u32> = name.split('_').map(|p| p.parse::<u32>().unwrap_or(0)).collect();
        let want = res!(std::fs::read(data(&fmt!("protomaps/{}.mvt", name))), File, Read);
        let got = res!(a.tile_decoded(parts[0] as u8, parts[1], parts[2]));
        req!(got, Some(want.clone()), "{} differs from its tile.", name);
    }
    // The run of three: consecutive Hilbert ids sharing one stored content.
    let ocean = res!(res!(a.tile(12, 2958, 2545)).ok_or_else(|| err!("No ocean."; Test)));
    for (x, y) in [(2957, 2545), (2957, 2544)] {
        req!(res!(a.tile(12, x, y)), Some(ocean.clone()), "12/{}/{} is not in the run.", x, y);
    }
    req!(res!(a.tile(12, 2956, 2544)), None::<Vec<u8>>);
    req!(res!(a.tile(11, 0, 0)), None::<Vec<u8>>);
    let meta = res!(a.metadata());
    req!(meta.contains("OpenStreetMap"), true, "The metadata is {:?}.", meta);
    Ok(())
}

/// Appends a varint.
fn put(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn directory(entries: &[(u64, u64, u32, u32)]) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, entries.len() as u64);
    let mut last = 0;
    for e in entries {
        put(&mut b, e.0 - last);
        last = e.0;
    }
    for e in entries {
        put(&mut b, e.3 as u64);
    }
    for e in entries {
        put(&mut b, e.2 as u64);
    }
    for (i, e) in entries.iter().enumerate() {
        // "Straight after the last" is written as nought.
        let follows = i > 0 && entries[i - 1].1 + entries[i - 1].2 as u64 == e.1;
        put(&mut b, if follows { 0 } else { e.1 + 1 });
    }
    b
}

#[test]
fn test_pmtiles_reads_a_small_archive_from_a_file_05() -> Outcome<()> {
    // Built here to the specification: tiles 0/0/0 and a run of the four zoom-1 tiles sharing
    // one content, in the root; zoom 2 behind a leaf directory, one tile of it stored.
    let t0 = b"world".to_vec();
    let t1 = b"four of a kind".to_vec();
    let t2 = b"one at zoom two".to_vec();
    let id2 = res!(pmtiles::zxy_to_id(2, 1, 2));
    let leaf = directory(&[(id2, (t0.len() + t1.len()) as u64, t2.len() as u32, 1)]);
    let root = directory(&[
        (0, 0, t0.len() as u32, 1),
        (1, t0.len() as u64, t1.len() as u32, 4),
        (5, 0, leaf.len() as u32, 0),
    ]);
    let root_off = 127u64;
    let leaf_off = root_off + root.len() as u64;
    let data_off = leaf_off + leaf.len() as u64;
    let mut h = vec![0u8; 127];
    h[..7].copy_from_slice(b"PMTiles");
    h[7] = 3;
    for (at, v) in [(8, root_off), (16, root.len() as u64), (24, 0), (32, 0), (40, leaf_off),
        (48, leaf.len() as u64), (56, data_off), (64, (t0.len() + t1.len() + t2.len()) as u64),
        (72, 21), (80, 3), (88, 3)]
    {
        h[at..at + 8].copy_from_slice(&(v as u64).to_le_bytes());
    }
    h[97] = 1; // internal compression: none
    h[98] = 1; // tile compression: none
    h[99] = 1; // mvt
    h[100] = 0;
    h[101] = 2;
    let mut file = h;
    file.extend_from_slice(&root);
    file.extend_from_slice(&leaf);
    file.extend_from_slice(&t0);
    file.extend_from_slice(&t1);
    file.extend_from_slice(&t2);
    let dir = std::env::temp_dir().join(fmt!("fe2o3_geom_pmtiles_{}", std::process::id()));
    res!(std::fs::create_dir_all(&dir), File, Write);
    let path = dir.join("small.pmtiles");
    res!(std::fs::write(&path, &file), File, Write);
    let a = res!(Archive::open(res!(FileSource::open(&path))));
    req!(res!(a.tile(0, 0, 0)), Some(t0.clone()));
    for (x, y) in [(0, 0), (0, 1), (1, 1), (1, 0)] {
        req!(res!(a.tile(1, x, y)), Some(t1.clone()), "1/{}/{} was not the shared content.", x, y);
    }
    req!(res!(a.tile(2, 1, 2)), Some(t2.clone()));
    req!(res!(a.tile(2, 0, 0)), None::<Vec<u8>>);
    req!(res!(a.tile(3, 0, 0)), None::<Vec<u8>>);
    // The same bytes from memory.
    let m = res!(Archive::open(&file[..]));
    req!(res!(m.tile(2, 1, 2)), Some(t2.clone()));
    let _ = std::fs::remove_dir_all(&dir);
    // Not an archive, and a directory whose count lies.
    req!(pmtiles::Header::parse(&file[1..]).is_err(), true, "A shifted header was read.");
    req!(pmtiles::decode_directory(&[0xff, 0xff, 0x03]).is_err(), true, "A lying count was believed.");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Mapbox Vector Tiles
// ---------------------------------------------------------------------------------------------

/// Does a decoded tile say what the fixture's JSON reading of its protocol buffer says?
fn same_as_fixture(t: &mvt::Tile, want: &Dat, name: &str) -> Outcome<()> {
    let layers = match res!(get(want, "layers")) {
        Some(l) => res!(list(l)).clone(),
        None => Vec::new(),
    };
    req!(t.layers.len(), layers.len(), "{}: layer count.", name);
    for (l, w) in t.layers.iter().zip(layers.iter()) {
        req!(l.name.clone(), res!(string(res!(res!(get(w, "name")).ok_or_else(|| err!("name"; Test))))));
        if let Some(v) = res!(get(w, "version")) {
            req!(l.version as i128, res!(int(v)), "{}: version.", name);
        }
        let extent = match res!(get(w, "extent")) { Some(e) => res!(int(e)), None => 4096 };
        req!(l.extent as i128, extent, "{}: extent.", name);
        let keys: Vec<String> = match res!(get(w, "keys")) {
            Some(k) => { let mut o = Vec::new(); for s in res!(list(k)) { o.push(res!(string(s))); } o },
            None => Vec::new(),
        };
        req!(l.keys.clone(), keys, "{}: keys.", name);
        let values = match res!(get(w, "values")) { Some(v) => res!(list(v)).clone(), None => Vec::new() };
        req!(l.values.len(), values.len(), "{}: value count.", name);
        for (v, wv) in l.values.iter().zip(values.iter()) {
            let ok = match v {
                // The fixtures are generated from this JSON, and 076's generator wrote a number
                // into a string field, which went into the tile as its decimal text.
                Value::Str(s)   => res!(get(wv, "string_value")).map(|d| string(d).ok() == Some(s.clone())
                    || int(d).ok().map(|i| i.to_string()) == Some(s.clone())),
                Value::Float(f) => res!(get(wv, "float_value")).map(|d| num(d).ok().map(|x| x as f32) == Some(*f)),
                Value::Double(f)=> res!(get(wv, "double_value")).map(|d| num(d).ok() == Some(*f)),
                Value::Int(i)   => res!(get(wv, "int_value")).map(|d| int(d).ok() == Some(*i as i128)),
                Value::Uint(u)  => res!(get(wv, "uint_value")).map(|d| int(d).ok() == Some(*u as i128)),
                Value::Sint(i)  => res!(get(wv, "sint_value")).map(|d| int(d).ok() == Some(*i as i128)),
                Value::Bool(b)  => res!(get(wv, "bool_value")).map(|d| matches!(d, Dat::Bool(x) if x == b)),
            };
            req!(ok, Some(true), "{}: value {:?} against {:?}.", name, v, wv);
        }
        let feats = match res!(get(w, "features")) { Some(f) => res!(list(f)).clone(), None => Vec::new() };
        req!(l.features.len(), feats.len(), "{}: feature count.", name);
        for (f, wf) in l.features.iter().zip(feats.iter()) {
            let id = match res!(get(wf, "id")) { Some(d) => Some(res!(int(d)) as u64), None => None };
            // A proto2 id of nought is indistinguishable from none.
            req!(f.id.filter(|v| *v != 0), id.filter(|v| *v != 0), "{}: id.", name);
            let ty = match res!(get(wf, "type")) { Some(d) => res!(int(d)), None => 0 };
            let ty = if ty > 3 { 0 } else { ty };
            req!(f.kind.code() as i128, ty, "{}: type.", name);
            let tags: Vec<i128> = match res!(get(wf, "tags")) {
                Some(d) => { let mut o = Vec::new(); for x in res!(list(d)) { o.push(res!(int(x))); } o },
                None => Vec::new(),
            };
            req!(f.tags.iter().map(|v| *v as i128).collect::<Vec<_>>(), tags, "{}: tags.", name);
            let geom: Vec<i128> = match res!(get(wf, "geometry")) {
                Some(d) => { let mut o = Vec::new(); for x in res!(list(d)) { o.push(res!(int(x))); } o },
                None => Vec::new(),
            };
            req!(f.geometry.iter().map(|v| *v as i128).collect::<Vec<_>>(), geom, "{}: geometry.", name);
        }
    }
    Ok(())
}

/// Decoding in full: the structure, then every feature's geometry and properties.
fn decode_fully(bytes: &[u8]) -> Outcome<mvt::Tile> {
    let t = res!(mvt::decode(bytes));
    for l in &t.layers {
        for f in &l.features {
            res!(f.geometry());
            res!(f.properties(l));
        }
    }
    Ok(t)
}

#[test]
fn test_the_specification_fixtures_decode_as_specified_06() -> Outcome<()> {
    let root = data("mvt-fixtures");
    let mut names: Vec<String> = Vec::new();
    for e in res!(std::fs::read_dir(&root), File, Read) {
        let e = res!(e, File, Read);
        if e.path().is_dir() {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    req!(names.len(), 74, "Expected the 74 fixtures of mvt-fixtures 4.0.0.");
    let (mut valid, mut fatal) = (0, 0);
    for name in &names {
        let info = res!(json_file(&fmt!("mvt-fixtures/{}/info.json", name)));
        let validity = res!(res!(get(&info, "validity")).ok_or_else(|| err!("validity"; Test)));
        let v2 = matches!(res!(get(validity, "v2")), Some(Dat::Bool(true)));
        let error = match res!(get(validity, "error")) { Some(d) => res!(string(d)), None => String::new() };
        let bytes = res!(std::fs::read(root.join(name).join("tile.mvt")), File, Read);
        if v2 {
            let t = res!(mvt::decode(&bytes));
            let want = res!(json_file(&fmt!("mvt-fixtures/{}/tile.json", name)));
            res!(same_as_fixture(&t, &want, name));
            // 057 is a MoveTo whose count asks for half a billion points and supplies one;
            // a count is never trusted, so its geometry is refused rather than allocated.
            if name != "057" {
                res!(decode_fully(&bytes));
            }
            valid += 1;
        } else if error == "fatal" {
            let refused = decode_fully(&bytes).is_err();
            req!(refused, true, "Fixture {} is fatally malformed and decoded.", name);
            fatal += 1;
        } else {
            // Recoverable: either answer will do, so long as it is an answer.
            let _ = decode_fully(&bytes);
        }
    }
    req!(valid, 46);
    let plenty = fatal >= 20;
    req!(plenty, true, "Only {} fatal fixtures.", fatal);
    Ok(())
}

#[test]
fn test_real_tiles_decode_as_the_reference_decoder_reads_them_07() -> Outcome<()> {
    for name in ["15_26930_19457", "13_6729_4865"] {
        let bytes = res!(std::fs::read(data(&fmt!("protomaps/{}.mvt", name))), File, Read);
        let gz = res!(std::fs::read(data(&fmt!("protomaps/{}.json.gz", name))), File, Read);
        let text = match String::from_utf8(res!(pmtiles::decompress(&gz, Compression::Gzip))) {
            Ok(s) => s,
            Err(_) => return Err(err!("The reference dump is not text."; Test)),
        };
        let want = res!(json(&text));
        let t = res!(mvt::decode(&bytes));
        let layers = res!(list(&want));
        req!(t.layers.len(), layers.len(), "{}: layer count.", name);
        let mut features = 0;
        for (l, w) in t.layers.iter().zip(layers.iter()) {
            req!(l.name.clone(), res!(string(res!(res!(get(w, "name")).ok_or_else(|| err!("name"; Test))))));
            req!(l.extent as i128, res!(int(res!(res!(get(w, "extent")).ok_or_else(|| err!("extent"; Test))))));
            let wf = res!(list(res!(res!(get(w, "features")).ok_or_else(|| err!("features"; Test)))));
            req!(l.features.len(), wf.len(), "{} {}: feature count.", name, l.name);
            for (f, w) in l.features.iter().zip(wf.iter()) {
                features += 1;
                match res!(get(w, "id")) {
                    Some(Dat::Empty) | Some(Dat::Opt(_)) | None => req!(f.id, None::<u64>),
                    Some(d) => req!(f.id.map(|v| v as i128), Some(res!(int(d))), "{} {}: id.", name, l.name),
                }
                req!(f.kind.code() as i128, res!(int(res!(res!(get(w, "type")).ok_or_else(|| err!("type"; Test))))));
                // Properties, as a map: a JavaScript object orders integer-like keys first.
                let props = res!(f.properties(l));
                let wp = res!(res!(get(w, "props")).ok_or_else(|| err!("props"; Test)));
                let mut count = 0;
                for (k, v) in &props {
                    count += 1;
                    let d = res!(res!(get(wp, k)).ok_or_else(|| err!("{} {}: no {} in the reference.", name, l.name, k; Test)));
                    let ok = match (v, d) {
                        (Value::Str(s), Dat::Str(r)) => s == r,
                        (Value::Bool(b), Dat::Bool(r)) => b == r,
                        (v, d) => match (v.as_f64(), d.get_float64()) {
                            (Some(a), Some(b)) => a == b.0,
                            _ => false,
                        },
                    };
                    req!(ok, true, "{} {}: {} is {:?}, the reference {:?}.", name, l.name, k, v, d);
                }
                let wlen = match wp { Dat::Map(m) => m.len(), Dat::OrdMap(m) => m.len(), _ => 0 };
                req!(count, wlen, "{} {}: property count.", name, l.name);
                // Geometry, point for point.
                let g = res!(f.geometry());
                let wg = res!(list(res!(res!(get(w, "geom")).ok_or_else(|| err!("geom"; Test)))));
                req!(g.len(), wg.len(), "{} {}: ring count.", name, l.name);
                for (r, wr) in g.iter().zip(wg.iter()) {
                    let wr = res!(list(wr));
                    req!(r.len(), wr.len(), "{} {}: point count.", name, l.name);
                    for (p, wp) in r.iter().zip(wr.iter()) {
                        let wp = res!(list(wp));
                        req!((p.0 as i128, p.1 as i128), (res!(int(&wp[0])), res!(int(&wp[1]))),
                            "{} {}: a point differs.", name, l.name);
                    }
                }
            }
        }
        let plenty = features > 20;
        req!(plenty, true, "{} had only {} features.", name, features);
    }
    Ok(())
}

#[test]
fn test_features_sort_into_style_classes_08() -> Outcome<()> {
    let bytes = res!(std::fs::read(data("protomaps/13_6729_4865.mvt")), File, Read);
    let t = res!(mvt::decode(&bytes));
    let names: Vec<&str> = t.layers.iter().map(|l| l.name.as_str()).collect();
    req!(names.contains(&"roads"), true, "No roads layer in {:?}.", names);
    let rules = vec![
        StyleRule { layer: "water".into(), key: "kind".into(), values: vec![], min_zoom: 0, class: 1 },
        StyleRule { layer: "roads".into(), key: "kind".into(), values: vec!["highway".into(), "major_road".into()], min_zoom: 0, class: 2 },
        StyleRule { layer: "roads".into(), key: "kind".into(), values: vec![], min_zoom: 14, class: 3 },
    ];
    let at13 = mvt::classify(&t, &rules, 13, Some("min_zoom"));
    let at14 = mvt::classify(&t, &rules, 14, Some("min_zoom"));
    // Rule order is paint order, and the catch-all minor-road rule only opens at 14.
    let order_ok = at13.windows(2).all(|p| p[0].0 <= p[1].0);
    req!(order_ok, true, "Classes out of rule order.");
    req!(at13.iter().any(|h| h.0 == 3), false, "Class 3 drawn below its zoom.");
    let more = at14.len() >= at13.len();
    req!(more, true, "Zooming in drew less.");
    for (class, li, fi) in &at14 {
        let l = &t.layers[*li];
        let f = &l.features[*fi];
        if *class == 2 {
            let k = l.property(f, "kind").and_then(|v| v.as_str()).unwrap_or("");
            req!((k == "highway" || k == "major_road"), true, "A {} road in class 2.", k);
        }
        if let Some(mz) = l.property(f, "min_zoom").and_then(|v| v.as_f64()) {
            req!((mz <= 14.0), true, "A feature of min_zoom {} drawn at 14.", mz);
        }
    }
    // Geometry types are what the layers imply.
    for l in &t.layers {
        for f in &l.features {
            if l.name == "roads" {
                req!(f.kind, GeomKind::LineString);
            }
        }
    }
    Ok(())
}
