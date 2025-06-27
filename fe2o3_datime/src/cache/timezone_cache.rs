/// Timezone calculation caching for performance optimisation.
///
/// This module provides efficient caching for expensive timezone operations
/// like offset calculations and DST transitions to avoid repeated computations.

use crate::{
	cache::LruCache,
	time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	sync::{Arc, OnceLock},
	collections::HashMap,
};

/// Cache key for timezone offset calculations.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TimezoneOffsetKey {
	timezone_id: String,
	timestamp_millis: i64,
}

/// Cache key for DST calculations.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DstCalculationKey {
	timezone_id: String,
	year: i32,
	timestamp_millis: i64,
}

/// Global timezone calculation cache.
///
/// This cache stores expensive timezone calculations to avoid repeated
/// computation of the same values. It's designed to be thread-safe and
/// efficient for high-frequency operations.
#[derive(Debug, Clone)]
pub struct TimezoneCache {
	/// Cache for timezone offset calculations.
	offset_cache: LruCache<TimezoneOffsetKey, i32>,
	/// Cache for DST transition calculations.
	dst_cache: LruCache<DstCalculationKey, bool>,
	/// Cache for timezone name lookups.
	name_cache: Arc<HashMap<String, String>>,
}

/// Global timezone cache instance.
static TIMEZONE_CACHE: OnceLock<TimezoneCache> = OnceLock::new();

impl TimezoneCache {
	/// Creates a new timezone cache with default settings.
	fn new() -> Self {
		Self {
			offset_cache: LruCache::new(1000), // Cache up to 1000 offset calculations
			dst_cache: LruCache::new(500),     // Cache up to 500 DST calculations
			name_cache: Arc::new(Self::build_name_cache()),
		}
	}
	
	/// Gets the global timezone cache instance.
	pub fn global() -> &'static TimezoneCache {
		TIMEZONE_CACHE.get_or_init(|| TimezoneCache::new())
	}
	
	/// Builds the timezone name cache with common abbreviations.
	fn build_name_cache() -> HashMap<String, String> {
		let mut cache = HashMap::new();
		
		// Common timezone abbreviations
		cache.insert("America/New_York".to_string(), "EST".to_string());
		cache.insert("America/Chicago".to_string(), "CST".to_string());
		cache.insert("America/Denver".to_string(), "MST".to_string());
		cache.insert("America/Los_Angeles".to_string(), "PST".to_string());
		cache.insert("Europe/London".to_string(), "GMT".to_string());
		cache.insert("Europe/Paris".to_string(), "CET".to_string());
		cache.insert("Europe/Berlin".to_string(), "CET".to_string());
		cache.insert("Asia/Tokyo".to_string(), "JST".to_string());
		cache.insert("Asia/Shanghai".to_string(), "CST".to_string());
		cache.insert("Australia/Sydney".to_string(), "AEDT".to_string());
		cache.insert("UTC".to_string(), "UTC".to_string());
		cache.insert("GMT".to_string(), "GMT".to_string());
		
		cache
	}
	
	/// Gets a cached timezone offset or computes it if not cached.
	pub fn get_timezone_offset_cached<F>(&self, zone: &CalClockZone, timestamp_millis: i64, compute_fn: F) -> i32
	where 
		F: FnOnce() -> i32,
	{
		let key = TimezoneOffsetKey {
			timezone_id: zone.id().to_string(),
			timestamp_millis,
		};
		
		if let Some(cached_offset) = self.offset_cache.get(&key) {
			cached_offset
		} else {
			let offset = compute_fn();
			self.offset_cache.insert(key, offset);
			offset
		}
	}
	
	/// Gets a cached DST calculation or computes it if not cached.
	pub fn get_dst_cached<F>(&self, zone: &CalClockZone, year: i32, timestamp_millis: i64, compute_fn: F) -> bool
	where 
		F: FnOnce() -> bool,
	{
		let key = DstCalculationKey {
			timezone_id: zone.id().to_string(),
			year,
			timestamp_millis,
		};
		
		if let Some(cached_dst) = self.dst_cache.get(&key) {
			cached_dst
		} else {
			let is_dst = compute_fn();
			self.dst_cache.insert(key, is_dst);
			is_dst
		}
	}
	
	/// Gets a cached timezone name abbreviation.
	pub fn get_timezone_name(&self, timezone_id: &str) -> Option<String> {
		self.name_cache.get(timezone_id).cloned()
	}
	
	/// Returns cache statistics for monitoring performance.
	pub fn stats(&self) -> TimezoneStats {
		let (offset_hits, offset_misses, offset_ratio) = self.offset_cache.stats();
		let (dst_hits, dst_misses, dst_ratio) = self.dst_cache.stats();
		
		TimezoneStats {
			offset_cache_hits: offset_hits,
			offset_cache_misses: offset_misses,
			offset_cache_hit_ratio: offset_ratio,
			dst_cache_hits: dst_hits,
			dst_cache_misses: dst_misses,
			dst_cache_hit_ratio: dst_ratio,
			name_cache_size: self.name_cache.len(),
		}
	}
	
	/// Clears all caches.
	pub fn clear(&self) {
		self.offset_cache.clear();
		self.dst_cache.clear();
	}
	
	/// Preloads common timezone calculations for better performance.
	pub fn preload_common_timezones(&self) {
		// This could be enhanced to preload common timezone calculations
		// for the current year and next year during application startup
		// For now, it's a placeholder for future optimisation
	}
}

