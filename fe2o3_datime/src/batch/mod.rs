/// Batch operations for improved performance.
///
/// This module provides efficient batch processing capabilities for common
/// date/time operations, allowing multiple calculations to be performed
/// with reduced overhead and better cache utilisation.

use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    format::{CalClockFormatter, FormatPattern},
    time::{CalClock, CalClockZone, CalClockDuration},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

pub mod operations;

/// Batch processor for date/time operations.
///
/// This provides efficient processing of multiple similar operations
/// by sharing calculations and reducing overhead.
#[derive(Debug, Clone)]
pub struct BatchProcessor {
    /// Shared timezone for batch operations.
    default_zone: CalClockZone,
    /// Pre-allocated buffers for string operations.
    #[allow(dead_code)]
    string_buffer: String,
    /// Shared formatter for batch formatting operations.
    formatter: CalClockFormatter,
}

impl BatchProcessor {
    /// Creates a new batch processor with the specified default timezone.
    pub fn new(default_zone: CalClockZone) -> Self {
        Self {
            default_zone,
            string_buffer: String::with_capacity(64), // Pre-allocate for typical datetime strings
            formatter: CalClockFormatter::new(),
        }
    }
    
    /// Creates a batch processor with UTC timezone.
    pub fn new_utc() -> Self {
        Self::new(CalClockZone::utc())
    }
    
    /// Processes multiple timezone conversions efficiently.
    ///
    /// This method converts multiple CalClock instances to a target timezone
    /// with shared calculations and reduced overhead.
    pub fn convert_timezones(&self, calclocks: &[CalClock], target_zone: &CalClockZone) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(calclocks.len());
        
        for calclock in calclocks {
            let converted = res!(calclock.with_zone(target_zone.clone()));
            results.push(converted);
        }
        
