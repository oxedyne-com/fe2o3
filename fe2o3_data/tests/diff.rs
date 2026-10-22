//! Tests for the shortest edit script.

use oxedyne_fe2o3_data::diff::{
	changed_runs,
	matches,
	Change,
};


#[test]
fn empty_sides_match_nothing() {
	let e: [u8; 0] = [];
	assert_eq!(matches(&e, &e, 10), Some(Vec::new()));
	assert_eq!(changed_runs(&e, &e, 10), Some(Vec::new()));
	assert_eq!(matches(&e, b"abc", 10), Some(Vec::new()));
	assert_eq!(changed_runs(b"abc", &e, 0), Some(vec![Change { a: 0..3, b: 0..0 }]));
	assert_eq!(changed_runs(&e, b"abc", 0), Some(vec![Change { a: 0..0, b: 0..3 }]));
}

#[test]
fn equal_sides_match_everywhere() {
	let a = b"abcdef";
	assert_eq!(matches(a, a, 0), Some((0..6).map(|i| (i, i)).collect()));
	assert_eq!(changed_runs(a, a, 0), Some(Vec::new()));
}

#[test]
fn disjoint_sides_are_one_change() {
	let (a, b) = (b"abc", b"xyzw");
	assert_eq!(matches(a, b, 7), Some(Vec::new()));
	assert_eq!(changed_runs(a, b, 7), Some(vec![Change { a: 0..3, b: 0..4 }]));
}

#[test]
fn budget_exceeded_gives_none() {
	let (a, b) = (b"abc", b"xyz");
	// Six edits are needed, so five is one short.
	assert_eq!(matches(a, b, 5), None);
	assert!(matches(a, b, 6).is_some());
	assert_eq!(changed_runs(b"pabcq", b"pxyzq", 5), None);
}

#[test]
fn shortest_script_keeps_the_common_subsequence() {
	let (a, b) = (b"ABCABBA", b"CBABAC");
	let m = match matches(a, b, 100) {
		Some(m)	=> m,
		None	=> panic!("budget of 100 refused"),
	};
	// The longest common subsequence has length four, so D = 13 - 8 = 5.
	assert_eq!(m.len(), 4);
	assert!(matches(a, b, 4).is_none());
	for w in m.windows(2) {
		assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1);
	}
	for (i, j) in &m {
		assert_eq!(a[*i], b[*j]);
	}
}

#[test]
fn changed_runs_cover_only_what_changed() {
	let a = ["one", "two", "three", "four"];
	let b = ["one", "2", "three", "four", "five"];
	assert_eq!(changed_runs(&a, &b, 10), Some(vec![
		Change { a: 1..2, b: 1..2 },
		Change { a: 4..4, b: 4..5 },
	]));
}
