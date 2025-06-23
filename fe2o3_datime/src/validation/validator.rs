use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClock,
    validation::ValidationRule,
};

use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a validation error with detailed context.
#[derive(Clone, Debug)]
pub struct ValidationError {
    /// The type of validation that failed.
    pub rule: String,
    /// Human-readable error message.
    pub message: String,
    /// The field that failed validation.
    pub field: Option<String>,
    /// The invalid value.
    pub value: Option<String>,
    /// Expected value or range.
    pub expected: Option<String>,
}

impl ValidationError {
    /// Creates a new validation error.
    pub fn new<S: Into<String>>(rule: S, message: S) -> Self {
        Self {
            rule: rule.into(),
            message: message.into(),
            field: None,
            value: None,
            expected: None,
        }
    }
    
    /// Sets the field that failed validation.
    pub fn field<S: Into<String>>(mut self, field: S) -> Self {
        self.field = Some(field.into());
        self
    }
    
    /// Sets the invalid value.
    pub fn value<S: Into<String>>(mut self, value: S) -> Self {
        self.value = Some(value.into());
        self
    }
    
    /// Sets the expected value or range.
    pub fn expected<S: Into<String>>(mut self, expected: S) -> Self {
        self.expected = Some(expected.into());
        self
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)?;
        
        if let Some(ref field) = self.field {
            write!(f, " (field: {})", field)?;
        }
        
        if let Some(ref value) = self.value {
            write!(f, " (value: {})", value)?;
        }
        
        if let Some(ref expected) = self.expected {
            write!(f, " (expected: {})", expected)?;
        }
        
        Ok(())
    }
}

/// Result type for validation operations.
pub type ValidationResult = Result<(), Vec<ValidationError>>;

/// Comprehensive validator for CalClock, CalendarDate, and ClockTime.
///
/// The CalClockValidator provides detailed validation with contextual error
/// messages for all date and time components. It supports custom validation
/// rules and provides both quick validation and detailed error reporting.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::{
///     validation::CalClockValidator,
///     time::CalClock,
/// };
///
/// let validator = CalClockValidator::new();
/// let calclock = res!(CalClock::now_utc());
///
/// // Quick validation
/// if validator.is_valid_calclock(&calclock) {
///     println!("CalClock is valid");
/// }
///
/// // Detailed validation with error messages
/// match validator.validate_calclock(&calclock) {
///     Ok(_) => println!("CalClock is valid"),
///     Err(errors) => {
///         for error in errors {
///             println!("Validation error: {}", error);
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct CalClockValidator {
    /// Custom validation rules.
    rules: Vec<ValidationRule>,
    /// Whether to enforce strict validation.
    strict: bool,
}

