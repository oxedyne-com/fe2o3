use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::BTreeMap;


#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
struct Unit;

#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
struct Pair(u32, String);

#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
struct Wrap(Vec<u8>);

#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
enum Shape {
    Empty,
    Point(i64, i64),
    Named { label: String, size: Option<u16> },
    One(Box<Shape>),
}

#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
struct Change {
    id:         u64,
    body:       Shape,
    bytes:      Option<Vec<u8>>,
    shapes:     Option<Vec<Shape>>,
    pairs:      Vec<(String, u8)>,
    triple:     (bool, f64, i128),
    quad:       (u8, u16, u32, Pair),
    index:      BTreeMap<String, Vec<Shape>>,
    flag:       bool,
    size:       usize,
    unit:       Unit,
    wrap:       Wrap,
    nothing:    Option<String>,
    outcome:    Result<u32, String>,
}

#[derive(Clone, Debug, PartialEq, ToDat, FromDat)]
struct Generic<T> {
    item: T,
    more: Vec<T>,
}

fn round<T: ToDat + FromDat + PartialEq + std::fmt::Debug>(v: &T) -> Outcome<()> {
    let dat = res!(v.to_dat());
    let back = res!(T::from_dat(dat.clone()));
    assert_eq!(&back, v);
    // And through the binary form.
    let bytes = res!(dat.to_bytes(Vec::new()));
    let (dat2, n) = res!(Dat::from_bytes(&bytes));
    assert_eq!(n, bytes.len());
    let back2 = res!(T::from_dat(dat2));
    assert_eq!(&back2, v);
    Ok(())
}

fn change() -> Change {
    let mut index = BTreeMap::new();
    index.insert("a".to_string(), vec![Shape::Empty, Shape::Point(-1, 2)]);
    index.insert("b".to_string(), Vec::new());
    Change {
        id:         7,
        body:       Shape::Named { label: "x".to_string(), size: Some(3) },
        bytes:      Some(vec![0, 1, 255]),
        shapes:     Some(vec![Shape::Empty, Shape::One(Box::new(Shape::Point(1, 1)))]),
        pairs:      vec![("p".to_string(), 1), ("q".to_string(), 2)],
        triple:     (true, 2.5, -9),
        quad:       (1, 2, 3, Pair(4, "four".to_string())),
        index,
        flag:       false,
        size:       usize::MAX,
        unit:       Unit,
        wrap:       Wrap(vec![9; 40]),
        nothing:    None,
        outcome:    Err("no".to_string()),
    }
}

#[test]
fn every_shape_round_trips() -> Outcome<()> {
    res!(round(&Unit));
    res!(round(&Pair(1, "one".to_string())));
    res!(round(&Wrap(Vec::new())));
    res!(round(&Shape::Empty));
    res!(round(&Shape::Point(i64::MIN, i64::MAX)));
    res!(round(&Shape::Named { label: String::new(), size: None }));
    res!(round(&Shape::One(Box::new(Shape::Empty))));
    res!(round(&change()));
    res!(round(&Generic { item: 3u8, more: vec![4, 5] }));
    res!(round(&Generic { item: Shape::Empty, more: vec![Shape::Point(0, 0)] }));
    let nested: Option<Vec<Shape>> = Some(vec![Shape::Empty, Shape::Point(1, 2)]);
    res!(round(&nested));
    res!(round(&(1u8, "two".to_string())));
    res!(round(&Ok::<u8, String>(3)));
    Ok(())
}

