use oxedyne_fe2o3_datime::{
	time::{CalClock, CalClockZone},
	clock::ClockTime,
	calendar::CalendarDate,
	format::{Rfc9557Format, Rfc9557Config, PrecisionLevel},
};

use oxedyne_fe2o3_core::prelude::*;

#[test]
fn test_rfc9557_basic_format() -> Outcome<()> {
	let zone = CalClockZone::utc();
	let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
	
	// Basic RFC 3339 format (no timezone name).
	let basic = res!(calclock.to_rfc9557_basic());
	assert!(basic.starts_with("2024-06-15T14:30:45"));
	assert!(basic.ends_with("Z"));
	assert!(!basic.contains('['));
	
	println!("RFC 9557 Basic: {}", basic);
	Ok(())
}

#[test]
fn test_rfc9557_extended_format() -> Outcome<()> {
	let zone = res!(CalClockZone::new("America/New_York"));
	let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
	
	// Extended RFC 9557 format (with timezone name).
	let extended = res!(calclock.to_rfc9557_extended());
	assert!(extended.starts_with("2024-06-15T14:30:45"));
	assert!(extended.contains("[America/New_York]"));
	
	println!("RFC 9557 Extended: {}", extended);
	Ok(())
}

#[test]
fn test_rfc9557_precision_indicators() -> Outcome<()> {
	let zone = CalClockZone::utc();
	let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 0, zone));
	
	// Test different precision levels.
	let config_approximate = Rfc9557Config {
		include_precision_indicators: true,
		precision_level: PrecisionLevel::Approximate,
		..Default::default()
	};
	
	let approximate = res!(calclock.to_rfc9557_with_config(&config_approximate));
	assert!(approximate.contains('~'));
	println!("Approximate time: {}", approximate);
	
	let config_uncertain = Rfc9557Config {
		include_precision_indicators: true,
		precision_level: PrecisionLevel::Uncertain,
		..Default::default()
	};
	
	let uncertain = res!(calclock.to_rfc9557_with_config(&config_uncertain));
	assert!(uncertain.contains('%'));
	println!("Uncertain time: {}", uncertain);
	
	Ok(())
}

#[test]
fn test_rfc9557_nanosecond_handling() -> Outcome<()> {
	let zone = CalClockZone::utc();
	
	// Test with zero nanoseconds.
	let calclock_no_nanos = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 0, zone.clone()));
	let basic_no_nanos = res!(calclock_no_nanos.to_rfc9557_basic());
	assert!(!basic_no_nanos.contains('.'));
	
	// Test with nanoseconds.
	let calclock_with_nanos = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone.clone()));
	let basic_with_nanos = res!(calclock_with_nanos.to_rfc9557_basic());
	assert!(basic_with_nanos.contains(".123456789"));
	
	// Test forcing nanoseconds even when zero.
	let config_force_nanos = Rfc9557Config {
		always_include_nanoseconds: true,
		..Default::default()
	};
	let forced_nanos = res!(calclock_no_nanos.to_rfc9557_with_config(&config_force_nanos));
	assert!(forced_nanos.contains(".000000000"));
	
	println!("No nanos: {}", basic_no_nanos);
	println!("With nanos: {}", basic_with_nanos);
	println!("Forced nanos: {}", forced_nanos);
	
	Ok(())
}

#[test]
fn test_rfc9557_timezone_offset_formats() -> Outcome<()> {
	// Test UTC representation.
	let utc_zone = CalClockZone::utc();
	let utc_calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, utc_zone));
	let utc_format = res!(utc_calclock.to_rfc9557_basic());
	assert!(utc_format.ends_with('Z'));
	
	// Test with use_z_for_utc disabled.
	let config_no_z = Rfc9557Config {
		use_z_for_utc: false,
		..Default::default()
	};
	let utc_no_z = res!(utc_calclock.to_rfc9557_with_config(&config_no_z));
	assert!(utc_no_z.contains("+00:00"));
	
	println!("UTC with Z: {}", utc_format);
	println!("UTC without Z: {}", utc_no_z);
	
	Ok(())
}

#[test]
fn test_rfc9557_parsing_basic() -> Outcome<()> {
	// Test parsing basic RFC 3339 format.
	let input = "2024-06-15T14:30:45Z";
	let parsed = res!(CalClock::from_rfc9557(input));
	
	assert_eq!(parsed.year(), 2024);
	assert_eq!(parsed.month(), 6);
	assert_eq!(parsed.day(), 15);
	assert_eq!(parsed.hour(), 14);
	assert_eq!(parsed.minute(), 30);
	assert_eq!(parsed.second(), 45);
	assert_eq!(parsed.zone().id(), "UTC");
	
	println!("Parsed basic: {:?}", parsed);
	Ok(())
}

