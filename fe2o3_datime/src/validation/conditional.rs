use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClock,
    validation::{ValidationError, ValidationRule},
    constant::{DayOfWeek, MonthOfYear},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

/// Advanced conditional validation rule system.
///
/// ConditionalRule allows complex validation logic with conditions, branches,
/// and context-aware rules. This enables sophisticated validation scenarios
/// like "if it's a weekend, allow extended hours" or "if it's December,
/// require holiday scheduling rules".
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::validation::{ConditionalRule, ValidationCondition};
///
/// // Rule: Allow extended hours on weekends
/// let weekend_rule = ConditionalRule::new("weekend_extended_hours")
///     .condition(ValidationCondition::IsWeekend)
///     .if_true(ValidationRules::hour_range(0, 23))  // 24 hour access
///     .if_false(ValidationRules::hour_range(9, 17)); // Business hours only
/// ```
#[derive(Debug)]
pub struct ConditionalRule {
    /// Name of this conditional rule.
    name: String,
    /// The condition to evaluate.
    condition: ValidationCondition,
    /// Rule to apply if condition is true.
    true_rule: Option<ValidationRule>,
    /// Rule to apply if condition is false.
    false_rule: Option<ValidationRule>,
    /// Rules to apply regardless of condition.
    always_rules: Vec<ValidationRule>,
}

impl ConditionalRule {
    /// Creates a new conditional rule.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            condition: ValidationCondition::Always,
            true_rule: None,
            false_rule: None,
            always_rules: Vec::new(),
        }
    }

    /// Sets the condition for this rule.
    pub fn condition(mut self, condition: ValidationCondition) -> Self {
        self.condition = condition;
        self
    }

    /// Sets the rule to apply when condition is true.
    pub fn if_true(mut self, rule: ValidationRule) -> Self {
        self.true_rule = Some(rule);
        self
    }

    /// Sets the rule to apply when condition is false.
    pub fn if_false(mut self, rule: ValidationRule) -> Self {
        self.false_rule = Some(rule);
        self
    }

    /// Adds a rule that always applies regardless of condition.
    pub fn always(mut self, rule: ValidationRule) -> Self {
        self.always_rules.push(rule);
        self
    }

    /// Converts this conditional rule into a standard ValidationRule.
    pub fn into_rule(self) -> ValidationRule {
        let name = self.name.clone();
        let condition = self.condition;
        let true_rule = self.true_rule;
        let false_rule = self.false_rule;
        let always_rules = self.always_rules;
        
        ValidationRule::new(name)
            .with_calclock_validator(move |calclock| {
                let mut errors = Vec::new();

                // Apply always rules first
                for rule in &always_rules {
                    if let Err(mut rule_errors) = rule.validate_calclock(calclock) {
                        errors.append(&mut rule_errors);
                    }
                }

                // Evaluate condition and apply appropriate rule
                let condition_met = condition.evaluate_calclock(calclock);
                
                let applicable_rule = if condition_met {
                    &true_rule
                } else {
                    &false_rule
                };

                if let Some(rule) = applicable_rule {
                    if let Err(mut rule_errors) = rule.validate_calclock(calclock) {
                        errors.append(&mut rule_errors);
                    }
                }

                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors)
                }
            })
    }

    /// Gets the name of this rule.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Validation conditions for conditional rules.
