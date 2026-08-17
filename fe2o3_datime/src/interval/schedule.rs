//! Schedules: named collections of events, each an interval with an optional
//! recurrence, that can be searched by date and checked for conflicts.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    interval::{CalClockRange, RecurrencePattern},
    time::CalClock,
};

use oxedyne_fe2o3_core::prelude::*;


#[derive(Clone, Debug)]
pub struct ScheduleEvent {
    id:             String,
    range:          CalClockRange,
    title:          String,
    description:    Option<String>,
    recurrence:     Option<RecurrencePattern>,
    priority:       u8,                         // higher number wins
    flexible:       bool,                       // may be moved to clear a conflict
}

impl ScheduleEvent {
    pub fn new<S: Into<String>>(
        id: S,
        range: CalClockRange,
        title: S,
    ) -> Self {
        Self {
            id: id.into(),
            range,
            title: title.into(),
            description: None,
            recurrence: None,
            priority: 0,
            flexible: false,
        }
    }
    
    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }
    
    pub fn recurrence(mut self, pattern: RecurrencePattern) -> Self {
        self.recurrence = Some(pattern);
        self
    }
    
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
    
    pub fn flexible(mut self, flexible: bool) -> Self {
        self.flexible = flexible;
        self
    }
    
    pub fn id(&self) -> &str {
        &self.id
    }
    
    pub fn range(&self) -> &CalClockRange {
        &self.range
    }
    
    pub fn title(&self) -> &str {
        &self.title
    }
    
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    
    pub fn get_recurrence(&self) -> Option<&RecurrencePattern> {
        self.recurrence.as_ref()
    }
    
    pub fn get_priority(&self) -> u8 {
        self.priority
    }
    
    pub fn is_flexible(&self) -> bool {
        self.flexible
    }
    
    pub fn occurrences_in_range(
        &self,
        start_date: &CalendarDate,
        end_date: &CalendarDate,
    ) -> Outcome<Vec<CalClockRange>> {
        if let Some(ref recurrence) = self.get_recurrence() {
            let occurrences = res!(recurrence.occurrences_in_range(start_date, end_date));
            let duration = res!(self.range.duration());
            
            let mut ranges = Vec::new();
            for occurrence in occurrences {
                let end_time = res!(occurrence.add_duration(&duration));
                ranges.push(res!(CalClockRange::new(occurrence, end_time)));
            }
            
            Ok(ranges)
        } else {
            // Single occurrence
            if self.range.start().date() >= start_date && self.range.start().date() <= end_date {
                Ok(vec![self.range.clone()])
            } else {
                Ok(vec![])
            }
        }
    }
}

#[derive(Debug)]
pub struct Schedule {
    events: Vec<ScheduleEvent>,
    name:   String,
}

impl Schedule {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            events: Vec::new(),
            name: name.into(),
        }
    }
    
    pub fn name(&self) -> &str {
        &self.name
    }
    
    pub fn add_event(&mut self, event: ScheduleEvent) {
        self.events.push(event);
    }
    
    pub fn remove_event(&mut self, event_id: &str) -> bool {
        if let Some(pos) = self.events.iter().position(|e| e.id() == event_id) {
            self.events.remove(pos);
            true
        } else {
            false
        }
    }
    
    pub fn events(&self) -> &[ScheduleEvent] {
        &self.events
    }
    
    pub fn find_event(&self, event_id: &str) -> Option<&ScheduleEvent> {
        self.events.iter().find(|e| e.id() == event_id)
    }
    
    pub fn find_event_mut(&mut self, event_id: &str) -> Option<&mut ScheduleEvent> {
        self.events.iter_mut().find(|e| e.id() == event_id)
    }
    
    pub fn events_in_range(
        &self,
        start_date: &CalendarDate,
        end_date: &CalendarDate,
    ) -> Outcome<Vec<(String, CalClockRange)>> {
        let mut result = Vec::new();
        
        for event in &self.events {
            let occurrences = res!(event.occurrences_in_range(start_date, end_date));
            for occurrence in occurrences {
                result.push((event.id().to_string(), occurrence));
            }
        }
        
        // Sort by start time
        result.sort_by(|a, b| a.1.start().cmp(b.1.start()));
        
        Ok(result)
    }
    
    pub fn events_on_date(&self, date: &CalendarDate) -> Outcome<Vec<(String, CalClockRange)>> {
        self.events_in_range(date, date)
    }
    
    /// Overlapping events are gathered into groups, so three events that all
    /// clash appear once rather than as three pairs.
    pub fn detect_conflicts(&self, start_date: &CalendarDate, end_date: &CalendarDate) -> Outcome<Vec<ConflictGroup>> {
        let events_in_range = res!(self.events_in_range(start_date, end_date));
        let mut conflicts: Vec<ConflictGroup> = Vec::new();
        
        for i in 0..events_in_range.len() {
            for j in (i + 1)..events_in_range.len() {
                let (ref event1_id, ref range1) = &events_in_range[i];
                let (ref event2_id, ref range2) = &events_in_range[j];
                
                if res!(range1.overlaps(range2)) {
                    // Check if we already have a conflict group containing these events
                    let mut found_group = None;
                    for (idx, group) in conflicts.iter_mut().enumerate() {
                        if group.contains_event(&event1_id) || group.contains_event(&event2_id) {
                            found_group = Some(idx);
                            break;
                        }
                    }
                    
                    if let Some(idx) = found_group {
                        conflicts[idx].add_event(event1_id.clone(), range1.clone());
                        conflicts[idx].add_event(event2_id.clone(), range2.clone());
                    } else {
                        let mut group = ConflictGroup::new();
                        group.add_event(event1_id.clone(), range1.clone());
                        group.add_event(event2_id.clone(), range2.clone());
                        conflicts.push(group);
                    }
                }
            }
        }
        
        Ok(conflicts)
    }
    
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[derive(Debug)]
pub struct ConflictGroup {
    events: Vec<(String, CalClockRange)>,   // id and range of each
}

