use crate::{
    calendar::{
        CalendarDay,
        CalendarDuration,
        CalendarInterval,
        CalendarMonth,
        CalendarYear,
        MonthPeriod,
        system::CalendarSystem,
    },
    constant::{
        DayOfWeek,
        MonthOfYear,
    },
    core::Time,
    parser::Parser,
    time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    cmp::Ordering,
    fmt::{self, Display},
};

/// Represents a calendar date with year, month and day.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CalendarDate {
    year:		i32,
    month:		MonthOfYear,
    day:		u8,
    zone:		CalClockZone,
    calendar:	CalendarSystem,
}

impl CalendarDate {
    /// Create a new calendar date using the Gregorian calendar system (default).
    pub fn new(year: i32, month: u8, day: u8, zone: CalClockZone) -> Outcome<Self> {
        Self::new_with_system(year, month, day, zone, CalendarSystem::default())
    }

    /// Create a new calendar date with a specific calendar system.
    pub fn new_with_system(year: i32, month: u8, day: u8, zone: CalClockZone, calendar: CalendarSystem) -> Outcome<Self> {
        let month = res!(MonthOfYear::from_number(month));
        
        // Validate the date using the calendar system
        res!(calendar.validate_date(year, month, day));
        
        Ok(Self {
            year,
            month,
            day,
            zone,
            calendar,
        })
    }
    
    /// Create from year, month enum and day using Gregorian calendar.
    pub fn from_ymd(year: i32, month: MonthOfYear, day: u8, zone: CalClockZone) -> Outcome<Self> {
        Self::from_ymd_with_system(year, month, day, zone, CalendarSystem::default())
    }

    /// Create from year, month enum and day with a specific calendar system.
    pub fn from_ymd_with_system(year: i32, month: MonthOfYear, day: u8, zone: CalClockZone, calendar: CalendarSystem) -> Outcome<Self> {
        res!(calendar.validate_date(year, month, day));
        
        Ok(Self {
            year,
            month,
            day,
            zone,
            calendar,
        })
    }
    
    /// Get today's date in the given timezone.
    pub fn today(zone: CalClockZone) -> Outcome<Self> {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let now = SystemTime::now();
        let duration = res!(now.duration_since(UNIX_EPOCH)
            .map_err(|_| err!("System time is before Unix epoch"; System)));
        
        let days_since_epoch = duration.as_secs() / (24 * 60 * 60);
        Self::from_days_since_epoch(days_since_epoch as i64, zone)
    }
    
    /// Parse a date from a string.
    pub fn parse(input: &str, zone: CalClockZone) -> Outcome<Self> {
        Parser::parse_date(input, zone)
    }
    
    /// Validate a date.
    #[allow(dead_code)]
    fn validate(year: i32, month: MonthOfYear, day: u8) -> Outcome<()> {
        // Validate year range (somewhat arbitrary but reasonable)
        if year < -9999 || year > 9999 {
            return Err(err!(
                "Year {} is out of range (-9999 to 9999)", year;
                Invalid, Input, Range));
        }
        
        // Validate day
        let days_in_month = month.days_in_month(year);
        if day == 0 || day > days_in_month {
            return Err(err!(
                "Day {} is invalid for {}/{} (must be 1-{})",
                day, month.of(), year, days_in_month;
                Invalid, Input, Range));
        }
        
        Ok(())
    }
    
    // Getters
    pub fn year(&self) -> i32 { self.year }
    pub fn month(&self) -> u8 { self.month.of() }
    pub fn month_of_year(&self) -> MonthOfYear { self.month }
    pub fn day(&self) -> u8 { self.day }
    pub fn zone(&self) -> &CalClockZone { &self.zone }
    pub fn calendar_system(&self) -> &CalendarSystem { &self.calendar }
    
    /// Get the day of the week for this date.
    pub fn day_of_week(&self) -> DayOfWeek {
        // Using Zeller's congruence algorithm
        let mut year = self.year;
        let mut month = self.month.of() as i32;
        let day = self.day as i32;
        
        // Adjust for Zeller's algorithm (Jan and Feb are months 13 and 14 of previous year)
        if month <= 2 {
            month += 12;
            year -= 1;
        }
        
        let century = year / 100;
        let year_of_century = year % 100;
        
        let h = (day + (13 * (month + 1)) / 5 + year_of_century + year_of_century / 4 + century / 4 - 2 * century) % 7;
        
        // Convert from Zeller's output (0 = Saturday) to our enum (1 = Monday)
        match h {
            0 => DayOfWeek::Saturday,
            1 => DayOfWeek::Sunday,
            2 => DayOfWeek::Monday,
            3 => DayOfWeek::Tuesday,
            4 => DayOfWeek::Wednesday,
            5 => DayOfWeek::Thursday,
            6 => DayOfWeek::Friday,
            _ => unreachable!(),
        }
    }
    
