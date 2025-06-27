/// Comprehensive demonstration of the advanced validation framework features.
/// 
/// This test showcases all the new advanced validation capabilities including:
/// - Performance analytics and metrics
/// - High-performance caching
/// - Conditional validation rules
/// - Parallel batch processing
/// - Validation profiles and registry
use oxedyne_fe2o3_datime::prelude::*;
use oxedyne_fe2o3_core::prelude::*;


#[test]
fn test_validation_analytics() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let mut analytics = ValidationAnalytics::new();
    
    // Create some test data
    let valid_calclock = res!(CalClock::new(2024, 6, 15, 10, 30, 0, 0, zone.clone()));
    let validator = CalClockValidator::new();
    
    // Track some specific validation results
    analytics.track_validation_result(
        &validator.validate_calclock(&valid_calclock),
        "test_rule",
        std::time::Duration::from_millis(5)
    );
    
    // Get analytics metrics
    let metrics = analytics.get_metrics();
    assert_eq!(metrics.total_validations, 1);
    assert!(metrics.success_rate() > 0.0);
    
    // Generate a report
    let report = analytics.generate_report();
    println!("Analytics Report:\n{}", report.format());
    
    Ok(())
}

#[test]
fn test_cached_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let base_validator = CalClockValidator::new();
    let mut cached_validator = CachedValidator::new(base_validator);
    
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone.clone()));
    
    // First validation - will be computed and cached
    let result1 = cached_validator.validate_calclock(&calclock);
    assert!(result1.is_ok());
    
    // Second validation - should come from cache
    let result2 = cached_validator.validate_calclock(&calclock);
    assert!(result2.is_ok());
    
    // Check cache statistics
    let stats = cached_validator.cache_stats();
    assert_eq!(stats.total_operations(), 2);
    assert!(stats.hit_rate() > 0.0); // Should have at least one cache hit
    
    println!("Cache Statistics:\n{}", stats.format());
    
    Ok(())
}

#[test]
fn test_conditional_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create a conditional rule: extended hours on weekends, business hours on weekdays
    let conditional_rule = ConditionalRule::new("weekend_extended_hours")
        .condition(ValidationCondition::IsWeekend)
        .if_true(ValidationRules::hour_range(8, 22))  // 8 AM - 10 PM on weekends
        .if_false(ValidationRules::hour_range(9, 17)) // 9 AM - 5 PM on weekdays
        .into_rule();
    
    let mut validator = CalClockValidator::new();
    validator.add_rule(conditional_rule);
    
    // Test weekend time (should allow extended hours)
    let weekend_time = res!(CalClock::new(2024, 6, 16, 20, 0, 0, 0, zone.clone())); // Sunday 8 PM
    let weekend_result = validator.validate_calclock(&weekend_time);
    println!("Weekend (Sunday 8 PM) validation result: {:?}", weekend_result);
    assert!(weekend_result.is_ok(), "Weekend extended hours should be allowed");
    
    // Test weekday time outside business hours (should fail)
    let weekday_time = res!(CalClock::new(2024, 6, 17, 20, 0, 0, 0, zone.clone())); // Monday 8 PM
    let weekday_result = validator.validate_calclock(&weekday_time);
    println!("Weekday (Monday 8 PM) validation result: {:?}", weekday_result);
    assert!(weekday_result.is_err(), "Weekday late hours should be rejected");
    
    Ok(())
}

#[test]
fn test_parallel_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let validator = CalClockValidator::new();
    let parallel_validator = ParallelValidator::new(validator, 4); // 4 threads
    
    // Create a large batch of CalClocks to validate
    let mut calclocks = Vec::new();
    for hour in 0..24 {
        for minute in [0, 15, 30, 45] {
            calclocks.push(res!(CalClock::new(2024, 6, 15, hour, minute, 0, 0, zone.clone())));
        }
    }
    
    // Validate in parallel
    let results = parallel_validator.validate_batch(&calclocks);
    
    assert_eq!(results.total_items, calclocks.len());
    assert!(results.all_valid(), "All basic CalClocks should be valid");
    assert!(results.throughput() > 0.0, "Should have positive throughput");
    
    println!("Parallel Validation Results:\n{}", results.format());
    
    Ok(())
}

