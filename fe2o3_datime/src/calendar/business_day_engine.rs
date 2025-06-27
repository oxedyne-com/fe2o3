/// Advanced business day calculation engine with sophisticated rules.
/// 
/// This module provides comprehensive business day calculation capabilities,
/// including custom business week definitions, holiday following rules,
/// and complex date adjustment algorithms commonly used in financial systems.

use crate::{
    calendar::{CalendarDate, holiday_engines::HolidayEngine},
    constant::DayOfWeek,
    time::CalClockZone,
};
use oxedyne_fe2o3_core::prelude::*;
use std::collections::HashSet;

/// Defines which days of the week are considered business days.
#[derive(Clone, Debug, PartialEq)]
pub struct BusinessWeek {
    /// Set of days that are considered business days.
    business_days: HashSet<DayOfWeek>,
}

/// Rules for adjusting dates when they fall on non-business days.
#[derive(Clone, Debug, PartialEq)]
pub enum BusinessDayAdjustment {
    /// No adjustment - return the date as-is.
    None,
    /// Move to the following business day.
    Following,
    /// Move to the preceding business day.
    Preceding,
    /// Move to the nearest business day (prefer following if equidistant).
    ModifiedFollowing,
    /// Same as Following, but if that moves to the next month, use Preceding instead.
    ModifiedPreceding,
    /// Same as Preceding, but if that moves to the previous month, use Following instead.
    EndOfMonth,
}

/// Comprehensive business day calculation engine.
#[derive(Clone, Debug)]
pub struct BusinessDayEngine {
    /// Definition of which days constitute a business week.
    business_week: BusinessWeek,
    /// Holiday engine for determining non-business days.
    holiday_engine: Option<HolidayEngine>,
    /// Additional non-business dates (ad-hoc holidays).
    additional_holidays: HashSet<CalendarDate>,
    /// Default adjustment rule.
    default_adjustment: BusinessDayAdjustment,
}

impl Default for BusinessWeek {
    /// Standard Monday-Friday business week.
    fn default() -> Self {
        let mut business_days = HashSet::new();
        business_days.insert(DayOfWeek::Monday);
        business_days.insert(DayOfWeek::Tuesday);
        business_days.insert(DayOfWeek::Wednesday);
        business_days.insert(DayOfWeek::Thursday);
        business_days.insert(DayOfWeek::Friday);
        
        Self { business_days }
    }
}

impl BusinessWeek {
    /// Creates a new business week definition.
    pub fn new() -> Self {
        Self { business_days: HashSet::new() }
    }

    /// Creates a standard Monday-Friday business week.
    pub fn monday_to_friday() -> Self {
        Self::default()
    }

    /// Creates a Sunday-Thursday business week (common in Middle East).
    pub fn sunday_to_thursday() -> Self {
        let mut business_days = HashSet::new();
        business_days.insert(DayOfWeek::Sunday);
        business_days.insert(DayOfWeek::Monday);
        business_days.insert(DayOfWeek::Tuesday);
        business_days.insert(DayOfWeek::Wednesday);
        business_days.insert(DayOfWeek::Thursday);
        
        Self { business_days }
    }

    /// Creates a custom business week.
    pub fn custom(days: Vec<DayOfWeek>) -> Self {
        let business_days = days.into_iter().collect();
        Self { business_days }
    }

    /// Adds a day to the business week.
    pub fn add_day(mut self, day: DayOfWeek) -> Self {
        self.business_days.insert(day);
        self
    }

    /// Removes a day from the business week.
    pub fn remove_day(mut self, day: DayOfWeek) -> Self {
        self.business_days.remove(&day);
        self
    }

    /// Checks if a day of the week is a business day.
    pub fn is_business_day(&self, day: DayOfWeek) -> bool {
        self.business_days.contains(&day)
    }

    /// Gets all business days in the week.
    pub fn business_days(&self) -> Vec<DayOfWeek> {
        let mut days: Vec<_> = self.business_days.iter().cloned().collect();
        // Sort by weekday order (Sunday = 0, Monday = 1, etc.).
        days.sort_by_key(|day| day.of());
        days
    }

    /// Gets the number of business days per week.
    pub fn business_days_per_week(&self) -> usize {
        self.business_days.len()
    }
}

impl BusinessDayEngine {
    /// Creates a new business day engine with standard Monday-Friday week.
    pub fn new() -> Self {
        Self {
            business_week: BusinessWeek::default(),
            holiday_engine: None,
            additional_holidays: HashSet::new(),
            default_adjustment: BusinessDayAdjustment::Following,
        }
    }

