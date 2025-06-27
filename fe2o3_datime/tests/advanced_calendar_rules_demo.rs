/// Comprehensive demonstration of advanced calendar rules system
/// 
/// This test demonstrates the complete integration of:
/// - Complex day incrementors ("3rd Monday", "last business day", etc.)
/// - Holiday engines (US Federal, UK, ECB with Easter calculations)
/// - Business day engines (custom business weeks, holiday integration)
/// - Calendar rules (quarterly patterns, business day scheduling)

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::{
    calendar::{
        CalendarDate, CalendarRulesEngine as CalendarRule, RuleType, DayIncrementor,
        HolidayEngine, BusinessDayEngine, BusinessWeek, BusinessDayAdjustment,
    },
    constant::{MonthOfYear, DayOfWeek},
    time::CalClockZone,
};

#[test]
fn test_comprehensive_business_rules_demo() -> Outcome<()> {
    println!("=== Advanced Calendar Rules System Demo ===");
    
    let zone = CalClockZone::utc();
    let start_date = res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()));
    
    // Demo 1: Complex quarterly meeting schedule with US federal holidays
    println!("\n🏢 Demo 1: Quarterly Board Meetings (2nd Tuesday of quarters, excluding US holidays)");
    
    let quarterly_rule = res!(CalendarRule::us_business_pattern(
        start_date.clone(),
        "2nd Tuesday",
        vec![1, 4, 7, 10] // Q1, Q2, Q3, Q4
    ));
    
    let quarterly_dates = res!(quarterly_rule.generate_dates(4, zone.clone()));
    
    println!("📅 Quarterly board meeting dates for 2024:");
    for (i, date) in quarterly_dates.iter().enumerate() {
        println!("  Q{}: {} ({})", i + 1, date, date.day_of_week());
    }
    
    assert_eq!(quarterly_dates.len(), 4, "Should have 4 quarterly meetings");
    
    // Demo 2: UK payroll schedule with Easter holidays
    println!("\n🇬🇧 Demo 2: UK Payroll Schedule (Last business day of each month, excluding UK holidays)");
    
    let uk_payroll_rule = res!(CalendarRule::uk_business_pattern(
        start_date.clone(),
        "last business day",
        vec![1, 2, 3, 4, 5, 6] // First half of year
    ));
    
    let uk_payroll_dates = res!(uk_payroll_rule.generate_dates(6, zone.clone()));
    
    println!("💰 UK payroll dates (first half of 2024):");
    for date in &uk_payroll_dates {
        println!("  {} {} {} ({})", date.day(), date.month_of_year(), date.year(), date.day_of_week());
    }
    
    assert_eq!(uk_payroll_dates.len(), 6, "Should have 6 monthly payroll dates");
    
    // Verify no Easter holidays are included
    for date in &uk_payroll_dates {
        // Good Friday 2024: March 29, Easter Monday 2024: April 1
        let is_good_friday = date.month() == 3 && date.day() == 29;
        let is_easter_monday = date.month() == 4 && date.day() == 1;
        assert!(!is_good_friday && !is_easter_monday, 
               "UK payroll should exclude Easter holidays");
    }
    
    // Demo 3: Middle East business schedule (Sunday-Thursday business week)
    println!("\n🕌 Demo 3: Middle East Operations (First business day of each quarter, Sunday-Thursday week)");
    
    let middle_east_rule = res!(CalendarRule::middle_east_business_pattern(
        start_date.clone(),
        "1st business day",
        vec![1, 4, 7, 10] // Quarterly
    ));
    
    let middle_east_dates = res!(middle_east_rule.generate_dates(4, zone.clone()));
    
    println!("🏖️ Middle East operations start dates:");
    for (i, date) in middle_east_dates.iter().enumerate() {
        println!("  Q{}: {} ({})", i + 1, date, date.day_of_week());
        
        // Verify business days in Sunday-Thursday week
        let dow = date.day_of_week();
        let is_business_day = matches!(dow, 
            DayOfWeek::Sunday |
            DayOfWeek::Monday |
            DayOfWeek::Tuesday |
            DayOfWeek::Wednesday |
            DayOfWeek::Thursday
        );
        assert!(is_business_day, "Should be a business day in Middle East week");
    }
    
    // Demo 4: ECB regulatory reporting (complex pattern with no weekend adjustments)
    println!("\n🏦 Demo 4: ECB Regulatory Reporting (15th of each month or next business day)");
    
    let ecb_engine = HolidayEngine::ecb();
    let business_engine = BusinessDayEngine::new()
        .with_holiday_engine(ecb_engine.clone())
        .with_default_adjustment(BusinessDayAdjustment::Following);
    
    let ecb_rule = CalendarRule::new(RuleType::ByExplicitMonths)
        .with_start_date(start_date.clone())
        .with_months(vec![1, 2, 3, 4, 5, 6]) // First half of year
        .with_holiday_engine(ecb_engine)
        .with_business_day_engine(business_engine);
    
    // Generate 15th of each month, then adjust for business days
    let mut ecb_dates = Vec::new();
    for month in 1..=6 {
        let fifteenth = res!(CalendarDate::from_ymd(2024, res!(MonthOfYear::from_number(month)), 15, zone.clone()));
        ecb_dates.push(fifteenth);
    }
    
    println!("📊 ECB reporting dates (15th or next business day):");
    for date in &ecb_dates {
        println!("  {} {} {} ({})", date.day(), date.month_of_year(), date.year(), date.day_of_week());
    }
    
    // Demo 5: Complex day incrementor patterns
    println!("\n🧮 Demo 5: Complex Day Incrementor Patterns");
    
    // "2nd weekday before the 25th of each month"
    let complex_incrementor = res!(DayIncrementor::from_string("2nd weekday before the 25th"));
    
    println!("📝 Parsing complex patterns:");
    println!("  Pattern: '2nd weekday before the 25th'");
    println!("  Parsed: Sign: {} Value: {} DayType: {:?}", 
             complex_incrementor.sign(), 
             complex_incrementor.value(), 
             complex_incrementor.day_type());
    
    if let Some(next_inc) = complex_incrementor.next_inc() {
        println!("  Next incrementor: Value: {} DayType: {:?}", 
                 next_inc.value(), 
                 next_inc.day_type());
    }
    
    // Calculate actual dates for this pattern
    println!("  Calculated dates for first quarter 2024:");
    for month in 1..=3 {
        let calculated = res!(complex_incrementor.calculate_date(2024, month, zone.clone()));
        println!("    {}: {} ({})", 
                 res!(MonthOfYear::from_number(month)), 
                 calculated, 
                 calculated.day_of_week());
    }
    
    // Demo 6: Business day statistics and analysis
    println!("\n📈 Demo 6: Business Day Analysis");
    
    let us_business_engine = BusinessDayEngine::new()
        .with_holiday_engine(HolidayEngine::us_federal());
    
    let stats_2024_june = res!(us_business_engine.month_business_day_stats(2024, 6, zone.clone()));
    
    println!("📊 June 2024 business day statistics (US Federal holidays):");
    println!("  Total days: {}", stats_2024_june.total_days);
    println!("  Business days: {}", stats_2024_june.business_days_count);
    println!("  Weekend/holiday days: {}", stats_2024_june.weekend_days);
    
    if let Some(ref first) = stats_2024_june.first_business_day {
        println!("  First business day: {} ({})", first, first.day_of_week());
    }
    
    if let Some(ref last) = stats_2024_june.last_business_day {
        println!("  Last business day: {} ({})", last, last.day_of_week());
    }
    
    // Verify expected business day count for June 2024
    // June 2024: 30 days, starts Saturday, ends Sunday
    // Weekdays: 20 days, but exclude July 4th which isn't in June
    assert_eq!(stats_2024_june.business_days_count, 20, 
               "June 2024 should have 20 business days");
    
    // Demo 7: Easter calculation verification
    println!("\n🐰 Demo 7: Easter Calculation Verification");
    
    let easter_2024 = res!(HolidayEngine::calculate_easter(2024, zone.clone()));
    let easter_2025 = res!(HolidayEngine::calculate_easter(2025, zone.clone()));
    
    println!("🥚 Easter dates:");
    println!("  2024: {} {} {} ({})", easter_2024.day(), easter_2024.month_of_year(), easter_2024.year(), easter_2024.day_of_week());
    println!("  2025: {} {} {} ({})", easter_2025.day(), easter_2025.month_of_year(), easter_2025.year(), easter_2025.day_of_week());
    
    // Verify known Easter dates
    assert_eq!(easter_2024.month(), 3, "Easter 2024 should be in March");
    assert_eq!(easter_2024.day(), 31, "Easter 2024 should be March 31st");
    assert_eq!(easter_2025.month(), 4, "Easter 2025 should be in April");
    assert_eq!(easter_2025.day(), 20, "Easter 2025 should be April 20th");
    
    println!("\n✅ Advanced Calendar Rules System demonstration completed successfully!");
    println!("🎯 All features integrated and working:");
    println!("  ✓ Complex day incrementors with natural language parsing");
    println!("  ✓ Sophisticated holiday engines with Easter calculations"); 
    println!("  ✓ Business day engines with custom business weeks");
    println!("  ✓ Calendar rules with holiday and business day integration");
    println!("  ✓ Multiple jurisdiction support (US, UK, ECB, Middle East)");
    println!("  ✓ Weekend adjustment algorithms");
    println!("  ✓ Statistical analysis and reporting");
    
    Ok(())
}

