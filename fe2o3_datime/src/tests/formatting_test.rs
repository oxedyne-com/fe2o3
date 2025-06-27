use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClockZone,
    core::Time,
};

#[test]
fn test_date_formatting_comprehensive() {
    let zone = CalClockZone::utc();
    let date = CalendarDate::new(2024, 6, 15, zone).unwrap(); // Saturday
    
    // Year formatting
    assert_eq!(date.format("y"), "4");
    assert_eq!(date.format("yy"), "24");
    assert_eq!(date.format("yyyy"), "2024");
    
    // Month formatting
    assert_eq!(date.format("M"), "6");
    assert_eq!(date.format("MM"), "06");
    assert_eq!(date.format("MMM"), "Jun");
    assert_eq!(date.format("MMMM"), "June");
    
    // Day formatting
    assert_eq!(date.format("d"), "15");
    assert_eq!(date.format("dd"), "15");
    assert_eq!(date.format("ddd"), "Sat");
    assert_eq!(date.format("dddd"), "Saturday");
    
    // Combined formats
    assert_eq!(date.format("yyyy-MM-dd"), "2024-06-15");
    assert_eq!(date.format("dd/MM/yyyy"), "15/06/2024");
    assert_eq!(date.format("MMM d, yyyy"), "Jun 15, 2024");
    assert_eq!(date.format("dddd, MMMM d, yyyy"), "Saturday, June 15, 2024");
}

#[test]
fn test_time_formatting_comprehensive() {
    let zone = CalClockZone::utc();
    let time = ClockTime::new(14, 30, 45, 123_456_789, zone.clone()).unwrap();
    
    // 24-hour formatting
    assert_eq!(time.format("H"), "14");
    assert_eq!(time.format("HH"), "14");
    assert_eq!(time.format("HH:mm"), "14:30");
    assert_eq!(time.format("HH:mm:ss"), "14:30:45");
    
    // 12-hour formatting
    assert_eq!(time.format("h"), "2");
    assert_eq!(time.format("hh"), "02");
    assert_eq!(time.format("h:mm tt"), "2:30 PM");
    assert_eq!(time.format("hh:mm:ss tt"), "02:30:45 PM");
    
    // Fractional seconds
    assert_eq!(time.format("HH:mm:ss.f"), "14:30:45.1");
    assert_eq!(time.format("HH:mm:ss.fff"), "14:30:45.123");
    assert_eq!(time.format("HH:mm:ss.ffffff"), "14:30:45.123456");
    
    // Test zero suppression with F
    let time_no_frac = ClockTime::new(14, 30, 45, 0, zone.clone()).unwrap();
    assert_eq!(time_no_frac.format("HH:mm:ss.FFF"), "14:30:45.0");
    
    // AM/PM tests
    let morning = ClockTime::new(9, 15, 30, 0, zone.clone()).unwrap();
    assert_eq!(morning.format("h:mm tt"), "9:15 AM");
    assert_eq!(morning.format("h:mm t"), "9:15 A");
    
    let midnight = ClockTime::new(0, 0, 0, 0, zone.clone()).unwrap();
    assert_eq!(midnight.format("h:mm tt"), "12:00 AM");
}

#[test]
fn test_today_returns_current_date() {
    let zone = CalClockZone::utc();
    let today = CalendarDate::today(zone).unwrap();
    
    // Just verify it's not the dummy date (2024-01-01)
    let dummy_date = CalendarDate::new(2024, 1, 1, CalClockZone::utc()).unwrap();
    
    // The chance of today being exactly 2024-01-01 is extremely low
    // (this test will fail only on Jan 1, 2024 - acceptable for verification)
    assert_ne!(today, dummy_date, "today() should return actual current date, not dummy date");
    
    println!("Today is: {}", today.format("dddd, MMMM d, yyyy"));
}