    /// Creates a business day engine with a specific business week.
    pub fn with_business_week(mut self, business_week: BusinessWeek) -> Self {
        self.business_week = business_week;
        self
    }

    /// Sets the holiday engine for determining holidays.
    pub fn with_holiday_engine(mut self, holiday_engine: HolidayEngine) -> Self {
        self.holiday_engine = Some(holiday_engine);
        self
    }

    /// Sets the default adjustment rule.
    pub fn with_default_adjustment(mut self, adjustment: BusinessDayAdjustment) -> Self {
        self.default_adjustment = adjustment;
        self
    }

    /// Adds an additional holiday date.
    pub fn add_holiday(mut self, date: CalendarDate) -> Self {
        self.additional_holidays.insert(date);
        self
    }

    /// Checks if a date is a business day.
    pub fn is_business_day(&self, date: &CalendarDate) -> Outcome<bool> {
        // Check if the day of week is a business day.
        if !self.business_week.is_business_day(date.day_of_week()) {
            return Ok(false);
        }

        // Check additional holidays.
        if self.additional_holidays.contains(date) {
            return Ok(false);
        }

        // Check holiday engine.
        if let Some(ref engine) = self.holiday_engine {
            if res!(engine.is_holiday(date)) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Adjusts a date to be a business day using the default adjustment rule.
    pub fn adjust_date(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        self.adjust_date_with_rule(date, &self.default_adjustment)
    }

    /// Adjusts a date to be a business day using a specific adjustment rule.
    pub fn adjust_date_with_rule(&self, date: &CalendarDate, adjustment: &BusinessDayAdjustment) -> Outcome<CalendarDate> {
        match adjustment {
            BusinessDayAdjustment::None => Ok(date.clone()),
            BusinessDayAdjustment::Following => self.following_business_day(date),
            BusinessDayAdjustment::Preceding => self.preceding_business_day(date),
            BusinessDayAdjustment::ModifiedFollowing => self.modified_following_business_day(date),
            BusinessDayAdjustment::ModifiedPreceding => self.modified_preceding_business_day(date),
            BusinessDayAdjustment::EndOfMonth => self.end_of_month_business_day(date),
        }
    }

    /// Finds the next business day on or after the given date.
    pub fn following_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        // Safety limit to prevent infinite loops.
        for _ in 0..14 { // Maximum 2 weeks search.
            if res!(self.is_business_day(&current)) {
                return Ok(current);
            }
            current = res!(current.add_days(1));
        }
        
        Err(err!("Could not find business day within 14 days of {}", date; Invalid, Input))
    }

    /// Finds the previous business day on or before the given date.
    pub fn preceding_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        // Safety limit to prevent infinite loops.
        for _ in 0..14 { // Maximum 2 weeks search.
            if res!(self.is_business_day(&current)) {
                return Ok(current);
            }
            current = res!(current.add_days(-1));
        }
        
        Err(err!("Could not find business day within 14 days of {}", date; Invalid, Input))
    }

    /// Modified following: following business day, but if that's in the next month, use preceding.
    pub fn modified_following_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let following = res!(self.following_business_day(date));
        
