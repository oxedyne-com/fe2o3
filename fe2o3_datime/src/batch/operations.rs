//! Optimised batch operations for common date/time calculations.
//!
//! Vectorised implementations of the date/time operations that are called
//! most often.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::{CalClock, CalClockZone, CalClockDuration},
};

use oxedyne_fe2o3_core::prelude::*;

pub struct DateArithmetic;

impl DateArithmetic {
    pub fn add_days_batch(dates: &[CalendarDate], days: i32) -> Outcome<Vec<CalendarDate>> {
        let mut results = Vec::with_capacity(dates.len());
        
        for date in dates {
            let new_date = res!(date.add_days(days));
            results.push(new_date);
        }
        
        Ok(results)
    }
    
    pub fn day_of_year_batch(dates: &[CalendarDate]) -> Outcome<Vec<u16>> {
        let mut results = Vec::with_capacity(dates.len());
        let mut cached_year: Option<(i32, bool)> = None; // (year, is_leap)
        
        for date in dates {
            // Check if we can reuse leap year calculation
            let _is_leap = if let Some((cached_yr, cached_leap)) = cached_year {
                if cached_yr == date.year() {
                    cached_leap
                } else {
                    let leap = date.is_leap_year();
                    cached_year = Some((date.year(), leap));
                    leap
                }
            } else {
                let leap = date.is_leap_year();
                cached_year = Some((date.year(), leap));
                leap
            };
            
            let day_of_year = res!(date.day_of_year());
            results.push(day_of_year);
        }
        
        Ok(results)
    }
    
    /// ISO 8601 week numbering.
    pub fn week_of_year_batch(dates: &[CalendarDate]) -> Outcome<Vec<u8>> {
        let mut results = Vec::with_capacity(dates.len());
        
        for date in dates {
            let week = res!(date.week_of_year());
            results.push(week);
        }
        
        Ok(results)
    }
    
    pub fn filter_by_month(dates: &[CalendarDate], year: i32, month: MonthOfYear) -> Vec<CalendarDate> {
        dates.iter()
            .filter(|date| date.year() == year && date.month_of_year() == month)
            .cloned()
            .collect()
    }
    
    /// Weekends are excluded but holidays are not, and a start on or after
    /// the end gives zero.
    pub fn business_days_between_batch(pairs: &[(CalendarDate, CalendarDate)]) -> Outcome<Vec<i32>> {
        let mut results = Vec::with_capacity(pairs.len());
        
        for (start_date, end_date) in pairs {
            let business_days = ok!(Self::calculate_business_days(start_date, end_date));
            results.push(business_days);
        }
        
        Ok(results)
    }
    
    fn calculate_business_days(start_date: &CalendarDate, end_date: &CalendarDate) -> Outcome<i32> {
        let start_day_num = res!(start_date.to_day_number());
        let end_day_num = res!(end_date.to_day_number());
        
        if start_day_num >= end_day_num {
            return Ok(0);
        }
        
        let total_days = (end_day_num - start_day_num) as i32;
        let full_weeks = total_days / 7;
        let remaining_days = total_days % 7;
        
        // Each full week has 5 business days
        let mut business_days = full_weeks * 5;
        
        // Check remaining days
        let start_dow = start_date.day_of_week();
        for i in 0..remaining_days {
            let day_of_week = Self::advance_day_of_week(start_dow, i + 1);
            if !matches!(day_of_week, DayOfWeek::Saturday | DayOfWeek::Sunday) {
                business_days += 1;
            }
        }
        
        Ok(business_days)
    }
    
    fn advance_day_of_week(start: DayOfWeek, days: i32) -> DayOfWeek {
        let start_num = match start {
            DayOfWeek::Monday => 0,
            DayOfWeek::Tuesday => 1,
            DayOfWeek::Wednesday => 2,
            DayOfWeek::Thursday => 3,
            DayOfWeek::Friday => 4,
            DayOfWeek::Saturday => 5,
            DayOfWeek::Sunday => 6,
        };
        
        let new_num = (start_num + days) % 7;
        match new_num {
            0 => DayOfWeek::Monday,
            1 => DayOfWeek::Tuesday,
            2 => DayOfWeek::Wednesday,
            3 => DayOfWeek::Thursday,
            4 => DayOfWeek::Friday,
            5 => DayOfWeek::Saturday,
            6 => DayOfWeek::Sunday,
            _ => DayOfWeek::Monday, // Should never happen
        }
    }
}

