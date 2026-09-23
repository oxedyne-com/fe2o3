//! Mapbox Vector Tiles: the protocol buffer a street map arrives in, decoded.
//!
//! A tile is layers; a layer is features, a table of keys and a table of values; a feature is
//! a geometry type, pairs of indices into those tables, and a run of geometry commands in tile
//! coordinates, conventionally 0 to 4096 with `y` downward.  The format is version 2.1 of the
//! specification at <https://github.com/mapbox/vector-tile-spec>, and version 1 is read too.
//!
//! [`decode`] reads the message structure and checks it; a feature's geometry and properties
//! are resolved when asked for ([`Feature::geometry`], [`Feature::properties`]), because a
//! painter at a given zoom draws a fraction of what a tile carries.  The decoder is held to the
//! specification's own fixtures (`@mapbox/mvt-fixtures`) and to real Protomaps tiles decoded by
//! the reference JavaScript decoder.
//!
//! What counts as malformed follows those fixtures: a field of the wrong wire type, a value of
//! an unknown kind or of none, a layer with no name or version or of a version other than 1 or
//! 2, a tag pointing past its table, and a geometry command whose count runs past the
//! parameters that follow it are all refused, and a count is never trusted to size an
//! allocation.

use oxedyne_fe2o3_core::prelude::*;

// ---------------------------------------------------------------------------------------------
// Protocol buffer wire format
// ---------------------------------------------------------------------------------------------

const WIRE_VARINT:  u8 = 0;
const WIRE_I64:     u8 = 1;
const WIRE_LEN:     u8 = 2;
const WIRE_I32:     u8 = 5;

/// A cursor over protocol buffer bytes.
struct Pb<'a> {
    buf:    &'a [u8],
    pos:    usize,
}

impl<'a> Pb<'a> {
    fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }

    fn done(&self) -> bool { self.pos >= self.buf.len() }

    fn varint(&mut self) -> Outcome<u64> {
        let mut v: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = match self.buf.get(self.pos) {
                Some(b) => *b,
                None => return Err(err!("A varint runs off the end of the buffer at byte {}.",
                    self.pos; Invalid, Input, Decode)),
            };
            self.pos += 1;
            if shift >= 64 {
                return Err(err!("A varint at byte {} is longer than ten bytes.", self.pos;
                    Invalid, Input, Decode));
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b < 0x80 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    /// A field's number and wire type.
    fn key(&mut self) -> Outcome<(u32, u8)> {
        let k = res!(self.varint());
        Ok(((k >> 3) as u32, (k & 7) as u8))
    }

    fn bytes(&mut self) -> Outcome<&'a [u8]> {
        let n = res!(self.varint()) as usize;
        let end = match self.pos.checked_add(n) {
            Some(e) if e <= self.buf.len() => e,
            _ => return Err(err!("A field of {} bytes at byte {} runs past the {} there are.",
                n, self.pos, self.buf.len(); Invalid, Input, Decode)),
        };
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn fixed(&mut self, n: usize) -> Outcome<&'a [u8]> {
        match self.buf.get(self.pos..self.pos + n) {
            Some(b) => {
                self.pos += n;
                Ok(b)
            },
            None => Err(err!("A {}-byte field at byte {} runs off the end.", n, self.pos;
                Invalid, Input, Decode)),
        }
    }

    fn skip(&mut self, wire: u8) -> Outcome<()> {
        match wire {
            WIRE_VARINT => { res!(self.varint()); },
            WIRE_I64    => { res!(self.fixed(8)); },
            WIRE_LEN    => { res!(self.bytes()); },
            WIRE_I32    => { res!(self.fixed(4)); },
            _ => return Err(err!("Wire type {} at byte {} is not one this reads.", wire, self.pos;
                Invalid, Input, Decode)),
        }
        Ok(())
    }

    /// A packed repeated `uint32`, or one element of an unpacked one.
    fn u32s(&mut self, wire: u8, out: &mut Vec<u32>, what: &str) -> Outcome<()> {
        match wire {
            WIRE_LEN => {
                let mut inner = Pb::new(res!(self.bytes()));
                while !inner.done() {
                    out.push(res!(u32_of(res!(inner.varint()), what)));
                }
            },
            WIRE_VARINT => out.push(res!(u32_of(res!(self.varint()), what))),
            _ => return Err(err!("{} has wire type {}, not a packed integer.", what, wire;
                Invalid, Input, Decode)),
        }
        Ok(())
    }
}

