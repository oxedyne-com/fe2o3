//! Validation that branches on a condition.
//!
//! A conditional rule holds a condition and a rule for each way it can go,
//! which is how "extended hours at the weekend" or "holiday rules in
//! December" are expressed.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClock,
    validation::{ValidationError, ValidationRule},
    constant::{DayOfWeek, MonthOfYear},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;

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
    name:           String,
    condition:      ValidationCondition,
    true_rule:      Option<ValidationRule>,
    false_rule:     Option<ValidationRule>,
    always_rules:   Vec<ValidationRule>,    // run whichever way it goes
}

impl ConditionalRule {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            condition: ValidationCondition::Always,
            true_rule: None,
            false_rule: None,
            always_rules: Vec::new(),
        }
    }

    pub fn condition(mut self, condition: ValidationCondition) -> Self {
        self.condition = condition;
        self
    }

    pub fn if_true(mut self, rule: ValidationRule) -> Self {
        self.true_rule = Some(rule);
        self
    }

    pub fn if_false(mut self, rule: ValidationRule) -> Self {
        self.false_rule = Some(rule);
        self
    }

    pub fn always(mut self, rule: ValidationRule) -> Self {
        self.always_rules.push(rule);
        self
    }

    /// The always rules run first, then whichever branch the condition
    /// selects, and the errors of both are returned together.
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

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub enum ValidationCondition {
    Always,
    Never,
    IsWeekend,
    IsBusinessDay,
    IsDayOfWeek(DayOfWeek),
    IsMonth(MonthOfYear),
    IsMonthIn(HashSet<MonthOfYear>),
    // Ranges are inclusive at both ends.
    IsHourInRange(u8, u8),
    IsYearInRange(i32, i32),
    IsDayInRange(u8, u8),
    IsLeapYear,
    IsAfterDate(i32, u8, u8), // year, month, day
    IsBeforeDate(i32, u8, u8), // year, month, day
    HasFractionalSeconds,
    And(Vec<ValidationCondition>),
    Or(Vec<ValidationCondition>),
    Not(Box<ValidationCondition>),
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

    /// The date is given midnight so that time conditions can be evaluated;
    /// a condition on the clock will read as if it were midnight.
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

    /// The time is given 2024-01-01 for the same reason, so a condition on
    /// the date says nothing useful here.
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

pub struct ConditionalRuleBuilder {
    rules: Vec<ConditionalRule>,
}

impl ConditionalRuleBuilder {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
        }
    }

    pub fn rule(mut self, rule: ConditionalRule) -> Self {
        self.rules.push(rule);
        self
    }

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
    pub fn business_hours_by_day() -> Self {
        use crate::validation::ValidationRules;
        
        ConditionalRule::new("business_hours_by_day")
            .condition(ValidationCondition::IsBusinessDay)
            .if_true(ValidationRules::hour_range(9, 17))  // 9 AM - 5 PM on weekdays
            .if_false(ValidationRules::hour_range(10, 14)) // 10 AM - 2 PM on weekends
    }

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