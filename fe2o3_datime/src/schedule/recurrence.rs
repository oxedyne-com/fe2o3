/// Recurrence patterns and scheduling rules for recurring tasks
/// 
/// This module provides comprehensive support for defining when and how
/// often scheduled tasks should recur, including cron-style expressions
/// and business calendar integration.

use oxedyne_fe2o3_core::prelude::*;
use crate::{
    time::{CalClock, CalClockZone},
    constant::{DayOfWeek, MonthOfYear},
    calendar::Calendar,
};
use std::collections::HashSet;

/// Predefined recurrence patterns for common scheduling needs
#[derive(Debug, Clone, PartialEq)]
pub enum RecurrencePattern {
    /// Execute once every N minutes
    EveryMinutes(u32),
    /// Execute once every N hours
    EveryHours(u32),
    /// Execute once every day at the same time
    Daily,
    /// Execute once every N days
    EveryDays(u32),
    /// Execute on specific days of the week
    Weekly(HashSet<DayOfWeek>),
    /// Execute once every N weeks on the same day
    EveryWeeks(u32),
    /// Execute on a specific day of each month
    Monthly(u8), // Day of month (1-31)
    /// Execute once every N months
    EveryMonths(u32),
    /// Execute on specific months of the year
    Yearly(HashSet<MonthOfYear>),
    /// Execute based on business days (excludes weekends and holidays)
    BusinessDaily,
    /// Execute on the first/last/nth occurrence of a weekday in a month
    MonthlyWeekday {
        week: WeekOccurrence,
        day: DayOfWeek,
    },
    /// Custom cron-style expression
    Cron(CronExpression),
}

/// Week occurrence in a month (first, second, last, etc.)
#[derive(Debug, Clone, PartialEq)]
pub enum WeekOccurrence {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

impl RecurrencePattern {
    /// Calculates the next execution time based on the current time
    pub fn next_execution(&self, current_time: &CalClock, zone: &CalClockZone) -> Outcome<Option<CalClock>> {
        match self {
            RecurrencePattern::EveryMinutes(minutes) => {
                let next = res!(current_time.add_minutes(*minutes as i32));
                Ok(Some(next))
            },
            RecurrencePattern::EveryHours(hours) => {
                let next = res!(current_time.add_hours(*hours as i32));
                Ok(Some(next))
            },
            RecurrencePattern::Daily => {
                let next = res!(current_time.add_days(1));
                Ok(Some(next))
            },
            RecurrencePattern::EveryDays(days) => {
                let next = res!(current_time.add_days(*days as i32));
                Ok(Some(next))
            },
            RecurrencePattern::Weekly(days) => {
                self.next_weekly_execution(current_time, days)
            },
            RecurrencePattern::EveryWeeks(weeks) => {
                let next = res!(current_time.add_days((*weeks as i32) * 7));
                Ok(Some(next))
            },
            RecurrencePattern::Monthly(day) => {
                self.next_monthly_execution(current_time, *day)
            },
            RecurrencePattern::EveryMonths(months) => {
                let next = res!(current_time.add_months(*months as i32));
                Ok(Some(next))
            },
            RecurrencePattern::Yearly(months) => {
                self.next_yearly_execution(current_time, months)
            },
            RecurrencePattern::BusinessDaily => {
                self.next_business_day_execution(current_time, zone)
            },
            RecurrencePattern::MonthlyWeekday { week, day } => {
                self.next_monthly_weekday_execution(current_time, week, day, zone)
            },
            RecurrencePattern::Cron(cron) => {
                cron.next_execution(current_time)
            },
        }
    }

    /// Calculates next weekly execution
    fn next_weekly_execution(&self, current_time: &CalClock, target_days: &HashSet<DayOfWeek>) -> Outcome<Option<CalClock>> {
        let current_day = current_time.day_of_week();
        let current_day_num = current_day.of() as i32;
        
        // Find the next occurrence in this week
        for day_offset in 1..=7 {
            let next_day_num = (current_day_num + day_offset - 1) % 7 + 1;
            if let Ok(next_day) = DayOfWeek::from_number(next_day_num as u8) {
                if target_days.contains(&next_day) {
                    let next_time = res!(current_time.add_days(day_offset));
                    return Ok(Some(next_time));
                }
            }
        }
        
        Ok(None) // Should not happen if target_days is not empty
    }

