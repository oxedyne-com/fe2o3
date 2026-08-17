//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::core::Duration;

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

/// Calendar units, whose length in days depends on where they are applied.
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
    
    pub fn years(&self) -> i32 {
        self.years
    }
    
    pub fn months(&self) -> i32 {
        self.months
    }
    
    pub fn days(&self) -> i32 {
        self.days
    }
    
    pub fn in_days(&self) -> i32 {
        // Approximate: 365.25 days per year, 30.44 days per month
        (self.years as f64 * 365.25 + self.months as f64 * 30.44 + self.days as f64) as i32
    }
    
    pub fn is_negative(&self) -> bool {
        self.years < 0 || self.months < 0 || self.days < 0
    }
    
    pub fn negate(&self) -> Self {
        Self {
            years: -self.years,
            months: -self.months,
            days: -self.days,
        }
    }
    
    pub fn plus(&self, other: &Self) -> Self {
        Self {
            years: self.years + other.years,
            months: self.months + other.months,
            days: self.days + other.days,
        }
    }
    
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
            ok!(write!(f, "{}Y", self.years));
        }
        if self.months != 0 {
            if self.years != 0 {
                ok!(write!(f, " "));
            }
            ok!(write!(f, "{}M", self.months));
        }
        if self.days != 0 || (self.years == 0 && self.months == 0) {
            if self.years != 0 || self.months != 0 {
                ok!(write!(f, " "));
            }
            ok!(write!(f, "{}D", self.days));
        }
        Ok(())
    }
}