    /// Check if this is a leap year in this calendar system.
    pub fn is_leap_year(&self) -> bool {
        self.calendar.is_leap_year(self.year)
    }
    
    /// Get the day of year (1-366).
    pub fn day_of_year(&self) -> Outcome<u16> {
        let mut days = self.day as u16;
        
        for month_num in 1..self.month.of() {
            let month = res!(MonthOfYear::from_number(month_num));
            days += self.calendar.days_in_month(self.year, month) as u16;
        }
        
        Ok(days)
    }
    
    /// Get the week of year (1-53) using ISO 8601 week numbering.
    pub fn week_of_year(&self) -> Outcome<u8> {
        // ISO 8601 week numbering - weeks start on Monday
        // Week 1 is the first week with at least 4 days in the new year
        
        // Get January 1st of this year
        let jan1 = res!(Self::new(self.year, 1, 1, self.zone.clone()));
        let jan1_dow = jan1.day_of_week() as u8; // Monday = 1, Sunday = 7
        
        // Calculate the day of year
        let day_of_year = res!(self.day_of_year());
        
        // ISO week calculation
        let jan1_iso_dow = if jan1_dow == 7 { 7 } else { jan1_dow }; // Sunday = 7 in ISO
        
        // Days before the first Monday of the year
        let days_before_first_monday = if jan1_iso_dow == 1 { 0u16 } else { (8 - jan1_iso_dow) as u16 };
        
        // If January 1st is Friday, Saturday, or Sunday, week 1 starts in previous year
        if jan1_iso_dow >= 5 {
            // This date is in week 1 of next year if it's in the last few days
            if day_of_year > (365 + if self.is_leap_year() { 1 } else { 0 } - (jan1_iso_dow - 1) as u16) {
                return Ok(1); // Week 1 of next year
            }
            
            // Otherwise, calculate week number normally
            let adjusted_day = day_of_year + (7u16 - days_before_first_monday);
            Ok(((adjusted_day - 1) / 7 + 1) as u8)
        } else {
            // Week 1 starts in current year
            if day_of_year <= days_before_first_monday {
                // This date belongs to the last week of previous year
                let prev_year_date = res!(Self::new(self.year - 1, 12, 31, self.zone.clone()));
                return prev_year_date.week_of_year();
            }
            
            let adjusted_day = day_of_year - days_before_first_monday;
            Ok(((adjusted_day - 1) / 7 + 1) as u8)
        }
    }
    
    /// Add years, months and days to this date.
    pub fn plus(&self, years: i32, months: i32, days: i32) -> Outcome<Self> {
        // Add years
        let mut new_year = self.year + years;
        
        // Add months
        let total_months = self.month.of() as i32 + months;
        let new_month_num = ((total_months - 1) % 12 + 12) % 12 + 1;
        new_year += (total_months - new_month_num) / 12;
        
        let new_month = res!(MonthOfYear::from_number(new_month_num as u8));
        
        // Adjust day if necessary (e.g., Jan 31 + 1 month = Feb 28/29)
        let new_day = self.day.min(self.calendar.days_in_month(new_year, new_month));
        
        // Create the intermediate date
        let mut result = res!(Self::from_ymd_with_system(new_year, new_month, new_day, self.zone.clone(), self.calendar.clone()));
        
        // Add days
        if days != 0 {
            result = res!(result.add_days(days));
        }
        
        Ok(result)
    }
    
    /// Subtract years, months and days from this date.
    pub fn minus(&self, years: i32, months: i32, days: i32) -> Outcome<Self> {
        self.plus(-years, -months, -days)
    }
    
    /// Add days to this date.
    pub fn add_days(&self, days: i32) -> Outcome<Self> {
        if days == 0 {
            return Ok(self.clone());
        }
        
        // Use Julian day number for accurate date arithmetic
        let julian_day = self.to_julian_day_number();
        let new_julian_day = julian_day + days as i64;
        
        Self::from_julian_day_number(new_julian_day, self.zone.clone())
    }
    
    /// Increment by one day.
    pub fn inc(&self) -> Outcome<Self> {
        self.add_days(1)
    }
    
