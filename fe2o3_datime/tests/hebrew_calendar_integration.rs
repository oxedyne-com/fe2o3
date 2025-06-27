use oxedyne_fe2o3_datime::{
	calendar::{CalendarDate, hebrew::{HebrewCalendar, HebrewMonth}},
	time::{CalClock, CalClockZone},
};
use oxedyne_fe2o3_core::prelude::*;

#[test]
fn test_hebrew_calendar_with_calendar_date() {
	let zone = CalClockZone::utc();
	let hebrew_cal = HebrewCalendar::new();
	
	// Create a known date: Rosh Hashanah 5784 (September 16, 2023)
	let date = CalendarDate::new(2023, 9, 16, zone).unwrap();
	
	// Test calendar properties
	assert_eq!(hebrew_cal.calendar_name(), "Hebrew");
	assert!(hebrew_cal.is_leap_year_for_date(&date)); // 5784 is a leap year
	assert_eq!(hebrew_cal.months_in_year_for_date(&date), 13);
	
	// Test formatting
	let formatted = hebrew_cal.format_calendar_date(&date);
	assert!(formatted.contains("Tishrei"));
	assert!(formatted.contains("5784"));
}

#[test]
fn test_hebrew_calendar_holidays() {
	// Test major Jewish holidays in 2024 (Hebrew year 5784)
	let test_cases = vec![
		// (Gregorian date, Hebrew date expected, Holiday name)
		((2023, 9, 16), (5784, 1, 1), "Rosh Hashanah"),
		((2023, 9, 25), (5784, 1, 10), "Yom Kippur"),
		((2023, 9, 30), (5784, 1, 15), "Sukkot"),
		((2023, 12, 8), (5784, 3, 25), "Chanukah (First Night)"),
		((2024, 3, 24), (5784, 7, 14), "Purim"),
		((2024, 4, 23), (5784, 8, 15), "Passover (First Day)"),
		((2024, 6, 12), (5784, 10, 6), "Shavuot"),
	];
	
	for ((g_year, g_month, g_day), (h_year, h_month, h_day), holiday) in test_cases {
		let (calc_year, calc_month, calc_day) = 
			HebrewCalendar::from_gregorian(g_year, g_month, g_day).unwrap();
		
		assert_eq!(calc_year, h_year, "Year mismatch for {}", holiday);
		assert_eq!(calc_month, h_month, "Month mismatch for {}", holiday);
		assert_eq!(calc_day, h_day, "Day mismatch for {}", holiday);
	}
}

#[test]
fn test_hebrew_calendar_with_calclock() {
	let zone = CalClockZone::utc();
	
	// Create a CalClock for Rosh Hashanah 5784
	let calclock = CalClock::new(2023, 9, 16, 10, 0, 0, 0, zone).unwrap();
	let date = calclock.date();
	
	let hebrew_cal = HebrewCalendar::new();
	
	// Verify the date converts correctly
	let (h_year, h_month, h_day) = HebrewCalendar::from_gregorian(
		date.year(),
		date.month_of_year().of(),
		date.day()
	).unwrap();
	
	assert_eq!(h_year, 5784);
	assert_eq!(h_month, 1); // Tishrei
	assert_eq!(h_day, 1);
}

#[test]
fn test_hebrew_leap_year_months() {
	// Test Adar I and Adar II in a leap year
	let leap_year = 5784;
	
	// Verify 5784 is a leap year
	assert!(HebrewCalendar::is_hebrew_leap_year(leap_year));
	assert_eq!(HebrewCalendar::months_in_hebrew_year(leap_year), 13);
	
	// Test Adar I (month 6 in leap year)
	assert_eq!(HebrewCalendar::days_in_hebrew_month(leap_year, 6).unwrap(), 30);
	
	// Test Adar II (month 7 in leap year)
	assert_eq!(HebrewCalendar::days_in_hebrew_month(leap_year, 7).unwrap(), 29);
	
	// Test that we can convert dates in both Adar months
	let (g_year1, g_month1, g_day1) = HebrewCalendar::to_gregorian(leap_year, 6, 15).unwrap();
	let (g_year2, g_month2, g_day2) = HebrewCalendar::to_gregorian(leap_year, 7, 15).unwrap();
	
	// Verify they convert to different Gregorian dates
	assert!(g_year1 == g_year2);
	assert!(g_month1 < g_month2 || (g_month1 == g_month2 && g_day1 < g_day2));
}

