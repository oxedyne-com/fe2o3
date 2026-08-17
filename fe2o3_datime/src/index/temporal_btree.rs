//! Temporal B-tree for time-based data.
//!
//! Entries are held in timestamp order with secondary indexes on category,
//! recurrence, business day and priority, so a query filtered on any of those
//! can start from the most selective index rather than scanning.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use crate::time::{CalClock, CalClockZone};
use std::{
    collections::BTreeMap,
    fmt,
    cmp::Ordering,
};

#[derive(Debug, Clone)]
pub struct TemporalEntry<T> 
where 
    T: Clone,
{
    pub timestamp:      i64,            // Unix milliseconds
    pub time:           CalClock,
    pub data:           T,
    pub temporal_attrs: TemporalAttributes,
}

#[derive(Debug, Clone)]
pub struct TemporalAttributes {
    pub duration_millis:    Option<i64>,    // None means a point in time
    pub recurrence_id:      Option<String>,
    pub category:           Option<String>,
    pub is_business_day:    bool,
    pub is_holiday:         bool,
    pub priority:           u8,
}

impl Default for TemporalAttributes {
    fn default() -> Self {
        TemporalAttributes {
            duration_millis: None,
            recurrence_id: None,
            category: None,
            is_business_day: true,
            is_holiday: false,
            priority: 0,
        }
    }
}

impl<T: Clone> TemporalEntry<T> {
    pub fn new(time: CalClock, data: T) -> Outcome<Self> {
        let timestamp = res!(time.to_millis());
        Ok(TemporalEntry {
            timestamp,
            time,
            data,
            temporal_attrs: TemporalAttributes::default(),
        })
    }

    pub fn with_attributes(time: CalClock, data: T, attrs: TemporalAttributes) -> Outcome<Self> {
        let timestamp = res!(time.to_millis());
        Ok(TemporalEntry {
            timestamp,
            time,
            data,
            temporal_attrs: attrs,
        })
    }

    pub fn with_duration(mut self, duration_millis: i64) -> Self {
        self.temporal_attrs.duration_millis = Some(duration_millis);
        self
    }

    pub fn with_recurrence(mut self, recurrence_id: String) -> Self {
        self.temporal_attrs.recurrence_id = Some(recurrence_id);
        self
    }

    pub fn with_category(mut self, category: String) -> Self {
        self.temporal_attrs.category = Some(category);
        self
    }

    pub fn with_business_day(mut self, is_business_day: bool) -> Self {
        self.temporal_attrs.is_business_day = is_business_day;
        self
    }

    pub fn with_holiday(mut self, is_holiday: bool) -> Self {
        self.temporal_attrs.is_holiday = is_holiday;
        self
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.temporal_attrs.priority = priority;
        self
    }

    pub fn end_timestamp(&self) -> Option<i64> {
        self.temporal_attrs.duration_millis.map(|d| self.timestamp + d)
    }