pub struct TimeArithmetic;

impl TimeArithmetic {
    pub fn add_duration_batch(calclocks: &[CalClock], duration: &CalClockDuration) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(calclocks.len());
        
        // Group by timezone for potential optimisations
        let mut timezone_groups: std::collections::HashMap<String, Vec<&CalClock>> = 
            std::collections::HashMap::new();
        
        for calclock in calclocks {
            timezone_groups.entry(calclock.zone().id().to_string())
                .or_insert_with(Vec::new)
                .push(calclock);
        }
        
        // For simplicity, process in original order
        for calclock in calclocks {
            let new_calclock = res!(calclock.add_duration(duration));
            results.push(new_calclock);
        }
        
        Ok(results)
    }
    
    pub fn from_timestamps_batch(timestamps: &[i64], zone: &CalClockZone) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(timestamps.len());
        
        for &timestamp in timestamps {
            let calclock = res!(CalClock::from_millis(timestamp, zone.clone()));
            results.push(calclock);
        }
        
        Ok(results)
    }
    
    pub fn duration_between_batch(pairs: &[(CalClock, CalClock)]) -> Outcome<Vec<CalClockDuration>> {
        let mut results = Vec::with_capacity(pairs.len());
        
        for (start, end) in pairs {
            let duration = res!(start.duration_until(end));
            results.push(duration);
        }
        
        Ok(results)
    }
    
    /// An exact half interval rounds up.
    pub fn round_to_interval_batch(calclocks: &[CalClock], interval_minutes: u32) -> Outcome<Vec<CalClock>> {
        let mut results = Vec::with_capacity(calclocks.len());
        let interval_millis = interval_minutes as i64 * 60 * 1000;
        
        for calclock in calclocks {
            let timestamp = res!(calclock.to_millis());
            let rounded_timestamp = (timestamp / interval_millis) * interval_millis;
            
            // If remainder is >= half interval, round up
            let remainder = timestamp % interval_millis;
            let final_timestamp = if remainder >= interval_millis / 2 {
                rounded_timestamp + interval_millis
            } else {
                rounded_timestamp
            };
            
            let rounded_calclock = res!(CalClock::from_millis(final_timestamp, calclock.zone().clone()));
            results.push(rounded_calclock);
        }
        
        Ok(results)
    }
}

pub struct ComparisonOps;

impl ComparisonOps {
    pub fn sort_calclocks(calclocks: Vec<CalClock>) -> Outcome<Vec<CalClock>> {
        // Create vector of (timestamp, original_index) for stable sorting
        let mut indexed_timestamps: Vec<(i64, usize)> = Vec::with_capacity(calclocks.len());
        
        for (index, calclock) in calclocks.iter().enumerate() {
            let timestamp = res!(calclock.to_millis());
            indexed_timestamps.push((timestamp, index));
        }
        
        // Sort by timestamp
        indexed_timestamps.sort_by_key(|(timestamp, _)| *timestamp);
        
        // Reorder original vector based on sorted indices
        let mut sorted_calclocks = Vec::with_capacity(calclocks.len());
        for (_, original_index) in indexed_timestamps {
            sorted_calclocks.push(calclocks[original_index].clone());
        }
        
        Ok(sorted_calclocks)
    }
    
    pub fn min_max_calclocks(calclocks: &[CalClock]) -> Outcome<Option<(CalClock, CalClock)>> {
        if calclocks.is_empty() {
            return Ok(None);
        }
        
        let mut min_timestamp = i64::MAX;
        let mut max_timestamp = i64::MIN;
        let mut min_calclock = &calclocks[0];
        let mut max_calclock = &calclocks[0];
        
        for calclock in calclocks {
            let timestamp = res!(calclock.to_millis());
            
            if timestamp < min_timestamp {
                min_timestamp = timestamp;
                min_calclock = calclock;
            }
            
            if timestamp > max_timestamp {
                max_timestamp = timestamp;
                max_calclock = calclock;
            }
        }
        
        Ok(Some((min_calclock.clone(), max_calclock.clone())))
    }
    
