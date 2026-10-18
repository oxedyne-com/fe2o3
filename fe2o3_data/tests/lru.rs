//! Integration tests for the byte-capped least-recently-used map.

use oxedyne_fe2o3_data::lru::SizedLru;


#[test]
fn the_least_recently_used_goes_first_00() {
	let mut m: SizedLru<u32, &str> = SizedLru::new(30);
	assert!(m.insert(1, "one", 10));
	assert!(m.insert(2, "two", 10));
	assert!(m.insert(3, "three", 10));
	// Reading 1 makes 2 the oldest.
	assert_eq!(m.get(&1), Some(&"one"));
	assert!(m.insert(4, "four", 10));
	assert!(m.contains(&1));
	assert!(!m.contains(&2));
	assert_eq!(m.bytes(), 30);
	assert_eq!(m.evicted(), 1);
}

#[test]
fn a_large_entry_evicts_as_many_as_it_needs_01() {
	let mut m: SizedLru<u32, u32> = SizedLru::new(30);
	for k in 0..3 {
		assert!(m.insert(k, k, 10));
	}
	assert!(m.insert(9, 9, 25));
	assert_eq!(m.len(), 1);
	assert_eq!(m.bytes(), 25);
	assert_eq!(m.evicted(), 3);
}

#[test]
fn an_entry_over_the_ceiling_is_refused_and_evicts_nothing_02() {
	let mut m: SizedLru<u32, u32> = SizedLru::new(30);
	assert!(m.insert(1, 1, 10));
	assert!(!m.insert(2, 2, 31));
	assert!(m.contains(&1));
	assert_eq!(m.bytes(), 10);
}

#[test]
fn a_replacement_counts_its_own_size_once_03() {
	let mut m: SizedLru<u32, u32> = SizedLru::new(30);
	assert!(m.insert(1, 1, 20));
	assert!(m.insert(1, 2, 25));
	assert_eq!(m.len(), 1);
	assert_eq!(m.bytes(), 25);
	assert_eq!(m.peek(&1), Some(&2));
	assert_eq!(m.remove(&1), Some(2));
	assert_eq!(m.bytes(), 0);
	assert!(m.is_empty());
}

#[test]
fn retain_removes_without_counting_an_eviction_04() {
	let mut m: SizedLru<String, u32> = SizedLru::new(3);
	assert!(m.insert("/a".to_string(), 1, 1));
	assert!(m.insert("/a/b".to_string(), 2, 1));
	assert!(m.insert("/ab".to_string(), 3, 1));
	// Every key at or under "/a", but not the sibling "/ab".
	let gone = m.retain(|k, _| k != "/a" && !k.starts_with("/a/"));
	assert_eq!(gone, 2);
	assert_eq!(m.len(), 1);
	assert_eq!(m.bytes(), 1);
	assert_eq!(m.evicted(), 0);
	assert_eq!(m.peek("/ab"), Some(&3));
	// The order forgot the removed keys too: two inserts fill the room left without evicting "/ab"
	// early, and a third evicts it as the oldest.
	assert!(m.insert("/c".to_string(), 4, 1));
	assert!(m.insert("/d".to_string(), 5, 1));
	assert_eq!(m.evicted(), 0);
	assert!(m.insert("/e".to_string(), 6, 1));
	assert_eq!(m.evicted(), 1);
	assert!(!m.contains("/ab"));
	assert!(m.contains("/c"));
}
