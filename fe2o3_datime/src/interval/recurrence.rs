use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::CalClock,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// Frequency of recurrence.
#[derive(Clone, Debug, PartialEq)]
pub enum Frequency {
    /// Every day.
    Daily,
    /// Every week.
    Weekly,
    /// Every month.
    Monthly,
    /// Every year.
    Yearly,
    /// Every hour.
    Hourly,
    /// Every minute.
    Minutely,
    /// Every second.
    Secondly,
}

/// A recurrence rule defines how events repeat.
#[derive(Clone, Debug)]
pub struct RecurrenceRule {
    /// The base frequency of recurrence.
    frequency: Frequency,
    /// Interval between recurrences (e.g., every 2 weeks).
    interval: u32,
    /// Optional end date for the recurrence.
    until: Option<CalendarDate>,
    /// Optional count of occurrences.
    count: Option<u32>,
    /// Days of the week for weekly/monthly/yearly patterns.
    by_weekday: Option<HashSet<DayOfWeek>>,
    /// Days of the month for monthly/yearly patterns (1-31).
    by_month_day: Option<HashSet<u8>>,
    /// Months of the year for yearly patterns.
    by_month: Option<HashSet<MonthOfYear>>,
    /// Hours of the day (0-23).
    by_hour: Option<HashSet<u8>>,
    /// Minutes of the hour (0-59).
    by_minute: Option<HashSet<u8>>,
    /// Seconds of the minute (0-59).
    by_second: Option<HashSet<u8>>,
}

impl RecurrenceRule {
    /// Creates a new recurrence rule with the specified frequency.
    pub fn new(frequency: Frequency) -> Self {
        Self {
            frequency,
            interval: 1,
            until: None,
            count: None,
            by_weekday: None,
            by_month_day: None,
            by_month: None,
            by_hour: None,
            by_minute: None,
            by_second: None,
        }
    }
    
    /// Sets the interval between recurrences.
    pub fn interval(mut self, interval: u32) -> Self {
        self.interval = interval.max(1);
        self
    }
    
    /// Sets an end date for the recurrence.
    pub fn until(mut self, until: CalendarDate) -> Self {
        self.until = Some(until);
        self
    }
    
