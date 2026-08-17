//! A validator that remembers its answers.
//!
//! Three LRU caches, one each for CalClock, CalendarDate and ClockTime, keyed
//! on the fields that decide the outcome.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClock,
    validation::{CalClockValidator, ValidationResult},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    hash::Hash,
};

/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::validation::{CachedValidator, CalClockValidator};
///
/// let base_validator = CalClockValidator::new();
/// let mut cached_validator = CachedValidator::new(base_validator);
///
/// // First validation - computed and cached
/// let result1 = cached_validator.validate_calclock(&some_calclock);
///
/// // Second validation of same CalClock - returned from cache
/// let result2 = cached_validator.validate_calclock(&some_calclock);
/// ```
#[derive(Debug)]
pub struct CachedValidator {
    validator: CalClockValidator,
    calclock_cache: ValidationCache<CalClockKey, ValidationResult>,
    date_cache: ValidationCache<DateKey, ValidationResult>,
    time_cache: ValidationCache<TimeKey, ValidationResult>,
    cache_hits: u64,
    cache_misses: u64,
}

impl CachedValidator {
    pub fn new(validator: CalClockValidator) -> Self {
        Self {
            validator,
            calclock_cache: ValidationCache::new(1000),
            date_cache: ValidationCache::new(1000),
            time_cache: ValidationCache::new(1000),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn with_cache_sizes(
        validator: CalClockValidator,
        calclock_cache_size: usize,
        date_cache_size: usize,
        time_cache_size: usize,
    ) -> Self {
        Self {
            validator,
            calclock_cache: ValidationCache::new(calclock_cache_size),
            date_cache: ValidationCache::new(date_cache_size),
            time_cache: ValidationCache::new(time_cache_size),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn validate_calclock(&mut self, calclock: &CalClock) -> ValidationResult {
        let key = CalClockKey::from_calclock(calclock);
        
        if let Some(cached_result) = self.calclock_cache.get(&key) {
            self.cache_hits += 1;
            return cached_result.clone();
        }

        self.cache_misses += 1;
        let result = self.validator.validate_calclock(calclock);
        self.calclock_cache.insert(key, result.clone());
        result
    }

    pub fn validate_date(&mut self, date: &CalendarDate) -> ValidationResult {
        let key = DateKey::from_date(date);
        
        if let Some(cached_result) = self.date_cache.get(&key) {
            self.cache_hits += 1;
            return cached_result.clone();
        }

        self.cache_misses += 1;
        let result = self.validator.validate_date(date);
        self.date_cache.insert(key, result.clone());
        result
    }

    pub fn validate_time(&mut self, time: &ClockTime) -> ValidationResult {
        let key = TimeKey::from_time(time);
        
        if let Some(cached_result) = self.time_cache.get(&key) {
            self.cache_hits += 1;
            return cached_result.clone();
        }

        self.cache_misses += 1;
        let result = self.validator.validate_time(time);
        self.time_cache.insert(key, result.clone());
        result
    }

    pub fn is_valid_calclock(&mut self, calclock: &CalClock) -> bool {
        self.validate_calclock(calclock).is_ok()
    }

    pub fn is_valid_date(&mut self, date: &CalendarDate) -> bool {
        self.validate_date(date).is_ok()
    }

    pub fn is_valid_time(&mut self, time: &ClockTime) -> bool {
        self.validate_time(time).is_ok()
    }

    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.cache_hits,
            misses: self.cache_misses,
            calclock_cache_size: self.calclock_cache.len(),
            date_cache_size: self.date_cache.len(),
            time_cache_size: self.time_cache.len(),
            calclock_cache_capacity: self.calclock_cache.capacity(),
            date_cache_capacity: self.date_cache.capacity(),
            time_cache_capacity: self.time_cache.capacity(),
        }
    }

    /// A fraction between zero and one, not a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    pub fn clear_cache(&mut self) {
        self.calclock_cache.clear();
        self.date_cache.clear();
        self.time_cache.clear();
        self.cache_hits = 0;
        self.cache_misses = 0;
    }

    pub fn validator(&self) -> &CalClockValidator {
        &self.validator
    }

    pub fn validator_mut(&mut self) -> &mut CalClockValidator {
        // Clear cache when validator is modified
        self.clear_cache();
        &mut self.validator
    }
}

#[derive(Debug)]
pub struct ValidationCache<K, V> {
    data: HashMap<K, CacheEntry<V>>,
    capacity: usize,
    access_counter: u64,
}

impl<K: Hash + Eq + Clone, V: Clone> ValidationCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: HashMap::new(),
            capacity,
            access_counter: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.data.get_mut(key) {
            entry.last_accessed = self.access_counter;
            self.access_counter += 1;
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// Evicts the least recently used entry when the cache is full.
    pub fn insert(&mut self, key: K, value: V) {
        if self.data.len() >= self.capacity && !self.data.contains_key(&key) {
            self.evict_lru();
        }

        let entry = CacheEntry {
            value,
            last_accessed: self.access_counter,
        };
        
        self.data.insert(key, entry);
        self.access_counter += 1;
    }

    fn evict_lru(&mut self) {
        if let Some((key_to_remove, _)) = self.data
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(k, v)| (k.clone(), v.last_accessed))
        {
            self.data.remove(&key_to_remove);
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clear(&mut self) {
        self.data.clear();
        self.access_counter = 0;
    }
}

#[derive(Debug, Clone)]
struct CacheEntry<V> {
    value: V,
    last_accessed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CalClockKey {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
    // Note: We don't include timezone in the key for simplicity
    // In a production system, you'd want to include timezone info
}

impl CalClockKey {
    fn from_calclock(calclock: &CalClock) -> Self {
        Self {
            year: calclock.year(),
            month: calclock.month(),
            day: calclock.day(),
            hour: calclock.hour(),
            minute: calclock.minute(),
            second: calclock.second(),
            nanosecond: calclock.nanosecond(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DateKey {
    year: i32,
    month: u8,
    day: u8,
}

impl DateKey {
    fn from_date(date: &CalendarDate) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
            day: date.day(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TimeKey {
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

impl TimeKey {
    fn from_time(time: &ClockTime) -> Self {
        Self {
            hour: time.hour().of(),
            minute: time.minute().of(),
            second: time.second().of(),
            nanosecond: time.nanosecond().of(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub calclock_cache_size: usize,
    pub date_cache_size: usize,
    pub time_cache_size: usize,
    pub calclock_cache_capacity: usize,
    pub date_cache_capacity: usize,
    pub time_cache_capacity: usize,
}

impl CacheStats {
    /// A fraction between zero and one, not a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    pub fn total_operations(&self) -> u64 {
        self.hits + self.misses
    }

    pub fn format(&self) -> String {
        format!(
            "Cache Stats: {:.1}% hit rate ({} hits, {} misses)\n\
             CalClock cache: {}/{} entries\n\
             Date cache: {}/{} entries\n\
             Time cache: {}/{} entries",
            self.hit_rate() * 100.0,
            self.hits,
            self.misses,
            self.calclock_cache_size,
            self.calclock_cache_capacity,
            self.date_cache_size,
            self.date_cache_capacity,
            self.time_cache_size,
            self.time_cache_capacity
        )
    }
}