    /// Calculates next monthly execution on a specific day
    fn next_monthly_execution(&self, current_time: &CalClock, target_day: u8) -> Outcome<Option<CalClock>> {
        let current_day = current_time.day();
        let current_month = current_time.month();
        let current_year = current_time.year();
        
        // Try current month first
        if target_day > current_day {
            if let Ok(next_time) = CalClock::new(
                current_year, current_month, target_day,
                current_time.hour(), current_time.minute(), current_time.second(), current_time.nanosecond(),
                current_time.zone().clone()
            ) {
                return Ok(Some(next_time));
            }
        }
        
        // Move to next month
        let (next_year, next_month) = if current_month == 12 {
            (current_year + 1, 1)
        } else {
            (current_year, current_month + 1)
        };
        
        // Find valid day in next month (handle month-end edge cases)
        let month_enum = res!(MonthOfYear::from_number(next_month));
        let days_in_month = month_enum.days_in_month(next_year);
        let actual_day = std::cmp::min(target_day, days_in_month);
        
        let next_time = res!(CalClock::new(
            next_year, next_month, actual_day,
            current_time.hour(), current_time.minute(), current_time.second(), current_time.nanosecond(),
            current_time.zone().clone()
        ));
        
        Ok(Some(next_time))
    }

    /// Calculates next yearly execution
    fn next_yearly_execution(&self, current_time: &CalClock, target_months: &HashSet<MonthOfYear>) -> Outcome<Option<CalClock>> {
        let _current_month = current_time.month_of_year();
        let current_year = current_time.year();
        
        // Try remaining months in current year
        for month_num in (current_time.month() + 1)..=12 {
            if let Ok(month) = MonthOfYear::from_number(month_num) {
                if target_months.contains(&month) {
                    let next_time = res!(CalClock::new(
                        current_year, month_num, current_time.day(),
                        current_time.hour(), current_time.minute(), current_time.second(), current_time.nanosecond(),
                        current_time.zone().clone()
                    ));
                    return Ok(Some(next_time));
                }
            }
        }
        
        // Move to next year, find first matching month
        for month_num in 1..=12 {
            if let Ok(month) = MonthOfYear::from_number(month_num) {
                if target_months.contains(&month) {
                    let next_time = res!(CalClock::new(
                        current_year + 1, month_num, current_time.day(),
                        current_time.hour(), current_time.minute(), current_time.second(), current_time.nanosecond(),
                        current_time.zone().clone()
                    ));
                    return Ok(Some(next_time));
                }
            }
        }
        
        Ok(None)
    }

    /// Calculates next business day execution
    fn next_business_day_execution(&self, current_time: &CalClock, _zone: &CalClockZone) -> Outcome<Option<CalClock>> {
        let mut candidate = res!(current_time.add_days(1));
        
        // Find next business day (up to 10 days ahead to avoid infinite loop)
        for _ in 0..10 {
            let day_of_week = candidate.day_of_week();
            // Simple business day check - Monday through Friday
            if !matches!(day_of_week, DayOfWeek::Saturday | DayOfWeek::Sunday) {
                return Ok(Some(candidate));
            }
            candidate = res!(candidate.add_days(1));
        }
        
        Err(err!("Could not find next business day within 10 days"; Invalid, Range))
    }

    /// Calculates next monthly weekday execution (e.g., "first Monday of month")
    fn next_monthly_weekday_execution(
        &self, 
        current_time: &CalClock, 
        week: &WeekOccurrence, 
        target_day: &DayOfWeek, 
        zone: &CalClockZone
    ) -> Outcome<Option<CalClock>> {
        let current_year = current_time.year();
        let current_month = current_time.month();
        
        // Try current month first
        if let Some(target_date) = self.find_monthly_weekday(current_year, current_month, week, target_day, zone)? {
            if target_date > *current_time {
                return Ok(Some(target_date));
            }
        }
        
        // Move to next month
        let (next_year, next_month) = if current_month == 12 {
            (current_year + 1, 1)
        } else {
            (current_year, current_month + 1)
        };
        
        if let Some(target_date) = self.find_monthly_weekday(next_year, next_month, week, target_day, zone)? {
            Ok(Some(target_date))
        } else {
            Err(err!("Could not calculate monthly weekday occurrence"; Invalid, Range))
        }
    }

