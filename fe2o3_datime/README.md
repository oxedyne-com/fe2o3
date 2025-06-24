# fe2o3_datime

A comprehensive date and time library for the Hematite ecosystem with modern serialization and universal identification.

## Features

### 🗓️ **Multiple Calendar Systems**
- **Gregorian** (default): International standard calendar
- **Julian**: Pre-reform calendar with simpler leap year rules
- **Islamic/Hijri**: Lunar calendar starting from Hijra (622 CE)
- **Japanese**: Imperial era-based calendar system
- **Thai Buddhist**: Gregorian structure + 543 years
- **Minguo (ROC)**: Republic of China calendar starting from 1912
- **Holocene**: Scientific calendar adding 10,000 years

### 📊 **JDAT Serialization Integration**
- **String format**: Human-readable, leverages existing parsers
- **Binary format**: Ultra-compact with namex LocalId (1 byte per calendar)
- **Structured format**: Rich metadata for configuration and debugging

### 🏷️ **Namex Universal Identification**
- **NamexId**: 256-bit globally unique identifiers
- **LocalId**: 8-bit efficient identifiers for binary operations
- **Database integration**: Support for namex metadata databases

### 🌐 **Advanced Timezone Support**
- **IANA TZif integration**: Complete binary format parsing
- **DST handling**: Automatic daylight saving time transitions
- **Historical accuracy**: Support for timezone rule changes over time
- **Ambiguity resolution**: Handles "spring forward" and "fall back" scenarios

### ⚡ **High Performance**
- **Nanosecond precision**: Sub-second accuracy for all operations
- **Efficient conversions**: Optimized calendar-to-calendar transformations
- **Binary serialization**: Minimal overhead for storage and transmission
- **Batch operations**: Optimized time series processing

## Quick Start

```rust
use oxedyne_fe2o3_datime::{
    calendar::Calendar,
    time::{CalClock, CalClockZone},
};
use oxedyne_fe2o3_core::prelude::*;

// Create dates in different calendar systems
let gregorian = Calendar::Gregorian;
let islamic = Calendar::Islamic;
let zone = res!(CalClockZone::new("UTC"));

// Create a date - new API using Calendar enum
let greg_date = res!(gregorian.date(2024, 6, 23, zone.clone()));
let islamic_date = res!(islamic.date(1445, 12, 15, zone.clone()));

// Convert between calendar systems
let converted = res!(gregorian.convert_date(&islamic_date, &gregorian));

// Create complete date-time objects
let now = res!(CalClock::now_utc());
let custom_time = res!(CalClock::new(2024, 6, 23, 14, 30, 15, 123456789, zone));

// JDAT serialization examples
let calendar_text = res!(gregorian.to_dat()); // "gregorian"
let calendar_binary = res!(gregorian.to_dat_binary()); // 1 byte
let datetime_text = res!(now.to_dat()); // "2024-06-23 14:30:15.123456789 UTC"
let datetime_binary = res!(now.to_dat_binary()); // 16 bytes
```

## JDAT Integration Examples

### Configuration with Multiple Formats

```rust
use oxedyne_fe2o3_jdat::prelude::*;

// User-friendly configuration
#[derive(FromDatMap, ToDatMap)]
struct CalendarConfig {
    default_calendar: Calendar,     // Serializes as "gregorian"
    timezone: String,              // "America/New_York"
    business_hours_start: ClockTime, // "09:00:00"
    business_hours_end: ClockTime,   // "17:00:00"
}

// Time series with efficient binary storage
let measurements: Vec<(CalClock, f64)> = collect_sensor_data();
let binary_data = res!(measurements.to_dat()?.to_bytes(Vec::new()));
// Saves ~60% space compared to JSON
```

### API Integration

```rust
// REST API response with type-safe serialization
let api_response = mapdat! {
    "current_time" => res!(CalClock::now_utc().to_dat()),
    "supported_calendars" => listdat![
        Calendar::all().map(|c| res!(c.to_dat())).collect::<Result<Vec<_>, _>>()
    ],
    "timezone_info" => res!(zone.to_dat_structured()),
};

let json_response = res!(api_response.encode_string());
```

## Namex Integration

### Universal Calendar Identification

```rust
use oxedyne_fe2o3_namex::id::InNamex;

// Get universal 256-bit identifier
let namex_id = res!(Calendar::Gregorian.name_id());
let local_id = Calendar::Gregorian.local_id(); // LocalId(1)

// Binary serialization uses efficient LocalId
let compact_binary = res!(Calendar::Islamic.to_dat_binary()); // Just 1 byte!

// Structured format includes rich metadata
let metadata = res!(Calendar::Japanese.to_dat_structured());
// Includes: id, name, description, namex_id, local_id, epoch_year
```

## Performance Characteristics

### Space Efficiency

| Format | Calendar Reference | CalClock Timestamp |
|--------|-------------------|-------------------|
| **String** | ~10 bytes ("gregorian") | ~30 bytes ("2024-06-23T14:30:15Z") |
| **Binary** | **1 byte** (LocalId) | **16 bytes** (i64 + zone) |
| **Savings** | **90%** | **47%** |

### Use Case Performance

- **Configuration files**: Human-readable with automatic parsing
- **Time series**: Ultra-compact binary with nanosecond precision  
- **APIs**: JSON-compatible with type safety
- **Inter-service**: Efficient binary with universal identification

## Calendar System Details

### Epoch Years and Conversions

| Calendar | Epoch Year | Example Conversion |
|----------|------------|-------------------|
| **Gregorian** | 1 CE | 2024 = 2024 |
| **Islamic** | 622 CE | 1445 ≈ 2024 |
| **Thai** | -543 CE | 2567 = 2024 |
| **Minguo** | 1912 CE | 113 = 2024 |
| **Holocene** | -9999 CE | 12024 = 2024 |

### Leap Year Rules

- **Gregorian/Thai/Minguo/Holocene**: Every 4 years, except centuries not divisible by 400
- **Julian**: Every 4 years, no exceptions
- **Islamic**: 30-year cycle with leap years in positions 2, 5, 7, 10, 13, 16, 18, 21, 24, 26, 29

## Integration with fe2o3 Ecosystem

fe2o3_datime seamlessly integrates with other Hematite components:

- **fe2o3_jdat**: Type-safe serialization with multiple format levels
- **fe2o3_namex**: Universal identification and metadata databases  
- **fe2o3_core**: Error handling with `res!` macro and `Outcome` types
- **fe2o3_data**: Efficient data structures for time series operations

## Migration from calclock

```rust
// OLD API (calclock)
use oxedyne_fe2o3_calclock::calendar::CalendarDate;
let date = CalendarDate::new(2024, 1, 15, zone)?;

// NEW API (datime)
use oxedyne_fe2o3_datime::calendar::Calendar;
let calendar = Calendar::new(); // Default Gregorian
let date = res!(calendar.date(2024, 1, 15, zone));
```

The new API provides:
- **Type safety**: Calendar system is explicit
- **Extensibility**: Easy to add new calendar systems
- **Efficiency**: Direct conversion between any calendar systems
- **Serialization**: Built-in JDAT and namex support