#[test]
fn test_rfc9557_parsing_extended() -> Outcome<()> {
	// Test parsing extended format with timezone.
	let input = "2024-06-15T14:30:45.123456789-04:00[America/New_York]";
	let parsed = res!(CalClock::from_rfc9557(input));
	
	assert_eq!(parsed.year(), 2024);
	assert_eq!(parsed.month(), 6);
	assert_eq!(parsed.day(), 15);
	assert_eq!(parsed.hour(), 14);
	assert_eq!(parsed.minute(), 30);
	assert_eq!(parsed.second(), 45);
	assert_eq!(parsed.nanosecond(), 123_456_789);
	assert_eq!(parsed.zone().id(), "America/New_York");
	
	println!("Parsed extended: {:?}", parsed);
	Ok(())
}

#[test]
fn test_rfc9557_parsing_with_precision() -> Outcome<()> {
	// Test parsing with precision indicators.
	let inputs = [
		"2024-06-15T14:30:45~[UTC]",
		"2024-06-15T14:30:45%[UTC]",
		"2024-06-15T14:30:45@[UTC]",
		"2024-06-15T14:30:45*[UTC]",
	];
	
	for input in &inputs {
		let parsed = res!(CalClock::from_rfc9557(input));
		assert_eq!(parsed.year(), 2024);
		assert_eq!(parsed.zone().id(), "UTC");
		println!("Parsed with precision: {} -> {:?}", input, parsed);
	}
	
	Ok(())
}

#[test]
fn test_rfc9557_round_trip() -> Outcome<()> {
	let zone = res!(CalClockZone::new("Europe/London"));
	let original = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
	
	// Test round-trip with extended format.
	let serialised = res!(original.to_rfc9557_extended());
	let parsed = res!(CalClock::from_rfc9557(&serialised));
	
	// Should preserve the same instant in time.
	let original_utc = res!(original.to_millis());
	let parsed_utc = res!(parsed.to_millis());
	assert_eq!(original_utc, parsed_utc);
	
	// Should preserve timezone.
	assert_eq!(original.zone().id(), parsed.zone().id());
	
	println!("Original: {}", res!(original.to_rfc9557_extended()));
	println!("Serialised: {}", serialised);
	println!("Parsed: {}", res!(parsed.to_rfc9557_extended()));
	
	Ok(())
}

#[test]
fn test_rfc9557_time_only() -> Outcome<()> {
	let zone = res!(CalClockZone::new("Asia/Tokyo"));
	let time = res!(ClockTime::new(14, 30, 45, 123_456_789, zone));
	
	// Test time-only formatting.
	let basic = res!(time.to_rfc9557_basic());
	let extended = res!(time.to_rfc9557_extended());
	
	assert!(basic.starts_with("14:30:45"));
	assert!(!basic.contains('['));
	assert!(extended.contains("[Asia/Tokyo]"));
	
	println!("Time basic: {}", basic);
	println!("Time extended: {}", extended);
	
	// Test parsing.
	let parsed = res!(ClockTime::from_rfc9557(&extended));
	assert_eq!(parsed.hour().of(), 14);
	assert_eq!(parsed.zone().id(), "Asia/Tokyo");
	
	Ok(())
}

#[test]
fn test_rfc9557_date_only() -> Outcome<()> {
	let zone = res!(CalClockZone::new("Australia/Sydney"));
	let date = res!(CalendarDate::new(2024, 6, 15, zone));
	
	// Test date-only formatting.
	let basic = res!(date.to_rfc9557_basic());
	let extended = res!(date.to_rfc9557_extended());
	
	assert_eq!(basic, "2024-06-15");
	assert_eq!(extended, "2024-06-15[Australia/Sydney]");
	
	println!("Date basic: {}", basic);
	println!("Date extended: {}", extended);
	
	// Test parsing.
	let parsed = res!(CalendarDate::from_rfc9557(&extended));
	assert_eq!(parsed.year(), 2024);
	assert_eq!(parsed.month(), 6);
	assert_eq!(parsed.day(), 15);
	assert_eq!(parsed.zone().id(), "Australia/Sydney");
	
	Ok(())
}