    /// A point entry matches either bound, a range entry treats its own end as exclusive.
    pub fn overlaps_range(&self, start_ts: i64, end_ts: i64) -> bool {
        if let Some(end_timestamp) = self.end_timestamp() {
            // Range-based entry
            !(end_timestamp <= start_ts || self.timestamp >= end_ts)
        } else {
            // Point-in-time entry
            self.timestamp >= start_ts && self.timestamp <= end_ts
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemporalQuery {
    pub start_time:         Option<CalClock>,
    pub end_time:           Option<CalClock>,
    pub category_filter:    Option<String>,
    pub recurrence_filter:  Option<String>,
    pub business_days_only: bool,
    pub exclude_holidays:   bool,
    pub min_priority:       Option<u8>,     // inclusive floor
    pub limit:              Option<usize>,
    pub sort_order:         SortOrder,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortOrder {
    TimeAscending,      // chronological
    TimeDescending,
    PriorityThenTime,
    CategoryThenTime,
}

impl Default for TemporalQuery {
    fn default() -> Self {
        TemporalQuery {
            start_time: None,
            end_time: None,
            category_filter: None,
            recurrence_filter: None,
            business_days_only: false,
            exclude_holidays: false,
            min_priority: None,
            limit: None,
            sort_order: SortOrder::TimeAscending,
        }
    }
}

impl TemporalQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn time_range(mut self, start: CalClock, end: CalClock) -> Self {
        self.start_time = Some(start);
        self.end_time = Some(end);
        self
    }

    pub fn category(mut self, category: String) -> Self {
        self.category_filter = Some(category);
        self
    }

    pub fn recurrence(mut self, recurrence_id: String) -> Self {
        self.recurrence_filter = Some(recurrence_id);
        self
    }

    pub fn business_days_only(mut self) -> Self {
        self.business_days_only = true;
        self
    }

    pub fn exclude_holidays(mut self) -> Self {
        self.exclude_holidays = true;
        self
    }

    pub fn min_priority(mut self, priority: u8) -> Self {
        self.min_priority = Some(priority);
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn sort_by(mut self, order: SortOrder) -> Self {
        self.sort_order = order;
        self
    }
}

#[derive(Debug)]
pub struct TemporalBTree<T: Clone> {
    // The primary tree owns clones of the entries. Each secondary index is
    // keyed by its attribute and then by timestamp, and holds positions into
    // `all_entries`, so a query can pick whichever index narrows the search
    // most and still come out in time order.
    primary_tree: BTreeMap<i64, Vec<TemporalEntry<T>>>,
    category_index: BTreeMap<String, BTreeMap<i64, Vec<usize>>>,
    recurrence_index: BTreeMap<String, BTreeMap<i64, Vec<usize>>>,
    business_day_index: BTreeMap<i64, Vec<usize>>,
    priority_index: BTreeMap<u8, BTreeMap<i64, Vec<usize>>>,
    all_entries: Vec<TemporalEntry<T>>,
    #[allow(dead_code)]
    zone: CalClockZone,
}

impl<T: Clone> TemporalBTree<T> {
    pub fn new(zone: CalClockZone) -> Self {
        TemporalBTree {
            primary_tree: BTreeMap::new(),
            category_index: BTreeMap::new(),
            recurrence_index: BTreeMap::new(),
            business_day_index: BTreeMap::new(),
            priority_index: BTreeMap::new(),
            all_entries: Vec::new(),
            zone,
        }
    }

    /// Returns the position of the new entry, which the secondary indexes use to refer to it.
    pub fn insert(&mut self, entry: TemporalEntry<T>) -> Outcome<usize> {
        let entry_index = self.all_entries.len();
        let timestamp = entry.timestamp;

        // Add to primary tree
        self.primary_tree
            .entry(timestamp)
            .or_insert_with(Vec::new)
            .push(entry.clone());

        // Add to secondary indexes
        self.add_to_secondary_indexes(entry_index, &entry);

        // Store the entry
        self.all_entries.push(entry);

        Ok(entry_index)
    }

    fn add_to_secondary_indexes(&mut self, entry_index: usize, entry: &TemporalEntry<T>) {
        let timestamp = entry.timestamp;

        // Category index
        if let Some(ref category) = entry.temporal_attrs.category {
            self.category_index
                .entry(category.clone())
                .or_insert_with(BTreeMap::new)
                .entry(timestamp)
                .or_insert_with(Vec::new)
                .push(entry_index);
        }

        // Recurrence index
        if let Some(ref recurrence_id) = entry.temporal_attrs.recurrence_id {
            self.recurrence_index
                .entry(recurrence_id.clone())
                .or_insert_with(BTreeMap::new)
                .entry(timestamp)
                .or_insert_with(Vec::new)
                .push(entry_index);
        }

        // Business day index
        if entry.temporal_attrs.is_business_day {
            self.business_day_index
                .entry(timestamp)
                .or_insert_with(Vec::new)
                .push(entry_index);
        }

        // Priority index
        self.priority_index
            .entry(entry.temporal_attrs.priority)
            .or_insert_with(BTreeMap::new)
            .entry(timestamp)
            .or_insert_with(Vec::new)
            .push(entry_index);
    }

    pub fn query(&self, query: &TemporalQuery) -> Outcome<Vec<&TemporalEntry<T>>> {
        let mut results = Vec::new();

        // Determine time range
        let (start_ts, end_ts) = if let (Some(start), Some(end)) = (&query.start_time, &query.end_time) {
            (res!(start.to_millis()), res!(end.to_millis()))
        } else if let Some(start) = &query.start_time {
            (res!(start.to_millis()), i64::MAX)
        } else if let Some(end) = &query.end_time {
            (i64::MIN, res!(end.to_millis()))
        } else {
            (i64::MIN, i64::MAX)
        };

        // Choose the most selective index
        let candidate_indices = ok!(self.get_candidate_indices(query, start_ts, end_ts));

        // Filter candidates
        for &entry_index in &candidate_indices {
            if let Some(entry) = self.all_entries.get(entry_index) {
                if self.matches_query(entry, query, start_ts, end_ts) {
                    results.push(entry);
                }
            }
        }

        // Sort results
        self.sort_results(&mut results, &query.sort_order);

        // Apply limit
        if let Some(limit) = query.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    /// Picks whichever index narrows the search most.
    fn get_candidate_indices(&self, query: &TemporalQuery, start_ts: i64, end_ts: i64) -> Outcome<Vec<usize>> {
        let mut candidates = Vec::new();

        // Use the most selective index available
        if let Some(ref category) = query.category_filter {
            if let Some(category_tree) = self.category_index.get(category) {
                for (_, indices) in category_tree.range(start_ts..=end_ts) {
                    candidates.extend(indices);
                }
            }
        } else if let Some(ref recurrence) = query.recurrence_filter {
            if let Some(recurrence_tree) = self.recurrence_index.get(recurrence) {
                for (_, indices) in recurrence_tree.range(start_ts..=end_ts) {
                    candidates.extend(indices);
                }
            }
        } else if query.business_days_only {
            for (_, indices) in self.business_day_index.range(start_ts..=end_ts) {
                candidates.extend(indices);
            }
        } else if let Some(priority) = query.min_priority {
            for priority_level in priority..=255 {
                if let Some(priority_tree) = self.priority_index.get(&priority_level) {
                    for (_, indices) in priority_tree.range(start_ts..=end_ts) {
                        candidates.extend(indices);
                    }
                }
            }
        } else {
            // Use primary tree - add all entries in range
            for i in 0..self.all_entries.len() {
                let entry = &self.all_entries[i];
                if entry.timestamp >= start_ts && entry.timestamp <= end_ts {
                    candidates.push(i);
                }
            }
        }

        // Remove duplicates
        candidates.sort();
        candidates.dedup();

        Ok(candidates)
    }

    fn matches_query(&self, entry: &TemporalEntry<T>, query: &TemporalQuery, start_ts: i64, end_ts: i64) -> bool {
        // Time range check
        if !entry.overlaps_range(start_ts, end_ts) {
            return false;
        }

        // Category filter
        if let Some(ref category) = query.category_filter {
            if entry.temporal_attrs.category.as_ref() != Some(category) {
                return false;
            }
        }

        // Recurrence filter
        if let Some(ref recurrence) = query.recurrence_filter {
            if entry.temporal_attrs.recurrence_id.as_ref() != Some(recurrence) {
                return false;
            }
        }

        // Business days filter
        if query.business_days_only && !entry.temporal_attrs.is_business_day {
            return false;
        }

        // Holiday exclusion
        if query.exclude_holidays && entry.temporal_attrs.is_holiday {
            return false;
        }

        // Priority filter
        if let Some(min_priority) = query.min_priority {
            if entry.temporal_attrs.priority < min_priority {
                return false;
            }
        }

        true
    }

    fn sort_results(&self, results: &mut Vec<&TemporalEntry<T>>, sort_order: &SortOrder) {
        match sort_order {
            SortOrder::TimeAscending => {
                results.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
            }
            SortOrder::TimeDescending => {
                results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            }
            SortOrder::PriorityThenTime => {
                results.sort_by(|a, b| {
                    match b.temporal_attrs.priority.cmp(&a.temporal_attrs.priority) {
                        Ordering::Equal => a.timestamp.cmp(&b.timestamp),
                        other => other,
                    }
                });
            }
            SortOrder::CategoryThenTime => {
                results.sort_by(|a, b| {
                    match a.temporal_attrs.category.cmp(&b.temporal_attrs.category) {
                        Ordering::Equal => a.timestamp.cmp(&b.timestamp),
                        other => other,
                    }
                });
            }
        }
    }

    /// Nearest in either direction.
    pub fn find_nearest(&self, target: &CalClock) -> Outcome<Option<&TemporalEntry<T>>> {
        let target_ts = res!(target.to_millis());
        
        let mut nearest: Option<&TemporalEntry<T>> = None;
        let mut min_distance = i64::MAX;

        // Search around the target timestamp
        let search_range = 100; // entries to check on each side
        let mut count = 0;

        // Check entries before and after target
        for (_, entries) in self.primary_tree.iter() {
            for entry in entries {
                let distance = (entry.timestamp - target_ts).abs();
                if distance < min_distance {
                    min_distance = distance;
                    nearest = Some(entry);
                }
            }
            count += 1;
            if count > search_range {
                break;
            }
        }

        Ok(nearest)
    }

    pub fn find_within_duration(&self, target: &CalClock, duration_millis: i64) -> Outcome<Vec<&TemporalEntry<T>>> {
        let target_ts = res!(target.to_millis());
        let start_ts = target_ts - duration_millis;
        let end_ts = target_ts + duration_millis;

        let mut results = Vec::new();

        for (_, entries) in self.primary_tree.range(start_ts..=end_ts) {
            results.extend(entries.iter());
        }

        Ok(results)
    }

    pub fn statistics(&self) -> TemporalStatistics {
        TemporalStatistics {
            total_entries: self.all_entries.len(),
            unique_timestamps: self.primary_tree.len(),
            categories: self.category_index.len(),
            recurrence_patterns: self.recurrence_index.len(),
            business_day_entries: self.business_day_index.values().map(|v| v.len()).sum(),
            priority_levels: self.priority_index.len(),
        }
    }

    pub fn len(&self) -> usize {
        self.all_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all_entries.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct TemporalStatistics {
    pub total_entries: usize,
    pub unique_timestamps: usize,
    pub categories: usize,
    pub recurrence_patterns: usize,
    pub business_day_entries: usize,
    pub priority_levels: usize,
}

impl fmt::Display for TemporalStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,
            "Temporal B-Tree Statistics:\n\
             - Total entries: {}\n\
             - Unique timestamps: {}\n\
             - Categories: {}\n\
             - Recurrence patterns: {}\n\
             - Business day entries: {}\n\
             - Priority levels: {}",
            self.total_entries,
            self.unique_timestamps,
            self.categories,
            self.recurrence_patterns,
            self.business_day_entries,
            self.priority_levels
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temporal_btree_basic_operations() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time1, "data1").unwrap()
            .with_category("work".to_string())
            .with_priority(1);
        
        let entry2 = TemporalEntry::new(time2, "data2").unwrap()
            .with_category("personal".to_string())
            .with_priority(2);

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();

        assert_eq!(btree.len(), 2);
    }

    #[test]
    fn test_temporal_query_by_category() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 1, 16, 9, 0, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time1, "work1").unwrap()
            .with_category("work".to_string());
        
        let entry2 = TemporalEntry::new(time2, "personal1").unwrap()
            .with_category("personal".to_string());
        
        let entry3 = TemporalEntry::new(time3, "work2").unwrap()
            .with_category("work".to_string());

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();
        btree.insert(entry3).unwrap();

        let query = TemporalQuery::new()
            .category("work".to_string());

        let results = btree.query(&query).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_temporal_query_with_time_range() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 10, 12, 0, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 12, 0, 0, 0, zone.clone()).unwrap();
        let time3 = CalClock::new(2024, 1, 20, 12, 0, 0, 0, zone.clone()).unwrap();
        let time4 = CalClock::new(2024, 1, 25, 12, 0, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time1, "entry1").unwrap();
        let entry2 = TemporalEntry::new(time2, "entry2").unwrap();
        let entry3 = TemporalEntry::new(time3, "entry3").unwrap();
        let entry4 = TemporalEntry::new(time4, "entry4").unwrap();

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();
        btree.insert(entry3).unwrap();
        btree.insert(entry4).unwrap();

        let start = CalClock::new(2024, 1, 12, 0, 0, 0, 0, zone.clone()).unwrap();
        let end = CalClock::new(2024, 1, 22, 23, 59, 59, 0, zone).unwrap();

        let query = TemporalQuery::new()
            .time_range(start, end);

        let results = btree.query(&query).unwrap();
        assert_eq!(results.len(), 2); // Should include time2 and time3
    }

    #[test]
    fn test_priority_sorting() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time = CalClock::new(2024, 1, 15, 12, 0, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time.clone(), "low").unwrap().with_priority(1);
        let entry2 = TemporalEntry::new(time.clone(), "high").unwrap().with_priority(5);
        let entry3 = TemporalEntry::new(time, "medium").unwrap().with_priority(3);

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();
        btree.insert(entry3).unwrap();

        let query = TemporalQuery::new()
            .sort_by(SortOrder::PriorityThenTime);

        let results = btree.query(&query).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].temporal_attrs.priority, 5); // Highest priority first
        assert_eq!(results[1].temporal_attrs.priority, 3);
        assert_eq!(results[2].temporal_attrs.priority, 1);
    }

    #[test]
    fn test_nearest_entry_search() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 0, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 16, 0, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time1, "morning").unwrap();
        let entry2 = TemporalEntry::new(time2, "afternoon").unwrap();

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();

        let target = CalClock::new(2024, 1, 15, 12, 0, 0, 0, zone).unwrap();
        let nearest = btree.find_nearest(&target).unwrap();

        assert!(nearest.is_some());
        // Should find the 10:00 entry as it's closer to 12:00 than 16:00
        assert_eq!(nearest.unwrap().data, "morning");
    }

    #[test]
    fn test_statistics() {
        let zone = CalClockZone::utc();
        let mut btree = TemporalBTree::new(zone.clone());

        let time1 = CalClock::new(2024, 1, 15, 10, 30, 0, 0, zone.clone()).unwrap();
        let time2 = CalClock::new(2024, 1, 15, 14, 45, 0, 0, zone.clone()).unwrap();

        let entry1 = TemporalEntry::new(time1, "data1").unwrap()
            .with_category("work".to_string())
            .with_business_day(true);
        
        let entry2 = TemporalEntry::new(time2, "data2").unwrap()
            .with_category("personal".to_string())
            .with_business_day(false);

        btree.insert(entry1).unwrap();
        btree.insert(entry2).unwrap();

        let stats = btree.statistics();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.categories, 2);
        assert_eq!(stats.business_day_entries, 1);
    }
}