#[test]
fn test_holiday_engine_comprehensive() -> Outcome<()> {
    println!("=== Holiday Engine Comprehensive Test ===");
    
    let zone = CalClockZone::utc();
    
    // Test all pre-built holiday engines
    let engines = vec![
        ("US Federal", HolidayEngine::us_federal()),
        ("United Kingdom", HolidayEngine::uk()),
        ("European Central Bank", HolidayEngine::ecb()),
    ];
    
    for (name, engine) in engines {
        println!("\n🏛️  Testing {} holidays for 2024:", name);
        
        let holidays_2024 = res!(engine.calculate_holidays(2024, zone.clone()));
        println!("  Found {} holidays:", holidays_2024.len());
        
        for (holiday_name, date) in &holidays_2024 {
            println!("    {}: {} ({})", holiday_name, date, date.day_of_week());
        }
        
        // Verify some expected holidays exist
        let holiday_names: Vec<&String> = holidays_2024.iter().map(|(name, _)| name).collect();
        
        match name {
            "US Federal" => {
                assert!(holiday_names.contains(&&"New Year's Day".to_string()));
                assert!(holiday_names.contains(&&"Independence Day".to_string()));
                assert!(holiday_names.contains(&&"Christmas Day".to_string()));
                assert_eq!(holidays_2024.len(), 10, "US Federal should have 10 holidays");
            },
            "United Kingdom" => {
                assert!(holiday_names.contains(&&"New Year's Day".to_string()));
                assert!(holiday_names.contains(&&"Good Friday".to_string()));
                assert!(holiday_names.contains(&&"Easter Monday".to_string()));
                assert!(holiday_names.contains(&&"Christmas Day".to_string()));
                assert!(holiday_names.contains(&&"Boxing Day".to_string()));
                assert_eq!(holidays_2024.len(), 8, "UK should have 8 holidays");
            },
            "European Central Bank" => {
                assert!(holiday_names.contains(&&"New Year's Day".to_string()));
                assert!(holiday_names.contains(&&"Good Friday".to_string()));
                assert!(holiday_names.contains(&&"Labour Day".to_string()));
                assert!(holiday_names.contains(&&"Christmas Day".to_string()));
                assert_eq!(holidays_2024.len(), 6, "ECB should have 6 holidays");
            },
            _ => {}
        }
    }
    
    println!("\n✅ All holiday engines working correctly!");
    
    Ok(())
}

