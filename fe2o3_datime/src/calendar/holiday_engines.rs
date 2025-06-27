/// Advanced holiday calculation engines for different jurisdictions.
/// 
/// This module provides sophisticated holiday engines that can automatically
/// calculate complex holidays like Easter, Thanksgiving, and other date-based
/// holidays for various countries and jurisdictions.

use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::CalClockZone,
};
use oxedyne_fe2o3_core::prelude::*;
use std::collections::HashMap;

/// Represents different types of holiday calculation methods.
#[derive(Clone, Debug, PartialEq)]
pub enum HolidayType {
    /// Fixed date holiday (e.g., Christmas on December 25).
    Fixed { month: u8, day: u8 },
    /// Easter-based holiday (offset from Easter Sunday).
    EasterBased { offset_days: i32 },
    /// Relative holiday (e.g., "3rd Monday in February").
    Relative { month: u8, weekday: DayOfWeek, occurrence: i32 },
    /// Last occurrence of a weekday in a month (e.g., "last Monday in May").
    LastWeekday { month: u8, weekday: DayOfWeek },
    /// First occurrence of a weekday after a specific date.
    FirstWeekdayAfter { month: u8, day: u8, weekday: DayOfWeek },
}

/// Represents a holiday with its calculation method and metadata.
#[derive(Clone, Debug)]
pub struct HolidayDefinition {
    /// Name of the holiday.
    pub name: String,
    /// How to calculate the holiday date.
    pub holiday_type: HolidayType,
    /// Whether this holiday is observed if it falls on a weekend.
    pub weekend_adjustment: WeekendAdjustment,
    /// Optional description.
    pub description: Option<String>,
}

/// Defines how holidays are adjusted when they fall on weekends.
#[derive(Clone, Debug, PartialEq)]
pub enum WeekendAdjustment {
    /// No adjustment - holiday is observed on the actual date.
    None,
    /// If holiday falls on Saturday, observe on Friday; if Sunday, observe on Monday.
    Nearest,
    /// Always observe on the following Monday if weekend.
    Monday,
    /// Always observe on the preceding Friday if weekend.
    Friday,
    /// Custom adjustment rules.
    Custom { saturday_shift: i32, sunday_shift: i32 },
}

/// A comprehensive holiday engine for calculating holidays in specific jurisdictions.
#[derive(Clone, Debug)]
pub struct HolidayEngine {
    /// Name of the jurisdiction (e.g., "United States", "United Kingdom").
    jurisdiction: String,
    /// Map of holiday definitions by name.
    holidays: HashMap<String, HolidayDefinition>,
    /// Default weekend adjustment policy.
    default_weekend_adjustment: WeekendAdjustment,
}

impl HolidayEngine {
    /// Creates a new holiday engine for a specific jurisdiction.
    pub fn new<S: Into<String>>(jurisdiction: S) -> Self {
        Self {
            jurisdiction: jurisdiction.into(),
            holidays: HashMap::new(),
            default_weekend_adjustment: WeekendAdjustment::None,
        }
    }

    /// Sets the default weekend adjustment policy.
    pub fn with_default_weekend_adjustment(mut self, adjustment: WeekendAdjustment) -> Self {
        self.default_weekend_adjustment = adjustment;
        self
    }

    /// Adds a holiday definition to the engine.
    pub fn add_holiday(mut self, holiday: HolidayDefinition) -> Self {
        self.holidays.insert(holiday.name.clone(), holiday);
        self
    }

    /// Calculates all holidays for a given year.
    pub fn calculate_holidays(&self, year: i32, zone: CalClockZone) -> Outcome<Vec<(String, CalendarDate)>> {
        let mut holidays = Vec::new();

        for (name, definition) in &self.holidays {
            let calculated_date = res!(self.calculate_holiday(definition, year, zone.clone()));
            holidays.push((name.clone(), calculated_date));
        }

        // Sort by date.
        holidays.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(holidays)
    }

    /// Calculates a specific holiday for a given year.
    pub fn calculate_holiday(&self, definition: &HolidayDefinition, year: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        let base_date = match &definition.holiday_type {
            HolidayType::Fixed { month, day } => {
                let month_enum = res!(MonthOfYear::from_number(*month));
                res!(CalendarDate::from_ymd(year, month_enum, *day, zone.clone()))
            },
            HolidayType::EasterBased { offset_days } => {
                let easter = res!(Self::calculate_easter(year, zone.clone()));
                res!(easter.add_days(*offset_days))
            },
            HolidayType::Relative { month, weekday, occurrence } => {
                res!(Self::calculate_nth_weekday(year, *month, *weekday, *occurrence, zone.clone()))
            },
            HolidayType::LastWeekday { month, weekday } => {
                res!(Self::calculate_last_weekday(year, *month, *weekday, zone.clone()))
            },
            HolidayType::FirstWeekdayAfter { month, day, weekday } => {
                res!(Self::calculate_first_weekday_after(year, *month, *day, *weekday, zone.clone()))
            },
        };

        // Apply weekend adjustment.
        self.apply_weekend_adjustment(&base_date, &definition.weekend_adjustment)
    }

