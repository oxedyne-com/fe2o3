/// Comprehensive demonstration of advanced formatter capabilities.
/// 
/// This test demonstrates the enhanced formatting features including:
/// - Locale-specific formatting
/// - Custom month and day names
/// - Ordinal suffixes (1st, 2nd, 3rd)
/// - Week of year and era formatting
/// - AM/PM customization

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::{
    format::{CalClockFormatter, FormatPattern, Locale},
    calendar::CalendarDate,
    clock::ClockTime,
    constant::{MonthOfYear, DayOfWeek},
    time::{CalClock, CalClockZone},
};

use std::collections::HashMap;

#[test]
fn test_advanced_formatter_features() -> Outcome<()> {
    println!("=== Advanced Formatter Features Demo ===");
    
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::now(zone.clone()));
    
    // Test 1: Basic formatting with various patterns
    println!("\n📅 Test 1: Basic Pattern Formatting");
    
    let formatter = CalClockFormatter::new();
    let patterns = vec![
        ("yyyy-MM-dd", "ISO date"),
        ("dd/MM/yyyy", "European date"),
        ("MM/dd/yyyy", "US date"),
        ("EEEE, MMMM d, yyyy", "Full date"),
        ("EEE, MMM d, ''yy", "Short date"),
        ("h:mm:ss a", "12-hour time"),
        ("HH:mm:ss", "24-hour time"),
        ("yyyy-MM-dd'T'HH:mm:ss", "ISO datetime"),
        ("yyyy-MM-dd HH:mm:ss Z", "With timezone offset"),
        ("EEEE, MMMM d, yyyy 'at' h:mm a", "Natural format"),
    ];
    
    for (pattern_str, description) in patterns {
        let pattern = res!(FormatPattern::new(pattern_str));
        let formatted = res!(formatter.format_with_pattern(&calclock, &pattern));
        println!("  {} → {}: {}", pattern_str, description, formatted);
    }
    
    // Test 2: Locale-specific formatting
    println!("\n🌍 Test 2: Locale-Specific Formatting");
    
    let locales = vec![
        ("en-US", Locale::us()),
        ("en-GB", Locale::uk()),
        ("de-DE", Locale::germany()),
        ("fr-FR", Locale::france()),
        ("ja-JP", Locale::japan()),
        ("ISO", Locale::iso()),
    ];
    
    for (locale_id, locale) in locales {
        let formatted = res!(formatter.format_with_locale(&calclock, &locale));
        println!("  {}: {}", locale_id, formatted);
    }
    
    // Test 3: Custom localization
    println!("\n🎨 Test 3: Custom Localization");
    
    // Create formatter with Spanish month and day names
    let mut spanish_months = HashMap::new();
    spanish_months.insert(MonthOfYear::January, ("ene".to_string(), "enero".to_string()));
    spanish_months.insert(MonthOfYear::February, ("feb".to_string(), "febrero".to_string()));
    spanish_months.insert(MonthOfYear::March, ("mar".to_string(), "marzo".to_string()));
    spanish_months.insert(MonthOfYear::April, ("abr".to_string(), "abril".to_string()));
    spanish_months.insert(MonthOfYear::May, ("may".to_string(), "mayo".to_string()));
    spanish_months.insert(MonthOfYear::June, ("jun".to_string(), "junio".to_string()));
    spanish_months.insert(MonthOfYear::July, ("jul".to_string(), "julio".to_string()));
    spanish_months.insert(MonthOfYear::August, ("ago".to_string(), "agosto".to_string()));
    spanish_months.insert(MonthOfYear::September, ("sep".to_string(), "septiembre".to_string()));
    spanish_months.insert(MonthOfYear::October, ("oct".to_string(), "octubre".to_string()));
    spanish_months.insert(MonthOfYear::November, ("nov".to_string(), "noviembre".to_string()));
    spanish_months.insert(MonthOfYear::December, ("dic".to_string(), "diciembre".to_string()));
    
    let mut spanish_days = HashMap::new();
    spanish_days.insert(DayOfWeek::Monday, ("lun".to_string(), "lunes".to_string()));
    spanish_days.insert(DayOfWeek::Tuesday, ("mar".to_string(), "martes".to_string()));
    spanish_days.insert(DayOfWeek::Wednesday, ("mié".to_string(), "miércoles".to_string()));
    spanish_days.insert(DayOfWeek::Thursday, ("jue".to_string(), "jueves".to_string()));
    spanish_days.insert(DayOfWeek::Friday, ("vie".to_string(), "viernes".to_string()));
    spanish_days.insert(DayOfWeek::Saturday, ("sáb".to_string(), "sábado".to_string()));
    spanish_days.insert(DayOfWeek::Sunday, ("dom".to_string(), "domingo".to_string()));
    
    let spanish_formatter = CalClockFormatter::new()
        .set_month_names(spanish_months)
        .set_day_names(spanish_days)
        .set_am_pm_markers("a.m.".to_string(), "p.m.".to_string());
    
    let pattern = res!(FormatPattern::new("EEEE, d 'de' MMMM 'de' yyyy, h:mm a"));
    let spanish_formatted = res!(spanish_formatter.format_with_pattern(&calclock, &pattern));
    println!("  Spanish: {}", spanish_formatted);
    
    // Test 4: Ordinal suffixes
    println!("\n🔢 Test 4: Ordinal Suffixes");
    
    let ordinal_formatter = CalClockFormatter::new().enable_ordinals();
    
    // Test various dates to show ordinal suffixes
    let test_dates = vec![
        (2024, MonthOfYear::January, 1),
        (2024, MonthOfYear::January, 2),
        (2024, MonthOfYear::January, 3),
        (2024, MonthOfYear::January, 21),
        (2024, MonthOfYear::January, 22),
        (2024, MonthOfYear::January, 23),
        (2024, MonthOfYear::January, 31),
    ];
    
    let pattern = res!(FormatPattern::new("MMMM d, yyyy"));
    
    for (year, month, day) in test_dates {
        let date = res!(CalendarDate::from_ymd(year, month, day, zone.clone()));
        let time = res!(ClockTime::new(0, 0, 0, 0, zone.clone()));
        let clock = res!(CalClock::from_date_time(date, time));
        let formatted = res!(ordinal_formatter.format_with_pattern(&clock, &pattern));
        println!("  {}", formatted);
    }
    
    // Test 5: Week of year and era formatting
    println!("\n📆 Test 5: Week of Year and Era");
    
    let patterns_advanced = vec![
        ("'Week' w 'of' yyyy", "Week of year"),
        ("'Week' ww 'of' yyyy", "2-digit week of year"),
        ("yyyy G", "Year with era"),
        ("yyyy GGGG", "Year with full era name"),
        ("MMMM d, yyyy G", "Date with era"),
    ];
    
    for (pattern_str, description) in patterns_advanced {
        let pattern = res!(FormatPattern::new(pattern_str));
        let formatted = res!(formatter.format_with_pattern(&calclock, &pattern));
        println!("  {} → {}: {}", pattern_str, description, formatted);
    }
    
    // Test 6: Japanese calendar with eras
    println!("\n🇯🇵 Test 6: Japanese Calendar with Eras");
    
    let japanese_date = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 15, zone.clone()));
    let japanese_time = res!(ClockTime::new(0, 0, 0, 0, zone.clone()));
    let japanese_clock = res!(CalClock::from_date_time(japanese_date, japanese_time));
    
    let pattern = res!(FormatPattern::new("GGGG yyyy'年'M'月'd'日'"));
    let formatted = res!(formatter.format_with_pattern(&japanese_clock, &pattern));
    println!("  Japanese format: {}", formatted);
    
    // Test historical eras
    let historical_dates = vec![
        (1900, "Meiji era"),
        (1920, "Taishō era"),
        (1950, "Shōwa era"),
        (2000, "Heisei era"),
        (2020, "Reiwa era"),
    ];
    
    for (year, expected_era) in historical_dates {
        let date = res!(CalendarDate::from_ymd(year, MonthOfYear::January, 1, zone.clone()));
        let time = res!(ClockTime::new(0, 0, 0, 0, zone.clone()));
        let clock = res!(CalClock::from_date_time(date, time));
        let formatted = res!(formatter.format(&clock, "yyyy GGGG"));
        println!("  {}: {} ({})", year, formatted, expected_era);
    }
    
    // Test 7: Complex formatting combinations
    println!("\n🎯 Test 7: Complex Formatting");
    
    let complex_patterns = vec![
        "'Today is' EEEE, 'the' d 'of' MMMM, yyyy",
        "'Quarter' Q 'of' yyyy, 'week' w",
        "yyyy-MM-dd'T'HH:mm:ss.SSSZ '['v']'",
        "'The time is' h:mm:ss a 'on' EEEE, MMMM d",
    ];
    
    for pattern_str in complex_patterns {
        let pattern = res!(FormatPattern::new(pattern_str));
        let formatted = res!(formatter.format_with_pattern(&calclock, &pattern));
        println!("  {}", formatted);
    }
    
    println!("\n✅ Advanced Formatter Demo completed successfully!");
    println!("🎯 Key features demonstrated:");
    println!("  ✓ Multiple date/time format patterns");
    println!("  ✓ Locale-specific formatting");
    println!("  ✓ Custom month and day name localization");
    println!("  ✓ Ordinal suffixes (1st, 2nd, 3rd)");
    println!("  ✓ Week of year calculation");
    println!("  ✓ Era formatting (CE/BCE, Japanese eras)");
    println!("  ✓ Complex pattern combinations");
    
    Ok(())
}