#[test]
fn the_encoding_is_the_documented_one() -> Outcome<()> {
    // A unit variant is a one-entry map from its name to an empty daticle.
    assert_eq!(res!(Shape::Empty.to_dat()), mapdat!{ "Empty" => Dat::Empty });
    // A tuple variant maps to a list, a struct variant to a map of its fields.
    assert_eq!(
        res!(Shape::Point(1, 2).to_dat()),
        mapdat!{ "Point" => listdat![1i64, 2i64] },
    );
    assert_eq!(
        res!(Shape::Named { label: "l".to_string(), size: None }.to_dat()),
        mapdat!{ "Named" => mapdat!{ "label" => "l", "size" => Dat::Opt(Box::new(None)) } },
    );
    // A named struct is a map, a tuple struct a list, a unit struct empty.
    assert_eq!(res!(Pair(1, "a".to_string()).to_dat()), listdat![1u32, "a"]);
    assert_eq!(res!(Unit.to_dat()), Dat::Empty);
    // Bytes stay bytes.
    assert_eq!(res!(vec![1u8, 2].to_dat()), Dat::BU64(vec![1, 2]));
    Ok(())
}

#[test]
fn an_unknown_variant_is_refused() -> Outcome<()> {
    let dat = mapdat!{ "Hexagon" => Dat::Empty };
    match Shape::from_dat(dat) {
        Ok(v) => return Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            let s = e.plain();
            assert!(s.contains("Shape") && s.contains("Hexagon"), "{}", s);
        },
    }
    // Two entries are not one variant.
    let dat = mapdat!{ "Empty" => Dat::Empty, "Point" => listdat![1i64, 2i64] };
    assert!(Shape::from_dat(dat).is_err());
    Ok(())
}

#[test]
fn a_missing_field_is_refused() -> Outcome<()> {
    let mut dat = res!(change().to_dat());
    if let Dat::Map(map) = &mut dat {
        map.remove(&dat!("body"));
    }
    match Change::from_dat(dat) {
        Ok(v) => return Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            let s = e.plain();
            assert!(s.contains("Change") && s.contains("body") && s.contains("Decoding"), "{}", s);
        },
    }
    let dat = mapdat!{ "Named" => mapdat!{ "label" => "l" } };
    match Shape::from_dat(dat) {
        Ok(v) => return Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            let s = e.plain();
            assert!(s.contains("Shape::Named") && s.contains("size"), "{}", s);
        },
    }
    Ok(())
}

#[test]
fn a_wrong_shape_is_refused() -> Outcome<()> {
    // An unknown field, a short list, a nested fault named by its field.
    let mut dat = res!(change().to_dat());
    if let Dat::Map(map) = &mut dat {
        map.insert(dat!("extra"), dat!(1u8));
    }
    assert!(Change::from_dat(dat).is_err());
    assert!(Pair::from_dat(listdat![1u32]).is_err());
    assert!(Pair::from_dat(listdat![1u32, "a", "b"]).is_err());
    let mut dat = res!(change().to_dat());
    if let Dat::Map(map) = &mut dat {
        map.insert(dat!("body"), mapdat!{ "Point" => listdat![1i64] });
    }
    match Change::from_dat(dat) {
        Ok(v) => return Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            let s = e.plain();
            assert!(s.contains("body") && s.contains("Shape::Point"), "{}", s);
        },
    }
    Ok(())
}

#[test]
fn an_error_survives_the_trip() -> Outcome<()> {
    let inner = err!("the inner fault"; Input, Missing);
    let e: Error<ErrTag> = Error::Upstream(std::sync::Arc::new(inner), ErrMsg {
        msg:    "outer".to_string(),
        tags:   &[ErrTag::Wire],
    });
    let dat = res!(e.to_dat());
    let bytes = res!(dat.to_bytes(Vec::new()));
    let (dat2, _) = res!(Dat::from_bytes(&bytes));
    let back = res!(Error::<ErrTag>::from_dat(dat2));
    assert_eq!(back.tags(), e.tags());
    assert_eq!(back.msgs(), e.msgs());
    let r: Result<u8, Error<ErrTag>> = Err(e.clone());
    let back = res!(Result::<u8, Error<ErrTag>>::from_dat(res!(r.to_dat())));
    match back {
        Err(b)  => assert_eq!(b.tags(), e.tags()),
        Ok(_)   => return Err(err!("Lost the error."; Test)),
    }
    Ok(())
}
