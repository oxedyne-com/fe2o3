/// Performance optimisation caching infrastructure.
///
/// This module provides efficient caching mechanisms to improve performance
/// across the fe2o3_datime library. It includes LRU caches for timezone
/// calculations, string interning for format patterns, and result memoisation.

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	hash::Hash,
	sync::{Arc, RwLock},
};

pub mod lru;
pub mod string_intern;
pub mod timezone_cache;

/// A thread-safe LRU cache implementation.
///
/// This cache provides O(1) access time for cached values and automatically
/// evicts least recently used entries when capacity is exceeded.
#[derive(Debug)]
pub struct LruCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	inner: Arc<RwLock<lru::LruCacheInner<K, V>>>,
}

impl<K, V> LruCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	/// Creates a new LRU cache with the specified capacity.
	pub fn new(capacity: usize) -> Self {
		Self {
			inner: Arc::new(RwLock::new(lru::LruCacheInner::new(capacity))),
		}
	}
	
	/// Gets a value from the cache if present.
	pub fn get(&self, key: &K) -> Option<V> {
		if let Ok(mut cache) = self.inner.write() {
			cache.get(key)
		} else {
			None
		}
	}
	
	/// Inserts a value into the cache.
	pub fn insert(&self, key: K, value: V) {
		if let Ok(mut cache) = self.inner.write() {
			cache.insert(key, value);
		}
	}
	
	/// Returns the current size of the cache.
	pub fn len(&self) -> usize {
		if let Ok(cache) = self.inner.read() {
			cache.len()
		} else {
			0
		}
	}
	
	/// Returns true if the cache is empty.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
	
	/// Clears all entries from the cache.
	pub fn clear(&self) {
		if let Ok(mut cache) = self.inner.write() {
			cache.clear();
		}
	}
	
	/// Returns cache statistics (hits, misses, hit ratio).
	pub fn stats(&self) -> (u64, u64, f64) {
		if let Ok(cache) = self.inner.read() {
			cache.stats()
		} else {
			(0, 0, 0.0)
		}
	}
}

impl<K, V> Clone for LruCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	fn clone(&self) -> Self {
		Self {
			inner: Arc::clone(&self.inner),
		}
	}
}

/// A thread-safe string interning system.
///
/// String interning reduces memory usage and improves performance by
/// ensuring that equal strings share the same memory allocation.
#[derive(Debug, Clone)]
pub struct StringInterner {
	inner: Arc<RwLock<HashMap<String, Arc<String>>>>,
}

impl StringInterner {
	/// Creates a new string interner.
	pub fn new() -> Self {
		Self {
			inner: Arc::new(RwLock::new(HashMap::new())),
		}
	}
	
	/// Interns a string, returning a reference-counted handle.
	pub fn intern(&self, s: &str) -> Arc<String> {
		if let Ok(cache) = self.inner.read() {
			if let Some(interned) = cache.get(s) {
				return Arc::clone(interned);
			}
		}
		
		// Need to insert - upgrade to write lock
		if let Ok(mut cache) = self.inner.write() {
			// Double-check in case another thread inserted while we waited
			if let Some(interned) = cache.get(s) {
				return Arc::clone(interned);
			}
			
			let arc_string = Arc::new(s.to_string());
			cache.insert(s.to_string(), Arc::clone(&arc_string));
			arc_string
		} else {
			// Fallback if lock fails
			Arc::new(s.to_string())
		}
	}
	
	/// Returns the number of interned strings.
	pub fn len(&self) -> usize {
		if let Ok(cache) = self.inner.read() {
			cache.len()
		} else {
			0
		}
	}
	
	/// Returns true if no strings are interned.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
	
	/// Clears all interned strings.
	pub fn clear(&self) {
		if let Ok(mut cache) = self.inner.write() {
			cache.clear();
		}
	}
}

impl Default for StringInterner {
	fn default() -> Self {
		Self::new()
	}
}