pub enum ValidationCondition {
    /// Always true.
    Always,
    /// Never true.
    Never,
    /// True if date is a weekend.
    IsWeekend,
    /// True if date is a business day.
    IsBusinessDay,
    /// True if date is a specific day of week.
    IsDayOfWeek(DayOfWeek),
    /// True if date is in a specific month.
    IsMonth(MonthOfYear),
    /// True if date is in a set of months.
    IsMonthIn(HashSet<MonthOfYear>),
    /// True if hour is in a specific range.
    IsHourInRange(u8, u8),
    /// True if year is in a specific range.
    IsYearInRange(i32, i32),
    /// True if day of month is in a specific range.
    IsDayInRange(u8, u8),
    /// True if date is a leap year.
    IsLeapYear,
    /// True if date is after a specific date.
    IsAfterDate(i32, u8, u8), // year, month, day
    /// True if date is before a specific date.
    IsBeforeDate(i32, u8, u8), // year, month, day
    /// True if time has fractional seconds.
    HasFractionalSeconds,
    /// True if all specified conditions are true (AND).
    And(Vec<ValidationCondition>),
    /// True if any specified condition is true (OR).
    Or(Vec<ValidationCondition>),
    /// True if the specified condition is false (NOT).
    Not(Box<ValidationCondition>),
    /// Custom condition with user-defined logic.
    Custom(Box<dyn Fn(&CalClock) -> bool + Send + Sync>),
}

impl Clone for ValidationCondition {
    fn clone(&self) -> Self {
        match self {
            ValidationCondition::Always => ValidationCondition::Always,
            ValidationCondition::Never => ValidationCondition::Never,
            ValidationCondition::IsWeekend => ValidationCondition::IsWeekend,
            ValidationCondition::IsBusinessDay => ValidationCondition::IsBusinessDay,
            ValidationCondition::IsDayOfWeek(day) => ValidationCondition::IsDayOfWeek(*day),
            ValidationCondition::IsMonth(month) => ValidationCondition::IsMonth(*month),
            ValidationCondition::IsMonthIn(months) => ValidationCondition::IsMonthIn(months.clone()),
            ValidationCondition::IsHourInRange(min, max) => ValidationCondition::IsHourInRange(*min, *max),
            ValidationCondition::IsYearInRange(min, max) => ValidationCondition::IsYearInRange(*min, *max),
            ValidationCondition::IsDayInRange(min, max) => ValidationCondition::IsDayInRange(*min, *max),
            ValidationCondition::IsLeapYear => ValidationCondition::IsLeapYear,
            ValidationCondition::IsAfterDate(y, m, d) => ValidationCondition::IsAfterDate(*y, *m, *d),
            ValidationCondition::IsBeforeDate(y, m, d) => ValidationCondition::IsBeforeDate(*y, *m, *d),
            ValidationCondition::HasFractionalSeconds => ValidationCondition::HasFractionalSeconds,
            ValidationCondition::And(conditions) => ValidationCondition::And(conditions.clone()),
            ValidationCondition::Or(conditions) => ValidationCondition::Or(conditions.clone()),
            ValidationCondition::Not(condition) => ValidationCondition::Not(condition.clone()),
            ValidationCondition::Custom(_) => {
                // Custom conditions with closures cannot be cloned
                panic!("Cannot clone ValidationCondition::Custom variant")
            }
        }
    }
}

impl std::fmt::Debug for ValidationCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationCondition::Always => write!(f, "Always"),
            ValidationCondition::Never => write!(f, "Never"),
            ValidationCondition::IsWeekend => write!(f, "IsWeekend"),
            ValidationCondition::IsBusinessDay => write!(f, "IsBusinessDay"),
            ValidationCondition::IsDayOfWeek(day) => write!(f, "IsDayOfWeek({:?})", day),
            ValidationCondition::IsMonth(month) => write!(f, "IsMonth({:?})", month),
            ValidationCondition::IsMonthIn(months) => write!(f, "IsMonthIn({:?})", months),
            ValidationCondition::IsHourInRange(min, max) => write!(f, "IsHourInRange({}, {})", min, max),
            ValidationCondition::IsYearInRange(min, max) => write!(f, "IsYearInRange({}, {})", min, max),
            ValidationCondition::IsDayInRange(min, max) => write!(f, "IsDayInRange({}, {})", min, max),
            ValidationCondition::IsLeapYear => write!(f, "IsLeapYear"),
            ValidationCondition::IsAfterDate(y, m, d) => write!(f, "IsAfterDate({}, {}, {})", y, m, d),
            ValidationCondition::IsBeforeDate(y, m, d) => write!(f, "IsBeforeDate({}, {}, {})", y, m, d),
            ValidationCondition::HasFractionalSeconds => write!(f, "HasFractionalSeconds"),
            ValidationCondition::And(conditions) => write!(f, "And({:?})", conditions),
            ValidationCondition::Or(conditions) => write!(f, "Or({:?})", conditions),
            ValidationCondition::Not(condition) => write!(f, "Not({:?})", condition),
            ValidationCondition::Custom(_) => write!(f, "Custom(<function>)"),
        }
    }
}