#[test]
fn test_hebrew_year_lengths() {
	// Hebrew years can be deficient (353/383), regular (354/384), or abundant (355/385) days
	let years_to_test = vec![5780, 5781, 5782, 5783, 5784, 5785];
	
	for year in years_to_test {
		let days = HebrewCalendar::days_in_hebrew_year(year);
		let is_leap = HebrewCalendar::is_hebrew_leap_year(year);
		
		if is_leap {
			// Leap years: 383, 384, or 385 days
			assert!(days >= 383 && days <= 385, 
				"Leap year {} has {} days, expected 383-385", year, days);
		} else {
			// Regular years: 353, 354, or 355 days
			assert!(days >= 353 && days <= 355,
				"Regular year {} has {} days, expected 353-355", year, days);
		}
	}
}

#[test]
fn test_hebrew_month_enum() {
	// Test month number conversions
	assert_eq!(HebrewMonth::Tishrei.number(false), 1);
	assert_eq!(HebrewMonth::Adar.number(false), 6);
	assert_eq!(HebrewMonth::Adar.number(true), 7); // Adar II in leap year
	assert_eq!(HebrewMonth::AdarI.number(true), 6);
	assert_eq!(HebrewMonth::Nisan.number(false), 7);
	assert_eq!(HebrewMonth::Nisan.number(true), 8);
	assert_eq!(HebrewMonth::Elul.number(false), 12);
	assert_eq!(HebrewMonth::Elul.number(true), 13);
	
	// Test invalid case
	assert_eq!(HebrewMonth::AdarI.number(false), 0); // AdarI doesn't exist in non-leap years
}

#[test]
fn test_hebrew_date_edge_cases() {
	// Test very early date
	let (h_year, h_month, h_day) = HebrewCalendar::from_gregorian(1, 1, 1).unwrap();
	assert!(h_year > 0);
	
	// Test date validation
	assert!(HebrewCalendar::validate_hebrew_date(5784, 1, 30).is_ok()); // Valid
	assert!(HebrewCalendar::validate_hebrew_date(5784, 1, 31).is_err()); // Invalid - Tishrei has 30 days
	assert!(HebrewCalendar::validate_hebrew_date(5783, 13, 1).is_err()); // Invalid - non-leap year
	assert!(HebrewCalendar::validate_hebrew_date(10000, 1, 1).is_err()); // Invalid - year too large
}

#[test]
fn test_hebrew_calendar_date_arithmetic() {
	let zone = CalClockZone::utc();
	
	// Start with a known date
	let date = CalendarDate::new(2023, 9, 16, zone).unwrap(); // 1 Tishrei 5784
	
	// Add 29 days to get to 30 Tishrei
	let new_date = date.add_days(29).unwrap();
	let (h_year, h_month, h_day) = HebrewCalendar::from_gregorian(
		new_date.year(),
		new_date.month_of_year().of(),
		new_date.day()
	).unwrap();
	
	assert_eq!(h_year, 5784);
	assert_eq!(h_month, 1); // Still Tishrei
	assert_eq!(h_day, 30);
	
	// Add one more day to get to 1 Cheshvan
	let next_month = new_date.add_days(1).unwrap();
	let (h_year2, h_month2, h_day2) = HebrewCalendar::from_gregorian(
		next_month.year(),
		next_month.month_of_year().of(),
		next_month.day()
	).unwrap();
	
	assert_eq!(h_year2, 5784);
	assert_eq!(h_month2, 2); // Cheshvan
	assert_eq!(h_day2, 1);
}