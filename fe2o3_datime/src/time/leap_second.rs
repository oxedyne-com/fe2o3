use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;

/// Represents a leap second transition in the TAI-UTC conversion table.
///
/// Leap seconds are additions or subtractions of one second to UTC to keep it
/// synchronised with Earth's rotation. They are announced by IERS and require
/// separate tracking from timezone data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeapSecondEntry {
    /// UTC timestamp when the leap second takes effect (seconds since Unix epoch)
    pub utc_timestamp: i64,
    /// Total TAI-UTC offset after this leap second (in seconds)
    pub tai_utc_offset: i32,
    /// Description of this leap second event
    pub description: String,
}

impl PartialOrd for LeapSecondEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LeapSecondEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.utc_timestamp.cmp(&other.utc_timestamp)
    }
}

/// Comprehensive leap second table for TAI-UTC conversion.
///
/// This table contains all leap seconds from 1972 to present, allowing
/// accurate conversion between UTC and TAI (International Atomic Time).
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::time::LeapSecondTable;
///
/// let table = LeapSecondTable::standard();
/// let utc_timestamp = 1609459200; // 2021-01-01 00:00:00 UTC
/// let tai_utc_offset = table.tai_utc_offset_at(utc_timestamp);
/// println!("TAI-UTC offset: {} seconds", tai_utc_offset);
/// ```
#[derive(Clone, Debug)]
pub struct LeapSecondTable {
    /// Sorted list of leap second entries
    entries: Vec<LeapSecondEntry>,
    /// Whether to handle leap seconds at all (can be disabled)
    enabled: bool,
}

