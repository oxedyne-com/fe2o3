//! Database index definitions and generation for the datetime types.
//!
//! Emits CREATE INDEX statements for SQL stores and index specifications for
//! MongoDB.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    time::{CalClock, CalClockZone},
    clock::ClockTime,
    calendar::CalendarDate,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum IndexType {
    BTree,                  // range queries and ordering
    Hash,                   // exact equality only
    Partial(String),        // WHERE clause
    Composite(Vec<String>), // column names
    Functional(String),     // computed expression
    FullText,
}

#[derive(Clone, Debug)]
pub struct DatabaseIndex {
    pub name:       String,
    pub fields:     Vec<String>,
    pub index_type: IndexType,
    pub condition:  Option<String>,     // partial indexes only
    pub unique:     bool,
    pub priority:   u8,                 // higher is more important
}

impl DatabaseIndex {
    pub fn new(name: &str, fields: Vec<String>, index_type: IndexType) -> Self {
        Self {
            name: name.to_string(),
            fields,
            index_type,
            condition: None,
            unique: false,
            priority: 5,
        }
    }
    
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }
    
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
    
    pub fn condition(mut self, condition: &str) -> Self {
        self.condition = Some(condition.to_string());
        self
    }
    
    pub fn to_sql(&self, table_name: &str) -> String {
        let unique_clause = if self.unique { "UNIQUE " } else { "" };
        let fields_clause = self.fields.join(", ");
        
        let index_clause = match &self.index_type {
            IndexType::BTree => format!("USING BTREE ({})", fields_clause),
            IndexType::Hash => format!("USING HASH ({})", fields_clause),
            IndexType::Partial(_) => format!("({})", fields_clause),
            IndexType::Composite(_) => format!("({})", fields_clause),
            IndexType::Functional(expr) => format!("({})", expr),
            IndexType::FullText => format!("USING GIN ({})", fields_clause),
        };
        
        let condition_clause = if let Some(ref cond) = self.condition {
            format!(" WHERE {}", cond)
        } else {
            String::new()
        };
        
        format!(
            "CREATE {}INDEX {} ON {} {}{}",
            unique_clause, self.name, table_name, index_clause, condition_clause
        )
    }
    
    /// The key specification first, then the options map.
    pub fn to_mongodb(&self) -> (HashMap<String, i32>, HashMap<String, Dat>) {
        let mut index_spec = HashMap::new();
        let mut options = HashMap::new();
        
        // Field specifications
        for field in &self.fields {
            match &self.index_type {
                IndexType::BTree | IndexType::Hash | IndexType::Composite(_) => {
                    index_spec.insert(field.clone(), 1); // Ascending
                },
                IndexType::FullText => {
                    index_spec.insert(field.clone(), 0); // Text index
                },
                _ => {
                    index_spec.insert(field.clone(), 1);
                }
            }
        }
        
        // Options
        options.insert("name".to_string(), Dat::Str(self.name.clone()));
        if self.unique {
            options.insert("unique".to_string(), Dat::Bool(true));
        }
        
        if let Some(ref cond) = self.condition {
            options.insert("partialFilterExpression".to_string(), Dat::Str(cond.clone()));
        }
        
        (index_spec, options)
    }
}

pub trait IndexGenerator {
    fn generate_indexes(&self, table_name: &str) -> Vec<DatabaseIndex>;
    
    fn generate_query_specific_indexes(&self, table_name: &str, query_patterns: &[&str]) -> Vec<DatabaseIndex>;
    
    /// Create these before the rest.
    fn critical_indexes(&self, table_name: &str) -> Vec<DatabaseIndex>;
}

impl IndexGenerator for CalClock {
    fn generate_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            // Primary timestamp index for range queries
            DatabaseIndex::new(
                &format!("idx_{}_timestamp_nanos", table_name),
                vec!["timestamp_nanos".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            // Timezone index for filtering
            DatabaseIndex::new(
                &format!("idx_{}_timezone", table_name),
                vec!["timezone".to_string()],
                IndexType::Hash,
            ).priority(8),
            
            // Date components for calendar queries
            DatabaseIndex::new(
                &format!("idx_{}_date", table_name),
                vec!["year".to_string(), "month".to_string(), "day".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string()]),
            ).priority(9),
            
            // Year-month for monthly reports
            DatabaseIndex::new(
                &format!("idx_{}_year_month", table_name),
                vec!["year".to_string(), "month".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string()]),
            ).priority(7),
            
