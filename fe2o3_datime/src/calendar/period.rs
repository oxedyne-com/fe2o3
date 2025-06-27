use crate::{
    calendar::DayIncrementor,
    core::IntervalList,
    constant::MonthOfYear,
    time::{
        CalClockInterval,
        CalClockZone,
    },
};

use oxedyne_fe2o3_core::prelude::*;

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
    
    /// Find a specific day in this month period using the day incrementor.
    pub fn find(
        &self, 
        day_incrementor: &DayIncrementor, 
        holidays: Option<&IntervalList<CalClockInterval>>
    ) -> Outcome<crate::calendar::CalendarDate> {
        use crate::calendar::incrementor::DayType;
        
        // Start with the first day of the month
        let mut candidate_date = res!(crate::calendar::CalendarDate::from_ymd(
            self.year, 
            self.month, 
            1, 
            self.zone.clone()
        ));
        
        // Apply the day incrementor logic
        match day_incrementor.day_type() {
            Some(DayType::DayOfMonthFromStart) => {
                // Find the Nth day from the start of the month
                let day_number = day_incrementor.value();
                if day_number > 0 {
                    let max_days = candidate_date.days_in_month().unwrap_or(31);
                    let target_day = (day_number as u8).min(max_days);
                    candidate_date = res!(crate::calendar::CalendarDate::from_ymd(
                        self.year, 
                        self.month, 
                        target_day, 
                        self.zone.clone()
                    ));
                }
            },
            Some(DayType::DayOfMonthFromEnd) => {
                // Find the Nth day from the end of the month
                let day_number = day_incrementor.value();
                if day_number > 0 {
                    let max_days = res!(candidate_date.days_in_month());
                    let target_day = (max_days as i32 - day_number + 1).max(1) as u8;
                    candidate_date = res!(crate::calendar::CalendarDate::from_ymd(
                        self.year, 
                        self.month, 
                        target_day, 
                        self.zone.clone()
                    ));
                }
            },
            Some(DayType::DayOfWeek) => {
                // Find a specific day of the week in the month
                if let Some(target_dow) = day_incrementor.day_of_week() {
                    let mut search_date = candidate_date.clone();
                    let max_days = res!(candidate_date.days_in_month());
                    
                    // Find the first occurrence of the target day of week
                    for day in 1..=max_days {
                        search_date = res!(crate::calendar::CalendarDate::from_ymd(
                            self.year, 
                            self.month, 
                            day, 
                            self.zone.clone()
                        ));
                        
                        if search_date.day_of_week() == target_dow {
                            candidate_date = search_date;
                            break;
                        }
                    }
                    
                    // If we need the Nth occurrence (not the first)
                    let occurrence = day_incrementor.value();
                    if occurrence > 1 {
                        for _ in 1..occurrence {
                            // Move to next week
                            candidate_date = res!(candidate_date.add_days(7));
                            
                            // Check if we're still in the same month
                            if candidate_date.month_of_year() != self.month {
                                // Went past the end of the month, return the last valid occurrence
                                candidate_date = res!(candidate_date.add_days(-7));
                                break;
                            }
                        }
                    }
                }
            },
            Some(DayType::Workday) => {
                // Find business days (excluding weekends and optionally holidays)
                let day_number = day_incrementor.value();
                if day_number > 0 {
                    let mut business_day_count = 0;
                    let max_days = res!(candidate_date.days_in_month());
                    
                    for day in 1..=max_days {
                        let test_date = res!(crate::calendar::CalendarDate::from_ymd(
                            self.year, 
                            self.month, 
                            day, 
                            self.zone.clone()
                        ));
                        
                        // Check if it's a business day (includes holiday checking)
                        if day_incrementor.is_business_day(&test_date) {
                            // The business day engine already handles holidays,
                            // so we don't need additional holiday checking here
                            business_day_count += 1;
                            if business_day_count == day_number {
                                candidate_date = test_date;
                                break;
                            }
                        }
                    }
                }
            },
            _ => {
                // Default: return the first day of the month
                // This handles cases where day_type is None or other variants
            }
        }
        
        Ok(candidate_date)
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