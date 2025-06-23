use oxedize_fe2o3_core::prelude::*;

/// Time validation utilities.
pub struct TimeValidation;

impl TimeValidation {
    /// Check if a year is valid (within reasonable bounds).
    pub fn is_valid_year(year: i32) -> bool {
        year >= -9999 && year <= 9999
    }
    
    /// Check if a month is valid (1-12).
    pub fn is_valid_month(month: u8) -> bool {
        month >= 1 && month <= 12
    }
    
    /// Check if a day is valid for the given month and year.
    pub fn is_valid_day(year: i32, month: u8, day: u8) -> Outcome<bool> {
        use crate::constant::MonthOfYear;
        
        if day == 0 {
            return Ok(false);
        }
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        Ok(day <= days_in_month)
    }
    
    /// Check if an hour is valid (0-23).
    pub fn is_valid_hour(hour: u8) -> bool {
        hour <= 23
    }
    
    /// Check if a minute is valid (0-59).
    pub fn is_valid_minute(minute: u8) -> bool {
        minute <= 59
    }
    
    /// Check if a second is valid (0-59, allowing for leap seconds).
    pub fn is_valid_second(second: u8) -> bool {
        second <= 59
    }
    
    /// Check if nanoseconds are valid (0-999,999,999).
    pub fn is_valid_nanosecond(nano: u32) -> bool {
        nano <= 999_999_999
    }
}