/// Safe LRU (Least Recently Used) cache implementation.
///
/// This module provides a safe LRU cache implementation without unsafe code,
/// using Vec-based storage with O(n) operations for simplicity and safety.

use std::hash::Hash;

/// Safe LRU cache implementation.
///
/// This implementation prioritises safety over performance by avoiding
/// unsafe code. It uses Vec-based storage which results in O(n) operations
/// for some methods, but ensures memory safety.
#[derive(Debug)]
pub struct LruCacheInner<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	/// Storage for key-value pairs in LRU order (most recent first).
	items: Vec<(K, V)>,
	/// Maximum capacity of the cache.
	capacity: usize,
	/// Hit counter for statistics.
	hits: u64,
	/// Miss counter for statistics.
	misses: u64,
}

impl<K, V> LruCacheInner<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	/// Creates a new LRU cache with the specified capacity.
	pub fn new(capacity: usize) -> Self {
		Self {
			items: Vec::with_capacity(capacity),
			capacity,
			hits: 0,
			misses: 0,
		}
	}
	
	/// Gets a value from the cache, moving it to the front if found.
	pub fn get(&mut self, key: &K) -> Option<V> {
		if let Some(pos) = self.find_key_position(key) {
			self.hits += 1;
			let (_, value) = self.items[pos].clone();
			
			// Move to front (most recent)
			let item = self.items.remove(pos);
			self.items.insert(0, item);
			
			Some(value)
		} else {
			self.misses += 1;
			None
		}
	}
	
	/// Inserts a key-value pair into the cache.
	pub fn insert(&mut self, key: K, value: V) {
		// Check if key already exists
		if let Some(pos) = self.find_key_position(&key) {
			// Update existing entry and move to front
			self.items.remove(pos);
			self.items.insert(0, (key, value));
		} else {
			// Insert new entry at front
			self.items.insert(0, (key, value));
			
			// Remove oldest entry if over capacity
			if self.items.len() > self.capacity {
				self.items.pop();
			}
		}
	}
	
	/// Finds the position of a key in the items vector.
	fn find_key_position(&self, key: &K) -> Option<usize> {
		self.items.iter().position(|(k, _)| k == key)
	}
	
	/// Returns the number of items in the cache.
	pub fn len(&self) -> usize {
		self.items.len()
	}
	
	/// Returns true if the cache is empty.
	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}
	
	/// Clears all items from the cache.
	pub fn clear(&mut self) {
		self.items.clear();
		self.hits = 0;
		self.misses = 0;
	}
	
	/// Returns cache statistics (hits, misses, hit ratio).
	pub fn stats(&self) -> (u64, u64, f64) {
		let total = self.hits + self.misses;
		let hit_ratio = if total > 0 { 
			self.hits as f64 / total as f64 
		} else { 
			0.0 
		};
		(self.hits, self.misses, hit_ratio)
	}
	
	/// Returns an iterator over the cache items in LRU order.
	pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
		self.items.iter().map(|(k, v)| (k, v))
	}
	
	/// Returns true if the cache contains the specified key.
	pub fn contains_key(&self, key: &K) -> bool {
		self.find_key_position(key).is_some()
	}
	
	/// Removes a key from the cache if present.
	pub fn remove(&mut self, key: &K) -> Option<V> {
		if let Some(pos) = self.find_key_position(key) {
			let (_, value) = self.items.remove(pos);
			Some(value)
		} else {
			None
		}
	}
	
	/// Returns the capacity of the cache.
	pub fn capacity(&self) -> usize {
		self.capacity
	}
	
	/// Resizes the cache capacity.
	pub fn resize(&mut self, new_capacity: usize) {
		self.capacity = new_capacity;
		
		// Trim items if new capacity is smaller
		if self.items.len() > new_capacity {
			self.items.truncate(new_capacity);
		}
	}
	
	/// Peek at a value without updating LRU order.
	pub fn peek(&self, key: &K) -> Option<&V> {
		self.find_key_position(key)
			.map(|pos| &self.items[pos].1)
	}
	
	/// Gets the least recently used item without removing it.
	pub fn peek_lru(&self) -> Option<(&K, &V)> {
		self.items.last().map(|(k, v)| (k, v))
	}
	
	/// Gets the most recently used item without removing it.
	pub fn peek_mru(&self) -> Option<(&K, &V)> {
		self.items.first().map(|(k, v)| (k, v))
	}
}