fn u32_of(v: u64, what: &str) -> Outcome<u32> {
    if v > u32::MAX as u64 {
        return Err(err!("{} holds {}, more than 32 bits.", what, v; Invalid, Input, Decode));
    }
    Ok(v as u32)
}

fn want(wire: u8, expected: u8, what: &str) -> Outcome<()> {
    if wire != expected {
        return Err(err!("{} has wire type {}, not {}.", what, wire, expected; Invalid, Input, Decode));
    }
    Ok(())
}

fn zigzag(v: u64) -> i64 { ((v >> 1) as i64) ^ -((v & 1) as i64) }

fn text(b: &[u8], what: &str) -> Outcome<String> {
    match std::str::from_utf8(b) {
        Ok(s)   => Ok(s.to_string()),
        Err(_)  => Err(err!("{} is not UTF-8.", what; Invalid, Input, Decode, UTF8)),
    }
}

// ---------------------------------------------------------------------------------------------
// The tile
// ---------------------------------------------------------------------------------------------

/// A feature's geometry type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomKind {
    Unknown,
    Point,
    LineString,
    Polygon,
}

impl GeomKind {
    /// The number the protocol buffer carries: 0 to 3.
    pub fn code(self) -> u32 {
        match self {
            Self::Unknown       => 0,
            Self::Point         => 1,
            Self::LineString    => 2,
            Self::Polygon       => 3,
        }
    }
}

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Float(f32),
    Double(f64),
    Int(i64),
    Uint(u64),
    Sint(i64),
    Bool(bool),
}

impl Value {
    /// The value as a number, for any numeric kind and for a boolean as 0 or 1.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Str(_)    => None,
            Self::Float(v)  => Some(*v as f64),
            Self::Double(v) => Some(*v),
            Self::Int(v)    => Some(*v as f64),
            Self::Uint(v)   => Some(*v as f64),
            Self::Sint(v)   => Some(*v as f64),
            Self::Bool(v)   => Some(if *v { 1.0 } else { 0.0 }),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s)    => Some(s),
            _               => None,
        }
    }
}

/// One feature, its geometry still as commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub id:         Option<u64>,
    pub kind:       GeomKind,
    pub tags:       Vec<u32>,   // key index, value index, key index, ...
    pub geometry:   Vec<u32>,   // command integers and zig-zag parameters
}

/// One layer of a tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub version:    u32,
    pub name:       String,
    pub extent:     u32,        // tile coordinates across, 4096 by default
    pub keys:       Vec<String>,
    pub values:     Vec<Value>,
    pub features:   Vec<Feature>,
}

impl Layer {
    /// The value of one property of a feature of this layer, if it has it.
    pub fn property(&self, feature: &Feature, key: &str) -> Option<&Value> {
        for pair in feature.tags.chunks(2) {
            if pair.len() == 2 {
                if let (Some(k), Some(v)) = (self.keys.get(pair[0] as usize), self.values.get(pair[1] as usize)) {
                    if k == key {
                        return Some(v);
                    }
                }
            }
        }
        None
    }
}

/// A decoded tile: its layers in the order the tile holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub layers: Vec<Layer>,
}

impl Tile {
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }
}

