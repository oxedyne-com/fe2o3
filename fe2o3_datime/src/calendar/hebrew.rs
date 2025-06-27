use crate::{
	calendar::CalendarDate,
};
use oxedyne_fe2o3_core::prelude::*;

/// Hebrew calendar implementation.
///
/// The Hebrew calendar is a lunisolar calendar used for Jewish religious observances.
/// It has 12 months in common years and 13 months in leap years, with months having
/// either 29 or 30 days. Years can be deficient, regular, or abundant.
#[derive(Debug, Clone, PartialEq)]
pub struct HebrewCalendar;

/// Hebrew months enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HebrewMonth {
	Tishrei = 1,
	Cheshvan = 2,
	Kislev = 3,
	Tevet = 4,
	Shevat = 5,
	Adar = 6,      // In leap years, this becomes Adar II
	AdarI = 7,     // Only exists in leap years
	Nisan = 8,
	Iyar = 9,
	Sivan = 10,
	Tammuz = 11,
	Av = 12,
	Elul = 13,
}

impl HebrewMonth {
	/// Returns the month number for regular years.
	pub fn number(&self, is_leap_year: bool) -> u8 {
		match (self, is_leap_year) {
			(Self::Tishrei, _) => 1,
			(Self::Cheshvan, _) => 2,
			(Self::Kislev, _) => 3,
			(Self::Tevet, _) => 4,
			(Self::Shevat, _) => 5,
			(Self::AdarI, true) => 6,
			(Self::Adar, false) => 6,
			(Self::Adar, true) => 7,  // Adar II in leap years
			(Self::AdarI, false) => return 0, // Invalid in non-leap years
			(Self::Nisan, false) => 7,
			(Self::Nisan, true) => 8,
			(Self::Iyar, false) => 8,
			(Self::Iyar, true) => 9,
			(Self::Sivan, false) => 9,
			(Self::Sivan, true) => 10,
			(Self::Tammuz, false) => 10,
			(Self::Tammuz, true) => 11,
			(Self::Av, false) => 11,
			(Self::Av, true) => 12,
			(Self::Elul, false) => 12,
			(Self::Elul, true) => 13,
		}
	}
	
	/// Creates a Hebrew month from its number.
	pub fn from_number(month: u8, is_leap_year: bool) -> Outcome<Self> {
		match (month, is_leap_year) {
			(1, _) => Ok(Self::Tishrei),
			(2, _) => Ok(Self::Cheshvan),
			(3, _) => Ok(Self::Kislev),
			(4, _) => Ok(Self::Tevet),
			(5, _) => Ok(Self::Shevat),
			(6, false) => Ok(Self::Adar),
			(6, true) => Ok(Self::AdarI),
			(7, false) => Ok(Self::Nisan),
			(7, true) => Ok(Self::Adar), // Adar II
			(8, false) => Ok(Self::Iyar),
			(8, true) => Ok(Self::Nisan),
			(9, false) => Ok(Self::Sivan),
			(9, true) => Ok(Self::Iyar),
			(10, false) => Ok(Self::Tammuz),
			(10, true) => Ok(Self::Sivan),
			(11, false) => Ok(Self::Av),
			(11, true) => Ok(Self::Tammuz),
			(12, false) => Ok(Self::Elul),
			(12, true) => Ok(Self::Av),
			(13, true) => Ok(Self::Elul),
			_ => Err(err!("Invalid Hebrew month number: {}", month; Invalid, Input)),
		}
	}
	
	/// Returns the English name of the month.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Tishrei => "Tishrei",
			Self::Cheshvan => "Cheshvan",
			Self::Kislev => "Kislev",
			Self::Tevet => "Tevet",
			Self::Shevat => "Shevat",
			Self::Adar => "Adar",
			Self::AdarI => "Adar I",
			Self::Nisan => "Nisan",
			Self::Iyar => "Iyar",
			Self::Sivan => "Sivan",
			Self::Tammuz => "Tammuz",
			Self::Av => "Av",
			Self::Elul => "Elul",
		}
	}
}

impl HebrewCalendar {
	/// Creates a new Hebrew calendar instance.
	pub fn new() -> Self {
		Self
	}
	
	/// Checks if a Hebrew year is a leap year.
	/// Hebrew leap years occur 7 times in a 19-year cycle.
	pub fn is_hebrew_leap_year(year: i32) -> bool {
		((year * 7 + 1) % 19) < 7
	}
	
	/// Returns the number of months in a Hebrew year.
	pub fn months_in_hebrew_year(year: i32) -> u8 {
		if Self::is_hebrew_leap_year(year) {
			13
		} else {
			12
		}
	}
	
