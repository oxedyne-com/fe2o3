use crate::{
    time::{CalClock, CalClockZone},
    clock::ClockTime,
    calendar::CalendarDate,
    database::StorageFormat,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::HashMap;

/// SQL-compatible format methods for database schema generation and queries.
pub trait SqlCompatible {
    /// Returns the SQL data type definition for storing this type.
    fn sql_data_type(format: StorageFormat) -> &'static str;
    
    /// Returns the SQL CREATE TABLE statement for this type.
    fn sql_create_table(table_name: &str, format: StorageFormat) -> String;
    
    /// Returns SQL to create indexes for efficient queries.
    fn sql_create_indexes(table_name: &str) -> Vec<String>;
    
    /// Converts to SQL-compatible values for INSERT/UPDATE statements.
    fn to_sql_values(&self, format: StorageFormat) -> Outcome<HashMap<String, String>>;
    
    /// Returns SQL WHERE clause fragments for common queries.
    fn sql_query_fragments() -> Vec<(&'static str, &'static str)>;
}

impl SqlCompatible for CalClock {
    fn sql_data_type(format: StorageFormat) -> &'static str {
        match format {
            StorageFormat::Binary => "BIGINT, VARCHAR(50)", // nanos, timezone
            StorageFormat::Iso8601 => "VARCHAR(35)",         // ISO 8601 string
            StorageFormat::Component => "JSONB",             // JSON object
            StorageFormat::UnixTimestamp => "BIGINT, VARCHAR(50)", // millis, timezone
        }
    }
    
    fn sql_create_table(table_name: &str, format: StorageFormat) -> String {
        match format {
            StorageFormat::Binary => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    timestamp_nanos BIGINT NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    year INTEGER NOT NULL,\n\
                    month SMALLINT NOT NULL,\n\
                    day SMALLINT NOT NULL,\n\
                    hour SMALLINT NOT NULL,\n\
                    minute SMALLINT NOT NULL,\n\
                    second SMALLINT NOT NULL,\n\
                    nanosecond INTEGER NOT NULL,\n\
                    day_of_week SMALLINT NOT NULL,\n\
                    day_of_year INTEGER NOT NULL,\n\
                    is_leap_second BOOLEAN NOT NULL DEFAULT FALSE,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Iso8601 => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    iso_timestamp VARCHAR(35) NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Component => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    datetime_components JSONB NOT NULL,\n\
                    year INTEGER GENERATED ALWAYS AS ((datetime_components->>'year')::INTEGER) STORED,\n\
                    month INTEGER GENERATED ALWAYS AS ((datetime_components->>'month')::INTEGER) STORED,\n\
                    day INTEGER GENERATED ALWAYS AS ((datetime_components->>'day')::INTEGER) STORED,\n\
                    timezone VARCHAR(50) GENERATED ALWAYS AS (datetime_components->>'timezone') STORED,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::UnixTimestamp => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    unix_millis BIGINT NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
        }
    }
    
    fn sql_create_indexes(table_name: &str) -> Vec<String> {
        vec![
            fmt!("CREATE INDEX idx_{}_timestamp ON {} (timestamp_nanos);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_timezone ON {} (timezone);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_date ON {} (year, month, day);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_time ON {} (hour, minute);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_year_month ON {} (year, month);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_day_of_week ON {} (day_of_week);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_leap_second ON {} (is_leap_second) WHERE is_leap_second = TRUE;", table_name, table_name),
        ]
    }
    
    fn to_sql_values(&self, format: StorageFormat) -> Outcome<HashMap<String, String>> {
        let mut values = HashMap::new();
        
        match format {
            StorageFormat::Binary => {
                let nanos = res!(self.to_nanos_since_epoch());
                values.insert("timestamp_nanos".to_string(), nanos.to_string());
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
                values.insert("year".to_string(), self.year().to_string());
                values.insert("month".to_string(), self.month().to_string());
                values.insert("day".to_string(), self.day().to_string());
                values.insert("hour".to_string(), self.hour().to_string());
                values.insert("minute".to_string(), self.minute().to_string());
                values.insert("second".to_string(), self.second().to_string());
                values.insert("nanosecond".to_string(), self.nanosecond().to_string());
                values.insert("day_of_week".to_string(), (self.day_of_week().of() as u8).to_string());
                values.insert("day_of_year".to_string(), res!(self.day_of_year()).to_string());
                values.insert("is_leap_second".to_string(), self.is_leap_second().to_string());
            },
            
            StorageFormat::Iso8601 => {
                values.insert("iso_timestamp".to_string(), fmt!("'{}'", res!(self.to_iso8601())));
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
            
            StorageFormat::Component => {
                let components = fmt!(
                    "'{{\
                        \"year\": {}, \
                        \"month\": {}, \
                        \"day\": {}, \
                        \"hour\": {}, \
                        \"minute\": {}, \
                        \"second\": {}, \
                        \"nanosecond\": {}, \
                        \"timezone\": \"{}\"\
                    }}'",
                    self.year(), self.month(), self.day(),
                    self.hour(), self.minute(), self.second(), self.nanosecond(),
                    self.zone().id()
                );
                values.insert("datetime_components".to_string(), components);
            },
            
            StorageFormat::UnixTimestamp => {
                let millis = res!(self.to_millis());
                values.insert("unix_millis".to_string(), millis.to_string());
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
        }
        
        Ok(values)
    }
    
    fn sql_query_fragments() -> Vec<(&'static str, &'static str)> {
        vec![
            ("date_range", "timestamp_nanos BETWEEN $1 AND $2"),
            ("year_filter", "year = $1"),
            ("month_filter", "year = $1 AND month = $2"),
            ("day_filter", "year = $1 AND month = $2 AND day = $3"),
            ("timezone_filter", "timezone = $1"),
            ("time_range", "hour BETWEEN $1 AND $2"),
            ("weekend_filter", "day_of_week IN (6, 7)"), // Saturday, Sunday
            ("weekday_filter", "day_of_week BETWEEN 1 AND 5"),
            ("leap_seconds", "is_leap_second = TRUE"),
            ("recent_dates", "timestamp_nanos >= $1"), // Recent since timestamp
        ]
    }
}

impl SqlCompatible for ClockTime {
    fn sql_data_type(format: StorageFormat) -> &'static str {
        match format {
            StorageFormat::Binary => "BIGINT, VARCHAR(50)", // nanos_of_day, timezone
            StorageFormat::Iso8601 => "TIME",                // ISO time
            StorageFormat::Component => "JSONB",             // JSON object
            StorageFormat::UnixTimestamp => "INTEGER, VARCHAR(50)", // millis_of_day, timezone
        }
    }
    
    fn sql_create_table(table_name: &str, format: StorageFormat) -> String {
        match format {
            StorageFormat::Binary => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    nanos_of_day BIGINT NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    hour SMALLINT NOT NULL,\n\
                    minute SMALLINT NOT NULL,\n\
                    second SMALLINT NOT NULL,\n\
                    nanosecond INTEGER NOT NULL,\n\
                    is_leap_second BOOLEAN NOT NULL DEFAULT FALSE,\n\
                    is_end_of_day BOOLEAN NOT NULL DEFAULT FALSE,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Iso8601 => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    time_value TIME NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Component => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    time_components JSONB NOT NULL,\n\
                    hour INTEGER GENERATED ALWAYS AS ((time_components->>'hour')::INTEGER) STORED,\n\
                    minute INTEGER GENERATED ALWAYS AS ((time_components->>'minute')::INTEGER) STORED,\n\
                    timezone VARCHAR(50) GENERATED ALWAYS AS (time_components->>'timezone') STORED,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::UnixTimestamp => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    millis_of_day INTEGER NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
        }
    }
    
    fn sql_create_indexes(table_name: &str) -> Vec<String> {
        vec![
            fmt!("CREATE INDEX idx_{}_nanos_of_day ON {} (nanos_of_day);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_timezone ON {} (timezone);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_hour ON {} (hour);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_hour_minute ON {} (hour, minute);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_leap_second ON {} (is_leap_second) WHERE is_leap_second = TRUE;", table_name, table_name),
        ]
    }
    
    fn to_sql_values(&self, format: StorageFormat) -> Outcome<HashMap<String, String>> {
        let mut values = HashMap::new();
        
        match format {
            StorageFormat::Binary => {
                values.insert("nanos_of_day".to_string(), self.to_nanos_of_day().to_string());
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
                values.insert("hour".to_string(), self.hour().of().to_string());
                values.insert("minute".to_string(), self.minute().of().to_string());
                values.insert("second".to_string(), self.second().of().to_string());
                values.insert("nanosecond".to_string(), self.nanosecond().of().to_string());
                values.insert("is_leap_second".to_string(), self.is_leap_second().to_string());
                values.insert("is_end_of_day".to_string(), self.is_end_of_day().to_string());
            },
            
            StorageFormat::Iso8601 => {
                values.insert("time_value".to_string(), fmt!("'{}'", self.to_iso_string()));
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
            
            StorageFormat::Component => {
                let components = fmt!(
                    "'{{\
                        \"hour\": {}, \
                        \"minute\": {}, \
                        \"second\": {}, \
                        \"nanosecond\": {}, \
                        \"timezone\": \"{}\"\
                    }}'",
                    self.hour().of(), self.minute().of(), self.second().of(), 
                    self.nanosecond().of(), self.zone().id()
                );
                values.insert("time_components".to_string(), components);
            },
            
            StorageFormat::UnixTimestamp => {
                values.insert("millis_of_day".to_string(), self.millis_of_day().to_string());
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
        }
        
        Ok(values)
    }
    
    fn sql_query_fragments() -> Vec<(&'static str, &'static str)> {
        vec![
            ("time_range", "nanos_of_day BETWEEN $1 AND $2"),
            ("hour_filter", "hour = $1"),
            ("hour_range", "hour BETWEEN $1 AND $2"),
            ("minute_filter", "hour = $1 AND minute = $2"),
            ("timezone_filter", "timezone = $1"),
            ("morning_hours", "hour BETWEEN 6 AND 11"),
            ("afternoon_hours", "hour BETWEEN 12 AND 17"),
            ("evening_hours", "hour BETWEEN 18 AND 23"),
            ("night_hours", "hour BETWEEN 0 AND 5"),
            ("business_hours", "hour BETWEEN 9 AND 17"),
            ("leap_seconds", "is_leap_second = TRUE"),
            ("end_of_day", "is_end_of_day = TRUE"),
        ]
    }
}

impl SqlCompatible for CalendarDate {
    fn sql_data_type(format: StorageFormat) -> &'static str {
        match format {
            StorageFormat::Binary => "BIGINT, VARCHAR(50)", // julian_day, timezone
            StorageFormat::Iso8601 => "DATE",                // ISO date
            StorageFormat::Component => "JSONB",             // JSON object
            StorageFormat::UnixTimestamp => "INTEGER, VARCHAR(50)", // days_since_epoch, timezone
        }
    }
    
    fn sql_create_table(table_name: &str, format: StorageFormat) -> String {
        match format {
            StorageFormat::Binary => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    julian_day BIGINT NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    year INTEGER NOT NULL,\n\
                    month SMALLINT NOT NULL,\n\
                    day SMALLINT NOT NULL,\n\
                    day_of_week SMALLINT NOT NULL,\n\
                    day_of_year INTEGER NOT NULL,\n\
                    week_of_year SMALLINT NOT NULL,\n\
                    quarter SMALLINT NOT NULL,\n\
                    is_leap_year BOOLEAN NOT NULL DEFAULT FALSE,\n\
                    is_weekend BOOLEAN NOT NULL DEFAULT FALSE,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Iso8601 => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    date_value DATE NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::Component => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    date_components JSONB NOT NULL,\n\
                    year INTEGER GENERATED ALWAYS AS ((date_components->>'year')::INTEGER) STORED,\n\
                    month INTEGER GENERATED ALWAYS AS ((date_components->>'month')::INTEGER) STORED,\n\
                    day INTEGER GENERATED ALWAYS AS ((date_components->>'day')::INTEGER) STORED,\n\
                    timezone VARCHAR(50) GENERATED ALWAYS AS (date_components->>'timezone') STORED,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
            
            StorageFormat::UnixTimestamp => fmt!(
                "CREATE TABLE {} (\n\
                    id SERIAL PRIMARY KEY,\n\
                    days_since_epoch INTEGER NOT NULL,\n\
                    timezone VARCHAR(50) NOT NULL,\n\
                    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()\n\
                );", table_name
            ),
        }
    }
    
    fn sql_create_indexes(table_name: &str) -> Vec<String> {
        vec![
            fmt!("CREATE INDEX idx_{}_julian_day ON {} (julian_day);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_timezone ON {} (timezone);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_year ON {} (year);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_year_month ON {} (year, month);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_date ON {} (year, month, day);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_day_of_week ON {} (day_of_week);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_quarter ON {} (quarter);", table_name, table_name),
            fmt!("CREATE INDEX idx_{}_weekend ON {} (is_weekend) WHERE is_weekend = TRUE;", table_name, table_name),
        ]
    }
    
    fn to_sql_values(&self, format: StorageFormat) -> Outcome<HashMap<String, String>> {
        let mut values = HashMap::new();
        
        match format {
            StorageFormat::Binary => {
                values.insert("julian_day".to_string(), self.to_julian_day_number().to_string());
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
                values.insert("year".to_string(), self.year().to_string());
                values.insert("month".to_string(), self.month().to_string());
                values.insert("day".to_string(), self.day().to_string());
                values.insert("day_of_week".to_string(), (self.day_of_week().of() as u8).to_string());
                values.insert("day_of_year".to_string(), res!(self.day_of_year()).to_string());
                values.insert("week_of_year".to_string(), res!(self.week_of_year()).to_string());
                values.insert("quarter".to_string(), ((self.month() - 1) / 3 + 1).to_string());
                values.insert("is_leap_year".to_string(), self.is_leap_year().to_string());
                values.insert("is_weekend".to_string(), self.is_weekend().to_string());
            },
            
            StorageFormat::Iso8601 => {
                values.insert("date_value".to_string(), fmt!("'{}'", self.to_string()));
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
            
            StorageFormat::Component => {
                let components = fmt!(
                    "'{{\
                        \"year\": {}, \
                        \"month\": {}, \
                        \"day\": {}, \
                        \"timezone\": \"{}\"\
                    }}'",
                    self.year(), self.month(), self.day(), self.zone().id()
                );
                values.insert("date_components".to_string(), components);
            },
            
            StorageFormat::UnixTimestamp => {
                if let Ok(days) = self.days_since_epoch() {
                    values.insert("days_since_epoch".to_string(), days.to_string());
                }
                values.insert("timezone".to_string(), fmt!("'{}'", self.zone().id()));
            },
        }
        
        Ok(values)
    }
    
    fn sql_query_fragments() -> Vec<(&'static str, &'static str)> {
        vec![
            ("date_range", "julian_day BETWEEN $1 AND $2"),
            ("year_filter", "year = $1"),
            ("month_filter", "year = $1 AND month = $2"),
            ("day_filter", "year = $1 AND month = $2 AND day = $3"),
            ("timezone_filter", "timezone = $1"),
            ("weekends", "is_weekend = TRUE"),
            ("weekdays", "is_weekend = FALSE"),
            ("day_of_week", "day_of_week = $1"),
            ("quarter", "quarter = $1"),
            ("leap_years", "is_leap_year = TRUE"),
            ("recent_days", "julian_day >= $1"),
            ("current_month", "year = EXTRACT(YEAR FROM NOW()) AND month = EXTRACT(MONTH FROM NOW())"),
            ("current_year", "year = EXTRACT(YEAR FROM NOW())"),
        ]
    }
}

/// NoSQL document format methods for document databases.
pub trait NoSqlDocument {
    /// Converts to a document format suitable for MongoDB, CouchDB, etc.
    fn to_document(&self) -> HashMap<String, Dat>;
    
    /// Creates from a document format.
    fn from_document(doc: &HashMap<String, Dat>) -> Outcome<Self>
    where
        Self: Sized;
    
    /// Returns field names that should be indexed in document databases.
    fn index_fields() -> Vec<&'static str>;
    
    /// Returns compound index definitions for efficient queries.
    fn compound_indexes() -> Vec<Vec<&'static str>>;
}

impl NoSqlDocument for CalClock {
    fn to_document(&self) -> HashMap<String, Dat> {
        let mut doc = HashMap::new();
        
        // Primary timestamp data
        if let Ok(nanos) = self.to_nanos_since_epoch() {
            doc.insert("timestamp_nanos".to_string(), Dat::I64(nanos));
        }
        if let Ok(millis) = self.to_millis() {
            doc.insert("timestamp_millis".to_string(), Dat::I64(millis));
        }
        
        // Date components
        doc.insert("year".to_string(), Dat::I32(self.year()));
        doc.insert("month".to_string(), Dat::U8(self.month()));
        doc.insert("day".to_string(), Dat::U8(self.day()));
        
        // Time components
        doc.insert("hour".to_string(), Dat::U8(self.hour()));
        doc.insert("minute".to_string(), Dat::U8(self.minute()));
        doc.insert("second".to_string(), Dat::U8(self.second()));
        doc.insert("nanosecond".to_string(), Dat::U32(self.nanosecond()));
        
        // Calendar information
        doc.insert("day_of_week".to_string(), Dat::U8(self.day_of_week().of() as u8));
        if let Ok(day_of_year) = self.day_of_year() {
            doc.insert("day_of_year".to_string(), Dat::U16(day_of_year));
        }
        if let Ok(week) = self.week_of_year() {
            doc.insert("week_of_year".to_string(), Dat::U8(week));
        }
        doc.insert("quarter".to_string(), Dat::U8((self.month() - 1) / 3 + 1));
        
        // Timezone
        doc.insert("timezone".to_string(), Dat::Str(self.zone().id().to_string()));
        
        // Computed fields for queries
        doc.insert("date_str".to_string(), Dat::Str(fmt!("{}-{:02}-{:02}", self.year(), self.month(), self.day())));
        doc.insert("time_str".to_string(), Dat::Str(fmt!("{:02}:{:02}:{:02}", self.hour(), self.minute(), self.second())));
        if let Ok(iso_str) = self.to_iso8601() {
            doc.insert("iso_string".to_string(), Dat::Str(iso_str));
        }
        
        // Flags
        doc.insert("is_leap_second".to_string(), Dat::Bool(self.is_leap_second()));
        doc.insert("is_leap_year".to_string(), Dat::Bool(self.is_leap_year()));
        doc.insert("is_weekend".to_string(), Dat::Bool(self.date().is_weekend()));
        
        doc
    }
    
    fn from_document(doc: &HashMap<String, Dat>) -> Outcome<Self> {
        let year = res!(doc.get("year")
            .ok_or_else(|| err!("Missing year in document"; Invalid, Input))?
            .get_i32().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let month = res!(doc.get("month")
            .ok_or_else(|| err!("Missing month in document"; Invalid, Input))?
            .get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let day = res!(doc.get("day")
            .ok_or_else(|| err!("Missing day in document"; Invalid, Input))?
            .get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let hour = res!(doc.get("hour")
            .ok_or_else(|| err!("Missing hour in document"; Invalid, Input))?
            .get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let minute = res!(doc.get("minute")
            .ok_or_else(|| err!("Missing minute in document"; Invalid, Input))?
            .get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let second = res!(doc.get("second")
            .ok_or_else(|| err!("Missing second in document"; Invalid, Input))?
            .get_u8().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let nanosecond = res!(doc.get("nanosecond")
            .ok_or_else(|| err!("Missing nanosecond in document"; Invalid, Input))?
            .get_u32().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        let timezone_id = res!(doc.get("timezone")
            .ok_or_else(|| err!("Missing timezone in document"; Invalid, Input))?
            .get_string().ok_or_else(|| err!("Invalid type"; Invalid, Input)));
        
        let zone = res!(CalClockZone::new(&timezone_id));
        Self::new(year, month, day, hour, minute, second, nanosecond, zone)
    }
    
    fn index_fields() -> Vec<&'static str> {
        vec![
            "timestamp_nanos",
            "timestamp_millis", 
            "year",
            "month",
            "day",
            "hour",
            "timezone",
            "day_of_week",
            "is_weekend",
            "is_leap_second",
        ]
    }
    
    fn compound_indexes() -> Vec<Vec<&'static str>> {
        vec![
            vec!["year", "month", "day"],
            vec!["year", "month"],
            vec!["timezone", "year"],
            vec!["hour", "minute"],
            vec!["day_of_week", "hour"],
            vec!["timestamp_nanos", "timezone"],
            vec!["is_weekend", "hour"],
        ]
    }
}