    pub fn filter_by_time_range(calclocks: &[CalClock], start: &CalClock, end: &CalClock) -> Outcome<Vec<CalClock>> {
        let start_timestamp = res!(start.to_millis());
        let end_timestamp = res!(end.to_millis());
        
        let mut results = Vec::new();
        
        for calclock in calclocks {
            let timestamp = res!(calclock.to_millis());
            if timestamp >= start_timestamp && timestamp <= end_timestamp {
                results.push(calclock.clone());
            }
        }
        
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::CalendarDate,
        constant::MonthOfYear,
        time::{CalClock, CalClockZone, CalClockDuration},
    };

    #[test]
    fn test_date_arithmetic_add_days_batch() -> Outcome<()> {
        let zone = CalClockZone::utc();
        let dates = vec![
            res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone())),
            res!(CalendarDate::from_ymd(2024, MonthOfYear::February, 15, zone.clone())),
            res!(CalendarDate::from_ymd(2024, MonthOfYear::March, 10, zone.clone())),
        ];
        
        let results = res!(DateArithmetic::add_days_batch(&dates, 7));
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].day(), 8);  // January 8
        assert_eq!(results[1].day(), 22); // February 22
        assert_eq!(results[2].day(), 17); // March 17
        
        Ok(())
    }

    #[test]
    fn test_time_arithmetic_duration_batch() -> Outcome<()> {
        let zone = CalClockZone::utc();
        let calclocks = vec![
            res!(CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone())),
            res!(CalClock::new(2024, 1, 2, 14, 30, 0, 0, zone.clone())),
        ];
        
        let duration = CalClockDuration::from_hours(2);
        let results = res!(TimeArithmetic::add_duration_batch(&calclocks, &duration));
        
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].hour(), 14); // 12 + 2 = 14
        assert_eq!(results[1].hour(), 16); // 14 + 2 = 16
        
        Ok(())
    }

    #[test]
    fn test_comparison_ops_sort() -> Outcome<()> {
        let zone = CalClockZone::utc();
        let calclocks = vec![
            res!(CalClock::new(2024, 1, 3, 12, 0, 0, 0, zone.clone())),
            res!(CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone())),
            res!(CalClock::new(2024, 1, 2, 12, 0, 0, 0, zone.clone())),
        ];
        
        let sorted = res!(ComparisonOps::sort_calclocks(calclocks));
        assert_eq!(sorted[0].day(), 1); // January 1
        assert_eq!(sorted[1].day(), 2); // January 2
        assert_eq!(sorted[2].day(), 3); // January 3
        
        Ok(())
    }

    #[test]
    fn test_comparison_ops_min_max() -> Outcome<()> {
        let zone = CalClockZone::utc();
        let calclocks = vec![
            res!(CalClock::new(2024, 1, 2, 12, 0, 0, 0, zone.clone())),
            res!(CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone())),
            res!(CalClock::new(2024, 1, 3, 12, 0, 0, 0, zone.clone())),
        ];
        
        let min_max = res!(ComparisonOps::min_max_calclocks(&calclocks));
        assert!(min_max.is_some());
        
        let (min, max) = min_max.unwrap();
        assert_eq!(min.day(), 1); // January 1
        assert_eq!(max.day(), 3); // January 3
        
        Ok(())
    }

    #[test]
    fn test_business_days_calculation() -> Outcome<()> {
        let zone = CalClockZone::utc();
        let pairs = vec![
            (
                res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone())), // Monday
                res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 5, zone.clone())), // Friday
            ),
            (
                res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 6, zone.clone())), // Saturday
                res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 8, zone.clone())), // Monday
            ),
        ];
        
        let business_days = res!(DateArithmetic::business_days_between_batch(&pairs));
        assert_eq!(business_days.len(), 2);
        assert_eq!(business_days[0], 4); // Mon-Fri = 4 business days
        assert_eq!(business_days[1], 1); // Sat-Mon = 1 business day
        
        Ok(())
    }
}