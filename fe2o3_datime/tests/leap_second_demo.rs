use oxedyne_fe2o3_datime::{
    time::{
        CalClock,
        CalClockZone,
        LeapSecondTable,
        LeapSecondConfig,
        LeapSecondEntry,
        LeapSecondStatistics,
    },
    clock::ClockTime,
};

use oxedyne_fe2o3_core::prelude::*;

#[test]
fn test_leap_second_table_creation() -> Outcome<()> {
    // Test standard leap second table
    let table = LeapSecondTable::standard();
    assert!(table.is_enabled());
    assert!(table.leap_second_count() > 0);
    
    // Test disabled table
    let disabled = LeapSecondTable::disabled();
    assert!(!disabled.is_enabled());
    assert_eq!(disabled.leap_second_count(), 0);
    
    Ok(())
}

#[test]
fn test_tai_utc_offset_calculation() -> Outcome<()> {
    let table = LeapSecondTable::standard();
    
    // Before first leap second (1972-07-01)
    assert_eq!(table.tai_utc_offset_at(0), 10);
    
    // First leap second: 1972-07-01 00:00:00 UTC = 78796800 seconds since epoch
    assert_eq!(table.tai_utc_offset_at(78796800), 10);
    assert_eq!(table.tai_utc_offset_at(78796801), 10);
    
    // Most recent leap second: 2017-01-01 00:00:00 UTC = 1483228800 seconds
    assert_eq!(table.tai_utc_offset_at(1483228800), 36);
    
    // Current time (should still be 36 as no leap seconds since 2017)
    assert_eq!(table.tai_utc_offset_at(1600000000), 36); // 2020
    
    Ok(())
}

#[test]
fn test_utc_tai_conversion() -> Outcome<()> {
    let table = LeapSecondTable::standard();
    
    // Test conversion for 2020-01-01 00:00:00 UTC
    let utc_timestamp = 1577836800; // 2020-01-01 00:00:00 UTC
    let tai_timestamp = table.utc_to_tai(utc_timestamp);
    assert_eq!(tai_timestamp, utc_timestamp + 37); // Should be 37 seconds ahead
    
    // Round trip conversion
    let converted_utc = res!(table.tai_to_utc(tai_timestamp));
    assert_eq!(converted_utc, utc_timestamp);
    
    Ok(())
}

#[test]
fn test_leap_second_detection() -> Outcome<()> {
    let table = LeapSecondTable::standard();
    
    // Test known leap second: 2017-01-01 00:00:00 UTC
    assert!(table.is_leap_second(1483228800));
    
    // Test non-leap second
    assert!(!table.is_leap_second(1483228801));
    
    // Test leap second validation for specific dates
    assert!(table.validate_leap_second(2017, 1, 1, 23, 59)); // Valid leap second
    assert!(!table.validate_leap_second(2017, 1, 2, 23, 59)); // Not a leap second date
    assert!(!table.validate_leap_second(2017, 1, 1, 12, 0)); // Wrong time
    
    Ok(())
}

#[test]
fn test_leap_second_configuration() -> Outcome<()> {
    // Test default configuration
    let config = LeapSecondConfig::default();
    assert!(config.enabled);
    assert!(config.allow_leap_second_parsing);
    assert!(config.validate_leap_seconds);
    
    // Test disabled configuration
    let disabled = LeapSecondConfig::disabled();
    assert!(!disabled.enabled);
    assert!(!disabled.allow_leap_second_parsing);
    assert!(!disabled.validate_leap_seconds);
    
    // Test permissive configuration
    let permissive = LeapSecondConfig::permissive();
    assert!(permissive.enabled);
    assert!(permissive.allow_leap_second_parsing);
    assert!(!permissive.validate_leap_seconds);
    
    Ok(())
}

#[test]
fn test_clock_time_with_leap_seconds() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let config = LeapSecondConfig::default();
    
    // Test creating normal time
    let normal_time = res!(ClockTime::new_with_leap_seconds(23, 59, 59, 0, zone.clone(), &config));
    assert!(!normal_time.is_leap_second());
    
    // Test creating leap second time with validation disabled
    let permissive_config = LeapSecondConfig::permissive();
    let leap_time = res!(ClockTime::new_with_leap_seconds(23, 59, 60, 0, zone.clone(), &permissive_config));
    assert!(leap_time.is_leap_second());
    assert!(leap_time.is_potential_leap_second());
    
    // Test that invalid leap second time fails with strict validation
    let strict_config = LeapSecondConfig::default();
    let result = ClockTime::new_with_leap_seconds(12, 30, 60, 0, zone.clone(), &strict_config);
    assert!(result.is_err()); // Should fail because 12:30:60 is not valid leap second time
    
    Ok(())
}