#[test]
fn test_rfc9557_utility_functions() -> Outcome<()> {
	use oxedyne_fe2o3_datime::format::rfc9557::utils;
	
	// Test validation.
	let valid_timestamp = "2024-06-15T14:30:45Z[UTC]";
	assert!(utils::validate_rfc9557(valid_timestamp).is_ok());
	
	let invalid_timestamp = "not-a-timestamp";
	assert!(utils::validate_rfc9557(invalid_timestamp).is_err());
	
	// Test timezone extraction.
	assert_eq!(utils::extract_timezone_name(valid_timestamp), Some("UTC"));
	assert_eq!(utils::extract_timezone_name("2024-06-15T14:30:45Z"), None);
	
	// Test precision extraction.
	assert_eq!(utils::extract_precision_indicator("2024-06-15T14:30:45~"), Some(PrecisionLevel::Approximate));
	assert_eq!(utils::extract_precision_indicator("2024-06-15T14:30:45"), None);
	
	println!("✓ Utility functions work correctly");
	Ok(())
}

#[test]
fn test_rfc9557_complex_scenarios() -> Outcome<()> {
	// Test with different timezone formats.
	let scenarios = [
		("UTC", "2024-06-15T14:30:45Z[UTC]"),
		("America/New_York", "2024-06-15T14:30:45-04:00[America/New_York]"), // Summer time.
		("Europe/Paris", "2024-06-15T14:30:45+02:00[Europe/Paris]"), // Summer time.
		("Asia/Tokyo", "2024-06-15T14:30:45+09:00[Asia/Tokyo]"),
	];
	
	for (zone_name, expected_pattern) in &scenarios {
		let zone = res!(CalClockZone::new(*zone_name));
		let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 0, zone));
		let formatted = res!(calclock.to_rfc9557_extended());
		
		assert!(formatted.starts_with("2024-06-15T14:30:45"));
		assert!(formatted.contains(&format!("[{}]", zone_name)));
		
		// Test parsing back.
		let parsed = res!(CalClock::from_rfc9557(&formatted));
		assert_eq!(parsed.zone().id(), *zone_name);
		
		println!("Zone {}: {}", zone_name, formatted);
	}
	
	Ok(())
}

pub fn test_rfc9557_integration(filter: &str) -> Outcome<()> {
	println!("=== RFC 9557 Timezone-Preserving Serialisation Demo ===");
	
	res!(test_it(filter, &["basic", "all", "rfc9557"], || {
		test_rfc9557_basic_format()
	}));
	
	res!(test_it(filter, &["extended", "all", "rfc9557"], || {
		test_rfc9557_extended_format()
	}));
	
	res!(test_it(filter, &["precision", "all", "rfc9557"], || {
		test_rfc9557_precision_indicators()
	}));
	
	res!(test_it(filter, &["nanoseconds", "all", "rfc9557"], || {
		test_rfc9557_nanosecond_handling()
	}));
	
	res!(test_it(filter, &["offsets", "all", "rfc9557"], || {
		test_rfc9557_timezone_offset_formats()
	}));
	
	res!(test_it(filter, &["parsing_basic", "all", "rfc9557"], || {
		test_rfc9557_parsing_basic()
	}));
	
	res!(test_it(filter, &["parsing_extended", "all", "rfc9557"], || {
		test_rfc9557_parsing_extended()
	}));
	
	res!(test_it(filter, &["parsing_precision", "all", "rfc9557"], || {
		test_rfc9557_parsing_with_precision()
	}));
	
	res!(test_it(filter, &["round_trip", "all", "rfc9557"], || {
		test_rfc9557_round_trip()
	}));
	
	res!(test_it(filter, &["time_only", "all", "rfc9557"], || {
		test_rfc9557_time_only()
	}));
	
	res!(test_it(filter, &["date_only", "all", "rfc9557"], || {
		test_rfc9557_date_only()
	}));
	
	res!(test_it(filter, &["utilities", "all", "rfc9557"], || {
		test_rfc9557_utility_functions()
	}));
	
	res!(test_it(filter, &["complex", "all", "rfc9557"], || {
		test_rfc9557_complex_scenarios()
	}));
	
	println!("✓ All RFC 9557 tests passed!");
	Ok(())
}

fn test_it<F>(filter: &str, keywords: &[&str], test_fn: F) -> Outcome<()>
where
	F: FnOnce() -> Outcome<()>,
{
	if keywords.iter().any(|&kw| filter.contains(kw)) {
		test_fn()
	} else {
		Ok(())
	}
}