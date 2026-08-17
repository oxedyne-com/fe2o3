//! Spans of time, in three flavours: a CalClockRange between two instants, a
//! DateRange over whole days, and a TimeRange within a day.
//!
//! They do not agree on their end bounds, so check each one.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    core::Duration,
    time::{CalClock, CalClockDuration},
};

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct CalClockRange {
    start: CalClock,
    end: CalClock,
}

impl CalClockRange {
    /// The start is inclusive and the end exclusive.
    pub fn new(start: CalClock, end: CalClock) -> Outcome<Self> {
        if start > end {
            return Err(err!("Start time cannot be after end time"; Invalid, Input));
        }
        Ok(Self { start, end })
    }
    
    pub fn from_start_and_duration(start: CalClock, duration: CalClockDuration) -> Outcome<Self> {
        let end = res!(start.add_duration(&duration));
        Self::new(start, end)
    }
    
    pub fn start(&self) -> &CalClock {
        &self.start
    }
    
    pub fn end(&self) -> &CalClock {
        &self.end
    }
    
    pub fn duration(&self) -> Outcome<CalClockDuration> {
        self.start.duration_until(&self.end)
    }
    
    pub fn contains(&self, time: &CalClock) -> Outcome<bool> {
        Ok(time >= &self.start && time < &self.end)
    }
    
    pub fn overlaps(&self, other: &Self) -> Outcome<bool> {
        Ok(!(self.end <= other.start || other.end <= self.start))
    }
    
    pub fn intersection(&self, other: &Self) -> Outcome<Option<Self>> {
        if !res!(self.overlaps(other)) {
            return Ok(None);
        }
        
        let start = if self.start > other.start { self.start.clone() } else { other.start.clone() };
        let end = if self.end < other.end { self.end.clone() } else { other.end.clone() };
        
        Ok(Some(res!(Self::new(start, end))))
    }
    
    /// Adjacent ranges unite; disjoint ones give None.
    pub fn union(&self, other: &Self) -> Outcome<Option<Self>> {
        // Check if ranges overlap or are adjacent
        let gap_duration = if self.end <= other.start {
            res!(self.end.duration_until(&other.start))
        } else if other.end <= self.start {
            res!(other.end.duration_until(&self.start))
        } else {
            // Ranges overlap
            CalClockDuration::from_nanos(0)
        };
        
        // Only union if ranges overlap or are adjacent (no gap)
        if res!(gap_duration.to_nanos()) > 0 {
            return Ok(None);
        }
        
        let start = if self.start < other.start { self.start.clone() } else { other.start.clone() };
        let end = if self.end > other.end { self.end.clone() } else { other.end.clone() };
        
        Ok(Some(res!(Self::new(start, end))))
    }
    
    /// A split point outside the range is an error, and one at either end
    /// gives a single range rather than two.
    pub fn split_at(&self, split_time: &CalClock) -> Outcome<Vec<Self>> {
        if !res!(self.contains(split_time)) {
            return Err(err!("Split time is not within the range"; Invalid, Input));
        }
        
        let mut result = Vec::new();
        
        // First range: start to split_time
        if split_time > &self.start {
            result.push(res!(Self::new(self.start.clone(), split_time.clone())));
        }
        
        // Second range: split_time to end
        if split_time < &self.end {
            result.push(res!(Self::new(split_time.clone(), self.end.clone())));
        }
        
        Ok(result)
    }
    
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
    
