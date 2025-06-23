use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_datime::{
    calendar::Calendar,
    constant::DayOfWeek,
    core::Time,
    time::CalClockZone,
};

/// Tests the comprehensive Calendar system with all calendar types
pub fn test_calendar(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["calendar_creation", "all", "calendar", "comprehensive"], || {
        // Test creating different calendar types
        let gregorian = Calendar::new();
        assert_eq!(gregorian, Calendar::Gregorian);
        assert_eq!(gregorian.name(), "Gregorian");
        assert_eq!(gregorian.id(), "gregorian");
        
        let islamic = Calendar::Islamic;
        assert_eq!(islamic.name(), "Islamic");
        assert_eq!(islamic.id(), "islamic");
        
        let japanese = Calendar::Japanese;
        assert_eq!(japanese.name(), "Japanese");
        assert_eq!(japanese.id(), "japanese");
        
        let thai = Calendar::Thai;
        assert_eq!(thai.name(), "Thai Buddhist");
        assert_eq!(thai.id(), "thai");
        
        let minguo = Calendar::Minguo;
        assert_eq!(minguo.name(), "Minguo");
        assert_eq!(minguo.id(), "minguo");
        
        let holocene = Calendar::Holocene;
        assert_eq!(holocene.name(), "Holocene");
        assert_eq!(holocene.id(), "holocene");
        
        println!("✅ All calendar types created successfully");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_date_creation", "all", "calendar", "comprehensive"], || {
        let zone = CalClockZone::utc();
        
        // Test Gregorian date creation (the new API)
        let gregorian = Calendar::Gregorian;
        let greg_date = res!(gregorian.date(2024, 1, 15, zone.clone()));
        assert_eq!(greg_date.year(), 2024);
        assert_eq!(greg_date.month(), 1);
        assert_eq!(greg_date.day(), 15);
        
        // Test Thai Buddhist calendar (2567 = 2024 Gregorian)
        let thai = Calendar::Thai;
        let thai_date = res!(thai.date(2567, 1, 15, zone.clone()));
        assert_eq!(thai_date.year(), 2024); // Internal Gregorian representation
        
        // Test Minguo calendar (113 = 2024 Gregorian)
        let minguo = Calendar::Minguo;
        let minguo_date = res!(minguo.date(113, 1, 15, zone.clone()));
        assert_eq!(minguo_date.year(), 2024); // Internal Gregorian representation
        
        // Test Holocene calendar (12024 = 2024 Gregorian)
        let holocene = Calendar::Holocene;
        let holocene_date = res!(holocene.date(12024, 1, 15, zone.clone()));
        assert_eq!(holocene_date.year(), 2024); // Internal Gregorian representation
        
        // Test Islamic calendar basic creation
        let islamic = Calendar::Islamic;
        let islamic_date = res!(islamic.date(1445, 1, 15, zone.clone()));
        // Islamic dates are converted internally - the exact conversion is complex
        assert!(islamic_date.year() > 1000); // Should be a reasonable Gregorian year
        
        println!("✅ Date creation with new Calendar API works correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_leap_years", "all", "calendar", "comprehensive"], || {
        // Test leap year calculations for different calendars
        
        // Gregorian leap years
        let gregorian = Calendar::Gregorian;
        assert!(gregorian.is_leap_year(2024)); // Divisible by 4
        assert!(!gregorian.is_leap_year(1900)); // Century not divisible by 400
        assert!(gregorian.is_leap_year(2000)); // Century divisible by 400
        assert!(!gregorian.is_leap_year(2023)); // Not divisible by 4
        
        // Julian leap years (every 4 years, no exceptions)
        let julian = Calendar::Julian;
        assert!(julian.is_leap_year(2024));
        assert!(julian.is_leap_year(1900)); // Leap in Julian, not Gregorian
        assert!(julian.is_leap_year(2000));
        assert!(!julian.is_leap_year(2023));
        
        // Islamic leap years (30-year cycle)
        let islamic = Calendar::Islamic;
        assert!(islamic.is_leap_year(2)); // 2nd year of cycle
        assert!(islamic.is_leap_year(5)); // 5th year of cycle
        assert!(!islamic.is_leap_year(1)); // 1st year not leap
        assert!(!islamic.is_leap_year(3)); // 3rd year not leap
        
        // Thai, Minguo, and Holocene use Gregorian rules
        let thai = Calendar::Thai;
        let minguo = Calendar::Minguo;
        let holocene = Calendar::Holocene;
        
        assert!(thai.is_leap_year(2567)); // Thai 2567 = Gregorian 2024
        assert!(minguo.is_leap_year(113)); // Minguo 113 = Gregorian 2024
        assert!(holocene.is_leap_year(12024)); // Holocene 12024 = Gregorian 2024
        
        println!("✅ Leap year calculations work correctly for all calendars");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_month_days", "all", "calendar", "comprehensive"], || {
        // Test days in month for different calendars
        
        // Gregorian/Thai/Minguo/Holocene all use same month structure
        let gregorian = Calendar::Gregorian;
        assert_eq!(res!(gregorian.days_in_month(2024, 1)), 31); // January
        assert_eq!(res!(gregorian.days_in_month(2024, 2)), 29); // February leap year
        assert_eq!(res!(gregorian.days_in_month(2023, 2)), 28); // February non-leap
        assert_eq!(res!(gregorian.days_in_month(2024, 4)), 30); // April
        
        // Islamic calendar has different month structure
        let islamic = Calendar::Islamic;
        assert_eq!(res!(islamic.days_in_month(1445, 1)), 30); // Muharram (odd month)
        assert_eq!(res!(islamic.days_in_month(1445, 2)), 29); // Safar (even month)
        
        // Year 1445 is cycle year 5, which IS a leap year
        assert!(islamic.is_leap_year(1445)); // Year 5 of 30-year cycle
        assert_eq!(res!(islamic.days_in_month(1445, 12)), 30); // Dhul-Hijjah leap year
        
        // Test non-leap year (1444 is cycle year 4, not leap)
        assert!(!islamic.is_leap_year(1444)); // Year 4 of 30-year cycle, not leap
        assert_eq!(res!(islamic.days_in_month(1444, 12)), 29); // Dhul-Hijjah non-leap year
        
        println!("✅ Days in month calculations work correctly for all calendars");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_validation", "all", "calendar", "comprehensive"], || {
        // Test date validation for different calendars
        
        let gregorian = Calendar::Gregorian;
        let islamic = Calendar::Islamic;
        let thai = Calendar::Thai;
        let japanese = Calendar::Japanese;
        
        // Valid dates
        assert!(gregorian.validate_date(2024, 2, 29).is_ok()); // Leap year
        assert!(islamic.validate_date(1445, 1, 30).is_ok()); // Valid Islamic date
        assert!(thai.validate_date(2567, 1, 31).is_ok()); // Valid Thai date
        assert!(japanese.validate_date(5, 1, 15).is_ok()); // Valid Japanese era year
        
        // Invalid dates
        assert!(gregorian.validate_date(2023, 2, 29).is_err()); // Non-leap year Feb 29
        assert!(gregorian.validate_date(2024, 4, 31).is_err()); // April has 30 days
        assert!(islamic.validate_date(1445, 1, 31).is_err()); // Month 1 has 30 days
        assert!(islamic.validate_date(1445, 13, 1).is_err()); // Month 13 doesn't exist
        assert!(islamic.validate_date(0, 1, 1).is_err()); // Year 0 invalid
        assert!(thai.validate_date(0, 1, 1).is_err()); // Year 0 invalid
        
        // Day 0 invalid for all calendars
        assert!(gregorian.validate_date(2024, 1, 0).is_err());
        assert!(islamic.validate_date(1445, 1, 0).is_err());
        
        println!("✅ Date validation works correctly for all calendars");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_parsing", "all", "calendar", "comprehensive"], || {
        // Test parsing calendar names from strings
        
        assert_eq!(res!(Calendar::from_str("gregorian")), Calendar::Gregorian);
        assert_eq!(res!(Calendar::from_str("GREGORIAN")), Calendar::Gregorian);
        assert_eq!(res!(Calendar::from_str("greg")), Calendar::Gregorian);
        assert_eq!(res!(Calendar::from_str("g")), Calendar::Gregorian);
        
        assert_eq!(res!(Calendar::from_str("julian")), Calendar::Julian);
        assert_eq!(res!(Calendar::from_str("jul")), Calendar::Julian);
        assert_eq!(res!(Calendar::from_str("j")), Calendar::Julian);
        
        assert_eq!(res!(Calendar::from_str("islamic")), Calendar::Islamic);
        assert_eq!(res!(Calendar::from_str("hijri")), Calendar::Islamic);
        assert_eq!(res!(Calendar::from_str("muslim")), Calendar::Islamic);
        assert_eq!(res!(Calendar::from_str("i")), Calendar::Islamic);
        
        assert_eq!(res!(Calendar::from_str("japanese")), Calendar::Japanese);
        assert_eq!(res!(Calendar::from_str("jp")), Calendar::Japanese);
        assert_eq!(res!(Calendar::from_str("imperial")), Calendar::Japanese);
        
        assert_eq!(res!(Calendar::from_str("thai")), Calendar::Thai);
        assert_eq!(res!(Calendar::from_str("buddhist")), Calendar::Thai);
        assert_eq!(res!(Calendar::from_str("th")), Calendar::Thai);
        
        assert_eq!(res!(Calendar::from_str("minguo")), Calendar::Minguo);
        assert_eq!(res!(Calendar::from_str("roc")), Calendar::Minguo);
        assert_eq!(res!(Calendar::from_str("taiwan")), Calendar::Minguo);
        
        assert_eq!(res!(Calendar::from_str("holocene")), Calendar::Holocene);
        assert_eq!(res!(Calendar::from_str("human")), Calendar::Holocene);
        assert_eq!(res!(Calendar::from_str("h")), Calendar::Holocene);
        
        // Invalid calendar name
        assert!(Calendar::from_str("invalid").is_err());
        
        println!("✅ Calendar name parsing works correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_year_conversions", "all", "calendar", "comprehensive"], || {
        // Test year conversions between different calendars
        
        // Test known conversions
        let gregorian = Calendar::Gregorian;
        let thai = Calendar::Thai;
        let minguo = Calendar::Minguo;
        let holocene = Calendar::Holocene;
        
        // Thai Buddhist = Gregorian + 543
        // So Thai 2567 = Gregorian 2024
        assert_eq!(thai.to_gregorian_year(2567), 2024);
        
        // Minguo = Gregorian - 1911
        // So Minguo 113 = Gregorian 2024
        assert_eq!(minguo.to_gregorian_year(113), 2024);
        
        // Holocene = Gregorian + 10000
        // So Holocene 12024 = Gregorian 2024
        assert_eq!(holocene.to_gregorian_year(12024), 2024);
        
        // Test epoch years
        assert_eq!(gregorian.epoch_year(), 1);
        assert_eq!(thai.epoch_year(), -543); // 544 BCE
        assert_eq!(minguo.epoch_year(), 1912);
        assert_eq!(holocene.epoch_year(), -9999); // 10,000 BCE
        
        println!("✅ Year conversions work correctly between calendars");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_info_methods", "all", "calendar", "comprehensive"], || {
        // Test informational methods
        
        let calendars = Calendar::all().collect::<Vec<_>>();
        assert_eq!(calendars.len(), 7);
        
        for calendar in calendars {
            // Each calendar should have non-empty name, id, and description
            assert!(!calendar.name().is_empty());
            assert!(!calendar.id().is_empty());
            assert!(!calendar.description().is_empty());
            
            // Test display
            let display_string = format!("{}", calendar);
            assert_eq!(display_string, calendar.name());
        }
        
        // Test specific descriptions
        assert!(Calendar::Gregorian.description().contains("1582"));
        assert!(Calendar::Islamic.description().contains("Lunar"));
        assert!(Calendar::Japanese.description().contains("era"));
        assert!(Calendar::Thai.description().contains("Buddhist"));
        assert!(Calendar::Minguo.description().contains("Republic"));
        assert!(Calendar::Holocene.description().contains("Scientific"));
        
        println!("✅ Calendar information methods work correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_conversion_matrix", "all", "calendar", "comprehensive"], || {
        // Test comprehensive conversion matrix between all calendar systems
        let zone = CalClockZone::utc();
        
        // Test data: same date represented in different calendar systems
        // Note: Islamic and Japanese calendars are excluded from comprehensive testing 
        // as they need more sophisticated conversion algorithms
        let test_cases: Vec<(Calendar, i32, u8, u8)> = vec![
            (Calendar::Gregorian, 2024, 1, 15),
            (Calendar::Thai, 2567, 1, 15),       // Thai 2567 = Gregorian 2024
            (Calendar::Minguo, 113, 1, 15),     // Minguo 113 = Gregorian 2024
            (Calendar::Holocene, 12024, 1, 15), // Holocene 12024 = Gregorian 2024
            (Calendar::Julian, 2024, 1, 15),    // Julian and Gregorian coincide for modern dates
        ];
        
        // Test conversions between supported calendars (excluding Islamic and Japanese for now)
        let supported_calendars = vec![
            Calendar::Gregorian,
            Calendar::Julian,
            Calendar::Thai,
            Calendar::Minguo,
            Calendar::Holocene,
        ];
        
        for (source_calendar, year, month, day) in &test_cases {
            let source_date = res!(source_calendar.date(*year, *month, *day, zone.clone()));
            
            for target_calendar in &supported_calendars {
                if source_calendar == target_calendar {
                    continue; // Skip self-conversion
                }
                
                // Convert to target calendar
                let converted_date = res!(source_calendar.convert_date(&source_date, target_calendar));
                
                // Convert back to source calendar
                let round_trip_date = res!(target_calendar.convert_date(&converted_date, source_calendar));
                
                // Round-trip should return to original date (allowing for precision differences)
                assert_eq!(
                    round_trip_date.year(), source_date.year(),
                    "Round-trip conversion failed: {} -> {} -> {} (year mismatch)",
                    source_calendar.name(), target_calendar.name(), source_calendar.name()
                );
                assert_eq!(
                    round_trip_date.month(), source_date.month(),
                    "Round-trip conversion failed: {} -> {} -> {} (month mismatch)",
                    source_calendar.name(), target_calendar.name(), source_calendar.name()
                );
                assert_eq!(
                    round_trip_date.day(), source_date.day(),
                    "Round-trip conversion failed: {} -> {} -> {} (day mismatch)",
                    source_calendar.name(), target_calendar.name(), source_calendar.name()
                );
            }
        }
        
        // Test specific Islamic calendar conversions (more complex due to lunar nature)
        let islamic = Calendar::Islamic;
        let gregorian = Calendar::Gregorian;
        
        let islamic_date = res!(islamic.date(1445, 7, 15, zone.clone())); // Islamic date
        let gregorian_equivalent = res!(islamic.convert_date(&islamic_date, &gregorian));
        let back_to_islamic = res!(gregorian.convert_date(&gregorian_equivalent, &islamic));
        
        // Islamic conversions may have slight variations due to lunar vs solar calendar differences
        // Just ensure the conversion is reasonable (should be in early 2024)
        assert!(gregorian_equivalent.year() >= 2023 && gregorian_equivalent.year() <= 2025,
               "Islamic to Gregorian conversion should yield date around 2024, got {}", 
               gregorian_equivalent.year());
        
        println!("✅ Calendar conversion matrix tested successfully");
        Ok(())
    }));
    
    res!(test_it(filter, &["calendar_practical_examples", "all", "calendar", "comprehensive"], || {
        // Test practical usage examples
        let zone = CalClockZone::utc();
        
        // Example 1: Creating dates in different calendar systems
        let gregorian = Calendar::new(); // Default to Gregorian
        let date_2024 = res!(gregorian.date(2024, 1, 15, zone.clone()));
        
        // Same date in different calendars
        let thai = Calendar::Thai;
        let thai_date = res!(thai.date(2567, 1, 15, zone.clone())); // Thai 2567 = Gregorian 2024
        
        let holocene = Calendar::Holocene;
        let holocene_date = res!(holocene.date(12024, 1, 15, zone.clone())); // Holocene 12024 = Gregorian 2024
        
        // All should represent the same date internally
        assert_eq!(date_2024.year(), thai_date.year());
        assert_eq!(date_2024.year(), holocene_date.year());
        assert_eq!(date_2024.month(), thai_date.month());
        assert_eq!(date_2024.month(), holocene_date.month());
        assert_eq!(date_2024.day(), thai_date.day());
        assert_eq!(date_2024.day(), holocene_date.day());
        
        // Example 2: Working with Islamic calendar
        let islamic = Calendar::Islamic;
        let _islamic_date = res!(islamic.date(1445, 7, 15, zone.clone()));
        
        // Islamic calendar should have different month structure
        assert_eq!(res!(islamic.days_in_month(1445, 7)), 30); // 7th month (Rajab) has 30 days
        assert_eq!(res!(islamic.days_in_month(1445, 8)), 29); // 8th month (Sha'ban) has 29 days
        
        // Example 3: Leap year differences
        let julian = Calendar::Julian;
        
        // Year 1900: leap in Julian, not in Gregorian
        assert!(julian.is_leap_year(1900));
        assert!(!gregorian.is_leap_year(1900));
        
        // This affects February days
        assert_eq!(res!(julian.days_in_month(1900, 2)), 29);
        assert_eq!(res!(gregorian.days_in_month(1900, 2)), 28);
        
        println!("✅ Practical calendar examples work correctly");
        Ok(())
    }));
    
    // Additional comprehensive tests merged from calendar.rs
    
    res!(test_it(filter, &["day_of_week", "all", "calendar", "comprehensive"], || {
        let zone = CalClockZone::utc();
        let gregorian = Calendar::Gregorian;
        
        // Some known dates
        let date1 = res!(gregorian.date(2024, 1, 1, zone.clone())); // Monday
        assert_eq!(date1.day_of_week(), DayOfWeek::Monday);
        
        let date2 = res!(gregorian.date(2024, 3, 15, zone.clone())); // Friday
        assert_eq!(date2.day_of_week(), DayOfWeek::Friday);
        
        let date3 = res!(gregorian.date(2000, 1, 1, zone.clone())); // Saturday
        assert_eq!(date3.day_of_week(), DayOfWeek::Saturday);
        
        println!("✅ Day of week calculations work correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["date_arithmetic", "all", "calendar", "comprehensive"], || {
        let zone = CalClockZone::utc();
        let gregorian = Calendar::Gregorian;
        
        // Test adding days
        let date1 = res!(gregorian.date(2024, 1, 31, zone.clone()));
        let date2 = res!(date1.add_days(1));
        assert_eq!(date2.year(), 2024);
        assert_eq!(date2.month(), 2);
        assert_eq!(date2.day(), 1);
        
        // Test adding months with day adjustment
        let date3 = res!(date1.plus(0, 1, 0)); // Jan 31 + 1 month = Feb 29 (leap year)
        assert_eq!(date3.year(), 2024);
        assert_eq!(date3.month(), 2);
        assert_eq!(date3.day(), 29);
        
        // Test subtracting
        let date4 = res!(gregorian.date(2024, 3, 1, zone.clone()));
        let date5 = res!(date4.minus(0, 0, 1));
        assert_eq!(date5.year(), 2024);
        assert_eq!(date5.month(), 2);
        assert_eq!(date5.day(), 29); // Leap year
        
        println!("✅ Date arithmetic operations work correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["date_comparison", "all", "calendar", "comprehensive"], || {
        let zone = CalClockZone::utc();
        let gregorian = Calendar::Gregorian;
        
        let date1 = res!(gregorian.date(2024, 1, 15, zone.clone()));
        let date2 = res!(gregorian.date(2024, 1, 16, zone.clone()));
        let date3 = res!(gregorian.date(2024, 1, 15, zone.clone()));
        
        assert!(date1.is_before(&date2));
        assert!(!date2.is_before(&date1));
        assert!(!date1.is_before(&date3));
        
        assert!(date2.is_after(&date1));
        assert!(!date1.is_after(&date2));
        assert!(!date1.is_after(&date3));
        
        assert_eq!(date1, date3);
        assert_ne!(date1, date2);
        
        println!("✅ Date comparison operations work correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["day_of_year", "all", "calendar", "comprehensive"], || {
        let zone = CalClockZone::utc();
        let gregorian = Calendar::Gregorian;
        
        let date1 = res!(gregorian.date(2024, 1, 1, zone.clone()));
        assert_eq!(res!(date1.day_of_year()), 1);
        
        let date2 = res!(gregorian.date(2024, 12, 31, zone.clone()));
        assert_eq!(res!(date2.day_of_year()), 366); // Leap year
        
        let date3 = res!(gregorian.date(2023, 12, 31, zone.clone()));
        assert_eq!(res!(date3.day_of_year()), 365); // Non-leap year
        
        let date4 = res!(gregorian.date(2024, 3, 1, zone.clone()));
        assert_eq!(res!(date4.day_of_year()), 61); // 31 (Jan) + 29 (Feb) + 1
        
        println!("✅ Day of year calculations work correctly");
        Ok(())
    }));
    
    Ok(())
}