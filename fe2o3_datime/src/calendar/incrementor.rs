use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, OrdinalEnglish},
    time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

/// Represents an abstract incremental day descriptor.
///
/// For example:
/// - "second business day"
/// - "3rd Monday"
/// - "4th weekday"
/// 
/// This is a port of the Java DayIncrementor system to support
/// complex relative date parsing like "second business day after the 1st Monday".
#[derive(Clone, Debug, PartialEq)]
pub struct DayIncrementor {
    value: i32,
    sign: i32, // Relative to child (next) incrementor: -1 = before, +1 = after
    day_of_week: Option<DayOfWeek>,
    day_type: Option<DayType>,
    next_inc: Option<Box<DayIncrementor>>,
}

/// Types of days that can be referenced in relative date expressions.
#[derive(Clone, Debug, PartialEq)]
pub enum DayType {
    /// Any weekday (Monday-Friday)
    Weekday,
    /// Work/business day (Monday-Friday, excluding holidays)
    Workday,
    /// Specific day of the week (Monday, Tuesday, etc.)
    DayOfWeek,
    /// Any day
    OrdinaryDay,
    /// Day of month counting from start (1st, 2nd, etc.)
    DayOfMonthFromStart,
    /// Day of month counting from end (last day, 2nd last, etc.)
    DayOfMonthFromEnd,
}

/// Special token for end-of-month references
pub const END_OF_MONTH_TOKEN: &str = "END_OF_MONTH_TOKEN";

impl DayIncrementor {
    /// Create a blank DayIncrementor.
    pub fn new() -> Self {
        Self {
            value: 0,
            sign: 0,
            day_of_week: None,
            day_type: None,
            next_inc: None,
        }
    }

    /// Create a DayIncrementor with a next incrementor.
    pub fn with_next(next_inc: DayIncrementor) -> Self {
        Self {
            value: 0,
            sign: 0,
            day_of_week: None,
            day_type: None,
            next_inc: Some(Box::new(next_inc)),
        }
    }

    /// Day of week constructor - sets the incrementor to the nth occurrence of the given day.
    pub fn with_day_of_week(value: i32, day_of_week: DayOfWeek) -> Self {
        let mut inc = Self::new();
        inc.value = value.abs();
        inc.set_day_of_week(day_of_week);
        inc.next_inc = Some(Box::new(DayIncrementor::new()));
        inc
    }

    /// Day of month constructor - sets the incrementor to a day of the month.
    pub fn with_day_of_month(value: i32) -> Self {
        let mut inc = Self::new();
        inc.value = value.abs();
        inc.day_type = Some(DayType::DayOfMonthFromStart);
        inc.next_inc = Some(Box::new(DayIncrementor::new()));
        inc
    }