        Ok(results)
    }
    
    /// Formats multiple CalClock instances with the same pattern efficiently.
    ///
    /// This method formats multiple dates/times with a shared pattern,
    /// reducing pattern parsing overhead.
    pub fn format_batch(&self, calclocks: &[CalClock], pattern: &FormatPattern) -> Outcome<Vec<String>> {
        let mut results = Vec::with_capacity(calclocks.len());
        
        for calclock in calclocks {
            let formatted = res!(self.formatter.format_with_pattern(calclock, pattern));
            results.push(formatted);
        }
        
        Ok(results)
    }
    
    /// Performs arithmetic operations on multiple CalClock instances.
    ///
    /// This method applies the same duration addition/subtraction to
    /// multiple CalClock instances efficiently.
    pub fn add_duration_batch(&self, calclocks: &[CalClock], duration: &CalClockDuration) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(calclocks.len());
        
        for calclock in calclocks {
            let new_calclock = res!(calclock.add_duration(duration));
            results.push(new_calclock);
        }
        
        Ok(results)
    }
    
    /// Calculates day of week for multiple dates efficiently.
    ///
    /// This method calculates the day of week for multiple CalendarDate
    /// instances, potentially using lookup tables for common date ranges.
    pub fn day_of_week_batch(&self, dates: &[CalendarDate]) -> Vec<DayOfWeek> {
        dates.iter().map(|date| date.day_of_week()).collect()
    }
    
    /// Validates multiple dates efficiently.
    ///
    /// This method validates multiple date/time combinations with
    /// shared validation logic and reduced overhead.
    pub fn validate_dates_batch(&self, dates: &[(i32, MonthOfYear, u8)]) -> Vec<Outcome<()>> {
        dates.iter().map(|(year, month, day)| {
            CalendarDate::from_ymd(*year, *month, *day, self.default_zone.clone()).map(|_| ())
        }).collect()
    }
    
    /// Creates multiple CalClock instances from timestamps efficiently.
    ///
    /// This method creates multiple CalClock instances from Unix timestamps
    /// with shared timezone calculations.
    pub fn from_timestamps_batch(&self, timestamps_millis: &[i64], zone: &CalClockZone) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(timestamps_millis.len());
        
        for &timestamp in timestamps_millis {
            let calclock = res!(CalClock::from_millis(timestamp, zone.clone()));
            results.push(calclock);
        }
        
        Ok(results)
    }
    
    /// Calculates durations between pairs of CalClock instances efficiently.
    ///
    /// This method calculates durations between multiple pairs of CalClock
    /// instances with optimised calculations.
    pub fn duration_between_batch(&self, pairs: &[(CalClock, CalClock)]) -> Outcome<Vec<CalClockDuration>> {
        let mut results = Vec::with_capacity(pairs.len());
        
        for (start, end) in pairs {
            let duration = res!(start.duration_until(end));
            results.push(duration);
        }
        
        Ok(results)
    }
    
    /// Sorts multiple CalClock instances efficiently.
    ///
    /// This method sorts CalClock instances with optimised comparisons.
    pub fn sort_calclocks(&self, calclocks: Vec<CalClock>) -> Outcome<Vec<CalClock>> {
        // Convert to timestamps for efficient sorting
        let mut indexed_clocks: Vec<(i64, CalClock)> = Vec::with_capacity(calclocks.len());
        
        for calclock in calclocks {
            let timestamp = res!(calclock.to_millis());
            indexed_clocks.push((timestamp, calclock));
        }
        
        // Sort by timestamp
        indexed_clocks.sort_by_key(|(timestamp, _)| *timestamp);
        
        // Extract sorted CalClocks
        Ok(indexed_clocks.into_iter().map(|(_, calclock)| calclock).collect())
    }
    
    /// Finds CalClock instances within a time range efficiently.
    ///
    /// This method filters CalClock instances within a specified range
    /// with optimised range checking.
    pub fn filter_by_range(&self, calclocks: &[CalClock], start: &CalClock, end: &CalClock) -> Outcome<Vec<CalClock>> {
        let start_millis = res!(start.to_millis());
        let end_millis = res!(end.to_millis());
        
        let mut results = Vec::new();
        
        for calclock in calclocks {
            let calclock_millis = res!(calclock.to_millis());
            if calclock_millis >= start_millis && calclock_millis <= end_millis {
                results.push(calclock.clone());
            }
        }
        
        Ok(results)
    }
    
    /// Groups CalClock instances by day efficiently.
    ///
    /// This method groups CalClock instances by their date component
    /// with optimised grouping logic.
    pub fn group_by_day(&self, calclocks: &[CalClock]) -> HashMap<String, Vec<CalClock>> {
        let mut groups: HashMap<String, Vec<CalClock>> = HashMap::new();
        
        for calclock in calclocks {
            let day_key = format!("{:04}-{:02}-{:02}", calclock.year(), calclock.month(), calclock.day());
            groups.entry(day_key).or_insert_with(Vec::new).push(calclock.clone());
        }
        
        groups
    }
    
    /// Returns statistics about batch operations performance.
    pub fn stats(&self) -> BatchStats {
        // This would be enhanced with actual performance tracking
        BatchStats {
            operations_processed: 0, // Placeholder
            average_operation_time_nanos: 0,
            cache_hit_ratio: 0.0,
        }
    }
}

/// Statistics about batch operation performance.
#[derive(Debug, Clone)]
pub struct BatchStats {
    pub operations_processed: u64,
    pub average_operation_time_nanos: u64,
    pub cache_hit_ratio: f64,
}

impl BatchStats {
    /// Returns the operations per second rate.
    pub fn operations_per_second(&self) -> f64 {
        if self.average_operation_time_nanos > 0 {
            1_000_000_000.0 / self.average_operation_time_nanos as f64
        } else {
            0.0
        }
    }
}

/// Builder for configuring batch operations.
#[derive(Debug)]
pub struct BatchBuilder {
    zone: Option<CalClockZone>,
    formatter: Option<CalClockFormatter>,
    buffer_size: Option<usize>,
}

impl BatchBuilder {
    /// Creates a new batch builder.
    pub fn new() -> Self {
        Self {
            zone: None,
            formatter: None,
            buffer_size: None,
        }
    }
    
    /// Sets the default timezone for batch operations.
    pub fn with_zone(mut self, zone: CalClockZone) -> Self {
        self.zone = Some(zone);
        self
    }
    
