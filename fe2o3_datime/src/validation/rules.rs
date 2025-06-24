use crate::{
    calendar::{Calendar, CalendarDate},
    clock::ClockTime,
    time::CalClock,
    validation::{ValidationError, ValidationResult},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// A custom validation rule that can be applied to CalClock components.
///
/// ValidationRule provides a flexible way to define custom validation logic
/// for dates, times, and combined CalClock instances. Rules can be composed
/// and applied in sequence to implement complex validation requirements.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     validation::{ValidationRule, CalClockValidator},
///     time::CalClock,
/// }res!();
///
/// // Create a rule that only allows business hours
/// let business_hours_rule = ValidationRule::new("business_hours")
///     .with_time_validator(|time| {
///         let hour = time.hour().of()res!();
///         if hour >= 9 && hour < 17 {
///             Ok(())
///         } else {
///             Err(vec![ValidationError::new(
///                 "business_hours",
///                 "Time must be during business hours (9 AM - 5 PM)"
///             )])
///         }
///     })res!();
///
/// let mut validator = CalClockValidator::new()res!();
/// validator.add_rule(business_hours_rule)res!();
/// ```
pub struct ValidationRule {
    /// Name of this validation rule.
    name: String,
    /// Optional description of what this rule validates.
    description: Option<String>,
    /// Custom CalClock validation function.
    calclock_validator: Option<Box<dyn Fn(&CalClock) -> ValidationResult + Send + Sync>>,
    /// Custom CalendarDate validation function.
    date_validator: Option<Box<dyn Fn(&CalendarDate) -> ValidationResult + Send + Sync>>,
    /// Custom ClockTime validation function.
    time_validator: Option<Box<dyn Fn(&ClockTime) -> ValidationResult + Send + Sync>>,
}

impl std::fmt::Debug for ValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationRule")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("has_calclock_validator", &self.calclock_validator.is_some())
            .field("has_date_validator", &self.date_validator.is_some())
            .field("has_time_validator", &self.time_validator.is_some())
            .finish()
    }
}

impl ValidationRule {
    /// Creates a new validation rule with the given name.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            description: None,
            calclock_validator: None,
            date_validator: None,
            time_validator: None,
        }
    }
    
    /// Sets the description for this rule.
    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }
    
    /// Sets a custom CalClock validator function.
    pub fn with_calclock_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&CalClock) -> ValidationResult + Send + Sync + 'static,
    {
        self.calclock_validator = Some(Box::new(validator));
        self
    }
    
    /// Sets a custom CalendarDate validator function.
    pub fn with_date_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&CalendarDate) -> ValidationResult + Send + Sync + 'static,
    {
        self.date_validator = Some(Box::new(validator));
        self
    }
    
    /// Sets a custom ClockTime validator function.
    pub fn with_time_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&ClockTime) -> ValidationResult + Send + Sync + 'static,
    {
        self.time_validator = Some(Box::new(validator));
        self
    }
    
    /// Returns the name of this rule.
    pub fn name(&self) -> &str {
        &self.name
    }
    
    /// Returns the description of this rule.
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    
    /// Validates a CalClock using this rule.
    pub fn validate_calclock(&self, calclock: &CalClock) -> ValidationResult {
        if let Some(ref validator) = self.calclock_validator {
            validator(calclock)
        } else {
            Ok(())
        }
    }
    
    /// Validates a CalendarDate using this rule.
    pub fn validate_date(&self, date: &CalendarDate) -> ValidationResult {
        if let Some(ref validator) = self.date_validator {
            validator(date)
        } else {
            Ok(())
        }
    }
    
    /// Validates a ClockTime using this rule.
    pub fn validate_time(&self, time: &ClockTime) -> ValidationResult {
        if let Some(ref validator) = self.time_validator {
            validator(time)
        } else {
            Ok(())
        }
    }
}

/// A collection of commonly used validation rules.
///
/// ValidationRules provides pre-built validation rules for common scenarios
/// such as business hours, holidays, weekends, and date ranges. These rules
/// can be used directly or serve as examples for creating custom rules.
pub struct ValidationRules;

impl ValidationRules {
    /// Creates a rule that only allows business hours (9 AM - 5 PM, Monday-Friday).
    pub fn business_hours() -> ValidationRule {
        ValidationRule::new("business_hours")
            .description("Only allows times during standard business hours (9 AM - 5 PM, Monday-Friday)")
            .with_calclock_validator(|calclock| {
                // Check if it's a weekday
                if !calclock.date().is_business_day() {
                    return Err(vec![ValidationError::new(
                        "business_hours",
                        "Date must be a business day (Monday-Friday)"
                    )]);
                }
                
                // Check if time is within business hours
                let hour = calclock.time().hour().of();
                if hour < 9 || hour >= 17 {
                    return Err(vec![ValidationError::new(
                        "business_hours",
                        "Time must be during business hours (9 AM - 5 PM)"
                    ).field("hour")
                     .value(hour.to_string())
                     .expected("9 to 16")]);
                }
                
                Ok(())
            })
    }
    