            // Hour for time-based filtering
            DatabaseIndex::new(
                &format!("idx_{}_hour", table_name),
                vec!["hour".to_string()],
                IndexType::BTree,
            ).priority(6),
            
            // Day of week for weekly patterns
            DatabaseIndex::new(
                &format!("idx_{}_day_of_week", table_name),
                vec!["day_of_week".to_string()],
                IndexType::Hash,
            ).priority(5),
            
            // Leap seconds (partial index - rare data)
            DatabaseIndex::new(
                &format!("idx_{}_leap_second", table_name),
                vec!["is_leap_second".to_string()],
                IndexType::Partial("is_leap_second = TRUE".to_string()),
            ).condition("is_leap_second = TRUE").priority(3),
            
            // Timezone + timestamp for efficient filtering
            DatabaseIndex::new(
                &format!("idx_{}_timezone_timestamp", table_name),
                vec!["timezone".to_string(), "timestamp_nanos".to_string()],
                IndexType::Composite(vec!["timezone".to_string(), "timestamp_nanos".to_string()]),
            ).priority(8),
            
            // Weekend filter (partial index)
            DatabaseIndex::new(
                &format!("idx_{}_weekend", table_name),
                vec!["day_of_week".to_string()],
                IndexType::Partial("day_of_week IN (6, 7)".to_string()),
            ).condition("day_of_week IN (6, 7)").priority(4),
            
            // Business hours (partial index)
            DatabaseIndex::new(
                &format!("idx_{}_business_hours", table_name),
                vec!["hour".to_string()],
                IndexType::Partial("hour BETWEEN 9 AND 17".to_string()),
            ).condition("hour BETWEEN 9 AND 17").priority(4),
        ]
    }
    
    fn generate_query_specific_indexes(&self, table_name: &str, query_patterns: &[&str]) -> Vec<DatabaseIndex> {
        let mut indexes = Vec::new();
        
        for pattern in query_patterns {
            match *pattern {
                "time_range_queries" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_time_range", table_name),
                        vec!["timestamp_nanos".to_string()],
                        IndexType::BTree,
                    ).priority(10));
                },
                
                "calendar_navigation" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_calendar_nav", table_name),
                        vec!["year".to_string(), "month".to_string(), "day".to_string()],
                        IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string()]),
                    ).priority(9));
                },
                
                "timezone_aware" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_tz_aware", table_name),
                        vec!["timezone".to_string(), "timestamp_nanos".to_string()],
                        IndexType::Composite(vec!["timezone".to_string(), "timestamp_nanos".to_string()]),
                    ).priority(9));
                },
                
                "hourly_analytics" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_hourly", table_name),
                        vec!["year".to_string(), "month".to_string(), "day".to_string(), "hour".to_string()],
                        IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string(), "hour".to_string()]),
                    ).priority(7));
                },
                
                "weekly_patterns" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_weekly", table_name),
                        vec!["day_of_week".to_string(), "hour".to_string()],
                        IndexType::Composite(vec!["day_of_week".to_string(), "hour".to_string()]),
                    ).priority(6));
                },
                
                "recent_data" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_recent", table_name),
                        vec!["timestamp_nanos".to_string()],
                        IndexType::Functional("timestamp_nanos DESC".to_string()),
                    ).priority(8));
                },
                
                _ => {} // Unknown pattern
            }
        }
        
        indexes
    }
    
    fn critical_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            // Primary timestamp index (most critical)
            DatabaseIndex::new(
                &format!("idx_{}_timestamp_nanos", table_name),
                vec!["timestamp_nanos".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            // Date components (very important for calendar queries)
            DatabaseIndex::new(
                &format!("idx_{}_date", table_name),
                vec!["year".to_string(), "month".to_string(), "day".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string()]),
            ).priority(9),
            
            // Timezone filtering (important for multi-timezone systems)
            DatabaseIndex::new(
                &format!("idx_{}_timezone", table_name),
                vec!["timezone".to_string()],
                IndexType::Hash,
            ).priority(8),
        ]
    }
}