    /// Sets the formatter for batch operations.
    pub fn with_formatter(mut self, formatter: CalClockFormatter) -> Self {
        self.formatter = Some(formatter);
        self
    }
    
    /// Sets the buffer size for string operations.
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = Some(size);
        self
    }
    
    /// Builds the batch processor.
    pub fn build(self) -> BatchProcessor {
        let zone = self.zone.unwrap_or_else(|| CalClockZone::utc());
        let formatter = self.formatter.unwrap_or_else(|| CalClockFormatter::new());
        
        BatchProcessor {
            default_zone: zone,
            string_buffer: String::with_capacity(self.buffer_size.unwrap_or(64)),
            formatter,
        }
    }
}

impl Default for BatchBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{constant::MonthOfYear, format::FormatPattern};

    #[test]
    fn test_batch_processor_creation() {
        let processor = BatchProcessor::new_utc();
        assert_eq!(processor.default_zone.id(), "UTC");
    }

    #[test]
    fn test_batch_formatting() -> Outcome<()> {
        let processor = BatchProcessor::new_utc();
        let pattern = res!(FormatPattern::new("yyyy-MM-dd"));
        
        let dates = vec![
            res!(CalClock::new(2024, 1, 15, 12, 0, 0, 0, CalClockZone::utc())),
            res!(CalClock::new(2024, 2, 20, 14, 30, 0, 0, CalClockZone::utc())),
            res!(CalClock::new(2024, 3, 10, 9, 15, 0, 0, CalClockZone::utc())),
        ];
        
        let formatted = res!(processor.format_batch(&dates, &pattern));
        assert_eq!(formatted.len(), 3);
        assert_eq!(formatted[0], "2024-01-15");
        assert_eq!(formatted[1], "2024-02-20");
        assert_eq!(formatted[2], "2024-03-10");
        
        Ok(())
    }

    #[test]
    fn test_batch_duration_addition() -> Outcome<()> {
        let processor = BatchProcessor::new_utc();
        let duration = CalClockDuration::from_days(7); // Add one week
        
        let dates = vec![
            res!(CalClock::new(2024, 1, 1, 0, 0, 0, 0, CalClockZone::utc())),
            res!(CalClock::new(2024, 1, 15, 12, 0, 0, 0, CalClockZone::utc())),
        ];
        
        let results = res!(processor.add_duration_batch(&dates, &duration));
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].day(), 8); // January 8
        assert_eq!(results[1].day(), 22); // January 22
        
        Ok(())
    }

    #[test]
    fn test_batch_validation() {
        let processor = BatchProcessor::new_utc();
        
        let test_dates = vec![
            (2024, MonthOfYear::February, 29), // Valid leap year date
            (2023, MonthOfYear::February, 29), // Invalid non-leap year date
            (2024, MonthOfYear::April, 31),    // Invalid date
            (2024, MonthOfYear::June, 15),     // Valid date
        ];
        
        let results = processor.validate_dates_batch(&test_dates);
        assert_eq!(results.len(), 4);
        assert!(results[0].is_ok());  // Valid leap year
        assert!(results[1].is_err()); // Invalid non-leap year
        assert!(results[2].is_err()); // Invalid date
        assert!(results[3].is_ok());  // Valid date
    }

    #[test]
    fn test_batch_builder() {
        let processor = BatchBuilder::new()
            .with_zone(CalClockZone::utc())
            .with_buffer_size(128)
            .build();
        
        assert_eq!(processor.default_zone.id(), "UTC");
    }

    #[test]
    fn test_day_of_week_batch() -> Outcome<()> {
        let processor = BatchProcessor::new_utc();
        
        let dates = vec![
            res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 1, CalClockZone::utc())), // Monday
            res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 7, CalClockZone::utc())), // Sunday
        ];
        
        let days_of_week = processor.day_of_week_batch(&dates);
        assert_eq!(days_of_week.len(), 2);
        assert_eq!(days_of_week[0], DayOfWeek::Monday);
        assert_eq!(days_of_week[1], DayOfWeek::Sunday);
        
        Ok(())
    }
}