    /// Creates a rule that only allows weekend dates.
    pub fn weekends_only() -> ValidationRule {
        ValidationRule::new("weekends_only")
            .description("Only allows weekend dates (Saturday and Sunday)")
            .with_date_validator(|date| {
                if !date.is_weekend() {
                    Err(vec![ValidationError::new(
                        "weekends_only",
                        "Date must be a weekend (Saturday or Sunday)"
                    ).field("day_of_week")
                     .value(date.day_of_week().long_name().to_string())
                     .expected("Saturday or Sunday")])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that excludes common holidays.
    pub fn no_holidays() -> ValidationRule {
        ValidationRule::new("no_holidays")
            .description("Excludes common holidays")
            .with_date_validator(|date| {
                // Simple holiday check - this should be expanded based on requirements
                let is_common_holiday = (date.month() == 12 && date.day() == 25) || // Christmas
                                       (date.month() == 1 && date.day() == 1) ||   // New Year
                                       (date.month() == 7 && date.day() == 4);     // Independence Day (US)
                
                if is_common_holiday {
                    Err(vec![ValidationError::new(
                        "no_holidays",
                        "Date cannot be a holiday"
                    ).field("date")
                     .value(date.to_string())
                     .expected("Non-holiday date")])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows specific days of the week.
    pub fn allowed_weekdays(allowed_days: HashSet<crate::constant::DayOfWeek>) -> ValidationRule {
        ValidationRule::new("allowed_weekdays")
            .description("Only allows specific days of the week")
            .with_date_validator(move |date| {
                if !allowed_days.contains(&date.day_of_week()) {
                    let allowed_names: Vec<String> = allowed_days
                        .iter()
                        .map(|day| day.long_name().to_string())
                        .collect();
                    
                    Err(vec![ValidationError::new(
                        "allowed_weekdays",
                        "Date must be one of the allowed weekdays"
                    ).field("day_of_week")
                     .value(date.day_of_week().long_name().to_string())
                     .expected(allowed_names.join(", "))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows times within a specific hour range.
    pub fn hour_range(min_hour: u8, max_hour: u8) -> ValidationRule {
        ValidationRule::new("hour_range")
            .description(format!("Only allows times between {} and {} hours", min_hour, max_hour))
            .with_time_validator(move |time| {
                let hour = time.hour().of();
                if hour < min_hour || hour > max_hour {
                    Err(vec![ValidationError::new(
                        "hour_range",
                        "Hour must be within allowed range"
                    ).field("hour")
                     .value(hour.to_string())
                     .expected(format!("{} to {}", min_hour, max_hour))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows dates after a minimum date.
    pub fn min_date(min_date: CalendarDate) -> ValidationRule {
        ValidationRule::new("min_date")
            .description(format!("Only allows dates on or after {}", min_date))
            .with_date_validator(move |date| {
                if date < &min_date {
                    Err(vec![ValidationError::new(
                        "min_date",
                        "Date must be on or after minimum date"
                    ).field("date")
                     .value(date.to_string())
                     .expected(format!("{} or later", min_date))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows dates before a maximum date.
    pub fn max_date(max_date: CalendarDate) -> ValidationRule {
        ValidationRule::new("max_date")
            .description(format!("Only allows dates on or before {}", max_date))
            .with_date_validator(move |date| {
                if date > &max_date {
                    Err(vec![ValidationError::new(
                        "max_date",
                        "Date must be on or before maximum date"
                    ).field("date")
                     .value(date.to_string())
                     .expected(format!("{} or earlier", max_date))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows dates within a specific year range.
    pub fn year_range(min_year: i32, max_year: i32) -> ValidationRule {
        ValidationRule::new("year_range")
            .description(format!("Only allows years between {} and {}", min_year, max_year))
            .with_date_validator(move |date| {
                let year = date.year();
                if year < min_year || year > max_year {
                    Err(vec![ValidationError::new(
                        "year_range",
                        "Year must be within allowed range"
                    ).field("year")
                     .value(year.to_string())
                     .expected(format!("{} to {}", min_year, max_year))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that only allows specific months.
    pub fn allowed_months(allowed_months: HashSet<crate::constant::MonthOfYear>) -> ValidationRule {
        ValidationRule::new("allowed_months")
            .description("Only allows specific months")
            .with_date_validator(move |date| {
                if !allowed_months.contains(&date.month_of_year()) {
                    let allowed_names: Vec<String> = allowed_months
                        .iter()
                        .map(|month| month.long_name().to_string())
                        .collect();
                    
                    Err(vec![ValidationError::new(
                        "allowed_months",
                        "Month must be one of the allowed months"
                    ).field("month")
                     .value(date.month_of_year().long_name().to_string())
                     .expected(allowed_names.join(", "))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that requires times to have zero nanoseconds (whole seconds only).
    pub fn whole_seconds_only() -> ValidationRule {
        ValidationRule::new("whole_seconds_only")
            .description("Only allows times with zero nanoseconds (whole seconds)")
            .with_time_validator(|time| {
                if time.nanosecond().of() != 0 {
                    Err(vec![ValidationError::new(
                        "whole_seconds_only",
                        "Time must have zero nanoseconds (whole seconds only)"
                    ).field("nanosecond")
                     .value(time.nanosecond().of().to_string())
                     .expected("0")])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that requires times to have zero seconds and nanoseconds (whole minutes only).
    pub fn whole_minutes_only() -> ValidationRule {
        ValidationRule::new("whole_minutes_only")
            .description("Only allows times with zero seconds and nanoseconds (whole minutes)")
            .with_time_validator(|time| {
                if time.second().of() != 0 || time.nanosecond().of() != 0 {
                    Err(vec![ValidationError::new(
                        "whole_minutes_only",
                        "Time must have zero seconds and nanoseconds (whole minutes only)"
                    ).field("time_precision")
                     .value(format!("{}s {}ns", time.second().of(), time.nanosecond().of()))
                     .expected("0s 0ns")])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that prevents dates too far in the past (more than specified years ago).
    pub fn not_too_old(max_years_ago: u32) -> ValidationRule {
        ValidationRule::new("not_too_old")
            .description(format!("Prevents dates more than {} years in the past", max_years_ago))
            .with_date_validator(move |date| {
                // Use a simple approximation: current year - max_years_ago
                let current_year = 2024; // This should ideally use current date
                let min_allowed_year = current_year - max_years_ago as i32;
                
                if date.year() < min_allowed_year {
                    Err(vec![ValidationError::new(
                        "not_too_old",
                        "Date cannot be too far in the past"
                    ).field("year")
                     .value(date.year().to_string())
                     .expected(format!("{} or later", min_allowed_year))])
                } else {
                    Ok(())
                }
            })
    }
    
    /// Creates a rule that prevents dates too far in the future (more than specified years ahead).
    pub fn not_too_future(max_years_ahead: u32) -> ValidationRule {
        ValidationRule::new("not_too_future")
            .description(format!("Prevents dates more than {} years in the future", max_years_ahead))
            .with_date_validator(move |date| {
                // Use a simple approximation: current year + max_years_ahead
                let current_year = 2024; // This should ideally use current date
                let max_allowed_year = current_year + max_years_ahead as i32;
                
                if date.year() > max_allowed_year {
                    Err(vec![ValidationError::new(
                        "not_too_future",
                        "Date cannot be too far in the future"
                    ).field("year")
                     .value(date.year().to_string())
                     .expected(format!("{} or earlier", max_allowed_year))])
                } else {
                    Ok(())
                }
            })
    }
}

// ========================================================================
// Rule Combinators
// ========================================================================

/// Combines multiple validation rules with AND logic.
pub fn all_rules(rules: Vec<ValidationRule>) -> ValidationRule {
    ValidationRule::new("combined_all")
        .description("All specified rules must pass")
        .with_calclock_validator(move |calclock| {
            let mut all_errors = Vec::new();
            
            for rule in &rules {
                if let Err(mut errors) = rule.validate_calclock(calclock) {
                    all_errors.append(&mut errors);
                }
            }
            
            if all_errors.is_empty() {
                Ok(())
            } else {
                Err(all_errors)
            }
        })
}

/// Combines multiple validation rules with OR logic.
pub fn any_rule(rules: Vec<ValidationRule>) -> ValidationRule {
    ValidationRule::new("combined_any")
        .description("At least one of the specified rules must pass")
        .with_calclock_validator(move |calclock| {
            for rule in &rules {
                if rule.validate_calclock(calclock).is_ok() {
                    return Ok(());
                }
            }
            
            Err(vec![ValidationError::new(
                "combined_any",
                "None of the alternative validation rules passed"
            )])
        })
}

// ========================================================================
// Common Rule Combinations
// ========================================================================

impl ValidationRules {
    /// Creates a comprehensive business rule (business hours + no holidays + weekdays only).
    pub fn strict_business() -> ValidationRule {
        let mut weekdays = HashSet::new();
        weekdays.insert(crate::constant::DayOfWeek::Monday);
        weekdays.insert(crate::constant::DayOfWeek::Tuesday);
        weekdays.insert(crate::constant::DayOfWeek::Wednesday);
        weekdays.insert(crate::constant::DayOfWeek::Thursday);
        weekdays.insert(crate::constant::DayOfWeek::Friday);
        
        all_rules(vec![
            Self::business_hours(),
            Self::no_holidays(),
            Self::allowed_weekdays(weekdays),
        ])
    }
    
    /// Creates a rule for scheduling appointments (business hours + whole minutes + reasonable timeframe).
    pub fn appointment_scheduling() -> ValidationRule {
        all_rules(vec![
            Self::business_hours(),
            Self::whole_minutes_only(),
            Self::not_too_old(1),
            Self::not_too_future(2),
        ])
    }
    
    /// Creates a rule for historical data (no future dates + reasonable past limit).
    pub fn historical_data() -> ValidationRule {
        all_rules(vec![
            Self::max_date({
                let calendar = Calendar::new();
                calendar.date(2024, 12, 31, crate::time::CalClockZone::utc()).unwrap()
            }),
            Self::not_too_old(100),
        ])
    }
}