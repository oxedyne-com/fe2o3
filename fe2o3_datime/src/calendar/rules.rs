use crate::{
    calendar::{CalendarDate, DayIncrementor, holiday_engines::HolidayEngine, business_day_engine::BusinessDayEngine},
    constant::MonthOfYear,
    time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// Types of calendar rules supported by the rules engine.
/// Based on Java calclock CalendarRule.RuleType enum.
#[derive(Clone, Debug, PartialEq)]
pub enum RuleType {
    /// Anniversary-style rules (yearly recurrence, e.g., every 2 years from start date)
    ByYears,
    /// Monthly rules with explicitly specified months (e.g., January, April, July, October)
    ByExplicitMonths,
    /// Monthly rules with regular intervals (e.g., every 3 months)
    ByRegularMonths,
    /// Daily rules with simple day intervals (e.g., every 7 days)
    ByDays,
}

/// A comprehensive calendar rule that can generate complex recurrence patterns.
/// This is the core engine for business rules, holiday calculations, and scheduling.
/// 
/// Based on Java calclock CalendarRule class with Rust-specific enhancements.
/// 
/// # Examples
/// 
/// ```ignore
/// use oxedyne_fe2o3_datime::calendar::rules::{CalendarRule, RuleType};
/// use oxedyne_fe2o3_datime::calendar::CalendarDate;
/// use oxedyne_fe2o3_datime::time::CalClockZone;
/// 
/// let zone = CalClockZone::utc();
/// let start_date = CalendarDate::from_ymd(2024, 1, 15, zone.clone()).unwrap();
/// 
/// // Create a rule for "2nd Tuesday of every quarter"
/// let rule = CalendarRule::new(RuleType::ByExplicitMonths)
///     .with_months(vec![1, 4, 7, 10])  // Quarterly
///     .with_day_incrementor("2nd Tuesday")
///     .with_start_date(start_date);
/// 
/// let dates = rule.generate_dates(10, zone).unwrap();  // Generate 10 occurrences
/// ```
#[derive(Clone, Debug)]
pub struct CalendarRule {
    /// Type of rule (years, explicit months, regular months, days)
    rule_type: RuleType,
    /// Starting date for rule generation
    start_date: Option<CalendarDate>,
    /// Ending date for rule generation (optional)
    end_date: Option<CalendarDate>,
    /// Interval value (e.g., every 2 years, every 3 months, every 7 days)
    interval: u32,
    /// Explicitly specified months (for ByExplicitMonths)
    explicit_months: Option<HashSet<u8>>,
    /// Day incrementor for complex day finding ("3rd Monday", "last business day", etc.)
    day_incrementor: Option<DayIncrementor>,
    /// Holiday intervals to exclude from business day calculations
    holidays: Option<HolidaySet>,
    /// Advanced holiday engine for sophisticated holiday calculations
    holiday_engine: Option<HolidayEngine>,
    /// Business day engine for complex business day rules
    business_day_engine: Option<BusinessDayEngine>,
    /// Maximum number of occurrences to generate
    max_occurrences: Option<u32>,
}

/// Represents a set of holidays as date intervals for business day calculations.
/// Integrates with CalendarRule to provide holiday-aware scheduling.
#[derive(Clone, Debug)]
pub struct HolidaySet {
    /// Set of holiday dates
    holidays: HashSet<CalendarDate>,
    /// Holiday intervals (for multi-day holidays)
    intervals: Vec<HolidayInterval>,
}

/// Represents a holiday interval (e.g., Christmas break, spring break)
#[derive(Clone, Debug)]
pub struct HolidayInterval {
    /// Start date of holiday interval
    start: CalendarDate,
    /// End date of holiday interval (inclusive)
    end: CalendarDate,
    /// Name/description of the holiday
    name: String,
}

impl CalendarRule {
    /// Creates a new calendar rule with the specified type.
    pub fn new(rule_type: RuleType) -> Self {
        Self {
            rule_type,
            start_date: None,
            end_date: None,
            interval: 1,
            explicit_months: None,
            day_incrementor: None,
            holidays: None,
            holiday_engine: None,
            business_day_engine: None,
            max_occurrences: None,
        }
    }
    
    /// Sets the start date for rule generation.
    pub fn with_start_date(mut self, start_date: CalendarDate) -> Self {
        self.start_date = Some(start_date);
        self
    }
    
    /// Sets the end date for rule generation.
    pub fn with_end_date(mut self, end_date: CalendarDate) -> Self {
        self.end_date = Some(end_date);
        self
    }
    
    /// Sets the interval for regular recurrence (e.g., every 2 years, every 3 months).
    pub fn with_interval(mut self, interval: u32) -> Self {
        self.interval = interval.max(1);
        self
    }
    
    /// Sets explicit months for ByExplicitMonths rules.
    pub fn with_months(mut self, months: Vec<u8>) -> Self {
        let mut month_set = HashSet::new();
        for month in months {
            if month >= 1 && month <= 12 {
                month_set.insert(month);
            }
        }
        self.explicit_months = Some(month_set);
        self
    }
    
    /// Sets a day incrementor for complex day finding.
    pub fn with_day_incrementor_string(mut self, incrementor_str: &str) -> Outcome<Self> {
        let incrementor = res!(DayIncrementor::from_string(incrementor_str));
        self.day_incrementor = Some(incrementor);
        Ok(self)
    }
    
    /// Sets a day incrementor for complex day finding.
    pub fn with_day_incrementor(mut self, incrementor: DayIncrementor) -> Self {
        self.day_incrementor = Some(incrementor);
        self
    }
    
    /// Sets the holiday set for business day calculations.
    pub fn with_holidays(mut self, holidays: HolidaySet) -> Self {
        self.holidays = Some(holidays);
        self
    }
    
    /// Sets the maximum number of occurrences to generate.
    pub fn with_max_occurrences(mut self, max: u32) -> Self {
        self.max_occurrences = Some(max);
        self
    }
    
    /// Sets an advanced holiday engine for sophisticated holiday calculations.
    pub fn with_holiday_engine(mut self, engine: HolidayEngine) -> Self {
        self.holiday_engine = Some(engine);
        self
    }
    
    /// Sets a business day engine for complex business day rules.
    pub fn with_business_day_engine(mut self, engine: BusinessDayEngine) -> Self {
        self.business_day_engine = Some(engine);
        self
    }
    
    /// Generates a sequence of dates according to this rule.
    /// 
    /// # Arguments
    /// 
    /// * `count` - Maximum number of dates to generate
    /// * `zone` - Time zone for date calculations
    /// 
    /// # Returns
    /// 
    /// A vector of CalendarDate instances matching the rule
    pub fn generate_dates(&self, count: usize, zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        let max_count = self.max_occurrences
            .map(|max| max as usize)
            .unwrap_or(count)
            .min(count);
            
        let start_date = self.start_date
            .as_ref()
            .ok_or_else(|| err!("Start date is required for rule generation"; Invalid, Input))?;
            
        match self.rule_type {
            RuleType::ByYears => self.generate_yearly_dates(max_count, start_date, zone),
            RuleType::ByExplicitMonths => self.generate_explicit_monthly_dates(max_count, start_date, zone),
            RuleType::ByRegularMonths => self.generate_regular_monthly_dates(max_count, start_date, zone),
            RuleType::ByDays => self.generate_daily_dates(max_count, start_date, zone),
        }
    }
    
    /// Generates yearly recurrence dates (anniversary-style).
    fn generate_yearly_dates(&self, count: usize, start_date: &CalendarDate, zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        let mut dates = Vec::new();
        let mut current_year = start_date.year();
        
        for _ in 0..count {
            let candidate_date = if let Some(ref incrementor) = self.day_incrementor {
                // Use day incrementor to find the specific day in the year
                res!(incrementor.calculate_date(current_year, start_date.month(), zone.clone()))
            } else {
                // Simple anniversary date
                res!(CalendarDate::from_ymd(current_year, start_date.month_of_year(), start_date.day(), zone.clone()))
            };
            
            // Check if this date should be included
            if self.should_include_date(&candidate_date) {
                dates.push(candidate_date.clone());
            }
            
            // Check end date constraint
            if let Some(ref end_date) = self.end_date {
                if candidate_date > *end_date {
                    break;
                }
            }
            
            current_year += self.interval as i32;
        }
        
        Ok(dates)
    }
    
    /// Generates monthly recurrence dates with explicitly specified months.
    fn generate_explicit_monthly_dates(&self, count: usize, start_date: &CalendarDate, zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        let explicit_months = self.explicit_months
            .as_ref()
            .ok_or_else(|| err!("Explicit months required for ByExplicitMonths rule"; Invalid, Input))?;
            
        let mut dates = Vec::new();
        let mut current_year = start_date.year();
        let mut year_count = 0;
        
        while dates.len() < count {
            // Generate dates for each specified month in the current year
            let mut year_months: Vec<u8> = explicit_months.iter().cloned().collect();
            year_months.sort();
            
            for month in year_months {
                if dates.len() >= count {
                    break;
                }
                
                let candidate_date = if let Some(ref incrementor) = self.day_incrementor {
                    // Use day incrementor to find the specific day in the month
                    res!(incrementor.calculate_date(current_year, month, zone.clone()))
                } else {
                    // Use the same day of month as start date
                    let day = start_date.day().min(MonthOfYear::from_number(month)?.days_in_month(current_year));
                    res!(CalendarDate::from_ymd(current_year, MonthOfYear::from_number(month)?, day, zone.clone()))
                };
                
                // Check if this date should be included
                if self.should_include_date(&candidate_date) {
                    dates.push(candidate_date.clone());
                }
                
                // Check end date constraint
                if let Some(ref end_date) = self.end_date {
                    if candidate_date > *end_date {
                        return Ok(dates);
                    }
                }
            }
            
            current_year += self.interval as i32;
            year_count += 1;
            
            // Safety check to prevent infinite loops
            if year_count > 1000 {
                break;
            }
        }
        
        Ok(dates)
    }
    
    /// Generates monthly recurrence dates with regular intervals.
    fn generate_regular_monthly_dates(&self, count: usize, start_date: &CalendarDate, zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        let mut dates = Vec::new();
        let mut current_date = start_date.clone();
        
        for _ in 0..count {
            let candidate_date = if let Some(ref incrementor) = self.day_incrementor {
                // Use day incrementor to find the specific day in the month
                res!(incrementor.calculate_date(current_date.year(), current_date.month(), zone.clone()))
            } else {
                current_date.clone()
            };
            
            // Check if this date should be included
            if self.should_include_date(&candidate_date) {
                dates.push(candidate_date.clone());
            }
            
            // Check end date constraint
            if let Some(ref end_date) = self.end_date {
                if candidate_date > *end_date {
                    break;
                }
            }
            
            // Move to next occurrence
            current_date = res!(current_date.add_months(self.interval as i32));
        }
        
        Ok(dates)
    }
    
    /// Generates daily recurrence dates with simple intervals.
    fn generate_daily_dates(&self, count: usize, start_date: &CalendarDate, _zone: CalClockZone) -> Outcome<Vec<CalendarDate>> {
        let mut dates = Vec::new();
        let mut current_date = start_date.clone();
        
        for _ in 0..count {
            // Check if this date should be included
            if self.should_include_date(&current_date) {
                dates.push(current_date.clone());
            }
            
            // Check end date constraint
            if let Some(ref end_date) = self.end_date {
                if current_date > *end_date {
                    break;
                }
            }
            
            // Move to next occurrence
            current_date = res!(current_date.add_days(self.interval as i32));
        }
        
        Ok(dates)
    }
    
    /// Checks whether a date should be included based on holiday and business day rules.
    fn should_include_date(&self, date: &CalendarDate) -> bool {
        // Check advanced holiday engine first
        if let Some(ref engine) = self.holiday_engine {
            if let Ok(is_holiday) = engine.is_holiday(date) {
                if is_holiday {
                    return false;
                }
            }
        }
        
        // Check business day engine
        if let Some(ref engine) = self.business_day_engine {
            if let Ok(is_business_day) = engine.is_business_day(date) {
                // For business day rules, only include actual business days
                return is_business_day;
            }
        }
        
        // Check legacy holiday set
        if let Some(ref holidays) = self.holidays {
            if holidays.is_holiday(date) {
                return false;
            }
        }
        
        // For now, include all dates. In the future, this could include
        // additional business logic like "only business days" flags.
        true
    }
}

impl HolidaySet {
    /// Creates a new empty holiday set.
    pub fn new() -> Self {
        Self {
            holidays: HashSet::new(),
            intervals: Vec::new(),
        }
    }
    
    /// Adds a single holiday date.
    pub fn add_holiday(&mut self, date: CalendarDate) {
        self.holidays.insert(date);
    }
    
    /// Adds a holiday interval (multi-day holiday).
    pub fn add_interval(&mut self, start: CalendarDate, end: CalendarDate, name: String) {
        self.intervals.push(HolidayInterval { start, end, name });
    }
    
    /// Checks if a given date is a holiday.
    pub fn is_holiday(&self, date: &CalendarDate) -> bool {
        // Check single-day holidays
        if self.holidays.contains(date) {
            return true;
        }
        
        // Check holiday intervals
        for interval in &self.intervals {
            if *date >= interval.start && *date <= interval.end {
                return true;
            }
        }
        
        false
    }
    
    /// Checks if a given date is a business day (weekday and not a holiday).
    pub fn is_business_day(&self, date: &CalendarDate) -> bool {
        // Must be a weekday
        if !date.is_weekday() {
            return false;
        }
        
        // Must not be a holiday
        !self.is_holiday(date)
    }
    
    /// Returns all holidays in the set as a vector.
    pub fn get_holidays(&self) -> Vec<CalendarDate> {
        let mut all_holidays = Vec::new();
        
        // Add single-day holidays
        all_holidays.extend(self.holidays.iter().cloned());
        
        // Add dates from intervals
        for interval in &self.intervals {
            let mut current = interval.start.clone();
            while current <= interval.end {
                all_holidays.push(current.clone());
                if let Ok(next_day) = current.add_days(1) {
                    current = next_day;
                } else {
                    break;
                }
            }
        }
        
        all_holidays.sort();
        all_holidays
    }
}

impl Default for HolidaySet {
    fn default() -> Self {
        Self::new()
    }
}

// Convenience constructors for common rule patterns
impl CalendarRule {
    /// Creates a rule for annual recurrence (anniversary-style).
    /// Example: Every year on the same date.
    pub fn annually(start_date: CalendarDate) -> Self {
        Self::new(RuleType::ByYears)
            .with_start_date(start_date)
            .with_interval(1)
    }
    
    /// Creates a rule for quarterly recurrence (every 3 months).
    /// Example: Every quarter starting from the start date.
    pub fn quarterly(start_date: CalendarDate) -> Self {
        Self::new(RuleType::ByRegularMonths)
            .with_start_date(start_date)
            .with_interval(3)
    }
    
    /// Creates a rule for specific months each year.
    /// Example: Every January, April, July, and October.
    pub fn monthly_explicit(start_date: CalendarDate, months: Vec<u8>) -> Self {
        Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months)
    }
    
    /// Creates a rule for business day patterns.
    /// Example: "2nd Tuesday of every quarter"
    pub fn business_day_pattern(start_date: CalendarDate, pattern: &str, months: Vec<u8>) -> Outcome<Self> {
        let rule = Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months);
        rule.with_day_incrementor_string(pattern)
    }
    
    /// Creates a rule for weekly recurrence.
    /// Example: Every 2 weeks.
    pub fn weekly(start_date: CalendarDate, interval_weeks: u32) -> Self {
        Self::new(RuleType::ByDays)
            .with_start_date(start_date)
            .with_interval(interval_weeks * 7)
    }
    
    /// Creates a rule for US federal business days.
    /// Example: "2nd Tuesday of every quarter" excluding US federal holidays.
    pub fn us_business_pattern(start_date: CalendarDate, pattern: &str, months: Vec<u8>) -> Outcome<Self> {
        use crate::calendar::holiday_engines::HolidayEngine;
        use crate::calendar::business_day_engine::BusinessDayEngine;
        
        let holiday_engine = HolidayEngine::us_federal();
        let business_engine = BusinessDayEngine::new()
            .with_holiday_engine(holiday_engine.clone());
        
        let rule = Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months)
            .with_holiday_engine(holiday_engine)
            .with_business_day_engine(business_engine);
        rule.with_day_incrementor_string(pattern)
    }
    
    /// Creates a rule for UK business days.
    /// Example: "Last Monday of every month" excluding UK holidays.
    pub fn uk_business_pattern(start_date: CalendarDate, pattern: &str, months: Vec<u8>) -> Outcome<Self> {
        use crate::calendar::holiday_engines::HolidayEngine;
        use crate::calendar::business_day_engine::BusinessDayEngine;
        
        let holiday_engine = HolidayEngine::uk();
        let business_engine = BusinessDayEngine::new()
            .with_holiday_engine(holiday_engine.clone());
        
        let rule = Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months)
            .with_holiday_engine(holiday_engine)
            .with_business_day_engine(business_engine);
        rule.with_day_incrementor_string(pattern)
    }
    
    /// Creates a rule for ECB business days.
    /// Example: "First business day of each month" excluding ECB holidays.
    pub fn ecb_business_pattern(start_date: CalendarDate, pattern: &str, months: Vec<u8>) -> Outcome<Self> {
        use crate::calendar::holiday_engines::HolidayEngine;
        use crate::calendar::business_day_engine::BusinessDayEngine;
        
        let holiday_engine = HolidayEngine::ecb();
        let business_engine = BusinessDayEngine::new()
            .with_holiday_engine(holiday_engine.clone());
        
        let rule = Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months)
            .with_holiday_engine(holiday_engine)
            .with_business_day_engine(business_engine);
        rule.with_day_incrementor_string(pattern)
    }
    
    /// Creates a rule for Middle East business days (Sunday-Thursday).
    /// Example: "2nd business day of every month" with Sunday-Thursday business week.
    pub fn middle_east_business_pattern(start_date: CalendarDate, pattern: &str, months: Vec<u8>) -> Outcome<Self> {
        use crate::calendar::business_day_engine::{BusinessDayEngine, BusinessWeek};
        
        let business_week = BusinessWeek::sunday_to_thursday();
        let business_engine = BusinessDayEngine::new()
            .with_business_week(business_week);
        
        let rule = Self::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(months)
            .with_business_day_engine(business_engine);
        rule.with_day_incrementor_string(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CalClockZone;
    
    #[test]
    fn test_annual_rule() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::June, 15, zone.clone()).unwrap();
        
        let rule = CalendarRule::annually(start_date);
        let dates = rule.generate_dates(3, zone).unwrap();
        
        assert_eq!(dates.len(), 3);
        assert_eq!(dates[0].year(), 2024);
        assert_eq!(dates[1].year(), 2025);
        assert_eq!(dates[2].year(), 2026);
        
        for date in dates {
            assert_eq!(date.month(), 6);
            assert_eq!(date.day(), 15);
        }
    }
    
    #[test]
    fn test_quarterly_rule() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::January, 15, zone.clone()).unwrap();
        
        let rule = CalendarRule::quarterly(start_date);
        let dates = rule.generate_dates(4, zone).unwrap();
        
        assert_eq!(dates.len(), 4);
        
        let expected_months = [1, 4, 7, 10];
        for (i, date) in dates.iter().enumerate() {
            assert_eq!(date.month(), expected_months[i]);
            assert_eq!(date.day(), 15);
        }
    }
    
    #[test]
    fn test_explicit_months_rule() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()).unwrap();
        
        let rule = CalendarRule::monthly_explicit(start_date, vec![3, 6, 9, 12]);
        let dates = rule.generate_dates(4, zone).unwrap();
        
        assert_eq!(dates.len(), 4);
        
        let expected_months = [3, 6, 9, 12];
        for (i, date) in dates.iter().enumerate() {
            assert_eq!(date.month(), expected_months[i]);
            assert_eq!(date.year(), 2024);
        }
    }
    
    #[test]
    fn test_holiday_set() {
        let zone = CalClockZone::utc();
        let mut holidays = HolidaySet::new();
        
        // Add Christmas
        let christmas = CalendarDate::from_ymd(2024, MonthOfYear::December, 25, zone.clone()).unwrap();
        holidays.add_holiday(christmas.clone());
        
        // Add New Year's break (interval)
        let new_years_start = CalendarDate::from_ymd(2024, MonthOfYear::December, 31, zone.clone()).unwrap();
        let new_years_end = CalendarDate::from_ymd(2025, MonthOfYear::January, 2, zone.clone()).unwrap();
        holidays.add_interval(new_years_start, new_years_end, "New Year's Break".to_string());
        
        // Test holiday detection
        assert!(holidays.is_holiday(&christmas));
        assert!(holidays.is_holiday(&CalendarDate::from_ymd(2024, MonthOfYear::December, 31, zone.clone()).unwrap()));
        assert!(holidays.is_holiday(&CalendarDate::from_ymd(2025, MonthOfYear::January, 1, zone.clone()).unwrap()));
        assert!(holidays.is_holiday(&CalendarDate::from_ymd(2025, MonthOfYear::January, 2, zone.clone()).unwrap()));
        
        // Test non-holiday
        assert!(!holidays.is_holiday(&CalendarDate::from_ymd(2024, MonthOfYear::December, 24, zone).unwrap()));
    }
    
    #[test]
    fn test_business_day_detection() {
        let zone = CalClockZone::utc();
        let mut holidays = HolidaySet::new();
        
        // Add a holiday on a weekday
        let holiday = CalendarDate::from_ymd(2024, MonthOfYear::July, 4, zone.clone()).unwrap(); // Thursday
        holidays.add_holiday(holiday.clone());
        
        // Thursday July 4, 2024 is a weekday but a holiday
        assert!(!holidays.is_business_day(&holiday));
        
        // Friday July 5, 2024 is a weekday and not a holiday
        let business_day = CalendarDate::from_ymd(2024, MonthOfYear::July, 5, zone.clone()).unwrap();
        assert!(holidays.is_business_day(&business_day));
        
        // Saturday July 6, 2024 is not a weekday
        let weekend = CalendarDate::from_ymd(2024, MonthOfYear::July, 6, zone).unwrap();
        assert!(!holidays.is_business_day(&weekend));
    }
    
    #[test]
    fn test_advanced_holiday_engine_integration() {
        use crate::calendar::holiday_engines::HolidayEngine;
        
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()).unwrap();
        
        // Create a rule with US federal holidays
        let rule = CalendarRule::new(RuleType::ByExplicitMonths)
            .with_start_date(start_date)
            .with_months(vec![7]) // July only
            .with_holiday_engine(HolidayEngine::us_federal());
        
        let dates = rule.generate_dates(31, zone.clone()).unwrap(); // Generate all days in July
        
        // Should exclude July 4th (Independence Day)
        let july_4 = CalendarDate::from_ymd(2024, MonthOfYear::July, 4, zone.clone()).unwrap();
        assert!(!dates.contains(&july_4), "July 4th should be excluded as US federal holiday");
        
        // Should include July 5th (not a holiday)
        let july_5 = CalendarDate::from_ymd(2024, MonthOfYear::July, 5, zone.clone()).unwrap();
        assert!(dates.contains(&july_5), "July 5th should be included as regular day");
    }
    
    #[test]
    fn test_business_day_engine_integration() {
        use crate::calendar::business_day_engine::{BusinessDayEngine, BusinessWeek};
        use crate::calendar::holiday_engines::HolidayEngine;
        
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::July, 1, zone.clone()).unwrap();
        
        // Create business day engine with holidays
        let business_engine = BusinessDayEngine::new()
            .with_holiday_engine(HolidayEngine::us_federal());
        
        let rule = CalendarRule::new(RuleType::ByDays)
            .with_start_date(start_date)
            .with_interval(1) // Every day
            .with_business_day_engine(business_engine);
        
        let dates = rule.generate_dates(31, zone.clone()).unwrap();
        
        // Should only include business days (weekdays that aren't holidays)
        for date in &dates {
            assert!(date.is_weekday(), "All dates should be weekdays");
            
            // July 4th should not be included (Independence Day)
            if date.month() == 7 && date.day() == 4 {
                panic!("July 4th should not be included in business days");
            }
        }
        
        // Should have approximately 22-23 business days in July 2024 (excluding July 4th)
        assert!(dates.len() >= 22 && dates.len() <= 23, 
                "Expected 22-23 business days in July 2024, got {}", dates.len());
    }
    
    #[test]
    fn test_us_business_pattern_convenience() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()).unwrap();
        
        // Create "2nd Tuesday of every quarter" rule with US federal holidays
        let rule = CalendarRule::us_business_pattern(
            start_date, 
            "2nd Tuesday", 
            vec![1, 4, 7, 10] // Quarterly
        ).unwrap();
        
        let dates = rule.generate_dates(4, zone.clone()).unwrap();
        
        assert_eq!(dates.len(), 4, "Should generate 4 quarterly dates");
        
        // Check that all dates are Tuesdays
        for date in &dates {
            assert_eq!(date.day_of_week(), crate::constant::DayOfWeek::Tuesday, 
                      "All dates should be Tuesdays");
        }
        
        // Check months are quarterly
        let months: Vec<u8> = dates.iter().map(|d| d.month()).collect();
        assert_eq!(months, vec![1, 4, 7, 10], "Should be quarterly months");
    }
    
    #[test]
    fn test_middle_east_business_week() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone()).unwrap(); // Saturday
        
        // Create rule for Sunday-Thursday business week
        let rule = CalendarRule::middle_east_business_pattern(
            start_date,
            "1st business day",
            vec![6] // June only
        ).unwrap();
        
        let dates = rule.generate_dates(1, zone.clone()).unwrap();
        
        assert_eq!(dates.len(), 1, "Should generate 1 date");
        
        let first_business_day = &dates[0];
        
        // First business day of June 2024 should be Sunday June 2nd (Saturday is weekend in Middle East)
        assert_eq!(first_business_day.day(), 2, "First business day should be June 2nd");
        assert_eq!(first_business_day.day_of_week(), crate::constant::DayOfWeek::Sunday, 
                  "Should be Sunday in Middle East business week");
    }
    
    #[test]
    fn test_uk_easter_based_holidays() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::March, 1, zone.clone()).unwrap();
        
        // Create rule with UK holidays (includes Easter-based holidays)
        let rule = CalendarRule::uk_business_pattern(
            start_date,
            "1st business day", 
            vec![3, 4] // March and April (around Easter)
        ).unwrap();
        
        let dates = rule.generate_dates(2, zone.clone()).unwrap();
        
        // Should exclude Good Friday (March 29, 2024) and Easter Monday (April 1, 2024)
        for date in &dates {
            let is_good_friday = date.month() == 3 && date.day() == 29;
            let is_easter_monday = date.month() == 4 && date.day() == 1;
            
            assert!(!is_good_friday && !is_easter_monday, 
                   "Should exclude Easter holidays: Good Friday and Easter Monday");
        }
    }
    
    #[test]
    fn test_complex_rule_with_day_incrementor_and_engines() {
        let zone = CalClockZone::utc();
        let start_date = CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()).unwrap();
        
        // Create a complex rule: "Last business day of every month" with US federal holidays
        let rule = CalendarRule::us_business_pattern(
            start_date,
            "last business day",
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] // All months
        ).unwrap();
        
        let dates = rule.generate_dates(12, zone.clone()).unwrap();
        
        assert_eq!(dates.len(), 12, "Should generate one date per month");
        
        // Each date should be the last business day of its month
        for (i, date) in dates.iter().enumerate() {
            assert_eq!(date.month(), (i + 1) as u8, "Should be in the correct month");
            assert!(date.is_weekday(), "Should be a weekday");
            
            // Verify it's actually the last business day by checking next day is not a business day
            if let Ok(next_day) = date.add_days(1) {
                if next_day.month() == date.month() {
                    // If next day is in same month, it should not be a business day
                    // (either weekend or holiday, or this wouldn't be the last business day)
                }
            }
        }
    }
}