//! `cfg::fill_from_template`, an installed configuration's missing optional keys read from the
//! release's embedded template (CFG-COMPAT, 31 Oct 2026).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg,
};


const TEMPLATE: &str = r#"{
    "name": "shipped",
    "run": {
        "pool":     4,
        "queue":    8,
    },
    "os": {
        "slice":    "app.slice",
    },
}"#;

fn map_of(text: &str) -> Outcome<DaticleMap> {
    cfg::template_map(text)
}

fn get<'a>(map: &'a DaticleMap, sec: &str, key: &str) -> Option<&'a Dat> {
    match map.get(&Dat::Str(fmt!("{}", sec))) {
        Some(Dat::Map(m))   => m.get(&Dat::Str(fmt!("{}", key))),
        _                   => None,
    }
}

#[test]
fn an_absent_key_reads_the_template() -> Outcome<()> {
    let mut map = res!(map_of(r#"{ "run": { "pool": 4 }, "os": { "slice": "app.slice" } }"#));
    res!(cfg::fill_from_template(&mut map, TEMPLATE, &["run", "os"]));
    assert_eq!(get(&map, "run", "queue"), Some(&Dat::U8(8)));
    Ok(())
}

#[test]
fn an_installed_key_is_kept() -> Outcome<()> {
    let mut map = res!(map_of(r#"{ "run": { "pool": 2 } }"#));
    res!(cfg::fill_from_template(&mut map, TEMPLATE, &["run"]));
    assert_eq!(get(&map, "run", "pool"),  Some(&Dat::U8(2)));
    assert_eq!(get(&map, "run", "queue"), Some(&Dat::U8(8)));
    Ok(())
}

#[test]
fn an_absent_section_is_taken_whole() -> Outcome<()> {
    let mut map = res!(map_of(r#"{ "run": {} }"#));
    res!(cfg::fill_from_template(&mut map, TEMPLATE, &["run", "os"]));
    assert_eq!(get(&map, "os", "slice"), Some(&Dat::Str(fmt!("app.slice"))));
    Ok(())
}

#[test]
fn a_key_outside_the_sections_stays_required() -> Outcome<()> {
    let mut map = res!(map_of(r#"{ "run": {} }"#));
    res!(cfg::fill_from_template(&mut map, TEMPLATE, &["run"]));
    assert!(map.get(&Dat::Str(fmt!("name"))).is_none(), "a top-level key is never filled");
    assert!(map.get(&Dat::Str(fmt!("os"))).is_none(),   "an unnamed section is never filled");
    Ok(())
}

#[test]
fn a_template_that_fails_to_decode_is_refused() {
    let broken = r#"{ "run": { "pool": "#;
    assert!(cfg::template_map(broken).is_err(), "a broken template is never an empty map");
    let mut map = DaticleMap::new();
    let r = cfg::fill_from_template(&mut map, broken, &["run"]);
    assert!(r.is_err(), "a broken template is never read as an empty one");
    assert!(map.is_empty());
    assert!(cfg::template_map("[1, 2]").is_err(), "a template that is no map is refused");
}

#[test]
fn a_section_the_template_lacks_is_refused_by_name() {
    let mut map = DaticleMap::new();
    match cfg::fill_from_template(&mut map, TEMPLATE, &["net"]) {
        Err(e)  => assert!(fmt!("{}", e).contains("'net'"), "{}", e),
        Ok(())  => panic!("a section the template lacks was accepted"),
    }
}

#[test]
fn an_installed_section_that_is_no_map_is_refused_by_name() -> Outcome<()> {
    let mut map = res!(map_of(r#"{ "run": 5 }"#));
    match cfg::fill_from_template(&mut map, TEMPLATE, &["run"]) {
        Err(e)  => assert!(fmt!("{}", e).contains("'run'"), "{}", e),
        Ok(())  => panic!("a section that is no map was accepted"),
    }
    Ok(())
}