    /// Decrement by one day.
    pub fn dec(&self) -> Outcome<Self> {
        self.add_days(-1)
    }
    
    /// Convert to a day number (days since a reference epoch).
    /// Using a simple proleptic Gregorian calendar with epoch at year 0.
    pub fn to_day_number(&self) -> Outcome<i64> {
        let mut days: i64 = 0;
        
        // Add days for complete years
        for y in 0..self.year {
            days += if self.calendar.is_leap_year(y) { 366 } else { 365 };
        }
        
        // Subtract days for negative years
        for y in self.year..0 {
            days -= if self.calendar.is_leap_year(y) { 366 } else { 365 };
        }
        
        // Add days for complete months in this year
        for m in 1..self.month.of() {
            let month = res!(MonthOfYear::from_number(m));
            days += month.days_in_month(self.year) as i64;
        }
        
        // Add days in this month
        days += self.day as i64;
        
        Ok(days)
    }
    
    /// Create from a day number.
    #[allow(dead_code)]
    fn from_day_number(mut day_number: i64, zone: CalClockZone) -> Outcome<Self> {
        // Start from year 0 and count forward or backward
        let mut year = 0;
        
        // Find the year
        if day_number > 0 {
            while day_number > 0 {
                let calendar = CalendarSystem::default();
                let days_in_year = if calendar.is_leap_year(year) { 366 } else { 365 };
                if day_number > days_in_year {
                    day_number -= days_in_year;
                    year += 1;
                } else {
                    break;
                }
            }
        } else {
            while day_number <= 0 {
                year -= 1;
                let calendar = CalendarSystem::default();
                let days_in_year = if calendar.is_leap_year(year) { 366 } else { 365 };
                day_number += days_in_year;
            }
        }
        
        // Find the month and day
        for month_num in 1..=12 {
            let month = res!(MonthOfYear::from_number(month_num));
            let days_in_month = month.days_in_month(year) as i64;
            
            if day_number <= days_in_month {
                return Self::from_ymd(year, month, day_number as u8, zone);
            }
            
            day_number -= days_in_month;
        }
        
        Err(err!("Failed to convert day number to date"; Bug, Conversion))
    }
    
    /// Calculate the duration between this date and another.
    pub fn minus_date(&self, other: &Self) -> Outcome<CalendarDuration> {
        let days = self.to_julian_day_number() - other.to_julian_day_number();
        Ok(CalendarDuration::from_days(days as i32))
    }
    
    /// Create an interval from this date to another.
    pub fn to(&self, other: &Self) -> Outcome<CalendarInterval> {
        if other.is_before(self) {
            return Err(err!(
                "End date {} is before start date {}",
                other, self;
                Invalid, Input, Order));
        }
        CalendarInterval::new(self.clone(), other.clone())
    }
    
    /// Get the CalendarYear for this date.
    pub fn get_year(&self) -> CalendarYear {
        CalendarYear::new(self.year)
    }
    
    /// Get the CalendarMonth for this date.  
    pub fn get_month(&self) -> CalendarMonth {
        CalendarMonth::new(self.month.of() as i32)
    }
    
    /// Get the CalendarDay for this date.
    pub fn get_day(&self) -> CalendarDay {
        CalendarDay::new(self.day as i32)
    }
    
    /// Get the MonthPeriod for this date.
    pub fn get_month_period(&self) -> Outcome<MonthPeriod> {
        MonthPeriod::new(self.year, self.month.of(), self.zone.clone())
    }
    
    /// Get the number of days since the Unix epoch (1970-01-01).
    pub fn days_since_epoch(&self) -> Outcome<i64> {
        // Unix epoch (1970-01-01) Julian day number is 2440588
        let unix_epoch_julian_day = 2440588i64;
        let this_julian_day = self.to_julian_day_number();
        
        Ok(this_julian_day - unix_epoch_julian_day)
    }
    
    /// Create a CalendarDate from the number of days since the Unix epoch.
    pub fn from_days_since_epoch(days: i64, zone: CalClockZone) -> Outcome<Self> {
        // Unix epoch (1970-01-01) Julian day number is 2440588
        let unix_epoch_julian_day = 2440588i64;
        let julian_day = unix_epoch_julian_day + days;
        
        Self::from_julian_day_number(julian_day, zone)
    }
    
