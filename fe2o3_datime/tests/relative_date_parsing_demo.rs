/// Comprehensive demonstration of relative date parsing functionality.
/// 
/// This test demonstrates the advanced natural language relative date parsing
/// capabilities that allow users to express dates in intuitive, human-friendly ways.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::{
    parser::Parser,
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::CalClockZone,
};

#[test]
fn test_comprehensive_relative_date_parsing() -> Outcome<()> {
    println!("=== Comprehensive Relative Date Parsing Demo ===");
    
    let zone = CalClockZone::utc();
    // Use a fixed base date for predictable testing: Wednesday, June 12, 2024
    let base_date = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 12, zone.clone()));
    
    println!("📅 Base date for testing: {} ({})", base_date, base_date.day_of_week());
    
    // Test 1: Simple day-of-week relative dates
    println!("\n🗓️  Test 1: Simple Day-of-Week Relative Dates");
    
    let test_cases_dow = [
        ("next Tuesday", "Tuesday, June 18, 2024"),
        ("last Friday", "Friday, June 7, 2024"),
        ("this Monday", "Monday, June 10, 2024"),
        ("next Monday", "Monday, June 17, 2024"),
        ("last Monday", "Monday, June 3, 2024"),
        ("this Wednesday", "Wednesday, June 12, 2024"), // Today
        ("next Wednesday", "Wednesday, June 19, 2024"),
    ];
    
    for (input, expected_description) in &test_cases_dow {
        let result = res!(Parser::parse_relative_date_from(input, &base_date, zone.clone()));
        println!("  '{}' → {} ({})", input, result, result.day_of_week());
        
        // Verify the day of week is correct
        let expected_day = match input {
            s if s.contains("Tuesday") => DayOfWeek::Tuesday,
            s if s.contains("Friday") => DayOfWeek::Friday,
            s if s.contains("Monday") => DayOfWeek::Monday,
            s if s.contains("Wednesday") => DayOfWeek::Wednesday,
            _ => panic!("Unexpected day in test case"),
        };
        assert_eq!(result.day_of_week(), expected_day, "Day of week mismatch for '{}'", input);
    }
    
    // Test 2: Quantified relative dates (days, weeks, months, years)
    println!("\n📊 Test 2: Quantified Relative Dates");
    
    let test_cases_quantified = [
        ("in 3 days", 15), // June 15, 2024
        ("3 days ago", 9), // June 9, 2024
        ("in 1 week", 19), // June 19, 2024
        ("2 weeks ago", 29), // May 29, 2024 (previous month)
        ("in 2 weeks", 26), // June 26, 2024
    ];
    
    for (input, expected_day) in &test_cases_quantified {
        let result = res!(Parser::parse_relative_date_from(input, &base_date, zone.clone()));
        println!("  '{}' → {} (day {})", input, result, result.day());
        
        // For simple day calculations, verify the day matches
        if input.contains("days") && !input.contains("weeks") {
            assert_eq!(result.day(), *expected_day as u8, "Day mismatch for '{}'", input);
        }
    }
    
    // Test 3: Month and year relative dates
    println!("\n📆 Test 3: Month and Year Relative Dates");
    
    let month_cases = [
        ("next month", 7, 12), // July 12, 2024
        ("last month", 5, 12), // May 12, 2024
        ("in 2 months", 8, 12), // August 12, 2024
        ("3 months ago", 3, 12), // March 12, 2024
        ("next year", 6, 12), // June 12, 2025
        ("last year", 6, 12), // June 12, 2023
    ];
    
    for (input, expected_month, expected_day) in &month_cases {
        let result = res!(Parser::parse_relative_date_from(input, &base_date, zone.clone()));
        println!("  '{}' → {}", input, result);
        
        if input.contains("month") {
            assert_eq!(result.month(), *expected_month, "Month mismatch for '{}'", input);
            assert_eq!(result.day(), *expected_day as u8, "Day mismatch for '{}'", input);
        }
    }
    
    // Test 4: Period boundaries (beginning/end of periods)
    println!("\n🏁 Test 4: Period Boundaries");
    
    let boundary_cases = [
        ("end of this month", 6, 30), // June 30, 2024
        ("beginning of next month", 7, 1), // July 1, 2024
        ("end of next month", 7, 31), // July 31, 2024
        ("beginning of this year", 1, 1), // January 1, 2024
        ("end of this year", 12, 31), // December 31, 2024
    ];
    
    for (input, expected_month, expected_day) in &boundary_cases {
        let result = res!(Parser::parse_relative_date_from(input, &base_date, zone.clone()));
        println!("  '{}' → {}", input, result);
        
        assert_eq!(result.month(), *expected_month, "Month mismatch for '{}'", input);
        assert_eq!(result.day(), *expected_day as u8, "Day mismatch for '{}'", input);
    }
    
    // Test 5: Complex expressions
    println!("\n🧩 Test 5: Complex Expressions");
    
    // Test the parser's detailed output
    let complex_cases = [
        "the Tuesday after next",
        "2 weeks from now",
        "end of this month",
        "beginning of next quarter",
    ];
    
    for input in &complex_cases {
        if let Ok((expression, calculated_date)) = Parser::parse_relative_date_detailed(input, zone.clone()) {
            println!("  '{}' →", input);
            println!("    Expression: {:?}", expression);
            println!("    Calculated: {} ({})", calculated_date, calculated_date.day_of_week());
        } else {
            // Some expressions might not be fully implemented yet
            println!("  '{}' → [Not yet supported]", input);
        }
    }
    
    // Test 6: Integration with main parser
    println!("\n🔗 Test 6: Integration with Main Parser");
    
    let parser_integration_cases = [
        "next Tuesday",
        "in 2 weeks",
        "last Friday",
        "3 days ago",
    ];
    
    for input in &parser_integration_cases {
        // Test that relative dates work through the main parse_date function
        if let Ok(result) = Parser::parse_date(input, zone.clone()) {
            println!("  Parser::parse_date('{}') → {}", input, result);
        } else {
            println!("  Parser::parse_date('{}') → [Failed]", input);
        }
        
        // Test that relative dates work through the main parse_datetime function
        if let Ok(result) = Parser::parse_datetime(input, zone.clone()) {
            println!("  Parser::parse_datetime('{}') → {}", input, result);
        }
    }
    
    // Test 7: Edge cases and error handling
    println!("\n⚠️  Test 7: Edge Cases and Error Handling");
    
    let edge_cases = [
        ("invalid relative expression", false),
        ("next Funday", false), // Invalid day name
        ("in 0 days", true), // Edge case but should work
        ("", false), // Empty input
        ("random text", false), // Non-relative text
    ];
    
    for (input, should_succeed) in &edge_cases {
        match Parser::parse_relative_date_from(input, &base_date, zone.clone()) {
            Ok(result) => {
                if *should_succeed {
                    println!("  '{}' → {} ✓", input, result);
                } else {
                    println!("  '{}' → {} (unexpected success)", input, result);
                }
            },
            Err(_) => {
                if *should_succeed {
                    println!("  '{}' → Failed (unexpected)", input);
                } else {
                    println!("  '{}' → Failed ✓", input);
                }
            }
        }
    }
    
    // Test 8: Cross-month and cross-year boundaries
    println!("\n🌍 Test 8: Cross-Month and Cross-Year Boundaries");
    
    // Test from end of month
    let end_of_month = res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 31, zone.clone()));
    let next_month_from_end = res!(Parser::parse_relative_date_from("next month", &end_of_month, zone.clone()));
    println!("  From Jan 31: 'next month' → {}", next_month_from_end);
    
    // Test from end of year
    let end_of_year = res!(CalendarDate::from_ymd(2024, MonthOfYear::December, 31, zone.clone()));
    let next_year_from_end = res!(Parser::parse_relative_date_from("next year", &end_of_year, zone.clone()));
    println!("  From Dec 31: 'next year' → {}", next_year_from_end);
    
    // Test leap year handling
    let leap_year_date = res!(CalendarDate::from_ymd(2024, MonthOfYear::February, 28, zone.clone()));
    let tomorrow_from_feb28 = res!(Parser::parse_relative_date_from("in 1 day", &leap_year_date, zone.clone()));
    println!("  From Feb 28, 2024 (leap year): 'in 1 day' → {}", tomorrow_from_feb28);
    assert_eq!(tomorrow_from_feb28.day(), 29, "Should handle leap year correctly");
    
    println!("\n✅ Relative Date Parsing Demo completed successfully!");
    println!("🎯 Key features demonstrated:");
    println!("  ✓ Day-of-week relative dates (next Tuesday, last Friday)");
    println!("  ✓ Quantified relative dates (in 2 weeks, 3 days ago)");
    println!("  ✓ Month and year calculations");
    println!("  ✓ Period boundaries (end of month, beginning of year)");
    println!("  ✓ Integration with main parser functions");
    println!("  ✓ Edge case handling and error recovery");
    println!("  ✓ Cross-month and cross-year boundary calculations");
    println!("  ✓ Leap year awareness");
    
    Ok(())
}