/// Decodes a tile's message structure: layers, their tables and their features.
///
/// The bytes are the tile itself, not gzipped; a PMTiles archive says how its tiles are
/// compressed ([`crate::tile::pmtiles::decompress`]).
pub fn decode(bytes: &[u8]) -> Outcome<Tile> {
    let mut pb = Pb::new(bytes);
    let mut tile = Tile::default();
    while !pb.done() {
        let (field, wire) = res!(pb.key());
        match field {
            3 => {
                res!(want(wire, WIRE_LEN, "A layer"));
                let n = tile.layers.len();
                tile.layers.push(res!(layer(res!(pb.bytes()), n)));
            },
            _ => res!(pb.skip(wire)),
        }
    }
    Ok(tile)
}

fn layer(buf: &[u8], index: usize) -> Outcome<Layer> {
    let mut pb = Pb::new(buf);
    let mut version: Option<u32> = None;
    let mut name: Option<String> = None;
    let mut extent = 4096u32;
    let mut keys = Vec::new();
    let mut values = Vec::new();
    let mut features = Vec::new();
    while !pb.done() {
        let (field, wire) = res!(pb.key());
        match field {
            15 => {
                res!(want(wire, WIRE_VARINT, "A layer's version"));
                version = Some(res!(u32_of(res!(pb.varint()), "A layer's version")));
            },
            1 => {
                res!(want(wire, WIRE_LEN, "A layer's name"));
                name = Some(res!(text(res!(pb.bytes()), "A layer's name")));
            },
            2 => {
                res!(want(wire, WIRE_LEN, "A feature"));
                features.push(res!(feature(res!(pb.bytes()))));
            },
            3 => {
                res!(want(wire, WIRE_LEN, "A layer key"));
                keys.push(res!(text(res!(pb.bytes()), "A layer key")));
            },
            4 => {
                res!(want(wire, WIRE_LEN, "A layer value"));
                values.push(res!(value(res!(pb.bytes()))));
            },
            5 => {
                res!(want(wire, WIRE_VARINT, "A layer's extent"));
                extent = res!(u32_of(res!(pb.varint()), "A layer's extent"));
            },
            _ => res!(pb.skip(wire)),
        }
    }
    let name = match name {
        Some(n) => n,
        None => return Err(err!("Layer {} has no name.", index; Invalid, Input, Decode, Missing)),
    };
    let version = match version {
        Some(v @ 1) | Some(v @ 2) => v,
        Some(v) => return Err(err!("Layer {:?} is version {}; versions 1 and 2 exist.", name, v;
            Invalid, Input, Decode, Version)),
        None => return Err(err!("Layer {:?} has no version.", name; Invalid, Input, Decode, Missing)),
    };
    Ok(Layer { version, name, extent, keys, values, features })
}

fn feature(buf: &[u8]) -> Outcome<Feature> {
    let mut pb = Pb::new(buf);
    let mut f = Feature { id: None, kind: GeomKind::Unknown, tags: Vec::new(), geometry: Vec::new() };
    while !pb.done() {
        let (field, wire) = res!(pb.key());
        match field {
            1 => {
                res!(want(wire, WIRE_VARINT, "A feature's id"));
                f.id = Some(res!(pb.varint()));
            },
            2 => res!(pb.u32s(wire, &mut f.tags, "A feature's tags")),
            3 => {
                res!(want(wire, WIRE_VARINT, "A feature's type"));
                // An unknown type is kept as unknown, as a proto2 enum would be.
                f.kind = match res!(pb.varint()) {
                    1   => GeomKind::Point,
                    2   => GeomKind::LineString,
                    3   => GeomKind::Polygon,
                    _   => GeomKind::Unknown,
                };
            },
            4 => res!(pb.u32s(wire, &mut f.geometry, "A feature's geometry")),
            _ => res!(pb.skip(wire)),
        }
    }
    Ok(f)
}