impl ConflictGroup {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
        }
    }
    
    pub fn add_event(&mut self, event_id: String, range: CalClockRange) {
        // Only add if not already present
        if !self.events.iter().any(|(id, _)| id == &event_id) {
            self.events.push((event_id, range));
        }
    }
    
    pub fn contains_event(&self, event_id: &str) -> bool {
        self.events.iter().any(|(id, _)| id == event_id)
    }
    
    pub fn events(&self) -> &[(String, CalClockRange)] {
        &self.events
    }
    
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
    
    pub fn overall_range(&self) -> Outcome<Option<CalClockRange>> {
        if self.events.is_empty() {
            return Ok(None);
        }
        
        let mut min_start = self.events[0].1.start().clone();
        let mut max_end = self.events[0].1.end().clone();
        
        for (_, range) in &self.events[1..] {
            if range.start() < &min_start {
                min_start = range.start().clone();
            }
            if range.end() > &max_end {
                max_end = range.end().clone();
            }
        }
        
        Ok(Some(res!(CalClockRange::new(min_start, max_end))))
    }
}

impl Default for ConflictGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct ScheduleBuilder {
    schedule: Schedule,
}

impl ScheduleBuilder {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            schedule: Schedule::new(name),
        }
    }
    
    pub fn add_event(mut self, event: ScheduleEvent) -> Self {
        self.schedule.add_event(event);
        self
    }
    
    pub fn add_simple_event<S: Into<String>>(
        mut self,
        id: S,
        range: CalClockRange,
        title: S,
    ) -> Self {
        let event = ScheduleEvent::new(id, range, title);
        self.schedule.add_event(event);
        self
    }
    
    pub fn add_recurring_event<S: Into<String>>(
        mut self,
        id: S,
        range: CalClockRange,
        title: S,
        recurrence: RecurrencePattern,
    ) -> Self {
        let event = ScheduleEvent::new(id, range, title).recurrence(recurrence);
        self.schedule.add_event(event);
        self
    }
    
    pub fn build(self) -> Schedule {
        self.schedule
    }
}

// ========================================================================
// Common Schedule Patterns
// ========================================================================

impl Schedule {
    pub fn work_schedule(name: String, start_date: CalendarDate) -> Outcome<Self> {
        use crate::interval::RecurrenceRule;
        use std::collections::HashSet;
        
        let mut weekdays = HashSet::new();
        weekdays.insert(crate::constant::DayOfWeek::Monday);
        weekdays.insert(crate::constant::DayOfWeek::Tuesday);
        weekdays.insert(crate::constant::DayOfWeek::Wednesday);
        weekdays.insert(crate::constant::DayOfWeek::Thursday);
        weekdays.insert(crate::constant::DayOfWeek::Friday);
        
        let work_start = res!(CalClock::new(
            start_date.year(),
            start_date.month(),
            start_date.day(),
            9, 0, 0, 0,
            start_date.zone().clone()
        ));
        
        let work_end = res!(CalClock::new(
            start_date.year(),
            start_date.month(),
            start_date.day(),
            17, 0, 0, 0,
            start_date.zone().clone()
        ));
        
        let work_range = res!(CalClockRange::new(work_start.clone(), work_end));
        
        let recurrence_rule = RecurrenceRule::new(crate::interval::Frequency::Weekly)
            .by_weekday(weekdays);
        
        let recurrence_pattern = RecurrencePattern::new(work_start, recurrence_rule);
        
        let work_event = ScheduleEvent::new("work", work_range, "Work Hours")
            .recurrence(recurrence_pattern)
            .priority(5);
        
        let mut schedule = Schedule::new(name);
        schedule.add_event(work_event);
        
        Ok(schedule)
    }
    
    pub fn class_schedule(name: String, start_date: CalendarDate) -> Outcome<Self> {
        let mut schedule = Schedule::new(name);
        
        // Add some example classes
        let zone = start_date.zone().clone();
        
        // Monday, Wednesday, Friday - Math class
        let math_start = res!(CalClock::new(
            start_date.year(), start_date.month(), start_date.day(),
            10, 0, 0, 0, zone.clone()
        ));
        let math_end = res!(CalClock::new(
            start_date.year(), start_date.month(), start_date.day(),
            11, 30, 0, 0, zone.clone()
        ));
        let math_range = res!(CalClockRange::new(math_start.clone(), math_end));
        
        use std::collections::HashSet;
        let mut mwf_days = HashSet::new();
        mwf_days.insert(crate::constant::DayOfWeek::Monday);
        mwf_days.insert(crate::constant::DayOfWeek::Wednesday);
        mwf_days.insert(crate::constant::DayOfWeek::Friday);
        
        let math_rule = crate::interval::RecurrenceRule::new(crate::interval::Frequency::Weekly)
            .by_weekday(mwf_days);
        let math_pattern = RecurrencePattern::new(math_start, math_rule);
        
        let math_event = ScheduleEvent::new("math", math_range, "Mathematics")
            .recurrence(math_pattern)
            .description("Advanced Calculus")
            .priority(8);
        
        schedule.add_event(math_event);
        
        Ok(schedule)
    }
}