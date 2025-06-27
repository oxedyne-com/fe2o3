use crate::{
    calendar::CalendarDate,
    time::CalClockZone,
    core::Time,
};

#[test]
fn test_julian_day_roundtrip() {
    let zone = CalClockZone::utc();
    
    // Test some known dates
    let test_dates = [
        (2024, 6, 15),  // Modern date
        (2000, 1, 1),   // Y2K
        (1970, 1, 1),   // Unix epoch
        (1582, 10, 15), // Gregorian calendar start
        (1, 1, 1),      // Year 1
    ];
    
    for (year, month, day) in test_dates {
        let original = CalendarDate::new(year, month, day, zone.clone()).unwrap();
        let julian_day = original.to_julian_day_number();
        let reconstructed = CalendarDate::from_julian_day_number(julian_day, zone.clone()).unwrap();
        
        assert_eq!(original.year(), reconstructed.year());
        assert_eq!(original.month(), reconstructed.month());
        assert_eq!(original.day(), reconstructed.day());
        
        println!("{}-{:02}-{:02} -> Julian Day {} -> {}-{:02}-{:02}", 
                 year, month, day, julian_day, 
                 reconstructed.year(), reconstructed.month(), reconstructed.day());
    }
}

#[test]
fn test_add_days_with_julian_arithmetic() {
    let zone = CalClockZone::utc();
    let start_date = CalendarDate::new(2024, 2, 28, zone.clone()).unwrap(); // Day before leap day
    
    // Test crossing leap day
    let next_day = start_date.add_days(1).unwrap();
    assert_eq!(next_day.year(), 2024);
    assert_eq!(next_day.month(), 2);
    assert_eq!(next_day.day(), 29); // Leap day
    
    let day_after_leap = start_date.add_days(2).unwrap();
    assert_eq!(day_after_leap.year(), 2024);
    assert_eq!(day_after_leap.month(), 3);
    assert_eq!(day_after_leap.day(), 1); // March 1st
    
    // Test subtracting days
    let earlier = start_date.add_days(-28).unwrap();
    assert_eq!(earlier.year(), 2024);
    assert_eq!(earlier.month(), 1);
    assert_eq!(earlier.day(), 31); // January 31st
    
    println!("Start: {}", start_date.format("yyyy-MM-dd"));
    println!("Next day: {}", next_day.format("yyyy-MM-dd"));
    println!("Day after leap: {}", day_after_leap.format("yyyy-MM-dd"));
    println!("Earlier: {}", earlier.format("yyyy-MM-dd"));
}

#[test]
fn test_unix_epoch_conversion() {
    let zone = CalClockZone::utc();
    let unix_epoch = CalendarDate::new(1970, 1, 1, zone.clone()).unwrap();
    
    // Unix epoch should be 0 days since epoch
    assert_eq!(unix_epoch.days_since_epoch().unwrap(), 0);
    
    // Test creating from 0 days since epoch
    let reconstructed = CalendarDate::from_days_since_epoch(0, zone.clone()).unwrap();
    assert_eq!(reconstructed.year(), 1970);
    assert_eq!(reconstructed.month(), 1);
    assert_eq!(reconstructed.day(), 1);
    
    // Test a few days after epoch
    let few_days_later = CalendarDate::from_days_since_epoch(100, zone.clone()).unwrap();
    assert_eq!(few_days_later.days_since_epoch().unwrap(), 100);
    
    println!("Unix epoch: {}", unix_epoch.format("yyyy-MM-dd"));
    println!("100 days later: {}", few_days_later.format("yyyy-MM-dd"));
}