impl ValidationCondition {
    /// Evaluates this condition against a CalClock.
    pub fn evaluate_calclock(&self, calclock: &CalClock) -> bool {
        match self {
            ValidationCondition::Always => true,
            ValidationCondition::Never => false,
            ValidationCondition::IsWeekend => calclock.date().is_weekend(),
            ValidationCondition::IsBusinessDay => calclock.date().is_business_day(),
            ValidationCondition::IsDayOfWeek(day) => calclock.date().day_of_week() == *day,
            ValidationCondition::IsMonth(month) => calclock.date().month_of_year() == *month,
            ValidationCondition::IsMonthIn(months) => months.contains(&calclock.date().month_of_year()),
            ValidationCondition::IsHourInRange(min, max) => {
                let hour = calclock.time().hour().of();
                hour >= *min && hour <= *max
            }
            ValidationCondition::IsYearInRange(min, max) => {
                let year = calclock.date().year();
                year >= *min && year <= *max
            }
            ValidationCondition::IsDayInRange(min, max) => {
                let day = calclock.date().day();
                day >= *min && day <= *max
            }
            ValidationCondition::IsLeapYear => calclock.date().is_leap_year(),
            ValidationCondition::IsAfterDate(year, month, day) => {
                let date = calclock.date();
                date.year() > *year ||
                (date.year() == *year && date.month() > *month) ||
                (date.year() == *year && date.month() == *month && date.day() > *day)
            }
            ValidationCondition::IsBeforeDate(year, month, day) => {
                let date = calclock.date();
                date.year() < *year ||
                (date.year() == *year && date.month() < *month) ||
                (date.year() == *year && date.month() == *month && date.day() < *day)
            }
            ValidationCondition::HasFractionalSeconds => {
                calclock.time().nanosecond().of() > 0
            }
            ValidationCondition::And(conditions) => {
                conditions.iter().all(|cond| cond.evaluate_calclock(calclock))
            }
            ValidationCondition::Or(conditions) => {
                conditions.iter().any(|cond| cond.evaluate_calclock(calclock))
            }
            ValidationCondition::Not(condition) => {
                !condition.evaluate_calclock(calclock)
            }
            ValidationCondition::Custom(func) => func(calclock),
        }
    }

    /// Evaluates this condition against a CalendarDate.
    pub fn evaluate_date(&self, date: &CalendarDate) -> bool {
        // For date-only evaluation, we create a minimal CalClock
        // In a real implementation, you might want date-specific conditions
        let zone = date.zone().clone();
        if let Ok(time) = crate::clock::ClockTime::new(0, 0, 0, 0, zone.clone()) {
            if let Ok(calclock) = crate::time::CalClock::from_date_time(date.clone(), time) {
                return self.evaluate_calclock(&calclock);
            }
        }
        false
    }

    /// Evaluates this condition against a ClockTime.
    pub fn evaluate_time(&self, time: &ClockTime) -> bool {
        // For time-only evaluation, we create a minimal CalClock with today's date
        let zone = time.zone().clone();
        if let Ok(date) = crate::calendar::CalendarDate::new(2024, 1, 1, zone.clone()) {
            if let Ok(calclock) = crate::time::CalClock::from_date_time(date, time.clone()) {
                return self.evaluate_calclock(&calclock);
            }
        }
        false
    }
}