    /// Parse a DayIncrementor from a string input.
    /// 
    /// Examples:
    /// - "3rd Monday"
    /// - "second business day"
    /// - "last Sunday"
    /// - "2nd weekday before the 25th"
    /// - "final day of the month"
    pub fn from_string(input: &str) -> Outcome<Self> {
        let mut input = input.trim().to_lowercase();
        
        // Remove leading "the"
        if input.starts_with("the ") {
            input = input[4..].to_string();
        }

        // Convert last/final colloquialisms into our formalism
        // "last Sunday" -> "1st Sunday before the end of the month"
        let word_count = input.split_whitespace().count();
        if word_count == 2 {
            if input.starts_with("last ") || input.starts_with("final ") {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.len() == 2 {
                    input = format!("1st {} before the end of the month", parts[1]);
                }
            }
        }

        // Replace multiple word expressions with single tokens
        input = input
            .replace("end of month", END_OF_MONTH_TOKEN)
            .replace("end of the month", END_OF_MONTH_TOKEN)
            .replace("last day of month", END_OF_MONTH_TOKEN)
            .replace("last day of the month", END_OF_MONTH_TOKEN)
            .replace("final day of month", END_OF_MONTH_TOKEN)
            .replace("final day of the month", END_OF_MONTH_TOKEN);

        let words: Vec<&str> = input.split_whitespace().collect();
        
        // Create two-step incrementor structure
        let mut main_inc = Self::new();
        main_inc.next_inc = Some(Box::new(DayIncrementor::new()));

        // Parse each word
        let current_inc = &mut main_inc;
        let mut i = 0;
        
        while i < words.len() {
            let word = words[i];
            
            if current_inc.is_complete() {
                if current_inc.is_sign_defined() {
                    // Move to next incrementor
                    if let Some(ref mut next) = current_inc.next_inc {
                        res!(next.lex_word(word));
                    }
                } else {
                    // Parse qualifier (before/after)
                    res!(current_inc.lex_qualifier(word));
                }
            } else {
                res!(current_inc.lex_word(word));
            }
            
            i += 1;
        }

        // Post-processing: handle incomplete incrementors
        if !main_inc.is_complete() && !main_inc.next_inc.as_ref().unwrap().is_complete() 
           && !main_inc.next_inc.as_ref().unwrap().is_value_defined() {
            main_inc.day_type = Some(DayType::DayOfMonthFromStart);
        }

        if main_inc.is_complete() && !main_inc.next_inc.as_ref().unwrap().is_complete()
           && main_inc.next_inc.as_ref().unwrap().is_value_defined() {
            main_inc.next_inc.as_mut().unwrap().day_type = Some(DayType::DayOfMonthFromStart);
        }

        if !main_inc.is_complete() {
            return Err(err!("Incomplete DayIncrementor from input: {}", input; Invalid, Input));
        }

        Ok(main_inc)
    }

    /// Lexically analyze a single word and update the incrementor state.
    fn lex_word(&mut self, word: &str) -> Outcome<()> {
        // Try to parse as ordinal
        if let Some(ordinal) = OrdinalEnglish::from_name(word) {
            self.value = ordinal.value() as i32;
            return Ok(());
        }

        // Try to parse as day of week
        if let Some(dow) = DayOfWeek::from_name(word) {
            self.set_day_of_week(dow);
            return Ok(());
        }

        // Parse day type keywords
        match word {
            "business" | "working" | "work" => {
                self.day_type = Some(DayType::Workday);
            },
            "weekday" => {
                self.day_type = Some(DayType::Weekday);
            },
            "day" => {
                if self.day_type.is_none() {
                    self.day_type = Some(DayType::OrdinaryDay);
                }
            },
            END_OF_MONTH_TOKEN => {
                self.day_type = Some(DayType::DayOfMonthFromEnd);
            },
            _ => {
                // Unknown word - ignore for now
            }
        }

        Ok(())
    }

    /// Parse qualifier words (before, after, prior, following).
    fn lex_qualifier(&mut self, word: &str) -> Outcome<()> {
        match word {
            "before" | "prior" => {
                self.sign = -1;
            },
            "after" | "following" => {
                self.sign = 1;
            },
            _ => {
                // Unknown qualifier - ignore
            }
        }
        Ok(())
    }

    /// Set the day of week and automatically set day type.
    pub fn set_day_of_week(&mut self, day_of_week: DayOfWeek) {
        self.day_of_week = Some(day_of_week);
        self.day_type = Some(DayType::DayOfWeek);
    }

    /// Check if this incrementor is complete (has all required fields).
    pub fn is_complete(&self) -> bool {
        (self.is_value_defined() && self.is_day_type_defined()) ||
        (self.day_type == Some(DayType::DayOfMonthFromEnd))
    }

    /// Get the last incrementor in the chain.
    pub fn last(&self) -> &DayIncrementor {
        let mut current = self;
        while let Some(ref next) = current.next_inc {
            current = next;
        }
        current
    }

    // Getters
    pub fn value(&self) -> i32 { self.value }
    pub fn sign(&self) -> i32 { self.sign }
    pub fn day_of_week(&self) -> Option<DayOfWeek> { self.day_of_week }
    pub fn day_type(&self) -> Option<DayType> { self.day_type.clone() }
    pub fn next_inc(&self) -> Option<&DayIncrementor> { 
        self.next_inc.as_ref().map(|b| b.as_ref()) 
    }

