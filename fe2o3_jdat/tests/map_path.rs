//! Setting a value at a path into nested maps, the way `austenite --set colour.space=rgb` does.

use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::BTreeMap;

fn s(k: &str) -> Dat { Dat::Str(k.to_string()) }

fn found<'a>(d: &'a Dat, path: &[&str]) -> Option<&'a Dat> {
	let keys = Dat::List(path.iter().map(|k| s(k)).collect());
	d.find(&keys).expect("a find over a map")
}

#[test]
fn a_dotted_path_makes_the_levels_it_lacks() {
	let mut d = Dat::Map(BTreeMap::new());
	let was = d.map_put_dotted("colour.space", s("rgb")).expect("a set");
	assert_eq!(was, None);
	assert_eq!(found(&d, &["colour", "space"]), Some(&s("rgb")));
}

#[test]
fn a_set_replaces_the_value_and_returns_it_and_keeps_its_siblings() {
	let mut d = Dat::Map(BTreeMap::new());
	d.map_put_dotted("a.b", Dat::U8(1)).expect("a set");
	d.map_put_dotted("a.c", Dat::U8(2)).expect("a set");
	let was = d.map_put_dotted("a.b", Dat::U8(3)).expect("a set");
	assert_eq!(was, Some(Dat::U8(1)));
	assert_eq!(found(&d, &["a", "b"]), Some(&Dat::U8(3)));
	assert_eq!(found(&d, &["a", "c"]), Some(&Dat::U8(2)));
}

#[test]
fn a_single_key_is_a_plain_put() {
	let mut d = Dat::Map(BTreeMap::new());
	d.map_put_dotted("strict", Dat::Bool(true)).expect("a set");
	assert_eq!(found(&d, &["strict"]), Some(&Dat::Bool(true)));
}

#[test]
fn a_path_through_a_value_that_is_not_a_map_is_refused_and_changes_nothing() {
	let mut d = Dat::Map(BTreeMap::new());
	d.map_put_dotted("strict", Dat::Bool(true)).expect("a set");
	let before = d.clone();
	let e = d.map_put_dotted("strict.deeper", Dat::U8(1));
	assert!(e.is_err(), "a bool is no map");
	assert_eq!(d, before);
}

#[test]
fn an_empty_key_is_refused() {
	for bad in ["", ".a", "a.", "a..b"] {
		let mut d = Dat::Map(BTreeMap::new());
		assert!(d.map_put_dotted(bad, Dat::U8(1)).is_err(), "'{}' has an empty key", bad);
	}
}

#[test]
fn a_value_that_is_no_map_takes_no_path() {
	let mut d = Dat::Bool(true);
	assert!(d.map_put_dotted("a", Dat::U8(1)).is_err());
}

#[test]
fn an_ordered_map_keeps_its_order_and_gains_an_ordered_child() {
	let mut d = Dat::OrdMap(BTreeMap::new());
	d.map_put_dotted("a.b", Dat::U8(1)).expect("a set");
	d.map_put_dotted("z", Dat::U8(9)).expect("a set");
	d.map_put_dotted("a.b", Dat::U8(2)).expect("a set");
	match found(&d, &["a"]) {
		Some(Dat::OrdMap(_))    => (),
		other                   => panic!("the child of an ordered map is ordered, got {:?}", other),
	}
	assert_eq!(found(&d, &["a", "b"]), Some(&Dat::U8(2)));
	assert_eq!(found(&d, &["z"]), Some(&Dat::U8(9)));
}

#[test]
fn the_mutable_getter_changes_a_value_in_place() {
	let mut d = Dat::Map(BTreeMap::new());
	d.map_put_dotted("k", Dat::U8(1)).expect("a set");
	*d.map_get_mut(&s("k")).expect("a lookup").expect("present") = Dat::U8(7);
	assert_eq!(found(&d, &["k"]), Some(&Dat::U8(7)));
	assert!(d.map_get_mut(&s("absent")).expect("a lookup").is_none());
}