        if following.month() != date.month() || following.year() != date.year() {
            // Following day is in next month, use preceding instead.
            self.preceding_business_day(date)
        } else {
            Ok(following)
        }
    }

    /// Modified preceding: preceding business day, but if that's in the previous month, use following.
    pub fn modified_preceding_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let preceding = res!(self.preceding_business_day(date));
        
        if preceding.month() != date.month() || preceding.year() != date.year() {
            // Preceding day is in previous month, use following instead.
            self.following_business_day(date)
        } else {
            Ok(preceding)
        }
    }

    /// End of month: if date is last business day of month, keep it; otherwise use following.
    pub fn end_of_month_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        // Check if this is the last business day of the month.
        if res!(self.is_last_business_day_of_month(date)) {
            Ok(date.clone())
        } else {
            self.following_business_day(date)
        }
    }

    /// Checks if a date is the last business day of its month.
    pub fn is_last_business_day_of_month(&self, date: &CalendarDate) -> Outcome<bool> {
        // Check if the current date is a business day.
        if !res!(self.is_business_day(date)) {
            return Ok(false);
        }

        // Check all subsequent days in the month.
        let days_in_month = res!(date.days_in_month());
        for day in (date.day() + 1)..=days_in_month {
            let test_date = res!(CalendarDate::from_ymd(
                date.year(), 
                date.month_of_year(), 
                day, 
                date.zone().clone()
            ));
            
            if res!(self.is_business_day(&test_date)) {
                return Ok(false); // Found another business day this month.
            }
        }

        Ok(true)
    }

    /// Adds business days to a date (can be negative to subtract).
    pub fn add_business_days(&self, date: &CalendarDate, business_days: i32) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        let mut remaining = business_days.abs();
        let direction = if business_days >= 0 { 1 } else { -1 };

        // Safety limit.
        let max_iterations = remaining as usize * 14; // Generous limit.
        let mut iterations = 0;

        while remaining > 0 && iterations < max_iterations {
            current = res!(current.add_days(direction));
            iterations += 1;

            if res!(self.is_business_day(&current)) {
                remaining -= 1;
            }
        }

        if remaining > 0 {
            return Err(err!("Could not add {} business days within reasonable time limit", business_days; Invalid, Input));
        }

        Ok(current)
    }

    /// Counts business days between two dates (exclusive of start, inclusive of end).
    pub fn business_days_between(&self, start: &CalendarDate, end: &CalendarDate) -> Outcome<i32> {
        if start > end {
            return Ok(-res!(self.business_days_between(end, start)));
        }

        let mut count = 0;
        let mut current = res!(start.add_days(1)); // Start from day after start date.

        // Safety limit - calculate maximum days between start and end
        let max_days = if end.year() == start.year() && end.month() == start.month() {
            (end.day() as i32 - start.day() as i32).abs() + 1
        } else {
            // For different months/years, use a generous limit
            365 * (end.year() - start.year()).abs() + 31
        };
        let mut iterations = 0;

        while current <= *end && iterations < max_days {
            if res!(self.is_business_day(&current)) {
                count += 1;
            }
            current = res!(current.add_days(1));
            iterations += 1;
        }

        Ok(count)
    }

    /// Gets the last business day of a month.
    pub fn last_business_day_of_month(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        // Start from the last day and work backwards.
        for day in (1..=days_in_month).rev() {
            let candidate = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if res!(self.is_business_day(&candidate)) {
                return Ok(candidate);
            }
        }

        Err(err!("No business day found in {} {}", month_enum, year; Invalid, Input))
    }

    /// Gets the first business day of a month.
    pub fn first_business_day_of_month(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        // Start from the first day and work forwards.
        for day in 1..=days_in_month {
            let candidate = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if res!(self.is_business_day(&candidate)) {
                return Ok(candidate);
            }
        }

        Err(err!("No business day found in {} {}", month_enum, year; Invalid, Input))
    }

    /// Gets all business days in a given month.
    pub fn business_days_in_month(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        let mut business_days = Vec::new();
        
        for day in 1..=days_in_month {
            let candidate = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if res!(self.is_business_day(&candidate)) {
                business_days.push(candidate);
            }
        }

        Ok(business_days)
    }

    /// Calculates business day statistics for a month.
    pub fn month_business_day_stats(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<BusinessDayStats> {
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        let business_days = res!(self.business_days_in_month(year, month, zone));
        
        let first_business_day = business_days.first().cloned();
        let last_business_day = business_days.last().cloned();
        
        Ok(BusinessDayStats {
            total_days: days_in_month,
            business_days_count: business_days.len() as u8,
            weekend_days: days_in_month - business_days.len() as u8,
            first_business_day,
            last_business_day,
            business_days,
        })
    }

    /// Gets the business week definition.
    pub fn business_week(&self) -> &BusinessWeek {
        &self.business_week
    }

    /// Checks if the engine has a holiday engine configured.
    pub fn has_holiday_engine(&self) -> bool {
        self.holiday_engine.is_some()
    }
}

/// Statistics about business days in a month.
#[derive(Clone, Debug)]
pub struct BusinessDayStats {
    /// Total calendar days in the month.
    pub total_days: u8,
    /// Number of business days in the month.
    pub business_days_count: u8,
    /// Number of weekend/non-business days.
    pub weekend_days: u8,
    /// First business day of the month.
    pub first_business_day: Option<CalendarDate>,
    /// Last business day of the month.
    pub last_business_day: Option<CalendarDate>,
    /// All business days in the month.
    pub business_days: Vec<CalendarDate>,
}