	/// Returns the number of days in a Hebrew month.
	pub fn days_in_hebrew_month(year: i32, month: u8) -> Outcome<u8> {
		let is_leap = Self::is_hebrew_leap_year(year);
		
		match (month, is_leap) {
			(1, _) => Ok(30),  // Tishrei always has 30 days
			(2, _) => {        // Cheshvan can have 29 or 30 days
				let year_length = Self::days_in_hebrew_year(year);
				if year_length % 10 == 3 || year_length % 10 == 5 {
					Ok(29)
				} else {
					Ok(30)
				}
			},
			(3, _) => {        // Kislev can have 29 or 30 days
				let year_length = Self::days_in_hebrew_year(year);
				if year_length % 10 == 3 {
					Ok(29)
				} else {
					Ok(30)
				}
			},
			(4, _) => Ok(29),  // Tevet
			(5, _) => Ok(30),  // Shevat
			(6, false) => Ok(29), // Adar in regular year
			(6, true) => Ok(30),  // Adar I in leap year
			(7, false) => Ok(30), // Nisan in regular year
			(7, true) => Ok(29),  // Adar II in leap year
			(8, false) => Ok(29), // Iyar in regular year
			(8, true) => Ok(30),  // Nisan in leap year
			(9, false) => Ok(30), // Sivan in regular year
			(9, true) => Ok(29),  // Iyar in leap year
			(10, false) => Ok(29), // Tammuz in regular year
			(10, true) => Ok(30),  // Sivan in leap year
			(11, false) => Ok(30), // Av in regular year
			(11, true) => Ok(29),  // Tammuz in leap year
			(12, false) => Ok(29), // Elul in regular year
			(12, true) => Ok(30),  // Av in leap year
			(13, true) => Ok(29),  // Elul in leap year
			_ => Err(err!("Invalid Hebrew month: {}", month; Invalid, Input)),
		}
	}
	
	/// Returns the number of days in a Hebrew year.
	pub fn days_in_hebrew_year(year: i32) -> i32 {
		Self::hebrew_elapsed_days(year + 1) - Self::hebrew_elapsed_days(year)
	}
	
	/// Calculates elapsed days since Hebrew epoch (1 Tishrei year 1).
	/// Uses Gauss's algorithm for Hebrew calendar calculations.
	pub fn hebrew_elapsed_days(year: i32) -> i32 {
		let months_elapsed = (235 * ((year - 1) / 19)) + // Complete cycles of 19 years
		                    (12 * ((year - 1) % 19)) +   // Regular months in incomplete cycle
		                    ((7 * ((year - 1) % 19) + 1) / 19); // Leap months
		
		let parts_elapsed = 204 + 793 * (months_elapsed % 1080);
		let hours_elapsed = 5 + 12 * months_elapsed + 793 * (months_elapsed / 1080) + 
		                   parts_elapsed / 1080;
		
		let day = 1 + 29 * months_elapsed + hours_elapsed / 24;
		let parts = 1080 * (hours_elapsed % 24) + parts_elapsed % 1080;
		
		let mut alternative_day = day;
		if parts >= 19440 || // If new moon is at or after noon
		   (day % 7 == 2 && parts >= 9924 && !Self::is_hebrew_leap_year(year)) || // Tuesday restriction
		   (day % 7 == 1 && parts >= 16789 && Self::is_hebrew_leap_year(year - 1)) { // Monday restriction
			alternative_day += 1;
		}
		
		// Adjust for forbidden days (Rosh Hashanah cannot fall on Sunday, Wednesday, or Friday)
		if alternative_day % 7 == 0 || alternative_day % 7 == 3 || alternative_day % 7 == 5 {
			alternative_day += 1;
		}
		
		alternative_day
	}
	
	/// Converts a Gregorian date to Hebrew date.
	pub fn from_gregorian(year: i32, month: u8, day: u8) -> Outcome<(i32, u8, u8)> {
		// Convert Gregorian to Julian Day Number
		let jdn = Self::gregorian_to_jdn(year, month, day);
		
		// Convert JDN to Hebrew date
		Self::jdn_to_hebrew(jdn)
	}
	
	/// Converts a Hebrew date to Gregorian date.
	pub fn to_gregorian(hebrew_year: i32, hebrew_month: u8, hebrew_day: u8) -> Outcome<(i32, u8, u8)> {
		// Validate Hebrew date
		res!(Self::validate_hebrew_date(hebrew_year, hebrew_month, hebrew_day));
		
		// Convert Hebrew to Julian Day Number
		let jdn = res!(Self::hebrew_to_jdn(hebrew_year, hebrew_month, hebrew_day));
		
		// Convert JDN to Gregorian date
		Ok(Self::jdn_to_gregorian(jdn))
	}
	