    /// Finds a specific weekday occurrence in a month
    fn find_monthly_weekday(
        &self,
        year: i32,
        month: u8,
        week: &WeekOccurrence,
        target_day: &DayOfWeek,
        zone: &CalClockZone
    ) -> Outcome<Option<CalClock>> {
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        let mut candidates = Vec::new();
        
        // Find all occurrences of target_day in the month
        for day in 1..=days_in_month {
            if let Ok(date) = CalClock::new(year, month, day, 0, 0, 0, 0, zone.clone()) {
                if date.day_of_week() == *target_day {
                    candidates.push(date);
                }
            }
        }
        
        if candidates.is_empty() {
            return Ok(None);
        }
        
        let selected_date = match week {
            WeekOccurrence::First => candidates[0].clone(),
            WeekOccurrence::Second => {
                if candidates.len() >= 2 {
                    candidates[1].clone()
                } else {
                    return Ok(None);
                }
            },
            WeekOccurrence::Third => {
                if candidates.len() >= 3 {
                    candidates[2].clone()
                } else {
                    return Ok(None);
                }
            },
            WeekOccurrence::Fourth => {
                if candidates.len() >= 4 {
                    candidates[3].clone()
                } else {
                    return Ok(None);
                }
            },
            WeekOccurrence::Last => candidates.last().unwrap().clone(),
        };
        
        Ok(Some(selected_date))
    }
}

/// Simplified cron expression support
#[derive(Debug, Clone, PartialEq)]
pub struct CronExpression {
    /// Minute (0-59)
    pub minute: CronField,
    /// Hour (0-23)
    pub hour: CronField,
    /// Day of month (1-31)
    pub day: CronField,
    /// Month (1-12)
    pub month: CronField,
    /// Day of week (0-6, Sunday = 0)
    pub day_of_week: CronField,
}

/// Cron field specification
#[derive(Debug, Clone, PartialEq)]
pub enum CronField {
    /// Any value (*)
    Any,
    /// Specific value
    Value(u8),
    /// List of values
    List(Vec<u8>),
    /// Range of values
    Range(u8, u8),
    /// Step values (e.g., */5)
    Step(u8),
}

impl CronExpression {
    /// Creates a new cron expression from string (simplified parser)
    pub fn parse(expr: &str) -> Outcome<Self> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(err!("Cron expression must have 5 fields: minute hour day month day_of_week"; Invalid, Input));
        }
        
        Ok(CronExpression {
            minute: res!(Self::parse_field(parts[0])),
            hour: res!(Self::parse_field(parts[1])),
            day: res!(Self::parse_field(parts[2])),
            month: res!(Self::parse_field(parts[3])),
            day_of_week: res!(Self::parse_field(parts[4])),
        })
    }

    /// Creates a cron expression for daily execution at a specific time
    pub fn daily(hour: u8, minute: u8) -> Self {
        CronExpression {
            minute: CronField::Value(minute),
            hour: CronField::Value(hour),
            day: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::Any,
        }
    }

    /// Creates a cron expression for weekly execution
    pub fn weekly(day_of_week: DayOfWeek, hour: u8, minute: u8) -> Self {
        CronExpression {
            minute: CronField::Value(minute),
            hour: CronField::Value(hour),
            day: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::Value(day_of_week.of() % 7), // Convert to cron format (0=Sunday)
        }
    }

    /// Calculates the next execution time
    pub fn next_execution(&self, current_time: &CalClock) -> Outcome<Option<CalClock>> {
        // Simplified implementation - finds next matching minute
        let mut candidate = res!(current_time.add_minutes(1));
        
        // Search for next matching time (limit to reasonable range)
        for _ in 0..(60 * 24 * 32) { // Search up to 32 days
            if self.matches_time(&candidate) {
                return Ok(Some(candidate));
            }
            candidate = res!(candidate.add_minutes(1));
        }
        
        Err(err!("Could not find next cron execution within 32 days"; Invalid, Range))
    }

    /// Checks if a time matches the cron expression
    fn matches_time(&self, time: &CalClock) -> bool {
        self.field_matches(&self.minute, time.minute()) &&
        self.field_matches(&self.hour, time.hour()) &&
        self.field_matches(&self.day, time.day()) &&
        self.field_matches(&self.month, time.month()) &&
        self.field_matches(&self.day_of_week, time.day_of_week().of() % 7)
    }

    /// Checks if a field matches the current value
    fn field_matches(&self, field: &CronField, value: u8) -> bool {
        match field {
            CronField::Any => true,
            CronField::Value(v) => *v == value,
            CronField::List(values) => values.contains(&value),
            CronField::Range(start, end) => value >= *start && value <= *end,
            CronField::Step(step) => value % step == 0,
        }
    }

    /// Parses a single cron field
    fn parse_field(field: &str) -> Outcome<CronField> {
        if field == "*" {
            Ok(CronField::Any)
        } else if field.contains(',') {
            let values: Result<Vec<u8>, _> = field.split(',')
                .map(|s| s.parse::<u8>())
                .collect();
            Ok(CronField::List(res!(values.map_err(|e| err!("Invalid cron field value: {}", e; Invalid, Input)))))
        } else if field.contains('-') {
            let parts: Vec<&str> = field.split('-').collect();
            if parts.len() != 2 {
                return Err(err!("Invalid range format in cron field"; Invalid, Input));
            }
            let start = res!(parts[0].parse::<u8>().map_err(|e| err!("Invalid range start: {}", e; Invalid, Input)));
            let end = res!(parts[1].parse::<u8>().map_err(|e| err!("Invalid range end: {}", e; Invalid, Input)));
            Ok(CronField::Range(start, end))
        } else if field.starts_with("*/") {
            let step_str = &field[2..];
            let step = res!(step_str.parse::<u8>().map_err(|e| err!("Invalid step value: {}", e; Invalid, Input)));
            Ok(CronField::Step(step))
        } else {
            let value = res!(field.parse::<u8>().map_err(|e| err!("Invalid field value: {}", e; Invalid, Input)));
            Ok(CronField::Value(value))
        }
    }
}