/// Builder for complex conditional validation rules.
pub struct ConditionalRuleBuilder {
    rules: Vec<ConditionalRule>,
}

impl ConditionalRuleBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
        }
    }

    /// Adds a conditional rule.
    pub fn rule(mut self, rule: ConditionalRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Builds a combined validation rule from all conditional rules.
    pub fn build(self, name: &str) -> ValidationRule {
        // Convert all conditional rules to validation rules first
        let rules: Vec<ValidationRule> = self.rules
            .into_iter()
            .map(|conditional_rule| conditional_rule.into_rule())
            .collect();
        
        ValidationRule::new(name)
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
}

impl Default for ConditionalRuleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for creating common conditional rules
impl ConditionalRule {
    /// Creates a rule for business hours that varies by day type.
    pub fn business_hours_by_day() -> Self {
        use crate::validation::ValidationRules;
        
        ConditionalRule::new("business_hours_by_day")
            .condition(ValidationCondition::IsBusinessDay)
            .if_true(ValidationRules::hour_range(9, 17))  // 9 AM - 5 PM on weekdays
            .if_false(ValidationRules::hour_range(10, 14)) // 10 AM - 2 PM on weekends
    }

    /// Creates a rule for holiday scheduling.
    pub fn holiday_scheduling() -> Self {
        use crate::validation::ValidationRules;
        
        // December - require advance scheduling
        let december_months = {
            let mut months = HashSet::new();
            months.insert(MonthOfYear::December);
            months
        };
        
        ConditionalRule::new("holiday_scheduling")
            .condition(ValidationCondition::IsMonthIn(december_months))
            .if_true(ValidationRules::not_too_future(30)) // 30 days max in advance
            .if_false(ValidationRules::not_too_future(365)) // 1 year max normally
    }

    /// Creates a rule for seasonal time restrictions.
    pub fn seasonal_hours() -> Self {
        use crate::validation::ValidationRules;
        
        // Summer months (June, July, August) - extended hours
        let summer_months = {
            let mut months = HashSet::new();
            months.insert(MonthOfYear::June);
            months.insert(MonthOfYear::July);
            months.insert(MonthOfYear::August);
            months
        };
        
        ConditionalRule::new("seasonal_hours")
            .condition(ValidationCondition::IsMonthIn(summer_months))
            .if_true(ValidationRules::hour_range(8, 20))  // 8 AM - 8 PM in summer
            .if_false(ValidationRules::hour_range(9, 18)) // 9 AM - 6 PM otherwise
    }

    /// Creates a rule for leap year handling.
    pub fn leap_year_aware() -> Self {
        ConditionalRule::new("leap_year_aware")
            .condition(ValidationCondition::And(vec![
                ValidationCondition::IsMonth(MonthOfYear::February),
                ValidationCondition::IsDayInRange(29, 29),
            ]))
            .if_true(ValidationRule::new("require_leap_year").with_date_validator(|date| {
                if date.is_leap_year() {
                    Ok(())
                } else {
                    Err(vec![ValidationError::new(
                        "leap_year_required",
                        "February 29 is only valid in leap years"
                    )])
                }
            }))
    }

    /// Creates a rule for maintenance windows.
    pub fn maintenance_window() -> Self {
        // Sunday 2 AM - 4 AM maintenance window
        ConditionalRule::new("maintenance_window")
            .condition(ValidationCondition::And(vec![
                ValidationCondition::IsDayOfWeek(DayOfWeek::Sunday),
                ValidationCondition::IsHourInRange(2, 4),
            ]))
            .if_true(ValidationRule::new("maintenance_blocked").with_calclock_validator(|_| {
                Err(vec![ValidationError::new(
                    "maintenance_window",
                    "System maintenance window - operations not allowed"
                )])
            }))
    }
}