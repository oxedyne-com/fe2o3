//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

pub struct TimeValidation;

impl TimeValidation {
    pub fn is_valid_year(year: i32) -> bool {
        year >= -9999 && year <= 9999
    }
    
    pub fn is_valid_month(month: u8) -> bool {
        month >= 1 && month <= 12
    }
    
    pub fn is_valid_day(year: i32, month: u8, day: u8) -> Outcome<bool> {
        use crate::constant::MonthOfYear;
        
        if day == 0 {
            return Ok(false);
        }
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        Ok(day <= days_in_month)
    }
    
    pub fn is_valid_hour(hour: u8) -> bool {
        hour <= 23
    }
    
    pub fn is_valid_minute(minute: u8) -> bool {
        minute <= 59
    }
    
    pub fn is_valid_second(second: u8) -> bool {
        second <= 59
    }
    
    pub fn is_valid_nanosecond(nano: u32) -> bool {
        nano <= 999_999_999
    }
}