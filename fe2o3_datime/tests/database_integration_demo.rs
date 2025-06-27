use oxedyne_fe2o3_datime::{
    time::{CalClock, CalClockZone},
    clock::ClockTime,
    calendar::CalendarDate,
    database::{
        DatabaseStorable, DatabaseRecord, StorageFormat,
        SqlCompatible, NoSqlDocument, IndexGenerator,
        DatabaseIndex, IndexType,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::HashMap;

#[test]
fn test_calclock_database_storage_binary() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Convert to binary database record
    let record = res!(calclock.to_database_record(StorageFormat::Binary));
    assert_eq!(record.storage_format, StorageFormat::Binary);
    assert_eq!(record.get_metadata("type"), Some(&"CalClock".to_string()));
    assert_eq!(record.get_metadata("timezone"), Some(&"UTC".to_string()));
    
    // Verify binary format structure
    if let Dat::Tup2(arr) = &record.primary_data {
        let nanos_dat = &arr[0];
        let zone_dat = &arr[1];
        // Just verify types match expected patterns
        if let Dat::I64(_) = nanos_dat { } else { panic!("Expected I64"); }
        if let Dat::Str(zone_str) = zone_dat { assert_eq!(zone_str, "UTC"); } else { panic!("Expected Str"); }
    } else {
        return Err(err!("Invalid binary format structure"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(CalClock::from_database_record(&record));
    assert_eq!(calclock, restored);
    
    println!("✓ CalClock binary storage test passed");
    Ok(())
}

#[test]
fn test_calclock_database_storage_iso8601() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Convert to ISO 8601 database record
    let record = res!(calclock.to_database_record(StorageFormat::Iso8601));
    assert_eq!(record.storage_format, StorageFormat::Iso8601);
    
    // Verify ISO format
    if let Dat::Str(iso_string) = &record.primary_data {
        assert!(iso_string.contains("2024-06-15"));
        assert!(iso_string.contains("14:30:45"));
        println!("ISO 8601 format: {}", iso_string);
    } else {
        return Err(err!("Invalid ISO format structure"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(CalClock::from_database_record(&record));
    assert_eq!(calclock, restored);
    
    println!("✓ CalClock ISO 8601 storage test passed");
    Ok(())
}

#[test]
fn test_calclock_database_storage_component() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Convert to component database record
    let record = res!(calclock.to_database_record(StorageFormat::Component));
    assert_eq!(record.storage_format, StorageFormat::Component);
    
    // Verify component format
    if let Dat::Map(components) = &record.primary_data {
        assert_eq!(res!(components.get(&Dat::Str("year".to_string())).unwrap().get_i32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 2024);
        assert_eq!(res!(components.get(&Dat::Str("month".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 6);
        assert_eq!(res!(components.get(&Dat::Str("day".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 15);
        assert_eq!(res!(components.get(&Dat::Str("hour".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 14);
        assert_eq!(res!(components.get(&Dat::Str("minute".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 30);
        assert_eq!(res!(components.get(&Dat::Str("second".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 45);
        assert_eq!(res!(components.get(&Dat::Str("nanosecond".to_string())).unwrap().get_u32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 123_456_789);
        assert_eq!(res!(components.get(&Dat::Str("timezone".to_string())).unwrap().get_string().ok_or_else(|| err!("Invalid type"; Invalid, Input))), "UTC");
    } else {
        return Err(err!("Invalid component format structure"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(CalClock::from_database_record(&record));
    assert_eq!(calclock, restored);
    
    println!("✓ CalClock component storage test passed");
    Ok(())
}

#[test]
fn test_calclock_database_storage_unix_timestamp() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123, zone));
    
    // Convert to Unix timestamp database record
    let record = res!(calclock.to_database_record(StorageFormat::UnixTimestamp));
    assert_eq!(record.storage_format, StorageFormat::UnixTimestamp);
    
    // Verify Unix timestamp format
    if let Dat::Tup2(arr) = &record.primary_data {
        let millis_dat = &arr[0];
        let zone_dat = &arr[1];
        // Just verify types match expected patterns
        if let Dat::I64(_) = millis_dat { } else { panic!("Expected I64"); }
        if let Dat::Str(zone_str) = zone_dat { assert_eq!(zone_str, "UTC"); } else { panic!("Expected Str"); }
    } else {
        return Err(err!("Invalid Unix timestamp format structure"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(CalClock::from_database_record(&record));
    assert_eq!(calclock, restored);
    
    println!("✓ CalClock Unix timestamp storage test passed");
    Ok(())
}

#[test]
fn test_database_record_indexes() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    let record = res!(calclock.to_database_record(StorageFormat::Binary));
    
    // Verify indexes are created
    assert!(record.get_index("year").is_some());
    assert!(record.get_index("month").is_some());
    assert!(record.get_index("day").is_some());
    assert!(record.get_index("hour").is_some());
    assert!(record.get_index("day_of_week").is_some());
    assert!(record.get_index("timezone").is_some());
    
    // Verify index values
    assert_eq!(res!(record.get_index("year").unwrap().get_i32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 2024);
    assert_eq!(res!(record.get_index("month").unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 6);
    assert_eq!(res!(record.get_index("day").unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 15);
    assert_eq!(res!(record.get_index("hour").unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 14);
    assert_eq!(res!(record.get_index("timezone").unwrap().get_string().ok_or_else(|| err!("Invalid type"; Invalid, Input))), "UTC");
    
    println!("✓ Database record indexes test passed");
    Ok(())
}

#[test]
fn test_clock_time_database_storage() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let time = res!(ClockTime::new(14, 30, 45, 123_456_789, zone));
    
    // Test binary format
    let record = res!(time.to_database_record(StorageFormat::Binary));
    assert_eq!(record.storage_format, StorageFormat::Binary);
    
    if let Dat::Tup2(arr) = &record.primary_data {
        let nanos_dat = &arr[0];
        let zone_dat = &arr[1];
        // Just verify types match expected patterns
        if let Dat::U64(_) = nanos_dat { } else { panic!("Expected U64"); }
        if let Dat::Str(zone_str) = zone_dat { assert_eq!(zone_str, "UTC"); } else { panic!("Expected Str"); }
    } else {
        return Err(err!("Invalid binary format for ClockTime"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(ClockTime::from_database_record(&record));
    assert_eq!(time, restored);
    
    // Test component format
    let component_record = res!(time.to_database_record(StorageFormat::Component));
    if let Dat::Map(components) = &component_record.primary_data {
        assert_eq!(res!(components.get(&Dat::Str("hour".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 14);
        assert_eq!(res!(components.get(&Dat::Str("minute".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 30);
        assert_eq!(res!(components.get(&Dat::Str("second".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 45);
        assert_eq!(res!(components.get(&Dat::Str("nanosecond".to_string())).unwrap().get_u32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 123_456_789);
    } else {
        return Err(err!("Invalid component format for ClockTime"; Invalid));
    }
    
    println!("✓ ClockTime database storage test passed");
    Ok(())
}

#[test]
fn test_calendar_date_database_storage() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let date = res!(CalendarDate::new(2024, 6, 15, zone));
    
    // Test binary format (Julian day)
    let record = res!(date.to_database_record(StorageFormat::Binary));
    assert_eq!(record.storage_format, StorageFormat::Binary);
    
    if let Dat::Tup2(arr) = &record.primary_data {
        let julian_dat = &arr[0];
        let zone_dat = &arr[1];
        // Just verify types match expected patterns
        if let Dat::I64(_) = julian_dat { } else { panic!("Expected I64"); }
        if let Dat::Str(zone_str) = zone_dat { assert_eq!(zone_str, "UTC"); } else { panic!("Expected Str"); }
    } else {
        return Err(err!("Invalid binary format for CalendarDate"; Invalid));
    }
    
    // Round-trip conversion
    let restored = res!(CalendarDate::from_database_record(&record));
    assert_eq!(date, restored);
    
    // Test component format
    let component_record = res!(date.to_database_record(StorageFormat::Component));
    if let Dat::Map(components) = &component_record.primary_data {
        assert_eq!(res!(components.get(&Dat::Str("year".to_string())).unwrap().get_i32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 2024);
        assert_eq!(res!(components.get(&Dat::Str("month".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 6);
        assert_eq!(res!(components.get(&Dat::Str("day".to_string())).unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 15);
    } else {
        return Err(err!("Invalid component format for CalendarDate"; Invalid));
    }
    
    println!("✓ CalendarDate database storage test passed");
    Ok(())
}

#[test]
fn test_sql_compatibility() -> Outcome<()> {
    // Test SQL data types
    assert_eq!(CalClock::sql_data_type(StorageFormat::Binary), "BIGINT, VARCHAR(50)");
    assert_eq!(ClockTime::sql_data_type(StorageFormat::Iso8601), "TIME");
    assert_eq!(CalendarDate::sql_data_type(StorageFormat::Component), "JSONB");
    
    // Test SQL table creation
    let create_table_sql = CalClock::sql_create_table("events", StorageFormat::Binary);
    assert!(create_table_sql.contains("CREATE TABLE events"));
    assert!(create_table_sql.contains("timestamp_nanos BIGINT NOT NULL"));
    assert!(create_table_sql.contains("timezone VARCHAR(50) NOT NULL"));
    assert!(create_table_sql.contains("year INTEGER NOT NULL"));
    
    // Test SQL index creation
    let indexes = CalClock::sql_create_indexes("events");
    assert!(!indexes.is_empty());
    assert!(indexes.iter().any(|idx| idx.contains("timestamp_nanos")));
    assert!(indexes.iter().any(|idx| idx.contains("timezone")));
    
    // Test SQL values conversion
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    let sql_values = res!(calclock.to_sql_values(StorageFormat::Binary));
    
    assert!(sql_values.contains_key("timestamp_nanos"));
    assert!(sql_values.contains_key("timezone"));
    assert!(sql_values.contains_key("year"));
    assert_eq!(sql_values.get("year"), Some(&"2024".to_string()));
    assert_eq!(sql_values.get("timezone"), Some(&"'UTC'".to_string()));
    
    // Test query fragments
    let fragments = CalClock::sql_query_fragments();
    assert!(!fragments.is_empty());
    assert!(fragments.iter().any(|(name, _)| *name == "date_range"));
    assert!(fragments.iter().any(|(name, _)| *name == "timezone_filter"));
    
    println!("✓ SQL compatibility test passed");
    Ok(())
}

#[test]
fn test_nosql_document_format() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Convert to document format
    let document = calclock.to_document();
    
    // Verify document structure
    assert!(document.contains_key("timestamp_nanos"));
    assert!(document.contains_key("timestamp_millis"));
    assert!(document.contains_key("year"));
    assert!(document.contains_key("month"));
    assert!(document.contains_key("day"));
    assert!(document.contains_key("hour"));
    assert!(document.contains_key("minute"));
    assert!(document.contains_key("second"));
    assert!(document.contains_key("nanosecond"));
    assert!(document.contains_key("timezone"));
    assert!(document.contains_key("iso_string"));
    assert!(document.contains_key("is_leap_second"));
    assert!(document.contains_key("is_weekend"));
    
    // Verify values
    assert_eq!(res!(document.get("year").unwrap().get_i32().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 2024);
    assert_eq!(res!(document.get("month").unwrap().get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input))), 6);
    assert_eq!(res!(document.get("timezone").unwrap().get_string().ok_or_else(|| err!("Invalid type"; Invalid, Input))), "UTC");
    
    // Round-trip conversion
    let restored = res!(CalClock::from_document(&document));
    assert_eq!(calclock, restored);
    
    // Test index fields
    let index_fields = CalClock::index_fields();
    assert!(!index_fields.is_empty());
    assert!(index_fields.contains(&"timestamp_nanos"));
    assert!(index_fields.contains(&"year"));
    assert!(index_fields.contains(&"timezone"));
    
    // Test compound indexes
    let compound_indexes = CalClock::compound_indexes();
    assert!(!compound_indexes.is_empty());
    assert!(compound_indexes.iter().any(|idx| idx == &vec!["year", "month", "day"]));
    assert!(compound_indexes.iter().any(|idx| idx == &vec!["timezone", "year"]));
    
    println!("✓ NoSQL document format test passed");
    Ok(())
}

#[test]
fn test_database_index_generation() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Generate indexes
    let indexes = calclock.generate_indexes("events");
    assert!(!indexes.is_empty());
    
    // Verify high-priority indexes exist
    let critical_indexes = calclock.critical_indexes("events");
    assert!(!critical_indexes.is_empty());
    
    // Find timestamp index
    let timestamp_index = indexes.iter()
        .find(|idx| idx.name.contains("timestamp_nanos"))
        .expect("Timestamp index should exist");
    assert_eq!(timestamp_index.priority, 10); // Highest priority
    assert_eq!(timestamp_index.fields, vec!["timestamp_nanos".to_string()]);
    
    // Find date composite index
    let date_index = indexes.iter()
        .find(|idx| idx.name.contains("date") && idx.fields.len() == 3)
        .expect("Date composite index should exist");
    assert_eq!(date_index.fields, vec!["year".to_string(), "month".to_string(), "day".to_string()]);
    
    // Test SQL generation
    let sql = timestamp_index.to_sql("events");
    assert!(sql.contains("CREATE"));
    assert!(sql.contains("INDEX"));
    assert!(sql.contains("events"));
    assert!(sql.contains("timestamp_nanos"));
    
    // Test MongoDB generation
    let (spec, options) = timestamp_index.to_mongodb();
    assert!(spec.contains_key("timestamp_nanos"));
    assert!(options.contains_key("name"));
    
    println!("✓ Database index generation test passed");
    Ok(())
}

#[test]
fn test_query_specific_indexes() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 45, 123_456_789, zone));
    
    // Generate indexes for specific query patterns
    let query_patterns = vec![
        "time_range_queries",
        "calendar_navigation", 
        "timezone_aware",
        "hourly_analytics",
        "weekly_patterns",
        "recent_data",
    ];
    
    let indexes = calclock.generate_query_specific_indexes("events", &query_patterns);
    assert!(!indexes.is_empty());
    
    // Verify specific indexes
    let time_range_index = indexes.iter()
        .find(|idx| idx.name.contains("time_range"))
        .expect("Time range index should exist");
    assert!(time_range_index.fields.contains(&"timestamp_nanos".to_string()));
    
    let calendar_nav_index = indexes.iter()
        .find(|idx| idx.name.contains("calendar_nav"))
        .expect("Calendar navigation index should exist");
    assert_eq!(calendar_nav_index.fields.len(), 3); // year, month, day
    
    println!("✓ Query-specific indexes test passed");
    Ok(())
}

#[test]
fn test_storage_format_recommendations() -> Outcome<()> {
    // Test recommended storage formats
    assert_eq!(CalClock::recommended_storage_format(), StorageFormat::Binary);
    assert_eq!(ClockTime::recommended_storage_format(), StorageFormat::Binary);
    assert_eq!(CalendarDate::recommended_storage_format(), StorageFormat::Binary);
    
    println!("✓ Storage format recommendations test passed");
    Ok(())
}

#[test]
fn test_database_record_metadata() -> Outcome<()> {
    let zone = CalClockZone::utc();
    let calclock = res!(CalClock::new(2024, 6, 15, 23, 59, 60, 0, zone)); // Leap second
    
    let record = res!(calclock.to_database_record(StorageFormat::Binary));
    
    // Verify metadata
    assert_eq!(record.get_metadata("type"), Some(&"CalClock".to_string()));
    assert_eq!(record.get_metadata("timezone"), Some(&"UTC".to_string()));
    assert_eq!(record.get_metadata("leap_second"), Some(&"true".to_string()));
    
    // Test adding custom metadata
    let mut custom_record = record.clone();
    custom_record.add_metadata("source", "user_input");
    custom_record.add_metadata("processed", "2024-06-15");
    
    assert_eq!(custom_record.get_metadata("source"), Some(&"user_input".to_string()));
    assert_eq!(custom_record.get_metadata("processed"), Some(&"2024-06-15".to_string()));
    
    println!("✓ Database record metadata test passed");
    Ok(())
}

#[test]
fn test_database_index_types() -> Outcome<()> {
    // Test different index types
    let btree_index = DatabaseIndex::new(
        "test_btree",
        vec!["timestamp".to_string()],
        IndexType::BTree,
    );
    
    let hash_index = DatabaseIndex::new(
        "test_hash",
        vec!["timezone".to_string()],
        IndexType::Hash,
    );
    
    let partial_index = DatabaseIndex::new(
        "test_partial",
        vec!["is_weekend".to_string()],
        IndexType::Partial("is_weekend = TRUE".to_string()),
    ).condition("is_weekend = TRUE");
    
    let composite_index = DatabaseIndex::new(
        "test_composite",
        vec!["year".to_string(), "month".to_string()],
        IndexType::Composite(vec!["year".to_string(), "month".to_string()]),
    );
    
    // Test SQL generation for different types
    let btree_sql = btree_index.to_sql("test_table");
    assert!(btree_sql.contains("USING BTREE"));
    
    let hash_sql = hash_index.to_sql("test_table");
    assert!(hash_sql.contains("USING HASH"));
    
    let partial_sql = partial_index.to_sql("test_table");
    assert!(partial_sql.contains("WHERE is_weekend = TRUE"));
    
    let composite_sql = composite_index.to_sql("test_table");
    assert!(composite_sql.contains("year, month"));
    
    println!("✓ Database index types test passed");
    Ok(())
}

pub fn test_database_integration(filter: &str) -> Outcome<()> {
    println!("=== Database Integration Demo ===");
    
    res!(test_it(filter, &["calclock_binary", "all", "database", "binary"], || {
        test_calclock_database_storage_binary()
    }));
    
    res!(test_it(filter, &["calclock_iso", "all", "database", "iso"], || {
        test_calclock_database_storage_iso8601()
    }));
    
    res!(test_it(filter, &["calclock_component", "all", "database", "component"], || {
        test_calclock_database_storage_component()
    }));
    
    res!(test_it(filter, &["calclock_unix", "all", "database", "unix"], || {
        test_calclock_database_storage_unix_timestamp()
    }));
    
    res!(test_it(filter, &["record_indexes", "all", "database", "indexes"], || {
        test_database_record_indexes()
    }));
    
    res!(test_it(filter, &["clocktime_storage", "all", "database", "clocktime"], || {
        test_clock_time_database_storage()
    }));
    
    res!(test_it(filter, &["caldate_storage", "all", "database", "caldate"], || {
        test_calendar_date_database_storage()
    }));
    
    res!(test_it(filter, &["sql_compat", "all", "database", "sql"], || {
        test_sql_compatibility()
    }));
    
    res!(test_it(filter, &["nosql_document", "all", "database", "nosql"], || {
        test_nosql_document_format()
    }));
    
    res!(test_it(filter, &["index_generation", "all", "database", "generation"], || {
        test_database_index_generation()
    }));
    
    res!(test_it(filter, &["query_indexes", "all", "database", "query"], || {
        test_query_specific_indexes()
    }));
    
    res!(test_it(filter, &["format_recommendations", "all", "database", "recommendations"], || {
        test_storage_format_recommendations()
    }));
    
    res!(test_it(filter, &["record_metadata", "all", "database", "metadata"], || {
        test_database_record_metadata()
    }));
    
    res!(test_it(filter, &["index_types", "all", "database", "types"], || {
        test_database_index_types()
    }));
    
    println!("✓ All database integration tests passed!");
    Ok(())
}

fn test_it<F>(filter: &str, keywords: &[&str], test_fn: F) -> Outcome<()>
where
    F: FnOnce() -> Outcome<()>,
{
    if keywords.iter().any(|&kw| filter.contains(kw)) {
        test_fn()
    } else {
        Ok(())
    }
}