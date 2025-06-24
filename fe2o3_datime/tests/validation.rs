use oxedyne_fe2o3_datime::prelude::*;
use oxedyne_fe2o3_core::prelude::*;

#[test]
fn test_basic_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Test valid CalClock
    let valid_calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone.clone()));
    let validator = CalClockValidator::new();
    
    // Should pass basic validation
    assert!(validator.is_valid_calclock(&valid_calclock));
    assert!(validator.validate_calclock(&valid_calclock).is_ok());
    
    Ok(())
}

#[test]
fn test_business_hours_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Test during business hours (weekday, 10 AM)
    let business_time = res!(CalClock::new(2024, 6, 17, 10, 0, 0, 0, zone.clone())); // Monday
    
    // Test outside business hours (weekend)
    let weekend_time = res!(CalClock::new(2024, 6, 16, 10, 0, 0, 0, zone.clone())); // Sunday
    
    let business_rule = ValidationRules::business_hours();
    
    // Business hours should pass
    assert!(business_rule.validate_calclock(&business_time).is_ok());
    
    // Weekend should fail
    assert!(business_rule.validate_calclock(&weekend_time).is_err());
    
    Ok(())
}

#[test]
fn test_custom_validation_rule() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    // Create a custom rule that only allows even hours
    let even_hours_rule = ValidationRule::new("even_hours_only")
        .description("Only allows even hour values")
        .with_time_validator(|time| {
            if time.hour().of() % 2 == 0 {
                Ok(())
            } else {
                Err(vec![ValidationError::new(
                    "even_hours_only",
                    "Hour must be even"
                ).field("hour")
                 .value(time.hour().of().to_string())
                 .expected("even number")])
            }
        });
    
    let even_hour_time = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone.clone())); // 14 is even
    let odd_hour_time = res!(CalClock::new(2024, 6, 15, 15, 30, 0, 0, zone.clone())); // 15 is odd
    
    // Test the time component directly since our rule is for time
    assert!(even_hours_rule.validate_time(even_hour_time.time()).is_ok());
    
    // Odd hour should fail  
    let result = even_hours_rule.validate_time(odd_hour_time.time());
    assert!(result.is_err());
    
    if let Err(errors) = result {
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule, "even_hours_only");
    }
    
    Ok(())
}

#[test]
fn test_validator_with_multiple_rules() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    let mut validator = CalClockValidator::new();
    validator.add_rule(ValidationRules::whole_minutes_only());
    validator.add_rule(ValidationRules::hour_range(9, 17));
    
    // Valid: 10:30:00 (whole minute, within hour range)
    let valid_time = res!(CalClock::new(2024, 6, 17, 10, 30, 0, 0, zone.clone()));
    assert!(validator.validate_calclock(&valid_time).is_ok());
    
    // Invalid: 10:30:30 (has seconds)
    let invalid_seconds = res!(CalClock::new(2024, 6, 17, 10, 30, 30, 0, zone.clone()));
    assert!(validator.validate_calclock(&invalid_seconds).is_err());
    
    // Invalid: 8:30:00 (outside hour range)
    let invalid_hour = res!(CalClock::new(2024, 6, 17, 8, 30, 0, 0, zone.clone()));
    assert!(validator.validate_calclock(&invalid_hour).is_err());
    
    Ok(())
}

#[test]
fn test_date_range_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    let min_date = res!(CalendarDate::new(2024, 1, 1, zone.clone()));
    let max_date = res!(CalendarDate::new(2024, 12, 31, zone.clone()));
    
    let validator = CalClockValidator::new();
    
    // Valid date within range
    let valid_date = res!(CalendarDate::new(2024, 6, 15, zone.clone()));
    assert!(validator.validate_date_range(&valid_date, &min_date, &max_date).is_ok());
    
    // Invalid date before range
    let too_early = res!(CalendarDate::new(2023, 12, 31, zone.clone()));
    assert!(validator.validate_date_range(&too_early, &min_date, &max_date).is_err());
    
    // Invalid date after range
    let too_late = res!(CalendarDate::new(2025, 1, 1, zone.clone()));
    assert!(validator.validate_date_range(&too_late, &min_date, &max_date).is_err());
    
    Ok(())
}

#[test]
fn test_batch_validation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    
    let calclocks = vec![
        res!(CalClock::new(2024, 6, 15, 10, 0, 0, 0, zone.clone())), // Valid
        res!(CalClock::new(2024, 6, 15, 12, 30, 0, 0, zone.clone())), // Valid but let's make invalid with validator
    ];
    
    // Create a valid date for testing (constructors validate inputs)
    let test_date = res!(CalendarDate::new(2024, 6, 15, zone.clone()));
    
    // We'll test validation rules rather than invalid construction
    
    let validator = CalClockValidator::new();
    let result = validator.validate_many_calclocks(&calclocks);
    
    // Basic CalClocks should be valid
    assert!(result.is_ok());
    
    // Test date validation - valid date should pass
    let date_result = validator.validate_date(&test_date);
    assert!(date_result.is_ok());
    
    // All CalClocks should be valid with basic validator
    assert_eq!(validator.count_valid_calclocks(&calclocks), 2);
    
    // Filter should return all valid ones
    let valid_only = validator.filter_valid_calclocks(calclocks);
    assert_eq!(valid_only.len(), 2);
    
    Ok(())
}