    // State checkers
    pub fn is_value_defined(&self) -> bool { self.value != 0 }
    pub fn is_sign_defined(&self) -> bool { self.sign != 0 }
    pub fn is_day_of_week_defined(&self) -> bool { self.day_of_week.is_some() }
    pub fn is_day_type_defined(&self) -> bool { self.day_type.is_some() }
    pub fn has_next_inc(&self) -> bool { self.next_inc.is_some() }

    // Setters
    pub fn set_value(&mut self, value: i32) {
        self.value = value.abs();
    }

    pub fn set_sign(&mut self, sign: i32) {
        self.sign = if sign < 0 { -1 } else if sign > 0 { 1 } else { 0 };
    }

    pub fn set_day_type(&mut self, day_type: DayType) {
        self.day_type = Some(day_type);
    }

    pub fn set_next_inc(&mut self, next_inc: DayIncrementor) {
        self.next_inc = Some(Box::new(next_inc));
    }

    /// Calculate the actual date from this DayIncrementor expression.
    /// 
    /// Examples:
    /// - "3rd Monday" in June 2024 -> June 17, 2024
    /// - "2nd business day after the 15th" in June 2024 -> June 18, 2024
    /// - "last Sunday" in June 2024 -> June 30, 2024
    pub fn calculate_date(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        
        // If this is a simple day-of-month reference
        if self.day_type == Some(DayType::DayOfMonthFromStart) && self.next_inc.is_none() {
            return CalendarDate::from_ymd(year, month_enum, self.value as u8, zone);
        }
        
        // If this is an end-of-month reference
        if self.day_type == Some(DayType::DayOfMonthFromEnd) {
            let days_in_month = month_enum.days_in_month(year);
            if self.value == 0 {
                // "end of month" -> last day
                return CalendarDate::from_ymd(year, month_enum, days_in_month, zone);
            } else {
                // "2nd last day" -> days_in_month - value + 1
                let day = days_in_month.saturating_sub(self.value as u8 - 1);
                return CalendarDate::from_ymd(year, month_enum, day.max(1), zone);
            }
        }
        
        // Handle simple cases without next_inc first
        if self.next_inc.is_none() || !self.next_inc.as_ref().unwrap().is_complete() {
            // Simple case: "3rd Monday", "2nd business day", etc.
            match self.day_type {
                Some(DayType::DayOfWeek) => {
                    if let Some(target_dow) = self.day_of_week {
                        return self.find_nth_day_of_week(year, month, target_dow, self.value, zone);
                    }
                },
                Some(DayType::Weekday) => {
                    return self.find_nth_weekday(year, month, self.value, zone);
                },
                Some(DayType::Workday) => {
                    return self.find_nth_business_day(year, month, self.value, zone);
                },
                Some(DayType::DayOfMonthFromStart) => {
                    return self.calculate_day_of_month(year, month, zone);
                },
                _ => {}
            }
        }
        
        // For complex expressions with next_inc, we need a base date to work from
        let base_date = if let Some(ref next_inc) = self.next_inc {
            if next_inc.is_complete() {
                res!(next_inc.calculate_date(year, month, zone.clone()))
            } else {
                // Default to first day of month for incomplete next_inc
                res!(CalendarDate::from_ymd(year, month_enum, 1, zone.clone()))
            }
        } else {
            // Default to first day of month
            res!(CalendarDate::from_ymd(year, month_enum, 1, zone.clone()))
        };
        
        // Apply this incrementor to the base date
        self.apply_incrementor_to_date(&base_date)
    }
    