    /// Check if this date represents a valid calendar date.
    pub fn is_valid(&self) -> bool {
        // Check year range
        if self.year < -9999 || self.year > 9999 {
            return false;
        }
        
        // Check day range for the month
        let days_in_month = self.month.days_in_month(self.year);
        self.day > 0 && self.day <= days_in_month
    }
    
    /// Get the number of days in the month of this date.
    pub fn days_in_month(&self) -> Outcome<u8> {
        Ok(self.month.days_in_month(self.year))
    }
    
    /// Add a CalClockDuration to this date.
    pub fn add_duration(&self, duration: &crate::time::CalClockDuration) -> Outcome<Self> {
        // Extract days from duration and add them
        let days = duration.days();
        self.add_days(days)
    }
    
    /// Add a CalendarDuration to this date.
    pub fn add_calendar_duration(&self, duration: &CalendarDuration) -> Outcome<Self> {
        self.plus(duration.years(), duration.months(), duration.days())
    }
    
    /// Subtract a CalClockDuration from this date.
    pub fn subtract_duration(&self, duration: &crate::time::CalClockDuration) -> Outcome<Self> {
        let days = duration.days();
        self.add_days(-days)
    }
    
    /// Add years to this date.
    pub fn add_years(&self, years: i32) -> Outcome<Self> {
        self.plus(years, 0, 0)
    }
    
    /// Add months to this date.
    pub fn add_months(&self, months: i32) -> Outcome<Self> {
        self.plus(0, months, 0)
    }
    
    // ========================================================================
    // Advanced Calendar Arithmetic
    // ========================================================================
    
    
    // ========================================================================
    // Calendar System Conversion Methods
    // ========================================================================

    /// Converts this date to another calendar system.
    pub fn to_calendar_system(&self, target_calendar: CalendarSystem) -> Outcome<Self> {
        self.calendar.convert_to(&target_calendar, self.year, self.month, self.day, self.zone.clone())
    }

    /// Converts this date to Gregorian calendar.
    pub fn to_gregorian(&self) -> Outcome<Self> {
        self.to_calendar_system(CalendarSystem::Gregorian)
    }

    /// Converts this date to Julian calendar.
    pub fn to_julian(&self) -> Outcome<Self> {
        self.to_calendar_system(CalendarSystem::Julian)
    }

    /// Returns true if this date is in the Gregorian calendar system.
    pub fn is_gregorian(&self) -> bool {
        self.calendar.is_gregorian()
    }

    /// Returns true if this date is in the Julian calendar system.
    pub fn is_julian(&self) -> bool {
        self.calendar.is_julian()
    }

    /// Returns true if this is a weekday (Monday through Friday).
    pub fn is_weekday(&self) -> bool {
        self.day_of_week().is_weekday()
    }
    
    /// Returns true if this is a business day (Monday through Friday).
    pub fn is_business_day(&self) -> bool {
        use crate::constant::DayOfWeek::*;
        matches!(self.day_of_week(), Monday | Tuesday | Wednesday | Thursday | Friday)
    }
    
    /// Returns true if this is a weekend (Saturday or Sunday).
    pub fn is_weekend(&self) -> bool {
        !self.is_business_day()
    }
    
    /// Adds the specified number of business days to this date.
    ///
    /// This method skips weekends and moves to the next business day.
    pub fn add_business_days(&self, business_days: i32) -> Outcome<Self> {
        if business_days == 0 {
            return Ok(self.clone());
        }
        
        let mut current = self.clone();
        let mut remaining = business_days.abs();
        let direction = if business_days > 0 { 1 } else { -1 };
        
        while remaining > 0 {
            current = res!(current.add_days(direction));
            if current.is_business_day() {
                remaining -= 1;
            }
        }
        
        Ok(current)
    }
    
    /// Calculates the number of business days between this date and another.
    pub fn business_days_until(&self, other: &Self) -> Outcome<i32> {
        let start = if self <= other { self.clone() } else { other.clone() };
        let end = if self <= other { other.clone() } else { self.clone() };
        let sign = if self <= other { 1 } else { -1 };
        
        let mut current = start;
        let mut count = 0;
        
        while current < end {
            current = res!(current.add_days(1));
            if current.is_business_day() {
                count += 1;
            }
        }
        
        Ok(count * sign)
    }
    
    /// Returns the next business day from this date.
    pub fn next_business_day(&self) -> Outcome<Self> {
        self.add_business_days(1)
    }
    
    /// Returns the previous business day from this date.
    pub fn previous_business_day(&self) -> Outcome<Self> {
        self.add_business_days(-1)
    }
    
