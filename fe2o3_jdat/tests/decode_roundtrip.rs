//! The encoder's own output must read back as the daticle it was written from.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{prelude::*, map::MapKey, note::NoteConfig};
use oxedyne_fe2o3_num::float::{Float32, Float64};
use num_bigint::BigInt;
use std::collections::BTreeMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; x }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
}

const ALPHA: &[char] = &['a','b','Z','0','9',' ','"','\'','\\','{','}','[',']','(',')','|',':',',','!','#','\n','\t','\r','é','😀','\u{1}','\u{7f}','\u{feff}','u','/','-','.','e','x'];
const CALPHA: &[char] = &['a','b',' ','"','\'','{','}','[',']','(',')','|',':',',','\\','x','.'];

fn rstr(r: &mut Rng, alpha: &[char], max: u64) -> String {
    let n = r.below(max);
    (0..n).map(|_| alpha[r.below(alpha.len() as u64) as usize]).collect()
}

fn gen(r: &mut Rng, depth: u32, notes: bool) -> Dat {
    let leaf = depth == 0 || r.below(3) == 0;
    if leaf {
        match r.below(20) {
            0 => Dat::Empty,
            1 => Dat::Bool(r.below(2) == 0),
            2 => Dat::U8(r.next() as u8),
            3 => Dat::U16(r.next() as u16),
            4 => Dat::U32(r.next() as u32),
            5 => Dat::U64(r.next()),
            6 => Dat::U128(((r.next() as u128) << 64) | r.next() as u128),
            7 => Dat::I8(r.next() as i8),
            8 => Dat::I16(r.next() as i16),
            9 => Dat::I32(r.next() as i32),
            10 => Dat::I64(r.next() as i64),
            11 => Dat::I128((((r.next() as u128) << 64) | r.next() as u128) as i128),
            12 => { let f = f32::from_bits(r.next() as u32); Dat::F32(Float32(if f.is_finite() { f } else { 1.5 })) }
            13 => { let f = f64::from_bits(r.next()); Dat::F64(Float64(if f.is_finite() { f } else { -2.5 })) }
            14 => Dat::Aint(BigInt::from(r.next() as i64) * BigInt::from(r.next()) * BigInt::from(r.next())),
            15 => Dat::C64(r.next() >> r.below(64)),
            16 => Dat::BU8((0..r.below(6)).map(|_| r.next() as u8).collect()),
            17 => Dat::B2([r.next() as u8, r.next() as u8]),
            18 => Dat::Opt(Box::new(None)),
            _ => Dat::Str(rstr(r, ALPHA, 12)),
        }
    } else {
        let d = depth - 1;
        match r.below(8) {
            0 => Dat::List((0..r.below(5)).map(|_| child(r, d, notes)).collect()),
            1 => Dat::Box(Box::new(gen(r, d, notes))),
            2 => Dat::Opt(Box::new(Some(gen(r, d, notes)))),
            3 => Dat::Tup2(Box::new([gen(r, d, notes), gen(r, d, notes)])),
            4 | 5 => {
                let mut m = BTreeMap::new();
                for _ in 0..r.below(5) {
                    let k = if r.below(4) == 0 { Dat::U32(r.next() as u32) } else { Dat::Str(rstr(r, ALPHA, 8)) };
                    m.insert(k, child(r, d, notes));
                }
                Dat::Map(m)
            }
            6 => {
                let mut m = BTreeMap::new();
                let mut used = std::collections::BTreeSet::new();
                let mut ord = Dat::OMAP_ORDER_START_DEFAULT; // The text holds no order, the reader numbers the keys.
                for _ in 0..r.below(5) {
                    let k = Dat::Str(rstr(r, ALPHA, 8));
                    if !used.insert(k.clone()) { continue; }
                    m.insert(MapKey::new(ord, k), child(r, d, notes));
                    ord += Dat::OMAP_ORDER_DELTA_DEFAULT;
                }
                Dat::OrdMap(m)
            }
            _ => Dat::List(vec![child(r, d, notes)]),
        }
    }
}

// A list item or a map value, which may carry a note; the reader keeps a note nowhere else.
fn child(r: &mut Rng, depth: u32, notes: bool) -> Dat {
    let v = gen(r, depth, notes);
    if notes && r.below(4) == 0 {
        // A note is kept trimmed, and an empty one is not kept at all.
        let note = rstr(r, CALPHA, 10).trim().to_string();
        if note.is_empty() { v } else { Dat::ABox(NoteConfig::default(), Box::new(v), note) }
    } else { v }
}