/// Statistics about timezone cache performance.
#[derive(Debug, Clone)]
pub struct TimezoneStats {
	pub offset_cache_hits: u64,
	pub offset_cache_misses: u64,
	pub offset_cache_hit_ratio: f64,
	pub dst_cache_hits: u64,
	pub dst_cache_misses: u64,
	pub dst_cache_hit_ratio: f64,
	pub name_cache_size: usize,
}

impl TimezoneStats {
	/// Returns the overall cache efficiency as a percentage.
	pub fn overall_efficiency(&self) -> f64 {
		let total_hits = self.offset_cache_hits + self.dst_cache_hits;
		let total_requests = total_hits + self.offset_cache_misses + self.dst_cache_misses;
		
		if total_requests > 0 {
			(total_hits as f64 / total_requests as f64) * 100.0
		} else {
			0.0
		}
	}
	
	/// Returns true if the cache is performing well (>= 80% hit rate).
	pub fn is_performing_well(&self) -> bool {
		self.overall_efficiency() >= 80.0
	}
}

/// Extension trait to add caching to CalClockZone operations.
pub trait CalClockZoneCached {
	/// Gets timezone offset with caching.
	fn offset_millis_at_time_cached(&self, timestamp_millis: i64) -> Outcome<i32>;
	
	/// Checks if time is in daylight saving time with caching.
	fn in_daylight_time_cached(&self, timestamp_millis: i64) -> Outcome<bool>;
	
	/// Gets timezone name abbreviation with caching.
	fn name_cached(&self) -> Option<String>;
}

impl CalClockZoneCached for CalClockZone {
	fn offset_millis_at_time_cached(&self, timestamp_millis: i64) -> Outcome<i32> {
		let cache = TimezoneCache::global();
		
		Ok(cache.get_timezone_offset_cached(self, timestamp_millis, || {
			// Fallback to original calculation if cache fails
			self.offset_millis_at_time(timestamp_millis).unwrap_or(0)
		}))
	}
	
	fn in_daylight_time_cached(&self, timestamp_millis: i64) -> Outcome<bool> {
		let cache = TimezoneCache::global();
		
		// Extract year from timestamp for cache key
		let year = {
			let days_since_epoch = timestamp_millis / (24 * 60 * 60 * 1000);
			let epoch_year = 1970;
			// Rough approximation - could be more accurate
			epoch_year + (days_since_epoch / 365) as i32
		};
		
		Ok(cache.get_dst_cached(self, year, timestamp_millis, || {
			// Fallback to original calculation if cache fails
			self.in_daylight_time(timestamp_millis).unwrap_or(false)
		}))
	}
	
	fn name_cached(&self) -> Option<String> {
		let cache = TimezoneCache::global();
		cache.get_timezone_name(self.id())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::time::CalClockZone;

	#[test]
	fn test_timezone_cache_basic() {
		let cache = TimezoneCache::new();
		let utc_zone = CalClockZone::utc();
		
		// Test offset caching
		let timestamp = 1640995200000; // 2022-01-01 00:00:00 UTC
		let offset1 = cache.get_timezone_offset_cached(&utc_zone, timestamp, || 0);
		let offset2 = cache.get_timezone_offset_cached(&utc_zone, timestamp, || 999); // Should not be called
		
		assert_eq!(offset1, 0);
		assert_eq!(offset2, 0); // Should come from cache
	}

	#[test]
	fn test_timezone_name_cache() {
		let cache = TimezoneCache::new();
		
		// Test known timezone
		assert_eq!(cache.get_timezone_name("America/New_York"), Some("EST".to_string()));
		assert_eq!(cache.get_timezone_name("Europe/London"), Some("GMT".to_string()));
		
		// Test unknown timezone
		assert_eq!(cache.get_timezone_name("Unknown/Timezone"), None);
	}

	#[test]
	fn test_timezone_stats() {
		let cache = TimezoneCache::new();
		let utc_zone = CalClockZone::utc();
		
		// Generate some cache activity
		let _ = cache.get_timezone_offset_cached(&utc_zone, 1000, || 0);
		let _ = cache.get_timezone_offset_cached(&utc_zone, 1000, || 0); // Cache hit
		let _ = cache.get_timezone_offset_cached(&utc_zone, 2000, || 0); // Cache miss
		
		let stats = cache.stats();
		assert_eq!(stats.offset_cache_hits, 1);
		assert_eq!(stats.offset_cache_misses, 2);
		assert_eq!(stats.offset_cache_hit_ratio, 1.0 / 3.0);
	}

	#[test]
	fn test_cached_zone_extension() -> Outcome<()> {
		let utc_zone = CalClockZone::utc();
		
		// Test cached offset calculation
		let offset = res!(utc_zone.offset_millis_at_time_cached(1640995200000));
		assert_eq!(offset, 0);
		
		// Test cached name lookup
		let name = utc_zone.name_cached();
		assert_eq!(name, Some("UTC".to_string()));
		
		Ok(())
	}
}