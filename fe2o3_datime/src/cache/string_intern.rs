/// String interning system for performance optimisation.
///
/// This module provides efficient string interning to reduce memory usage
/// and improve performance by ensuring equal strings share the same allocation.

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	sync::{Arc, RwLock, OnceLock},
};

/// Global string interner for commonly used strings in datetime formatting.
///
/// This includes month names, day names, timezone abbreviations, format patterns,
/// and other frequently repeated strings to reduce memory allocations.
#[derive(Debug)]
pub struct GlobalStringInterner {
	/// Cache for month names (short and long).
	month_names: RwLock<HashMap<String, Arc<String>>>,
	/// Cache for day names (short and long).
	day_names: RwLock<HashMap<String, Arc<String>>>,
	/// Cache for timezone names and abbreviations.
	timezone_names: RwLock<HashMap<String, Arc<String>>>,
	/// Cache for format patterns.
	format_patterns: RwLock<HashMap<String, Arc<String>>>,
	/// Cache for general strings.
	general_strings: RwLock<HashMap<String, Arc<String>>>,
}

/// Global interner instance.
static GLOBAL_INTERNER: OnceLock<GlobalStringInterner> = OnceLock::new();

impl GlobalStringInterner {
	/// Creates a new global string interner with preloaded common strings.
	fn new() -> Self {
		let mut interner = Self {
			month_names: RwLock::new(HashMap::new()),
			day_names: RwLock::new(HashMap::new()),
			timezone_names: RwLock::new(HashMap::new()),
			format_patterns: RwLock::new(HashMap::new()),
			general_strings: RwLock::new(HashMap::new()),
		};
		
		interner.preload_common_strings();
		interner
	}
	
	/// Gets the global string interner instance.
	pub fn global() -> &'static GlobalStringInterner {
		GLOBAL_INTERNER.get_or_init(|| GlobalStringInterner::new())
	}
	
	/// Preloads commonly used strings to improve performance.
	fn preload_common_strings(&mut self) {
		// Preload month names
		let month_names = [
			"January", "February", "March", "April", "May", "June",
			"July", "August", "September", "October", "November", "December",
			"Jan", "Feb", "Mar", "Apr", "May", "Jun",
			"Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
		];
		
		if let Ok(mut cache) = self.month_names.write() {
			for name in &month_names {
				cache.insert(name.to_string(), Arc::new(name.to_string()));
			}
		}
		
		// Preload day names
		let day_names = [
			"Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
			"Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun",
		];
		
		if let Ok(mut cache) = self.day_names.write() {
			for name in &day_names {
				cache.insert(name.to_string(), Arc::new(name.to_string()));
			}
		}
		
		// Preload timezone names
		let timezone_names = [
			"UTC", "GMT", "EST", "CST", "MST", "PST", "EDT", "CDT", "MDT", "PDT",
			"CET", "EET", "JST", "AEDT", "AEST", "BST", "IST",
		];
		
		if let Ok(mut cache) = self.timezone_names.write() {
			for name in &timezone_names {
				cache.insert(name.to_string(), Arc::new(name.to_string()));
			}
		}
		
		// Preload common format patterns
		let format_patterns = [
			"yyyy-MM-dd", "dd/MM/yyyy", "MM/dd/yyyy", "yyyy/MM/dd",
			"HH:mm:ss", "h:mm:ss a", "HH:mm", "h:mm a",
			"yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd HH:mm:ss",
			"EEEE, MMMM d, yyyy", "EEE, MMM d, yyyy",
		];
		
		if let Ok(mut cache) = self.format_patterns.write() {
			for pattern in &format_patterns {
				cache.insert(pattern.to_string(), Arc::new(pattern.to_string()));
			}
		}
		
		// Preload general strings
		let general_strings = [
			"AM", "PM", "am", "pm", "a.m.", "p.m.",
			"CE", "BCE", "Common Era", "Before Common Era",
			"st", "nd", "rd", "th", // Ordinal suffixes
			"T", "Z", "+", "-", ":", "/", "-", ".", ",", " ",
		];
		
		if let Ok(mut cache) = self.general_strings.write() {
			for s in &general_strings {
				cache.insert(s.to_string(), Arc::new(s.to_string()));
			}
		}
	}
	
	/// Interns a month name string.
	pub fn intern_month_name(&self, name: &str) -> Arc<String> {
		self.intern_in_cache(&self.month_names, name)
	}
	
	/// Interns a day name string.
	pub fn intern_day_name(&self, name: &str) -> Arc<String> {
		self.intern_in_cache(&self.day_names, name)
	}
	
	/// Interns a timezone name string.
	pub fn intern_timezone_name(&self, name: &str) -> Arc<String> {
		self.intern_in_cache(&self.timezone_names, name)
	}
	
	/// Interns a format pattern string.
	pub fn intern_format_pattern(&self, pattern: &str) -> Arc<String> {
		self.intern_in_cache(&self.format_patterns, pattern)
	}
	
	/// Interns a general string.
	pub fn intern_general(&self, s: &str) -> Arc<String> {
		self.intern_in_cache(&self.general_strings, s)
	}
	
	/// Generic intern function for a specific cache.
	fn intern_in_cache(&self, cache: &RwLock<HashMap<String, Arc<String>>>, s: &str) -> Arc<String> {
		// Try read lock first for common case
		if let Ok(read_cache) = cache.read() {
			if let Some(interned) = read_cache.get(s) {
				return Arc::clone(interned);
			}
		}
		
		// Need to insert - upgrade to write lock
		if let Ok(mut write_cache) = cache.write() {
			// Double-check in case another thread inserted while we waited
			if let Some(interned) = write_cache.get(s) {
				return Arc::clone(interned);
			}
			
			let arc_string = Arc::new(s.to_string());
			write_cache.insert(s.to_string(), Arc::clone(&arc_string));
			arc_string
		} else {
			// Fallback if lock fails
			Arc::new(s.to_string())
		}
	}
	
	/// Returns statistics about interned strings.
	pub fn stats(&self) -> InternerStats {
		let month_count = if let Ok(cache) = self.month_names.read() { cache.len() } else { 0 };
		let day_count = if let Ok(cache) = self.day_names.read() { cache.len() } else { 0 };
		let timezone_count = if let Ok(cache) = self.timezone_names.read() { cache.len() } else { 0 };
		let pattern_count = if let Ok(cache) = self.format_patterns.read() { cache.len() } else { 0 };
		let general_count = if let Ok(cache) = self.general_strings.read() { cache.len() } else { 0 };
		
		InternerStats {
			month_names_count: month_count,
			day_names_count: day_count,
			timezone_names_count: timezone_count,
			format_patterns_count: pattern_count,
			general_strings_count: general_count,
			total_interned: month_count + day_count + timezone_count + pattern_count + general_count,
		}
	}
	
	/// Clears all interned strings (useful for testing).
	pub fn clear_all(&self) {
		let _ = self.month_names.write().map(|mut cache| cache.clear());
		let _ = self.day_names.write().map(|mut cache| cache.clear());
		let _ = self.timezone_names.write().map(|mut cache| cache.clear());
		let _ = self.format_patterns.write().map(|mut cache| cache.clear());
		let _ = self.general_strings.write().map(|mut cache| cache.clear());
	}
}

