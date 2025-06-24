use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_datime::{
    calendar::{CalendarDate, CalendarSystem},
    time::{
        CalClock,
        CalClockZone, 
        SystemTimezoneManager, 
        SystemTimezoneConfig, 
        SystemTimezoneExt,
        LeapSecondCapability,
    },
};

pub fn test_new_features(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["calendar_systems", "all", "calendar", "gregorian", "julian"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Test default Gregorian calendar
        let gregorian_date = res!(CalendarDate::new(2024, 6, 15, zone.clone()));
        assert!(gregorian_date.is_gregorian());
        assert!(!gregorian_date.is_julian());
        assert_eq!(gregorian_date.calendar_system(), &CalendarSystem::Gregorian);
        
        // Test explicit Gregorian calendar
        let gregorian_explicit = res!(CalendarDate::new_with_system(
            2024, 6, 15, zone.clone(), CalendarSystem::Gregorian
        ));
        assert_eq!(gregorian_date, gregorian_explicit);
        
        // Test Julian calendar
        let julian_date = res!(CalendarDate::new_with_system(
            2024, 6, 15, zone.clone(), CalendarSystem::Julian
        ));
        assert!(!julian_date.is_gregorian());
        assert!(julian_date.is_julian());
        assert_eq!(julian_date.calendar_system(), &CalendarSystem::Julian);
        
        // Test calendar conversion
        let converted_to_julian = res!(gregorian_date.to_julian());
        assert!(converted_to_julian.is_julian());
        
        let converted_to_gregorian = res!(julian_date.to_gregorian());
        assert!(converted_to_gregorian.is_gregorian());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["leap_year_differences", "all", "calendar", "leap"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Test year 1900 - leap in Julian, not in Gregorian
        let gregorian_1900 = res!(CalendarDate::new_with_system(
            1900, 2, 28, zone.clone(), CalendarSystem::Gregorian
        ));
        let julian_1900 = res!(CalendarDate::new_with_system(
            1900, 2, 29, zone.clone(), CalendarSystem::Julian
        ));
        
        assert!(!gregorian_1900.is_leap_year());
        assert!(julian_1900.is_leap_year());
        
        // Test year 2000 - leap in both
        let gregorian_2000 = res!(CalendarDate::new_with_system(
            2000, 2, 29, zone.clone(), CalendarSystem::Gregorian
        ));
        let julian_2000 = res!(CalendarDate::new_with_system(
            2000, 2, 29, zone.clone(), CalendarSystem::Julian
        ));
        
        assert!(gregorian_2000.is_leap_year());
        assert!(julian_2000.is_leap_year());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["system_timezone_config", "all", "timezone", "system"], || {
        // Test default conservative configuration
        let default_config = SystemTimezoneConfig::default();
        assert!(!default_config.use_system_data);
        assert!(default_config.require_consent);
        assert!(default_config.detect_conflicts);
        assert!(!default_config.search_paths.is_empty());
        
        // Test automatic configuration (Jiff-style)
        let auto_config = SystemTimezoneConfig::automatic();
        assert!(auto_config.use_system_data);
        assert!(!auto_config.require_consent);
        assert!(auto_config.detect_conflicts);
        
        // Test with consent configuration
        let consent_config = SystemTimezoneConfig::with_consent();
        assert!(consent_config.use_system_data);
        assert!(consent_config.require_consent);
        assert!(consent_config.detect_conflicts);
        
        Ok(())
    }));
    
    res!(test_it(filter, &["system_timezone_manager", "all", "timezone", "manager"], || {
        let config = SystemTimezoneConfig::default();
        let manager = SystemTimezoneManager::new(config);
        
        // Test initial state
        assert!(!manager.has_consent());
        
        // Test cache stats
        let stats = manager.cache_stats();
        assert_eq!(stats.cached_zones, 0);
        
        // Test system timezone list (should work even without consent for listing)
        let _zones = res!(manager.list_system_timezones());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["embedded_vs_system_timezone", "all", "timezone", "embedded"], || {
        // Test embedded-only timezone creation
        let embedded_utc = res!(CalClockZone::new_embedded("UTC"));
        assert_eq!(embedded_utc.id(), "UTC");
        
        // Test system-or-embedded timezone creation (should fall back to embedded)
        let system_utc = res!(CalClockZone::from_system_or_embedded("UTC"));
        assert_eq!(system_utc.id(), "UTC");
        
        // Test conflict detection (should return empty since we don't have system data)
        let conflicts = res!(embedded_utc.detect_conflicts());
        assert!(conflicts.is_empty());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["leap_second_capability", "all", "timezone", "leap_seconds"], || {
        // Test leap second capability assessment
        assert!(!LeapSecondCapability::handles_leap_seconds());
        assert!(LeapSecondCapability::requires_separate_implementation());
        
        let explanation = LeapSecondCapability::leap_second_explanation();
        assert!(explanation.contains("NOT handle leap seconds"));
        assert!(explanation.contains("TAI-UTC"));
        assert!(explanation.contains("separate"));
        
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_display", "all", "calendar", "display"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Test Gregorian display (should not show calendar type)
        let gregorian = res!(CalendarDate::new(2024, 6, 15, zone.clone()));
        let gregorian_str = format!("{}", gregorian);
        assert_eq!(gregorian_str, "2024-06-15");
        
        // Test Julian display (should show calendar type)
        let julian = res!(CalendarDate::new_with_system(
            2024, 6, 15, zone.clone(), CalendarSystem::Julian
        ));
        let julian_str = format!("{}", julian);
        assert_eq!(julian_str, "2024-06-15 (Julian)");
        
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_system_parsing", "all", "calendar", "parsing"], || {
        // Test calendar system parsing from string
        assert_eq!(res!(CalendarSystem::from_str("gregorian")), CalendarSystem::Gregorian);
        assert_eq!(res!(CalendarSystem::from_str("GREGORIAN")), CalendarSystem::Gregorian);
        assert_eq!(res!(CalendarSystem::from_str("greg")), CalendarSystem::Gregorian);
        assert_eq!(res!(CalendarSystem::from_str("g")), CalendarSystem::Gregorian);
        
        assert_eq!(res!(CalendarSystem::from_str("julian")), CalendarSystem::Julian);
        assert_eq!(res!(CalendarSystem::from_str("JULIAN")), CalendarSystem::Julian);
        assert_eq!(res!(CalendarSystem::from_str("jul")), CalendarSystem::Julian);
        assert_eq!(res!(CalendarSystem::from_str("j")), CalendarSystem::Julian);
        
        // Test invalid parsing
        assert!(CalendarSystem::from_str("invalid").is_err());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_integration_with_calclock", "all", "calendar", "calclock"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Test that CalClock still uses Gregorian calendar by default
        let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone.clone()));
        assert!(calclock.date().is_gregorian());
        
        // Test CalClock with Julian calendar date
        let julian_date = res!(CalendarDate::new_with_system(
            2024, 6, 15, zone.clone(), CalendarSystem::Julian
        ));
        assert!(julian_date.is_julian());
        
        Ok(())
    }));
    
    Ok(())
}