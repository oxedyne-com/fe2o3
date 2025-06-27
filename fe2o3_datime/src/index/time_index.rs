/// Primary time indexing structure for fast temporal lookups
/// 
/// This module provides the main TimeIndex structure for indexing
/// time-based data with efficient lookup and range query capabilities.

use oxedyne_fe2o3_core::prelude::*;
use crate::time::{CalClock, CalClockZone};
use std::{
    collections::{HashMap, BTreeMap},
    fmt,
    hash::Hash,
};

/// Key used for indexing time-based data
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IndexKey {
    /// Unix timestamp in milliseconds
    Timestamp(i64),
    /// Year-month-day key for daily indexing
    Date(i32, u8, u8),
    /// Year-month key for monthly indexing
    Month(i32, u8),
    /// Year key for yearly indexing
    Year(i32),
    /// Hour-minute key for time-of-day indexing
    TimeOfDay(u8, u8),
    /// Day of week (1=Monday, 7=Sunday)
    DayOfWeek(u8),
    /// Custom string key
    Custom(String),
}

impl IndexKey {
    /// Creates a timestamp key from CalClock
    pub fn from_timestamp(calclock: &CalClock) -> Outcome<Self> {
        let timestamp = res!(calclock.to_millis());
        Ok(IndexKey::Timestamp(timestamp))
    }

    /// Creates a date key from CalClock
    pub fn from_date(calclock: &CalClock) -> Self {
        IndexKey::Date(
            calclock.year(),
            calclock.month(),
            calclock.day(),
        )
    }

    /// Creates a month key from CalClock
    pub fn from_month(calclock: &CalClock) -> Self {
        IndexKey::Month(calclock.year(), calclock.month())
    }

    /// Creates a year key from CalClock
    pub fn from_year(calclock: &CalClock) -> Self {
        IndexKey::Year(calclock.year())
    }

    /// Creates a time-of-day key from CalClock
    pub fn from_time_of_day(calclock: &CalClock) -> Self {
        IndexKey::TimeOfDay(calclock.hour(), calclock.minute())
    }

    /// Creates a day-of-week key from CalClock
    pub fn from_day_of_week(calclock: &CalClock) -> Self {
        IndexKey::DayOfWeek(calclock.day_of_week().of())
    }
}

impl fmt::Display for IndexKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexKey::Timestamp(ts) => write!(f, "ts:{}", ts),
            IndexKey::Date(y, m, d) => write!(f, "date:{:04}-{:02}-{:02}", y, m, d),
            IndexKey::Month(y, m) => write!(f, "month:{:04}-{:02}", y, m),
            IndexKey::Year(y) => write!(f, "year:{}", y),
            IndexKey::TimeOfDay(h, m) => write!(f, "time:{:02}:{:02}", h, m),
            IndexKey::DayOfWeek(d) => write!(f, "dow:{}", d),
            IndexKey::Custom(s) => write!(f, "custom:{}", s),
        }
    }
}