impl Default for BusinessDayEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::holiday_engines::HolidayEngine,
        constant::MonthOfYear,
        time::CalClockZone,
    };

    #[test]
    fn test_business_week_definitions() {
        let standard = BusinessWeek::monday_to_friday();
        assert!(standard.is_business_day(DayOfWeek::Monday));
        assert!(standard.is_business_day(DayOfWeek::Friday));
        assert!(!standard.is_business_day(DayOfWeek::Saturday));
        assert!(!standard.is_business_day(DayOfWeek::Sunday));

        let middle_east = BusinessWeek::sunday_to_thursday();
        assert!(middle_east.is_business_day(DayOfWeek::Sunday));
        assert!(middle_east.is_business_day(DayOfWeek::Thursday));
        assert!(!middle_east.is_business_day(DayOfWeek::Friday));
        assert!(!middle_east.is_business_day(DayOfWeek::Saturday));
    }

    #[test]
    fn test_business_day_engine() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // Monday should be a business day.
        let monday = CalendarDate::from_ymd(2024, MonthOfYear::June, 3, zone.clone()).unwrap();
        assert!(engine.is_business_day(&monday).unwrap());

        // Saturday should not be a business day.
        let saturday = CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone()).unwrap();
        assert!(!engine.is_business_day(&saturday).unwrap());
    }

    #[test]
    fn test_following_business_day() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // Saturday June 1, 2024 -> Monday June 3, 2024.
        let saturday = CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone()).unwrap();
        let following = engine.following_business_day(&saturday).unwrap();
        assert_eq!(following.day(), 3); // Monday.
        assert_eq!(following.day_of_week(), DayOfWeek::Monday);
    }

    #[test]
    fn test_preceding_business_day() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // Sunday June 2, 2024 -> Friday May 31, 2024.
        let sunday = CalendarDate::from_ymd(2024, MonthOfYear::June, 2, zone.clone()).unwrap();
        let preceding = engine.preceding_business_day(&sunday).unwrap();
        assert_eq!(preceding.month(), 5); // May.
        assert_eq!(preceding.day(), 31); // Friday.
        assert_eq!(preceding.day_of_week(), DayOfWeek::Friday);
    }

    #[test]
    fn test_add_business_days() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // Add 5 business days to Friday June 7, 2024 -> Friday June 14, 2024.
        let friday = CalendarDate::from_ymd(2024, MonthOfYear::June, 7, zone.clone()).unwrap();
        let result = engine.add_business_days(&friday, 5).unwrap();
        assert_eq!(result.day(), 14);
        assert_eq!(result.day_of_week(), DayOfWeek::Friday);
    }

    #[test]
    fn test_business_days_between() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // From Monday June 3 to Friday June 7, 2024 = 4 business days.
        let start = CalendarDate::from_ymd(2024, MonthOfYear::June, 3, zone.clone()).unwrap();
        let end = CalendarDate::from_ymd(2024, MonthOfYear::June, 7, zone.clone()).unwrap();
        let count = engine.business_days_between(&start, &end).unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_with_holiday_engine() {
        let zone = CalClockZone::utc();
        let holiday_engine = HolidayEngine::us_federal();
        let business_engine = BusinessDayEngine::new()
            .with_holiday_engine(holiday_engine);

        // July 4, 2024 is Independence Day (Thursday) - should not be a business day.
        let july_4 = CalendarDate::from_ymd(2024, MonthOfYear::July, 4, zone.clone()).unwrap();
        assert!(!business_engine.is_business_day(&july_4).unwrap());

        // July 5, 2024 (Friday) should be a business day.
        let july_5 = CalendarDate::from_ymd(2024, MonthOfYear::July, 5, zone.clone()).unwrap();
        assert!(business_engine.is_business_day(&july_5).unwrap());
    }

    #[test]
    fn test_month_statistics() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();
        let stats = engine.month_business_day_stats(2024, 6, zone).unwrap(); // June 2024.

        assert_eq!(stats.total_days, 30);
        assert_eq!(stats.business_days_count, 20); // 20 weekdays in June 2024.
        assert_eq!(stats.weekend_days, 10); // 10 weekend days.
        assert!(stats.first_business_day.is_some());
        assert!(stats.last_business_day.is_some());
    }

    #[test]
    fn test_modified_following() {
        let zone = CalClockZone::utc();
        let engine = BusinessDayEngine::new();

        // Last day of May 2024 is Friday 31st.
        // If Saturday June 1st needs adjustment, modified following should give Friday May 31st.
        let saturday = CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone()).unwrap();
        let adjusted = engine.modified_following_business_day(&saturday).unwrap();
        
        // Should go to preceding business day since following would be in next month.
        assert_eq!(adjusted.month(), 5); // May.
        assert_eq!(adjusted.day(), 31); // Friday.
    }
}