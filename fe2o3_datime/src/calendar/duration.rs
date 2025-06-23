use crate::core::Duration;

use oxedize_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

/// A duration measured in calendar units (years, months, days).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarDuration {
    years:	i32,
    months:	i32,
    days:	i32,
}

impl CalendarDuration {
    pub fn new(years: i32, months: i32, days: i32) -> Self {
        Self {
            years,
            months,
            days,
        }
    }
    
    pub fn from_days(days: i32) -> Self {
        Self {
            years: 0,
            months: 0,
            days,
        }
    }
    
    pub fn from_months(months: i32) -> Self {
        Self {
            years: 0,
            months,
            days: 0,
        }
    }
    
    pub fn from_years(years: i32) -> Self {
        Self {
            years,
            months: 0,
            days: 0,
        }
    }
    
    /// Get the number of years.
    pub fn years(&self) -> i32 {
        self.years
    }
    
    /// Get the number of months.
    pub fn months(&self) -> i32 {
        self.months
    }
    
    /// Get the number of days.
    pub fn days(&self) -> i32 {
        self.days
    }
    
    /// Get total days (approximate, as months have variable lengths).
    pub fn in_days(&self) -> i32 {
        // Approximate: 365.25 days per year, 30.44 days per month
        (self.years as f64 * 365.25 + self.months as f64 * 30.44 + self.days as f64) as i32
    }
    
    /// Check if the duration is negative.
    pub fn is_negative(&self) -> bool {
        self.years < 0 || self.months < 0 || self.days < 0
    }
    
    /// Negate the duration.
    pub fn negate(&self) -> Self {
        Self {
            years: -self.years,
            months: -self.months,
            days: -self.days,
        }
    }
    
    /// Add another duration.
    pub fn plus(&self, other: &Self) -> Self {
        Self {
            years: self.years + other.years,
            months: self.months + other.months,
            days: self.days + other.days,
        }
    }
    
    /// Subtract another duration.
    pub fn minus(&self, other: &Self) -> Self {
        Self {
            years: self.years - other.years,
            months: self.months - other.months,
            days: self.days - other.days,
        }
    }
}

impl Duration for CalendarDuration {
    fn to_nanos(&self) -> Outcome<i64> {
        // Approximate conversion
        let total_days = self.in_days() as i64;
        Ok(total_days * 24 * 60 * 60 * 1_000_000_000)
    }
    
    fn to_seconds(&self) -> Outcome<i64> {
        // Approximate conversion
        let total_days = self.in_days() as i64;
        Ok(total_days * 24 * 60 * 60)
    }
    
    fn to_days(&self) -> Outcome<i32> {
        Ok(self.in_days())
    }
    
    fn is_negative(&self) -> bool {
        self.is_negative()
    }
}

impl Display for CalendarDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.years != 0 {
            write!(f, "{}Y", self.years)?;
        }
        if self.months != 0 {
            if self.years != 0 {
                write!(f, " ")?;
            }
            write!(f, "{}M", self.months)?;
        }
        if self.days != 0 || (self.years == 0 && self.months == 0) {
            if self.years != 0 || self.months != 0 {
                write!(f, " ")?;
            }
            write!(f, "{}D", self.days)?;
        }
        Ok(())
    }
}