// Where two debug forms part, with a little of each side.
fn first_diff(a: &str, b: &str) -> String {
    let n = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let from = n.saturating_sub(40);
    let cut = |s: &str| s.chars().skip(from).take(100).collect::<String>();
    fmt!("at {}: want {:?} got {:?}", n, cut(a), cut(b))
}

// Round trip: every daticle the encoder writes must read back equal, in both encodings.
fn run(n: u64, notes: bool) -> (u64, Vec<String>) {
    let mut r = Rng(0x9E3779B97F4A7C15);
    let mut ok = 0;
    let mut bad = Vec::new();
    for i in 0..n {
        let dat = gen(&mut r, 4, notes);
        // The lines encoder drops a note's value from a list or map, so notes go through the one-line form only.
        let mut encs = vec![("jdat", dat.jdat())];
        if !notes { encs.push(("lines", dat.jdat_to_lines("  "))); }
        for (mode, enc) in encs {
            let s = match enc { Ok(s) => s, Err(e) => { bad.push(fmt!("#{} {} encoder: {}", i, mode, e.plain())); continue; } };
            match Dat::decode_string(&s) {
                Ok(d2) if d2 == dat => ok += 1,
                Ok(d2) => bad.push(fmt!("#{} {} DIFF {}", i, mode, first_diff(&fmt!("{:?}", dat), &fmt!("{:?}", d2)))),
                Err(e) => bad.push(fmt!("#{} {} ERR text={:?} {}", i, mode, s, e.plain().replace('\n', " "))),
            }
        }
    }
    (ok, bad)
}

#[test]
fn test_roundtrip_random_00() {
    random(false);
}

#[test]
fn test_roundtrip_random_notes_00() {
    random(true);
}

fn random(notes: bool) {
    let (ok, bad) = run(3000, notes);
    for b in bad.iter().take(25) {
        println!("{}", b.chars().take(400).collect::<String>());
    }
    assert!(bad.is_empty(), "{} of {} decodes failed, the first being: {}", bad.len(), ok + bad.len() as u64,
        bad.first().map(|b| b.chars().take(600).collect::<String>()).unwrap_or_default());
}


fn leaves() -> Vec<Dat> {
    vec![
        Dat::Empty,
        Dat::Bool(true),
        Dat::Opt(Box::new(None)),
        Dat::U8(1),
        Dat::Str(fmt!("a")),
        Dat::List(Vec::new()),
        Dat::Map(BTreeMap::new()),
        Dat::OrdMap(BTreeMap::new()),
        Dat::Tup2(Box::new([Dat::U8(1), Dat::U8(2)])),
    ]
}

fn wrap(x: &Dat) -> Vec<Dat> {
    let mut m = BTreeMap::new();
    m.insert(Dat::Str(fmt!("k")), x.clone());
    let mut o = BTreeMap::new();
    o.insert(MapKey::new(Dat::OMAP_ORDER_START_DEFAULT, Dat::Str(fmt!("k"))), x.clone());
    vec![
        Dat::Box(Box::new(x.clone())),
        Dat::Opt(Box::new(Some(x.clone()))),
        Dat::List(vec![x.clone()]),
        Dat::List(vec![x.clone(), Dat::U8(3)]),
        Dat::Map(m),
        Dat::OrdMap(o),
        Dat::Tup2(Box::new([x.clone(), Dat::U8(1)])),
        Dat::Tup2(Box::new([Dat::U8(1), x.clone()])),
    ]
}

#[test]
fn test_roundtrip_systematic_00() {
    let mut all = leaves();
    let mut level = all.clone();
    for _ in 0..2 {
        let mut next = Vec::new();
        for x in &level { next.extend(wrap(x)); }
        all.extend(next.iter().cloned());
        level = next;
    }
    let mut bad = Vec::new();
    for dat in &all {
        for (mode, enc) in [("jdat", dat.jdat()), ("lines", dat.jdat_to_lines("  "))] {
            let s = match enc { Ok(s) => s, Err(e) => { bad.push(fmt!("{} encoder: {}", mode, e.plain())); continue; } };
            match Dat::decode_string(&s) {
                Ok(d2) if d2 == *dat => (),
                Ok(d2) => bad.push(fmt!("{} DIFF text={:?} got={:?}", mode, s, d2)),
                Err(e) => bad.push(fmt!("{} ERR text={:?} {}", mode, s, e.plain().replace('\n', " "))),
            }
        }
    }
    bad.sort_by_key(|b| b.len());
    for b in bad.iter().take(25) {
        println!("{}", b.chars().take(260).collect::<String>());
    }
    assert!(bad.is_empty(), "{} of {} decodes failed, the shortest being: {}", bad.len(), 2 * all.len(),
        bad.first().map(|b| b.chars().take(400).collect::<String>()).unwrap_or_default());
}
