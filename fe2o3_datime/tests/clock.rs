use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_datime::{
    clock::{
        ClockDuration,
        ClockFields,
        ClockHour,
        ClockTime,
    },
    core::Duration,
    time::CalClockZone,
};

pub fn test_clock(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["clock_time_creation", "all", "clock", "time"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Valid times
        let time1 = res!(ClockTime::new(14, 30, 45, 123_456_789, zone.clone()));
        assert_eq!(time1.hour().of(), 14);
        assert_eq!(time1.minute().of(), 30);
        assert_eq!(time1.second().of(), 45);
        assert_eq!(time1.nanosecond().of(), 123_456_789);
        
        // Special times
        let noon = res!(ClockTime::noon(zone.clone()));
        assert_eq!(noon.hour().of(), 12);
        assert_eq!(noon.minute().of(), 0);
        
        let midnight = res!(ClockTime::midnight(zone.clone()));
        assert_eq!(midnight.hour().of(), 0);
        assert_eq!(midnight.minute().of(), 0);
        
        // Invalid times should fail
        assert!(ClockTime::new(24, 0, 0, 0, zone.clone()).is_err());
        assert!(ClockTime::new(23, 60, 0, 0, zone.clone()).is_err());
        assert!(ClockTime::new(23, 59, 60, 0, zone.clone()).is_err());
        assert!(ClockTime::new(23, 59, 59, 1_000_000_000, zone.clone()).is_err());
        Ok(())
    }));
    
    res!(test_it(filter, &["clock_duration", "all", "clock", "duration"], || {
        // Test duration creation and arithmetic
        let dur1 = ClockDuration::from_seconds(3661); // 1 hour, 1 minute, 1 second
        assert_eq!(res!(dur1.to_seconds()), 3661);
        
        let dur2 = ClockDuration::from_millis(123_456);
        assert_eq!(dur2.total_millis(), 123_456);
        
        let dur3 = ClockDuration::from_nanos(123_456_789);
        assert_eq!(res!(dur3.to_nanos()), 123_456_789);
        
        // Test duration arithmetic
        let sum = dur1.plus(&ClockDuration::from_seconds(60));
        assert_eq!(res!(sum.to_seconds()), 3721);
        
        let diff = dur1.minus(&ClockDuration::from_seconds(61));
        assert_eq!(res!(diff.to_seconds()), 3600);
        Ok(())
    }));
    
    res!(test_it(filter, &["clock_fields", "all", "clock", "fields"], || {
        // Test clock fields normalization
        let mut fields = ClockFields::from_time(10, 30, 90, 0); // 90 seconds should normalize
        fields.normalize();
        assert_eq!(fields.hour, 10);
        assert_eq!(fields.minute, 31);
        assert_eq!(fields.second, 30);
        
        // Test negative normalization - create with valid time then modify
        let mut fields2 = ClockFields::from_time(10, 30, 0, 0);
        fields2.second = -70; // -70 seconds
        fields2.normalize();
        assert_eq!(fields2.hour, 10);
        assert_eq!(fields2.minute, 28);
        assert_eq!(fields2.second, 50);
        
        // Test arithmetic
        let mut fields3 = ClockFields::from_time(23, 45, 30, 0);
        let fields4 = ClockFields::from_time(0, 30, 45, 0);
        fields3.add(&fields4);
        fields3.normalize();
        assert_eq!(fields3.hour, 0); // Wrapped to next day
        assert_eq!(fields3.minute, 16);
        assert_eq!(fields3.second, 15);
        Ok(())
    }));
    
    res!(test_it(filter, &["clock_hour_12", "all", "clock", "hour", "12hour"], || {
        // Test 12-hour format
        let hour0 = res!(ClockHour::new(0));
        assert_eq!(hour0.to_twelve_hour(), (12, false)); // 12 AM
        
        let hour12 = res!(ClockHour::new(12));
        assert_eq!(hour12.to_twelve_hour(), (12, true)); // 12 PM
        
        let hour13 = res!(ClockHour::new(13));
        assert_eq!(hour13.to_twelve_hour(), (1, true)); // 1 PM
        
        let hour23 = res!(ClockHour::new(23));
        assert_eq!(hour23.to_twelve_hour(), (11, true)); // 11 PM
        
        // Test from 12-hour
        let hour_am = res!(ClockHour::from_12_hour(10, false));
        assert_eq!(hour_am.of(), 10);
        
        let hour_pm = res!(ClockHour::from_12_hour(10, true));
        assert_eq!(hour_pm.of(), 22);
        
        let hour_12am = res!(ClockHour::from_12_hour(12, false));
        assert_eq!(hour_12am.of(), 0);
        
        let hour_12pm = res!(ClockHour::from_12_hour(12, true));
        assert_eq!(hour_12pm.of(), 12);
        Ok(())
    }));
    
    res!(test_it(filter, &["time_conversions", "all", "clock", "time", "conversion"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let time = res!(ClockTime::new(14, 30, 45, 123_456_789, zone.clone()));
        
        // Test conversions
        let nanos = time.to_nanos_of_day();
        assert_eq!(nanos, (14 * 3600 + 30 * 60 + 45) * 1_000_000_000 + 123_456_789);
        
        let millis = time.millis_of_day();
        assert_eq!(millis, (14 * 3600 + 30 * 60 + 45) * 1000 + 123);
        
        // Test from conversions
        let time2 = res!(ClockTime::from_nanos_of_day(nanos, zone));
        assert_eq!(time2.hour().of(), 14);
        assert_eq!(time2.minute().of(), 30);
        assert_eq!(time2.second().of(), 45);
        assert_eq!(time2.nanosecond().of(), 123_456_789);
        Ok(())
    }));
    
    res!(test_it(filter, &["time_arithmetic", "all", "clock", "time", "arithmetic"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let time1 = res!(ClockTime::new(10, 30, 0, 0, zone.clone()));
        
        // Add duration
        let time2 = res!(time1.add_duration(&ClockDuration::from_seconds(3600))); // Add 1 hour
        assert_eq!(time2.hour().of(), 11);
        assert_eq!(time2.minute().of(), 30);
        
        // Add across midnight
        let time3 = res!(ClockTime::new(23, 30, 0, 0, zone.clone()));
        let time4 = res!(time3.add_duration(&ClockDuration::from_seconds(3600))); // Add 1 hour
        assert_eq!(time4.hour().of(), 0);
        assert_eq!(time4.minute().of(), 30);
        
        // Duration between times
        let duration = time1.duration_until(&time2);
        assert_eq!(res!(duration.to_seconds()), 3600);
        Ok(())
    }));
    
    res!(test_it(filter, &["time_comparison", "all", "clock", "time", "comparison"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        let time1 = res!(ClockTime::new(10, 30, 0, 0, zone.clone()));
        let time2 = res!(ClockTime::new(10, 30, 0, 1, zone.clone()));
        let time3 = res!(ClockTime::new(10, 30, 0, 0, zone.clone()));
        
        assert!(time1 < time2);
        assert!(time2 > time1);
        assert_eq!(time1, time3);
        
        assert!(time1.is_before(&time2));
        assert!(time2.is_after(&time1));
        assert!(!time1.is_before(&time3));
        Ok(())
    }));
    
    Ok(())
}