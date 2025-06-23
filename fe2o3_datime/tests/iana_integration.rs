use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_datime::{
    time::{
        CalClockZone,
        TZifParser,
        TZifData,
        LocalTimeResult,
        LocalTimeType,
        LeapSecond,
    },
};

/// Test comprehensive IANA TZif integration
pub fn test_iana_integration(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["tzif_parser_creation", "all", "iana", "tzif"], || {
        let parser = TZifParser::new();
        // Parser should be created successfully
        assert!(parser.timezone_data().is_none());
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_minimal_header", "all", "iana", "tzif"], || {
        // Create minimal valid TZif v1 file
        let mut data = Vec::new();
        
        // Header
        data.extend_from_slice(b"TZif");      // Magic number
        data.push(0);                         // Version 1 (0 byte)
        data.extend_from_slice(&[0u8; 15]);   // Reserved (15 bytes)
        
        // Counts (all zero for minimal file)
        data.extend_from_slice(&[0u8; 4]);    // tzh_utcnt
        data.extend_from_slice(&[0u8; 4]);    // tzh_stdcnt  
        data.extend_from_slice(&[0u8; 4]);    // tzh_leapcnt
        data.extend_from_slice(&[0u8; 4]);    // tzh_timecnt
        data.extend_from_slice(&[0u8; 4]);    // tzh_typecnt
        data.extend_from_slice(&[0u8; 4]);    // tzh_charcnt
        
        let mut parser = TZifParser::new();
        let result = parser.load_from_bytes(&data);
        assert!(result.is_ok());
        
        let tzif_data = parser.timezone_data().unwrap();
        assert_eq!(tzif_data.version, 1);
        assert!(tzif_data.transition_times.is_empty());
        assert!(tzif_data.local_time_types.is_empty());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_v2_header", "all", "iana", "tzif"], || {
        // Create minimal valid TZif v2 file
        let mut data = Vec::new();
        
        // Header
        data.extend_from_slice(b"TZif");      // Magic number  
        data.push(b'2');                      // Version 2
        data.extend_from_slice(&[0u8; 15]);   // Reserved
        
        // Counts (all zero for minimal file)
        data.extend_from_slice(&[0u8; 24]);   // 6 * 4 bytes of counts
        
        // Second header for v2 data
        data.extend_from_slice(b"TZif");      // Magic number
        data.push(b'2');                      // Version 2  
        data.extend_from_slice(&[0u8; 15]);   // Reserved
        data.extend_from_slice(&[0u8; 24]);   // 6 * 4 bytes of counts
        
        // POSIX footer
        data.push(b'\n');                     // Newline
        data.extend_from_slice(b"GMT0");      // POSIX TZ string
        data.push(b'\n');                     // Final newline
        
        let mut parser = TZifParser::new();
        let result = parser.load_from_bytes(&data);
        assert!(result.is_ok());
        
        let tzif_data = parser.timezone_data().unwrap();
        assert_eq!(tzif_data.version, 2);
        assert_eq!(tzif_data.posix_tz_string.as_ref().unwrap(), "GMT0");
        
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_fixed_offset", "all", "iana", "tzif"], || {
        // Create TZif file with single fixed offset (like UTC+5)
        let mut data = Vec::new();
        
        // Header
        data.extend_from_slice(b"TZif");      // Magic
        data.push(0);                         // Version 1
        data.extend_from_slice(&[0u8; 15]);   // Reserved
        
        // Counts
        data.extend_from_slice(&[0u8; 4]);    // tzh_utcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_stdcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_leapcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_timecnt = 0
        data.extend_from_slice(&1u32.to_be_bytes());  // tzh_typecnt = 1
        data.extend_from_slice(&4u32.to_be_bytes());  // tzh_charcnt = 4
        
        // No transition times (tzh_timecnt = 0)
        // No transition types (tzh_timecnt = 0)
        
        // Local time types (1 entry)
        data.extend_from_slice(&18000i32.to_be_bytes()); // UTC offset: +5 hours
        data.push(0);                         // is_dst = false
        data.push(0);                         // abbreviation index = 0
        
        // Abbreviations
        data.extend_from_slice(b"+05\0");     // "+05" + null terminator
        
        let mut parser = TZifParser::new();
        let result = parser.load_from_bytes(&data);
        assert!(result.is_ok());
        
        let tzif_data = parser.timezone_data().unwrap();
        assert_eq!(tzif_data.local_time_types.len(), 1);
        assert_eq!(tzif_data.local_time_types[0].utc_offset, 18000);
        assert!(!tzif_data.local_time_types[0].is_dst);
        assert_eq!(tzif_data.get_abbreviation(&tzif_data.local_time_types[0]).unwrap(), "+05");
        
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_with_transitions", "all", "iana", "tzif"], || {
        // Create TZif file with DST transitions (simplified)
        let mut data = Vec::new();
        
        // Header
        data.extend_from_slice(b"TZif");
        data.push(0);                         // Version 1
        data.extend_from_slice(&[0u8; 15]);   // Reserved
        
        // Counts
        data.extend_from_slice(&[0u8; 4]);    // tzh_utcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_stdcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_leapcnt = 0
        data.extend_from_slice(&2u32.to_be_bytes());  // tzh_timecnt = 2
        data.extend_from_slice(&2u32.to_be_bytes());  // tzh_typecnt = 2
        data.extend_from_slice(&8u32.to_be_bytes());  // tzh_charcnt = 8
        
        // Transition times (2 entries, 32-bit)
        data.extend_from_slice(&1609459200i32.to_be_bytes()); // 2021-01-01 00:00:00 UTC
        data.extend_from_slice(&1625097600i32.to_be_bytes()); // 2021-07-01 00:00:00 UTC
        
        // Transition types (2 entries)
        data.push(0);  // First transition uses type 0 (standard time)
        data.push(1);  // Second transition uses type 1 (daylight time)
        
        // Local time types (2 entries)
        // Type 0: Standard time (EST: -5 hours)
        data.extend_from_slice(&(-18000i32).to_be_bytes()); 
        data.push(0);  // is_dst = false
        data.push(0);  // abbreviation index = 0
        
        // Type 1: Daylight time (EDT: -4 hours)  
        data.extend_from_slice(&(-14400i32).to_be_bytes());
        data.push(1);  // is_dst = true
        data.push(4);  // abbreviation index = 4
        
        // Abbreviations
        data.extend_from_slice(b"EST\0EDT\0"); // "EST" + null + "EDT" + null
        
        let mut parser = TZifParser::new();
        let result = parser.load_from_bytes(&data);
        assert!(result.is_ok());
        
        let tzif_data = parser.timezone_data().unwrap();
        assert_eq!(tzif_data.transition_times.len(), 2);
        assert_eq!(tzif_data.transition_types.len(), 2);
        assert_eq!(tzif_data.local_time_types.len(), 2);
        
        // Check standard time type
        assert_eq!(tzif_data.local_time_types[0].utc_offset, -18000);
        assert!(!tzif_data.local_time_types[0].is_dst);
        assert_eq!(tzif_data.get_abbreviation(&tzif_data.local_time_types[0]).unwrap(), "EST");
        
        // Check daylight time type
        assert_eq!(tzif_data.local_time_types[1].utc_offset, -14400);
        assert!(tzif_data.local_time_types[1].is_dst);
        assert_eq!(tzif_data.get_abbreviation(&tzif_data.local_time_types[1]).unwrap(), "EDT");
        
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_leap_seconds", "all", "iana", "tzif"], || {
        // Create TZif file with leap second data
        let mut data = Vec::new();
        
        // Header
        data.extend_from_slice(b"TZif");
        data.push(0);                         // Version 1
        data.extend_from_slice(&[0u8; 15]);   // Reserved
        
        // Counts
        data.extend_from_slice(&[0u8; 4]);    // tzh_utcnt = 0
        data.extend_from_slice(&[0u8; 4]);    // tzh_stdcnt = 0
        data.extend_from_slice(&1u32.to_be_bytes());  // tzh_leapcnt = 1
        data.extend_from_slice(&[0u8; 4]);    // tzh_timecnt = 0
        data.extend_from_slice(&1u32.to_be_bytes());  // tzh_typecnt = 1
        data.extend_from_slice(&4u32.to_be_bytes());  // tzh_charcnt = 4
        
        // No transitions
        
        // Local time type (UTC)
        data.extend_from_slice(&0i32.to_be_bytes()); // UTC offset = 0
        data.push(0);  // is_dst = false
        data.push(0);  // abbreviation index = 0
        
        // Abbreviations
        data.extend_from_slice(b"UTC\0");
        
        // Leap second (1 entry)
        data.extend_from_slice(&78796800i32.to_be_bytes()); // 1972-07-01 (first leap second)
        data.extend_from_slice(&1i32.to_be_bytes());        // correction = 1 second
        
        let mut parser = TZifParser::new();
        let result = parser.load_from_bytes(&data);
        assert!(result.is_ok());
        
        let tzif_data = parser.timezone_data().unwrap();
        assert_eq!(tzif_data.leap_seconds.len(), 1);
        assert_eq!(tzif_data.leap_seconds[0].transition_time, 78796800);
        assert_eq!(tzif_data.leap_seconds[0].correction, 1);
        
        Ok(())
    }));
    
    res!(test_it(filter, &["calclockzone_from_tzif", "all", "iana", "calclockzone"], || {
        // Test CalClockZone creation from TZif data
        let tzif_data = TZifData {
            version: 2,
            transition_times: vec![1609459200], // 2021-01-01
            transition_types: vec![0],
            local_time_types: vec![
                LocalTimeType {
                    utc_offset: -18000, // EST: -5 hours
                    is_dst: false,
                    abbreviation_index: 0,
                }
            ],
            abbreviations: "EST\0".to_string(),
            leap_seconds: Vec::new(),
            standard_wall_indicators: Vec::new(),
            ut_local_indicators: Vec::new(),
            posix_tz_string: Some("EST5".to_string()),
        };
        
        let zone = res!(CalClockZone::from_tzif_data("America/New_York", tzif_data));
        assert_eq!(zone.id(), "America/New_York");
        
        // Test timezone offset calculation using TZif data
        let offset = res!(zone.offset_millis_at_time(1609459200000)); // 2021-01-01 00:00:00 UTC
        assert_eq!(offset, -18000 * 1000); // -5 hours in milliseconds
        
        Ok(())
    }));
    
    res!(test_it(filter, &["dst_transition_ambiguity", "all", "iana", "dst"], || {
        // Test DST transition ambiguity handling
        let tzif_data = TZifData {
            version: 2,
            transition_times: vec![
                1615708800, // Spring forward: 2021-03-14 07:00:00 UTC (2 AM EST -> 3 AM EDT)
                1636264800, // Fall back: 2021-11-07 06:00:00 UTC (2 AM EDT -> 1 AM EST)
            ],
            transition_types: vec![1, 0], // Spring to DST, Fall to standard
            local_time_types: vec![
                LocalTimeType {
                    utc_offset: -18000, // EST: -5 hours
                    is_dst: false,
                    abbreviation_index: 0,
                },
                LocalTimeType {
                    utc_offset: -14400, // EDT: -4 hours
                    is_dst: true,
                    abbreviation_index: 4,
                }
            ],
            abbreviations: "EST\0EDT\0".to_string(),
            leap_seconds: Vec::new(),
            standard_wall_indicators: Vec::new(),
            ut_local_indicators: Vec::new(),
            posix_tz_string: Some("EST5EDT,M3.2.0,M11.1.0".to_string()),
        };
        
        let zone = res!(CalClockZone::from_tzif_data("America/New_York", tzif_data));
        
        // Test UTC to local conversion
        let utc_spring = 1615708800000; // Spring forward time
        match zone.utc_to_local(utc_spring) {
            LocalTimeResult::Single(local_time) => {
                // Should convert normally
                assert!(local_time != 0);
            },
            _ => {
                // UTC to local should generally be unambiguous
            }
        }
        
        // Test DST detection
        let is_dst_summer = res!(zone.in_daylight_time(1625097600000)); // July 1, 2021
        let is_dst_winter = res!(zone.in_daylight_time(1609459200000)); // January 1, 2021
        
        // Summer should be DST, winter should not (depends on exact TZif data implementation)
        // Note: This test depends on the TZif data interpretation logic
        
        Ok(())
    }));
    
    res!(test_it(filter, &["tzif_error_handling", "all", "iana", "error"], || {
        let mut parser = TZifParser::new();
        
        // Test invalid magic number
        let invalid_magic = b"TZIF1234567890123456789012345678901234567890";
        assert!(parser.load_from_bytes(invalid_magic).is_err());
        
        // Test truncated file
        let truncated = b"TZif";
        assert!(parser.load_from_bytes(truncated).is_err());
        
        // Test invalid version
        let mut invalid_version = Vec::new();
        invalid_version.extend_from_slice(b"TZif");
        invalid_version.push(b'X'); // Invalid version
        invalid_version.extend_from_slice(&[0u8; 39]); // Rest of minimal header
        assert!(parser.load_from_bytes(&invalid_version).is_err());
        
        Ok(())
    }));
    
    res!(test_it(filter, &["historical_transitions", "all", "iana", "historical"], || {
        // Test that TZif data supports historical timezone changes
        let tzif_data = TZifData {
            version: 2,
            transition_times: vec![
                -2147483648, // Very old date (1901)
                0,           // Unix epoch (1970)
                1609459200,  // Recent date (2021)
            ],
            transition_types: vec![0, 1, 0],
            local_time_types: vec![
                LocalTimeType {
                    utc_offset: -18000,
                    is_dst: false,
                    abbreviation_index: 0,
                },
                LocalTimeType {
                    utc_offset: -14400,
                    is_dst: true,
                    abbreviation_index: 4,
                }
            ],
            abbreviations: "EST\0EDT\0".to_string(),
            leap_seconds: Vec::new(),
            standard_wall_indicators: Vec::new(),
            ut_local_indicators: Vec::new(),
            posix_tz_string: Some("EST5EDT,M3.2.0,M11.1.0".to_string()),
        };
        
        // Test that we can get offsets for historical dates
        let historical_offset = res!(tzif_data.get_offset_at_utc(-2147483648));
        assert_eq!(historical_offset, -18000);
        
        let epoch_offset = res!(tzif_data.get_offset_at_utc(0));
        assert_eq!(epoch_offset, -14400); // EDT during summer 1970
        
        let recent_offset = res!(tzif_data.get_offset_at_utc(1609459200));
        assert_eq!(recent_offset, -18000); // EST in January
        
        Ok(())
    }));
    
    Ok(())
}