#[test]
fn test_validation_profiles() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create a business scheduling profile
    let business_profile = ProfileBuilder::new("business_scheduling")
        .description("Standard business scheduling validation")
        .tag("business")
        .business_hours_only()
        .no_weekends()
        .whole_minutes_only()
        .build();
    
    // Create validator from profile
    let validator = business_profile.create_validator();
    
    // Test valid business time
    let business_time = res!(CalClock::new(2024, 6, 17, 10, 30, 0, 0, zone.clone())); // Monday 10:30 AM
    assert!(validator.validate_calclock(&business_time).is_ok());
    
    // Test invalid weekend time
    let weekend_time = res!(CalClock::new(2024, 6, 16, 10, 30, 0, 0, zone.clone())); // Sunday 10:30 AM
    assert!(validator.validate_calclock(&weekend_time).is_err());
    
    Ok(())
}

#[test]
fn test_profile_registry() -> Outcome<()> {
    let mut registry = ProfileRegistry::new();
    
    // Register standard profiles
    res!(registry.register(StandardProfiles::business_scheduling()));
    res!(registry.register(StandardProfiles::appointment_booking()));
    res!(registry.register(StandardProfiles::historical_data()));
    
    // Check registry contents
    assert_eq!(registry.len(), 3);
    assert!(registry.get("business_scheduling").is_some());
    assert!(registry.get("nonexistent").is_none());
    
    // Find profiles by tag
    let business_profiles = registry.find_by_tag("business");
    assert!(!business_profiles.is_empty());
    
    println!("Registered profiles: {:?}", registry.list_names());
    
    Ok(())
}

#[test]
fn test_complex_conditional_rules() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create complex conditional logic: maintenance window detection
    let maintenance_condition = ValidationCondition::And(vec![
        ValidationCondition::IsDayOfWeek(DayOfWeek::Sunday),
        ValidationCondition::IsHourInRange(2, 4), // 2-4 AM
    ]);
    
    let maintenance_rule = ConditionalRule::new("maintenance_window")
        .condition(maintenance_condition)
        .if_true(ValidationRule::new("block_maintenance").with_calclock_validator(|_| {
            Err(vec![ValidationError::new(
                "maintenance_window",
                "System maintenance window - operations blocked"
            )])
        }))
        .into_rule();
    
    let mut validator = CalClockValidator::new();
    validator.add_rule(maintenance_rule);
    
    // Test during maintenance window
    let maintenance_time = res!(CalClock::new(2024, 6, 16, 3, 0, 0, 0, zone.clone())); // Sunday 3 AM
    let maintenance_result = validator.validate_calclock(&maintenance_time);
    assert!(maintenance_result.is_err(), "Maintenance window should block operations");
    
    // Test outside maintenance window
    let normal_time = res!(CalClock::new(2024, 6, 16, 10, 0, 0, 0, zone.clone())); // Sunday 10 AM
    let normal_result = validator.validate_calclock(&normal_time);
    assert!(normal_result.is_ok(), "Normal hours should be allowed");
    
    Ok(())
}