    /// Returns the last day of the current month.
    pub fn end_of_month(&self) -> Outcome<Self> {
        let days_in_month = self.month.days_in_month(self.year);
        Self::new(self.year, self.month.of(), days_in_month, self.zone.clone())
    }
    
    /// Returns the first day of the current month.
    pub fn start_of_month(&self) -> Outcome<Self> {
        Self::new(self.year, self.month.of(), 1, self.zone.clone())
    }
    
    /// Returns the first day of the current year.
    pub fn start_of_year(&self) -> Outcome<Self> {
        Self::new(self.year, 1, 1, self.zone.clone())
    }
    
    /// Returns the last day of the current year.
    pub fn end_of_year(&self) -> Outcome<Self> {
        Self::new(self.year, 12, 31, self.zone.clone())
    }
    
    /// Returns the first day of the current quarter.
    pub fn start_of_quarter(&self) -> Outcome<Self> {
        let quarter_month = match self.month.of() {
            1..=3 => 1,   // Q1
            4..=6 => 4,   // Q2  
            7..=9 => 7,   // Q3
            10..=12 => 10, // Q4
            _ => return Err(err!("Invalid month: {}", self.month.of(); Invalid, Input)),
        };
        Self::new(self.year, quarter_month, 1, self.zone.clone())
    }
    
    /// Returns the last day of the current quarter.
    pub fn end_of_quarter(&self) -> Outcome<Self> {
        let (quarter_month, day) = match self.month.of() {
            1..=3 => (3, 31),   // Q1 - March 31
            4..=6 => (6, 30),   // Q2 - June 30
            7..=9 => (9, 30),   // Q3 - September 30
            10..=12 => (12, 31), // Q4 - December 31
            _ => return Err(err!("Invalid month: {}", self.month.of(); Invalid, Input)),
        };
        Self::new(self.year, quarter_month, day, self.zone.clone())
    }
    
    /// Returns the quarter number (1-4) for this date.
    pub fn quarter(&self) -> u8 {
        match self.month.of() {
            1..=3 => 1,
            4..=6 => 2,
            7..=9 => 3,
            10..=12 => 4,
            _ => 1, // Fallback, though this should never happen
        }
    }
    
    /// Returns the nth occurrence of a specific day of week in the month.
    ///
    /// For example, to find the 2nd Tuesday of the month.
    pub fn nth_weekday_of_month(year: i32, month: u8, weekday: crate::constant::DayOfWeek, n: u8, zone: CalClockZone) -> Outcome<Self> {
        if n == 0 || n > 5 {
            return Err(err!("n must be between 1 and 5, got {}", n; Invalid, Input, Range));
        }
        
        // Start with the first day of the month
        let first_day = res!(Self::new(year, month, 1, zone));
        let first_weekday = first_day.day_of_week();
        
        // Calculate days to add to get to the first occurrence of the target weekday
        let target_weekday_num = weekday as u8;
        let first_weekday_num = first_weekday as u8;
        
        let days_to_first = (target_weekday_num + 7 - first_weekday_num) % 7;
        let target_day = 1 + days_to_first + (n - 1) * 7;
        
        // Check if this day exists in the month
        let month_enum = res!(crate::constant::MonthOfYear::from_number(month));
        if target_day > month_enum.days_in_month(year) {
            return Err(err!("The {}th {} does not exist in {}/{}", n, weekday, month, year; Invalid, Input));
        }
        
        Self::new(year, month, target_day, first_day.zone.clone())
    }
    
    /// Returns the last occurrence of a specific day of week in the month.
    pub fn last_weekday_of_month(year: i32, month: u8, weekday: crate::constant::DayOfWeek, zone: CalClockZone) -> Outcome<Self> {
        let month_enum = res!(crate::constant::MonthOfYear::from_number(month));
        let last_day = res!(Self::new(year, month, month_enum.days_in_month(year), zone));
        let last_weekday = last_day.day_of_week();
        
        // Calculate days to subtract to get to the last occurrence of the target weekday
        let target_weekday_num = weekday as u8;
        let last_weekday_num = last_weekday as u8;
        
        let days_to_subtract = (last_weekday_num + 7 - target_weekday_num) % 7;
        let target_day = month_enum.days_in_month(year) - days_to_subtract;
        
        Self::new(year, month, target_day, last_day.zone.clone())
    }
    
    // ========================================================================
    // Holiday and Special Date Calculations
    // ========================================================================
    