    /// Applies weekend adjustment rules to a date.
    fn apply_weekend_adjustment(&self, date: &CalendarDate, adjustment: &WeekendAdjustment) -> Outcome<CalendarDate> {
        let dow = date.day_of_week();
        
        match adjustment {
            WeekendAdjustment::None => Ok(date.clone()),
            WeekendAdjustment::Nearest => {
                match dow {
                    DayOfWeek::Saturday => date.add_days(-1),  // Friday
                    DayOfWeek::Sunday => date.add_days(1),     // Monday
                    _ => Ok(date.clone()),
                }
            },
            WeekendAdjustment::Monday => {
                match dow {
                    DayOfWeek::Saturday | DayOfWeek::Sunday => {
                        // Find next Monday.
                        let days_to_monday = match dow {
                            DayOfWeek::Saturday => 2,
                            DayOfWeek::Sunday => 1,
                            _ => 0,
                        };
                        date.add_days(days_to_monday)
                    },
                    _ => Ok(date.clone()),
                }
            },
            WeekendAdjustment::Friday => {
                match dow {
                    DayOfWeek::Saturday | DayOfWeek::Sunday => {
                        // Find previous Friday.
                        let days_to_friday = match dow {
                            DayOfWeek::Saturday => -1,
                            DayOfWeek::Sunday => -2,
                            _ => 0,
                        };
                        date.add_days(days_to_friday)
                    },
                    _ => Ok(date.clone()),
                }
            },
            WeekendAdjustment::Custom { saturday_shift, sunday_shift } => {
                match dow {
                    DayOfWeek::Saturday => date.add_days(*saturday_shift),
                    DayOfWeek::Sunday => date.add_days(*sunday_shift),
                    _ => Ok(date.clone()),
                }
            },
        }
    }

    /// Calculates Easter Sunday for a given year using the Western (Gregorian) algorithm.
    /// This implements the algorithm used by most Western churches.
    pub fn calculate_easter(year: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        // Use the algorithm from Oudin (1940) for Gregorian calendar Easter.
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

        let month_enum = res!(MonthOfYear::from_number(month as u8));
        CalendarDate::from_ymd(year, month_enum, day as u8, zone)
    }

    /// Calculates the nth occurrence of a weekday in a given month.
    /// For example, the 3rd Monday in February.
    fn calculate_nth_weekday(year: i32, month: u8, weekday: DayOfWeek, occurrence: i32, zone: CalClockZone) -> Outcome<CalendarDate> {
        let month_enum = res!(MonthOfYear::from_number(month));
        let first_of_month = res!(CalendarDate::from_ymd(year, month_enum, 1, zone.clone()));
        
        // Find the first occurrence of the target weekday.
        let first_weekday_day = {
            let first_dow = first_of_month.day_of_week();
            let target_dow_num = weekday.of() as i32;
            let first_dow_num = first_dow.of() as i32;
            
            let days_to_add = if target_dow_num >= first_dow_num {
                target_dow_num - first_dow_num
            } else {
                7 - (first_dow_num - target_dow_num)
            };
            1 + days_to_add
        };

        // Calculate the nth occurrence.
        let target_day = first_weekday_day + (occurrence - 1) * 7;
        
        // Verify the day is within the month.
        let days_in_month = month_enum.days_in_month(year);
        if target_day > days_in_month as i32 {
            return Err(err!("No {}th {} in {} {}", occurrence, weekday, month_enum, year; Invalid, Input));
        }

        CalendarDate::from_ymd(year, month_enum, target_day as u8, zone)
    }

    /// Calculates the last occurrence of a weekday in a given month.
    /// For example, the last Monday in May.
    fn calculate_last_weekday(year: i32, month: u8, weekday: DayOfWeek, zone: CalClockZone) -> Outcome<CalendarDate> {
        let month_enum = res!(MonthOfYear::from_number(month));
        let days_in_month = month_enum.days_in_month(year);
        
        // Start from the last day and work backwards.
        for day in (1..=days_in_month).rev() {
            let candidate = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
            if candidate.day_of_week() == weekday {
                return Ok(candidate);
            }
        }

        Err(err!("No {} found in {} {}", weekday, month_enum, year; Invalid, Input))
    }