impl IndexGenerator for ClockTime {
    fn generate_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            // Primary time index
            DatabaseIndex::new(
                &format!("idx_{}_nanos_of_day", table_name),
                vec!["nanos_of_day".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            // Hour index for time-based queries
            DatabaseIndex::new(
                &format!("idx_{}_hour", table_name),
                vec!["hour".to_string()],
                IndexType::BTree,
            ).priority(8),
            
            // Hour-minute composite for precise time queries
            DatabaseIndex::new(
                &format!("idx_{}_hour_minute", table_name),
                vec!["hour".to_string(), "minute".to_string()],
                IndexType::Composite(vec!["hour".to_string(), "minute".to_string()]),
            ).priority(7),
            
            // Timezone index
            DatabaseIndex::new(
                &format!("idx_{}_timezone", table_name),
                vec!["timezone".to_string()],
                IndexType::Hash,
            ).priority(6),
            
            // Leap second partial index
            DatabaseIndex::new(
                &format!("idx_{}_leap_second", table_name),
                vec!["is_leap_second".to_string()],
                IndexType::Partial("is_leap_second = TRUE".to_string()),
            ).condition("is_leap_second = TRUE").priority(3),
            
            // End of day partial index
            DatabaseIndex::new(
                &format!("idx_{}_end_of_day", table_name),
                vec!["is_end_of_day".to_string()],
                IndexType::Partial("is_end_of_day = TRUE".to_string()),
            ).condition("is_end_of_day = TRUE").priority(2),
        ]
    }
    
    fn generate_query_specific_indexes(&self, table_name: &str, query_patterns: &[&str]) -> Vec<DatabaseIndex> {
        let mut indexes = Vec::new();
        
        for pattern in query_patterns {
            match *pattern {
                "business_hours" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_business_hours", table_name),
                        vec!["hour".to_string()],
                        IndexType::Partial("hour BETWEEN 9 AND 17".to_string()),
                    ).condition("hour BETWEEN 9 AND 17").priority(7));
                },
                
                "time_periods" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_time_periods", table_name),
                        vec!["hour".to_string()],
                        IndexType::BTree,
                    ).priority(6));
                },
                
                "precise_timing" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_precise", table_name),
                        vec!["hour".to_string(), "minute".to_string(), "second".to_string()],
                        IndexType::Composite(vec!["hour".to_string(), "minute".to_string(), "second".to_string()]),
                    ).priority(8));
                },
                
                _ => {}
            }
        }
        
        indexes
    }
    
    fn critical_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            DatabaseIndex::new(
                &format!("idx_{}_nanos_of_day", table_name),
                vec!["nanos_of_day".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            DatabaseIndex::new(
                &format!("idx_{}_hour", table_name),
                vec!["hour".to_string()],
                IndexType::BTree,
            ).priority(8),
        ]
    }
}

impl IndexGenerator for CalendarDate {
    fn generate_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            // Primary date index
            DatabaseIndex::new(
                &format!("idx_{}_julian_day", table_name),
                vec!["julian_day".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            // Year index for yearly queries
            DatabaseIndex::new(
                &format!("idx_{}_year", table_name),
                vec!["year".to_string()],
                IndexType::BTree,
            ).priority(9),
            
            // Year-month composite
            DatabaseIndex::new(
                &format!("idx_{}_year_month", table_name),
                vec!["year".to_string(), "month".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string()]),
            ).priority(8),
            
            // Full date composite
            DatabaseIndex::new(
                &format!("idx_{}_date", table_name),
                vec!["year".to_string(), "month".to_string(), "day".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string()]),
            ).priority(9),
            
            // Day of week for weekly patterns
            DatabaseIndex::new(
                &format!("idx_{}_day_of_week", table_name),
                vec!["day_of_week".to_string()],
                IndexType::Hash,
            ).priority(6),
            
            // Quarter for quarterly reports
            DatabaseIndex::new(
                &format!("idx_{}_quarter", table_name),
                vec!["quarter".to_string()],
                IndexType::Hash,
            ).priority(5),
            
            // Weekend partial index
            DatabaseIndex::new(
                &format!("idx_{}_weekend", table_name),
                vec!["is_weekend".to_string()],
                IndexType::Partial("is_weekend = TRUE".to_string()),
            ).condition("is_weekend = TRUE").priority(4),
            
            // Leap year partial index
            DatabaseIndex::new(
                &format!("idx_{}_leap_year", table_name),
                vec!["is_leap_year".to_string()],
                IndexType::Partial("is_leap_year = TRUE".to_string()),
            ).condition("is_leap_year = TRUE").priority(3),
            