    /// Returns Easter Sunday for the given year using the Western (Gregorian) calculation.
    pub fn easter_sunday(year: i32, zone: CalClockZone) -> Outcome<Self> {
        // Using the anonymous Gregorian algorithm
        let a = year % 19;
        let b = year / 100;
        let c = year % 100;
        let d = b / 4;
        let e = b % 4;
        let f = (b + 8) / 25;
        let g = (b - f + 1) / 3;
        let h = (19 * a + b - d - g + 15) % 30;
        let i = c / 4;
        let k = c % 4;
        let l = (32 + 2 * e + 2 * i - h - k) % 7;
        let m = (a + 11 * h + 22 * l) / 451;
        let month = (h + l - 7 * m + 114) / 31;
        let day = ((h + l - 7 * m + 114) % 31) + 1;
        
        Self::new(year, month as u8, day as u8, zone)
    }
    
    /// Returns Good Friday for the given year.
    pub fn good_friday(year: i32, zone: CalClockZone) -> Outcome<Self> {
        let easter = res!(Self::easter_sunday(year, zone));
        easter.add_days(-2)
    }
    
    /// Returns Easter Monday for the given year.
    pub fn easter_monday(year: i32, zone: CalClockZone) -> Outcome<Self> {
        let easter = res!(Self::easter_sunday(year, zone));
        easter.add_days(1)
    }
    
    /// Returns the date of Mother's Day in the US (second Sunday in May).
    pub fn mothers_day_us(year: i32, zone: CalClockZone) -> Outcome<Self> {
        Self::nth_weekday_of_month(year, 5, crate::constant::DayOfWeek::Sunday, 2, zone)
    }
    
    /// Returns the date of Father's Day in the US (third Sunday in June).
    pub fn fathers_day_us(year: i32, zone: CalClockZone) -> Outcome<Self> {
        Self::nth_weekday_of_month(year, 6, crate::constant::DayOfWeek::Sunday, 3, zone)
    }
    
    /// Returns Thanksgiving Day in the US (fourth Thursday in November).
    pub fn thanksgiving_us(year: i32, zone: CalClockZone) -> Outcome<Self> {
        Self::nth_weekday_of_month(year, 11, crate::constant::DayOfWeek::Thursday, 4, zone)
    }
    
    /// Returns Memorial Day in the US (last Monday in May).
    pub fn memorial_day_us(year: i32, zone: CalClockZone) -> Outcome<Self> {
        Self::last_weekday_of_month(year, 5, crate::constant::DayOfWeek::Monday, zone)
    }
    
    /// Returns Labor Day in the US (first Monday in September).
    pub fn labor_day_us(year: i32, zone: CalClockZone) -> Outcome<Self> {
        Self::nth_weekday_of_month(year, 9, crate::constant::DayOfWeek::Monday, 1, zone)
    }
    
    // ========================================================================
    // Advanced Date Range Operations
    // ========================================================================
    
    /// Returns true if this date falls within the specified range (inclusive).
    pub fn is_between(&self, start: &Self, end: &Self) -> bool {
        self >= start && self <= end
    }
    
    /// Returns the number of years between this date and another.
    pub fn years_until(&self, other: &Self) -> i32 {
        let mut years = other.year - self.year;
        
        // Adjust if we haven't reached the anniversary yet
        if other.month.of() < self.month.of() || 
           (other.month.of() == self.month.of() && other.day < self.day) {
            years -= 1;
        }
        
        years
    }
    
    /// Returns the number of months between this date and another.
    pub fn months_until(&self, other: &Self) -> i32 {
        let mut months = (other.year - self.year) * 12 + (other.month.of() as i32 - self.month.of() as i32);
        
        // Adjust if we haven't reached the anniversary day yet
        if other.day < self.day {
            months -= 1;
        }
        
        months
    }
    
    /// Returns the age in years as of this date, given a birth date.
    pub fn age_years(&self, birth_date: &Self) -> Outcome<u32> {
        if birth_date > self {
            return Err(err!("Birth date cannot be after current date"; Invalid, Input));
        }
        
        let years = self.years_until(birth_date).abs() as u32;
        Ok(years)
    }
    