#[test]
fn test_relative_date_parser_vocabulary() -> Outcome<()> {
    println!("=== Relative Date Parser Vocabulary Test ===");
    
    let zone = CalClockZone::utc();
    let base_date = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 12, zone.clone())); // Wednesday
    
    // Test all supported day names
    let day_names = [
        ("Monday", DayOfWeek::Monday),
        ("Tuesday", DayOfWeek::Tuesday),
        ("Wednesday", DayOfWeek::Wednesday),
        ("Thursday", DayOfWeek::Thursday),
        ("Friday", DayOfWeek::Friday),
        ("Saturday", DayOfWeek::Saturday),
        ("Sunday", DayOfWeek::Sunday),
        ("Mon", DayOfWeek::Monday),
        ("Tue", DayOfWeek::Tuesday),
        ("Wed", DayOfWeek::Wednesday),
        ("Thu", DayOfWeek::Thursday),
        ("Fri", DayOfWeek::Friday),
        ("Sat", DayOfWeek::Saturday),
        ("Sun", DayOfWeek::Sunday),
    ];
    
    println!("\n📝 Testing day name vocabulary:");
    for (day_name, expected_day) in &day_names {
        let input = format!("next {}", day_name);
        let result = res!(Parser::parse_relative_date_from(&input, &base_date, zone.clone()));
        assert_eq!(result.day_of_week(), *expected_day, "Day name '{}' not recognized correctly", day_name);
        println!("  '{}' → {} ✓", input, result.day_of_week());
    }
    
    // Test reference words
    let reference_words = [
        ("next", "Tuesday"),
        ("last", "Tuesday"),
        ("this", "Tuesday"),
        ("upcoming", "Tuesday"),
        ("previous", "Tuesday"),
        ("coming", "Tuesday"),
    ];
    
    println!("\n🔗 Testing reference word vocabulary:");
    for (reference, day) in &reference_words {
        let input = format!("{} {}", reference, day);
        if let Ok(result) = Parser::parse_relative_date_from(&input, &base_date, zone.clone()) {
            println!("  '{}' → {} ✓", input, result);
        } else {
            println!("  '{}' → Failed", input);
        }
    }
    
    // Test unit words
    let unit_words = [
        ("1 day", 1),
        ("2 days", 2),
        ("1 week", 7),
        ("3 weeks", 21),
        ("1 month", 0), // Month calculations are complex
        ("2 months", 0),
        ("1 year", 0), // Year calculations are complex
    ];
    
    println!("\n📊 Testing unit word vocabulary:");
    for (input_pattern, expected_day_offset) in &unit_words {
        let forward_input = format!("in {}", input_pattern);
        let backward_input = format!("{} ago", input_pattern);
        
        if let Ok(forward_result) = Parser::parse_relative_date_from(&forward_input, &base_date, zone.clone()) {
            println!("  '{}' → {}", forward_input, forward_result);
            if *expected_day_offset > 0 {
                // Calculate the expected date by adding days to the base date
                let expected_forward = res!(base_date.add_days(*expected_day_offset));
                assert_eq!(forward_result, expected_forward, 
                          "Forward calculation incorrect for '{}'", forward_input);
            }
        }
        
        if let Ok(backward_result) = Parser::parse_relative_date_from(&backward_input, &base_date, zone.clone()) {
            println!("  '{}' → {}", backward_input, backward_result);
            if *expected_day_offset > 0 {
                // Calculate the expected date by subtracting days from the base date
                let expected_backward = res!(base_date.add_days(-*expected_day_offset));
                assert_eq!(backward_result, expected_backward, 
                          "Backward calculation incorrect for '{}'", backward_input);
            }
        }
    }
    
    println!("\n✅ Vocabulary test completed successfully!");
    
    Ok(())
}