/// Rule-based recurrence for complex scheduling scenarios
#[derive(Debug, Clone)]
pub struct RecurrenceRule {
    /// Base recurrence pattern
    pub pattern: RecurrencePattern,
    /// Exceptions - dates when the rule should not apply
    pub exceptions: HashSet<CalClock>,
    /// Overrides - specific dates when the rule should apply regardless
    pub overrides: HashSet<CalClock>,
    /// Business calendar integration
    pub respect_business_calendar: bool,
    /// Time zone for calculations
    pub zone: CalClockZone,
}

impl RecurrenceRule {
    /// Creates a new recurrence rule
    pub fn new(pattern: RecurrencePattern, zone: CalClockZone) -> Self {
        RecurrenceRule {
            pattern,
            exceptions: HashSet::new(),
            overrides: HashSet::new(),
            respect_business_calendar: false,
            zone,
        }
    }

    /// Adds an exception date
    pub fn add_exception(mut self, date: CalClock) -> Self {
        self.exceptions.insert(date);
        self
    }

    /// Adds an override date
    pub fn add_override(mut self, date: CalClock) -> Self {
        self.overrides.insert(date);
        self
    }

    /// Enables business calendar integration
    pub fn with_business_calendar(mut self) -> Self {
        self.respect_business_calendar = true;
        self
    }

    /// Calculates the next execution time considering all rules
    pub fn next_execution(&self, current_time: &CalClock) -> Outcome<Option<CalClock>> {
        // Check for immediate overrides
        for override_date in &self.overrides {
            if override_date > current_time {
                return Ok(Some(override_date.clone()));
            }
        }

        // Get next time from base pattern
        let mut candidate = res!(self.pattern.next_execution(current_time, &self.zone));

        while let Some(next_time) = candidate {
            // Check if it's an exception
            if self.exceptions.contains(&next_time) {
                candidate = res!(self.pattern.next_execution(&next_time, &self.zone));
                continue;
            }

            // Check business calendar if enabled
            if self.respect_business_calendar {
                let _calendar = Calendar::new();
                let calendar_date = next_time.date();
                if !calendar_date.is_business_day() {
                    candidate = res!(self.pattern.next_execution(&next_time, &self.zone));
                    continue;
                }
            }

            return Ok(Some(next_time));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily_recurrence() {
        let zone = CalClockZone::utc();
        let pattern = RecurrencePattern::Daily;
        let current = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap();
        
        let next = pattern.next_execution(&current, &zone).unwrap().unwrap();
        assert_eq!(next.day(), 2);
        assert_eq!(next.hour(), 12);
    }

    #[test]
    fn test_weekly_recurrence() {
        let zone = CalClockZone::utc();
        let mut days = HashSet::new();
        days.insert(DayOfWeek::Monday);
        days.insert(DayOfWeek::Friday);
        
        let pattern = RecurrencePattern::Weekly(days);
        let current = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap(); // Monday
        
        let next = pattern.next_execution(&current, &zone).unwrap().unwrap();
        assert_eq!(next.day_of_week(), DayOfWeek::Friday);
    }

    #[test]
    fn test_cron_expression_parsing() {
        let cron = CronExpression::parse("0 9 * * 1").unwrap(); // Every Monday at 9 AM
        
        assert_eq!(cron.minute, CronField::Value(0));
        assert_eq!(cron.hour, CronField::Value(9));
        assert_eq!(cron.day, CronField::Any);
        assert_eq!(cron.month, CronField::Any);
        assert_eq!(cron.day_of_week, CronField::Value(1));
    }

    #[test]
    fn test_cron_daily() {
        let cron = CronExpression::daily(14, 30); // 2:30 PM daily
        
        assert_eq!(cron.minute, CronField::Value(30));
        assert_eq!(cron.hour, CronField::Value(14));
        assert_eq!(cron.day, CronField::Any);
        assert_eq!(cron.month, CronField::Any);
        assert_eq!(cron.day_of_week, CronField::Any);
    }

    #[test]
    fn test_recurrence_rule_exceptions() {
        let zone = CalClockZone::utc();
        let pattern = RecurrencePattern::Daily;
        
        let exception_date = CalClock::new(2024, 1, 2, 12, 0, 0, 0, zone.clone()).unwrap();
        let rule = RecurrenceRule::new(pattern, zone.clone())
            .add_exception(exception_date);
        
        let current = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone).unwrap();
        let next = rule.next_execution(&current).unwrap().unwrap();
        
        // Should skip January 2nd and go to January 3rd
        assert_eq!(next.day(), 3);
    }
}