    /// Converts this date to Julian day number.
    /// 
    /// Uses the same algorithm as Java calclock toJulianDayNumber4
    /// Source: http://pmyers.pcug.org.au/General/JulianDates.htm
    pub fn to_julian_day_number(&self) -> i64 {
        let mut year = self.year as i64;
        let mut month = self.month.of() as i64;
        let day = self.day as i64;
        
        if month > 2 {
            month -= 3;
        } else {
            month += 9;
            year -= 1;
        }
        
        let c = year / 100;
        let ya = year - 100 * c;
        
        (146097 * c) / 4
            + (1461 * ya) / 4
            + (153 * month + 2) / 5
            + day
            + 1721119
    }
    
    /// Creates a CalendarDate from Julian day number.
    /// 
    /// Uses the same algorithm as Java calclock fromJulianDayNumber4
    /// Source: http://pmyers.pcug.org.au/General/JulianDates.htm#J2G
    pub fn from_julian_day_number(mut julian_day: i64, zone: CalClockZone) -> Outcome<Self> {
        julian_day -= 1721119;
        let y = (4 * julian_day - 1) / 146097;
        julian_day = 4 * julian_day - 1 - 146097 * y;
        let mut d = julian_day / 4;
        julian_day = (4 * d + 3) / 1461;
        d = 4 * d + 3 - 1461 * julian_day;
        d = (d + 4) / 4;
        let mut m = (5 * d - 3) / 153;
        d = 5 * d - 3 - 153 * m;
        d = (d + 5) / 5;
        let mut year = 100 * y + julian_day;
        
        if m < 10 {
            m += 3;
        } else {
            m -= 9;
            year += 1;
        }
        
        Self::new(year as i32, m as u8, d as u8, zone)
    }
    
    // ========================================================================
    // String Formatting Implementation (ported from Java original)
    // ========================================================================
    
    /// Formats the date according to the stencil pattern.
    /// 
    /// Format scheme from original Java implementation:
    /// - y yy yyy yyyy yyyyy    Year            8 08 008 2008 02008
    /// - M MM MMM MMMM          Month           3 03 Mar March
    /// - d dd ddd dddd          Day             9 09 Sun Sunday
    /// - z zz zzz zzzz          Timezone        +1 +01 +01:00 UTC
    pub fn format_with_stencil(&self, stencil: &str) -> String {
        let mut output = String::new();
        let chars: Vec<char> = stencil.chars().collect();
        let mut i = 0;
        
        while i < chars.len() {
            let curr = chars[i];
            
            if self.is_recognised_format_char(curr) {
                // Count consecutive occurrences of the same format character
                let mut token_len = 1;
                while i + token_len < chars.len() && chars[i + token_len] == curr {
                    token_len += 1;
                }
                
                // Process the token
                self.switch_on_format_char(curr, token_len, &mut output);
                i += token_len;
            } else {
                // Regular character, append as-is
                output.push(curr);
                i += 1;
            }
        }
        
        output
    }
    
    /// Processes a single format token (ported from Java switchOnFormatChar)
    fn switch_on_format_char(&self, c: char, n: usize, sb: &mut String) {
        match c {
            'y' => self.format_year(n, sb, self.year),
            'M' => self.format_month(n, sb, self.month.of() as i32),
            'd' => {
                if n < 3 {
                    self.format_day(n, sb, self.day as i32);
                } else {
                    self.format_day_of_week(n, sb, self.day_of_week() as i32);
                }
            },
            'z' => self.format_timezone(n, sb),
            _ => {}
        }
    }
    
    /// Formats year component (ported from Java formatYear)
    fn format_year(&self, n: usize, sb: &mut String, val: i32) {
        let val_str = val.to_string();
        let val_len = val_str.len();
        
        if val_len > n {
            // Restrict to last n digits
            let start = val_len - n;
            sb.push_str(&val_str[start..]);
        } else {
            // Zero-pad to n digits
            sb.push_str(&fmt!("{:0width$}", val, width = n));
        }
    }
    
    /// Formats month component (ported from Java formatMonth)
    fn format_month(&self, n: usize, sb: &mut String, val: i32) {
        match n {
            1 => sb.push_str(&val.to_string()),
            2 => sb.push_str(&fmt!("{:02}", val)),
            3 => sb.push_str(self.month.to_short_name()),
            _ => sb.push_str(self.month.to_long_name()),
        }
    }
    
    /// Formats day component (ported from Java formatDay)
    fn format_day(&self, n: usize, sb: &mut String, val: i32) {
        match n {
            1 => sb.push_str(&val.to_string()),
            _ => sb.push_str(&fmt!("{:02}", val)),
        }
    }
    