impl LeapSecondTable {
    /// Creates a new empty leap second table.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            enabled: true,
        }
    }

    /// Creates the standard leap second table with all historical leap seconds.
    ///
    /// This includes all leap seconds from 1972 through 2017 (the most recent).
    /// The table should be updated when new leap seconds are announced by IERS.
    pub fn standard() -> Self {
        let mut table = Self::new();
        
        // Historical leap seconds from 1972-2017
        // Source: IERS Bulletin C and NIST leap second data
        table.add_entry(78796800, 10, "1972-07-01: First leap second");
        table.add_entry(94694400, 11, "1973-01-01: +1 second");
        table.add_entry(126230400, 12, "1974-01-01: +1 second");
        table.add_entry(157766400, 13, "1975-01-01: +1 second");
        table.add_entry(189302400, 14, "1976-01-01: +1 second");
        table.add_entry(220924800, 15, "1977-01-01: +1 second");
        table.add_entry(252460800, 16, "1978-01-01: +1 second");
        table.add_entry(283996800, 17, "1979-01-01: +1 second");
        table.add_entry(315532800, 18, "1980-01-01: +1 second");
        table.add_entry(362793600, 19, "1981-07-01: +1 second");
        table.add_entry(394329600, 20, "1982-07-01: +1 second");
        table.add_entry(425865600, 21, "1983-07-01: +1 second");
        table.add_entry(489024000, 22, "1985-07-01: +1 second");
        table.add_entry(567993600, 23, "1988-01-01: +1 second");
        table.add_entry(631152000, 24, "1990-01-01: +1 second");
        table.add_entry(662688000, 25, "1991-01-01: +1 second");
        table.add_entry(709948800, 26, "1992-07-01: +1 second");
        table.add_entry(741484800, 27, "1993-07-01: +1 second");
        table.add_entry(773020800, 28, "1994-07-01: +1 second");
        table.add_entry(820454400, 29, "1996-01-01: +1 second");
        table.add_entry(867715200, 30, "1997-07-01: +1 second");
        table.add_entry(915148800, 31, "1999-01-01: +1 second");
        table.add_entry(1136073600, 32, "2006-01-01: +1 second");
        table.add_entry(1230768000, 33, "2009-01-01: +1 second");
        table.add_entry(1341100800, 34, "2012-07-01: +1 second");
        table.add_entry(1435708800, 35, "2015-07-01: +1 second");
        table.add_entry(1483228800, 36, "2017-01-01: +1 second (most recent)");

        table
    }

    /// Creates a leap second table with leap second handling disabled.
    ///
    /// When disabled, all TAI-UTC conversions return 0 offset, effectively
    /// treating TAI and UTC as identical.
    pub fn disabled() -> Self {
        Self {
            entries: Vec::new(),
            enabled: false,
        }
    }

    /// Adds a leap second entry to the table.
    pub fn add_entry(&mut self, utc_timestamp: i64, tai_utc_offset: i32, description: &str) {
        self.entries.push(LeapSecondEntry {
            utc_timestamp,
            tai_utc_offset,
            description: description.to_string(),
        });
        
        // Keep entries sorted by timestamp
        self.entries.sort();
    }

    /// Returns the TAI-UTC offset at the given UTC timestamp.
    ///
    /// This is the number of seconds to add to UTC to get TAI time.
    /// Returns 0 if leap seconds are disabled or if the timestamp is before 1972.
    pub fn tai_utc_offset_at(&self, utc_timestamp: i64) -> i32 {
        if !self.enabled {
            return 0;
        }

        // Before 1972, there were no leap seconds (TAI-UTC was 10 seconds)
        if utc_timestamp < 78796800 { // 1972-07-01
            return 10; // Initial TAI-UTC offset before leap seconds began
        }

        // Find the most recent leap second entry before or at this timestamp
        match self.entries.binary_search_by_key(&utc_timestamp, |entry| entry.utc_timestamp) {
            Ok(index) => self.entries[index].tai_utc_offset,
            Err(index) => {
                if index == 0 {
                    10 // Before first leap second
                } else {
                    self.entries[index - 1].tai_utc_offset
                }
            }
        }
    }

    /// Converts UTC timestamp to TAI timestamp.
    ///
    /// TAI (International Atomic Time) is a continuous time scale that doesn't
    /// have leap seconds, making it useful for precise time calculations.
    pub fn utc_to_tai(&self, utc_timestamp: i64) -> i64 {
        utc_timestamp + self.tai_utc_offset_at(utc_timestamp) as i64
    }

    /// Converts TAI timestamp to UTC timestamp.
    ///
    /// This is more complex than UTC→TAI because leap seconds can create
    /// ambiguity (during a positive leap second, the same UTC time can
    /// correspond to two different TAI times).
    pub fn tai_to_utc(&self, tai_timestamp: i64) -> Outcome<i64> {
        if !self.enabled {
            return Ok(tai_timestamp);
        }

        // Approximate UTC time (ignoring leap seconds)
        let approx_utc = tai_timestamp - 36; // Rough estimate using latest offset

        // Find the correct offset by checking around the approximate time
        let offset = self.tai_utc_offset_at(approx_utc);
        let precise_utc = tai_timestamp - offset as i64;

        // Verify our calculation is correct
        let verification_tai = self.utc_to_tai(precise_utc);
        if verification_tai == tai_timestamp {
            Ok(precise_utc)
        } else {
            // Handle edge cases around leap second boundaries
            self.handle_leap_second_boundary(tai_timestamp, precise_utc)
        }
    }

    /// Handles TAI to UTC conversion at leap second boundaries.
    fn handle_leap_second_boundary(&self, tai_timestamp: i64, initial_utc: i64) -> Outcome<i64> {
        // Check timestamps within ±2 seconds to handle leap second ambiguity
        for offset in -2..=2 {
            let test_utc = initial_utc + offset;
            if self.utc_to_tai(test_utc) == tai_timestamp {
                return Ok(test_utc);
            }
        }

        Err(err!("Cannot convert TAI timestamp {} to UTC: ambiguous leap second boundary", tai_timestamp; Invalid, Input))
    }

    /// Returns true if leap second handling is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enables or disables leap second handling.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Returns the number of leap seconds in the table.
    pub fn leap_second_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns information about the most recent leap second.
    pub fn latest_leap_second(&self) -> Option<&LeapSecondEntry> {
        self.entries.last()
    }

    /// Returns all leap seconds in chronological order.
    pub fn all_leap_seconds(&self) -> &[LeapSecondEntry] {
        &self.entries
    }

    /// Checks if a UTC timestamp falls exactly on a leap second.
    ///
    /// This is useful for determining if second=60 should be allowed
    /// in time parsing/formatting.
    pub fn is_leap_second(&self, utc_timestamp: i64) -> bool {
        if !self.enabled {
            return false;
        }

        self.entries.iter().any(|entry| entry.utc_timestamp == utc_timestamp)
    }

    /// Returns the leap second entry for a specific UTC timestamp, if any.
    pub fn leap_second_at(&self, utc_timestamp: i64) -> Option<&LeapSecondEntry> {
        if !self.enabled {
            return None;
        }

        self.entries.iter().find(|entry| entry.utc_timestamp == utc_timestamp)
    }

    /// Validates that a time with second=60 is actually a valid leap second.
    ///
    /// This should be used when parsing times to determine if second=60
    /// is allowed for a specific date/time.
    pub fn validate_leap_second(&self, year: i32, month: u8, day: u8, hour: u8, minute: u8) -> bool {
        if !self.enabled || hour != 23 || minute != 59 {
            return false; // Leap seconds only occur at 23:59:60 UTC
        }

        // Convert date to UTC timestamp using proper calendar arithmetic
        let utc_timestamp = match self.date_to_utc_timestamp(year, month, day, hour, minute, 60) {
            Ok(ts) => ts,
            Err(_) => return false,
        };

        self.is_leap_second(utc_timestamp)
    }
    
    /// Converts date/time components to UTC timestamp using proper calendar arithmetic.
    fn date_to_utc_timestamp(&self, year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Result<i64, &'static str> {
        // Validate input parameters
        if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 60 {
            return Err("Invalid date/time parameters");
        }
        
        // Calculate days since Unix epoch using proper calendar algorithm
        let days_since_epoch = self.calculate_days_since_epoch(year, month, day)?;
        
        // Convert to seconds and add time components
        let timestamp = days_since_epoch as i64 * 86400 + 
                       hour as i64 * 3600 + 
                       minute as i64 * 60 + 
                       second as i64;
        
        Ok(timestamp)
    }
    
    /// Calculates days since Unix epoch (January 1, 1970) using proper calendar arithmetic.
    fn calculate_days_since_epoch(&self, year: i32, month: u8, day: u8) -> Result<i32, &'static str> {
        // Validate month and day ranges
        if month < 1 || month > 12 || day < 1 {
            return Err("Invalid month or day");
        }
        
        // Check day validity for the given month/year
        let days_in_month = self.days_in_month(year, month);
        if day > days_in_month {
            return Err("Day out of range for month");
        }
        
        // Use Julian day number algorithm for accurate calculation
        // Convert to Julian day number first
        let jdn = self.gregorian_to_jdn(year, month, day);
        
        // Unix epoch is January 1, 1970 = JDN 2440588
        let unix_epoch_jdn = 2440588i64;
        let days_since_epoch = jdn - unix_epoch_jdn;
        
        // Check for reasonable range (avoid overflow)
        if days_since_epoch < i32::MIN as i64 || days_since_epoch > i32::MAX as i64 {
            return Err("Date too far from Unix epoch");
        }
        
        Ok(days_since_epoch as i32)
    }
    
    /// Converts Gregorian date to Julian Day Number.
    fn gregorian_to_jdn(&self, year: i32, month: u8, day: u8) -> i64 {
        let (y, m) = if month <= 2 {
            (year - 1, month as i32 + 12)
        } else {
            (year, month as i32)
        };
        
        let a = y / 100;
        let b = 2 - a + a / 4;
        
        let jdn = (365.25 * (y + 4716) as f64) as i64 +
                 (30.6001 * (m + 1) as f64) as i64 +
                 day as i64 + b as i64 - 1524;
        
        jdn
    }
    
    /// Returns the number of days in a given month/year.
    fn days_in_month(&self, year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => if self.is_leap_year(year) { 29 } else { 28 },
            _ => 0, // Invalid month
        }
    }
    
    /// Checks if a year is a leap year.
    fn is_leap_year(&self, year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }

    /// Returns leap second statistics.
    pub fn statistics(&self) -> LeapSecondStatistics {
        LeapSecondStatistics {
            total_leap_seconds: self.entries.len(),
            enabled: self.enabled,
            first_leap_second: self.entries.first().map(|e| e.utc_timestamp),
            latest_leap_second: self.entries.last().map(|e| e.utc_timestamp),
            current_tai_utc_offset: self.entries.last().map(|e| e.tai_utc_offset).unwrap_or(10),
        }
    }
}