    /// Apply this incrementor's logic to a base date.
    fn apply_incrementor_to_date(&self, base_date: &CalendarDate) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        
        match self.day_type {
            Some(DayType::DayOfWeek) => {
                if let Some(target_dow) = self.day_of_week {
                    if self.sign == 0 {
                        // Find nth occurrence in same month
                        self.find_nth_day_of_week(base_date.year(), base_date.month(), target_dow, self.value, base_date.zone().clone())
                    } else if self.sign > 0 {
                        // Find nth occurrence after base_date
                        let mut current = base_date.clone();
                        for _ in 0..self.value {
                            current = res!(self.find_next_day_of_week(&current, target_dow));
                        }
                        Ok(current)
                    } else {
                        // Find nth occurrence before base_date
                        // Special case: if base_date is end of month and we want "last" day of week,
                        // we want the last occurrence in the month, not before the end
                        if self.value == 1 && base_date.day() == res!(base_date.days_in_month()) {
                            // This is "last X" in month - find the last occurrence of target_dow in this month
                            self.find_last_day_of_week_in_month(base_date.year(), base_date.month(), target_dow, base_date.zone().clone())
                        } else {
                            let mut current = base_date.clone();
                            for _ in 0..self.value {
                                current = res!(self.find_previous_day_of_week(&current, target_dow));
                            }
                            Ok(current)
                        }
                    }
                } else {
                    Err(err!("DayOfWeek type requires day_of_week to be set"; Invalid, Input))
                }
            },
            Some(DayType::Weekday) => {
                if self.sign == 0 {
                    self.find_nth_weekday(base_date.year(), base_date.month(), self.value, base_date.zone().clone())
                } else if self.sign > 0 {
                    let mut current = base_date.clone();
                    for _ in 0..self.value {
                        current = res!(self.find_next_weekday(&current));
                    }
                    Ok(current)
                } else {
                    let mut current = base_date.clone();
                    for _ in 0..self.value {
                        current = res!(self.find_previous_weekday(&current));
                    }
                    Ok(current)
                }
            },
            Some(DayType::Workday) => {
                if self.sign == 0 {
                    self.find_nth_business_day(base_date.year(), base_date.month(), self.value, base_date.zone().clone())
                } else if self.sign > 0 {
                    let mut current = base_date.clone();
                    for _ in 0..self.value {
                        current = res!(self.find_next_business_day(&current));
                    }
                    Ok(current)
                } else {
                    let mut current = base_date.clone();
                    for _ in 0..self.value {
                        current = res!(self.find_previous_business_day(&current));
                    }
                    Ok(current)
                }
            },
            Some(DayType::DayOfMonthFromStart) => {
                self.calculate_day_of_month(base_date.year(), base_date.month(), base_date.zone().clone())
            },
            Some(DayType::DayOfMonthFromEnd) => {
                let month_enum = res!(crate::constant::MonthOfYear::from_number(base_date.month()));
                let days_in_month = month_enum.days_in_month(base_date.year());
                let day = days_in_month.saturating_sub(self.value as u8 - 1);
                CalendarDate::from_ymd(base_date.year(), month_enum, day.max(1), base_date.zone().clone())
            },
            _ => {
                Err(err!("Unsupported DayType for date calculation: {:?}", self.day_type; Invalid, Input))
            }
        }
    }
    
    /// Find the last occurrence of a specific day of the week in a given month.
    fn find_last_day_of_week_in_month(&self, year: i32, month: u8, target_dow: DayOfWeek, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        // Start from the last day and work backwards
        for day in (1..=days_in_month).rev() {
            let current = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if current.day_of_week() == target_dow {
                return Ok(current);
            }
        }
        
        Err(err!("No {} found in {}/{}", target_dow, month, year; Invalid, Input, Range))
    }

    /// Find the nth occurrence of a specific day of the week in a given month.
    fn find_nth_day_of_week(&self, year: i32, month: u8, target_dow: DayOfWeek, n: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let first_day = res!(CalendarDate::from_ymd(year, month_enum, 1, zone.clone()));
        let first_dow = first_day.day_of_week();
        
        // Calculate offset to first occurrence of target day
        let target_dow_num = target_dow.of() as i32;
        let first_dow_num = first_dow.of() as i32;
        let mut offset = (target_dow_num - first_dow_num + 7) % 7;
        
        // Calculate the nth occurrence
        offset += (n - 1) * 7;
        let target_day = 1 + offset;
        
        // Validate the day is within the month
        let days_in_month = month_enum.days_in_month(year);
        if target_day < 1 || target_day > days_in_month as i32 {
            return Err(err!("Day {} of {} {} does not exist in month", n, target_dow, month; Invalid, Input, Range));
        }
        
        CalendarDate::from_ymd(year, month_enum, target_day as u8, zone)
    }
    
    /// Find the nth weekday (Monday-Friday) in a given month.
    fn find_nth_weekday(&self, year: i32, month: u8, n: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let mut current: CalendarDate;
        let days_in_month = month_enum.days_in_month(year);
        let mut weekday_count = 0;
        
        for day in 1..=days_in_month {
            current = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if self.is_weekday(&current) {
                weekday_count += 1;
                if weekday_count == n {
                    return Ok(current);
                }
            }
        }
        
        Err(err!("Only {} weekdays in {}/{}, cannot find {}", weekday_count, month, year, n; Invalid, Input, Range))
    }
    
    /// Find the nth business day in a given month.
    fn find_nth_business_day(&self, year: i32, month: u8, n: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let mut current: CalendarDate;
        let days_in_month = month_enum.days_in_month(year);
        let mut business_day_count = 0;
        
        for day in 1..=days_in_month {
            current = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if self.is_business_day(&current) {
                business_day_count += 1;
                if business_day_count == n {
                    return Ok(current);
                }
            }
        }
        
        Err(err!("Only {} business days in {}/{}, cannot find {}", business_day_count, month, year, n; Invalid, Input, Range))
    }
    
    /// Calculate a specific day of the month.
    fn calculate_day_of_month(&self, year: i32, month: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        use crate::calendar::CalendarDate;
        use crate::constant::MonthOfYear;
        
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        let day = if self.value <= days_in_month as i32 {
            self.value as u8
        } else {
            return Err(err!("Day {} does not exist in month {} of year {}", self.value, month, year; Invalid, Input, Range));
        };
        
        CalendarDate::from_ymd(year, month_enum, day, zone)
    }
    
    /// Find the next occurrence of a specific day of the week after the given date.
    fn find_next_day_of_week(&self, date: &CalendarDate, target_dow: DayOfWeek) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        let target_dow_num = target_dow.of();
        
        loop {
            current = res!(current.add_days(1));
            if current.day_of_week().of() == target_dow_num {
                return Ok(current);
            }
        }
    }
    
    /// Find the previous occurrence of a specific day of the week before the given date.
    fn find_previous_day_of_week(&self, date: &CalendarDate, target_dow: DayOfWeek) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        let target_dow_num = target_dow.of();
        
        loop {
            current = res!(current.add_days(-1));
            if current.day_of_week().of() == target_dow_num {
                return Ok(current);
            }
        }
    }
    
    /// Find the next weekday (Monday-Friday) after the given date.
    fn find_next_weekday(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        loop {
            current = res!(current.add_days(1));
            if self.is_weekday(&current) {
                return Ok(current);
            }
        }
    }
    
    /// Find the previous weekday (Monday-Friday) before the given date.
    fn find_previous_weekday(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        loop {
            current = res!(current.add_days(-1));
            if self.is_weekday(&current) {
                return Ok(current);
            }
        }
    }
    
    /// Find the next business day after the given date.
    fn find_next_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        loop {
            current = res!(current.add_days(1));
            if self.is_business_day(&current) {
                return Ok(current);
            }
        }
    }
    
    /// Find the previous business day before the given date.
    fn find_previous_business_day(&self, date: &CalendarDate) -> Outcome<CalendarDate> {
        let mut current = date.clone();
        
        loop {
            current = res!(current.add_days(-1));
            if self.is_business_day(&current) {
                return Ok(current);
            }
        }
    }
    
    /// Check if a date is a weekday (Monday-Friday).
    fn is_weekday(&self, date: &CalendarDate) -> bool {
        let dow_num = date.day_of_week().of();
        dow_num >= 1 && dow_num <= 5 // Monday=1 to Friday=5
    }
    
    /// Check if a date is a business day.
    /// Uses the BusinessDayEngine for proper holiday integration.
    pub fn is_business_day(&self, date: &CalendarDate) -> bool {
        use crate::calendar::business_day_engine::BusinessDayEngine;
        
        // Create a standard business day engine
        let engine = BusinessDayEngine::new();
        
        // Use the engine's comprehensive business day logic
        match engine.is_business_day(date) {
            Ok(is_business) => is_business,
            Err(_) => {
                // Fallback to weekday check if engine fails
                self.is_weekday(date)
            }
        }
    }
}