    /// Formats day of week (ported from Java formatDay for n >= 3)
    fn format_day_of_week(&self, n: usize, sb: &mut String, _val: i32) {
        let day_of_week = self.day_of_week();
        match n {
            3 => sb.push_str(day_of_week.to_short_name()),
            _ => sb.push_str(day_of_week.to_long_name()),
        }
    }
    
    /// Formats timezone component
    fn format_timezone(&self, n: usize, sb: &mut String) {
        match n {
            1 => sb.push_str(self.zone.id()), // Simple ID
            2 => sb.push_str(self.zone.id()), // Short format  
            3 => sb.push_str(self.zone.id()), // Medium format
            _ => sb.push_str(self.zone.id()), // Long format
        }
    }
}

// Add to constant modules - these need implementations
impl MonthOfYear {
    /// Returns short name for month (ported from Java)
    pub fn to_short_name(&self) -> &'static str {
        match self {
            MonthOfYear::January => "Jan",
            MonthOfYear::February => "Feb", 
            MonthOfYear::March => "Mar",
            MonthOfYear::April => "Apr",
            MonthOfYear::May => "May",
            MonthOfYear::June => "Jun",
            MonthOfYear::July => "Jul",
            MonthOfYear::August => "Aug",
            MonthOfYear::September => "Sep",
            MonthOfYear::October => "Oct",
            MonthOfYear::November => "Nov",
            MonthOfYear::December => "Dec",
        }
    }
    
    /// Returns long name for month (ported from Java)
    pub fn to_long_name(&self) -> &'static str {
        match self {
            MonthOfYear::January => "January",
            MonthOfYear::February => "February",
            MonthOfYear::March => "March",
            MonthOfYear::April => "April",
            MonthOfYear::May => "May",
            MonthOfYear::June => "June",
            MonthOfYear::July => "July",
            MonthOfYear::August => "August",
            MonthOfYear::September => "September",
            MonthOfYear::October => "October",
            MonthOfYear::November => "November",
            MonthOfYear::December => "December",
        }
    }
}

impl DayOfWeek {
    /// Returns short name for day of week (ported from Java)
    pub fn to_short_name(&self) -> &'static str {
        match self {
            DayOfWeek::Monday => "Mon",
            DayOfWeek::Tuesday => "Tue",
            DayOfWeek::Wednesday => "Wed",
            DayOfWeek::Thursday => "Thu",
            DayOfWeek::Friday => "Fri",
            DayOfWeek::Saturday => "Sat",
            DayOfWeek::Sunday => "Sun",
        }
    }
    
    /// Returns long name for day of week (ported from Java)
    pub fn to_long_name(&self) -> &'static str {
        match self {
            DayOfWeek::Monday => "Monday",
            DayOfWeek::Tuesday => "Tuesday",
            DayOfWeek::Wednesday => "Wednesday",
            DayOfWeek::Thursday => "Thursday",
            DayOfWeek::Friday => "Friday",
            DayOfWeek::Saturday => "Saturday",
            DayOfWeek::Sunday => "Sunday",
        }
    }
}

impl Time for CalendarDate {
    fn get_zone(&self) -> &CalClockZone {
        &self.zone
    }
    
    fn to_zone(&self, new_zone: CalClockZone) -> Outcome<Self> {
        Ok(Self {
            zone: new_zone,
            ..self.clone()
        })
    }
    
    fn format(&self, stencil: &str) -> String {
        self.format_with_stencil(stencil)
    }
    
    fn is_recognised_format_char(&self, c: char) -> bool {
        matches!(c, 'y' | 'M' | 'd' | 'E' | 'z')
    }
    
    fn is_before(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Less
    }
    
    fn is_after(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Greater
    }
    
    fn or_earlier(&self, other: &Self) -> Self {
        if self.is_before(other) { self.clone() } else { other.clone() }
    }
    
    fn or_later(&self, other: &Self) -> Self {
        if self.is_after(other) { self.clone() } else { other.clone() }
    }
}

impl Ord for CalendarDate {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.year.cmp(&other.year) {
            Ordering::Equal => match self.month.cmp(&other.month) {
                Ordering::Equal => self.day.cmp(&other.day),
                other => other,
            },
            other => other,
        }
    }
}

impl PartialOrd for CalendarDate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for CalendarDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.calendar.is_gregorian() {
            write!(f, "{:04}-{:02}-{:02}", self.year, self.month.of(), self.day)
        } else {
            write!(f, "{:04}-{:02}-{:02} ({})", self.year, self.month.of(), self.day, self.calendar)
        }
    }
}