/// Range-based indexing for efficient temporal range queries
/// 
/// This module provides specialized indexing for time ranges and intervals,
/// optimized for overlapping range queries and interval operations.

use oxedyne_fe2o3_core::prelude::*;
use crate::{
    time::{CalClock, CalClockZone},
    interval::CalClockRange,
};
use std::{
    collections::BTreeMap,
    fmt,
};

/// Query parameters for range-based searches
#[derive(Debug, Clone)]
pub struct RangeQuery {
    /// Start time of the query range
    pub start: CalClock,
    /// End time of the query range
    pub end: CalClock,
    /// Whether to include ranges that partially overlap
    pub include_partial_overlaps: bool,
    /// Whether to include ranges that are completely contained within the query
    pub include_contained: bool,
    /// Whether to include ranges that completely contain the query
    pub include_containing: bool,
}

impl RangeQuery {
    /// Creates a new range query
    pub fn new(start: CalClock, end: CalClock) -> Self {
        RangeQuery {
            start,
            end,
            include_partial_overlaps: true,
            include_contained: true,
            include_containing: true,
        }
    }

    /// Creates a query that only finds exact overlaps
    pub fn exact_overlaps(start: CalClock, end: CalClock) -> Self {
        RangeQuery {
            start,
            end,
            include_partial_overlaps: true,
            include_contained: false,
            include_containing: false,
        }
    }

    /// Creates a query that only finds contained ranges
    pub fn contained_ranges(start: CalClock, end: CalClock) -> Self {
        RangeQuery {
            start,
            end,
            include_partial_overlaps: false,
            include_contained: true,
            include_containing: false,
        }
    }

    /// Creates a query that only finds containing ranges
    pub fn containing_ranges(start: CalClock, end: CalClock) -> Self {
        RangeQuery {
            start,
            end,
            include_partial_overlaps: false,
            include_contained: false,
            include_containing: true,
        }
    }
}

/// Result of a range query
#[derive(Debug, Clone)]
pub struct RangeResult<T> {
    /// The indexed range
    pub range: CalClockRange,
    /// The associated data
    pub data: T,
    /// Type of overlap with the query
    pub overlap_type: OverlapType,
    /// Percentage of overlap (0.0 to 1.0)
    pub overlap_percentage: f64,
}

/// Type of overlap between ranges
#[derive(Debug, Clone, PartialEq)]
pub enum OverlapType {
    /// Query range is completely contained within this range
    QueryContained,
    /// This range is completely contained within query range
    RangeContained,
    /// Ranges partially overlap at the start
    PartialStart,
    /// Ranges partially overlap at the end
    PartialEnd,
    /// Ranges are identical
    Identical,
    /// No overlap
    NoOverlap,
}

/// Indexed range entry
#[derive(Debug, Clone)]
pub struct RangeEntry<T> {
    /// The time range
    pub range: CalClockRange,
    /// Associated data
    pub data: T,
    /// Entry ID for tracking
    pub id: usize,
}

impl<T> RangeEntry<T> {
    /// Creates a new range entry
    pub fn new(range: CalClockRange, data: T, id: usize) -> Self {
        RangeEntry { range, data, id }
    }

    /// Checks if this range overlaps with another range
    pub fn overlaps_with(&self, other_range: &CalClockRange) -> Outcome<bool> {
        self.range.overlaps(other_range)
    }

