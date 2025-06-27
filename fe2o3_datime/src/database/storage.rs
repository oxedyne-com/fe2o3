use crate::{
    time::{CalClock, CalClockZone},
    clock::ClockTime,
    calendar::CalendarDate,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{prelude::*, tup2dat};

use std::collections::{HashMap, BTreeMap};

/// Represents different storage formats optimised for database usage.
#[derive(Clone, Debug, PartialEq)]
pub enum StorageFormat {
    /// Binary format: Most efficient for storage and time-based queries
    /// CalClock: (nanoseconds_since_epoch: i64, timezone: String)
    /// ClockTime: (nanoseconds_of_day: u64, timezone: String)
    /// CalendarDate: (julian_day_number: i64, timezone: String)
    Binary,
    
    /// ISO 8601 string format: Human-readable and standard-compliant
    /// CalClock: "2024-06-15T14:30:00.123456789Z"
    /// ClockTime: "14:30:00.123456789"
    /// CalendarDate: "2024-06-15"
    Iso8601,
    
    /// Component format: Optimal for partial queries and calendar operations
    /// Stores individual year, month, day, hour, minute, second, nanosecond
    Component,
    
    /// Unix timestamp: Compatible with existing systems
    /// Millisecond precision for compatibility
    UnixTimestamp,
}

/// Database record containing datetime data in various formats.
#[derive(Clone, Debug, PartialEq)]
pub struct DatabaseRecord {
    /// Primary storage data (format depends on storage_format)
    pub primary_data: Dat,
    
    /// Storage format used for primary_data
    pub storage_format: StorageFormat,
    
    /// Secondary indexes for query optimization
    pub indexes: HashMap<String, Dat>,
    
    /// Metadata about the record
    pub metadata: HashMap<String, String>,
}

impl DatabaseRecord {
    /// Creates a new database record with the specified format.
    pub fn new(primary_data: Dat, storage_format: StorageFormat) -> Self {
        Self {
            primary_data,
            storage_format,
            indexes: HashMap::new(),
            metadata: HashMap::new(),
        }
    }
    
    /// Adds an index entry for query optimization.
    pub fn add_index(&mut self, key: &str, value: Dat) {
        self.indexes.insert(key.to_string(), value);
    }
    
    /// Adds metadata to the record.
    pub fn add_metadata(&mut self, key: &str, value: &str) {
        self.metadata.insert(key.to_string(), value.to_string());
    }
    
    /// Gets an index value by key.
    pub fn get_index(&self, key: &str) -> Option<&Dat> {
        self.indexes.get(key)
    }
    
    /// Gets metadata by key.
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}

/// Trait for types that can be stored in and retrieved from databases.
pub trait DatabaseStorable {
    /// Converts to a database record using the specified storage format.
    fn to_database_record(&self, format: StorageFormat) -> Outcome<DatabaseRecord>;
    
    /// Creates an instance from a database record.
    fn from_database_record(record: &DatabaseRecord) -> Outcome<Self>
    where
        Self: Sized;
    
    /// Returns the recommended storage format for this type.
    fn recommended_storage_format() -> StorageFormat;
    
    /// Creates query indexes for efficient database operations.
    fn create_indexes(&self) -> HashMap<String, Dat>;
}

impl DatabaseStorable for CalClock {
    fn to_database_record(&self, format: StorageFormat) -> Outcome<DatabaseRecord> {
        let primary_data = match format {
            StorageFormat::Binary => {
                let nanos = res!(self.to_nanos_since_epoch());
                let zone_id = self.zone().id().to_string();
                tup2dat!(nanos, zone_id)
            },
            
            StorageFormat::Iso8601 => {
                let iso_string = res!(self.to_iso8601());
                Dat::Str(iso_string)
            },
            
            StorageFormat::Component => {
                let mut components = BTreeMap::new();
                components.insert(Dat::Str("year".to_string()), Dat::I32(self.year()));
                components.insert(Dat::Str("month".to_string()), Dat::U8(self.month()));
                components.insert(Dat::Str("day".to_string()), Dat::U8(self.day()));
                components.insert(Dat::Str("hour".to_string()), Dat::U8(self.hour()));
                components.insert(Dat::Str("minute".to_string()), Dat::U8(self.minute()));
                components.insert(Dat::Str("second".to_string()), Dat::U8(self.second()));
                components.insert(Dat::Str("nanosecond".to_string()), Dat::U32(self.nanosecond()));
                components.insert(Dat::Str("timezone".to_string()), Dat::Str(self.zone().id().to_string()));
                Dat::Map(components)
            },
            
            StorageFormat::UnixTimestamp => {
                let millis = res!(self.to_millis());
                let zone_id = self.zone().id().to_string();
                tup2dat!(millis, zone_id)
            },
        };
        
        let mut record = DatabaseRecord::new(primary_data, format);
        
        // Add query indexes
        let indexes = self.create_indexes();
        for (key, value) in indexes {
            record.add_index(&key, value);
        }
        
        // Add metadata
        record.add_metadata("type", "CalClock");
        record.add_metadata("timezone", self.zone().id());
        record.add_metadata("leap_second", &self.is_leap_second().to_string());
        
        Ok(record)
    }
    
    fn from_database_record(record: &DatabaseRecord) -> Outcome<Self> {
        match record.storage_format {
            StorageFormat::Binary => {
                let (nanos, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_i64().ok_or_else(|| err!("Invalid nanos in binary format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in binary format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for binary format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_nanos_since_epoch(nanos, zone)
            },
            
            StorageFormat::Iso8601 => {
                let iso_string = res!(record.primary_data.get_string().ok_or_else(|| err!("Expected string for ISO format"; Invalid, Input)));
                Self::parse_iso(&iso_string)
            },
            
            StorageFormat::Component => {
                let components = res!(record.primary_data.get_map().ok_or_else(|| err!("Expected map for component format"; Invalid, Input)));
                let year = res!(components.get(&Dat::Str("year".to_string()))
                    .ok_or_else(|| err!("Missing year component"; Invalid, Input))?
                    .get_i32().ok_or_else(|| err!("Invalid year type"; Invalid, Input)));
                let month = res!(components.get(&Dat::Str("month".to_string()))
                    .ok_or_else(|| err!("Missing month component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid month type"; Invalid, Input)));
                let day = res!(components.get(&Dat::Str("day".to_string()))
                    .ok_or_else(|| err!("Missing day component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid day type"; Invalid, Input)));
                let hour = res!(components.get(&Dat::Str("hour".to_string()))
                    .ok_or_else(|| err!("Missing hour component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid hour type"; Invalid, Input)));
                let minute = res!(components.get(&Dat::Str("minute".to_string()))
                    .ok_or_else(|| err!("Missing minute component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid minute type"; Invalid, Input)));
                let second = res!(components.get(&Dat::Str("second".to_string()))
                    .ok_or_else(|| err!("Missing second component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid second type"; Invalid, Input)));
                let nanosecond = res!(components.get(&Dat::Str("nanosecond".to_string()))
                    .ok_or_else(|| err!("Missing nanosecond component"; Invalid, Input))?
                    .get_u32().ok_or_else(|| err!("Invalid nanosecond type"; Invalid, Input)));
                let zone_id = res!(components.get(&Dat::Str("timezone".to_string()))
                    .ok_or_else(|| err!("Missing timezone component"; Invalid, Input))?
                    .get_string().ok_or_else(|| err!("Invalid timezone type"; Invalid, Input)));
                let zone = res!(CalClockZone::new(&zone_id));
                
                Self::new(year, month, day, hour, minute, second, nanosecond, zone)
            },
            
            StorageFormat::UnixTimestamp => {
                let (millis, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_i64().ok_or_else(|| err!("Invalid millis in timestamp format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in timestamp format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for timestamp format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_millis(millis, zone)
            },
        }
    }
    
    fn recommended_storage_format() -> StorageFormat {
        StorageFormat::Binary
    }
    
    fn create_indexes(&self) -> HashMap<String, Dat> {
        let mut indexes = HashMap::new();
        
        // Time-based indexes
        indexes.insert("year".to_string(), Dat::I32(self.year()));
        indexes.insert("month".to_string(), Dat::U8(self.month()));
        indexes.insert("day".to_string(), Dat::U8(self.day()));
        indexes.insert("hour".to_string(), Dat::U8(self.hour()));
        indexes.insert("day_of_week".to_string(), Dat::U8(self.day_of_week().of()));
        if let Ok(day_of_year) = self.day_of_year() {
            indexes.insert("day_of_year".to_string(), Dat::U16(day_of_year));
        }
        
        // Timestamp indexes for range queries
        if let Ok(nanos) = self.to_nanos_since_epoch() {
            indexes.insert("nanos_since_epoch".to_string(), Dat::I64(nanos));
        }
        if let Ok(millis) = self.to_millis() {
            indexes.insert("millis_since_epoch".to_string(), Dat::I64(millis));
        }
        
        // Calendar indexes
        indexes.insert("year_month".to_string(), Dat::Str(fmt!("{}-{:02}", self.year(), self.month())));
        indexes.insert("date".to_string(), Dat::Str(fmt!("{}-{:02}-{:02}", self.year(), self.month(), self.day())));
        
        // Special flags
        indexes.insert("is_leap_second".to_string(), Dat::Bool(self.is_leap_second()));
        indexes.insert("timezone".to_string(), Dat::Str(self.zone().id().to_string()));
        
        indexes
    }
}

impl DatabaseStorable for ClockTime {
    fn to_database_record(&self, format: StorageFormat) -> Outcome<DatabaseRecord> {
        let primary_data = match format {
            StorageFormat::Binary => {
                let nanos = self.to_nanos_of_day();
                let zone_id = self.zone().id().to_string();
                tup2dat!(nanos, zone_id)
            },
            
            StorageFormat::Iso8601 => {
                Dat::Str(self.to_iso_string())
            },
            
            StorageFormat::Component => {
                let mut components = BTreeMap::new();
                components.insert(Dat::Str("hour".to_string()), Dat::U8(self.hour().of()));
                components.insert(Dat::Str("minute".to_string()), Dat::U8(self.minute().of()));
                components.insert(Dat::Str("second".to_string()), Dat::U8(self.second().of()));
                components.insert(Dat::Str("nanosecond".to_string()), Dat::U32(self.nanosecond().of()));
                components.insert(Dat::Str("timezone".to_string()), Dat::Str(self.zone().id().to_string()));
                Dat::Map(components)
            },
            
            StorageFormat::UnixTimestamp => {
                // For time-only, we store milliseconds of day
                let millis = self.millis_of_day();
                let zone_id = self.zone().id().to_string();
                tup2dat!(millis, zone_id)
            },
        };
        
        let mut record = DatabaseRecord::new(primary_data, format);
        
        // Add query indexes
        let indexes = self.create_indexes();
        for (key, value) in indexes {
            record.add_index(&key, value);
        }
        
        // Add metadata
        record.add_metadata("type", "ClockTime");
        record.add_metadata("timezone", self.zone().id());
        record.add_metadata("is_leap_second", &self.is_leap_second().to_string());
        record.add_metadata("is_end_of_day", &self.is_end_of_day().to_string());
        
        Ok(record)
    }
    
    fn from_database_record(record: &DatabaseRecord) -> Outcome<Self> {
        match record.storage_format {
            StorageFormat::Binary => {
                let (nanos, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_u64().ok_or_else(|| err!("Invalid nanos in binary format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in binary format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for binary format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_nanos_of_day(nanos, zone)
            },
            
            StorageFormat::Iso8601 => {
                let iso_string = res!(record.primary_data.get_string().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                // Parse ISO time string - this would need a proper parser
                // For now, return an error as ClockTime doesn't have direct ISO parsing
                Err(err!("ClockTime ISO parsing not implemented"; Unimplemented))
            },
            
            StorageFormat::Component => {
                let components = res!(record.primary_data.get_map().ok_or_else(|| err!("Expected map"; Invalid, Input)));
                let hour = res!(components.get(&Dat::Str("hour".to_string()))
                    .ok_or_else(|| err!("Missing hour component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let minute = res!(components.get(&Dat::Str("minute".to_string()))
                    .ok_or_else(|| err!("Missing minute component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let second = res!(components.get(&Dat::Str("second".to_string()))
                    .ok_or_else(|| err!("Missing second component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let nanosecond = res!(components.get(&Dat::Str("nanosecond".to_string()))
                    .ok_or_else(|| err!("Missing nanosecond component"; Invalid, Input))?
                    .get_u32().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let zone_id = res!(components.get(&Dat::Str("timezone".to_string()))
                    .ok_or_else(|| err!("Missing timezone component"; Invalid, Input))?
                    .get_string().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let zone = res!(CalClockZone::new(&zone_id));
                
                Self::new(hour, minute, second, nanosecond, zone)
            },
            
            StorageFormat::UnixTimestamp => {
                let (millis, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_u32().ok_or_else(|| err!("Invalid millis in timestamp format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in timestamp format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for timestamp format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_millis_of_day(millis, zone)
            },
        }
    }
    
    fn recommended_storage_format() -> StorageFormat {
        StorageFormat::Binary
    }
    
    fn create_indexes(&self) -> HashMap<String, Dat> {
        let mut indexes = HashMap::new();
        
        // Time component indexes
        indexes.insert("hour".to_string(), Dat::U8(self.hour().of()));
        indexes.insert("minute".to_string(), Dat::U8(self.minute().of()));
        indexes.insert("second".to_string(), Dat::U8(self.second().of()));
        indexes.insert("hour_minute".to_string(), Dat::Str(fmt!("{:02}:{:02}", self.hour().of(), self.minute().of())));
        
        // Nanosecond-based indexes for precise queries
        indexes.insert("nanos_of_day".to_string(), Dat::U64(self.to_nanos_of_day()));
        indexes.insert("millis_of_day".to_string(), Dat::U32(self.millis_of_day()));
        
        // Special time flags
        indexes.insert("is_leap_second".to_string(), Dat::Bool(self.is_leap_second()));
        indexes.insert("is_end_of_day".to_string(), Dat::Bool(self.is_end_of_day()));
        indexes.insert("is_potential_leap_second".to_string(), Dat::Bool(self.is_potential_leap_second()));
        
        // Time periods for grouping
        let hour = self.hour().of();
        indexes.insert("time_period".to_string(), Dat::Str(
            if hour < 6 { "night" }
            else if hour < 12 { "morning" }
            else if hour < 18 { "afternoon" }
            else { "evening" }
            .to_string()
        ));
        
        indexes.insert("timezone".to_string(), Dat::Str(self.zone().id().to_string()));
        
        indexes
    }
}

impl DatabaseStorable for CalendarDate {
    fn to_database_record(&self, format: StorageFormat) -> Outcome<DatabaseRecord> {
        let primary_data = match format {
            StorageFormat::Binary => {
                let julian_day = self.to_julian_day_number();
                let zone_id = self.zone().id().to_string();
                tup2dat!(julian_day, zone_id)
            },
            
            StorageFormat::Iso8601 => {
                Dat::Str(self.to_string())
            },
            
            StorageFormat::Component => {
                let mut components = BTreeMap::new();
                components.insert(Dat::Str("year".to_string()), Dat::I32(self.year()));
                components.insert(Dat::Str("month".to_string()), Dat::U8(self.month()));
                components.insert(Dat::Str("day".to_string()), Dat::U8(self.day()));
                components.insert(Dat::Str("timezone".to_string()), Dat::Str(self.zone().id().to_string()));
                Dat::Map(components)
            },
            
            StorageFormat::UnixTimestamp => {
                // For dates, we use days since epoch
                let days = res!(self.days_since_epoch());
                let zone_id = self.zone().id().to_string();
                tup2dat!(days, zone_id)
            },
        };
        
        let mut record = DatabaseRecord::new(primary_data, format);
        
        // Add query indexes
        let indexes = self.create_indexes();
        for (key, value) in indexes {
            record.add_index(&key, value);
        }
        
        // Add metadata
        record.add_metadata("type", "CalendarDate");
        record.add_metadata("timezone", self.zone().id());
        record.add_metadata("is_leap_year", &self.is_leap_year().to_string());
        
        Ok(record)
    }
    
    fn from_database_record(record: &DatabaseRecord) -> Outcome<Self> {
        match record.storage_format {
            StorageFormat::Binary => {
                let (julian_day, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_i64().ok_or_else(|| err!("Invalid julian_day in binary format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in binary format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for binary format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_julian_day_number(julian_day, zone)
            },
            
            StorageFormat::Iso8601 => {
                let iso_string = res!(record.primary_data.get_string().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                // Parse using the default zone (UTC) and then convert if needed
                Self::parse(&iso_string, CalClockZone::utc())
            },
            
            StorageFormat::Component => {
                let components = res!(record.primary_data.get_map().ok_or_else(|| err!("Expected map"; Invalid, Input)));
                let year = res!(components.get(&Dat::Str("year".to_string()))
                    .ok_or_else(|| err!("Missing year component"; Invalid, Input))?
                    .get_i32().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let month = res!(components.get(&Dat::Str("month".to_string()))
                    .ok_or_else(|| err!("Missing month component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let day = res!(components.get(&Dat::Str("day".to_string()))
                    .ok_or_else(|| err!("Missing day component"; Invalid, Input))?
                    .get_u8().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let zone_id = res!(components.get(&Dat::Str("timezone".to_string()))
                    .ok_or_else(|| err!("Missing timezone component"; Invalid, Input))?
                    .get_string().ok_or_else(|| err!("Invalid component type"; Invalid, Input)));
                let zone = res!(CalClockZone::new(&zone_id));
                
                Self::new(year, month, day, zone)
            },
            
            StorageFormat::UnixTimestamp => {
                let (days, zone_id) = if let Dat::Tup2(arr) = &record.primary_data {
                    let first = res!(arr[0].get_i32().ok_or_else(|| err!("Invalid days in timestamp format"; Invalid, Input)));
                    let second = res!(arr[1].get_string().ok_or_else(|| err!("Invalid zone_id in timestamp format"; Invalid, Input)));
                    (first, second)
                } else {
                    return Err(err!("Expected Tup2 for timestamp format"; Invalid, Input));
                };
                let zone = res!(CalClockZone::new(&zone_id));
                Self::from_days_since_epoch(days as i64, zone)
            },
        }
    }
    
    fn recommended_storage_format() -> StorageFormat {
        StorageFormat::Binary
    }
    
    fn create_indexes(&self) -> HashMap<String, Dat> {
        let mut indexes = HashMap::new();
        
        // Date component indexes
        indexes.insert("year".to_string(), Dat::I32(self.year()));
        indexes.insert("month".to_string(), Dat::U8(self.month()));
        indexes.insert("day".to_string(), Dat::U8(self.day()));
        indexes.insert("day_of_week".to_string(), Dat::U8(self.day_of_week().of() as u8));
        if let Ok(day_of_year) = self.day_of_year() {
            indexes.insert("day_of_year".to_string(), Dat::U16(day_of_year));
        }
        
        // Julian day for efficient date calculations
        indexes.insert("julian_day".to_string(), Dat::I64(self.to_julian_day_number()));
        if let Ok(days) = self.days_since_epoch() {
            indexes.insert("days_since_epoch".to_string(), Dat::I64(days));
        }
        
        // Calendar groupings
        indexes.insert("year_month".to_string(), Dat::Str(fmt!("{}-{:02}", self.year(), self.month())));
        indexes.insert("quarter".to_string(), Dat::U8((self.month() - 1) / 3 + 1));
        if let Ok(week) = self.week_of_year() {
            indexes.insert("week_of_year".to_string(), Dat::U8(week));
        }
        
        // Special date flags
        indexes.insert("is_leap_year".to_string(), Dat::Bool(self.is_leap_year()));
        indexes.insert("is_weekend".to_string(), Dat::Bool(self.is_weekend()));
        
        indexes.insert("timezone".to_string(), Dat::Str(self.zone().id().to_string()));
        
        indexes
    }
}