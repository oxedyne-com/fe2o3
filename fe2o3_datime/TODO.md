# TODO for fe2o3_calclock

## ✅ RECENTLY COMPLETED

### Major Feature Implementation (2025-06-23)
- [x] **Calendar System Architecture**: Implemented foundation for multiple calendar systems
  - [x] Added `CalendarSystem` enum with Gregorian and Julian calendar support
  - [x] Integrated calendar systems into `CalendarDate` with backward compatibility
  - [x] Implemented calendar conversion via Julian day numbers
  - [x] Added calendar-specific leap year rules (1900: leap in Julian, not Gregorian)
  - [x] Support for calendar system parsing from strings ("gregorian", "julian", etc.)
  
- [x] **Jiff-Style System Timezone Integration**: Complete implementation
  - [x] `SystemTimezoneManager` with comprehensive timezone database integration
  - [x] Cross-platform timezone data search paths (Unix, Windows, Android)
  - [x] User consent mechanisms for system data access (`FE2O3_CALCLOCK_TIMEZONE_CONSENT`)
  - [x] Timezone caching with conflict detection and cache invalidation
  - [x] Embedded vs system timezone creation options (`new_embedded()`, `from_system_or_embedded()`)
  - [x] System timezone listing and scanning functionality
  
- [x] **Full IANA TZif Integration**: Complete RFC 8536 implementation
  - [x] TZif binary format parser supporting versions 1, 2, and 3
  - [x] Historical timezone transition support with accurate offset calculations
  - [x] DST transition ambiguity handling (`LocalTimeResult` enum)
  - [x] POSIX TZ string parsing for future transition rules
  - [x] Leap second record parsing (though timezone rules don't use them)
  - [x] CalClockZone integration with TZif data for precise timezone calculations
  
- [x] **Leap Second Capability Assessment**: Research and documentation
  - [x] Confirmed timezone databases do NOT handle leap seconds
  - [x] Documented requirement for separate TAI-UTC conversion implementation
  - [x] Added `LeapSecondCapability` API explaining system limitations
  
- [x] **Comprehensive Testing**: All new features fully tested
  - [x] Calendar system creation, conversion, and validation tests
  - [x] System timezone configuration and manager tests
  - [x] Full IANA TZif format parsing and integration tests
  - [x] DST transition ambiguity and historical timezone tests
  - [x] Leap second capability assessment tests
  - [x] Calendar display formatting and parsing tests
  - [x] Integration tests with existing CalClock functionality

## High Priority

### 1. Fix Test Framework ✅ COMPLETED
- [x] Update all tests to use proper `test_it` format with label arrays including "all"
- [x] Add comprehensive tests for all modules  
- [x] Fix CalClock comparison test (updated to use nanosecond precision)
- [x] Fix ClockTime end_of_day test (special constructor for hour 24)
- [ ] Fix the ignored parser test for ISO datetime parsing

### 2. External Database Integration & Automatic Updates ✅ COMPLETED
- [x] **Automatic Timezone Database Updates**: Implement Jiff-style system timezone integration
  - [x] Read timezone data from system `/usr/share/zoneinfo` on Unix
  - [x] Support Windows timezone database lookup  
  - [x] Add Android platform support (`ANDROID_ROOT`/`ANDROID_DATA` env vars)
  - [x] Implement user consent mechanism for automatic updates
  - [x] Add fallback to embedded timezone data when system data unavailable
- [x] **Cross-platform Timezone Support**:
  - [x] Detect and handle timezone rule changes automatically
  - [x] Provide conflict detection when stored times become invalid due to rule changes
  - [x] Add configuration option to disable automatic updates for security-conscious users
- [x] **IANA Database Integration**: ✅ COMPLETED - Full TZif format parsing implemented
  - [x] Handle ambiguous times during DST transitions (spring forward/fall back)
  - [x] Add `LocalTimeResult` enum for `Single`, `Ambiguous`, and `None` cases
  - [x] Support historical timezone rule changes
  - [x] Parse TZif binary format (versions 1, 2, and 3)
  - [x] Support leap second records from IANA data
  - [x] Implement POSIX TZ string parsing (version 2+ footer)
  - [x] Add comprehensive error handling for malformed TZif files

### 3. Locale Support ✅ COMPLETED
- [x] **Locale System Implementation**: Complete locale-based formatting system
  - [x] `Locale` struct with predefined locales (en-US, en-GB, de-DE, fr-FR, ja-JP, zh-CN, ISO)
  - [x] Default format patterns for each locale (date, time, datetime, short/long formats)
  - [x] CalClockFormatter integration with locale-specific formatting methods
  - [x] Locale database with fallback to US English for unknown locales
  - [x] Locale listing functionality (`available_locales()`, `available_locales_with_names()`)
  - [x] Convenience constructors for common locales (`us()`, `uk()`, `germany()`, etc.)
  - [x] Comprehensive tests covering all locale functionality
- [ ] **Future Locale Enhancements**:
  - [ ] Implement locale-aware date parsing (MDY vs DMY vs YMD)
  - [ ] Add locale-specific month and day names
  - [ ] Support locale-specific AM/PM indicators
  - [ ] Add locale-aware week start day (Sunday vs Monday)

## Medium Priority

### 4. Leap Second Support ⚠️ RESEARCH COMPLETED  
- [x] **CAPABILITY ASSESSMENT**: Analyzed leap second handling in timezone systems
  - [x] Confirmed timezone databases do NOT handle leap seconds
  - [x] Documented that leap seconds require separate TAI-UTC conversion
  - [x] Added `LeapSecondCapability` API explaining limitations
- [ ] **IMPLEMENTATION** (requires separate system from timezone data):
  - [ ] Implement `LeapSecondTable` for TAI-UTC conversions
  - [ ] Add methods for converting between UTC and TAI
  - [ ] Support seconds value of 60 for leap seconds
  - [ ] Add configuration option to enable/disable leap second support

### 5. Serialization with Timezone Preservation
- [ ] Implement RFC 9557 format support
- [ ] Add `to_rfc9557()` and `from_rfc9557()` methods
- [ ] Ensure lossless timezone serialization/deserialization
- [ ] Add Serde support with custom serializers

### 6. Relative Time Parsing
- [ ] Parse "next Tuesday", "last Monday"
- [ ] Parse "3 days ago", "in 2 weeks"
- [ ] Parse "tomorrow", "yesterday"
- [ ] Parse "next month", "last year"
- [ ] Add `parse_relative()` method to Parser

### 7. Enhanced Formatting
- [ ] Complete implementation of all format tokens
- [ ] Add support for custom padding characters
- [ ] Implement timezone name formatting
- [ ] Add more pre-defined format styles

## Lower Priority

### 8. Multiple Calendar Systems ⚠️ PARTIALLY COMPLETED
- [x] **FOUNDATION IMPLEMENTED**: Basic calendar system architecture
  - [x] Design `CalendarSystem` enum with Gregorian and Julian calendars
  - [x] Add calendar-aware `CalendarDate` with system field
  - [x] Implement calendar conversion via Julian day numbers
  - [x] Add calendar-specific leap year rules (1900: leap in Julian, not Gregorian)
  - [x] Support calendar system parsing from strings
  - [x] Integrate with existing CalClock for backward compatibility
- [ ] **MISSING CALENDAR SYSTEMS**: Expand beyond Gregorian/Julian
  - [ ] Implement Islamic/Hijri calendar (`HijrahChronology` in Java)
  - [ ] Implement Japanese Imperial calendar (`JapaneseChronology` in Java)
  - [ ] Implement Thai Buddhist calendar (`ThaiBuddhistChronology` in Java)
  - [ ] Implement Minguo calendar (`MinguoChronology` in Java)
  - [ ] Implement Hebrew calendar (not in Java but commonly requested)
  - [ ] Implement Chinese calendar (not in Java but commonly requested)
- [ ] **ADVANCED FEATURES**: 
  - [ ] Implement `ChronoLocalDate` equivalent for calendar-agnostic dates
  - [ ] Support era definitions for each calendar system
  - [ ] Add locale-aware calendar selection

### 9. Database Integration
- [ ] Add `to_storage_format()` and `from_storage_format()` methods
- [ ] Document best practices for database storage
- [ ] Add examples for common databases (PostgreSQL, SQLite)

### 10. Performance Optimizations
- [ ] Profile and optimize hot paths
- [ ] Consider SIMD optimizations for batch operations
- [ ] Add benchmarks for all major operations
- [ ] Optimize memory allocation patterns

### 11. Advanced Recurrence
- [ ] Add exception dates to recurrence rules
- [ ] Support "except holidays" in recurrence patterns
- [ ] Add more complex recurrence patterns (e.g., "last Friday of month")
- [ ] Implement iCalendar RRULE compatibility

### 12. Historical Calendar Support ⚠️ PARTIALLY COMPLETED
- [x] **BASIC IMPLEMENTATION**: Calendar system support with historical awareness
  - [x] Implement Gregorian and Julian calendar systems
  - [x] Add Julian day number conversion for calendar transitions
  - [x] Support calendar conversion between systems
  - [x] Add Gregorian reform date detection (October 5-14, 1582 "lost days")
- [ ] **COMPREHENSIVE HISTORICAL SUPPORT**:
  - [ ] Handle Julian to Gregorian calendar transition more comprehensively
  - [ ] Support proleptic Julian calendar for dates before 1582
  - [ ] Add historical date validation for different regions (transition dates varied)
  - [ ] Document limitations for historical dates and regional differences
  - [ ] Add support for Old Style vs New Style date notation

## Documentation

### 13. Improve Documentation
- [ ] Add comprehensive examples for all major features
- [ ] Create a user guide with common use cases
- [ ] Add migration guide from chrono/time
- [ ] Document performance characteristics
- [ ] Add cookbook-style examples

### 14. Integration Examples
- [ ] Create example web server with date handling
- [ ] Add CLI tool examples
- [ ] Show integration with async code
- [ ] Demonstrate timezone-aware scheduling

## Testing

### 15. Expand Test Coverage ⚠️ PARTIALLY COMPLETED
- [x] **NEW FEATURES TESTING**: Comprehensive tests for calendar systems and timezone integration
  - [x] Add calendar system creation and conversion tests
  - [x] Add system timezone configuration tests  
  - [x] Add leap second capability assessment tests
  - [x] Add calendar display and parsing tests
  - [x] Add calendar integration with CalClock tests
- [ ] **ADDITIONAL TESTING NEEDS**:
  - [ ] Add property-based tests using proptest
  - [ ] Add fuzzing for parser
  - [ ] Test edge cases (year 0, far future dates)
  - [ ] Add integration tests with real timezone data
  - [ ] Add performance regression tests

## Community

### 16. Ecosystem Integration
- [ ] Add compatibility layer for chrono types
- [ ] Create migration tools from other date libraries
- [ ] Add common format converters (Unix timestamp, Excel dates, etc.)
- [ ] Support more serialization formats (bincode, postcard, etc.)

## Known Issues

### 17. Bug Fixes & Technical Debt
- [ ] Parser fails on "2024-06-15" ISO date format (month parsing issue)
- [ ] Fix unused variable warnings in formatter.rs
- [ ] Remove dead code warnings for unused enum variants
- [x] **Implementation Gap ADDRESSED**: Multiple calendar system foundation implemented
  - [x] Added CalendarSystem enum with Gregorian and Julian calendars (foundation for expansion)
  - [x] Implemented calendar-aware CalendarDate with system field
  - [x] Added calendar conversion methods via Julian day numbers
  - [ ] Still missing: Islamic, Japanese, Thai Buddhist, Minguo calendars (planned for future expansion)
  - Foundation now exists to add remaining calendar systems incrementally

## Research & Implementation Notes

### External Database Integration Approaches

Based on research of other popular date/time libraries:

**1. Jiff Approach (Recommended)**:
- Automatically reads from system timezone databases (`/usr/share/zoneinfo` on Unix)
- Falls back to embedded data when system data unavailable  
- Provides user consent mechanisms
- Detects timezone rule conflicts automatically
- Cross-platform support (Unix, Windows, Android)

**2. Chrono-TZ Approach**:
- Static timezone data compiled into binary at build time
- Requires rebuilding application to get timezone updates
- Uses `CHRONO_TZ_TIMEZONE_FILTER` environment variable for selective inclusion
- No automatic updates - more secure but less convenient

**3. Hybrid Approach (✅ IMPLEMENTED in fe2o3_calclock)**:
- [x] Default: Embedded timezone data for security and reliability
- [x] Optional: System timezone integration with user consent
- [x] Configuration flags to control update behavior
- [x] Validation to detect conflicts between embedded and system data
- [x] Graceful fallback when system data unavailable or corrupted

**Implementation Status**: ✅ COMPLETED - Full Jiff-style system timezone integration implemented with all major features working.

## Future Considerations

### 18. Advanced Features
- [ ] Add astronomical calculations (sunrise/sunset, moon phases)
- [ ] Support for geological time scales
- [ ] Add financial calendar support (settlement dates, holidays)
- [ ] Implement interval algebra operations
- [ ] Add support for partial dates (e.g., "June 2024" without day)

## Architecture

### 19. Code Quality
- [ ] Review and refactor error handling patterns
- [ ] Ensure consistent API design across modules
- [ ] Add #[must_use] attributes where appropriate
- [ ] Review and optimize memory usage
- [ ] Consider const fn where possible

### 20. Feature Flags
- [ ] Add feature flags for optional components
- [ ] Create minimal build configuration
- [ ] Allow disabling specific timezone databases
- [ ] Make validation framework optional
- [ ] Support no_std environments better