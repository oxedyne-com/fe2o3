use crate::{
    calendar::DayIncrementor,
    core::IntervalList,
    constant::MonthOfYear,
    time::{
        CalClockInterval,
        CalClockZone,
    },
};

use oxedize_fe2o3_core::prelude::*;

/// Represents a specific month period.
#[derive(Clone, Debug)]
pub struct MonthPeriod {
    year:	i32,
    month:	MonthOfYear,
    zone:	CalClockZone,
}

impl MonthPeriod {
    pub fn new(year: i32, month: u8, zone: CalClockZone) -> Outcome<Self> {
        let month = res!(MonthOfYear::from_number(month));
        Ok(Self { year, month, zone })
    }
    
    pub fn from_month(year: i32, month: MonthOfYear, zone: CalClockZone) -> Self {
        Self { year, month, zone }
    }
    
    pub fn year(&self) -> i32 {
        self.year
    }
    
    pub fn month(&self) -> MonthOfYear {
        self.month
    }
    
    pub fn get_month_of_year(&self) -> MonthOfYear {
        self.month
    }
    
    pub fn inc(&self, months: i32) -> Self {
        let total_months = self.month.of() as i32 + months;
        let new_month = ((total_months - 1) % 12 + 12) % 12 + 1;
        let year_adjust = (total_months - new_month) / 12;
        
        Self {
            year: self.year + year_adjust,
            month: MonthOfYear::from_number(new_month as u8).unwrap(),
            zone: self.zone.clone(),
        }
    }
    
    /// Find a specific day in this month period (stub implementation).
    pub fn find(
        &self, 
        _day_incrementor: &DayIncrementor, 
        _holidays: Option<&IntervalList<CalClockInterval>>
    ) -> Outcome<crate::calendar::CalendarDate> {
        // Stub implementation - just return the first day of the month
        crate::calendar::CalendarDate::from_ymd(
            self.year, 
            self.month, 
            1, 
            self.zone.clone()
        )
    }
}

/// Represents a specific year period.
#[derive(Clone, Debug)]
pub struct YearPeriod {
    year: i32,
    zone: CalClockZone,
}

impl YearPeriod {
    pub fn new(year: i32, zone: CalClockZone) -> Self {
        Self { year, zone }
    }
    
    pub fn year(&self) -> i32 {
        self.year
    }
}