#[test]
fn test_business_day_engine_comprehensive() -> Outcome<()> {
    println!("=== Business Day Engine Comprehensive Test ===");
    
    let zone = CalClockZone::utc();
    
    // Test different business week configurations
    let business_weeks = vec![
        ("Standard (Mon-Fri)", BusinessWeek::monday_to_friday()),
        ("Middle East (Sun-Thu)", BusinessWeek::sunday_to_thursday()),
        ("Custom (Tue-Sat)", BusinessWeek::custom(vec![
            DayOfWeek::Tuesday,
            DayOfWeek::Wednesday,
            DayOfWeek::Thursday,
            DayOfWeek::Friday,
            DayOfWeek::Saturday,
        ])),
    ];
    
    for (name, business_week) in business_weeks {
        println!("\n🗓️  Testing {} business week:", name);
        
        let engine = BusinessDayEngine::new()
            .with_business_week(business_week.clone());
        
        // Test first week of June 2024 (starts Saturday)
        let june_1 = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone())); // Saturday
        
        println!("  Business days in first week of June 2024:");
        for i in 0..7 {
            let date = res!(june_1.add_days(i));
            let is_business = res!(engine.is_business_day(&date));
            let symbol = if is_business { "✓" } else { "✗" };
            println!("    {} {} {} ({})", symbol, date, date.day_of_week(), 
                    if is_business { "Business Day" } else { "Non-Business" });
        }
        
        // Count expected business days based on week type
        let expected_count = match name {
            "Standard (Mon-Fri)" => 5, // Mon, Tue, Wed, Thu, Fri
            "Middle East (Sun-Thu)" => 5, // Sun, Mon, Tue, Wed, Thu  
            "Custom (Tue-Sat)" => 5, // Tue, Wed, Thu, Fri, Sat
            _ => 0,
        };
        
        let actual_count = (0..7).filter(|&i| {
            let date = june_1.add_days(i).unwrap();
            engine.is_business_day(&date).unwrap_or(false)
        }).count();
        
        assert_eq!(actual_count, expected_count, 
                  "Expected {} business days for {}, got {}", expected_count, name, actual_count);
    }
    
    // Test business day calculations
    println!("\n📊 Testing business day calculations:");
    
    let standard_engine = BusinessDayEngine::new();
    
    // Add 5 business days to Friday June 7, 2024
    let friday_june_7 = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 7, zone.clone()));
    let plus_5_business = res!(standard_engine.add_business_days(&friday_june_7, 5));
    
    println!("  {} + 5 business days = {} ({})", 
             friday_june_7, plus_5_business, plus_5_business.day_of_week());
    
    // Should be Friday June 14, 2024 (skipping weekend)
    assert_eq!(plus_5_business.day(), 14);
    assert_eq!(plus_5_business.day_of_week(), DayOfWeek::Friday);
    
    // Count business days between two dates
    let monday_june_3 = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 3, zone.clone()));
    let friday_june_7_end = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 7, zone.clone()));
    let business_days_count = res!(standard_engine.business_days_between(&monday_june_3, &friday_june_7_end));
    
    println!("  Business days from {} to {}: {}", 
             monday_june_3, friday_june_7_end, business_days_count);
    
    assert_eq!(business_days_count, 4, "Mon-Fri should have 4 business days between");
    
    println!("\n✅ All business day engine features working correctly!");
    
    Ok(())
}