	/// Converts a Gregorian date to Julian Day Number.
	pub fn gregorian_to_jdn(year: i32, month: u8, day: u8) -> i64 {
		let (y, m) = if month <= 2 {
			(year - 1, month as i32 + 12)
		} else {
			(year, month as i32)
		};
		
		let a = y / 100;
		let b = 2 - a + a / 4;
		
		let jdn = (365.25 * (y + 4716) as f64) as i64 +
		         (30.6001 * (m + 1) as f64) as i64 +
		         day as i64 + b as i64 - 1524;
		
		jdn
	}
	
	/// Converts Julian Day Number to Gregorian date.
	fn jdn_to_gregorian(jdn: i64) -> (i32, u8, u8) {
		let a = jdn + 32044;
		let b = (4 * a + 3) / 146097;
		let c = a - (146097 * b) / 4;
		let d = (4 * c + 3) / 1461;
		let e = c - (1461 * d) / 4;
		let m = (5 * e + 2) / 153;
		
		let day = (e - (153 * m + 2) / 5 + 1) as u8;
		let month = (m + 3 - 12 * (m / 10)) as u8;
		let year = (100 * b + d - 4800 + m / 10) as i32;
		
		(year, month, day)
	}
	
	/// Returns days in a Gregorian month.
	fn gregorian_month_days(year: i32, month: u8) -> u8 {
		match month {
			1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
			4 | 6 | 9 | 11 => 30,
			2 => {
				if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
					29
				} else {
					28
				}
			},
			_ => 0,
		}
	}
	
	/// Converts Hebrew date to Julian Day Number.
	fn hebrew_to_jdn(year: i32, month: u8, day: u8) -> Outcome<i64> {
		// Calculate days elapsed from 1 Tishrei year 1
		let mut days = Self::hebrew_elapsed_days(year) + day as i32 - 1;
		
		// Add days for months in the current year (before the specified month)
		for m in 1..month {
			days += res!(Self::days_in_hebrew_month(year, m)) as i32;
		}
		
		// The Hebrew epoch (1 Tishrei 1) corresponds to Julian Day Number 347997
		// This is Sunday, October 6, 3761 BCE in the proleptic Gregorian calendar
		let hebrew_epoch_jdn = 347997i64;
		
		Ok(days as i64 + hebrew_epoch_jdn)
	}
	
	/// Converts Julian Day Number to Hebrew date.
	fn jdn_to_hebrew(jdn: i64) -> Outcome<(i32, u8, u8)> {
		// Convert JDN to Hebrew days since epoch
		let hebrew_epoch_jdn = 347997i64;
		let hebrew_days = (jdn - hebrew_epoch_jdn) as i32;
		
		// Start from a reasonable year estimate based on the day count
		// Approximate: 354 days per year on average
		let mut year = 1 + hebrew_days / 354;
		if year < 1 {
			year = 1;
		}
		
		// Adjust year to find the correct one
		while year > 1 && Self::hebrew_elapsed_days(year) > hebrew_days {
			year -= 1;
		}
		
		while Self::hebrew_elapsed_days(year + 1) <= hebrew_days {
			year += 1;
		}
		
		// Find month and day within the year
		let year_start = Self::hebrew_elapsed_days(year);
		let day_of_year = hebrew_days - year_start + 1;
		
		if day_of_year <= 0 {
			return Err(err!("Invalid day calculation for JDN {}", jdn; Invalid));
		}
		
		let mut month = 1;
		let mut days_counted = 0;
		
		let months_in_year = Self::months_in_hebrew_year(year);
		while month <= months_in_year {
			let days_in_month = res!(Self::days_in_hebrew_month(year, month)) as i32;
			if days_counted + days_in_month >= day_of_year {
				let day = day_of_year - days_counted;
				if day <= 0 || day > days_in_month {
					return Err(err!("Invalid day {} for month {} in Hebrew year {}", day, month, year; Invalid));
				}
				return Ok((year, month, day as u8));
			}
			days_counted += days_in_month;
			month += 1;
		}
		
		Err(err!("Failed to convert JDN {} to Hebrew date (year {}, day_of_year {})", jdn, year, day_of_year; Invalid))
	}
	
	/// Validates a Hebrew date.
	pub fn validate_hebrew_date(year: i32, month: u8, day: u8) -> Outcome<()> {
		if year < 1 || year > 9999 {
			return Err(err!("Hebrew year {} out of range", year; Invalid, Input));
		}
		
		let months_in_year = Self::months_in_hebrew_year(year);
		if month < 1 || month > months_in_year {
			return Err(err!("Hebrew month {} invalid for year {}", month, year; Invalid, Input));
		}
		
		let days_in_month = res!(Self::days_in_hebrew_month(year, month));
		if day < 1 || day > days_in_month {
			return Err(err!("Hebrew day {} invalid for month {}/{}", day, month, year; Invalid, Input));
		}
		
		Ok(())
	}
}