#[test]
fn test_calclock_with_leap_seconds() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let permissive_config = LeapSecondConfig::permissive();
    
    // Test creating CalClock with leap second (validation disabled)
    let leap_calclock = res!(CalClock::new_with_leap_seconds(
        2017, 1, 1, 23, 59, 60, 0, zone.clone(), &permissive_config
    ));
    
    assert!(leap_calclock.is_leap_second());
    assert!(leap_calclock.is_potential_leap_second());
    assert_eq!(leap_calclock.year(), 2017);
    assert_eq!(leap_calclock.month(), 1);
    assert_eq!(leap_calclock.day(), 1);
    assert_eq!(leap_calclock.hour(), 23);
    assert_eq!(leap_calclock.minute(), 59);
    assert_eq!(leap_calclock.second(), 60);
    
    // Test leap second validation
    let strict_config = LeapSecondConfig::default();
    assert!(leap_calclock.validate_leap_second(&permissive_config));
    // Note: strict validation would require the actual leap second table to contain this date
    
    Ok(())
}

#[test]
fn test_leap_second_normalization() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let config = LeapSecondConfig::permissive();
    
    // Create a leap second CalClock
    let leap_calclock = res!(CalClock::new_with_leap_seconds(
        2017, 1, 1, 23, 59, 60, 500_000_000, zone.clone(), &config
    ));
    
    // Normalize the leap second
    let (normalized, day_advanced) = res!(leap_calclock.normalize_leap_second());
    
    assert!(day_advanced);
    assert_eq!(normalized.year(), 2017);
    assert_eq!(normalized.month(), 1);
    assert_eq!(normalized.day(), 2); // Should advance to next day
    assert_eq!(normalized.hour(), 0);
    assert_eq!(normalized.minute(), 0);
    assert_eq!(normalized.second(), 0);
    assert_eq!(normalized.nanosecond(), 500_000_000); // Nanoseconds preserved
    
    // Test normalizing non-leap second (should be unchanged)
    let normal_calclock = res!(CalClock::new(2017, 1, 1, 12, 30, 45, 0, zone));
    let (normalized_normal, advanced) = res!(normal_calclock.normalize_leap_second());
    assert!(!advanced);
    assert_eq!(normalized_normal, normal_calclock);
    
    Ok(())
}

#[test]
fn test_tai_utc_conversions() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let config = LeapSecondConfig::default();
    
    // Create a CalClock for testing
    let calclock = res!(CalClock::new(2020, 1, 1, 12, 0, 0, 0, zone.clone()));
    
    // Convert to TAI timestamp
    let tai_timestamp = res!(calclock.to_tai_timestamp(&config));
    
    // Convert back from TAI timestamp
    let converted_back = res!(CalClock::from_tai_timestamp(tai_timestamp, zone, &config));
    
    // Should be approximately equal (might differ slightly due to timezone calculations)
    assert_eq!(converted_back.year(), calclock.year());
    assert_eq!(converted_back.month(), calclock.month());
    assert_eq!(converted_back.day(), calclock.day());
    assert_eq!(converted_back.hour(), calclock.hour());
    assert_eq!(converted_back.minute(), calclock.minute());
    assert_eq!(converted_back.second(), calclock.second());
    
    Ok(())
}

#[test]
fn test_leap_second_statistics() -> Outcome<()> {
    let table = LeapSecondTable::standard();
    let stats = table.statistics();
    
    assert!(stats.total_leap_seconds > 0);
    assert!(stats.enabled);
    assert!(stats.first_leap_second.is_some());
    assert!(stats.latest_leap_second.is_some());
    assert_eq!(stats.current_tai_utc_offset, 36); // As of 2017
    
    // First leap second should be 1972-07-01
    assert_eq!(stats.first_leap_second.unwrap(), 78796800);
    
    // Latest leap second should be 2017-01-01
    assert_eq!(stats.latest_leap_second.unwrap(), 1483228800);
    
    Ok(())
}