impl CalClockValidator {
    /// Creates a new validator with default settings.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            strict: false,
        }
    }
    
    /// Creates a new strict validator that enforces all validation rules.
    pub fn strict() -> Self {
        Self {
            rules: Vec::new(),
            strict: true,
        }
    }
    
    /// Adds a custom validation rule.
    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.push(rule);
    }
    
    /// Sets strict validation mode.
    pub fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }
    
    // ========================================================================
    // CalClock Validation
    // ========================================================================
    
    /// Validates a CalClock and returns detailed errors if invalid.
    pub fn validate_calclock(&self, calclock: &CalClock) -> ValidationResult {
        let mut errors = Vec::new();
        
        // Validate date component
        if let Err(mut date_errors) = self.validate_date(calclock.date()) {
            errors.append(&mut date_errors);
        }
        
        // Validate time component
        if let Err(mut time_errors) = self.validate_time(calclock.time()) {
            errors.append(&mut time_errors);
        }
        
        // Validate timezone consistency
        if calclock.date().zone() != calclock.time().zone() {
            errors.push(ValidationError::new(
                "timezone_consistency",
                "Date and time components must use the same timezone"
            ).field("timezone"));
        }
        
        // Apply custom rules
        for rule in &self.rules {
            if let Err(mut rule_errors) = rule.validate_calclock(calclock) {
                errors.append(&mut rule_errors);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Quick validation check for CalClock.
    pub fn is_valid_calclock(&self, calclock: &CalClock) -> bool {
        self.validate_calclock(calclock).is_ok()
    }
    
    // ========================================================================
    // CalendarDate Validation
    // ========================================================================
    
    /// Validates a CalendarDate and returns detailed errors if invalid.
    pub fn validate_date(&self, date: &CalendarDate) -> ValidationResult {
        let mut errors = Vec::new();
        
        // Validate year range
        if self.strict && (date.year() < -9999 || date.year() > 9999) {
            errors.push(ValidationError::new(
                "year_range",
                "Year must be between -9999 and 9999 in strict mode"
            ).field("year")
             .value(date.year().to_string())
             .expected("-9999 to 9999"));
        }
        
        // Validate month range
        if date.month() < 1 || date.month() > 12 {
            errors.push(ValidationError::new(
                "month_range",
                "Month must be between 1 and 12"
            ).field("month")
             .value(date.month().to_string())
             .expected("1 to 12"));
        }
        
        // Validate day range
        if date.day() < 1 {
            errors.push(ValidationError::new(
                "day_range",
                "Day must be at least 1"
            ).field("day")
             .value(date.day().to_string())
             .expected("1 or greater"));
        }
        
        // Validate day against month length
        if date.month() >= 1 && date.month() <= 12 {
            let days_in_month = date.month_of_year().days_in_month(date.year());
            if date.day() > days_in_month {
                errors.push(ValidationError::new(
                    "day_in_month",
                    "Day does not exist in this month"
                ).field("day")
                 .value(date.day().to_string())
                 .expected(format!("1 to {}", days_in_month)));
            }
        }
        
        // Validate leap year February 29
        if date.month() == 2 && date.day() == 29 && !date.is_leap_year() {
            errors.push(ValidationError::new(
                "leap_year",
                "February 29 does not exist in non-leap year"
            ).field("day")
             .value("29".to_string())
             .expected("1 to 28 (non-leap year)"));
        }
        
        // Apply custom rules
        for rule in &self.rules {
            if let Err(mut rule_errors) = rule.validate_date(date) {
                errors.append(&mut rule_errors);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Quick validation check for CalendarDate.
    pub fn is_valid_date(&self, date: &CalendarDate) -> bool {
        self.validate_date(date).is_ok()
    }
    
    // ========================================================================
    // ClockTime Validation
    // ========================================================================
    
    /// Validates a ClockTime and returns detailed errors if invalid.
    pub fn validate_time(&self, time: &ClockTime) -> ValidationResult {
        let mut errors = Vec::new();
        
        // Validate hour range
        if time.hour().of() > 23 {
            errors.push(ValidationError::new(
                "hour_range",
                "Hour must be between 0 and 23"
            ).field("hour")
             .value(time.hour().of().to_string())
             .expected("0 to 23"));
        }
        
        // Validate minute range
        if time.minute().of() > 59 {
            errors.push(ValidationError::new(
                "minute_range",
                "Minute must be between 0 and 59"
            ).field("minute")
             .value(time.minute().of().to_string())
             .expected("0 to 59"));
        }
        
        // Validate second range
        if time.second().of() > 59 {
            errors.push(ValidationError::new(
                "second_range",
                "Second must be between 0 and 59"
            ).field("second")
             .value(time.second().of().to_string())
             .expected("0 to 59"));
        }
        
        // Validate nanosecond range
        if time.nanosecond().of() >= 1_000_000_000 {
            errors.push(ValidationError::new(
                "nanosecond_range",
                "Nanosecond must be between 0 and 999,999,999"
            ).field("nanosecond")
             .value(time.nanosecond().of().to_string())
             .expected("0 to 999,999,999"));
        }
        
        // Strict validation for leap seconds
        if self.strict && time.second().of() == 60 {
            errors.push(ValidationError::new(
                "leap_second",
                "Leap seconds (second = 60) are not supported in strict mode"
            ).field("second")
             .value("60".to_string())
             .expected("0 to 59"));
        }
        
        // Apply custom rules
        for rule in &self.rules {
            if let Err(mut rule_errors) = rule.validate_time(time) {
                errors.append(&mut rule_errors);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Quick validation check for ClockTime.
    pub fn is_valid_time(&self, time: &ClockTime) -> bool {
        self.validate_time(time).is_ok()
    }
    
    // ========================================================================
    // Batch Validation
    // ========================================================================
    
    /// Validates multiple CalClocks and returns all errors.
    pub fn validate_many_calclocks(&self, calclocks: &[CalClock]) -> ValidationResult {
        let mut all_errors = Vec::new();
        
        for (index, calclock) in calclocks.iter().enumerate() {
            if let Err(mut errors) = self.validate_calclock(calclock) {
                // Add index information to errors
                for error in &mut errors {
                    error.field = Some(format!("item[{}].{}", index, 
                        error.field.as_deref().unwrap_or("unknown")));
                }
                all_errors.append(&mut errors);
            }
        }
        
        if all_errors.is_empty() {
            Ok(())
        } else {
            Err(all_errors)
        }
    }
    
    /// Returns the number of valid CalClocks in a collection.
    pub fn count_valid_calclocks(&self, calclocks: &[CalClock]) -> usize {
        calclocks.iter()
            .filter(|calclock| self.is_valid_calclock(calclock))
            .count()
    }
    
    /// Filters a collection to return only valid CalClocks.
    pub fn filter_valid_calclocks(&self, calclocks: Vec<CalClock>) -> Vec<CalClock> {
        calclocks.into_iter()
            .filter(|calclock| self.is_valid_calclock(calclock))
            .collect()
    }
    
    // ========================================================================
    // Range Validation
    // ========================================================================
    
    /// Validates that a date falls within a specified range.
    pub fn validate_date_range(
        &self,
        date: &CalendarDate,
        min_date: &CalendarDate,
        max_date: &CalendarDate,
    ) -> ValidationResult {
        let mut errors = Vec::new();
        
        if date < min_date {
            errors.push(ValidationError::new(
                "date_range_min",
                "Date is before minimum allowed date"
            ).field("date")
             .value(date.to_string())
             .expected("minimum date or later"));
        }
        
        if date > max_date {
            errors.push(ValidationError::new(
                "date_range_max",
                "Date is after maximum allowed date"
            ).field("date")
             .value(date.to_string())
             .expected("maximum date or earlier"));
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Validates that a time falls within a specified range.
    pub fn validate_time_range(
        &self,
        time: &ClockTime,
        min_time: &ClockTime,
        max_time: &ClockTime,
    ) -> ValidationResult {
        let mut errors = Vec::new();
        
        if time < min_time {
            errors.push(ValidationError::new(
                "time_range_min",
                "Time is before minimum allowed time"
            ).field("time")
             .value(time.to_string())
             .expected("minimum time or later"));
        }
        
        if time > max_time {
            errors.push(ValidationError::new(
                "time_range_max",
                "Time is after maximum allowed time"
            ).field("time")
             .value(time.to_string())
             .expected("maximum time or earlier"));
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for CalClockValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ========================================================================
// Convenience Functions
// ========================================================================

/// Quick validation function for CalClock.
pub fn validate_calclock(calclock: &CalClock) -> ValidationResult {
    CalClockValidator::new().validate_calclock(calclock)
}

/// Quick validation function for CalendarDate.
pub fn validate_date(date: &CalendarDate) -> ValidationResult {
    CalClockValidator::new().validate_date(date)
}

/// Quick validation function for ClockTime.
pub fn validate_time(time: &ClockTime) -> ValidationResult {
    CalClockValidator::new().validate_time(time)
}

/// Strict validation function for CalClock.
pub fn validate_calclock_strict(calclock: &CalClock) -> ValidationResult {
    CalClockValidator::strict().validate_calclock(calclock)
}

/// Checks if a CalClock is valid.
pub fn is_valid_calclock(calclock: &CalClock) -> bool {
    validate_calclock(calclock).is_ok()
}

/// Checks if a CalendarDate is valid.
pub fn is_valid_date(date: &CalendarDate) -> bool {
    validate_date(date).is_ok()
}

/// Checks if a ClockTime is valid.
pub fn is_valid_time(time: &ClockTime) -> bool {
    validate_time(time).is_ok()
}