    pub fn extend_to_include(&mut self, time: &CalClock) -> Outcome<()> {
        if time < &self.start {
            self.start = time.clone();
        } else if time >= &self.end {
            self.end = res!(time.add_millis(1)); // Make end exclusive
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DateRange {
    start: CalendarDate,
    end: CalendarDate,
}

impl DateRange {
    /// Both ends are inclusive, unlike CalClockRange.
    pub fn new(start: CalendarDate, end: CalendarDate) -> Outcome<Self> {
        if start > end {
            return Err(err!("Start date cannot be after end date"; Invalid, Input));
        }
        Ok(Self { start, end })
    }
    
    pub fn start(&self) -> &CalendarDate {
        &self.start
    }
    
    pub fn end(&self) -> &CalendarDate {
        &self.end
    }
    
    pub fn days(&self) -> Outcome<i32> {
        let start_day_number = res!(self.start.to_day_number());
        let end_day_number = res!(self.end.to_day_number());
        Ok((end_day_number - start_day_number + 1) as i32)
    }
    
    pub fn contains(&self, date: &CalendarDate) -> bool {
        date >= &self.start && date <= &self.end
    }
    
    pub fn overlaps(&self, other: &Self) -> bool {
        !(self.end < other.start || other.end < self.start)
    }
    
    pub fn all_dates(&self) -> Outcome<Vec<CalendarDate>> {
        let mut dates = Vec::new();
        let mut current = self.start.clone();
        
        while current <= self.end {
            dates.push(current.clone());
            current = res!(current.add_days(1));
        }
        
        Ok(dates)
    }
    
    pub fn business_days(&self) -> Outcome<Vec<CalendarDate>> {
        let all_dates = res!(self.all_dates());
        Ok(all_dates.into_iter().filter(|date| date.is_business_day()).collect())
    }
    
    pub fn weekends(&self) -> Outcome<Vec<CalendarDate>> {
        let all_dates = res!(self.all_dates());
        Ok(all_dates.into_iter().filter(|date| date.is_weekend()).collect())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeRange {
    start: ClockTime,
    end: ClockTime,
}

impl TimeRange {
    /// Both ends are inclusive, and an end at or before the start means the
    /// range wraps midnight.
    pub fn new(start: ClockTime, end: ClockTime) -> Outcome<Self> {
        // Allow ranges that span midnight (end < start)
        Ok(Self { start, end })
    }
    
    pub fn start(&self) -> &ClockTime {
        &self.start
    }
    
    pub fn end(&self) -> &ClockTime {
        &self.end
    }
    
    pub fn spans_midnight(&self) -> bool {
        self.end <= self.start
    }
    
    pub fn contains(&self, time: &ClockTime) -> bool {
        if self.spans_midnight() {
            // Range spans midnight, so time is either >= start OR <= end
            time >= &self.start || time <= &self.end
        } else {
            // Normal range
            time >= &self.start && time <= &self.end
        }
    }
    
    pub fn duration(&self) -> Outcome<crate::clock::ClockDuration> {
        use crate::clock::ClockDuration;
        
        if self.spans_midnight() {
            // Calculate duration across midnight
            let nanos_to_midnight = (24 * 60 * 60 * 1_000_000_000) - self.start.to_nanos_of_day() as i64;
            let nanos_from_midnight = self.end.to_nanos_of_day() as i64;
            let total_nanos = nanos_to_midnight + nanos_from_midnight;
            Ok(ClockDuration::from_nanos(total_nanos))
        } else {
            let start_nanos = self.start.to_nanos_of_day() as i64;
            let end_nanos = self.end.to_nanos_of_day() as i64;
            Ok(ClockDuration::from_nanos(end_nanos - start_nanos))
        }
    }
    
    pub fn split_at(&self, split_time: &ClockTime) -> Outcome<Vec<Self>> {
        if !self.contains(split_time) {
            return Err(err!("Split time is not within the range"; Invalid, Input));
        }
        
        let mut result = Vec::new();
        
        if self.spans_midnight() {
            // Handle midnight-spanning ranges
            if split_time >= &self.start {
                // Split is in the first part (before midnight)
                result.push(res!(Self::new(self.start.clone(), split_time.clone())));
                if split_time != &self.start {
                    // Add midnight-spanning part
                    result.push(res!(Self::new(split_time.clone(), self.end.clone())));
                }
            } else {
                // Split is in the second part (after midnight)
                result.push(res!(Self::new(self.start.clone(), self.end.clone()))); // This will span midnight
                result.push(res!(Self::new(split_time.clone(), self.end.clone())));
            }
        } else {
            // Normal range
            if split_time > &self.start {
                result.push(res!(Self::new(self.start.clone(), split_time.clone())));
            }
            if split_time < &self.end {
                result.push(res!(Self::new(split_time.clone(), self.end.clone())));
            }
        }
        
        Ok(result)
    }
}

// ========================================================================
// Range Collection Operations
// ========================================================================

#[derive(Clone, Debug)]
pub struct CalClockRangeSet {
    ranges: Vec<CalClockRange>,
}

impl CalClockRangeSet {
    pub fn new() -> Self {
        Self {
            ranges: Vec::new(),
        }
    }
    
    pub fn from_ranges(ranges: Vec<CalClockRange>) -> Outcome<Self> {
        let mut set = Self::new();
        for range in ranges {
            res!(set.add_range(range));
        }
        Ok(set)
    }
    
    /// Overlapping and adjacent ranges are merged on the way in, so the set
    /// never holds two ranges that touch.
    pub fn add_range(&mut self, new_range: CalClockRange) -> Outcome<()> {
        // Find overlapping ranges
        let mut merged = new_range;
        let mut to_remove = Vec::new();
        
        for (i, existing) in self.ranges.iter().enumerate() {
            if let Some(union) = res!(merged.union(existing)) {
                merged = union;
                to_remove.push(i);
            }
        }
        
        // Remove merged ranges (in reverse order to maintain indices)
        for &i in to_remove.iter().rev() {
            self.ranges.remove(i);
        }
        
        // Add the merged range
        self.ranges.push(merged);
        
        // Sort ranges by start time
        self.ranges.sort_by(|a, b| a.start().cmp(b.start()));
        
        Ok(())
    }
    
    pub fn ranges(&self) -> &[CalClockRange] {
        &self.ranges
    }
    
    pub fn contains(&self, time: &CalClock) -> Outcome<bool> {
        for range in &self.ranges {
            if res!(range.contains(time)) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    
    pub fn gaps(&self) -> Outcome<Vec<CalClockRange>> {
        if self.ranges.len() <= 1 {
            return Ok(Vec::new());
        }
        
        let mut gaps = Vec::new();
        
        for i in 0..self.ranges.len() - 1 {
            let current_end = self.ranges[i].end();
            let next_start = self.ranges[i + 1].start();
            
            if current_end < next_start {
                gaps.push(res!(CalClockRange::new(current_end.clone(), next_start.clone())));
            }
        }
        
        Ok(gaps)
    }
    
    pub fn total_duration(&self) -> Outcome<CalClockDuration> {
        let mut total_nanos = 0i64;
        
        for range in &self.ranges {
            let duration = res!(range.duration());
            total_nanos += res!(duration.to_nanos());
        }
        
        Ok(CalClockDuration::from_nanos(total_nanos))
    }
    
    pub fn intersection(&self, other: &Self) -> Outcome<Self> {
        let mut result = Self::new();
        
        for range1 in &self.ranges {
            for range2 in &other.ranges {
                if let Some(intersection) = res!(range1.intersection(range2)) {
                    res!(result.add_range(intersection));
                }
            }
        }
        
        Ok(result)
    }
    
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    
    pub fn len(&self) -> usize {
        self.ranges.len()
    }
}

impl Default for CalClockRangeSet {
    fn default() -> Self {
        Self::new()
    }
}