/// A memoisation cache for expensive function results.
///
/// This cache stores the results of expensive computations to avoid
/// recalculating them for the same inputs.
#[derive(Debug)]
pub struct MemoCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	cache: LruCache<K, V>,
	hits: Arc<RwLock<u64>>,
	misses: Arc<RwLock<u64>>,
}

impl<K, V> MemoCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	/// Creates a new memoisation cache with the specified capacity.
	pub fn new(capacity: usize) -> Self {
		Self {
			cache: LruCache::new(capacity),
			hits: Arc::new(RwLock::new(0)),
			misses: Arc::new(RwLock::new(0)),
		}
	}
	
	/// Gets a cached result or computes it using the provided function.
	pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> V 
	where 
		F: FnOnce() -> V,
	{
		if let Some(cached_value) = self.cache.get(&key) {
			// Cache hit
			if let Ok(mut hits) = self.hits.write() {
				*hits += 1;
			}
			cached_value
		} else {
			// Cache miss - compute and store
			if let Ok(mut misses) = self.misses.write() {
				*misses += 1;
			}
			let computed_value = compute_fn();
			self.cache.insert(key, computed_value.clone());
			computed_value
		}
	}
	
	/// Returns cache statistics (hits, misses, hit ratio).
	pub fn stats(&self) -> (u64, u64, f64) {
		let hits = if let Ok(h) = self.hits.read() { *h } else { 0 };
		let misses = if let Ok(m) = self.misses.read() { *m } else { 0 };
		let total = hits + misses;
		let hit_ratio = if total > 0 { hits as f64 / total as f64 } else { 0.0 };
		(hits, misses, hit_ratio)
	}
	
	/// Clears the cache and resets statistics.
	pub fn clear(&self) {
		self.cache.clear();
		if let Ok(mut hits) = self.hits.write() {
			*hits = 0;
		}
		if let Ok(mut misses) = self.misses.write() {
			*misses = 0;
		}
	}
}

impl<K, V> Clone for MemoCache<K, V> 
where 
	K: Hash + Eq + Clone,
	V: Clone,
{
	fn clone(&self) -> Self {
		Self {
			cache: self.cache.clone(),
			hits: Arc::clone(&self.hits),
			misses: Arc::clone(&self.misses),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_lru_cache_basic_operations() {
		let cache = LruCache::new(2);
		
		// Test insertion and retrieval
		cache.insert("key1".to_string(), 42);
		cache.insert("key2".to_string(), 84);
		
		assert_eq!(cache.get(&"key1".to_string()), Some(42));
		assert_eq!(cache.get(&"key2".to_string()), Some(84));
		assert_eq!(cache.len(), 2);
		
		// Test LRU eviction
		cache.insert("key3".to_string(), 126);
		assert_eq!(cache.len(), 2);
		assert_eq!(cache.get(&"key1".to_string()), None); // Should be evicted
		assert_eq!(cache.get(&"key2".to_string()), Some(84));
		assert_eq!(cache.get(&"key3".to_string()), Some(126));
	}

	#[test]
	fn test_string_interner() {
		let interner = StringInterner::new();
		
		let str1 = interner.intern("hello");
		let str2 = interner.intern("world");
		let str3 = interner.intern("hello"); // Should reuse str1
		
		assert_eq!(*str1, "hello");
		assert_eq!(*str2, "world");
		assert!(Arc::ptr_eq(&str1, &str3)); // Same allocation
		assert_eq!(interner.len(), 2); // Only 2 unique strings
	}

	#[test]
	fn test_memo_cache() {
		let cache = MemoCache::new(10);
		
		// First call should compute
		let result1 = cache.get_or_compute("key1".to_string(), || 42);
		assert_eq!(result1, 42);
		
		// Second call should use cache
		let result2 = cache.get_or_compute("key1".to_string(), || 99); // Different value, should not be used
		assert_eq!(result2, 42); // Should return cached value
		
		let (hits, misses, hit_ratio) = cache.stats();
		assert_eq!(hits, 1);
		assert_eq!(misses, 1);
		assert_eq!(hit_ratio, 0.5);
	}
}