#[test]
fn test_leap_second_boundary_cases() -> Outcome<()> {
    let table = LeapSecondTable::standard();
    
    // Test edge cases around leap second boundaries
    let leap_second_time = 1483228800; // 2017-01-01 00:00:00 UTC (leap second)
    
    // Test times around the leap second
    assert_eq!(table.tai_utc_offset_at(leap_second_time - 1), 35); // Before leap second
    assert_eq!(table.tai_utc_offset_at(leap_second_time), 36);     // At leap second
    assert_eq!(table.tai_utc_offset_at(leap_second_time + 1), 36); // After leap second
    
    // Test TAI conversion around leap second
    let tai_before = table.utc_to_tai(leap_second_time - 1);
    let tai_at = table.utc_to_tai(leap_second_time);
    let tai_after = table.utc_to_tai(leap_second_time + 1);
    
    // TAI should be continuous despite leap second
    assert_eq!(tai_at - tai_before, 2); // Gap due to leap second
    assert_eq!(tai_after - tai_at, 1);  // Normal progression
    
    Ok(())
}

#[test]
fn test_custom_leap_second_table() -> Outcome<()> {
    // Create custom leap second table
    let mut custom_table = LeapSecondTable::new();
    custom_table.add_entry(946684800, 32, "2000-01-01: Custom leap second"); // Y2K
    custom_table.add_entry(978307200, 33, "2001-01-01: Another custom leap second");
    
    assert_eq!(custom_table.leap_second_count(), 2);
    assert_eq!(custom_table.tai_utc_offset_at(946684800), 32);
    assert_eq!(custom_table.tai_utc_offset_at(978307200), 33);
    assert_eq!(custom_table.tai_utc_offset_at(1000000000), 33); // After last leap second
    
    // Test custom configuration with custom table
    let custom_config = LeapSecondConfig {
        enabled: true,
        allow_leap_second_parsing: true,
        validate_leap_seconds: false,
        custom_table: Some(custom_table),
    };
    
    let table = custom_config.get_table();
    assert_eq!(table.leap_second_count(), 2);
    
    Ok(())
}

#[test]
fn test_disabled_leap_second_handling() -> Outcome<()> {
    let disabled_config = LeapSecondConfig::disabled();
    let zone = CalClockZone::utc();
    
    // Should not allow leap second parsing when disabled
    let result = ClockTime::new_with_leap_seconds(23, 59, 60, 0, zone.clone(), &disabled_config);
    assert!(result.is_err());
    
    // TAI-UTC conversion should return 0 offset when disabled
    let table = disabled_config.get_table();
    assert_eq!(table.tai_utc_offset_at(1600000000), 0);
    assert_eq!(table.utc_to_tai(1600000000), 1600000000); // No offset
    
    Ok(())
}

pub fn test_leap_second_support(filter: &str) -> Outcome<()> {
    println!("=== Leap Second Support Demo ===");
    
    res!(test_it(filter, &["leap_second_table", "all", "leap", "table"], || {
        test_leap_second_table_creation()
    }));
    
    res!(test_it(filter, &["tai_utc_offset", "all", "leap", "offset"], || {
        test_tai_utc_offset_calculation()
    }));
    
    res!(test_it(filter, &["utc_tai_conversion", "all", "leap", "conversion"], || {
        test_utc_tai_conversion()
    }));
    
    res!(test_it(filter, &["leap_second_detection", "all", "leap", "detection"], || {
        test_leap_second_detection()
    }));
    
    res!(test_it(filter, &["leap_second_config", "all", "leap", "config"], || {
        test_leap_second_configuration()
    }));
    
    res!(test_it(filter, &["clock_time_leap", "all", "leap", "time"], || {
        test_clock_time_with_leap_seconds()
    }));
    
    res!(test_it(filter, &["calclock_leap", "all", "leap", "calclock"], || {
        test_calclock_with_leap_seconds()
    }));
    
    res!(test_it(filter, &["leap_normalization", "all", "leap", "normalize"], || {
        test_leap_second_normalization()
    }));
    
    res!(test_it(filter, &["tai_conversions", "all", "leap", "tai"], || {
        test_tai_utc_conversions()
    }));
    
    res!(test_it(filter, &["leap_statistics", "all", "leap", "stats"], || {
        test_leap_second_statistics()
    }));
    
    res!(test_it(filter, &["leap_boundaries", "all", "leap", "boundary"], || {
        test_leap_second_boundary_cases()
    }));
    
    res!(test_it(filter, &["custom_leap_table", "all", "leap", "custom"], || {
        test_custom_leap_second_table()
    }));
    
    res!(test_it(filter, &["disabled_leap", "all", "leap", "disabled"], || {
        test_disabled_leap_second_handling()
    }));
    
    println!("✓ All leap second tests passed!");
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