    /// Calculates the overlap type and percentage with a query range
    pub fn calculate_overlap(&self, query: &RangeQuery) -> Outcome<(OverlapType, f64)> {
        let query_start_ts = res!(query.start.to_millis());
        let query_end_ts = res!(query.end.to_millis());
        let range_start_ts = res!(self.range.start().to_millis());
        let range_end_ts = res!(self.range.end().to_millis());

        // Check for no overlap
        if range_end_ts < query_start_ts || range_start_ts > query_end_ts {
            return Ok((OverlapType::NoOverlap, 0.0));
        }

        // Check for identical ranges
        if range_start_ts == query_start_ts && range_end_ts == query_end_ts {
            return Ok((OverlapType::Identical, 1.0));
        }

        // Check for containment relationships
        if range_start_ts <= query_start_ts && range_end_ts >= query_end_ts {
            // Query is contained within this range
            let query_duration = query_end_ts - query_start_ts;
            let range_duration = range_end_ts - range_start_ts;
            let percentage = if range_duration > 0 {
                query_duration as f64 / range_duration as f64
            } else {
                1.0
            };
            return Ok((OverlapType::QueryContained, percentage));
        }

        if query_start_ts <= range_start_ts && query_end_ts >= range_end_ts {
            // This range is contained within query
            let range_duration = range_end_ts - range_start_ts;
            let query_duration = query_end_ts - query_start_ts;
            let percentage = if query_duration > 0 {
                range_duration as f64 / query_duration as f64
            } else {
                1.0
            };
            return Ok((OverlapType::RangeContained, percentage));
        }

        // Partial overlaps
        let overlap_start = std::cmp::max(range_start_ts, query_start_ts);
        let overlap_end = std::cmp::min(range_end_ts, query_end_ts);
        let overlap_duration = overlap_end - overlap_start;

        let range_duration = range_end_ts - range_start_ts;
        let percentage = if range_duration > 0 {
            overlap_duration as f64 / range_duration as f64
        } else {
            0.0
        };

        let overlap_type = if range_start_ts < query_start_ts {
            OverlapType::PartialEnd
        } else {
            OverlapType::PartialStart
        };

        Ok((overlap_type, percentage))
    }
}

/// Range index for efficient range-based queries
#[derive(Debug)]
pub struct RangeIndex<T> {
    /// Index by start time for efficient range queries
    start_index: BTreeMap<i64, Vec<usize>>,
    /// Index by end time
    end_index: BTreeMap<i64, Vec<usize>>,
    /// All indexed ranges
    entries: Vec<RangeEntry<T>>,
    /// Time zone for calculations
    #[allow(dead_code)]
    zone: CalClockZone,
    /// Next ID for entries
    next_id: usize,
}

impl<T> RangeIndex<T> {
    /// Creates a new range index
    pub fn new(zone: CalClockZone) -> Self {
        RangeIndex {
            start_index: BTreeMap::new(),
            end_index: BTreeMap::new(),
            entries: Vec::new(),
            zone,
            next_id: 0,
        }
    }

    /// Adds a range to the index
    pub fn insert(&mut self, range: CalClockRange, data: T) -> Outcome<usize> {
        let entry_id = self.next_id;
        self.next_id += 1;

        let start_ts = res!(range.start().to_millis());
        let end_ts = res!(range.end().to_millis());

        // Add to start index
        self.start_index
            .entry(start_ts)
            .or_insert_with(Vec::new)
            .push(self.entries.len());

        // Add to end index
        self.end_index
            .entry(end_ts)
            .or_insert_with(Vec::new)
            .push(self.entries.len());

        // Store the entry
        let entry = RangeEntry::new(range, data, entry_id);
        self.entries.push(entry);

        Ok(entry_id)
    }