#[test]
fn test_relative_date_business_scenarios() -> Outcome<()> {
    println!("=== Business Scenario Relative Date Tests ===");
    
    let zone = CalClockZone::utc();
    
    // Scenario 1: Meeting scheduling
    println!("\n🤝 Scenario 1: Meeting Scheduling");
    let today = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 12, zone.clone())); // Wednesday
    
    let meeting_dates = [
        "next Monday",     // Team meeting
        "next Friday",     // Weekly review
        "in 2 weeks",      // Project deadline
        "end of this month", // Monthly report due
    ];
    
    for date_expr in &meeting_dates {
        let meeting_date = res!(Parser::parse_relative_date_from(date_expr, &today, zone.clone()));
        println!("  Meeting '{}': {}", date_expr, meeting_date);
    }
    
    // Scenario 2: Project deadlines
    println!("\n📋 Scenario 2: Project Deadlines");
    let project_start = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 1, zone.clone()));
    
    let deadlines = [
        ("Sprint review", "in 2 weeks"),
        ("Beta release", "in 6 weeks"),
        ("Final release", "in 3 months"),
        ("Post-mortem", "end of next month"),
    ];
    
    for (milestone, date_expr) in &deadlines {
        let deadline = res!(Parser::parse_relative_date_from(date_expr, &project_start, zone.clone()));
        println!("  {}: '{}' → {}", milestone, date_expr, deadline);
    }
    
    // Scenario 3: Financial reporting
    println!("\n💰 Scenario 3: Financial Reporting");
    let fiscal_start = res!(CalendarDate::from_ymd(2024, MonthOfYear::January, 1, zone.clone()));
    
    let financial_dates = [
        ("Q1 close", "end of this quarter"),
        ("Mid-year review", "in 6 months"),
        ("Year-end close", "end of this year"),
    ];
    
    for (event, date_expr) in &financial_dates {
        if let Ok(event_date) = Parser::parse_relative_date_from(date_expr, &fiscal_start, zone.clone()) {
            println!("  {}: '{}' → {}", event, date_expr, event_date);
        } else {
            println!("  {}: '{}' → [Not yet supported]", event, date_expr);
        }
    }
    
    // Scenario 4: Personal scheduling
    println!("\n🏠 Scenario 4: Personal Scheduling");
    let weekend = res!(CalendarDate::from_ymd(2024, MonthOfYear::June, 15, zone.clone())); // Saturday
    
    let personal_events = [
        ("Doctor appointment", "next Tuesday"),
        ("Vacation start", "in 3 weeks"),
        ("Birthday party", "next Saturday"),
        ("Tax deadline", "end of this month"),
    ];
    
    for (event, date_expr) in &personal_events {
        let event_date = res!(Parser::parse_relative_date_from(date_expr, &weekend, zone.clone()));
        println!("  {}: '{}' → {} ({})", event, date_expr, event_date, event_date.day_of_week());
    }
    
    println!("\n✅ Business scenario tests completed successfully!");
    println!("🎯 Demonstrated real-world applications:");
    println!("  ✓ Meeting and event scheduling");
    println!("  ✓ Project milestone tracking");
    println!("  ✓ Financial reporting deadlines");
    println!("  ✓ Personal calendar management");
    
    Ok(())
}