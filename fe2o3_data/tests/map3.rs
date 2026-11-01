//! Tests for the three-way map merge, one per cell of its table.

use oxedyne_fe2o3_data::map3::{
	merge3,
	Merge3,
};

use std::collections::{
	BTreeMap,
	BTreeSet,
};


fn map(kv: &[(&'static str, u8)]) -> BTreeMap<&'static str, u8> {
	kv.iter().cloned().collect()
}

fn set(ks: &[&'static str]) -> BTreeSet<&'static str> {
	ks.iter().cloned().collect()
}

#[test]
fn unchanged_both_sides_is_in_no_set() {
	let b = map(&[("a", 1)]);
	let m = merge3(&b, &b, &b);
	assert_eq!(m, Merge3::default());
	assert!(m.is_clean());
}

#[test]
fn theirs_changed_alone_is_taken() {
	let b = map(&[("a", 1), ("d", 4)]);
	let o = b.clone();
	let t = map(&[("a", 2), ("n", 9)]); // a edited, d deleted, n added
	let m = merge3(&b, &o, &t);
	assert_eq!(m.take, set(&["a", "d", "n"]));
	assert!(m.keep.is_empty());
	assert!(m.is_clean());
}

#[test]
fn ours_changed_alone_is_kept() {
	let b = map(&[("a", 1), ("d", 4)]);
	let o = map(&[("a", 2), ("n", 9)]);
	let t = b.clone();
	let m = merge3(&b, &o, &t);
	assert_eq!(m.keep, set(&["a", "d", "n"]));
	assert!(m.take.is_empty());
	assert!(m.is_clean());
}

#[test]
fn both_changed_differently_conflicts() {
	let b = map(&[("a", 1), ("d", 4)]);
	let o = map(&[("a", 2), ("n", 7)]); // d deleted here
	let t = map(&[("a", 3), ("d", 5), ("n", 8)]); // d edited there
	let m = merge3(&b, &o, &t);
	assert_eq!(m.conflict, set(&["a", "d", "n"]));
	assert!(m.take.is_empty());
	assert!(m.keep.is_empty());
	assert!(!m.is_clean());
}

#[test]
fn both_deleted_is_kept_without_conflict() {
	let b = map(&[("a", 1), ("s", 2)]);
	let o = map(&[("s", 2)]);
	let t = map(&[("s", 2)]);
	let m = merge3(&b, &o, &t);
	assert_eq!(m.keep, set(&["a"]));
	assert!(m.take.is_empty());
	assert!(m.is_clean());
}

#[test]
fn same_bytes_both_sides_is_kept_without_conflict() {
	let b = map(&[("a", 1)]);
	let o = map(&[("a", 2), ("n", 9)]);
	let t = map(&[("a", 2), ("n", 9)]);
	let m = merge3(&b, &o, &t);
	assert_eq!(m.keep, set(&["a", "n"]));
	assert!(m.take.is_empty());
	assert!(m.is_clean());
}

#[test]
fn mixed_keys_sort_independently() {
	let b = map(&[("k", 1), ("t", 1), ("c", 1), ("u", 1)]);
	let o = map(&[("k", 2), ("t", 1), ("c", 2), ("u", 1)]);
	let t = map(&[("k", 1), ("t", 2), ("c", 3), ("u", 1)]);
	let m = merge3(&b, &o, &t);
	assert_eq!(m.take, set(&["t"]));
	assert_eq!(m.keep, set(&["k"]));
	assert_eq!(m.conflict, set(&["c"]));
}