    /// Sets the maximum number of occurrences.
    pub fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }
    
    /// Sets the days of the week for recurrence.
    pub fn by_weekday(mut self, weekdays: HashSet<DayOfWeek>) -> Self {
        self.by_weekday = Some(weekdays);
        self
    }
    
    /// Sets the days of the month for recurrence.
    pub fn by_month_day(mut self, month_days: HashSet<u8>) -> Self {
        self.by_month_day = Some(month_days);
        self
    }
    
    /// Sets the months for yearly recurrence.
    pub fn by_month(mut self, months: HashSet<MonthOfYear>) -> Self {
        self.by_month = Some(months);
        self
    }
    
    /// Sets the hours for recurrence.
    pub fn by_hour(mut self, hours: HashSet<u8>) -> Self {
        self.by_hour = Some(hours);
        self
    }
    
    /// Sets the minutes for recurrence.
    pub fn by_minute(mut self, minutes: HashSet<u8>) -> Self {
        self.by_minute = Some(minutes);
        self
    }
    
    /// Sets the seconds for recurrence.
    pub fn by_second(mut self, seconds: HashSet<u8>) -> Self {
        self.by_second = Some(seconds);
        self
    }
    
    // ========================================================================
    // Convenience Constructors
    // ========================================================================
    
    /// Creates a daily recurrence rule.
    pub fn daily() -> Self {
        Self::new(Frequency::Daily)
    }
    
    /// Creates a weekly recurrence rule.
    pub fn weekly() -> Self {
        Self::new(Frequency::Weekly)
    }
    
    /// Creates a monthly recurrence rule.
    pub fn monthly() -> Self {
        Self::new(Frequency::Monthly)
    }
    
    /// Creates a yearly recurrence rule.
    pub fn yearly() -> Self {
        Self::new(Frequency::Yearly)
    }
    
    /// Creates a business day (Monday-Friday) recurrence rule.
    pub fn business_days() -> Self {
        let mut weekdays = HashSet::new();
        weekdays.insert(DayOfWeek::Monday);
        weekdays.insert(DayOfWeek::Tuesday);
        weekdays.insert(DayOfWeek::Wednesday);
        weekdays.insert(DayOfWeek::Thursday);
        weekdays.insert(DayOfWeek::Friday);
        
        Self::new(Frequency::Weekly).by_weekday(weekdays)
    }
    
    /// Creates a weekend (Saturday-Sunday) recurrence rule.
    pub fn weekends() -> Self {
        let mut weekdays = HashSet::new();
        weekdays.insert(DayOfWeek::Saturday);
        weekdays.insert(DayOfWeek::Sunday);
        
        Self::new(Frequency::Weekly).by_weekday(weekdays)
    }
    
    /// Creates a rule for the first day of each month.
    pub fn first_of_month() -> Self {
        let mut month_days = HashSet::new();
        month_days.insert(1);
        
        Self::new(Frequency::Monthly).by_month_day(month_days)
    }
    
    /// Creates a rule for the last day of each month.
    pub fn last_of_month() -> Self {
        // This requires special handling since month lengths vary
        Self::new(Frequency::Monthly)
    }
    
    // ========================================================================
    // Pattern Generation
    // ========================================================================
    
    /// Generates occurrences starting from the given date/time.
    pub fn generate_occurrences(&self, start: &CalClock, max_occurrences: usize) -> Outcome<Vec<CalClock>> {
        let mut occurrences = Vec::new();
        let mut current = start.clone();
        let mut count = 0;
        
        while occurrences.len() < max_occurrences {
            // Check if we've reached the count limit
            if let Some(max_count) = self.count {
                if count >= max_count {
                    break;
                }
            }
            
            // Check if we've reached the until date
            if let Some(until_date) = &self.until {
                if current.date() > until_date {
                    break;
                }
            }
            
            // Check if this occurrence matches the pattern
            if res!(self.matches(&current)) {
                occurrences.push(current.clone());
            }
            
            // Move to the next potential occurrence
            current = res!(self.advance(&current));
            count += 1;
            
            // Safety check to prevent infinite loops
            if count > 10000 {
                break;
            }
        }
        
        Ok(occurrences)
    }
    
    /// Checks if a given date/time matches this recurrence pattern.
    pub fn matches(&self, datetime: &CalClock) -> Outcome<bool> {
        // Check weekday constraint
        if let Some(ref weekdays) = self.by_weekday {
            if !weekdays.contains(&datetime.day_of_week()) {
                return Ok(false);
            }
        }
        
        // Check month day constraint
        if let Some(ref month_days) = self.by_month_day {
            if !month_days.contains(&datetime.day()) {
                return Ok(false);
            }
        }
        
        // Check month constraint
        if let Some(ref months) = self.by_month {
            if !months.contains(&datetime.month_of_year()) {
                return Ok(false);
            }
        }
        
        // Check hour constraint
        if let Some(ref hours) = self.by_hour {
            if !hours.contains(&datetime.hour()) {
                return Ok(false);
            }
        }
        
        // Check minute constraint
        if let Some(ref minutes) = self.by_minute {
            if !minutes.contains(&datetime.minute()) {
                return Ok(false);
            }
        }
        
        // Check second constraint
        if let Some(ref seconds) = self.by_second {
            if !seconds.contains(&datetime.second()) {
                return Ok(false);
            }
        }
        
        Ok(true)
    }
    
    /// Advances to the next potential occurrence based on frequency and interval.
    fn advance(&self, current: &CalClock) -> Outcome<CalClock> {
        match self.frequency {
            Frequency::Secondly => current.add_seconds(self.interval as i32),
            Frequency::Minutely => current.add_minutes(self.interval as i32),
            Frequency::Hourly => current.add_hours(self.interval as i32),
            Frequency::Daily => current.add_days(self.interval as i32),
            Frequency::Weekly => current.add_weeks(self.interval as i32),
            Frequency::Monthly => current.add_months(self.interval as i32),
            Frequency::Yearly => current.add_years(self.interval as i32),
        }
    }
}