/// Entry in the time index containing the indexed data
#[derive(Debug, Clone)]
pub struct TimeIndexEntry<T> 
where 
    T: Clone,
{
    /// The time this entry represents
    pub time: CalClock,
    /// The indexed data
    pub data: T,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl<T: Clone> TimeIndexEntry<T> {
    /// Creates a new time index entry
    pub fn new(time: CalClock, data: T) -> Self {
        TimeIndexEntry {
            time,
            data,
            metadata: HashMap::new(),
        }
    }

    /// Creates a new entry with metadata
    pub fn with_metadata(time: CalClock, data: T, metadata: HashMap<String, String>) -> Self {
        TimeIndexEntry {
            time,
            data,
            metadata,
        }
    }

    /// Adds metadata to the entry
    pub fn add_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Gets metadata value by key
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}

/// Multi-level time index for efficient temporal data storage and retrieval
#[derive(Debug)]
pub struct TimeIndex<T: Clone> {
    /// Primary timestamp-based index for chronological ordering
    timestamp_index: BTreeMap<i64, Vec<TimeIndexEntry<T>>>,
    /// Secondary indexes for different time granularities
    date_index: HashMap<IndexKey, Vec<usize>>,
    month_index: HashMap<IndexKey, Vec<usize>>,
    year_index: HashMap<IndexKey, Vec<usize>>,
    time_of_day_index: HashMap<IndexKey, Vec<usize>>,
    day_of_week_index: HashMap<IndexKey, Vec<usize>>,
    /// Custom indexes
    custom_indexes: HashMap<String, HashMap<IndexKey, Vec<usize>>>,
    /// All entries for index mapping
    entries: Vec<TimeIndexEntry<T>>,
    /// Time zone for calculations
    #[allow(dead_code)]
    zone: CalClockZone,
}

impl<T: Clone> TimeIndex<T> {
    /// Creates a new time index
    pub fn new(zone: CalClockZone) -> Self {
        TimeIndex {
            timestamp_index: BTreeMap::new(),
            date_index: HashMap::new(),
            month_index: HashMap::new(),
            year_index: HashMap::new(),
            time_of_day_index: HashMap::new(),
            day_of_week_index: HashMap::new(),
            custom_indexes: HashMap::new(),
            entries: Vec::new(),
            zone,
        }
    }

    /// Adds an entry to the index
    pub fn insert(&mut self, entry: TimeIndexEntry<T>) -> Outcome<usize> {
        let index_id = self.entries.len();
        
        // Generate timestamp key
        let timestamp = res!(entry.time.to_millis());
        
        // Add to timestamp index
        self.timestamp_index
            .entry(timestamp)
            .or_insert_with(Vec::new)
            .push(entry.clone());

        // Add to secondary indexes
        self.add_to_secondary_indexes(index_id, &entry);

        // Store the entry
        self.entries.push(entry);

        Ok(index_id)
    }

    /// Adds entry to all secondary indexes
    fn add_to_secondary_indexes(&mut self, index_id: usize, entry: &TimeIndexEntry<T>) {
        // Date index
        let date_key = IndexKey::from_date(&entry.time);
        self.date_index
            .entry(date_key)
            .or_insert_with(Vec::new)
            .push(index_id);

        // Month index
        let month_key = IndexKey::from_month(&entry.time);
        self.month_index
            .entry(month_key)
            .or_insert_with(Vec::new)
            .push(index_id);

        // Year index
        let year_key = IndexKey::from_year(&entry.time);
        self.year_index
            .entry(year_key)
            .or_insert_with(Vec::new)
            .push(index_id);

        // Time of day index
        let time_key = IndexKey::from_time_of_day(&entry.time);
        self.time_of_day_index
            .entry(time_key)
            .or_insert_with(Vec::new)
            .push(index_id);

        // Day of week index
        let dow_key = IndexKey::from_day_of_week(&entry.time);
        self.day_of_week_index
            .entry(dow_key)
            .or_insert_with(Vec::new)
            .push(index_id);
    }

    /// Finds entries by exact timestamp
    pub fn find_by_timestamp(&self, timestamp: i64) -> Vec<&TimeIndexEntry<T>> {
        self.timestamp_index
            .get(&timestamp)
            .map(|entries| entries.iter().collect())
            .unwrap_or_default()
    }

    /// Finds entries by date
    pub fn find_by_date(&self, year: i32, month: u8, day: u8) -> Vec<&TimeIndexEntry<T>> {
        let key = IndexKey::Date(year, month, day);
        self.find_by_secondary_index(&self.date_index, &key)
    }

    /// Finds entries by month
    pub fn find_by_month(&self, year: i32, month: u8) -> Vec<&TimeIndexEntry<T>> {
        let key = IndexKey::Month(year, month);
        self.find_by_secondary_index(&self.month_index, &key)
    }

    /// Finds entries by year
    pub fn find_by_year(&self, year: i32) -> Vec<&TimeIndexEntry<T>> {
        let key = IndexKey::Year(year);
        self.find_by_secondary_index(&self.year_index, &key)
    }

    /// Finds entries by time of day
    pub fn find_by_time_of_day(&self, hour: u8, minute: u8) -> Vec<&TimeIndexEntry<T>> {
        let key = IndexKey::TimeOfDay(hour, minute);
        self.find_by_secondary_index(&self.time_of_day_index, &key)
    }

    /// Finds entries by day of week
    pub fn find_by_day_of_week(&self, day_of_week: u8) -> Vec<&TimeIndexEntry<T>> {
        let key = IndexKey::DayOfWeek(day_of_week);
        self.find_by_secondary_index(&self.day_of_week_index, &key)
    }

    /// Helper method to find entries using secondary indexes
    fn find_by_secondary_index(&self, index: &HashMap<IndexKey, Vec<usize>>, key: &IndexKey) -> Vec<&TimeIndexEntry<T>> {
        index
            .get(key)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&i| self.entries.get(i))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Finds entries within a timestamp range
    pub fn find_in_range(&self, start_timestamp: i64, end_timestamp: i64) -> Vec<&TimeIndexEntry<T>> {
        self.timestamp_index
            .range(start_timestamp..=end_timestamp)
            .flat_map(|(_, entries)| entries.iter())
            .collect()
    }

    /// Finds entries within a time range
    pub fn find_in_time_range(&self, start: &CalClock, end: &CalClock) -> Outcome<Vec<&TimeIndexEntry<T>>> {
        let start_ts = res!(start.to_millis());
        let end_ts = res!(end.to_millis());
        Ok(self.find_in_range(start_ts, end_ts))
    }

    /// Creates a custom index
    pub fn create_custom_index<F>(&mut self, name: String, key_extractor: F) -> Outcome<()>
    where
        F: Fn(&TimeIndexEntry<T>) -> Vec<IndexKey>,
    {
        let mut custom_index = HashMap::new();

        for (index_id, entry) in self.entries.iter().enumerate() {
            let keys = key_extractor(entry);
            for key in keys {
                custom_index
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .push(index_id);
            }
        }

        self.custom_indexes.insert(name, custom_index);
        Ok(())
    }

    /// Finds entries using a custom index
    pub fn find_by_custom_index(&self, index_name: &str, key: &IndexKey) -> Vec<&TimeIndexEntry<T>> {
        self.custom_indexes
            .get(index_name)
            .and_then(|index| index.get(key))
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&i| self.entries.get(i))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets the total number of indexed entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Checks if the index is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Gets all entries in chronological order
    pub fn iter_chronological(&self) -> impl Iterator<Item = &TimeIndexEntry<T>> {
        self.timestamp_index
            .values()
            .flat_map(|entries| entries.iter())
    }

    /// Gets statistics about the index
    pub fn statistics(&self) -> IndexStatistics {
        IndexStatistics {
            total_entries: self.entries.len(),
            unique_timestamps: self.timestamp_index.len(),
            unique_dates: self.date_index.len(),
            unique_months: self.month_index.len(),
            unique_years: self.year_index.len(),
            unique_times_of_day: self.time_of_day_index.len(),
            custom_indexes: self.custom_indexes.len(),
        }
    }
}

/// Statistics about the time index
#[derive(Debug, Clone)]
pub struct IndexStatistics {
    pub total_entries: usize,
    pub unique_timestamps: usize,
    pub unique_dates: usize,
    pub unique_months: usize,
    pub unique_years: usize,
    pub unique_times_of_day: usize,
    pub custom_indexes: usize,
}

impl fmt::Display for IndexStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, 
            "Index Statistics:\n\
             - Total entries: {}\n\
             - Unique timestamps: {}\n\
             - Unique dates: {}\n\
             - Unique months: {}\n\
             - Unique years: {}\n\
             - Unique times of day: {}\n\
             - Custom indexes: {}",
            self.total_entries,
            self.unique_timestamps,
            self.unique_dates,
            self.unique_months,
            self.unique_years,
            self.unique_times_of_day,
            self.custom_indexes
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_index_basic_operations() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();

        let entry1 = TimeIndexEntry::new(time1, "data1");
        let entry2 = TimeIndexEntry::new(time2, "data2");

        let id1 = index.insert(entry1).unwrap();
        let id2 = index.insert(entry2).unwrap();

        assert_eq!(index.len(), 2);
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
    }

    #[test]
    fn test_date_based_lookups() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 1, 16, 9, 0, 0, 0, zone.clone()).unwrap();

        index.insert(TimeIndexEntry::new(time1, "data1")).unwrap();
        index.insert(TimeIndexEntry::new(time2, "data2")).unwrap();
        index.insert(TimeIndexEntry::new(time3, "data3")).unwrap();

        let jan_15_entries = index.find_by_date(2024, 1, 15);
        assert_eq!(jan_15_entries.len(), 2);

        let jan_16_entries = index.find_by_date(2024, 1, 16);
        assert_eq!(jan_16_entries.len(), 1);

        let jan_entries = index.find_by_month(2024, 1);
        assert_eq!(jan_entries.len(), 3);
    }

    #[test]
    fn test_time_of_day_lookups() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 2, 20, 10, 30, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 3, 25, 14, 45, 0, 0, zone.clone()).unwrap();

        index.insert(TimeIndexEntry::new(time1, "morning1")).unwrap();
        index.insert(TimeIndexEntry::new(time2, "morning2")).unwrap();
        index.insert(TimeIndexEntry::new(time3, "afternoon")).unwrap();

        let morning_entries = index.find_by_time_of_day(10, 30);
        assert_eq!(morning_entries.len(), 2);

        let afternoon_entries = index.find_by_time_of_day(14, 45);
        assert_eq!(afternoon_entries.len(), 1);
    }

    #[test]
    fn test_range_queries() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 10, 12, 0, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 12, 0, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 1, 20, 12, 0, 0, 0, zone.clone()).unwrap();
        let time4 = CalClock::new(2024, 1, 25, 12, 0, 0, 0, zone.clone()).unwrap();

        index.insert(TimeIndexEntry::new(time1, "entry1")).unwrap();
        index.insert(TimeIndexEntry::new(time2, "entry2")).unwrap();
        index.insert(TimeIndexEntry::new(time3, "entry3")).unwrap();
        index.insert(TimeIndexEntry::new(time4, "entry4")).unwrap();

        let start = CalClock::new(2024, 1, 12, 0, 0, 0, 0, zone.clone()).unwrap();
        let end = CalClock::new(2024, 1, 22, 23, 59, 59, 0, zone).unwrap();

        let range_entries = index.find_in_time_range(&start, &end).unwrap();
        assert_eq!(range_entries.len(), 2); // Should include time2 and time3
    }

    #[test]
    fn test_custom_index() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 16, 14, 45, 0, 0, zone.clone()).unwrap();

        let mut entry1 = TimeIndexEntry::new(time1, "data1");
        entry1.metadata.insert("category".to_string(), "work".to_string());
        
        let mut entry2 = TimeIndexEntry::new(time2, "data2");
        entry2.metadata.insert("category".to_string(), "personal".to_string());

        index.insert(entry1).unwrap();
        index.insert(entry2).unwrap();

        // Create custom index based on category metadata
        index.create_custom_index("category".to_string(), |entry| {
            if let Some(category) = entry.metadata.get("category") {
                vec![IndexKey::Custom(category.clone())]
            } else {
                vec![]
            }
        }).unwrap();

        let work_entries = index.find_by_custom_index("category", &IndexKey::Custom("work".to_string()));
        assert_eq!(work_entries.len(), 1);

        let personal_entries = index.find_by_custom_index("category", &IndexKey::Custom("personal".to_string()));
        assert_eq!(personal_entries.len(), 1);
    }

    #[test]
    fn test_index_statistics() {
        let zone = CalClockZone::utc();
        let mut index = TimeIndex::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 2, 20, 10, 30, 0, 0, zone.clone()).unwrap();

        index.insert(TimeIndexEntry::new(time1, "data1")).unwrap();
        index.insert(TimeIndexEntry::new(time2, "data2")).unwrap();
        index.insert(TimeIndexEntry::new(time3, "data3")).unwrap();

        let stats = index.statistics();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.unique_dates, 2);
        assert_eq!(stats.unique_months, 2);
        assert_eq!(stats.unique_years, 1);
        assert_eq!(stats.unique_times_of_day, 2);
    }
}