impl<K, V> Clone for LruCacheInner<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	fn clone(&self) -> Self {
		Self {
			items: self.items.clone(),
			capacity: self.capacity,
			hits: self.hits,
			misses: self.misses,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_lru_cache_inner_basic() {
		let mut cache = LruCacheInner::new(2);
		
		// Test insertion
		cache.insert(1, "one");
		cache.insert(2, "two");
		assert_eq!(cache.len(), 2);
		
		// Test retrieval
		assert_eq!(cache.get(&1), Some("one"));
		assert_eq!(cache.get(&2), Some("two"));
		assert_eq!(cache.get(&3), None);
		
		// Test LRU eviction
		cache.insert(3, "three");
		assert_eq!(cache.len(), 2);
		
		// After inserting 3, and getting 1 and 2, the order should be [2, 1]
		// So when we insert 3, it should evict the LRU item
		assert!(cache.contains_key(&2));
		assert!(cache.contains_key(&3));
	}

	#[test]
	fn test_lru_cache_inner_update() {
		let mut cache = LruCacheInner::new(2);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		
		// Update existing key
		cache.insert(1, "ONE");
		assert_eq!(cache.get(&1), Some("ONE"));
		assert_eq!(cache.len(), 2);
		
		// Key 1 should now be most recent
		cache.insert(3, "three");
		assert!(cache.contains_key(&1)); // Should still be present
		assert!(cache.contains_key(&3)); // Should be present
		assert_eq!(cache.len(), 2);
	}

	#[test]
	fn test_lru_cache_inner_stats() {
		let mut cache = LruCacheInner::new(2);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		
		// Generate some hits and misses
		let _ = cache.get(&1); // hit
		let _ = cache.get(&2); // hit
		let _ = cache.get(&3); // miss
		let _ = cache.get(&1); // hit
		
		let (hits, misses, hit_ratio) = cache.stats();
		assert_eq!(hits, 3);
		assert_eq!(misses, 1);
		assert_eq!(hit_ratio, 0.75);
	}

	#[test]
	fn test_lru_cache_inner_clear() {
		let mut cache = LruCacheInner::new(2);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		assert_eq!(cache.len(), 2);
		
		cache.clear();
		assert_eq!(cache.len(), 0);
		assert!(cache.is_empty());
		
		let (hits, misses, hit_ratio) = cache.stats();
		assert_eq!(hits, 0);
		assert_eq!(misses, 0);
		assert_eq!(hit_ratio, 0.0);
	}

	#[test]
	fn test_lru_cache_peek_operations() {
		let mut cache = LruCacheInner::new(3);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		cache.insert(3, "three");
		
		// Peek should not affect LRU order
		assert_eq!(cache.peek(&2), Some(&"two"));
		
		// MRU should be 3 (most recently inserted)
		assert_eq!(cache.peek_mru(), Some((&3, &"three")));
		
		// LRU should be 1 (least recently used)
		assert_eq!(cache.peek_lru(), Some((&1, &"one")));
	}

	#[test]
	fn test_lru_cache_resize() {
		let mut cache = LruCacheInner::new(3);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		cache.insert(3, "three");
		assert_eq!(cache.len(), 3);
		
		// Resize to smaller capacity
		cache.resize(2);
		assert_eq!(cache.capacity(), 2);
		assert_eq!(cache.len(), 2); // Should have trimmed one item
		
		// Resize to larger capacity
		cache.resize(5);
		assert_eq!(cache.capacity(), 5);
		assert_eq!(cache.len(), 2); // Length should remain the same
	}

	#[test]
	fn test_lru_cache_remove() {
		let mut cache = LruCacheInner::new(3);
		
		cache.insert(1, "one");
		cache.insert(2, "two");
		cache.insert(3, "three");
		
		// Remove existing key
		assert_eq!(cache.remove(&2), Some("two"));
		assert_eq!(cache.len(), 2);
		assert!(!cache.contains_key(&2));
		
		// Remove non-existing key
		assert_eq!(cache.remove(&4), None);
		assert_eq!(cache.len(), 2);
	}
}