/// A recurrence pattern combines a start time with a recurrence rule.
#[derive(Clone, Debug)]
pub struct RecurrencePattern {
    /// The start date/time for the recurrence.
    start: CalClock,
    /// The recurrence rule.
    rule: RecurrenceRule,
    /// Optional exceptions (dates to skip).
    exceptions: HashSet<CalendarDate>,
}

impl RecurrencePattern {
    /// Creates a new recurrence pattern.
    pub fn new(start: CalClock, rule: RecurrenceRule) -> Self {
        Self {
            start,
            rule,
            exceptions: HashSet::new(),
        }
    }
    
    /// Adds an exception date (a date to skip).
    pub fn add_exception(&mut self, date: CalendarDate) {
        self.exceptions.insert(date);
    }
    
    /// Removes an exception date.
    pub fn remove_exception(&mut self, date: &CalendarDate) {
        self.exceptions.remove(date);
    }
    
    /// Returns all exception dates.
    pub fn exceptions(&self) -> &HashSet<CalendarDate> {
        &self.exceptions
    }
    
    /// Generates occurrences for this pattern.
    pub fn occurrences(&self, max_occurrences: usize) -> Outcome<Vec<CalClock>> {
        let mut all_occurrences = res!(self.rule.generate_occurrences(&self.start, max_occurrences * 2));
        
        // Filter out exceptions
        all_occurrences.retain(|occurrence| {
            !self.exceptions.contains(occurrence.date())
        });
        
        // Truncate to requested number
        all_occurrences.truncate(max_occurrences);
        
        Ok(all_occurrences)
    }
    
    /// Generates occurrences within a date range.
    pub fn occurrences_in_range(&self, start_date: &CalendarDate, end_date: &CalendarDate) -> Outcome<Vec<CalClock>> {
        let max_occurrences = 1000; // Safety limit
        let all_occurrences = res!(self.occurrences(max_occurrences));
        
        let filtered: Vec<CalClock> = all_occurrences.into_iter()
            .filter(|occurrence| {
                occurrence.date() >= start_date && occurrence.date() <= end_date
            })
            .collect();
        
        Ok(filtered)
    }
    
    /// Returns the next occurrence after the given date/time.
    pub fn next_occurrence_after(&self, after: &CalClock) -> Outcome<Option<CalClock>> {
        let max_occurrences = 100; // Reasonable limit for searching
        let occurrences = res!(self.rule.generate_occurrences(after, max_occurrences));
        
        for occurrence in occurrences {
            if occurrence > *after && !self.exceptions.contains(occurrence.date()) {
                return Ok(Some(occurrence));
            }
        }
        
        Ok(None)
    }
}

// ========================================================================
// Common Recurrence Patterns
// ========================================================================

impl RecurrencePattern {
    /// Creates a pattern for daily recurrence.
    pub fn daily(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::daily())
    }
    
    /// Creates a pattern for weekly recurrence.
    pub fn weekly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::weekly())
    }
    
    /// Creates a pattern for monthly recurrence.
    pub fn monthly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::monthly())
    }
    
    /// Creates a pattern for yearly recurrence.
    pub fn yearly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::yearly())
    }
    
    /// Creates a pattern for business days.
    pub fn business_days(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::business_days())
    }
    
    /// Creates a pattern for weekends.
    pub fn weekends(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::weekends())
    }
    
    /// Creates a pattern for the first day of each month.
    pub fn first_of_month(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::first_of_month())
    }
}