/// Statistics about string interning performance.
#[derive(Debug, Clone)]
pub struct InternerStats {
	pub month_names_count: usize,
	pub day_names_count: usize,
	pub timezone_names_count: usize,
	pub format_patterns_count: usize,
	pub general_strings_count: usize,
	pub total_interned: usize,
}

impl InternerStats {
	/// Returns the estimated memory saved by interning (rough estimate).
	pub fn estimated_memory_saved_bytes(&self) -> usize {
		// Rough estimate: each interned string saves about 24 bytes
		// (String overhead) per duplicate reference
		self.total_interned * 24
	}
}

/// Convenience functions for interning commonly used strings.
pub mod convenience {
	use super::*;
	
	/// Interns a month name.
	pub fn intern_month(name: &str) -> Arc<String> {
		GlobalStringInterner::global().intern_month_name(name)
	}
	
	/// Interns a day name.
	pub fn intern_day(name: &str) -> Arc<String> {
		GlobalStringInterner::global().intern_day_name(name)
	}
	
	/// Interns a timezone name.
	pub fn intern_timezone(name: &str) -> Arc<String> {
		GlobalStringInterner::global().intern_timezone_name(name)
	}
	
	/// Interns a format pattern.
	pub fn intern_pattern(pattern: &str) -> Arc<String> {
		GlobalStringInterner::global().intern_format_pattern(pattern)
	}
	
	/// Interns a general string.
	pub fn intern(s: &str) -> Arc<String> {
		GlobalStringInterner::global().intern_general(s)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_string_interning() {
		let interner = GlobalStringInterner::new();
		
		// Test month name interning
		let jan1 = interner.intern_month_name("January");
		let jan2 = interner.intern_month_name("January");
		assert!(Arc::ptr_eq(&jan1, &jan2)); // Same allocation
		
		// Test day name interning
		let mon1 = interner.intern_day_name("Monday");
		let mon2 = interner.intern_day_name("Monday");
		assert!(Arc::ptr_eq(&mon1, &mon2)); // Same allocation
		
		// Test different strings
		let jan = interner.intern_month_name("January");
		let feb = interner.intern_month_name("February");
		assert!(!Arc::ptr_eq(&jan, &feb)); // Different allocations
	}

	#[test]
	fn test_preloaded_strings() {
		let interner = GlobalStringInterner::new();
		
		// Test that common strings are preloaded
		let stats = interner.stats();
		assert!(stats.month_names_count > 0);
		assert!(stats.day_names_count > 0);
		assert!(stats.timezone_names_count > 0);
		assert!(stats.format_patterns_count > 0);
		assert!(stats.general_strings_count > 0);
	}

	#[test]
	fn test_convenience_functions() {
		// Test convenience functions
		let jan1 = convenience::intern_month("January");
		let jan2 = convenience::intern_month("January");
		assert!(Arc::ptr_eq(&jan1, &jan2));
		
		let utc1 = convenience::intern_timezone("UTC");
		let utc2 = convenience::intern_timezone("UTC");
		assert!(Arc::ptr_eq(&utc1, &utc2));
	}

	#[test]
	fn test_interner_stats() {
		let interner = GlobalStringInterner::new();
		
		// Add some strings
		let _ = interner.intern_month_name("NewMonth");
		let _ = interner.intern_day_name("NewDay");
		
		let stats = interner.stats();
		assert!(stats.total_interned > 0);
		assert!(stats.estimated_memory_saved_bytes() > 0);
	}
}