impl Default for DayIncrementor {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DayIncrementor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sign: {} Value: {} DayType: {:?} DayOfWeek: {:?} NextInc: {:?}",
               self.sign, self.value, self.day_type, self.day_of_week, 
               self.next_inc.as_ref().map(|n| n.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_day_of_week() {
        let inc = DayIncrementor::from_string("3rd Monday").unwrap();
        assert_eq!(inc.value(), 3);
        assert_eq!(inc.day_of_week(), Some(DayOfWeek::Monday));
        assert_eq!(inc.day_type(), Some(DayType::DayOfWeek));
    }

    #[test]
    fn test_business_day() {
        let inc = DayIncrementor::from_string("second business day").unwrap();
        assert_eq!(inc.value(), 2);
        assert_eq!(inc.day_type(), Some(DayType::Workday));
    }

    #[test]
    fn test_last_sunday() {
        let inc = DayIncrementor::from_string("last Sunday").unwrap();
        assert_eq!(inc.value(), 1);
        assert_eq!(inc.day_of_week(), Some(DayOfWeek::Sunday));
        assert_eq!(inc.sign(), -1);
        
        // Should have next incrementor for "end of month"
        let next = inc.next_inc().unwrap();
        assert_eq!(next.day_type(), Some(DayType::DayOfMonthFromEnd));
    }

    #[test]
    fn test_complex_expression() {
        let inc = DayIncrementor::from_string("2nd weekday before the 25th").unwrap();
        assert_eq!(inc.value(), 2);
        assert_eq!(inc.day_type(), Some(DayType::Weekday));
        assert_eq!(inc.sign(), -1);
        
        let next = inc.next_inc().unwrap();
        assert_eq!(next.value(), 25);
        assert_eq!(next.day_type(), Some(DayType::DayOfMonthFromStart));
    }

    #[test]
    fn test_end_of_month() {
        let inc = DayIncrementor::from_string("end of the month").unwrap();
        assert_eq!(inc.day_type(), Some(DayType::DayOfMonthFromEnd));
    }

    #[test]
    fn test_calculate_date_simple_day_of_week() {
        // Test "3rd Monday" in June 2024
        // June 2024: starts on Saturday, so Mondays are 3rd, 10th, 17th, 24th
        let inc = DayIncrementor::from_string("3rd Monday").unwrap();
        let zone = crate::time::CalClockZone::utc();
        let result = inc.calculate_date(2024, 6, zone).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 6);
        assert_eq!(result.day(), 17); // 3rd Monday
        assert_eq!(result.day_of_week(), crate::constant::DayOfWeek::Monday);
    }

