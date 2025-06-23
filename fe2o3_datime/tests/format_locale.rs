use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_datime::{
    format::{CalClockFormatter, Locale},
    time::{CalClock, CalClockZone},
};

/// Tests locale-based formatting functionality
pub fn test_format_locale(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["locale_basic", "all", "format", "locale"], || {
        // Test basic locale creation and properties
        let us_locale = Locale::us();
        assert_eq!(us_locale.id(), "en-US");
        assert_eq!(us_locale.display_name(), "English (United States)");
        
        let german_locale = Locale::germany();
        assert_eq!(german_locale.id(), "de-DE");
        assert_eq!(german_locale.display_name(), "Deutsch (Deutschland)");
        
        let iso_locale = Locale::iso();
        assert_eq!(iso_locale.id(), "ISO");
        
        println!("✅ Basic locale creation and properties work correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_patterns", "all", "format", "locale"], || {
        // Test that different locales have different format patterns
        let us_locale = Locale::us();
        let uk_locale = Locale::uk();
        let german_locale = Locale::germany();
        let iso_locale = Locale::iso();
        
        // US uses MM/dd/yyyy
        assert_eq!(us_locale.date_pattern().pattern_string(), "MM/dd/yyyy");
        
        // UK uses dd/MM/yyyy
        assert_eq!(uk_locale.date_pattern().pattern_string(), "dd/MM/yyyy");
        
        // Germany uses dd.MM.yyyy
        assert_eq!(german_locale.date_pattern().pattern_string(), "dd.MM.yyyy");
        
        // ISO uses yyyy-MM-dd
        assert_eq!(iso_locale.date_pattern().pattern_string(), "yyyy-MM-dd");
        
        // Check time patterns differ too
        assert_eq!(us_locale.time_pattern().pattern_string(), "h:mm:ss a"); // 12-hour
        assert_eq!(uk_locale.time_pattern().pattern_string(), "HH:mm:ss");  // 24-hour
        
        println!("✅ Different locales have appropriate format patterns");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_formatting", "all", "format", "locale"], || {
        // Test actual formatting with different locales
        let formatter = CalClockFormatter::new();
        
        // Create a test CalClock: January 15, 2024, 14:30:45
        let zone = CalClockZone::utc();
        let test_calclock = res!(CalClock::new(2024, 1, 15, 14, 30, 45, 123_456_789, zone));
        
        // Test US formatting
        let us_locale = Locale::us();
        let us_date = res!(formatter.format_date_with_locale(&test_calclock, &us_locale));
        assert_eq!(us_date, "01/15/2024"); // MM/dd/yyyy
        
        let us_time = res!(formatter.format_time_with_locale(&test_calclock, &us_locale));
        assert_eq!(us_time, "2:30:45 PM"); // h:mm:ss a
        
        // Test German formatting
        let german_locale = Locale::germany();
        let german_date = res!(formatter.format_date_with_locale(&test_calclock, &german_locale));
        assert_eq!(german_date, "15.01.2024"); // dd.MM.yyyy
        
        let german_time = res!(formatter.format_time_with_locale(&test_calclock, &german_locale));
        assert_eq!(german_time, "14:30:45"); // HH:mm:ss
        
        // Test ISO formatting
        let iso_locale = Locale::iso();
        let iso_date = res!(formatter.format_date_with_locale(&test_calclock, &iso_locale));
        assert_eq!(iso_date, "2024-01-15"); // yyyy-MM-dd
        
        let iso_time = res!(formatter.format_time_with_locale(&test_calclock, &iso_locale));
        assert_eq!(iso_time, "14:30:45"); // HH:mm:ss
        
        println!("✅ Locale-specific formatting produces correct output");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_from_id", "all", "format", "locale"], || {
        // Test locale creation from ID strings
        let us_locale = Locale::from_id("en-US");
        assert_eq!(us_locale.id(), "en-US");
        
        let japanese_locale = Locale::from_id("ja-JP");
        assert_eq!(japanese_locale.id(), "ja-JP");
        assert_eq!(japanese_locale.date_pattern().pattern_string(), "yyyy/MM/dd");
        
        let chinese_locale = Locale::from_id("zh-CN");
        assert_eq!(chinese_locale.id(), "zh-CN");
        assert_eq!(chinese_locale.date_pattern().pattern_string(), "yyyy/M/d");
        
        // Test fallback for unknown locale
        let unknown_locale = Locale::from_id("xx-XX");
        assert_eq!(unknown_locale.id(), "en-US"); // Should fall back to US
        
        println!("✅ Locale creation from ID strings works correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_available_list", "all", "format", "locale"], || {
        // Test available locales functionality
        let available_locales = Locale::available_locales();
        assert!(available_locales.contains(&"en-US".to_string()));
        assert!(available_locales.contains(&"de-DE".to_string()));
        assert!(available_locales.contains(&"ja-JP".to_string()));
        assert!(available_locales.contains(&"ISO".to_string()));
        assert!(available_locales.len() >= 7);
        
        let locales_with_names = Locale::available_locales_with_names();
        assert!(locales_with_names.iter().any(|(id, name)| 
            id == "en-US" && name == "English (United States)"));
        assert!(locales_with_names.iter().any(|(id, name)| 
            id == "de-DE" && name == "Deutsch (Deutschland)"));
        
        println!("✅ Available locales listing works correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_display", "all", "format", "locale"], || {
        // Test locale display formatting
        let us_locale = Locale::us();
        let display_string = format!("{}", us_locale);
        assert!(display_string.contains("English (United States)"));
        assert!(display_string.contains("en-US"));
        
        let german_locale = Locale::germany();
        let german_display = format!("{}", german_locale);
        assert!(german_display.contains("Deutsch (Deutschland)"));
        assert!(german_display.contains("de-DE"));
        
        println!("✅ Locale display formatting works correctly");
        Ok(())
    }));
    
    res!(test_it(filter, &["locale_practical_example", "all", "format", "locale"], || {
        // Test a practical real-world example
        let formatter = CalClockFormatter::new();
        
        // Create a CalClock for testing - February 29, 2024 (leap year), 9:05:03 AM
        let zone = CalClockZone::utc();
        let test_calclock = res!(CalClock::new(2024, 2, 29, 9, 5, 3, 0, zone));
        
        // Test different locale outputs for the same date/time
        let locales_and_expected = vec![
            (Locale::us(), "02/29/2024", "9:05:03 AM"),
            (Locale::uk(), "29/02/2024", "09:05:03"),
            (Locale::germany(), "29.02.2024", "09:05:03"),
            (Locale::france(), "29/02/2024", "09:05:03"),
            (Locale::japan(), "2024/02/29", "09:05:03"),
            (Locale::china(), "2024/2/29", "09:05:03"),
            (Locale::iso(), "2024-02-29", "09:05:03"),
        ];
        
        for (locale, expected_date, expected_time) in locales_and_expected {
            let actual_date = res!(formatter.format_date_with_locale(&test_calclock, &locale));
            let actual_time = res!(formatter.format_time_with_locale(&test_calclock, &locale));
            
            assert_eq!(actual_date, expected_date, "Date mismatch for locale {}", locale.id());
            assert_eq!(actual_time, expected_time, "Time mismatch for locale {}", locale.id());
        }
        
        println!("✅ Practical locale formatting example works correctly");
        Ok(())
    }));
    
    Ok(())
}