            // Timezone index
            DatabaseIndex::new(
                &format!("idx_{}_timezone", table_name),
                vec!["timezone".to_string()],
                IndexType::Hash,
            ).priority(6),
        ]
    }
    
    fn generate_query_specific_indexes(&self, table_name: &str, query_patterns: &[&str]) -> Vec<DatabaseIndex> {
        let mut indexes = Vec::new();
        
        for pattern in query_patterns {
            match *pattern {
                "date_ranges" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_date_range", table_name),
                        vec!["julian_day".to_string()],
                        IndexType::BTree,
                    ).priority(10));
                },
                
                "monthly_reports" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_monthly", table_name),
                        vec!["year".to_string(), "month".to_string()],
                        IndexType::Composite(vec!["year".to_string(), "month".to_string()]),
                    ).priority(8));
                },
                
                "quarterly_reports" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_quarterly", table_name),
                        vec!["year".to_string(), "quarter".to_string()],
                        IndexType::Composite(vec!["year".to_string(), "quarter".to_string()]),
                    ).priority(7));
                },
                
                "weekly_analysis" => {
                    indexes.push(DatabaseIndex::new(
                        &format!("idx_{}_weekly", table_name),
                        vec!["day_of_week".to_string()],
                        IndexType::Hash,
                    ).priority(6));
                },
                
                _ => {}
            }
        }
        
        indexes
    }
    
    fn critical_indexes(&self, table_name: &str) -> Vec<DatabaseIndex> {
        vec![
            DatabaseIndex::new(
                &format!("idx_{}_julian_day", table_name),
                vec!["julian_day".to_string()],
                IndexType::BTree,
            ).priority(10),
            
            DatabaseIndex::new(
                &format!("idx_{}_year", table_name),
                vec!["year".to_string()],
                IndexType::BTree,
            ).priority(9),
            
            DatabaseIndex::new(
                &format!("idx_{}_date", table_name),
                vec!["year".to_string(), "month".to_string(), "day".to_string()],
                IndexType::Composite(vec!["year".to_string(), "month".to_string(), "day".to_string()]),
            ).priority(9),
        ]
    }
}

pub mod index_utils {
    use super::*;
    
    pub fn generate_create_indexes_sql(table_name: &str, datetime_type: &str) -> Outcome<String> {
        let mut sql = String::new();
        sql.push_str(&format!("-- Database indexes for {} table\n", table_name));
        sql.push_str(&format!("-- Generated for datetime type: {}\n\n", datetime_type));
        
        let indexes = match datetime_type {
            "CalClock" => {
                let sample = CalClock::new(2024, 1, 1, 0, 0, 0, 0, CalClockZone::utc()).unwrap();
                sample.generate_indexes(table_name)
            },
            "ClockTime" => {
                let sample = ClockTime::new(0, 0, 0, 0, CalClockZone::utc()).unwrap();
                sample.generate_indexes(table_name)
            },
            "CalendarDate" => {
                let sample = CalendarDate::new(2024, 1, 1, CalClockZone::utc()).unwrap();
                sample.generate_indexes(table_name)
            },
            _ => return Err(err!("Unknown datetime type: {}", datetime_type; Invalid, Input)),
        };
        
        // Sort by priority (highest first)
        let mut sorted_indexes = indexes;
        sorted_indexes.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        for index in sorted_indexes {
            sql.push_str(&format!("-- Priority: {}\n", index.priority));
            sql.push_str(&index.to_sql(table_name));
            sql.push_str(";\n\n");
        }
        
        Ok(sql)
    }
    
    pub fn generate_mongodb_indexes(collection_name: &str, datetime_type: &str) -> Outcome<String> {
        let indexes = match datetime_type {
            "CalClock" => {
                let sample = CalClock::new(2024, 1, 1, 0, 0, 0, 0, CalClockZone::utc()).unwrap();
                sample.generate_indexes(collection_name)
            },
            "ClockTime" => {
                let sample = ClockTime::new(0, 0, 0, 0, CalClockZone::utc()).unwrap();
                sample.generate_indexes(collection_name)
            },
            "CalendarDate" => {
                let sample = CalendarDate::new(2024, 1, 1, CalClockZone::utc()).unwrap();
                sample.generate_indexes(collection_name)
            },
            _ => return Err(err!("Unknown datetime type: {}", datetime_type; Invalid, Input)),
        };
        
        let mut commands = String::new();
        commands.push_str(&format!("// MongoDB indexes for {} collection\n", collection_name));
        commands.push_str(&format!("// Generated for datetime type: {}\n\n", datetime_type));
        
        for index in indexes {
            let (spec, options) = index.to_mongodb();
            commands.push_str(&format!("// Priority: {}\n", index.priority));
            commands.push_str(&format!("db.{}.createIndex(", collection_name));
            commands.push_str(&format!("{:?}, {:?}", spec, options));
            commands.push_str(");\n\n");
        }
        
        Ok(commands)
    }
}