    #[test]
    fn test_calculate_date_business_day() {
        // Test "2nd business day" in June 2024
        // June 2024: starts on Saturday, so business days start Monday 3rd
        let inc = DayIncrementor::from_string("2nd business day").unwrap();
        let zone = crate::time::CalClockZone::utc();
        let result = inc.calculate_date(2024, 6, zone).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 6);
        assert_eq!(result.day(), 4); // 2nd business day (Monday 3rd, Tuesday 4th)
    }

    #[test]
    fn test_calculate_date_end_of_month() {
        // Test "end of the month" in February 2024 (leap year)
        let inc = DayIncrementor::from_string("end of the month").unwrap();
        let zone = crate::time::CalClockZone::utc();
        let result = inc.calculate_date(2024, 2, zone.clone()).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 29); // February in leap year

        // Test in non-leap year
        let result = inc.calculate_date(2023, 2, zone).unwrap();
        assert_eq!(result.day(), 28); // February in non-leap year
    }

    #[test]
    fn test_calculate_last_sunday() {
        // Test "last Sunday" -> "1st Sunday before the end of the month"
        let inc = DayIncrementor::from_string("last Sunday").unwrap();
        let zone = crate::time::CalClockZone::utc();
        let result = inc.calculate_date(2024, 6, zone).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 6);
        assert_eq!(result.day_of_week(), crate::constant::DayOfWeek::Sunday);
        // Should be the last Sunday of June 2024 (June 30th is Sunday)
        assert_eq!(result.day(), 30);
    }

    #[test]
    fn test_calculate_complex_expression() {
        // Test "2nd weekday before the 25th" in June 2024
        // June 25th 2024 is Tuesday
        // Weekdays before: Monday 24th, Friday 21st
        // So 2nd weekday before should be Friday 21st
        let inc = DayIncrementor::from_string("2nd weekday before the 25th").unwrap();
        let zone = crate::time::CalClockZone::utc();
        let result = inc.calculate_date(2024, 6, zone).unwrap();
        assert_eq!(result.year(), 2024);
        assert_eq!(result.month(), 6);
        assert_eq!(result.day(), 21); // Friday 21st
    }

    #[test]
    fn test_is_weekday() {
        let inc = DayIncrementor::new();
        let zone = crate::time::CalClockZone::utc();
        let june = crate::constant::MonthOfYear::from_number(6).unwrap();
        
        // Test Monday (weekday)
        let monday = crate::calendar::CalendarDate::from_ymd(2024, june, 3, zone.clone()).unwrap(); // Monday
        assert!(inc.is_weekday(&monday));
        
        // Test Saturday (not weekday)
        let saturday = crate::calendar::CalendarDate::from_ymd(2024, june, 1, zone.clone()).unwrap(); // Saturday
        assert!(!inc.is_weekday(&saturday));
        
        // Test Sunday (not weekday)  
        let sunday = crate::calendar::CalendarDate::from_ymd(2024, june, 2, zone).unwrap(); // Sunday
        assert!(!inc.is_weekday(&sunday));
    }

    #[test]
    fn test_find_nth_day_of_week() {
        let inc = DayIncrementor::new();
        let zone = crate::time::CalClockZone::utc();
        
        // Find 1st Monday in June 2024 (should be 3rd)
        let result = inc.find_nth_day_of_week(2024, 6, crate::constant::DayOfWeek::Monday, 1, zone.clone()).unwrap();
        assert_eq!(result.day(), 3);
        
        // Find 3rd Monday in June 2024 (should be 17th)
        let result = inc.find_nth_day_of_week(2024, 6, crate::constant::DayOfWeek::Monday, 3, zone.clone()).unwrap();
        assert_eq!(result.day(), 17);
        
        // Try to find 5th Monday in June 2024 (should fail - only 4 Mondays)
        let result = inc.find_nth_day_of_week(2024, 6, crate::constant::DayOfWeek::Monday, 5, zone);
        assert!(result.is_err());
    }

    #[test]
    fn test_day_of_month_calculation() {
        let zone = crate::time::CalClockZone::utc();
        
        // Simple day of month
        let inc = DayIncrementor::with_day_of_month(15);
        let result = inc.calculate_date(2024, 6, zone.clone()).unwrap();
        assert_eq!(result.day(), 15);
        
        // Invalid day (32nd of June)
        let inc = DayIncrementor::with_day_of_month(32);
        let result = inc.calculate_date(2024, 6, zone);
        assert!(result.is_err());
    }
}