impl HebrewCalendar {
	/// Returns the name of this calendar.
	pub fn calendar_name(&self) -> &'static str {
		"Hebrew"
	}
	
	/// Checks if a year is a leap year in the context of a CalendarDate.
	pub fn is_leap_year_for_date(&self, date: &CalendarDate) -> bool {
		// Convert to Hebrew year first
		match Self::from_gregorian(date.year(), date.month_of_year().of(), date.day()) {
			Ok((hebrew_year, _, _)) => Self::is_hebrew_leap_year(hebrew_year),
			Err(_) => false,
		}
	}
	
	/// Returns days in month for a CalendarDate.
	pub fn days_in_month_for_date(&self, date: &CalendarDate) -> Outcome<u8> {
		// Convert to Hebrew date first
		let (hebrew_year, hebrew_month, _) = res!(Self::from_gregorian(
			date.year(), 
			date.month_of_year().of(), 
			date.day()
		));
		
		Self::days_in_hebrew_month(hebrew_year, hebrew_month)
	}
	
	/// Returns months in year for a CalendarDate.
	pub fn months_in_year_for_date(&self, date: &CalendarDate) -> u8 {
		// Convert to Hebrew year first
		match Self::from_gregorian(date.year(), date.month_of_year().of(), date.day()) {
			Ok((hebrew_year, _, _)) => Self::months_in_hebrew_year(hebrew_year),
			Err(_) => 12, // Default
		}
	}
	
	/// Returns days in year for a CalendarDate.
	pub fn days_in_year_for_date(&self, date: &CalendarDate) -> u16 {
		// Convert to Hebrew year first
		match Self::from_gregorian(date.year(), date.month_of_year().of(), date.day()) {
			Ok((hebrew_year, _, _)) => Self::days_in_hebrew_year(hebrew_year) as u16,
			Err(_) => 365, // Default
		}
	}
	
	/// Validates a Gregorian date for Hebrew calendar conversion.
	pub fn validate_gregorian_date(&self, year: i32, month: u8, day: u8) -> Outcome<()> {
		// This validates Gregorian dates that will be converted to Hebrew
		if year < 1 || year > 9999 {
			return Err(err!("Year {} out of range", year; Invalid, Input));
		}
		
		// Basic Gregorian validation
		if month < 1 || month > 12 {
			return Err(err!("Month {} out of range", month; Invalid, Input));
		}
		
		let days_in_month = Self::gregorian_month_days(year, month);
		if day < 1 || day > days_in_month {
			return Err(err!("Day {} out of range for month {}", day, month; Invalid, Input));
		}
		
		Ok(())
	}
	
	/// Formats a CalendarDate as a Hebrew date.
	pub fn format_calendar_date(&self, date: &CalendarDate) -> String {
		// Convert to Hebrew date and format
		match Self::from_gregorian(date.year(), date.month_of_year().of(), date.day()) {
			Ok((hebrew_year, hebrew_month, hebrew_day)) => {
				let month_enum = HebrewMonth::from_number(hebrew_month, Self::is_hebrew_leap_year(hebrew_year))
					.unwrap_or(HebrewMonth::Tishrei);
				format!("{} {}, {}", month_enum.name(), hebrew_day, hebrew_year)
			},
			Err(_) => format!("Invalid Hebrew date"),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	
	#[test]
	fn test_hebrew_leap_years() {
		// Known Hebrew leap years
		assert!(HebrewCalendar::is_hebrew_leap_year(5784)); // 2023-2024
		assert!(HebrewCalendar::is_hebrew_leap_year(5787)); // 2026-2027
		assert!(HebrewCalendar::is_hebrew_leap_year(5790)); // 2029-2030
		
		// Known non-leap years
		assert!(!HebrewCalendar::is_hebrew_leap_year(5783)); // 2022-2023
		assert!(!HebrewCalendar::is_hebrew_leap_year(5785)); // 2024-2025
		assert!(!HebrewCalendar::is_hebrew_leap_year(5786)); // 2025-2026
	}
	
	#[test]
	fn test_hebrew_month_days() {
		// Regular year
		assert_eq!(HebrewCalendar::days_in_hebrew_month(5783, 1).unwrap(), 30); // Tishrei
		assert_eq!(HebrewCalendar::days_in_hebrew_month(5783, 6).unwrap(), 29); // Adar
		
		// Leap year
		assert_eq!(HebrewCalendar::days_in_hebrew_month(5784, 6).unwrap(), 30); // Adar I
		assert_eq!(HebrewCalendar::days_in_hebrew_month(5784, 7).unwrap(), 29); // Adar II
	}
	
	#[test]
	fn test_gregorian_to_hebrew_conversion() {
		// Test known date conversions
		// September 16, 2023 = 1 Tishrei 5784 (Rosh Hashanah)
		let (year, month, day) = HebrewCalendar::from_gregorian(2023, 9, 16).unwrap();
		assert_eq!(year, 5784);
		assert_eq!(month, 1); // Tishrei
		assert_eq!(day, 1);
		
		// December 8, 2023 = 25 Kislev 5784 (Chanukah)
		let (year, month, day) = HebrewCalendar::from_gregorian(2023, 12, 8).unwrap();
		assert_eq!(year, 5784);
		assert_eq!(month, 3); // Kislev
		assert_eq!(day, 25);
	}
	
	#[test]
	fn test_hebrew_to_gregorian_conversion() {
		// Test inverse conversions
		// 1 Tishrei 5784 = September 16, 2023
		let (year, month, day) = HebrewCalendar::to_gregorian(5784, 1, 1).unwrap();
		assert_eq!(year, 2023);
		assert_eq!(month, 9);
		assert_eq!(day, 16);
		
		// 15 Nisan 5784 = April 23, 2024 (Passover)
		let (year, month, day) = HebrewCalendar::to_gregorian(5784, 8, 15).unwrap();
		assert_eq!(year, 2024);
		assert_eq!(month, 4);
		assert_eq!(day, 23);
	}
	
	#[test]
	fn test_round_trip_conversions() {
		// Test that conversions are reversible
		let test_dates = vec![
			(2023, 1, 1),
			(2023, 9, 16),
			(2024, 4, 23),
			(2025, 12, 31),
		];
		
		for (g_year, g_month, g_day) in test_dates {
			let (h_year, h_month, h_day) = HebrewCalendar::from_gregorian(g_year, g_month, g_day).unwrap();
			let (g_year2, g_month2, g_day2) = HebrewCalendar::to_gregorian(h_year, h_month, h_day).unwrap();
			
			assert_eq!(g_year, g_year2);
			assert_eq!(g_month, g_month2);
			assert_eq!(g_day, g_day2);
		}
	}
	
	#[test]
	fn test_hebrew_calendar_validation() {
		// Valid dates
		assert!(HebrewCalendar::validate_hebrew_date(5784, 1, 1).is_ok());
		assert!(HebrewCalendar::validate_hebrew_date(5784, 13, 29).is_ok()); // Leap year has 13 months
		
		// Invalid dates
		assert!(HebrewCalendar::validate_hebrew_date(5783, 13, 1).is_err()); // Non-leap year
		assert!(HebrewCalendar::validate_hebrew_date(5784, 1, 31).is_err()); // Tishrei has 30 days
		assert!(HebrewCalendar::validate_hebrew_date(0, 1, 1).is_err()); // Invalid year
	}
	
	#[test]
	fn test_hebrew_month_names() {
		assert_eq!(HebrewMonth::Tishrei.name(), "Tishrei");
		assert_eq!(HebrewMonth::AdarI.name(), "Adar I");
		assert_eq!(HebrewMonth::Elul.name(), "Elul");
	}
	
	#[test]
	fn test_calendar_interface() {
		let calendar = HebrewCalendar::new();
		let zone = crate::time::CalClockZone::utc();
		
		// Create a Gregorian date that we'll treat as Hebrew
		let date = CalendarDate::new(2023, 9, 16, zone).unwrap();
		
		// Test Calendar trait methods
		assert_eq!(calendar.calendar_name(), "Hebrew");
		assert!(calendar.is_leap_year_for_date(&date)); // 5784 is a leap year
		assert_eq!(calendar.months_in_year_for_date(&date), 13);
		
		// Test formatting
		let formatted = calendar.format_calendar_date(&date);
		assert!(formatted.contains("Tishrei"));
		assert!(formatted.contains("5784"));
	}
}