fn value(buf: &[u8]) -> Outcome<Value> {
    let mut pb = Pb::new(buf);
    let mut out: Option<Value> = None;
    while !pb.done() {
        let (field, wire) = res!(pb.key());
        let v = match field {
            1 => {
                res!(want(wire, WIRE_LEN, "A string value"));
                Value::Str(res!(text(res!(pb.bytes()), "A string value")))
            },
            2 => {
                res!(want(wire, WIRE_I32, "A float value"));
                let b = res!(pb.fixed(4));
                Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            },
            3 => {
                res!(want(wire, WIRE_I64, "A double value"));
                let b = res!(pb.fixed(8));
                Value::Double(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
            },
            4 => {
                res!(want(wire, WIRE_VARINT, "An int value"));
                Value::Int(res!(pb.varint()) as i64)
            },
            5 => {
                res!(want(wire, WIRE_VARINT, "A uint value"));
                Value::Uint(res!(pb.varint()))
            },
            6 => {
                res!(want(wire, WIRE_VARINT, "A sint value"));
                Value::Sint(zigzag(res!(pb.varint())))
            },
            7 => {
                res!(want(wire, WIRE_VARINT, "A bool value"));
                Value::Bool(res!(pb.varint()) != 0)
            },
            other => return Err(err!("A value has field {}, which is no kind of value.", other;
                Invalid, Input, Decode)),
        };
        if out.is_some() {
            return Err(err!("A value holds more than one kind of value."; Invalid, Input, Decode));
        }
        out = Some(v);
    }
    match out {
        Some(v) => Ok(v),
        None    => Err(err!("A value holds no value."; Invalid, Input, Decode, Missing)),
    }
}

// ---------------------------------------------------------------------------------------------
// Geometry and properties
// ---------------------------------------------------------------------------------------------

const CMD_MOVE_TO:      u32 = 1;
const CMD_LINE_TO:      u32 = 2;
const CMD_CLOSE_PATH:   u32 = 7;

impl Feature {
    /// The geometry in tile coordinates: one run of points per `MoveTo`.
    ///
    /// A point feature's runs are single points; a line's are its lines; a polygon's are its
    /// rings, each closed by repeating its first point, exterior rings wound clockwise on a
    /// `y`-down screen and holes the other way, so a nonzero fill paints them right.  Positions
    /// accumulate with 32-bit wrapping, as the specification's overflow fixtures expect.
    pub fn geometry(&self) -> Outcome<Vec<Vec<(i32, i32)>>> {
        let g = &self.geometry;
        let mut out: Vec<Vec<(i32, i32)>> = Vec::new();
        let (mut x, mut y) = (0i32, 0i32);
        let mut i = 0usize;
        while i < g.len() {
            let cmd = g[i] & 7;
            let count = (g[i] >> 3) as usize;
            i += 1;
            match cmd {
                CMD_MOVE_TO | CMD_LINE_TO => {
                    // A count is checked against what follows before anything is sized by it.
                    let left = (g.len() - i) / 2;
                    if count > left {
                        return Err(err!("A command at {} asks for {} points and {} follow.",
                            i - 1, count, left; Invalid, Input, Decode));
                    }
                    if count == 0 {
                        return Err(err!("A command at {} has a count of nought.", i - 1;
                            Invalid, Input, Decode));
                    }
                    if cmd == CMD_LINE_TO && out.last().map_or(true, |r| r.is_empty()) {
                        return Err(err!("A LineTo at {} comes before any MoveTo.", i - 1;
                            Invalid, Input, Decode));
                    }
                    for _ in 0..count {
                        x = x.wrapping_add(zigzag(g[i] as u64) as i32);
                        y = y.wrapping_add(zigzag(g[i + 1] as u64) as i32);
                        i += 2;
                        if cmd == CMD_MOVE_TO {
                            out.push(vec![(x, y)]);
                        } else if let Some(r) = out.last_mut() {
                            r.push((x, y));
                        }
                    }
                },
                CMD_CLOSE_PATH => {
                    if count != 1 {
                        return Err(err!("A ClosePath at {} has a count of {}, not one.", i - 1, count;
                            Invalid, Input, Decode));
                    }
                    if self.kind != GeomKind::Polygon {
                        return Err(err!("A ClosePath at {} in a {:?} feature.", i - 1, self.kind;
                            Invalid, Input, Decode));
                    }
                    match out.last_mut() {
                        Some(r) if !r.is_empty() => {
                            let first = r[0];
                            r.push(first);
                        },
                        _ => return Err(err!("A ClosePath at {} closes nothing.", i - 1;
                            Invalid, Input, Decode)),
                    }
                },
                other => return Err(err!("Command {} at {} is not MoveTo, LineTo or ClosePath.",
                    other, i - 1; Invalid, Input, Decode)),
            }
        }
        Ok(out)
    }

    /// The feature's properties, as its layer's keys and values, in the order its tags list
    /// them.
    pub fn properties<'a>(&'a self, layer: &'a Layer) -> Outcome<Vec<(&'a str, &'a Value)>> {
        if self.tags.len() % 2 != 0 {
            return Err(err!("A feature has {} tags, which do not pair.", self.tags.len();
                Invalid, Input, Decode));
        }
        let mut out = Vec::with_capacity(self.tags.len() / 2);
        for pair in self.tags.chunks(2) {
            let k = match layer.keys.get(pair[0] as usize) {
                Some(k) => k,
                None => return Err(err!("A tag names key {} of the {} layer {:?} has.",
                    pair[0], layer.keys.len(), layer.name; Invalid, Input, Decode, Index)),
            };
            let v = match layer.values.get(pair[1] as usize) {
                Some(v) => v,
                None => return Err(err!("A tag names value {} of the {} layer {:?} has.",
                    pair[1], layer.values.len(), layer.name; Invalid, Input, Decode, Index)),
            };
            out.push((k.as_str(), v));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// Style classes
// ---------------------------------------------------------------------------------------------

/// A painter's rule: features of a layer, optionally only those whose `key` property is one of
/// `values`, drawn from `min_zoom` on, belong to `class`.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub layer:      String,
    pub key:        String,         // the property that says what a feature is: `kind`, `class`
    pub values:     Vec<String>,    // empty for any
    pub min_zoom:   u8,
    pub class:      u16,
}

/// Sorts a tile's features into a painter's classes: `(class, layer index, feature index)` for
/// every feature some rule takes, the first matching rule deciding, in rule order and then
/// tile order, which is the order a painter draws them in.
///
/// `feature_min_zoom` names a per-feature property, as Protomaps' `min_zoom`, below which a
/// feature is not yet drawn even where its rule would take it.
pub fn classify(tile: &Tile, rules: &[StyleRule], zoom: u8, feature_min_zoom: Option<&str>)
    -> Vec<(u16, usize, usize)>
{
    let mut hits: Vec<(usize, u16, usize, usize)> = Vec::new();
    for (li, layer) in tile.layers.iter().enumerate() {
        for (fi, f) in layer.features.iter().enumerate() {
            if let Some(key) = feature_min_zoom {
                if let Some(mz) = layer.property(f, key).and_then(|v| v.as_f64()) {
                    if mz > zoom as f64 {
                        continue;
                    }
                }
            }
            for (ri, rule) in rules.iter().enumerate() {
                if rule.layer != layer.name || zoom < rule.min_zoom {
                    continue;
                }
                if !rule.values.is_empty() {
                    let kind = layer.property(f, &rule.key).and_then(|v| v.as_str());
                    if !kind.map_or(false, |k| rule.values.iter().any(|v| v == k)) {
                        continue;
                    }
                }
                hits.push((ri, rule.class, li, fi));
                break;
            }
        }
    }
    hits.sort_by_key(|h| (h.0, h.2, h.3));
    hits.into_iter().map(|(_, c, l, f)| (c, l, f)).collect()
}