impl Default for LeapSecondTable {
    fn default() -> Self {
        Self::standard()
    }
}

/// Statistics about the leap second table.
#[derive(Clone, Debug)]
pub struct LeapSecondStatistics {
    /// Total number of leap seconds in the table
    pub total_leap_seconds: usize,
    /// Whether leap second handling is enabled
    pub enabled: bool,
    /// UTC timestamp of the first leap second (if any)
    pub first_leap_second: Option<i64>,
    /// UTC timestamp of the most recent leap second (if any)
    pub latest_leap_second: Option<i64>,
    /// Current TAI-UTC offset in seconds
    pub current_tai_utc_offset: i32,
}

/// Configuration for leap second support in the datetime library.
#[derive(Clone, Debug)]
pub struct LeapSecondConfig {
    /// Whether to enable leap second support
    pub enabled: bool,
    /// Whether to allow parsing second=60 in time strings
    pub allow_leap_second_parsing: bool,
    /// Whether to validate leap seconds against the official table
    pub validate_leap_seconds: bool,
    /// Custom leap second table (if None, uses standard table)
    pub custom_table: Option<LeapSecondTable>,
}

impl Default for LeapSecondConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_leap_second_parsing: true,
            validate_leap_seconds: true,
            custom_table: None,
        }
    }
}

