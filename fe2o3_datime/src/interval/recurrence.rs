//! Recurrence rules and the patterns built from them.
//!
//! A rule fixes a frequency and an interval, then narrows the dates that
//! generates with by_weekday, by_month_day and the rest. A pattern pairs a
//! rule with a start time and a set of dates to skip.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::CalClock,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Hourly,
    Minutely,
    Secondly,
}

#[derive(Clone, Debug)]
pub struct RecurrenceRule {
    frequency:      Frequency,
    interval:       u32,                        // 2 means every other one
    until:          Option<CalendarDate>,
    count:          Option<u32>,
    // Filters applied to the dates the frequency generates.
    by_weekday:     Option<HashSet<DayOfWeek>>,
    by_month_day:   Option<HashSet<u8>>,        // 1-31
    by_month:       Option<HashSet<MonthOfYear>>,
    by_hour:        Option<HashSet<u8>>,        // 0-23
    by_minute:      Option<HashSet<u8>>,        // 0-59
    by_second:      Option<HashSet<u8>>,        // 0-59
}

impl RecurrenceRule {
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
    
    pub fn interval(mut self, interval: u32) -> Self {
        self.interval = interval.max(1);
        self
    }
    
    pub fn until(mut self, until: CalendarDate) -> Self {
        self.until = Some(until);
        self
    }
    
    pub fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }
    
    pub fn by_weekday(mut self, weekdays: HashSet<DayOfWeek>) -> Self {
        self.by_weekday = Some(weekdays);
        self
    }
    
    pub fn by_month_day(mut self, month_days: HashSet<u8>) -> Self {
        self.by_month_day = Some(month_days);
        self
    }
    
    pub fn by_month(mut self, months: HashSet<MonthOfYear>) -> Self {
        self.by_month = Some(months);
        self
    }
    
    pub fn by_hour(mut self, hours: HashSet<u8>) -> Self {
        self.by_hour = Some(hours);
        self
    }
    
    pub fn by_minute(mut self, minutes: HashSet<u8>) -> Self {
        self.by_minute = Some(minutes);
        self
    }
    
    pub fn by_second(mut self, seconds: HashSet<u8>) -> Self {
        self.by_second = Some(seconds);
        self
    }
    
    // ========================================================================
    // Convenience Constructors
    // ========================================================================
    
    pub fn daily() -> Self {
        Self::new(Frequency::Daily)
    }
    
    pub fn weekly() -> Self {
        Self::new(Frequency::Weekly)
    }
    
    pub fn monthly() -> Self {
        Self::new(Frequency::Monthly)
    }
    
    pub fn yearly() -> Self {
        Self::new(Frequency::Yearly)
    }
    
    pub fn business_days() -> Self {
        let mut weekdays = HashSet::new();
        weekdays.insert(DayOfWeek::Monday);
        weekdays.insert(DayOfWeek::Tuesday);
        weekdays.insert(DayOfWeek::Wednesday);
        weekdays.insert(DayOfWeek::Thursday);
        weekdays.insert(DayOfWeek::Friday);
        
        Self::new(Frequency::Weekly).by_weekday(weekdays)
    }
    
    pub fn weekends() -> Self {
        let mut weekdays = HashSet::new();
        weekdays.insert(DayOfWeek::Saturday);
        weekdays.insert(DayOfWeek::Sunday);
        
        Self::new(Frequency::Weekly).by_weekday(weekdays)
    }
    
    pub fn first_of_month() -> Self {
        let mut month_days = HashSet::new();
        month_days.insert(1);
        
        Self::new(Frequency::Monthly).by_month_day(month_days)
    }
    
    pub fn last_of_month() -> Self {
        // This requires special handling since month lengths vary
        Self::new(Frequency::Monthly)
    }
    
    // ========================================================================
    // Pattern Generation
    // ========================================================================
    
    /// Stops at whichever of the count, the until date and max_occurrences
    /// comes first, and gives up after ten thousand candidates in any case.
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

#[derive(Clone, Debug)]
pub struct RecurrencePattern {
    start:      CalClock,
    rule:       RecurrenceRule,
    exceptions: HashSet<CalendarDate>,  // dates to skip
}

impl RecurrencePattern {
    pub fn new(start: CalClock, rule: RecurrenceRule) -> Self {
        Self {
            start,
            rule,
            exceptions: HashSet::new(),
        }
    }
    
    pub fn add_exception(&mut self, date: CalendarDate) {
        self.exceptions.insert(date);
    }
    
    pub fn remove_exception(&mut self, date: &CalendarDate) {
        self.exceptions.remove(date);
    }
    
    pub fn exceptions(&self) -> &HashSet<CalendarDate> {
        &self.exceptions
    }
    
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
    pub fn daily(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::daily())
    }
    
    pub fn weekly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::weekly())
    }
    
    pub fn monthly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::monthly())
    }
    
    pub fn yearly(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::yearly())
    }
    
    pub fn business_days(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::business_days())
    }
    
    pub fn weekends(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::weekends())
    }
    
    pub fn first_of_month(start: CalClock) -> Self {
        Self::new(start, RecurrenceRule::first_of_month())
    }
}