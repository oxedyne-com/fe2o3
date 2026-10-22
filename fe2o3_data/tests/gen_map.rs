//! Integration tests for the generation-swept map.

use oxedyne_fe2o3_data::gen_map::GenMap;


#[test]
fn an_entry_untouched_for_two_generations_goes_at_the_sweep() {
	let mut m: GenMap<u32, &str> = GenMap::unbounded();
	m.begin();
	assert!(m.insert(1, "kept", 10));
	assert!(m.insert(2, "lost", 10));
	m.sweep();
	assert_eq!(m.len(), 2);

	// The next compile reads one of them and not the other. Both are within the last two generations.
	m.begin();
	assert_eq!(m.get(&1), Some(&"kept"));
	assert_eq!(m.sweep(), 0);
	assert_eq!(m.len(), 2);

	// The compile after reads neither generation's untouched entry: 2 was last touched two compiles ago.
	m.begin();
	assert_eq!(m.get(&1), Some(&"kept"));
	assert_eq!(m.sweep(), 1);
	assert!(m.contains(&1));
	assert!(!m.contains(&2));
	assert_eq!(m.bytes(), 10);
}

#[test]
fn a_read_in_every_compile_keeps_an_entry_for_ever() {
	let mut m: GenMap<u32, u32> = GenMap::unbounded();
	m.begin();
	m.insert(7, 70, 4);
	for _ in 0..20 {
		m.begin();
		assert_eq!(m.get(&7), Some(&70));
		m.sweep();
	}
	assert_eq!(m.len(), 1);
	assert_eq!(m.bytes(), 4);
}

#[test]
fn a_look_does_not_keep_an_entry_from_the_sweep() {
	let mut m: GenMap<u32, u32> = GenMap::unbounded();
	m.begin();
	m.insert(1, 10, 1);
	m.begin();
	m.begin();
	assert_eq!(m.peek(&1), Some(&10));
	assert_eq!(m.sweep(), 1);
	assert!(m.is_empty());
}

#[test]
fn the_ceiling_stops_inserts_and_evicts_nothing() {
	let mut m: GenMap<u32, u32> = GenMap::new(100);
	m.begin();
	assert!(m.insert(1, 1, 60));
	assert!(m.insert(2, 2, 40));
	assert_eq!(m.bytes(), 100);

	// Full: a third entry is refused, and the two held are still there to read.
	assert!(!m.insert(3, 3, 1));
	assert_eq!(m.refused(), 1);
	assert_eq!(m.len(), 2);
	assert_eq!(m.get(&1), Some(&1));
	assert_eq!(m.get(&2), Some(&2));
	assert_eq!(m.get(&3), None);

	// An entry larger than the whole ceiling is refused on an empty map as well.
	let mut e: GenMap<u32, u32> = GenMap::new(100);
	e.begin();
	assert!(!e.insert(1, 1, 101));
	assert!(e.is_empty());
	assert_eq!(e.bytes(), 0);
}

#[test]
fn a_sweep_makes_room_and_inserts_resume() {
	let mut m: GenMap<u32, u32> = GenMap::new(100);
	m.begin();
	assert!(m.insert(1, 1, 100));
	assert!(!m.insert(2, 2, 1));
	m.sweep();
	m.begin();
	m.sweep();
	m.begin();
	assert_eq!(m.sweep(), 1);
	assert_eq!(m.bytes(), 0);
	assert!(m.insert(2, 2, 100));
}

#[test]
fn a_replacement_is_counted_by_its_growth_and_may_be_refused() {
	let mut m: GenMap<u32, &str> = GenMap::new(100);
	m.begin();
	assert!(m.insert(1, "old", 70));
	assert!(m.insert(2, "other", 30));
	// The same key at the same size fits, though the map is full.
	assert!(m.insert(1, "same", 70));
	assert_eq!(m.bytes(), 100);
	// A shrunken replacement frees the difference.
	assert!(m.insert(1, "small", 20));
	assert_eq!(m.bytes(), 50);
	// A grown one that would pass the ceiling is refused, and the old entry stands, touched.
	assert!(!m.insert(1, "big", 71));
	assert_eq!(m.peek(&1), Some(&"small"));
	assert_eq!(m.bytes(), 50);
	m.begin();
	m.begin();
	assert!(!m.insert(1, "big", 71));
	assert_eq!(m.sweep(), 1);
	assert!(m.contains(&1));
	assert!(!m.contains(&2));
}

#[test]
fn a_ceiling_set_below_the_held_bytes_evicts_nothing_and_refuses_until_it_is_met() {
	let mut m: GenMap<u32, u32> = GenMap::new(100);
	m.begin();
	m.insert(1, 1, 80);
	m.set_ceiling(10);
	assert_eq!(m.len(), 1);
	assert_eq!(m.get(&1), Some(&1));
	assert!(!m.insert(2, 2, 1));
	assert!(!m.insert(1, 1, 80));
	m.begin();
	m.begin();
	m.sweep();
	assert!(m.insert(2, 2, 10));
}

#[test]
fn clear_empties_the_map_and_keeps_its_generation_and_ceiling() {
	let mut m: GenMap<String, Vec<u8>> = GenMap::new(64);
	m.begin();
	m.begin();
	m.insert("a".to_string(), vec![0; 8], 8);
	assert!(!m.insert("b".to_string(), vec![0; 65], 65));
	m.clear();
	assert!(m.is_empty());
	assert_eq!(m.bytes(), 0);
	assert_eq!(m.epoch(), 2);
	assert_eq!(m.ceiling(), 64);
	assert_eq!(m.refused(), 1);
	// A borrowed key reads an owned one.
	assert!(m.insert("c".to_string(), vec![1], 1));
	assert_eq!(m.get("c"), Some(&vec![1]));
}

#[test]
fn an_unbounded_map_counts_sizes_without_overflowing() {
	let mut m: GenMap<u32, u32> = GenMap::default();
	m.begin();
	assert!(m.insert(1, 1, usize::MAX - 1));
	assert!(!m.insert(2, 2, 2));
	assert_eq!(m.refused(), 1);
	assert!(m.insert(2, 2, 1));
	assert_eq!(m.bytes(), usize::MAX);
}
