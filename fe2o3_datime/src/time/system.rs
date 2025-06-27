use crate::time::{CalClockZone, tzif::{TZifParser, TZifData, LocalTimeResult}};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    env,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

/// Configuration for system timezone database integration.
#[derive(Clone, Debug)]
pub struct SystemTimezoneConfig {
    /// Whether to use system timezone data automatically.
    pub use_system_data: bool,
    
    /// Whether to require user consent before accessing system data.
    pub require_consent: bool,
    
    /// Paths to search for timezone data.
    pub search_paths: Vec<PathBuf>,
    
    /// Whether to detect timezone rule conflicts.
    pub detect_conflicts: bool,
    
    /// Maximum age of cached timezone data in seconds.
    pub cache_max_age: u64,
}

impl Default for SystemTimezoneConfig {
    fn default() -> Self {
        Self {
            use_system_data: false, // Conservative default
            require_consent: true,
            search_paths: Self::default_search_paths(),
            detect_conflicts: true,
            cache_max_age: 24 * 60 * 60, // 24 hours
        }
    }
}

impl SystemTimezoneConfig {
    /// Creates a new configuration optimised for automatic updates (Jiff-style).
    pub fn automatic() -> Self {
        Self {
            use_system_data: true,
            require_consent: false,
            detect_conflicts: true,
            ..Default::default()
        }
    }

    /// Creates a new configuration that requires explicit user consent.
    pub fn with_consent() -> Self {
        Self {
            use_system_data: true,
            require_consent: true,
            detect_conflicts: true,
            ..Default::default()
        }
    }

    /// Returns default search paths for timezone data based on platform.
    pub fn default_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Unix-like systems
        if cfg!(unix) {
            paths.push(PathBuf::from("/usr/share/zoneinfo"));
            paths.push(PathBuf::from("/usr/lib/zoneinfo"));
            paths.push(PathBuf::from("/etc/zoneinfo"));
            
            // macOS
            if cfg!(target_os = "macos") {
                paths.push(PathBuf::from("/var/db/timezone/zoneinfo"));
            }
        }

        // Windows
        if cfg!(windows) {
            if let Ok(windir) = env::var("WINDIR") {
                paths.push(PathBuf::from(windir).join("System32").join("tzdata"));
            }
        }

        // Android
        if cfg!(target_os = "android") {
            // Check environment variables for Android paths
            if let Ok(android_root) = env::var("ANDROID_ROOT") {
                paths.push(PathBuf::from(android_root).join("usr").join("share").join("zoneinfo"));
            } else {
                paths.push(PathBuf::from("/system/usr/share/zoneinfo"));
            }

            if let Ok(android_data) = env::var("ANDROID_DATA") {
                paths.push(PathBuf::from(android_data).join("misc").join("zoneinfo"));
            }

            // Try the concatenated timezone database location
            paths.push(PathBuf::from("/system/usr/share/zoneinfo/tzdata"));
        }

        paths
    }

    /// Adds a custom search path.
    pub fn add_search_path(&mut self, path: PathBuf) {
        self.search_paths.push(path);
    }

    /// Removes a search path.
    pub fn remove_search_path(&mut self, path: &Path) {
        self.search_paths.retain(|p| p != path);
    }
}

/// Manager for system timezone database integration.
pub struct SystemTimezoneManager {
    config: SystemTimezoneConfig,
    consent_given: Mutex<bool>,
    cache: Mutex<TimezoneCache>,
}

#[derive(Debug)]
struct TimezoneCache {
    data: HashMap<String, CachedTimezoneData>,
    last_updated: SystemTime,
    hit_count: u64,
    miss_count: u64,
}

#[derive(Clone, Debug)]
struct CachedTimezoneData {
    zone: CalClockZone,
    tzif_data: Option<TZifData>,
    source_path: PathBuf,
    last_modified: SystemTime,
    rules_version: String,
}

static TIMEZONE_MANAGER: OnceLock<SystemTimezoneManager> = OnceLock::new();

