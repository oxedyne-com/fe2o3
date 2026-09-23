//! Reads tiles from a remote PMTiles archive by HTTP range and checks them against the
//! reference JavaScript readers' output.
//!
//! ```text
//! cargo run --release -p oxedyne_fe2o3_net --example pmtiles_check -- \
//!     --expect fe2o3_geom/tests/data/protomaps/expect.json [--dumps DIR] [--url URL]
//! ```
//!
//! `--expect` is the file the capture wrote: the archive's URL, and for each tile its length
//! and FNV-1a-64 once decompressed, as `pmtiles` 4.5.0 read it.  `--dumps` is a directory of
//! `protomaps_<z>_<x>_<y>.json.gz`, `@mapbox/vector-tile` 3.0.0's reading of each tile; with it
//! every tile is also decoded here and compared feature by feature.  A few megabytes are read.

use oxedyne_fe2o3_geom::{
    mvt::{
        self,
        Value,
    },
    tile::pmtiles::{
        self,
        Archive,
        Compression,
    },
};
use oxedyne_fe2o3_net::{
    http::range_source::HttpRangeSource,
    tls::default_client_config,
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
    sync::Arc,
};

fn json(text: &str) -> Outcome<Dat> {
    let cfg = DecoderConfig::<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>::json(None);
    Dat::decode_string_with_config(text, &cfg)
}

fn field<'a>(d: &'a Dat, key: &str) -> Outcome<&'a Dat> {
    res!(d.map_get(&dat!(key))).ok_or_else(|| err!("No {} in {:?}.", key, d.kind(); Input, Missing))
}

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
        other => Err(err!("Expected an integer, found {:?}.", other; Input, Mismatch)),
    }
}

fn list(d: &Dat) -> Outcome<&Vec<Dat>> {
    match d {
        Dat::List(v) => Ok(v),
        other => Err(err!("Expected a list, found {:?}.", other.kind(); Input, Mismatch)),
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

/// Compares a decoded tile with the reference decoder's reading, returning the feature count.
fn compare(t: &mvt::Tile, want: &Dat, name: &str) -> Outcome<usize> {
    let layers = res!(list(want));
    if t.layers.len() != layers.len() {
        return Err(err!("{}: {} layers, the reference {}.", name, t.layers.len(), layers.len(); Mismatch));
    }
    let mut n = 0;
    for (l, w) in t.layers.iter().zip(layers.iter()) {
        let wf = res!(list(res!(field(w, "features"))));
        if l.features.len() != wf.len() {
            return Err(err!("{} {}: {} features, the reference {}.", name, l.name, l.features.len(),
                wf.len(); Mismatch));
        }
        for (f, w) in l.features.iter().zip(wf.iter()) {
            n += 1;
            if f.kind.code() as i128 != res!(int(res!(field(w, "type")))) {
                return Err(err!("{} {}: a type differs.", name, l.name; Mismatch));
            }
            let wp = res!(field(w, "props"));
            for (k, v) in res!(f.properties(l)) {
                let d = res!(field(wp, k));
                let ok = match (v, d) {
                    (Value::Str(s), Dat::Str(r)) => s == r,
                    (Value::Bool(b), Dat::Bool(r)) => b == r,
                    (v, d) => match (v.as_f64(), d.get_float64()) {
                        (Some(a), Some(b)) => a == b.0,
                        _ => false,
                    },
                };
                if !ok {
                    return Err(err!("{} {}: {} is {:?}, the reference {:?}.", name, l.name, k, v, d; Mismatch));
                }
            }
            let g = res!(f.geometry());
            let wg = res!(list(res!(field(w, "geom"))));
            if g.len() != wg.len() {
                return Err(err!("{} {}: ring counts differ.", name, l.name; Mismatch));
            }
            for (r, wr) in g.iter().zip(wg.iter()) {
                let wr = res!(list(wr));
                if r.len() != wr.len() {
                    return Err(err!("{} {}: point counts differ.", name, l.name; Mismatch));
                }
                for (p, q) in r.iter().zip(wr.iter()) {
                    let q = res!(list(q));
                    if (p.0 as i128, p.1 as i128) != (res!(int(&q[0])), res!(int(&q[1]))) {
                        return Err(err!("{} {}: a point differs.", name, l.name; Mismatch));
                    }
                }
            }
        }
    }
    Ok(n)
}

fn main() -> Outcome<()> {
    log_set_level!("warn");
    let mut url: Option<String> = None;
    let mut expect_path: Option<String> = None;
    let mut dumps: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let v = res!(it.next().ok_or_else(|| err!("{} wants a value.", a; Input, Missing)));
        match a.as_str() {
            "--url"     => url = Some(v),
            "--expect"  => expect_path = Some(v),
            "--dumps"   => dumps = Some(v),
            other       => return Err(err!("Unknown argument {:?}.", other; Input, Invalid)),
        }
    }
    let expect_path = res!(expect_path.ok_or_else(|| err!("--expect is needed."; Input, Missing)));
    let expect = res!(json(&res!(std::fs::read_to_string(&expect_path), File, Read)));
    let url = match url {
        Some(u) => u,
        None => match res!(field(&expect, "url")) {
            Dat::Str(s) => s.clone(),
            _ => return Err(err!("The expectations name no URL."; Input, Missing)),
        },
    };
    let tls = Arc::new(res!(default_client_config()));
    let archive = res!(Archive::open(res!(HttpRangeSource::new(&url, Some(tls)))));
    println!("{}: zooms {} to {}, {} tile contents", url, archive.header().min_zoom,
        archive.header().max_zoom, archive.header().tile_contents);
    let tiles = res!(list(res!(field(&expect, "tiles"))));
    let (mut same, mut decoded, mut features) = (0, 0, 0);
    for t in tiles {
        let (z, x, y) = (res!(int(res!(field(t, "z")))) as u8, res!(int(res!(field(t, "x")))) as u32,
            res!(int(res!(field(t, "y")))) as u32);
        let name = fmt!("{}/{}/{}", z, x, y);
        let tile = res!(res!(archive.tile_decoded(z, x, y)).ok_or_else(|| err!("{} is missing.", name; Mismatch)));
        let len = res!(int(res!(field(t, "len"))));
        let fnv = match res!(field(t, "fnv")) { Dat::Str(s) => s.clone(), _ => String::new() };
        if tile.len() as i128 != len || fnv1a64(&tile) != fnv {
            return Err(err!("{}: {} bytes {}, the reference {} bytes {}.", name, tile.len(),
                fnv1a64(&tile), len, fnv; Mismatch));
        }
        same += 1;
        if let Some(dir) = &dumps {
            let path = fmt!("{}/protomaps_{}_{}_{}.json.gz", dir, z, x, y);
            if let Ok(gz) = std::fs::read(&path) {
                let text = match String::from_utf8(res!(pmtiles::decompress(&gz, Compression::Gzip))) {
                    Ok(s) => s,
                    Err(_) => return Err(err!("{} is not text.", path; Input, UTF8)),
                };
                let want = res!(json(&text));
                let t = res!(mvt::decode(&tile));
                features += res!(compare(&t, &want, &name));
                decoded += 1;
            }
        }
    }
    println!("{} of {} tiles byte-identical to the pmtiles reader's", same, tiles.len());
    if dumps.is_some() {
        println!("{} tiles, {} features decoded identically to @mapbox/vector-tile", decoded, features);
    }
    Ok(())
}