    /// Calculates the first occurrence of a weekday after a specific date.
    /// For example, the first Monday after March 15th.
    fn calculate_first_weekday_after(year: i32, month: u8, day: u8, weekday: DayOfWeek, zone: CalClockZone) -> Outcome<CalendarDate> {
        let month_enum = res!(MonthOfYear::from_number(month));
        let start_date = res!(CalendarDate::from_ymd(year, month_enum, day, zone.clone()));
        
        // Search forward for the target weekday.
        let mut current = res!(start_date.add_days(1)); // Start from the day after.
        for _ in 0..7 { // Maximum 7 days to find any weekday.
            if current.day_of_week() == weekday {
                return Ok(current);
            }
            current = res!(current.add_days(1));
        }

        Err(err!("Could not find {} after {} {} {}", weekday, month_enum, day, year; Invalid, Input))
    }

    /// Checks if a given date is a holiday according to this engine.
    pub fn is_holiday(&self, date: &CalendarDate) -> Outcome<bool> {
        let year = date.year();
        let holidays = res!(self.calculate_holidays(year, date.zone().clone()));
        
        for (_, holiday_date) in holidays {
            if holiday_date == *date {
                return Ok(true);
            }
        }
        
        Ok(false)
    }

    /// Gets the name of the jurisdiction.
    pub fn jurisdiction(&self) -> &str {
        &self.jurisdiction
    }

    /// Gets all holiday names.
    pub fn holiday_names(&self) -> Vec<String> {
        self.holidays.keys().cloned().collect()
    }
}

