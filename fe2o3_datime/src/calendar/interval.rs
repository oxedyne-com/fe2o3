use crate::{
    calendar::{
        CalendarDate,
        CalendarDuration,
    },
    core::{Interval, Time},
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

/// An interval between two calendar dates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarInterval {
    start:	CalendarDate,
    end:	CalendarDate,
}

impl CalendarInterval {
    pub fn new(start: CalendarDate, end: CalendarDate) -> Outcome<Self> {
        if end.is_before(&start) {
            return Err(err!(
                "End date {} is before start date {}",
                end, start;
                Invalid, Input, Order));
        }
        
        Ok(Self { start, end })
    }
    
    pub fn start(&self) -> &CalendarDate {
        &self.start
    }
    
    pub fn end(&self) -> &CalendarDate {
        &self.end
    }
    
    pub fn duration(&self) -> Outcome<CalendarDuration> {
        self.end.minus_date(&self.start)
    }
}

impl Interval<CalendarDuration> for CalendarInterval {
    fn get_duration(&self) -> Outcome<CalendarDuration> {
        self.duration()
    }
    
    fn contains<T: Time>(&self, _time: &T) -> bool {
        // Placeholder - would need type-specific logic
        false
    }
    
    fn overlaps(&self, _other: &Self) -> bool {
        // Placeholder - would need proper date comparison logic
        false
    }
}

impl Display for CalendarInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} to {}", self.start, self.end)
    }
}