#[test]
fn test_validation_performance_comparison() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create test data
    let mut calclocks = Vec::new();
    for i in 0..1000u32 {
        let hour = (i % 24) as u8;
        let minute = ((i * 5) % 60) as u8;
        calclocks.push(res!(CalClock::new(2024, 6, 15, hour, minute, 0, 0, zone.clone())));
    }
    
    // Test regular validation
    let validator = CalClockValidator::new();
    let start = std::time::Instant::now();
    for calclock in &calclocks {
        let _ = validator.validate_calclock(calclock);
    }
    let regular_duration = start.elapsed();
    
    // Test cached validation
    let base_validator = CalClockValidator::new();
    let mut cached_validator = CachedValidator::new(base_validator);
    let start = std::time::Instant::now();
    for calclock in &calclocks {
        let _ = cached_validator.validate_calclock(calclock);
    }
    let cached_duration = start.elapsed();
    
    // Test parallel validation
    let parallel_validator = ParallelValidator::new(CalClockValidator::new(), 4);
    let start = std::time::Instant::now();
    let _results = parallel_validator.validate_batch(&calclocks);
    let parallel_duration = start.elapsed();
    
    println!("Performance Comparison:");
    println!("Regular validation: {:?}", regular_duration);
    println!("Cached validation: {:?}", cached_duration);
    println!("Parallel validation: {:?}", parallel_duration);
    
    // Cache should improve performance for repeated validations
    let cache_stats = cached_validator.cache_stats();
    println!("Cache hit rate: {:.1}%", cache_stats.hit_rate() * 100.0);
    
    Ok(())
}

#[test]
fn test_comprehensive_validation_scenario() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create a comprehensive validation setup combining multiple advanced features
    let mut analytics = ValidationAnalytics::new();
    
    // Create a sophisticated validation profile
    let profile = ProfileBuilder::new("enterprise_scheduling")
        .description("Enterprise-grade scheduling validation")
        .tag("enterprise")
        .tag("production")
        .conditional_business_hours()
        .seasonal_hours()
        .whole_minutes_only()
        .build();
    
    // Create cached validator from profile
    let base_validator = profile.create_validator();
    let mut cached_validator = CachedValidator::new(base_validator);
    
    // Test data representing various scheduling scenarios
    let test_scenarios = vec![
        // Valid business scenarios
        res!(CalClock::new(2024, 6, 17, 10, 30, 0, 0, zone.clone())), // Monday 10:30 AM
        res!(CalClock::new(2024, 7, 15, 14, 0, 0, 0, zone.clone())),  // Summer business hours
        res!(CalClock::new(2024, 6, 16, 10, 30, 0, 0, zone.clone())), // Sunday 10:30 AM (weekend hours 10-14)
        
        // Invalid scenarios
        res!(CalClock::new(2024, 6, 17, 22, 0, 0, 0, zone.clone())),  // Monday 10 PM (too late)
        res!(CalClock::new(2024, 6, 17, 10, 30, 30, 0, zone.clone())), // Has seconds (not whole minute)
    ];
    
    let mut valid_count = 0;
    let mut invalid_count = 0;
    
    // Validate each scenario with analytics tracking
    for (i, calclock) in test_scenarios.iter().enumerate() {
        let result = analytics.track_validation(|| {
            cached_validator.validate_calclock(calclock)
        });
        
        if result.is_ok() {
            valid_count += 1;
            println!("Scenario {}: VALID - {}", i + 1, calclock);
        } else {
            invalid_count += 1;
            println!("Scenario {}: INVALID - {} (errors: {:?})", 
                     i + 1, calclock, result.unwrap_err());
        }
    }
    
    // Generate comprehensive report
    let metrics = analytics.get_metrics();
    let cache_stats = cached_validator.cache_stats();
    let report = analytics.generate_report();
    
    println!("\n=== COMPREHENSIVE VALIDATION REPORT ===");
    println!("Valid scenarios: {}", valid_count);
    println!("Invalid scenarios: {}", invalid_count);
    println!("Success rate: {:.1}%", metrics.success_rate() * 100.0);
    println!("Average validation time: {:?}", metrics.average_validation_time());
    println!("Cache hit rate: {:.1}%", cache_stats.hit_rate() * 100.0);
    println!("\nDetailed Analytics:");
    println!("{}", report.format());
    
    // Verify expected results
    assert_eq!(valid_count, 3, "Should have 3 valid scenarios");
    assert_eq!(invalid_count, 2, "Should have 2 invalid scenarios");
    assert_eq!(metrics.total_validations, 5, "Should have processed 5 validations");
    
    Ok(())
}