/// Pre-built holiday engines for common jurisdictions.
impl HolidayEngine {
    /// Creates a United States federal holiday engine.
    pub fn us_federal() -> Self {
        Self::new("United States Federal")
            .with_default_weekend_adjustment(WeekendAdjustment::Nearest)
            .add_holiday(HolidayDefinition {
                name: "New Year's Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 1, day: 1 },
                weekend_adjustment: WeekendAdjustment::Nearest,
                description: Some("January 1st".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Martin Luther King Jr. Day".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 1, 
                    weekday: DayOfWeek::Monday, 
                    occurrence: 3 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Third Monday in January".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Presidents' Day".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 2, 
                    weekday: DayOfWeek::Monday, 
                    occurrence: 3 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Third Monday in February".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Memorial Day".to_string(),
                holiday_type: HolidayType::LastWeekday { 
                    month: 5, 
                    weekday: DayOfWeek::Monday 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Last Monday in May".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Independence Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 7, day: 4 },
                weekend_adjustment: WeekendAdjustment::Nearest,
                description: Some("July 4th".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Labour Day".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 9, 
                    weekday: DayOfWeek::Monday, 
                    occurrence: 1 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("First Monday in September".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Columbus Day".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 10, 
                    weekday: DayOfWeek::Monday, 
                    occurrence: 2 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Second Monday in October".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Veterans Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 11, day: 11 },
                weekend_adjustment: WeekendAdjustment::Nearest,
                description: Some("November 11th".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Thanksgiving".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 11, 
                    weekday: DayOfWeek::Thursday, 
                    occurrence: 4 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Thursday.
                description: Some("Fourth Thursday in November".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Christmas Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 12, day: 25 },
                weekend_adjustment: WeekendAdjustment::Nearest,
                description: Some("December 25th".to_string()),
            })
    }

    /// Creates a United Kingdom holiday engine.
    pub fn uk() -> Self {
        Self::new("United Kingdom")
            .with_default_weekend_adjustment(WeekendAdjustment::Monday)
            .add_holiday(HolidayDefinition {
                name: "New Year's Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 1, day: 1 },
                weekend_adjustment: WeekendAdjustment::Monday,
                description: Some("January 1st".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Good Friday".to_string(),
                holiday_type: HolidayType::EasterBased { offset_days: -2 },
                weekend_adjustment: WeekendAdjustment::None, // Always on Friday.
                description: Some("Friday before Easter".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Easter Monday".to_string(),
                holiday_type: HolidayType::EasterBased { offset_days: 1 },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Monday after Easter".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Early May Bank Holiday".to_string(),
                holiday_type: HolidayType::Relative { 
                    month: 5, 
                    weekday: DayOfWeek::Monday, 
                    occurrence: 1 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("First Monday in May".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Spring Bank Holiday".to_string(),
                holiday_type: HolidayType::LastWeekday { 
                    month: 5, 
                    weekday: DayOfWeek::Monday 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Last Monday in May".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Summer Bank Holiday".to_string(),
                holiday_type: HolidayType::LastWeekday { 
                    month: 8, 
                    weekday: DayOfWeek::Monday 
                },
                weekend_adjustment: WeekendAdjustment::None, // Always on Monday.
                description: Some("Last Monday in August".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Christmas Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 12, day: 25 },
                weekend_adjustment: WeekendAdjustment::Monday,
                description: Some("December 25th".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Boxing Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 12, day: 26 },
                weekend_adjustment: WeekendAdjustment::Monday,
                description: Some("December 26th".to_string()),
            })
    }

    /// Creates a European Central Bank holiday engine.
    pub fn ecb() -> Self {
        Self::new("European Central Bank")
            .with_default_weekend_adjustment(WeekendAdjustment::None)
            .add_holiday(HolidayDefinition {
                name: "New Year's Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 1, day: 1 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("January 1st".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Good Friday".to_string(),
                holiday_type: HolidayType::EasterBased { offset_days: -2 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("Friday before Easter".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Easter Monday".to_string(),
                holiday_type: HolidayType::EasterBased { offset_days: 1 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("Monday after Easter".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Labour Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 5, day: 1 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("May 1st".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Christmas Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 12, day: 25 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("December 25th".to_string()),
            })
            .add_holiday(HolidayDefinition {
                name: "Boxing Day".to_string(),
                holiday_type: HolidayType::Fixed { month: 12, day: 26 },
                weekend_adjustment: WeekendAdjustment::None,
                description: Some("December 26th".to_string()),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CalClockZone;

    #[test]
    fn test_easter_calculation() {
        let zone = CalClockZone::utc();
        
        // Test known Easter dates.
        let easter_2024 = HolidayEngine::calculate_easter(2024, zone.clone()).unwrap();
        assert_eq!(easter_2024.month(), 3);
        assert_eq!(easter_2024.day(), 31);
        
        let easter_2025 = HolidayEngine::calculate_easter(2025, zone.clone()).unwrap();
        assert_eq!(easter_2025.month(), 4);
        assert_eq!(easter_2025.day(), 20);
    }

    #[test]
    fn test_us_federal_holidays() {
        let zone = CalClockZone::utc();
        let engine = HolidayEngine::us_federal();
        let holidays_2024 = engine.calculate_holidays(2024, zone).unwrap();
        
        // Should have 10 federal holidays.
        assert_eq!(holidays_2024.len(), 10);
        
        // Check a few specific holidays.
        let new_years = holidays_2024.iter().find(|(name, _)| name == "New Year's Day").unwrap();
        assert_eq!(new_years.1.month(), 1);
        assert_eq!(new_years.1.day(), 1);
        
        let thanksgiving = holidays_2024.iter().find(|(name, _)| name == "Thanksgiving").unwrap();
        assert_eq!(thanksgiving.1.month(), 11);
        assert_eq!(thanksgiving.1.day(), 28); // Fourth Thursday in November 2024.
    }

    #[test]
    fn test_weekend_adjustment() {
        let zone = CalClockZone::utc();
        let engine = HolidayEngine::us_federal();
        
        // Test a year where July 4th falls on a weekend.
        let july_4_2021 = CalendarDate::from_ymd(2021, MonthOfYear::July, 4, zone.clone()).unwrap();
        assert_eq!(july_4_2021.day_of_week(), DayOfWeek::Sunday);
        
        let adjusted = engine.apply_weekend_adjustment(&july_4_2021, &WeekendAdjustment::Nearest).unwrap();
        assert_eq!(adjusted.day(), 5); // Should be observed on Monday.
    }

    #[test]
    fn test_uk_holidays() {
        let zone = CalClockZone::utc();
        let engine = HolidayEngine::uk();
        let holidays_2024 = engine.calculate_holidays(2024, zone).unwrap();
        
        // Should have 8 UK holidays.
        assert_eq!(holidays_2024.len(), 8);
        
        // Check Good Friday (Easter-based).
        let good_friday = holidays_2024.iter().find(|(name, _)| name == "Good Friday").unwrap();
        assert_eq!(good_friday.1.month(), 3);
        assert_eq!(good_friday.1.day(), 29); // Good Friday 2024.
    }

    #[test]
    fn test_holiday_type_calculations() {
        let zone = CalClockZone::utc();
        
        // Test relative calculation (3rd Monday in January).
        let mlk_day = HolidayEngine::calculate_nth_weekday(2024, 1, DayOfWeek::Monday, 3, zone.clone()).unwrap();
        assert_eq!(mlk_day.day(), 15); // January 15, 2024.
        
        // Test last weekday calculation (last Monday in May).
        let memorial_day = HolidayEngine::calculate_last_weekday(2024, 5, DayOfWeek::Monday, zone.clone()).unwrap();
        assert_eq!(memorial_day.day(), 27); // May 27, 2024.
    }
}