impl LeapSecondConfig {
    /// Creates a configuration with leap seconds disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            allow_leap_second_parsing: false,
            validate_leap_seconds: false,
            custom_table: None,
        }
    }

    /// Creates a configuration with leap second support enabled but no validation.
    ///
    /// This allows second=60 parsing but doesn't check if it's a real leap second.
    pub fn permissive() -> Self {
        Self {
            enabled: true,
            allow_leap_second_parsing: true,
            validate_leap_seconds: false,
            custom_table: None,
        }
    }

    /// Gets the leap second table to use.
    pub fn get_table(&self) -> LeapSecondTable {
        if !self.enabled {
            LeapSecondTable::disabled()
        } else if let Some(ref table) = self.custom_table {
            table.clone()
        } else {
            LeapSecondTable::standard()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leap_second_table_creation() {
        let table = LeapSecondTable::standard();
        assert!(table.is_enabled());
        assert!(table.leap_second_count() > 0);
        
        let disabled = LeapSecondTable::disabled();
        assert!(!disabled.is_enabled());
        assert_eq!(disabled.leap_second_count(), 0);
    }

    #[test]
    fn test_tai_utc_offset() {
        let table = LeapSecondTable::standard();
        
        // Before first leap second (1972-07-01)
        assert_eq!(table.tai_utc_offset_at(0), 10);
        
        // After first leap second
        assert_eq!(table.tai_utc_offset_at(78796800), 10); // Exactly at first leap second
        assert_eq!(table.tai_utc_offset_at(78796801), 10); // Just after first leap second
        
        // After most recent leap second (2017-01-01)
        assert_eq!(table.tai_utc_offset_at(1483228800), 36); // 2017-01-01
        assert_eq!(table.tai_utc_offset_at(1600000000), 36); // 2020 (no leap seconds since 2017)
    }

    #[test]
    fn test_utc_tai_conversion() -> Outcome<()> {
        let table = LeapSecondTable::standard();
        
        let utc_timestamp = 1483228800; // 2017-01-01 00:00:00 UTC
        let tai_timestamp = table.utc_to_tai(utc_timestamp);
        assert_eq!(tai_timestamp, utc_timestamp + 36);
        
        // Round trip conversion
        let converted_utc = res!(table.tai_to_utc(tai_timestamp));
        assert_eq!(converted_utc, utc_timestamp);
        Ok(())
    }

    #[test]
    fn test_leap_second_detection() {
        let table = LeapSecondTable::standard();
        
        // 2017-01-01 leap second
        assert!(table.is_leap_second(1483228800));
        
        // Not a leap second
        assert!(!table.is_leap_second(1483228801));
        
        // Test leap second validation
        assert!(table.validate_leap_second(2017, 1, 1, 23, 59)); // Valid leap second
        assert!(!table.validate_leap_second(2017, 1, 2, 23, 59)); // Not a leap second date
        assert!(!table.validate_leap_second(2017, 1, 1, 12, 0)); // Wrong time
    }

    #[test]
    fn test_leap_second_config() {
        let config = LeapSecondConfig::default();
        assert!(config.enabled);
        assert!(config.allow_leap_second_parsing);
        
        let disabled = LeapSecondConfig::disabled();
        assert!(!disabled.enabled);
        assert!(!disabled.allow_leap_second_parsing);
        
        let table = disabled.get_table();
        assert!(!table.is_enabled());
    }

    #[test]
    fn test_leap_second_statistics() {
        let table = LeapSecondTable::standard();
        let stats = table.statistics();
        
        assert!(stats.total_leap_seconds > 0);
        assert!(stats.enabled);
        assert!(stats.first_leap_second.is_some());
        assert!(stats.latest_leap_second.is_some());
        assert_eq!(stats.current_tai_utc_offset, 36); // As of 2017
    }

    #[test]
    fn test_leap_second_boundary_handling() -> Outcome<()> {
        let table = LeapSecondTable::standard();
        
        // Test conversion around leap second boundaries
        let leap_second_utc = 1483228800; // 2017-01-01 leap second
        let tai_at_leap = table.utc_to_tai(leap_second_utc);
        
        // Converting back should work
        let converted_back = res!(table.tai_to_utc(tai_at_leap));
        assert_eq!(converted_back, leap_second_utc);
        Ok(())
    }
}