impl SystemTimezoneManager {
    /// Gets the global timezone manager instance.
    pub fn global() -> &'static Self {
        TIMEZONE_MANAGER.get_or_init(|| Self::new(SystemTimezoneConfig::default()))
    }

    /// Creates a new timezone manager with the given configuration.
    pub fn new(config: SystemTimezoneConfig) -> Self {
        Self {
            config,
            consent_given: Mutex::new(false),
            cache: Mutex::new(TimezoneCache {
                data: HashMap::new(),
                last_updated: SystemTime::UNIX_EPOCH,
                hit_count: 0,
                miss_count: 0,
            }),
        }
    }

    /// Configures the global timezone manager.
    pub fn configure(config: SystemTimezoneConfig) -> Outcome<()> {
        if TIMEZONE_MANAGER.get().is_some() {
            return Err(err!("Timezone manager already initialised"; Invalid, Init));
        }
        
        let result = TIMEZONE_MANAGER.set(Self::new(config))
            .map_err(|_| err!("Failed to set timezone manager"; System));
        res!(result);
        
        Ok(())
    }

    /// Requests user consent for system timezone data access.
    pub fn request_consent(&self) -> Outcome<bool> {
        if !self.config.require_consent {
            let mut consent = lock_mutex!(self.consent_given);
            *consent = true;
            return Ok(true);
        }

        // In a real implementation, this would show a user dialog
        // For now, we'll check an environment variable
        if env::var("FE2O3_CALCLOCK_TIMEZONE_CONSENT").is_ok() {
            let mut consent = lock_mutex!(self.consent_given);
            *consent = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns true if user has given consent for system timezone access.
    pub fn has_consent(&self) -> bool {
        if !self.config.require_consent {
            return true;
        }
        
        match self.consent_given.lock() {
            Ok(guard) => *guard,
            Err(_) => {
                // If mutex is poisoned, assume no consent for safety
                eprintln!("Warning: Consent mutex poisoned, assuming no consent");
                false
            }
        }
    }

    /// Attempts to load a timezone from system data.
    pub fn load_system_timezone(&self, zone_id: &str) -> Outcome<Option<CalClockZone>> {
        if !self.config.use_system_data {
            return Ok(None);
        }

        if self.config.require_consent && !self.has_consent() {
            if !res!(self.request_consent()) {
                return Ok(None);
            }
        }

        // Check cache first
        if let Some(cached) = self.get_cached_timezone(zone_id) {
            if self.is_cache_valid(&cached) {
                return Ok(Some(cached.zone));
            }
        }

        // Try to load from system
        for search_path in &self.config.search_paths {
            if let Ok(zone) = self.load_timezone_from_path(search_path, zone_id) {
                res!(self.cache_timezone(zone_id, zone.clone(), search_path.clone()));
                return Ok(Some(zone));
            }
        }

        Ok(None)
    }

    /// Loads timezone data from a specific path using TZif parser.
    fn load_timezone_from_path(&self, base_path: &Path, zone_id: &str) -> Outcome<CalClockZone> {
        let zone_path = base_path.join(zone_id);
        
        if !zone_path.exists() {
            return Err(err!("Timezone file not found: {}", zone_path.display(); NotFound));
        }

        // Parse TZif file format
        let mut parser = TZifParser::new();
        res!(parser.load_from_file(&zone_path));
        
        if let Some(tzif_data) = parser.timezone_data() {
            // Create CalClockZone with TZif data
            CalClockZone::from_tzif_data(zone_id, tzif_data.clone())
        } else {
            // Fall back to embedded zone creation
            CalClockZone::new_embedded(zone_id)
        }
    }

    /// Gets cached timezone data.
    fn get_cached_timezone(&self, zone_id: &str) -> Option<CachedTimezoneData> {
        let mut cache = match self.cache.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Cache mutex poisoned in get_cached_timezone");
                return None;
            }
        };
        if let Some(cached) = cache.data.get(zone_id).cloned() {
            cache.hit_count += 1;
            Some(cached)
        } else {
            cache.miss_count += 1;
            None
        }
    }

    /// Checks if cached timezone data is still valid.
    fn is_cache_valid(&self, cached: &CachedTimezoneData) -> bool {
        // Check if cache has expired
        if let Ok(elapsed) = cached.last_modified.elapsed() {
            if elapsed.as_secs() > self.config.cache_max_age {
                return false;
            }
        }

        // Check if source file has been modified
        if let Ok(metadata) = fs::metadata(&cached.source_path) {
            if let Ok(modified) = metadata.modified() {
                return modified <= cached.last_modified;
            }
        }

        true
    }

    /// Caches timezone data with TZif information.
    fn cache_timezone(&self, zone_id: &str, zone: CalClockZone, source_path: PathBuf) -> Outcome<()> {
        let mut cache = lock_mutex!(self.cache);
        
        let last_modified = if let Ok(metadata) = fs::metadata(&source_path) {
            metadata.modified().unwrap_or(SystemTime::now())
        } else {
            SystemTime::now()
        };

        // Try to parse TZif data for caching
        let tzif_data = if source_path.exists() {
            let mut parser = TZifParser::new();
            if parser.load_from_file(&source_path).is_ok() {
                parser.timezone_data().cloned()
            } else {
                None
            }
        } else {
            None
        };

        let rules_version = if let Some(ref tzif) = tzif_data {
            format!("TZif v{}", tzif.version)
        } else {
            "unknown".to_string()
        };

        let cached_data = CachedTimezoneData {
            zone,
            tzif_data,
            source_path,
            last_modified,
            rules_version,
        };

        cache.data.insert(zone_id.to_string(), cached_data);
        cache.last_updated = SystemTime::now();
        
        Ok(())
    }

    /// Detects conflicts between system and embedded timezone data.
    pub fn detect_timezone_conflicts(&self, zone_id: &str, embedded_zone: &CalClockZone) -> Outcome<Vec<TimezoneConflict>> {
        if !self.config.detect_conflicts {
            return Ok(Vec::new());
        }

        let mut conflicts = Vec::new();

        if let Ok(Some(system_zone)) = self.load_system_timezone(zone_id) {
            // Compare the zones for conflicts with comprehensive detection
            if system_zone.id() != embedded_zone.id() {
                conflicts.push(TimezoneConflict::IdMismatch {
                    zone_id: zone_id.to_string(),
                    system_id: system_zone.id().to_string(),
                    embedded_id: embedded_zone.id().to_string(),
                });
            }

            // Check for offset conflicts at different times of year
            let test_times = self.generate_test_timestamps();
            for &timestamp in &test_times {
                if let (Ok(system_offset), Ok(embedded_offset)) = (
                    system_zone.offset_millis_at_time(timestamp),
                    embedded_zone.offset_millis_at_time(timestamp)
                ) {
                    if system_offset != embedded_offset {
                        conflicts.push(TimezoneConflict::OffsetMismatch {
                            zone_id: zone_id.to_string(),
                            timestamp,
                            system_offset,
                            embedded_offset,
                        });
                    }
                }
            }

            // Check for DST transition differences
            if let (Ok(system_dst), Ok(embedded_dst)) = (
                self.get_dst_transitions(&system_zone),
                self.get_dst_transitions(&embedded_zone)
            ) {
                if system_dst != embedded_dst {
                    conflicts.push(TimezoneConflict::DstMismatch {
                        zone_id: zone_id.to_string(),
                        system_transitions: system_dst,
                        embedded_transitions: embedded_dst,
                    });
                }
            }
        }

        Ok(conflicts)
    }

    /// Clears the timezone cache.
    pub fn clear_cache(&self) {
        let mut cache = match self.cache.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Cache mutex poisoned in clear_cache");
                return;
            }
        };
        cache.data.clear();
        cache.last_updated = SystemTime::UNIX_EPOCH;
    }

    /// Returns statistics about cached timezone data.
    pub fn cache_stats(&self) -> TimezoneStats {
        let cache = match self.cache.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Cache mutex poisoned in cache_stats");
                return TimezoneStats {
                    cached_zones: 0,
                    last_updated: SystemTime::UNIX_EPOCH,
                    cache_hit_rate: 0.0,
                };
            }
        };
        let total_requests = cache.hit_count + cache.miss_count;
        let hit_rate = if total_requests > 0 {
            cache.hit_count as f64 / total_requests as f64
        } else {
            0.0
        };
        
        TimezoneStats {
            cached_zones: cache.data.len(),
            last_updated: cache.last_updated,
            cache_hit_rate: hit_rate,
        }
    }

    /// Lists available timezone IDs from system data.
    pub fn list_system_timezones(&self) -> Outcome<Vec<String>> {
        if !self.config.use_system_data || (self.config.require_consent && !self.has_consent()) {
            return Ok(Vec::new());
        }

        let mut zone_ids = Vec::new();

        for search_path in &self.config.search_paths {
            if let Ok(zones) = self.scan_timezone_directory(search_path) {
                zone_ids.extend(zones);
            }
        }

        zone_ids.sort();
        zone_ids.dedup();
        Ok(zone_ids)
    }

    /// Scans a directory for timezone files.
    fn scan_timezone_directory(&self, dir: &Path) -> Outcome<Vec<String>> {
        let mut zones = Vec::new();
        
        if !dir.exists() {
            return Ok(zones);
        }

        res!(self.scan_directory_recursive(dir, dir, &mut zones));
        Ok(zones)
    }

    /// Recursively scans directories for timezone files.
    fn scan_directory_recursive(&self, base_dir: &Path, current_dir: &Path, zones: &mut Vec<String>) -> Outcome<()> {
        for entry in res!(fs::read_dir(current_dir)) {
            let entry = res!(entry);
            let path = entry.path();
            
            if path.is_dir() {
                // Skip some known non-timezone directories
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "posix" | "right" | "." | "..") {
                        continue;
                    }
                }
                
                res!(self.scan_directory_recursive(base_dir, &path, zones));
            } else if path.is_file() {
                // Convert absolute path to relative zone ID
                if let Ok(relative) = path.strip_prefix(base_dir) {
                    if let Some(zone_id) = relative.to_str() {
                        // Filter out obvious non-timezone files
                        if !zone_id.contains('.') && !zone_id.starts_with("leap") {
                            zones.push(zone_id.to_string());
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Generates test timestamps for conflict detection throughout the year.
    fn generate_test_timestamps(&self) -> Vec<i64> {
        let current_year = 2024; // Could be made dynamic
        let mut timestamps = Vec::new();
        
        // Test timestamps at different points throughout the year
        // to catch seasonal DST differences
        for month in [1, 3, 6, 9, 12] {
            for day in [1, 15] {
                for hour in [0, 12] {
                    // Convert to Unix timestamp (approximate)
                    let days_since_epoch = (current_year - 1970) * 365 + (current_year - 1969) / 4;
                    let month_days = (month - 1) * 30; // Simplified
                    let total_days = days_since_epoch + month_days + day - 1;
                    let timestamp = total_days as i64 * 86400 + hour * 3600;
                    timestamps.push(timestamp * 1000); // Convert to milliseconds
                }
            }
        }
        
        timestamps
    }
    
    /// Extracts DST transition timestamps from a timezone.
    fn get_dst_transitions(&self, zone: &CalClockZone) -> Outcome<Vec<i64>> {
        // This is a simplified implementation that would need to be
        // expanded based on the actual timezone implementation
        let mut transitions = Vec::new();
        
        // For zones with DST rules, extract transition points
        // This would require access to the zone's internal DST rules
        // For now, return empty vector as placeholder
        
        Ok(transitions)
    }
}

/// Represents a conflict between system and embedded timezone data.
#[derive(Clone, Debug)]
pub enum TimezoneConflict {
    /// The timezone ID differs between system and embedded data.
    IdMismatch {
        zone_id: String,
        system_id: String,
        embedded_id: String,
    },
    /// The offset rules differ for a specific datetime.
    OffsetMismatch {
        zone_id: String,
        timestamp: i64,
        system_offset: i32,
        embedded_offset: i32,
    },
    /// DST transition dates differ.
    DstMismatch {
        zone_id: String,
        system_transitions: Vec<i64>,
        embedded_transitions: Vec<i64>,
    },
}

/// Statistics about timezone cache usage.
#[derive(Clone, Debug)]
pub struct TimezoneStats {
    pub cached_zones: usize,
    pub last_updated: SystemTime,
    pub cache_hit_rate: f64,
}

/// Extension trait for CalClockZone to integrate with system timezone data.
pub trait SystemTimezoneExt {
    /// Attempts to load this timezone from system data, falling back to embedded data.
    fn from_system_or_embedded(zone_id: &str) -> Outcome<CalClockZone>;
    
    /// Detects conflicts between this zone and system timezone data.
    fn detect_conflicts(&self) -> Outcome<Vec<TimezoneConflict>>;
    
    /// Returns true if this zone was loaded from system data.
    fn is_from_system(&self) -> bool;
}

impl SystemTimezoneExt for CalClockZone {
    fn from_system_or_embedded(zone_id: &str) -> Outcome<CalClockZone> {
        let manager = SystemTimezoneManager::global();
        
        // Try system data first
        if let Ok(Some(system_zone)) = manager.load_system_timezone(zone_id) {
            return Ok(system_zone);
        }
        
        // Fall back to embedded data
        CalClockZone::new(zone_id)
    }
    
    fn detect_conflicts(&self) -> Outcome<Vec<TimezoneConflict>> {
        let manager = SystemTimezoneManager::global();
        manager.detect_timezone_conflicts(self.id(), self)
    }
    
    fn is_from_system(&self) -> bool {
        // TODO: Track this in CalClockZone
        false
    }
}

/// Leap second handling capability assessment.
/// 
/// This determines whether the system timezone integration handles leap seconds.
/// Based on research, most timezone databases (including IANA) do NOT handle leap seconds
/// directly - they use UTC time which doesn't account for leap seconds.
///
/// Leap seconds are typically handled at a different layer (TAI-UTC conversion).
pub struct LeapSecondCapability;

impl LeapSecondCapability {
    /// Returns true if the system timezone integration handles leap seconds.
    ///
    /// **Answer: NO** - Standard timezone databases (including Jiff's approach) do NOT handle leap seconds.
    /// Timezone databases handle DST transitions and timezone rule changes, but leap seconds
    /// are handled separately through TAI-UTC conversion tables.
    ///
    /// Leap seconds:
    /// - Are added/removed from UTC at the atomic clock level
    /// - Are announced by IERS (International Earth Rotation and Reference Systems Service)
    /// - Require separate leap second tables (not timezone data)
    /// - Must be handled by converting between UTC and TAI (International Atomic Time)
    pub fn handles_leap_seconds() -> bool {
        false
    }

    /// Returns a description of leap second handling in timezone systems.
    pub fn leap_second_explanation() -> &'static str {
        "Timezone databases (including IANA and Jiff-style integration) do NOT handle leap seconds. \
         Leap seconds are handled separately through TAI-UTC conversion tables. Timezone data \
         handles DST transitions and timezone rule changes, but leap seconds require a separate \
         leap second table that tracks UTC-TAI differences over time."
    }

    /// Returns true if leap second support should be implemented separately.
    pub fn requires_separate_implementation() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_timezone_config_default() {
        let config = SystemTimezoneConfig::default();
        assert!(!config.use_system_data); // Conservative default
        assert!(config.require_consent);
        assert!(config.detect_conflicts);
        assert!(!config.search_paths.is_empty());
    }

    #[test]
    fn test_system_timezone_config_automatic() {
        let config = SystemTimezoneConfig::automatic();
        assert!(config.use_system_data);
        assert!(!config.require_consent);
        assert!(config.detect_conflicts);
    }

    #[test]
    fn test_default_search_paths() {
        let paths = SystemTimezoneConfig::default_search_paths();
        assert!(!paths.is_empty());
        
        if cfg!(unix) {
            assert!(paths.contains(&PathBuf::from("/usr/share/zoneinfo")));
        }
    }

    #[test]
    fn test_leap_second_capability() {
        assert!(!LeapSecondCapability::handles_leap_seconds());
        assert!(LeapSecondCapability::requires_separate_implementation());
        assert!(!LeapSecondCapability::leap_second_explanation().is_empty());
    }

    #[test]
    fn test_timezone_manager_creation() {
        let config = SystemTimezoneConfig::default();
        let manager = SystemTimezoneManager::new(config);
        
        // Initially no consent
        assert!(!manager.has_consent());
        
        // Stats should be empty
        let stats = manager.cache_stats();
        assert_eq!(stats.cached_zones, 0);
    }

    #[test]
    fn test_timezone_manager_consent() {
        let config = SystemTimezoneConfig::with_consent();
        let manager = SystemTimezoneManager::new(config);
        
        assert!(!manager.has_consent());
        
        // Set environment variable to simulate consent
        env::set_var("FE2O3_CALCLOCK_TIMEZONE_CONSENT", "1");
        assert!(manager.request_consent().unwrap_or(false));
        assert!(manager.has_consent());
        
        // Clean up
        env::remove_var("FE2O3_CALCLOCK_TIMEZONE_CONSENT");
    }
}