    /// Queries ranges that overlap with the given query
    pub fn query(&self, query: &RangeQuery) -> Outcome<Vec<RangeResult<&T>>> {
        let query_start_ts = res!(query.start.to_millis());
        let query_end_ts = res!(query.end.to_millis());

        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Find all ranges that could potentially overlap
        // Look at ranges that start before or at query end
        for (&_start_ts, entry_indices) in self.start_index.range(..=query_end_ts) {
            for &entry_idx in entry_indices {
                if seen.contains(&entry_idx) {
                    continue;
                }
                seen.insert(entry_idx);

                if let Some(entry) = self.entries.get(entry_idx) {
                    let end_ts = res!(entry.range.end().to_millis());
                    
                    // Skip if range ends before query starts
                    if end_ts < query_start_ts {
                        continue;
                    }

                    let (overlap_type, overlap_percentage) = res!(entry.calculate_overlap(query));

                    // Filter based on query preferences
                    let include = match overlap_type {
                        OverlapType::NoOverlap => false,
                        OverlapType::Identical => true,
                        OverlapType::QueryContained => query.include_containing,
                        OverlapType::RangeContained => query.include_contained,
                        OverlapType::PartialStart | OverlapType::PartialEnd => query.include_partial_overlaps,
                    };

                    if include {
                        results.push(RangeResult {
                            range: entry.range.clone(),
                            data: &entry.data,
                            overlap_type,
                            overlap_percentage,
                        });
                    }
                }
            }
        }

        // Sort results by start time
        results.sort_by(|a, b| {
            a.range.start().partial_cmp(b.range.start()).unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Finds all ranges that start within a time period
    pub fn find_starting_in_range(&self, start: &CalClock, end: &CalClock) -> Outcome<Vec<&RangeEntry<T>>> {
        let start_ts = res!(start.to_millis());
        let end_ts = res!(end.to_millis());

        let mut results = Vec::new();

        for (&_ts, entry_indices) in self.start_index.range(start_ts..=end_ts) {
            for &entry_idx in entry_indices {
                if let Some(entry) = self.entries.get(entry_idx) {
                    results.push(entry);
                }
            }
        }

        Ok(results)
    }

    /// Finds all ranges that end within a time period
    pub fn find_ending_in_range(&self, start: &CalClock, end: &CalClock) -> Outcome<Vec<&RangeEntry<T>>> {
        let start_ts = res!(start.to_millis());
        let end_ts = res!(end.to_millis());

        let mut results = Vec::new();

        for (&_ts, entry_indices) in self.end_index.range(start_ts..=end_ts) {
            for &entry_idx in entry_indices {
                if let Some(entry) = self.entries.get(entry_idx) {
                    results.push(entry);
                }
            }
        }

        Ok(results)
    }

    /// Gets the total number of indexed ranges
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Checks if the index is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Gets all entries
    pub fn all_entries(&self) -> &[RangeEntry<T>] {
        &self.entries
    }

    /// Removes an entry by ID
    pub fn remove(&mut self, entry_id: usize) -> Outcome<Option<RangeEntry<T>>> {
        if let Some(pos) = self.entries.iter().position(|e| e.id == entry_id) {
            let entry = self.entries.remove(pos);
            
            // Rebuild indexes (could be optimized but good enough for now)
            self.rebuild_indexes()?;
            
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }

    /// Rebuilds the indexes from current entries
    fn rebuild_indexes(&mut self) -> Outcome<()> {
        self.start_index.clear();
        self.end_index.clear();

        for (idx, entry) in self.entries.iter().enumerate() {
            let start_ts = res!(entry.range.start().to_millis());
            let end_ts = res!(entry.range.end().to_millis());

            self.start_index
                .entry(start_ts)
                .or_insert_with(Vec::new)
                .push(idx);

            self.end_index
                .entry(end_ts)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        Ok(())
    }
}

impl fmt::Display for OverlapType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverlapType::QueryContained => write!(f, "Query Contained"),
            OverlapType::RangeContained => write!(f, "Range Contained"),
            OverlapType::PartialStart => write!(f, "Partial Start"),
            OverlapType::PartialEnd => write!(f, "Partial End"),
            OverlapType::Identical => write!(f, "Identical"),
            OverlapType::NoOverlap => write!(f, "No Overlap"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_index_basic_operations() {
        let zone = CalClockZone::utc();
        let mut index = RangeIndex::new(zone.clone());

        let start1 = CalClock::new(2024, 1, 1, 10, 0, 0, 0, zone.clone()).unwrap();
        let end1 = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap();
        let range1 = CalClockRange::new(start1, end1).unwrap();

        let start2 = CalClock::new(2024, 1, 1, 14, 0, 0, 0, zone.clone()).unwrap();
        let end2 = CalClock::new(2024, 1, 1, 16, 0, 0, 0, zone.clone()).unwrap();
        let range2 = CalClockRange::new(start2, end2).unwrap();

        let id1 = index.insert(range1, "meeting1").unwrap();
        let id2 = index.insert(range2, "meeting2").unwrap();

        assert_eq!(index.len(), 2);
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
    }

    #[test]
    fn test_overlap_detection() {
        let zone = CalClockZone::utc();
        let mut index = RangeIndex::new(zone.clone());

        // Add overlapping ranges
        let range1_start = CalClock::new(2024, 1, 1, 10, 0, 0, 0, zone.clone()).unwrap();
        let range1_end = CalClock::new(2024, 1, 1, 14, 0, 0, 0, zone.clone()).unwrap();
        let range1 = CalClockRange::new(range1_start, range1_end).unwrap();

        let range2_start = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap();
        let range2_end = CalClock::new(2024, 1, 1, 16, 0, 0, 0, zone.clone()).unwrap();
        let range2 = CalClockRange::new(range2_start, range2_end).unwrap();

        index.insert(range1, "event1").unwrap();
        index.insert(range2, "event2").unwrap();

        // Query for overlaps
        let query_start = CalClock::new(2024, 1, 1, 11, 0, 0, 0, zone.clone()).unwrap();
        let query_end = CalClock::new(2024, 1, 1, 13, 0, 0, 0, zone).unwrap();
        let query = RangeQuery::new(query_start, query_end);

        let results = index.query(&query).unwrap();
        assert_eq!(results.len(), 2); // Both ranges should overlap
    }

    #[test]
    fn test_containment_queries() {
        let zone = CalClockZone::utc();
        let mut index = RangeIndex::new(zone.clone());

        // Large containing range
        let large_start = CalClock::new(2024, 1, 1, 8, 0, 0, 0, zone.clone()).unwrap();
        let large_end = CalClock::new(2024, 1, 1, 18, 0, 0, 0, zone.clone()).unwrap();
        let large_range = CalClockRange::new(large_start, large_end).unwrap();

        // Small contained range
        let small_start = CalClock::new(2024, 1, 1, 10, 0, 0, 0, zone.clone()).unwrap();
        let small_end = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap();
        let small_range = CalClockRange::new(small_start, small_end).unwrap();

        index.insert(large_range, "full_day").unwrap();
        index.insert(small_range, "meeting").unwrap();

        // Query that should be contained within large range
        let query_start = CalClock::new(2024, 1, 1, 9, 0, 0, 0, zone.clone()).unwrap();
        let query_end = CalClock::new(2024, 1, 1, 11, 0, 0, 0, zone).unwrap();
        let query = RangeQuery::containing_ranges(query_start, query_end);

        let results = index.query(&query).unwrap();
        assert_eq!(results.len(), 1); // Only the large range should match
        assert_eq!(results[0].overlap_type, OverlapType::QueryContained);
    }

    #[test]
    fn test_range_removal() {
        let zone = CalClockZone::utc();
        let mut index = RangeIndex::new(zone.clone());

        let start = CalClock::new(2024, 1, 1, 10, 0, 0, 0, zone.clone()).unwrap();
        let end = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone).unwrap();
        let range = CalClockRange::new(start, end).unwrap();

        let id = index.insert(range, "test").unwrap();
        assert_eq!(index.len(), 1);

        let removed = index.remove(id).unwrap();
        assert!(removed.is_some());
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_overlap_percentage_calculation() {
        let zone = CalClockZone::utc();
        
        let start1 = CalClock::new(2024, 1, 1, 10, 0, 0, 0, zone.clone()).unwrap();
        let end1 = CalClock::new(2024, 1, 1, 14, 0, 0, 0, zone.clone()).unwrap(); // 4 hours
        let range = CalClockRange::new(start1, end1).unwrap();
        let entry = RangeEntry::new(range, "test", 0);

        // Query that overlaps 2 hours (50% of the range)
        let query_start = CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap();
        let query_end = CalClock::new(2024, 1, 1, 16, 0, 0, 0, zone).unwrap();
        let query = RangeQuery::new(query_start, query_end);

        let (overlap_type, percentage) = entry.calculate_overlap(&query).unwrap();
        assert_eq!(overlap_type, OverlapType::PartialEnd);
        assert!((percentage - 0.5).abs() < 0.01); // Should be 50%
    }
}